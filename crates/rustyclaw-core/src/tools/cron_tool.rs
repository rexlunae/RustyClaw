//! Cron tool: scheduled job management.

use serde_json::Value;
use std::path::Path;
use tracing::{debug, instrument, warn};

/// Cron job management.
#[instrument(skip(args, workspace_dir), fields(action))]
pub fn exec_cron(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    use crate::cron::*;

    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing cron tool");

    let cron_dir = workspace_dir.join(".cron");
    let mut store = CronStore::new(&cron_dir)?;

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
                output.push_str(&format!(
                    "{} {} [{}] — {}\n",
                    status, job.job_id, name, schedule
                ));
            }
            Ok(output)
        }

        "add" => {
            let job_obj = args.get("job").ok_or("Missing required parameter: job")?;

            let job: CronJob = serde_json::from_value(job_obj.clone())
                .map_err(|e| format!("Invalid job definition: {}", e))?;

            let id = store.add(job)?;
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
                .map_err(|e| format!("Invalid patch: {}", e))?;

            store.update(job_id, patch)?;
            debug!(job_id, "Updated cron job");
            Ok(format!("Updated job: {}", job_id))
        }

        "remove" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for remove")?;

            store.remove(job_id)?;
            debug!(job_id, "Removed cron job");
            Ok(format!("Removed job: {}", job_id))
        }

        "run" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for run")?;

            let job = store
                .get(job_id)
                .ok_or_else(|| format!("Job not found: {}", job_id))?;

            debug!(job_id, "Manual run requested");
            Ok(format!(
                "Would run job '{}' ({}). Note: actual execution requires gateway integration.",
                job.name.as_deref().unwrap_or("unnamed"),
                job_id
            ))
        }

        "runs" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for runs")?;

            let runs = store.get_runs(job_id, 10)?;
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
                "Unknown action: {}. Valid: status, list, add, update, remove, run, runs",
                action
            ))
        }
    }
}
