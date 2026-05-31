pub mod agent_setup;
// Agent tool system for RustyClaw.
//
// Provides a registry of tools that the language model can invoke, and
// formatters that serialise the tool definitions into each provider's
// native schema (OpenAI function-calling, Anthropic tool-use, Google
// function declarations).

use tracing::{debug, instrument, warn};

mod ast_grep;
mod browser;
mod cron_tool;
mod devices;
pub mod exo_ai;
mod file;
mod gateway_tools;
pub(crate) mod helpers;
mod memory_tools;
pub mod npm;
pub mod ollama;
mod patch;
mod pdf;
mod runtime;
mod schema;
mod secrets_tools;
mod sessions_tools;
mod skills_tools;
mod swarm_tools;
mod sysadmin;
mod system_tools;
pub mod uv;
mod web;
// ast-grep structural code tool
use ast_grep::exec_ast_grep;

// UV tool
use uv::exec_uv_manage;

// npm / Node.js tool
use npm::exec_npm_manage;

// Agent setup orchestrator
use agent_setup::exec_agent_setup;
mod params;

// Swarm tools
use swarm_tools::{
    exec_swarm_create, exec_swarm_list, exec_swarm_send, exec_swarm_status, exec_swarm_stop,
    exec_swarm_templates,
};

// Re-export helpers for external use
pub use helpers::{
    SharedVault, VAULT_ACCESS_DENIED, command_references_credentials, expand_tilde, init_sandbox,
    is_protected_path, process_manager, run_sandboxed_command, sandbox, sanitize_tool_output,
    set_credentials_dir, set_vault, vault,
};

// File operations
use file::{
    exec_edit_file, exec_find_files, exec_list_directory, exec_read_file, exec_search_files,
    exec_write_file,
};

// Runtime operations
use runtime::{exec_execute_command, exec_process};

// Web operations
use web::{exec_web_fetch, exec_web_search};

// Memory operations
use memory_tools::exec_add_memory;
use memory_tools::{exec_memory_get, exec_memory_search, exec_save_memory, exec_search_history};

// Cron operations
use cron_tool::exec_cron;

// Session operations
use sessions_tools::{
    exec_agents_list, exec_session_status, exec_sessions_history, exec_sessions_list,
    exec_sessions_send, exec_sessions_spawn,
};

// Patch operations
use patch::exec_apply_patch;

// Gateway operations
use gateway_tools::{exec_gateway, exec_image, exec_message, exec_tts};

// Device operations
use devices::{exec_canvas, exec_nodes};

// Browser automation (separate module with feature-gated implementation)
use browser::exec_browser;

// Skill operations
use skills_tools::{
    exec_skill_create, exec_skill_enable, exec_skill_info, exec_skill_install,
    exec_skill_link_secret, exec_skill_list, exec_skill_search,
};

// MCP operations
mod mcp_tools;
use mcp_tools::{exec_mcp_connect, exec_mcp_disconnect, exec_mcp_list};

// Task operations
mod task_tools;
use task_tools::{
    exec_task_background, exec_task_cancel, exec_task_describe, exec_task_foreground,
    exec_task_input, exec_task_list, exec_task_pause, exec_task_resume, exec_task_status,
};

// Model operations
mod model_tools;
use model_tools::{
    exec_model_disable, exec_model_enable, exec_model_list, exec_model_recommend, exec_model_set,
};

// Secrets operations
use secrets_tools::exec_secrets_stub;

// System tools
use system_tools::{
    exec_app_index, exec_audit_sensitive, exec_battery_health, exec_browser_cache,
    exec_classify_files, exec_clipboard, exec_cloud_browse, exec_disk_usage, exec_screenshot,
    exec_secure_delete, exec_summarize_file, exec_system_monitor,
};

// System administration tools
use sysadmin::{
    exec_firewall, exec_net_info, exec_net_scan, exec_pkg_manage, exec_service_manage,
    exec_user_manage,
};

// PDF tool
use pdf::exec_pdf;

// Exo AI tools
use exo_ai::exec_exo_manage;

// Ollama tools
use ollama::exec_ollama_manage;

/// Stub executor for the `ask_user` tool — never called directly.
/// Execution is intercepted by the gateway, which forwards the prompt
/// to the TUI and returns the user's response as the tool result.
fn exec_ask_user_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Err("ask_user must be executed via the gateway".into())
}

/// Stub executor for the `client_dom_query` tool — never called directly.
/// Execution is intercepted by the gateway, which forwards the query to
/// the desktop client's webview and returns the evaluated result.
fn exec_client_dom_query_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Err("client_dom_query must be executed via the gateway".into())
}

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

// ── Tool permissions ────────────────────────────────────────────────────────

/// Permission level for a tool, controlling whether the agent can invoke it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    /// Tool is always allowed — no confirmation needed.
    #[default]
    Allow,
    /// Tool is always denied — the model receives an error.
    Deny,
    /// Tool requires user confirmation each time.
    Ask,
    /// Tool is only allowed when invoked by a named skill.
    SkillOnly(Vec<String>),
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
        "add_memory" => "Add memory to semantic index",
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
        "task_describe" => "Set task description (shown in sidebar)",
        "thread_describe" => "Set conversation thread description (shown in sidebar)",
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
        "client_dom_query" => "Evaluate JavaScript in the desktop client's webview DOM",
        "ollama_manage" => "Administer the Ollama model server",
        "exo_manage" => "Administer the Exo distributed AI cluster (git clone + uv run)",
        "ast_grep_manage" => "Structural code search, lint & rewrite via ast-grep",
        "uv_manage" => "Manage Python envs & packages via uv",
        "npm_manage" => "Manage Node.js packages & scripts via npm",
        "agent_setup" => "Set up local model infrastructure",
        "pdf" => "Analyze PDF files (extract text, metadata, page counts)",
        "swarm_create" => "Create and start a multi-agent swarm",
        "swarm_list" => "List all swarms and their status",
        "swarm_status" => "Get detailed status for a swarm",
        "swarm_send" => "Send a task to a swarm agent",
        "swarm_stop" => "Stop a running swarm",
        "swarm_templates" => "List available swarm templates",
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
        &ADD_MEMORY,
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
        &TASK_DESCRIBE,
        &THREAD_DESCRIBE,
        &SET_THREAD_CAPTION,
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
        &AST_GREP_MANAGE,
        &UV_MANAGE,
        &NPM_MANAGE,
        &AGENT_SETUP,
        &ASK_USER,
        &CLIENT_DOM_QUERY,
        &PDF,
        &SWARM_CREATE,
        &SWARM_LIST,
        &SWARM_STATUS,
        &SWARM_SEND,
        &SWARM_STOP,
        &SWARM_TEMPLATES,
    ]
}

mod definitions;
pub use definitions::*;

// ── Tool execution ──────────────────────────────────────────────────────────

/// Returns `true` for tools that must be routed through the gateway
/// (i.e. handled by `execute_secrets_tool`) rather than `execute_tool`.
pub fn is_secrets_tool(name: &str) -> bool {
    matches!(
        name,
        "secrets_list" | "secrets_get" | "secrets_store" | "secrets_set_policy"
    )
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

/// Returns `true` for the DOM query tool that must be routed through
/// the gateway → desktop client → gateway → tool-result path.
pub fn is_dom_query_tool(name: &str) -> bool {
    name == "client_dom_query"
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
    "exo_manage",
    "uv_manage",
    "npm_manage",
    "pkg_manage",
    "net_info",
    "net_scan",
    "service_manage",
    "user_manage",
    "firewall",
    "disk_usage",
    "classify_files",
    "system_monitor",
    "battery_health",
    "app_index",
    "cloud_browse",
    "browser_cache",
    "screenshot",
    "clipboard",
    "audit_sensitive",
    "secure_delete",
    "summarize_file",
    "nodes",
    "canvas",
];

/// Find a tool by name and execute it with the given arguments.
///
/// Tools with async implementations are called directly.
/// Other tools run on a blocking thread pool to avoid blocking the async runtime.
#[instrument(skip(args, workspace_dir), fields(tool = name))]
pub async fn execute_tool(
    name: &str,
    args: &Value,
    workspace_dir: &Path,
) -> Result<String, String> {
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
            "exo_manage" => exo_ai::exec_exo_manage_async(args, workspace_dir).await,
            "uv_manage" => uv::exec_uv_manage_async(args, workspace_dir).await,
            "npm_manage" => npm::exec_npm_manage_async(args, workspace_dir).await,
            "pkg_manage" => sysadmin::exec_pkg_manage_async(args, workspace_dir).await,
            "net_info" => sysadmin::exec_net_info_async(args, workspace_dir).await,
            "net_scan" => sysadmin::exec_net_scan_async(args, workspace_dir).await,
            "service_manage" => sysadmin::exec_service_manage_async(args, workspace_dir).await,
            "user_manage" => sysadmin::exec_user_manage_async(args, workspace_dir).await,
            "firewall" => sysadmin::exec_firewall_async(args, workspace_dir).await,
            "disk_usage" => system_tools::exec_disk_usage_async(args, workspace_dir).await,
            "classify_files" => system_tools::exec_classify_files_async(args, workspace_dir).await,
            "system_monitor" => system_tools::exec_system_monitor_async(args, workspace_dir).await,
            "battery_health" => system_tools::exec_battery_health_async(args, workspace_dir).await,
            "app_index" => system_tools::exec_app_index_async(args, workspace_dir).await,
            "cloud_browse" => system_tools::exec_cloud_browse_async(args, workspace_dir).await,
            "browser_cache" => system_tools::exec_browser_cache_async(args, workspace_dir).await,
            "screenshot" => system_tools::exec_screenshot_async(args, workspace_dir).await,
            "clipboard" => system_tools::exec_clipboard_async(args, workspace_dir).await,
            "audit_sensitive" => {
                system_tools::exec_audit_sensitive_async(args, workspace_dir).await
            }
            "secure_delete" => system_tools::exec_secure_delete_async(args, workspace_dir).await,
            "summarize_file" => system_tools::exec_summarize_file_async(args, workspace_dir).await,
            "nodes" => devices::exec_nodes_async(args, workspace_dir).await,
            "canvas" => devices::exec_canvas_async(args, workspace_dir).await,
            _ => unreachable!(),
        };
        if result.is_err() {
            warn!(error = ?result.as_ref().err(), "Tool execution failed");
        }
        return result.map(|s| crate::tool_pipeline::apply_global(name, args, s));
    }

    // Find the tool for sync execution
    let tool = all_tools().into_iter().find(|t| t.name == name);

    let Some(tool) = tool else {
        warn!(tool = name, "Unknown tool requested");
        return Err(format!("Unknown tool: {}", name));
    };

    // Clone what we need for the blocking task
    let execute_fn = tool.execute;
    let args_for_pipeline = args.clone();
    let args = args.clone();
    let workspace_dir = workspace_dir.to_path_buf();

    // Run sync tools on blocking thread pool
    let result = tokio::task::spawn_blocking(move || execute_fn(&args, &workspace_dir))
        .await
        .map_err(|e| format!("Task join error: {}", e))?;

    if result.is_err() {
        warn!(error = ?result.as_ref().err(), "Tool execution failed");
    }

    result.map(|s| crate::tool_pipeline::apply_global(name, &args_for_pipeline, s))
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
mod tests_a;
#[cfg(test)]
mod tests_b;
