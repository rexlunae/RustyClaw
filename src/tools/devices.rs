//! Device tools: nodes and canvas.
//!
//! The nodes tool provides remote device control via:
//! - SSH: For Linux/macOS/Unix remote machines
//! - ADB: For Android devices
//! - VNC: For graphical remote access (requires vncdo or tigervnc)
//! - RDP: For Windows remote desktop (requires xfreerdp or rdesktop)
//!
//! Canvas remains a stub (requires OpenClaw canvas service).

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

/// Discover and control paired nodes via SSH, ADB, VNC, or RDP.
///
/// Supports four transport types:
/// - `ssh`: Remote machines via SSH (requires ssh CLI)
/// - `adb`: Android devices via ADB (requires adb CLI)
/// - `vnc`: VNC remote desktop (requires vncdo or tigervnc-utils)
/// - `rdp`: Windows RDP (requires xfreerdp or rdesktop)
///
/// Node identifiers:
/// - SSH: `user@host` or `ssh:user@host:port`
/// - ADB: `adb:device_id` or just device serial
/// - VNC: `vnc:host:display` or `vnc:host:port` (port > 99 = raw port, else display number)
/// - RDP: `rdp:host` or `rdp:user@host:port`
pub fn exec_nodes(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    match action {
        "status" => node_status(),
        "describe" => {
            let node = get_node(args)?;
            node_describe(&node)
        }
        "run" => {
            let node = get_node(args)?;
            let command = get_command_array(args)?;
            node_run(&node, &command)
        }
        "camera_snap" | "screen_snap" => {
            let node = get_node(args)?;
            let facing = args.get("facing").and_then(|v| v.as_str()).unwrap_or("back");
            node_screen_snap(&node, facing)
        }
        "camera_list" => {
            let node = get_node(args)?;
            adb_camera_list(&node)
        }
        "screen_record" => {
            let node = get_node(args)?;
            let duration = args.get("durationMs").and_then(|v| v.as_u64()).unwrap_or(5000);
            adb_screen_record(&node, duration)
        }
        "location_get" => {
            let node = get_node(args)?;
            adb_location_get(&node)
        }
        "notify" => {
            let node = get_node(args)?;
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("RustyClaw");
            let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
            node_notify(&node, title, body)
        }
        // Mouse/keyboard actions for VNC/RDP
        "click" => {
            let node = get_node(args)?;
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            let button = args.get("button").and_then(|v| v.as_str()).unwrap_or("left");
            node_click(&node, x as i32, y as i32, button)
        }
        "type" => {
            let node = get_node(args)?;
            let text = args.get("text").and_then(|v| v.as_str())
                .ok_or("Missing 'text' for type action")?;
            node_type_text(&node, text)
        }
        "key" => {
            let node = get_node(args)?;
            let key = args.get("key").and_then(|v| v.as_str())
                .ok_or("Missing 'key' for key action")?;
            node_send_key(&node, key)
        }
        // Pairing actions - not applicable for SSH/ADB/VNC/RDP model
        "pending" => Ok(json!({
            "pending": [],
            "note": "Direct connection nodes don't require pairing. Use 'status' to list available devices."
        }).to_string()),
        "approve" | "reject" => Ok("Direct connection nodes don't require pairing approval.".to_string()),
        "invoke" => {
            // Map invoke to run for compatibility
            let node = get_node(args)?;
            let cmd = args.get("invokeCommand").and_then(|v| v.as_str())
                .ok_or("Missing 'invokeCommand'")?;
            node_run(&node, &[cmd])
        }
        _ => Err(format!(
            "Unknown action: {}. Valid: status, describe, run, screen_snap, camera_snap, camera_list, screen_record, location_get, notify, click, type, key, invoke",
            action
        )),
    }
}

/// Extract node identifier from args.
fn get_node(args: &Value) -> Result<String, String> {
    args.get("node")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Missing 'node' parameter".to_string())
}

/// Extract command array from args.
fn get_command_array(args: &Value) -> Result<Vec<String>, String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
        .unwrap_or_default();

    if command.is_empty() {
        return Err("Missing 'command' array for run action".to_string());
    }
    Ok(command)
}

/// Determine node type from identifier.
enum NodeType {
    Ssh { user: String, host: String, port: u16 },
    Adb { device: String },
    Vnc { host: String, port: u16, password: Option<String> },
    Rdp { user: Option<String>, host: String, port: u16 },
}

fn parse_node(node: &str) -> NodeType {
    // Check for explicit prefix
    if node.starts_with("adb:") {
        return NodeType::Adb { device: node[4..].to_string() };
    }
    if node.starts_with("ssh:") {
        let rest = &node[4..];
        return parse_ssh_target(rest);
    }
    if node.starts_with("vnc:") {
        return parse_vnc_target(&node[4..]);
    }
    if node.starts_with("rdp:") {
        return parse_rdp_target(&node[4..]);
    }
    
    // Heuristic: if it contains '@', treat as SSH
    if node.contains('@') {
        return parse_ssh_target(node);
    }
    
    // Default to ADB for device-serial-like strings
    NodeType::Adb { device: node.to_string() }
}

fn parse_ssh_target(target: &str) -> NodeType {
    // Parse user@host:port or user@host
    let (user_host, port) = if let Some(idx) = target.rfind(':') {
        if let Ok(p) = target[idx+1..].parse::<u16>() {
            (&target[..idx], p)
        } else {
            (target, 22)
        }
    } else {
        (target, 22)
    };

    let (user, host) = if let Some(idx) = user_host.find('@') {
        (user_host[..idx].to_string(), user_host[idx+1..].to_string())
    } else {
        ("root".to_string(), user_host.to_string())
    };

    NodeType::Ssh { user, host, port }
}

/// Parse VNC target: host:display or host:port (port > 99 = raw port)
fn parse_vnc_target(target: &str) -> NodeType {
    let (host, port) = if let Some(idx) = target.rfind(':') {
        if let Ok(p) = target[idx+1..].parse::<u16>() {
            // If port > 99, use as-is; otherwise treat as display number
            let actual_port = if p > 99 { p } else { 5900 + p };
            (target[..idx].to_string(), actual_port)
        } else {
            (target.to_string(), 5900)
        }
    } else {
        (target.to_string(), 5900)
    };

    NodeType::Vnc { host, port, password: None }
}

/// Parse RDP target: user@host:port or host:port or just host
fn parse_rdp_target(target: &str) -> NodeType {
    let (user_host, port) = if let Some(idx) = target.rfind(':') {
        if let Ok(p) = target[idx+1..].parse::<u16>() {
            (&target[..idx], p)
        } else {
            (target, 3389)
        }
    } else {
        (target, 3389)
    };

    let (user, host) = if let Some(idx) = user_host.find('@') {
        (Some(user_host[..idx].to_string()), user_host[idx+1..].to_string())
    } else {
        (None, user_host.to_string())
    };

    NodeType::Rdp { user, host, port }
}

/// List available nodes (SSH hosts from config + ADB devices).
fn node_status() -> Result<String, String> {
    let mut nodes = Vec::new();

    // Check for ADB devices
    if let Ok(output) = Command::new("adb").args(["devices", "-l"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[1] == "device" {
                    let device_id = parts[0];
                    let model = parts.iter()
                        .find(|p| p.starts_with("model:"))
                        .map(|p| &p[6..])
                        .unwrap_or("unknown");
                    nodes.push(json!({
                        "id": format!("adb:{}", device_id),
                        "type": "adb",
                        "device": device_id,
                        "model": model,
                        "status": "connected"
                    }));
                }
            }
        }
    }

    // Check SSH config for known hosts
    if let Ok(config) = std::fs::read_to_string(dirs::home_dir().unwrap_or_default().join(".ssh/config")) {
        let mut current_host: Option<String> = None;
        let mut current_user: Option<String> = None;
        let mut current_hostname: Option<String> = None;

        for line in config.lines() {
            let line = line.trim();
            if line.starts_with("Host ") && !line.contains('*') {
                // Save previous host if any
                if let Some(host) = current_host.take() {
                    let user = current_user.take().unwrap_or_else(|| "root".to_string());
                    let hostname = current_hostname.take().unwrap_or_else(|| host.clone());
                    nodes.push(json!({
                        "id": format!("ssh:{}@{}", user, hostname),
                        "type": "ssh",
                        "alias": host,
                        "user": user,
                        "host": hostname,
                        "status": "configured"
                    }));
                }
                current_host = Some(line[5..].trim().to_string());
            } else if line.to_lowercase().starts_with("user ") {
                current_user = Some(line[5..].trim().to_string());
            } else if line.to_lowercase().starts_with("hostname ") {
                current_hostname = Some(line[9..].trim().to_string());
            }
        }

        // Don't forget last host
        if let Some(host) = current_host {
            let user = current_user.unwrap_or_else(|| "root".to_string());
            let hostname = current_hostname.unwrap_or_else(|| host.clone());
            nodes.push(json!({
                "id": format!("ssh:{}@{}", user, hostname),
                "type": "ssh",
                "alias": host,
                "user": user,
                "host": hostname,
                "status": "configured"
            }));
        }
    }

    // Check for VNC/RDP tool availability
    let vnc_tool = if which::which("vncdo").is_ok() {
        Some("vncdo")
    } else if which::which("vncdotool").is_ok() {
        Some("vncdotool")
    } else if which::which("xdotool").is_ok() {
        Some("xdotool (via SSH X11)")
    } else {
        None
    };

    let rdp_tool = if which::which("xfreerdp").is_ok() {
        Some("xfreerdp")
    } else if which::which("xfreerdp3").is_ok() {
        Some("xfreerdp3")
    } else if which::which("rdesktop").is_ok() {
        Some("rdesktop")
    } else {
        None
    };

    Ok(json!({
        "nodes": nodes,
        "tools": {
            "adb": which::which("adb").is_ok(),
            "ssh": which::which("ssh").is_ok(),
            "vnc": vnc_tool,
            "rdp": rdp_tool,
        },
        "formats": {
            "ssh": "ssh:user@host:port or user@host",
            "adb": "adb:device_id or device_serial",
            "vnc": "vnc:host:display (display 0-99) or vnc:host:port (port > 99)",
            "rdp": "rdp:user@host:port or rdp:host"
        }
    }).to_string())
}

/// Get details about a specific node.
fn node_describe(node: &str) -> Result<String, String> {
    match parse_node(node) {
        NodeType::Ssh { user, host, port } => {
            // Try to get system info via SSH
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=5",
                    "-o", "BatchMode=yes",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    "uname -a && hostname && uptime"
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    let info = String::from_utf8_lossy(&out.stdout);
                    Ok(json!({
                        "node": node,
                        "type": "ssh",
                        "user": user,
                        "host": host,
                        "port": port,
                        "status": "reachable",
                        "info": info.trim()
                    }).to_string())
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr);
                    Ok(json!({
                        "node": node,
                        "type": "ssh",
                        "user": user,
                        "host": host,
                        "port": port,
                        "status": "unreachable",
                        "error": err.trim()
                    }).to_string())
                }
                Err(e) => Err(format!("Failed to run ssh: {}", e))
            }
        }
        NodeType::Adb { device } => {
            // Get device properties
            let output = Command::new("adb")
                .args(["-s", &device, "shell", "getprop ro.product.model && getprop ro.build.version.release && getprop ro.serialno"])
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    let info = String::from_utf8_lossy(&out.stdout);
                    let lines: Vec<&str> = info.lines().collect();
                    Ok(json!({
                        "node": node,
                        "type": "adb",
                        "device": device,
                        "status": "connected",
                        "model": lines.get(0).unwrap_or(&""),
                        "android_version": lines.get(1).unwrap_or(&""),
                        "serial": lines.get(2).unwrap_or(&"")
                    }).to_string())
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr);
                    Ok(json!({
                        "node": node,
                        "type": "adb",
                        "device": device,
                        "status": "error",
                        "error": err.trim()
                    }).to_string())
                }
                Err(e) => Err(format!("Failed to run adb: {}", e))
            }
        }
        NodeType::Vnc { host, port, .. } => {
            // Try to connect briefly to check if VNC is responding
            // Using nc (netcat) for a quick port check
            let output = Command::new("nc")
                .args(["-zv", "-w", "3", &host, &port.to_string()])
                .output();

            let reachable = output.map(|o| o.status.success()).unwrap_or(false);

            Ok(json!({
                "node": node,
                "type": "vnc",
                "host": host,
                "port": port,
                "display": if port >= 5900 { port - 5900 } else { 0 },
                "status": if reachable { "reachable" } else { "unknown" },
                "tools": {
                    "vncdo": which::which("vncdo").is_ok(),
                    "vncdotool": which::which("vncdotool").is_ok(),
                },
                "note": "Use vncdo/vncdotool for automation, or connect via VNC viewer"
            }).to_string())
        }
        NodeType::Rdp { user, host, port } => {
            // Check if port is open
            let output = Command::new("nc")
                .args(["-zv", "-w", "3", &host, &port.to_string()])
                .output();

            let reachable = output.map(|o| o.status.success()).unwrap_or(false);

            Ok(json!({
                "node": node,
                "type": "rdp",
                "user": user,
                "host": host,
                "port": port,
                "status": if reachable { "reachable" } else { "unknown" },
                "tools": {
                    "xfreerdp": which::which("xfreerdp").is_ok() || which::which("xfreerdp3").is_ok(),
                    "rdesktop": which::which("rdesktop").is_ok(),
                },
                "note": "Use xfreerdp for screenshots and automation"
            }).to_string())
        }
    }
}

/// Run a command on a remote node.
fn node_run(node: &str, command: &[impl AsRef<str>]) -> Result<String, String> {
    match parse_node(node) {
        NodeType::Ssh { user, host, port } => {
            let cmd_str = command.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(" ");
            
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=10",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    &cmd_str
                ])
                .output()
                .map_err(|e| format!("Failed to run ssh: {}", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            Ok(json!({
                "node": node,
                "command": cmd_str,
                "exit_code": output.status.code(),
                "stdout": stdout.trim(),
                "stderr": stderr.trim()
            }).to_string())
        }
        NodeType::Adb { device } => {
            let cmd_str = command.iter().map(|s| s.as_ref()).collect::<Vec<_>>().join(" ");
            
            let output = Command::new("adb")
                .args(["-s", &device, "shell", &cmd_str])
                .output()
                .map_err(|e| format!("Failed to run adb: {}", e))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            Ok(json!({
                "node": node,
                "command": cmd_str,
                "exit_code": output.status.code(),
                "stdout": stdout.trim(),
                "stderr": stderr.trim()
            }).to_string())
        }
        NodeType::Vnc { .. } => {
            Err("VNC nodes don't support arbitrary command execution. Use click, type, or key actions.".to_string())
        }
        NodeType::Rdp { .. } => {
            Err("RDP nodes don't support arbitrary command execution. Use click, type, or key actions.".to_string())
        }
    }
}

// ── Screen/Input actions (multi-transport) ──────────────────────────────────

/// Take a screenshot on any supported node type.
fn node_screen_snap(node: &str, _facing: &str) -> Result<String, String> {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    
    match parse_node(node) {
        NodeType::Adb { device } => {
            let remote_path = format!("/sdcard/rustyclaw_snap_{}.png", timestamp);
            let local_path = format!("/tmp/adb_snap_{}.png", timestamp);
            
            // Take screenshot
            let output = Command::new("adb")
                .args(["-s", &device, "shell", "screencap", "-p", &remote_path])
                .output()
                .map_err(|e| format!("Failed to run adb: {}", e))?;

            if !output.status.success() {
                let err = String::from_utf8_lossy(&output.stderr);
                return Err(format!("Screenshot failed: {}", err));
            }

            // Pull file
            let pull = Command::new("adb")
                .args(["-s", &device, "pull", &remote_path, &local_path])
                .output()
                .map_err(|e| format!("Failed to pull file: {}", e))?;

            if !pull.status.success() {
                return Err("Failed to pull screenshot from device".to_string());
            }

            // Clean up remote file
            let _ = Command::new("adb")
                .args(["-s", &device, "shell", "rm", &remote_path])
                .output();

            Ok(json!({
                "node": node,
                "type": "adb",
                "action": "screen_snap",
                "path": local_path
            }).to_string())
        }
        
        NodeType::Ssh { user, host, port } => {
            // Take screenshot via SSH + scrot/import
            let local_path = format!("/tmp/ssh_snap_{}.png", timestamp);
            
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=10",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    "DISPLAY=:0 scrot -o /tmp/screenshot.png && cat /tmp/screenshot.png"
                ])
                .output()
                .map_err(|e| format!("Failed to run ssh: {}", e))?;

            if output.status.success() && !output.stdout.is_empty() {
                std::fs::write(&local_path, &output.stdout)
                    .map_err(|e| format!("Failed to write screenshot: {}", e))?;
                Ok(json!({
                    "node": node,
                    "type": "ssh",
                    "action": "screen_snap",
                    "path": local_path
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("Screenshot failed (need scrot on remote): {}", err))
            }
        }
        
        NodeType::Vnc { host, port, password } => {
            let local_path = format!("/tmp/vnc_snap_{}.png", timestamp);
            
            // Try vncdo first, then vncdotool
            let mut cmd = if which::which("vncdo").is_ok() {
                let mut c = Command::new("vncdo");
                c.args(["-s", &format!("{}:{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["--password", pw]);
                }
                c.args(["capture", &local_path]);
                c
            } else if which::which("vncdotool").is_ok() {
                let mut c = Command::new("vncdotool");
                c.args(["-s", &format!("{}::{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["-p", pw]);
                }
                c.args(["capture", &local_path]);
                c
            } else {
                return Err("No VNC tool available. Install vncdo or vncdotool.".to_string());
            };

            let output = cmd.output()
                .map_err(|e| format!("Failed to run VNC tool: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "type": "vnc",
                    "action": "screen_snap",
                    "path": local_path
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("VNC screenshot failed: {}", err))
            }
        }
        
        NodeType::Rdp { user, host, port } => {
            let _local_path = format!("/tmp/rdp_snap_{}.png", timestamp);
            
            // xfreerdp can take screenshots via /exec or by capturing initial frame
            // We'll use a quick connect + screenshot approach
            let xfreerdp = if which::which("xfreerdp3").is_ok() {
                "xfreerdp3"
            } else if which::which("xfreerdp").is_ok() {
                "xfreerdp"
            } else {
                return Err("No RDP tool available. Install xfreerdp (freerdp2-x11).".to_string());
            };

            let mut args = vec![
                format!("/v:{}", host),
                format!("/port:{}", port),
                "/cert:ignore".to_string(),
                // Connect briefly and capture
                "+clipboard".to_string(),
            ];

            if let Some(ref u) = user {
                args.push(format!("/u:{}", u));
            }

            // Note: xfreerdp doesn't have a simple screenshot mode
            // For now, return guidance
            Ok(json!({
                "node": node,
                "type": "rdp",
                "action": "screen_snap",
                "status": "not_implemented",
                "note": "RDP screenshot requires interactive session. Use 'xfreerdp /v:host' to connect, or use VNC for headless screenshots.",
                "command": format!("{} {}", xfreerdp, args.join(" "))
            }).to_string())
        }
    }
}

/// Click at coordinates on VNC/RDP node.
fn node_click(node: &str, x: i32, y: i32, button: &str) -> Result<String, String> {
    match parse_node(node) {
        NodeType::Vnc { host, port, password } => {
            let btn = match button {
                "left" | "1" => "1",
                "middle" | "2" => "2",
                "right" | "3" => "3",
                _ => "1",
            };

            let mut cmd = if which::which("vncdo").is_ok() {
                let mut c = Command::new("vncdo");
                c.args(["-s", &format!("{}:{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["--password", pw]);
                }
                c.args(["move", &x.to_string(), &y.to_string(), "click", btn]);
                c
            } else if which::which("vncdotool").is_ok() {
                let mut c = Command::new("vncdotool");
                c.args(["-s", &format!("{}::{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["-p", pw]);
                }
                c.args(["move", &x.to_string(), &y.to_string(), "click", btn]);
                c
            } else {
                return Err("No VNC tool available. Install vncdo or vncdotool.".to_string());
            };

            let output = cmd.output()
                .map_err(|e| format!("Failed to run VNC tool: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "click",
                    "x": x,
                    "y": y,
                    "button": button
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("VNC click failed: {}", err))
            }
        }
        
        NodeType::Adb { device } => {
            // ADB input tap
            let output = Command::new("adb")
                .args(["-s", &device, "shell", "input", "tap", &x.to_string(), &y.to_string()])
                .output()
                .map_err(|e| format!("Failed to run adb: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "click",
                    "x": x,
                    "y": y
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("ADB tap failed: {}", err))
            }
        }
        
        NodeType::Ssh { user, host, port } => {
            // Use xdotool over SSH
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=5",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    &format!("DISPLAY=:0 xdotool mousemove {} {} click 1", x, y)
                ])
                .output()
                .map_err(|e| format!("Failed to run ssh: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "click",
                    "x": x,
                    "y": y,
                    "via": "xdotool"
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("Click failed (need xdotool on remote): {}", err))
            }
        }
        
        NodeType::Rdp { .. } => {
            Err("RDP click requires interactive session. Consider using VNC or SSH+xdotool.".to_string())
        }
    }
}

/// Type text on node.
fn node_type_text(node: &str, text: &str) -> Result<String, String> {
    match parse_node(node) {
        NodeType::Vnc { host, port, password } => {
            let mut cmd = if which::which("vncdo").is_ok() {
                let mut c = Command::new("vncdo");
                c.args(["-s", &format!("{}:{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["--password", pw]);
                }
                c.args(["type", text]);
                c
            } else if which::which("vncdotool").is_ok() {
                let mut c = Command::new("vncdotool");
                c.args(["-s", &format!("{}::{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["-p", pw]);
                }
                c.args(["type", text]);
                c
            } else {
                return Err("No VNC tool available.".to_string());
            };

            let output = cmd.output()
                .map_err(|e| format!("Failed to run VNC tool: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "type",
                    "length": text.len()
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("VNC type failed: {}", err))
            }
        }
        
        NodeType::Adb { device } => {
            // Escape text for adb input
            let escaped = text.replace(' ', "%s").replace('\'', "\\'");
            let output = Command::new("adb")
                .args(["-s", &device, "shell", "input", "text", &escaped])
                .output()
                .map_err(|e| format!("Failed to run adb: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "type",
                    "length": text.len()
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("ADB type failed: {}", err))
            }
        }
        
        NodeType::Ssh { user, host, port } => {
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=5",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    &format!("DISPLAY=:0 xdotool type '{}'", text.replace('\'', "'\\''"))
                ])
                .output()
                .map_err(|e| format!("Failed to run ssh: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "type",
                    "length": text.len(),
                    "via": "xdotool"
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("Type failed (need xdotool on remote): {}", err))
            }
        }
        
        NodeType::Rdp { .. } => {
            Err("RDP typing requires interactive session.".to_string())
        }
    }
}

/// Send a key press on node.
fn node_send_key(node: &str, key: &str) -> Result<String, String> {
    match parse_node(node) {
        NodeType::Vnc { host, port, password } => {
            let mut cmd = if which::which("vncdo").is_ok() {
                let mut c = Command::new("vncdo");
                c.args(["-s", &format!("{}:{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["--password", pw]);
                }
                c.args(["key", key]);
                c
            } else if which::which("vncdotool").is_ok() {
                let mut c = Command::new("vncdotool");
                c.args(["-s", &format!("{}::{}", host, port)]);
                if let Some(ref pw) = password {
                    c.args(["-p", pw]);
                }
                c.args(["key", key]);
                c
            } else {
                return Err("No VNC tool available.".to_string());
            };

            let output = cmd.output()
                .map_err(|e| format!("Failed to run VNC tool: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "key",
                    "key": key
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("VNC key failed: {}", err))
            }
        }
        
        NodeType::Adb { device } => {
            // Map common key names to Android keycodes
            let key_upper = key.to_uppercase();
            let keycode = match key_upper.as_str() {
                "ENTER" | "RETURN" => "66",
                "BACK" | "ESCAPE" => "4",
                "HOME" => "3",
                "TAB" => "61",
                "SPACE" => "62",
                "DELETE" | "BACKSPACE" => "67",
                "UP" => "19",
                "DOWN" => "20",
                "LEFT" => "21",
                "RIGHT" => "22",
                k if k.parse::<u32>().is_ok() => k, // Raw keycode
                _ => return Err(format!("Unknown key: {}. Use ENTER, BACK, HOME, TAB, SPACE, DELETE, UP/DOWN/LEFT/RIGHT, or keycode number.", key)),
            };

            let output = Command::new("adb")
                .args(["-s", &device, "shell", "input", "keyevent", keycode])
                .output()
                .map_err(|e| format!("Failed to run adb: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "key",
                    "key": key,
                    "keycode": keycode
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("ADB keyevent failed: {}", err))
            }
        }
        
        NodeType::Ssh { user, host, port } => {
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=5",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    &format!("DISPLAY=:0 xdotool key {}", key)
                ])
                .output()
                .map_err(|e| format!("Failed to run ssh: {}", e))?;

            if output.status.success() {
                Ok(json!({
                    "node": node,
                    "action": "key",
                    "key": key,
                    "via": "xdotool"
                }).to_string())
            } else {
                let err = String::from_utf8_lossy(&output.stderr);
                Err(format!("Key failed (need xdotool on remote): {}", err))
            }
        }
        
        NodeType::Rdp { .. } => {
            Err("RDP key press requires interactive session.".to_string())
        }
    }
}

/// Send notification to node.
fn node_notify(node: &str, title: &str, body: &str) -> Result<String, String> {
    match parse_node(node) {
        NodeType::Adb { device } => {
            // Use cmd notification (Android 10+)
            let output = Command::new("adb")
                .args([
                    "-s", &device,
                    "shell",
                    &format!("cmd notification post -t '{}' 'RustyClaw' '{}'", title, body)
                ])
                .output()
                .map_err(|e| format!("Failed to run adb: {}", e))?;

            let status = if output.status.success() { "sent" } else { "attempted" };

            Ok(json!({
                "node": node,
                "action": "notify",
                "title": title,
                "body": body,
                "status": status
            }).to_string())
        }
        
        NodeType::Ssh { user, host, port } => {
            let output = Command::new("ssh")
                .args([
                    "-o", "ConnectTimeout=5",
                    "-p", &port.to_string(),
                    &format!("{}@{}", user, host),
                    &format!("notify-send '{}' '{}'", title, body)
                ])
                .output();

            match output {
                Ok(out) if out.status.success() => Ok(json!({
                    "node": node,
                    "action": "notify",
                    "title": title,
                    "body": body,
                    "status": "sent"
                }).to_string()),
                _ => Ok(json!({
                    "node": node,
                    "action": "notify",
                    "status": "failed",
                    "note": "notify-send may not be available on target"
                }).to_string())
            }
        }
        
        NodeType::Vnc { .. } | NodeType::Rdp { .. } => {
            Err("Notifications require ADB or SSH (with notify-send on remote).".to_string())
        }
    }
}

// ── ADB-specific actions ────────────────────────────────────────────────────

/// List cameras on Android device.
fn adb_camera_list(node: &str) -> Result<String, String> {
    let device = match parse_node(node) {
        NodeType::Adb { device } => device,
        _ => return Err("camera_list only works with ADB nodes".to_string()),
    };

    // Query camera info via dumpsys
    let output = Command::new("adb")
        .args(["-s", &device, "shell", "dumpsys media.camera | grep -E 'Camera|Facing'"])
        .output()
        .map_err(|e| format!("Failed to run adb: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    Ok(json!({
        "node": node,
        "cameras": stdout.trim(),
        "note": "Use camera app + screen_record for actual camera capture"
    }).to_string())
}

/// Record screen on Android device.
fn adb_screen_record(node: &str, duration_ms: u64) -> Result<String, String> {
    let device = match parse_node(node) {
        NodeType::Adb { device } => device,
        _ => return Err("screen_record only works with ADB nodes".to_string()),
    };

    let duration_secs = (duration_ms / 1000).max(1).min(180); // 1-180 seconds
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let remote_path = format!("/sdcard/rustyclaw_rec_{}.mp4", timestamp);
    let local_path = format!("/tmp/adb_rec_{}.mp4", timestamp);

    // Start recording (this blocks for duration)
    let output = Command::new("adb")
        .args([
            "-s", &device,
            "shell",
            "screenrecord",
            "--time-limit", &duration_secs.to_string(),
            &remote_path
        ])
        .output()
        .map_err(|e| format!("Failed to run adb: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Screen record failed: {}", err));
    }

    // Pull file
    let pull = Command::new("adb")
        .args(["-s", &device, "pull", &remote_path, &local_path])
        .output()
        .map_err(|e| format!("Failed to pull file: {}", e))?;

    if !pull.status.success() {
        return Err("Failed to pull recording from device".to_string());
    }

    // Clean up
    let _ = Command::new("adb")
        .args(["-s", &device, "shell", "rm", &remote_path])
        .output();

    Ok(json!({
        "node": node,
        "action": "screen_record",
        "duration_secs": duration_secs,
        "path": local_path
    }).to_string())
}

/// Get location from Android device.
fn adb_location_get(node: &str) -> Result<String, String> {
    let device = match parse_node(node) {
        NodeType::Adb { device } => device,
        _ => return Err("location_get only works with ADB nodes".to_string()),
    };

    // Try to get last known location from location providers
    let output = Command::new("adb")
        .args([
            "-s", &device,
            "shell",
            "dumpsys location | grep -A2 'last location'"
        ])
        .output()
        .map_err(|e| format!("Failed to run adb: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Also try settings secure for mock location
    let mock_output = Command::new("adb")
        .args(["-s", &device, "shell", "settings get secure mock_location"])
        .output()
        .ok();

    let mock_enabled = mock_output
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
        .unwrap_or(false);

    Ok(json!({
        "node": node,
        "location_info": stdout.trim(),
        "mock_location": mock_enabled,
        "note": "For real-time location, use a location app or enable developer options"
    }).to_string())
}

// ── Canvas (stub) ───────────────────────────────────────────────────────────

/// Canvas control for UI presentation.
/// This remains a stub as it requires OpenClaw canvas service infrastructure.
pub fn exec_canvas(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let node = args.get("node").and_then(|v| v.as_str());

    match action {
        "present" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for present action")?;
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800);
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600);

            Ok(format!(
                "Would present canvas:\n- URL: {}\n- Size: {}x{}\n- Node: {}\n\nNote: Requires canvas integration.",
                url,
                width,
                height,
                node.unwrap_or("default")
            ))
        }

        "hide" => Ok(format!(
            "Would hide canvas on node: {}\n\nNote: Requires canvas integration.",
            node.unwrap_or("default")
        )),

        "navigate" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for navigate action")?;
            Ok(format!(
                "Would navigate canvas to: {}\n\nNote: Requires canvas integration.",
                url
            ))
        }

        "eval" => {
            let js = args
                .get("javaScript")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'javaScript' for eval action")?;
            Ok(format!(
                "Would evaluate JavaScript ({} chars):\n{}\n\nNote: Requires canvas integration.",
                js.len(),
                if js.len() > 100 { &js[..100] } else { js }
            ))
        }

        "snapshot" => Ok(format!(
            "Would capture canvas snapshot on node: {}\n\nNote: Requires canvas integration.",
            node.unwrap_or("default")
        )),

        "a2ui_push" => Ok(
            "Would push A2UI (accessibility-to-UI) update.\n\nNote: Requires canvas integration."
                .to_string(),
        ),

        "a2ui_reset" => {
            Ok("Would reset A2UI state.\n\nNote: Requires canvas integration.".to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: present, hide, navigate, eval, snapshot, a2ui_push, a2ui_reset",
            action
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_ssh_node() {
        match parse_node("user@example.com") {
            NodeType::Ssh { user, host, port } => {
                assert_eq!(user, "user");
                assert_eq!(host, "example.com");
                assert_eq!(port, 22);
            }
            _ => panic!("Expected SSH node"),
        }
    }

    #[test]
    fn test_parse_ssh_with_port() {
        match parse_node("ssh:admin@192.168.1.1:2222") {
            NodeType::Ssh { user, host, port } => {
                assert_eq!(user, "admin");
                assert_eq!(host, "192.168.1.1");
                assert_eq!(port, 2222);
            }
            _ => panic!("Expected SSH node"),
        }
    }

    #[test]
    fn test_parse_adb_node() {
        match parse_node("adb:emulator-5554") {
            NodeType::Adb { device } => {
                assert_eq!(device, "emulator-5554");
            }
            _ => panic!("Expected ADB node"),
        }
    }

    #[test]
    fn test_nodes_status() {
        let args = json!({ "action": "status" });
        let result = exec_nodes(&args, &PathBuf::from("/tmp"));
        assert!(result.is_ok());
    }
}
