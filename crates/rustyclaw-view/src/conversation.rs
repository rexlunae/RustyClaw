//! Shared conversation display models used by UI renderers.

use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{StreamingState, ThreadInfo};

use crate::{MessageBubbleData, ToolCallData};

/// A renderer-facing message row with optional extended details payload.
///
/// This model is intentionally separate from `rustyclaw_core::ui::ChatMessage`:
/// it carries only what frontends need for display and details overlays.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayMessageData {
    pub role: MessageRole,
    pub content: String,
    pub details: Option<String>,
    pub tool_calls: Vec<ToolCallData>,
}

impl DisplayMessageData {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            details: None,
            tool_calls: Vec::new(),
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
            tool_calls: Vec::new(),
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

    /// Add a structured tool call rendered alongside this message.
    pub fn add_tool_call(&mut self, id: String, name: String, arguments: String) {
        self.tool_calls.push(ToolCallData {
            id,
            name,
            arguments,
            result: None,
            is_error: false,
            collapsed: true,
        });
    }

    /// Set the result for a tool call by id.
    pub fn set_tool_result(&mut self, id: &str, result: String, is_error: bool) {
        for tc in &mut self.tool_calls {
            if tc.id == id {
                tc.result = Some(result);
                tc.is_error = is_error;
                return;
            }
        }
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

/// A suggested starter prompt shown in empty-chat states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StarterPromptData {
    pub icon: &'static str,
    pub title: &'static str,
    pub prompt: &'static str,
}

const STARTER_PROMPTS: [StarterPromptData; 4] = [
    StarterPromptData {
        icon: "🔍",
        title: "Explore the system",
        prompt: "What can you do? List your available tools and capabilities.",
    },
    StarterPromptData {
        icon: "📚",
        title: "Summarise a topic",
        prompt: "Give me a concise overview of how RustyClaw secures agent runtimes.",
    },
    StarterPromptData {
        icon: "🛠️",
        title: "Run a quick task",
        prompt: "Help me draft a TOML config for connecting to a local Ollama provider.",
    },
    StarterPromptData {
        icon: "🧠",
        title: "Think out loud",
        prompt: "Walk me through how you'd debug a failing tool call step by step.",
    },
];

/// Shared empty-state copy and starter prompts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmptyStateData {
    pub agent_name: Option<String>,
}

impl EmptyStateData {
    pub fn greeting(&self) -> String {
        match self.agent_name.as_deref() {
            Some(name) if !name.is_empty() => format!("How can {name} help you today?"),
            _ => "How can I help you today?".to_string(),
        }
    }

    pub fn subtitle(&self) -> &'static str {
        "Pick a starter or just type below to begin."
    }

    pub fn starters(&self) -> &'static [StarterPromptData] {
        starter_prompts()
    }
}

pub fn starter_prompts() -> &'static [StarterPromptData] {
    &STARTER_PROMPTS
}

/// Shared chat-surface progress state used by message panes and status areas.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatSurfaceData {
    pub is_processing: bool,
    pub is_thinking: bool,
    pub is_streaming: bool,
    pub streaming_chunks: u32,
    pub streaming_bytes: usize,
    pub elapsed: Option<String>,
    pub spinner_tick: usize,
}

impl ChatSurfaceData {
    pub fn task_label(&self) -> &'static str {
        if self.is_streaming || self.is_thinking {
            "Streaming…"
        } else if self.is_processing {
            "Processing…"
        } else {
            "Idle"
        }
    }

    pub fn progress_summary(&self) -> Option<String> {
        if self.is_streaming {
            Some(
                StreamingState {
                    is_streaming: true,
                    is_thinking: self.is_thinking,
                    chunks: self.streaming_chunks,
                    bytes: self.streaming_bytes,
                    start_time: None,
                }
                .progress_summary(),
            )
        } else if self.is_processing {
            Some("Processing…".to_string())
        } else {
            None
        }
    }

    pub fn status_hint_text(&self, hint: &str) -> String {
        if self.is_streaming || self.is_thinking {
            let elapsed = self.elapsed.as_deref().unwrap_or("");
            if elapsed.is_empty() {
                "Streaming response".to_string()
            } else {
                format!("Streaming response {elapsed}")
            }
        } else if hint.is_empty() {
            "Ctrl+C quit · /help commands · ↑↓ scroll".to_string()
        } else {
            hint.to_string()
        }
    }
}

/// Shared title/subtitle modelling for the desktop top bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopBarData {
    pub title: String,
    pub subtitle: String,
}

impl TopBarData {
    pub fn from_threads(
        foreground_id: Option<u64>,
        threads: &[ThreadInfo],
        agent_name: Option<String>,
        provider: Option<String>,
        model: Option<String>,
    ) -> Self {
        let title = foreground_id
            .and_then(|id| threads.iter().find(|t| t.id == id).cloned())
            .and_then(|t| t.label.clone().or(Some(format!("Session #{}", t.id))))
            .unwrap_or_else(|| "New conversation".to_string());

        let sub_parts: Vec<String> = [
            agent_name,
            match (provider.as_ref(), model.as_ref()) {
                (Some(p), Some(m)) => Some(format!("{p} · {m}")),
                (None, Some(m)) => Some(m.clone()),
                (Some(p), None) => Some(p.clone()),
                _ => None,
            },
        ]
        .into_iter()
        .flatten()
        .collect();

        Self {
            title,
            subtitle: sub_parts.join(" — "),
        }
    }
}