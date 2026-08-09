//! Session management for RustyClaw multi-agent support.
//!
//! Provides tools for spawning sub-agents, sending messages between sessions,
//! and managing session state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Session key format: agent:<agentId>:subagent:<uuid> or agent:<agentId>:main
pub type SessionKey = String;

/// Generate a unique session key for a sub-agent.
fn generate_subagent_key(agent_id: &str) -> SessionKey {
    let uuid = generate_uuid();
    format!("agent:{}:subagent:{}", agent_id, uuid)
}

/// Generate a simple UUID-like string.
fn generate_uuid() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", timestamp)
}

/// Session status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Active,
    Completed,
    Error,
    Timeout,
    Stopped,
}

/// Session kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionKind {
    Main,
    Subagent,
    Cron,
}

/// A message in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String, // "user", "assistant", "system", "tool"
    pub content: String,
    pub timestamp_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

/// A session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub key: SessionKey,
    pub agent_id: String,
    pub kind: SessionKind,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub created_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    /// Recent messages (limited for memory efficiency).
    pub messages: Vec<SessionMessage>,
    /// Run ID for sub-agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Parent session key (for sub-agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_key: Option<SessionKey>,
}

impl Session {
    /// Create a new main session.
    pub fn new_main(agent_id: &str) -> Self {
        let now_ms = now_millis();
        Self {
            key: format!("agent:{}:main", agent_id),
            agent_id: agent_id.to_string(),
            kind: SessionKind::Main,
            status: SessionStatus::Active,
            label: None,
            task: None,
            created_ms: now_ms,
            finished_ms: None,
            messages: Vec::new(),
            run_id: None,
            parent_key: None,
        }
    }

    /// Create a new sub-agent session.
    pub fn new_subagent(
        agent_id: &str,
        task: &str,
        label: Option<String>,
        parent_key: Option<SessionKey>,
    ) -> Self {
        let now_ms = now_millis();
        let run_id = generate_uuid();
        Self {
            key: generate_subagent_key(agent_id),
            agent_id: agent_id.to_string(),
            kind: SessionKind::Subagent,
            status: SessionStatus::Active,
            label,
            task: Some(task.to_string()),
            created_ms: now_ms,
            finished_ms: None,
            messages: Vec::new(),
            run_id: Some(run_id),
            parent_key,
        }
    }

    /// Add a message to the session.
    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(SessionMessage {
            role: role.to_string(),
            content: content.to_string(),
            timestamp_ms: now_millis(),
            tool_name: None,
        });

        // Keep only last 100 messages in memory
        if self.messages.len() > 100 {
            self.messages.remove(0);
        }
    }

    /// Mark session as completed.
    pub fn complete(&mut self) {
        self.status = SessionStatus::Completed;
        self.finished_ms = Some(now_millis());
    }

    /// Mark session as errored.
    pub fn error(&mut self) {
        self.status = SessionStatus::Error;
        self.finished_ms = Some(now_millis());
    }

    /// Mark session as stopped before it finished its task.
    ///
    /// Stamping `finished_ms` is the point: [`runtime_secs`](Self::runtime_secs)
    /// measures to *now* while it is unset, so a stopped run would otherwise
    /// keep reporting a growing runtime as though it were still working.
    pub fn stop(&mut self) {
        self.status = SessionStatus::Stopped;
        self.finished_ms = Some(now_millis());
    }

    /// Record a terminal status, but only if the session has not already
    /// ended. Returns whether it was applied.
    ///
    /// **A record that has already ended keeps its ending.** A run can be
    /// stopped while still winding down — `sessions_kill` writes the ending
    /// and tells the caller immediately, and the run may only notice seconds
    /// later. If whatever it produces in that window could still call
    /// [`complete`](Self::complete), a job the user cancelled would report
    /// itself as having succeeded, and the agent that spawned it would act on
    /// results that were never meant to exist.
    ///
    /// Use this from anywhere that closes a run it does not exclusively own.
    /// [`complete`](Self::complete), [`error`](Self::error) and
    /// [`stop`](Self::stop) stay unconditional for callers that are the sole
    /// authority on a record's fate.
    pub fn settle(&mut self, status: SessionStatus) -> bool {
        if self.status != SessionStatus::Active {
            return false;
        }
        self.status = status;
        self.finished_ms = Some(now_millis());
        true
    }

    /// Get runtime in seconds.
    pub fn runtime_secs(&self) -> u64 {
        let end = self.finished_ms.unwrap_or_else(now_millis);
        (end - self.created_ms) / 1000
    }
}

/// Errors produced by [`SessionManager`] operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// No session with this key exists.
    #[error("Session not found: {0}")]
    NotFound(String),
    /// The session exists but is not active.
    #[error("Session is not active: {0:?}")]
    NotActive(SessionStatus),
}

/// Global session manager.
pub struct SessionManager {
    sessions: HashMap<SessionKey, Session>,
    /// Map labels to session keys for easy lookup.
    labels: HashMap<String, SessionKey>,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            labels: HashMap::new(),
        }
    }

    /// Create or get a main session.
    pub fn get_or_create_main(&mut self, agent_id: &str) -> &Session {
        let key = format!("agent:{}:main", agent_id);
        self.sessions
            .entry(key.clone())
            .or_insert_with(|| Session::new_main(agent_id))
    }

    /// Spawn a sub-agent session.
    pub fn spawn_subagent(
        &mut self,
        agent_id: &str,
        task: &str,
        label: Option<String>,
        parent_key: Option<SessionKey>,
    ) -> SessionKey {
        let session = Session::new_subagent(agent_id, task, label.clone(), parent_key);
        let key = session.key.clone();

        if let Some(ref lbl) = label {
            self.labels.insert(lbl.clone(), key.clone());
        }

        self.sessions.insert(key.clone(), session);
        key
    }

    /// How many sub-agents spawned by `parent_key` are still active.
    ///
    /// Used to bound fan-out: a sub-agent can itself spawn sub-agents, so
    /// without a ceiling one prompt can expand into an unbounded tree.
    pub fn active_subagents_of(&self, parent_key: &str) -> usize {
        self.sessions
            .values()
            .filter(|s| {
                s.kind == SessionKind::Subagent
                    && s.status == SessionStatus::Active
                    && s.parent_key.as_deref() == Some(parent_key)
            })
            .count()
    }

    /// Every session started, directly or indirectly, by the run at `key`.
    ///
    /// A run's children record it as their parent under the identity it ran
    /// as — see [`crate::tool_caller`] — so the search is by identity string,
    /// not by key. Needed because stopping a run has to reach its whole
    /// subtree: once the parent's task is gone, nothing can present its
    /// identity, and an orphaned descendant would be unstoppable forever.
    pub fn descendants_of(&self, key: &str) -> Vec<SessionKey> {
        let mut found = Vec::new();
        let mut seen: std::collections::HashSet<SessionKey> =
            std::collections::HashSet::from([key.to_string()]);
        let mut frontier = vec![key.to_string()];

        while let Some(parent) = frontier.pop() {
            // The two identities a run can execute under. Both appear as a
            // child's `parent_key`.
            let owners = [format!("spawn:{parent}"), format!("subagent:{parent}")];
            for session in self.sessions.values() {
                let Some(child_parent) = session.parent_key.as_deref() else {
                    continue;
                };
                if !owners.iter().any(|o| o == child_parent) {
                    continue;
                }
                // `seen` guards against a cycle; keys are unique so one should
                // not exist, but an unbounded walk here would hang the caller.
                if seen.insert(session.key.clone()) {
                    found.push(session.key.clone());
                    frontier.push(session.key.clone());
                }
            }
        }
        found
    }

    /// Get a session by key.
    pub fn get(&self, key: &str) -> Option<&Session> {
        self.sessions.get(key)
    }

    /// Get a session by label.
    pub fn get_by_label(&self, label: &str) -> Option<&Session> {
        self.labels.get(label).and_then(|k| self.sessions.get(k))
    }

    /// Get a mutable session by key.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Session> {
        self.sessions.get_mut(key)
    }

    /// List sessions with optional filters.
    pub fn list(
        &self,
        kinds: Option<&[SessionKind]>,
        active_only: bool,
        limit: usize,
    ) -> Vec<&Session> {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|s| {
                let kind_match = kinds.map(|ks| ks.contains(&s.kind)).unwrap_or(true);
                let active_match = !active_only || s.status == SessionStatus::Active;
                kind_match && active_match
            })
            .collect();

        // Sort by created time descending
        sessions.sort_by_key(|s| std::cmp::Reverse(s.created_ms));
        sessions.truncate(limit);
        sessions
    }

    /// Get message history for a session.
    pub fn history(
        &self,
        key: &str,
        limit: usize,
        include_tools: bool,
    ) -> Option<Vec<&SessionMessage>> {
        self.sessions.get(key).map(|s| {
            s.messages
                .iter()
                .filter(|m| include_tools || m.role != "tool")
                .rev()
                .take(limit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect()
        })
    }

    /// Send a message to a session.
    pub fn send_message(&mut self, key: &str, message: &str) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or_else(|| SessionError::NotFound(key.to_string()))?;

        if session.status != SessionStatus::Active {
            return Err(SessionError::NotActive(session.status.clone()));
        }

        session.add_message("user", message);
        Ok(())
    }

    /// Complete a session.
    pub fn complete_session(&mut self, key: &str) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or_else(|| SessionError::NotFound(key.to_string()))?;
        session.complete();
        Ok(())
    }

    /// Mark a session stopped at the user's request.
    ///
    /// Distinct from [`complete_session`](Self::complete_session): a stopped
    /// run did not finish its task, and reporting it as Completed would tell
    /// the parent agent the work was done.
    pub fn stop_session(&mut self, key: &str) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or_else(|| SessionError::NotFound(key.to_string()))?;
        session.stop();
        Ok(())
    }
}

// ── Running spawned sessions ────────────────────────────────────────────────

/// Handle on a spawned session that is actually executing.
///
/// The session *record* says a run exists; this says it is still running and
/// gives the means to stop it. Kept apart from [`Session`] because a record
/// outlives its run — history stays readable long after the task is gone —
/// and because the handle is not serializable.
struct RunningSession {
    handle: tokio::task::JoinHandle<()>,
    /// Asked first, so a run can unwind and record its own outcome rather
    /// than being torn down mid-write.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// How long a stopped run has to notice the cancel flag and unwind before it
/// is aborted outright.
///
/// Long enough for a run between rounds to bail and write its own ending,
/// short enough that a user who asked for a stop is not left watching.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Registry of spawned sessions that are still executing.
#[derive(Default)]
pub struct RunningSessions {
    runs: HashMap<SessionKey, RunningSession>,
}

impl RunningSessions {
    /// Record a newly spawned run.
    pub fn register(
        &mut self,
        key: SessionKey,
        handle: tokio::task::JoinHandle<()>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        if let Some(old) = self.runs.insert(key, RunningSession { handle, cancel }) {
            // Should not happen — keys are unique — but a leaked task would
            // keep making model calls forever.
            old.handle.abort();
        }
    }

    /// Whether this session is still executing.
    pub fn is_running(&self, key: &str) -> bool {
        self.runs.get(key).is_some_and(|r| !r.handle.is_finished())
    }

    /// Stop a running session. Returns whether there was one to stop.
    ///
    /// The cancel flag is raised first and the abort follows only after
    /// [`STOP_GRACE`], so a run that checks the flag between rounds can finish
    /// the write it is in the middle of and record its own ending. Aborting in
    /// the same breath as setting the flag — as this used to — meant the run
    /// could never observe it: a `tokio` abort lands at the next await point,
    /// and every flag check sits after one. The grace is what makes the
    /// cooperative path reachable at all.
    ///
    /// The abort still has to happen. A run parked inside a five-minute model
    /// call would otherwise keep spending tokens until it returned.
    pub fn stop(&mut self, key: &str) -> bool {
        let Some(run) = self.runs.remove(key) else {
            return false;
        };
        run.cancel.store(true, std::sync::atomic::Ordering::Relaxed);

        // Needs a reactor to time the grace period. `sessions_kill` is a
        // synchronous tool and runs on the blocking pool, which still carries
        // the runtime context; with no runtime at all (a plain unit test)
        // there is nothing to wait with, so stop hard.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    tokio::time::sleep(STOP_GRACE).await;
                    // A no-op if the run already unwound on its own, which is
                    // the outcome the grace period is here to allow.
                    run.handle.abort();
                });
            }
            Err(_) => run.handle.abort(),
        }
        true
    }

    /// Drop entries whose task has ended, so the registry does not grow for
    /// the life of the process.
    pub fn reap_finished(&mut self) {
        self.runs.retain(|_, r| !r.handle.is_finished());
    }

    /// Keys of every run still executing.
    pub fn running_keys(&self) -> Vec<SessionKey> {
        self.runs
            .iter()
            .filter(|(_, r)| !r.handle.is_finished())
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// Global registry of executing spawned sessions.
pub fn running_sessions() -> &'static Arc<Mutex<RunningSessions>> {
    static RUNNING: OnceLock<Arc<Mutex<RunningSessions>>> = OnceLock::new();
    RUNNING.get_or_init(|| Arc::new(Mutex::new(RunningSessions::default())))
}

// ── The thing that actually runs a spawned session ──────────────────────────

/// What a spawned session needs in order to run.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    /// Key of the session record already created for this run.
    pub session_key: SessionKey,
    /// Agent whose workspace and system prompt the run adopts.
    pub agent_id: String,
    /// The work to do.
    pub task: String,
    /// Model to use instead of the gateway's current one.
    pub model: Option<String>,
    /// Stop the run after this many seconds if it is still working.
    pub timeout_secs: Option<u64>,
}

/// Starts the background work behind `sessions_spawn`.
///
/// Core owns the session *records*; it has no model client, no provider
/// credentials, and no system prompt of its own, so it cannot run an agent
/// turn. The gateway installs an implementation of this at startup and every
/// caller of the tool — client dispatch, a trigger fire, a cron job, another
/// sub-agent — gets real execution through the same door.
pub trait SpawnRunner: Send + Sync {
    /// Start `req` in the background and return immediately.
    ///
    /// `Err` means nothing was started, and the caller is responsible for
    /// disposing of the session record.
    fn start(&self, req: SpawnRequest) -> Result<(), String>;
}

static SPAWN_RUNNER: OnceLock<Mutex<Option<Arc<dyn SpawnRunner>>>> = OnceLock::new();

fn spawn_runner_slot() -> &'static Mutex<Option<Arc<dyn SpawnRunner>>> {
    SPAWN_RUNNER.get_or_init(|| Mutex::new(None))
}

/// Install the runner that gives `sessions_spawn` its teeth.
pub fn set_spawn_runner(runner: Arc<dyn SpawnRunner>) {
    if let Ok(mut slot) = spawn_runner_slot().lock() {
        *slot = Some(runner);
    }
}

/// The installed runner, if a gateway is hosting these tools.
pub fn spawn_runner() -> Option<Arc<dyn SpawnRunner>> {
    spawn_runner_slot().lock().ok().and_then(|s| s.clone())
}

/// Remove the installed runner, restoring the no-gateway behaviour.
#[cfg(test)]
pub(crate) fn clear_spawn_runner() {
    if let Ok(mut slot) = spawn_runner_slot().lock() {
        *slot = None;
    }
}

/// Serializes tests that install or clear the process-wide spawn runner:
/// without it, one test's runner answers another test's spawn.
#[cfg(test)]
pub(crate) static SPAWN_RUNNER_TEST_LOCK: Mutex<()> = Mutex::new(());

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe session manager.
pub type SharedSessionManager = Arc<Mutex<SessionManager>>;

/// Global session manager instance.
static SESSION_MANAGER: OnceLock<SharedSessionManager> = OnceLock::new();

/// Get the global session manager.
pub fn session_manager() -> &'static SharedSessionManager {
    SESSION_MANAGER.get_or_init(|| Arc::new(Mutex::new(SessionManager::new())))
}

/// Spawn result returned to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnResult {
    pub status: String,
    pub run_id: String,
    pub session_key: SessionKey,
    pub message: String,
}

/// Get current time in milliseconds.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_registered_run_is_stoppable() {
        let mut runs = RunningSessions::default();
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watched = flag.clone();
        let handle = tokio::spawn(async move {
            // Long enough that the test does the stopping, not the clock.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            watched.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        runs.register("k".to_string(), handle, flag.clone());
        assert!(runs.is_running("k"));

        assert!(runs.stop("k"), "a registered run must be stoppable");
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
        assert!(!runs.is_running("k"));
        // Stopping twice is not an error, just a no-op.
        assert!(!runs.stop("k"));
    }

    #[tokio::test]
    async fn finished_runs_are_reaped() {
        let mut runs = RunningSessions::default();
        let handle = tokio::spawn(async {});
        runs.register(
            "done".to_string(),
            handle,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        // Let the task complete before reaping.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;

        runs.reap_finished();
        assert!(runs.running_keys().is_empty());
    }

    #[test]
    fn a_stopped_session_is_not_reported_as_completed() {
        let mut manager = SessionManager::new();
        let key = manager.spawn_subagent("main", "half-done work", None, None);
        manager.stop_session(&key).unwrap();

        let session = manager.get(&key).unwrap();
        assert_eq!(session.status, SessionStatus::Stopped);
        assert!(session.finished_ms.is_some());
    }

    #[test]
    fn test_session_creation() {
        let session = Session::new_main("main");
        assert_eq!(session.key, "agent:main:main");
        assert_eq!(session.kind, SessionKind::Main);
        assert_eq!(session.status, SessionStatus::Active);
    }

    #[test]
    fn test_subagent_spawn() {
        let mut manager = SessionManager::new();
        let key =
            manager.spawn_subagent("main", "Research task", Some("research".to_string()), None);

        assert!(key.contains("subagent"));

        let session = manager.get(&key).unwrap();
        assert_eq!(session.kind, SessionKind::Subagent);
        assert_eq!(session.task, Some("Research task".to_string()));
        assert_eq!(session.label, Some("research".to_string()));

        // Should be findable by label
        let by_label = manager.get_by_label("research").unwrap();
        assert_eq!(by_label.key, key);
    }

    #[test]
    fn test_message_history() {
        let mut manager = SessionManager::new();
        let key = manager.spawn_subagent("main", "Test", None, None);

        manager.send_message(&key, "Hello").unwrap();

        let session = manager.get_mut(&key).unwrap();
        session.add_message("assistant", "Hi there!");

        let history = manager.history(&key, 10, false).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "Hello");
        assert_eq!(history[1].content, "Hi there!");
    }

    #[test]
    fn test_session_listing() {
        let mut manager = SessionManager::new();
        manager.get_or_create_main("main");
        manager.spawn_subagent("main", "Task 1", None, None);
        manager.spawn_subagent("main", "Task 2", None, None);

        let all = manager.list(None, false, 10);
        assert_eq!(all.len(), 3);

        let subagents = manager.list(Some(&[SessionKind::Subagent]), false, 10);
        assert_eq!(subagents.len(), 2);
    }

    #[test]
    fn test_subagent_appears_in_active_list() {
        // This test verifies that spawned subagents show up when listing active sessions
        // which is what the gateway uses to populate the sidebar
        let mut manager = SessionManager::new();

        // Spawn a subagent
        let key = manager.spawn_subagent("main", "Test task", Some("test".to_string()), None);

        // Verify it appears in active-only list (what sidebar uses)
        let active = manager.list(Some(&[SessionKind::Subagent]), true, 10);
        assert_eq!(active.len(), 1, "Subagent should appear in active list");
        assert_eq!(active[0].key, key);
        assert_eq!(active[0].status, SessionStatus::Active);
        assert_eq!(active[0].label, Some("test".to_string()));
    }
}
