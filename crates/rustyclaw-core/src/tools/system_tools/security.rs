//! Security tools: audit sensitive data and secure file deletion.

use super::{sh, sh_async, resolve_path, expand_tilde};
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, warn, instrument};

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    for unit in UNITS {
        if val < 1024.0 { return format!("{:.1} {}", val, unit); }
        val /= 1024.0;
    }
    format!("{:.1} PB", val)
}

fn walkdir_limited(root: &Path, max: usize) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if files.len() >= max { break; }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue; };
        for entry in entries.flatten() {
            if files.len() >= max { break; }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name == "node_modules" || name == "target" { continue; }
            if path.is_dir() { stack.push(path); }
            else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "rs"|"py"|"js"|"ts"|"json"|"toml"|"yaml"|"yml"|"env"|"cfg"|"conf"|"ini"|"sh"|"txt"|"md"|"go"|"java"|"rb"|"php"|"xml"|"tf"|"tfvars") {
                        files.push(path);
                    }
                } else if matches!(name.as_str(), ".env"|"Dockerfile"|"Makefile") {
                    files.push(path);
                }
            }
        }
    }
    files
}

// ── Async implementations ───────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub async fn exec_audit_sensitive_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let max_files = args.get("max_files").and_then(|v| v.as_u64()).unwrap_or(500) as usize;

    debug!(path = path_str, max_files, "Auditing for sensitive data");

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    let patterns: Vec<(&str, &str)> = vec![
        ("AWS Access Key", r"AKIA[0-9A-Z]{16}"),
        ("Private Key Header", r"-----BEGIN (RSA |DSA |EC |OPENSSH )?PRIVATE KEY-----"),
        ("GitHub Token", r"gh[ps]_[A-Za-z0-9_]{36,}"),
        ("Generic API Key", r#"(?i)(api[_-]?key|apikey|secret[_-]?key)\s*[=:"]\s*[A-Za-z0-9_\-]{20,}"#),
        ("JWT Token", r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
        ("Slack Token", r"xox[bpras]-[A-Za-z0-9-]{10,}"),
    ];

    // Use spawn_blocking for filesystem walk
    let target_clone = target.clone();
    let files = tokio::task::spawn_blocking(move || walkdir_limited(&target_clone, max_files))
        .await.map_err(|e| format!("Task error: {}", e))?;

    let mut findings: Vec<Value> = Vec::new();
    for file_path in &files {
        for (label, pattern) in &patterns {
            let script = format!("grep -nE '{}' '{}' 2>/dev/null | head -3", pattern, file_path.display());
            let matches = sh_async(&script).await.unwrap_or_default();
            if !matches.trim().is_empty() {
                let lines: Vec<String> = matches.lines()
                    .map(|l| if let Some(p) = l.find(':') { format!("line {}: [REDACTED]", &l[..p]) } else { "[REDACTED]".into() })
                    .collect();
                findings.push(json!({ "file": file_path.display().to_string(), "pattern": label, "matches": lines }));
            }
        }
    }

    Ok(json!({ "scanned_files": files.len(), "findings": findings.len(), "details": findings }).to_string())
}

#[instrument(skip(args, workspace_dir))]
pub async fn exec_secure_delete_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing path")?;
    let passes = args.get("passes").and_then(|v| v.as_u64()).unwrap_or(3);
    let confirm = args.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);

    debug!(path = path_str, passes, confirm, "Secure delete request");

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    let exists = tokio::fs::try_exists(&target).await.unwrap_or(false);
    if !exists {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    let critical = ["/", "/Users", "/home", "/System", "/Library", "/bin", "/usr"];
    let target_str = target.display().to_string();
    for c in &critical {
        if target_str == *c { return Err(format!("Refusing to delete: {}", target_str)); }
    }

    if !confirm {
        let is_dir = tokio::fs::metadata(&target).await.map(|m| m.is_dir()).unwrap_or(false);
        let size = if is_dir {
            let s = sh_async(&format!("du -sk '{}' 2>/dev/null | cut -f1", target.display())).await.unwrap_or_default();
            human_size(s.trim().parse::<u64>().unwrap_or(0) * 1024)
        } else {
            let m = tokio::fs::metadata(&target).await.ok();
            m.map(|m| human_size(m.len())).unwrap_or_default()
        };
        return Ok(json!({ "status": "confirm_required", "path": target_str, "size": size, "is_directory": is_dir, "message": "Set confirm=true to proceed." }).to_string());
    }

    warn!(path = %target_str, "Performing secure delete");

    // Try srm, shred, or manual overwrite
    let result = sh_async(&format!("srm -szr '{}' 2>&1 || shred -vzn {} -u '{}' 2>&1 || (dd if=/dev/urandom of='{}' bs=4k count=$(stat -f%z '{}' 2>/dev/null | awk '{{print int($1/4096)+1}}') 2>/dev/null && rm -rf '{}')", target.display(), passes, target.display(), target.display(), target.display(), target.display())).await;

    let still_exists = tokio::fs::try_exists(&target).await.unwrap_or(true);
    if still_exists {
        Err(format!("Secure delete failed. Path may still exist: {}", target_str))
    } else {
        Ok(json!({ "status": "deleted", "path": target_str, "passes": passes }).to_string())
    }
}

// ── Sync implementations ────────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub fn exec_audit_sensitive(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let max_files = args.get("max_files").and_then(|v| v.as_u64()).unwrap_or(500) as usize;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    let files = walkdir_limited(&target, max_files);
    let patterns: Vec<(&str, &str)> = vec![
        ("AWS Access Key", r"AKIA[0-9A-Z]{16}"),
        ("Private Key Header", r"-----BEGIN (RSA |DSA |EC )?PRIVATE KEY-----"),
        ("GitHub Token", r"gh[ps]_[A-Za-z0-9_]{36,}"),
    ];

    let mut findings: Vec<Value> = Vec::new();
    for file_path in &files {
        for (label, pattern) in &patterns {
            let script = format!("grep -nE '{}' '{}' 2>/dev/null | head -3", pattern, file_path.display());
            let matches = sh(&script).unwrap_or_default();
            if !matches.trim().is_empty() {
                findings.push(json!({ "file": file_path.display().to_string(), "pattern": label }));
            }
        }
    }

    Ok(json!({ "scanned_files": files.len(), "findings": findings.len(), "details": findings }).to_string())
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_secure_delete(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing path")?;
    let confirm = args.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false);

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.exists() {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    if !confirm {
        let is_dir = target.is_dir();
        return Ok(json!({ "status": "confirm_required", "path": target.display().to_string(), "is_directory": is_dir }).to_string());
    }

    let _ = sh(&format!("rm -rf '{}'", target.display()));
    if target.exists() {
        Err("Delete failed".to_string())
    } else {
        Ok(json!({ "status": "deleted", "path": target.display().to_string() }).to_string())
    }
}
