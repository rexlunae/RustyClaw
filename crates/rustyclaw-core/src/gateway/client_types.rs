//! Client-facing protocol types for gateway communication.
//!
//! These types represent the higher-level events a client (TUI, desktop,
//! CLI) receives from the gateway, and the commands a client sends to it.
//! They are distinct from the binary frame-level protocol types in
//! [`super::protocol`], which handle the wire format.

use serde::{Deserialize, Serialize};

use crate::user_prompt_types::UserPrompt;

// ── Re-export ────────────────────────────────────────────────────────────────

pub use crate::gateway::protocol::SecretEntryDto;

// ── Events (server → client) ────────────────────────────────────────────────

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
        prompt: UserPrompt,
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

    /// Authoritative, cross-session conversation history for a thread.
    ThreadHistory {
        thread_id: u64,
        ok: bool,
        messages: Vec<crate::gateway::protocol::types::ChatMessage>,
        error: Option<String>,
    },

    /// Error from gateway
    Error { message: String },

    /// Info message
    Info { message: String },

    /// DOM query request — evaluate JS in webview
    DomQuery {
        id: String,
        js: String,
    },

    /// Secrets list result from gateway vault
    SecretsListResult {
        ok: bool,
        entries: Vec<SecretEntryInfo>,
    },

    /// Secrets store result
    SecretsStoreResult {
        ok: bool,
        message: String,
    },

    /// Secrets delete result
    SecretsDeleteResult {
        ok: bool,
        message: Option<String>,
    },

    /// Secrets set policy result
    SecretsSetPolicyResult {
        ok: bool,
        message: Option<String>,
    },
}

// ── Commands (client → server) ──────────────────────────────────────────────

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
        value: crate::user_prompt_types::PromptResponseValue,
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

    /// Request the current thread list
    #[serde(rename = "thread_list")]
    ThreadList,

    /// Request the gateway-persisted history for a thread
    #[serde(rename = "thread_history_request")]
    ThreadHistoryRequest { thread_id: u64 },

    /// Close/delete a thread
    #[serde(rename = "thread_close")]
    ThreadClose { thread_id: u64 },

    /// Rename a thread
    #[serde(rename = "thread_rename")]
    ThreadRename { thread_id: u64, new_label: String },

    /// List secrets
    #[serde(rename = "secrets_list")]
    SecretsList,

    /// Cancel current operation
    #[serde(rename = "cancel")]
    Cancel,

    /// Switch to a different provider/model
    #[serde(rename = "model_switch")]
    ModelSwitch { provider: String, model: String },

    /// Respond to a DOM query
    #[serde(rename = "dom_query_response")]
    DomQueryResponse {
        id: String,
        result: String,
        is_error: bool,
    },

    /// Set the agent display name (persisted to gateway config)
    #[serde(rename = "set_agent_name")]
    SetAgentName { name: String },

    /// Set the working directory for tool execution
    #[serde(rename = "set_working_directory")]
    SetWorkingDirectory { path: String },

    /// Store a secret (API key) in the gateway vault
    #[serde(rename = "secrets_store")]
    SecretsStore { key: String, value: String },

    /// Delete a secret from the gateway vault
    #[serde(rename = "secrets_delete")]
    SecretsDelete { key: String },

    /// Set access policy for a secret
    #[serde(rename = "secrets_set_policy")]
    SecretsSetPolicy {
        name: String,
        policy: String,
        skills: Vec<String>,
    },
}

// ── DTOs ─────────────────────────────────────────────────────────────────────

/// A single secret entry as presented to clients.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecretEntryInfo {
    /// Secret name/key.
    pub name: String,
    /// Human-readable label.
    pub label: String,
    /// Kind/category (api_key, token, username_password, etc.).
    pub kind: String,
    /// Access policy (OPEN, ASK, AUTH, SKILL, DISABLED).
    pub policy: String,
    /// Whether the secret is disabled.
    pub disabled: bool,
}

impl From<SecretEntryDto> for SecretEntryInfo {
    fn from(dto: SecretEntryDto) -> Self {
        Self {
            name: dto.name,
            label: dto.label,
            kind: dto.kind,
            policy: dto.policy,
            disabled: dto.disabled,
        }
    }
}

/// Thread info from gateway (client-facing, simplified view).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadInfoDto {
    pub id: u64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
}
