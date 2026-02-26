//! MCP tool execution handler for the gateway.

#[cfg(feature = "mcp")]
use tracing::{debug, instrument, warn};

#[cfg(feature = "mcp")]
use crate::mcp::McpManager;
#[cfg(feature = "mcp")]
use std::sync::Arc;
#[cfg(feature = "mcp")]
use tokio::sync::Mutex;

#[cfg(feature = "mcp")]
pub type SharedMcpManager = Arc<Mutex<McpManager>>;

/// Check if a tool name is an MCP tool.
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp_")
}

/// Execute an MCP tool call.
///
/// Returns Ok(output) on success, Err(error_message) on failure.
#[cfg(feature = "mcp")]
#[instrument(skip(args, mcp_mgr), fields(%name))]
pub async fn execute_mcp_tool(
    name: &str,
    args: &serde_json::Value,
    mcp_mgr: &SharedMcpManager,
) -> Result<String, String> {
    debug!("Executing MCP tool");

    let mgr = mcp_mgr.lock().await;

    match mgr.call_tool_by_name(name, args.clone()).await {
        Ok(result) => {
            if result.success {
                Ok(result.to_llm_string())
            } else {
                Err(result
                    .error
                    .unwrap_or_else(|| "Unknown MCP error".to_string()))
            }
        }
        Err(e) => {
            warn!(tool = name, error = %e, "MCP tool call failed");
            Err(e.to_string())
        }
    }
}

/// Stub for when MCP feature is disabled.
#[cfg(not(feature = "mcp"))]
pub async fn execute_mcp_tool(
    name: &str,
    _args: &serde_json::Value,
    _mcp_mgr: &(),
) -> Result<String, String> {
    Err(format!(
        "MCP tool '{}' called but MCP support is not enabled. Rebuild with --features mcp",
        name
    ))
}

/// Get MCP tool schemas for the system prompt.
#[cfg(feature = "mcp")]
pub async fn get_mcp_tool_schemas(mcp_mgr: &SharedMcpManager) -> Vec<serde_json::Value> {
    let mgr = mcp_mgr.lock().await;
    mgr.get_tool_schemas().await
}

/// Stub for when MCP feature is disabled.
#[cfg(not(feature = "mcp"))]
pub async fn get_mcp_tool_schemas(_mcp_mgr: &()) -> Vec<serde_json::Value> {
    Vec::new()
}

/// Generate MCP tools section for the system prompt.
#[cfg(feature = "mcp")]
pub async fn generate_mcp_prompt_section(mcp_mgr: &SharedMcpManager) -> String {
    let mgr = mcp_mgr.lock().await;
    let tools = mgr.list_all_tools().await;

    if tools.is_empty() {
        return String::new();
    }

    let mut section = String::from("\n## MCP Tools\n\n");
    section.push_str("The following tools are provided by connected MCP servers:\n\n");

    // Group by server
    let mut by_server: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
    for tool in tools {
        by_server
            .entry(tool.server_name.clone())
            .or_default()
            .push(tool);
    }

    for (server, tools) in by_server {
        section.push_str(&format!("### {} (MCP server)\n\n", server));
        for tool in tools {
            section.push_str(&format!(
                "- **{}**: {}\n",
                tool.prefixed_name(),
                tool.description.as_deref().unwrap_or("(no description)")
            ));
        }
        section.push('\n');
    }

    section
}

/// Stub for when MCP feature is disabled.
#[cfg(not(feature = "mcp"))]
pub async fn generate_mcp_prompt_section(_mcp_mgr: &()) -> String {
    String::new()
}
