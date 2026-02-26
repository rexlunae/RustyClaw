//! Task threads — independent conversation contexts for multi-tasking.
//!
//! Each thread has its own conversation history that compacts when the user
//! switches away, with summaries merged into global context.

use super::model::{Task, TaskId, TaskKind, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;

/// A message in a task thread's conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: SystemTime,
    /// Tool calls/results are stored separately for compaction
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_interactions: Vec<ToolInteraction>,
}

/// Message roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool call and its result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInteraction {
    pub tool_name: String,
    pub arguments: String,
    pub result: Option<String>,
    pub success: bool,
}

/// A task thread — an independent conversation context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskThread {
    /// Associated task ID
    pub task_id: TaskId,

    /// User-provided label for this thread
    pub label: String,

    /// Full conversation history (compacted when switching away)
    pub messages: Vec<ThreadMessage>,

    /// Compact summary of this thread's work (generated on switch-away)
    pub compact_summary: Option<String>,

    /// Whether this thread is the foreground thread
    pub is_foreground: bool,

    /// When the thread was created
    pub created_at: SystemTime,

    /// Last activity time
    pub last_activity: SystemTime,

    /// Model to use for this thread (None = inherit from session)
    pub model: Option<String>,

    /// Whether to include this thread's summary in other threads' context
    pub share_context: bool,
}

impl TaskThread {
    /// Create a new thread.
    pub fn new(label: impl Into<String>) -> Self {
        let now = SystemTime::now();
        Self {
            task_id: TaskId::new(),
            label: label.into(),
            messages: Vec::new(),
            compact_summary: None,
            is_foreground: true,
            created_at: now,
            last_activity: now,
            model: None,
            share_context: true,
        }
    }

    /// Add a message to the thread.
    pub fn add_message(&mut self, role: MessageRole, content: impl Into<String>) {
        self.messages.push(ThreadMessage {
            role,
            content: content.into(),
            timestamp: SystemTime::now(),
            tool_interactions: Vec::new(),
        });
        self.last_activity = SystemTime::now();
    }

    /// Get the message count.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Estimate token count (rough approximation: 4 chars ≈ 1 token).
    pub fn estimated_tokens(&self) -> usize {
        self.messages.iter().map(|m| m.content.len() / 4).sum()
    }

    /// Generate a compaction prompt for this thread.
    pub fn compaction_prompt(&self) -> String {
        let mut history = String::new();
        for msg in &self.messages {
            let role = match msg.role {
                MessageRole::System => "System",
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::Tool => "Tool",
            };
            history.push_str(&format!("{}: {}\n\n", role, msg.content));
        }

        format!(
            r#"Summarize the following conversation thread titled "{}".

Focus on:
- Key decisions made
- Important information discovered
- Current state of the task
- Any pending actions or blockers

Keep the summary concise but complete enough for context continuity.

---

{}

---

Summary:"#,
            self.label, history
        )
    }

    /// Apply a compact summary and clear old messages.
    pub fn apply_compaction(&mut self, summary: String) {
        self.compact_summary = Some(summary);
        // Keep only the most recent messages (configurable later)
        let keep_recent = 3;
        if self.messages.len() > keep_recent {
            self.messages = self.messages.split_off(self.messages.len() - keep_recent);
        }
    }

    /// Build context for this thread including compact summary.
    pub fn build_context(&self) -> String {
        let mut ctx = String::new();

        if let Some(ref summary) = self.compact_summary {
            ctx.push_str(&format!(
                "## Thread Summary: {}\n\n{}\n\n",
                self.label, summary
            ));
        }

        ctx.push_str("## Recent Messages\n\n");
        for msg in &self.messages {
            let role = match msg.role {
                MessageRole::System => "System",
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::Tool => "Tool",
            };
            ctx.push_str(&format!("**{}:** {}\n\n", role, msg.content));
        }

        ctx
    }
}

/// Manager for task threads within a session.
#[derive(Debug, Default)]
pub struct ThreadManager {
    /// All threads for this session
    threads: HashMap<TaskId, TaskThread>,

    /// Currently foreground thread
    foreground_id: Option<TaskId>,
}

impl ThreadManager {
    /// Create a new thread manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new thread and make it foreground.
    pub fn create_thread(&mut self, label: impl Into<String>) -> TaskId {
        let thread = TaskThread::new(label);
        let id = thread.task_id;

        // Background the current foreground
        if let Some(fg_id) = self.foreground_id {
            if let Some(fg) = self.threads.get_mut(&fg_id) {
                fg.is_foreground = false;
            }
        }

        self.threads.insert(id, thread);
        self.foreground_id = Some(id);
        id
    }

    /// Get the foreground thread.
    pub fn foreground(&self) -> Option<&TaskThread> {
        self.foreground_id.and_then(|id| self.threads.get(&id))
    }

    /// Get mutable foreground thread.
    pub fn foreground_mut(&mut self) -> Option<&mut TaskThread> {
        self.foreground_id.and_then(|id| self.threads.get_mut(&id))
    }

    /// Switch to a different thread (returns the old foreground if any).
    pub fn switch_to(&mut self, id: TaskId) -> Option<TaskId> {
        if !self.threads.contains_key(&id) {
            return None;
        }

        let old_fg = self.foreground_id;

        // Background the old foreground
        if let Some(fg_id) = old_fg {
            if fg_id != id {
                if let Some(fg) = self.threads.get_mut(&fg_id) {
                    fg.is_foreground = false;
                }
            }
        }

        // Foreground the new one
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.is_foreground = true;
            thread.last_activity = SystemTime::now();
        }

        self.foreground_id = Some(id);
        old_fg
    }

    /// Get all threads.
    pub fn all_threads(&self) -> Vec<&TaskThread> {
        self.threads.values().collect()
    }

    /// Get a thread by ID.
    pub fn get(&self, id: TaskId) -> Option<&TaskThread> {
        self.threads.get(&id)
    }

    /// Get a thread by ID mutably.
    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut TaskThread> {
        self.threads.get_mut(&id)
    }

    /// Remove a thread.
    pub fn remove(&mut self, id: TaskId) -> Option<TaskThread> {
        if self.foreground_id == Some(id) {
            self.foreground_id = None;
        }
        self.threads.remove(&id)
    }

    /// Rename a thread.
    pub fn rename(&mut self, id: TaskId, new_label: impl Into<String>) -> bool {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.label = new_label.into();
            true
        } else {
            false
        }
    }

    /// Build combined context from all threads with share_context=true.
    pub fn build_global_context(&self) -> String {
        let mut ctx = String::new();

        for thread in self.threads.values() {
            if thread.share_context && !thread.is_foreground {
                if let Some(ref summary) = thread.compact_summary {
                    ctx.push_str(&format!(
                        "## Background Task: {}\n\n{}\n\n---\n\n",
                        thread.label, summary
                    ));
                }
            }
        }

        ctx
    }

    /// Count active threads.
    pub fn count(&self) -> usize {
        self.threads.len()
    }

    /// List thread info for display.
    pub fn list_info(&self) -> Vec<ThreadInfo> {
        self.threads
            .values()
            .map(|t| ThreadInfo {
                id: t.task_id,
                label: t.label.clone(),
                is_foreground: t.is_foreground,
                message_count: t.messages.len(),
                has_summary: t.compact_summary.is_some(),
            })
            .collect()
    }
}

/// Summary info about a thread for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: TaskId,
    pub label: String,
    pub is_foreground: bool,
    pub message_count: usize,
    pub has_summary: bool,
}

/// Shared thread manager.
pub type SharedThreadManager = Arc<RwLock<ThreadManager>>;

// ── Persistence ─────────────────────────────────────────────────────────────

/// Serializable state for thread persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadManagerState {
    pub threads: Vec<TaskThread>,
    pub foreground_id: Option<u64>,
}

impl ThreadManager {
    /// Save thread state to a file.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let state = ThreadManagerState {
            threads: self.threads.values().cloned().collect(),
            foreground_id: self.foreground_id.map(|id| id.0),
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// Load thread state from a file.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state: ThreadManagerState = serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut threads = HashMap::new();
        for thread in state.threads {
            threads.insert(thread.task_id, thread);
        }

        Ok(Self {
            threads,
            foreground_id: state.foreground_id.map(TaskId),
        })
    }

    /// Load from file or create new with default thread.
    pub fn load_or_default(path: &std::path::Path) -> Self {
        match Self::load_from_file(path) {
            Ok(mgr) if !mgr.threads.is_empty() => mgr,
            _ => {
                let mut mgr = Self::new();
                mgr.create_thread("Main");
                mgr
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_creation() {
        let thread = TaskThread::new("Test task");
        assert_eq!(thread.label, "Test task");
        assert!(thread.messages.is_empty());
        assert!(thread.is_foreground);
    }

    #[test]
    fn test_thread_manager() {
        let mut mgr = ThreadManager::new();

        let id1 = mgr.create_thread("Task 1");
        assert!(mgr.foreground().is_some());
        assert_eq!(mgr.foreground().unwrap().label, "Task 1");

        let id2 = mgr.create_thread("Task 2");
        assert_eq!(mgr.foreground().unwrap().label, "Task 2");
        assert!(!mgr.get(id1).unwrap().is_foreground);

        mgr.switch_to(id1);
        assert_eq!(mgr.foreground().unwrap().label, "Task 1");
    }

    #[test]
    fn test_message_adding() {
        let mut thread = TaskThread::new("Test");
        thread.add_message(MessageRole::User, "Hello");
        thread.add_message(MessageRole::Assistant, "Hi there!");
        assert_eq!(thread.message_count(), 2);
    }
}
