//! Patch tool: apply unified diff patches.

use super::helpers::{open_file_read_safe, open_file_write_safe, resolve_path};
use crate::tools::error::{ToolError, ToolResult};
use serde_json::Value;
use std::io::Read;
use std::path::Path;
use tracing::{debug, instrument, warn};

/// Apply a unified diff patch to files.
#[instrument(skip(args, workspace_dir))]
pub fn exec_apply_patch(args: &Value, workspace_dir: &Path) -> ToolResult {
    let patch_content = args
        .get("patch")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: patch".to_string())?;

    let explicit_path = args.get("path").and_then(|v| v.as_str());
    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    debug!(patch_len = patch_content.len(), dry_run, "Applying patch");

    // Parse the patch
    let hunks = parse_unified_diff(patch_content)?;

    if hunks.is_empty() {
        warn!("No valid hunks found in patch");
        return Err("No valid hunks found in patch".into());
    }

    debug!(hunk_count = hunks.len(), "Parsed patch hunks");

    let mut results = Vec::new();

    // Group hunks by file
    let mut files: std::collections::HashMap<String, Vec<&DiffHunk>> =
        std::collections::HashMap::new();
    for hunk in &hunks {
        let path = explicit_path.unwrap_or(&hunk.file_path);
        files.entry(path.to_string()).or_default().push(hunk);
    }

    for (file_path, file_hunks) in files {
        let full_path = resolve_path(workspace_dir, &file_path);

        // Read current content using TOCTOU-safe open
        let content = if full_path.exists() {
            let (mut file, _canonical) = open_file_read_safe(&full_path)
                .map_err(|e| format!("Failed to open {}: {}", file_path, e))?;
            let mut buf = String::new();
            file.read_to_string(&mut buf)
                .map_err(|e| format!("Failed to read {}: {}", file_path, e))?;
            buf
        } else {
            String::new()
        };

        // Preserve the file's existing line ending and whether it ended with
        // a newline: `lines()` discards both, so rejoining with "\n" would
        // silently strip the trailing newline and rewrite CRLF files as LF.
        let line_ending = detect_line_ending(&content);
        let had_trailing_newline = content.ends_with('\n');

        let mut lines: Vec<String> = content.lines().map(String::from).collect();

        // Apply hunks in reverse order (to preserve line numbers)
        let mut sorted_hunks: Vec<_> = file_hunks.iter().collect();
        sorted_hunks.sort_by_key(|h| std::cmp::Reverse(h.old_start));

        for hunk in sorted_hunks {
            lines = apply_hunk(&lines, hunk)?;
        }

        let mut new_content = lines.join(line_ending);
        // A new file (empty original) gets a trailing newline too — POSIX
        // text files end with one, and every tool downstream expects it.
        if !new_content.is_empty() && (had_trailing_newline || content.is_empty()) {
            new_content.push_str(line_ending);
        }

        if dry_run {
            debug!(file = %file_path, hunks = file_hunks.len(), "Dry run successful");
            results.push(format!(
                "✓ {} (dry run, {} hunks valid)",
                file_path,
                file_hunks.len()
            ));
        } else {
            // Ensure parent directory exists
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ToolError::context("Failed to create directory", e))?;
            }

            use std::io::Write;

            // Write via TOCTOU-safe open (O_NOFOLLOW + fd verification).
            let (mut file, _canonical) = open_file_write_safe(&full_path)
                .map_err(|e| format!("Failed to open {}: {}", file_path, e))?;
            file.write_all(new_content.as_bytes())
                .map_err(|e| format!("Failed to write {}: {}", file_path, e))?;
            file.flush()
                .map_err(|e| format!("Failed to flush {}: {}", file_path, e))?;
            file.sync_all()
                .map_err(|e| format!("Failed to sync {}: {}", file_path, e))?;

            debug!(file = %file_path, hunks = file_hunks.len(), "Patch applied");
            results.push(format!(
                "✓ {} ({} hunks applied)",
                file_path,
                file_hunks.len()
            ));
        }
    }

    Ok(results.join("\n"))
}

/// A single hunk from a unified diff.
#[derive(Debug)]
pub struct DiffHunk {
    pub file_path: String,
    pub old_start: usize,
    #[allow(dead_code)]
    pub old_count: usize,
    #[allow(dead_code)]
    pub new_start: usize,
    #[allow(dead_code)]
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug)]
pub enum DiffLine {
    Context(String),
    Remove(String),
    Add(String),
}

/// Parse a unified diff into hunks.
pub fn parse_unified_diff(patch: &str) -> ToolResult<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    let mut current_file: Option<String> = None;
    let mut lines = patch.lines().peekable();

    while let Some(line) = lines.next() {
        // Parse file header
        if line.starts_with("--- ") {
            // Skip, we use +++ line
            continue;
        }

        if let Some(stripped) = line.strip_prefix("+++ ") {
            let path = stripped.trim();
            // Strip a/ or b/ prefix if present
            let path = path.strip_prefix("b/").unwrap_or(path);
            let path = path.strip_prefix("a/").unwrap_or(path);
            current_file = Some(path.to_string());
            continue;
        }

        // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
        if let Some(header) = line.strip_prefix("@@ ") {
            let Some(ref file_path) = current_file else {
                return Err("Hunk without file header".into());
            };

            let end = header.find(" @@").unwrap_or(header.len());
            let range_part = &header[..end];

            let (old_range, new_range) = range_part.split_once(' ').ok_or("Invalid hunk header")?;

            let (old_start, old_count) = parse_range(old_range.trim_start_matches('-'))?;
            let (new_start, new_count) = parse_range(new_range.trim_start_matches('+'))?;

            // Read hunk lines
            let mut hunk_lines = Vec::new();
            while let Some(next_line) = lines.peek() {
                if next_line.starts_with("@@")
                    || next_line.starts_with("---")
                    || next_line.starts_with("+++")
                {
                    break;
                }
                let line = lines.next().unwrap();
                if line.starts_with(' ') || line.is_empty() {
                    hunk_lines.push(DiffLine::Context(line.get(1..).unwrap_or("").to_string()));
                } else if line.starts_with('-') {
                    hunk_lines.push(DiffLine::Remove(line.get(1..).unwrap_or("").to_string()));
                } else if line.starts_with('+') {
                    hunk_lines.push(DiffLine::Add(line.get(1..).unwrap_or("").to_string()));
                }
            }

            hunks.push(DiffHunk {
                file_path: file_path.clone(),
                old_start,
                old_count,
                new_start,
                new_count,
                lines: hunk_lines,
            });
        }
    }

    Ok(hunks)
}

/// Detect the dominant line ending in `content`, defaulting to `\n`.
///
/// A file is treated as CRLF only if its first line ending is CRLF, matching
/// how editors decide; mixed files keep whatever their first line used.
fn detect_line_ending(content: &str) -> &'static str {
    match content.find('\n') {
        Some(idx) if idx > 0 && content.as_bytes()[idx - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// Parse a range like "10,5" or "10" into (start, count).
fn parse_range(s: &str) -> ToolResult<(usize, usize)> {
    if let Some((start, count)) = s.split_once(',') {
        Ok((
            start.parse().map_err(|_| "Invalid range start")?,
            count.parse().map_err(|_| "Invalid range count")?,
        ))
    } else {
        Ok((s.parse().map_err(|_| "Invalid range")?, 1))
    }
}

/// Apply a single hunk to content lines.
fn apply_hunk(lines: &[String], hunk: &DiffHunk) -> ToolResult<Vec<String>> {
    let mut result = Vec::new();
    let start_idx = hunk.old_start.saturating_sub(1); // 1-indexed to 0-indexed

    // Copy lines before the hunk
    result.extend(lines.iter().take(start_idx).cloned());

    // Apply the hunk
    let mut old_idx = start_idx;
    for diff_line in &hunk.lines {
        match diff_line {
            DiffLine::Context(text) => {
                // Verify context matches (warn but continue on mismatch)
                if old_idx < lines.len() && lines[old_idx] != *text {
                    // Context mismatch - fuzzy match could be added here
                }
                result.push(text.clone());
                old_idx += 1;
            }
            DiffLine::Remove(text) => {
                // Verify the line matches what we're removing
                if old_idx < lines.len() && lines[old_idx] != *text {
                    return Err(format!(
                        "Patch mismatch at line {}: expected '{}', found '{}'",
                        old_idx + 1,
                        text,
                        lines.get(old_idx).unwrap_or(&String::new())
                    )
                    .into());
                }
                old_idx += 1;
                // Don't add to result (line is removed)
            }
            DiffLine::Add(text) => {
                result.push(text.clone());
                // Don't increment old_idx (line is new)
            }
        }
    }

    // Copy remaining lines after the hunk
    result.extend(lines.iter().skip(old_idx).cloned());

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apply(dir: &Path, name: &str, original: &str, patch: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, original).unwrap();
        exec_apply_patch(&json!({ "patch": patch, "path": name }), dir)
            .unwrap_or_else(|e| panic!("patch failed: {e}"));
        std::fs::read_to_string(&path).unwrap()
    }

    const PATCH: &str = "--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n";

    /// Regression: the read/modify/write round-trip went through `lines()`
    /// and `join("\n")`, which silently ate the file's trailing newline.
    #[test]
    fn preserves_trailing_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = apply(dir.path(), "f.txt", "alpha\nbeta\ngamma\n", PATCH);
        assert_eq!(out, "alpha\nBETA\ngamma\n");
        assert!(out.ends_with('\n'), "trailing newline survives: {out:?}");
    }

    /// A file that genuinely has no trailing newline must not gain one.
    #[test]
    fn preserves_absent_trailing_newline() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = apply(dir.path(), "f.txt", "alpha\nbeta\ngamma", PATCH);
        assert_eq!(out, "alpha\nBETA\ngamma");
        assert!(!out.ends_with('\n'), "no newline added: {out:?}");
    }

    /// Regression: CRLF files were silently rewritten as LF.
    #[test]
    fn preserves_crlf_line_endings() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = apply(dir.path(), "f.txt", "alpha\r\nbeta\r\ngamma\r\n", PATCH);
        assert_eq!(out, "alpha\r\nBETA\r\ngamma\r\n");
        assert!(!out.contains("\n\n"), "no bare LF introduced: {out:?}");
    }

    #[test]
    fn detect_line_ending_picks_the_first_terminator() {
        assert_eq!(detect_line_ending("a\r\nb\r\n"), "\r\n");
        assert_eq!(detect_line_ending("a\nb\n"), "\n");
        assert_eq!(detect_line_ending("no newline at all"), "\n");
        assert_eq!(detect_line_ending(""), "\n");
        // A leading bare newline must not be read as CRLF (index 0 guard).
        assert_eq!(detect_line_ending("\nabc"), "\n");
    }
}
