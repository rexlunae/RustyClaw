//! MCP manager for handling multiple server connections.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::client::McpClient;
use super::config::{McpConfig, McpServerConfig};
use super::tools::{McpTool, McpToolCall, McpToolResult};

/// Manages multiple MCP server connections.
pub struct McpManager {
    /// Connected clients by server name
    clients: Arc<RwLock<HashMap<String, McpClient>>>,

    /// Configuration
    config: McpConfig,
}

impl McpManager {
    /// Create a new MCP manager with configuration.
    pub fn new(config: McpConfig) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    /// Connect to all enabled MCP servers.
    pub async fn connect_all(&self) -> Result<()> {
        for (name, server_config) in self.config.enabled_servers() {
            if let Err(e) = self.connect(name, server_config).await {
                warn!(server = name, error = %e, "Failed to connect to MCP server");
            }
        }
        Ok(())
    }

    /// Connect to a specific MCP server.
    pub async fn connect(&self, name: &str, config: &McpServerConfig) -> Result<()> {
        let client = McpClient::new(name.to_string(), config.clone());
        client.connect().await?;

        self.clients.write().await.insert(name.to_string(), client);
        info!(server = name, "MCP server added to manager");
        Ok(())
    }

    /// Disconnect from all MCP servers.
    pub async fn disconnect_all(&self) -> Result<()> {
        let mut clients = self.clients.write().await;
        for (name, client) in clients.drain() {
            if let Err(e) = client.disconnect().await {
                warn!(server = name, error = %e, "Error disconnecting MCP server");
            }
        }
        Ok(())
    }

    /// Disconnect from a specific server.
    pub async fn disconnect(&self, name: &str) -> Result<()> {
        if let Some(client) = self.clients.write().await.remove(name) {
            client.disconnect().await?;
        }
        Ok(())
    }

    /// List all connected servers.
    pub async fn list_servers(&self) -> Vec<String> {
        self.clients.read().await.keys().cloned().collect()
    }

    /// Get all available tools from all connected servers.
    pub async fn list_all_tools(&self) -> Vec<McpTool> {
        let clients = self.clients.read().await;
        let mut all_tools = Vec::new();

        for client in clients.values() {
            all_tools.extend(client.get_tools().await);
        }

        all_tools
    }

    /// Get tools from a specific server.
    pub async fn list_tools(&self, server_name: &str) -> Result<Vec<McpTool>> {
        let clients = self.clients.read().await;
        let client = clients
            .get(server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not connected", server_name))?;

        Ok(client.get_tools().await)
    }

    /// Refresh tools from all connected servers.
    pub async fn refresh_all_tools(&self) -> Result<()> {
        let clients = self.clients.read().await;
        for (name, client) in clients.iter() {
            if let Err(e) = client.refresh_tools().await {
                warn!(server = name, error = %e, "Failed to refresh tools");
            }
        }
        Ok(())
    }

    /// Call a tool by its prefixed name (e.g., "mcp_filesystem_read_file").
    pub async fn call_tool_by_name(
        &self,
        prefixed_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult> {
        let call = McpToolCall::from_prefixed_name(prefixed_name, arguments)
            .ok_or_else(|| anyhow::anyhow!("Invalid MCP tool name: {}", prefixed_name))?;

        self.call_tool(&call).await
    }

    /// Call a tool.
    pub async fn call_tool(&self, call: &McpToolCall) -> Result<McpToolResult> {
        let clients = self.clients.read().await;
        let client = clients
            .get(&call.server_name)
            .ok_or_else(|| anyhow::anyhow!("MCP server '{}' not connected", call.server_name))?;

        client
            .call_tool(&call.tool_name, call.arguments.clone())
            .await
    }

    /// Check if a tool name is an MCP tool (starts with "mcp_").
    pub fn is_mcp_tool(name: &str) -> bool {
        name.starts_with("mcp_")
    }

    /// Generate tool schemas for LLM consumption.
    pub async fn get_tool_schemas(&self) -> Vec<serde_json::Value> {
        self.list_all_tools()
            .await
            .into_iter()
            .map(|t| t.to_llm_tool_schema())
            .collect()
    }

    /// Get status information for all servers.
    pub async fn status(&self) -> McpStatus {
        let clients = self.clients.read().await;
        let mut servers = Vec::new();

        for (name, client) in clients.iter() {
            servers.push(McpServerStatus {
                name: name.clone(),
                connected: client.is_connected().await,
                tool_count: client.get_tools().await.len(),
            });
        }

        McpStatus {
            enabled: !self.config.servers.is_empty(),
            servers,
        }
    }
}

/// Status information for the MCP manager.
#[derive(Debug, Clone)]
pub struct McpStatus {
    pub enabled: bool,
    pub servers: Vec<McpServerStatus>,
}

/// Status information for a single MCP server.
#[derive(Debug, Clone)]
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
}
