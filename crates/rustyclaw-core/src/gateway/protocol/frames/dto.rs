//! Data-transfer objects carried inside protocol result frames.
//!
//! These are wire-facing mirrors of core domain types. Conversions from the
//! domain types live here as `From` impls so producers can use `.into()`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// DTO for local engine info in protocol results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfoDto {
    pub id: String,
    pub display_name: String,
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub endpoint: Option<String>,
    pub available_models: u32,
    pub loaded_models: u32,
    pub capabilities: EngineInfoCaps,
}

/// Capability flags exposed to the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfoCaps {
    pub can_install: bool,
    pub can_start: bool,
    pub can_stop: bool,
    pub can_pull: bool,
    pub can_remove: bool,
    pub can_load: bool,
    pub can_unload: bool,
}

impl From<crate::engines::EngineCaps> for EngineInfoCaps {
    fn from(caps: crate::engines::EngineCaps) -> Self {
        Self {
            can_install: caps.can_install,
            can_start: caps.can_start,
            can_stop: caps.can_stop,
            can_pull: caps.can_pull,
            can_remove: caps.can_remove,
            can_load: caps.can_load,
            can_unload: caps.can_unload,
        }
    }
}

/// DTO for a local model in protocol results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineModelDto {
    pub name: String,
    pub size_bytes: u64,
    pub quantization: Option<String>,
    pub context_length: Option<u32>,
    pub loaded: bool,
    pub vram_bytes: Option<u64>,
    pub family: Option<String>,
    pub format: Option<String>,
    /// Whether the model fits the current host's resources.
    #[serde(default = "default_true")]
    pub fits_host: bool,
    /// Warning message if the model doesn't fit (empty if it does).
    #[serde(default)]
    pub fit_warning: String,
}

fn default_true() -> bool {
    true
}

impl From<crate::engines::LocalModel> for EngineModelDto {
    fn from(m: crate::engines::LocalModel) -> Self {
        let fit = crate::engines::check_model_fit(&m);
        Self {
            name: m.name,
            size_bytes: m.size_bytes,
            quantization: m.quantization,
            context_length: m.context_length,
            loaded: m.loaded,
            vram_bytes: m.vram_bytes,
            family: m.family,
            format: m.format,
            fits_host: fit.fits,
            fit_warning: fit.warning,
        }
    }
}

/// DTO for service info in protocol results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceInfoDto {
    pub name: String,
    pub service_type: String,
    pub status: String,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub restart_count: u32,
    pub exit_code: Option<i32>,
    pub health_ok: Option<bool>,
    pub mcp_tools: u32,
}

impl From<crate::services::ServiceInfo> for ServiceInfoDto {
    fn from(info: crate::services::ServiceInfo) -> Self {
        Self {
            name: info.name,
            service_type: info.service_type.display_name().to_string(),
            status: info.status.display_name().to_string(),
            pid: info.pid,
            uptime_secs: info.uptime_secs,
            restart_count: info.restart_count,
            exit_code: info.exit_code,
            health_ok: info.health_ok,
            mcp_tools: info.mcp_tools,
        }
    }
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
    /// Working-directory override, or `None` when the thread inherits its
    /// project's directory. Appended last, as above.
    ///
    /// A `PathBuf` rather than a `String`: bincode encodes both as a
    /// length-prefixed UTF-8 run, so the wire format is unchanged, but the
    /// type keeps the value from being laundered through `display()` — which
    /// silently mangles a path that isn't valid UTF-8 — at every hop.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    /// Whether the thread is pinned to the top of its project's list in the
    /// sidebar. Appended last, as above.
    #[serde(default)]
    pub pinned: bool,
}

/// DTO for project info in `ProjectsUpdate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInfoDto {
    pub id: u64,
    pub name: String,
    /// The project's working directory. See [`ThreadInfoDto::working_dir`]
    /// for why this is a `PathBuf`.
    pub path: PathBuf,
    /// Whether the project is pinned to the top of the sidebar.
    #[serde(default)]
    pub pinned: bool,
}

/// DTO for one plugin in `PluginsUpdate`.
///
/// Mirrors [`crate::plugins::Plugin`] plus its live state. `state` and the
/// action list are what the client renders; `html_template` is the plugin's
/// declared custom template, carried so a client that can render one knows it
/// exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInfoDto {
    pub name: String,
    pub description: String,
    pub emoji: Option<String>,
    pub version: String,
    pub enabled: bool,
    /// Live plugin state as JSON, serialized because the wire format is
    /// bincode and `serde_json::Value` is not directly encodable by it.
    pub state_json: String,
    pub actions: Vec<PluginActionDto>,
    pub html_template: Option<String>,
}

/// DTO for one declared plugin action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginActionDto {
    pub name: String,
    pub description: String,
}

/// DTO for agent info in `AgentsUpdate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfoDto {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

impl From<crate::agents::AgentInfo> for AgentInfoDto {
    fn from(info: crate::agents::AgentInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            description: info.description,
        }
    }
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
// Media payload (A1)
// ============================================================================

/// Kind of media attached to a tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Audio,
    Pdf,
    Html,
    Canvas,
}

/// A media payload attached to a `ToolResult` frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaPayload {
    pub kind: MediaKind,
    /// Path to the media file on the agent's filesystem.
    pub path: Option<String>,
    /// MIME type (e.g. "image/png", "audio/wav").
    pub mime: Option<String>,
    /// Inline bytes (base64-encoded for transport where needed).
    pub data: Option<Vec<u8>>,
}

// ============================================================================
// Cron DTOs (A2)
// ============================================================================

/// DTO for a scheduled cron job.
///
/// The trailing fields are appended (positional bincode: new fields go
/// last, defaulted) and carry the wake-schedule surface: whether the
/// payload is an agent turn, its model override, the target thread, and
/// the owning agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronJobDto {
    pub id: String,
    pub name: String,
    pub expr: String,
    pub payload: String,
    pub paused: bool,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_status: Option<String>,
    pub run_count: u64,
    #[serde(default)]
    pub agent_turn: bool,
    /// Provider the pinned model belongs to. `None` follows the gateway's
    /// current provider, which is what pre-provider jobs and clients do.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thread_id: Option<u64>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

// ============================================================================
// Memory DTOs (A3)
// ============================================================================

/// DTO for a memory entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntryDto {
    pub id: String,
    pub content: String,
    pub category: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub score: Option<f64>,
}

/// DTO for a history entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntryDto {
    pub timestamp: String,
    pub role: String,
    pub content: String,
    pub thread_id: Option<u64>,
}

// ============================================================================
// Analytics DTOs (A4)
// ============================================================================

/// Aggregate usage totals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageTotalsDto {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_latency_ms: u64,
    pub period: String,
}

/// Per-model usage breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelUsageDto {
    pub provider: String,
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub avg_latency_ms: u64,
}

/// Per-session usage breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionUsageDto {
    pub session_id: String,
    pub thread_label: Option<String>,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

// ============================================================================
// MCP DTOs (A6)
// ============================================================================

/// DTO for an MCP server connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerDto {
    pub name: String,
    pub status: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub tools: Vec<String>,
    pub health_ok: Option<bool>,
}

// ============================================================================
// Tool Config DTOs (A7)
// ============================================================================

/// DTO for tool configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolConfigDto {
    pub name: String,
    pub category: String,
    pub enabled: bool,
    pub policy: String,
    pub description: String,
}

// ============================================================================
// Channel DTOs (A8)
// ============================================================================

/// DTO for messenger channel status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelStatusDto {
    pub name: String,
    pub channel_type: String,
    pub paired: bool,
    pub online: bool,
    pub last_message: Option<String>,
}

// ============================================================================
// Messenger setup DTOs
// ============================================================================

/// One messenger account as the configuration UI sees it.
///
/// Secret values are never carried here in either direction on a *read*: the
/// gateway reports which secret fields are set and where they live, not what
/// they are. A client that wanted to display a bot token would be a client
/// that leaks one over the wire and into a scrollback buffer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessengerAccountDto {
    /// Account name, unique within the config.
    pub name: String,
    /// Messenger type id, matching a `KindSpec` in
    /// [`crate::messengers::setup::KINDS`].
    pub messenger_type: String,
    /// Whether the gateway should connect this account.
    pub enabled: bool,
    /// Non-secret field values, keyed by schema field name.
    pub fields: BTreeMap<String, String>,
    /// Secret fields that have a value in the vault, and the credential name
    /// holding it.
    pub vaulted: BTreeMap<String, String>,
    /// Secret fields still sitting in plaintext config, as `(field, label)`.
    /// Empty for accounts created through this UI.
    pub plaintext: Vec<(String, String)>,
    /// Presented identity, with unset fields already resolved against the
    /// agent's own name and description.
    pub profile: MessengerProfileDto,
    /// Whether the gateway build can actually run this messenger type.
    pub available: bool,
    /// Why not, when `available` is false.
    pub unavailable_reason: Option<String>,
}

/// The identity an account presents, and how much of it is inherited.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MessengerProfileDto {
    /// Name after fallback — what other people in the chat actually see.
    pub display_name: String,
    /// About/status text after fallback.
    pub bio: Option<String>,
    /// Avatar image path, where the backend supports one.
    pub avatar_path: Option<PathBuf>,
    /// Agent this account speaks as.
    pub agent_id: String,
    /// Whether `display_name` is an override rather than the agent's name.
    pub display_name_overridden: bool,
    /// Whether `bio` is an override rather than the agent's description.
    pub bio_overridden: bool,
}

/// A binding from a messenger channel to a gateway thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadRouteDto {
    /// Account the route applies to.
    pub messenger: String,
    /// Channel id, or `None` for every channel on the account.
    pub channel: Option<String>,
    /// Thread the conversation belongs to.
    pub thread_id: u64,
    /// Agent owning the thread.
    pub agent_id: String,
    /// Whether the route is in effect.
    pub enabled: bool,
    /// Thread's label, for display. `None` when the route points at a thread
    /// that no longer exists — which is exactly when a user needs to see it.
    pub thread_label: Option<String>,
}

/// A thread a route may point at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoutableThreadDto {
    pub thread_id: u64,
    pub label: String,
    pub agent_id: String,
}

// ============================================================================
// Approvals DTOs (B4)
// ============================================================================

/// DTO for a pending approval entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingApprovalDto {
    pub id: String,
    pub tool_name: String,
    pub arguments: String,
    pub requested_at: String,
}

/// DTO for one transfer in `DownloadsUpdate`.
///
/// Mirrors [`crate::downloads::Download`] minus its origin: the gateway sends
/// a client only the transfers that client's own connection started, so
/// repeating the connection id on the wire would tell it nothing it could act
/// on and would leak how many other agents the gateway is serving.
///
/// The status is split into a name and an optional reason rather than sent as
/// an enum with a payload. A client older than a future status renders the
/// name it does not recognise as-is instead of failing to decode the whole
/// frame — and every status a transfer can end in is worth showing even to a
/// client that has no special handling for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadInfoDto {
    pub id: String,
    pub url: String,
    /// Where the bytes are going, as an absolute path. The user needs to be
    /// able to find the file afterwards, which a relative path would not
    /// allow — the client does not know the workspace directory.
    pub dest: PathBuf,
    /// `None` for a chunked response, which declares no length. The panel
    /// shows progress without a percentage rather than inventing one.
    pub total_bytes: Option<u64>,
    pub received_bytes: u64,
    /// One of `running`, `complete`, `failed`, `cancelled`.
    pub status: String,
    /// Why it failed, when it did.
    pub error: Option<String>,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
}

impl From<crate::downloads::Download> for DownloadInfoDto {
    fn from(d: crate::downloads::Download) -> Self {
        use crate::downloads::DownloadStatus;
        let (status, error) = match d.status {
            DownloadStatus::Running => ("running", None),
            DownloadStatus::Complete => ("complete", None),
            DownloadStatus::Failed { error } => ("failed", Some(error)),
            DownloadStatus::Cancelled => ("cancelled", None),
        };
        Self {
            id: d.id,
            url: d.url,
            dest: d.dest,
            total_bytes: d.total_bytes,
            received_bytes: d.received_bytes,
            status: status.to_string(),
            error,
            started_ms: d.started_ms,
            finished_ms: d.finished_ms,
        }
    }
}
