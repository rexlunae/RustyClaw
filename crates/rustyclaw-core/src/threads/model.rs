//! Thread model — core types for the unified thread system.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::SystemTime;

/// Unique identifier for a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(pub u64);

impl ThreadId {
    /// Generate a new unique thread ID.
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for ThreadId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// What kind of thread this is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadKind {
    /// User-interactive chat thread (persistent, has messages)
    Chat,
    
    /// Spawned sub-agent (ephemeral, autonomous, may return result)
    SubAgent {
        /// Agent ID that's running this
        agent_id: String,
        /// The task/prompt given to the sub-agent
        task: String,
    },
    
    /// Long-running background work (persistent, autonomous)
    Background {
        /// What this background thread is monitoring/doing
        purpose: String,
    },
    
    /// One-shot task (ephemeral, returns result and exits)
    Task {
        /// What this task is doing
        action: String,
    },
}

impl ThreadKind {
    /// Display name for the kind.
    pub fn display_name(&self) -> &str {
        match self {
            Self::Chat => "Chat",
            Self::SubAgent { .. } => "Sub-agent",
            Self::Background { .. } => "Background",
            Self::Task { .. } => "Task",
        }
    }
    
    /// Icon for sidebar display.
    pub fn icon(&self) -> &str {
        match self {
            Self::Chat => "💬",
            Self::SubAgent { .. } => "🤖",
            Self::Background { .. } => "⚙️",
            Self::Task { .. } => "📋",
        }
    }
    
    /// Is this an interactive thread?
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::Chat)
    }
    
    /// Is this ephemeral (auto-cleanup when done)?
    pub fn is_ephemeral(&self) -> bool {
        matches!(self, Self::SubAgent { .. } | Self::Task { .. })
    }
}

/// Thread status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThreadStatus {
    /// Thread is active/running
    Active,
    
    /// Thread is running but backgrounded (not user-focused)
    Running {
        /// Progress indicator (0.0 - 1.0) if known
        progress: Option<f32>,
        /// Current status message
        message: Option<String>,
    },
    
    /// Thread is waiting for user input
    WaitingForInput {
        prompt: String,
    },
    
    /// Thread is paused
    Paused,
    
    /// Thread completed successfully
    Completed {
        /// Summary of what was accomplished
        summary: Option<String>,
    },
    
    /// Thread failed
    Failed {
        error: String,
    },
    
    /// Thread was cancelled
    Cancelled,
}

impl ThreadStatus {
    /// Is this a terminal state?
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled)
    }
    
    /// Is the thread actively running?
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Active | Self::Running { .. })
    }
    
    /// Status icon for display.
    pub fn icon(&self) -> &str {
        match self {
            Self::Active => "▶",
            Self::Running { .. } => "▶",
            Self::WaitingForInput { .. } => "⏸",
            Self::Paused => "⏸",
            Self::Completed { .. } => "✓",
            Self::Failed { .. } => "✗",
            Self::Cancelled => "⊘",
        }
    }
    
    /// Short display string.
    pub fn display(&self) -> String {
        match self {
            Self::Active => "Active".to_string(),
            Self::Running { message, .. } => {
                message.clone().unwrap_or_else(|| "Running".to_string())
            }
            Self::WaitingForInput { prompt } => format!("Waiting: {}", prompt),
            Self::Paused => "Paused".to_string(),
            Self::Completed { summary } => {
                summary.clone().unwrap_or_else(|| "Completed".to_string())
            }
            Self::Failed { error } => format!("Failed: {}", error),
            Self::Cancelled => "Cancelled".to_string(),
        }
    }
}

/// A message in a thread's conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: SystemTime,
}

/// Message role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// An agent thread — the unified representation of all concurrent work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentThread {
    /// Unique identifier
    pub id: ThreadId,
    
    /// What kind of thread this is
    pub kind: ThreadKind,
    
    /// User-visible label
    pub label: String,
    
    /// Agent-settable description of current activity
    pub description: Option<String>,
    
    /// Current status
    pub status: ThreadStatus,
    
    /// Parent thread that spawned this (if any)
    pub parent_id: Option<ThreadId>,
    
    /// When the thread was created
    pub created_at: SystemTime,
    
    /// When the thread last had activity
    pub last_activity: SystemTime,
    
    /// Is this the foreground (user-focused) thread?
    pub is_foreground: bool,
    
    /// Conversation history (for interactive threads)
    pub messages: VecDeque<ThreadMessage>,
    
    /// Compacted summary of older messages
    pub compact_summary: Option<String>,
    
    /// Result value (for task/sub-agent threads)
    pub result: Option<String>,
    
    /// Should this thread's context be shared with parent?
    pub share_context: bool,
}

impl AgentThread {
    /// Backwards-compatible alias for id (used by gateway code).
    pub fn task_id(&self) -> ThreadId {
        self.id
    }
    
    /// Create a new chat thread.
    pub fn new_chat(label: impl Into<String>) -> Self {
        let now = SystemTime::now();
        Self {
            id: ThreadId::new(),
            kind: ThreadKind::Chat,
            label: label.into(),
            description: None,
            status: ThreadStatus::Active,
            parent_id: None,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            result: None,
            share_context: true,
        }
    }
    
    /// Create a new sub-agent thread.
    pub fn new_subagent(
        label: impl Into<String>,
        agent_id: impl Into<String>,
        task: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: ThreadId::new(),
            kind: ThreadKind::SubAgent {
                agent_id: agent_id.into(),
                task: task.into(),
            },
            label: label.into(),
            description: None,
            status: ThreadStatus::Running { progress: None, message: None },
            parent_id,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            result: None,
            share_context: true,
        }
    }
    
    /// Create a new background thread.
    pub fn new_background(
        label: impl Into<String>,
        purpose: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: ThreadId::new(),
            kind: ThreadKind::Background {
                purpose: purpose.into(),
            },
            label: label.into(),
            description: None,
            status: ThreadStatus::Running { progress: None, message: None },
            parent_id,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            result: None,
            share_context: false,
        }
    }
    
    /// Create a new task thread.
    pub fn new_task(
        label: impl Into<String>,
        action: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> Self {
        let now = SystemTime::now();
        Self {
            id: ThreadId::new(),
            kind: ThreadKind::Task {
                action: action.into(),
            },
            label: label.into(),
            description: None,
            status: ThreadStatus::Running { progress: None, message: None },
            parent_id,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            result: None,
            share_context: true,
        }
    }
    
    /// Update the description.
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = Some(description.into());
        self.last_activity = SystemTime::now();
    }
    
    /// Update the status.
    pub fn set_status(&mut self, status: ThreadStatus) {
        self.status = status;
        self.last_activity = SystemTime::now();
    }
    
    /// Mark as completed with optional result.
    pub fn complete(&mut self, summary: Option<String>, result: Option<String>) {
        self.status = ThreadStatus::Completed { summary };
        self.result = result;
        self.last_activity = SystemTime::now();
    }
    
    /// Mark as failed.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.status = ThreadStatus::Failed { error: error.into() };
        self.last_activity = SystemTime::now();
    }
    
    /// Add a message to the conversation history.
    pub fn add_message(&mut self, role: MessageRole, content: impl Into<String>) {
        self.messages.push_back(ThreadMessage {
            role,
            content: content.into(),
            timestamp: SystemTime::now(),
        });
        self.last_activity = SystemTime::now();
    }
    
    /// Get message count.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
    
    /// Generate a prompt for compacting this thread's conversation.
    pub fn compaction_prompt(&self) -> String {
        let mut prompt = String::from(
            "Summarize the following conversation in 2-3 sentences, \
             capturing the key topics, decisions, and any pending items:\n\n",
        );
        
        for msg in &self.messages {
            let role = match msg.role {
                MessageRole::User => "User",
                MessageRole::Assistant => "Assistant",
                MessageRole::System => "System",
                MessageRole::Tool => "Tool",
            };
            prompt.push_str(&format!("{}: {}\n", role, msg.content));
        }
        
        prompt
    }
    
    /// Apply a compaction summary, keeping only recent messages.
    pub fn apply_compaction(&mut self, summary: String) {
        // Keep the last 3 messages
        const KEEP_RECENT: usize = 3;
        
        while self.messages.len() > KEEP_RECENT {
            self.messages.pop_front();
        }
        
        self.compact_summary = Some(summary);
        self.last_activity = SystemTime::now();
    }
    
    /// Build context string for this thread (for system prompt injection).
    pub fn build_context(&self) -> String {
        let mut ctx = String::new();
        
        // Include compact summary if present
        if let Some(summary) = &self.compact_summary {
            ctx.push_str("## Previous Context\n");
            ctx.push_str(summary);
            ctx.push_str("\n\n");
        }
        
        // Include recent messages
        if !self.messages.is_empty() {
            ctx.push_str("## Recent Messages\n");
            for msg in &self.messages {
                let role = match msg.role {
                    MessageRole::User => "User",
                    MessageRole::Assistant => "Assistant",
                    MessageRole::System => "System",
                    MessageRole::Tool => "Tool",
                };
                ctx.push_str(&format!("{}: {}\n", role, msg.content));
            }
        }
        
        ctx
    }
    
    /// Get info for sidebar display.
    pub fn to_info(&self) -> ThreadInfo {
        ThreadInfo {
            id: self.id,
            kind: self.kind.display_name().to_string(),
            icon: self.kind.icon().to_string(),
            label: self.label.clone(),
            description: self.description.clone(),
            status: self.status.display(),
            status_icon: self.status.icon().to_string(),
            is_foreground: self.is_foreground,
            is_interactive: self.kind.is_interactive(),
            message_count: self.messages.len(),
            has_summary: self.compact_summary.is_some(),
            has_result: self.result.is_some(),
        }
    }
}

/// Summary info for sidebar display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: ThreadId,
    pub kind: String,
    pub icon: String,
    pub label: String,
    pub description: Option<String>,
    pub status: String,
    pub status_icon: String,
    pub is_foreground: bool,
    pub is_interactive: bool,
    pub message_count: usize,
    pub has_summary: bool,
    pub has_result: bool,
}
