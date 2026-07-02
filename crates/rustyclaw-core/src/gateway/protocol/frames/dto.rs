//! Data-transfer objects carried inside protocol result frames.
//!
//! These are wire-facing mirrors of core domain types. Conversions from the
//! domain types live here as `From` impls so producers can use `.into()`.

use serde::{Deserialize, Serialize};

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
