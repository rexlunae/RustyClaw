// ── Display types for TUI messages ──────────────────────────────────────────

use rustyclaw_core::types::MessageRole;

/// A single message displayed in the chat pane.
#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub role: MessageRole,
    pub content: String,
}

impl DisplayMessage {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
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
