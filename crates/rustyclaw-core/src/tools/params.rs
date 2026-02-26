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
            description:
                "First line to read (1-based, inclusive). Omit to start from the beginning.".into(),
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
            description:
                "Run in background immediately. Returns a sessionId for use with process tool."
                    .into(),
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
        ToolParam {
            name: "authorization".into(),
            description: "Authorization header value for API requests. For Bearer tokens: \
                          'Bearer <token>'. For GitHub PATs: 'token <pat>'. This is sent \
                          as the Authorization HTTP header."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "headers".into(),
            description: "Additional HTTP headers as JSON object, e.g. {\"X-Api-Key\": \"...\"}. \
                          Use for custom headers beyond Authorization."
                .into(),
            param_type: "object".into(),
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
    ]
}

pub fn process_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'list', 'poll', 'log', 'write', 'send_keys', 'kill', 'clear', 'remove'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "sessionId".into(),
            description: "Session ID for poll/log/write/send_keys/kill/remove actions.".into(),
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
            name: "keys".into(),
            description: "Space-separated key names to send (for 'send_keys' action). Supports: Enter, Tab, Escape, Space, Backspace, Up, Down, Left, Right, Home, End, PageUp, PageDown, Delete, Insert, Ctrl-A..Ctrl-Z, F1..F12, or literal text.".into(),
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
        ToolParam {
            name: "recencyBoost".into(),
            description: "Enable recency weighting to boost recent memories. Default: true.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "halfLifeDays".into(),
            description: "Half-life for temporal decay in days. Lower values favor recent memories more strongly. Default: 30.".into(),
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

pub fn save_memory_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "history_entry".into(),
            description: "A summary to append to HISTORY.md with timestamp. Use for logging events, decisions, and facts.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "memory_update".into(),
            description: "Optional: full new content for MEMORY.md. Replaces the entire file. Use to curate long-term facts.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn search_history_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "pattern".into(),
            description: "Text pattern to search for in HISTORY.md entries.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "maxResults".into(),
            description: "Maximum number of entries to return. Default: 10.".into(),
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
        name: "name".into(),
        description: "The name of the credential to retrieve.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn secrets_store_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "The name under which to store the credential.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "kind".into(),
            description: "Credential kind: api_key, token, username_password, ssh_key, secure_note, http_passkey, form_autofill, payment_method, other.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "value".into(),
            description: "The secret value to encrypt and store.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "policy".into(),
            description: "Access policy: always (agent can read freely), approval (requires user approval, default), auth (requires re-authentication), skill:<name> (only accessible by named skill).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "description".into(),
            description: "Optional description of the credential.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "username".into(),
            description: "Username (required for username_password kind).".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn secrets_set_policy_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "The credential name to update.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "policy".into(),
            description: "New access policy: always (agent can read freely), approval (requires user approval), auth (requires re-authentication), skill:<name> (only accessible by named skill).".into(),
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
            description: "Question or instruction about the image. Default: 'Describe the image.'"
                .into(),
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
            description: "Action: 'status', 'list', 'add', 'update', 'remove', 'run', 'runs'."
                .into(),
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

pub fn skill_create_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "name".into(),
            description: "Kebab-case skill name used as the directory name (e.g. 'deploy-s3').".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "description".into(),
            description: "A concise one-line description of what this skill does.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "instructions".into(),
            description: "The full markdown body of the skill (everything after the YAML frontmatter). \
                           Include step-by-step guidance, tool usage patterns, and any constraints.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "metadata".into(),
            description: "Optional JSON metadata string, e.g. \
                           '{\"openclaw\": {\"emoji\": \"⚡\", \"requires\": {\"bins\": [\"git\"]}}}'.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

// ── Thread tools ────────────────────────────────────────────────────────────

pub fn thread_describe_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "description".into(),
        description: "Description of what this thread is about or currently doing.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

// ── System tools ────────────────────────────────────────────────────────────

pub fn disk_usage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Directory to scan. Defaults to '~'.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "depth".into(),
            description: "Max depth to traverse (default 1).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "top".into(),
            description: "Number of largest entries to return (default 20).".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn classify_files_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "path".into(),
        description: "Directory whose contents should be classified.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn system_monitor_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "metric".into(),
        description:
            "Which metric to query: 'cpu', 'memory', 'disk', 'network', or 'all' (default 'all')."
                .into(),
        param_type: "string".into(),
        required: false,
    }]
}

pub fn battery_health_params() -> Vec<ToolParam> {
    vec![]
}

pub fn app_index_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "filter".into(),
            description: "Optional substring filter for app names.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "sort".into(),
            description: "Sort order: 'size' (default) or 'name'.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn cloud_browse_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'detect' (find cloud folders, default) or 'list' (list files in a cloud folder).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "path".into(),
            description: "Path to list (required when action='list').".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn browser_cache_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'scan' (default) to report sizes or 'clean' to remove cache data.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "browser".into(),
            description: "Browser to target: 'chrome', 'firefox', 'safari', 'edge', 'arc', or 'all' (default).".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn screenshot_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Output file path (default 'screenshot.png').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "region".into(),
            description: "Capture region as 'x,y,width,height'. Omit for full screen.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "delay".into(),
            description: "Seconds to wait before capturing (default 0).".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn clipboard_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'read' to get clipboard contents, 'write' to set them.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "content".into(),
            description: "Text to write to clipboard (required when action='write').".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn audit_sensitive_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Directory to scan for sensitive data (default '.').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "max_files".into(),
            description: "Maximum number of files to scan (default 500).".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn secure_delete_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Path to the file or directory to securely delete.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "passes".into(),
            description: "Number of overwrite passes (default 3).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "confirm".into(),
            description: "Must be true to proceed. First call without confirm returns file info."
                .into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

pub fn summarize_file_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "path".into(),
            description: "Path to the file to summarize.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "max_lines".into(),
            description: "Maximum lines for the head preview (default 50).".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn ask_user_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "prompt_type".into(),
            description: "The kind of input to request. One of: 'select' (pick one option), \
                          'multi_select' (pick multiple), 'confirm' (yes/no), \
                          'text' (free text input), 'form' (multiple named fields)."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "title".into(),
            description: "The question or instruction to show the user.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "description".into(),
            description: "Optional longer explanation shown below the title.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "options".into(),
            description: "Array of option objects for select/multi_select. \
                          Each object has 'label' (required), optional 'description', \
                          and optional 'value' (defaults to label). \
                          Example: [{\"label\":\"Option A\"},{\"label\":\"Option B\",\"description\":\"detailed\"}]"
                .into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "default_value".into(),
            description: "Default value: index for select (number), array of indices for \
                          multi_select, boolean for confirm, string for text."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "fields".into(),
            description: "Array of field objects for 'form' type. Each has 'name' (key), \
                          'label' (display), optional 'placeholder', optional 'default', \
                          and optional 'required' (boolean). \
                          Example: [{\"name\":\"host\",\"label\":\"Hostname\",\"required\":true}]"
                .into(),
            param_type: "array".into(),
            required: false,
        },
    ]
}

// ── Sysadmin tools ──────────────────────────────────────────────────────────

pub fn pkg_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'install', 'uninstall', 'upgrade', 'search', \
                          'list', 'info', 'detect'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "package".into(),
            description: "Package name for install/uninstall/upgrade/search/info actions. \
                          For search, this is the query string."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "manager".into(),
            description: "Override auto-detected package manager (brew, apt, dnf, pacman, etc.). \
                          Omit to auto-detect."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn net_info_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'interfaces', 'connections', 'routing', 'dns', \
                          'ping', 'traceroute', 'whois', 'arp', 'public_ip', 'wifi', 'bandwidth'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "target".into(),
            description: "Target host/IP for ping, traceroute, dns, whois. \
                          Filter string for connections."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "count".into(),
            description: "Number of pings to send (default: 4).".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn net_scan_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Scan action: 'nmap', 'tcpdump', 'port_check', 'listen', \
                          'sniff', 'discover'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "target".into(),
            description: "Target host/IP/subnet for nmap, port_check, discover. \
                          BPF filter expression for tcpdump."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "scan_type".into(),
            description: "nmap scan type: 'quick' (default), 'full', 'service', 'os', \
                          'udp', 'vuln', 'ping', 'stealth'."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "ports".into(),
            description: "Port range for nmap (e.g. '80,443' or '1-1024'). \
                          Single port for port_check."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "interface".into(),
            description: "Network interface for tcpdump/sniff (default: 'any').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "count".into(),
            description: "Number of packets to capture for tcpdump (default: 20).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "seconds".into(),
            description: "Duration in seconds for sniff action (default: 5).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "port".into(),
            description: "Single port number for port_check action.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn service_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'list', 'status', 'start', 'stop', 'restart', \
                          'enable', 'disable', 'logs'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "service".into(),
            description: "Service name for status/start/stop/restart/enable/disable/logs. \
                          Filter string for list."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "lines".into(),
            description: "Number of log lines to show (default: 50). Used with 'logs' action."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

pub fn user_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'whoami', 'list_users', 'list_groups', 'user_info', \
                          'add_user', 'remove_user', 'add_to_group', 'last_logins'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "name".into(),
            description: "Username for user_info, add_user, remove_user, add_to_group.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "group".into(),
            description: "Group name for add_to_group action.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "shell".into(),
            description: "Login shell for add_user (default: /bin/bash). Linux only.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn firewall_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action: 'status', 'rules', 'allow', 'deny', 'enable', 'disable'.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "port".into(),
            description: "Port number for allow/deny actions.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "protocol".into(),
            description: "Protocol for allow/deny: 'tcp' (default) or 'udp'.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

// ── Local model & environment tool params ───────────────────────────────────

pub fn ollama_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'setup', 'serve', 'stop', 'status', 'pull', \
                          'rm', 'list', 'show', 'ps', 'load', 'unload', 'copy'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "model".into(),
            description: "Model name for pull/rm/show/load/unload/copy \
                          (e.g. 'llama3.1', 'mistral:7b-instruct')."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "destination".into(),
            description: "Destination tag for the 'copy' action.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn exo_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'setup', 'start', 'stop', 'status', \
                          'models', 'state', 'downloads', 'preview', 'load', 'unload', 'update', 'log'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "model".into(),
            description: "Model short ID for load/preview/unload \
                          (e.g. 'llama-3.2-1b', 'Qwen3-30B-A3B-4bit').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "instance_id".into(),
            description: "Instance ID for 'unload' action (from /state endpoint).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "port".into(),
            description: "API port for 'start' action (default: 52415).".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "no_worker".into(),
            description: "If true, start exo without worker (coordinator-only node).".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "offline".into(),
            description: "If true, start in offline/air-gapped mode.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "verbose".into(),
            description: "If true, enable verbose logging for 'start'.".into(),
            param_type: "boolean".into(),
            required: false,
        },
    ]
}

pub fn uv_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'setup', 'version', 'venv', 'pip-install', \
                          'pip-uninstall', 'pip-list', 'pip-freeze', 'sync', 'run', \
                          'python', 'init'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "package".into(),
            description: "Single package name for pip-install/pip-uninstall.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "packages".into(),
            description: "Array of package names for pip-install/pip-uninstall.".into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "name".into(),
            description: "Name for venv (default '.venv') or init (project name).".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "python".into(),
            description: "Python version specifier for venv (e.g. '3.12').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "version".into(),
            description: "Python version for 'python' action (e.g. '3.12', '3.11.6').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "command".into(),
            description: "Command string for 'run' action.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "file".into(),
            description: "Requirements file path for 'sync' action.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn npm_manage_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'setup', 'version', 'init', 'npm-install', \
                          'uninstall', 'list', 'outdated', 'update', 'run', 'start', \
                          'build', 'test', 'npx', 'audit', 'cache-clean', 'info', \
                          'search', 'status'."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "package".into(),
            description: "Single package name for install/uninstall/update/info.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "packages".into(),
            description: "Array of package names for install/uninstall.".into(),
            param_type: "array".into(),
            required: false,
        },
        ToolParam {
            name: "script".into(),
            description: "Script name for 'run' action (e.g. 'build', 'dev', 'start').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "command".into(),
            description: "Command string for 'npx' action (e.g. 'create-react-app my-app').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "query".into(),
            description: "Search query for 'search' action.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "dev".into(),
            description: "Install as devDependency (--save-dev). Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "global".into(),
            description: "Install/uninstall/list globally (-g). Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "depth".into(),
            description: "Depth for 'list' action. Default: 0.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "fix".into(),
            description: "Run 'npm audit fix' instead of just 'npm audit'. Default: false.".into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "args".into(),
            description: "Extra arguments to pass after '--' when running scripts.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

pub fn agent_setup_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "components".into(),
        description: "Array of components to set up: 'uv', 'exo', 'ollama'. \
                          Defaults to all three if omitted."
            .into(),
        param_type: "array".into(),
        required: false,
    }]
}
