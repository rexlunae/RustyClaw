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
