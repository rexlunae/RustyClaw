//! File summarization: quick summaries of files based on type.

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

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub async fn exec_summarize_file_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing required parameter: path")?;
    let max_lines = args.get("max_lines").and_then(|v| v.as_u64()).unwrap_or(30) as usize;

    debug!(path = path_str, max_lines, "Summarize file");

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    let exists = tokio::fs::try_exists(&target).await.unwrap_or(false);
    if !exists {
        return Err(format!("File not found: {}", target.display()));
    }

    let meta = tokio::fs::metadata(&target).await.map_err(|e| format!("Cannot read: {}", e))?;
    if meta.is_dir() {
        return Err(format!("Path is a directory: {}", target.display()));
    }

    let mut result = serde_json::Map::new();
    result.insert("path".into(), json!(target.display().to_string()));
    result.insert("size".into(), json!(human_size(meta.len())));
    result.insert("size_bytes".into(), json!(meta.len()));

    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    result.insert("extension".into(), json!(ext));

    match ext.as_str() {
        "rs"|"py"|"js"|"ts"|"go"|"java"|"c"|"cpp"|"rb"|"sh"|"txt"|"md"|"toml"|"yaml"|"yml"|"json"|"xml"|"html"|"css"|"sql"|"csv"|"log" => {
            let content = tokio::fs::read_to_string(&target).await.unwrap_or_default();
            let total_lines = content.lines().count();
            let head: Vec<&str> = content.lines().take(max_lines).collect();

            result.insert("type".into(), json!("text"));
            result.insert("total_lines".into(), json!(total_lines));
            result.insert("head".into(), json!(head.join("\n")));

            // Extract definitions for code files
            if matches!(ext.as_str(), "rs"|"py"|"js"|"ts"|"go"|"java"|"rb") {
                let defs: Vec<&str> = content.lines()
                    .filter(|l| {
                        let t = l.trim();
                        t.starts_with("pub fn ") || t.starts_with("fn ") || t.starts_with("def ") ||
                        t.starts_with("class ") || t.starts_with("function ") || t.starts_with("export ") ||
                        t.starts_with("func ") || t.starts_with("struct ") || t.starts_with("pub struct ") ||
                        t.starts_with("enum ") || t.starts_with("trait ") || t.starts_with("impl ") ||
                        t.starts_with("interface ") || t.starts_with("type ")
                    })
                    .take(40)
                    .collect();
                if !defs.is_empty() {
                    result.insert("definitions".into(), json!(defs));
                }
            }
        }

        "pdf" => {
            result.insert("type".into(), json!("pdf"));
            let pages = sh_async(&format!("mdls -name kMDItemNumberOfPages -raw '{}' 2>/dev/null", target.display())).await.unwrap_or_default();
            if !pages.trim().is_empty() && pages.trim() != "(null)" {
                result.insert("pages".into(), json!(pages.trim()));
            }
            let text = sh_async(&format!("textutil -convert txt -stdout '{}' 2>/dev/null | head -{}", target.display(), max_lines)).await.unwrap_or_default();
            if !text.trim().is_empty() {
                result.insert("preview".into(), json!(text.trim()));
            }
        }

        "jpg"|"jpeg"|"png"|"gif"|"heic"|"webp"|"bmp"|"tiff"|"svg" => {
            result.insert("type".into(), json!("image"));
            let sips = sh_async(&format!("sips -g pixelWidth -g pixelHeight '{}' 2>/dev/null", target.display())).await.unwrap_or_default();
            if !sips.trim().is_empty() {
                result.insert("dimensions".into(), json!(sips.trim()));
            }
        }

        "mp4"|"mov"|"avi"|"mkv"|"webm"|"mp3"|"wav"|"m4a"|"flac"|"aac"|"ogg" => {
            result.insert("type".into(), json!("media"));
            let duration = sh_async(&format!("mdls -name kMDItemDurationSeconds -raw '{}' 2>/dev/null", target.display())).await.unwrap_or_default();
            if !duration.trim().is_empty() && duration.trim() != "(null)" {
                result.insert("duration_seconds".into(), json!(duration.trim()));
            }
        }

        "zip"|"tar"|"gz"|"bz2"|"xz"|"7z" => {
            result.insert("type".into(), json!("archive"));
            let listing = match ext.as_str() {
                "zip" => sh_async(&format!("unzip -l '{}' 2>/dev/null | tail -n +4 | head -30", target.display())).await,
                "tar" => sh_async(&format!("tar tf '{}' 2>/dev/null | head -30", target.display())).await,
                "gz" => sh_async(&format!("tar tzf '{}' 2>/dev/null | head -30", target.display())).await,
                _ => Ok(String::new()),
            }.unwrap_or_default();
            if !listing.trim().is_empty() {
                result.insert("contents_preview".into(), json!(listing.trim()));
            }
        }

        _ => {
            result.insert("type".into(), json!("unknown"));
            let mime = sh_async(&format!("file -b --mime-type '{}' 2>/dev/null", target.display())).await.unwrap_or_default();
            if !mime.trim().is_empty() {
                result.insert("mime_type".into(), json!(mime.trim()));
            }
        }
    }

    Ok(Value::Object(result).to_string())
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, workspace_dir))]
pub fn exec_summarize_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args.get("path").and_then(|v| v.as_str()).ok_or("Missing required parameter: path")?;
    let max_lines = args.get("max_lines").and_then(|v| v.as_u64()).unwrap_or(30) as usize;

    let target = if path_str.starts_with('~') || path_str.starts_with('/') {
        expand_tilde(path_str)
    } else {
        resolve_path(workspace_dir, path_str)
    };

    if !target.exists() {
        return Err(format!("File not found: {}", target.display()));
    }

    let meta = std::fs::metadata(&target).map_err(|e| format!("Cannot read: {}", e))?;
    if meta.is_dir() {
        return Err(format!("Path is a directory: {}", target.display()));
    }

    let mut result = serde_json::Map::new();
    result.insert("path".into(), json!(target.display().to_string()));
    result.insert("size".into(), json!(human_size(meta.len())));
    result.insert("size_bytes".into(), json!(meta.len()));

    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    result.insert("extension".into(), json!(ext));

    match ext.as_str() {
        "rs"|"py"|"js"|"ts"|"txt"|"md"|"toml"|"yaml"|"json" => {
            let content = std::fs::read_to_string(&target).unwrap_or_default();
            let total_lines = content.lines().count();
            let head: Vec<&str> = content.lines().take(max_lines).collect();
            result.insert("type".into(), json!("text"));
            result.insert("total_lines".into(), json!(total_lines));
            result.insert("head".into(), json!(head.join("\n")));
        }
        _ => {
            result.insert("type".into(), json!("unknown"));
            let mime = sh(&format!("file -b --mime-type '{}' 2>/dev/null", target.display())).unwrap_or_default();
            if !mime.trim().is_empty() {
                result.insert("mime_type".into(), json!(mime.trim()));
            }
        }
    }

    Ok(Value::Object(result).to_string())
}
