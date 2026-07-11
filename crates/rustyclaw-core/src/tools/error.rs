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
//! * [`ToolError::Context`] for wrapping a convertible error with a human
//!   prefix while keeping it reachable via `source()`:
//!   `.map_err(|e| ToolError::context("Failed to read config", e))?`.
//!   Prefer it over `format!` whenever the source implements
//!   `Into<ToolError>` — the rendered text is identical.
//! * [`ToolError::Msg`] for bespoke, hand-written messages. `From<String>`
//!   and `From<&str>` route existing `format!`-style message construction
//!   here, so third-party leaf errors with no `ToolError` conversion keep
//!   the one-liner: `.map_err(|e| format!("Failed to parse {}: {}", path, e))?`.

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
    /// SSRF verdict (blocked URL) or resolution failure.
    #[error(transparent)]
    Ssrf(#[from] crate::security::ssrf::SsrfError),
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
    /// Blocking-task join failure (the worker panicked or was cancelled).
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    /// Swarm-manager failure.
    #[error(transparent)]
    Swarm(#[from] crate::swarm::SwarmError),
    /// Skill-manager failure.
    #[error(transparent)]
    Skill(#[from] crate::skills::SkillError),
    /// Semantic-memory failure.
    #[cfg(feature = "semantic-memory")]
    #[error(transparent)]
    SteelMemory(#[from] crate::steel_memory::SteelMemoryError),
    /// A typed error wrapped with human context.
    ///
    /// Renders as `"{context}: {source}"` — the same text as the old
    /// `format!("context: {e}")` flattening — while keeping the typed
    /// error reachable via [`std::error::Error::source`].
    #[error("{context}: {source}")]
    Context {
        context: String,
        #[source]
        source: Box<ToolError>,
    },
    /// Bespoke tool error message.
    #[error("{0}")]
    Msg(String),
}

impl ToolError {
    /// Construct a bespoke message error.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Msg(message.into())
    }

    /// Wrap an error with a context prefix, preserving it as `source()`.
    ///
    /// Prefer this over `format!("context: {e}")` when the underlying
    /// error converts into [`ToolError`] — the rendered message is
    /// identical, but the typed source survives.
    pub fn context(context: impl Into<String>, source: impl Into<ToolError>) -> Self {
        Self::Context {
            context: context.into(),
            source: Box::new(source.into()),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn context_preserves_typed_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let err = ToolError::context("Failed to read config", io);

        // Message is identical to the old format!("{context}: {e}") text.
        assert_eq!(err.to_string(), "Failed to read config: gone");

        // The typed source survives the wrapping and is matchable.
        let source = err.source().expect("context keeps its source");
        assert_eq!(source.to_string(), "gone");
        match &err {
            ToolError::Context { source, .. } => assert!(matches!(**source, ToolError::Io(_))),
            other => panic!("expected Context variant, got {other:?}"),
        }
    }

    #[test]
    fn from_str_and_string_produce_msg() {
        assert!(matches!(ToolError::from("boom"), ToolError::Msg(_)));
        assert!(matches!(
            ToolError::from(String::from("boom")),
            ToolError::Msg(_)
        ));
    }
}
