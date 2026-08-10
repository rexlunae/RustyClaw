//! Runs a focused subagent when the model calls `subagent_run`.
//!
//! This is a headless tool loop modeled on `trigger_dispatch`: no client
//! frames, no streaming, no user present. What makes it different from every
//! other loop in the gateway is *focus*:
//!
//! * The subagent's system prompt is the profile's prompt — not the main
//!   agent's — plus isolated workspace context and a listing of its toolset.
//! * Its model calls only present the profile's tool allowlist
//!   (`ProviderRequest::allowed_tools`), so the model never even sees the
//!   rest of the registry.
//! * The only conversation context it has is the task and the context the
//!   parent explicitly fed it.
//!
//! The run is synchronous from the parent's perspective: the parent's
//! `subagent_run` tool call blocks until the subagent finishes, and the
//! subagent's final message is returned as the tool result. The run is also
//! recorded in the global session manager so `sessions_list` /
//! `sessions_history` show it.

use std::path::Path;

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{ChatMessage, ProviderRequest, ToolCallResult};
use rustyclaw_core::sessions::{SessionStatus, session_manager};
use rustyclaw_core::subagents::{SubagentProfile, SubagentRegistry};
use rustyclaw_core::tools::{self, ToolPermission};
use rustyclaw_core::workspace_context::{SessionType, SubagentInfo, WorkspaceContext};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::providers;

/// Per-model-call timeout inside a subagent run.
const MODEL_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Execute a `subagent_run` tool call. Returns `(output, is_error)` in the
/// same shape as the other gateway tool executors.
pub async fn execute_subagent_run(
    http: &reqwest::Client,
    parent: &ProviderRequest,
    args: &Value,
    config: &Config,
    workspace_dir: &Path,
) -> (String, bool) {
    match run_inner(http, parent, args, config, workspace_dir).await {
        Ok(output) => (output, false),
        Err(e) => (format!("subagent_run failed: {:#}", e), true),
    }
}

async fn run_inner(
    http: &reqwest::Client,
    parent: &ProviderRequest,
    args: &Value,
    config: &Config,
    workspace_dir: &Path,
) -> anyhow::Result<String> {
    let profile_id = args
        .get("profile")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: profile"))?;
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("Missing required parameter: task"))?;
    let context = args.get("context").and_then(|v| v.as_str());
    let label = args.get("label").and_then(|v| v.as_str()).map(String::from);
    let model_override = args.get("model").and_then(|v| v.as_str());

    let registry = SubagentRegistry::new(&config.settings_dir);
    let Some(profile) = registry.get(profile_id) else {
        let available: Vec<String> = registry.list().into_iter().map(|p| p.id).collect();
        anyhow::bail!(
            "Unknown subagent profile '{}'. Available profiles: {}. \
             Use subagent_list for details or subagent_create to add one.",
            profile_id,
            available.join(", ")
        );
    };

    info!(
        profile = %profile.id,
        task = %task.chars().take(80).collect::<String>(),
        "Starting subagent run"
    );

    // ── Record the run as a session (visible via sessions_list) ─────────
    let agent_id =
        rustyclaw_core::runtime_ctx::get_active_agent().unwrap_or_else(|| "main".to_string());
    // Whoever asked for this run. Recorded as the parent so the fan-out
    // ceiling can count a caller's children — including a sub-agent's own,
    // since a sub-agent spawning sub-agents is how a single prompt turns
    // into an unbounded tree.
    let parent_caller = rustyclaw_core::tool_caller::current();
    let session_key = {
        let mut mgr = session_manager()
            .lock()
            .map_err(|_| anyhow::anyhow!("Session manager lock poisoned"))?;
        if let Some(parent_key) = parent_caller.as_deref() {
            let running = mgr.active_subagents_of(parent_key);
            if let Err(e) = rustyclaw_core::tool_limits::check_subagent(running) {
                anyhow::bail!("{e}");
            }
        }
        let key = mgr.spawn_subagent(&agent_id, task, label.clone(), parent_caller.clone());
        if let Some(session) = mgr.get_mut(&key) {
            session.add_message("system", &format!("subagent profile: {}", profile.id));
            session.add_message("user", task);
        }
        key
    };

    // Owned by the same agent the session record above is filed under, so a
    // transfer this subagent starts is listed and stoppable in that agent's
    // panel. Untagged it would belong to nobody — and because the write loop
    // only stops when the registry marks the transfer terminal, nothing could
    // stop it.
    // Scoped to this subagent's own session key, not the parent's thread: a
    // subagent is exactly the "other session" that must not reach into the
    // conversation that spawned it, or into a sibling subagent.
    let result = rustyclaw_core::tool_caller::with_caller(
        format!("subagent:{session_key}"),
        rustyclaw_core::downloads::with_origin(
            rustyclaw_core::downloads::headless_origin(&agent_id),
            drive_tool_loop(
                http,
                parent,
                &profile,
                task,
                context,
                label.as_deref(),
                model_override,
                config,
                workspace_dir,
            ),
        ),
    )
    .await;

    // ── Close the session record ────────────────────────────────────────
    //
    // Through `settle`, which leaves an already-ended record alone. A
    // subagent runs inside its caller's task, so when `sessions_kill` stops a
    // background run it marks this record Stopped too — correctly, since
    // aborting the ancestor ends this run with it. If the synchronous run
    // happened to return inside the stop's grace window, an unconditional
    // `complete()` here would flip that Stopped back to Completed and report
    // cancelled work as having succeeded.
    if let Ok(mut mgr) = session_manager().lock() {
        if let Some(session) = mgr.get_mut(&session_key) {
            match &result {
                Ok(text) => {
                    session.add_message("assistant", text);
                    session.settle(SessionStatus::Completed);
                }
                Err(e) => {
                    session.add_message("assistant", &format!("error: {:#}", e));
                    session.settle(SessionStatus::Error);
                }
            };
        }
    }

    let text = result?;
    Ok(format!("Subagent '{}' completed.\n\n{}", profile.id, text))
}

/// Build the subagent's system prompt: the profile's focused prompt, the
/// isolated-session workspace context, and its exact toolset.
fn build_subagent_system_prompt(
    profile: &SubagentProfile,
    task: &str,
    label: Option<&str>,
    config: &Config,
    workspace_dir: &Path,
) -> String {
    let mut prompt = format!("# {}\n\n{}\n\n", profile.name, profile.system_prompt);

    let ws = WorkspaceContext::for_subagent(
        workspace_dir.to_path_buf(),
        config.workspace_context.clone(),
        SubagentInfo {
            parent_key: None,
            task: Some(task.to_string()),
            label: label.map(String::from),
        },
    );
    // Files only: the generic isolated-session guidance claims the parent's
    // full toolset (sessions_send, secrets_list, …), which would contradict
    // the restricted toolset section below.
    let ws_context = ws.build_context_files_only(SessionType::Isolated);
    if !ws_context.is_empty() {
        prompt.push_str(&ws_context);
        prompt.push_str("\n\n");
    }

    prompt.push_str(&tools::subagent_tools::describe_toolset(&profile.tools));
    prompt.push_str(
        "\n## Session Rules\n\
         - You are a subagent run by a main agent; there is no human in this \
         session. Never ask questions — decide, act, and report.\n\
         - Work only from the task and context given. When something essential \
         is missing, state exactly what is missing in your final report.\n\
         - Your final message is delivered to the main agent as your complete \
         result. Make it self-contained.\n",
    );
    prompt
}

/// Drive the bounded headless tool loop for one subagent run.
#[allow(clippy::too_many_arguments)]
async fn drive_tool_loop(
    http: &reqwest::Client,
    parent: &ProviderRequest,
    profile: &SubagentProfile,
    task: &str,
    context: Option<&str>,
    label: Option<&str>,
    model_override: Option<&str>,
    config: &Config,
    workspace_dir: &Path,
) -> anyhow::Result<String> {
    let system_prompt = build_subagent_system_prompt(profile, task, label, config, workspace_dir);

    let mut user_message = format!("## Task\n{}\n", task);
    if let Some(context) = context.filter(|c| !c.trim().is_empty()) {
        user_message.push_str(&format!("\n## Context from the main agent\n{}\n", context));
    }

    // The subagent reuses the parent's provider credentials. A model
    // override (run arg, then profile) must stay within the same provider —
    // cross-provider runs would need different endpoints and keys.
    let model = model_override
        .or(profile.model.as_deref())
        .unwrap_or(&parent.model)
        .to_string();

    let mut resolved = ProviderRequest {
        messages: vec![
            ChatMessage::text("system", &system_prompt),
            ChatMessage::text("user", &user_message),
        ],
        model,
        provider: parent.provider.clone(),
        base_url: parent.base_url.clone(),
        api_key: parent.api_key.clone(),
        allowed_tools: Some(profile.tools.clone()),
        reasoning_effort: parent.reasoning_effort.clone(),
        max_tokens: parent.max_tokens,
        temperature: parent.temperature,
    };

    let max_rounds = profile.effective_max_rounds();
    let mut last_text = String::new();

    for round in 0..max_rounds {
        let model_resp = tokio::time::timeout(
            MODEL_CALL_TIMEOUT,
            providers::call_with_tools(http, &resolved, None),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Subagent model call timed out after {}s",
                MODEL_CALL_TIMEOUT.as_secs()
            )
        })??;

        if !model_resp.text.is_empty() {
            last_text = model_resp.text.clone();
        }
        if model_resp.tool_calls.is_empty() {
            debug!(profile = %profile.id, rounds = round + 1, "Subagent finished");
            if last_text.trim().is_empty() {
                anyhow::bail!("Subagent produced no output");
            }
            return Ok(last_text);
        }

        let mut tool_results: Vec<ToolCallResult> = Vec::new();
        for tc in &model_resp.tool_calls {
            debug!(profile = %profile.id, tool = %tc.name, "Subagent tool call");
            let (output, is_error) =
                execute_subagent_tool(&tc.name, &tc.arguments, profile, config, workspace_dir)
                    .await;
            if is_error {
                warn!(profile = %profile.id, tool = %tc.name, "Subagent tool call failed");
            }
            tool_results.push(ToolCallResult {
                id: tc.id.clone(),
                name: tc.name.clone(),
                output,
                is_error,
            });
        }

        providers::append_tool_round(
            &resolved.provider,
            &mut resolved.messages,
            &model_resp,
            &tool_results,
        );
    }

    anyhow::bail!(
        "Subagent hit its round limit ({}) before finishing. Last message: {}",
        max_rounds,
        if last_text.is_empty() {
            "(none)"
        } else {
            &last_text
        }
    )
}

/// Execute one tool call inside a subagent run, enforcing the profile
/// allowlist and the user's per-tool permission policy.
async fn execute_subagent_tool(
    name: &str,
    arguments: &Value,
    profile: &SubagentProfile,
    config: &Config,
    workspace_dir: &Path,
) -> (String, bool) {
    // Defense in depth: the model should only ever see allowlisted tools
    // (allowed_tools filters the schemas), but a model can still hallucinate
    // a call to a tool it was never offered.
    if !profile.tools.iter().any(|t| t == name) {
        return (
            format!(
                "Tool '{}' is not in this subagent's toolset. Available tools: {}.",
                name,
                profile.tools.join(", ")
            ),
            true,
        );
    }

    // Honor the user's per-tool permission policy. Like trigger-fired runs,
    // a subagent run is fully autonomous — nobody can answer an approval
    // prompt — so anything not outright `Allow` is refused.
    let permission = config
        .tool_permissions
        .get(name)
        .cloned()
        .unwrap_or_default();
    if !matches!(permission, ToolPermission::Allow) {
        let reason = match permission {
            ToolPermission::Deny => "denied by user policy",
            ToolPermission::Ask => {
                "requires user approval, which is unavailable in an autonomous subagent run"
            }
            ToolPermission::SkillOnly(_) => "restricted to skill invocations",
            ToolPermission::Allow => unreachable!(),
        };
        return (format!("Tool '{}' {}.", name, reason), true);
    }

    // Rate limiting shared with every other loop in the gateway.
    if let Err(err) = crate::tool_executor::check_rate_limit(name) {
        return (err.to_string(), true);
    }

    match tools::execute_tool(name, arguments, workspace_dir).await {
        Ok(text) => (tools::sanitize_tool_output(text), false),
        Err(err) => (err.to_string(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::subagents::builtin_profiles;

    fn test_config(dir: &Path) -> Config {
        Config {
            settings_dir: dir.to_path_buf(),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn tool_outside_allowlist_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        let profile = builtin_profiles()
            .into_iter()
            .find(|p| p.id == "doc-writer")
            .unwrap();

        let (output, is_error) = execute_subagent_tool(
            "execute_command",
            &serde_json::json!({"command": "true"}),
            &profile,
            &config,
            tmp.path(),
        )
        .await;
        assert!(is_error);
        assert!(output.contains("not in this subagent's toolset"));
    }

    #[tokio::test]
    async fn non_allow_permission_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = test_config(tmp.path());
        config
            .tool_permissions
            .insert("read_file".to_string(), ToolPermission::Ask);
        let profile = builtin_profiles()
            .into_iter()
            .find(|p| p.id == "doc-writer")
            .unwrap();

        let (output, is_error) = execute_subagent_tool(
            "read_file",
            &serde_json::json!({"path": "x.txt"}),
            &profile,
            &config,
            tmp.path(),
        )
        .await;
        assert!(is_error);
        assert!(output.contains("requires user approval"));
    }

    #[tokio::test]
    async fn allowlisted_tool_executes() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        std::fs::write(tmp.path().join("note.txt"), "hello subagent").unwrap();
        let profile = builtin_profiles()
            .into_iter()
            .find(|p| p.id == "doc-writer")
            .unwrap();

        let (output, is_error) = execute_subagent_tool(
            "read_file",
            &serde_json::json!({"path": "note.txt"}),
            &profile,
            &config,
            tmp.path(),
        )
        .await;
        assert!(!is_error, "unexpected error: {output}");
        assert!(output.contains("hello subagent"));
    }

    #[test]
    fn system_prompt_is_focused() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(tmp.path());
        let profile = builtin_profiles()
            .into_iter()
            .find(|p| p.id == "code-reviewer")
            .unwrap();

        let prompt = build_subagent_system_prompt(
            &profile,
            "review auth.rs",
            Some("review-auth"),
            &config,
            tmp.path(),
        );
        // Profile prompt and toolset are present…
        assert!(prompt.contains("code-review agent"));
        assert!(prompt.contains("`read_file`"));
        // …and the toolset section only lists allowlisted tools.
        assert!(!prompt.contains("`write_file`"));
        assert!(prompt.contains("no human in this session"));
        // The generic isolated-session guidance must not leak in: it claims
        // the parent's full toolset and names tools (plain, un-backticked)
        // that this subagent can never use.
        assert!(!prompt.contains("Sub-Agent Guidelines"));
        assert!(!prompt.contains("same tools as the parent"));
        for denied in [
            "write_file",
            "sessions_send",
            "secrets_list",
            "task_describe",
            "cron",
        ] {
            assert!(
                !prompt.contains(denied),
                "prompt must not mention excluded tool '{denied}'"
            );
        }
    }
}
