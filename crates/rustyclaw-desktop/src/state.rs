//! Application state management.
//!
//! Shared UI types (`ChatMessage`, `ToolCallInfo`, `ThreadInfo`,
//! `ConnectionStatus`) live in [`rustyclaw_core::ui`]. This module
//! adds desktop-specific wrappers: the Dioxus-friendly `AppState` struct
//! and the `Theme` enum.

use std::collections::{HashMap, VecDeque};

use rustyclaw_core::ui::{ChatMessage, ConnectionStatus, ThreadInfo};
use rustyclaw_core::user_prompt_types::UserPrompt;
use rustyclaw_view::{PromptAttachment, SecretsDialogData};

/// UI theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    pub fn as_attr(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
}

/// Main application state.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Current connection status
    pub connection: ConnectionStatus,

    /// Gateway URL
    pub gateway_url: String,

    /// Chat messages for the current thread
    pub messages: VecDeque<ChatMessage>,

    /// Per-thread message history (thread_id → messages)
    thread_messages: HashMap<u64, VecDeque<ChatMessage>>,

    /// Current input text
    pub input: String,

    /// Whether we're waiting for a response
    pub is_processing: bool,

    /// Whether the assistant is currently streaming
    pub is_streaming: bool,

    /// Current thinking state (for extended thinking models)
    pub is_thinking: bool,

    /// Active threads/sessions
    pub threads: Vec<ThreadInfo>,

    /// Current foreground thread ID
    pub foreground_thread_id: Option<u64>,

    /// Agent name from hatching
    pub agent_name: Option<String>,

    /// Whether vault is locked
    pub vault_locked: bool,

    /// Whether we need to show hatching dialog
    pub needs_hatching: bool,

    /// Current model name
    pub model: Option<String>,

    /// Current provider name
    pub provider: Option<String>,

    /// Files and directories attached to the next prompt.
    pub prompt_attachments: Vec<PromptAttachment>,

    /// Status messages
    pub status_message: Option<String>,

    /// Whether the sidebar is collapsed.
    pub sidebar_collapsed: bool,

    /// Active UI theme.
    pub theme: Theme,

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

    /// Current secrets dialog data.
    pub secrets_data: SecretsDialogData,

    /// Current working directory path
    pub working_directory: Option<String>,

    /// Available directories for selection (favorites/recent)
    pub available_directories: Vec<rustyclaw_view::DirectoryOption>,

    /// Whether the directory selector is expanded
    pub directory_selector_expanded: bool,

    /// Error message from directory operations if any
    pub directory_selector_error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        let working_directory = std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string());
        let configured_model = rustyclaw_core::config::Config::load(None)
            .ok()
            .and_then(|cfg| cfg.model);
        let provider = configured_model.as_ref().map(|m| m.provider.clone());
        let model = configured_model.and_then(|m| m.model);

        Self {
            connection: ConnectionStatus::Disconnected,
            gateway_url: crate::configured_gateway_url()
                .unwrap_or_else(|| "ssh://127.0.0.1:2222".to_string()),
            messages: VecDeque::new(),
            thread_messages: HashMap::new(),
            input: String::new(),
            is_processing: false,
            is_streaming: false,
            is_thinking: false,
            threads: Vec::new(),
            foreground_thread_id: None,
            agent_name: None,
            vault_locked: false,
            needs_hatching: false,
            model,
            provider,
            prompt_attachments: Vec::new(),
            status_message: None,
            sidebar_collapsed: false,
            theme: Theme::default(),
            pending_tool_approval: None,
            pending_user_prompt: None,
            pending_credential_request: None,
            pending_device_flow: None,
            streaming_chunks: 0,
            streaming_bytes: 0,
            agent_access: false,
            secrets_data: SecretsDialogData::from_vault(Vec::new(), false, false),
            working_directory,
            available_directories: Vec::new(),
            directory_selector_expanded: false,
            directory_selector_error: None,
        }
    }
}

impl AppState {
    /// Add a user message to the conversation.
    pub fn add_user_message(&mut self, content: String) {
        let msg = ChatMessage::user(content);
        self.messages.push_back(msg);
    }

    /// Start a new assistant message (streaming).
    pub fn start_assistant_message(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let msg = ChatMessage::start_assistant(id.clone());
        self.messages.push_back(msg);
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

    /// Finish the current streaming message.
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
            // If this message had the matching tool call, the set was done.
            // We only need to check if it updated, but for simplicity just scan.
        }
    }

    /// Save messages for a specific thread.
    pub fn save_thread_messages(
        &mut self,
        thread_id: u64,
        messages: VecDeque<ChatMessage>,
    ) {
        self.thread_messages.insert(thread_id, messages);
    }

    /// Replace the cached messages for a thread with an authoritative
    /// history from the gateway. If the thread is currently in the
    /// foreground, also refresh the live view.
    pub fn apply_thread_history(
        &mut self,
        thread_id: u64,
        messages: VecDeque<ChatMessage>,
    ) {
        self.thread_messages.insert(thread_id, messages.clone());
        if self.foreground_thread_id == Some(thread_id) {
            self.messages = messages;
        }
    }

    /// Switch to a different thread, saving current messages and
    /// restoring the target thread's history.
    pub fn switch_thread(&mut self, target_id: u64) {
        // Save current thread's messages
        if let Some(current_id) = self.foreground_thread_id {
            if !self.messages.is_empty() {
                self.thread_messages
                    .insert(current_id, self.messages.clone());
            }
        }

        // Restore target thread's messages (or start empty)
        self.messages = self
            .thread_messages
            .get(&target_id)
            .cloned()
            .unwrap_or_default();

        // Reset streaming state
        self.is_processing = false;
        self.is_streaming = false;
        self.is_thinking = false;
        self.streaming_chunks = 0;
        self.streaming_bytes = 0;
    }
}
