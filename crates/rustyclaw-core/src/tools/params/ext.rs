//! Tool parameter definitions (continued).

use crate::tools::ToolParam;

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

pub fn set_thread_caption_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "caption".into(),
        description: "Short caption for the thread (2-6 words). Summarises the topic.".into(),
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

pub fn pdf_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description: "Action to perform: 'extract' (default) to extract text, \
                          'info' to get metadata, 'page_count' to get number of pages."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "path".into(),
            description: "Path to the PDF file.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "start_page".into(),
            description: "First page to extract (1-based, inclusive). Omit to start from page 1."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "end_page".into(),
            description: "Last page to extract (1-based, inclusive). Omit to read to the end."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "max_chars".into(),
            description: "Maximum characters to return. Truncates output if exceeded.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

// ── Swarm tool parameters ───────────────────────────────────────────────────

pub fn swarm_create_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "template".into(),
            description: "Name of a built-in template (e.g. 'swarm'). \
                          Use swarm_templates to see available templates. \
                          Ignored if 'config' is provided."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "config".into(),
            description: "Full swarm configuration as a JSON object. \
                          Overrides the template parameter. Must include \
                          'name', 'agents', and 'flows' fields."
                .into(),
            param_type: "object".into(),
            required: false,
        },
    ]
}

pub fn swarm_list_params() -> Vec<ToolParam> {
    vec![]
}

pub fn swarm_status_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "name".into(),
        description: "Name of the swarm to inspect.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn swarm_send_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "swarm".into(),
            description: "Name of the swarm to route through.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "agent".into(),
            description: "Target agent ID within the swarm (e.g. 'deep_research', \
                          'data_analyst'). Defaults to 'orchestrator' if omitted."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "message".into(),
            description: "The task or message to send to the agent.".into(),
            param_type: "string".into(),
            required: true,
        },
    ]
}

pub fn swarm_stop_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "name".into(),
        description: "Name of the swarm to stop.".into(),
        param_type: "string".into(),
        required: true,
    }]
}

pub fn swarm_templates_params() -> Vec<ToolParam> {
    vec![]
}

pub fn client_dom_query_params() -> Vec<ToolParam> {
    vec![ToolParam {
        name: "js".into(),
        description: "JavaScript expression to evaluate in the desktop client's webview. \
                      The expression is wrapped in a try/catch and its return value is \
                      JSON-stringified. Examples: \
                      'document.querySelector(\".messages\").scrollTop', \
                      'document.querySelector(\".messages\").scrollHeight', \
                      'document.querySelector(\".messages\").innerHTML.length', \
                      'JSON.stringify({scrollTop: el.scrollTop, scrollHeight: el.scrollHeight, clientHeight: el.clientHeight})'"
            .into(),
        param_type: "string".into(),
        required: true,
    }]
}
