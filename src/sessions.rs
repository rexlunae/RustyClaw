//! Session management for RustyClaw multi-agent support.
//!
//! Provides tools for spawning sub-agents, sending messages between sessions,
//! and managing session state.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
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
    pub fn new_subagent(agent_id: &str, task: &str, label: Option<String>, parent_key: Option<SessionKey>) -> Self {
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

    /// Get runtime in seconds.
    pub fn runtime_secs(&self) -> u64 {
        let end = self.finished_ms.unwrap_or_else(now_millis);
        (end - self.created_ms) / 1000
    }
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
    pub fn list(&self, kinds: Option<&[SessionKind]>, active_only: bool, limit: usize) -> Vec<&Session> {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|s| {
                let kind_match = kinds
                    .map(|ks| ks.contains(&s.kind))
                    .unwrap_or(true);
                let active_match = !active_only || s.status == SessionStatus::Active;
                kind_match && active_match
            })
            .collect();

        // Sort by created time descending
        sessions.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
        sessions.truncate(limit);
        sessions
    }

    /// Get message history for a session.
    pub fn history(&self, key: &str, limit: usize, include_tools: bool) -> Option<Vec<&SessionMessage>> {
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
    pub fn send_message(&mut self, key: &str, message: &str) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or_else(|| format!("Session not found: {}", key))?;

        if session.status != SessionStatus::Active {
            return Err(format!("Session is not active: {:?}", session.status));
        }

        session.add_message("user", message);
        Ok(())
    }

    /// Complete a session.
    pub fn complete_session(&mut self, key: &str) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or_else(|| format!("Session not found: {}", key))?;
        session.complete();
        Ok(())
    }
}

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

const DEFAULT_ARCHIVE_RETENTION_DAYS: u64 = 30;

fn archive_file(workspace_dir: &Path) -> PathBuf {
    workspace_dir
        .join(".rustyclaw")
        .join("sessions")
        .join("archive.jsonl")
}

fn load_archive(workspace_dir: &Path) -> Result<Vec<Session>, String> {
    let path = archive_file(workspace_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read archive file {}: {}", path.display(), e))?;

    let mut out = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(session) = serde_json::from_str::<Session>(line) {
            out.push(session);
        }
    }
    Ok(out)
}

fn save_archive(workspace_dir: &Path, sessions: &[Session]) -> Result<(), String> {
    let path = archive_file(workspace_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create archive directory {}: {}", parent.display(), e))?;
    }

    let mut file = std::fs::File::create(&path)
        .map_err(|e| format!("Failed to open archive file {}: {}", path.display(), e))?;
    for session in sessions {
        let line = serde_json::to_string(session)
            .map_err(|e| format!("Failed to serialize archived session: {}", e))?;
        writeln!(file, "{}", line)
            .map_err(|e| format!("Failed to write archive file {}: {}", path.display(), e))?;
    }
    Ok(())
}

fn retention_cutoff_ms(retention_days: u64) -> u64 {
    let days = if retention_days == 0 {
        DEFAULT_ARCHIVE_RETENTION_DAYS
    } else {
        retention_days
    };
    let now = now_millis();
    let window_ms = days
        .saturating_mul(24)
        .saturating_mul(60)
        .saturating_mul(60)
        .saturating_mul(1000);
    now.saturating_sub(window_ms)
}

/// Archive a single session by key.
///
/// If the session is still active, it is marked as stopped and finished
/// before being archived.
pub fn archive_session(
    manager: &mut SessionManager,
    key: &str,
    workspace_dir: &Path,
) -> Result<(), String> {
    let mut session = manager
        .sessions
        .remove(key)
        .ok_or_else(|| format!("Session not found: {}", key))?;

    if session.status == SessionStatus::Active {
        session.status = SessionStatus::Stopped;
    }
    if session.finished_ms.is_none() {
        session.finished_ms = Some(now_millis());
    }

    if let Some(label) = &session.label {
        manager.labels.remove(label);
    }

    let mut archived = load_archive(workspace_dir)?;
    archived.retain(|s| s.key != session.key);
    archived.push(session);
    save_archive(workspace_dir, &archived)
}

/// Archive all non-active sessions and return the number archived.
pub fn archive_completed_sessions(
    manager: &mut SessionManager,
    workspace_dir: &Path,
) -> Result<usize, String> {
    let keys: Vec<String> = manager
        .sessions
        .values()
        .filter(|s| s.status != SessionStatus::Active)
        .map(|s| s.key.clone())
        .collect();

    let mut archived_count = 0usize;
    for key in &keys {
        archive_session(manager, key, workspace_dir)?;
        archived_count += 1;
    }
    Ok(archived_count)
}

/// List archived sessions (newest first), limited by `limit`.
pub fn list_archived_sessions(workspace_dir: &Path, limit: usize) -> Result<Vec<Session>, String> {
    let mut archived = load_archive(workspace_dir)?;
    archived.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
    archived.truncate(limit);
    Ok(archived)
}

/// Get one archived session by key.
pub fn get_archived_session(workspace_dir: &Path, key: &str) -> Result<Option<Session>, String> {
    let archived = load_archive(workspace_dir)?;
    Ok(archived.into_iter().find(|s| s.key == key))
}

/// Prune archived sessions using a retention window in days.
/// Returns the number of deleted sessions.
pub fn prune_archived_sessions(workspace_dir: &Path, retention_days: u64) -> Result<usize, String> {
    let cutoff = retention_cutoff_ms(retention_days);
    let mut archived = load_archive(workspace_dir)?;
    let original_len = archived.len();

    archived.retain(|s| {
        let ts = s.finished_ms.unwrap_or(s.created_ms);
        ts >= cutoff
    });

    let deleted = original_len.saturating_sub(archived.len());
    if deleted > 0 {
        save_archive(workspace_dir, &archived)?;
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
        let key = manager.spawn_subagent("main", "Research task", Some("research".to_string()), None);

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
    fn test_archive_single_session() {
        let mut manager = SessionManager::new();
        let key = manager.spawn_subagent("main", "Task", Some("arch".to_string()), None);
        let ws = TempDir::new().unwrap();

        archive_session(&mut manager, &key, ws.path()).unwrap();
        assert!(manager.get(&key).is_none());

        let archived = list_archived_sessions(ws.path(), 10).unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].key, key);
    }

    #[test]
    fn test_archive_pruning() {
        let ws = TempDir::new().unwrap();
        let old = Session {
            key: "agent:main:subagent:old".to_string(),
            agent_id: "main".to_string(),
            kind: SessionKind::Subagent,
            status: SessionStatus::Completed,
            label: None,
            task: Some("old".to_string()),
            created_ms: 1,
            finished_ms: Some(1),
            messages: Vec::new(),
            run_id: None,
            parent_key: None,
        };
        save_archive(ws.path(), &[old]).unwrap();
        let deleted = prune_archived_sessions(ws.path(), 1).unwrap();
        assert_eq!(deleted, 1);
    }
}
