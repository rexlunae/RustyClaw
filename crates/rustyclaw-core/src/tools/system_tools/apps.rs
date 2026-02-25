//! Application index, cloud storage browsing, browser cache auditing.

use super::{sh, sh_async, expand_tilde};
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

#[instrument(skip(args, _workspace_dir))]
pub async fn exec_app_index_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");
    let sort_by = args.get("sort").and_then(|v| v.as_str()).unwrap_or("size");
    debug!(filter, sort = sort_by, "App index");

    let mut apps = Vec::new();

    // Native macOS apps
    let app_list = sh_async("ls -1 /Applications 2>/dev/null | grep '.app$'").await.unwrap_or_default();
    for name in app_list.lines() {
        let trimmed = name.trim();
        if trimmed.is_empty() { continue; }
        if !filter.is_empty() && !trimmed.to_lowercase().contains(&filter.to_lowercase()) { continue; }

        let app_path = format!("/Applications/{}", trimmed);
        let size_str = sh_async(&format!("du -sk '{}' 2>/dev/null | cut -f1", app_path)).await.unwrap_or_default();
        let size_kb: u64 = size_str.trim().parse().unwrap_or(0);
        let version = sh_async(&format!("defaults read '/Applications/{}/Contents/Info' CFBundleShortVersionString 2>/dev/null", trimmed)).await.unwrap_or_default();

        apps.push(json!({
            "name": trimmed.strip_suffix(".app").unwrap_or(trimmed),
            "path": app_path,
            "size": human_size(size_kb * 1024),
            "size_bytes": size_kb * 1024,
            "version": version.trim(),
            "source": "native",
        }));
    }

    // Homebrew casks
    let brew_list = sh_async("brew list --cask 2>/dev/null").await.unwrap_or_default();
    for name in brew_list.lines() {
        let trimmed = name.trim();
        if trimmed.is_empty() { continue; }
        if !filter.is_empty() && !trimmed.to_lowercase().contains(&filter.to_lowercase()) { continue; }
        apps.push(json!({ "name": trimmed, "source": "homebrew" }));
    }

    // Sort
    if sort_by == "size" {
        apps.sort_by(|a, b| {
            let sa = a.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let sb = b.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            sb.cmp(&sa)
        });
    } else {
        apps.sort_by(|a, b| {
            let na = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let nb = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
            na.to_lowercase().cmp(&nb.to_lowercase())
        });
    }

    Ok(json!({ "count": apps.len(), "sort": sort_by, "apps": apps }).to_string())
}

#[instrument(skip(args, _workspace_dir))]
pub async fn exec_cloud_browse_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("detect");

    match action {
        "detect" => {
            let home = expand_tilde("~");
            let candidates = vec![
                ("Google Drive", home.join("Google Drive")),
                ("Dropbox", home.join("Dropbox")),
                ("OneDrive", home.join("OneDrive")),
                ("iCloud Drive", home.join("Library/Mobile Documents/com~apple~CloudDocs")),
            ];

            let mut found = Vec::new();
            for (label, path) in &candidates {
                if tokio::fs::try_exists(path).await.unwrap_or(false) {
                    let size = sh_async(&format!("du -sk '{}' 2>/dev/null | cut -f1", path.display())).await.unwrap_or_default();
                    let kb: u64 = size.trim().parse().unwrap_or(0);
                    found.push(json!({ "provider": label, "path": path.display().to_string(), "local_size": human_size(kb * 1024) }));
                }
            }
            Ok(json!({ "cloud_folders": found }).to_string())
        }
        "list" => {
            let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing required parameter: path")?;
            let target = expand_tilde(path_str);
            let exists = tokio::fs::try_exists(&target).await.unwrap_or(false);
            if !exists { return Err(format!("Not found: {}", target.display())); }
            let listing = sh_async(&format!("ls -lhS '{}' 2>/dev/null | head -50", target.display())).await.unwrap_or_default();
            Ok(json!({ "path": target.display().to_string(), "listing": listing.trim() }).to_string())
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}

#[instrument(skip(args, _workspace_dir))]
pub async fn exec_browser_cache_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("scan");
    let browser = args.get("browser").and_then(|v| v.as_str()).unwrap_or("all");

    let home = expand_tilde("~");
    let browsers: Vec<(&str, Vec<std::path::PathBuf>)> = vec![
        ("Chrome", vec![home.join("Library/Caches/Google/Chrome"), home.join(".cache/google-chrome")]),
        ("Firefox", vec![home.join("Library/Caches/Firefox"), home.join(".cache/mozilla/firefox")]),
        ("Safari", vec![home.join("Library/Caches/com.apple.Safari")]),
        ("Edge", vec![home.join("Library/Caches/Microsoft Edge"), home.join(".cache/microsoft-edge")]),
    ];

    match action {
        "scan" => {
            let mut results = Vec::new();
            for (name, paths) in &browsers {
                if browser != "all" && !name.to_lowercase().contains(&browser.to_lowercase()) { continue; }
                for path in paths {
                    if tokio::fs::try_exists(path).await.unwrap_or(false) {
                        let size = sh_async(&format!("du -sk '{}' 2>/dev/null | cut -f1", path.display())).await.unwrap_or_default();
                        let kb: u64 = size.trim().parse().unwrap_or(0);
                        if kb > 0 {
                            results.push(json!({ "browser": name, "path": path.display().to_string(), "size": human_size(kb * 1024), "size_bytes": kb * 1024 }));
                        }
                    }
                }
            }
            Ok(json!({ "action": "scan", "caches": results }).to_string())
        }
        "clear" => {
            let mut cleared = Vec::new();
            for (name, paths) in &browsers {
                if browser != "all" && !name.to_lowercase().contains(&browser.to_lowercase()) { continue; }
                for path in paths {
                    if tokio::fs::try_exists(path).await.unwrap_or(false) {
                        let _ = sh_async(&format!("rm -rf '{}'/* 2>/dev/null", path.display())).await;
                        cleared.push(json!({ "browser": name, "path": path.display().to_string() }));
                    }
                }
            }
            Ok(json!({ "action": "clear", "cleared": cleared }).to_string())
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}

// ── Sync implementations ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir))]
pub fn exec_app_index(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let filter = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");
    let sort_by = args.get("sort").and_then(|v| v.as_str()).unwrap_or("size");

    let mut apps = Vec::new();
    let app_list = sh("ls -1 /Applications 2>/dev/null | grep '.app$'").unwrap_or_default();
    for name in app_list.lines() {
        let trimmed = name.trim();
        if trimmed.is_empty() { continue; }
        if !filter.is_empty() && !trimmed.to_lowercase().contains(&filter.to_lowercase()) { continue; }
        let app_path = format!("/Applications/{}", trimmed);
        let size_str = sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", app_path)).unwrap_or_default();
        let size_kb: u64 = size_str.trim().parse().unwrap_or(0);
        apps.push(json!({ "name": trimmed.strip_suffix(".app").unwrap_or(trimmed), "path": app_path, "size": human_size(size_kb * 1024), "size_bytes": size_kb * 1024, "source": "native" }));
    }

    if sort_by == "size" {
        apps.sort_by(|a, b| {
            let sa = a.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let sb = b.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
            sb.cmp(&sa)
        });
    }

    Ok(json!({ "count": apps.len(), "apps": apps }).to_string())
}

#[instrument(skip(args, _workspace_dir))]
pub fn exec_cloud_browse(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("detect");
    match action {
        "detect" => {
            let home = expand_tilde("~");
            let candidates = vec![("Google Drive", home.join("Google Drive")), ("Dropbox", home.join("Dropbox"))];
            let mut found = Vec::new();
            for (label, path) in &candidates {
                if path.exists() { found.push(json!({ "provider": label, "path": path.display().to_string() })); }
            }
            Ok(json!({ "cloud_folders": found }).to_string())
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}

#[instrument(skip(args, _workspace_dir))]
pub fn exec_browser_cache(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("scan");
    match action {
        "scan" => {
            let home = expand_tilde("~");
            let browsers = vec![("Chrome", home.join("Library/Caches/Google/Chrome")), ("Safari", home.join("Library/Caches/com.apple.Safari"))];
            let mut results = Vec::new();
            for (name, path) in &browsers {
                if path.exists() {
                    let size = sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", path.display())).unwrap_or_default();
                    let kb: u64 = size.trim().parse().unwrap_or(0);
                    results.push(json!({ "browser": name, "path": path.display().to_string(), "size": human_size(kb * 1024) }));
                }
            }
            Ok(json!({ "action": "scan", "caches": results }).to_string())
        }
        _ => Err(format!("Unknown action: {}", action)),
    }
}
