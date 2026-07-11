//! Typed errors for the skills module.

use std::path::PathBuf;

/// Errors produced by [`SkillManager`](super::SkillManager) operations.
///
/// Leaf failures (I/O, HTTP, parsing, archives) convert in via `#[from]`
/// so `?` propagates them without stringifying; registry-protocol
/// failures get semantic variants so callers can match on them.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// Filesystem operation on a skill or skills directory failed.
    #[error("Skill I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// HTTP transport failure talking to the ClawHub registry.
    #[error("ClawHub HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// JSON (de)serialization of skill data failed.
    #[error("Skill JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// YAML parsing of a skill file or its frontmatter failed.
    #[error("Skill YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    /// A downloaded skill archive could not be extracted.
    #[error("Skill archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// The referenced skill does not exist.
    #[error("Skill not found: {0}")]
    NotFound(String),
    /// A skill with this name already exists on disk.
    #[error("Skill already exists: {name} (at {})", path.display())]
    AlreadyExists { name: String, path: PathBuf },
    /// The skill name is not usable as a directory name.
    #[error("Invalid skill name '{name}': {reason}")]
    InvalidName { name: String, reason: &'static str },
    /// No writable skills directory is configured.
    #[error("No skills directory configured")]
    NoSkillsDir,
    /// The ClawHub registry base URL did not respond.
    #[error(
        "ClawHub registry ({0}) is not reachable. Check your internet connection \
         or set a custom registry URL with `clawhub_url` in your config."
    )]
    Unreachable(String),
    /// ClawHub replied with a non-success HTTP status.
    #[error("{operation} failed (HTTP {status}): {body}")]
    Status {
        operation: String,
        status: reqwest::StatusCode,
        body: String,
    },
    /// The operation requires ClawHub authentication.
    #[error(
        "Not authenticated with ClawHub. Run `/clawhub auth login` or set `clawhub_token` in config."
    )]
    NotAuthenticated,
    /// A typed error wrapped with human context.
    ///
    /// Renders as `"{context}: {source}"` while keeping the typed error
    /// reachable via [`std::error::Error::source`].
    #[error("{context}: {source}")]
    Context {
        context: String,
        #[source]
        source: Box<SkillError>,
    },
    /// Bespoke error message.
    #[error("{0}")]
    Msg(String),
}

impl SkillError {
    /// Construct a bespoke message error.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Msg(message.into())
    }

    /// Wrap an error with a context prefix, preserving it as `source()`.
    pub fn context(context: impl Into<String>, source: impl Into<SkillError>) -> Self {
        Self::Context {
            context: context.into(),
            source: Box::new(source.into()),
        }
    }

    /// Construct a [`SkillError::Status`] for a non-success registry reply.
    pub(super) fn status(
        operation: impl Into<String>,
        status: reqwest::StatusCode,
        body: String,
    ) -> Self {
        Self::Status {
            operation: operation.into(),
            status,
            body,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn context_preserves_typed_source() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let err = SkillError::context("Failed to read skill file", io);

        assert_eq!(
            err.to_string(),
            "Failed to read skill file: Skill I/O error: gone"
        );

        // The typed source survives the wrapping and is matchable.
        assert!(err.source().is_some());
        match &err {
            SkillError::Context { source, .. } => assert!(matches!(**source, SkillError::Io(_))),
            other => panic!("expected Context variant, got {other:?}"),
        }
    }

    #[test]
    fn status_renders_operation_and_body() {
        let err = SkillError::status(
            "ClawHub publish",
            reqwest::StatusCode::FORBIDDEN,
            "nope".into(),
        );
        assert_eq!(
            err.to_string(),
            "ClawHub publish failed (HTTP 403 Forbidden): nope"
        );
    }
}
