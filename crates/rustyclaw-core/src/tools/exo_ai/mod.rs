//! Exo AI administration tools (shared helpers + sync entry points).
//!
//! Async implementations live in `async_impl`.

use serde_json::Value;
use std::path::{Path, PathBuf};

mod async_impl;
pub use async_impl::*;

// ── Shared helpers (sync, used by both) ─────────────────────────────────────

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

fn parse_downloads_from_state(state: &Value) -> String {
    let downloads = match state.get("downloads") {
        Some(d) => d,
        None => return "No download information available.".into(),
    };

    let mut pending: Vec<String> = Vec::new();
    let mut completed: Vec<String> = Vec::new();

    if let Some(obj) = downloads.as_object() {
        for (_node_id, entries) in obj {
            if let Some(arr) = entries.as_array() {
                for entry in arr {
                    if let Some(dp) = entry.get("DownloadPending") {
                        let model_id = dp
                            .pointer("/shardMetadata/PipelineShardMetadata/modelCard/modelId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let downloaded = dp
                            .pointer("/downloaded/inBytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let total = dp
                            .pointer("/total/inBytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1);
                        let pct = if total > 0 {
                            (downloaded as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        let filled = (pct / 5.0) as usize;
                        let bar = format!(
                            "{}{}",
                            "█".repeat(filled),
                            "░".repeat(20_usize.saturating_sub(filled))
                        );
                        pending.push(format!(
                            "  ⏳ {} [{bar}] {:.1}%  ({} / {})",
                            model_id,
                            pct,
                            format_bytes(downloaded),
                            format_bytes(total)
                        ));
                    } else if let Some(dc) = entry.get("DownloadCompleted") {
                        let model_id = dc
                            .pointer("/shardMetadata/PipelineShardMetadata/modelCard/modelId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let total = dc
                            .pointer("/total/inBytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        completed.push(format!("  ✅ {} ({})", model_id, format_bytes(total)));
                    }
                }
            }
        }
    }

    if pending.is_empty() && completed.is_empty() {
        return "No downloads in progress or completed.".into();
    }

    let mut out = Vec::new();
    if !completed.is_empty() {
        out.push(format!("Completed ({}):", completed.len()));
        out.extend(completed);
    }
    pending.sort_by(|a, b| {
        let pct_a = a.split(']').next().unwrap_or("").matches('█').count();
        let pct_b = b.split(']').next().unwrap_or("").matches('█').count();
        pct_b.cmp(&pct_a).then_with(|| a.cmp(b))
    });
    if !pending.is_empty() {
        out.push(format!("Pending/Downloading ({}):", pending.len()));
        out.extend(pending);
    }
    out.join("\n")
}

fn parse_runner_errors(state: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(runners) = state.get("runners").and_then(|r| r.as_object()) {
        for (runner_id, info) in runners {
            if let Some(failed) = info.get("RunnerFailed") {
                let msg = failed
                    .get("errorMessage")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                errors.push(format!(
                    "  ⚠ Runner {}: {}",
                    &runner_id[..8.min(runner_id.len())],
                    msg
                ));
            }
        }
    }
    errors
}

fn parse_instances(state: &Value) -> Vec<String> {
    let mut instances = Vec::new();
    if let Some(inst_map) = state.get("instances").and_then(|i| i.as_object()) {
        for (id, info) in inst_map {
            let model_id = info
                .pointer("/MlxRingInstance/shardAssignments/modelId")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            instances.push(format!(
                "  🟢 {} (instance: {})",
                model_id,
                &id[..8.min(id.len())]
            ));
        }
    }
    instances
}

fn parse_node_info(state: &Value) -> Vec<String> {
    let mut nodes = Vec::new();
    if let Some(identities) = state.get("nodeIdentities").and_then(|n| n.as_object()) {
        for (node_id, info) in identities {
            let name = info
                .get("friendlyName")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let chip = info.get("chipId").and_then(|v| v.as_str()).unwrap_or("");
            let short_id = &node_id[..12.min(node_id.len())];

            let mem_info = state
                .pointer(&format!("/nodeMemory/{}", node_id))
                .map(|m| {
                    let ram_total = m
                        .pointer("/ramTotal/inBytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let ram_avail = m
                        .pointer("/ramAvailable/inBytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    format!(
                        " — RAM: {} / {}",
                        format_bytes(ram_avail),
                        format_bytes(ram_total)
                    )
                })
                .unwrap_or_default();

            let disk_info = state
                .pointer(&format!("/nodeDisk/{}", node_id))
                .map(|d| {
                    let disk_avail = d
                        .pointer("/available/inBytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let disk_total = d
                        .pointer("/total/inBytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    format!(
                        " — Disk: {} / {}",
                        format_bytes(disk_avail),
                        format_bytes(disk_total)
                    )
                })
                .unwrap_or_default();

            nodes.push(format!(
                "  📱 {} ({}) [{}…]{}{}",
                name, chip, short_id, mem_info, disk_info
            ));
        }
    }
    nodes
}

fn exo_repo_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".rustyclaw").join("exo")
}

fn find_exo_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = PathBuf::from(home);
    let candidates = [home.join(".rustyclaw").join("exo"), home.join("exo")];
    for p in &candidates {
        if p.join("pyproject.toml").exists() && p.join("src").join("exo").exists() {
            return Some(p.clone());
        }
    }
    None
}

fn exo_log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let log_dir = PathBuf::from(home).join(".rustyclaw");
    let _ = std::fs::create_dir_all(&log_dir);
    log_dir.join("exo.log")
}

fn is_exo_cloned() -> bool {
    find_exo_repo().is_some()
}

fn is_exo_installed() -> bool {
    find_exo_bin().is_some()
}

fn is_dashboard_built() -> bool {
    find_exo_repo()
        .map(|r| r.join("dashboard").join("build").exists())
        .unwrap_or(false)
}

fn find_exo_bin() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = PathBuf::from(home);
    let venv_candidates = [
        home.join(".rustyclaw")
            .join("exo")
            .join(".venv")
            .join("bin")
            .join("exo"),
        home.join("exo").join(".venv").join("bin").join("exo"),
    ];
    for p in &venv_candidates {
        if p.exists() {
            return Some(p.clone());
        }
    }
    if let Ok(out) = std::process::Command::new("which").arg("exo").output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

// ── Sync implementation ─────────────────────────────────────────────────────

fn sh(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Command exited with {}", output.status)
        };
        return Err(detail);
    }
    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

fn exo_api(method: &str, path: &str) -> Result<String, String> {
    exo_api_port(method, path, 52415, None)
}

fn exo_api_port(method: &str, path: &str, port: u64, body: Option<&str>) -> Result<String, String> {
    let url = format!("http://localhost:{}{}", port, path);
    let mut script = match method.to_uppercase().as_str() {
        "GET" => format!("curl -sf --max-time 5 '{}'", url),
        "POST" => {
            if let Some(b) = body {
                format!(
                    "curl -sf --max-time 30 -X POST -H 'Content-Type: application/json' -d '{}' '{}'",
                    b.replace('\'', "'\\''"),
                    url
                )
            } else {
                format!("curl -sf --max-time 30 -X POST '{}'", url)
            }
        }
        "DELETE" => format!("curl -sf --max-time 10 -X DELETE '{}'", url),
        _ => format!("curl -sf --max-time 5 -X {} '{}'", method, url),
    };
    script.push_str(" 2>/dev/null");
    sh(&script)
}

fn is_exo_running() -> bool {
    let proc_check = std::process::Command::new("sh")
        .arg("-c")
        .arg("pgrep -f '[e]xo\\.main' >/dev/null 2>&1 || pgrep -f '[u]v run exo' >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if proc_check {
        return true;
    }
    exo_api("GET", "/node_id").is_ok()
}

/// Execute an exo management action (sync wrapper).
pub fn exec_exo_manage(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    // For sync, we just call a simplified version or error out
    // In practice, the async version will be used via execute_tool
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status");

    // Basic sync implementations for critical paths
    match action {
        "status" => {
            let cloned = is_exo_cloned();
            let installed = is_exo_installed();
            let running = is_exo_running();
            Ok(format!(
                "exo status: cloned={}, installed={}, running={}",
                cloned, installed, running
            ))
        }
        _ => Err(format!(
            "Sync execution not supported for action '{}'. Use async dispatch.",
            action
        )),
    }
}
