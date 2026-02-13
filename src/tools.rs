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

// ── Credentials directory protection ────────────────────────────────────────

/// Absolute path of the credentials directory, set once at gateway startup.
static CREDENTIALS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Called once from the gateway to register the credentials path.
pub fn set_credentials_dir(path: PathBuf) {
    let _ = CREDENTIALS_DIR.set(path);
}

/// Returns `true` when `path` falls inside the credentials directory.
pub fn is_protected_path(path: &Path) -> bool {
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        // Canonicalise both so symlinks / ".." can't bypass the check.
        let canon_cred = match cred_dir.canonicalize() {
            Ok(p) => p,
            Err(_) => return false, // dir doesn't exist yet – nothing to protect
        };
        let canon_path = match path.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // File may not exist yet (write_file).  Fall back to
                // starts_with on the raw absolute path.
                return path.starts_with(cred_dir);
            }
        };
        canon_path.starts_with(&canon_cred)
    } else {
        false
    }
}

/// Standard denial message when a tool tries to touch the vault.
const VAULT_ACCESS_DENIED: &str =
    "Access denied: the credentials directory is protected. Use the secrets_list / secrets_get / secrets_store tools instead.";

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
        if matches!(
            name.as_ref(),
            ".git" | "node_modules" | "target" | ".hg" | ".svn"
                | "__pycache__" | "dist" | "build"
        ) {
            return false;
        }
        // Never recurse into the credentials directory.
        if is_protected_path(entry.path()) {
            return false;
        }
        true
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
        &MEMORY_SEARCH,
        &MEMORY_GET,
        &CRON,
        &SESSIONS_LIST,
        &SESSIONS_SPAWN,
        &SESSIONS_SEND,
        &SESSIONS_HISTORY,
        &SESSION_STATUS,
        &AGENTS_LIST,
        &APPLY_PATCH,
        &SECRETS_LIST,
        &SECRETS_GET,
        &SECRETS_STORE,
        &GATEWAY,
        &MESSAGE,
        &TTS,
        &IMAGE,
        &NODES,
        &BROWSER,
        &CANVAS,
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

pub static MEMORY_SEARCH: ToolDef = ToolDef {
    name: "memory_search",
    description: "Semantically search MEMORY.md and memory/*.md files for relevant information. \
                  Use before answering questions about prior work, decisions, dates, people, \
                  preferences, or todos. Returns matching snippets with file path and line numbers.",
    parameters: vec![],
    execute: exec_memory_search,
};

pub static MEMORY_GET: ToolDef = ToolDef {
    name: "memory_get",
    description: "Read content from a memory file (MEMORY.md or memory/*.md). \
                  Use after memory_search to get full context around a snippet. \
                  Supports optional line range for large files.",
    parameters: vec![],
    execute: exec_memory_get,
};

pub static CRON: ToolDef = ToolDef {
    name: "cron",
    description: "Manage scheduled jobs. Actions: status (scheduler status), list (show jobs), \
                  add (create job), update (modify job), remove (delete job), run (trigger immediately), \
                  runs (get run history). Use for reminders and recurring tasks.",
    parameters: vec![],
    execute: exec_cron,
};

pub static SESSIONS_LIST: ToolDef = ToolDef {
    name: "sessions_list",
    description: "List active sessions with optional filters. Shows main sessions and sub-agents. \
                  Use to check on running background tasks.",
    parameters: vec![],
    execute: exec_sessions_list,
};

pub static SESSIONS_SPAWN: ToolDef = ToolDef {
    name: "sessions_spawn",
    description: "Spawn a sub-agent to run a task in the background. The sub-agent runs in its own \
                  isolated session and announces results back when finished. Non-blocking.",
    parameters: vec![],
    execute: exec_sessions_spawn,
};

pub static SESSIONS_SEND: ToolDef = ToolDef {
    name: "sessions_send",
    description: "Send a message to another session. Use sessionKey or label to identify the target. \
                  Returns immediately after sending.",
    parameters: vec![],
    execute: exec_sessions_send,
};

pub static SESSIONS_HISTORY: ToolDef = ToolDef {
    name: "sessions_history",
    description: "Fetch message history for a session. Returns recent messages from the specified session.",
    parameters: vec![],
    execute: exec_sessions_history,
};

pub static SESSION_STATUS: ToolDef = ToolDef {
    name: "session_status",
    description: "Show session status including usage, time, and cost. Use for model-use questions. \
                  Can also set per-session model override.",
    parameters: vec![],
    execute: exec_session_status,
};

pub static AGENTS_LIST: ToolDef = ToolDef {
    name: "agents_list",
    description: "List available agent IDs that can be targeted with sessions_spawn. \
                  Returns the configured agents based on allowlists.",
    parameters: vec![],
    execute: exec_agents_list,
};

pub static APPLY_PATCH: ToolDef = ToolDef {
    name: "apply_patch",
    description: "Apply a unified diff patch to one or more files. Supports multi-hunk patches. \
                  Use for complex multi-line edits where edit_file would be cumbersome.",
    parameters: vec![],
    execute: exec_apply_patch,
};

pub static SECRETS_LIST: ToolDef = ToolDef {
    name: "secrets_list",
    description: "List the names (keys) stored in the encrypted secrets vault. \
                  Returns only key names, never values. Use secrets_get to \
                  retrieve a specific value.",
    parameters: vec![],
    execute: exec_secrets_stub,
};

pub static SECRETS_GET: ToolDef = ToolDef {
    name: "secrets_get",
    description: "Retrieve a secret value from the encrypted vault by key name. \
                  The value is returned as a string. Prefer injecting it directly \
                  into environment variables or config rather than echoing it.",
    parameters: vec![],
    execute: exec_secrets_stub,
};

pub static SECRETS_STORE: ToolDef = ToolDef {
    name: "secrets_store",
    description: "Store or update a key/value pair in the encrypted secrets vault. \
                  The value is encrypted at rest. Use for API keys, tokens, and \
                  other sensitive material.",
    parameters: vec![],
    execute: exec_secrets_stub,
};

pub static GATEWAY: ToolDef = ToolDef {
    name: "gateway",
    description: "Manage the gateway daemon. Actions: restart (restart gateway), \
                  config.get (get current config), config.schema (get config schema), \
                  config.apply (replace entire config), config.patch (partial config update), \
                  update.run (update gateway).",
    parameters: vec![],
    execute: exec_gateway,
};

pub static MESSAGE: ToolDef = ToolDef {
    name: "message",
    description: "Send messages via channel plugins. Actions: send (send a message), \
                  broadcast (send to multiple targets). Supports various channels \
                  like telegram, discord, whatsapp, signal, etc.",
    parameters: vec![],
    execute: exec_message,
};

pub static TTS: ToolDef = ToolDef {
    name: "tts",
    description: "Convert text to speech and return a media path. Use when the user \
                  requests audio or TTS is enabled.",
    parameters: vec![],
    execute: exec_tts,
};

pub static IMAGE: ToolDef = ToolDef {
    name: "image",
    description: "Analyze an image using the configured image/vision model. \
                  Pass a local file path or URL. Returns a text description or \
                  answers the prompt about the image.",
    parameters: vec![],
    execute: exec_image,
};

pub static NODES: ToolDef = ToolDef {
    name: "nodes",
    description: "Discover and control paired nodes (companion devices). Actions: \
                  status (list nodes), describe (node details), pending/approve/reject (pairing), \
                  notify (send notification), camera_snap/camera_list (camera), \
                  screen_record (screen capture), location_get (GPS), run/invoke (remote commands).",
    parameters: vec![],
    execute: exec_nodes,
};

pub static BROWSER: ToolDef = ToolDef {
    name: "browser",
    description: "Control web browser for automation. Actions: status, start, stop, \
                  profiles, tabs, open, focus, close, snapshot, screenshot, navigate, \
                  console, pdf, act (click/type/press/hover/drag). Use snapshot to get \
                  page accessibility tree for element targeting.",
    parameters: vec![],
    execute: exec_browser,
};

pub static CANVAS: ToolDef = ToolDef {
    name: "canvas",
    description: "Control node canvases for UI presentation. Actions: present (show content), \
                  hide, navigate, eval (run JavaScript), snapshot (capture rendered UI), \
                  a2ui_push/a2ui_reset (accessibility-to-UI).",
    parameters: vec![],
    execute: exec_canvas,
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

fn memory_search_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "query".into(),
            description: "Search query for finding relevant memory content.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "maxResults".into(),
            description: "Maximum number of results to return. Default: 5.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "minScore".into(),
            description: "Minimum relevance score threshold (0.0-1.0). Default: 0.1.".into(),
            param_type: "number".into(),
            required: false,
        },
    ]
}

fn memory_get_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Path to the memory file (MEMORY.md or memory/*.md).".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "from".into(),
            description: "Starting line number (1-indexed). Default: 1.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "lines".into(),
            description: "Number of lines to read. Default: entire file.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn secrets_list_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "prefix".into(),
            description: "Optional prefix to filter key names.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn secrets_get_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "key".into(),
            description: "The name of the secret to retrieve.".into(),
            param_type: "string".into(),
            required: true,
        },
    ]
}

fn secrets_store_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "key".into(),
            description: "The name under which to store the secret.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "value".into(),
            description: "The secret value to encrypt and store.".into(),
            param_type: "string".into(),
            required: true,
        },
    ]
}

fn gateway_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'restart', 'config.get', 'config.schema', 'config.apply', 'config.patch', 'update.run'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "raw".into(),
            description: "JSON config content for config.apply or config.patch.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "baseHash".into(),
            description: "Config hash from config.get (required for apply/patch when config exists).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "reason".into(),
            description: "Reason for restart or config change.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "delayMs".into(),
            description: "Delay before restart in milliseconds. Default: 2000.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn message_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'send' or 'broadcast'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "message".into(),
            description: "Message content to send.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "target".into(),
            description: "Target channel/user ID or name.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "channel".into(),
            description: "Channel type: telegram, discord, whatsapp, signal, slack, etc.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "targets".into(),
            description: "Multiple targets for broadcast action.".into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "replyTo".into(),
            description: "Message ID to reply to.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "silent".into(),
            description: "Send without notification. Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

fn tts_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "text".into(),
            description: "Text to convert to speech.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "channel".into(),
            description: "Optional channel ID to pick output format.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn image_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "image".into(),
            description: "Path to local image file or URL.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "prompt".into(),
            description: "Question or instruction about the image. Default: 'Describe the image.'".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn nodes_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'status', 'describe', 'pending', 'approve', 'reject', 'notify', 'camera_snap', 'camera_list', 'screen_record', 'location_get', 'run', 'invoke'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "node".into(),
            description: "Node ID or name to target.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "requestId".into(),
            description: "Pairing request ID for approve/reject.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "title".into(),
            description: "Notification title.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "body".into(),
            description: "Notification body text.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "command".into(),
            description: "Command array for 'run' action.".into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "invokeCommand".into(),
            description: "Command name for 'invoke' action.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "facing".into(),
            description: "Camera facing: 'front', 'back', or 'both'.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn browser_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'status', 'start', 'stop', 'profiles', 'tabs', 'open', 'focus', 'close', 'snapshot', 'screenshot', 'navigate', 'console', 'pdf', 'act'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "profile".into(),
            description: "Browser profile: 'openclaw' (managed) or 'chrome' (extension relay).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "targetUrl".into(),
            description: "URL for 'open' or 'navigate' actions.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "targetId".into(),
            description: "Tab ID for targeting specific tab.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "ref".into(),
            description: "Element reference from snapshot for actions.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "request".into(),
            description: "Action request object with kind (click/type/press/hover/drag), ref, text, etc.".into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "fullPage".into(),
            description: "Capture full page for screenshot. Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

fn canvas_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'present', 'hide', 'navigate', 'eval', 'snapshot', 'a2ui_push', 'a2ui_reset'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "node".into(),
            description: "Target node for canvas operations.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "url".into(),
            description: "URL to present or navigate to.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "javaScript".into(),
            description: "JavaScript code for 'eval' action.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "width".into(),
            description: "Canvas width in pixels.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "height".into(),
            description: "Canvas height in pixels.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

/// Stub executor for secrets tools – always errors.
///
/// Real execution is intercepted by the gateway *before* `execute_tool` is
/// reached.  If we end up here it means something bypassed the gateway
/// interception layer, so we refuse with a clear message.
fn exec_secrets_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Err("Secrets tools must be executed through the gateway layer".to_string())
}

fn cron_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'status', 'list', 'add', 'update', 'remove', 'run', 'runs'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "jobId".into(),
            description: "Job ID for update/remove/run/runs actions.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "job".into(),
            description: "Job definition object for 'add' action.".into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "patch".into(),
            description: "Patch object for 'update' action.".into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "includeDisabled".into(),
            description: "Include disabled jobs in list. Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

fn sessions_list_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "kinds".into(),
            description: "Filter by session kinds: 'main', 'subagent', 'cron'.".into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "activeMinutes".into(),
            description: "Only show sessions active within N minutes.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "limit".into(),
            description: "Maximum sessions to return. Default: 20.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "messageLimit".into(),
            description: "Include last N messages per session.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn sessions_spawn_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "task".into(),
            description: "What the sub-agent should do (required).".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "label".into(),
            description: "Short label for identification.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "agentId".into(),
            description: "Spawn under a different agent ID.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "model".into(),
            description: "Override the model for this sub-agent.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "thinking".into(),
            description: "Override thinking level (off/low/medium/high).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "runTimeoutSeconds".into(),
            description: "Abort sub-agent after N seconds.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "cleanup".into(),
            description: "'delete' or 'keep' session after completion.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn sessions_send_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "message".into(),
            description: "Message to send to the target session.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "sessionKey".into(),
            description: "Session key to send to.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "label".into(),
            description: "Session label to send to (alternative to sessionKey).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "timeoutSeconds".into(),
            description: "Timeout for waiting on response.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

fn sessions_history_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "sessionKey".into(),
            description: "Session key to get history for.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "limit".into(),
            description: "Maximum messages to return. Default: 20.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "includeTools".into(),
            description: "Include tool call messages. Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

fn session_status_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "sessionKey".into(),
            description: "Session key to get status for. Default: current session.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "model".into(),
            description: "Set per-session model override. Use 'default' to reset.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

fn agents_list_params() -> Vec<ToolParam> {
    // No parameters needed
    vec![]
}

fn apply_patch_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "patch".into(),
            description: "Unified diff patch content to apply.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "path".into(),
            description: "Target file path. If not specified, parsed from patch header.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "dry_run".into(),
            description: "If true, validate patch without applying. Default: false.".into(),
            param_type: "boolean".into(),
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

    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

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

    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

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

    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

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

    if is_protected_path(&path) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

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

    // Block commands that reference the credentials directory.
    if let Some(cred_dir) = CREDENTIALS_DIR.get() {
        let cred_str = cred_dir.to_string_lossy();
        if command.contains(cred_str.as_ref()) {
            return Err(VAULT_ACCESS_DENIED.to_string());
        }
    }
    if is_protected_path(&cwd) {
        return Err(VAULT_ACCESS_DENIED.to_string());
    }

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

            let new_output = session.poll_output().to_string();
            let status_str = match &session.status {
                SessionStatus::Running => "running".to_string(),
                SessionStatus::Exited(code) => format!("exited ({})", code),
                SessionStatus::Killed => "killed".to_string(),
                SessionStatus::TimedOut => "timed out".to_string(),
            };

            let mut result = String::new();
            if !new_output.is_empty() {
                result.push_str(&new_output);
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

/// Search memory files for relevant content.
fn exec_memory_search(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    let max_results = args
        .get("maxResults")
        .and_then(|v| v.as_u64())
        .unwrap_or(5) as usize;

    let min_score = args
        .get("minScore")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.1);

    // Build index and search
    let index = crate::memory::MemoryIndex::index_workspace(workspace_dir)?;
    let results = index.search(query, max_results);

    if results.is_empty() {
        return Ok("No matching memories found.".to_string());
    }

    // Filter by minimum score and format results
    let mut output = String::new();
    output.push_str(&format!("Memory search results for: {}\n\n", query));

    let mut count = 0;
    for result in results {
        if result.score < min_score {
            continue;
        }
        count += 1;

        // Truncate snippet to ~700 chars
        let snippet = if result.chunk.text.len() > 700 {
            format!("{}...", &result.chunk.text[..700])
        } else {
            result.chunk.text.clone()
        };

        output.push_str(&format!(
            "{}. **{}** (lines {}-{}, score: {:.2})\n",
            count,
            result.chunk.path,
            result.chunk.start_line,
            result.chunk.end_line,
            result.score
        ));
        output.push_str(&format!("{}\n\n", snippet));
        output.push_str(&format!(
            "Source: {}#L{}-L{}\n\n",
            result.chunk.path, result.chunk.start_line, result.chunk.end_line
        ));
    }

    if count == 0 {
        return Ok("No matching memories found above the minimum score threshold.".to_string());
    }

    Ok(output)
}

/// Read content from a memory file.
fn exec_memory_get(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: path".to_string())?;

    let from_line = args
        .get("from")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    let num_lines = args
        .get("lines")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize);

    crate::memory::read_memory_file(workspace_dir, path, from_line, num_lines)
}

/// Cron job management.
fn exec_cron(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    use crate::cron::*;
    
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let cron_dir = workspace_dir.join(".cron");
    let mut store = CronStore::new(&cron_dir)?;

    match action {
        "status" => {
            let jobs = store.list(false);
            let enabled_count = jobs.len();
            let all_count = store.list(true).len();
            Ok(format!(
                "Cron scheduler status:\n- Enabled jobs: {}\n- Total jobs: {}\n- Store: {:?}",
                enabled_count, all_count, cron_dir
            ))
        }

        "list" => {
            let include_disabled = args
                .get("includeDisabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let jobs = store.list(include_disabled);
            if jobs.is_empty() {
                return Ok("No cron jobs configured.".to_string());
            }

            let mut output = String::from("Cron jobs:\n\n");
            for job in jobs {
                let status = if job.enabled { "✓" } else { "○" };
                let name = job.name.as_deref().unwrap_or("(unnamed)");
                let schedule = match &job.schedule {
                    Schedule::At { at } => format!("at {}", at),
                    Schedule::Every { every_ms, .. } => format!("every {}ms", every_ms),
                    Schedule::Cron { expr, tz } => {
                        format!("cron '{}'{}", expr, tz.as_ref().map(|t| format!(" ({})", t)).unwrap_or_default())
                    }
                };
                output.push_str(&format!("{} {} [{}] — {}\n", status, job.job_id, name, schedule));
            }
            Ok(output)
        }

        "add" => {
            let job_obj = args
                .get("job")
                .ok_or("Missing required parameter: job")?;

            let job: CronJob = serde_json::from_value(job_obj.clone())
                .map_err(|e| format!("Invalid job definition: {}", e))?;

            let id = store.add(job)?;
            Ok(format!("Created job: {}", id))
        }

        "update" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for update")?;

            let patch_obj = args
                .get("patch")
                .ok_or("Missing patch for update")?;

            let patch: CronJobPatch = serde_json::from_value(patch_obj.clone())
                .map_err(|e| format!("Invalid patch: {}", e))?;

            store.update(job_id, patch)?;
            Ok(format!("Updated job: {}", job_id))
        }

        "remove" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for remove")?;

            store.remove(job_id)?;
            Ok(format!("Removed job: {}", job_id))
        }

        "run" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for run")?;

            let job = store.get(job_id)
                .ok_or_else(|| format!("Job not found: {}", job_id))?;

            // In a real implementation, this would execute the job
            // For now, just record that we would run it
            Ok(format!(
                "Would run job '{}' ({}). Note: actual execution requires gateway integration.",
                job.name.as_deref().unwrap_or("unnamed"),
                job_id
            ))
        }

        "runs" => {
            let job_id = args
                .get("jobId")
                .and_then(|v| v.as_str())
                .ok_or("Missing jobId for runs")?;

            let runs = store.get_runs(job_id, 10)?;
            if runs.is_empty() {
                return Ok(format!("No run history for job: {}", job_id));
            }

            let mut output = format!("Run history for {}:\n\n", job_id);
            for run in runs {
                let status = match run.status {
                    RunStatus::Ok => "✓",
                    RunStatus::Error => "✗",
                    RunStatus::Running => "⟳",
                    RunStatus::Timeout => "⏱",
                    RunStatus::Skipped => "○",
                };
                output.push_str(&format!("{} {} — {:?}\n", status, run.run_id, run.status));
            }
            Ok(output)
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: status, list, add, update, remove, run, runs",
            action
        )),
    }
}

/// List sessions.
fn exec_sessions_list(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let sessions = mgr.list(None, false, limit);

    if sessions.is_empty() {
        return Ok("No active sessions.".to_string());
    }

    let mut output = String::from("Sessions:\n\n");
    for session in sessions {
        let kind = match session.kind {
            SessionKind::Main => "main",
            SessionKind::Subagent => "subagent",
            SessionKind::Cron => "cron",
        };
        let status = match session.status {
            SessionStatus::Active => "🔄",
            SessionStatus::Completed => "✅",
            SessionStatus::Error => "❌",
            SessionStatus::Timeout => "⏱",
            SessionStatus::Stopped => "⏹",
        };
        let label = session.label.as_deref().unwrap_or("");
        let runtime = session.runtime_secs();

        output.push_str(&format!(
            "{} [{}] {} — {}s{}\n",
            status,
            kind,
            session.key,
            runtime,
            if label.is_empty() { String::new() } else { format!(" ({})", label) }
        ));
    }

    Ok(output)
}

/// Spawn a sub-agent.
fn exec_sessions_spawn(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: task".to_string())?;

    let label = args.get("label").and_then(|v| v.as_str()).map(String::from);
    let agent_id = args
        .get("agentId")
        .and_then(|v| v.as_str())
        .unwrap_or("main");

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let session_key = mgr.spawn_subagent(agent_id, task, label.clone(), None);

    // Get the run_id
    let run_id = mgr
        .get(&session_key)
        .and_then(|s| s.run_id.clone())
        .unwrap_or_default();

    let result = SpawnResult {
        status: "accepted".to_string(),
        run_id: run_id.clone(),
        session_key: session_key.clone(),
        message: format!(
            "Sub-agent spawned. Task: '{}'. Use sessions_history or sessions_send to interact.",
            task
        ),
    };

    serde_json::to_string_pretty(&result)
        .map_err(|e| format!("Failed to serialize result: {}", e))
}

/// Send a message to a session.
fn exec_sessions_send(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let message = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: message".to_string())?;

    let session_key = args.get("sessionKey").and_then(|v| v.as_str());
    let label = args.get("label").and_then(|v| v.as_str());

    let manager = session_manager();
    let mut mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    // Find session by key or label
    let key = if let Some(k) = session_key {
        k.to_string()
    } else if let Some(l) = label {
        mgr.get_by_label(l)
            .map(|s| s.key.clone())
            .ok_or_else(|| format!("No session found with label: {}", l))?
    } else {
        return Err("Must provide sessionKey or label".to_string());
    };

    mgr.send_message(&key, message)?;

    Ok(format!("Message sent to session: {}", key))
}

/// Get session history.
fn exec_sessions_history(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;

    let session_key = args
        .get("sessionKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: sessionKey".to_string())?;

    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;

    let include_tools = args
        .get("includeTools")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let history = mgr
        .history(session_key, limit, include_tools)
        .ok_or_else(|| format!("Session not found: {}", session_key))?;

    if history.is_empty() {
        return Ok(format!("No messages in session: {}", session_key));
    }

    let mut output = format!("History for {}:\n\n", session_key);
    for msg in history {
        output.push_str(&format!("[{}] {}\n", msg.role, msg.content));
    }

    Ok(output)
}

/// Get session status.
fn exec_session_status(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    use crate::sessions::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Get current time info
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let session_key = args.get("sessionKey").and_then(|v| v.as_str());

    let manager = session_manager();
    let mgr = manager
        .lock()
        .map_err(|_| "Failed to acquire session manager lock".to_string())?;

    let mut output = String::from("📊 Session Status\n\n");

    if let Some(key) = session_key {
        if let Some(session) = mgr.get(key) {
            output.push_str(&format!("Session: {}\n", session.key));
            output.push_str(&format!("Agent: {}\n", session.agent_id));
            output.push_str(&format!("Kind: {:?}\n", session.kind));
            output.push_str(&format!("Status: {:?}\n", session.status));
            output.push_str(&format!("Runtime: {}s\n", session.runtime_secs()));
            output.push_str(&format!("Messages: {}\n", session.messages.len()));
        } else {
            return Err(format!("Session not found: {}", key));
        }
    } else {
        // Show general status
        let all_sessions = mgr.list(None, false, 100);
        let active = all_sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Active)
            .count();

        output.push_str(&format!("Active sessions: {}\n", active));
        output.push_str(&format!("Total sessions: {}\n", all_sessions.len()));
        output.push_str(&format!("Timestamp: {} ms\n", now.as_millis()));
    }

    Ok(output)
}

/// List available agent IDs.
fn exec_agents_list(_args: &Value, workspace_dir: &Path) -> Result<String, String> {
    // In a full implementation, this would read from config
    // For now, return a simple list based on workspace structure
    
    let mut agents = vec!["main".to_string()];
    
    // Check for agents directory
    let agents_dir = workspace_dir.join("agents");
    if agents_dir.exists() && agents_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.') && name != "main" {
                            agents.push(name.to_string());
                        }
                    }
                }
            }
        }
    }
    
    let mut output = String::from("Available agents for sessions_spawn:\n\n");
    for agent in &agents {
        output.push_str(&format!("- {}\n", agent));
    }
    
    Ok(output)
}

/// Apply a unified diff patch to files.
fn exec_apply_patch(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let patch_content = args
        .get("patch")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: patch".to_string())?;

    let explicit_path = args.get("path").and_then(|v| v.as_str());
    let dry_run = args.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(false);

    // Parse the patch
    let hunks = parse_unified_diff(patch_content)?;
    
    if hunks.is_empty() {
        return Err("No valid hunks found in patch".to_string());
    }

    let mut results = Vec::new();
    
    // Group hunks by file
    let mut files: std::collections::HashMap<String, Vec<&DiffHunk>> = std::collections::HashMap::new();
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
            results.push(format!("✓ {} (dry run, {} hunks valid)", file_path, file_hunks.len()));
        } else {
            // Ensure parent directory exists
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
            
            std::fs::write(&full_path, new_content)
                .map_err(|e| format!("Failed to write {}: {}", file_path, e))?;
            
            results.push(format!("✓ {} ({} hunks applied)", file_path, file_hunks.len()));
        }
    }

    Ok(results.join("\n"))
}

/// A single hunk from a unified diff.
#[derive(Debug)]
struct DiffHunk {
    file_path: String,
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<DiffLine>,
}

#[derive(Debug)]
enum DiffLine {
    Context(String),
    Remove(String),
    Add(String),
}

/// Parse a unified diff into hunks.
fn parse_unified_diff(patch: &str) -> Result<Vec<DiffHunk>, String> {
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
        if line.starts_with("@@ ") {
            let Some(ref file_path) = current_file else {
                return Err("Hunk without file header".to_string());
            };
            
            let header = &line[3..];
            let end = header.find(" @@").unwrap_or(header.len());
            let range_part = &header[..end];
            
            let (old_range, new_range) = range_part
                .split_once(' ')
                .ok_or("Invalid hunk header")?;
            
            let (old_start, old_count) = parse_range(old_range.trim_start_matches('-'))?;
            let (new_start, new_count) = parse_range(new_range.trim_start_matches('+'))?;
            
            // Read hunk lines
            let mut hunk_lines = Vec::new();
            while let Some(next_line) = lines.peek() {
                if next_line.starts_with("@@") || next_line.starts_with("---") || next_line.starts_with("+++") {
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
                // Verify context matches
                if old_idx < lines.len() && lines[old_idx] != *text {
                    // Context mismatch - try fuzzy match
                    // For now, just warn but continue
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

/// Gateway management.
fn exec_gateway(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let config_path = workspace_dir
        .parent()
        .unwrap_or(workspace_dir)
        .join("openclaw.json");

    match action {
        "restart" => {
            let reason = args
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("Restart requested via gateway tool");
            
            Ok(format!(
                "Gateway restart requested.\nReason: {}\nNote: Actual restart requires daemon integration.",
                reason
            ))
        }

        "config.get" => {
            if !config_path.exists() {
                return Ok(serde_json::json!({
                    "config": {},
                    "hash": "",
                    "exists": false
                }).to_string());
            }

            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| format!("Failed to read config: {}", e))?;
            
            let hash = format!("{:x}", content.len() * 31 + content.bytes().map(|b| b as usize).sum::<usize>());
            
            Ok(serde_json::json!({
                "config": content,
                "hash": hash,
                "exists": true,
                "path": config_path.display().to_string()
            }).to_string())
        }

        "config.schema" => {
            Ok(serde_json::json!({
                "type": "object",
                "properties": {
                    "agents": { "type": "object", "description": "Agent configuration" },
                    "channels": { "type": "object", "description": "Channel plugins" },
                    "session": { "type": "object", "description": "Session settings" },
                    "messages": { "type": "object", "description": "Message formatting" },
                    "providers": { "type": "object", "description": "AI providers" }
                }
            }).to_string())
        }

        "config.apply" => {
            let raw = args
                .get("raw")
                .and_then(|v| v.as_str())
                .ok_or("Missing raw config for config.apply")?;

            let _: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| format!("Invalid JSON config: {}", e))?;

            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }
            
            std::fs::write(&config_path, raw)
                .map_err(|e| format!("Failed to write config: {}", e))?;

            Ok(format!(
                "Config written to {}. Gateway restart required for changes to take effect.",
                config_path.display()
            ))
        }

        "config.patch" => {
            let raw = args
                .get("raw")
                .and_then(|v| v.as_str())
                .ok_or("Missing raw patch for config.patch")?;

            let patch: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| format!("Invalid JSON patch: {}", e))?;

            let existing = if config_path.exists() {
                let content = std::fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to read config: {}", e))?;
                serde_json::from_str(&content)
                    .map_err(|e| format!("Failed to parse existing config: {}", e))?
            } else {
                serde_json::json!({})
            };

            let merged = merge_json(existing, patch);

            let output = serde_json::to_string_pretty(&merged)
                .map_err(|e| format!("Failed to serialize config: {}", e))?;

            std::fs::write(&config_path, &output)
                .map_err(|e| format!("Failed to write config: {}", e))?;

            Ok(format!(
                "Config patched at {}. Gateway restart required for changes to take effect.",
                config_path.display()
            ))
        }

        "update.run" => {
            Ok("Update check requested. Note: Self-update requires external tooling (npm/cargo).".to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: restart, config.get, config.schema, config.apply, config.patch, update.run",
            action
        )),
    }
}

/// Recursively merge two JSON values (patch semantics).
fn merge_json(base: Value, patch: Value) -> Value {
    match (base, patch) {
        (Value::Object(mut base_map), Value::Object(patch_map)) => {
            for (key, patch_val) in patch_map {
                if patch_val.is_null() {
                    base_map.remove(&key);
                } else if let Some(base_val) = base_map.remove(&key) {
                    base_map.insert(key, merge_json(base_val, patch_val));
                } else {
                    base_map.insert(key, patch_val);
                }
            }
            Value::Object(base_map)
        }
        (_, patch) => patch,
    }
}

/// Send messages via channel plugins.
fn exec_message(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    match action {
        "send" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing message for send action")?;

            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .ok_or("Missing target for send action")?;

            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("default");

            Ok(format!(
                "Message queued for delivery:\n- Channel: {}\n- Target: {}\n- Message: {} chars\nNote: Actual delivery requires messenger integration.",
                channel,
                target,
                message.len()
            ))
        }

        "broadcast" => {
            let message = args
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or("Missing message for broadcast action")?;

            let targets = args
                .get("targets")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();

            if targets.is_empty() {
                return Err("No targets specified for broadcast".to_string());
            }

            Ok(format!(
                "Broadcast queued:\n- Targets: {}\n- Message: {} chars\nNote: Actual delivery requires messenger integration.",
                targets.join(", "),
                message.len()
            ))
        }

        _ => Err(format!("Unknown action: {}. Valid: send, broadcast", action)),
    }
}

/// Text-to-speech conversion.
fn exec_tts(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: text".to_string())?;

    let output_path = workspace_dir.join(".tts").join(format!(
        "speech_{}.mp3",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));

    Ok(format!(
        "TTS conversion requested:\n- Text: {} chars\n- Output would be: {}\nNote: Actual TTS requires external service (ElevenLabs, etc.).\n\nMEDIA: {}",
        text.len(),
        output_path.display(),
        output_path.display()
    ))
}

/// Analyze an image using a vision model.
fn exec_image(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let image_path = args
        .get("image")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: image".to_string())?;

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("Describe the image.");

    // Check if it's a URL or local path
    let is_url = image_path.starts_with("http://") || image_path.starts_with("https://");
    
    if !is_url {
        // Resolve local path
        let full_path = resolve_path(workspace_dir, image_path);
        if !full_path.exists() {
            return Err(format!("Image file not found: {}", image_path));
        }
        
        // Check it's actually an image
        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        
        let valid_exts = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg"];
        if !valid_exts.contains(&ext.as_str()) {
            return Err(format!(
                "Unsupported image format: {}. Supported: {}",
                ext,
                valid_exts.join(", ")
            ));
        }
    }

    // In a real implementation, this would:
    // 1. Load the image (from file or URL)
    // 2. Send to configured vision model (GPT-4V, Claude, Gemini, etc.)
    // 3. Return the model's response
    
    Ok(format!(
        "Image analysis requested:\n- Image: {}\n- Prompt: {}\n- Is URL: {}\n\nNote: Actual image analysis requires vision model integration (GPT-4V, Claude 3, Gemini Pro Vision, etc.).",
        image_path,
        prompt,
        is_url
    ))
}

/// Discover and control paired nodes.
fn exec_nodes(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let node = args.get("node").and_then(|v| v.as_str());

    match action {
        "status" => {
            // In real implementation, query gateway for connected nodes
            Ok("Node status:\n\nNo nodes currently paired.\n\nTo pair a node:\n1. Run `openclaw node run` on the target device\n2. Use `nodes` with action='pending' to see pairing requests\n3. Use `nodes` with action='approve' to approve".to_string())
        }

        "describe" => {
            let node_id = node.ok_or("Missing 'node' parameter for describe action")?;
            Ok(format!(
                "Node description requested for: {}\n\nNote: Requires gateway integration to fetch node details.",
                node_id
            ))
        }

        "pending" => {
            Ok("Pending pairing requests:\n\nNo pending requests.\n\nNote: Requires gateway integration.".to_string())
        }

        "approve" => {
            let request_id = args
                .get("requestId")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'requestId' for approve action")?;
            Ok(format!("Would approve pairing request: {}\n\nNote: Requires gateway integration.", request_id))
        }

        "reject" => {
            let request_id = args
                .get("requestId")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'requestId' for reject action")?;
            Ok(format!("Would reject pairing request: {}\n\nNote: Requires gateway integration.", request_id))
        }

        "notify" => {
            let node_id = node.ok_or("Missing 'node' parameter for notify action")?;
            let title = args.get("title").and_then(|v| v.as_str()).unwrap_or("Notification");
            let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
            
            Ok(format!(
                "Notification queued:\n- Node: {}\n- Title: {}\n- Body: {}\n\nNote: Requires node connection.",
                node_id, title, body
            ))
        }

        "camera_snap" => {
            let node_id = node.ok_or("Missing 'node' parameter for camera_snap")?;
            let facing = args.get("facing").and_then(|v| v.as_str()).unwrap_or("back");
            
            Ok(format!(
                "Camera snapshot requested:\n- Node: {}\n- Facing: {}\n\nNote: Requires paired node with camera access.",
                node_id, facing
            ))
        }

        "camera_list" => {
            let node_id = node.ok_or("Missing 'node' parameter for camera_list")?;
            Ok(format!(
                "Camera list requested for node: {}\n\nNote: Requires paired node.",
                node_id
            ))
        }

        "screen_record" => {
            let node_id = node.ok_or("Missing 'node' parameter for screen_record")?;
            Ok(format!(
                "Screen recording requested for node: {}\n\nNote: Requires paired node with screen recording permission.",
                node_id
            ))
        }

        "location_get" => {
            let node_id = node.ok_or("Missing 'node' parameter for location_get")?;
            Ok(format!(
                "Location requested for node: {}\n\nNote: Requires paired node with location permission.",
                node_id
            ))
        }

        "run" => {
            let node_id = node.ok_or("Missing 'node' parameter for run action")?;
            let command = args
                .get("command")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            
            if command.is_empty() {
                return Err("Missing 'command' array for run action".to_string());
            }
            
            Ok(format!(
                "Remote command requested:\n- Node: {}\n- Command: {}\n\nNote: Requires paired node host.",
                node_id,
                command.join(" ")
            ))
        }

        "invoke" => {
            let node_id = node.ok_or("Missing 'node' parameter for invoke action")?;
            let invoke_cmd = args
                .get("invokeCommand")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'invokeCommand' for invoke action")?;
            
            Ok(format!(
                "Node invoke requested:\n- Node: {}\n- Command: {}\n\nNote: Requires paired node.",
                node_id, invoke_cmd
            ))
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: status, describe, pending, approve, reject, notify, camera_snap, camera_list, screen_record, location_get, run, invoke",
            action
        )),
    }
}

/// Browser automation control.
fn exec_browser(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let profile = args.get("profile").and_then(|v| v.as_str()).unwrap_or("openclaw");

    match action {
        "status" => {
            Ok(format!(
                "Browser status:\n- Profile: {}\n- Status: Not running\n\nNote: Browser control requires Playwright/CDP integration.",
                profile
            ))
        }

        "start" => {
            Ok(format!(
                "Would start browser with profile: {}\n\nNote: Requires Playwright/CDP integration.",
                profile
            ))
        }

        "stop" => {
            Ok(format!(
                "Would stop browser profile: {}\n\nNote: Requires Playwright/CDP integration.",
                profile
            ))
        }

        "profiles" => {
            Ok("Available browser profiles:\n- openclaw (managed, isolated)\n- chrome (extension relay)\n\nNote: Requires browser integration.".to_string())
        }

        "tabs" => {
            Ok(format!(
                "Would list tabs for profile: {}\n\nNote: Requires browser integration.",
                profile
            ))
        }

        "open" => {
            let url = args
                .get("targetUrl")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'targetUrl' for open action")?;
            Ok(format!(
                "Would open URL: {}\n- Profile: {}\n\nNote: Requires browser integration.",
                url, profile
            ))
        }

        "focus" | "close" => {
            let tab_id = args
                .get("targetId")
                .and_then(|v| v.as_str())
                .ok_or(format!("Missing 'targetId' for {} action", action))?;
            Ok(format!(
                "Would {} tab: {}\n\nNote: Requires browser integration.",
                action, tab_id
            ))
        }

        "snapshot" => {
            Ok(format!(
                "Would capture accessibility snapshot for profile: {}\n\nReturns ARIA tree with element refs for targeting.\nNote: Requires browser integration.",
                profile
            ))
        }

        "screenshot" => {
            let full_page = args.get("fullPage").and_then(|v| v.as_bool()).unwrap_or(false);
            Ok(format!(
                "Would capture screenshot:\n- Profile: {}\n- Full page: {}\n\nNote: Requires browser integration.",
                profile, full_page
            ))
        }

        "navigate" => {
            let url = args
                .get("targetUrl")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'targetUrl' for navigate action")?;
            Ok(format!(
                "Would navigate to: {}\n\nNote: Requires browser integration.",
                url
            ))
        }

        "console" => {
            Ok("Would fetch browser console logs.\n\nNote: Requires browser integration.".to_string())
        }

        "pdf" => {
            Ok("Would generate PDF from current page.\n\nNote: Requires browser integration.".to_string())
        }

        "act" => {
            let request = args.get("request");
            if let Some(req) = request {
                let kind = req.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown");
                let element_ref = req.get("ref").and_then(|v| v.as_str()).unwrap_or("none");
                Ok(format!(
                    "Would perform action:\n- Kind: {}\n- Element ref: {}\n\nNote: Requires browser integration.",
                    kind, element_ref
                ))
            } else {
                Err("Missing 'request' object for act action".to_string())
            }
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: status, start, stop, profiles, tabs, open, focus, close, snapshot, screenshot, navigate, console, pdf, act",
            action
        )),
    }
}

/// Canvas control for UI presentation.
fn exec_canvas(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    let node = args.get("node").and_then(|v| v.as_str());

    match action {
        "present" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for present action")?;
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800);
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600);
            
            Ok(format!(
                "Would present canvas:\n- URL: {}\n- Size: {}x{}\n- Node: {}\n\nNote: Requires canvas integration.",
                url, width, height, node.unwrap_or("default")
            ))
        }

        "hide" => {
            Ok(format!(
                "Would hide canvas on node: {}\n\nNote: Requires canvas integration.",
                node.unwrap_or("default")
            ))
        }

        "navigate" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for navigate action")?;
            Ok(format!(
                "Would navigate canvas to: {}\n\nNote: Requires canvas integration.",
                url
            ))
        }

        "eval" => {
            let js = args
                .get("javaScript")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'javaScript' for eval action")?;
            Ok(format!(
                "Would evaluate JavaScript ({} chars):\n{}\n\nNote: Requires canvas integration.",
                js.len(),
                if js.len() > 100 { &js[..100] } else { js }
            ))
        }

        "snapshot" => {
            Ok(format!(
                "Would capture canvas snapshot on node: {}\n\nNote: Requires canvas integration.",
                node.unwrap_or("default")
            ))
        }

        "a2ui_push" => {
            Ok("Would push A2UI (accessibility-to-UI) update.\n\nNote: Requires canvas integration.".to_string())
        }

        "a2ui_reset" => {
            Ok("Would reset A2UI state.\n\nNote: Requires canvas integration.".to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: present, hide, navigate, eval, snapshot, a2ui_push, a2ui_reset",
            action
        )),
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
        
        // Arrays need an items schema
        if p.param_type == "array" {
            prop.insert("items".into(), json!({"type": "string"}));
        }
        
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
        "memory_search" => memory_search_params(),
        "memory_get" => memory_get_params(),
        "cron" => cron_params(),
        "sessions_list" => sessions_list_params(),
        "sessions_spawn" => sessions_spawn_params(),
        "sessions_send" => sessions_send_params(),
        "sessions_history" => sessions_history_params(),
        "session_status" => session_status_params(),
        "agents_list" => agents_list_params(),
        "apply_patch" => apply_patch_params(),
        "secrets_list" => secrets_list_params(),
        "secrets_get" => secrets_get_params(),
        "secrets_store" => secrets_store_params(),
        "gateway" => gateway_params(),
        "message" => message_params(),
        "tts" => tts_params(),
        "image" => image_params(),
        "nodes" => nodes_params(),
        "browser" => browser_params(),
        "canvas" => canvas_params(),
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

/// Returns `true` for tools that must be routed through the gateway
/// (i.e. handled by `execute_secrets_tool`) rather than `execute_tool`.
pub fn is_secrets_tool(name: &str) -> bool {
    matches!(name, "secrets_list" | "secrets_get" | "secrets_store")
}

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
        assert_eq!(tools.len(), 30);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "read_file");
        assert!(tools[0]["function"]["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_anthropic_format() {
        let tools = tools_anthropic();
        assert_eq!(tools.len(), 30);
        assert_eq!(tools[0]["name"], "read_file");
        assert!(tools[0]["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_google_format() {
        let tools = tools_google();
        assert_eq!(tools.len(), 30);
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
        // SAFETY: This test is single-threaded and no other thread reads BRAVE_API_KEY.
        unsafe { std::env::remove_var("BRAVE_API_KEY") };
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

    // ── memory_search ───────────────────────────────────────────────

    #[test]
    fn test_memory_search_params_defined() {
        let params = memory_search_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "query" && p.required));
        assert!(params.iter().any(|p| p.name == "maxResults" && !p.required));
        assert!(params.iter().any(|p| p.name == "minScore" && !p.required));
    }

    #[test]
    fn test_memory_search_missing_query() {
        let args = json!({});
        let result = exec_memory_search(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    // ── memory_get ──────────────────────────────────────────────────

    #[test]
    fn test_memory_get_params_defined() {
        let params = memory_get_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "path" && p.required));
        assert!(params.iter().any(|p| p.name == "from" && !p.required));
        assert!(params.iter().any(|p| p.name == "lines" && !p.required));
    }

    #[test]
    fn test_memory_get_missing_path() {
        let args = json!({});
        let result = exec_memory_get(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_memory_get_invalid_path() {
        let args = json!({ "path": "../etc/passwd" });
        let result = exec_memory_get(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a valid memory file"));
    }

    // ── cron ────────────────────────────────────────────────────────

    #[test]
    fn test_cron_params_defined() {
        let params = cron_params();
        assert_eq!(params.len(), 5);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
        assert!(params.iter().any(|p| p.name == "jobId" && !p.required));
    }

    #[test]
    fn test_cron_missing_action() {
        let args = json!({});
        let result = exec_cron(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_cron_invalid_action() {
        let args = json!({ "action": "invalid" });
        let result = exec_cron(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown action"));
    }

    // ── sessions_list ───────────────────────────────────────────────

    #[test]
    fn test_sessions_list_params_defined() {
        let params = sessions_list_params();
        assert_eq!(params.len(), 4);
        assert!(params.iter().all(|p| !p.required));
    }

    // ── sessions_spawn ──────────────────────────────────────────────

    #[test]
    fn test_sessions_spawn_params_defined() {
        let params = sessions_spawn_params();
        assert_eq!(params.len(), 7);
        assert!(params.iter().any(|p| p.name == "task" && p.required));
    }

    #[test]
    fn test_sessions_spawn_missing_task() {
        let args = json!({});
        let result = exec_sessions_spawn(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    // ── sessions_send ───────────────────────────────────────────────

    #[test]
    fn test_sessions_send_params_defined() {
        let params = sessions_send_params();
        assert_eq!(params.len(), 4);
        assert!(params.iter().any(|p| p.name == "message" && p.required));
    }

    #[test]
    fn test_sessions_send_missing_message() {
        let args = json!({});
        let result = exec_sessions_send(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    // ── sessions_history ────────────────────────────────────────────

    #[test]
    fn test_sessions_history_params_defined() {
        let params = sessions_history_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "sessionKey" && p.required));
    }

    // ── session_status ──────────────────────────────────────────────

    #[test]
    fn test_session_status_params_defined() {
        let params = session_status_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| !p.required));
    }

    #[test]
    fn test_session_status_general() {
        let args = json!({});
        let result = exec_session_status(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Session Status"));
    }

    // ── agents_list ─────────────────────────────────────────────────

    #[test]
    fn test_agents_list_params_defined() {
        let params = agents_list_params();
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_agents_list_returns_main() {
        let args = json!({});
        let result = exec_agents_list(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("main"));
    }

    // ── apply_patch ─────────────────────────────────────────────────

    #[test]
    fn test_apply_patch_params_defined() {
        let params = apply_patch_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "patch" && p.required));
        assert!(params.iter().any(|p| p.name == "dry_run" && !p.required));
    }

    #[test]
    fn test_apply_patch_missing_patch() {
        let args = json!({});
        let result = exec_apply_patch(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_parse_unified_diff() {
        let patch = r#"--- a/test.txt
+++ b/test.txt
@@ -1,3 +1,4 @@
 line1
+new line
 line2
 line3
"#;
        let hunks = parse_unified_diff(patch).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].file_path, "test.txt");
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].old_count, 3);
    }

    // ── secrets tools ───────────────────────────────────────────────

    #[test]
    fn test_secrets_stub_rejects() {
        let args = json!({});
        let result = exec_secrets_stub(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("gateway"));
    }

    #[test]
    fn test_is_secrets_tool() {
        assert!(is_secrets_tool("secrets_list"));
        assert!(is_secrets_tool("secrets_get"));
        assert!(is_secrets_tool("secrets_store"));
        assert!(!is_secrets_tool("read_file"));
        assert!(!is_secrets_tool("memory_get"));
    }

    #[test]
    fn test_secrets_list_params_defined() {
        let params = secrets_list_params();
        assert_eq!(params.len(), 1);
        assert!(params.iter().any(|p| p.name == "prefix" && !p.required));
    }

    #[test]
    fn test_secrets_get_params_defined() {
        let params = secrets_get_params();
        assert_eq!(params.len(), 1);
        assert!(params.iter().any(|p| p.name == "key" && p.required));
    }

    #[test]
    fn test_secrets_store_params_defined() {
        let params = secrets_store_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "key" && p.required));
        assert!(params.iter().any(|p| p.name == "value" && p.required));
    }

    #[test]
    fn test_protected_path_without_init() {
        // Before set_credentials_dir is called, nothing is protected.
        assert!(!is_protected_path(Path::new("/some/random/path")));
    }

    // ── gateway ─────────────────────────────────────────────────────

    #[test]
    fn test_gateway_params_defined() {
        let params = gateway_params();
        assert_eq!(params.len(), 5);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
    }

    #[test]
    fn test_gateway_missing_action() {
        let args = json!({});
        let result = exec_gateway(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_gateway_config_schema() {
        let args = json!({ "action": "config.schema" });
        let result = exec_gateway(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("properties"));
    }

    // ── message ─────────────────────────────────────────────────────

    #[test]
    fn test_message_params_defined() {
        let params = message_params();
        assert_eq!(params.len(), 7);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
    }

    #[test]
    fn test_message_missing_action() {
        let args = json!({});
        let result = exec_message(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    // ── tts ─────────────────────────────────────────────────────────

    #[test]
    fn test_tts_params_defined() {
        let params = tts_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "text" && p.required));
    }

    #[test]
    fn test_tts_missing_text() {
        let args = json!({});
        let result = exec_tts(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_tts_returns_media_path() {
        let args = json!({ "text": "Hello world" });
        let result = exec_tts(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("MEDIA:"));
    }

    // ── image ───────────────────────────────────────────────────────

    #[test]
    fn test_image_params_defined() {
        let params = image_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "image" && p.required));
        assert!(params.iter().any(|p| p.name == "prompt" && !p.required));
    }

    #[test]
    fn test_image_missing_image() {
        let args = json!({});
        let result = exec_image(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_image_url_detection() {
        let args = json!({ "image": "https://example.com/photo.jpg" });
        let result = exec_image(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Is URL: true"));
    }

    // ── nodes ───────────────────────────────────────────────────────

    #[test]
    fn test_nodes_params_defined() {
        let params = nodes_params();
        assert_eq!(params.len(), 8);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
        assert!(params.iter().any(|p| p.name == "node" && !p.required));
    }

    #[test]
    fn test_nodes_missing_action() {
        let args = json!({});
        let result = exec_nodes(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_nodes_status() {
        let args = json!({ "action": "status" });
        let result = exec_nodes(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Node status"));
    }

    // ── browser ─────────────────────────────────────────────────────

    #[test]
    fn test_browser_params_defined() {
        let params = browser_params();
        assert_eq!(params.len(), 7);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
    }

    #[test]
    fn test_browser_missing_action() {
        let args = json!({});
        let result = exec_browser(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_browser_status() {
        let args = json!({ "action": "status" });
        let result = exec_browser(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Browser status"));
    }

    // ── canvas ──────────────────────────────────────────────────────

    #[test]
    fn test_canvas_params_defined() {
        let params = canvas_params();
        assert_eq!(params.len(), 6);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
    }

    #[test]
    fn test_canvas_missing_action() {
        let args = json!({});
        let result = exec_canvas(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_canvas_snapshot() {
        let args = json!({ "action": "snapshot" });
        let result = exec_canvas(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("canvas snapshot"));
    }
}
