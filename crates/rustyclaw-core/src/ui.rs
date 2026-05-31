//! Client-agnostic UI state types.
//!
//! These types are shared across all RustyClaw clients (desktop, TUI, CLI)
//! so that conversation models, thread info, and tool-call state have a
//! single canonical definition instead of living in each client crate.

use crate::types::MessageRole;
use crate::user_prompt_types::UserPrompt;

// ── Connection status ───────────────────────────────────────────────────────

/// Connection status to the gateway.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Authenticating,
    Authenticated,
    Error(String),
}

// ── Chat message types ──────────────────────────────────────────────────────

/// A chat message in the conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub tool_calls: Vec<ToolCallInfo>,
    pub is_streaming: bool,
}

/// Information about a tool call within a message.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<String>,
    pub is_error: bool,
    pub collapsed: bool,
}

// ── Thread / session types ──────────────────────────────────────────────────

/// Thread/session info for the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadInfo {
    pub id: u64,
    /// Project this thread belongs to (0 = the active project).
    pub project_id: u64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
}

/// Project info for the sidebar's top level.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProjectInfo {
    pub id: u64,
    pub name: String,
    pub path: String,
}

// ── Dialog state helpers ────────────────────────────────────────────────────

/// Pending state for dialogs that the UI reads to show/hide overlays.
///
/// This is an intermediate representation: the client's signal-based or
/// channel-based state reacts to `GatewayEvent`s by pushing entries here,
/// and the UI rendering code reads these to display the relevant dialog.
#[derive(Clone, Debug, Default)]
pub struct DialogState {
    /// Pending tool approval request (id, name, arguments).
    pub pending_tool_approval: Option<(String, String, String)>,

    /// Pending user prompt from the agent.
    pub pending_user_prompt: Option<UserPrompt>,

    /// Pending credential request (id, provider, secret_name, message).
    pub pending_credential_request: Option<(String, String, String, String)>,

    /// Pending device flow (url, code, message).
    pub pending_device_flow: Option<(String, String, Option<String>)>,

    /// Number of streaming chunks received in the current response.
    pub streaming_chunks: u32,

    /// Total bytes received in the current streaming response.
    pub streaming_bytes: usize,
}

impl ChatMessage {
    /// Create a new user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content: content.into(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: false,
        }
    }

    /// Start a new assistant message (streaming).
    pub fn start_assistant(id: String) -> Self {
        Self {
            id,
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: true,
        }
    }

    /// Append text content to this message (used during streaming).
    pub fn append_content(&mut self, delta: &str) {
        if self.is_streaming {
            self.content.push_str(delta);
        }
    }

    /// Mark this message as finished streaming.
    pub fn finish(&mut self) {
        self.is_streaming = false;
    }

    /// Add a tool call to this message.
    pub fn add_tool_call(&mut self, id: String, name: String, arguments: String) {
        self.tool_calls.push(ToolCallInfo {
            id,
            name,
            arguments,
            result: None,
            is_error: false,
            collapsed: true,
        });
    }

    /// Set the result for a tool call by ID.
    pub fn set_tool_result(&mut self, id: &str, result: String, is_error: bool) {
        for tool in &mut self.tool_calls {
            if tool.id == id {
                tool.result = Some(result);
                tool.is_error = is_error;
                return;
            }
        }
    }
}

impl DialogState {
    /// Merge a [`super::gateway::client_types::GatewayEvent`] into the dialog state.
    ///
    /// Returns `true` if the event was consumed (set some dialog state),
    /// `false` if the event is unrelated to dialogs.
    pub fn handle_gateway_event(
        &mut self,
        event: &crate::gateway::client_types::GatewayEvent,
    ) -> bool {
        match event {
            crate::gateway::client_types::GatewayEvent::ToolApprovalRequest {
                id,
                name,
                arguments,
            } => {
                self.pending_tool_approval = Some((id.clone(), name.clone(), arguments.clone()));
                true
            }
            crate::gateway::client_types::GatewayEvent::UserPromptRequest { prompt, .. } => {
                self.pending_user_prompt = Some(prompt.clone());
                true
            }
            crate::gateway::client_types::GatewayEvent::CredentialRequest {
                id,
                provider,
                secret_name,
                message,
            } => {
                self.pending_credential_request = Some((
                    id.clone(),
                    provider.clone(),
                    secret_name.clone(),
                    message.clone(),
                ));
                true
            }
            crate::gateway::client_types::GatewayEvent::DeviceFlowStart { url, code, message } => {
                self.pending_device_flow = Some((url.clone(), code.clone(), message.clone()));
                true
            }
            crate::gateway::client_types::GatewayEvent::DeviceFlowComplete => {
                self.pending_device_flow = None;
                true
            }
            _ => false,
        }
    }
}

// ── Content formatting helpers ──────────────────────────────────────────────

/// Pretty-print a JSON string for display (tool call arguments, results).
///
/// Falls back to the raw string if JSON deserialisation fails.
pub fn pretty_print_json(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| input.to_string())
}

/// Truncate content to fit within display constraints.
///
/// - `max_chars`: hard character limit (applies before `max_lines`)
/// - `max_lines`: maximum number of lines to show (beyond this, `…` is appended)
///
/// When both limits apply, the shorter one wins.
pub fn truncate_content(content: &str, max_chars: usize, max_lines: usize) -> String {
    let mut result = String::with_capacity(content.len().min(max_chars));
    let mut line_count = 0usize;

    for (char_count, ch) in content.chars().enumerate() {
        if char_count >= max_chars {
            result.push('…');
            break;
        }
        if ch == '\n' {
            line_count += 1;
            if line_count >= max_lines {
                result.push('…');
                result.push('\n');
                break;
            }
        }
        result.push(ch);
    }

    result
}

/// Format a tool call name and arguments into a human-readable label.
///
/// Uses [`pretty_print_json`] on the arguments and truncates long content.
pub fn format_tool_call(name: &str, arguments: &str) -> String {
    let pretty = pretty_print_json(arguments);
    if pretty.len() > 200 {
        format!("🔧 {}\n{}\n…", name, truncate_content(&pretty, 200, 8))
    } else {
        format!("🔧 {}\n{}", name, pretty)
    }
}

/// Format a tool result for display.
///
/// Truncates long results and marks errors.
pub fn format_tool_result(result: &str, is_error: bool) -> String {
    let preview = if result.len() > 100 {
        truncate_content(&pretty_print_json(result), 500, 15)
    } else {
        result.to_string()
    };

    if is_error {
        format!("✕ Error:\n{}", preview)
    } else {
        format!("✓ Result:\n{}", preview)
    }
}

/// Format a UTC timestamp as a relative human-readable string.
///
/// "Just now" (< 10s ago), "12m ago", "2h ago", "3d ago", or the
/// date ("Jan 15") for older timestamps.
pub fn format_relative_time(timestamp: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*timestamp);

    let secs = duration.num_seconds();
    if secs < 10 {
        "Just now".to_string()
    } else if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 604800 {
        format!("{}d ago", secs / 86400)
    } else {
        timestamp.format("%b %d").to_string()
    }
}

/// Format a timestamp as a short time string for chat bubbles
/// (e.g. "14:32" or "Yesterday 14:32").
pub fn format_chat_timestamp(timestamp: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(*timestamp);

    if duration.num_seconds() < 10 {
        "Just now".to_string()
    } else if duration.num_hours() < 24 {
        timestamp.format("%H:%M").to_string()
    } else if duration.num_hours() < 48 {
        format!("Yesterday {}", timestamp.format("%H:%M"))
    } else {
        timestamp.format("%b %d %H:%M").to_string()
    }
}

// ── Streaming state ─────────────────────────────────────────────────────────

use std::time::Instant;

/// Tracks the progress of an active streaming response.
///
/// Both the TUI and desktop clients need to track similar streaming
/// metrics. This struct consolidates that tracking in one place.
#[derive(Clone, Debug, Default)]
pub struct StreamingState {
    /// Whether we are currently receiving a streaming response.
    pub is_streaming: bool,

    /// Whether the model is currently in "thinking" mode.
    pub is_thinking: bool,

    /// Number of streaming chunks received so far.
    pub chunks: u32,

    /// Total bytes received across all chunks.
    pub bytes: usize,

    /// When the current stream started.
    pub start_time: Option<Instant>,
}

impl StreamingState {
    /// Start tracking a new streaming response.
    pub fn start_streaming(&mut self) {
        self.is_streaming = true;
        self.is_thinking = false;
        self.chunks = 0;
        self.bytes = 0;
        self.start_time = Some(Instant::now());
    }

    /// Start thinking mode (extended thinking models).
    pub fn start_thinking(&mut self) {
        self.is_thinking = true;
        self.is_streaming = false;
    }

    /// End thinking mode and begin actual streaming.
    pub fn end_thinking(&mut self) {
        self.is_thinking = false;
        self.is_streaming = true;
        self.start_time = Some(Instant::now());
    }

    /// Record one chunk of streaming data.
    pub fn record_chunk(&mut self, data: &str) {
        self.chunks += 1;
        self.bytes += data.len();
    }

    /// Finish the streaming response.
    pub fn finish(&mut self) {
        self.is_streaming = false;
        self.is_thinking = false;
        self.start_time = None;
    }

    /// Human-readable progress summary (e.g. "42 chunks, 12.3 KB").
    pub fn progress_summary(&self) -> String {
        if self.bytes >= 1024 {
            format!(
                "{} chunks, {:.1} KB",
                self.chunks,
                self.bytes as f64 / 1024.0,
            )
        } else if self.chunks > 0 {
            format!("{} chunks, {} B", self.chunks, self.bytes)
        } else {
            "Streaming…".to_string()
        }
    }
}
