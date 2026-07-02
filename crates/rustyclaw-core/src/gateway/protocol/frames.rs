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
    /// Request list of managed services.
    ServiceListRequest = 38,
    /// Start a managed service.
    ServiceStartRequest = 39,
    /// Stop a managed service.
    ServiceStopRequest = 40,
    /// Restart a managed service.
    ServiceRestartRequest = 41,
    /// Request logs for a managed service.
    ServiceLogsRequest = 42,
    /// Request cron job list.
    CronListRequest = 43,
    /// Create or update a cron job.
    CronUpsertRequest = 44,
    /// Perform an action on a cron job (pause/resume/run/remove).
    CronActionRequest = 45,
    /// Request memory entries (search/list).
    MemoryListRequest = 46,
    /// Create or update a memory entry.
    MemoryUpsertRequest = 47,
    /// Delete a memory entry.
    MemoryDeleteRequest = 48,
    /// Search conversation history.
    HistorySearchRequest = 49,
    /// Request usage/analytics stats.
    UsageStatsRequest = 50,
    /// Request general logs (agent/gateway/cron).
    LogsRequest = 51,
    /// Request MCP server list.
    McpListRequest = 52,
    /// Connect an MCP server.
    McpConnectRequest = 53,
    /// Disconnect an MCP server.
    McpDisconnectRequest = 54,
    /// Request tool configuration.
    ToolConfigRequest = 55,
    /// Toggle a tool's enabled state.
    ToolToggleRequest = 56,
    /// Request channel/messenger status.
    ChannelStatusRequest = 57,
    /// Pair/unpair a channel.
    ChannelPairRequest = 58,
    /// Request pending approvals list.
    PendingApprovalsRequest = 59,
    /// Batch approve/deny pending approvals.
    ApprovalsBatchAction = 60,
    /// Start voice recording (STT).
    VoiceStart = 61,
    /// Stop voice recording.
    VoiceStop = 62,
    /// Send voice audio chunk.
    VoiceAudioChunk = 63,
    /// Request file preview.
    PreviewRequest = 64,
    /// Toggle file-follow mode.
    PreviewFollowToggle = 65,
    /// List all local engines and their status.
    EngineList = 66,
    /// Perform an engine action (install/start/stop).
    EngineAction = 67,
    /// List models for a specific engine.
    EngineModelList = 68,
    /// Pull/download a model for an engine.
    EngineModelPull = 69,
    /// Perform a model action (remove/load/unload).
    EngineModelAction = 70,
    /// Set per-engine configuration.
    EngineConfigSet = 71,
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
    /// Service list result.
    ServiceListResult = 44,
    /// Service start result.
    ServiceStartResult = 45,
    /// Service stop result.
    ServiceStopResult = 46,
    /// Service restart result.
    ServiceRestartResult = 47,
    /// Service logs result.
    ServiceLogsResult = 48,
    /// Cron job list result.
    CronListResult = 49,
    /// Cron upsert result.
    CronUpsertResult = 50,
    /// Cron action result.
    CronActionResult = 51,
    /// Memory list/search result.
    MemoryListResult = 52,
    /// Memory upsert result.
    MemoryUpsertResult = 53,
    /// Memory delete result.
    MemoryDeleteResult = 54,
    /// History search result.
    HistorySearchResult = 55,
    /// Usage/analytics stats result.
    UsageStatsResult = 56,
    /// General logs result.
    LogsResult = 57,
    /// Streaming log append (for follow/tail).
    LogsAppend = 58,
    /// MCP server list result.
    McpListResult = 59,
    /// MCP connect result.
    McpConnectResult = 60,
    /// MCP disconnect result.
    McpDisconnectResult = 61,
    /// Tool configuration result.
    ToolConfigResult = 62,
    /// Tool toggle result.
    ToolToggleResult = 63,
    /// Channel status result.
    ChannelStatusResult = 64,
    /// Channel pair result.
    ChannelPairResult = 65,
    /// Pending approvals list result.
    PendingApprovalsResult = 66,
    /// Approvals batch action result.
    ApprovalsBatchResult = 67,
    /// Streaming tool output start.
    ToolOutputStart = 68,
    /// Streaming tool output delta.
    ToolOutputDelta = 69,
    /// Streaming tool output end.
    ToolOutputEnd = 70,
    /// Voice transcription result (STT).
    VoiceTranscript = 71,
    /// Voice state update.
    VoiceStateUpdate = 72,
    /// TTS audio chunk for playback.
    VoiceTtsChunk = 73,
    /// File preview result.
    PreviewResult = 74,
    /// File preview update (file-follow).
    PreviewUpdate = 75,
    /// Tool result with attached media.
    ToolResultMedia = 76,
    /// Engine list result.
    EngineListResult = 77,
    /// Engine model list result.
    EngineModelListResult = 78,
    /// Engine pull progress (streamed).
    EnginePullProgress = 79,
    /// Engine action result (start/stop/install/remove/load/unload).
    EngineActionResult = 80,
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

/// Action verbs for [`ClientPayload::CronActionRequest`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum CronActionKind {
    Pause,
    Resume,
    Run,
    Remove,
}

/// Action verbs for [`ClientPayload::ChannelPairRequest`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChannelPairActionKind {
    Pair,
    Unpair,
}

/// Action verbs for [`ClientPayload::EngineAction`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EngineActionKind {
    Install,
    Start,
    Stop,
}

/// Action verbs for [`ClientPayload::EngineModelAction`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ModelActionKind {
    Load,
    Unload,
    Remove,
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
    /// Request list of managed services.
    ServiceListRequest,
    /// Start a managed service by name.
    ServiceStartRequest {
        name: String,
    },
    /// Stop a managed service by name.
    ServiceStopRequest {
        name: String,
    },
    /// Restart a managed service by name.
    ServiceRestartRequest {
        name: String,
    },
    /// Request recent logs for a managed service.
    ServiceLogsRequest {
        name: String,
        tail: Option<usize>,
    },
    // ── Cron ──────────────────────────────────────────────────────────────
    /// Request cron job list.
    CronListRequest,
    /// Create or update a cron job.
    CronUpsertRequest {
        id: Option<String>,
        name: String,
        expr: String,
        payload: String,
        paused: bool,
    },
    /// Perform an action on a cron job.
    CronActionRequest {
        id: String,
        action: CronActionKind,
    },
    // ── Memory ────────────────────────────────────────────────────────────
    /// Request memory entries.
    MemoryListRequest {
        query: Option<String>,
        limit: Option<usize>,
    },
    /// Create or update a memory entry.
    MemoryUpsertRequest {
        id: Option<String>,
        content: String,
        category: Option<String>,
    },
    /// Delete a memory entry.
    MemoryDeleteRequest {
        id: String,
    },
    /// Search conversation history.
    HistorySearchRequest {
        query: String,
        limit: Option<usize>,
    },
    // ── Analytics ─────────────────────────────────────────────────────────
    /// Request usage/analytics stats.
    UsageStatsRequest {
        period: Option<String>, // "day" | "week" | "month" | "all"
    },
    // ── Logs ──────────────────────────────────────────────────────────────
    /// Request general logs.
    LogsRequest {
        source: String, // "gateway" | "agent" | "cron" | service name
        tail: Option<usize>,
        follow: bool,
    },
    // ── MCP ───────────────────────────────────────────────────────────────
    /// Request MCP server list.
    McpListRequest,
    /// Connect an MCP server.
    McpConnectRequest {
        name: String,
        command: Option<String>,
        url: Option<String>,
        env: Vec<(String, String)>,
    },
    /// Disconnect an MCP server.
    McpDisconnectRequest {
        name: String,
    },
    // ── Tool Config ───────────────────────────────────────────────────────
    /// Request tool configuration.
    ToolConfigRequest,
    /// Toggle a tool's enabled state.
    ToolToggleRequest {
        tool_name: String,
        enabled: bool,
    },
    // ── Channels ──────────────────────────────────────────────────────────
    /// Request channel/messenger status.
    ChannelStatusRequest,
    /// Pair/unpair a channel.
    ChannelPairRequest {
        channel: String,
        action: ChannelPairActionKind,
    },
    // ── Approvals ─────────────────────────────────────────────────────────
    /// Request pending approvals list.
    PendingApprovalsRequest,
    /// Batch approve/deny pending approvals.
    ApprovalsBatchAction {
        ids: Vec<String>,
        approved: bool,
        always_allow: bool,
    },
    // ── Voice ────────────────────────────────────────────────────────────
    /// Start voice recording.
    VoiceStart {
        /// Preferred language code (e.g. "en-US").
        language: Option<String>,
    },
    /// Stop voice recording.
    VoiceStop,
    /// Audio chunk (PCM/opus bytes).
    VoiceAudioChunk {
        data: Vec<u8>,
    },
    // ── Preview ──────────────────────────────────────────────────────────
    /// Request file preview.
    PreviewRequest {
        path: String,
    },
    /// Toggle file-follow mode for the current preview.
    PreviewFollowToggle {
        path: String,
        follow: bool,
    },
    // ── Engines ──────────────────────────────────────────────────────────
    /// List all local engines and their status.
    EngineList,
    /// Perform an engine action (install/start/stop).
    EngineAction {
        engine: String,
        action: EngineActionKind,
    },
    /// List models for a specific engine.
    EngineModelList {
        engine: String,
    },
    /// Pull/download a model.
    EngineModelPull {
        engine: String,
        model: String,
        /// Optional expected size hint for disk pre-flight (bytes).
        #[serde(default)]
        expected_size_bytes: Option<u64>,
    },
    /// Perform a model action (remove/load/unload).
    EngineModelAction {
        engine: String,
        model: String,
        action: ModelActionKind,
        /// Per-model knobs: context length override for load.
        #[serde(default)]
        context_length: Option<u32>,
        /// Per-model knobs: extra engine-specific args for load.
        #[serde(default)]
        extra_args: Vec<String>,
    },
    /// Set per-engine configuration.
    EngineConfigSet {
        engine: String,
        config: crate::engines::EngineConfig,
    },
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
    /// Service list result.
    ServiceListResult {
        services: Vec<ServiceInfoDto>,
    },
    /// Service action result (start/stop/restart).
    ServiceActionResult {
        ok: bool,
        service: Option<ServiceInfoDto>,
        message: Option<String>,
    },
    /// Service logs result.
    ServiceLogsResult {
        ok: bool,
        name: String,
        lines: Vec<String>,
        message: Option<String>,
    },
    // ── Cron ──────────────────────────────────────────────────────────────
    CronListResult {
        jobs: Vec<CronJobDto>,
    },
    CronUpsertResult {
        ok: bool,
        job: Option<CronJobDto>,
        message: Option<String>,
    },
    CronActionResult {
        ok: bool,
        message: Option<String>,
    },
    // ── Memory ────────────────────────────────────────────────────────────
    MemoryListResult {
        entries: Vec<MemoryEntryDto>,
    },
    MemoryUpsertResult {
        ok: bool,
        id: Option<String>,
        message: Option<String>,
    },
    MemoryDeleteResult {
        ok: bool,
        message: Option<String>,
    },
    HistorySearchResult {
        entries: Vec<HistoryEntryDto>,
    },
    // ── Analytics ─────────────────────────────────────────────────────────
    UsageStatsResult {
        totals: UsageTotalsDto,
        per_model: Vec<ModelUsageDto>,
        per_session: Vec<SessionUsageDto>,
    },
    // ── Logs ──────────────────────────────────────────────────────────────
    LogsResult {
        ok: bool,
        source: String,
        lines: Vec<String>,
        message: Option<String>,
    },
    LogsAppend {
        source: String,
        lines: Vec<String>,
    },
    // ── MCP ───────────────────────────────────────────────────────────────
    McpListResult {
        servers: Vec<McpServerDto>,
    },
    McpConnectResult {
        ok: bool,
        server: Option<McpServerDto>,
        message: Option<String>,
    },
    McpDisconnectResult {
        ok: bool,
        message: Option<String>,
    },
    // ── Tool Config ───────────────────────────────────────────────────────
    ToolConfigResult {
        tools: Vec<ToolConfigDto>,
    },
    ToolToggleResult {
        ok: bool,
        message: Option<String>,
    },
    // ── Channels ──────────────────────────────────────────────────────────
    ChannelStatusResult {
        channels: Vec<ChannelStatusDto>,
    },
    ChannelPairResult {
        ok: bool,
        channel: Option<ChannelStatusDto>,
        message: Option<String>,
    },
    // ── Approvals ─────────────────────────────────────────────────────────
    PendingApprovalsResult {
        approvals: Vec<PendingApprovalDto>,
    },
    ApprovalsBatchResult {
        ok: bool,
        message: Option<String>,
    },
    // ── Streaming tool output ─────────────────────────────────────────────
    ToolOutputStart {
        tool_id: String,
        name: String,
    },
    ToolOutputDelta {
        tool_id: String,
        chunk: String,
        is_stderr: bool,
    },
    ToolOutputEnd {
        tool_id: String,
    },
    // ── Voice ────────────────────────────────────────────────────────────
    /// Transcription result from STT.
    VoiceTranscript {
        text: String,
        is_final: bool,
    },
    /// Voice state changed (listening, processing, etc.)
    VoiceStateUpdate {
        state: String,
    },
    /// TTS audio chunk for playback.
    VoiceTtsChunk {
        data: Vec<u8>,
        is_final: bool,
    },
    // ── Preview ──────────────────────────────────────────────────────────
    /// File preview content.
    PreviewResult {
        path: String,
        kind: String,
        content: String,
        error: Option<String>,
    },
    /// File preview update (file-follow push).
    PreviewUpdate {
        path: String,
        content: String,
    },
    /// Tool result with attached media (separate from ToolResult for wire compat).
    ToolResultMedia {
        id: String,
        name: String,
        result: String,
        is_error: bool,
        media: MediaPayload,
    },
    // ── Engines ──────────────────────────────────────────────────────────
    /// Engine list result.
    EngineListResult {
        engines: Vec<EngineInfoDto>,
    },
    /// Engine model list result.
    EngineModelListResult {
        engine: String,
        models: Vec<EngineModelDto>,
    },
    /// Streaming pull progress.
    EnginePullProgress {
        engine: String,
        model: String,
        percent: f32,
        downloaded_bytes: u64,
        total_bytes: u64,
        status: String,
    },
    /// Engine action result (start/stop/install/remove/load/unload/config).
    EngineActionResult {
        engine: String,
        model: Option<String>,
        ok: bool,
        message: String,
    },
}

mod codec;
mod dto;

pub use codec::{
    FrameCodecError, MAX_FRAME_SIZE, deserialize_frame, deserialize_wire_frame, serialize_frame,
    serialize_wire_frame,
};
pub use dto::*;

#[cfg(test)]
mod frame_size_tests;
#[cfg(test)]
mod tests;
