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
        Payload::AgentTurn { message, model, .. } => {
            run_agent_turn(
                deps,
                &config,
                job,
                &job_label,
                &thread_mgr,
                &threads_path,
                &workspace_dir,
                message,
                model.as_deref(),
            )
            .await
        }
    }
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
    model_override: Option<&str>,
) -> anyhow::Result<()> {
    let Some(model_ctx) = deps.model_ctx.read().await.clone() else {
        anyhow::bail!("No model configured — cannot run scheduled agent turn");
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
            deps.copilot.read().await.clone().as_deref(),
        )
        .await
        .ok()
        .flatten();

        // The job's model rides the configured provider, exactly as a
        // subagent's model override does.
        let mut resolved = ProviderRequest {
            provider: model_ctx.provider.clone(),
            model: model_override
                .map(str::to_string)
                .unwrap_or_else(|| model_ctx.model.clone()),
            base_url: model_ctx.base_url.clone(),
            api_key: effective_key,
            messages,
            allowed_tools: None,
        };

        let http = rustyclaw_core::providers::http_client();
        let session_key = format!("cron:{}", job.job_id);
        let mut final_response = String::new();

        for _round in 0..MAX_TOOL_ROUNDS {
            let model_resp = providers::call_with_tools(&http, &resolved, None).await?;

            if !model_resp.text.is_empty() {
                final_response.push_str(&model_resp.text);
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
                    // user conversation.
                    match rustyclaw_core::tool_caller::with_caller(
                        format!("cron:{}", job.job_id),
                        tools::execute_tool(&tc.name, &tc.arguments, workspace_dir),
                    )
                    .await
                    {
                        Ok(text) => (text, false),
                        Err(err) => (err.to_string(), true),
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

        anyhow::Ok(final_response)
    }
    .await;

    // Close the turn on every path; only success lands a response message.
    let mut tm = thread_mgr.lock().await;
    match &result {
        Ok(response) => {
            if !response.is_empty() {
                tm.add_message(thread, MessageRole::Assistant, response.clone());
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

    result.map(|response| {
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
