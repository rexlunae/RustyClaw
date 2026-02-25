pub mod agent_setup;
// Agent tool system for RustyClaw.
//
// Provides a registry of tools that the language model can invoke, and
// formatters that serialise the tool definitions into each provider's
// native schema (OpenAI function-calling, Anthropic tool-use, Google
// function declarations).

use tracing::{debug, warn, instrument};

mod helpers;
mod file;
mod runtime;
mod web;
mod memory_tools;
mod cron_tool;
mod sessions_tools;
mod patch;
mod gateway_tools;
mod devices;
mod browser;
mod skills_tools;
mod secrets_tools;
mod system_tools;
mod sysadmin;
pub mod exo_ai;
pub mod npm;
pub mod ollama;
pub mod uv;
// UV tool
use uv::exec_uv_manage;

// npm / Node.js tool
use npm::exec_npm_manage;

// Agent setup orchestrator
use agent_setup::exec_agent_setup;
mod params;

// Re-export helpers for external use
pub use helpers::{
    process_manager, set_credentials_dir, is_protected_path,
    expand_tilde, VAULT_ACCESS_DENIED, command_references_credentials,
    init_sandbox, sandbox, run_sandboxed_command,
    set_vault, vault, SharedVault,
    sanitize_tool_output,
};

// File operations
use file::{
    exec_read_file, exec_write_file, exec_edit_file, exec_list_directory, exec_search_files, exec_find_files,
    exec_read_file_async, exec_write_file_async, exec_edit_file_async, exec_list_directory_async,
    exec_search_files_async, exec_find_files_async,
};

// Runtime operations
use runtime::{exec_execute_command, exec_process, exec_execute_command_async, exec_process_async};

// Web operations
use web::{exec_web_fetch, exec_web_search, exec_web_fetch_async, exec_web_search_async};

// Memory operations
use memory_tools::{exec_memory_search, exec_memory_get, exec_save_memory, exec_search_history};

// Cron operations
use cron_tool::exec_cron;

// Session operations
use sessions_tools::{exec_sessions_list, exec_sessions_spawn, exec_sessions_send, exec_sessions_history, exec_session_status, exec_agents_list};

// Patch operations
use patch::exec_apply_patch;

// Gateway operations
use gateway_tools::{
    exec_gateway, exec_message, exec_tts, exec_image,
    exec_gateway_async, exec_message_async, exec_tts_async, exec_image_async,
};

// Device operations
use devices::{exec_nodes, exec_canvas};

// Browser automation (separate module with feature-gated implementation)
use browser::exec_browser;

// Skill operations
use skills_tools::{exec_skill_list, exec_skill_search, exec_skill_install, exec_skill_info, exec_skill_enable, exec_skill_link_secret, exec_skill_create};

// MCP operations
mod mcp_tools;
use mcp_tools::{exec_mcp_list, exec_mcp_connect, exec_mcp_disconnect};

// Task operations
mod task_tools;
use task_tools::{
    exec_task_list, exec_task_status, exec_task_foreground, exec_task_background,
    exec_task_cancel, exec_task_pause, exec_task_resume, exec_task_input,
};

// Model operations
mod model_tools;
use model_tools::{
    exec_model_list, exec_model_enable, exec_model_disable, exec_model_set, exec_model_recommend,
};

// Secrets operations
use secrets_tools::exec_secrets_stub;

// System tools
use system_tools::{
    exec_disk_usage, exec_classify_files, exec_system_monitor,
    exec_battery_health, exec_app_index, exec_cloud_browse,
    exec_browser_cache, exec_screenshot, exec_clipboard,
    exec_audit_sensitive, exec_secure_delete, exec_summarize_file,
};

// System administration tools
use sysadmin::{
    exec_pkg_manage, exec_net_info, exec_net_scan,
    exec_service_manage, exec_user_manage, exec_firewall,
};

// Exo AI tools
use exo_ai::exec_exo_manage;

// Ollama tools
use ollama::{exec_ollama_manage, exec_ollama_manage_async};

/// Stub executor for the `ask_user` tool — never called directly.
/// Execution is intercepted by the gateway, which forwards the prompt
/// to the TUI and returns the user's response as the tool result.
fn exec_ask_user_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Err("ask_user must be executed via the gateway".into())
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

// ── Tool permissions ────────────────────────────────────────────────────────

/// Permission level for a tool, controlling whether the agent can invoke it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    /// Tool is always allowed — no confirmation needed.
    Allow,
    /// Tool is always denied — the model receives an error.
    Deny,
    /// Tool requires user confirmation each time.
    Ask,
    /// Tool is only allowed when invoked by a named skill.
    SkillOnly(Vec<String>),
}

impl Default for ToolPermission {
    fn default() -> Self {
        Self::Allow
    }
}

impl std::fmt::Display for ToolPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allow => write!(f, "Allow"),
            Self::Deny => write!(f, "Deny"),
            Self::Ask => write!(f, "Ask"),
            Self::SkillOnly(skills) => {
                if skills.is_empty() {
                    write!(f, "Skill Only (none)")
                } else {
                    write!(f, "Skill Only ({})", skills.join(", "))
                }
            }
        }
    }
}

impl ToolPermission {
    /// Cycle to the next simple permission level (for UI toggle).
    /// SkillOnly is accessed via a separate edit flow.
    pub fn cycle(&self) -> Self {
        match self {
            Self::Allow => Self::Ask,
            Self::Ask => Self::Deny,
            Self::Deny => Self::SkillOnly(Vec::new()),
            Self::SkillOnly(_) => Self::Allow,
        }
    }

    /// Short badge text for the TUI.
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Allow => "ALLOW",
            Self::Deny => "DENY",
            Self::Ask => "ASK",
            Self::SkillOnly(_) => "SKILL",
        }
    }

    /// Human-readable description of what this permission level does.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Allow => "Tool runs automatically — no confirmation needed.",
            Self::Deny => "Tool is blocked — the model receives an error and cannot use it.",
            Self::Ask => "You will be prompted to approve or deny each use of this tool.",
            Self::SkillOnly(_) => "Tool can only be used by named skills, not in direct chat.",
        }
    }
}

/// Return all tool names as a sorted list.
pub fn all_tool_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = all_tools().iter().map(|t| t.name).collect();
    names.sort();
    names
}

/// Short, user-facing summary of what each tool lets the agent do.
pub fn tool_summary(name: &str) -> &'static str {
    match name {
        "read_file" => "Read files on your computer",
        "write_file" => "Create or overwrite files",
        "edit_file" => "Edit existing files",
        "list_directory" => "List folder contents",
        "search_files" => "Search inside file contents",
        "find_files" => "Find files by name",
        "execute_command" => "Run shell commands",
        "web_fetch" => "Fetch content from URLs",
        "web_search" => "Search the web",
        "process" => "Manage background processes",
        "memory_search" => "Search agent memory files",
        "memory_get" => "Read agent memory files",
        "save_memory" => "Save memories (two-layer consolidation)",
        "search_history" => "Search HISTORY.md for past entries",
        "cron" => "Manage scheduled jobs",
        "sessions_list" => "List active sessions",
        "sessions_spawn" => "Spawn async sub-agents (use cheaper models for simple tasks)",
        "sessions_send" => "Send messages to sessions",
        "sessions_history" => "Read session message history",
        "session_status" => "Check session status & usage",
        "agents_list" => "List available agent types",
        "apply_patch" => "Apply diff patches to files",
        "secrets_list" => "List vault secret names",
        "secrets_get" => "Read secrets from the vault",
        "secrets_store" => "Store secrets in the vault",
        "secrets_set_policy" => "Change credential access policy",
        "gateway" => "Control the gateway daemon",
        "message" => "Send messages via channels",
        "tts" => "Convert text to speech",
        "image" => "Analyze images with vision AI",
        "nodes" => "Control paired companion devices",
        "browser" => "Automate a web browser",
        "canvas" => "Display UI on node canvases",
        "skill_list" => "List loaded skills",
        "skill_search" => "Search the skill registry",
        "skill_install" => "Install skills from registry",
        "skill_info" => "View skill details",
        "skill_enable" => "Enable or disable skills",
        "skill_link_secret" => "Link vault secrets to skills",
        "skill_create" => "Create a new skill from scratch",
        "mcp_list" => "List connected MCP servers",
        "mcp_connect" => "Connect to an MCP server",
        "mcp_disconnect" => "Disconnect from an MCP server",
        "task_list" => "List active tasks",
        "task_status" => "Get task status by ID",
        "task_foreground" => "Bring task to foreground",
        "task_background" => "Move task to background",
        "task_cancel" => "Cancel a running task",
        "task_pause" => "Pause a running task",
        "task_resume" => "Resume a paused task",
        "task_input" => "Send input to a task",
        "model_list" => "List available models with cost tiers",
        "model_enable" => "Enable a model for use",
        "model_disable" => "Disable a model",
        "model_set" => "Set the active model",
        "model_recommend" => "Get model recommendation for task complexity",
        "disk_usage" => "Scan disk usage by folder",
        "classify_files" => "Categorize files as docs, caches, etc.",
        "system_monitor" => "View CPU, memory & process info",
        "battery_health" => "Check battery status & health",
        "app_index" => "List installed apps by size",
        "cloud_browse" => "Browse local cloud storage folders",
        "browser_cache" => "Audit or clean browser caches",
        "screenshot" => "Capture a screenshot",
        "clipboard" => "Read or write the clipboard",
        "audit_sensitive" => "Scan files for exposed secrets",
        "secure_delete" => "Securely overwrite & delete files",
        "summarize_file" => "Preview-summarize any file type",
        "ask_user" => "Ask the user structured questions",
        "ollama_manage" => "Administer the Ollama model server",
        "exo_manage" => "Administer the Exo distributed AI cluster (git clone + uv run)",
        "uv_manage" => "Manage Python envs & packages via uv",
        "npm_manage" => "Manage Node.js packages & scripts via npm",
        "agent_setup" => "Set up local model infrastructure",
        _ => "Unknown tool",
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

/// Sync tool execution function type (legacy, for static definitions).
pub type SyncExecuteFn = fn(args: &Value, workspace_dir: &Path) -> Result<String, String>;

/// A tool that the agent can invoke.
#[derive(Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Vec<ToolParam>,
    /// The sync function that executes the tool.
    /// This is wrapped in an async context by execute_tool.
    pub execute: SyncExecuteFn,
}

impl std::fmt::Debug for ToolDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDef")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("parameters", &self.parameters)
            .finish()
    }
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
        &SAVE_MEMORY,
        &SEARCH_HISTORY,
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
        &SECRETS_SET_POLICY,
        &GATEWAY,
        &MESSAGE,
        &TTS,
        &IMAGE,
        &NODES,
        &BROWSER,
        &CANVAS,
        &SKILL_LIST,
        &SKILL_SEARCH,
        &SKILL_INSTALL,
        &SKILL_INFO,
        &SKILL_ENABLE,
        &SKILL_LINK_SECRET,
        &SKILL_CREATE,
        &MCP_LIST,
        &MCP_CONNECT,
        &MCP_DISCONNECT,
        &TASK_LIST,
        &TASK_STATUS,
        &TASK_FOREGROUND,
        &TASK_BACKGROUND,
        &TASK_CANCEL,
        &TASK_PAUSE,
        &TASK_RESUME,
        &TASK_INPUT,
        &MODEL_LIST,
        &MODEL_ENABLE,
        &MODEL_DISABLE,
        &MODEL_SET,
        &MODEL_RECOMMEND,
        &DISK_USAGE,
        &CLASSIFY_FILES,
        &SYSTEM_MONITOR,
        &BATTERY_HEALTH,
        &APP_INDEX,
        &CLOUD_BROWSE,
        &BROWSER_CACHE,
        &SCREENSHOT,
        &CLIPBOARD,
        &AUDIT_SENSITIVE,
        &SECURE_DELETE,
        &SUMMARIZE_FILE,
        &PKG_MANAGE,
        &NET_INFO,
        &NET_SCAN,
        &SERVICE_MANAGE,
        &USER_MANAGE,
        &FIREWALL,
        &OLLAMA_MANAGE,
        &EXO_MANAGE,
        &UV_MANAGE,
        &NPM_MANAGE,
        &AGENT_SETUP,
        &ASK_USER,
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
    description: "Execute a shell command and return output (stdout + stderr). \
                  Runs via `sh -c` in the workspace directory.\n\n\
                  **Common uses:**\n\
                  - Git: git status, git commit, git push\n\
                  - Build: cargo build, npm install, make\n\
                  - System: find, grep, curl, ssh\n\
                  - API calls: curl -H 'Authorization: token ...' URL\n\n\
                  For long-running commands, use background=true and poll with process tool. \
                  Set working_dir for different directory.",
    parameters: vec![],
    execute: exec_execute_command,
};

pub static WEB_FETCH: ToolDef = ToolDef {
    name: "web_fetch",
    description: "Fetch and extract readable content from a URL (HTML → markdown or plain text). \
                  Use for reading web pages, documentation, articles, APIs, or any HTTP content.\n\n\
                  **For API calls:** Use the 'authorization' parameter to pass auth headers:\n\
                  - GitHub: authorization='token ghp_...'\n\
                  - Bearer tokens: authorization='Bearer eyJ...'\n\
                  - API keys: Use 'headers' param: {\"X-Api-Key\": \"...\"}\n\n\
                  Set use_cookies=true for sites requiring login cookies. \
                  For JavaScript-heavy sites, use browser tools instead.",
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

pub static SAVE_MEMORY: ToolDef = ToolDef {
    name: "save_memory",
    description: "Save memories using two-layer consolidation. Appends a timestamped entry to HISTORY.md \
                  (grep-searchable log) and optionally updates MEMORY.md (curated long-term facts). \
                  Use to persist important context, decisions, and facts for future recall.",
    parameters: vec![],
    execute: exec_save_memory,
};

pub static SEARCH_HISTORY: ToolDef = ToolDef {
    name: "search_history",
    description: "Search HISTORY.md for past entries matching a pattern. Returns timestamped entries \
                  that match the query. Use to recall when something happened or find past events.",
    parameters: vec![],
    execute: exec_search_history,
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
    description: "Spawn a sub-agent to run a task asynchronously. Sub-agents run in isolated sessions \
                  and announce results when finished. SPAWN FREELY — the system handles concurrency efficiently.\n\n\
                  **Model selection guidance:**\n\
                  - Use `model_recommend` to get a cost-appropriate model for the task\n\
                  - Simple tasks (grep, format, list) → use free/economy models (llama3.2, claude-haiku)\n\
                  - Medium tasks (code edits, analysis) → use economy/standard models\n\
                  - Complex tasks (debugging, architecture) → use standard models\n\
                  - Critical tasks (security, production) → use premium models\n\n\
                  Multiple sub-agents can run concurrently. Continue working while they run.",
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
    description: "**CHECK THIS FIRST** before asking the user for API keys or tokens! \
                  Lists all credentials stored in the encrypted vault with their names, types, and access policies. \
                  If a credential exists here, use secrets_get to retrieve it — don't ask the user for it again.",
    parameters: vec![],
    execute: exec_secrets_stub,
};

pub static SECRETS_GET: ToolDef = ToolDef {
    name: "secrets_get",
    description: "Retrieve a credential from the vault by name. Returns the value directly.\n\n\
                  **Common workflow:**\n\
                  1. secrets_list() → see available credentials\n\
                  2. secrets_get(name='github_token') → get the token value\n\
                  3. web_fetch(url='...', authorization='token <value>') → use it\n\n\
                  For HTTP APIs, pass the token to web_fetch via 'authorization' parameter. \
                  For CLI tools, use execute_command with the token in headers or env vars.",
    parameters: vec![],
    execute: exec_secrets_stub,
};

pub static SECRETS_STORE: ToolDef = ToolDef {
    name: "secrets_store",
    description: "Store or update a credential in the encrypted secrets vault. \
                  The value is encrypted at rest. Use for API keys, tokens, and \
                  other sensitive material. Set policy to 'always' for agent access.",
    parameters: vec![],
    execute: exec_secrets_stub,
};

pub static SECRETS_SET_POLICY: ToolDef = ToolDef {
    name: "secrets_set_policy",
    description: "Change the access policy of an existing credential. Policies: \
                  always (agent can read freely), approval (requires user approval), \
                  auth (requires re-authentication), skill:<name> (only named skill).",
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
    description: "Send messages via configured channels (telegram, discord, whatsapp, signal, matrix, etc.).\n\n\
                  **Actions:** send, poll, react, thread-create, thread-reply, search, pin, edit, delete\n\n\
                  **Example:** message(action='send', channel='telegram', target='@username', message='Hello')\n\n\
                  Use for proactive notifications, cross-channel messaging, or channel-specific features \
                  like reactions, threads, and polls. The channel parameter selects which messenger to use.",
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

pub static SKILL_LIST: ToolDef = ToolDef {
    name: "skill_list",
    description: "List all loaded skills with their status (enabled, gates, source, linked secrets). \
                  Use to discover what capabilities are available.",
    parameters: vec![],
    execute: exec_skill_list,
};

pub static SKILL_SEARCH: ToolDef = ToolDef {
    name: "skill_search",
    description: "Search the ClawHub registry for installable skills. Returns skill names, \
                  descriptions, versions, and required secrets.",
    parameters: vec![],
    execute: exec_skill_search,
};

pub static SKILL_INSTALL: ToolDef = ToolDef {
    name: "skill_install",
    description: "Install a skill from the ClawHub registry by name. Optionally specify a version. \
                  After installation the skill is immediately available. Use skill_link_secret to \
                  bind required credentials.",
    parameters: vec![],
    execute: exec_skill_install,
};

pub static SKILL_INFO: ToolDef = ToolDef {
    name: "skill_info",
    description: "Show detailed information about a loaded skill: description, source, linked \
                  secrets, gating status, and instructions summary.",
    parameters: vec![],
    execute: exec_skill_info,
};

pub static SKILL_ENABLE: ToolDef = ToolDef {
    name: "skill_enable",
    description: "Enable or disable a loaded skill. Disabled skills are not injected into the \
                  agent prompt and cannot be activated.",
    parameters: vec![],
    execute: exec_skill_enable,
};

pub static SKILL_LINK_SECRET: ToolDef = ToolDef {
    name: "skill_link_secret",
    description: "Link or unlink a vault credential to a skill. When linked, the secret is \
                  accessible under the SkillOnly policy while the skill is active. Use action \
                  'link' to bind or 'unlink' to remove the binding.",
    parameters: vec![],
    execute: exec_skill_link_secret,
};

pub static SKILL_CREATE: ToolDef = ToolDef {
    name: "skill_create",
    description: "Create a new skill on disk. Provide a name (kebab-case), a one-line \
                  description, and the full markdown instructions body. The skill directory \
                  and SKILL.md file are created automatically and the skill is immediately \
                  available for use.",
    parameters: vec![],
    execute: exec_skill_create,
};

// ── MCP tools ───────────────────────────────────────────────────────────────

pub static MCP_LIST: ToolDef = ToolDef {
    name: "mcp_list",
    description: "List connected MCP (Model Context Protocol) servers and their available tools. \
                  Shows server name, connection status, and tool count.",
    parameters: vec![],
    execute: exec_mcp_list,
};

pub static MCP_CONNECT: ToolDef = ToolDef {
    name: "mcp_connect",
    description: "Connect to an MCP server by name (from config) or command. \
                  Parameters: name (string, server name from config), or command (string) + args (array).",
    parameters: vec![],
    execute: exec_mcp_connect,
};

pub static MCP_DISCONNECT: ToolDef = ToolDef {
    name: "mcp_disconnect",
    description: "Disconnect from an MCP server by name.",
    parameters: vec![],
    execute: exec_mcp_disconnect,
};


// ── Task tools ──────────────────────────────────────────────────────────────

pub static TASK_LIST: ToolDef = ToolDef {
    name: "task_list",
    description: "List active tasks. Tasks include running commands, sub-agents, cron jobs, \
                  and other long-running operations. Shows task ID, kind, status, and progress.",
    parameters: vec![],
    execute: exec_task_list,
};

pub static TASK_STATUS: ToolDef = ToolDef {
    name: "task_status",
    description: "Get detailed status of a specific task by ID.",
    parameters: vec![],
    execute: exec_task_status,
};

pub static TASK_FOREGROUND: ToolDef = ToolDef {
    name: "task_foreground",
    description: "Bring a task to the foreground. Foreground tasks stream their output \
                  to the user in real-time. Only one task per session can be foregrounded.",
    parameters: vec![],
    execute: exec_task_foreground,
};

pub static TASK_BACKGROUND: ToolDef = ToolDef {
    name: "task_background",
    description: "Move a task to the background. Background tasks continue running but \
                  don't stream output. Their output is buffered for later review.",
    parameters: vec![],
    execute: exec_task_background,
};

pub static TASK_CANCEL: ToolDef = ToolDef {
    name: "task_cancel",
    description: "Cancel a running task. The task will be terminated and marked as cancelled.",
    parameters: vec![],
    execute: exec_task_cancel,
};

pub static TASK_PAUSE: ToolDef = ToolDef {
    name: "task_pause",
    description: "Pause a running task. Not all task types support pausing.",
    parameters: vec![],
    execute: exec_task_pause,
};

pub static TASK_RESUME: ToolDef = ToolDef {
    name: "task_resume",
    description: "Resume a paused task.",
    parameters: vec![],
    execute: exec_task_resume,
};

pub static TASK_INPUT: ToolDef = ToolDef {
    name: "task_input",
    description: "Send input to a task that is waiting for user input.",
    parameters: vec![],
    execute: exec_task_input,
};


// ── Model tools ─────────────────────────────────────────────────────────────

pub static MODEL_LIST: ToolDef = ToolDef {
    name: "model_list",
    description: "List available models with their cost tiers and status. \
                  Models are categorized as: 🆓 Free, 💰 Economy, ⚖️ Standard, 💎 Premium. \
                  Use tier parameter to filter. Shows enabled/disabled and available status.",
    parameters: vec![],
    execute: exec_model_list,
};

pub static MODEL_ENABLE: ToolDef = ToolDef {
    name: "model_enable",
    description: "Enable a model for use. Enabling a model makes it available for selection \
                  as the active model or for sub-agent use.",
    parameters: vec![],
    execute: exec_model_enable,
};

pub static MODEL_DISABLE: ToolDef = ToolDef {
    name: "model_disable",
    description: "Disable a model. Disabled models won't be used even if credentials are available.",
    parameters: vec![],
    execute: exec_model_disable,
};

pub static MODEL_SET: ToolDef = ToolDef {
    name: "model_set",
    description: "Set the active model for this session. The active model handles all chat requests.",
    parameters: vec![],
    execute: exec_model_set,
};

pub static MODEL_RECOMMEND: ToolDef = ToolDef {
    name: "model_recommend",
    description: "Get a model recommendation for a given task complexity. \
                  Complexity levels: simple (use free/economy), medium (economy/standard), \
                  complex (standard), critical (premium). \
                  Use this when spawning sub-agents to pick cost-effective models.",
    parameters: vec![],
    execute: exec_model_recommend,
};


// ── System tools ────────────────────────────────────────────────────────────

pub static DISK_USAGE: ToolDef = ToolDef {
    name: "disk_usage",
    description: "Scan disk usage for a directory tree. Returns the largest entries \
                  sorted by size. Defaults to the home directory. Use `depth` to \
                  control how deep to scan and `top` to limit results.",
    parameters: vec![],
    execute: exec_disk_usage,
};

pub static CLASSIFY_FILES: ToolDef = ToolDef {
    name: "classify_files",
    description: "Classify files in a directory as user documents, caches, logs, \
                  build artifacts, cloud storage, images, video, audio, archives, \
                  installers, or app config. Useful for understanding what's in a folder.",
    parameters: vec![],
    execute: exec_classify_files,
};

pub static SYSTEM_MONITOR: ToolDef = ToolDef {
    name: "system_monitor",
    description: "Return current system resource usage: CPU load, memory, disk space, \
                  network, and top processes. Use `metric` to query a specific area \
                  or 'all' for everything.",
    parameters: vec![],
    execute: exec_system_monitor,
};

pub static BATTERY_HEALTH: ToolDef = ToolDef {
    name: "battery_health",
    description: "Report battery status including charge level, cycle count, capacity, \
                  temperature, and charging state. Works on macOS and Linux laptops.",
    parameters: vec![],
    execute: exec_battery_health,
};

pub static APP_INDEX: ToolDef = ToolDef {
    name: "app_index",
    description: "List installed applications with their size, version, and source \
                  (native or Homebrew). Sort by size or name. Filter by substring.",
    parameters: vec![],
    execute: exec_app_index,
};

pub static CLOUD_BROWSE: ToolDef = ToolDef {
    name: "cloud_browse",
    description: "Detect and browse local cloud storage sync folders (Google Drive, \
                  Dropbox, OneDrive, iCloud). Use 'detect' to find them or 'list' \
                  to browse files in a specific cloud folder.",
    parameters: vec![],
    execute: exec_cloud_browse,
};

pub static BROWSER_CACHE: ToolDef = ToolDef {
    name: "browser_cache",
    description: "Audit or clean browser cache and download folders. Supports Chrome, \
                  Firefox, Safari, Edge, and Arc. Use 'scan' to see sizes or 'clean' \
                  to remove cache data.",
    parameters: vec![],
    execute: exec_browser_cache,
};

pub static SCREENSHOT: ToolDef = ToolDef {
    name: "screenshot",
    description: "Capture a screenshot of the full screen or a specific region. \
                  Supports optional delay. Saves as PNG. Uses screencapture on macOS \
                  or imagemagick on Linux.",
    parameters: vec![],
    execute: exec_screenshot,
};

pub static CLIPBOARD: ToolDef = ToolDef {
    name: "clipboard",
    description: "Read from or write to the system clipboard. Uses pbcopy/pbpaste \
                  on macOS or xclip/xsel on Linux.",
    parameters: vec![],
    execute: exec_clipboard,
};

pub static AUDIT_SENSITIVE: ToolDef = ToolDef {
    name: "audit_sensitive",
    description: "Scan source files for potentially sensitive data: AWS keys, private \
                  keys, GitHub tokens, API keys, passwords, JWTs, Slack tokens. \
                  Matches are redacted in output. Use for security reviews.",
    parameters: vec![],
    execute: exec_audit_sensitive,
};

pub static SECURE_DELETE: ToolDef = ToolDef {
    name: "secure_delete",
    description: "Securely overwrite and delete a file or directory. Overwrites with \
                  random data multiple passes before unlinking. Requires confirm=true \
                  to proceed (first call returns file info for review). Refuses \
                  critical system paths.",
    parameters: vec![],
    execute: exec_secure_delete,
};

pub static SUMMARIZE_FILE: ToolDef = ToolDef {
    name: "summarize_file",
    description: "Generate a preview summary of any file: text files get head/tail and \
                  definition extraction; PDFs get page count and text preview; images \
                  get dimensions; media gets duration and codecs; archives get content \
                  listing. Returns structured metadata.",
    parameters: vec![],
    execute: exec_summarize_file,
};

// ── System administration tools ─────────────────────────────────────────────

pub static PKG_MANAGE: ToolDef = ToolDef {
    name: "pkg_manage",
    description: "Install, uninstall, upgrade, search, and list software packages. \
                  Auto-detects the system package manager (brew, apt, dnf, pacman, \
                  zypper, apk, snap, flatpak, port, nix-env) or use the manager \
                  parameter to override. Also supports querying package info and \
                  listing installed packages.",
    parameters: vec![],
    execute: exec_pkg_manage,
};

pub static NET_INFO: ToolDef = ToolDef {
    name: "net_info",
    description: "Query network information: interfaces, active connections, routing \
                  table, DNS lookups, ping, traceroute, whois, ARP table, public IP, \
                  Wi-Fi details, and bandwidth statistics.",
    parameters: vec![],
    execute: exec_net_info,
};

pub static NET_SCAN: ToolDef = ToolDef {
    name: "net_scan",
    description: "Network scanning and packet capture: run nmap scans (quick, full, \
                  service, OS, UDP, vuln, ping, stealth), capture packets with tcpdump, \
                  check if a specific port is open, listen for connections, sniff \
                  traffic summaries, and discover hosts on the local network.",
    parameters: vec![],
    execute: exec_net_scan,
};

pub static SERVICE_MANAGE: ToolDef = ToolDef {
    name: "service_manage",
    description: "Manage system services: list running/loaded services, check status, \
                  start, stop, restart, enable, disable, and view logs. Auto-detects \
                  the init system (systemd, launchd, sysvinit).",
    parameters: vec![],
    execute: exec_service_manage,
};

pub static USER_MANAGE: ToolDef = ToolDef {
    name: "user_manage",
    description: "Manage system users and groups: whoami, list users, list groups, \
                  get user info, add/remove users, add user to group, and view last \
                  login history.",
    parameters: vec![],
    execute: exec_user_manage,
};

pub static FIREWALL: ToolDef = ToolDef {
    name: "firewall",
    description: "Manage the system firewall: check status, list rules, allow or deny \
                  a port (TCP/UDP), enable or disable the firewall. Auto-detects the \
                  firewall backend (pf, ufw, firewalld, iptables, nftables).",
    parameters: vec![],
    execute: exec_firewall,
};

// ── Local model & environment tools ────────────────────────────────────────

pub static OLLAMA_MANAGE: ToolDef = ToolDef {
    name: "ollama_manage",
    description: "Administer the Ollama local model server. Actions: setup (install \
                  ollama), serve/start, stop, status, pull/add (download a model), \
                  rm/remove (delete a model), list (show downloaded models), \
                  show/info (model details), ps/running (loaded models), \
                  load/warm (preload into VRAM), unload/evict (free VRAM), \
                  copy/cp (duplicate a model tag).",
    parameters: vec![],
    execute: exec_ollama_manage,
};

pub static EXO_MANAGE: ToolDef = ToolDef {
    name: "exo_manage",
    description: "Administer the Exo distributed AI inference cluster (exo-explore/exo). \
                  Actions: setup (clone repo, install prereqs, build dashboard), \
                  start/run (launch exo node), stop, status (cluster overview with download \
                  progress), models/list (available models), state/topology (cluster nodes, \
                  instances & downloads), downloads/progress (show model download status with \
                  progress bars), preview (placement previews for a model), load/add/pull \
                  (create model instance / start download), unload/remove (delete instance), \
                  update (git pull + rebuild), log (view logs).",
    parameters: vec![],
    execute: exec_exo_manage,
};

pub static UV_MANAGE: ToolDef = ToolDef {
    name: "uv_manage",
    description: "Manage Python environments and packages via uv (ultra-fast package \
                  manager). Actions: setup (install uv), version, venv (create virtualenv), \
                  pip-install/add (install packages), pip-uninstall/remove (uninstall), \
                  pip-list/list (show installed), pip-freeze/freeze (export requirements), \
                  sync (install from requirements), run (execute in env), python \
                  (install a Python version), init (create new project).",
    parameters: vec![],
    execute: exec_uv_manage,
};

pub static NPM_MANAGE: ToolDef = ToolDef {
    name: "npm_manage",
    description: "Manage Node.js packages and scripts via npm. Actions: setup \
                  (install Node.js/npm), version, init (create package.json), \
                  npm-install/add (install packages), uninstall/remove, list, \
                  outdated, update, run (run a script), start, build, test, \
                  npx/exec (run a package binary), audit, cache-clean, info, \
                  search, status.",
    parameters: vec![],
    execute: exec_npm_manage,
};

pub static AGENT_SETUP: ToolDef = ToolDef {
    name: "agent_setup",
    description: "Set up the local model infrastructure in one command. Installs and \
                  verifies uv (Python package manager), exo (distributed AI cluster), \
                  and ollama (local model server). Use the optional 'components' \
                  parameter to set up only specific tools (e.g. ['ollama','uv']).",
    parameters: vec![],
    execute: exec_agent_setup,
};

// ── Interactive prompt tool ────────────────────────────────────────────────

pub static ASK_USER: ToolDef = ToolDef {
    name: "ask_user",
    description: "Ask the user a structured question. Opens an interactive dialog \
                  in the TUI for the user to respond. Supports five prompt types: \
                  'select' (pick one from a list), 'multi_select' (pick multiple), \
                  'confirm' (yes/no), 'text' (free text input), and 'form' \
                  (multiple named fields). Returns the user's answer as a JSON value. \
                  Use this when you need specific, structured input rather than free chat.",
    parameters: vec![],
    execute: exec_ask_user_stub,
};

// Re-export parameter functions from params module
pub use params::*;

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
        "save_memory" => save_memory_params(),
        "search_history" => search_history_params(),
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
        "secrets_set_policy" => secrets_set_policy_params(),
        "gateway" => gateway_params(),
        "message" => message_params(),
        "tts" => tts_params(),
        "image" => image_params(),
        "nodes" => nodes_params(),
        "browser" => browser_params(),
        "canvas" => canvas_params(),
        "skill_list" => skill_list_params(),
        "skill_search" => skill_search_params(),
        "skill_install" => skill_install_params(),
        "skill_info" => skill_info_params(),
        "skill_enable" => skill_enable_params(),
        "skill_link_secret" => skill_link_secret_params(),
        "skill_create" => skill_create_params(),
        "mcp_list" => mcp_tools::mcp_list_params(),
        "mcp_connect" => mcp_tools::mcp_connect_params(),
        "mcp_disconnect" => mcp_tools::mcp_disconnect_params(),
        "task_list" => task_tools::task_list_params(),
        "task_status" => task_tools::task_id_param(),
        "task_foreground" => task_tools::task_id_param(),
        "task_background" => task_tools::task_id_param(),
        "task_cancel" => task_tools::task_id_param(),
        "task_pause" => task_tools::task_id_param(),
        "task_resume" => task_tools::task_id_param(),
        "task_input" => task_tools::task_input_params(),
        "model_list" => model_tools::model_list_params(),
        "model_enable" => model_tools::model_id_param(),
        "model_disable" => model_tools::model_id_param(),
        "model_set" => model_tools::model_id_param(),
        "model_recommend" => model_tools::model_recommend_params(),
        "disk_usage" => disk_usage_params(),
        "classify_files" => classify_files_params(),
        "system_monitor" => system_monitor_params(),
        "battery_health" => battery_health_params(),
        "app_index" => app_index_params(),
        "cloud_browse" => cloud_browse_params(),
        "browser_cache" => browser_cache_params(),
        "screenshot" => screenshot_params(),
        "clipboard" => clipboard_params(),
        "audit_sensitive" => audit_sensitive_params(),
        "secure_delete" => secure_delete_params(),
        "summarize_file" => summarize_file_params(),
        "ask_user" => ask_user_params(),
        "pkg_manage" => pkg_manage_params(),
        "net_info" => net_info_params(),
        "net_scan" => net_scan_params(),
        "service_manage" => service_manage_params(),
        "user_manage" => user_manage_params(),
        "firewall" => firewall_params(),
        "ollama_manage" => ollama_manage_params(),
        "exo_manage" => exo_manage_params(),
        "uv_manage" => uv_manage_params(),
        "npm_manage" => npm_manage_params(),
        "agent_setup" => agent_setup_params(),
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
    matches!(name, "secrets_list" | "secrets_get" | "secrets_store" | "secrets_set_policy")
}

/// Returns `true` for skill-management tools that are routed through the
/// gateway (i.e. handled by `execute_skill_tool`) because they need access
/// to the process-global `SkillManager`.
pub fn is_skill_tool(name: &str) -> bool {
    matches!(
        name,
        "skill_list"
            | "skill_search"
            | "skill_install"
            | "skill_info"
            | "skill_enable"
            | "skill_link_secret"
            | "skill_create"
    )
}

/// Returns `true` for the interactive prompt tool that must be routed
/// through the gateway → TUI → user → gateway → tool-result path.
pub fn is_user_prompt_tool(name: &str) -> bool {
    name == "ask_user"
}

/// Tools that have native async implementations.
const ASYNC_NATIVE_TOOLS: &[&str] = &[
    "execute_command",
    "process",
    "web_fetch",
    "web_search",
    "read_file",
    "write_file",
    "edit_file",
    "list_directory",
    "search_files",
    "find_files",
    "gateway",
    "message",
    "tts",
    "image",
    "ollama_manage",
];

/// Find a tool by name and execute it with the given arguments.
/// 
/// Tools with async implementations are called directly.
/// Other tools run on a blocking thread pool to avoid blocking the async runtime.
#[instrument(skip(args, workspace_dir), fields(tool = name))]
pub async fn execute_tool(name: &str, args: &Value, workspace_dir: &Path) -> Result<String, String> {
    debug!("Executing tool");
    
    // Handle async-native tools directly
    if ASYNC_NATIVE_TOOLS.contains(&name) {
        let result = match name {
            "execute_command" => runtime::exec_execute_command_async(args, workspace_dir).await,
            "process" => runtime::exec_process_async(args, workspace_dir).await,
            "web_fetch" => web::exec_web_fetch_async(args, workspace_dir).await,
            "web_search" => web::exec_web_search_async(args, workspace_dir).await,
            "read_file" => file::exec_read_file_async(args, workspace_dir).await,
            "write_file" => file::exec_write_file_async(args, workspace_dir).await,
            "edit_file" => file::exec_edit_file_async(args, workspace_dir).await,
            "list_directory" => file::exec_list_directory_async(args, workspace_dir).await,
            "search_files" => file::exec_search_files_async(args, workspace_dir).await,
            "find_files" => file::exec_find_files_async(args, workspace_dir).await,
            "gateway" => gateway_tools::exec_gateway_async(args, workspace_dir).await,
            "message" => gateway_tools::exec_message_async(args, workspace_dir).await,
            "tts" => gateway_tools::exec_tts_async(args, workspace_dir).await,
            "image" => gateway_tools::exec_image_async(args, workspace_dir).await,
            "ollama_manage" => ollama::exec_ollama_manage_async(args, workspace_dir).await,
            _ => unreachable!(),
        };
        if result.is_err() {
            warn!(error = ?result.as_ref().err(), "Tool execution failed");
        }
        return result;
    }
    
    // Find the tool for sync execution
    let tool = all_tools().into_iter().find(|t| t.name == name);
    
    let Some(tool) = tool else {
        warn!(tool = name, "Unknown tool requested");
        return Err(format!("Unknown tool: {}", name));
    };
    
    // Clone what we need for the blocking task
    let execute_fn = tool.execute;
    let args = args.clone();
    let workspace_dir = workspace_dir.to_path_buf();
    
    // Run sync tools on blocking thread pool
    let result = tokio::task::spawn_blocking(move || {
        execute_fn(&args, &workspace_dir)
    })
    .await
    .map_err(|e| format!("Task join error: {}", e))?;
    
    if result.is_err() {
        warn!(error = ?result.as_ref().err(), "Tool execution failed");
    }
    
    result
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
        // In the workspace, CARGO_MANIFEST_DIR is crates/rustyclaw-core.
        // The workspace root is two levels up.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
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
        assert!(text.contains("workspace"));
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
        let args = json!({ "path": "crates/rustyclaw-core/src" });
        let result = exec_list_directory(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        // tools is now a directory
        assert!(text.contains("tools/"));
        assert!(text.contains("lib.rs"));
    }

    // ── search_files ────────────────────────────────────────────────

    #[test]
    fn test_search_files_finds_pattern() {
        let args = json!({ "pattern": "exec_read_file", "path": "crates/rustyclaw-core/src", "include": "*.rs" });
        let result = exec_search_files(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        // The function is now in tools/file.rs
        assert!(text.contains("tools/file.rs") || text.contains("tools\\file.rs"));
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

    #[tokio::test]
    async fn test_execute_tool_dispatch() {
        let args = json!({ "path": file!() });
        let result = execute_tool("read_file", &args, ws()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_tool_unknown() {
        let result = execute_tool("no_such_tool", &json!({}), ws()).await;
        assert!(result.is_err());
    }

    // ── Provider format tests ───────────────────────────────────────

    #[test]
    fn test_openai_format() {
        let tools = tools_openai();
        assert!(tools.len() >= 60, "Expected at least 60 tools, got {}", tools.len());
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "read_file");
        assert!(tools[0]["function"]["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_anthropic_format() {
        let tools = tools_anthropic();
        assert!(tools.len() >= 60, "Expected at least 60 tools, got {}", tools.len());
        assert_eq!(tools[0]["name"], "read_file");
        assert!(tools[0]["input_schema"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_google_format() {
        let tools = tools_google();
        assert!(tools.len() >= 60, "Expected at least 60 tools, got {}", tools.len());
        assert_eq!(tools[0]["name"], "read_file");
    }

    // ── resolve_path helper ─────────────────────────────────────────

    #[test]
    fn test_resolve_path_absolute() {
        let result = helpers::resolve_path(Path::new("/workspace"), "/absolute/path.txt");
        assert_eq!(result, std::path::PathBuf::from("/absolute/path.txt"));
    }

    #[test]
    fn test_resolve_path_relative() {
        let result = helpers::resolve_path(Path::new("/workspace"), "relative/path.txt");
        assert_eq!(result, std::path::PathBuf::from("/workspace/relative/path.txt"));
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
        assert_eq!(params.len(), 6);
        assert!(params.iter().any(|p| p.name == "url" && p.required));
        assert!(params.iter().any(|p| p.name == "extract_mode" && !p.required));
        assert!(params.iter().any(|p| p.name == "max_chars" && !p.required));
        assert!(params.iter().any(|p| p.name == "use_cookies" && !p.required));
        assert!(params.iter().any(|p| p.name == "authorization" && !p.required));
        assert!(params.iter().any(|p| p.name == "headers" && !p.required));
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
        assert_eq!(params.len(), 6);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
        assert!(params.iter().any(|p| p.name == "sessionId" && !p.required));
        assert!(params.iter().any(|p| p.name == "data" && !p.required));
        assert!(params.iter().any(|p| p.name == "keys" && !p.required));
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
        assert_eq!(params.len(), 5);
        assert!(params.iter().any(|p| p.name == "query" && p.required));
        assert!(params.iter().any(|p| p.name == "maxResults" && !p.required));
        assert!(params.iter().any(|p| p.name == "minScore" && !p.required));
        assert!(params.iter().any(|p| p.name == "recencyBoost" && !p.required));
        assert!(params.iter().any(|p| p.name == "halfLifeDays" && !p.required));
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
        let patch_str = r#"--- a/test.txt
+++ b/test.txt
@@ -1,3 +1,4 @@
 line1
+new line
 line2
 line3
"#;
        let hunks = patch::parse_unified_diff(patch_str).unwrap();
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
        assert!(params.iter().any(|p| p.name == "name" && p.required));
    }

    #[test]
    fn test_secrets_store_params_defined() {
        let params = secrets_store_params();
        assert_eq!(params.len(), 6);
        assert!(params.iter().any(|p| p.name == "name" && p.required));
        assert!(params.iter().any(|p| p.name == "kind" && p.required));
        assert!(params.iter().any(|p| p.name == "value" && p.required));
        assert!(params.iter().any(|p| p.name == "policy" && !p.required));
        assert!(params.iter().any(|p| p.name == "description" && !p.required));
        assert!(params.iter().any(|p| p.name == "username" && !p.required));
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
        let output = result.unwrap();
        assert!(output.contains("nodes"));
        assert!(output.contains("tools"));
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
        let output = result.unwrap();
        assert!(output.contains("running"));
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
        let output = result.unwrap();
        // Without a URL presented first, snapshot returns no_canvas
        assert!(output.contains("no_canvas") || output.contains("snapshot_captured"));
    }

    // ── skill tools ─────────────────────────────────────────────────

    #[test]
    fn test_skill_list_params_defined() {
        let params = skill_list_params();
        assert_eq!(params.len(), 1);
        assert!(params.iter().any(|p| p.name == "filter" && !p.required));
    }

    #[test]
    fn test_skill_search_params_defined() {
        let params = skill_search_params();
        assert_eq!(params.len(), 1);
        assert!(params.iter().any(|p| p.name == "query" && p.required));
    }

    #[test]
    fn test_skill_install_params_defined() {
        let params = skill_install_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "name" && p.required));
        assert!(params.iter().any(|p| p.name == "version" && !p.required));
    }

    #[test]
    fn test_skill_info_params_defined() {
        let params = skill_info_params();
        assert_eq!(params.len(), 1);
        assert!(params.iter().any(|p| p.name == "name" && p.required));
    }

    #[test]
    fn test_skill_enable_params_defined() {
        let params = skill_enable_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "name" && p.required));
        assert!(params.iter().any(|p| p.name == "enabled" && p.required));
    }

    #[test]
    fn test_skill_link_secret_params_defined() {
        let params = skill_link_secret_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
        assert!(params.iter().any(|p| p.name == "skill" && p.required));
        assert!(params.iter().any(|p| p.name == "secret" && p.required));
    }

    #[test]
    fn test_skill_list_standalone_stub() {
        let result = exec_skill_list(&json!({}), ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("standalone mode"));
    }

    #[test]
    fn test_skill_search_missing_query() {
        let result = exec_skill_search(&json!({}), ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_skill_install_missing_name() {
        let result = exec_skill_install(&json!({}), ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_skill_info_missing_name() {
        let result = exec_skill_info(&json!({}), ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_skill_enable_missing_params() {
        let result = exec_skill_enable(&json!({}), ws());
        assert!(result.is_err());
    }

    #[test]
    fn test_skill_link_secret_bad_action() {
        let args = json!({ "action": "nope", "skill": "x", "secret": "y" });
        let result = exec_skill_link_secret(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown action"));
    }

    #[test]
    fn test_is_skill_tool() {
        assert!(is_skill_tool("skill_list"));
        assert!(is_skill_tool("skill_search"));
        assert!(is_skill_tool("skill_install"));
        assert!(is_skill_tool("skill_info"));
        assert!(is_skill_tool("skill_enable"));
        assert!(is_skill_tool("skill_link_secret"));
        assert!(!is_skill_tool("read_file"));
        assert!(!is_skill_tool("secrets_list"));
    }

    // ── disk_usage ──────────────────────────────────────────────────

    #[test]
    fn test_disk_usage_params_defined() {
        let params = disk_usage_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().all(|p| !p.required));
    }

    #[test]
    fn test_disk_usage_workspace() {
        let args = json!({ "path": ".", "depth": 1, "top": 5 });
        let result = exec_disk_usage(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("entries"));
    }

    #[test]
    fn test_disk_usage_nonexistent() {
        let args = json!({ "path": "/nonexistent_path_xyz" });
        let result = exec_disk_usage(&args, ws());
        assert!(result.is_err());
    }

    // ── classify_files ──────────────────────────────────────────────

    #[test]
    fn test_classify_files_params_defined() {
        let params = classify_files_params();
        assert_eq!(params.len(), 1);
        assert!(params[0].required);
    }

    #[test]
    fn test_classify_files_workspace() {
        let args = json!({ "path": "." });
        let result = exec_classify_files(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("path"));
    }

    #[test]
    fn test_classify_files_missing_path() {
        let args = json!({});
        let result = exec_classify_files(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    // ── system_monitor ──────────────────────────────────────────────

    #[test]
    fn test_system_monitor_params_defined() {
        let params = system_monitor_params();
        assert_eq!(params.len(), 1);
        assert!(!params[0].required);
    }

    #[test]
    fn test_system_monitor_all() {
        let args = json!({});
        let result = exec_system_monitor(&args, ws());
        assert!(result.is_ok());
    }

    #[test]
    fn test_system_monitor_cpu() {
        let args = json!({ "metric": "cpu" });
        let result = exec_system_monitor(&args, ws());
        assert!(result.is_ok());
    }

    // ── battery_health ──────────────────────────────────────────────

    #[test]
    fn test_battery_health_params_defined() {
        let params = battery_health_params();
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_battery_health_runs() {
        let args = json!({});
        let result = exec_battery_health(&args, ws());
        assert!(result.is_ok());
    }

    // ── app_index ───────────────────────────────────────────────────

    #[test]
    fn test_app_index_params_defined() {
        let params = app_index_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| !p.required));
    }

    #[test]
    fn test_app_index_runs() {
        let args = json!({ "filter": "nonexistent_app_xyz" });
        let result = exec_app_index(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("apps"));
    }

    // ── cloud_browse ────────────────────────────────────────────────

    #[test]
    fn test_cloud_browse_params_defined() {
        let params = cloud_browse_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| !p.required));
    }

    #[test]
    fn test_cloud_browse_detect() {
        let args = json!({ "action": "detect" });
        let result = exec_cloud_browse(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("cloud_folders"));
    }

    #[test]
    fn test_cloud_browse_invalid_action() {
        let args = json!({ "action": "invalid" });
        let result = exec_cloud_browse(&args, ws());
        assert!(result.is_err());
    }

    // ── browser_cache ───────────────────────────────────────────────

    #[test]
    fn test_browser_cache_params_defined() {
        let params = browser_cache_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| !p.required));
    }

    #[test]
    fn test_browser_cache_scan() {
        let args = json!({ "action": "scan" });
        let result = exec_browser_cache(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("caches"));
    }

    // ── screenshot ──────────────────────────────────────────────────

    #[test]
    fn test_screenshot_params_defined() {
        let params = screenshot_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().all(|p| !p.required));
    }

    // ── clipboard ───────────────────────────────────────────────────

    #[test]
    fn test_clipboard_params_defined() {
        let params = clipboard_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "action" && p.required));
    }

    #[test]
    fn test_clipboard_missing_action() {
        let args = json!({});
        let result = exec_clipboard(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_clipboard_invalid_action() {
        let args = json!({ "action": "invalid" });
        let result = exec_clipboard(&args, ws());
        assert!(result.is_err());
    }

    // ── audit_sensitive ─────────────────────────────────────────────

    #[test]
    fn test_audit_sensitive_params_defined() {
        let params = audit_sensitive_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().all(|p| !p.required));
    }

    #[test]
    fn test_audit_sensitive_runs() {
        let dir = std::env::temp_dir().join("rustyclaw_test_audit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("safe.txt"), "nothing sensitive here").unwrap();
        let args = json!({ "path": ".", "max_files": 10 });
        let result = exec_audit_sensitive(&args, &dir);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("scanned_files"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── secure_delete ───────────────────────────────────────────────

    #[test]
    fn test_secure_delete_params_defined() {
        let params = secure_delete_params();
        assert_eq!(params.len(), 3);
        assert!(params.iter().any(|p| p.name == "path" && p.required));
    }

    #[test]
    fn test_secure_delete_missing_path() {
        let args = json!({});
        let result = exec_secure_delete(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_secure_delete_nonexistent() {
        let args = json!({ "path": "/tmp/nonexistent_rustyclaw_xyz" });
        let result = exec_secure_delete(&args, ws());
        assert!(result.is_err());
    }

    #[test]
    fn test_secure_delete_requires_confirm() {
        let dir = std::env::temp_dir().join("rustyclaw_test_secdelete");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("victim.txt"), "data").unwrap();
        let args = json!({ "path": dir.join("victim.txt").display().to_string() });
        let result = exec_secure_delete(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("confirm_required"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_secure_delete_with_confirm() {
        let dir = std::env::temp_dir().join("rustyclaw_test_secdelete2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "secret data").unwrap();
        let args = json!({
            "path": victim.display().to_string(),
            "confirm": true,
        });
        let result = exec_secure_delete(&args, ws());
        assert!(result.is_ok());
        assert!(result.unwrap().contains("deleted"));
        assert!(!victim.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── summarize_file ──────────────────────────────────────────────

    #[test]
    fn test_summarize_file_params_defined() {
        let params = summarize_file_params();
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| p.name == "path" && p.required));
    }

    #[test]
    fn test_summarize_file_missing_path() {
        let args = json!({});
        let result = exec_summarize_file(&args, ws());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing required parameter"));
    }

    #[test]
    fn test_summarize_file_this_file() {
        let args = json!({ "path": file!(), "max_lines": 10 });
        let result = exec_summarize_file(&args, ws());
        assert!(result.is_ok());
        let text = result.unwrap();
        assert!(text.contains("text"));
        assert!(text.contains("total_lines"));
    }

    #[test]
    fn test_summarize_file_nonexistent() {
        let args = json!({ "path": "/nonexistent/file.txt" });
        let result = exec_summarize_file(&args, ws());
        assert!(result.is_err());
    }
}
