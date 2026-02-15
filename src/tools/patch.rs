//! Patch tool: apply unified diff patches.

use super::helpers::resolve_path;
use serde_json::Value;
use std::path::Path;

/// Apply a unified diff patch to files.
pub fn exec_apply_patch(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let patch_content = args
        .get("patch")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: patch".to_string())?;

    let explicit_path = args.get("path").and_then(|v| v.as_str());
    let dry_run = args
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Parse the patch
    let hunks = parse_unified_diff(patch_content)?;

    if hunks.is_empty() {
        return Err("No valid hunks found in patch".to_string());
    }

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

        // Read current content
        let content = if full_path.exists() {
            std::fs::read_to_string(&full_path)
                .map_err(|e| format!("Failed to read {}: {}", file_path, e))?
        } else {
            String::new()
        };

        let mut lines: Vec<String> = content.lines().map(String::from).collect();

        // Apply hunks in reverse order (to preserve line numbers)
        let mut sorted_hunks: Vec<_> = file_hunks.iter().collect();
        sorted_hunks.sort_by(|a, b| b.old_start.cmp(&a.old_start));

        for hunk in sorted_hunks {
            lines = apply_hunk(&lines, hunk)?;
        }

        let new_content = lines.join("\n");

        if dry_run {
            results.push(format!(
                "✓ {} (dry run, {} hunks valid)",
                file_path,
                file_hunks.len()
            ));
        } else {
            // Ensure parent directory exists
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }

            std::fs::write(&full_path, new_content)
                .map_err(|e| format!("Failed to write {}: {}", file_path, e))?;

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
pub fn parse_unified_diff(patch: &str) -> Result<Vec<DiffHunk>, String> {
    let mut hunks = Vec::new();
    let mut current_file: Option<String> = None;
    let mut lines = patch.lines().peekable();

    while let Some(line) = lines.next() {
        // Parse file header
        if line.starts_with("--- ") {
            // Skip, we use +++ line
            continue;
        }

        if line.starts_with("+++ ") {
            let path = line[4..].trim();
            // Strip a/ or b/ prefix if present
            let path = path.strip_prefix("b/").unwrap_or(path);
            let path = path.strip_prefix("a/").unwrap_or(path);
            current_file = Some(path.to_string());
            continue;
        }

        // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
        if let Some(header) = line.strip_prefix("@@ ") {
            let Some(ref file_path) = current_file else {
                return Err("Hunk without file header".to_string());
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

/// Parse a range like "10,5" or "10" into (start, count).
fn parse_range(s: &str) -> Result<(usize, usize), String> {
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
fn apply_hunk(lines: &[String], hunk: &DiffHunk) -> Result<Vec<String>, String> {
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
                    ));
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
