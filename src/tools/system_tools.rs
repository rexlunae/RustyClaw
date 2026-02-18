//! System-level tools: disk analysis, monitoring, app management,
//! GUI automation, security auditing, and file summarization.

use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

use super::helpers::{resolve_path, expand_tilde};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Run a command and collect stdout, or return an error.
fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run `{}`: {}", cmd, e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "`{}` exited with {}: {}",
            cmd,
            output.status,
            stderr.trim()
        ))
    }
}

/// Run a shell pipeline via `sh -c`.
fn sh(script: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ── 1. disk_usage ───────────────────────────────────────────────────────────

/// Scan disk usage for a directory tree, returning the largest entries.
pub fn exec_disk_usage(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("~");
    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as usize;
    let top_n = args
        .get("top")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.exists() {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    // Use `du` with depth limit, sort numerically, take top N
    let script = format!(
        "du -d {} -k '{}' 2>/dev/null | sort -rn | head -{}",
        depth,
        target.display(),
        top_n + 1 // +1 because the root total is usually first
    );
    let raw = sh(&script)?;

    let mut entries = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() == 2 {
            if let Ok(kb) = parts[0].trim().parse::<u64>() {
                let size = human_size(kb * 1024);
                entries.push(json!({
                    "path": parts[1],
                    "size": size,
                    "bytes": kb * 1024,
                }));
            }
        }
    }

    Ok(json!({
        "path": target.display().to_string(),
        "depth": depth,
        "entries": entries,
    })
    .to_string())
}

/// Convert bytes to a human-readable string.
fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    for unit in UNITS {
        if val < 1024.0 {
            return format!("{:.1} {}", val, unit);
        }
        val /= 1024.0;
    }
    format!("{:.1} PB", val)
}

// ── 2. classify_files ───────────────────────────────────────────────────────

/// Classify files in a directory as user documents, caches, logs, etc.
pub fn exec_classify_files(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.is_dir() {
        return Err(format!("Not a directory: {}", target.display()));
    }

    let mut categories: std::collections::HashMap<&str, Vec<String>> =
        std::collections::HashMap::new();

    let entries = std::fs::read_dir(&target)
        .map_err(|e| format!("Cannot read directory: {}", e))?;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let cat = classify_entry(&name, &entry.path());
        categories.entry(cat).or_default().push(name);
    }

    let mut result = serde_json::Map::new();
    result.insert(
        "path".into(),
        json!(target.display().to_string()),
    );
    for (cat, files) in &categories {
        result.insert(cat.to_string(), json!(files));
    }

    Ok(Value::Object(result).to_string())
}

/// Heuristic classification of a file/directory entry.
fn classify_entry(name: &str, path: &Path) -> &'static str {
    let lower = name.to_lowercase();

    // Known cache patterns
    if lower.contains("cache")
        || lower.contains("caches")
        || lower == ".cache"
        || lower == "__pycache__"
        || lower.ends_with(".tmp")
    {
        return "cache";
    }

    // Logs
    if lower.contains("log") || lower.ends_with(".log") {
        return "log";
    }

    // App support / config
    if lower == ".config"
        || lower == "application support"
        || lower.starts_with('.')
        || lower == "library"
    {
        return "app_config";
    }

    // Build artifacts
    if lower == "node_modules"
        || lower == "target"
        || lower == "build"
        || lower == "dist"
        || lower == ".build"
    {
        return "build_artifact";
    }

    // Common user document directories
    if lower == "documents"
        || lower == "desktop"
        || lower == "downloads"
        || lower == "pictures"
        || lower == "music"
        || lower == "movies"
        || lower == "photos"
    {
        return "user_document";
    }

    // Cloud storage
    if lower.contains("dropbox")
        || lower.contains("google drive")
        || lower.contains("onedrive")
        || lower.contains("icloud")
    {
        return "cloud_storage";
    }

    // By extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_lowercase().as_str() {
            "pdf" | "doc" | "docx" | "txt" | "md" | "odt" | "rtf" | "csv" | "xlsx" | "pptx" => {
                return "user_document";
            }
            "jpg" | "jpeg" | "png" | "gif" | "heic" | "svg" | "bmp" | "webp" => {
                return "image";
            }
            "mp4" | "mov" | "avi" | "mkv" | "webm" => return "video",
            "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" => return "audio",
            "zip" | "tar" | "gz" | "bz2" | "7z" | "rar" | "xz" | "dmg" | "iso" => {
                return "archive";
            }
            "app" | "exe" | "msi" | "pkg" | "deb" | "rpm" => return "installer",
            _ => {}
        }
    }

    "other"
}

// ── 3. system_monitor ───────────────────────────────────────────────────────

/// Return current CPU, memory, and top-process information.
pub fn exec_system_monitor(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let metric = args
        .get("metric")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    let mut result = serde_json::Map::new();

    if metric == "all" || metric == "cpu" {
        // macOS: sysctl; Linux: /proc/loadavg
        let load = sh("sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg 2>/dev/null")
            .unwrap_or_default();
        result.insert("load_average".into(), json!(load.trim()));

        // Top CPU consumers
        let top_cpu = sh(
            "ps aux --sort=-%cpu 2>/dev/null | head -11 || ps aux -r | head -11"
        )
        .unwrap_or_default();
        result.insert("top_cpu_processes".into(), json!(top_cpu.trim()));
    }

    if metric == "all" || metric == "memory" {
        // macOS memory pressure
        let mem = sh(
            "vm_stat 2>/dev/null | head -10 || free -h 2>/dev/null"
        )
        .unwrap_or_default();
        result.insert("memory".into(), json!(mem.trim()));

        // Top memory consumers
        let top_mem = sh(
            "ps aux --sort=-%mem 2>/dev/null | head -11 || ps aux -m | head -11"
        )
        .unwrap_or_default();
        result.insert("top_memory_processes".into(), json!(top_mem.trim()));
    }

    if metric == "all" || metric == "disk" {
        let df = sh("df -h / 2>/dev/null").unwrap_or_default();
        result.insert("disk".into(), json!(df.trim()));
    }

    if metric == "all" || metric == "network" {
        let net = sh(
            "netstat -ib 2>/dev/null | head -5 || ip -s link 2>/dev/null | head -20"
        )
        .unwrap_or_default();
        result.insert("network".into(), json!(net.trim()));
    }

    Ok(Value::Object(result).to_string())
}

// ── 4. battery_health ───────────────────────────────────────────────────────

/// Report battery status, cycle count, and health on laptops.
pub fn exec_battery_health(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    // macOS: pmset + ioreg
    let pmset = sh("pmset -g batt 2>/dev/null").unwrap_or_default();
    let ioreg = sh(
        "ioreg -r -c AppleSmartBattery 2>/dev/null | \
         grep -E '(CycleCount|MaxCapacity|DesignCapacity|Temperature|FullyCharged|IsCharging)' | \
         head -10"
    )
    .unwrap_or_default();

    // Linux fallback: /sys/class/power_supply
    let linux = sh(
        "cat /sys/class/power_supply/BAT0/status 2>/dev/null && \
         cat /sys/class/power_supply/BAT0/capacity 2>/dev/null && \
         cat /sys/class/power_supply/BAT0/cycle_count 2>/dev/null"
    )
    .unwrap_or_default();

    if pmset.trim().is_empty() && linux.trim().is_empty() {
        return Ok(json!({
            "available": false,
            "note": "No battery detected or battery information not accessible."
        })
        .to_string());
    }

    let mut result = serde_json::Map::new();
    result.insert("available".into(), json!(true));
    if !pmset.trim().is_empty() {
        result.insert("pmset".into(), json!(pmset.trim()));
    }
    if !ioreg.trim().is_empty() {
        result.insert("battery_details".into(), json!(ioreg.trim()));
    }
    if !linux.trim().is_empty() {
        result.insert("linux_battery".into(), json!(linux.trim()));
    }

    Ok(Value::Object(result).to_string())
}

// ── 5. app_index ────────────────────────────────────────────────────────────

/// List installed applications with size, version, and source.
pub fn exec_app_index(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let filter = args
        .get("filter")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sort_by = args
        .get("sort")
        .and_then(|v| v.as_str())
        .unwrap_or("size");

    // macOS: scan /Applications, also check Homebrew
    let mut apps = Vec::new();

    // Native macOS apps
    let app_list = sh(
        "ls -1 /Applications 2>/dev/null | grep '.app$'"
    )
    .unwrap_or_default();

    for name in app_list.lines() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !filter.is_empty() && !trimmed.to_lowercase().contains(&filter.to_lowercase()) {
            continue;
        }
        let app_path = format!("/Applications/{}", trimmed);
        let size_str =
            sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", app_path)).unwrap_or_default();
        let size_kb: u64 = size_str.trim().parse().unwrap_or(0);

        // Try to get version from Info.plist
        let version = sh(&format!(
            "defaults read '/Applications/{}/Contents/Info' CFBundleShortVersionString 2>/dev/null",
            trimmed
        ))
        .unwrap_or_default();

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
    let brew_list = sh("brew list --cask 2>/dev/null").unwrap_or_default();
    for name in brew_list.lines() {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !filter.is_empty() && !trimmed.to_lowercase().contains(&filter.to_lowercase()) {
            continue;
        }
        let info = sh(&format!("brew info --cask --json=v2 '{}' 2>/dev/null | head -200", trimmed))
            .unwrap_or_default();
        apps.push(json!({
            "name": trimmed,
            "source": "homebrew",
            "info_snippet": info.chars().take(500).collect::<String>(),
        }));
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

    Ok(json!({
        "count": apps.len(),
        "sort": sort_by,
        "apps": apps,
    })
    .to_string())
}

// ── 6. cloud_browse ─────────────────────────────────────────────────────────

/// Browse and list files in local cloud storage sync folders.
pub fn exec_cloud_browse(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("detect");

    match action {
        "detect" => {
            // Detect known cloud storage folders
            let home = expand_tilde("~");
            let candidates = vec![
                ("Google Drive", home.join("Google Drive")),
                ("Google Drive (Stream)", home.join("Library/CloudStorage/GoogleDrive")),
                ("Dropbox", home.join("Dropbox")),
                ("OneDrive", home.join("OneDrive")),
                ("OneDrive (Business)", home.join("Library/CloudStorage/OneDrive")),
                ("iCloud Drive", home.join("Library/Mobile Documents/com~apple~CloudDocs")),
            ];

            let mut found = Vec::new();
            for (label, path) in &candidates {
                if path.exists() {
                    let size = sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", path.display()))
                        .unwrap_or_default();
                    let kb: u64 = size.trim().parse().unwrap_or(0);
                    found.push(json!({
                        "provider": label,
                        "path": path.display().to_string(),
                        "local_size": human_size(kb * 1024),
                    }));
                }
            }

            Ok(json!({ "cloud_folders": found }).to_string())
        }
        "list" => {
            let path_str = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: path (for list action)")?;
            let target = expand_tilde(path_str);
            if !target.is_dir() {
                return Err(format!("Not a directory: {}", target.display()));
            }
            let listing = sh(&format!(
                "ls -lhS '{}' 2>/dev/null | head -50",
                target.display()
            ))
            .unwrap_or_default();

            Ok(json!({
                "path": target.display().to_string(),
                "listing": listing.trim(),
            })
            .to_string())
        }
        _ => Err(format!("Unknown action: {}. Use 'detect' or 'list'.", action)),
    }
}

// ── 7. browser_cache ────────────────────────────────────────────────────────

/// Audit browser caches and download folders.
pub fn exec_browser_cache(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("scan");
    let browser = args
        .get("browser")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    let home = expand_tilde("~");

    let browsers: Vec<(&str, Vec<std::path::PathBuf>)> = vec![
        (
            "Chrome",
            vec![
                home.join("Library/Caches/Google/Chrome"),
                home.join("Library/Application Support/Google/Chrome/Default/Cache"),
                home.join(".cache/google-chrome"),
            ],
        ),
        (
            "Firefox",
            vec![
                home.join("Library/Caches/Firefox"),
                home.join(".cache/mozilla/firefox"),
            ],
        ),
        (
            "Safari",
            vec![
                home.join("Library/Caches/com.apple.Safari"),
                home.join("Library/Caches/com.apple.WebKit.WebProcess"),
            ],
        ),
        (
            "Edge",
            vec![
                home.join("Library/Caches/Microsoft Edge"),
                home.join(".cache/microsoft-edge"),
            ],
        ),
        (
            "Arc",
            vec![
                home.join("Library/Caches/company.thebrowser.Browser"),
            ],
        ),
    ];

    let downloads = home.join("Downloads");

    match action {
        "scan" => {
            let mut results = Vec::new();

            for (name, paths) in &browsers {
                if browser != "all" && !name.to_lowercase().contains(&browser.to_lowercase()) {
                    continue;
                }
                for p in paths {
                    if p.exists() {
                        let size =
                            sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", p.display()))
                                .unwrap_or_default();
                        let kb: u64 = size.trim().parse().unwrap_or(0);
                        if kb > 0 {
                            results.push(json!({
                                "browser": name,
                                "path": p.display().to_string(),
                                "size": human_size(kb * 1024),
                                "size_bytes": kb * 1024,
                            }));
                        }
                    }
                }
            }

            // Downloads folder
            let dl_size = sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", downloads.display()))
                .unwrap_or_default();
            let dl_kb: u64 = dl_size.trim().parse().unwrap_or(0);

            Ok(json!({
                "caches": results,
                "downloads": {
                    "path": downloads.display().to_string(),
                    "size": human_size(dl_kb * 1024),
                    "size_bytes": dl_kb * 1024,
                }
            })
            .to_string())
        }
        "clean" => {
            // Only clean caches, never downloads
            let mut cleaned = Vec::new();
            for (name, paths) in &browsers {
                if browser != "all" && !name.to_lowercase().contains(&browser.to_lowercase()) {
                    continue;
                }
                for p in paths {
                    if p.exists() {
                        let before = sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", p.display()))
                            .unwrap_or_default();
                        let _ = sh(&format!("rm -rf '{}'/Cache* '{}'/data_* 2>/dev/null",
                            p.display(), p.display()));
                        cleaned.push(json!({
                            "browser": name,
                            "path": p.display().to_string(),
                            "freed_approx": format!("~{}", human_size(
                                before.trim().parse::<u64>().unwrap_or(0) * 1024
                            )),
                        }));
                    }
                }
            }
            Ok(json!({ "cleaned": cleaned }).to_string())
        }
        _ => Err(format!("Unknown action: {}. Use 'scan' or 'clean'.", action)),
    }
}

// ── 8. screenshot ───────────────────────────────────────────────────────────

/// Take a screenshot of the screen or a specific region.
pub fn exec_screenshot(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let output_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("screenshot.png");
    let region = args.get("region").and_then(|v| v.as_str()); // "x,y,w,h"
    let delay = args.get("delay").and_then(|v| v.as_u64()).unwrap_or(0);

    let target = if output_path.starts_with('/') || output_path.starts_with('~') {
        expand_tilde(output_path)
    } else {
        resolve_path(workspace_dir, output_path)
    };

    // Create parent directories
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // macOS: screencapture
    let mut cmd_args = vec!["-x".to_string()]; // no sound
    if delay > 0 {
        cmd_args.push("-T".to_string());
        cmd_args.push(delay.to_string());
    }
    if let Some(r) = region {
        cmd_args.push("-R".to_string());
        cmd_args.push(r.to_string());
    }
    cmd_args.push(target.display().to_string());

    let output = Command::new("screencapture")
        .args(&cmd_args)
        .output()
        .or_else(|_| {
            // Linux fallback: import (ImageMagick)
            Command::new("import")
                .arg("-window")
                .arg("root")
                .arg(target.display().to_string())
                .output()
        })
        .map_err(|e| format!("Screenshot failed: {}. Install screencapture (macOS) or imagemagick (Linux).", e))?;

    if output.status.success() && target.exists() {
        let meta = std::fs::metadata(&target).ok();
        let size = meta.map(|m| human_size(m.len())).unwrap_or_default();
        Ok(json!({
            "path": target.display().to_string(),
            "size": size,
            "format": "png",
        })
        .to_string())
    } else {
        Err(format!(
            "Screenshot command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

// ── 9. clipboard ────────────────────────────────────────────────────────────

/// Read from or write to the system clipboard.
pub fn exec_clipboard(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action (read|write)".to_string())?;

    match action {
        "read" => {
            // macOS: pbpaste, Linux: xclip/xsel
            let text = sh("pbpaste 2>/dev/null || xclip -selection clipboard -o 2>/dev/null || xsel --clipboard --output 2>/dev/null")
                .unwrap_or_default();
            let len = text.len();
            Ok(json!({
                "content": text.trim(),
                "length": len,
            })
            .to_string())
        }
        "write" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: content")?;

            // Write via pipe to pbcopy/xclip
            let mut child = Command::new("sh")
                .arg("-c")
                .arg("pbcopy 2>/dev/null || xclip -selection clipboard 2>/dev/null || xsel --clipboard --input 2>/dev/null")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("Clipboard write failed: {}", e))?;

            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                stdin
                    .write_all(content.as_bytes())
                    .map_err(|e| format!("Failed to write to clipboard: {}", e))?;
            }
            let status = child.wait().map_err(|e| format!("Wait failed: {}", e))?;

            if status.success() {
                Ok(json!({
                    "status": "ok",
                    "length": content.len(),
                })
                .to_string())
            } else {
                Err("Clipboard write failed. No clipboard provider found.".to_string())
            }
        }
        _ => Err(format!("Unknown action: {}. Use 'read' or 'write'.", action)),
    }
}

// ── 10. audit_sensitive ─────────────────────────────────────────────────────

/// Scan files for potentially sensitive data patterns.
pub fn exec_audit_sensitive(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or(".");
    let max_files = args
        .get("max_files")
        .and_then(|v| v.as_u64())
        .unwrap_or(500) as usize;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    // Patterns to look for (each: label, grep -E pattern)
    let patterns: Vec<(&str, &str)> = vec![
        ("AWS Access Key", r"AKIA[0-9A-Z]{16}"),
        ("AWS Secret Key", r"(?i)aws[_-]?secret[_-]?access[_-]?key.*[=:]\s*[A-Za-z0-9/+=]{40}"),
        ("Private Key Header", r"-----BEGIN (RSA |DSA |EC |OPENSSH )?PRIVATE KEY-----"),
        ("GitHub Token", r"gh[ps]_[A-Za-z0-9_]{36,}"),
        ("Generic API Key", r#"(?i)(api[_-]?key|apikey|secret[_-]?key)\s*[=:"]\s*[A-Za-z0-9_\-]{20,}"#),
        ("Password Assignment", r#"(?i)(password|passwd|pwd)\s*[=:"]\s*[^\s]{8,}"#),
        ("JWT Token", r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
        ("Slack Token", r"xox[bpras]-[A-Za-z0-9-]{10,}"),
    ];

    let mut findings: Vec<Value> = Vec::new();

    // Collect files to scan
    let files: Vec<_> = walkdir_limited(&target, max_files);

    for file_path in &files {
        // Skip binary-looking files
        let _content = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (label, pattern) in &patterns {
            // Simple grep-style matching using the regex crate would be ideal,
            // but for now we shell out to grep for reliability
            let script = format!(
                "grep -nE '{}' '{}' 2>/dev/null | head -3",
                pattern,
                file_path.display()
            );
            let matches = sh(&script).unwrap_or_default();
            if !matches.trim().is_empty() {
                // Redact the actual values but keep the line numbers
                let lines: Vec<String> = matches
                    .lines()
                    .map(|l| {
                        if let Some(colon_pos) = l.find(':') {
                            format!("line {}: [REDACTED MATCH]", &l[..colon_pos])
                        } else {
                            "[REDACTED MATCH]".to_string()
                        }
                    })
                    .collect();

                findings.push(json!({
                    "file": file_path.display().to_string(),
                    "pattern": label,
                    "matches": lines,
                }));
            }
        }
    }

    // Also suppress the actual content of findings
    let _ = content_unused(&findings);

    Ok(json!({
        "scanned_files": files.len(),
        "findings": findings.len(),
        "details": findings,
    })
    .to_string())
}

/// Walk a directory up to `max` files, skipping hidden/binary dirs.
fn walkdir_limited(root: &Path, max: usize) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if files.len() >= max {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if files.len() >= max {
                break;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden, node_modules, .git
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                // Only text-ish files
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    match ext {
                        "rs" | "py" | "js" | "ts" | "json" | "toml" | "yaml" | "yml"
                        | "env" | "cfg" | "conf" | "ini" | "sh" | "bash" | "zsh"
                        | "txt" | "md" | "go" | "java" | "rb" | "php" | "xml"
                        | "properties" | "tf" | "tfvars" | "hcl" => {
                            files.push(path);
                        }
                        _ => {}
                    }
                } else {
                    // No extension — might be .env, Dockerfile, etc.
                    if name == ".env"
                        || name == "Dockerfile"
                        || name == "Makefile"
                        || name == "Vagrantfile"
                    {
                        files.push(path);
                    }
                }
            }
        }
    }
    files
}

/// Suppress "unused variable" warnings for the audit tool.
fn content_unused(_: &[Value]) {}

// ── 11. secure_delete ───────────────────────────────────────────────────────

/// Securely overwrite and delete a file or directory.
pub fn exec_secure_delete(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;
    let passes = args
        .get("passes")
        .and_then(|v| v.as_u64())
        .unwrap_or(3);
    let confirm = args
        .get("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.exists() {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    // Safety check: refuse to delete critical paths
    let critical = ["/", "/Users", "/home", "/System", "/Library", "/bin", "/usr"];
    let target_str = target.display().to_string();
    for c in &critical {
        if target_str == *c || target_str.starts_with(&format!("{}/.", c)) {
            return Err(format!("Refusing to delete critical path: {}", target_str));
        }
    }

    if !confirm {
        let size = if target.is_file() {
            std::fs::metadata(&target)
                .map(|m| human_size(m.len()))
                .unwrap_or_default()
        } else {
            let s = sh(&format!("du -sk '{}' 2>/dev/null | cut -f1", target.display()))
                .unwrap_or_default();
            human_size(s.trim().parse::<u64>().unwrap_or(0) * 1024)
        };
        return Ok(json!({
            "status": "confirm_required",
            "path": target.display().to_string(),
            "size": size,
            "is_directory": target.is_dir(),
            "message": "Set confirm=true to proceed with secure deletion.",
        })
        .to_string());
    }

    // Perform secure deletion
    if target.is_file() {
        // Overwrite with random data
        let len = std::fs::metadata(&target)
            .map(|m| m.len())
            .unwrap_or(0);
        for _ in 0..passes {
            let _ = sh(&format!(
                "dd if=/dev/urandom of='{}' bs=1 count={} conv=notrunc 2>/dev/null",
                target.display(),
                len
            ));
        }
        // Then overwrite with zeros
        let _ = sh(&format!(
            "dd if=/dev/zero of='{}' bs=1 count={} conv=notrunc 2>/dev/null",
            target.display(),
            len
        ));
        // Remove
        std::fs::remove_file(&target)
            .map_err(|e| format!("Failed to remove file: {}", e))?;

        Ok(json!({
            "status": "deleted",
            "path": target.display().to_string(),
            "passes": passes,
            "method": "overwrite + zero + unlink",
        })
        .to_string())
    } else {
        // For directories, use srm if available, else manual
        let result = sh(&format!(
            "srm -rzf '{}' 2>/dev/null",
            target.display()
        ));
        if result.is_ok() && !target.exists() {
            Ok(json!({
                "status": "deleted",
                "path": target.display().to_string(),
                "method": "srm (secure remove)",
            })
            .to_string())
        } else {
            // Fallback: overwrite each file, then remove tree
            let files = walkdir_limited(&target, 10000);
            for f in &files {
                let len = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                for _ in 0..passes {
                    let _ = sh(&format!(
                        "dd if=/dev/urandom of='{}' bs=1 count={} conv=notrunc 2>/dev/null",
                        f.display(),
                        len
                    ));
                }
            }
            std::fs::remove_dir_all(&target)
                .map_err(|e| format!("Failed to remove directory: {}", e))?;
            Ok(json!({
                "status": "deleted",
                "path": target.display().to_string(),
                "files_overwritten": files.len(),
                "passes": passes,
                "method": "overwrite + remove_dir_all",
            })
            .to_string())
        }
    }
}

// ── 12. summarize_file ──────────────────────────────────────────────────────

/// Generate a preview summary of a file based on its type.
pub fn exec_summarize_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;
    let max_lines = args
        .get("max_lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.exists() {
        return Err(format!("File does not exist: {}", target.display()));
    }

    let meta = std::fs::metadata(&target)
        .map_err(|e| format!("Cannot stat file: {}", e))?;
    let size = human_size(meta.len());

    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mut result = serde_json::Map::new();
    result.insert("path".into(), json!(target.display().to_string()));
    result.insert("size".into(), json!(size));
    result.insert("size_bytes".into(), json!(meta.len()));

    // Modified time
    if let Ok(modified) = meta.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            result.insert("modified_epoch".into(), json!(duration.as_secs()));
        }
    }

    // Type-specific summaries
    match ext.as_str() {
        // Text files — head + tail + line count
        "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "rb"
        | "php" | "sh" | "bash" | "zsh" | "txt" | "md" | "toml" | "yaml"
        | "yml" | "json" | "xml" | "html" | "css" | "sql" | "csv" | "log"
        | "cfg" | "conf" | "ini" => {
            let content = std::fs::read_to_string(&target).unwrap_or_default();
            let total_lines = content.lines().count();
            let head: Vec<&str> = content.lines().take(max_lines).collect();
            let tail_n = if total_lines > max_lines * 2 {
                max_lines / 4
            } else {
                0
            };
            let tail: Vec<&str> = if tail_n > 0 {
                content.lines().rev().take(tail_n).collect::<Vec<_>>().into_iter().rev().collect()
            } else {
                vec![]
            };

            result.insert("type".into(), json!("text"));
            result.insert("total_lines".into(), json!(total_lines));
            result.insert("head".into(), json!(head.join("\n")));
            if !tail.is_empty() {
                result.insert("tail".into(), json!(tail.join("\n")));
            }

            // For code files: extract top-level definitions
            if matches!(ext.as_str(), "rs" | "py" | "js" | "ts" | "go" | "java" | "rb") {
                let defs: Vec<&str> = content
                    .lines()
                    .filter(|l| {
                        let t = l.trim();
                        t.starts_with("pub fn ")
                            || t.starts_with("fn ")
                            || t.starts_with("def ")
                            || t.starts_with("class ")
                            || t.starts_with("function ")
                            || t.starts_with("export ")
                            || t.starts_with("func ")
                            || t.starts_with("struct ")
                            || t.starts_with("pub struct ")
                            || t.starts_with("enum ")
                            || t.starts_with("pub enum ")
                            || t.starts_with("impl ")
                            || t.starts_with("trait ")
                            || t.starts_with("pub trait ")
                            || t.starts_with("interface ")
                            || t.starts_with("type ")
                            || t.starts_with("pub type ")
                    })
                    .take(40)
                    .collect();
                if !defs.is_empty() {
                    result.insert("definitions".into(), json!(defs));
                }
            }
        }

        // PDF — page count + first page text
        "pdf" => {
            result.insert("type".into(), json!("pdf"));
            let page_count =
                sh(&format!("mdls -name kMDItemNumberOfPages -raw '{}' 2>/dev/null", target.display()))
                    .unwrap_or_default();
            if !page_count.trim().is_empty() && page_count.trim() != "(null)" {
                result.insert("pages".into(), json!(page_count.trim()));
            }
            // Try to extract first page text
            let text = sh(&format!(
                "textutil -convert txt -stdout '{}' 2>/dev/null | head -{}",
                target.display(),
                max_lines
            ))
            .unwrap_or_default();
            if !text.trim().is_empty() {
                result.insert("preview".into(), json!(text.trim()));
            }
        }

        // Image — dimensions + basic info
        "jpg" | "jpeg" | "png" | "gif" | "heic" | "webp" | "bmp" | "tiff" | "svg" => {
            result.insert("type".into(), json!("image"));
            let info = sh(&format!(
                "mdls -name kMDItemPixelWidth -name kMDItemPixelHeight -name kMDItemColorSpace '{}' 2>/dev/null",
                target.display()
            ))
            .unwrap_or_default();
            if !info.trim().is_empty() {
                result.insert("metadata".into(), json!(info.trim()));
            }
            // sips for basic dimensions
            let sips = sh(&format!(
                "sips -g pixelWidth -g pixelHeight '{}' 2>/dev/null",
                target.display()
            ))
            .unwrap_or_default();
            if !sips.trim().is_empty() {
                result.insert("dimensions".into(), json!(sips.trim()));
            }
        }

        // Video / audio — duration + codec
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "mp3" | "wav" | "m4a" | "flac" | "aac" | "ogg" => {
            result.insert("type".into(), json!("media"));
            let duration = sh(&format!(
                "mdls -name kMDItemDurationSeconds -raw '{}' 2>/dev/null",
                target.display()
            ))
            .unwrap_or_default();
            if !duration.trim().is_empty() && duration.trim() != "(null)" {
                result.insert("duration_seconds".into(), json!(duration.trim()));
            }
            let codec = sh(&format!(
                "mdls -name kMDItemCodecs '{}' 2>/dev/null",
                target.display()
            ))
            .unwrap_or_default();
            if !codec.trim().is_empty() {
                result.insert("codecs".into(), json!(codec.trim()));
            }
        }

        // Archive — list contents
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" => {
            result.insert("type".into(), json!("archive"));
            let listing = match ext.as_str() {
                "zip" => sh(&format!("unzip -l '{}' 2>/dev/null | tail -n +4 | head -30", target.display())),
                "tar" => sh(&format!("tar tf '{}' 2>/dev/null | head -30", target.display())),
                "gz" => sh(&format!("tar tzf '{}' 2>/dev/null | head -30", target.display())),
                "bz2" => sh(&format!("tar tjf '{}' 2>/dev/null | head -30", target.display())),
                "xz" => sh(&format!("tar tJf '{}' 2>/dev/null | head -30", target.display())),
                _ => Ok(String::new()),
            }
            .unwrap_or_default();
            if !listing.trim().is_empty() {
                result.insert("contents_preview".into(), json!(listing.trim()));
            }
        }

        _ => {
            result.insert("type".into(), json!("unknown"));
            // Try file command for MIME type
            let mime = sh(&format!("file -b --mime-type '{}' 2>/dev/null", target.display()))
                .unwrap_or_default();
            result.insert("mime".into(), json!(mime.trim()));
        }
    }

    Ok(Value::Object(result).to_string())
}
