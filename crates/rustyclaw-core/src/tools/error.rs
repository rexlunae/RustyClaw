//! Typed errors for AI-tool implementations.
//!
//! [`ToolError`]'s `Display` output is the exact text sent back to the
//! model. Tool implementations return `ToolResult` and propagate typed
//! errors with `?`; the dispatch layer ([`crate::tools::execute_tool`] and
//! the gateway's tool executor) is the **single** place the error is
//! flattened to a string for the model payload.
//!
//! Two kinds of variants exist:
//!
//! * Typed sources (`Io`, `Json`, `Http`, `Sandbox`, `Process`, …) with
//!   `#[from]` conversions so `?` propagates the per-module error enums
//!   without stringifying.
//! * [`ToolError::Msg`] for bespoke, hand-written messages. `From<String>`
//!   and `From<&str>` route existing `format!`-style message construction
//!   here, so adding context at the failure site stays a one-liner:
//!   `.map_err(|e| format!("Failed to read {}: {}", path, e))?`.

use serde_json::Value;

/// Error from an AI-tool implementation. `Display` is the model-facing text.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// I/O failure (files, pipes, spawned processes).
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// HTTP request failure.
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    /// Sandbox policy verdict or sandboxed-execution failure.
    #[error(transparent)]
    Sandbox(#[from] crate::sandbox::SandboxError),
    /// Background-process manager failure.
    #[error(transparent)]
    Process(#[from] crate::process_manager::ProcessError),
    /// Task-manager failure.
    #[error(transparent)]
    Task(#[from] crate::tasks::TaskError),
    /// Service-manager failure.
    #[error(transparent)]
    Service(#[from] crate::services::ServiceError),
    /// Model-registry failure.
    #[error(transparent)]
    Registry(#[from] crate::models::RegistryError),
    /// Cron scheduler failure.
    #[error(transparent)]
    Cron(#[from] crate::cron::CronError),
    /// Memory consolidation failure.
    #[error(transparent)]
    Consolidation(#[from] crate::memory_consolidation::ConsolidationError),
    /// Memory file/index failure.
    #[error(transparent)]
    MemoryIndex(#[from] crate::memory::MemoryIndexError),
    /// Session-manager failure.
    #[error(transparent)]
    Session(#[from] crate::sessions::SessionError),
    /// Swarm-manager failure.
    #[error(transparent)]
    Swarm(#[from] crate::swarm::SwarmError),
    /// Semantic-memory failure.
    #[cfg(feature = "semantic-memory")]
    #[error(transparent)]
    SteelMemory(#[from] crate::steel_memory::SteelMemoryError),
    /// Bespoke tool error message.
    #[error("{0}")]
    Msg(String),
}

impl ToolError {
    /// Construct a bespoke message error.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Msg(message.into())
    }
}

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        Self::Msg(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        Self::Msg(message.to_string())
    }
}

/// Result type for AI-tool implementations.
///
/// The `Ok` payload defaults to the tool's output string; internal helpers
/// use other payloads (e.g. `ToolResult<Value>`, `ToolResult<PathBuf>`).
pub type ToolResult<T = String> = std::result::Result<T, ToolError>;

/// Convenience for argument validation.
pub fn missing_param(name: &str) -> ToolError {
    ToolError::Msg(format!("Missing required parameter: {}", name))
}

/// Extract a required string parameter from tool arguments.
pub fn require_str<'a>(args: &'a Value, name: &str) -> ToolResult<&'a str> {
    args.get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| missing_param(name))
}
