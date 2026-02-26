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
        sessions.sort_by(|a, b| b.created_ms.cmp(&a.created_ms));
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
