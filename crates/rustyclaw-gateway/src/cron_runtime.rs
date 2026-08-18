//! The cron scheduler: fires stored jobs at their appointed times.
//!
//! Every part of cron existed before this file except the part that acts —
//! the store, the schedule types, the agent tool, the wire frames, the
//! panels. This is the missing middle: a loop that keeps `next_run_ms`
//! honest, sleeps until the earliest of them, and runs each due job as a
//! headless agent turn in the thread the job names.
//!
//! A wake lands somewhere a person can see it. The turn runs against the
//! shared thread manager (`threads::manager_for`), so the prompt and the
//! response are appended to the target thread, turn markers make the
//! sidebar show it streaming, and every connected client watches it happen
//! live — the same machinery a typed message uses, minus the client.

use std::sync::Arc;
use std::time::Duration;

use rustyclaw_core::config::Config;
use rustyclaw_core::cron::{self, CronJob, Payload, RunEntry, RunStatus};
use rustyclaw_core::gateway::{ChatMessage, ProviderRequest, ToolCallResult};
use rustyclaw_core::threads::{MessageRole, ThreadId};
use rustyclaw_core::tools;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedSkillManager,
    SharedTaskManager, SharedVault, providers, secrets_handler, skills_handler,
};

/// Maximum tool loop rounds for a scheduled turn (matches the trigger and
/// messenger handlers).
const MAX_TOOL_ROUNDS: usize = 25;

/// Upper bound on one model call inside a scheduled turn, matching the
/// spawned-run and subagent runners.
///
/// This is a *total* deadline, not a silence detector. The shared provider
/// client's read timeout only fires between bytes, so a trickling or stalled
/// connection can otherwise hold the turn open for hours while the scheduler
/// sits in its sequential loop behind it (observed: a 5.6 h hang, issue #447).
const MODEL_CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Upper bound on one scheduler sleep. The loop wakes on edits anyway;
/// this bounds how long a missed notify (or clock weirdness) can delay a
/// fire.
const MAX_SLEEP: Duration = Duration::from_secs(3600);

/// Everything a scheduled turn needs from the gateway. Bundled so the
/// scheduler entry point stays one argument wide as the gateway grows.
#[derive(Clone)]
pub struct CronDeps {
    pub config: SharedConfig,
    pub model_ctx: SharedModelCtx,
    pub copilot: SharedCopilotSession,
    pub vault: SharedVault,
    pub skill_mgr: SharedSkillManager,
    pub task_mgr: SharedTaskManager,
    pub model_registry: SharedModelRegistry,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Run the scheduler until cancelled.
///
/// Each pass: settle every job's `next_run_ms`, take what is due, fire it,
/// then sleep until the earliest remaining fire — or until an edit
/// notifies, so a new "in one minute" job never waits out an hour-long
/// sleep armed before it existed.
pub async fn run_cron_scheduler(
    deps: CronDeps,
    notify: Arc<tokio::sync::Notify>,
    cancel: CancellationToken,
) {
    info!("Cron scheduler started");
    loop {
        let settings_dir = deps.config.read().await.settings_dir.clone();
        let now = now_ms();
        let planned = tokio::task::spawn_blocking(move || {
            cron::with_store(&settings_dir, |store| {
                let due = store.take_due_jobs(now)?;
                let earliest = store.ensure_next_runs(now)?;
                Ok((due, earliest))
            })
        })
        .await;

        let (due, earliest) = match planned {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                // The store is unreadable; keep the loop alive and retry on
                // the fallback cadence rather than dying quietly.
                error!(error = %e, "Cron store unavailable; scheduler retrying later");
                (Vec::new(), None)
            }
            Err(e) => {
                error!(error = %e, "Cron store task failed");
                (Vec::new(), None)
            }
        };

        for job in due {
            run_due_job(&deps, job).await;
        }

        let sleep = earliest
            .map(|t| Duration::from_millis(t.saturating_sub(now_ms())))
            .unwrap_or(MAX_SLEEP)
            .min(MAX_SLEEP);
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = notify.notified() => {}
            _ = tokio::time::sleep(sleep) => {}
        }
    }
    info!("Cron scheduler stopped");
}

/// Fire one job, recording the run either way.
async fn run_due_job(deps: &CronDeps, job: CronJob) {
    let started = now_ms();
    let run_id = format!("run-{started:x}");
    let settings_dir = deps.config.read().await.settings_dir.clone();

    let record = |status: RunStatus, error: Option<String>, finished: Option<u64>| RunEntry {
        job_id: job.job_id.clone(),
        run_id: run_id.clone(),
        started_ms: started,
        finished_ms: finished,
        status,
        error,
    };

    let cron_dir = cron::central_cron_dir(&settings_dir);
    if let Ok(store) = cron::CronStore::new(&cron_dir) {
        store
            .record_run(&record(RunStatus::Running, None, None))
            .ok();
    }

    info!(
        job = %job.job_id,
        name = job.name.as_deref().unwrap_or("unnamed"),
        "Cron job firing"
    );
    let outcome = rustyclaw_core::downloads::with_origin(
        rustyclaw_core::downloads::headless_origin(
            job.agent_id
                .as_deref()
                .unwrap_or(rustyclaw_core::agents::MAIN_AGENT_ID),
        ),
        run_job_inner(deps, &job),
    )
    .await;

    let (status, error) = match outcome {
        Ok(()) => {
            info!(job = %job.job_id, "Cron job completed");
            (RunStatus::Ok, None)
        }
        Err(e) => {
            error!(job = %job.job_id, error = %e, "Cron job failed");
            (RunStatus::Error, Some(format!("{e:#}")))
        }
    };
    if let Ok(store) = cron::CronStore::new(&cron_dir) {
        store
            .record_run(&record(status, error, Some(now_ms())))
            .ok();
    }
}

/// The job itself: resolve agent and thread, land the payload.
async fn run_job_inner(deps: &CronDeps, job: &CronJob) -> anyhow::Result<()> {
    let base_config = deps.config.read().await.clone();

    // Aim at the target agent, exactly as a trigger fire does.
    let agent_id = job
        .agent_id
        .clone()
        .unwrap_or_else(|| rustyclaw_core::agents::MAIN_AGENT_ID.to_string());
    let registry = base_config.agent_registry();
    if registry.get(&agent_id).is_none() {
        anyhow::bail!("Cron job targets unknown agent '{agent_id}'");
    }
    let mut config = base_config.clone();
    config.workspace_dir = Some(config.workspace_dir_for(&agent_id));
    if let Some(prompt) = registry.manifest(&agent_id).and_then(|m| m.system_prompt) {
        config.system_prompt = Some(prompt);
    }
    let workspace_dir = config.workspace_dir();
    std::fs::create_dir_all(&workspace_dir).ok();
    rustyclaw_core::runtime_ctx::set_active_agent(&agent_id);

    // The shared manager for this agent's store: connected clients hold the
    // same one, so everything appended here reaches their sidebars live.
    let threads_path = config.sessions_dir_for(&agent_id).join("threads.json");
    let thread_mgr = rustyclaw_core::threads::manager_for(&threads_path);

    let job_label = job.name.clone().unwrap_or_else(|| job.job_id.clone());

    match &job.payload {
        // A system event is a note, not a turn: it lands in the thread and
        // that is the whole job.
        Payload::SystemEvent { text } => {
            let mut tm = thread_mgr.lock().await;
            let thread = resolve_target_thread(&mut tm, job, &job_label)?;
            tm.add_message(thread, MessageRole::System, text.clone());
            crate::helpers::persist_threads(&mut tm, &threads_path);
            Ok(())
        }
        Payload::AgentTurn {
            message,
            provider,
            model,
            ..
        } => {
            run_agent_turn(
                deps,
                &config,
                job,
                &job_label,
                &thread_mgr,
                &threads_path,
                &workspace_dir,
                message,
                provider.as_deref(),
                model.as_deref(),
            )
            .await
        }
    }
}

/// Build a [`ModelContext`] for a provider the gateway is not currently on.
///
/// Resolved the same way a `ModelSwitch` resolves it — base URL from the
/// provider table, key from the vault with an environment fallback — so a
/// pinned job and an interactive switch reach the same endpoint with the same
/// credential.
///
/// Fails rather than falling back to the active provider: quietly running a
/// job against the wrong vendor is what this whole field exists to prevent,
/// and a job that stops with "no API key" is one the user can fix.
async fn provider_context(
    deps: &CronDeps,
    provider: &str,
    model_override: Option<&str>,
) -> anyhow::Result<rustyclaw_core::gateway::ModelContext> {
    let model = model_override
        .map(str::to_string)
        .or_else(|| {
            rustyclaw_core::providers::models_for_provider(provider)
                .first()
                .map(|m| (*m).to_string())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Job pins provider '{provider}' but names no model, and it has none known"
            )
        })?;

    let api_key = match rustyclaw_core::providers::secret_key_for_provider(provider) {
        Some(name) => {
            let mut vault = deps.vault.lock().await;
            vault
                .get_secret(name, true)
                .ok()
                .flatten()
                .or_else(|| std::env::var(name).ok())
        }
        None => None,
    };

    let configured = deps.config.read().await.model.clone();
    let base_url = {
        resolve_base_url(
            provider,
            configured.as_ref().map(|m| m.provider.as_str()),
            configured.as_ref().and_then(|m| m.base_url.as_deref()),
        )?
    };

    Ok(rustyclaw_core::gateway::ModelContext {
        provider: provider.to_string(),
        model,
        base_url,
        api_key,
        reasoning_effort: configured.as_ref().and_then(|m| m.reasoning_effort.clone()),
        max_tokens: configured.as_ref().and_then(|m| m.max_tokens),
        temperature: configured.as_ref().and_then(|m| m.temperature),
        top_p: configured.as_ref().and_then(|m| m.top_p),
    })
}

/// The endpoint a job pinned to `provider` should be sent to.
///
/// `custom` and `copilot-proxy` ship no built-in address on purpose: the
/// operator supplies it, and it lives in the config. Defaulting to the empty
/// string sent them to `/`, so every fire failed with a connection error that
/// named nothing.
///
/// The configured URL is only usable when the config is *on* the same
/// provider. This runs precisely when a job pins something other than the
/// gateway's active provider, so borrowing the URL unconditionally would aim
/// a `custom` job at whatever endpoint the gateway happens to use — worse
/// than not running, because it would run somewhere wrong. When there is no
/// endpoint to be had, say so, the way the missing-model case does.
fn resolve_base_url(
    provider: &str,
    configured_provider: Option<&str>,
    configured_url: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(known) = rustyclaw_core::providers::base_url_for_provider(provider) {
        return Ok(known.to_string());
    }
    configured_url
        .filter(|url| !url.trim().is_empty())
        .filter(|_| configured_provider == Some(provider))
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Job pins provider '{provider}', which has no built-in endpoint and none \
                 configured for it. Set the endpoint for '{provider}' in the config, or pin \
                 the job to a provider that has one."
            )
        })
}

/// The thread a job lands in. A named thread must exist — falling back
/// silently would put a wake where nobody set it. An unnamed target means
/// "wherever the agent is": the persisted foreground, any electable
/// thread, or a fresh one when the store is empty.
fn resolve_target_thread(
    tm: &mut rustyclaw_core::threads::ThreadManager,
    job: &CronJob,
    job_label: &str,
) -> anyhow::Result<ThreadId> {
    if let Some(id) = job.thread_id {
        let id = ThreadId(id);
        if tm.get(id).is_none() {
            anyhow::bail!(
                "Cron job '{job_label}' targets thread {} which no longer exists",
                id.0
            );
        }
        return Ok(id);
    }
    if let Some(id) = tm.foreground_id().filter(|id| tm.get(*id).is_some()) {
        return Ok(id);
    }
    if let Some(id) = tm.elect_foreground() {
        return Ok(id);
    }
    Ok(tm.create_chat(format!("Scheduled: {job_label}")))
}

/// A full headless turn in the target thread: prompt in, model + tools,
/// response in, turn markers around it all.
#[allow(clippy::too_many_arguments)]
async fn run_agent_turn(
    deps: &CronDeps,
    config: &Config,
    job: &CronJob,
    job_label: &str,
    thread_mgr: &crate::SharedThreadMgr,
    threads_path: &std::path::Path,
    workspace_dir: &std::path::Path,
    message: &str,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> anyhow::Result<()> {
    let Some(gateway_ctx) = deps.model_ctx.read().await.clone() else {
        anyhow::bail!("No model configured — cannot run scheduled agent turn");
    };

    // A job pinned to a provider runs against that provider, not whichever
    // one the gateway happens to be on now. Without this the model override
    // was only half a choice: switch the gateway from Anthropic to OpenAI and
    // every job pinned to a Claude model started being sent to OpenAI's
    // endpoint under a Claude model name.
    let (model_ctx, copilot) = match provider_override {
        Some(pinned) if pinned != gateway_ctx.provider => {
            let ctx = provider_context(deps, pinned, model_override).await?;
            // Copilot trades an OAuth token for a short-lived session token,
            // and the gateway's session belongs to the *active* provider. A
            // job on a different one needs its own.
            let session =
                crate::session::init_copilot_session(pinned, ctx.api_key.as_deref(), &deps.vault)
                    .await;
            (std::sync::Arc::new(ctx), session)
        }
        _ => (gateway_ctx, deps.copilot.read().await.clone()),
    };

    let prompt = format!("[Scheduled wake: {job_label}] {message}");

    // Open the turn and collect context in one lock scope. The prompt goes
    // in first so the thread reads true even if the model call fails.
    let (thread, history, compact_summary) = {
        let mut tm = thread_mgr.lock().await;
        let thread = resolve_target_thread(&mut tm, job, job_label)?;
        tm.begin_turn(thread);
        tm.add_message(thread, MessageRole::User, prompt.clone());
        let t = tm.get(thread).expect("thread resolved above");
        let history: Vec<rustyclaw_core::threads::ThreadMessage> =
            t.context_messages().cloned().collect();
        let summary = t.compact_summary.clone();
        crate::helpers::persist_threads(&mut tm, threads_path);
        (thread, history, summary)
    };

    // From here every exit must close the turn — a wake that leaves its
    // thread marked streaming forever is a bug this codebase has paid for
    // before.
    let result = async {
        let system_prompt = crate::system_prompt::build_system_prompt_full(
            config,
            &deps.task_mgr,
            None,
            &deps.skill_mgr,
            crate::system_prompt::SessionContext {
                platform: Some("cron"),
                origin: Some(rustyclaw_core::gateway::SessionOrigin::Trigger),
                ..Default::default()
            },
        )
        .await;

        let mut messages =
            providers::thread_history_to_chat_messages(&model_ctx.provider, &history);
        if let Some(summary) = &compact_summary {
            messages.insert(
                0,
                ChatMessage::text(
                    "system",
                    &format!("# Previous conversation summary\n\n{summary}"),
                ),
            );
        }
        messages.insert(0, ChatMessage::text("system", &system_prompt));

        let effective_key = crate::auth::resolve_bearer_token(
            &rustyclaw_core::providers::http_client(),
            &model_ctx.provider,
            model_ctx.api_key.as_deref(),
            copilot.as_deref(),
        )
        .await
        .ok()
        .flatten();

        let mut resolved = ProviderRequest {
            provider: model_ctx.provider.clone(),
            model: model_override
                .map(str::to_string)
                .unwrap_or_else(|| model_ctx.model.clone()),
            base_url: model_ctx.base_url.clone(),
            api_key: effective_key,
            messages,
            allowed_tools: None,
            reasoning_effort: model_ctx.reasoning_effort.clone(),
            max_tokens: model_ctx.max_tokens,
            temperature: model_ctx.temperature,
            top_p: model_ctx.top_p,
        };

        let http = rustyclaw_core::providers::http_client();
        let session_key = format!("cron:{}", job.job_id);
        let mut final_response = String::new();
        // The final turn's reasoning (thinking-mode providers require it to
        // be echoed back if this thread is ever continued with a chat turn).
        let mut final_reasoning = String::new();

        for _round in 0..MAX_TOOL_ROUNDS {
            let model_resp = tokio::time::timeout(
                MODEL_CALL_TIMEOUT,
                providers::call_with_tools(&http, &resolved, None),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "model call timed out after {}s",
                    MODEL_CALL_TIMEOUT.as_secs()
                )
            })??;

            if !model_resp.text.is_empty() {
                final_response.push_str(&model_resp.text);
            }
            if !model_resp.reasoning.is_empty() {
                final_reasoning.push_str(&model_resp.reasoning);
            }
            if model_resp.tool_calls.is_empty() {
                break;
            }

            let mut tool_results: Vec<ToolCallResult> = Vec::new();
            for tc in &model_resp.tool_calls {
                debug!(job = %job.job_id, tool = %tc.name, "Scheduled turn tool call");

                // Headless permission clamp, same as trigger turns: nobody
                // is present to answer an Ask.
                let permission = config
                    .tool_permissions
                    .get(&tc.name)
                    .cloned()
                    .unwrap_or_default();
                if !matches!(permission, rustyclaw_core::tools::ToolPermission::Allow) {
                    let reason = match permission {
                        rustyclaw_core::tools::ToolPermission::Deny => {
                            "denied by user policy".to_string()
                        }
                        rustyclaw_core::tools::ToolPermission::Ask => {
                            "requires user approval, which is unavailable in a scheduled run"
                                .to_string()
                        }
                        rustyclaw_core::tools::ToolPermission::SkillOnly(_) => {
                            "restricted to skill invocations".to_string()
                        }
                        rustyclaw_core::tools::ToolPermission::Allow => unreachable!(),
                    };
                    tool_results.push(ToolCallResult {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        output: format!("Tool '{}' {}.", tc.name, reason),
                        is_error: true,
                    });
                    continue;
                }

                let (output, is_error) = if tools::is_secrets_tool(&tc.name) {
                    match secrets_handler::execute_secrets_tool(
                        &tc.name,
                        &tc.arguments,
                        &deps.vault,
                    )
                    .await
                    {
                        Ok(text) => (text, false),
                        Err(err) => (err.to_string(), true),
                    }
                } else if tools::is_skill_tool(&tc.name) {
                    match skills_handler::execute_skill_tool(
                        &tc.name,
                        &tc.arguments,
                        &deps.skill_mgr,
                    )
                    .await
                    {
                        Ok(text) => (text, false),
                        Err(err) => (err.to_string(), true),
                    }
                } else if crate::task_handler::is_task_tool(&tc.name) {
                    match crate::task_handler::execute_task_tool(
                        &tc.name,
                        &tc.arguments,
                        &deps.task_mgr,
                        Some(&session_key),
                    )
                    .await
                    {
                        Ok(text) => (text, false),
                        Err(err) => (err.to_string(), true),
                    }
                } else if crate::model_handler::is_model_tool(&tc.name) {
                    match crate::model_handler::execute_model_tool(
                        &tc.name,
                        &tc.arguments,
                        &deps.model_registry,
                    )
                    .await
                    {
                        Ok(text) => (text, false),
                        Err(err) => (err.to_string(), true),
                    }
                } else if crate::mcp_handler::is_mcp_tool(&tc.name)
                    || crate::canvas_handler::is_canvas_tool(&tc.name)
                {
                    // Same limitation as the other headless paths.
                    (
                        format!("Tool '{}' is not available in scheduled turns", tc.name),
                        true,
                    )
                } else {
                    // Scoped to the job, so one schedule's backgrounded
                    // process is not reachable from another job or from a
                    // user conversation, and its tool budget is its own.
                    let caller = format!("cron:{}", job.job_id);
                    match rustyclaw_core::tool_limits::check_rate(Some(&caller), &tc.name) {
                        Err(err) => (err.to_string(), true),
                        Ok(()) => match rustyclaw_core::tool_caller::with_caller(
                            caller,
                            tools::execute_tool(&tc.name, &tc.arguments, workspace_dir),
                        )
                        .await
                        {
                            Ok(text) => (text, false),
                            Err(err) => (err.to_string(), true),
                        },
                    }
                };
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

        anyhow::Ok((final_response, final_reasoning))
    }
    .await;

    // Close the turn on every path; only success lands a response message.
    let mut tm = thread_mgr.lock().await;
    match &result {
        Ok((response, reasoning_text)) => {
            if !response.is_empty() {
                let reasoning = if reasoning_text.is_empty() {
                    None
                } else {
                    Some(reasoning_text.clone())
                };
                tm.add_message_with_reasoning(
                    thread,
                    MessageRole::Assistant,
                    response.clone(),
                    reasoning,
                );
            }
            tm.end_turn(thread, true);
        }
        Err(e) => {
            tm.add_message(
                thread,
                MessageRole::System,
                format!("[Scheduled wake '{job_label}' failed: {e:#}]"),
            );
            tm.end_turn(thread, false);
        }
    }
    crate::helpers::persist_threads(&mut tm, threads_path);
    drop(tm);

    result.map(|(response, _)| {
        warn_if_silent(&response, job_label);
    })
}

/// A wake that produced nothing is worth a log line — the user scheduled
/// it to hear something.
fn warn_if_silent(response: &str, job_label: &str) {
    if response.trim().is_empty() {
        warn!(job = %job_label, "Scheduled turn produced no response text");
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::resolve_base_url;

    /// A provider with a built-in endpoint uses it, whatever the config is
    /// currently pointed at.
    #[test]
    fn a_known_provider_brings_its_own_endpoint() {
        let url = resolve_base_url("anthropic", Some("openai"), Some("https://elsewhere"))
            .expect("anthropic has a built-in endpoint");
        assert!(url.starts_with("http"));
        assert!(!url.contains("elsewhere"), "the config should not win here");
    }

    /// The case that was broken: the operator's endpoint is in the config,
    /// and the config is on that provider.
    #[test]
    fn a_configured_endpoint_is_used_for_a_provider_that_has_none() {
        let url = resolve_base_url("custom", Some("custom"), Some("https://my-llm.example/v1"))
            .expect("the configured endpoint should be used");
        assert_eq!(url, "https://my-llm.example/v1");
    }

    /// The trap in fixing it. This runs only when the job pins something
    /// other than the active provider, so the configured URL usually belongs
    /// to a *different* provider — using it would send a `custom` job to
    /// whatever endpoint the gateway is on. Failing beats running somewhere
    /// wrong.
    #[test]
    fn another_providers_endpoint_is_not_borrowed() {
        let err = resolve_base_url(
            "custom",
            Some("anthropic"),
            Some("https://api.anthropic.com"),
        )
        .expect_err("must not reuse another provider's endpoint")
        .to_string();
        assert!(err.contains("custom"), "the error should name the provider");
        assert!(
            !err.contains("anthropic.com"),
            "and must not suggest the wrong endpoint was considered usable"
        );
    }

    /// Nothing configured, and nothing built in: say why rather than sending
    /// the request to `/`.
    #[test]
    fn no_endpoint_anywhere_is_an_error_that_names_the_cause() {
        for configured in [None, Some("")] {
            let err = resolve_base_url("copilot-proxy", Some("copilot-proxy"), configured)
                .expect_err("no endpoint means no run")
                .to_string();
            assert!(err.contains("copilot-proxy"));
            assert!(
                err.contains("configured"),
                "the message should point at the fix, got: {err}"
            );
        }
    }
}
