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
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
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
                self.pending_credential_request =
                    Some((id.clone(), provider.clone(), secret_name.clone(), message.clone()));
                true
            }
            crate::gateway::client_types::GatewayEvent::DeviceFlowStart { url, code, message } => {
                self.pending_device_flow =
                    Some((url.clone(), code.clone(), message.clone()));
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
