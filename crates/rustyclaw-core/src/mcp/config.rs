//! MCP server configuration types.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Transport used to reach an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// Spawn a local child process and speak MCP over stdio (default).
    #[default]
    Stdio,
    /// Connect to a remote server over streamable HTTP (SSE-style).
    Http,
}

/// Configuration for an MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to run (e.g., "npx", "uvx", "/path/to/binary").
    /// Unused when `transport = "http"`.
    #[serde(default)]
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

    /// Transport to reach the server: `"stdio"` (default) or `"http"`.
    #[serde(default)]
    pub transport: McpTransport,

    /// Base URL for `transport = "http"`, e.g. `https://mcp.example.com/mcp`.
    #[serde(default)]
    pub url: Option<String>,

    /// Bearer token sent in the `Authorization` header for HTTP transport.
    #[serde(default)]
    pub auth_token: Option<String>,
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
            transport: McpTransport::default(),
            url: None,
            auth_token: None,
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
        // Old configs (no transport key) default to stdio with no URL.
        assert_eq!(config.servers["github"].transport, McpTransport::Stdio);
        assert_eq!(config.servers["github"].url, None);
        assert_eq!(config.servers["github"].auth_token, None);
    }

    #[test]
    fn test_http_server_deserialize() {
        let toml = r#"
            [servers.remote]
            transport = "http"
            url = "https://mcp.example.com/mcp"
            auth_token = "sekret"

            [servers.bare]
            transport = "http"
            url = "https://mcp2.example.com/mcp"
        "#;

        let config: McpConfig = toml::from_str(toml).unwrap();
        let remote = &config.servers["remote"];
        assert_eq!(remote.transport, McpTransport::Http);
        assert_eq!(remote.url.as_deref(), Some("https://mcp.example.com/mcp"));
        assert_eq!(remote.auth_token.as_deref(), Some("sekret"));
        // command is optional; defaults to empty for http-only servers.
        assert_eq!(remote.command, "");
        // No command field required at all.
        assert_eq!(config.servers["bare"].command, "");
        assert_eq!(config.servers["bare"].auth_token, None);
    }

    #[test]
    fn test_http_config_round_trip() {
        let config = McpConfig {
            servers: [(
                "remote".to_string(),
                McpServerConfig {
                    command: String::new(),
                    transport: McpTransport::Http,
                    url: Some("https://mcp.example.com/mcp".to_string()),
                    auth_token: Some("sekret".to_string()),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
        };
        let toml_str = toml::to_string(&config).unwrap();
        let loaded: McpConfig = toml::from_str(&toml_str).unwrap();
        let remote = &loaded.servers["remote"];
        assert_eq!(remote.transport, McpTransport::Http);
        assert_eq!(remote.url.as_deref(), Some("https://mcp.example.com/mcp"));
        assert_eq!(remote.auth_token.as_deref(), Some("sekret"));
    }
}
