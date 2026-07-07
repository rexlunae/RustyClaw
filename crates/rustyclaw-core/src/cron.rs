//! Cron job scheduling for RustyClaw.
//!
//! Provides a simple job scheduler that persists jobs to disk and can
//! trigger agent turns or system events on schedule.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Unique identifier for a cron job.
pub type JobId = String;

/// Generate a unique job ID.
fn generate_job_id() -> JobId {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("job-{:x}", timestamp)
}

/// Schedule kinds for cron jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// One-shot at an absolute time (ISO 8601).
    At { at: String },
    /// Recurring interval in milliseconds.
    Every {
        every_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        anchor_ms: Option<u64>,
    },
    /// Cron expression (5-field).
    Cron {
        expr: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
    },
}

/// Payload kinds for cron jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Payload {
    /// System event injected into main session.
    SystemEvent { text: String },
    /// Agent turn in an isolated session.
    AgentTurn {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_seconds: Option<u64>,
    },
}

/// Delivery configuration for isolated jobs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    #[serde(default)]
    pub mode: DeliveryMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default)]
    pub best_effort: bool,
}

/// Delivery mode.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DeliveryMode {
    #[default]
    Announce,
    None,
}

/// Session target for job execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SessionTarget {
    Main,
    Isolated,
}

/// A cron job definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub job_id: JobId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub schedule: Schedule,
    pub session_target: SessionTarget,
    pub payload: Payload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// For one-shot jobs, delete after successful run.
    #[serde(default)]
    pub delete_after_run: bool,
    /// Last run timestamp (ms since epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run_ms: Option<u64>,
    /// Next scheduled run timestamp (ms since epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_ms: Option<u64>,
    /// Created timestamp (ms since epoch).
    pub created_ms: u64,
}

fn default_true() -> bool {
    true
}

impl CronJob {
    /// Create a new job with the given parameters.
    pub fn new(
        name: Option<String>,
        schedule: Schedule,
        session_target: SessionTarget,
        payload: Payload,
    ) -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let delete_after_run = matches!(schedule, Schedule::At { .. });

        Self {
            job_id: generate_job_id(),
            name,
            description: None,
            schedule,
            session_target,
            payload,
            delivery: None,
            enabled: true,
            agent_id: None,
            delete_after_run,
            last_run_ms: None,
            next_run_ms: None,
            created_ms: now_ms,
        }
    }
}

/// Run history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEntry {
    pub job_id: JobId,
    pub run_id: String,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Run status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    Ok,
    Error,
    Timeout,
    Skipped,
}

/// Errors produced by [`CronStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum CronError {
    /// Filesystem operation on the jobs/runs store failed.
    #[error("Cron store I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing or parsing job/run JSON failed.
    #[error("Cron store serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    /// The referenced job does not exist.
    #[error("Job not found: {0}")]
    JobNotFound(String),
}

/// Cron job store that persists jobs to disk.
pub struct CronStore {
    /// Path to the jobs file.
    jobs_path: PathBuf,
    /// Path to the runs directory.
    runs_dir: PathBuf,
    /// In-memory job cache.
    jobs: HashMap<JobId, CronJob>,
}

impl CronStore {
    /// Create or load a cron store from the given directory.
    pub fn new(cron_dir: &Path) -> Result<Self, CronError> {
        let jobs_path = cron_dir.join("jobs.json");
        let runs_dir = cron_dir.join("runs");

        // Ensure directories exist
        fs::create_dir_all(cron_dir)?;
        fs::create_dir_all(&runs_dir)?;

        // Load existing jobs
        let jobs = if jobs_path.exists() {
            let content = fs::read_to_string(&jobs_path)?;
            serde_json::from_str(&content)?
        } else {
            HashMap::new()
        };

        Ok(Self {
            jobs_path,
            runs_dir,
            jobs,
        })
    }

    /// Save jobs to disk.
    fn save(&self) -> Result<(), CronError> {
        let content = serde_json::to_string_pretty(&self.jobs)?;
        fs::write(&self.jobs_path, content)?;
        Ok(())
    }

    /// Add a new job.
    pub fn add(&mut self, job: CronJob) -> Result<JobId, CronError> {
        let id = job.job_id.clone();
        self.jobs.insert(id.clone(), job);
        self.save()?;
        Ok(id)
    }

    /// Get a job by ID.
    pub fn get(&self, job_id: &str) -> Option<&CronJob> {
        self.jobs.get(job_id)
    }

    /// List all jobs.
    pub fn list(&self, include_disabled: bool) -> Vec<&CronJob> {
        self.jobs
            .values()
            .filter(|j| include_disabled || j.enabled)
            .collect()
    }

    /// Update a job with a patch.
    pub fn update(&mut self, job_id: &str, patch: CronJobPatch) -> Result<(), CronError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;

        if let Some(name) = patch.name {
            job.name = Some(name);
        }
        if let Some(enabled) = patch.enabled {
            job.enabled = enabled;
        }
        if let Some(schedule) = patch.schedule {
            job.schedule = schedule;
        }
        if let Some(payload) = patch.payload {
            job.payload = payload;
        }
        if let Some(delivery) = patch.delivery {
            job.delivery = Some(delivery);
        }

        self.save()
    }

    /// Remove a job.
    pub fn remove(&mut self, job_id: &str) -> Result<CronJob, CronError> {
        let job = self
            .jobs
            .remove(job_id)
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        self.save()?;
        Ok(job)
    }

    /// Get run history for a job.
    pub fn get_runs(&self, job_id: &str, limit: usize) -> Result<Vec<RunEntry>, CronError> {
        let runs_file = self.runs_dir.join(format!("{}.jsonl", job_id));
        if !runs_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&runs_file)?;

        let runs: Vec<RunEntry> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str(line) {
                Ok(entry) => Some(entry),
                Err(e) => {
                    tracing::warn!(job_id, error = %e, "Skipping corrupt run-history line");
                    None
                }
            })
            .collect();

        // Return last N runs
        Ok(runs.into_iter().rev().take(limit).collect())
    }

    /// Record a run.
    pub fn record_run(&self, entry: &RunEntry) -> Result<(), CronError> {
        let runs_file = self.runs_dir.join(format!("{}.jsonl", entry.job_id));
        let line = serde_json::to_string(entry)?;

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&runs_file)?;

        writeln!(file, "{}", line)?;

        Ok(())
    }
}

/// Patch for updating a cron job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Schedule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Payload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_create_job() {
        let job = CronJob::new(
            Some("Test Job".to_string()),
            Schedule::At {
                at: "2026-02-12T18:00:00Z".to_string(),
            },
            SessionTarget::Main,
            Payload::SystemEvent {
                text: "Test reminder".to_string(),
            },
        );

        assert!(job.job_id.starts_with("job-"));
        assert_eq!(job.name, Some("Test Job".to_string()));
        assert!(job.enabled);
        assert!(job.delete_after_run); // One-shot jobs auto-delete
    }

    #[test]
    fn test_cron_store_add_list() {
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();

        let job = CronJob::new(
            Some("Test".to_string()),
            Schedule::Every {
                every_ms: 60000,
                anchor_ms: None,
            },
            SessionTarget::Isolated,
            Payload::AgentTurn {
                message: "Do something".to_string(),
                model: None,
                thinking: None,
                timeout_seconds: None,
            },
        );

        let id = store.add(job).unwrap();
        let jobs = store.list(false);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, id);
    }

    #[test]
    fn test_cron_store_persistence() {
        let dir = TempDir::new().unwrap();

        // Create and add job
        {
            let mut store = CronStore::new(dir.path()).unwrap();
            let job = CronJob::new(
                Some("Persistent".to_string()),
                Schedule::Cron {
                    expr: "0 * * * *".to_string(),
                    tz: None,
                },
                SessionTarget::Main,
                Payload::SystemEvent {
                    text: "Hourly check".to_string(),
                },
            );
            store.add(job).unwrap();
        }

        // Reload and verify
        {
            let store = CronStore::new(dir.path()).unwrap();
            let jobs = store.list(false);
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].name, Some("Persistent".to_string()));
        }
    }
}
