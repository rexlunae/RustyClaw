//! Frame types and serialization for gateway protocol.
//!
//! This module contains the shared types used by both client and server.

use serde::{Deserialize, Serialize};

/// Incoming frame types from client to gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ClientFrameType {
    /// Authentication response with TOTP code.
    AuthResponse = 0,
    /// Unlock the vault with password.
    UnlockVault = 1,
    /// List all secrets.
    SecretsList = 2,
    /// Get a specific secret.
    SecretsGet = 3,
    /// Store a secret.
    SecretsStore = 4,
    /// Delete a secret.
    SecretsDelete = 5,
    /// Peek at a credential (display without exposing value).
    SecretsPeek = 6,
    /// Set access policy for a credential.
    SecretsSetPolicy = 7,
    /// Enable/disable a credential.
    SecretsSetDisabled = 8,
    /// Delete a credential entirely.
    SecretsDeleteCredential = 9,
    /// Check if TOTP is configured.
    SecretsHasTotp = 10,
    /// Set up TOTP for the vault.
    SecretsSetupTotp = 11,
    /// Verify a TOTP code.
    SecretsVerifyTotp = 12,
    /// Remove TOTP from the vault.
    SecretsRemoveTotp = 13,
    /// Reload configuration.
    Reload = 14,
    /// Cancel the current tool loop.
    Cancel = 15,
    /// Chat message (default).
    Chat = 16,
    /// User response to a tool approval request.
    ToolApprovalResponse = 17,
    /// User response to a structured prompt (ask_user tool).
    UserPromptResponse = 18,
    /// Request current task list.
    TasksRequest = 19,
    /// Create a new thread.
    ThreadCreate = 20,
    /// Switch to a different thread.
    ThreadSwitch = 21,
    /// Request thread list.
    ThreadList = 22,
    /// Close/delete a thread.
    ThreadClose = 23,
    /// Rename a thread.
    ThreadRename = 24,
    /// User response to a credential request.
    CredentialResponse = 25,
    /// Switch to a different provider/model.
    ModelSwitch = 26,
    /// Response to a DOM query request.
    DomQueryResponse = 27,
    /// Set the agent display name (persisted to config).
    SetAgentName = 28,
    /// Set the working directory for tool execution.
    SetWorkingDirectory = 29,
    /// Request the persisted conversation history for a thread.
    ThreadHistoryRequest = 30,
    /// Request the current project list.
    ProjectList = 31,
    /// Create a new project (a named working directory).
    ProjectCreate = 32,
    /// Rename a project.
    ProjectRename = 33,
    /// Delete a project.
    ProjectDelete = 34,
    /// Switch the active project.
    ProjectSwitch = 35,
    /// Request host hardware capabilities.
    HostInfoRequest = 36,
    /// Request current system load status.
    LoadStatusRequest = 37,
}

/// Outgoing frame types from gateway to client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ServerFrameType {
    /// Authentication challenge request.
    AuthChallenge = 0,
    /// Authentication result.
    AuthResult = 1,
    /// Too many auth attempts, locked out.
    AuthLocked = 2,
    /// Hello message on connect.
    Hello = 3,
    /// Status update frame.
    Status = 4,
    /// Vault unlocked result.
    VaultUnlocked = 5,
    /// Secrets list result.
    SecretsListResult = 6,
    /// Secrets store result.
    SecretsStoreResult = 7,
    /// Secrets get result.
    SecretsGetResult = 8,
    /// Secrets delete result.
    SecretsDeleteResult = 9,
    /// Secrets peek result.
    SecretsPeekResult = 10,
    /// Secrets set policy result.
    SecretsSetPolicyResult = 11,
    /// Secrets set disabled result.
    SecretsSetDisabledResult = 12,
    /// Secrets delete credential result.
    SecretsDeleteCredentialResult = 13,
    /// Secrets has TOTP result.
    SecretsHasTotpResult = 14,
    /// Secrets setup TOTP result.
    SecretsSetupTotpResult = 15,
    /// Secrets verify TOTP result.
    SecretsVerifyTotpResult = 16,
    /// Secrets remove TOTP result.
    SecretsRemoveTotpResult = 17,
    /// Reload result.
    ReloadResult = 18,
    /// Error frame.
    Error = 19,
    /// Info frame.
    Info = 20,
    /// Stream start.
    StreamStart = 21,
    /// Chunk of response text.
    Chunk = 22,
    /// Thinking start (for extended thinking).
    ThinkingStart = 23,
    /// Thinking delta (streaming thinking content).
    ThinkingDelta = 24,
    /// Thinking end.
    ThinkingEnd = 25,
    /// Tool call from model.
    ToolCall = 26,
    /// Tool result from execution.
    ToolResult = 27,
    /// Response complete.
    ResponseDone = 28,
    /// Tool approval request — ask user to approve a tool call.
    ToolApprovalRequest = 29,
    /// Structured user prompt request (ask_user tool).
    UserPromptRequest = 30,
    /// Task list update.
    TasksUpdate = 31,
    /// Thread list update.
    ThreadsUpdate = 32,
    /// Thread created result.
    ThreadCreated = 33,
    /// Thread switched result.
    ThreadSwitched = 34,
    /// Credential request — gateway needs the user to provide an API key or credential.
    CredentialRequest = 35,
    /// Device flow started — gateway is running OAuth device flow; show URL + code to user.
    DeviceFlowStart = 36,
    /// Device flow completed — dismiss the device flow dialog.
    DeviceFlowComplete = 37,
    /// DOM query — gateway requests the client to evaluate JavaScript
    /// against the webview DOM and return the result.
    DomQuery = 38,
    /// Reply carrying a thread's persisted conversation history.
    ThreadHistoryReply = 39,

    /// Thread messages/history update.
    ThreadMessages = 40,
    /// Project list update.
    ProjectsUpdate = 41,
    /// Host info result.
    HostInfoResult = 42,
    /// Load status result.
    LoadStatusResult = 43,
}

/// Status frame sub-types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum StatusType {
    /// Model is configured.
    ModelConfigured = 0,
    /// Credentials loaded.
    CredentialsLoaded = 1,
    /// Credentials missing.
    CredentialsMissing = 2,
    /// Model connecting.
    ModelConnecting = 3,
    /// Model ready.
    ModelReady = 4,
    /// Model error.
    ModelError = 5,
    /// No model configured.
    NoModel = 6,
    /// Vault is locked.
    VaultLocked = 7,
}

// ============================================================================
// Binary Frame Types
// ============================================================================

/// Protocol version for multiplexed SSH/stdin wire envelopes.
pub const WIRE_PROTOCOL_VERSION: u16 = 1;

/// Stream ID used for connection-level control frames.
pub const CONTROL_STREAM_ID: u64 = 0;

/// A multiplexed wire envelope around an application frame.
///
/// The outer SSH/stdin transport still uses a length prefix. The payload inside
/// that length prefix is this bincode-serialized envelope, which gives each
/// logical request/response flow an independent stream ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireFrame<T> {
    pub version: u16,
    pub stream_id: u64,
    pub sequence: u64,
    pub flags: u16,
    pub frame: T,
}

impl<T> WireFrame<T> {
    pub fn new(stream_id: u64, frame: T) -> Self {
        Self {
            version: WIRE_PROTOCOL_VERSION,
            stream_id,
            sequence: 0,
            flags: 0,
            frame,
        }
    }

    pub fn control(frame: T) -> Self {
        Self::new(CONTROL_STREAM_ID, frame)
    }
}

/// Generic client frame envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFrame {
    pub frame_type: ClientFrameType,
    pub payload: ClientPayload,
}

/// Payload variants for client frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientPayload {
    Empty,
    AuthChallenge {
        method: String,
    },
    AuthResponse {
        code: String,
    },
    UnlockVault {
        password: String,
    },
    Reload,
    Chat {
        messages: Vec<super::types::ChatMessage>,
    },
    SecretsList,
    SecretsGet {
        key: String,
    },
    SecretsStore {
        key: String,
        value: String,
    },
    SecretsDelete {
        key: String,
    },
    SecretsPeek {
        name: String,
    },
    SecretsSetPolicy {
        name: String,
        policy: String,
        skills: Vec<String>,
    },
    SecretsSetDisabled {
        name: String,
        disabled: bool,
    },
    SecretsDeleteCredential {
        name: String,
    },
    SecretsHasTotp,
    SecretsSetupTotp,
    SecretsVerifyTotp {
        code: String,
    },
    SecretsRemoveTotp,
    ToolApprovalResponse {
        id: String,
        approved: bool,
    },
    UserPromptResponse {
        id: String,
        dismissed: bool,
        value: crate::user_prompt_types::PromptResponseValue,
    },
    /// Request current task list (optionally filtered by session).
    TasksRequest {
        session: Option<String>,
    },
    /// Create a new thread. `project_id` of 0 means "the active project".
    ThreadCreate {
        label: String,
        project_id: u64,
    },
    /// Switch to a different thread.
    ThreadSwitch {
        thread_id: u64,
    },
    /// Request list of threads.
    ThreadList,
    /// Close/delete a thread.
    ThreadClose {
        thread_id: u64,
    },
    /// Rename a thread.
    ThreadRename {
        thread_id: u64,
        new_label: String,
    },
    /// User response to a credential request.
    CredentialResponse {
        /// Matches the `id` from `CredentialRequest`.
        id: String,
        /// Whether the user dismissed the request without providing a value.
        dismissed: bool,
        /// The credential value (API key, token, etc.).
        value: Option<String>,
    },
    /// Switch to a different provider/model.
    ModelSwitch {
        provider: String,
        model: String,
    },
    /// Response to a DOM query.
    DomQueryResponse {
        /// Matches the `id` from the `DomQuery` server frame.
        id: String,
        /// The JSON-serialised result of evaluating the JS expression.
        result: String,
        /// `true` if the evaluation threw an error.
        is_error: bool,
    },
    /// Set the agent display name (persisted to config).
    SetAgentName {
        name: String,
    },
    /// Set the working directory for all tool execution.
    SetWorkingDirectory {
        path: String,
    },
    /// Request the gateway-persisted conversation history for a thread.
    ThreadHistoryRequest {
        thread_id: u64,
    },
    // ── Projects ──────────────────────────────────────────────────────────
    // Appended at the end: `ClientPayload` is encoded positionally (bincode),
    // so new variants must not shift existing discriminants.
    /// Request the current project list.
    ProjectList,
    /// Create a new project (a named working directory).
    ProjectCreate {
        name: String,
        path: String,
    },
    /// Rename a project.
    ProjectRename {
        project_id: u64,
        new_name: String,
    },
    /// Delete a project.
    ProjectDelete {
        project_id: u64,
    },
    /// Switch the active project.
    ProjectSwitch {
        project_id: u64,
    },
    /// Request host hardware capabilities.
    HostInfoRequest,
    /// Request current system load status.
    LoadStatusRequest,
}

/// Generic server frame envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerFrame {
    pub frame_type: ServerFrameType,
    pub payload: ServerPayload,
}

/// Payload variants for server frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerPayload {
    Empty,
    Hello {
        agent: String,
        settings_dir: String,
        vault_locked: bool,
        provider: Option<String>,
        model: Option<String>,
    },
    AuthChallenge {
        method: String,
    },
    AuthResult {
        ok: bool,
        message: Option<String>,
        retry: Option<bool>,
    },
    AuthLocked {
        message: String,
        retry_after: Option<u64>,
    },
    Status {
        status: StatusType,
        detail: String,
    },
    VaultUnlocked {
        ok: bool,
        message: Option<String>,
    },
    SecretsListResult {
        ok: bool,
        entries: Vec<SecretEntryDto>,
    },
    SecretsStoreResult {
        ok: bool,
        message: String,
    },
    SecretsGetResult {
        ok: bool,
        key: String,
        value: Option<String>,
        message: Option<String>,
    },
    SecretsDeleteResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsPeekResult {
        ok: bool,
        fields: Vec<(String, String)>,
        message: Option<String>,
    },
    SecretsSetPolicyResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsSetDisabledResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsDeleteCredentialResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsHasTotpResult {
        has_totp: bool,
    },
    SecretsSetupTotpResult {
        ok: bool,
        uri: Option<String>,
        message: Option<String>,
    },
    SecretsVerifyTotpResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsRemoveTotpResult {
        ok: bool,
        message: Option<String>,
    },
    ReloadResult {
        ok: bool,
        provider: String,
        model: String,
        message: Option<String>,
    },
    Error {
        ok: bool,
        message: String,
    },
    Info {
        message: String,
    },
    StreamStart,
    Chunk {
        delta: String,
    },
    ThinkingStart,
    ThinkingDelta {
        delta: String,
    },
    ThinkingEnd,
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },
    ResponseDone {
        ok: bool,
    },
    ToolApprovalRequest {
        id: String,
        name: String,
        arguments: String,
    },
    UserPromptRequest {
        id: String,
        prompt: crate::user_prompt_types::UserPrompt,
    },
    TasksUpdate {
        tasks: Vec<TaskInfoDto>,
    },
    ThreadsUpdate {
        threads: Vec<ThreadInfoDto>,
        foreground_id: Option<u64>,
    },
    ThreadCreated {
        thread_id: u64,
        label: String,
    },
    ThreadSwitched {
        thread_id: u64,
        /// Optional summary of the thread being switched to
        context_summary: Option<String>,
    },
    /// Credential request — the gateway detected an auth failure and needs the
    /// user to supply an API key or other credential.
    CredentialRequest {
        /// Unique request ID.
        id: String,
        /// Provider that needs the credential (e.g. "anthropic", "openai").
        provider: String,
        /// Name of the secret/env-var the credential will be stored as.
        secret_name: String,
        /// Human-readable message explaining what is needed.
        message: String,
    },
    /// Device flow started — the gateway is running an OAuth device flow and
    /// needs the user to visit a URL and enter a code.
    DeviceFlowStart {
        /// Verification URL the user should open.
        url: String,
        /// One-time code to enter at that URL.
        code: String,
        /// Optional error message from the provider that triggered the flow.
        /// Do NOT use skip_serializing_if here — bincode is positional.
        message: Option<String>,
    },
    /// Device flow completed — the gateway obtained the token; dismiss the dialog.
    DeviceFlowComplete,
    /// DOM query — evaluate a JavaScript expression in the webview and
    /// return the result via `DomQueryResponse`.
    DomQuery {
        /// Unique request ID.
        id: String,
        /// JavaScript expression to evaluate.
        js: String,
    },

    /// Reply to a `ThreadHistoryRequest` — the full persisted message
    /// log for a thread, in chronological order, in the wire `ChatMessage`
    /// shape suitable for re-display by any client.
    ThreadHistoryReply {
        thread_id: u64,
        ok: bool,
        messages: Vec<super::types::ChatMessage>,
        error: Option<String>,
    },

    /// Live message/history update for a thread.
    ThreadMessages {
        thread_id: u64,
        messages: Vec<super::types::ChatMessage>,
    },
    /// Project list update (appended — positional encoding, see `ClientPayload`).
    ProjectsUpdate {
        projects: Vec<ProjectInfoDto>,
        active_id: u64,
    },
    /// Host hardware capabilities result.
    HostInfoResult {
        hostname: String,
        os: String,
        arch: String,
        cpu_brand: String,
        cpu_cores_physical: usize,
        cpu_cores_logical: usize,
        cpu_frequency_mhz: u64,
        total_memory_bytes: u64,
        total_swap_bytes: u64,
        disk_total_bytes: u64,
        disk_available_bytes: u64,
        gpus: Vec<GpuInfoDto>,
        summary: String,
    },
    /// Current system load status result.
    LoadStatusResult {
        load_score: f64,
        avg_load_score: f64,
        cpu_percent: f32,
        memory_percent: f32,
        summary: String,
    },
}

/// DTO for GPU info in host capabilities results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuInfoDto {
    pub name: String,
    pub vendor: String,
    pub vram_bytes: u64,
}

/// DTO for task info in updates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskInfoDto {
    pub id: u64,
    pub label: String,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
}

/// DTO for thread info in updates (unified tasks + threads).
/// NOTE: Do NOT use skip_serializing_if with bincode - it breaks deserialization
/// since bincode is not self-describing (positional format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadInfoDto {
    pub id: u64,
    pub label: String,
    /// Description (for spawned tasks)
    pub description: Option<String>,
    /// Task status (None = simple thread, Some = spawned task)
    pub status: Option<String>,
    /// Icon for the thread kind (e.g. chat, sub-agent, background, task)
    pub kind_icon: Option<String>,
    /// Icon for the thread status (e.g. running, completed, failed)
    pub status_icon: Option<String>,
    pub is_foreground: bool,
    pub message_count: usize,
    pub has_summary: bool,
    /// Project this thread belongs to. Appended last (positional bincode
    /// encoding); 0 / absent maps to the Default project.
    #[serde(default)]
    pub project_id: u64,
}

/// DTO for project info in `ProjectsUpdate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfoDto {
    pub id: u64,
    pub name: String,
    pub path: String,
}

/// DTO for secret entries in list results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretEntryDto {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub policy: String,
    pub disabled: bool,
}

// ============================================================================
// Serialization
// ============================================================================

/// Serialize a frame to binary using bincode with serde.
pub fn serialize_frame<T: serde::Serialize>(frame: &T) -> Result<Vec<u8>, String> {
    bincode::serde::encode_to_vec(frame, bincode::config::standard()).map_err(|e| e.to_string())
}

/// Maximum frame payload size accepted by deserialization (defense-in-depth).
///
/// The SSH/stdin transport already enforces this limit, but we check here too
/// so that any future transport that forgets the check cannot OOM the process.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024; // 16 MB

/// Deserialize a frame from binary using bincode with serde.
pub fn deserialize_frame<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(format!(
            "Frame too large: {} bytes (max {})",
            bytes.len(),
            MAX_FRAME_SIZE,
        ));
    }
    let (result, _) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map_err(|e| e.to_string())?;
    Ok(result)
}

/// Serialize a multiplexed wire frame.
pub fn serialize_wire_frame<T: serde::Serialize>(frame: &WireFrame<T>) -> Result<Vec<u8>, String> {
    serialize_frame(frame)
}

/// Deserialize a multiplexed wire frame.
pub fn deserialize_wire_frame<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<WireFrame<T>, String> {
    deserialize_frame(bytes)
}

#[cfg(test)]
mod frame_size_tests;
#[cfg(test)]
mod tests;
