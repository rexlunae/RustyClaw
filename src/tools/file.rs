//! File operation tools: read, write, edit, list, search, find.

use super::helpers::{resolve_path, expand_tilde, is_protected_path, display_path, should_visit, VAULT_ACCESS_DENIED};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tracing::{debug, warn, instrument};

/// Extensions that `textutil` (macOS) can convert to plain text.
const TEXTUTIL_EXTENSIONS: &[&str] = &[
    "doc", "docx", "rtf", "rtfd", "odt", "wordml", "webarchive", "html",
];

/// Try to extract plain text from a rich document using macOS `textutil`.
fn textutil_to_text(path: &Path) -> Option<String> {
    debug!(path = %path.display(), "Attempting textutil conversion");
    let output = std::process::Command::new("textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        if text.trim().is_empty() {
            debug!(path = %path.display(), "textutil returned empty text");
            None
        } else {
            debug!(path = %path.display(), len = text.len(), "textutil conversion successful");
            Some(text)
        }
    } else {
        debug!(path = %path.display(), "textutil conversion failed");
        None
    }
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_read_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

    if is_protected_path(&path) {
        warn!(path = %path.display(), "Attempted access to protected path");
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

    debug!(path = %path.display(), "Reading file");

    // First, try reading as UTF-8 plain text.
    let content = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            // If the file doesn't exist or can't be accessed at all, fail fast.
            if e.kind() == std::io::ErrorKind::NotFound
                || e.kind() == std::io::ErrorKind::PermissionDenied
            {
                return Err(format!("Failed to read file '{}': {}", path.display(), e));
            }

            // For binary / non-UTF8 files, try textutil on known document types.
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if TEXTUTIL_EXTENSIONS.contains(&ext.as_str()) {
                match textutil_to_text(&path) {
                    Some(text) => text,
                    None => {
                        return Err(format!(
                            "Failed to extract text from '{}': textutil conversion failed",
                            path.display(),
                        ));
                    }
                }
            } else if ext == "pdf" {
                // Try textutil first (works for some PDFs on macOS), then
                // fall back to pdftotext if available.
                if let Some(text) = textutil_to_text(&path) {
                    text
                } else if let Ok(output) = std::process::Command::new("pdftotext")
                    .args([path.to_string_lossy().as_ref(), "-"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                {
                    if output.status.success() {
                        let text = String::from_utf8_lossy(&output.stdout).to_string();
                        if text.trim().is_empty() {
                            return Err(format!(
                                "'{}' is a PDF but no text could be extracted.",
                                path.display(),
                            ));
                        }
                        text
                    } else {
                        return Err(format!(
                            "'{}' is a PDF. Install poppler (`brew install poppler`) \
                             for pdftotext, or use execute_command to process it.",
                            path.display(),
                        ));
                    }
                } else {
                    return Err(format!(
                        "'{}' is a PDF. Install poppler (`brew install poppler`) for \
                         pdftotext, or use execute_command to process it.",
                        path.display(),
                    ));
                }
            } else {
                return Err(format!(
                    "Failed to read file '{}': {} (binary file — use execute_command \
                     to process it with an appropriate tool)",
                    path.display(),
                    e,
                ));
            }
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).saturating_sub(1)) // 1-based → 0-based
        .unwrap_or(0);

    let end = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).min(total))
        .unwrap_or(total);

    if start >= total {
        return Err(format!(
            "start_line {} is past end of file ({} lines)",
            start + 1,
            total,
        ));
    }

    let slice = &lines[start..end.min(total)];
    // Prefix each line with its 1-based line number for model context.
    let numbered: Vec<String> = slice
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{:>4} │ {}", start + i + 1, line))
        .collect();

    debug!(path = %path.display(), lines_read = numbered.len(), "File read complete");
    Ok(numbered.join("\n"))
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_write_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: content".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

    if is_protected_path(&path) {
        warn!(path = %path.display(), "Attempted write to protected path");
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

    debug!(path = %path.display(), bytes = content.len(), "Writing file");

    // Always create parent directories.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directories for '{}': {}", path.display(), e))?;
    }

    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write file '{}': {}", path.display(), e))?;

    debug!(path = %path.display(), "File written successfully");
    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_edit_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;
    let old_string = args
        .get("old_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: old_string".to_string())?;
    let new_string = args
        .get("new_string")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: new_string".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

    if is_protected_path(&path) {
        warn!(path = %path.display(), "Attempted edit to protected path");
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

    debug!(path = %path.display(), "Editing file");

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;

    let count = content.matches(old_string).count();
    if count == 0 {
        debug!(path = %path.display(), "old_string not found");
        return Err(format!(
            "old_string not found in {}",
            path.display()
        ));
    }
    if count > 1 {
        debug!(path = %path.display(), count, "old_string found multiple times");
        return Err(format!(
            "old_string found {} times in {} — must match exactly once. \
             Add more surrounding context to make the match unique.",
            count,
            path.display()
        ));
    }

    let new_content = content.replacen(old_string, new_string, 1);
    std::fs::write(&path, &new_content)
        .map_err(|e| format!("Failed to write file '{}': {}", path.display(), e))?;

    debug!(path = %path.display(), "File edited successfully");
    Ok(format!("Successfully edited {}", path.display()))
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_list_directory(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

    if is_protected_path(&path) {
        warn!(path = %path.display(), "Attempted list of protected path");
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

    debug!(path = %path.display(), "Listing directory");

    let entries = std::fs::read_dir(&path)
        .map_err(|e| format!("Failed to read directory '{}': {}", path.display(), e))?;

    let mut items: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Error reading entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = entry
            .file_type()
            .map_err(|e| format!("Error reading file type: {}", e))?;
        if ft.is_dir() {
            items.push(format!("{}/", name));
        } else if ft.is_symlink() {
            items.push(format!("{}@", name));
        } else {
            items.push(name);
        }
    }

    items.sort();
    debug!(path = %path.display(), count = items.len(), "Directory listing complete");
    Ok(items.join("\n"))
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_search_files(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: pattern".to_string())?;
    let search_path = args.get("path").and_then(|v| v.as_str());
    let include = args.get("include").and_then(|v| v.as_str());

    let base = match search_path {
        Some(p) if p.starts_with('~') => expand_tilde(p),
        Some(p) => resolve_path(workspace_dir, p),
        None => workspace_dir.to_path_buf(),
    };

    let include_glob = match include {
        Some(pat) => Some(
            glob::Pattern::new(pat)
                .map_err(|e| format!("Invalid include glob '{}': {}", pat, e))?,
        ),
        None => None,
    };

    // Case-insensitive content search.
    let pattern_lower = pattern.to_lowercase();

    debug!(pattern, base = %base.display(), "Searching files for pattern");

    let mut results = Vec::new();
    let max_results: usize = 100;

    for entry in walkdir::WalkDir::new(&base)
        .follow_links(true)
        .into_iter()
        .filter_entry(should_visit)
    {
        if results.len() >= max_results {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }

        // Apply include filter.
        if let Some(ref glob_pat) = include_glob {
            if !glob_pat.matches(&entry.file_name().to_string_lossy()) {
                continue;
            }
        }

        // Read and search (case-insensitive).
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue, // skip binary / unreadable files
        };

        for (line_num, line) in content.lines().enumerate() {
            if results.len() >= max_results {
                break;
            }
            if line.to_lowercase().contains(&pattern_lower) {
                results.push(format!(
                    "{}:{}: {}",
                    display_path(entry.path(), workspace_dir),
                    line_num + 1,
                    line.trim()
                ));
            }
        }
    }

    if results.is_empty() {
        debug!(pattern, "No matches found");
        Ok("No matches found.".to_string())
    } else {
        let count = results.len();
        debug!(pattern, count, "Search complete");
        let mut output = results.join("\n");
        if count >= max_results {
            output.push_str(&format!(
                "\n\n(Results truncated at {} matches)",
                max_results
            ));
        }
        Ok(output)
    }
}

/// Returns `true` if the pattern string contains glob special characters.
fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

#[instrument(skip(args, workspace_dir))]
pub fn exec_find_files(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: pattern".to_string())?;
    let search_path = args.get("path").and_then(|v| v.as_str());

    let base = match search_path {
        Some(p) if p.starts_with('~') => expand_tilde(p),
        Some(p) => resolve_path(workspace_dir, p),
        None => workspace_dir.to_path_buf(),
    };

    let max_results: usize = 200;

    debug!(pattern, base = %base.display(), is_glob = is_glob_pattern(pattern), "Finding files");

    if is_glob_pattern(pattern) {
        // ── Glob mode ───────────────────────────────────────────────
        let effective = if pattern.contains('/') || pattern.starts_with("**") {
            pattern.to_string()
        } else {
            format!("**/{}", pattern)
        };

        let full = base.join(&effective);
        let full_str = full.to_string_lossy();

        let mut results = Vec::new();
        for entry in glob::glob(&full_str)
            .map_err(|e| format!("Invalid glob pattern: {}", e))?
        {
            if results.len() >= max_results {
                break;
            }
            if let Ok(path) = entry {
                results.push(display_path(&path, workspace_dir));
            }
        }

        format_find_results(results, max_results)
    } else {
        // ── Keyword mode — case-insensitive substring match ─────────
        // Multiple space-separated keywords: file matches if its name
        // contains ANY of them.
        let keywords: Vec<String> = pattern
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();

        if keywords.is_empty() {
            return Err("pattern must not be empty".to_string());
        }

        let mut results = Vec::new();

        for entry in walkdir::WalkDir::new(&base)
            .follow_links(true)
            .max_depth(8)
            .into_iter()
            .filter_entry(should_visit)
        {
            if results.len() >= max_results {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }

            let name_lower = entry.file_name().to_string_lossy().to_lowercase();
            if keywords.iter().any(|kw| name_lower.contains(kw.as_str())) {
                results.push(display_path(entry.path(), workspace_dir));
            }
        }

        format_find_results(results, max_results)
    }
}

fn format_find_results(results: Vec<String>, max_results: usize) -> Result<String, String> {
    if results.is_empty() {
        debug!("No files found");
        Ok("No files found.".to_string())
    } else {
        let count = results.len();
        debug!(count, "Find complete");
        let has_absolute = results.iter().any(|p| p.starts_with('/'));
        let mut output = String::new();
        if has_absolute {
            output.push_str("(Use these exact paths with read_file)\n");
        }
        output.push_str(&results.join("\n"));
        if count >= max_results {
            output.push_str(&format!(
                "\n\n(Results truncated at {} files)",
                max_results
            ));
        }
        Ok(output)
    }
}
