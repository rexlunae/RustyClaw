use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum PluginError {
    /// I/O error during plugin load or state persistence.
    Io(std::io::Error, PathBuf),
    /// TOML parse error in a plugin manifest or state file.
    Parse(toml::de::Error, PathBuf),
    /// Serialization error.
    Serialize(toml::ser::Error, String),
    /// Plugin not found.
    NotFound(String),
    /// Plugin already exists (on create).
    AlreadyExists(String),
    /// Validation error (name, schema, etc.).
    Invalid(String),
    /// Other error.
    Other(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e, path) => write!(f, "plugin I/O error at {}: {e}", path.display()),
            Self::Parse(e, path) => write!(f, "plugin TOML error in {}: {e}", path.display()),
            Self::Serialize(e, name) => write!(f, "plugin serialization error ({name}): {e}"),
            Self::NotFound(name) => write!(f, "plugin not found: {name}"),
            Self::AlreadyExists(name) => write!(f, "plugin already exists: {name}"),
            Self::Invalid(msg) => write!(f, "invalid plugin: {msg}"),
            Self::Other(msg) => write!(f, "plugin error: {msg}"),
        }
    }
}

impl std::error::Error for PluginError {}

impl From<std::io::Error> for PluginError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e, PathBuf::new())
    }
}
