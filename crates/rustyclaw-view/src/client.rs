//! Centralized, renderer-agnostic client state shared by every frontend.
//!
//! Historically the desktop client owned a large `AppState` struct
//! (`rustyclaw-desktop/src/state.rs`) holding the conversation, connection,
//! session and model state, while the TUI kept the equivalent state scattered
//! inside its iocraft component tree. The two could drift apart, and there was
//! no single definition of "the client".
//!
//! [`ClientState`] is that single definition. It holds everything that is
//! common to the clients and is independent of any rendering backend — the
//! gateway connection status, the conversation, threads, the selected
//! model/provider, streaming progress and the pending interactive requests.
//! Each frontend embeds a `ClientState` and adds only the fields its renderer
//! needs (the desktop's theme, sidebar visibility and file browser; the TUI's
//! scroll offset, spinner frame and dialog widgets). The goal is that the only
//! difference between the TUI and the desktop app is how they render this
//! shared state.
//!
//! The state-mutation logic (appending streaming chunks, folding tool results
//! into the originating turn, switching threads, hydrating history from the
//! gateway) lives here too, so it is defined once rather than reimplemented per
//! client.

use std::collections::{HashMap, VecDeque};

use rustyclaw_core::gateway::protocol;
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ChatMessage, ConnectionStatus, ProjectInfo, ThreadInfo, ToolCallInfo};
use rustyclaw_core::user_prompt_types::UserPrompt;

use crate::PromptAttachment;

/// State common to every RustyClaw frontend.
///
/// See the [module documentation](self) for the rationale. This is the
/// renderer-agnostic "client structure"; frontends wrap it and add their own
/// rendering-specific fields.
#[derive(Clone, Debug)]
pub struct ClientState {
    /// Current connection status.
    pub connection: ConnectionStatus,
    /// Gateway URL.
    pub gateway_url: String,
    /// Chat messages for the current (foreground) thread.
    pub messages: VecDeque<ChatMessage>,
    /// Per-thread message history (thread_id → messages).
    pub thread_messages: HashMap<u64, VecDeque<ChatMessage>>,
    /// Current input text.
    pub input: String,
    /// Whether we're waiting for a response.
    pub is_processing: bool,
    /// Whether the assistant is currently streaming.
    pub is_streaming: bool,
    /// Current thinking state (for extended-thinking models).
    pub is_thinking: bool,
    /// Active threads/sessions.
    pub threads: Vec<ThreadInfo>,
    /// Known projects (the sidebar's top level).
    pub projects: Vec<ProjectInfo>,
    /// The active project's ID (its threads run in its directory).
    pub active_project_id: u64,
    /// Current foreground thread ID.
    pub foreground_thread_id: Option<u64>,
    /// Agent name from hatching.
    pub agent_name: Option<String>,
    /// Whether the vault is locked.
    pub vault_locked: bool,
    /// Whether the first-run hatching dialog is needed.
    pub needs_hatching: bool,
    /// Current model name.
    pub model: Option<String>,
    /// Current provider name.
    pub provider: Option<String>,
    /// Files and directories attached to the next prompt.
    pub prompt_attachments: Vec<PromptAttachment>,
    /// Transient status message.
    pub status_message: Option<String>,
    /// Working directory used for tool execution.
    pub working_directory: Option<String>,
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
    /// Whether the agent currently has access to vault secrets.
    pub agent_access: bool,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            connection: ConnectionStatus::Disconnected,
            gateway_url: String::new(),
            messages: VecDeque::new(),
            thread_messages: HashMap::new(),
            input: String::new(),
            is_processing: false,
            is_streaming: false,
            is_thinking: false,
            threads: Vec::new(),
            projects: Vec::new(),
            active_project_id: 0,
            foreground_thread_id: None,
            agent_name: None,
            vault_locked: false,
            needs_hatching: false,
            model: None,
            provider: None,
            prompt_attachments: Vec::new(),
            status_message: None,
            working_directory: None,
            pending_tool_approval: None,
            pending_user_prompt: None,
            pending_credential_request: None,
            pending_device_flow: None,
            streaming_chunks: 0,
            streaming_bytes: 0,
            agent_access: false,
        }
    }
}

impl ClientState {
    /// Create a new, disconnected client state for the given gateway URL.
    pub fn new(gateway_url: impl Into<String>) -> Self {
        Self {
            gateway_url: gateway_url.into(),
            ..Self::default()
        }
    }

    /// Whether a gateway connection is established (connected or authenticated).
    pub fn is_connected(&self) -> bool {
        matches!(
            self.connection,
            ConnectionStatus::Connected | ConnectionStatus::Authenticated
        )
    }

    /// Add a user message to the conversation.
    pub fn add_user_message(&mut self, content: String) {
        self.messages.push_back(ChatMessage::user(content));
    }

    /// Start a new assistant message (streaming) and return its id.
    pub fn start_assistant_message(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.messages
            .push_back(ChatMessage::start_assistant(id.clone()));
        self.is_streaming = true;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
        id
    }

    /// Append content to the current streaming message.
    pub fn append_to_current_message(&mut self, delta: &str) {
        if let Some(msg) = self.messages.back_mut()
            && msg.is_streaming
        {
            msg.append_content(delta);
        }
    }

    /// Finish the current streaming message and clear streaming state.
    pub fn finish_current_message(&mut self) {
        if let Some(msg) = self.messages.back_mut() {
            msg.finish();
        }
        self.is_streaming = false;
        self.is_processing = false;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
    }

    /// Add a tool call to the current message.
    pub fn add_tool_call(&mut self, id: String, name: String, arguments: String) {
        if let Some(msg) = self.messages.back_mut() {
            msg.add_tool_call(id, name, arguments);
        }
    }

    /// Set the result for a tool call.
    pub fn set_tool_result(&mut self, id: &str, result: String, is_error: bool) {
        for msg in self.messages.iter_mut().rev() {
            msg.set_tool_result(id, result.clone(), is_error);
        }
    }

    /// Save messages for a specific thread.
    pub fn save_thread_messages(&mut self, thread_id: u64, messages: VecDeque<ChatMessage>) {
        self.thread_messages.insert(thread_id, messages);
    }

    /// Whether a request is currently in flight (waiting, thinking, or
    /// streaming). While true, history snapshots from the gateway must not
    /// replace the live view: doing so would drop the in-flight streaming
    /// bubble and clear the busy indicators, making the agent look idle
    /// while it is still working. The gateway sends another snapshot when
    /// the response completes.
    pub fn request_in_flight(&self) -> bool {
        self.is_processing || self.is_streaming || self.is_thinking
    }

    /// Replace the cached messages for a thread with an authoritative history.
    /// If the thread is in the foreground, also refresh the live view.
    pub fn apply_thread_history(&mut self, thread_id: u64, messages: VecDeque<ChatMessage>) {
        self.thread_messages.insert(thread_id, messages.clone());
        if self.foreground_thread_id == Some(thread_id) && !self.request_in_flight() {
            self.messages = messages;
            self.reset_streaming_state();
        }
    }

    /// Replace a thread's messages with canonical history from the gateway.
    pub fn hydrate_thread_messages(
        &mut self,
        thread_id: u64,
        messages: Vec<protocol::types::ChatMessage>,
    ) {
        let hydrated: VecDeque<ChatMessage> =
            messages.into_iter().map(ui_message_from_gateway).collect();
        self.thread_messages.insert(thread_id, hydrated.clone());
        if (self.foreground_thread_id == Some(thread_id) || thread_id == 0)
            && !self.request_in_flight()
        {
            self.messages = hydrated;
            self.reset_streaming_state();
        }
    }

    /// Switch to a different thread, saving the current messages and restoring
    /// the target thread's history.
    pub fn switch_thread(&mut self, target_id: u64) {
        if let Some(current_id) = self.foreground_thread_id
            && !self.messages.is_empty()
        {
            self.thread_messages
                .insert(current_id, self.messages.clone());
        }
        self.messages = self
            .thread_messages
            .get(&target_id)
            .cloned()
            .unwrap_or_default();
        self.reset_streaming_state();
    }

    /// Reset the processing/streaming bookkeeping to idle.
    fn reset_streaming_state(&mut self) {
        self.is_processing = false;
        self.is_streaming = false;
        self.is_thinking = false;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
    }
}

/// Convert a gateway protocol message into a display [`ChatMessage`].
fn ui_message_from_gateway(message: protocol::types::ChatMessage) -> ChatMessage {
    let role = match message.role.as_str() {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "system" => MessageRole::System,
        "tool" => MessageRole::ToolResult,
        _ => MessageRole::Info,
    };

    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: message.display_content(),
        timestamp: chrono::Utc::now(),
        tool_calls: Vec::<ToolCallInfo>::new(),
        is_streaming: false,
    }
}
