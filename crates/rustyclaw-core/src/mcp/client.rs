//! MCP client implementation using the rmcp crate.

use anyhow::{Context, Result};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, ListToolsResult},
    service::RunningService,
    transport::TokioChildProcess,
};
use serde_json::Map as JsonMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::config::McpServerConfig;
use super::tools::{McpContent, McpTool, McpToolResult};

/// A connected MCP server client.
pub struct McpClient {
    /// Server name for identification
    name: String,

    /// The underlying rmcp running service (owns the peer)
    service: Arc<Mutex<Option<RunningService<RoleClient, ()>>>>,

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
            service: Arc::new(Mutex::new(None)),
            tools: Arc::new(Mutex::new(Vec::new())),
            config,
        }
    }

    /// Connect to the MCP server.
    pub async fn connect(&self) -> Result<()> {
        info!(server = %self.name, command = %self.config.command, "Connecting to MCP server");

        // Validate the command before spawning (reject null bytes, excessive length,
        // credential exfiltration patterns).
        let full_cmd = format!("{} {}", self.config.command, self.config.args.join(" "));
        if let Err(e) = crate::tools::helpers::validate_command_safe(&full_cmd) {
            warn!(server = %self.name, error = %e, "MCP server command rejected by security validation");
            return Err(anyhow::anyhow!("MCP server command rejected: {}", e));
        }

        // Decide how to launch before touching a Command, so the decision is
        // a value that can be logged and tested rather than a sequence of
        // mutations (issue #230).
        // `cached()` so the probes run once per process rather than once per
        // server, and `spawn_blocking` because they are process spawns and
        // this is a tokio worker thread.
        let caps = tokio::task::spawn_blocking(crate::sandbox::SandboxCapabilities::cached)
            .await
            .map_err(|e| anyhow::anyhow!("Sandbox capability probe failed: {e}"))?;
        // The reason says why on its own. Naming `sandbox = "required"` here
        // was wrong for every refusal that is not about the platform's
        // capabilities — a misconfigured `cwd` is refused whatever the mode,
        // and telling the operator they had set `required` when they had not
        // sends them to fix the wrong line.
        let plan = super::spawn::plan_spawn(&self.config, caps).map_err(|reason| {
            anyhow::anyhow!("MCP server '{}' cannot start: {reason}", self.name)
        })?;

        match &plan.unconfined_reason {
            Some(reason) => warn!(
                server = %self.name,
                reason = %reason,
                "MCP server is running WITHOUT process confinement — it can read \
                 anything this process can, including the vault. Set sandbox = \
                 \"required\" to refuse instead, or sandbox = \"off\" to silence this."
            ),
            None if plan.confined => info!(server = %self.name, "MCP server confined"),
            None => debug!(server = %self.name, "MCP server confinement disabled by config"),
        }

        let mut cmd = Command::new(&plan.program);
        cmd.args(&plan.args);

        if plan.clear_env {
            cmd.env_clear();
        }
        for (key, value) in &plan.env {
            cmd.env(key, value);
        }

        // Set working directory if specified
        if let Some(ref cwd) = self.config.cwd {
            cmd.current_dir(cwd);
        }

        // Create stdio transport - pass owned Command (rmcp 0.16 API)
        let transport =
            TokioChildProcess::new(cmd).context("Failed to spawn MCP server process")?;

        // Connect and initialize - returns RunningService which derefs to Peer
        let running = ().serve(transport).await.context("Failed to initialize MCP connection")?;

        // Store the running service
        *self.service.lock().await = Some(running);

        // Refresh tools list
        self.refresh_tools().await?;

        info!(server = %self.name, "MCP server connected");
        Ok(())
    }

    /// Check if connected.
    pub async fn is_connected(&self) -> bool {
        self.service.lock().await.is_some()
    }

    /// Disconnect from the MCP server.
    pub async fn disconnect(&self) -> Result<()> {
        if let Some(service) = self.service.lock().await.take() {
            // The handle is taken either way, so this connection is finished
            // from our side. But announcing a clean disconnect after a failed
            // cancel would be reporting something that did not happen — the
            // server may still consider the session open.
            match service.cancel().await {
                Ok(reason) => {
                    info!(server = %self.name, ?reason, "MCP server disconnected")
                }
                Err(e) => warn!(
                    server = %self.name,
                    error = %e,
                    "MCP server dropped without a clean cancel"
                ),
            }
        }
        Ok(())
    }

    /// Refresh the list of available tools.
    pub async fn refresh_tools(&self) -> Result<Vec<McpTool>> {
        let service_guard = self.service.lock().await;
        let service = service_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected to MCP server"))?;

        let result: ListToolsResult = service
            .list_tools(None)
            .await
            .context("Failed to list tools")?;

        let tools: Vec<McpTool> = result
            .tools
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
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult> {
        let service_guard = self.service.lock().await;
        let service = service_guard
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Not connected to MCP server"))?;

        debug!(server = %self.name, tool = tool_name, "Calling MCP tool");

        // Convert arguments to JsonObject (serde_json::Map)
        let args_map: Option<JsonMap<String, serde_json::Value>> = match arguments {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            _ => {
                return Ok(McpToolResult::error("Arguments must be an object"));
            }
        };

        let params = CallToolRequestParams {
            name: tool_name.to_string().into(),
            arguments: args_map,
            meta: None,
            task: None,
        };

        match service.call_tool(params).await {
            Ok(result) => {
                use rmcp::model::RawContent;

                let content: Vec<McpContent> = result
                    .content
                    .into_iter()
                    .map(|c| {
                        // Annotated<RawContent> derefs to RawContent
                        match &*c {
                            RawContent::Text(t) => McpContent::Text {
                                text: t.text.clone(),
                            },
                            RawContent::Image(i) => McpContent::Image {
                                data: i.data.clone(),
                                mime_type: i.mime_type.clone(),
                            },
                            RawContent::Resource(r) => {
                                use rmcp::model::ResourceContents;
                                match &r.resource {
                                    ResourceContents::TextResourceContents {
                                        uri,
                                        mime_type,
                                        text,
                                        ..
                                    } => McpContent::Resource {
                                        uri: uri.clone(),
                                        mime_type: mime_type.clone(),
                                        text: Some(text.clone()),
                                    },
                                    ResourceContents::BlobResourceContents {
                                        uri,
                                        mime_type,
                                        ..
                                    } => McpContent::Resource {
                                        uri: uri.clone(),
                                        mime_type: mime_type.clone(),
                                        text: None,
                                    },
                                }
                            }
                            RawContent::Audio(a) => McpContent::Text {
                                text: format!("[Audio: {} bytes, {}]", a.data.len(), a.mime_type),
                            },
                            RawContent::ResourceLink(r) => McpContent::Resource {
                                uri: r.uri.clone(),
                                mime_type: r.mime_type.clone(),
                                text: r.description.clone(),
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
