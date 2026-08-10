//! MCP server configuration types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for an MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to run (e.g., "npx", "uvx", "/path/to/binary")
    pub command: String,

    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Working directory for the process
    #[serde(default)]
    pub cwd: Option<String>,

    /// Whether this server is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Timeout for tool calls in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// How this server's process should be confined.
    #[serde(default)]
    pub sandbox: McpSandbox,

    /// Directories the server may read and write beyond its working directory.
    ///
    /// Only consulted when the server is confined. A filesystem MCP server
    /// pointed at `~/Documents` needs that path listed here, which is the
    /// point: the server sees the directory it was configured with and not
    /// the whole disk (issue #230).
    #[serde(default)]
    pub allow_paths: Vec<String>,
}

/// Whether an MCP server process runs inside a sandbox.
///
/// MCP servers are arbitrary commands from config, and their tools land in the
/// model's tool list looking exactly like first-party ones. A compromised
/// third-party package used to inherit the gateway's whole filesystem and
/// environment, vault included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpSandbox {
    /// Confine if the platform can, and say so loudly if it cannot.
    ///
    /// The default. Running unconfined is reported at WARN with the reason,
    /// rather than silently — a sandbox that quietly is not there is worse
    /// than no sandbox, because it is believed.
    #[default]
    Auto,
    /// Confine, or refuse to start the server.
    ///
    /// For anyone who would rather lose the server than run it unconfined.
    Required,
    /// Do not confine. The pre-#230 behaviour, kept as an escape hatch for
    /// servers that genuinely need the run of the machine.
    Off,
}

fn default_enabled() -> bool {
    true
}

fn default_timeout() -> u64 {
    30
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            enabled: true,
            timeout_secs: 30,
            sandbox: McpSandbox::default(),
            allow_paths: Vec::new(),
        }
    }
}

/// Top-level MCP configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    /// Named MCP server configurations
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

impl McpConfig {
    /// Get enabled servers only
    pub fn enabled_servers(&self) -> impl Iterator<Item = (&String, &McpServerConfig)> {
        self.servers.iter().filter(|(_, cfg)| cfg.enabled)
    }

    /// Check if any servers are configured
    pub fn has_servers(&self) -> bool {
        !self.servers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_config() {
        let toml = r#"
            [servers.filesystem]
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
            
            [servers.github]
            command = "npx"
            args = ["-y", "@modelcontextprotocol/server-github"]
            env = { GITHUB_TOKEN = "test" }
        "#;

        let config: McpConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.servers.len(), 2);
        assert!(config.servers.contains_key("filesystem"));
        assert!(config.servers.contains_key("github"));
    }
}
