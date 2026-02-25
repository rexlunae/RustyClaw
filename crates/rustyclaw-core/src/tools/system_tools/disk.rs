//! Disk usage analysis and file classification.

use super::{sh, sh_async, resolve_path, expand_tilde};
use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, instrument};

// ── Helpers ─────────────────────────────────────────────────────────────────

fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    for unit in UNITS {
        if val < 1024.0 { return format!("{:.1} {}", val, unit); }
        val /= 1024.0;
    }
    format!("{:.1} PB", val)
}

fn classify_entry(name: &str, path: &Path) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("cache") || lower == "__pycache__" || lower.ends_with(".tmp") { return "cache"; }
    if lower.contains("log") && (lower.ends_with(".log") || path.is_dir()) { return "logs"; }
    if lower.starts_with('.') { return "hidden"; }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if matches!(ext.as_str(), "jpg"|"jpeg"|"png"|"gif"|"webp"|"svg"|"bmp"|"ico"|"heic"|"raw"|"cr2"|"nef") { return "images"; }
    if matches!(ext.as_str(), "mp4"|"mov"|"avi"|"mkv"|"webm"|"m4v"|"wmv"|"flv") { return "videos"; }
    if matches!(ext.as_str(), "mp3"|"wav"|"flac"|"aac"|"ogg"|"m4a"|"wma") { return "audio"; }
    if matches!(ext.as_str(), "pdf"|"doc"|"docx"|"txt"|"md"|"rtf"|"odt"|"xls"|"xlsx"|"ppt"|"pptx") { return "documents"; }
    if matches!(ext.as_str(), "zip"|"tar"|"gz"|"bz2"|"xz"|"7z"|"rar"|"dmg"|"iso") { return "archives"; }
    if matches!(ext.as_str(), "rs"|"py"|"js"|"ts"|"go"|"c"|"cpp"|"h"|"java"|"rb"|"swift"|"kt"|"sh"|"bash"|"zsh"|"fish") { return "code"; }
    if matches!(ext.as_str(), "json"|"yaml"|"yml"|"toml"|"ini"|"cfg"|"conf"|"xml"|"env") { return "config"; }
    if path.is_dir() { return "directories"; }
    "other"
}

// ── Async implementations ───────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub async fn exec_disk_usage_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("~");
    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let top_n = args.get("top").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    debug!(path = path_str, depth, top_n, "Disk usage scan");

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    let exists = tokio::fs::try_exists(&target).await.unwrap_or(false);
    if !exists {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    let script = format!("du -d {} -k '{}' 2>/dev/null | sort -rn | head -{}", depth, target.display(), top_n + 1);
    let raw = sh_async(&script).await?;

    let mut entries = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() == 2 {
            if let Ok(kb) = parts[0].trim().parse::<u64>() {
                entries.push(json!({ "path": parts[1], "size": human_size(kb * 1024), "bytes": kb * 1024 }));
            }
        }
    }

    Ok(json!({ "path": target.display().to_string(), "depth": depth, "entries": entries }).to_string())
}

#[instrument(skip(args, workspace_dir))]
pub async fn exec_classify_files_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing path")?;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    let metadata = tokio::fs::metadata(&target).await
        .map_err(|e| format!("Cannot read: {}", e))?;
    if !metadata.is_dir() {
        return Err(format!("Not a directory: {}", target.display()));
    }

    // Use spawn_blocking for sync directory walk
    let target_clone = target.clone();
    let categories = tokio::task::spawn_blocking(move || {
        let mut cats: std::collections::HashMap<&'static str, Vec<String>> = std::collections::HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&target_clone) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let cat = classify_entry(&name, &entry.path());
                cats.entry(cat).or_default().push(name);
            }
        }
        cats
    }).await.map_err(|e| format!("Task error: {}", e))?;

    let mut result = serde_json::Map::new();
    result.insert("path".into(), json!(target.display().to_string()));
    for (cat, files) in &categories {
        result.insert(cat.to_string(), json!(files));
    }
    Ok(Value::Object(result).to_string())
}

// ── Sync implementations ────────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub fn exec_disk_usage(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("~");
    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let top_n = args.get("top").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.exists() {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    let script = format!("du -d {} -k '{}' 2>/dev/null | sort -rn | head -{}", depth, target.display(), top_n + 1);
    let raw = sh(&script)?;

    let mut entries = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() == 2 {
            if let Ok(kb) = parts[0].trim().parse::<u64>() {
                entries.push(json!({ "path": parts[1], "size": human_size(kb * 1024), "bytes": kb * 1024 }));
            }
        }
    }

    Ok(json!({ "path": target.display().to_string(), "depth": depth, "entries": entries }).to_string())
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_classify_files(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing path")?;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.is_dir() {
        return Err(format!("Not a directory: {}", target.display()));
    }

    let mut categories: std::collections::HashMap<&str, Vec<String>> = std::collections::HashMap::new();
    for entry in std::fs::read_dir(&target).map_err(|e| format!("Cannot read: {}", e))?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let cat = classify_entry(&name, &entry.path());
        categories.entry(cat).or_default().push(name);
    }

    let mut result = serde_json::Map::new();
    result.insert("path".into(), json!(target.display().to_string()));
    for (cat, files) in &categories {
        result.insert(cat.to_string(), json!(files));
    }
    Ok(Value::Object(result).to_string())
}
