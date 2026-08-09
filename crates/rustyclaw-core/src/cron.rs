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
    // A millisecond timestamp alone collides when two jobs are created in the
    // same millisecond (the scheduler creates them back-to-back), so a
    // per-process counter disambiguates.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("job-{:x}-{:x}", timestamp, seq)
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
    /// Next scheduled run timestamp (ms since epoch). The scheduler's
    /// single source of "when": persisted, so a fire time survives a
    /// gateway restart, and a job whose moment passed while the gateway
    /// was down fires on the next boot instead of never.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_ms: Option<u64>,
    /// Created timestamp (ms since epoch).
    pub created_ms: u64,
    /// Thread the wake lands in: its history is the turn's context and the
    /// exchange is appended to it. `None` targets the agent's foreground
    /// thread at fire time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<u64>,
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
            thread_id: None,
        }
    }

    /// When this job should fire next, in ms since the epoch.
    ///
    /// `None` means "never again": a one-shot that already ran, an
    /// unparseable schedule, or a cron expression with no future match.
    /// A time in the past is a valid answer — it means the moment was
    /// missed (the gateway was down, or the one-shot was set for a time
    /// already gone) and the job should fire as soon as possible. Firing
    /// late beats never firing: the whole point of a wake is that it
    /// happens.
    pub fn next_fire_ms(&self, now_ms: u64) -> Option<u64> {
        match &self.schedule {
            Schedule::At { at } => {
                // One-shot: fires once, and having run is the only thing
                // that retires it.
                if self.last_run_ms.is_some() {
                    return None;
                }
                let t = chrono::DateTime::parse_from_rfc3339(at).ok()?;
                Some(t.timestamp_millis().max(0) as u64)
            }
            Schedule::Every {
                every_ms,
                anchor_ms,
            } => {
                if *every_ms == 0 {
                    return None;
                }
                // The grid is anchored: fires land at anchor + n·interval,
                // not at "whenever the last one happened plus interval", so
                // a slow run does not drift the schedule. Always the first
                // grid point after now: catch-up for downtime comes from
                // the *persisted* `next_run_ms` (armed before the gateway
                // went down, fired on boot), never from recomputation —
                // recomputing happens on resume-from-pause too, where
                // firing the slots the user paused through would be wrong.
                let anchor = anchor_ms.unwrap_or(self.created_ms);
                let base = self
                    .last_run_ms
                    .into_iter()
                    .chain([anchor, now_ms])
                    .max()
                    .unwrap_or(now_ms);
                let n = base.saturating_sub(anchor) / every_ms + 1;
                Some(anchor + n * every_ms)
            }
            Schedule::Cron { expr, tz } => {
                let cron = croner::Cron::new(expr)
                    .with_seconds_optional()
                    .parse()
                    .ok()?;
                let after_ms = self.last_run_ms.map_or(now_ms, |l| l.max(now_ms));
                let after = chrono::DateTime::from_timestamp_millis(after_ms as i64)?;
                match tz.as_deref() {
                    Some(name) => {
                        let zone: chrono_tz::Tz = name.parse().ok()?;
                        cron.find_next_occurrence(&after.with_timezone(&zone), false)
                            .ok()
                            .map(|t| t.timestamp_millis().max(0) as u64)
                    }
                    None => cron
                        .find_next_occurrence(&after.with_timezone(&chrono::Local), false)
                        .ok()
                        .map(|t| t.timestamp_millis().max(0) as u64),
                }
            }
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
    /// A job with this id is already in the store.
    #[error("Job already exists: {0} (use update to change it)")]
    JobExists(String),
}

/// Fold a patched payload onto the stored one, keeping settings the patch
/// left unspecified.
///
/// `Payload` is patched as a whole object — there is no field-level payload
/// patch — so a caller changing only the prompt sends an `AgentTurn` with
/// `model: None`. Taken literally that clears the model override, and the
/// commonest edit there is silently discards it. Within the same variant the
/// unspecified optionals therefore carry forward.
///
/// Switching variants replaces outright: a `SystemEvent` has no model to keep.
///
/// The limitation this accepts: with `None` meaning "leave alone", there is no
/// way to *clear* a model through a patch. That was already true of the client
/// panel, which folds the same way by hand, so nothing is lost here — and a
/// caller can still clear it by replacing the job.
fn merge_payload(current: &Payload, patch: Payload) -> Payload {
    match (current, patch) {
        (
            Payload::AgentTurn {
                model: old_model,
                thinking: old_thinking,
                timeout_seconds: old_timeout,
                ..
            },
            Payload::AgentTurn {
                message,
                model,
                thinking,
                timeout_seconds,
            },
        ) => Payload::AgentTurn {
            message,
            model: model.or_else(|| old_model.clone()),
            thinking: thinking.or_else(|| old_thinking.clone()),
            timeout_seconds: timeout_seconds.or(*old_timeout),
        },
        (_, patch) => patch,
    }
}

/// Serialize open→mutate→save cycles on the jobs file. Three writers share
/// it — the agent tool (blocking pool), the client panel handler (async),
/// and the scheduler — and each rewrites the whole file on save, so an
/// unserialized interleave silently drops the other writer's change.
static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The one place cron jobs live: `<settings_dir>/cron/`.
///
/// Centralized deliberately. The agent tool used to open `.cron` under the
/// *per-agent* workspace while the client panel opened it under the
/// *gateway's* — two stores, each seeing half the jobs. A scheduler can
/// only fire what it can see, so there is exactly one directory and every
/// caller derives it from here.
pub fn central_cron_dir(settings_dir: &Path) -> PathBuf {
    settings_dir.join("cron")
}

/// Open the central store and run `f` on it, holding the store lock for
/// the whole read-modify-write. Mutating methods save internally, so `f`
/// needs no explicit save call.
pub fn with_store<R>(
    settings_dir: &Path,
    f: impl FnOnce(&mut CronStore) -> Result<R, CronError>,
) -> Result<R, CronError> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = CronStore::new(&central_cron_dir(settings_dir))?;
    f(&mut store)
}

/// One-time adoption of a legacy `.cron` directory into the central store.
///
/// Only when the central store has no jobs file yet and the legacy one
/// does: the legacy `jobs.json` is copied in and the original renamed
/// aside, so it cannot be adopted twice or shadow the central store later.
pub fn adopt_legacy_store(settings_dir: &Path, legacy_dir: &Path) {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let central_jobs = central_cron_dir(settings_dir).join("jobs.json");
    let legacy_jobs = legacy_dir.join("jobs.json");
    if central_jobs.exists() || !legacy_jobs.exists() {
        return;
    }
    let adopted = fs::read(&legacy_jobs)
        .and_then(|bytes| crate::persist::write_atomically(&central_jobs, &bytes));
    match adopted {
        Ok(()) => {
            let retired = legacy_jobs.with_extension("json.migrated");
            fs::rename(&legacy_jobs, &retired).ok();
            tracing::info!(
                from = %legacy_jobs.display(),
                to = %central_jobs.display(),
                "Adopted legacy cron jobs into the central store"
            );
        }
        Err(e) => {
            tracing::warn!(
                from = %legacy_jobs.display(),
                error = %e,
                "Could not adopt legacy cron jobs"
            );
        }
    }
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

        // Load existing jobs. A corrupt jobs file used to fail `new`
        // outright, taking the whole cron subsystem down over one bad
        // byte; quarantine it and start empty instead — the bytes stay
        // recoverable, and scheduling keeps working.
        let jobs = if jobs_path.exists() {
            let content = fs::read_to_string(&jobs_path)?;
            match serde_json::from_str(&content) {
                Ok(jobs) => jobs,
                Err(e) => {
                    crate::persist::quarantine(&jobs_path, &e.to_string());
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Ok(Self {
            jobs_path,
            runs_dir,
            jobs,
        })
    }

    /// Save jobs to disk. Atomic: every job lives in this one file, so a
    /// torn write is not one job lost but all of them.
    fn save(&self) -> Result<(), CronError> {
        let content = serde_json::to_string_pretty(&self.jobs)?;
        crate::persist::write_atomically(&self.jobs_path, content.as_bytes())?;
        Ok(())
    }

    /// Add a new job.
    ///
    /// Refuses an id that is already taken. The caller supplies `job_id`, so a
    /// clash is a real possibility — and a plain insert would silently replace
    /// somebody else's schedule with this one, which looks like the old job
    /// simply vanished. Use [`update`](Self::update) to change an existing job.
    pub fn add(&mut self, job: CronJob) -> Result<JobId, CronError> {
        let id = job.job_id.clone();
        if self.jobs.contains_key(&id) {
            return Err(CronError::JobExists(id));
        }
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
            // A pause forgets the pending fire; a resume recomputes it
            // fresh rather than firing a moment that passed while paused.
            job.next_run_ms = None;
        }
        if let Some(schedule) = patch.schedule {
            job.schedule = schedule;
            // The stored fire time belonged to the old schedule.
            job.next_run_ms = None;
        }
        if let Some(payload) = patch.payload {
            job.payload = merge_payload(&job.payload, payload);
        }
        if let Some(delivery) = patch.delivery {
            job.delivery = Some(delivery);
        }
        if let Some(description) = patch.description {
            job.description = Some(description);
        }
        if let Some(agent_id) = patch.agent_id {
            job.agent_id = Some(agent_id);
        }
        if let Some(thread_id) = patch.thread_id {
            job.thread_id = Some(thread_id);
        }
        if let Some(session_target) = patch.session_target {
            job.session_target = session_target;
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

    /// Bring every job's `next_run_ms` in line with its schedule and
    /// enabled state. Returns the earliest pending fire time, which is
    /// what the scheduler sleeps until.
    ///
    /// Only fills what is missing: an already-computed `next_run_ms` is
    /// left alone, because "run now" works by writing an immediate time
    /// there and recomputing would erase the request.
    pub fn ensure_next_runs(&mut self, now_ms: u64) -> Result<Option<u64>, CronError> {
        let mut changed = false;
        for job in self.jobs.values_mut() {
            let wanted = if job.enabled {
                match job.next_run_ms {
                    Some(t) => Some(t),
                    None => job.next_fire_ms(now_ms),
                }
            } else {
                None
            };
            if job.next_run_ms != wanted {
                job.next_run_ms = wanted;
                changed = true;
            }
        }
        if changed {
            self.save()?;
        }
        Ok(self
            .jobs
            .values()
            .filter(|j| j.enabled)
            .filter_map(|j| j.next_run_ms)
            .min())
    }

    /// Take every enabled job whose moment has arrived, advancing each
    /// one's bookkeeping in the same step: `last_run_ms` becomes now, the
    /// following fire time is computed and stored, and a one-shot with
    /// nothing left to do is deleted if it asked to be. Returns snapshots
    /// for the caller to execute — the store is already consistent by the
    /// time they run, so a crash mid-execution costs one wake, not a
    /// double-fire on restart.
    pub fn take_due_jobs(&mut self, now_ms: u64) -> Result<Vec<CronJob>, CronError> {
        let due_ids: Vec<JobId> = self
            .jobs
            .values()
            .filter(|j| j.enabled && j.next_run_ms.is_some_and(|t| t <= now_ms))
            .map(|j| j.job_id.clone())
            .collect();

        let mut due = Vec::new();
        for id in due_ids {
            let Some(job) = self.jobs.get_mut(&id) else {
                continue;
            };
            job.last_run_ms = Some(now_ms);
            job.next_run_ms = None;
            let next = job.next_fire_ms(now_ms);
            job.next_run_ms = next;
            let snapshot = job.clone();
            if next.is_none() && job.delete_after_run {
                self.jobs.remove(&id);
            }
            due.push(snapshot);
        }
        if !due.is_empty() {
            self.save()?;
        }
        Ok(due)
    }

    /// Ask for an immediate fire: the job's next run becomes "now", and
    /// the scheduler's next pass picks it up like any other due job. The
    /// regular schedule resumes afterwards, because `take_due_jobs`
    /// recomputes from the schedule when it fires.
    pub fn request_run_now(&mut self, job_id: &str) -> Result<(), CronError> {
        let job = self
            .jobs
            .get_mut(job_id)
            .ok_or_else(|| CronError::JobNotFound(job_id.to_string()))?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        job.enabled = true;
        job.next_run_ms = Some(now_ms);
        self.save()
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

    /// Record a run. Durable append: the old path dropped the handle
    /// without a flush, so a write error surfaced nowhere and the record
    /// could vanish with the page cache.
    pub fn record_run(&self, entry: &RunEntry) -> Result<(), CronError> {
        let runs_file = self.runs_dir.join(format!("{}.jsonl", entry.job_id));
        let mut line = serde_json::to_string(entry)?;
        line.push('\n');
        crate::persist::append_durably(&runs_file, line.as_bytes())?;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_target: Option<SessionTarget>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// An `AgentTurn` job with a model override, as the panel or tool creates.
    fn job_with_model(model: &str) -> CronJob {
        CronJob::new(
            Some("nightly".to_string()),
            Schedule::Cron {
                expr: "0 3 * * *".to_string(),
                tz: None,
            },
            SessionTarget::Main,
            Payload::AgentTurn {
                message: "summarize yesterday".to_string(),
                model: Some(model.to_string()),
                thinking: Some("high".to_string()),
                timeout_seconds: Some(600),
            },
        )
    }

    #[test]
    fn patching_a_prompt_keeps_the_model_override() {
        // `Payload` is patched whole, so editing only the prompt sends
        // `model: None`. Taken literally that silently discards the override
        // the user chose — the commonest edit destroying the setting.
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();
        let id = store.add(job_with_model("claude-haiku")).unwrap();

        store
            .update(
                &id,
                CronJobPatch {
                    payload: Some(Payload::AgentTurn {
                        message: "summarize the last 24h".to_string(),
                        model: None,
                        thinking: None,
                        timeout_seconds: None,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();

        match &store.get(&id).unwrap().payload {
            Payload::AgentTurn {
                message,
                model,
                thinking,
                timeout_seconds,
            } => {
                assert_eq!(message, "summarize the last 24h", "the prompt does change");
                assert_eq!(model.as_deref(), Some("claude-haiku"));
                assert_eq!(thinking.as_deref(), Some("high"));
                assert_eq!(*timeout_seconds, Some(600));
            }
            other => panic!("expected an agent turn, got {other:?}"),
        }
    }

    #[test]
    fn a_patch_can_still_change_the_model() {
        // Carry-forward must not become "the model is frozen".
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();
        let id = store.add(job_with_model("claude-haiku")).unwrap();

        store
            .update(
                &id,
                CronJobPatch {
                    payload: Some(Payload::AgentTurn {
                        message: "summarize yesterday".to_string(),
                        model: Some("claude-opus".to_string()),
                        thinking: None,
                        timeout_seconds: None,
                    }),
                    ..Default::default()
                },
            )
            .unwrap();

        match &store.get(&id).unwrap().payload {
            Payload::AgentTurn { model, .. } => assert_eq!(model.as_deref(), Some("claude-opus")),
            other => panic!("expected an agent turn, got {other:?}"),
        }
    }

    #[test]
    fn switching_payload_kind_replaces_outright() {
        // A system event has no model to carry, so nothing should be folded in.
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();
        let id = store.add(job_with_model("claude-haiku")).unwrap();

        store
            .update(
                &id,
                CronJobPatch {
                    payload: Some(Payload::SystemEvent {
                        text: "just a note".to_string(),
                    }),
                    ..Default::default()
                },
            )
            .unwrap();

        assert!(matches!(
            store.get(&id).unwrap().payload,
            Payload::SystemEvent { .. }
        ));
    }

    #[test]
    fn adding_a_job_twice_is_refused_rather_than_silently_replacing() {
        // The caller supplies `job_id`, so a clash is reachable. A plain
        // insert would overwrite someone else's schedule and look like the
        // original job had simply vanished.
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();
        let first = job_with_model("claude-haiku");
        let id = first.job_id.clone();
        store.add(first).unwrap();

        let mut clash = job_with_model("claude-opus");
        clash.job_id = id.clone();
        let err = store.add(clash).unwrap_err();
        assert!(
            matches!(err, CronError::JobExists(ref got) if got == &id),
            "{err}"
        );

        // The original survives untouched.
        match &store.get(&id).unwrap().payload {
            Payload::AgentTurn { model, .. } => assert_eq!(model.as_deref(), Some("claude-haiku")),
            other => panic!("expected an agent turn, got {other:?}"),
        }
    }

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
    /// One bad byte in jobs.json used to refuse to initialize the whole
    /// cron subsystem. The store must come up empty instead, with the
    /// corrupt file set aside rather than destroyed.
    #[test]
    fn a_corrupt_jobs_file_is_quarantined_not_fatal() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("jobs.json"), b"{ this is not json").unwrap();

        let store = CronStore::new(dir.path()).expect("store must initialize");
        assert!(store.list(true).is_empty());

        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(quarantined.len(), 1, "corrupt file should be set aside");
    }
    // ── Schedule math ───────────────────────────────────────────────────

    fn job_with(schedule: Schedule) -> CronJob {
        CronJob::new(
            Some("t".into()),
            schedule,
            SessionTarget::Main,
            Payload::SystemEvent { text: "hi".into() },
        )
    }

    /// Interval fires land on an anchored grid — a slow run or downtime
    /// must not drift the schedule, and a missed slot fires once, not as
    /// a burst of catch-ups.
    #[test]
    fn every_fires_on_an_anchored_grid() {
        let mut job = job_with(Schedule::Every {
            every_ms: 100,
            anchor_ms: Some(1_000),
        });

        // Never run: the first fire is the first grid point after the anchor.
        assert_eq!(job.next_fire_ms(1_000), Some(1_100));

        // Ran on time: the next grid point.
        job.last_run_ms = Some(1_100);
        assert_eq!(job.next_fire_ms(1_105), Some(1_200));

        // Recomputing after downtime aims forward — slots missed while
        // down are NOT re-derived here (catch-up is the job of the
        // persisted next_run_ms, armed before the gateway went down), so
        // a resume-from-pause cannot fire the slots the user paused
        // through.
        assert_eq!(job.next_fire_ms(1_550), Some(1_600));
        job.last_run_ms = Some(1_550);
        assert_eq!(job.next_fire_ms(1_550), Some(1_600));
    }

    /// A one-shot fires even if its moment passed while the gateway was
    /// down — and never fires twice.
    #[test]
    fn a_one_shot_fires_late_but_only_once() {
        let mut job = job_with(Schedule::At {
            at: "2026-01-01T09:00:00Z".to_string(),
        });
        assert!(job.delete_after_run, "one-shots ask to be deleted");

        let nine_am_ms = chrono::DateTime::parse_from_rfc3339("2026-01-01T09:00:00Z")
            .unwrap()
            .timestamp_millis() as u64;
        // Asked long after the moment: still that moment (i.e. "overdue").
        assert_eq!(job.next_fire_ms(nine_am_ms + 3_600_000), Some(nine_am_ms));

        job.last_run_ms = Some(nine_am_ms + 3_600_000);
        assert_eq!(job.next_fire_ms(nine_am_ms + 3_600_001), None);
    }

    /// A cron expression resolves to the next matching wall-clock moment
    /// in its timezone.
    #[test]
    fn a_cron_expression_finds_the_next_match() {
        let job = job_with(Schedule::Cron {
            expr: "0 * * * *".to_string(),
            tz: Some("UTC".to_string()),
        });
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:30:00Z")
            .unwrap()
            .timestamp_millis() as u64;
        let next = job.next_fire_ms(now).expect("hourly always has a next");
        let expected = chrono::DateTime::parse_from_rfc3339("2026-01-01T01:00:00Z")
            .unwrap()
            .timestamp_millis() as u64;
        assert_eq!(next, expected);
    }

    /// An unparseable cron expression schedules nothing rather than
    /// wedging the scheduler.
    #[test]
    fn a_bad_cron_expression_never_fires() {
        let job = job_with(Schedule::Cron {
            expr: "not a cron line".to_string(),
            tz: None,
        });
        assert_eq!(job.next_fire_ms(0), None);
    }

    // ── Scheduler bookkeeping ───────────────────────────────────────────

    /// take_due_jobs advances last/next in the same step it hands the job
    /// out, and a spent one-shot that asked to be deleted is gone.
    #[test]
    fn due_jobs_are_taken_with_their_bookkeeping_settled() {
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();

        let mut interval = job_with(Schedule::Every {
            every_ms: 60_000,
            anchor_ms: Some(0),
        });
        interval.next_run_ms = Some(60_000);
        let interval_id = store.add(interval).unwrap();

        let mut oneshot = job_with(Schedule::At {
            at: "1970-01-01T00:00:30Z".to_string(),
        });
        oneshot.next_run_ms = Some(30_000);
        let oneshot_id = store.add(oneshot).unwrap();

        let due = store.take_due_jobs(61_000).unwrap();
        assert_eq!(due.len(), 2, "both were due");

        // The interval advanced onto its next grid point.
        let again = store.get(&interval_id).unwrap();
        assert_eq!(again.last_run_ms, Some(61_000));
        assert_eq!(again.next_run_ms, Some(120_000));

        // The one-shot is spent and deleted.
        assert!(store.get(&oneshot_id).is_none());

        // Nothing is due twice.
        assert!(store.take_due_jobs(61_000).unwrap().is_empty());
    }

    /// ensure_next_runs fills only what is missing — a pending "run now"
    /// request must survive it — and reports the earliest fire.
    #[test]
    fn ensure_next_runs_fills_gaps_without_erasing_requests() {
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();

        let job = job_with(Schedule::Every {
            every_ms: 100_000,
            anchor_ms: Some(0),
        });
        let id = store.add(job).unwrap();

        let earliest = store.ensure_next_runs(50_000).unwrap();
        assert_eq!(earliest, Some(100_000));

        store.request_run_now(&id).unwrap();
        let requested = store.get(&id).unwrap().next_run_ms.unwrap();
        let earliest = store.ensure_next_runs(50_000).unwrap();
        assert_eq!(earliest, Some(requested), "run-now request survives");
    }

    /// Disabling a job clears its pending fire; re-enabling recomputes it
    /// instead of firing a moment that passed while paused.
    #[test]
    fn pausing_forgets_the_pending_fire() {
        let dir = TempDir::new().unwrap();
        let mut store = CronStore::new(dir.path()).unwrap();
        let job = job_with(Schedule::Every {
            every_ms: 100,
            anchor_ms: Some(0),
        });
        let id = store.add(job).unwrap();
        store.ensure_next_runs(50).unwrap();
        assert!(store.get(&id).unwrap().next_run_ms.is_some());

        store
            .update(
                &id,
                CronJobPatch {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        store.ensure_next_runs(150).unwrap();
        assert_eq!(store.get(&id).unwrap().next_run_ms, None);

        store
            .update(
                &id,
                CronJobPatch {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        let earliest = store.ensure_next_runs(150).unwrap();
        assert_eq!(earliest, Some(200), "recomputed forward from now");
    }
}
