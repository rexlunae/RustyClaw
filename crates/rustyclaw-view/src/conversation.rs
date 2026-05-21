//! Shared conversation display models used by UI renderers.

use rustyclaw_core::types::MessageRole;

use crate::MessageBubbleData;

/// A renderer-facing message row with optional extended details payload.
///
/// This model is intentionally separate from `rustyclaw_core::ui::ChatMessage`:
/// it carries only what frontends need for display and details overlays.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayMessageData {
    pub role: MessageRole,
    pub content: String,
    pub details: Option<String>,
}

impl DisplayMessageData {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            details: None,
        }
    }

    /// Create a message with extended details for details dialogs.
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

    /// Convert into a reusable message-bubble view model.
    pub fn to_bubble_data(&self, agent_name: Option<String>, has_details: bool) -> MessageBubbleData {
        MessageBubbleData {
            role: self.role,
            content: self.content.clone(),
            timestamp: None,
            is_streaming: false,
            agent_name,
            has_details,
        }
    }
}

/// Find the newest warning/error message that carries extended details.
pub fn latest_details_index(messages: &[DisplayMessageData]) -> Option<usize> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| {
            matches!(m.role, MessageRole::Warning | MessageRole::Error) && m.details.is_some()
        })
        .map(|(idx, _)| idx)
}