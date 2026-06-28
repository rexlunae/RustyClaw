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
pub use crate::gateway::protocol::ServiceInfoDto;

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
    StreamStart,

    /// Thinking started (extended thinking)
    ThinkingStart,

    /// A thinking delta was received (used to keep the thinking clock alive)
    ThinkingDelta,

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

    /// Projects updated
    ProjectsUpdate {
        projects: Vec<ProjectInfoDto>,
        active_id: u64,
    },

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
    ProjectCreate { name: String, path: String },

    /// Rename a project
    #[serde(rename = "project_rename")]
    ProjectRename { project_id: u64, new_name: String },

    /// Delete a project
    #[serde(rename = "project_delete")]
    ProjectDelete { project_id: u64 },

    /// Switch the active project
    #[serde(rename = "project_switch")]
    ProjectSwitch { project_id: u64 },

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

    /// Delete a full credential from the gateway vault
    #[serde(rename = "secrets_delete_credential")]
    SecretsDeleteCredential { name: String },

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
}

// ── Protocol bridge (client types ⇄ wire frames) ────────────────────────────
//
// These conversions are the single shared translation between the
// client-facing command/event enums and the binary frame protocol.
// Both the TUI and desktop clients use them so the mapping lives in
// exactly one place.

use crate::gateway::{
    ChatMessage, ClientFrame, ClientFrameType, ClientPayload, ServerFrame, ServerPayload,
    StatusType,
};

impl GatewayCommand {
    /// Convert this command into the wire frame the gateway expects.
    pub fn into_frame(self) -> ClientFrame {
        match self {
            GatewayCommand::Chat { message } => ClientFrame {
                frame_type: ClientFrameType::Chat,
                payload: ClientPayload::Chat {
                    messages: vec![ChatMessage::text("user", &message)],
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
            GatewayCommand::Cancel => ClientFrame {
                frame_type: ClientFrameType::Cancel,
                payload: ClientPayload::Empty,
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
            GatewayCommand::Reload => ClientFrame {
                frame_type: ClientFrameType::Reload,
                payload: ClientPayload::Reload,
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
            ServerPayload::StreamStart => Some(GatewayEvent::StreamStart),
            ServerPayload::ThinkingStart => Some(GatewayEvent::ThinkingStart),
            ServerPayload::ThinkingDelta { .. } => Some(GatewayEvent::ThinkingDelta),
            ServerPayload::ThinkingEnd => Some(GatewayEvent::ThinkingEnd),
            ServerPayload::Chunk { delta } => Some(GatewayEvent::Chunk { delta }),
            ServerPayload::ResponseDone { .. } => Some(GatewayEvent::ResponseDone),
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
                    })
                    .collect(),
                foreground_id,
            }),
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
            // New panel results — handled by UI directly, not as GatewayEvent.
            ServerPayload::CronListResult { .. }
            | ServerPayload::CronUpsertResult { .. }
            | ServerPayload::CronActionResult { .. }
            | ServerPayload::MemoryListResult { .. }
            | ServerPayload::MemoryUpsertResult { .. }
            | ServerPayload::MemoryDeleteResult { .. }
            | ServerPayload::HistorySearchResult { .. }
            | ServerPayload::UsageStatsResult { .. }
            | ServerPayload::LogsResult { .. }
            | ServerPayload::LogsAppend { .. }
            | ServerPayload::McpListResult { .. }
            | ServerPayload::McpConnectResult { .. }
            | ServerPayload::McpDisconnectResult { .. }
            | ServerPayload::ToolConfigResult { .. }
            | ServerPayload::ToolToggleResult { .. }
            | ServerPayload::ChannelStatusResult { .. }
            | ServerPayload::ChannelPairResult { .. }
            | ServerPayload::PendingApprovalsResult { .. }
            | ServerPayload::ApprovalsBatchResult { .. }
            | ServerPayload::ToolOutputStart { .. }
            | ServerPayload::ToolOutputDelta { .. }
            | ServerPayload::ToolOutputEnd { .. }
            | ServerPayload::VoiceTranscript { .. }
            | ServerPayload::VoiceStateUpdate { .. }
            | ServerPayload::VoiceTtsChunk { .. }
            | ServerPayload::PreviewResult { .. }
            | ServerPayload::PreviewUpdate { .. } => None,
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
}

/// Project info from gateway (client-facing).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectInfoDto {
    pub id: u64,
    pub name: String,
    pub path: String,
}
