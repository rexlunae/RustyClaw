//! Application state management.

use std::collections::VecDeque;

/// Connection status to the gateway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Authenticating,
    Authenticated,
    Error(String),
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::Disconnected
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// Information about a tool call.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<String>,
    pub is_error: bool,
    pub collapsed: bool,
}

/// Thread/session info.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadInfo {
    pub id: u64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
}

/// Main application state.
#[derive(Clone, Debug)]
pub struct AppState {
    /// Current connection status
    pub connection: ConnectionStatus,

    /// Gateway URL
    pub gateway_url: String,

    /// Chat messages
    pub messages: VecDeque<ChatMessage>,

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

    /// Status messages
    pub status_message: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            connection: ConnectionStatus::Disconnected,
            gateway_url: "ws://127.0.0.1:9001".to_string(),
            messages: VecDeque::new(),
            input: String::new(),
            is_processing: false,
            is_streaming: false,
            is_thinking: false,
            threads: Vec::new(),
            foreground_thread_id: None,
            agent_name: None,
            vault_locked: false,
            needs_hatching: false,
            model: None,
            provider: None,
            status_message: None,
        }
    }
}

impl AppState {
    /// Add a user message to the conversation.
    pub fn add_user_message(&mut self, content: String) {
        let msg = ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: MessageRole::User,
            content,
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: false,
        };
        self.messages.push_back(msg);
    }

    /// Start a new assistant message (streaming).
    pub fn start_assistant_message(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let msg = ChatMessage {
            id: id.clone(),
            role: MessageRole::Assistant,
            content: String::new(),
            timestamp: chrono::Utc::now(),
            tool_calls: Vec::new(),
            is_streaming: true,
        };
        self.messages.push_back(msg);
        self.is_streaming = true;
        id
    }

    /// Append content to the current streaming message.
    pub fn append_to_current_message(&mut self, delta: &str) {
        if let Some(msg) = self.messages.back_mut() {
            if msg.is_streaming {
                msg.content.push_str(delta);
            }
        }
    }

    /// Finish the current streaming message.
    pub fn finish_current_message(&mut self) {
        if let Some(msg) = self.messages.back_mut() {
            msg.is_streaming = false;
        }
        self.is_streaming = false;
        self.is_processing = false;
    }

    /// Add a tool call to the current message.
    pub fn add_tool_call(&mut self, id: String, name: String, arguments: String) {
        if let Some(msg) = self.messages.back_mut() {
            msg.tool_calls.push(ToolCallInfo {
                id,
                name,
                arguments,
                result: None,
                is_error: false,
                collapsed: true,
            });
        }
    }

    /// Set the result for a tool call.
    pub fn set_tool_result(&mut self, id: &str, result: String, is_error: bool) {
        for msg in self.messages.iter_mut().rev() {
            for tool in &mut msg.tool_calls {
                if tool.id == id {
                    tool.result = Some(result);
                    tool.is_error = is_error;
                    return;
                }
            }
        }
    }

    /// Clear all messages.
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }
}
