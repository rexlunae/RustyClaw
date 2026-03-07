//! Dedicated PDF analysis tool.
//!
//! Provides first-class PDF support beyond what `read_file` offers:
//! - Text extraction (pdftotext, textutil fallback)
//! - Page count and metadata
//! - Per-page extraction with page ranges
//! - Configurable output limits

use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
use tracing::{debug, instrument};

use super::helpers::{VAULT_ACCESS_DENIED, is_protected_path, resolve_path};

/// Maximum characters to return from a PDF extraction (default 100k).
const MAX_OUTPUT_CHARS: usize = 100_000;

/// Execute the `pdf` tool.
#[instrument(skip(args, workspace_dir))]
pub fn exec_pdf(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("extract");

    match action {
        "extract" => exec_pdf_extract(args, workspace_dir),
        "info" => exec_pdf_info(args, workspace_dir),
        "page_count" => exec_pdf_page_count(args, workspace_dir),
        other => Err(format!(
            "Unknown pdf action '{}'. Available: extract, info, page_count",
            other
        )),
    }
}

/// Extract text from a PDF file.
fn exec_pdf_extract(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);
    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    let start_page = args
        .get("start_page")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let end_page = args
        .get("end_page")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(MAX_OUTPUT_CHARS);

    debug!(path = %path.display(), "Extracting PDF text");

    // Try pdftotext first (from poppler-utils)
    let mut cmd = Command::new("pdftotext");
    if let Some(start) = start_page {
        cmd.arg("-f").arg(start.to_string());
    }
    if let Some(end) = end_page {
        cmd.arg("-l").arg(end.to_string());
    }
    cmd.arg(path.to_string_lossy().as_ref())
        .arg("-") // output to stdout
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if !text.trim().is_empty() {
                return Ok(truncate_output(&text, max_chars));
            }
        }
    }

    // Fallback: textutil (macOS)
    let output = Command::new("textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(path.to_string_lossy().as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if !text.trim().is_empty() {
                return Ok(truncate_output(&text, max_chars));
            }
        }
    }

    // Fallback: python pdfminer
    let mut python_cmd = Command::new("python3");
    python_cmd
        .arg("-c")
        .arg(format!(
            "from pdfminer.high_level import extract_text; print(extract_text('{}'))",
            path.display()
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Ok(output) = python_cmd.output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            if !text.trim().is_empty() {
                return Ok(truncate_output(&text, max_chars));
            }
        }
    }

    Err(format!(
        "Failed to extract text from '{}'. Install poppler-utils (pdftotext) \
         or python3 pdfminer.six for PDF support.",
        path.display()
    ))
}

/// Get PDF metadata/info.
fn exec_pdf_info(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);
    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    // Try pdfinfo (from poppler-utils)
    let output = Command::new("pdfinfo")
        .arg(path.to_string_lossy().as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let info = String::from_utf8_lossy(&output.stdout).to_string();
            return Ok(info);
        }
    }

    // Fallback: basic file info
    let metadata = std::fs::metadata(&path)
        .map_err(|e| format!("Failed to read file metadata: {}", e))?;

    Ok(format!(
        "File: {}\nSize: {} bytes\nNote: Install poppler-utils for full PDF metadata (pdfinfo).",
        path.display(),
        metadata.len()
    ))
}

/// Get PDF page count.
fn exec_pdf_page_count(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);
    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    // Try pdfinfo and parse "Pages:" line
    let output = Command::new("pdfinfo")
        .arg(path.to_string_lossy().as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let info = String::from_utf8_lossy(&output.stdout);
            for line in info.lines() {
                if let Some(pages) = line.strip_prefix("Pages:") {
                    return Ok(pages.trim().to_string());
                }
            }
        }
    }

    Err(format!(
        "Could not determine page count for '{}'. Install poppler-utils for pdfinfo.",
        path.display()
    ))
}

/// Truncate output to max characters, adding a notice if truncated.
fn truncate_output(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        format!(
            "{}\n\n[Output truncated at {} characters. Use start_page/end_page to read specific pages.]",
            &text[..max_chars],
            max_chars
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_pdf_missing_path() {
        let args = json!({"action": "extract"});
        let result = exec_pdf(&args, Path::new("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_pdf_nonexistent_file() {
        let args = json!({"action": "extract", "path": "/tmp/nonexistent.pdf"});
        let result = exec_pdf(&args, Path::new("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_pdf_unknown_action() {
        let args = json!({"action": "foobar", "path": "/tmp/test.pdf"});
        let result = exec_pdf(&args, Path::new("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown pdf action"));
    }

    #[test]
    fn test_truncate_output() {
        let text = "a".repeat(200);
        let truncated = truncate_output(&text, 100);
        assert!(truncated.len() > 100); // includes notice
        assert!(truncated.contains("[Output truncated"));

        let short = "hello";
        assert_eq!(truncate_output(short, 100), "hello");
    }
}
