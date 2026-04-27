// ── Display types for TUI messages ──────────────────────────────────────────

use rustyclaw_core::types::MessageRole;

/// A single message displayed in the chat pane.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
    /// Extended structured details (URL, redacted headers, status,
    /// body excerpt, full cause chain).  Populated by warning/error
    /// messages that originated from `anyhow_tracing::Error` carrying
    /// `RequestDetails`.  When `Some(_)`, the TUI's details-dialog
    /// keybind will surface this in a scrollable popup.
    pub details: Option<String>,
}

impl DisplayMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            details: None,
        }
    }

    /// Create a message with extended details for the TUI's details dialog.
    pub fn with_details(
        role: MessageRole,
        content: impl Into<String>,
        details: impl Into<String>,
    ) -> Self {
        Self {
            role,
            content: content.into(),
            details: Some(details.into()),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content)
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Assistant, content)
    }
    pub fn info(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Info, content)
    }
    pub fn success(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Success, content)
    }
    pub fn warning(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Warning, content)
    }
    pub fn error(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Error, content)
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self::new(MessageRole::System, content)
    }
    pub fn tool_call(content: impl Into<String>) -> Self {
        Self::new(MessageRole::ToolCall, content)
    }
    pub fn tool_result(content: impl Into<String>) -> Self {
        Self::new(MessageRole::ToolResult, content)
    }
    pub fn thinking(content: impl Into<String>) -> Self {
        Self::new(MessageRole::Thinking, content)
    }

    /// Append text to the message content.
    pub fn append(&mut self, text: &str) {
        self.content.push_str(text);
    }
}
