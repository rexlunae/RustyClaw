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
    pub collapsed: bool,
}

impl DisplayMessageData {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            details: None,
            tool_calls: Vec::new(),
            collapsed: false,
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
            collapsed: false,
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

    pub const AUTO_COLLAPSE_LINES: usize = 40;
    pub const AUTO_COLLAPSE_CHARS: usize = 2000;

    /// Set collapsed=true if this is a long assistant/tool message.
    pub fn auto_collapse_if_needed(&mut self) {
        if matches!(self.role, MessageRole::Assistant | MessageRole::ToolResult) {
            let line_count = self.content.lines().count();
            if line_count > Self::AUTO_COLLAPSE_LINES || self.content.len() > Self::AUTO_COLLAPSE_CHARS {
                self.collapsed = true;
            }
        }
    }

    /// Toggle collapsed state. Returns new state.
    pub fn toggle_collapse(&mut self) -> bool {
        self.collapsed = !self.collapsed;
        self.collapsed
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
            collapsed: self.collapsed,
        }
    }

    /// Convert a wire `ChatMessage` (as carried in `ThreadHistoryReply`) into a
    /// renderer-facing `DisplayMessageData`. Unknown roles fall back to
    /// `MessageRole::System` so the message is still surfaced.
    pub fn from_chat_message(
        msg: &rustyclaw_core::gateway::protocol::types::ChatMessage,
    ) -> Self {
        let role = msg.to_core_message_role();
        let mut data = Self::new(role, msg.content.clone());
        // Surface tool calls embedded in an assistant turn so the
        // history view shows the same activity that was visible live.
        // We accept the normalized form `[{id, name, arguments}, ...]`
        // emitted by the gateway when persisting tool rounds.
        if let Some(tcs) = &msg.tool_calls {
            if let Some(arr) = tcs.as_array() {
                for tc in arr {
                    let id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = tc
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = tc
                        .get("arguments")
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    data.add_tool_call(id, name, arguments);
                }
            }
        }
        data
    }
}

/// Convert a contiguous slice of wire `ChatMessage`s into renderer-facing
/// `DisplayMessageData`. Tool-result messages (role == \"tool\") are
/// folded into the preceding assistant turn's matching tool call by id
/// rather than emitted as a separate bubble — matching how the live
/// stream rendered them. Tool results without a matching call are kept
/// as standalone `ToolResult` messages.
pub fn convert_history(
    msgs: &[rustyclaw_core::gateway::protocol::types::ChatMessage],
) -> Vec<DisplayMessageData> {
    let mut out: Vec<DisplayMessageData> = Vec::with_capacity(msgs.len());
    for m in msgs {
        if m.to_core_message_role() == MessageRole::ToolResult {
            if let Some(call_id) = &m.tool_call_id {
                if let Some(prev) = out.iter_mut().rev().find(|d| {
                    d.role == MessageRole::Assistant
                        && d.tool_calls.iter().any(|tc| &tc.id == call_id)
                }) {
                    prev.set_tool_result(call_id, m.content.clone(), false);
                    continue;
                }
            }
        }
        let mut msg = DisplayMessageData::from_chat_message(m);
        msg.auto_collapse_if_needed();
        out.push(msg);
    }
    out
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