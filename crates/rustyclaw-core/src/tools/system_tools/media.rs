//! Media tools: screenshot capture and clipboard access.

use super::{sh, sh_async, resolve_path, expand_tilde};
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    for unit in UNITS {
        if val < 1024.0 { return format!("{:.1} {}", val, unit); }
        val /= 1024.0;
    }
    format!("{:.1} PB", val)
}

// ── Async implementations ───────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub async fn exec_screenshot_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let output_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("screenshot.png");
    let region = args.get("region").and_then(|v| v.as_str());
    let delay = args.get("delay").and_then(|v| v.as_u64()).unwrap_or(0);

    debug!(output_path, ?region, delay, "Screenshot");

    let target = if output_path.starts_with('/') || output_path.starts_with('~') {
        expand_tilde(output_path)
    } else {
        resolve_path(workspace_dir, output_path)
    };

    if let Some(parent) = target.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let mut cmd_parts = vec!["screencapture", "-x"];
    let delay_str;
    let region_str;
    if delay > 0 {
        delay_str = format!("-T{}", delay);
        cmd_parts.push(&delay_str);
    }
    if let Some(r) = region {
        region_str = format!("-R{}", r);
        cmd_parts.push(&region_str);
    }
    let target_str = target.display().to_string();
    cmd_parts.push(&target_str);

    let cmd = cmd_parts.join(" ");
    let fallback = format!("import -window root '{}'", target.display());

    let result = sh_async(&cmd).await;
    if result.is_err() || !tokio::fs::try_exists(&target).await.unwrap_or(false) {
        let _ = sh_async(&fallback).await;
    }

    if tokio::fs::try_exists(&target).await.unwrap_or(false) {
        let meta = tokio::fs::metadata(&target).await.ok();
        let size = meta.map(|m| human_size(m.len())).unwrap_or_default();
        Ok(json!({ "path": target.display().to_string(), "size": size, "format": "png" }).to_string())
    } else {
        Err("Screenshot failed. Install screencapture (macOS) or imagemagick (Linux).".to_string())
    }
}

#[instrument(skip(args, _workspace_dir))]
pub async fn exec_clipboard_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).ok_or("Missing action (read|write)")?;

    match action {
        "read" => {
            let text = sh_async("pbpaste 2>/dev/null || xclip -selection clipboard -o 2>/dev/null || xsel --clipboard --output 2>/dev/null").await.unwrap_or_default();
            Ok(json!({ "content": text.trim(), "length": text.len() }).to_string())
        }
        "write" => {
            let content = args.get("content").and_then(|v| v.as_str()).ok_or("Missing content")?;
            let cmd = format!("echo -n '{}' | (pbcopy 2>/dev/null || xclip -selection clipboard 2>/dev/null || xsel --clipboard --input 2>/dev/null)", content.replace('\'', "'\\''"));
            sh_async(&cmd).await?;
            Ok(json!({ "status": "ok", "length": content.len() }).to_string())
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}

// ── Sync implementations ────────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub fn exec_screenshot(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let output_path = args.get("path").and_then(|v| v.as_str()).unwrap_or("screenshot.png");
    let region = args.get("region").and_then(|v| v.as_str());
    let delay = args.get("delay").and_then(|v| v.as_u64()).unwrap_or(0);

    let target = if output_path.starts_with('/') || output_path.starts_with('~') {
        expand_tilde(output_path)
    } else {
        resolve_path(workspace_dir, output_path)
    };

    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut cmd_args = vec!["-x".to_string()];
    if delay > 0 {
        cmd_args.push("-T".to_string());
        cmd_args.push(delay.to_string());
    }
    if let Some(r) = region {
        cmd_args.push("-R".to_string());
        cmd_args.push(r.to_string());
    }
    cmd_args.push(target.display().to_string());

    let output = std::process::Command::new("screencapture")
        .args(&cmd_args)
        .output()
        .or_else(|_| {
            std::process::Command::new("import")
                .arg("-window").arg("root")
                .arg(target.display().to_string())
                .output()
        })
        .map_err(|e| format!("Screenshot failed: {}", e))?;

    if output.status.success() && target.exists() {
        let size = std::fs::metadata(&target).ok().map(|m| human_size(m.len())).unwrap_or_default();
        Ok(json!({ "path": target.display().to_string(), "size": size, "format": "png" }).to_string())
    } else {
        Err(format!("Screenshot failed: {}", String::from_utf8_lossy(&output.stderr)))
    }
}

#[instrument(skip(args, _workspace_dir))]
pub fn exec_clipboard(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).ok_or("Missing action")?;

    match action {
        "read" => {
            let text = sh("pbpaste 2>/dev/null || xclip -selection clipboard -o 2>/dev/null").unwrap_or_default();
            Ok(json!({ "content": text.trim(), "length": text.len() }).to_string())
        }
        "write" => {
            let content = args.get("content").and_then(|v| v.as_str()).ok_or("Missing content")?;
            let mut child = std::process::Command::new("sh")
                .arg("-c")
                .arg("pbcopy 2>/dev/null || xclip -selection clipboard 2>/dev/null")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("Clipboard write failed: {}", e))?;

            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                stdin.write_all(content.as_bytes()).map_err(|e| format!("Write failed: {}", e))?;
            }
            let status = child.wait().map_err(|e| format!("Wait failed: {}", e))?;
            if status.success() {
                Ok(json!({ "status": "ok", "length": content.len() }).to_string())
            } else {
                Err("Clipboard write failed".to_string())
            }
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}
