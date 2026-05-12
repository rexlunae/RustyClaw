//! Protocol types for gateway communication.

use serde::{Deserialize, Serialize};

/// Events received from the gateway.
#[derive(Clone, Debug)]
pub enum GatewayEvent {
    /// Connected to gateway
    Connected {
        agent: Option<String>,
        vault_locked: bool,
        provider: Option<String>,
        model: Option<String>,
    },

    /// Disconnected from gateway
    Disconnected { reason: Option<String> },

    /// Authentication required
    AuthRequired,

    /// Authentication succeeded
    AuthSuccess,

    /// Authentication failed
    AuthFailed { message: String, retry: bool },

    /// Vault needs unlocking
    VaultLocked,

    /// Vault unlocked successfully
    VaultUnlocked,

    /// Model is ready
    ModelReady { model: String },

    /// Model error
    ModelError { message: String },

    /// Stream starting
    StreamStart,

    /// Thinking started (extended thinking)
    ThinkingStart,

    /// Thinking ended
    ThinkingEnd,

    /// Text chunk received
    Chunk { delta: String },

    /// Response complete
    ResponseDone,

    /// Tool call initiated
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },

    /// Tool call result
    ToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },

    /// Tool approval request
    ToolApprovalRequest {
        id: String,
        name: String,
        arguments: String,
    },

    /// User prompt request (agent asking for user input)
    UserPromptRequest {
        #[allow(dead_code)]
        id: String,
        prompt: rustyclaw_core::user_prompt_types::UserPrompt,
    },

    /// Credential request (gateway needs an API key/token)
    CredentialRequest {
        id: String,
        provider: String,
        secret_name: String,
        message: String,
    },

    /// Device flow started (OAuth)
    DeviceFlowStart {
        url: String,
        code: String,
        message: Option<String>,
    },

    /// Device flow completed
    DeviceFlowComplete,

    /// Threads/sessions updated
    ThreadsUpdate {
        threads: Vec<ThreadInfoDto>,
        foreground_id: Option<u64>,
    },

    /// Error from gateway
    Error { message: String },

    /// Info message
    Info { message: String },
}

/// Thread info from gateway.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadInfoDto {
    pub id: u64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
}

/// Commands to send to the gateway.
#[derive(Clone, Debug, Serialize)]
#[allow(dead_code)]
#[serde(tag = "type")]
pub enum GatewayCommand {
    /// Send a chat message
    #[serde(rename = "chat")]
    Chat { message: String },

    /// Authenticate with TOTP code
    #[serde(rename = "auth")]
    Auth { code: String },

    /// Unlock vault with password
    #[serde(rename = "vault_unlock")]
    VaultUnlock { password: String },

    /// Approve tool call
    #[serde(rename = "tool_approve")]
    ToolApprove { id: String, approved: bool },

    /// Respond to a user prompt
    #[serde(rename = "user_prompt_response")]
    UserPromptResponse {
        id: String,
        dismissed: bool,
        value: rustyclaw_core::user_prompt_types::PromptResponseValue,
    },

    /// Respond to a credential request
    #[serde(rename = "credential_response")]
    CredentialResponse {
        id: String,
        dismissed: bool,
        value: Option<String>,
    },

    /// Switch to a thread
    #[serde(rename = "thread_switch")]
    ThreadSwitch { thread_id: u64 },

    /// Create a new thread
    #[serde(rename = "thread_create")]
    ThreadCreate { label: Option<String> },

    /// List secrets
    #[serde(rename = "secrets_list")]
    SecretsList,

    /// Cancel current operation
    #[serde(rename = "cancel")]
    Cancel,
}
