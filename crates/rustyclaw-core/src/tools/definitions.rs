//! Built-in tool definitions (static `ToolDef`s) and small inline tool execs.

#![allow(unused_imports)]
use super::*;

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
    parameters: vec![], // filled by init; see `read_file_params()`.
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

#[cfg(feature = "semantic-memory")]
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

#[cfg(feature = "semantic-memory")]
pub static ADD_MEMORY: ToolDef = ToolDef {
    name: "add_memory",
    description: "Add a memory to the semantic vector index. Use to store important facts, decisions, \
                  or context that should be searchable later. Memories are embedded and stored in \
                  .steel-memory/ for fast semantic retrieval.",
    parameters: vec![],
    execute: exec_add_memory,
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

pub static TASK_DESCRIBE: ToolDef = ToolDef {
    name: "task_describe",
    description: "Set a short description of what the task is currently doing. \
                  This description is displayed in the sidebar. \
                  If no task ID is provided, sets description for the current task.",
    parameters: vec![],
    execute: exec_task_describe,
};

// ── Thread tools ────────────────────────────────────────────────────────────

/// Marker prefix for thread update commands in tool output.
pub const THREAD_UPDATE_MARKER: &str = "🏷️THREAD_UPDATE:";

pub static THREAD_DESCRIBE: ToolDef = ToolDef {
    name: "thread_describe",
    description: "Set a description for the current conversation thread. \
                  This description is displayed in the sidebar and helps track what the thread is about. \
                  Call this when starting a new task or when the thread's focus changes significantly.",
    parameters: vec![],
    execute: exec_thread_describe,
};

fn exec_thread_describe(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: description")?;

    // Return a marker that the gateway will intercept
    let update = json!({
        "action": "set_description",
        "description": description,
    });

    Ok(format!("{}{}", THREAD_UPDATE_MARKER, update))
}

pub static SET_THREAD_CAPTION: ToolDef = ToolDef {
    name: "set_thread_caption",
    description: "Set a short caption for the current conversation thread. \
                  The caption is the thread title shown in the sidebar. \
                  Call this once at the start of a new conversation to give the thread a meaningful name.",
    parameters: vec![],
    execute: exec_set_thread_caption,
};

fn exec_set_thread_caption(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let caption = args
        .get("caption")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: caption")?;

    let trimmed = caption.trim();
    if trimmed.is_empty() {
        return Err("Caption cannot be empty.".to_string());
    }

    let update = json!({
        "action": "set_caption",
        "caption": trimmed,
    });

    Ok(format!("{}{}", THREAD_UPDATE_MARKER, update))
}

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

// ── Kernel awareness tools ──────────────────────────────────────────────────

pub static HOST_INFO: ToolDef = ToolDef {
    name: "host_info",
    description: "Return the gateway host's hardware capabilities: CPU (brand, cores, frequency), \
                  GPU (name, vendor, VRAM), RAM, swap, disk, OS, and architecture. \
                  Use this to understand what the system can run locally.",
    parameters: vec![],
    execute: exec_host_info_stub,
};

pub static LOAD_STATUS: ToolDef = ToolDef {
    name: "load_status",
    description: "Return the current system load: composite load score (0.0–1.0), \
                  CPU usage, memory usage, swap, active models and inferences. \
                  Use this to decide whether to run local models or defer to external providers.",
    parameters: vec![],
    execute: exec_load_status_stub,
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

// ── ast-grep tool ──────────────────────────────────────────────────────────

pub static AST_GREP_MANAGE: ToolDef = ToolDef {
    name: "ast_grep_manage",
    description: "Structural code search, lint, and rewriting via ast-grep. \
                  Uses tree-sitter AST patterns to match code structure instead of text. \
                  Actions: setup (install ast-grep), search/run (search or rewrite with \
                  AST patterns), scan (run lint rules by config), test (test rules), \
                  new (create rules/tests/projects), version, help. \
                  Pattern syntax: use code snippets with metavariables like $$VAR. \
                  Example: ast_grep_manage with pattern='Some($$VAL)', lang='rust' \
                  to match all Option::Some usages.",
    parameters: vec![],
    execute: exec_ast_grep,
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

// ── Client DOM query tool ───────────────────────────────────────────────────

pub static CLIENT_DOM_QUERY: ToolDef = ToolDef {
    name: "client_dom_query",
    description: "Evaluate a JavaScript expression inside the desktop client's webview \
                  and return the result. Use this to inspect the DOM, read element \
                  properties (scroll positions, dimensions, text content, computed \
                  styles), or diagnose rendering issues. The expression should \
                  return a JSON-serialisable value (string, number, boolean, object, \
                  or array). Example: \
                  `document.querySelector('.messages').scrollTop` to read scroll \
                  position, or `document.querySelector('.messages').innerHTML` to \
                  inspect rendered HTML. Only read-only queries are intended; \
                  do not modify the DOM.",
    parameters: vec![],
    execute: exec_client_dom_query_stub,
};

// ── PDF tool ────────────────────────────────────────────────────────────────

pub static PDF: ToolDef = ToolDef {
    name: "pdf",
    description: "Analyze PDF files. Actions:\n\
                  - extract: Extract text from a PDF (supports page ranges via start_page/end_page)\n\
                  - info: Get PDF metadata (title, author, pages, etc.)\n\
                  - page_count: Get the number of pages\n\n\
                  Requires poppler-utils (pdftotext, pdfinfo) for best results. \
                  Falls back to textutil (macOS) or pdfminer (Python).",
    parameters: vec![],
    execute: exec_pdf,
};

// ── Swarm tools ─────────────────────────────────────────────────────────────

pub static SWARM_CREATE: ToolDef = ToolDef {
    name: "swarm_create",
    description: "Create and start a multi-agent swarm from a built-in template or custom config. \
                  Templates: 'swarm' (8 agents: orchestrator + 7 specialists covering research, \
                  data analysis, slides, docs, images, video, and assistant tasks). \
                  Use swarm_templates to see available templates.",
    parameters: vec![],
    execute: exec_swarm_create,
};

pub static SWARM_LIST: ToolDef = ToolDef {
    name: "swarm_list",
    description: "List all swarms and their current status (running, idle, stopped).",
    parameters: vec![],
    execute: exec_swarm_list,
};

pub static SWARM_STATUS: ToolDef = ToolDef {
    name: "swarm_status",
    description: "Get detailed status for a named swarm including agents, communication flows, \
                  session mappings, and task routing statistics.",
    parameters: vec![],
    execute: exec_swarm_status,
};

pub static SWARM_SEND: ToolDef = ToolDef {
    name: "swarm_send",
    description: "Send a task or message to a specific agent within a running swarm. \
                  If no agent is specified, the message is routed to the orchestrator. \
                  The orchestrator can then delegate to the appropriate specialist(s).",
    parameters: vec![],
    execute: exec_swarm_send,
};

pub static SWARM_STOP: ToolDef = ToolDef {
    name: "swarm_stop",
    description: "Stop a running swarm and clean up all its agent sessions.",
    parameters: vec![],
    execute: exec_swarm_stop,
};

pub static SWARM_TEMPLATES: ToolDef = ToolDef {
    name: "swarm_templates",
    description: "List available built-in swarm templates with their agent rosters. \
                  Use swarm_create with a template name to instantiate one.",
    parameters: vec![],
    execute: exec_swarm_templates,
};

// Re-export parameter functions from params module
pub use params::*;

// Re-export provider-specific tool-schema formatters.
pub use schema::{tools_anthropic, tools_google, tools_openai};
