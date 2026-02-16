//! Tool parameter definitions for RustyClaw.
//!
//! Each `*_params()` function returns the parameter schema for a tool,
//! which is used by provider formatters to generate JSON Schema.

use super::ToolParam;

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

pub fn write_file_params() -> Vec<ToolParam> {
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

pub fn edit_file_params() -> Vec<ToolParam> {
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

pub fn list_directory_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "path".into(),
        description: "Path to the directory to list.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn search_files_params() -> Vec<ToolParam> {
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

pub fn find_files_params() -> Vec<ToolParam> {
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

pub fn execute_command_params() -> Vec<ToolParam> {
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

pub fn web_fetch_params() -> Vec<ToolParam> {
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
        ToolParam {
            name: "use_cookies".into(),
            description: "Use stored cookies for this request and save any \
                          Set-Cookie headers from the response. Follows browser \
                          security rules (domain scoping, Secure flag). Default: false."
                .into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

pub fn web_search_params() -> Vec<ToolParam> {
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
        ToolParam {
            name: "provider".into(),
            description: "Search provider: 'auto' (default), 'brave', or 'duckduckgo'. \
                          In auto mode, Brave is used when BRAVE_API_KEY is set, with \
                          automatic DuckDuckGo fallback on failure."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn process_params() -> Vec<ToolParam> {
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

pub fn memory_search_params() -> Vec<ToolParam> {
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

pub fn memory_get_params() -> Vec<ToolParam> {
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

pub fn secrets_list_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "prefix".into(),
        description: "Optional prefix to filter key names.".into(),
        param_type: "string".into(),
        required: false,
    }]
}

pub fn secrets_get_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "key".into(),
        description: "The name of the secret to retrieve.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn secrets_store_params() -> Vec<ToolParam> {
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

pub fn gateway_params() -> Vec<ToolParam> {
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

pub fn message_params() -> Vec<ToolParam> {
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

pub fn tts_params() -> Vec<ToolParam> {
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

pub fn image_params() -> Vec<ToolParam> {
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

pub fn nodes_params() -> Vec<ToolParam> {
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

pub fn browser_params() -> Vec<ToolParam> {
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

pub fn canvas_params() -> Vec<ToolParam> {
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

pub fn cron_params() -> Vec<ToolParam> {
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

pub fn sessions_list_params() -> Vec<ToolParam> {
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
        ToolParam {
            name: "includeArchived".into(),
            description: "Include archived sessions from disk. Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "retentionDays".into(),
            description: "Prune archived sessions older than N days before listing.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn sessions_spawn_params() -> Vec<ToolParam> {
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

pub fn sessions_send_params() -> Vec<ToolParam> {
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

pub fn sessions_history_params() -> Vec<ToolParam> {
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
        ToolParam {
            name: "includeArchived".into(),
            description: "Look up archived session history when active session is not found. Default: true.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

pub fn session_status_params() -> Vec<ToolParam> {
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
        ToolParam {
            name: "archive".into(),
            description: "Archive the specified sessionKey to disk (JSONL).".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "archiveCompleted".into(),
            description: "Archive all non-active sessions.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "retentionDays".into(),
            description: "Prune archived sessions older than N days.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "includeArchived".into(),
            description: "Include archived sessions in status lookups and totals. Default: true.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

pub fn agents_list_params() -> Vec<ToolParam> {
    vec![]
}

pub fn apply_patch_params() -> Vec<ToolParam> {
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

// ── Skill tool parameters ───────────────────────────────────────────────────

pub fn skill_list_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "filter".into(),
        description: "Optional filter: 'all' (default), 'enabled', 'disabled', 'registry'.".into(),
        param_type: "string".into(),
        required: false,
    }]
}

pub fn skill_search_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "query".into(),
        description: "Search query for the ClawHub registry.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn skill_install_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "Name of the skill to install from ClawHub.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "version".into(),
            description: "Specific version to install (default: latest).".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn skill_info_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "name".into(),
        description: "Name of the skill to get info about.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn skill_enable_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "Name of the skill to enable or disable.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "enabled".into(),
            description: "Whether to enable (true) or disable (false) the skill.".into(),
            param_type: "boolean".into(),
            required: true,
        },
    ]
}

pub fn skill_link_secret_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'link' or 'unlink'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "skill".into(),
            description: "Name of the skill.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "secret".into(),
            description: "Name of the vault credential to link/unlink.".into(),
            param_type: "string".into(),
            required: true,
        },
    ]
}
