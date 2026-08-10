//! Cron tool: scheduled job management.

use crate::tools::error::{ToolError, ToolResult};
use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument, warn};

/// Cron job management.
#[instrument(skip(args, workspace_dir), fields(action))]
pub fn exec_cron(args: &Value, workspace_dir: &Path) -> ToolResult {
    use crate::cron::*;

    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing cron tool");

    // One central store for the whole gateway. This tool used to open
    // `.cron` under the per-agent workspace while the client panel opened
    // the gateway's — two stores, each seeing half the jobs. Jobs an agent
    // created in the old location are adopted on first touch.
    let Some(settings_dir) = crate::runtime_ctx::get_settings_dir() else {
        return Err("Cron is unavailable: no gateway settings directory in this context".into());
    };
    adopt_legacy_store(&settings_dir, &workspace_dir.join(".cron"));
    let cron_dir = central_cron_dir(&settings_dir);

    // Through `with_store`, which holds the store lock across the whole
    // read-modify-write. Every save rewrites the entire jobs file, so an
    // interleave with the client panel or the scheduler drops one of the two
    // changes outright. The lock was written for exactly these three writers;
    // this one was opening the store directly and taking part in the race it
    // was meant to prevent.
    with_store(&settings_dir, |store| {
        Ok(run_action(action, args, store, &cron_dir, &settings_dir))
    })?
}

/// One cron action against an already-locked store.
fn run_action(
    action: &str,
    args: &Value,
    store: &mut crate::cron::CronStore,
    cron_dir: &Path,
    settings_dir: &Path,
) -> ToolResult {
    use crate::cron::*;

    match action {
        "status" => {
            let jobs = store.list(false);
            let enabled_count = jobs.len();
            let all_count = store.list(true).len();
            debug!(enabled = enabled_count, total = all_count, "Cron status");
            Ok(format!(
                "Cron scheduler status:\n- Enabled jobs: {}\n- Total jobs: {}\n- Store: {:?}",
                enabled_count, all_count, cron_dir
            ))
        }

        "list" => {
            let include_disabled = args
                .get("includeDisabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let jobs = store.list(include_disabled);
            debug!(count = jobs.len(), include_disabled, "Listing cron jobs");
            if jobs.is_empty() {
                return Ok("No cron jobs configured.".to_string());
            }

            let mut output = String::from("Cron jobs:\n\n");
            for job in jobs {
                let status = if job.enabled { "✓" } else { "○" };
                let name = job.name.as_deref().unwrap_or("(unnamed)");
                let schedule = match &job.schedule {
                    Schedule::At { at } => format!("at {}", at),
                    Schedule::Every { every_ms, .. } => format!("every {}ms", every_ms),
                    Schedule::Cron { expr, tz } => {
                        format!(
                            "cron '{}'{}",
                            expr,
                            tz.as_ref().map(|t| format!(" ({})", t)).unwrap_or_default()
                        )
                    }
                };
                // Where it runs and on what, not just when. Without these a
                // caller has to `get` every job to answer "which of these is
                // pinned to a thread" or "which override the model".
                let target = match job.thread_id {
                    Some(id) => format!("thread #{id}"),
                    None => "foreground".to_string(),
                };
                let model = match &job.payload {
                    Payload::AgentTurn {
                        model: Some(model), ..
                    } => format!(", model {model}"),
                    _ => String::new(),
                };
                output.push_str(&format!(
                    "{} {} [{}] — {} → {}{}\n",
                    status, job.job_id, name, schedule, target, model
                ));
            }
            output.push_str("\nUse action 'get' with a jobId for a job's full definition.\n");
            Ok(output)
        }

        "get" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for get")?;

            let job = store
                .get(job_id)
                .ok_or_else(|| format!("Job not found: {job_id}"))?;

            // Serialized whole rather than summarized: this is the action a
            // caller reaches for before editing, and a field omitted here is
            // one it cannot know to preserve.
            serde_json::to_string_pretty(job)
                .map_err(|e| ToolError::context("Failed to serialize job", e))
        }

        "add" => {
            let job_obj = args.get("job").ok_or("Missing required parameter: job")?;

            // Accept the documented minimal definition, not the full stored
            // `CronJob`: `jobId`/`createdMs` are server-owned and cannot be
            // known in advance, so demanding them rejected every valid call.
            let mut req: CronAddRequest = serde_json::from_value(job_obj.clone())
                .map_err(|e| ToolError::context("Invalid job definition", e))?;

            // Omit `threadId` → pin to the creating thread. An interactive
            // turn's caller identity is `thread:<id>`, so a job scheduled
            // from a conversation lands back in that conversation instead of
            // whatever thread happens to be foreground when it fires — the
            // failure mode in issue #444. Jobs created from a sub-agent, a
            // messenger conversation, or another cron job carry a different
            // caller and stay unpinned (foreground at fire time).
            //
            // Thread ids are per-agent: pinning only happens when the job
            // targets the caller's own agent, because thread #3 of agent A
            // is not thread #3 of agent B — a pin across agents would route
            // the wake into a different conversation.
            if req.thread_id.is_none() {
                let job_agent = req
                    .agent_id
                    .clone()
                    .unwrap_or_else(|| crate::agents::MAIN_AGENT_ID.to_string());
                let caller_agent = crate::runtime_ctx::get_active_agent();
                let same_agent = match &caller_agent {
                    Some(a) => *a == job_agent,
                    // No active-agent context (tests, embedded use): assume
                    // the main agent, which is the job's default target.
                    None => job_agent == crate::agents::MAIN_AGENT_ID,
                };
                if same_agent {
                    if let Some(tid) = current_thread_id() {
                        req.thread_id = Some(tid);
                    }
                }
            }

            // A pinned thread must exist for the job's agent, now — not just
            // at fire time when the failure is a misplaced activation.
            let agent_id = req
                .agent_id
                .clone()
                .unwrap_or_else(|| crate::agents::MAIN_AGENT_ID.to_string());
            if let Some(tid) = req.thread_id {
                validate_thread_id(settings_dir, &agent_id, tid)?;
            }

            let id = store.add(req.into_job())?;
            crate::runtime_ctx::notify_cron_changed();
            debug!(job_id = %id, "Created cron job");
            Ok(format!("Created job: {}", id))
        }

        "update" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for update")?;

            let patch_obj = args.get("patch").ok_or("Missing patch for update")?;

            let patch: CronJobPatch = serde_json::from_value(patch_obj.clone())
                .map_err(|e| ToolError::context("Invalid patch", e))?;

            // Same existence check as `add`: a patch that pins the job to a
            // thread must pin it to a thread that exists for the job's agent.
            if !patch.clear_thread {
                if let Some(tid) = patch.thread_id {
                    let agent_id = patch
                        .agent_id
                        .clone()
                        .or_else(|| store.get(job_id).and_then(|j| j.agent_id.clone()))
                        .unwrap_or_else(|| crate::agents::MAIN_AGENT_ID.to_string());
                    validate_thread_id(settings_dir, &agent_id, tid)?;
                }
            }

            store.update(job_id, patch)?;
            crate::runtime_ctx::notify_cron_changed();
            debug!(job_id, "Updated cron job");
            Ok(format!("Updated job: {}", job_id))
        }

        "remove" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for remove")?;

            store.remove(job_id)?;
            crate::runtime_ctx::notify_cron_changed();
            debug!(job_id, "Removed cron job");
            Ok(format!("Removed job: {}", job_id))
        }

        "run" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for run")?;

            let name = store
                .get(job_id)
                .ok_or_else(|| format!("Job not found: {}", job_id))?
                .name
                .clone();
            store.request_run_now(job_id)?;
            crate::runtime_ctx::notify_cron_changed();
            debug!(job_id, "Manual run requested");
            Ok(format!(
                "Requested an immediate run of job '{}' ({}). The scheduler fires it momentarily.",
                name.as_deref().unwrap_or("unnamed"),
                job_id
            ))
        }

        "runs" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for runs")?;

            // Bounded, but by the caller rather than by a constant: a job
            // that fires every few minutes buries the failure being chased
            // ten entries deep.
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .clamp(1, 200) as usize;
            let runs = store.get_runs(job_id, limit)?;
            debug!(job_id, run_count = runs.len(), "Fetching run history");
            if runs.is_empty() {
                return Ok(format!("No run history for job: {}", job_id));
            }

            let mut output = format!("Run history for {}:\n\n", job_id);
            for run in runs {
                let status = match run.status {
                    RunStatus::Ok => "✓",
                    RunStatus::Error => "✗",
                    RunStatus::Running => "⟳",
                    RunStatus::Timeout => "⏱",
                    RunStatus::Skipped => "○",
                };
                output.push_str(&format!("{} {} — {:?}\n", status, run.run_id, run.status));
            }
            Ok(output)
        }

        _ => {
            warn!(action, "Unknown cron action");
            Err(format!(
                "Unknown action: {}. Valid: status, list, get, add, update, remove, run, runs",
                action
            )
            .into())
        }
    }
}

/// The thread id of an interactive turn, when the caller is one.
///
/// The gateway scopes interactive thread turns with caller identity
/// `thread:<id>` (see dispatch.rs); sub-agents, messenger conversations,
/// and cron-fired turns carry other identities and return `None` here.
fn current_thread_id() -> Option<u64> {
    crate::tool_caller::current()
        .as_deref()
        .and_then(|id| id.strip_prefix("thread:"))
        .and_then(|id| id.parse().ok())
}

/// Reject a `threadId` that names no thread in the target agent's store,
/// with a helpful list of the ids that do exist.
///
/// Thread ids are per-agent: `<settings_dir>/agents/<agent>/sessions/
/// threads.json`. A job created with `threadId` targeting another agent's
/// thread would fire into the wrong conversation — or none at all — so the
/// tool checks at add/update time rather than letting the scheduler discover
/// it later.
fn validate_thread_id(settings_dir: &Path, agent_id: &str, thread_id: u64) -> Result<(), String> {
    let threads_json = crate::config::Config::agent_threads_path_from(settings_dir, agent_id);
    let known = crate::threads::ThreadStore::peek(&threads_json).unwrap_or_default();

    if known.iter().any(|t| t.id == thread_id) {
        return Ok(());
    }

    let mut msg = format!(
        "threadId {} does not exist for agent '{}'. ",
        thread_id, agent_id
    );
    if known.is_empty() {
        msg.push_str("That agent has no threads yet.");
    } else {
        let list = known
            .iter()
            .map(|t| format!("#{} ({})", t.id, t.label))
            .collect::<Vec<_>>()
            .join(", ");
        msg.push_str(&format!("Existing threads: {}.", list));
    }
    msg.push_str(
        " Omit threadId to target the current thread, or call threads_list to see valid ids.",
    );
    Err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::threads::ThreadStore;
    use serde_json::json;
    use tempfile::TempDir;

    fn ws() -> &'static Path {
        std::env::temp_dir().join("rustyclaw-cron-tool-ws").leak()
    }

    /// A settings dir with a thread store holding one chat thread, as the
    /// gateway would persist it. Returns (tempdir, thread id).
    fn ctx_with_threads() -> (TempDir, u64) {
        let tmp = tempfile::tempdir().unwrap();
        crate::runtime_ctx::set_agent_registry_info(tmp.path(), "RustyClaw");

        let threads_json = Config::agent_threads_path_from(tmp.path(), "main");
        let store = ThreadStore::at_legacy_path(&threads_json);
        let mut mgr = crate::threads::ThreadManager::new();
        let id = mgr.create_chat("Work");
        store.persist(&mut mgr).unwrap();
        (tmp, id.0)
    }

    fn add_job(thread_id: Option<u64>) -> ToolResult {
        let mut job = json!({
            "schedule": { "kind": "every", "everyMs": 600000 },
            "payload": { "kind": "agentTurn", "message": "ping" },
        });
        if let Some(tid) = thread_id {
            job["threadId"] = json!(tid);
        }
        exec_cron(&json!({ "action": "add", "job": job }), ws())
    }

    #[test]
    fn add_accepts_an_existing_thread_id() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, tid) = ctx_with_threads();
        let out = add_job(Some(tid)).unwrap();
        assert!(out.contains("Created job"), "{out}");
        drop(tmp);
    }

    #[test]
    fn add_rejects_a_unknown_thread_id_with_valid_ids() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, tid) = ctx_with_threads();

        let err = add_job(Some(9999)).unwrap_err().to_string();
        assert!(err.contains("threadId 9999 does not exist"), "{err}");
        assert!(err.contains(&format!("#{} (Work)", tid)), "{err}");

        drop(tmp);
    }

    #[test]
    fn add_without_thread_id_pins_to_the_calling_thread() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, tid) = ctx_with_threads();

        // An interactive turn's caller identity is `thread:<id>`.
        let out = crate::tool_caller::with_caller_blocking(Some(format!("thread:{tid}")), || {
            add_job(None)
        })
        .unwrap();
        assert!(out.contains("Created job"), "{out}");

        // The created job must be pinned to the calling thread.
        let listed = exec_cron(&json!({ "action": "list" }), ws()).unwrap();
        assert!(listed.contains(&format!("thread #{tid}")), "{listed}");

        drop(tmp);
    }

    #[test]
    fn pin_is_refused_across_agents() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, tid) = ctx_with_threads();

        // The caller is in thread `tid` of the *main* agent, but the job
        // targets agent "helper". Thread ids are per-agent, so pinning would
        // route the wake into helper's thread `tid` — a different
        // conversation. The job must stay unpinned instead.
        let job = json!({
            "agentId": "helper",
            "schedule": { "kind": "every", "everyMs": 600000 },
            "payload": { "kind": "agentTurn", "message": "ping" },
        });
        let out = crate::tool_caller::with_caller_blocking(Some(format!("thread:{tid}")), || {
            exec_cron(&json!({ "action": "add", "job": job }), ws())
        })
        .unwrap();
        assert!(out.contains("Created job"), "{out}");
        let listed = exec_cron(&json!({ "action": "list" }), ws()).unwrap();
        assert!(listed.contains("foreground"), "{listed}");
        assert!(!listed.contains("thread #"), "{listed}");

        drop(tmp);
    }

    #[test]
    fn add_without_thread_id_and_without_caller_stays_unpinned() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, _tid) = ctx_with_threads();

        // No caller identity → no pinning; the job targets the foreground
        // at fire time, as before.
        let out = add_job(None).unwrap();
        assert!(out.contains("Created job"), "{out}");
        let listed = exec_cron(&json!({ "action": "list" }), ws()).unwrap();
        assert!(listed.contains("foreground"), "{listed}");
        assert!(!listed.contains("thread #"), "{listed}");

        drop(tmp);
    }

    #[test]
    fn update_rejects_a_unknown_thread_id() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, _tid) = ctx_with_threads();
        let created = add_job(None).unwrap();
        let job_id = created
            .strip_prefix("Created job: ")
            .expect("created job id")
            .trim();

        let err = exec_cron(
            &json!({ "action": "update", "jobId": job_id, "patch": { "threadId": 424242 } }),
            ws(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("threadId 424242 does not exist"), "{err}");
        drop(tmp);
    }

    #[test]
    fn threads_list_shows_ids_labels_and_foreground() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, tid) = ctx_with_threads();

        let out = (crate::tools::definitions::THREADS_LIST.execute)(&json!({}), ws()).unwrap();
        assert!(
            out.contains(&format!("#{} — Work [foreground]", tid)),
            "{out}"
        );
        assert!(out.contains("main"), "{out}");
        assert!(out.contains("threadId"), "{out}");

        drop(tmp);
    }

    #[test]
    fn threads_list_without_a_settings_dir_explains() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // No gateway settings dir published: the tool must fail gracefully
        // rather than panic on a missing global.
        crate::runtime_ctx::runtime_ctx()
            .lock()
            .unwrap()
            .settings_dir = None;
        let err = (crate::tools::definitions::THREADS_LIST.execute)(&json!({}), ws())
            .unwrap_err()
            .to_string();
        assert!(err.contains("no gateway settings directory"), "{err}");
    }

    #[test]
    fn session_status_reports_the_calling_thread() {
        let _guard = crate::runtime_ctx::TEST_CTX_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (tmp, tid) = ctx_with_threads();

        let out = crate::tool_caller::with_caller_blocking(Some(format!("thread:{tid}")), || {
            crate::tools::sessions_tools::exec_session_status(&json!({}), ws())
        })
        .unwrap();
        assert!(out.contains(&format!("Current thread: {}", tid)), "{out}");

        // Without a thread caller there is no current thread to report.
        let out = crate::tools::sessions_tools::exec_session_status(&json!({}), ws()).unwrap();
        assert!(!out.contains("Current thread:"), "{out}");

        drop(tmp);
    }
}
