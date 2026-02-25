//! MCP (Model Context Protocol) tools.
//!
//! These tools manage MCP server connections. Note that MCP server tools
//! themselves are dynamically registered with prefixed names (mcp_<server>_<tool>)
//! and dispatched through the gateway's MCP handler.

use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

/// List connected MCP servers and their tools.
#[instrument(skip(_args, _workspace_dir), fields(action = "list"))]
pub fn exec_mcp_list(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    debug!("Listing MCP servers");

    // This is a stub — the gateway intercepts this and uses its MCP manager
    #[cfg(feature = "mcp")]
    {
        Ok(json!({
            "status": "stub",
            "note": "MCP server listing requires gateway connection. Use /mcp status in the TUI.",
            "feature": "mcp",
            "enabled": true,
        }).to_string())
    }

    #[cfg(not(feature = "mcp"))]
    {
        Ok(json!({
            "status": "disabled",
            "note": "MCP support requires the 'mcp' feature. Rebuild with: cargo build --features mcp",
            "feature": "mcp",
            "enabled": false,
        }).to_string())
    }
}

/// Connect to an MCP server.
#[instrument(skip(args, _workspace_dir), fields(action = "connect"))]
pub fn exec_mcp_connect(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let name = args.get("name").and_then(|v| v.as_str());
    let command = args.get("command").and_then(|v| v.as_str());

    debug!(?name, ?command, "Connecting to MCP server");

    if name.is_none() && command.is_none() {
        return Err("Either 'name' (from config) or 'command' is required".to_string());
    }

    // This is a stub — the gateway handles actual connections
    Ok(json!({
        "status": "stub",
        "note": "MCP server connection requires gateway. Use /mcp connect <name> in the TUI.",
        "name": name,
        "command": command,
    }).to_string())
}

/// Disconnect from an MCP server.
#[instrument(skip(args, _workspace_dir), fields(action = "disconnect"))]
pub fn exec_mcp_disconnect(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: name")?;

    debug!(name, "Disconnecting from MCP server");

    // This is a stub — the gateway handles actual disconnections
    Ok(json!({
        "status": "stub",
        "note": "MCP server disconnection requires gateway. Use /mcp disconnect <name> in the TUI.",
        "name": name,
    }).to_string())
}

/// Get parameter definitions for MCP tools.
pub fn mcp_list_params() -> Vec<super::ToolParam> {
    vec![]
}

pub fn mcp_connect_params() -> Vec<super::ToolParam> {
    use super::ToolParam;

    vec![
        ToolParam {
            name: "name".into(),
            description: "Server name from config (mcp.servers.<name>)".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "command".into(),
            description: "Command to run (e.g., 'npx', 'uvx', '/path/to/binary')".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "args".into(),
            description: "Arguments to pass to the command".into(),
            param_type: "array".into(),
            required: false,
        },
    ]
}

pub fn mcp_disconnect_params() -> Vec<super::ToolParam> {
    use super::ToolParam;

    vec![ToolParam {
        name: "name".into(),
        description: "Server name to disconnect".into(),
        param_type: "string".into(),
        required: true,
    }]
}
