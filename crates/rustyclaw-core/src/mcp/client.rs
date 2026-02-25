//! MCP client implementation using the rmcp crate.

use anyhow::{Context, Result};
use rmcp::{
    ServiceExt,
    transport::TokioChildProcess,
    model::{CallToolRequestParam, ListToolsResult},
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::config::McpServerConfig;
use super::tools::{McpTool, McpToolResult, McpContent};

/// A connected MCP server client.
pub struct McpClient {
    /// Server name for identification
    name: String,

    /// The underlying rmcp service peer
    peer: Arc<Mutex<Option<rmcp::Peer<rmcp::RoleClient>>>>,

    /// Cached tools from this server
    tools: Arc<Mutex<Vec<McpTool>>>,

    /// Configuration
    config: McpServerConfig,
}

impl McpClient {
    /// Create a new MCP client (not yet connected).
    pub fn new(name: String, config: McpServerConfig) -> Self {
        Self {
            name,
            peer: Arc::new(Mutex::new(None)),
            tools: Arc::new(Mutex::new(Vec::new())),
            config,
        }
    }

    /// Connect to the MCP server.
    pub async fn connect(&self) -> Result<()> {
        info!(server = %self.name, command = %self.config.command, "Connecting to MCP server");

        let mut cmd = Command::new(&self.config.command);
        cmd.args(&self.config.args);

        // Set environment variables
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        // Set working directory if specified
        if let Some(ref cwd) = self.config.cwd {
            cmd.current_dir(cwd);
        }

        // Create stdio transport
        let transport = TokioChildProcess::new(&mut cmd)
            .context("Failed to spawn MCP server process")?;

        // Connect and initialize
        let peer = ().serve(transport).await
            .context("Failed to initialize MCP connection")?;

        // Store the peer
        *self.peer.lock().await = Some(peer);

        // Refresh tools list
        self.refresh_tools().await?;

        info!(server = %self.name, "MCP server connected");
        Ok(())
    }

    /// Check if connected.
    pub async fn is_connected(&self) -> bool {
        self.peer.lock().await.is_some()
    }

    /// Disconnect from the MCP server.
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(peer) = self.peer.lock().await.take() {
            peer.cancel().await?;
            info!(server = %self.name, "MCP server disconnected");
        }
        Ok(())
    }

    /// Refresh the list of available tools.
    pub async fn refresh_tools(&self) -> Result<Vec<McpTool>> {
        let peer_guard = self.peer.lock().await;
        let peer = peer_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected to MCP server"))?;

        let result: ListToolsResult = peer.list_tools(None).await
            .context("Failed to list tools")?;

        let tools: Vec<McpTool> = result.tools
            .into_iter()
            .map(|t| McpTool {
                name: t.name.to_string(),
                description: t.description.map(|s| s.to_string()),
                input_schema: serde_json::to_value(&t.input_schema).unwrap_or_default(),
                server_name: self.name.clone(),
            })
            .collect();

        debug!(server = %self.name, count = tools.len(), "Refreshed MCP tools");

        // Cache the tools
        *self.tools.lock().await = tools.clone();

        Ok(tools)
    }

    /// Get cached tools.
    pub async fn get_tools(&self) -> Vec<McpTool> {
        self.tools.lock().await.clone()
    }

    /// Call a tool on this MCP server.
    pub async fn call_tool(&self, tool_name: &str, arguments: serde_json::Value) -> Result<McpToolResult> {
        let peer_guard = self.peer.lock().await;
        let peer = peer_guard.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected to MCP server"))?;

        debug!(server = %self.name, tool = tool_name, "Calling MCP tool");

        // Convert arguments to the expected format
        let args_map: Option<HashMap<String, serde_json::Value>> = match arguments {
            serde_json::Value::Object(map) => {
                Some(map.into_iter().collect())
            }
            serde_json::Value::Null => None,
            _ => {
                return Ok(McpToolResult::error("Arguments must be an object"));
            }
        };

        let params = CallToolRequestParam {
            name: tool_name.into(),
            arguments: args_map,
        };

        match peer.call_tool(params).await {
            Ok(result) => {
                let content: Vec<McpContent> = result.content
                    .into_iter()
                    .map(|c| {
                        // Convert rmcp content to our McpContent type
                        match c {
                            rmcp::model::Content::Text(t) => McpContent::Text { 
                                text: t.text.to_string() 
                            },
                            rmcp::model::Content::Image(i) => McpContent::Image {
                                data: i.data.to_string(),
                                mime_type: i.mime_type.to_string(),
                            },
                            rmcp::model::Content::Resource(r) => McpContent::Resource {
                                uri: r.resource.uri.to_string(),
                                mime_type: r.resource.mime_type.map(|s| s.to_string()),
                                text: r.resource.text.map(|s| s.to_string()),
                            },
                        }
                    })
                    .collect();

                Ok(McpToolResult {
                    success: !result.is_error.unwrap_or(false),
                    content,
                    error: None,
                })
            }
            Err(e) => {
                warn!(server = %self.name, tool = tool_name, error = %e, "MCP tool call failed");
                Ok(McpToolResult::error(e.to_string()))
            }
        }
    }

    /// Get the server name.
    pub fn name(&self) -> &str {
        &self.name
    }
}
