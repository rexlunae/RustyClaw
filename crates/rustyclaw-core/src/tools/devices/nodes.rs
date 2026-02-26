//! Nodes tool: discover and control remote devices via SSH, ADB, VNC, RDP.

use super::{get_command_array, get_node, has_command, has_command_async, sh, sh_async};
use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

/// Node type with parsed connection details.
#[allow(dead_code)]
enum ParsedNode {
    Ssh {
        user: String,
        host: String,
        port: u16,
    },
    Adb {
        device: String,
    },
    Vnc {
        host: String,
        port: u16,
        #[allow(dead_code)]
        password: Option<String>,
    },
    Rdp {
        user: Option<String>,
        host: String,
        port: u16,
    },
}

fn parse_node(node: &str) -> ParsedNode {
    if node.starts_with("adb:") {
        return ParsedNode::Adb {
            device: node[4..].to_string(),
        };
    }
    if let Some(rest) = node.strip_prefix("ssh:") {
        return parse_ssh_target(rest);
    }
    if node.starts_with("vnc:") {
        return parse_vnc_target(&node[4..]);
    }
    if node.starts_with("rdp:") {
        return parse_rdp_target(&node[4..]);
    }
    if node.contains('@') {
        return parse_ssh_target(node);
    }
    ParsedNode::Adb {
        device: node.to_string(),
    }
}

fn parse_ssh_target(target: &str) -> ParsedNode {
    let (user_host, port) = if let Some(idx) = target.rfind(':') {
        if let Ok(p) = target[idx + 1..].parse::<u16>() {
            (&target[..idx], p)
        } else {
            (target, 22)
        }
    } else {
        (target, 22)
    };

    let (user, host) = if let Some(idx) = user_host.find('@') {
        (
            user_host[..idx].to_string(),
            user_host[idx + 1..].to_string(),
        )
    } else {
        ("root".to_string(), user_host.to_string())
    };

    ParsedNode::Ssh { user, host, port }
}

fn parse_vnc_target(target: &str) -> ParsedNode {
    let (host, port) = if let Some(idx) = target.rfind(':') {
        if let Ok(p) = target[idx + 1..].parse::<u16>() {
            let actual_port = if p > 99 { p } else { 5900 + p };
            (target[..idx].to_string(), actual_port)
        } else {
            (target.to_string(), 5900)
        }
    } else {
        (target.to_string(), 5900)
    };
    ParsedNode::Vnc {
        host,
        port,
        password: None,
    }
}

fn parse_rdp_target(target: &str) -> ParsedNode {
    let (user_host, port) = if let Some(idx) = target.rfind(':') {
        if let Ok(p) = target[idx + 1..].parse::<u16>() {
            (&target[..idx], p)
        } else {
            (target, 3389)
        }
    } else {
        (target, 3389)
    };

    let (user, host) = if let Some(idx) = user_host.find('@') {
        (
            Some(user_host[..idx].to_string()),
            user_host[idx + 1..].to_string(),
        )
    } else {
        (None, user_host.to_string())
    };

    ParsedNode::Rdp { user, host, port }
}

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_nodes_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing nodes tool (async)");

    match action {
        "status" => node_status_async().await,
        "describe" => {
            let node = get_node(args)?;
            node_describe_async(&node).await
        }
        "run" => {
            let node = get_node(args)?;
            let command = get_command_array(args)?;
            node_run_async(&node, &command).await
        }
        "camera_snap" | "screen_snap" => {
            let node = get_node(args)?;
            let facing = args
                .get("facing")
                .and_then(|v| v.as_str())
                .unwrap_or("back");
            node_screen_snap_async(&node, facing).await
        }
        "camera_list" => {
            let node = get_node(args)?;
            adb_camera_list_async(&node).await
        }
        "screen_record" => {
            let node = get_node(args)?;
            let duration = args
                .get("durationMs")
                .and_then(|v| v.as_u64())
                .unwrap_or(5000);
            adb_screen_record_async(&node, duration).await
        }
        "location_get" => {
            let node = get_node(args)?;
            adb_location_get_async(&node).await
        }
        "notify" => {
            let node = get_node(args)?;
            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("RustyClaw");
            let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
            node_notify_async(&node, title, body).await
        }
        "click" => {
            let node = get_node(args)?;
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            let button = args
                .get("button")
                .and_then(|v| v.as_str())
                .unwrap_or("left");
            node_click_async(&node, x as i32, y as i32, button).await
        }
        "type" => {
            let node = get_node(args)?;
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'text' for type action")?;
            node_type_text_async(&node, text).await
        }
        "key" => {
            let node = get_node(args)?;
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'key' for key action")?;
            node_send_key_async(&node, key).await
        }
        "pending" => Ok(json!({
            "pending": [],
            "note": "Direct connection nodes don't require pairing."
        })
        .to_string()),
        "approve" | "reject" => {
            Ok("Direct connection nodes don't require pairing approval.".to_string())
        }
        "invoke" => {
            let node = get_node(args)?;
            let cmd = args
                .get("invokeCommand")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'invokeCommand'")?;
            node_run_async(&node, &[cmd.to_string()]).await
        }
        _ => Err(format!(
            "Unknown action: {}. Valid: status, describe, run, screen_snap, camera_snap, camera_list, screen_record, location_get, notify, click, type, key, invoke",
            action
        )),
    }
}

async fn node_status_async() -> Result<String, String> {
    let mut nodes = Vec::new();

    // Check ADB devices
    let adb_out = sh_async("adb devices -l 2>/dev/null")
        .await
        .unwrap_or_default();
    for line in adb_out.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "device" {
            let device_id = parts[0];
            let model = parts
                .iter()
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

    // Check SSH config
    if let Ok(config) =
        tokio::fs::read_to_string(dirs::home_dir().unwrap_or_default().join(".ssh/config")).await
    {
        let mut current_host: Option<String> = None;
        let mut current_user: Option<String> = None;
        let mut current_hostname: Option<String> = None;

        for line in config.lines() {
            let line = line.trim();
            if line.starts_with("Host ") && !line.contains('*') {
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

    let vnc_tool = if has_command_async("vncdo").await {
        Some("vncdo")
    } else if has_command_async("vncdotool").await {
        Some("vncdotool")
    } else {
        None
    };

    let rdp_tool = if has_command_async("xfreerdp").await {
        Some("xfreerdp")
    } else if has_command_async("xfreerdp3").await {
        Some("xfreerdp3")
    } else if has_command_async("rdesktop").await {
        Some("rdesktop")
    } else {
        None
    };

    Ok(json!({
        "nodes": nodes,
        "tools": {
            "adb": has_command_async("adb").await,
            "ssh": has_command_async("ssh").await,
            "vnc": vnc_tool,
            "rdp": rdp_tool,
        },
        "formats": {
            "ssh": "ssh:user@host:port or user@host",
            "adb": "adb:device_id or device_serial",
            "vnc": "vnc:host:display or vnc:host:port",
            "rdp": "rdp:user@host:port or rdp:host"
        }
    })
    .to_string())
}

async fn node_describe_async(node: &str) -> Result<String, String> {
    match parse_node(node) {
        ParsedNode::Ssh { user, host, port } => {
            let cmd = format!(
                "ssh -o ConnectTimeout=5 -o BatchMode=yes -p {} {}@{} 'uname -a && hostname && uptime'",
                port, user, host
            );
            match sh_async(&cmd).await {
                Ok(info) => Ok(json!({
                    "node": node, "type": "ssh", "user": user, "host": host, "port": port,
                    "status": "reachable", "info": info.trim()
                })
                .to_string()),
                Err(e) => Ok(json!({
                    "node": node, "type": "ssh", "user": user, "host": host, "port": port,
                    "status": "unreachable", "error": e
                })
                .to_string()),
            }
        }
        ParsedNode::Adb { device } => {
            let cmd = format!(
                "adb -s {} shell 'getprop ro.product.model && getprop ro.build.version.release'",
                device
            );
            match sh_async(&cmd).await {
                Ok(info) => {
                    let lines: Vec<&str> = info.lines().collect();
                    Ok(json!({
                        "node": node, "type": "adb", "device": device, "status": "connected",
                        "model": lines.first().unwrap_or(&""), "android_version": lines.get(1).unwrap_or(&"")
                    }).to_string())
                }
                Err(e) => Ok(json!({
                    "node": node, "type": "adb", "device": device, "status": "error", "error": e
                })
                .to_string()),
            }
        }
        ParsedNode::Vnc { host, port, .. } => {
            let reachable = sh_async(&format!("nc -zv -w 3 {} {} 2>&1", host, port))
                .await
                .is_ok();
            Ok(json!({
                "node": node, "type": "vnc", "host": host, "port": port,
                "display": port.saturating_sub(5900), "status": if reachable { "reachable" } else { "unknown" }
            }).to_string())
        }
        ParsedNode::Rdp { user, host, port } => {
            let reachable = sh_async(&format!("nc -zv -w 3 {} {} 2>&1", host, port))
                .await
                .is_ok();
            Ok(json!({
                "node": node, "type": "rdp", "user": user, "host": host, "port": port,
                "status": if reachable { "reachable" } else { "unknown" }
            })
            .to_string())
        }
    }
}

async fn node_run_async(node: &str, command: &[String]) -> Result<String, String> {
    let cmd_str = command.join(" ");
    match parse_node(node) {
        ParsedNode::Ssh { user, host, port } => {
            let script = format!(
                "ssh -o ConnectTimeout=10 -p {} {}@{} '{}'",
                port,
                user,
                host,
                cmd_str.replace('\'', "'\\''")
            );
            let output = sh_async(&script).await;
            Ok(json!({
                "node": node, "command": cmd_str,
                "exit_code": if output.is_ok() { 0 } else { 1 },
                "stdout": output.as_ref().map(|s| s.as_str()).unwrap_or(""),
                "stderr": output.as_ref().err().map(|s| s.as_str()).unwrap_or("")
            })
            .to_string())
        }
        ParsedNode::Adb { device } => {
            let script = format!(
                "adb -s {} shell '{}'",
                device,
                cmd_str.replace('\'', "'\\''")
            );
            let output = sh_async(&script).await;
            Ok(json!({
                "node": node, "command": cmd_str,
                "exit_code": if output.is_ok() { 0 } else { 1 },
                "stdout": output.as_ref().map(|s| s.as_str()).unwrap_or(""),
                "stderr": output.as_ref().err().map(|s| s.as_str()).unwrap_or("")
            })
            .to_string())
        }
        ParsedNode::Vnc { .. } => Err("VNC nodes don't support command execution.".to_string()),
        ParsedNode::Rdp { .. } => Err("RDP nodes don't support command execution.".to_string()),
    }
}

async fn node_screen_snap_async(node: &str, _facing: &str) -> Result<String, String> {
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    match parse_node(node) {
        ParsedNode::Adb { device } => {
            let remote = format!("/sdcard/snap_{}.png", timestamp);
            let local = format!("/tmp/adb_snap_{}.png", timestamp);
            sh_async(&format!("adb -s {} shell screencap -p {}", device, remote)).await?;
            sh_async(&format!("adb -s {} pull {} {}", device, remote, local)).await?;
            let _ = sh_async(&format!("adb -s {} shell rm {}", device, remote)).await;
            Ok(
                json!({"node": node, "type": "adb", "action": "screen_snap", "path": local})
                    .to_string(),
            )
        }
        ParsedNode::Ssh { user, host, port } => {
            let local = format!("/tmp/ssh_snap_{}.png", timestamp);
            let out = sh_async(&format!("ssh -o ConnectTimeout=10 -p {} {}@{} 'DISPLAY=:0 scrot -o /tmp/screenshot.png && cat /tmp/screenshot.png'", port, user, host)).await?;
            tokio::fs::write(&local, out.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            Ok(
                json!({"node": node, "type": "ssh", "action": "screen_snap", "path": local})
                    .to_string(),
            )
        }
        ParsedNode::Vnc { host, port, .. } => {
            let local = format!("/tmp/vnc_snap_{}.png", timestamp);
            if has_command_async("vncdo").await {
                sh_async(&format!("vncdo -s {}:{} capture {}", host, port, local)).await?;
            } else if has_command_async("vncdotool").await {
                sh_async(&format!(
                    "vncdotool -s {}::{} capture {}",
                    host, port, local
                ))
                .await?;
            } else {
                return Err("No VNC tool available.".to_string());
            }
            Ok(
                json!({"node": node, "type": "vnc", "action": "screen_snap", "path": local})
                    .to_string(),
            )
        }
        ParsedNode::Rdp { .. } => Ok(json!({
            "node": node, "type": "rdp", "action": "screen_snap", "status": "not_implemented",
            "note": "RDP screenshot requires interactive session."
        })
        .to_string()),
    }
}

async fn node_click_async(node: &str, x: i32, y: i32, button: &str) -> Result<String, String> {
    match parse_node(node) {
        ParsedNode::Adb { device } => {
            sh_async(&format!("adb -s {} shell input tap {} {}", device, x, y)).await?;
            Ok(json!({"node": node, "action": "click", "x": x, "y": y}).to_string())
        }
        ParsedNode::Ssh { user, host, port } => {
            sh_async(&format!(
                "ssh -o ConnectTimeout=5 -p {} {}@{} 'DISPLAY=:0 xdotool mousemove {} {} click 1'",
                port, user, host, x, y
            ))
            .await?;
            Ok(
                json!({"node": node, "action": "click", "x": x, "y": y, "via": "xdotool"})
                    .to_string(),
            )
        }
        ParsedNode::Vnc { host, port, .. } => {
            let btn = match button {
                "right" | "3" => "3",
                "middle" | "2" => "2",
                _ => "1",
            };
            if has_command_async("vncdo").await {
                sh_async(&format!(
                    "vncdo -s {}:{} move {} {} click {}",
                    host, port, x, y, btn
                ))
                .await?;
            } else if has_command_async("vncdotool").await {
                sh_async(&format!(
                    "vncdotool -s {}::{} move {} {} click {}",
                    host, port, x, y, btn
                ))
                .await?;
            } else {
                return Err("No VNC tool available.".to_string());
            }
            Ok(
                json!({"node": node, "action": "click", "x": x, "y": y, "button": button})
                    .to_string(),
            )
        }
        ParsedNode::Rdp { .. } => Err("RDP click requires interactive session.".to_string()),
    }
}

async fn node_type_text_async(node: &str, text: &str) -> Result<String, String> {
    match parse_node(node) {
        ParsedNode::Adb { device } => {
            let escaped = text.replace(' ', "%s").replace('\'', "\\'");
            sh_async(&format!("adb -s {} shell input text '{}'", device, escaped)).await?;
            Ok(json!({"node": node, "action": "type", "length": text.len()}).to_string())
        }
        ParsedNode::Ssh { user, host, port } => {
            let escaped = text.replace('\'', "'\\''");
            sh_async(&format!(
                "ssh -o ConnectTimeout=5 -p {} {}@{} \"DISPLAY=:0 xdotool type '{}'\"",
                port, user, host, escaped
            ))
            .await?;
            Ok(
                json!({"node": node, "action": "type", "length": text.len(), "via": "xdotool"})
                    .to_string(),
            )
        }
        ParsedNode::Vnc { host, port, .. } => {
            if has_command_async("vncdo").await {
                sh_async(&format!(
                    "vncdo -s {}:{} type '{}'",
                    host,
                    port,
                    text.replace('\'', "'\\''")
                ))
                .await?;
            } else if has_command_async("vncdotool").await {
                sh_async(&format!(
                    "vncdotool -s {}::{} type '{}'",
                    host,
                    port,
                    text.replace('\'', "'\\''")
                ))
                .await?;
            } else {
                return Err("No VNC tool available.".to_string());
            }
            Ok(json!({"node": node, "action": "type", "length": text.len()}).to_string())
        }
        ParsedNode::Rdp { .. } => Err("RDP typing requires interactive session.".to_string()),
    }
}

async fn node_send_key_async(node: &str, key: &str) -> Result<String, String> {
    match parse_node(node) {
        ParsedNode::Adb { device } => {
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
                k if k.parse::<u32>().is_ok() => k,
                _ => return Err(format!("Unknown key: {}", key)),
            };
            sh_async(&format!(
                "adb -s {} shell input keyevent {}",
                device, keycode
            ))
            .await?;
            Ok(json!({"node": node, "action": "key", "key": key, "keycode": keycode}).to_string())
        }
        ParsedNode::Ssh { user, host, port } => {
            sh_async(&format!(
                "ssh -o ConnectTimeout=5 -p {} {}@{} 'DISPLAY=:0 xdotool key {}'",
                port, user, host, key
            ))
            .await?;
            Ok(json!({"node": node, "action": "key", "key": key, "via": "xdotool"}).to_string())
        }
        ParsedNode::Vnc { host, port, .. } => {
            if has_command_async("vncdo").await {
                sh_async(&format!("vncdo -s {}:{} key {}", host, port, key)).await?;
            } else if has_command_async("vncdotool").await {
                sh_async(&format!("vncdotool -s {}::{} key {}", host, port, key)).await?;
            } else {
                return Err("No VNC tool available.".to_string());
            }
            Ok(json!({"node": node, "action": "key", "key": key}).to_string())
        }
        ParsedNode::Rdp { .. } => Err("RDP key press requires interactive session.".to_string()),
    }
}

async fn node_notify_async(node: &str, title: &str, body: &str) -> Result<String, String> {
    match parse_node(node) {
        ParsedNode::Adb { device } => {
            let _ = sh_async(&format!(
                "adb -s {} shell \"cmd notification post -t '{}' 'RustyClaw' '{}'\"",
                device, title, body
            ))
            .await;
            Ok(json!({"node": node, "action": "notify", "title": title, "body": body, "status": "sent"}).to_string())
        }
        ParsedNode::Ssh { user, host, port } => {
            let result = sh_async(&format!(
                "ssh -o ConnectTimeout=5 -p {} {}@{} \"notify-send '{}' '{}'\"",
                port, user, host, title, body
            ))
            .await;
            Ok(json!({
                "node": node, "action": "notify", "title": title, "body": body,
                "status": if result.is_ok() { "sent" } else { "failed" }
            })
            .to_string())
        }
        _ => Err("Notifications require ADB or SSH.".to_string()),
    }
}

async fn adb_camera_list_async(node: &str) -> Result<String, String> {
    let device = match parse_node(node) {
        ParsedNode::Adb { device } => device,
        _ => return Err("camera_list only works with ADB nodes".to_string()),
    };
    let out = sh_async(&format!(
        "adb -s {} shell \"dumpsys media.camera | grep -E 'Camera|Facing'\"",
        device
    ))
    .await?;
    Ok(json!({"node": node, "cameras": out.trim(), "note": "Use camera app + screen_record for capture"}).to_string())
}

async fn adb_screen_record_async(node: &str, duration_ms: u64) -> Result<String, String> {
    let device = match parse_node(node) {
        ParsedNode::Adb { device } => device,
        _ => return Err("screen_record only works with ADB nodes".to_string()),
    };
    let secs = (duration_ms / 1000).clamp(1, 180);
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let remote = format!("/sdcard/rec_{}.mp4", timestamp);
    let local = format!("/tmp/adb_rec_{}.mp4", timestamp);

    sh_async(&format!(
        "adb -s {} shell screenrecord --time-limit {} {}",
        device, secs, remote
    ))
    .await?;
    sh_async(&format!("adb -s {} pull {} {}", device, remote, local)).await?;
    let _ = sh_async(&format!("adb -s {} shell rm {}", device, remote)).await;

    Ok(
        json!({"node": node, "action": "screen_record", "duration_secs": secs, "path": local})
            .to_string(),
    )
}

async fn adb_location_get_async(node: &str) -> Result<String, String> {
    let device = match parse_node(node) {
        ParsedNode::Adb { device } => device,
        _ => return Err("location_get only works with ADB nodes".to_string()),
    };
    let out = sh_async(&format!(
        "adb -s {} shell \"dumpsys location | grep -A2 'last location'\"",
        device
    ))
    .await
    .unwrap_or_default();
    let mock = sh_async(&format!(
        "adb -s {} shell settings get secure mock_location",
        device
    ))
    .await
    .unwrap_or_default();
    Ok(json!({
        "node": node, "location_info": out.trim(),
        "mock_location": mock.trim() == "1"
    })
    .to_string())
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_nodes(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing nodes tool");

    match action {
        "status" => node_status_sync(),
        "describe" => {
            let node = get_node(args)?;
            node_describe_sync(&node)
        }
        "run" => {
            let node = get_node(args)?;
            let command = get_command_array(args)?;
            node_run_sync(&node, &command)
        }
        "pending" => Ok(
            json!({"pending": [], "note": "Direct connection nodes don't require pairing."})
                .to_string(),
        ),
        "approve" | "reject" => {
            Ok("Direct connection nodes don't require pairing approval.".to_string())
        }
        _ => Err(format!(
            "Sync nodes tool only supports: status, describe, run, pending. Use async for full support."
        )),
    }
}

fn node_status_sync() -> Result<String, String> {
    let adb_out = sh("adb devices -l 2>/dev/null").unwrap_or_default();
    let mut nodes = Vec::new();
    for line in adb_out.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "device" {
            nodes.push(
                json!({"id": format!("adb:{}", parts[0]), "type": "adb", "status": "connected"}),
            );
        }
    }
    Ok(json!({
        "nodes": nodes,
        "tools": {
            "adb": has_command("adb"),
            "ssh": has_command("ssh"),
        },
        "note": "Sync mode - limited info"
    })
    .to_string())
}

fn node_describe_sync(node: &str) -> Result<String, String> {
    match parse_node(node) {
        ParsedNode::Ssh { user, host, port } => {
            let out = sh(&format!(
                "ssh -o ConnectTimeout=5 -o BatchMode=yes -p {} {}@{} 'uname -a'",
                port, user, host
            ));
            Ok(json!({
                "node": node, "type": "ssh",
                "status": if out.is_ok() { "reachable" } else { "unreachable" },
                "info": out.unwrap_or_default()
            })
            .to_string())
        }
        ParsedNode::Adb { device } => {
            let out = sh(&format!("adb -s {} shell getprop ro.product.model", device));
            Ok(json!({"node": node, "type": "adb", "model": out.unwrap_or_default()}).to_string())
        }
        _ => Ok(json!({"node": node, "status": "sync mode limited"}).to_string()),
    }
}

fn node_run_sync(node: &str, command: &[String]) -> Result<String, String> {
    let cmd_str = command.join(" ");
    match parse_node(node) {
        ParsedNode::Ssh { user, host, port } => {
            let out = sh(&format!(
                "ssh -o ConnectTimeout=10 -p {} {}@{} '{}'",
                port,
                user,
                host,
                cmd_str.replace('\'', "'\\''")
            ));
            Ok(
                json!({"node": node, "command": cmd_str, "output": out.unwrap_or_default()})
                    .to_string(),
            )
        }
        ParsedNode::Adb { device } => {
            let out = sh(&format!(
                "adb -s {} shell '{}'",
                device,
                cmd_str.replace('\'', "'\\''")
            ));
            Ok(
                json!({"node": node, "command": cmd_str, "output": out.unwrap_or_default()})
                    .to_string(),
            )
        }
        _ => Err("Sync run only supports SSH and ADB.".to_string()),
    }
}
