//! Component data for chat message bubbles and tool call panels.
//!
//! These types represent the exact slice of data that `MessageBubble`
//! and `ToolCallPanel` need to render — distinct from the canonical
//! [`ChatMessage`] / [`ToolCallInfo`] models in `rustyclaw_core::ui`.
//!
//! The key difference: a `ChatMessage` owns tool calls and carries
//! enough state for translation from `GatewayEvent`. A `MessageBubbleData`
//! is the *rendered* view of just the bubble — no tool calls, no
//! intermediate state, no back-reference to the canonical message.
//! Tool calls are a separate [`ToolCallData`] component rendered alongside.

use chrono::{DateTime, Utc};
use rustyclaw_core::types::MessageRole;

// ── Message bubble ──────────────────────────────────────────────────────────

/// Everything a message-bubble component needs to render.
///
/// Used by both the desktop (Dioxus `MessageBubble`) and TUI
/// (iocraft `MessageBubble`) as the single source of truth for
/// rendering data.  Event handlers (click, long-press, etc.) are
/// provided by the framework-specific wrapper.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageBubbleData {
    /// Who sent this message (User, Assistant, System, etc.).
    pub role: MessageRole,

    /// The message text content (plain or markdown, depending on role).
    pub content: String,

    /// When the message was created.
    ///
    /// Optional — the TUI does not track per-message timestamps.
    pub timestamp: Option<DateTime<Utc>>,

    /// Whether this message is still being streamed.
    pub is_streaming: bool,

    /// Display name override for assistant messages.
    pub agent_name: Option<String>,

    /// Whether this message has extended structured details
    /// (request URL, headers, body excerpt) accessible via a
    /// "show details" action.
    pub has_details: bool,
}

impl Default for MessageBubbleData {
    fn default() -> Self {
        Self {
            role: MessageRole::System,
            content: String::new(),
            timestamp: None,
            is_streaming: false,
            agent_name: None,
            has_details: false,
        }
    }
}

impl MessageBubbleData {
    /// Build from a canonical [`rustyclaw_core::ui::ChatMessage`].
    ///
    /// Preserves role, content, timestamp, and streaming state.
    /// `agent_name` must be set by the caller (it depends on external
    /// state, not the message itself).
    pub fn from_chat_message(msg: &rustyclaw_core::ui::ChatMessage, agent_name: Option<String>) -> Self {
        Self {
            role: msg.role.clone(),
            content: msg.content.clone(),
            timestamp: Some(msg.timestamp),
            is_streaming: msg.is_streaming,
            agent_name,
            has_details: false,
        }
    }
}

// ── Tool call panel ─────────────────────────────────────────────────────────

/// Everything a tool-call panel component needs to render.
///
/// Represented as a component separate from the message bubble —
/// each message may have zero or more tool calls, and in both the
/// desktop and TUI they render as distinct nested elements.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallData {
    /// Unique tool call identifier (matches approval flow).
    pub id: String,

    /// Tool name, shown as the panel header.
    pub name: String,

    /// Pretty-printed JSON arguments.
    pub arguments: String,

    /// Optional result returned by the tool.
    pub result: Option<String>,

    /// Whether the tool returned an error.
    pub is_error: bool,

    /// Whether the panel starts collapsed.
    pub collapsed: bool,
}

impl From<&rustyclaw_core::ui::ToolCallInfo> for ToolCallData {
    fn from(tc: &rustyclaw_core::ui::ToolCallInfo) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: rustyclaw_core::ui::pretty_print_json(&tc.arguments),
            result: tc.result.clone(),
            is_error: tc.is_error,
            collapsed: tc.collapsed,
        }
    }
}

// ── Streaming indicator ─────────────────────────────────────────────────────

/// Data for the streaming progress indicator shown beneath a message
/// while the model is generating.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StreamingIndicatorData {
    /// Number of streaming chunks received so far.
    pub chunks: u32,

    /// Total bytes received across all chunks.
    pub bytes: usize,

    /// Whether the model is in thinking mode (extended reasoning).
    pub is_thinking: bool,
}
