//! Device tools: nodes and canvas.
//!
//! Split into submodules for maintainability.

mod canvas;
mod nodes;

// Re-export sync functions
pub use canvas::exec_canvas;
pub use nodes::exec_nodes;

// Re-export async functions
pub use canvas::exec_canvas_async;
pub use nodes::exec_nodes_async;

// ── Shared helpers ──────────────────────────────────────────────────────────

use serde_json::Value;
use std::process::Command;

/// Run a shell pipeline via `sh -c` (sync).
pub(crate) fn sh(script: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(stdout)
}

/// Run a shell pipeline via `sh -c` (async).
pub(crate) async fn sh_async(script: &str) -> Result<String, String> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .await
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(stdout)
}

/// Check if a command is available (sync).
pub(crate) fn has_command(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if a command is available (async).
pub(crate) async fn has_command_async(cmd: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(cmd)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Extract node identifier from args.
pub(crate) fn get_node(args: &Value) -> Result<String, String> {
    args.get("node")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing required parameter: node".to_string())
}

/// Extract command array from args.
pub(crate) fn get_command_array(args: &Value) -> Result<Vec<String>, String> {
    if let Some(arr) = args.get("command").and_then(|v| v.as_array()) {
        Ok(arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect())
    } else if let Some(s) = args.get("command").and_then(|v| v.as_str()) {
        Ok(vec![s.to_string()])
    } else {
        Err("Missing required parameter: command".to_string())
    }
}

/// Shell-quote a command (simple version).
#[allow(dead_code)]
pub(crate) fn shell_quote(parts: &[String]) -> String {
    parts
        .iter()
        .map(|s| {
            if s.contains(char::is_whitespace) || s.contains('\'') || s.contains('"') {
                format!("'{}'", s.replace('\'', "'\\''"))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Node type enumeration.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub(crate) enum NodeType {
    Ssh,
    Adb,
    Vnc,
    Rdp,
}

/// Parse node identifier to determine type.
#[allow(dead_code)]
pub(crate) fn parse_node_type(node: &str) -> (NodeType, String) {
    if node.starts_with("adb:") {
        (NodeType::Adb, node[4..].to_string())
    } else if node.starts_with("vnc:") {
        (NodeType::Vnc, node[4..].to_string())
    } else if node.starts_with("rdp:") {
        (NodeType::Rdp, node[4..].to_string())
    } else if node.starts_with("ssh:") {
        (NodeType::Ssh, node[4..].to_string())
    } else if node.contains('@') {
        // Default SSH for user@host
        (NodeType::Ssh, node.to_string())
    } else {
        // Could be ADB device serial or SSH host
        (NodeType::Adb, node.to_string())
    }
}
