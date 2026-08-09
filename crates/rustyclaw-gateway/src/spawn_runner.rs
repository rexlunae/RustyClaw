//! Runs `sessions_spawn` sub-agents in the background.
//!
//! `sessions_spawn` used to be a promise with nothing behind it: it wrote a
//! session record, returned `"accepted"`, and no work ever happened. The tool
//! description has been telling models it "runs asynchronously" the whole
//! time, so a model that delegated work simply watched it never get done.
//!
//! This makes the promise true. Core keeps the session records but has no
//! model client, so it exposes a [`SpawnRunner`] hook; the gateway installs
//! the implementation below at startup and *every* caller of the tool —
//! client dispatch, a trigger fire, a cron job, another sub-agent — gets a
//! real run through the same door, with no per-call-site interception.
//!
//! Because the run is detached, two things a synchronous `subagent_run` never
//! needed become mandatory:
//!
//! * **It must be stoppable.** Nothing else would ever end a runaway loop —
//!   the turn that spawned it returned long ago. Every run is registered in
//!   [`running_sessions`] with a cancel flag, and `sessions_kill` uses it.
//! * **It must not be able to ask for anything.** There is no user attached,
//!   so any tool the policy does not outright `Allow` is refused rather than
//!   left waiting for an approval that cannot arrive.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{ChatMessage, ProviderRequest, ToolCallResult};
use rustyclaw_core::sessions::{SpawnRequest, SpawnRunner, running_sessions, session_manager};
use rustyclaw_core::tools;
use tracing::{debug, info, warn};

use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedSkillManager, SharedTaskManager,
    SharedVault, providers, tool_executor,
};

/// Maximum tool rounds for one spawned run, matching the other headless loops.
const MAX_TOOL_ROUNDS: usize = 25;

/// Per-model-call timeout inside a spawned run.
const MODEL_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The gateway's implementation of core's spawn hook.
///
/// Everything is held by handle rather than by value: a run outlives the turn
/// that started it, and must see later config edits the same way a foreground
/// turn does.
pub struct GatewaySpawnRunner {
    http: reqwest::Client,
    config: SharedConfig,
    model_ctx: SharedModelCtx,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: SharedTaskManager,
    /// Needed to exchange a Copilot OAuth token for the short-lived session
    /// token the provider actually accepts.
    copilot_session: SharedCopilotSession,
    /// Captured at install time. The tool executors are synchronous functions
    /// and may be driven from a blocking pool, where `tokio::spawn` would
    /// panic for want of a reactor.
    runtime: tokio::runtime::Handle,
}

impl GatewaySpawnRunner {
    pub fn new(
        http: reqwest::Client,
        config: SharedConfig,
        model_ctx: SharedModelCtx,
        vault: SharedVault,
        skill_mgr: SharedSkillManager,
        task_mgr: SharedTaskManager,
        copilot_session: SharedCopilotSession,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            http,
            config,
            model_ctx,
            vault,
            skill_mgr,
            task_mgr,
            copilot_session,
            runtime,
        }
    }
}

impl SpawnRunner for GatewaySpawnRunner {
    fn start(&self, req: SpawnRequest) -> Result<(), String> {
        // Without a model there is nothing to run, and finding that out now
        // means the caller gets a refusal instead of a session that fails
        // silently a moment later.
        let model_ctx = self
            .model_ctx
            .try_read()
            .map_err(|_| "model context is busy".to_string())?
            .clone()
            .ok_or_else(|| "no model is configured".to_string())?;

        let config = self
            .config
            .try_read()
            .map_err(|_| "configuration is busy".to_string())?
            .clone();

        // Bound the whole tree, not just one caller's branch. A spawned run is
        // a caller in its own right with its own per-caller allowance, so
        // eight runs can each start eight; only a process-wide count stops
        // that from compounding.
        {
            let mut runs = running_sessions()
                .lock()
                .map_err(|_| "running-session registry is unavailable".to_string())?;
            runs.reap_finished();
            rustyclaw_core::tool_limits::check_background_session(runs.running_keys().len())
                .map_err(|e| e.to_string())?;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = RunContext {
            http: self.http.clone(),
            config,
            model_ctx,
            vault: self.vault.clone(),
            skill_mgr: self.skill_mgr.clone(),
            task_mgr: self.task_mgr.clone(),
            copilot_session: self.copilot_session.clone(),
        };

        let key = req.session_key.clone();
        let run_cancel = cancel.clone();
        info!(session = %key, agent = %req.agent_id, "Starting background sub-agent");

        let handle = self.runtime.spawn(async move {
            let run = run_session(&ctx, &req, &run_cancel);
            // Scoped to its own session key, so anything the run takes
            // ownership of — a backgrounded process, its own sub-agents —
            // belongs to it and not to the conversation that spawned it.
            let run = rustyclaw_core::tool_caller::with_caller(
                format!("spawn:{}", req.session_key),
                rustyclaw_core::downloads::with_origin(
                    rustyclaw_core::downloads::headless_origin(&req.agent_id),
                    run,
                ),
            );

            let outcome = match req.timeout_secs {
                Some(secs) => {
                    match tokio::time::timeout(std::time::Duration::from_secs(secs), run).await {
                        Ok(outcome) => outcome,
                        Err(_) => Err(anyhow::anyhow!(
                            "run timed out after {secs}s (runTimeoutSeconds)"
                        )),
                    }
                }
                None => run.await,
            };

            record_outcome(
                &req.session_key,
                outcome,
                run_cancel.load(Ordering::Relaxed),
            );
            if let Ok(mut runs) = running_sessions().lock() {
                runs.reap_finished();
            }
        });

        match running_sessions().lock() {
            Ok(mut runs) => runs.register(key, handle, cancel),
            Err(_) => {
                // Unstoppable is worse than not started: with no registry
                // entry `sessions_kill` has no handle to pull, so tear the
                // run down rather than leave it loose.
                handle.abort();
                return Err("running-session registry is unavailable".to_string());
            }
        }
        Ok(())
    }
}

/// Write the run's ending into its session record.
///
/// A cancelled run is recorded as Stopped rather than Error: the parent
/// polling it needs to tell "someone stopped this" from "this broke".
fn record_outcome(key: &str, outcome: anyhow::Result<String>, cancelled: bool) {
    let Ok(mut mgr) = session_manager().lock() else {
        return;
    };
    let Some(session) = mgr.get_mut(key) else {
        return;
    };
    match outcome {
        Ok(text) => {
            session.add_message("assistant", &text);
            session.complete();
        }
        Err(e) if cancelled => {
            session.add_message("system", &format!("stopped: {e:#}"));
            // Through `stop()` rather than assigning the status, so the record
            // gets an end time — `runtime_secs` measures to *now* while
            // `finished_ms` is unset, and a stopped run would keep reporting a
            // growing runtime as though it were still working.
            session.stop();
        }
        Err(e) => {
            session.add_message("assistant", &format!("error: {e:#}"));
            session.error();
        }
    }
}

/// Everything one run needs, owned so it can outlive the turn that started it.
struct RunContext {
    http: reqwest::Client,
    config: Config,
    model_ctx: Arc<rustyclaw_core::gateway::ModelContext>,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: SharedTaskManager,
    copilot_session: SharedCopilotSession,
}

/// The run itself: a headless agent turn, bounded and cancellable.
async fn run_session(
    ctx: &RunContext,
    req: &SpawnRequest,
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    let agent_id = req.agent_id.as_str();

    // Aim at the target agent's workspace and prompt, as a trigger fire does.
    let registry = ctx.config.agent_registry();
    let mut config = ctx.config.clone();
    config.workspace_dir = Some(config.workspace_dir_for(agent_id));
    if let Some(prompt) = registry.manifest(agent_id).and_then(|m| m.system_prompt) {
        config.system_prompt = Some(prompt);
    }
    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir).ok();

    let system_prompt = crate::system_prompt::build_system_prompt_full(
        &config,
        &ctx.task_mgr,
        None,
        &ctx.skill_mgr,
        crate::system_prompt::SessionContext {
            platform: Some("spawn"),
            session_key: Some(&req.session_key),
            origin: Some(rustyclaw_core::gateway::SessionOrigin::Trigger),
            ..Default::default()
        },
    )
    .await;

    // `ModelContext::api_key` holds the *raw* stored key. For Copilot that is
    // an OAuth token the chat endpoint rejects; it has to be traded for a
    // short-lived session token first, exactly as trigger, cron, and messenger
    // runs do. `call_with_tools` does not do this for us.
    let effective_key = crate::auth::resolve_bearer_token(
        &ctx.http,
        &ctx.model_ctx.provider,
        ctx.model_ctx.api_key.as_deref(),
        ctx.copilot_session.read().await.as_deref(),
    )
    .await
    .unwrap_or_else(|e| {
        warn!(error = %e, "Could not resolve bearer token for spawned run; using the stored key");
        ctx.model_ctx.api_key.clone()
    });

    let mut resolved = ProviderRequest {
        provider: ctx.model_ctx.provider.clone(),
        model: req
            .model
            .clone()
            .unwrap_or_else(|| ctx.model_ctx.model.clone()),
        base_url: ctx.model_ctx.base_url.clone(),
        api_key: effective_key,
        messages: vec![
            ChatMessage::text("system", &system_prompt),
            ChatMessage::text(
                "user",
                &format!(
                    "## Task\n{}\n\nYou are running in the background with no user \
                     attached. Never ask questions — decide, act, and report. Your \
                     final message is the result the agent that spawned you will read, \
                     so make it self-contained.",
                    req.task
                ),
            ),
        ],
        allowed_tools: None,
    };

    let mut last_text = String::new();

    for round in 0..MAX_TOOL_ROUNDS {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("stopped by request");
        }

        let model_resp = tokio::time::timeout(
            MODEL_CALL_TIMEOUT,
            providers::call_with_tools(&ctx.http, &resolved, None),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "model call timed out after {}s",
                MODEL_CALL_TIMEOUT.as_secs()
            )
        })??;

        if !model_resp.text.is_empty() {
            last_text = model_resp.text.clone();
        }
        if model_resp.tool_calls.is_empty() {
            debug!(session = %req.session_key, rounds = round + 1, "Spawned run finished");
            if last_text.trim().is_empty() {
                anyhow::bail!("the sub-agent finished without a final message");
            }
            // The final message is not appended here: `record_outcome` writes
            // every ending, success or not, and doing it in both places showed
            // a completed run's reply twice in its history.
            return Ok(last_text);
        }

        let mut tool_results: Vec<ToolCallResult> = Vec::new();
        for tc in &model_resp.tool_calls {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("stopped by request");
            }
            let (output, is_error) = execute_one_tool(ctx, &config, &workspace_dir, tc).await;
            if is_error {
                warn!(session = %req.session_key, tool = %tc.name, "Spawned run tool call failed");
            }
            // Mirror activity into the record as it happens, so a parent
            // polling `sessions_history` sees progress rather than silence
            // until the run ends.
            record_tool_call(&req.session_key, &tc.name, &output, is_error);
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
        "hit the round limit ({MAX_TOOL_ROUNDS}) before finishing. Last message: {}",
        if last_text.is_empty() {
            "(none)"
        } else {
            &last_text
        }
    )
}

/// Execute one tool call inside a spawned run.
async fn execute_one_tool(
    ctx: &RunContext,
    config: &Config,
    workspace_dir: &std::path::Path,
    tc: &rustyclaw_core::gateway::ParsedToolCall,
) -> (String, bool) {
    debug!(tool = %tc.name, "Spawned run tool call");

    // Nobody can answer an approval prompt in a detached run, so — like a
    // trigger fire — anything short of `Allow` is refused outright rather
    // than blocking until a timeout the parent never sees.
    let permission = config
        .tool_permissions
        .get(&tc.name)
        .cloned()
        .unwrap_or_default();
    if !matches!(permission, tools::ToolPermission::Allow) {
        let reason = match permission {
            tools::ToolPermission::Deny => "denied by user policy",
            tools::ToolPermission::Ask => {
                "requires user approval, which is unavailable in a background run"
            }
            tools::ToolPermission::SkillOnly(_) => "restricted to skill invocations",
            tools::ToolPermission::Allow => unreachable!(),
        };
        return (format!("Tool '{}' {}.", tc.name, reason), true);
    }

    // Tools that need a client frame to complete — an approval, a prompt, a
    // DOM query — have no counterparty here. Refusing beats waiting.
    if tools::is_user_prompt_tool(&tc.name) || tools::is_dom_query_tool(&tc.name) {
        return (
            format!(
                "Tool '{}' needs a connected client, which a background run does not have.",
                tc.name
            ),
            true,
        );
    }

    // Shared with every other loop in the gateway: rate limits, malformed-call
    // suppression, and secrets/skills routing all live behind this call.
    tool_executor::execute_tool_by_type(
        &tc.name,
        &tc.arguments,
        workspace_dir,
        &ctx.vault,
        &ctx.skill_mgr,
        None,
    )
    .await
}

/// Append a message to a run's session record, ignoring a poisoned lock: a
/// progress note is never worth failing the run over.
fn record_progress(key: &str, role: &str, content: &str) {
    if let Ok(mut mgr) = session_manager().lock() {
        if let Some(session) = mgr.get_mut(key) {
            session.add_message(role, content);
        }
    }
}

/// Record one tool call in the session history, trimmed: the record is a
/// progress view, and a full file read would crowd out everything else.
fn record_tool_call(key: &str, tool: &str, output: &str, is_error: bool) {
    const PREVIEW_CHARS: usize = 400;
    let preview: String = output.chars().take(PREVIEW_CHARS).collect();
    let ellipsis = if output.chars().count() > PREVIEW_CHARS {
        "…"
    } else {
        ""
    };
    let verb = if is_error { "failed" } else { "ran" };
    record_progress(key, "tool", &format!("{verb} {tool}: {preview}{ellipsis}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::gateway::ParsedToolCall;

    fn ctx_for(tmp: &std::path::Path) -> RunContext {
        RunContext {
            http: reqwest::Client::new(),
            config: Config {
                settings_dir: tmp.to_path_buf(),
                ..Config::default()
            },
            model_ctx: Arc::new(rustyclaw_core::gateway::ModelContext {
                provider: "openai".to_string(),
                model: "gpt-4o-mini".to_string(),
                base_url: "http://localhost".to_string(),
                api_key: Some(String::new()),
            }),
            vault: Arc::new(tokio::sync::Mutex::new(
                rustyclaw_core::secrets::SecretsManager::new(tmp),
            )),
            skill_mgr: Arc::new(tokio::sync::Mutex::new(
                rustyclaw_core::skills::SkillManager::new(tmp.to_path_buf()),
            )),
            task_mgr: Arc::new(rustyclaw_core::tasks::TaskManager::new()),
            copilot_session: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    fn call(name: &str, args: serde_json::Value) -> ParsedToolCall {
        ParsedToolCall {
            id: "call_1".to_string(),
            name: name.to_string(),
            arguments: args,
        }
    }

    #[tokio::test]
    async fn a_tool_needing_approval_is_refused_not_awaited() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx_for(tmp.path());
        let mut config = ctx.config.clone();
        config
            .tool_permissions
            .insert("read_file".to_string(), tools::ToolPermission::Ask);

        let (output, is_error) = execute_one_tool(
            &ctx,
            &config,
            tmp.path(),
            &call("read_file", serde_json::json!({"path": "x.txt"})),
        )
        .await;
        assert!(is_error);
        assert!(output.contains("requires user approval"), "{output}");
    }

    #[tokio::test]
    async fn a_denied_tool_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx_for(tmp.path());
        let mut config = ctx.config.clone();
        config
            .tool_permissions
            .insert("execute_command".to_string(), tools::ToolPermission::Deny);

        let (output, is_error) = execute_one_tool(
            &ctx,
            &config,
            tmp.path(),
            &call("execute_command", serde_json::json!({"command": "true"})),
        )
        .await;
        assert!(is_error);
        assert!(output.contains("denied by user policy"), "{output}");
    }

    #[tokio::test]
    async fn a_client_only_tool_is_refused_rather_than_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx_for(tmp.path());
        let config = ctx.config.clone();

        let (output, is_error) = execute_one_tool(
            &ctx,
            &config,
            tmp.path(),
            &call("ask_user", serde_json::json!({"question": "which one?"})),
        )
        .await;
        assert!(is_error);
        assert!(output.contains("needs a connected client"), "{output}");
    }

    #[tokio::test]
    async fn an_allowed_tool_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx_for(tmp.path());
        let config = ctx.config.clone();
        std::fs::write(tmp.path().join("note.txt"), "hello spawn").unwrap();

        let (output, is_error) = execute_one_tool(
            &ctx,
            &config,
            tmp.path(),
            &call("read_file", serde_json::json!({"path": "note.txt"})),
        )
        .await;
        assert!(!is_error, "unexpected error: {output}");
        assert!(output.contains("hello spawn"));
    }

    #[test]
    fn a_cancelled_run_gets_an_end_time() {
        // Without one, `runtime_secs` measures to now and the stopped run
        // shows an ever-growing runtime in session listings.
        let key = {
            let mut mgr = session_manager().lock().unwrap();
            mgr.spawn_subagent("main", "cancelled", None, None)
        };
        record_outcome(&key, Err(anyhow::anyhow!("stopped by request")), true);

        let mgr = session_manager().lock().unwrap();
        let session = mgr.get(&key).unwrap();
        assert_eq!(
            session.status,
            rustyclaw_core::sessions::SessionStatus::Stopped
        );
        assert!(
            session.finished_ms.is_some(),
            "stopped run needs an end time"
        );
    }

    #[test]
    fn a_completed_run_records_its_answer_once() {
        let key = {
            let mut mgr = session_manager().lock().unwrap();
            mgr.spawn_subagent("main", "completed", None, None)
        };
        record_outcome(&key, Ok("the answer".to_string()), false);

        let mgr = session_manager().lock().unwrap();
        let said: Vec<_> = mgr
            .get(&key)
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.content == "the answer")
            .collect();
        assert_eq!(said.len(), 1, "the final message must not be duplicated");
    }

    #[test]
    fn a_long_tool_output_is_trimmed_in_the_record() {
        let key = {
            let mut mgr = session_manager().lock().unwrap();
            mgr.spawn_subagent("main", "trim test", None, None)
        };
        record_tool_call(&key, "read_file", &"x".repeat(5_000), false);

        let mgr = session_manager().lock().unwrap();
        let last = mgr.get(&key).unwrap().messages.last().unwrap();
        assert!(last.content.ends_with('…'));
        assert!(last.content.chars().count() < 500);
    }
}
