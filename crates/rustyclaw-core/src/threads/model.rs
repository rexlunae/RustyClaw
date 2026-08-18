//! Thread model — core types for the unified thread system.

use crate::projects::ProjectId;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

/// Unique identifier for a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(pub u64);

/// Source of freshly-minted thread ids. Process-global, so it has to be
/// reconciled with ids restored from disk — see [`ThreadId::reserve_above`].
static THREAD_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl ThreadId {
    /// Generate a new unique thread ID.
    pub fn new() -> Self {
        use std::sync::atomic::Ordering;
        Self(THREAD_ID_COUNTER.fetch_add(1, Ordering::SeqCst))
    }

    /// Guarantee that every id minted from here on is greater than `id`.
    ///
    /// The counter lives in the process, but thread ids outlive it: they are
    /// persisted to `threads.json` and restored on the next run. Without this
    /// a restarted gateway hands out `1, 2, 3…` all over again, and the first
    /// thread the user creates silently *replaces* the restored thread that
    /// already owns that id — taking its whole message history with it.
    /// Loaders must call this with the highest id they restored.
    pub fn reserve_above(id: u64) {
        use std::sync::atomic::Ordering;
        THREAD_ID_COUNTER.fetch_max(id.saturating_add(1), Ordering::SeqCst);
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
    WaitingForInput { prompt: String },

    /// Thread is paused
    Paused,

    /// Thread completed successfully
    Completed {
        /// Summary of what was accomplished
        summary: Option<String>,
    },

    /// Thread failed
    Failed { error: String },

    /// Thread was cancelled
    Cancelled,
}

impl ThreadStatus {
    /// Is this a terminal state?
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::Cancelled
        )
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
    /// Stable identity, generated for every new message and carried over
    /// the wire so clients can refer to a specific message (delete-by-id,
    /// edit-by-id, …). Messages persisted before this field existed load
    /// with `None` — they are assigned one in memory on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: SystemTime,
    /// Tool calls requested by an assistant turn (normalized form:
    /// `Vec<{id, name, arguments}>`). Stored verbatim so any client can
    /// faithfully replay the activity for this turn. Optional and
    /// skipped on serialize when absent for backward compatibility with
    /// existing `threads.json` files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    /// For role == Tool: the id of the tool call this message is the
    /// result of. Used by clients to associate results with the call
    /// that produced them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Media attachments (images, audio clips, files) referenced by this
    /// message. Persisted as references (URL / local cache path), not
    /// inline bytes, so history reloads can re-render them. Optional and
    /// skipped when absent for backward compatibility with existing
    /// `threads.json` files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<crate::gateway::protocol::types::MediaRef>>,
    /// The model's reasoning/thinking text for an assistant turn, when the
    /// provider emitted one. Thinking-mode providers (DeepSeek, Kimi, …)
    /// require it to be passed back verbatim on later assistant messages,
    /// so it is persisted alongside the turn. Optional and skipped when
    /// absent for backward compatibility with existing `threads.json` files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Message role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// One record in a thread's persistent event log.
///
/// The log is the thread's history *as it happened*: messages in
/// chronological order, interleaved with explicit turn boundaries. The
/// boundaries are what make the thread's state derivable rather than
/// stored — a `TurnStarted` with no matching `TurnEnded` means the turn
/// is still open. A gateway that dies mid-turn leaves exactly that shape
/// behind, so the next start knows both that the thread was streaming
/// and that the turn needs resuming; no status enum can go stale the
/// same way, because nothing has to remember to reset it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadLogRecord {
    /// A conversation message.
    Message(ThreadMessage),
    /// A turn began — the stop indicator's opening half.
    TurnStarted { at: SystemTime },
    /// The turn's explicit stop indicator. `ok` is false for errors and
    /// cancellations.
    TurnEnded { at: SystemTime, ok: bool },
}

/// An agent thread — the unified representation of all concurrent work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentThread {
    /// Unique identifier
    pub id: ThreadId,

    /// Project (working directory) this thread belongs to. Defaults to the
    /// implicit Default project so threads from a pre-projects `threads.json`
    /// deserialize cleanly and migrate into it.
    #[serde(default)]
    pub project_id: ProjectId,

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

    /// Conversation history (for interactive threads).
    ///
    /// Chronological and complete: this is the transcript every client
    /// loads, and nothing may truncate it short of deleting the thread.
    /// Compaction summarises the model's *context*, not the record — see
    /// [`Self::apply_compaction_keeping`], which moves `compacted_up_to`
    /// instead of removing anything.
    pub messages: VecDeque<ThreadMessage>,

    /// Compacted summary of the messages before `compacted_up_to`.
    pub compact_summary: Option<String>,

    /// How many leading messages `compact_summary` covers. Those messages
    /// are excluded from model-context building but kept in `messages` —
    /// they are the user's conversation, not the model's scratch space.
    #[serde(default)]
    pub compacted_up_to: usize,

    /// Working directory override for this thread.
    ///
    /// `None` — the common case — means the thread inherits its project's
    /// directory, so moving a project moves all of its threads with it. A
    /// `Some` value pins this one thread elsewhere. Appended with
    /// `serde(default)` so threads persisted before overrides existed load
    /// as inheriting.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,

    /// Result value (for task/sub-agent threads)
    pub result: Option<String>,

    /// Whether the thread is pinned to the top of its project's list in the
    /// sidebar. Appended with `serde(default)` so threads persisted before
    /// pinning existed load as unpinned.
    #[serde(default)]
    pub pinned: bool,

    /// Should this thread's context be shared with parent?
    pub share_context: bool,

    /// Whether a pre-compaction memory flush already ran in the current
    /// compaction cycle. Set when the flush instruction is injected and
    /// cleared when compaction runs, so the flush fires once per cycle
    /// rather than on every prompt near the threshold.
    #[serde(default)]
    pub memory_flushed: bool,

    /// When the currently open turn started, if one is running. Derived
    /// from the log's turn markers — reconstructed on load, never
    /// persisted as state of its own — so it cannot disagree with the
    /// record. `Some` is what clients see as "streaming"/"open".
    #[serde(skip)]
    pub open_turn: Option<SystemTime>,

    /// How many times a crashed turn in this thread has been resumed.
    ///
    /// Persisted, unlike `open_turn` — surviving the gateway's death is the
    /// entire point. A turn that takes the gateway down with it can never
    /// write its own stop indicator, so the thread still looks crashed on
    /// the next start and is resumed again, and again. Without a durable
    /// count that loop has no exit: the gateway becomes unstartable, which
    /// is a far worse failure than one turn not finishing.
    #[serde(default)]
    pub resume_attempts: u32,

    /// Whether a summary of this thread is being produced right now.
    ///
    /// Eligibility for compaction is "long enough, and not summarised yet",
    /// and both stay true for the whole provider round trip — so without
    /// this, every switch away from the same thread while one call is in
    /// flight starts another, each paid for and all but one discarded.
    ///
    /// Not persisted: a request in flight belongs to a process, and a marker
    /// that survived a restart would be a thread that could never be
    /// compacted again.
    #[serde(skip)]
    pub compacting: bool,

    /// Log records appended since the store last persisted this thread.
    /// Every mutation that belongs in the record pushes here; the store
    /// drains it with appends instead of rewriting history.
    #[serde(skip)]
    pub pending_log: Vec<ThreadLogRecord>,
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
            project_id: ProjectId::default(),
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
            compacted_up_to: 0,
            working_dir: None,
            result: None,
            pinned: false,
            share_context: true,
            memory_flushed: false,
            open_turn: None,
            resume_attempts: 0,
            compacting: false,
            pending_log: Vec::new(),
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
            project_id: ProjectId::default(),
            kind: ThreadKind::SubAgent {
                agent_id: agent_id.into(),
                task: task.into(),
            },
            label: label.into(),
            description: None,
            status: ThreadStatus::Running {
                progress: None,
                message: None,
            },
            parent_id,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            compacted_up_to: 0,
            working_dir: None,
            result: None,
            pinned: false,
            share_context: true,
            memory_flushed: false,
            open_turn: None,
            resume_attempts: 0,
            compacting: false,
            pending_log: Vec::new(),
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
            project_id: ProjectId::default(),
            kind: ThreadKind::Background {
                purpose: purpose.into(),
            },
            label: label.into(),
            description: None,
            status: ThreadStatus::Running {
                progress: None,
                message: None,
            },
            parent_id,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            compacted_up_to: 0,
            working_dir: None,
            result: None,
            pinned: false,
            share_context: false,
            memory_flushed: false,
            open_turn: None,
            resume_attempts: 0,
            compacting: false,
            pending_log: Vec::new(),
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
            project_id: ProjectId::default(),
            kind: ThreadKind::Task {
                action: action.into(),
            },
            label: label.into(),
            description: None,
            status: ThreadStatus::Running {
                progress: None,
                message: None,
            },
            parent_id,
            created_at: now,
            last_activity: now,
            is_foreground: false,
            messages: VecDeque::new(),
            compact_summary: None,
            compacted_up_to: 0,
            working_dir: None,
            result: None,
            pinned: false,
            share_context: true,
            memory_flushed: false,
            open_turn: None,
            resume_attempts: 0,
            compacting: false,
            pending_log: Vec::new(),
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
        self.status = ThreadStatus::Failed {
            error: error.into(),
        };
        self.last_activity = SystemTime::now();
    }

    /// Append a message to the conversation history and the pending log.
    ///
    /// Every new message is stamped with a stable id (uuid) so clients can
    /// refer to it — the id is generated here, at the single write point,
    /// so no caller can forget it.
    fn push_message(&mut self, message: ThreadMessage) {
        let mut message = message;
        if message.id.is_none() {
            message.id = Some(uuid::Uuid::new_v4().to_string());
        }
        self.messages.push_back(message.clone());
        self.pending_log.push(ThreadLogRecord::Message(message));
        self.last_activity = SystemTime::now();
    }

    /// Add a message to the conversation history.
    pub fn add_message(&mut self, role: MessageRole, content: impl Into<String>) {
        self.push_message(ThreadMessage {
            id: None,
            role,
            content: content.into(),
            timestamp: SystemTime::now(),
            tool_calls: None,
            tool_call_id: None,
            media: None,
            reasoning: None,
        });
    }

    /// Add an assistant message whose response carried reasoning/thinking
    /// text. Thinking-mode providers (DeepSeek, Kimi, …) require the
    /// reasoning content to be passed back on later turns, so it is stored
    /// with the message for history reconstruction.
    pub fn add_message_with_reasoning(
        &mut self,
        role: MessageRole,
        content: impl Into<String>,
        reasoning: Option<String>,
    ) {
        self.push_message(ThreadMessage {
            id: None,
            role,
            content: content.into(),
            timestamp: SystemTime::now(),
            tool_calls: None,
            tool_call_id: None,
            media: None,
            reasoning,
        });
    }

    /// Add a message whose id the caller already knows (e.g. one echoed from
    /// a client, so the bubble the user sees and the persisted record agree).
    pub fn add_message_with_id(
        &mut self,
        id: Option<String>,
        role: MessageRole,
        content: impl Into<String>,
    ) {
        self.push_message(ThreadMessage {
            id,
            role,
            content: content.into(),
            timestamp: SystemTime::now(),
            tool_calls: None,
            tool_call_id: None,
            media: None,
            reasoning: None,
        });
    }

    /// Add a message carrying media references (images, audio clips, files).
    ///
    /// The refs are persisted with the record so history reloads re-render
    /// them; the media bytes themselves live outside the thread log.
    pub fn add_message_with_media(
        &mut self,
        id: Option<String>,
        role: MessageRole,
        content: impl Into<String>,
        media: Vec<crate::gateway::protocol::types::MediaRef>,
    ) {
        self.push_message(ThreadMessage {
            id,
            role,
            content: content.into(),
            timestamp: SystemTime::now(),
            tool_calls: None,
            tool_call_id: None,
            media: Some(media),
            reasoning: None,
        });
    }

    /// Remove a message by its stable id: from the in-memory history and
    /// from the pending log, so a not-yet-persisted message cannot come
    /// back with the next append. Returns the removed message.
    ///
    /// The removal is persisted by the store rewriting the thread's log
    /// (append-only logs only add records, so the caller must call
    /// `ThreadStore::rewrite_thread_log` after this).
    pub fn remove_message(&mut self, message_id: &str) -> Option<ThreadMessage> {
        let pos = self
            .messages
            .iter()
            .position(|m| m.id.as_deref() == Some(message_id))?;
        let removed = self.messages.remove(pos).unwrap();
        self.pending_log.retain(
            |r| !matches!(r, ThreadLogRecord::Message(m) if m.id.as_deref() == Some(message_id)),
        );
        // A removed message that sat before the compaction boundary shifts
        // the boundary; never let it drift past the end.
        self.compacted_up_to = self
            .compacted_up_to
            .saturating_sub(1)
            .min(self.messages.len());
        self.last_activity = SystemTime::now();
        Some(removed)
    }

    /// Record that a turn began in this thread. The log carries the marker;
    /// until [`Self::end_turn`] writes the matching stop indicator, the
    /// thread is open — on screen and on disk.
    pub fn begin_turn(&mut self) {
        let at = SystemTime::now();
        self.open_turn = Some(at);
        self.pending_log.push(ThreadLogRecord::TurnStarted { at });
        self.last_activity = at;
    }

    /// Write the open turn's stop indicator, if one is open. `ok` is false
    /// for errors and cancellations. A thread with no open turn is left
    /// alone — a stray stop marker would just be noise in the record.
    pub fn end_turn(&mut self, ok: bool) {
        if self.open_turn.take().is_some() {
            let at = SystemTime::now();
            self.pending_log.push(ThreadLogRecord::TurnEnded { at, ok });
            self.last_activity = at;
            if ok {
                // A turn that finished clears the thread's crash history.
                //
                // Deliberately not in `begin_turn`: the resume path opens a
                // turn marker of its own, so resetting there would zero the
                // count on the very attempt it exists to count, and the loop
                // this guards against would be back untouched.
                self.resume_attempts = 0;
            }
        }
    }

    /// Whether a turn is open — started, with no stop indicator yet.
    pub fn is_open(&self) -> bool {
        self.open_turn.is_some()
    }

    /// The messages inside the model-context window: everything after the
    /// compaction boundary. Prompt builders must go through this, never
    /// `messages` directly — the record keeps everything, and a prompt
    /// built from the whole record grows every turn until the provider
    /// rejects it, which is precisely what compaction exists to prevent.
    pub fn context_messages(&self) -> impl Iterator<Item = &ThreadMessage> {
        self.messages.iter().skip(self.compacted_up_to)
    }

    /// Add an assistant turn that issued tool calls. `text` may be empty
    /// when the model produced only tool calls. `tool_calls` is the
    /// normalized JSON form (`Vec<{id, name, arguments}>`). `reasoning`
    /// carries the turn's thinking text (thinking-mode providers require
    /// it back on later turns).
    pub fn add_assistant_with_tool_calls(
        &mut self,
        text: impl Into<String>,
        tool_calls: serde_json::Value,
        reasoning: Option<String>,
    ) {
        self.push_message(ThreadMessage {
            id: None,
            role: MessageRole::Assistant,
            content: text.into(),
            timestamp: SystemTime::now(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            media: None,
            reasoning,
        });
    }

    /// Add a tool-result message linked to the originating call's id.
    pub fn add_tool_result(&mut self, tool_call_id: impl Into<String>, output: impl Into<String>) {
        self.push_message(ThreadMessage {
            id: None,
            role: MessageRole::Tool,
            content: output.into(),
            timestamp: SystemTime::now(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            media: None,
            reasoning: None,
        });
    }

    /// Get message count.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Generate a prompt for compacting this thread's conversation.
    ///
    /// Covers only what the current summary does not: the prior summary
    /// (if any) plus the messages after `compacted_up_to`, so re-compacting
    /// folds forward instead of re-reading the whole transcript.
    pub fn compaction_prompt(&self) -> String {
        let mut prompt = String::from(
            "Summarize the following conversation in 2-3 sentences, \
             capturing the key topics, decisions, and any pending items:\n\n",
        );

        if let Some(summary) = &self.compact_summary {
            prompt.push_str(&format!("Summary of earlier conversation: {}\n\n", summary));
        }

        for msg in self.context_messages() {
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
        self.apply_compaction_keeping(summary, 3);
    }

    /// Apply a compaction summary, keeping the given number of recent
    /// messages in the model's context. Used when the caller knows exactly
    /// which tail of the conversation the summary does *not* cover.
    ///
    /// This moves the context boundary — it deletes nothing. The old
    /// implementation popped the summarised messages off `messages`, and
    /// since `messages` is the transcript every client loads, switching
    /// threads or crossing the context threshold permanently destroyed
    /// the conversation down to its last few messages. The record stays
    /// whole; only context building skips the summarised prefix.
    pub fn apply_compaction_keeping(&mut self, summary: String, keep_recent: usize) {
        self.compacted_up_to = self.messages.len().saturating_sub(keep_recent);
        self.compact_summary = Some(summary);
        // A new compaction cycle begins: allow the next pre-compaction
        // memory flush to fire again.
        self.memory_flushed = false;
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

        // Include the messages the summary does not cover
        if self.messages.len() > self.compacted_up_to {
            ctx.push_str("## Recent Messages\n");
            for msg in self.context_messages() {
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
}

/// Info for sidebar display.
///
/// The busy state comes from the log's turn markers, not the status enum:
/// an open turn is "Streaming" whatever the enum says, and an interactive
/// thread at rest is "Ready". The enum still speaks for the task/sub-agent
/// lifecycle (Completed, Failed, …).
impl From<&AgentThread> for ThreadInfo {
    fn from(t: &AgentThread) -> Self {
        let (status, status_icon) = if t.open_turn.is_some() {
            ("Streaming".to_string(), "▶".to_string())
        } else if t.status == ThreadStatus::Active {
            ("Ready".to_string(), "○".to_string())
        } else {
            (t.status.display(), t.status.icon().to_string())
        };
        Self {
            id: t.id,
            project_id: t.project_id,
            kind: t.kind.display_name().to_string(),
            icon: t.kind.icon().to_string(),
            label: t.label.clone(),
            description: t.description.clone(),
            status,
            status_icon,
            is_foreground: t.is_foreground,
            is_interactive: t.kind.is_interactive(),
            message_count: t.messages.len(),
            has_summary: t.compact_summary.is_some(),
            has_result: t.result.is_some(),
            working_dir: t.working_dir.clone(),
            pinned: t.pinned,
        }
    }
}

/// Summary info for sidebar display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: ThreadId,
    #[serde(default)]
    pub project_id: ProjectId,
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
    /// Working-directory override, or `None` when the thread inherits its
    /// project's directory.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    /// Whether the thread is pinned to the top of its project's list in the
    /// sidebar.
    #[serde(default)]
    pub pinned: bool,
}
