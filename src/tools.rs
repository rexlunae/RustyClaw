//! Agent tool system for RustyClaw.
//!
//! Provides a registry of tools that the language model can invoke, and
//! formatters that serialise the tool definitions into each provider's
//! native schema (OpenAI function-calling, Anthropic tool-use, Google
//! function declarations).

use crate::process_manager::{ProcessManager, SessionStatus, SharedProcessManager};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ── Global process manager ──────────────────────────────────────────────────

/// Global process manager for background exec sessions.
static PROCESS_MANAGER: OnceLock<SharedProcessManager> = OnceLock::new();

/// Get the global process manager instance.
pub fn process_manager() -> &'static SharedProcessManager {
    PROCESS_MANAGER.get_or_init(|| Arc::new(Mutex::new(ProcessManager::new())))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Resolve a path argument against the workspace root.
/// Absolute paths are used as-is; relative paths are joined to `workspace_dir`.
fn resolve_path(workspace_dir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_dir.join(p)
    }
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(p: &str) -> PathBuf {
    if p.starts_with('~') {
        dirs::home_dir()
            .map(|h| h.join(p.strip_prefix("~/").unwrap_or(&p[1..])))
            .unwrap_or_else(|| PathBuf::from(p))
    } else {
        PathBuf::from(p)
    }
}

/// Decide how to present a path found during a search.
///
/// If `found` lives inside `workspace_dir`, return a workspace-relative path
/// so the model can pass it directly to `read_file` (which will resolve it
/// back against `workspace_dir`).  Otherwise return the **absolute** path so
/// the model can still use it with tools that accept absolute paths.
fn display_path(found: &Path, workspace_dir: &Path) -> String {
    if let Ok(rel) = found.strip_prefix(workspace_dir) {
        rel.display().to_string()
    } else {
        found.display().to_string()
    }
}

/// Filter for `walkdir` — skip common non-content directories.
fn should_visit(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    if entry.file_type().is_dir() {
        !matches!(
            name.as_ref(),
            ".git" | "node_modules" | "target" | ".hg" | ".svn"
                | "__pycache__" | "dist" | "build"
        )
    } else {
        true
    }
}

// ── Tool definitions ────────────────────────────────────────────────────────

/// JSON-Schema-like parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParam {
    pub name: String,
    pub description: String,
    /// JSON Schema type: "string", "integer", "boolean", "array", "object".
    #[serde(rename = "type")]
    pub param_type: String,
    pub required: bool,
}

/// A tool that the agent can invoke.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Vec<ToolParam>,
    /// The function that executes the tool, returning a string result or error.
    pub execute: fn(args: &Value, workspace_dir: &Path) -> Result<String, String>,
}

// ── Tool registry ───────────────────────────────────────────────────────────

/// Return all available tools.
pub fn all_tools() -> Vec<&'static ToolDef> {
    vec![
        &READ_FILE,
        &WRITE_FILE,
        &EDIT_FILE,
        &LIST_DIRECTORY,
        &SEARCH_FILES,
        &FIND_FILES,
        &EXECUTE_COMMAND,
        &WEB_FETCH,
        &WEB_SEARCH,
        &PROCESS,
    ]
}

// ── Built-in tools ──────────────────────────────────────────────────────────

/// `read_file` — read the contents of a file on disk.
pub static READ_FILE: ToolDef = ToolDef {
    name: "read_file",
    description: "Read the contents of a file. Returns the file text. \
                  Handles plain text files directly and can also extract \
                  text from .docx, .doc, .rtf, .odt, .pdf, and .html files. \
                  If you have an absolute path from find_files or search_files, \
                  pass it exactly as-is. Use the optional start_line / end_line \
                  parameters to read a specific range (1-based, inclusive).",
    parameters: vec![],  // filled by init; see `read_file_params()`.
    execute: exec_read_file,
};

pub static WRITE_FILE: ToolDef = ToolDef {
    name: "write_file",
    description: "Create or overwrite a file with the given content. \
                  Parent directories are created automatically.",
    parameters: vec![],
    execute: exec_write_file,
};

pub static EDIT_FILE: ToolDef = ToolDef {
    name: "edit_file",
    description: "Make a targeted edit to an existing file using search-and-replace. \
                  The old_string must match exactly one location in the file. \
                  Include enough context lines to make the match unique.",
    parameters: vec![],
    execute: exec_edit_file,
};

pub static LIST_DIRECTORY: ToolDef = ToolDef {
    name: "list_directory",
    description: "List the contents of a directory. Returns file and \
                  directory names, with directories suffixed by '/'.",
    parameters: vec![],
    execute: exec_list_directory,
};

pub static SEARCH_FILES: ToolDef = ToolDef {
    name: "search_files",
    description: "Search file CONTENTS for a text pattern (like grep -i). \
                  The search is case-insensitive. Returns matching lines \
                  with paths and line numbers. Use `find_files` instead \
                  when searching by file name. Set `path` to an absolute \
                  directory (e.g. '/Users/alice') to search outside the \
                  workspace.",
    parameters: vec![],
    execute: exec_search_files,
};

pub static FIND_FILES: ToolDef = ToolDef {
    name: "find_files",
    description: "Find files by name. Returns paths that can be passed directly to read_file. Accepts plain keywords (case-insensitive \
                  substring match) OR glob patterns (e.g. '*.pdf'). Multiple \
                  keywords can be separated with spaces — a file matches if its \
                  name contains ANY keyword. Examples: 'resume', 'resume cv', \
                  '*.pdf'. Set `path` to an absolute directory to search outside \
                  the workspace (e.g. '/Users/alice'). Use `search_files` to \
                  search file CONTENTS instead.",
    parameters: vec![],
    execute: exec_find_files,
};

pub static EXECUTE_COMMAND: ToolDef = ToolDef {
    name: "execute_command",
    description: "Execute a shell command and return its output (stdout + stderr). \
                  Runs via `sh -c` in the workspace directory by default. \
                  Use for builds, tests, git operations, system lookups \
                  (e.g. `find ~ -name '*.pdf'`, `mdfind`, `which`), or \
                  any other CLI task. Set `working_dir` to an absolute \
                  path to run in a different directory.",
    parameters: vec![],
    execute: exec_execute_command,
};

pub static WEB_FETCH: ToolDef = ToolDef {
    name: "web_fetch",
    description: "Fetch and extract readable content from a URL (HTML → markdown or plain text). \
                  Use for reading web pages, documentation, articles, or any HTTP-accessible content. \
                  For JavaScript-heavy sites that require rendering, use a browser tool instead.",
    parameters: vec![],
    execute: exec_web_fetch,
};

pub static WEB_SEARCH: ToolDef = ToolDef {
    name: "web_search",
    description: "Search the web using Brave Search API. Returns titles, URLs, and snippets. \
                  Requires BRAVE_API_KEY environment variable to be set. \
                  Use for finding current information, research, and fact-checking.",
    parameters: vec![],
    execute: exec_web_search,
};

pub static PROCESS: ToolDef = ToolDef {
    name: "process",
    description: "Manage background exec sessions. Actions: list (show all sessions), \
                  poll (get new output + status for a session), log (get output with offset/limit), \
                  write (send data to stdin), kill (terminate a session), clear (remove completed sessions), \
                  remove (remove a specific session).",
    parameters: vec![],
    execute: exec_process,
};

/// We need a runtime-constructed param list because `Vec` isn't const.
/// This function is what the registry / formatters actually call.
pub fn read_file_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Path to the file to read. IMPORTANT: if you received \
                          an absolute path from find_files or search_files \
                          (starting with /), pass it exactly as-is. Only \
                          relative paths are resolved against the workspace root."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "start_line".into(),
            description: "First line to read (1-based, inclusive). Omit to start from the beginning.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "end_line".into(),
            description: "Last line to read (1-based, inclusive). Omit to read to the end.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn write_file_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Path to the file to create or overwrite.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "content".into(),
            description: "The full content to write to the file.".into(),
            param_type: "string".into(),
            required: true,
        },
    ]
}

fn edit_file_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Path to the file to edit.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "old_string".into(),
            description: "The exact text to find (must match exactly once). \
                          Include surrounding context lines for uniqueness."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "new_string".into(),
            description: "The replacement text.".into(),
            param_type: "string".into(),
            required: true,
        },
    ]
}

fn list_directory_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "path".into(),
        description: "Path to the directory to list.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

fn search_files_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "pattern".into(),
            description: "The text pattern to search for inside files.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "path".into(),
            description: "Directory to search in. Defaults to the workspace root. \
                          Use an absolute path (e.g. '/Users/alice/Documents') to \
                          search outside the workspace."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "include".into(),
            description: "Glob pattern to filter filenames (e.g. '*.rs').".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn find_files_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "pattern".into(),
            description: "Search term(s) or glob pattern. Plain words are matched \
                          case-insensitively against file names (e.g. 'resume' \
                          matches Resume.pdf). Separate multiple keywords with \
                          spaces to match ANY (e.g. 'resume cv'). Use glob \
                          syntax ('*', '?') for extension filters (e.g. '*.pdf')."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "path".into(),
            description: "Base directory for the search. Defaults to the workspace root. \
                          Use an absolute path (e.g. '/Users/alice' or '~') to \
                          search outside the workspace."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn execute_command_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "command".into(),
            description: "The shell command to execute (passed to sh -c).".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "working_dir".into(),
            description: "Working directory for the command. Defaults to the workspace root. \
                          Use an absolute path to run elsewhere."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "timeout_secs".into(),
            description: "Maximum seconds before killing the command (default: 30).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "background".into(),
            description: "Run in background immediately. Returns a sessionId for use with process tool.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "yieldMs".into(),
            description: "Milliseconds to wait before auto-backgrounding (default: 10000). \
                          Set to 0 to disable auto-background."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn web_fetch_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "url".into(),
            description: "HTTP or HTTPS URL to fetch.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "extract_mode".into(),
            description: "Extraction mode: 'markdown' (default) or 'text'. \
                          Markdown preserves links and structure; text is plain."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "max_chars".into(),
            description: "Maximum characters to return (truncates if exceeded). \
                          Default: 50000."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn web_search_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "query".into(),
            description: "Search query string.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "count".into(),
            description: "Number of results to return (1-10). Default: 5.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "country".into(),
            description: "2-letter country code for region-specific results (e.g., 'DE', 'US'). \
                          Default: 'US'."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "search_lang".into(),
            description: "ISO language code for search results (e.g., 'de', 'en', 'fr').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "freshness".into(),
            description: "Filter results by discovery time. Values: 'pd' (past 24h), \
                          'pw' (past week), 'pm' (past month), 'py' (past year), \
                          or date range 'YYYY-MM-DDtoYYYY-MM-DD'."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn process_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'list', 'poll', 'log', 'write', 'kill', 'clear', 'remove'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "sessionId".into(),
            description: "Session ID for poll/log/write/kill/remove actions.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "data".into(),
            description: "Data to write to stdin (for 'write' action).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "offset".into(),
            description: "Line offset for 'log' action (0-indexed). Omit to get last N lines.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "limit".into(),
            description: "Maximum lines to return for 'log' action. Default: 50.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

// ── Tool implementations ──────────────────────────────────────────────────────

/// Extensions that `textutil` (macOS) can convert to plain text.
const TEXTUTIL_EXTENSIONS: &[&str] = &[
    "doc", "docx", "rtf", "rtfd", "odt", "wordml", "webarchive", "html",
];

/// Try to extract plain text from a rich document using macOS `textutil`.
fn textutil_to_text(path: &Path) -> Option<String> {
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
            None
        } else {
            Some(text)
        }
    } else {
        None
    }
}

fn exec_read_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

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

    Ok(numbered.join("\n"))
}

fn exec_write_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: content".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

    // Always create parent directories.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directories for '{}': {}", path.display(), e))?;
    }

    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write file '{}': {}", path.display(), e))?;

    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

fn exec_edit_file(args: &Value, workspace_dir: &Path) -> Result<String, String> {
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

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read file '{}': {}", path.display(), e))?;

    let count = content.matches(old_string).count();
    if count == 0 {
        return Err(format!(
            "old_string not found in {}",
            path.display()
        ));
    }
    if count > 1 {
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

    Ok(format!("Successfully edited {}", path.display()))
}

fn exec_list_directory(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let path = resolve_path(workspace_dir, path_str);

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
    Ok(items.join("\n"))
}

fn exec_search_files(args: &Value, workspace_dir: &Path) -> Result<String, String> {
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
        Ok("No matches found.".to_string())
    } else {
        let count = results.len();
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

fn exec_find_files(args: &Value, workspace_dir: &Path) -> Result<String, String> {
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
        Ok("No files found.".to_string())
    } else {
        let count = results.len();
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

fn exec_execute_command(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: command".to_string())?;
    let working_dir = args.get("working_dir").and_then(|v| v.as_str());
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    
    // Background execution support
    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let yield_ms = args
        .get("yieldMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(10000); // Default 10 seconds before auto-background

    let cwd = match working_dir {
        Some(p) => resolve_path(workspace_dir, p),
        None => workspace_dir.to_path_buf(),
    };

    // If background requested immediately, spawn and return session ID
    if background {
        let manager = process_manager();
        let mut mgr = manager
            .lock()
            .map_err(|_| "Failed to acquire process manager lock".to_string())?;
        
        let session_id = mgr.spawn(command, cwd.to_string_lossy().as_ref(), Some(timeout_secs))?;
        
        return Ok(json!({
            "status": "running",
            "sessionId": session_id,
            "message": format!("Command backgrounded. Use process tool to poll session '{}'.", session_id)
        }).to_string());
    }

    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    // Poll for completion with yield/timeout logic
    let yield_deadline = Instant::now() + Duration::from_millis(yield_ms);
    let timeout_deadline = Instant::now() + Duration::from_secs(timeout_secs);
    
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break, // Process finished
            Ok(None) => {
                let now = Instant::now();
                
                // Check if we should auto-background
                if now >= yield_deadline && yield_ms > 0 {
                    // Move to background - transfer child to process manager
                    let manager = process_manager();
                    let mut mgr = manager
                        .lock()
                        .map_err(|_| "Failed to acquire process manager lock".to_string())?;
                    
                    // Create a session from the existing child
                    let remaining_timeout = timeout_deadline.saturating_duration_since(now);
                    let mut session = crate::process_manager::ExecSession::new(
                        command.to_string(),
                        cwd.to_string_lossy().to_string(),
                        Some(remaining_timeout),
                        child,
                    );
                    
                    // Try to read any output accumulated so far
                    session.try_read_output();
                    
                    // Insert session into manager
                    let session_id = mgr.insert(session);
                    
                    return Ok(json!({
                        "status": "running",
                        "sessionId": session_id,
                        "message": format!(
                            "Command still running after {}ms, backgrounded as session '{}'. \
                             Use process tool to poll.",
                            yield_ms, session_id
                        )
                    }).to_string());
                }
                
                // Check timeout
                if now >= timeout_deadline {
                    let _ = child.kill();
                    return Err(format!(
                        "Command timed out after {} seconds",
                        timeout_secs
                    ));
                }
                
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("Error waiting for command: {}", e)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to get command output: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str("[stderr]\n");
        result.push_str(&stderr);
    }

    if !output.status.success() {
        let exit = output.status.code().unwrap_or(-1);
        result.push_str(&format!("\n[exit code: {}]", exit));
    }

    // Truncate very long output.
    if result.len() > 50_000 {
        result.truncate(50_000);
        result.push_str("\n\n[output truncated at 50KB]");
    }

    if result.is_empty() {
        result = "(no output)".to_string();
    }

    Ok(result)
}

/// Fetch a URL and extract readable content as markdown or plain text.
///
/// This is a synchronous wrapper around the async HTTP fetch. In a real
/// async context you'd call the async version directly, but for the
/// tool interface we block on a runtime.
fn exec_web_fetch(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: url".to_string())?;

    let extract_mode = args
        .get("extract_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("markdown");

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(50_000) as usize;

    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // Use a blocking HTTP client since tools are sync
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("RustyClaw/0.1 (web_fetch tool)")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {} — {}", status.as_u16(), status.canonical_reason().unwrap_or("Unknown")));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = response
        .text()
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    // If it's not HTML, return as-is (might be JSON, plain text, etc.)
    if !content_type.contains("html") {
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        return Ok(result);
    }

    // Parse HTML and extract content
    let document = scraper::Html::parse_document(&body);

    // Try to find the main content area
    let content = extract_readable_content(&document);

    let result = match extract_mode {
        "text" => {
            // Plain text extraction
            html_to_text(&content)
        }
        _ => {
            // Markdown conversion (default)
            html2md::parse_html(&content)
        }
    };

    // Clean up the result
    let mut result = result
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");

    // Collapse multiple blank lines
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    // Truncate if needed
    if result.len() > max_chars {
        result.truncate(max_chars);
        result.push_str("\n\n[truncated]");
    }

    if result.trim().is_empty() {
        return Err("Page returned no extractable content".to_string());
    }

    Ok(result)
}

/// Extract the main readable content from an HTML document.
///
/// Tries common content selectors (article, main, etc.) and falls back
/// to the body. Strips navigation, scripts, styles, and other noise.
fn extract_readable_content(document: &scraper::Html) -> String {
    use scraper::Selector;

    // Selectors for main content areas (in priority order)
    let content_selectors = [
        "article",
        "main",
        "[role=\"main\"]",
        ".post-content",
        ".article-content",
        ".entry-content",
        ".content",
        "#content",
        ".post",
        ".article",
    ];

    // Try each content selector
    for selector_str in content_selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = document.select(&selector).next() {
                return element.html();
            }
        }
    }

    // Fall back to body, stripping unwanted elements
    if let Ok(body_selector) = Selector::parse("body") {
        if let Some(body) = document.select(&body_selector).next() {
            return body.html();
        }
    }

    // Last resort: return the whole document
    document.html()
}

/// Convert HTML to plain text, stripping all tags.
fn html_to_text(html: &str) -> String {
    use scraper::{Html, Selector};

    let document = Html::parse_fragment(html);

    // Remove script and style elements
    let mut text = String::new();

    // Walk the document and extract text nodes
    fn extract_text(node: scraper::ElementRef, text: &mut String) {
        for child in node.children() {
            if let Some(element) = scraper::ElementRef::wrap(child) {
                let tag = element.value().name();
                // Skip script, style, nav, header, footer
                if matches!(tag, "script" | "style" | "nav" | "header" | "footer" | "aside" | "noscript") {
                    continue;
                }
                // Add newlines for block elements
                if matches!(tag, "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr") {
                    text.push('\n');
                }
                extract_text(element, text);
                if matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                    text.push('\n');
                }
            } else if let Some(text_node) = child.value().as_text() {
                text.push_str(text_node.trim());
                text.push(' ');
            }
        }
    }

    if let Ok(selector) = Selector::parse("body") {
        if let Some(body) = document.select(&selector).next() {
            extract_text(body, &mut text);
        }
    }

    // If no body found, try the root
    if text.is_empty() {
        for element in document.root_element().children() {
            if let Some(el) = scraper::ElementRef::wrap(element) {
                extract_text(el, &mut text);
            }
        }
    }

    text
}

/// Search the web using Brave Search API.
fn exec_web_search(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(10)
        .max(1) as usize;

    let country = args
        .get("country")
        .and_then(|v| v.as_str())
        .unwrap_or("US");

    let search_lang = args.get("search_lang").and_then(|v| v.as_str());
    let freshness = args.get("freshness").and_then(|v| v.as_str());

    // Get API key from environment
    let api_key = std::env::var("BRAVE_API_KEY").map_err(|_| {
        "BRAVE_API_KEY environment variable not set. \
         Get a free API key at https://brave.com/search/api/"
            .to_string()
    })?;

    // Build the request URL
    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count,
    );

    if country != "ALL" {
        url.push_str(&format!("&country={}", country));
    }

    if let Some(lang) = search_lang {
        url.push_str(&format!("&search_lang={}", lang));
    }

    if let Some(fresh) = freshness {
        url.push_str(&format!("&freshness={}", fresh));
    }

    // Make the request
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", &api_key)
        .send()
        .map_err(|e| format!("Brave Search request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("Brave Search API error {}: {}", status.as_u16(), body));
    }

    let data: Value = response
        .json()
        .map_err(|e| format!("Failed to parse Brave Search response: {}", e))?;

    // Extract web results
    let web_results = data
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array());

    let Some(results) = web_results else {
        return Ok("No results found.".to_string());
    };

    if results.is_empty() {
        return Ok("No results found.".to_string());
    }

    // Format results
    let mut output = String::new();
    output.push_str(&format!("Search results for: {}\n\n", query));

    for (i, result) in results.iter().take(count).enumerate() {
        let title = result.get("title").and_then(|t| t.as_str()).unwrap_or("(no title)");
        let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let description = result
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");

        output.push_str(&format!("{}. {}\n", i + 1, title));
        output.push_str(&format!("   {}\n", url));
        if !description.is_empty() {
            output.push_str(&format!("   {}\n", description));
        }
        output.push('\n');
    }

    Ok(output)
}

/// Manage background exec sessions.
fn exec_process(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let session_id = args.get("sessionId").and_then(|v| v.as_str());

    let manager = process_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire process manager lock".to_string())?;

    match action {
        "list" => {
            // Poll all sessions first to update status
            mgr.poll_all();
            
            let sessions = mgr.list();
            if sessions.is_empty() {
                return Ok("No active sessions.".to_string());
            }

            let mut output = String::from("Background sessions:\n\n");
            for session in sessions {
                let status_str = match &session.status {
                    SessionStatus::Running => "running".to_string(),
                    SessionStatus::Exited(code) => format!("exited ({})", code),
                    SessionStatus::Killed => "killed".to_string(),
                    SessionStatus::TimedOut => "timed out".to_string(),
                };
                let elapsed = session.elapsed().as_secs();
                output.push_str(&format!(
                    "- {} [{}] ({}s)\n  {}\n",
                    session.id, status_str, elapsed, session.command
                ));
            }
            Ok(output)
        }

        "poll" => {
            let id = session_id.ok_or("Missing sessionId for poll action")?;
            
            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            // Try to read new output and check exit status
            session.try_read_output();
            let exited = session.check_exit();

            let new_output = session.poll_output();
            let status_str = match &session.status {
                SessionStatus::Running => "running".to_string(),
                SessionStatus::Exited(code) => format!("exited ({})", code),
                SessionStatus::Killed => "killed".to_string(),
                SessionStatus::TimedOut => "timed out".to_string(),
            };

            let mut result = String::new();
            if !new_output.is_empty() {
                result.push_str(new_output);
                if !new_output.ends_with('\n') {
                    result.push('\n');
                }
                result.push('\n');
            }
            
            if exited {
                result.push_str(&format!("Process {}.", status_str));
            } else {
                result.push_str(&format!("Process still {}.", status_str));
            }

            Ok(result)
        }

        "log" => {
            let id = session_id.ok_or("Missing sessionId for log action")?;
            
            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            // Update output first
            session.try_read_output();

            let offset = args.get("offset").and_then(|v| v.as_u64()).map(|n| n as usize);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .or(Some(50));

            let output = session.log_output(offset, limit);
            if output.is_empty() {
                Ok("(no output)".to_string())
            } else {
                Ok(output)
            }
        }

        "write" => {
            let id = session_id.ok_or("Missing sessionId for write action")?;
            let data = args
                .get("data")
                .and_then(|v| v.as_str())
                .ok_or("Missing data for write action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            session.write_stdin(data)?;
            Ok(format!("Wrote {} bytes to session {}", data.len(), id))
        }

        "kill" => {
            let id = session_id.ok_or("Missing sessionId for kill action")?;

            let session = mgr
                .get_mut(id)
                .ok_or_else(|| format!("No session found: {}", id))?;

            session.kill()?;
            Ok(format!("Killed session {}", id))
        }

        "clear" => {
            mgr.clear_completed();
            Ok("Cleared completed sessions.".to_string())
        }

        "remove" => {
            let id = session_id.ok_or("Missing sessionId for remove action")?;
            
            if let Some(mut session) = mgr.remove(id) {
                // Kill if still running
                if session.status == SessionStatus::Running {
                    let _ = session.kill();
                }
                Ok(format!("Removed session {}", id))
            } else {
                Err(format!("No session found: {}", id))
            }
        }

        _ => Err(format!("Unknown action: {}. Valid: list, poll, log, write, kill, clear, remove", action)),
    }
}

// ── Provider-specific formatters ────────────────────────────────────────────

/// Parameters for a tool, building a JSON Schema `properties` / `required`.
fn params_to_json_schema(params: &[ToolParam]) -> (Value, Value) {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for p in params {
        let mut prop = serde_json::Map::new();
        prop.insert("type".into(), json!(p.param_type));
        prop.insert("description".into(), json!(p.description));
        properties.insert(p.name.clone(), Value::Object(prop));
        if p.required {
            required.push(json!(p.name));
        }
    }

    (Value::Object(properties), Value::Array(required))
}

/// Resolve the parameter list for a tool (static defs use empty vecs
/// because Vec isn't const; we resolve at call time).
fn resolve_params(tool: &ToolDef) -> Vec<ToolParam> {
    if !tool.parameters.is_empty() {
        return tool.parameters.clone();
    }
    match tool.name {
        "read_file" => read_file_params(),
        "write_file" => write_file_params(),
        "edit_file" => edit_file_params(),
        "list_directory" => list_directory_params(),
        "search_files" => search_files_params(),
        "find_files" => find_files_params(),
        "execute_command" => execute_command_params(),
        "web_fetch" => web_fetch_params(),
        "web_search" => web_search_params(),
        "process" => process_params(),
        _ => vec![],
    }
}

/// OpenAI / OpenAI-compatible function-calling format.
///
/// ```json
/// { "type": "function", "function": { "name", "description", "parameters": { … } } }
/// ```
pub fn tools_openai() -> Vec<Value> {
    all_tools()
        .into_iter()
        .map(|t| {
            let params = resolve_params(t);
            let (properties, required) = params_to_json_schema(&params);
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": {
                        "type": "object",
                        "properties": properties,
                        "required": required,
                    }
                }
            })
        })
        .collect()
}

/// Anthropic tool-use format.
///
/// ```json
/// { "name", "description", "input_schema": { … } }
/// ```
pub fn tools_anthropic() -> Vec<Value> {
    all_tools()
        .into_iter()
        .map(|t| {
            let params = resolve_params(t);
            let (properties, required) = params_to_json_schema(&params);
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            })
        })
        .collect()
}

/// Google Gemini function-declaration format.
///
/// ```json
/// { "name", "description", "parameters": { … } }
/// ```
pub fn tools_google() -> Vec<Value> {
    all_tools()
        .into_iter()
        .map(|t| {
            let params = resolve_params(t);
            let (properties, required) = params_to_json_schema(&params);
            json!({
                "name": t.name,
                "description": t.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }
            })
        })
        .collect()
}

// ── Tool execution ──────────────────────────────────────────────────────────

/// Find a tool by name and execute it with the given arguments.
pub fn execute_tool(name: &str, args: &Value, workspace_dir: &Path) -> Result<String, String> {
    for tool in all_tools() {
        if tool.name == name {
            return (tool.execute)(args, workspace_dir);
        }
    }
    Err(format!("Unknown tool: {}", name))
}

// ── Wire types for WebSocket protocol ───────────────────────────────────────

/// A tool call requested by the model (sent gateway → client for display).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// The result of executing a tool (sent gateway → client for display,
/// and also injected back into the conversation for the model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: String,
    pub name: String,
    pub result: String,
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Helper: return the project root as workspace dir for tests.
    fn ws() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    // ── read_file ───────────────────────────────────────────────────

    #[test]
    fn test_read_file_this_file() {
        let args = json!({ "path": file!(), "start_line": 1, "end_line": 5 });
        let result = exec_read_file(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("Agent tool system"));
    }

    #[test]
    fn test_read_file_missing() {
        let args = json!({ "path": "/nonexistent/file.txt" });
        let result = exec_read_file(&args, ws());
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_no_path() {
        let args = json!({});
        let result = exec_read_file(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_read_file_relative() {
        // Relative path should resolve against workspace_dir.
        let args = json!({ "path": "Cargo.toml", "start_line": 1, "end_line": 3 });
        let result = exec_read_file(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("package"));
    }

    // ── write_file ──────────────────────────────────────────────────

    #[test]
    fn test_write_file_and_read_back() {
        let dir = std::env::temp_dir().join("rustyclaw_test_write");
        let _ = std::fs::remove_dir_all(&dir);
        let args = json!({
            "path": "sub/test.txt",
            "content": "hello world"
        });
        let result = exec_write_file(&args, &dir);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("11 bytes"));

        let content = std::fs::read_to_string(dir.join("sub/test.txt")).unwrap();
        assert_eq!(content, "hello world");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── edit_file ───────────────────────────────────────────────────

    #[test]
    fn test_edit_file_single_match() {
        let dir = std::env::temp_dir().join("rustyclaw_test_edit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "aaa\nbbb\nccc\n").unwrap();

        let args = json!({ "path": "f.txt", "old_string": "bbb", "new_string": "BBB" });
        let result = exec_edit_file(&args, &dir);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(dir.join("f.txt")).unwrap();
        assert_eq!(content, "aaa\nBBB\nccc\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edit_file_no_match() {
        let dir = std::env::temp_dir().join("rustyclaw_test_edit_no");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "aaa\nbbb\n").unwrap();

        let args = json!({ "path": "f.txt", "old_string": "zzz", "new_string": "ZZZ" });
        let result = exec_edit_file(&args, &dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edit_file_multiple_matches() {
        let dir = std::env::temp_dir().join("rustyclaw_test_edit_multi");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "aaa\naaa\n").unwrap();

        let args = json!({ "path": "f.txt", "old_string": "aaa", "new_string": "bbb" });
        let result = exec_edit_file(&args, &dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("2 times"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── list_directory ──────────────────────────────────────────────

    #[test]
    fn test_list_directory() {
        let args = json!({ "path": "src" });
        let result = exec_list_directory(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("tools.rs"));
        assert!(text.contains("main.rs"));
    }

    // ── search_files ────────────────────────────────────────────────

    #[test]
    fn test_search_files_finds_pattern() {
        let args = json!({ "pattern": "exec_read_file", "path": "src", "include": "*.rs" });
        let result = exec_search_files(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("tools.rs"));
    }

    #[test]
    fn test_search_files_no_match() {
        let dir = std::env::temp_dir().join("rustyclaw_test_search_none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello world\n").unwrap();

        let args = json!({ "pattern": "XYZZY_NEVER_42" });
        let result = exec_search_files(&args, &dir);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("No matches"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── find_files ──────────────────────────────────────────────────

    #[test]
    fn test_find_files_glob() {
        let args = json!({ "pattern": "*.toml" });
        let result = exec_find_files(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("Cargo.toml"));
    }

    #[test]
    fn test_find_files_keyword_case_insensitive() {
        // "cargo" should match "Cargo.toml" (case-insensitive).
        let args = json!({ "pattern": "cargo" });
        let result = exec_find_files(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("Cargo.toml"));
    }

    #[test]
    fn test_find_files_multiple_keywords() {
        // Space-separated keywords: match ANY.
        let args = json!({ "pattern": "cargo license" });
        let result = exec_find_files(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("Cargo.toml"));
        assert!(text.contains("LICENSE"));
    }

    #[test]
    fn test_find_files_keyword_no_match() {
        let dir = std::env::temp_dir().join("rustyclaw_test_find_kw");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "content").unwrap();

        let args = json!({ "pattern": "resume" });
        let result = exec_find_files(&args, &dir);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("No files found"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── execute_command ─────────────────────────────────────────────

    #[test]
    fn test_execute_command_echo() {
        let args = json!({ "command": "echo hello" });
        let result = exec_execute_command(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello"));
    }

    #[test]
    fn test_execute_command_failure() {
        let args = json!({ "command": "false" });
        let result = exec_execute_command(&args, ws());
        assert!(result.is_ok()); // still returns Ok with exit code
        assert!(result.unwrap().contains("exit code"));
    }

    // ── execute_tool dispatch ───────────────────────────────────────

    #[test]
    fn test_execute_tool_dispatch() {
        let args = json!({ "path": file!() });
        let result = execute_tool("read_file", &args, ws());
        assert!(result.is_ok());
    }

    #[test]
    fn test_execute_tool_unknown() {
        let result = execute_tool("no_such_tool", &json!({}), ws());
        assert!(result.is_err());
    }

    // ── Provider format tests ───────────────────────────────────────

    #[test]
    fn test_openai_format() {
        let tools = tools_openai();
        assert_eq!(tools.len(), 10);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "read_file");
        assert!(tools[0]["function"]["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_anthropic_format() {
        let tools = tools_anthropic();
        assert_eq!(tools.len(), 10);
        assert_eq!(tools[0]["name"], "read_file");
        assert!(tools[0]["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_google_format() {
        let tools = tools_google();
        assert_eq!(tools.len(), 10);
        assert_eq!(tools[0]["name"], "read_file");
    }

    // ── resolve_path helper ─────────────────────────────────────────

    #[test]
    fn test_resolve_path_absolute() {
        let result = resolve_path(Path::new("/workspace"), "/absolute/path.txt");
        assert_eq!(result, PathBuf::from("/absolute/path.txt"));
    }

    #[test]
    fn test_resolve_path_relative() {
        let result = resolve_path(Path::new("/workspace"), "relative/path.txt");
        assert_eq!(result, PathBuf::from("/workspace/relative/path.txt"));
    }

    // ── web_fetch ───────────────────────────────────────────────────

    #[test]
    fn test_web_fetch_missing_url() {
        let args = json!({});
        let result = exec_web_fetch(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_web_fetch_invalid_url() {
        let args = json!({ "url": "not-a-url" });
        let result = exec_web_fetch(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("http"));
    }

    #[test]
    fn test_web_fetch_params_defined() {
        let params = web_fetch_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "url" && p.required));
        assert!(params.iter().any(|p| p.name == "extract_mode" && !p.required));
        assert!(params.iter().any(|p| p.name == "max_chars" && !p.required));
    }

    // ── web_search ──────────────────────────────────────────────────

    #[test]
    fn test_web_search_missing_query() {
        let args = json!({});
        let result = exec_web_search(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_web_search_no_api_key() {
        // Clear any existing key for the test
        std::env::remove_var("BRAVE_API_KEY");
        let args = json!({ "query": "test" });
        let result = exec_web_search(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("BRAVE_API_KEY"));
    }

    #[test]
    fn test_web_search_params_defined() {
        let params = web_search_params();
        assert_eq!(params.len(), 5);
        assert!(params.iter().any(|p| p.name == "query" && p.required));
        assert!(params.iter().any(|p| p.name == "count" && !p.required));
        assert!(params.iter().any(|p| p.name == "country" && !p.required));
        assert!(params.iter().any(|p| p.name == "search_lang" && !p.required));
        assert!(params.iter().any(|p| p.name == "freshness" && !p.required));
    }

    // ── process ─────────────────────────────────────────────────────

    #[test]
    fn test_process_missing_action() {
        let args = json!({});
        let result = exec_process(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_process_invalid_action() {
        let args = json!({ "action": "invalid" });
        let result = exec_process(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown action"));
    }

    #[test]
    fn test_process_list_empty() {
        let args = json!({ "action": "list" });
        let result = exec_process(&args, ws());
        assert!(result.is_ok());
        // May have sessions from other tests, so just check it doesn't error
    }

    #[test]
    fn test_process_params_defined() {
        let params = process_params();
        assert_eq!(params.len(), 5);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
        assert!(params.iter().any(|p| p.name == "sessionId" && !p.required));
        assert!(params.iter().any(|p| p.name == "data" && !p.required));
        assert!(params.iter().any(|p| p.name == "offset" && !p.required));
        assert!(params.iter().any(|p| p.name == "limit" && !p.required));
    }

    #[test]
    fn test_execute_command_params_with_background() {
        let params = execute_command_params();
        assert_eq!(params.len(), 5);
        assert!(params.iter().any(|p| p.name == "command" && p.required));
        assert!(params.iter().any(|p| p.name == "background" && !p.required));
        assert!(params.iter().any(|p| p.name == "yieldMs" && !p.required));
    }
}
