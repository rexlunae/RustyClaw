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

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use rustyclaw_core::types::MessageRole;

// ── Message bubble ──────────────────────────────────────────────────────────

/// Everything a message-bubble component needs to render.
///
/// Used by both the desktop (Dioxus `MessageBubble`) and TUI
/// (iocraft `MessageBubble`) as the single source of truth for
/// rendering data.  Event handlers (click, long-press, etc.) are
/// provided by the framework-specific wrapper.
///
/// Methods on this struct centralise display logic so that both
/// clients derive the same labels, icons, and content transformations
/// without duplicating match arms.
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
    pub collapsed: bool,
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
            collapsed: false,
        }
    }
}

impl MessageBubbleData {
    /// Build from a canonical [`rustyclaw_core::ui::ChatMessage`].
    ///
    /// Preserves role, content, timestamp, and streaming state.
    /// `agent_name` must be set by the caller (it depends on external
    /// state, not the message itself).
    pub fn from_chat_message(
        msg: &rustyclaw_core::ui::ChatMessage,
        agent_name: Option<String>,
    ) -> Self {
        Self {
            role: msg.role,
            content: msg.content.clone(),
            timestamp: Some(msg.timestamp),
            is_streaming: msg.is_streaming,
            agent_name,
            has_details: false,
            collapsed: false,
        }
    }

    // ── Shared display logic ────────────────────────────────────────────

    /// The human-readable label for this message's role.
    ///
    /// Common values: "You", "Assistant", "System", "Thinking", etc.
    /// For assistant messages, [`agent_name`](Self::agent_name) takes
    /// precedence (with `"Assistant"` as fallback).
    pub fn display_name(&self) -> Cow<'_, str> {
        match self.role {
            MessageRole::User => "You".into(),
            MessageRole::Assistant => self
                .agent_name
                .as_deref()
                .filter(|n| !n.is_empty())
                .map(Cow::Borrowed)
                .unwrap_or(Cow::Borrowed("Assistant")),
            MessageRole::Info => "Info".into(),
            MessageRole::Success => "Success".into(),
            MessageRole::Warning => "Warning".into(),
            MessageRole::Error => "Error".into(),
            MessageRole::System => "System".into(),
            MessageRole::ToolCall => "Tool Call".into(),
            MessageRole::ToolResult => "Tool Result".into(),
            MessageRole::Thinking => "Thinking".into(),
        }
    }

    /// The icon/emoji associated with this message's role.
    ///
    /// Delegates to [`MessageRole::icon()`] which both clients already
    /// depend on.  Provided here for convenience so that a single method
    /// call replaces the manual match in each client.
    pub fn icon(&self) -> &'static str {
        self.role.icon()
    }

    /// Whether this message should be rendered as markdown.
    ///
    /// Assistant messages that aren't still streaming get markdown
    /// rendering.  All other roles (User, System, Error, etc.)
    /// display as plain text.
    pub fn should_render_markdown(&self) -> bool {
        self.role == MessageRole::Assistant && !self.is_streaming
    }

    /// The text to display, with role-specific transformations.
    ///
    /// - **Thinking** messages are truncated at `max_chars` (default 120)
    ///   to avoid overwhelming the chat area with raw reasoning.
    /// - All other roles return the raw content unchanged.
    ///
    /// Markdown rendering is **not** applied here — that's renderer-
    /// specific (the desktop renders HTML, the TUI renders ANSI).
    /// This method only applies plain-text transformations.
    pub fn display_content(&self) -> Cow<'_, str> {
        self.display_content_truncated(120)
    }

    /// Like [`display_content`](Self::display_content) but with a
    /// custom truncation limit for thinking messages.
    pub fn display_content_truncated(&self, thinking_max_chars: usize) -> Cow<'_, str> {
        if self.role == MessageRole::Thinking && self.content.len() > thinking_max_chars {
            format!("{}…", &self.content[..thinking_max_chars]).into()
        } else {
            self.content.as_str().into()
        }
    }

    pub const AUTO_COLLAPSE_LINES: usize = 40;
    pub const AUTO_COLLAPSE_CHARS: usize = 2000;
    /// Lines to show when collapsed.
    pub const COLLAPSED_PREVIEW_LINES: usize = 8;

    /// Whether this message is long enough to be collapsible.
    ///
    /// Checks byte length first (O(1)) before counting lines (O(N)).
    pub fn is_collapsible(&self) -> bool {
        self.content.len() > Self::AUTO_COLLAPSE_CHARS
            || self.content.lines().count() > Self::AUTO_COLLAPSE_LINES
    }

    /// Content to actually render — truncated when collapsed, full otherwise.
    ///
    /// Returns a borrow in the common (uncollapsed) case to avoid allocation.
    pub fn content_for_render(&self) -> Cow<'_, str> {
        if self.collapsed && self.is_collapsible() {
            let lines: Vec<&str> = self
                .content
                .lines()
                .take(Self::COLLAPSED_PREVIEW_LINES)
                .collect();
            let preview = lines.join("\n");
            let hidden = self
                .content
                .lines()
                .count()
                .saturating_sub(Self::COLLAPSED_PREVIEW_LINES);
            format!("{preview}\n\n… {hidden} lines hidden (Ctrl+E to expand)").into()
        } else {
            Cow::Borrowed(&self.content)
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

impl Default for ToolCallData {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            arguments: String::new(),
            result: None,
            is_error: false,
            collapsed: true,
        }
    }
}

impl ToolCallData {
    /// A short summary line for this tool call.
    ///
    /// e.g. `"🔧 web_search"` or `"🔧 write_file (error)"`
    pub fn summary(&self) -> String {
        if self.is_error {
            format!("🔧 {} (error)", self.name)
        } else {
            format!("🔧 {}", self.name)
        }
    }

    /// The arguments string, truncated for display.
    ///
    /// Uses `rustyclaw_core::ui::truncate_content` to limit both
    /// character count and line count.
    pub fn arguments_preview(&self, max_chars: usize, max_lines: usize) -> String {
        rustyclaw_core::ui::truncate_content(&self.arguments, max_chars, max_lines)
    }

    /// The result string, truncated for display.
    ///
    /// Tool results can be arbitrarily large (e.g. shell output, file contents).
    /// Rendering unbounded content freezes the TUI layout engine, so we cap it.
    pub fn result_preview(&self, max_chars: usize, max_lines: usize) -> Option<String> {
        self.result
            .as_deref()
            .map(|r| rustyclaw_core::ui::truncate_content(r, max_chars, max_lines))
    }
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
