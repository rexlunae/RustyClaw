//! Client-facing protocol types for gateway communication.
//!
//! These types represent the higher-level events a client (TUI, desktop,
//! CLI) receives from the gateway, and the commands a client sends to it.
//! They are distinct from the binary frame-level protocol types in
//! [`super::protocol`], which handle the wire format.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::user_prompt_types::UserPrompt;

// ── Re-export ────────────────────────────────────────────────────────────────

pub use crate::gateway::protocol::SecretEntryDto;
pub use crate::gateway::protocol::ServiceInfoDto;
pub use crate::gateway::protocol::frames::{
    ChannelStatusDto, CronJobDto, DownloadInfoDto, EngineInfoDto, EngineModelDto, HistoryEntryDto,
    McpServerDto, MemoryEntryDto, MessengerAccountDto, MessengerProfileDto, ModelUsageDto,
    PluginActionDto, PluginInfoDto, RoutableThreadDto, SessionUsageDto, ThreadRouteDto,
    ToolConfigDto, UsageTotalsDto,
};

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

    /// Provider/model reloaded (config change applied without restart)
    ModelReloaded { provider: String, model: String },

    /// Stream starting
    /// A turn began. `thread_id` is the thread its frames belong to; the
    /// client should route everything up to the matching `ResponseDone`
    /// there rather than to whatever is on screen.
    StreamStart { thread_id: Option<u64> },

    /// Thinking started (extended thinking)
    ThinkingStart,

    /// A chunk of the model's reasoning text. Clients accumulate these into
    /// a collapsible "thinking" block so the user can see *why* the agent
    /// did what it did, not just that it paused.
    ThinkingDelta { delta: String },

    /// Thinking ended
    ThinkingEnd,

    /// Text chunk received
    Chunk { delta: String },

    /// Response complete
    /// A turn ended. Carries the thread it belonged to.
    ResponseDone { thread_id: Option<u64> },

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

    /// Live status for a tool call that is still executing: elapsed time
    /// plus, when the tool is waiting on a child process, that process's
    /// CPU usage, memory, and scheduler state. A `pid` marks the process
    /// as controllable via [`GatewayCommand::ProcessControl`].
    ToolStatus {
        id: String,
        name: String,
        elapsed_ms: u64,
        pid: Option<u32>,
        cpu_percent: Option<f32>,
        memory_bytes: Option<u64>,
        state: Option<String>,
        message: Option<String>,
    },

    /// A chunk of live stdout/stderr from a still-running tool, so the
    /// tool's panel can show progress as it happens.
    ToolOutput {
        id: String,
        chunk: String,
        is_stderr: bool,
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

    /// Plugin list and state updated.
    PluginsUpdate { plugins: Vec<PluginInfoDto> },

    /// The messenger setup view. Also arrives after every mutation *this
    /// client* sends, so its form never shows state the gateway has moved
    /// past. Mutations made from other connections are not pushed here; the
    /// panel is current as of its last request or mutation.
    MessengerConfigResult {
        accounts: Vec<MessengerAccountDto>,
        routes: Vec<ThreadRouteDto>,
        threads: Vec<RoutableThreadDto>,
        available_kinds: Vec<String>,
        vault_locked: bool,
    },

    /// Outcome of an account save, delete, or credential migration.
    MessengerAccountResult {
        ok: bool,
        name: String,
        errors: Vec<String>,
        message: Option<String>,
    },

    /// Outcome of a route save or delete.
    MessengerRouteResult { ok: bool, message: Option<String> },

    /// Projects updated
    ProjectsUpdate {
        projects: Vec<ProjectInfoDto>,
        active_id: u64,
    },

    /// Agents in this installation (plus which one is active on this connection)
    AgentsUpdate {
        agents: Vec<AgentInfoDto>,
        active_id: String,
    },

    /// The connection's active agent changed
    AgentSwitched { agent_id: String, name: String },

    /// Authoritative, cross-session conversation history for a thread.
    ThreadHistory {
        thread_id: u64,
        ok: bool,
        messages: Vec<crate::gateway::protocol::types::ChatMessage>,
        error: Option<String>,
    },

    /// Thread messages/history updated
    ThreadMessages {
        thread_id: u64,
        messages: Vec<crate::gateway::protocol::types::ChatMessage>,
    },

    /// Thread switch confirmed — clear the live view and optionally show a
    /// context summary for the thread being switched to.
    ThreadSwitched {
        thread_id: u64,
        context_summary: Option<String>,
    },

    /// Error from gateway
    Error { message: String },

    /// Info message
    Info { message: String },

    /// Non-fatal warning message
    Warning { message: String },

    /// DOM query request — evaluate JS in webview
    DomQuery { id: String, js: String },

    /// Secrets list result from gateway vault
    SecretsListResult {
        ok: bool,
        entries: Vec<SecretEntryInfo>,
    },

    /// Secrets store result
    SecretsStoreResult { ok: bool, message: String },

    /// Secrets delete result
    SecretsDeleteResult { ok: bool, message: Option<String> },

    /// Secrets set policy result
    SecretsSetPolicyResult { ok: bool, message: Option<String> },

    /// Result of fetching a single secret's value
    SecretsGetResult { key: String, value: Option<String> },

    /// Result of peeking at a credential's fields
    SecretsPeekResult {
        ok: bool,
        fields: Vec<(String, String)>,
        message: Option<String>,
    },

    /// Result of enabling/disabling a credential
    SecretsSetDisabledResult { ok: bool },

    /// Result of deleting a full credential
    SecretsDeleteCredentialResult { ok: bool },

    /// Whether TOTP is configured for the vault
    SecretsHasTotpResult { has_totp: bool },

    /// Result of setting up TOTP (returns the provisioning URI on success)
    SecretsSetupTotpResult {
        ok: bool,
        uri: Option<String>,
        message: Option<String>,
    },

    /// Result of verifying a TOTP code
    SecretsVerifyTotpResult { ok: bool },

    /// Result of removing TOTP
    SecretsRemoveTotpResult { ok: bool },

    /// Host hardware capabilities received from gateway
    HostInfo {
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
        gpus: Vec<crate::gateway::protocol::frames::GpuInfoDto>,
        summary: String,
    },

    /// Current system load status received from gateway
    LoadStatus {
        load_score: f64,
        avg_load_score: f64,
        cpu_percent: f32,
        memory_percent: f32,
        summary: String,
    },

    /// Service list received from gateway
    ServiceList { services: Vec<ServiceInfoDto> },

    /// Service action result (start/stop/restart)
    ServiceActionResult {
        ok: bool,
        service: Option<ServiceInfoDto>,
        message: Option<String>,
    },

    /// Service logs received from gateway
    ServiceLogs {
        ok: bool,
        name: String,
        lines: Vec<String>,
        message: Option<String>,
    },

    // ── Engines ──────────────────────────────────────────────────────────
    /// Engine list result.
    EngineListResult { engines: Vec<EngineInfoDto> },
    /// Engine model list result.
    EngineModelListResult {
        engine: String,
        models: Vec<EngineModelDto>,
    },
    /// Live provider model list result.
    ProviderModelListResult {
        provider: String,
        models: Vec<String>,
        error: Option<String>,
    },
    /// Engine pull progress (streaming).
    EnginePullProgress {
        engine: String,
        model: String,
        percent: f32,
        downloaded_bytes: u64,
        total_bytes: u64,
        status: String,
    },
    /// Engine action result.
    EngineActionResult {
        engine: String,
        model: Option<String>,
        ok: bool,
        message: String,
    },
    /// Streamed output line from an in-progress engine action (install).
    EngineActionProgress {
        engine: String,
        line: String,
        percent: f32,
    },

    // ── Panels ────────────────────────────────────────────────────────────
    /// Cron job list result.
    CronListResult { jobs: Vec<CronJobDto> },
    /// Cron upsert result.
    CronUpsertResult {
        ok: bool,
        job: Option<CronJobDto>,
        message: Option<String>,
    },
    /// Cron action result.
    CronActionResult { ok: bool, message: Option<String> },
    /// Memory entry list result.
    MemoryListResult { entries: Vec<MemoryEntryDto> },
    /// Memory upsert result.
    MemoryUpsertResult {
        ok: bool,
        id: Option<String>,
        message: Option<String>,
    },
    /// Memory delete result.
    MemoryDeleteResult { ok: bool, message: Option<String> },
    /// History search result.
    HistorySearchResult { entries: Vec<HistoryEntryDto> },
    /// MCP server list result.
    McpListResult { servers: Vec<McpServerDto> },
    /// MCP connect result.
    McpConnectResult {
        ok: bool,
        server: Option<McpServerDto>,
        message: Option<String>,
    },
    /// MCP disconnect result.
    McpDisconnectResult { ok: bool, message: Option<String> },
    /// Tool configuration list result.
    ToolConfigResult { tools: Vec<ToolConfigDto> },
    /// Tool toggle result.
    ToolToggleResult { ok: bool, message: Option<String> },
    /// Channel status result.
    ChannelStatusResult { channels: Vec<ChannelStatusDto> },
    /// Channel pair result.
    ChannelPairResult {
        ok: bool,
        channel: Option<ChannelStatusDto>,
        message: Option<String>,
    },
    /// Usage/analytics stats result.
    UsageStatsResult {
        totals: UsageTotalsDto,
        per_model: Vec<ModelUsageDto>,
        per_session: Vec<SessionUsageDto>,
    },
    /// Logs result.
    LogsResult {
        ok: bool,
        source: String,
        lines: Vec<String>,
        message: Option<String>,
    },

    /// The transfers this connection started, newest first. Arrives whenever
    /// one changes, not only when asked for.
    DownloadsUpdate { downloads: Vec<DownloadInfoDto> },
}

// ── Commands (client → server) ──────────────────────────────────────────────

/// Commands to send to the gateway.
#[derive(Clone, Debug, Serialize, strum::IntoStaticStr)]
#[allow(dead_code)]
#[serde(tag = "type")]
pub enum GatewayCommand {
    /// Send a chat message.
    ///
    /// `thread_id` names the thread the message belongs to. A client that
    /// tracks threads should always set it: without it the gateway falls
    /// back to its own idea of the current thread, which the client cannot
    /// see and which either side may have changed in the meantime.
    #[serde(rename = "chat")]
    Chat {
        message: String,
        #[serde(default)]
        thread_id: Option<u64>,
        /// The client's own kind ([`SessionOrigin::Desktop`],
        /// [`SessionOrigin::Tui`], …). Sent so the gateway can tell the agent
        /// where the message comes from.
        #[serde(default)]
        client_kind: Option<SessionOrigin>,
    },

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

    /// Create a new thread. `project_id` of `None` means the active project.
    #[serde(rename = "thread_create")]
    ThreadCreate {
        label: Option<String>,
        project_id: Option<u64>,
    },

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

    /// Request the current project list
    #[serde(rename = "project_list")]
    ProjectList,

    /// Create a new project (a named working directory)
    #[serde(rename = "project_create")]
    ProjectCreate { name: String, path: PathBuf },

    /// Rename a project
    #[serde(rename = "project_rename")]
    ProjectRename { project_id: u64, new_name: String },

    /// Edit a project's display name and working directory together.
    #[serde(rename = "project_update")]
    ProjectUpdate {
        project_id: u64,
        name: String,
        path: PathBuf,
    },

    /// Edit a thread's caption and working-directory override. A
    /// `working_dir` of `None` clears the override.
    #[serde(rename = "thread_update")]
    ThreadUpdate {
        thread_id: u64,
        label: String,
        working_dir: Option<PathBuf>,
    },

    /// Request the plugin list and every plugin's current state.
    #[serde(rename = "plugin_list")]
    PluginList,

    /// Re-read one plugin's state from disk and push the refreshed list.
    #[serde(rename = "plugin_refresh")]
    PluginRefresh { plugin_name: String },

    /// Delete a project
    #[serde(rename = "project_delete")]
    ProjectDelete { project_id: u64 },

    /// Switch the active project
    #[serde(rename = "project_switch")]
    ProjectSwitch { project_id: u64 },

    /// List secrets
    #[serde(rename = "secrets_list")]
    SecretsList,

    /// Cancel current operation. `thread_id` names the turn to stop; `None`
    /// is honoured only when exactly one is running, since the gateway would
    /// otherwise have to guess which conversation the user meant.
    #[serde(rename = "cancel")]
    Cancel { thread_id: Option<u64> },

    /// Control a running exec process (pause/resume/stop/kill)
    #[serde(rename = "process_control")]
    ProcessControl {
        pid: u32,
        action: crate::exec_status::ProcessControlAction,
    },

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
    SetWorkingDirectory { path: PathBuf },

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

    /// Delete a full credential from the gateway vault
    #[serde(rename = "secrets_delete_credential")]
    SecretsDeleteCredential { name: String },

    /// Ask whether the vault has TOTP 2FA configured
    #[serde(rename = "secrets_has_totp")]
    SecretsHasTotp,

    /// Reload gateway configuration (apply provider/model changes without restart)
    #[serde(rename = "reload")]
    Reload,

    /// Request the current task list (optionally filtered by session)
    #[serde(rename = "tasks_request")]
    TasksRequest { session: Option<String> },

    /// Request host hardware capabilities
    #[serde(rename = "host_info_request")]
    HostInfoRequest,

    /// Request current system load status
    #[serde(rename = "load_status_request")]
    LoadStatusRequest,

    /// Request list of managed services
    #[serde(rename = "service_list")]
    ServiceList,

    /// Start a managed service
    #[serde(rename = "service_start")]
    ServiceStart { name: String },

    /// Stop a managed service
    #[serde(rename = "service_stop")]
    ServiceStop { name: String },

    /// Restart a managed service
    #[serde(rename = "service_restart")]
    ServiceRestart { name: String },

    /// Request logs for a managed service
    #[serde(rename = "service_logs")]
    ServiceLogs { name: String, tail: Option<usize> },

    // ── Engine commands ────────────────────────────────────────────────
    /// List local engines and their status.
    #[serde(rename = "engine_list")]
    EngineList,

    /// Perform an engine action (install/start/stop).
    #[serde(rename = "engine_action")]
    EngineAction {
        engine: String,
        action: EngineActionKind,
    },

    /// List models for a specific engine.
    #[serde(rename = "engine_model_list")]
    EngineModelList { engine: String },

    /// Request the live model list for a cloud provider.
    #[serde(rename = "provider_model_list")]
    ProviderModelList { provider: String },

    /// Pull/download a model.
    #[serde(rename = "engine_model_pull")]
    EngineModelPull {
        engine: String,
        model: String,
        #[serde(default)]
        expected_size_bytes: Option<u64>,
    },

    /// Perform a model action (remove/load/unload).
    #[serde(rename = "engine_model_action")]
    EngineModelAction {
        engine: String,
        model: String,
        action: ModelActionKind,
        #[serde(default)]
        context_length: Option<u32>,
        #[serde(default)]
        extra_args: Vec<String>,
    },

    // ── Panel commands ─────────────────────────────────────────────────
    /// List cron jobs.
    #[serde(rename = "cron_list")]
    CronList,

    /// Create or update a cron job.
    #[serde(rename = "cron_upsert")]
    CronUpsert {
        id: Option<String>,
        name: String,
        expr: String,
        payload: String,
        paused: bool,
        /// The payload is a scheduled agent turn (prompt), not a note.
        #[serde(default)]
        agent_turn: bool,
        /// Model override for the scheduled turn.
        #[serde(default)]
        model: Option<String>,
        /// Thread the wake uses for context and lands its response in.
        #[serde(default)]
        thread_id: Option<u64>,
    },

    /// Pause/resume/run/remove a cron job.
    #[serde(rename = "cron_action")]
    CronAction { id: String, action: CronActionKind },

    /// List memory entries (optionally filtered).
    #[serde(rename = "memory_list")]
    MemoryList {
        query: Option<String>,
        limit: Option<usize>,
    },

    /// Create or update a memory entry.
    #[serde(rename = "memory_upsert")]
    MemoryUpsert {
        id: Option<String>,
        content: String,
        category: Option<String>,
    },

    /// Delete a memory entry.
    #[serde(rename = "memory_delete")]
    MemoryDelete { id: String },

    /// Search conversation history.
    #[serde(rename = "history_search")]
    HistorySearch { query: String, limit: Option<usize> },

    /// List MCP servers.
    #[serde(rename = "mcp_list")]
    McpList,

    /// Connect an MCP server.
    #[serde(rename = "mcp_connect")]
    McpConnect {
        name: String,
        command: Option<String>,
        url: Option<String>,
        env: Vec<(String, String)>,
    },

    /// Disconnect an MCP server.
    #[serde(rename = "mcp_disconnect")]
    McpDisconnect { name: String },

    /// List tool configuration.
    #[serde(rename = "tool_config_list")]
    ToolConfigList,

    /// Toggle a tool's enabled state.
    #[serde(rename = "tool_toggle")]
    ToolToggle { tool_name: String, enabled: bool },

    /// Request messenger channel status.
    #[serde(rename = "channel_status")]
    ChannelStatus,

    /// Pair/unpair a messenger channel.
    #[serde(rename = "channel_pair")]
    ChannelPair {
        channel: String,
        action: ChannelPairActionKind,
    },

    /// Request usage/analytics stats.
    #[serde(rename = "usage_stats")]
    UsageStats { period: Option<String> },

    /// Request logs from a source ("gateway" | "agent" | "cron" | service name).
    #[serde(rename = "logs")]
    Logs { source: String, tail: Option<usize> },

    // ── Agent commands ─────────────────────────────────────────────────
    /// Request the list of agents in this installation
    #[serde(rename = "agent_list")]
    AgentList,

    /// Switch this connection's active agent
    #[serde(rename = "agent_switch")]
    AgentSwitch { agent_id: String },

    /// Create a new agent
    #[serde(rename = "agent_create")]
    AgentCreate {
        name: String,
        agent_id: Option<String>,
        description: Option<String>,
    },

    /// Delete an agent ('main' is protected)
    #[serde(rename = "agent_delete")]
    AgentDelete { agent_id: String },

    // ── Messenger setup ────────────────────────────────────────────────
    /// Request accounts, routes, and the threads routes may point at.
    #[serde(rename = "messenger_config")]
    MessengerConfig,

    /// Create or update a messenger account.
    ///
    /// `secrets` travels one way only: values go to the vault and are never
    /// returned. Leaving a secret field out keeps the credential already
    /// stored, so an edit does not force the user to retype a token.
    #[serde(rename = "messenger_account_save")]
    MessengerAccountSave {
        /// Account being renamed, or `None` when creating a new one.
        original_name: Option<String>,
        name: String,
        messenger_type: String,
        enabled: bool,
        fields: Vec<(String, String)>,
        secrets: Vec<(String, String)>,
        display_name: Option<String>,
        bio: Option<String>,
        avatar_path: Option<PathBuf>,
        agent_id: Option<String>,
    },

    /// Delete an account, its vault credentials, and its routes.
    #[serde(rename = "messenger_account_delete")]
    MessengerAccountDelete { name: String },

    /// Move an account's plaintext credentials into the vault.
    #[serde(rename = "messenger_secrets_migrate")]
    MessengerSecretsMigrate { name: String },

    /// Create or update a channel-to-thread route.
    #[serde(rename = "messenger_route_save")]
    MessengerRouteSave {
        messenger: String,
        channel: Option<String>,
        thread_id: u64,
        agent_id: Option<String>,
        enabled: bool,
    },

    /// Delete a channel-to-thread route.
    #[serde(rename = "messenger_route_delete")]
    MessengerRouteDelete {
        messenger: String,
        channel: Option<String>,
    },

    /// Ask for the current transfers.
    #[serde(rename = "downloads_request")]
    DownloadsRequest,

    /// Stop a running transfer.
    #[serde(rename = "download_cancel")]
    DownloadCancel { id: String },

    /// Forget the transfers that have finished.
    #[serde(rename = "downloads_clear_finished")]
    DownloadsClearFinished,
}

// ── Protocol bridge (client types ⇄ wire frames) ────────────────────────────
//
// These conversions are the single shared translation between the
// client-facing command/event enums and the binary frame protocol.
// Both the TUI and desktop clients use them so the mapping lives in
// exactly one place.

use crate::gateway::{
    ChannelPairActionKind, ChatMessage, ClientFrame, ClientFrameType, ClientPayload,
    CronActionKind, EngineActionKind, ModelActionKind, ServerFrame, ServerPayload, SessionOrigin,
    StatusType,
};

impl GatewayCommand {
    /// The variant's name, carrying none of its fields.
    ///
    /// Deliberately not `{:?}`. Commands carry TOTP codes, vault passwords,
    /// credential answers and secret values, so debug-formatting one into a
    /// log writes the secret to disk. This keeps only the leading identifier,
    /// which is all a diagnostic needs.
    ///
    /// Taken from the debug rendering rather than matched variant by variant:
    /// there are 77 of them, and a match would be one `_ => "Unknown"` away
    /// from silently mislabelling whichever command was added last.
    pub fn name(&self) -> &'static str {
        // `AsRefStr` generates a match returning a literal per variant, so the
        // payload is never rendered — not on the success path, not on the
        // failure path, not into a temporary that outlives the call. Getting
        // this from `{:?}` and truncating would materialise the TOTP code and
        // the vault password in a heap `String` first, which is the exact
        // thing naming instead of debugging was meant to avoid.
        //
        // Exhaustive by construction: a new variant that nobody names does
        // not compile, so this cannot drift behind the enum.
        self.into()
    }

    /// Convert this command into the wire frame the gateway expects.
    pub fn into_frame(self) -> ClientFrame {
        match self {
            GatewayCommand::Chat {
                message,
                thread_id,
                client_kind,
            } => ClientFrame {
                frame_type: ClientFrameType::Chat,
                payload: ClientPayload::Chat {
                    messages: vec![ChatMessage::text("user", &message)],
                    thread_id,
                    client_kind,
                },
            },
            GatewayCommand::Auth { code } => ClientFrame {
                frame_type: ClientFrameType::AuthResponse,
                payload: ClientPayload::AuthResponse { code },
            },
            GatewayCommand::VaultUnlock { password } => ClientFrame {
                frame_type: ClientFrameType::UnlockVault,
                payload: ClientPayload::UnlockVault { password },
            },
            GatewayCommand::ToolApprove { id, approved } => ClientFrame {
                frame_type: ClientFrameType::ToolApprovalResponse,
                payload: ClientPayload::ToolApprovalResponse { id, approved },
            },
            GatewayCommand::ThreadSwitch { thread_id } => ClientFrame {
                frame_type: ClientFrameType::ThreadSwitch,
                payload: ClientPayload::ThreadSwitch { thread_id },
            },
            GatewayCommand::ThreadCreate { label, project_id } => ClientFrame {
                frame_type: ClientFrameType::ThreadCreate,
                payload: ClientPayload::ThreadCreate {
                    label: label.unwrap_or_default(),
                    project_id: project_id.unwrap_or(0),
                },
            },
            GatewayCommand::ProjectList => ClientFrame {
                frame_type: ClientFrameType::ProjectList,
                payload: ClientPayload::ProjectList,
            },
            GatewayCommand::PluginList => ClientFrame {
                frame_type: ClientFrameType::PluginList,
                payload: ClientPayload::PluginList,
            },
            GatewayCommand::PluginRefresh { plugin_name } => ClientFrame {
                frame_type: ClientFrameType::PluginRefresh,
                payload: ClientPayload::PluginRefresh { plugin_name },
            },
            GatewayCommand::MessengerConfig => ClientFrame {
                frame_type: ClientFrameType::MessengerConfigRequest,
                payload: ClientPayload::MessengerConfigRequest,
            },
            GatewayCommand::MessengerAccountSave {
                original_name,
                name,
                messenger_type,
                enabled,
                fields,
                secrets,
                display_name,
                bio,
                avatar_path,
                agent_id,
            } => ClientFrame {
                frame_type: ClientFrameType::MessengerAccountSave,
                payload: ClientPayload::MessengerAccountSave {
                    original_name,
                    name,
                    messenger_type,
                    enabled,
                    fields,
                    secrets: secrets
                        .into_iter()
                        .map(|(field, value)| (field, value.into()))
                        .collect(),
                    display_name,
                    bio,
                    avatar_path,
                    agent_id,
                },
            },
            GatewayCommand::MessengerAccountDelete { name } => ClientFrame {
                frame_type: ClientFrameType::MessengerAccountDelete,
                payload: ClientPayload::MessengerAccountDelete { name },
            },
            GatewayCommand::MessengerSecretsMigrate { name } => ClientFrame {
                frame_type: ClientFrameType::MessengerSecretsMigrate,
                payload: ClientPayload::MessengerSecretsMigrate { name },
            },
            GatewayCommand::MessengerRouteSave {
                messenger,
                channel,
                thread_id,
                agent_id,
                enabled,
            } => ClientFrame {
                frame_type: ClientFrameType::MessengerRouteSave,
                payload: ClientPayload::MessengerRouteSave {
                    messenger,
                    channel,
                    thread_id,
                    agent_id,
                    enabled,
                },
            },
            GatewayCommand::MessengerRouteDelete { messenger, channel } => ClientFrame {
                frame_type: ClientFrameType::MessengerRouteDelete,
                payload: ClientPayload::MessengerRouteDelete { messenger, channel },
            },
            GatewayCommand::ProjectCreate { name, path } => ClientFrame {
                frame_type: ClientFrameType::ProjectCreate,
                payload: ClientPayload::ProjectCreate { name, path },
            },
            GatewayCommand::ProjectRename {
                project_id,
                new_name,
            } => ClientFrame {
                frame_type: ClientFrameType::ProjectRename,
                payload: ClientPayload::ProjectRename {
                    project_id,
                    new_name,
                },
            },
            GatewayCommand::ProjectUpdate {
                project_id,
                name,
                path,
            } => ClientFrame {
                frame_type: ClientFrameType::ProjectUpdate,
                payload: ClientPayload::ProjectUpdate {
                    project_id,
                    name,
                    path,
                },
            },
            GatewayCommand::ThreadUpdate {
                thread_id,
                label,
                working_dir,
            } => ClientFrame {
                frame_type: ClientFrameType::ThreadUpdate,
                payload: ClientPayload::ThreadUpdate {
                    thread_id,
                    label,
                    working_dir,
                },
            },
            GatewayCommand::ProjectDelete { project_id } => ClientFrame {
                frame_type: ClientFrameType::ProjectDelete,
                payload: ClientPayload::ProjectDelete { project_id },
            },
            GatewayCommand::ProjectSwitch { project_id } => ClientFrame {
                frame_type: ClientFrameType::ProjectSwitch,
                payload: ClientPayload::ProjectSwitch { project_id },
            },
            GatewayCommand::ThreadList => ClientFrame {
                frame_type: ClientFrameType::ThreadList,
                payload: ClientPayload::ThreadList,
            },
            GatewayCommand::ThreadHistoryRequest { thread_id } => ClientFrame {
                frame_type: ClientFrameType::ThreadHistoryRequest,
                payload: ClientPayload::ThreadHistoryRequest { thread_id },
            },
            GatewayCommand::ThreadClose { thread_id } => ClientFrame {
                frame_type: ClientFrameType::ThreadClose,
                payload: ClientPayload::ThreadClose { thread_id },
            },
            GatewayCommand::ThreadRename {
                thread_id,
                new_label,
            } => ClientFrame {
                frame_type: ClientFrameType::ThreadRename,
                payload: ClientPayload::ThreadRename {
                    thread_id,
                    new_label,
                },
            },
            GatewayCommand::UserPromptResponse {
                id,
                dismissed,
                value,
            } => ClientFrame {
                frame_type: ClientFrameType::UserPromptResponse,
                payload: ClientPayload::UserPromptResponse {
                    id,
                    dismissed,
                    value,
                },
            },
            GatewayCommand::CredentialResponse {
                id,
                dismissed,
                value,
            } => ClientFrame {
                frame_type: ClientFrameType::CredentialResponse,
                payload: ClientPayload::CredentialResponse {
                    id,
                    dismissed,
                    value,
                },
            },
            GatewayCommand::SecretsList => ClientFrame {
                frame_type: ClientFrameType::SecretsList,
                payload: ClientPayload::SecretsList,
            },
            GatewayCommand::Cancel { thread_id } => ClientFrame {
                frame_type: ClientFrameType::Cancel,
                payload: ClientPayload::Cancel { thread_id },
            },
            GatewayCommand::ProcessControl { pid, action } => ClientFrame {
                frame_type: ClientFrameType::ProcessControl,
                payload: ClientPayload::ProcessControl { pid, action },
            },
            GatewayCommand::ModelSwitch { provider, model } => ClientFrame {
                frame_type: ClientFrameType::ModelSwitch,
                payload: ClientPayload::ModelSwitch { provider, model },
            },
            GatewayCommand::DomQueryResponse {
                id,
                result,
                is_error,
            } => ClientFrame {
                frame_type: ClientFrameType::DomQueryResponse,
                payload: ClientPayload::DomQueryResponse {
                    id,
                    result,
                    is_error,
                },
            },
            GatewayCommand::SetAgentName { name } => ClientFrame {
                frame_type: ClientFrameType::SetAgentName,
                payload: ClientPayload::SetAgentName { name },
            },
            GatewayCommand::SetWorkingDirectory { path } => ClientFrame {
                frame_type: ClientFrameType::SetWorkingDirectory,
                payload: ClientPayload::SetWorkingDirectory { path },
            },
            GatewayCommand::SecretsStore { key, value } => ClientFrame {
                frame_type: ClientFrameType::SecretsStore,
                payload: ClientPayload::SecretsStore { key, value },
            },
            GatewayCommand::SecretsDelete { key } => ClientFrame {
                frame_type: ClientFrameType::SecretsDelete,
                payload: ClientPayload::SecretsDelete { key },
            },
            GatewayCommand::SecretsSetPolicy {
                name,
                policy,
                skills,
            } => ClientFrame {
                frame_type: ClientFrameType::SecretsSetPolicy,
                payload: ClientPayload::SecretsSetPolicy {
                    name,
                    policy,
                    skills,
                },
            },
            GatewayCommand::SecretsDeleteCredential { name } => ClientFrame {
                frame_type: ClientFrameType::SecretsDeleteCredential,
                payload: ClientPayload::SecretsDeleteCredential { name },
            },
            GatewayCommand::SecretsHasTotp => ClientFrame {
                frame_type: ClientFrameType::SecretsHasTotp,
                payload: ClientPayload::SecretsHasTotp,
            },
            GatewayCommand::Reload => ClientFrame {
                frame_type: ClientFrameType::Reload,
                payload: ClientPayload::Reload,
            },
            GatewayCommand::DownloadsRequest => ClientFrame {
                frame_type: ClientFrameType::DownloadsRequest,
                payload: ClientPayload::DownloadsRequest,
            },
            GatewayCommand::DownloadCancel { id } => ClientFrame {
                frame_type: ClientFrameType::DownloadCancel,
                payload: ClientPayload::DownloadCancel { id },
            },
            GatewayCommand::DownloadsClearFinished => ClientFrame {
                frame_type: ClientFrameType::DownloadsClearFinished,
                payload: ClientPayload::DownloadsClearFinished,
            },
            GatewayCommand::TasksRequest { session } => ClientFrame {
                frame_type: ClientFrameType::TasksRequest,
                payload: ClientPayload::TasksRequest { session },
            },
            GatewayCommand::HostInfoRequest => ClientFrame {
                frame_type: ClientFrameType::HostInfoRequest,
                payload: ClientPayload::HostInfoRequest,
            },
            GatewayCommand::LoadStatusRequest => ClientFrame {
                frame_type: ClientFrameType::LoadStatusRequest,
                payload: ClientPayload::LoadStatusRequest,
            },
            GatewayCommand::ServiceList => ClientFrame {
                frame_type: ClientFrameType::ServiceListRequest,
                payload: ClientPayload::ServiceListRequest,
            },
            GatewayCommand::ServiceStart { name } => ClientFrame {
                frame_type: ClientFrameType::ServiceStartRequest,
                payload: ClientPayload::ServiceStartRequest { name },
            },
            GatewayCommand::ServiceStop { name } => ClientFrame {
                frame_type: ClientFrameType::ServiceStopRequest,
                payload: ClientPayload::ServiceStopRequest { name },
            },
            GatewayCommand::ServiceRestart { name } => ClientFrame {
                frame_type: ClientFrameType::ServiceRestartRequest,
                payload: ClientPayload::ServiceRestartRequest { name },
            },
            GatewayCommand::ServiceLogs { name, tail } => ClientFrame {
                frame_type: ClientFrameType::ServiceLogsRequest,
                payload: ClientPayload::ServiceLogsRequest { name, tail },
            },
            // ── Engines ──────────────────────────────────────────────
            GatewayCommand::EngineList => ClientFrame {
                frame_type: ClientFrameType::EngineList,
                payload: ClientPayload::EngineList,
            },
            GatewayCommand::EngineAction { engine, action } => ClientFrame {
                frame_type: ClientFrameType::EngineAction,
                payload: ClientPayload::EngineAction { engine, action },
            },
            GatewayCommand::EngineModelList { engine } => ClientFrame {
                frame_type: ClientFrameType::EngineModelList,
                payload: ClientPayload::EngineModelList { engine },
            },
            GatewayCommand::ProviderModelList { provider } => ClientFrame {
                frame_type: ClientFrameType::ProviderModelList,
                payload: ClientPayload::ProviderModelList { provider },
            },
            GatewayCommand::EngineModelPull {
                engine,
                model,
                expected_size_bytes,
            } => ClientFrame {
                frame_type: ClientFrameType::EngineModelPull,
                payload: ClientPayload::EngineModelPull {
                    engine,
                    model,
                    expected_size_bytes,
                },
            },
            GatewayCommand::EngineModelAction {
                engine,
                model,
                action,
                context_length,
                extra_args,
            } => ClientFrame {
                frame_type: ClientFrameType::EngineModelAction,
                payload: ClientPayload::EngineModelAction {
                    engine,
                    model,
                    action,
                    context_length,
                    extra_args,
                },
            },
            // ── Panels ───────────────────────────────────────────────
            GatewayCommand::CronList => ClientFrame {
                frame_type: ClientFrameType::CronListRequest,
                payload: ClientPayload::CronListRequest,
            },
            GatewayCommand::CronUpsert {
                id,
                name,
                expr,
                payload,
                paused,
                agent_turn,
                model,
                thread_id,
            } => ClientFrame {
                frame_type: ClientFrameType::CronUpsertRequest,
                payload: ClientPayload::CronUpsertRequest {
                    id,
                    name,
                    expr,
                    payload,
                    paused,
                    agent_turn,
                    model,
                    thread_id,
                },
            },
            GatewayCommand::CronAction { id, action } => ClientFrame {
                frame_type: ClientFrameType::CronActionRequest,
                payload: ClientPayload::CronActionRequest { id, action },
            },
            GatewayCommand::MemoryList { query, limit } => ClientFrame {
                frame_type: ClientFrameType::MemoryListRequest,
                payload: ClientPayload::MemoryListRequest { query, limit },
            },
            GatewayCommand::MemoryUpsert {
                id,
                content,
                category,
            } => ClientFrame {
                frame_type: ClientFrameType::MemoryUpsertRequest,
                payload: ClientPayload::MemoryUpsertRequest {
                    id,
                    content,
                    category,
                },
            },
            GatewayCommand::MemoryDelete { id } => ClientFrame {
                frame_type: ClientFrameType::MemoryDeleteRequest,
                payload: ClientPayload::MemoryDeleteRequest { id },
            },
            GatewayCommand::HistorySearch { query, limit } => ClientFrame {
                frame_type: ClientFrameType::HistorySearchRequest,
                payload: ClientPayload::HistorySearchRequest { query, limit },
            },
            GatewayCommand::McpList => ClientFrame {
                frame_type: ClientFrameType::McpListRequest,
                payload: ClientPayload::McpListRequest,
            },
            GatewayCommand::McpConnect {
                name,
                command,
                url,
                env,
            } => ClientFrame {
                frame_type: ClientFrameType::McpConnectRequest,
                payload: ClientPayload::McpConnectRequest {
                    name,
                    command,
                    url,
                    env,
                },
            },
            GatewayCommand::McpDisconnect { name } => ClientFrame {
                frame_type: ClientFrameType::McpDisconnectRequest,
                payload: ClientPayload::McpDisconnectRequest { name },
            },
            GatewayCommand::ToolConfigList => ClientFrame {
                frame_type: ClientFrameType::ToolConfigRequest,
                payload: ClientPayload::ToolConfigRequest,
            },
            GatewayCommand::ToolToggle { tool_name, enabled } => ClientFrame {
                frame_type: ClientFrameType::ToolToggleRequest,
                payload: ClientPayload::ToolToggleRequest { tool_name, enabled },
            },
            GatewayCommand::ChannelStatus => ClientFrame {
                frame_type: ClientFrameType::ChannelStatusRequest,
                payload: ClientPayload::ChannelStatusRequest,
            },
            GatewayCommand::ChannelPair { channel, action } => ClientFrame {
                frame_type: ClientFrameType::ChannelPairRequest,
                payload: ClientPayload::ChannelPairRequest { channel, action },
            },
            GatewayCommand::UsageStats { period } => ClientFrame {
                frame_type: ClientFrameType::UsageStatsRequest,
                payload: ClientPayload::UsageStatsRequest { period },
            },
            GatewayCommand::Logs { source, tail } => ClientFrame {
                frame_type: ClientFrameType::LogsRequest,
                payload: ClientPayload::LogsRequest {
                    source,
                    tail,
                    follow: false,
                },
            },
            GatewayCommand::AgentList => ClientFrame {
                frame_type: ClientFrameType::AgentListRequest,
                payload: ClientPayload::AgentListRequest,
            },
            GatewayCommand::AgentSwitch { agent_id } => ClientFrame {
                frame_type: ClientFrameType::AgentSwitch,
                payload: ClientPayload::AgentSwitch { agent_id },
            },
            GatewayCommand::AgentCreate {
                name,
                agent_id,
                description,
            } => ClientFrame {
                frame_type: ClientFrameType::AgentCreate,
                payload: ClientPayload::AgentCreate {
                    name,
                    agent_id,
                    description,
                },
            },
            GatewayCommand::AgentDelete { agent_id } => ClientFrame {
                frame_type: ClientFrameType::AgentDelete,
                payload: ClientPayload::AgentDelete { agent_id },
            },
        }
    }
}

impl GatewayEvent {
    /// Convert a server frame into the client-facing event, if any.
    ///
    /// Returns `None` for frames that carry no client-visible state
    /// (e.g. `Empty`, legacy `TasksUpdate`, or `ThreadCreated`, which is
    /// always followed by a `ThreadsUpdate`).
    pub fn from_server_frame(frame: ServerFrame) -> Option<Self> {
        match frame.payload {
            ServerPayload::Hello {
                agent,
                vault_locked,
                provider,
                model,
                ..
            } => Some(GatewayEvent::Connected {
                agent: Some(agent),
                vault_locked,
                provider,
                model,
            }),
            ServerPayload::Status { status, detail } => Some(match status {
                StatusType::ModelReady => GatewayEvent::ModelReady { model: detail },
                StatusType::ModelError => GatewayEvent::ModelError { message: detail },
                StatusType::VaultLocked => GatewayEvent::VaultLocked,
                StatusType::ModelConfigured => GatewayEvent::Info {
                    message: format!("Model: {detail}"),
                },
                StatusType::CredentialsLoaded => GatewayEvent::Info { message: detail },
                StatusType::ModelConnecting => GatewayEvent::Info { message: detail },
                StatusType::CredentialsMissing => GatewayEvent::Warning { message: detail },
                StatusType::NoModel => GatewayEvent::Warning { message: detail },
            }),
            ServerPayload::AuthChallenge { .. } => Some(GatewayEvent::AuthRequired),
            ServerPayload::AuthResult { ok, message, retry } => Some(if ok {
                GatewayEvent::AuthSuccess
            } else {
                GatewayEvent::AuthFailed {
                    message: message.unwrap_or_default(),
                    retry: retry.unwrap_or(false),
                }
            }),
            ServerPayload::AuthLocked { message, .. } => Some(GatewayEvent::Error { message }),
            ServerPayload::VaultUnlocked { ok, message } => Some(if ok {
                GatewayEvent::VaultUnlocked
            } else {
                GatewayEvent::Error {
                    message: message.unwrap_or_else(|| "Failed to unlock vault".into()),
                }
            }),
            ServerPayload::ReloadResult {
                ok,
                provider,
                model,
                message,
            } => Some(if ok {
                GatewayEvent::ModelReloaded { provider, model }
            } else {
                GatewayEvent::Error {
                    message: format!(
                        "Reload failed: {}",
                        message.as_deref().unwrap_or("Unknown error")
                    ),
                }
            }),
            ServerPayload::StreamStart { thread_id } => {
                Some(GatewayEvent::StreamStart { thread_id })
            }
            ServerPayload::ThinkingStart => Some(GatewayEvent::ThinkingStart),
            ServerPayload::ThinkingDelta { delta } => Some(GatewayEvent::ThinkingDelta { delta }),
            ServerPayload::ThinkingEnd => Some(GatewayEvent::ThinkingEnd),
            ServerPayload::Chunk { delta } => Some(GatewayEvent::Chunk { delta }),
            ServerPayload::ResponseDone { thread_id, .. } => {
                Some(GatewayEvent::ResponseDone { thread_id })
            }
            ServerPayload::ToolCall {
                id,
                name,
                arguments,
            } => Some(GatewayEvent::ToolCall {
                id,
                name,
                arguments,
            }),
            ServerPayload::ToolResult {
                id,
                name,
                result,
                is_error,
            } => Some(GatewayEvent::ToolResult {
                id,
                name,
                result,
                is_error,
            }),
            ServerPayload::ToolResultMedia {
                id,
                name,
                result,
                is_error,
                media: _,
            } => Some(GatewayEvent::ToolResult {
                id,
                name,
                result,
                is_error,
            }),
            ServerPayload::ToolApprovalRequest {
                id,
                name,
                arguments,
            } => Some(GatewayEvent::ToolApprovalRequest {
                id,
                name,
                arguments,
            }),
            ServerPayload::UserPromptRequest { id, mut prompt } => {
                prompt.id = id.clone();
                Some(GatewayEvent::UserPromptRequest { id, prompt })
            }
            ServerPayload::CredentialRequest {
                id,
                provider,
                secret_name,
                message,
            } => Some(GatewayEvent::CredentialRequest {
                id,
                provider,
                secret_name,
                message,
            }),
            ServerPayload::DeviceFlowStart { url, code, message } => {
                Some(GatewayEvent::DeviceFlowStart { url, code, message })
            }
            ServerPayload::DeviceFlowComplete => Some(GatewayEvent::DeviceFlowComplete),
            ServerPayload::ThreadsUpdate {
                threads,
                foreground_id,
            } => Some(GatewayEvent::ThreadsUpdate {
                threads: threads
                    .into_iter()
                    .map(|t| ThreadInfoDto {
                        id: t.id,
                        project_id: t.project_id,
                        label: Some(t.label),
                        description: t.description,
                        status: t.status.unwrap_or_default(),
                        is_foreground: t.is_foreground,
                        message_count: t.message_count,
                        working_dir: t.working_dir,
                    })
                    .collect(),
                foreground_id,
            }),
            ServerPayload::DownloadsUpdate { downloads } => {
                Some(GatewayEvent::DownloadsUpdate { downloads })
            }
            ServerPayload::PluginsUpdate { plugins } => {
                Some(GatewayEvent::PluginsUpdate { plugins })
            }
            ServerPayload::MessengerConfigResult {
                accounts,
                routes,
                threads,
                available_kinds,
                vault_locked,
            } => Some(GatewayEvent::MessengerConfigResult {
                accounts,
                routes,
                threads,
                available_kinds,
                vault_locked,
            }),
            ServerPayload::MessengerAccountResult {
                ok,
                name,
                errors,
                message,
            } => Some(GatewayEvent::MessengerAccountResult {
                ok,
                name,
                errors,
                message,
            }),
            ServerPayload::MessengerRouteResult { ok, message } => {
                Some(GatewayEvent::MessengerRouteResult { ok, message })
            }
            ServerPayload::ProjectsUpdate {
                projects,
                active_id,
            } => Some(GatewayEvent::ProjectsUpdate {
                projects: projects
                    .into_iter()
                    .map(|p| ProjectInfoDto {
                        id: p.id,
                        name: p.name,
                        path: p.path,
                    })
                    .collect(),
                active_id,
            }),
            ServerPayload::AgentsUpdate { agents, active_id } => Some(GatewayEvent::AgentsUpdate {
                agents: agents
                    .into_iter()
                    .map(|a| AgentInfoDto {
                        id: a.id,
                        name: a.name,
                        description: a.description,
                    })
                    .collect(),
                active_id,
            }),
            ServerPayload::AgentSwitched { agent_id, name } => {
                Some(GatewayEvent::AgentSwitched { agent_id, name })
            }
            ServerPayload::ThreadSwitched {
                thread_id,
                context_summary,
            } => Some(GatewayEvent::ThreadSwitched {
                thread_id,
                context_summary,
            }),
            ServerPayload::ThreadHistoryReply {
                thread_id,
                ok,
                messages,
                error,
            } => Some(GatewayEvent::ThreadHistory {
                thread_id,
                ok,
                messages,
                error,
            }),
            ServerPayload::ThreadMessages {
                thread_id,
                messages,
            } => Some(GatewayEvent::ThreadMessages {
                thread_id,
                messages,
            }),
            ServerPayload::SecretsListResult { ok, entries } => {
                Some(GatewayEvent::SecretsListResult {
                    ok,
                    entries: entries.into_iter().map(Into::into).collect(),
                })
            }
            ServerPayload::SecretsStoreResult { ok, message } => {
                Some(GatewayEvent::SecretsStoreResult { ok, message })
            }
            ServerPayload::SecretsGetResult { key, value, .. } => {
                Some(GatewayEvent::SecretsGetResult { key, value })
            }
            ServerPayload::SecretsDeleteResult { ok, message } => {
                Some(GatewayEvent::SecretsDeleteResult { ok, message })
            }
            ServerPayload::SecretsPeekResult {
                ok,
                fields,
                message,
            } => Some(GatewayEvent::SecretsPeekResult {
                ok,
                fields,
                message,
            }),
            ServerPayload::SecretsSetPolicyResult { ok, message } => {
                Some(GatewayEvent::SecretsSetPolicyResult { ok, message })
            }
            ServerPayload::SecretsSetDisabledResult { ok, .. } => {
                Some(GatewayEvent::SecretsSetDisabledResult { ok })
            }
            ServerPayload::SecretsDeleteCredentialResult { ok, .. } => {
                Some(GatewayEvent::SecretsDeleteCredentialResult { ok })
            }
            ServerPayload::SecretsHasTotpResult { has_totp } => {
                Some(GatewayEvent::SecretsHasTotpResult { has_totp })
            }
            ServerPayload::SecretsSetupTotpResult { ok, uri, message } => {
                Some(GatewayEvent::SecretsSetupTotpResult { ok, uri, message })
            }
            ServerPayload::SecretsVerifyTotpResult { ok, .. } => {
                Some(GatewayEvent::SecretsVerifyTotpResult { ok })
            }
            ServerPayload::SecretsRemoveTotpResult { ok, .. } => {
                Some(GatewayEvent::SecretsRemoveTotpResult { ok })
            }
            ServerPayload::Error { message, .. } => Some(GatewayEvent::Error { message }),
            ServerPayload::Info { message } => Some(GatewayEvent::Info { message }),
            ServerPayload::DomQuery { id, js } => Some(GatewayEvent::DomQuery { id, js }),
            ServerPayload::HostInfoResult {
                hostname,
                os,
                arch,
                cpu_brand,
                cpu_cores_physical,
                cpu_cores_logical,
                cpu_frequency_mhz,
                total_memory_bytes,
                total_swap_bytes,
                disk_total_bytes,
                disk_available_bytes,
                gpus,
                summary,
            } => Some(GatewayEvent::HostInfo {
                hostname,
                os,
                arch,
                cpu_brand,
                cpu_cores_physical,
                cpu_cores_logical,
                cpu_frequency_mhz,
                total_memory_bytes,
                total_swap_bytes,
                disk_total_bytes,
                disk_available_bytes,
                gpus,
                summary,
            }),
            ServerPayload::LoadStatusResult {
                load_score,
                avg_load_score,
                cpu_percent,
                memory_percent,
                summary,
            } => Some(GatewayEvent::LoadStatus {
                load_score,
                avg_load_score,
                cpu_percent,
                memory_percent,
                summary,
            }),
            ServerPayload::ServiceListResult { services } => {
                Some(GatewayEvent::ServiceList { services })
            }
            ServerPayload::ServiceActionResult {
                ok,
                service,
                message,
            } => Some(GatewayEvent::ServiceActionResult {
                ok,
                service,
                message,
            }),
            ServerPayload::ServiceLogsResult {
                ok,
                name,
                lines,
                message,
            } => Some(GatewayEvent::ServiceLogs {
                ok,
                name,
                lines,
                message,
            }),
            // Frames with no client-visible state.
            ServerPayload::Empty
            | ServerPayload::TasksUpdate { .. }
            | ServerPayload::ThreadCreated { .. } => None,
            // ── Panels ───────────────────────────────────────────────
            ServerPayload::CronListResult { jobs } => Some(GatewayEvent::CronListResult { jobs }),
            ServerPayload::CronUpsertResult { ok, job, message } => {
                Some(GatewayEvent::CronUpsertResult { ok, job, message })
            }
            ServerPayload::CronActionResult { ok, message } => {
                Some(GatewayEvent::CronActionResult { ok, message })
            }
            ServerPayload::MemoryListResult { entries } => {
                Some(GatewayEvent::MemoryListResult { entries })
            }
            ServerPayload::MemoryUpsertResult { ok, id, message } => {
                Some(GatewayEvent::MemoryUpsertResult { ok, id, message })
            }
            ServerPayload::MemoryDeleteResult { ok, message } => {
                Some(GatewayEvent::MemoryDeleteResult { ok, message })
            }
            ServerPayload::HistorySearchResult { entries } => {
                Some(GatewayEvent::HistorySearchResult { entries })
            }
            ServerPayload::McpListResult { servers } => {
                Some(GatewayEvent::McpListResult { servers })
            }
            ServerPayload::McpConnectResult {
                ok,
                server,
                message,
            } => Some(GatewayEvent::McpConnectResult {
                ok,
                server,
                message,
            }),
            ServerPayload::McpDisconnectResult { ok, message } => {
                Some(GatewayEvent::McpDisconnectResult { ok, message })
            }
            ServerPayload::ToolConfigResult { tools } => {
                Some(GatewayEvent::ToolConfigResult { tools })
            }
            ServerPayload::ToolToggleResult { ok, message } => {
                Some(GatewayEvent::ToolToggleResult { ok, message })
            }
            ServerPayload::ChannelStatusResult { channels } => {
                Some(GatewayEvent::ChannelStatusResult { channels })
            }
            ServerPayload::ChannelPairResult {
                ok,
                channel,
                message,
            } => Some(GatewayEvent::ChannelPairResult {
                ok,
                channel,
                message,
            }),
            ServerPayload::UsageStatsResult {
                totals,
                per_model,
                per_session,
            } => Some(GatewayEvent::UsageStatsResult {
                totals,
                per_model,
                per_session,
            }),
            ServerPayload::LogsResult {
                ok,
                source,
                lines,
                message,
            } => Some(GatewayEvent::LogsResult {
                ok,
                source,
                lines,
                message,
            }),
            ServerPayload::ToolStatus {
                tool_id,
                name,
                elapsed_ms,
                pid,
                cpu_percent,
                memory_bytes,
                state,
                message,
            } => Some(GatewayEvent::ToolStatus {
                id: tool_id,
                name,
                elapsed_ms,
                pid,
                cpu_percent,
                memory_bytes,
                state,
                message,
            }),
            // Live stdout/stderr from a running tool. Start/End stay
            // unsurfaced: ToolCall/ToolResult already delimit the
            // lifecycle for clients.
            ServerPayload::ToolOutputDelta {
                tool_id,
                chunk,
                is_stderr,
            } => Some(GatewayEvent::ToolOutput {
                id: tool_id,
                chunk,
                is_stderr,
            }),
            // Panel results without a wired backend yet, and streaming
            // frames handled at a lower layer.
            ServerPayload::LogsAppend { .. }
            | ServerPayload::PendingApprovalsResult { .. }
            | ServerPayload::ApprovalsBatchResult { .. }
            | ServerPayload::ToolOutputStart { .. }
            | ServerPayload::ToolOutputEnd { .. }
            | ServerPayload::VoiceTranscript { .. }
            | ServerPayload::VoiceStateUpdate { .. }
            | ServerPayload::VoiceTtsChunk { .. }
            | ServerPayload::PreviewResult { .. }
            | ServerPayload::PreviewUpdate { .. } => None,
            // ── Engines ──────────────────────────────────────────────
            ServerPayload::EngineListResult { engines } => {
                Some(GatewayEvent::EngineListResult { engines })
            }
            ServerPayload::EngineModelListResult { engine, models } => {
                Some(GatewayEvent::EngineModelListResult { engine, models })
            }
            ServerPayload::ProviderModelListResult {
                provider,
                models,
                error,
            } => Some(GatewayEvent::ProviderModelListResult {
                provider,
                models,
                error,
            }),
            ServerPayload::EnginePullProgress {
                engine,
                model,
                percent,
                downloaded_bytes,
                total_bytes,
                status,
            } => Some(GatewayEvent::EnginePullProgress {
                engine,
                model,
                percent,
                downloaded_bytes,
                total_bytes,
                status,
            }),
            ServerPayload::EngineActionResult {
                engine,
                model,
                ok,
                message,
            } => Some(GatewayEvent::EngineActionResult {
                engine,
                model,
                ok,
                message,
            }),
            ServerPayload::EngineActionProgress {
                engine,
                line,
                percent,
            } => Some(GatewayEvent::EngineActionProgress {
                engine,
                line,
                percent,
            }),
        }
    }
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
    #[serde(default)]
    pub project_id: u64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
    pub message_count: usize,
    /// Working-directory override, or `None` when the thread inherits its
    /// project's directory. The edit dialog needs to tell those apart.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
}

/// Project info from gateway (client-facing).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectInfoDto {
    pub id: u64,
    pub name: String,
    /// The project's working directory.
    pub path: PathBuf,
}

/// Agent info from gateway (client-facing).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfoDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[cfg(test)]
mod command_name_tests {
    use super::*;

    /// Naming a command must never carry its payload.
    ///
    /// The whole reason `name` exists rather than `{:?}` is that these
    /// commands hold TOTP codes, vault passwords and credential answers, and
    /// they are named on a path that writes to the log. A regression here
    /// writes secrets to disk, so it is worth a test rather than a comment.
    ///
    /// The first version of this passed while still being wrong: it rendered
    /// the whole command with `{:?}` and truncated, so the log was clean but
    /// the password had already been materialised in a heap `String` that is
    /// never zeroized. The name now comes from a generated match, so the
    /// payload is not rendered at all — but the assertion below cannot tell
    /// those two apart, which is worth knowing when reading it.
    #[test]
    fn naming_a_command_never_carries_its_secrets() {
        let auth = GatewayCommand::Auth {
            code: "867530".to_string(),
        };
        assert_eq!(auth.name(), "Auth");
        assert!(
            !auth.name().contains("867530"),
            "the TOTP code must not appear in the name"
        );

        let unlock = GatewayCommand::VaultUnlock {
            password: "correct-horse-battery-staple".to_string(),
        };
        assert_eq!(unlock.name(), "VaultUnlock");
        assert!(
            !unlock.name().contains("horse"),
            "the vault password must not appear in the name"
        );
    }

    /// A unit variant has no delimiter to stop at, so it is its own name.
    #[test]
    fn a_unit_command_names_itself() {
        assert_eq!(GatewayCommand::ThreadList.name(), "ThreadList");
    }
}
