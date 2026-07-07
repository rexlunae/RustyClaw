use crate::config::Config;
use crate::gateway::{ChannelPairActionKind, CronActionKind, EngineActionKind, ModelActionKind};
use crate::providers;
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    None,
    ClearMessages,
    Quit,
    /// Show gateway status info (no subcommand given)
    GatewayInfo,
    /// Change the active provider
    SetProvider(String),
    /// Change the active model
    SetModel(String),
    /// Show skills dialog
    ShowSkills,
    /// Show the secrets dialog
    ShowSecrets,
    /// Show the provider selector dialog
    ShowProviderSelector,
    /// Show the tool permissions dialog
    ShowToolPermissions,
    /// Reload gateway configuration
    GatewayReload,
    /// Fetch the live model list from the provider API
    FetchModels,
    /// Create a new thread
    ThreadNew(String),
    /// Attach a file to the next prompt
    AttachPromptFile(String),
    /// Attach a directory to the next prompt
    AttachPromptDirectory(String),
    /// Clear prompt attachments
    ClearPromptAttachments,
    /// List threads (handled in TUI)
    ThreadList,
    /// Close a thread by ID
    ThreadClose(u64),
    /// Rename a thread (id, new_label)
    ThreadRename(u64, String),
    /// Background the current foreground thread
    ThreadBackground,
    /// Foreground a thread by ID
    ThreadForeground(u64),
    /// Show the local engines panel (fetches the engine list)
    ShowEngines,
    /// Engine lifecycle action: (engine, action)
    EngineAction(String, EngineActionKind),
    /// List models for an engine
    EngineModelList(String),
    /// Pull/download a model for an engine: (engine, model)
    EngineModelPull(String, String),
    /// Model-level engine action: (engine, model, action)
    EngineModelAction(String, String, ModelActionKind),
    /// Show the cron panel (fetches the job list)
    ShowCron,
    /// Cron job action: (job id, action)
    CronAction(String, CronActionKind),
    /// Create a cron job: (name, expr, payload)
    CronAdd(String, String, String),
    /// Show the memory panel, optionally filtered by a query
    ShowMemory(Option<String>),
    /// Add a memory entry: (category, content)
    MemoryAdd(Option<String>, String),
    /// Delete a memory entry by id
    MemoryDelete(String),
    /// Search conversation history (shows in the memory panel)
    HistorySearch(String),
    /// Show the MCP servers panel (fetches the server list)
    ShowMcp,
    /// Connect an MCP server: (name, optional stdio command line)
    McpConnect(String, Option<String>),
    /// Disconnect an MCP server
    McpDisconnect(String),
    /// Show the messenger channels panel
    ShowChannels,
    /// Pair/unpair a messenger channel
    ChannelPair(String, ChannelPairActionKind),
    /// Show the usage analytics panel with an optional period filter
    ShowAnalytics(Option<String>),
    /// Show the logs panel: (source, optional tail)
    ShowLogs(String, Option<usize>),
}

#[derive(Debug, Clone)]
pub struct CommandResponse {
    pub messages: Vec<String>,
    pub action: CommandAction,
}

pub struct CommandContext<'a> {
    pub secrets_manager: &'a mut SecretsManager,
    pub skill_manager: &'a mut SkillManager,
    pub config: &'a mut Config,
}

/// Base command names shared by both `command_names` and
/// `command_names_for_provider`.  Does NOT include `model <name>` entries.
fn base_command_names() -> Vec<String> {
    let mut names: Vec<String> = vec![
        "help".into(),
        "clear".into(),
        "attach".into(),
        "attach file".into(),
        "attach dir".into(),
        "attach clear".into(),
        "enable-access".into(),
        "disable-access".into(),
        "onboard".into(),
        "reload-skills".into(),
        "gateway".into(),
        "reload".into(),
        "provider".into(),
        "model".into(),
        "skills".into(),
        "skill".into(),
        "tools".into(),
        "skill info".into(),
        "skill remove".into(),
        "skill search".into(),
        "skill install".into(),
        "skill publish".into(),
        "skill link-secret".into(),
        "skill unlink-secret".into(),
        "skill create".into(),
        "secrets".into(),
        "thread".into(),
        "thread new".into(),
        "thread list".into(),
        "thread close".into(),
        "thread rename".into(),
        "thread bg".into(),
        "thread fg".into(),
        "clawhub".into(),
        "clawhub auth".into(),
        "clawhub auth login".into(),
        "clawhub auth status".into(),
        "clawhub auth logout".into(),
        "clawhub search".into(),
        "clawhub trending".into(),
        "clawhub categories".into(),
        "clawhub info".into(),
        "clawhub browse".into(),
        "clawhub profile".into(),
        "clawhub starred".into(),
        "clawhub star".into(),
        "clawhub unstar".into(),
        "clawhub install".into(),
        "clawhub publish".into(),
        "agent setup".into(),
        "ollama".into(),
        "exo".into(),
        "uv".into(),
        "npm".into(),
        "quit".into(),
        "cron".into(),
        "cron add".into(),
        "cron pause".into(),
        "cron resume".into(),
        "cron rm".into(),
        "memory".into(),
        "memory add".into(),
        "memory rm".into(),
        "memory history".into(),
        "mcp".into(),
        "mcp connect".into(),
        "mcp disconnect".into(),
        "channels".into(),
        "channels pair".into(),
        "channels unpair".into(),
        "analytics".into(),
        "analytics day".into(),
        "analytics week".into(),
        "analytics month".into(),
        "logs".into(),
        "logs gateway".into(),
        "logs agent".into(),
        "engines".into(),
        "engines start".into(),
        "engines stop".into(),
        "engines install".into(),
        "engines models".into(),
        "engines pull".into(),
        "engines load".into(),
        "engines unload".into(),
        "engines remove".into(),
        "provider add".into(),
        "provider remove".into(),
        "provider list".into(),
    ];
    for p in providers::provider_ids() {
        names.push(format!("provider {}", p));
    }
    names
}

/// List of all known command names (without the / prefix).
/// Includes subcommand forms so tab-completion works for them.
/// Model completions include ALL providers (use `command_names_for_provider`
/// for provider-scoped completions).
pub fn command_names() -> Vec<String> {
    let mut names = base_command_names();
    for m in providers::all_model_names() {
        names.push(format!("model {}", m));
    }
    names
}

/// Like `command_names` but model completions are scoped to the given
/// provider so the user only sees model IDs that their active provider
/// actually supports.
pub fn command_names_for_provider(provider_id: &str) -> Vec<String> {
    let mut names = base_command_names();
    let models = providers::models_for_provider(provider_id);
    if models.is_empty() {
        // Provider has no static model list (e.g. custom / LM Studio) —
        // fall back to showing all models so the user isn't left with zero
        // completions.
        for m in providers::all_model_names() {
            names.push(format!("model {}", m));
        }
    } else {
        for m in models {
            names.push(format!("model {}", m));
        }
    }
    names
}

pub fn handle_command(input: &str, context: &mut CommandContext<'_>) -> CommandResponse {
    // Strip the leading '/' if present
    let trimmed = input.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return CommandResponse {
            messages: Vec::new(),
            action: CommandAction::None,
        };
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return CommandResponse {
            messages: Vec::new(),
            action: CommandAction::None,
        };
    }

    match parts[0] {
        "agent" => {
            if parts.get(1) == Some(&"setup") {
                let ws_dir = context.config.workspace_dir();
                match crate::tools::agent_setup::exec_agent_setup(&serde_json::json!({}), &ws_dir) {
                    Ok(msg) => CommandResponse {
                        messages: vec![msg],
                        action: CommandAction::None,
                    },
                    Err(e) => CommandResponse {
                        messages: vec![format!("Agent setup failed: {}", e)],
                        action: CommandAction::None,
                    },
                }
            } else {
                CommandResponse {
                    messages: vec!["Usage: /agent setup".to_string()],
                    action: CommandAction::None,
                }
            }
        }
        "ollama" => {
            // /ollama <action> [model]
            let action = parts.get(1).copied().unwrap_or("status");
            let model = parts.get(2).copied();
            let dest = parts.get(3).copied();
            let mut args = serde_json::json!({"action": action});
            if let Some(m) = model {
                args["model"] = serde_json::json!(m);
            }
            if let Some(d) = dest {
                args["destination"] = serde_json::json!(d);
            }
            let ws_dir = context.config.workspace_dir();
            match crate::tools::ollama::exec_ollama_manage(&args, &ws_dir) {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("ollama error: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        "exo" => {
            // /exo <action> [model]
            let action = parts.get(1).copied().unwrap_or("status");
            let model = parts.get(2).copied();
            let mut args = serde_json::json!({"action": action});
            if let Some(m) = model {
                args["model"] = serde_json::json!(m);
            }
            let ws_dir = context.config.workspace_dir();
            match crate::tools::exo_ai::exec_exo_manage(&args, &ws_dir) {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("exo error: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        "uv" => {
            // /uv <action> [package ...]
            let action = parts.get(1).copied().unwrap_or("version");
            let rest: Vec<&str> = parts.iter().skip(2).copied().collect();
            let mut args = serde_json::json!({"action": action});
            if rest.len() == 1 {
                args["package"] = serde_json::json!(rest[0]);
            } else if rest.len() > 1 {
                args["packages"] = serde_json::json!(rest);
            }
            let ws_dir = context.config.workspace_dir();
            match crate::tools::uv::exec_uv_manage(&args, &ws_dir) {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("uv error: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        "npm" => {
            // /npm <action> [package ...]
            let action = parts.get(1).copied().unwrap_or("status");
            let rest: Vec<&str> = parts.iter().skip(2).copied().collect();
            let mut args = serde_json::json!({"action": action});
            if rest.len() == 1 {
                args["package"] = serde_json::json!(rest[0]);
            } else if rest.len() > 1 {
                args["packages"] = serde_json::json!(rest);
            }
            let ws_dir = context.config.workspace_dir();
            match crate::tools::npm::exec_npm_manage(&args, &ws_dir) {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("npm error: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        "help" => CommandResponse {
            messages: vec![
                "Available commands:".to_string(),
                "  /help                    - Show this help".to_string(),
                "  /clear                   - Clear the message display".to_string(),
                "  /attach file <path>      - Attach a file to the next prompt".to_string(),
                "  /attach dir <path>       - Attach a directory to the next prompt".to_string(),
                "  /attach clear            - Clear prompt attachments".to_string(),
                "  /enable-access           - Enable agent access to secrets".to_string(),
                "  /disable-access          - Disable agent access to secrets".to_string(),
                "  /onboard                 - Run setup wizard (use CLI: rustyclaw onboard)"
                    .to_string(),
                "  /reload-skills           - Reload skills".to_string(),
                "  /gateway                 - Show gateway connection status".to_string(),
                "  /reload                  - Reload gateway config (no restart)".to_string(),
                "  /provider <name>         - Change the AI provider".to_string(),
                "  /provider add <id> <url> - Add a custom provider (local/self-hosted)"
                    .to_string(),
                "  /provider remove <id>    - Remove a custom provider".to_string(),
                "  /provider list           - List built-in and custom providers".to_string(),
                "  /model <name>            - Change the AI model".to_string(),
                "  /engines                 - Local engines panel (Ollama/llama.cpp/Joshua/…)"
                    .to_string(),
                "  /engines start <engine>  - Start a local engine (also: stop/install)"
                    .to_string(),
                "  /engines pull <e> <m>    - Download a model for an engine".to_string(),
                "  /engines models <engine> - List an engine's models (also: load/unload/remove)"
                    .to_string(),
                "  /skills                  - Show loaded skills".to_string(),
                "  /skill                   - Skill management (info/install/publish/link)"
                    .to_string(),
                "  /tools                   - Edit tool permissions (allow/deny/ask/skill)"
                    .to_string(),
                "  /secrets                 - Open the secrets vault".to_string(),
                "  /cron                    - Scheduled jobs panel (add/pause/resume/rm)"
                    .to_string(),
                "  /memory [query]          - Browse MEMORY.md (add/rm/history)".to_string(),
                "  /mcp                     - MCP servers panel (connect/disconnect)".to_string(),
                "  /channels                - Messenger channels panel (pair/unpair)".to_string(),
                "  /analytics [period]      - Usage stats (day/week/month/all)".to_string(),
                "  /logs [source] [n]       - Recent logs (gateway/agent/service name)".to_string(),
                "  /clawhub                 - ClawHub skill registry commands".to_string(),
                "  /agent setup             - Set up local model tools (uv, exo, ollama)"
                    .to_string(),
                "  /ollama <action> [model] - Ollama admin (setup/pull/list/ps/status/…)"
                    .to_string(),
                "  /exo <action> [model]    - Exo cluster admin (setup/start/stop/status/…)"
                    .to_string(),
                "  /uv <action> [pkg …]     - Python/uv admin (setup/pip-install/list/…)"
                    .to_string(),
                "  /npm <action> [pkg …]    - Node.js/npm admin (setup/install/run/build/…)"
                    .to_string(),
                "  /thread new <label>      - Create a new chat thread".to_string(),
                "  /thread list             - Show threads (or focus sidebar)".to_string(),
                "  /thread close <id>       - Close a thread".to_string(),
                "  /thread rename <id> <l>  - Rename a thread".to_string(),
                "  /thread bg               - Background the current thread".to_string(),
                "  /thread fg <id>          - Foreground a thread by ID".to_string(),
                "  /quit (/q, /exit)        - Exit the TUI".to_string(),
                "".to_string(),
                "Keyboard shortcuts:".to_string(),
                "  Ctrl+C quit · Esc cancel run/close dialog · Tab focus threads".to_string(),
                "  Ctrl+P pair gateway · Ctrl+H system info · Ctrl+J services".to_string(),
                "  Ctrl+D message details · Ctrl+E collapse message".to_string(),
                "  Ctrl+Y copy message · Ctrl+S save message to ~/.rustyclaw/messages".to_string(),
                "  ↑/↓ scroll messages · Enter send · Shift+Enter newline".to_string(),
            ],
            action: CommandAction::None,
        },
        "clear" => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ClearMessages,
        },
        "download" => CommandResponse {
            // No client-side media registry exists yet to download from.
            messages: vec![
                "Media downloads aren't implemented in this client yet.".to_string(),
                "Tool results that produce files save them on the gateway host.".to_string(),
            ],
            action: CommandAction::None,
        },
        "attach" => match parts.get(1).copied() {
            Some("file") => {
                let path = trimmed
                    .strip_prefix("attach file")
                    .map(str::trim_start)
                    .unwrap_or_default()
                    .to_string();
                if path.is_empty() {
                    CommandResponse {
                        messages: vec!["Usage: /attach file <path>".to_string()],
                        action: CommandAction::None,
                    }
                } else {
                    CommandResponse {
                        messages: vec![format!("Attached file: {}", path)],
                        action: CommandAction::AttachPromptFile(path),
                    }
                }
            }
            Some("dir") | Some("directory") => {
                let path = trimmed
                    .strip_prefix("attach dir")
                    .or_else(|| trimmed.strip_prefix("attach directory"))
                    .map(str::trim_start)
                    .unwrap_or_default()
                    .to_string();
                if path.is_empty() {
                    CommandResponse {
                        messages: vec!["Usage: /attach dir <path>".to_string()],
                        action: CommandAction::None,
                    }
                } else {
                    CommandResponse {
                        messages: vec![format!("Attached directory: {}", path)],
                        action: CommandAction::AttachPromptDirectory(path),
                    }
                }
            }
            Some("clear") => CommandResponse {
                messages: vec!["Cleared prompt attachments.".to_string()],
                action: CommandAction::ClearPromptAttachments,
            },
            _ => CommandResponse {
                messages: vec![
                    "Usage: /attach file <path>".to_string(),
                    "Usage: /attach dir <path>".to_string(),
                    "Usage: /attach clear".to_string(),
                ],
                action: CommandAction::None,
            },
        },
        "enable-access" => {
            context.secrets_manager.set_agent_access(true);
            context.config.agent_access = true;
            if let Err(e) = context.config.save(None) {
                tracing::warn!("failed to persist config: {e}");
            }
            CommandResponse {
                messages: vec!["Agent access to secrets enabled.".to_string()],
                action: CommandAction::None,
            }
        }
        "disable-access" => {
            context.secrets_manager.set_agent_access(false);
            context.config.agent_access = false;
            if let Err(e) = context.config.save(None) {
                tracing::warn!("failed to persist config: {e}");
            }
            CommandResponse {
                messages: vec!["Agent access to secrets disabled.".to_string()],
                action: CommandAction::None,
            }
        }
        "reload-skills" => match context.skill_manager.load_skills() {
            Ok(_) => CommandResponse {
                messages: vec![format!(
                    "Reloaded {} skills.",
                    context.skill_manager.get_skills().len()
                )],
                action: CommandAction::None,
            },
            Err(err) => CommandResponse {
                messages: vec![format!("Error reloading skills: {}", err)],
                action: CommandAction::None,
            },
        },
        "onboard" => CommandResponse {
            messages: vec![
                "The onboard wizard is an interactive CLI command.".to_string(),
                "Run it from your terminal:  rustyclaw onboard".to_string(),
            ],
            action: CommandAction::None,
        },
        "gateway" => match parts.get(1).copied() {
            Some("start" | "stop" | "restart") => CommandResponse {
                // The TUI's connection is established by the startup
                // connection dialog; the daemon itself is controlled from
                // the CLI.
                messages: vec![
                    "The TUI cannot control the gateway daemon.".to_string(),
                    "Control it from a terminal:  rustyclaw gateway start|stop|restart".to_string(),
                    "To reconnect this client, restart the TUI.".to_string(),
                ],
                action: CommandAction::None,
            },
            Some(sub) => CommandResponse {
                messages: vec![
                    format!("Unknown gateway subcommand: {}", sub),
                    "Usage: /gateway — show connection status".to_string(),
                ],
                action: CommandAction::None,
            },
            None => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::GatewayInfo,
            },
        },
        "reload" => CommandResponse {
            messages: vec!["Reloading gateway configuration…".to_string()],
            action: CommandAction::GatewayReload,
        },
        "skills" => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowSkills,
        },
        "tools" => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowToolPermissions,
        },
        "skill" => handle_skill_subcommand(&parts[1..], context),
        "secrets" => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowSecrets,
        },
        "provider" => match parts.get(1) {
            Some(&"add") => handle_provider_add(&parts[2..], context),
            Some(&"remove") | Some(&"rm") | Some(&"delete") => {
                handle_provider_remove(&parts[2..], context)
            }
            Some(&"list") | Some(&"ls") => handle_provider_list(),
            Some(name) => {
                let name = name.to_string();
                CommandResponse {
                    messages: vec![format!("Switching provider to {}…", name)],
                    action: CommandAction::SetProvider(name),
                }
            }
            None => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::ShowProviderSelector,
            },
        },
        "engines" | "engine" => handle_engines_subcommand(&parts[1..]),
        "cron" => handle_cron_subcommand(&parts[1..]),
        "memory" | "mem" => handle_memory_subcommand(&parts[1..]),
        "mcp" => handle_mcp_subcommand(&parts[1..]),
        "channels" | "channel" => handle_channels_subcommand(&parts[1..]),
        "analytics" | "usage" => match parts.get(1).copied() {
            None => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::ShowAnalytics(None),
            },
            Some(period @ ("day" | "week" | "month" | "all")) => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::ShowAnalytics(Some(period.to_string())),
            },
            Some(other) => CommandResponse {
                messages: vec![
                    format!("Unknown period: '{}'", other),
                    "Usage: /analytics [day|week|month|all]".to_string(),
                ],
                action: CommandAction::None,
            },
        },
        "logs" | "log" => {
            let source = parts.get(1).unwrap_or(&"gateway").to_string();
            let tail = parts.get(2).and_then(|s| s.parse::<usize>().ok());
            CommandResponse {
                messages: Vec::new(),
                action: CommandAction::ShowLogs(source, tail),
            }
        }
        "model" => match parts.get(1) {
            Some(name) => {
                let name = name.to_string();
                CommandResponse {
                    messages: vec![format!("Switching model to {}…", name)],
                    action: CommandAction::SetModel(name),
                }
            }
            None => {
                // Trigger an async fetch from the provider API so the user
                // sees the full, live model list (with pricing where available).
                CommandResponse {
                    messages: vec!["Fetching models from provider…".to_string()],
                    action: CommandAction::FetchModels,
                }
            }
        },
        "clawhub" | "hub" | "registry" => handle_clawhub_subcommand(&parts[1..], context),
        "thread" => handle_thread_subcommand(&parts[1..]),
        "q" | "quit" | "exit" => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::Quit,
        },
        _ => CommandResponse {
            messages: vec![
                format!("Unknown command: /{}", parts[0]),
                "Type /help for available commands".to_string(),
            ],
            action: CommandAction::None,
        },
    }
}

// ── Custom provider management ──────────────────────────────────────────────

const PROVIDER_ADD_USAGE: &str = "Usage: /provider add <id> <base_url> [format=openai|anthropic|gemini|xai] \
     [key=SECRET_NAME] [models=a,b,c] [name=Display Name]";

/// `/provider add <id> <base_url> [format=…] [key=…] [models=…] [name=…]`
fn handle_provider_add(args: &[&str], context: &mut CommandContext<'_>) -> CommandResponse {
    let (Some(id), Some(base_url)) = (args.first(), args.get(1)) else {
        return CommandResponse {
            messages: vec![
                PROVIDER_ADD_USAGE.to_string(),
                "Example: /provider add my-vllm http://192.168.1.50:8000/v1 models=qwen3-coder"
                    .to_string(),
            ],
            action: CommandAction::None,
        };
    };

    let mut cfg = crate::providers::CustomProviderConfig {
        id: id.to_string(),
        display_name: None,
        base_url: base_url.to_string(),
        api_format: crate::providers::ApiFormat::default(),
        api_key_secret: None,
        models: Vec::new(),
    };

    // Remaining tokens are key=value options; `name=` consumes the rest
    // of the line so display names can contain spaces.
    let mut i = 2;
    while i < args.len() {
        let token = args[i];
        if let Some(value) = token.strip_prefix("format=") {
            match crate::providers::ApiFormat::parse(value) {
                Some(f) => cfg.api_format = f,
                None => {
                    return CommandResponse {
                        messages: vec![format!(
                            "Unknown API format '{}'. Options: openai, anthropic, gemini, xai",
                            value
                        )],
                        action: CommandAction::None,
                    };
                }
            }
        } else if let Some(value) = token.strip_prefix("key=") {
            cfg.api_key_secret = Some(value.to_string());
        } else if let Some(value) = token.strip_prefix("models=") {
            cfg.models = value
                .split(',')
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect();
        } else if let Some(value) = token.strip_prefix("name=") {
            let mut name = value.to_string();
            for rest in &args[i + 1..] {
                name.push(' ');
                name.push_str(rest);
            }
            cfg.display_name = Some(name);
            break;
        } else {
            return CommandResponse {
                messages: vec![
                    format!("Unknown option '{}'.", token),
                    PROVIDER_ADD_USAGE.to_string(),
                ],
                action: CommandAction::None,
            };
        }
        i += 1;
    }

    if let Err(e) = cfg.validate() {
        return CommandResponse {
            messages: vec![format!("Invalid provider: {}", e)],
            action: CommandAction::None,
        };
    }

    let replacing = context
        .config
        .custom_providers
        .iter()
        .position(|p| p.id == cfg.id);
    let mut messages = Vec::new();
    match replacing {
        Some(idx) => {
            context.config.custom_providers[idx] = cfg.clone();
            messages.push(format!("Updated custom provider '{}'.", cfg.id));
        }
        None => {
            context.config.custom_providers.push(cfg.clone());
            messages.push(format!(
                "Added custom provider '{}' ({}).",
                cfg.id,
                cfg.api_format.display()
            ));
        }
    }

    if let Err(e) = context.config.save(None) {
        return CommandResponse {
            messages: vec![format!("Failed to save config: {}", e)],
            action: CommandAction::None,
        };
    }

    if let Some(secret) = &cfg.api_key_secret {
        messages.push(format!(
            "API key: store it with /secrets as '{}' (or export it as an environment variable).",
            secret
        ));
    }
    messages.push(format!("Switch to it with: /provider {}", cfg.id));
    CommandResponse {
        messages,
        action: CommandAction::None,
    }
}

/// `/provider remove <id>`
fn handle_provider_remove(args: &[&str], context: &mut CommandContext<'_>) -> CommandResponse {
    let Some(id) = args.first() else {
        return CommandResponse {
            messages: vec!["Usage: /provider remove <id>".to_string()],
            action: CommandAction::None,
        };
    };

    let before = context.config.custom_providers.len();
    context.config.custom_providers.retain(|p| p.id != *id);
    if context.config.custom_providers.len() == before {
        let hint = if crate::providers::PROVIDERS.iter().any(|p| p.id == *id) {
            format!(" ('{}' is a built-in provider and can't be removed)", id)
        } else {
            String::new()
        };
        return CommandResponse {
            messages: vec![format!("No custom provider named '{}'{}.", id, hint)],
            action: CommandAction::None,
        };
    }

    if let Err(e) = context.config.save(None) {
        return CommandResponse {
            messages: vec![format!("Failed to save config: {}", e)],
            action: CommandAction::None,
        };
    }

    let mut messages = vec![format!("Removed custom provider '{}'.", id)];
    let active = context
        .config
        .model
        .as_ref()
        .map(|m| m.provider.as_str())
        .unwrap_or("");
    if active == *id {
        messages
            .push("Note: it was the active provider — pick another with /provider.".to_string());
    }
    CommandResponse {
        messages,
        action: CommandAction::None,
    }
}

/// `/provider list`
fn handle_provider_list() -> CommandResponse {
    let mut lines = vec!["Providers:".to_string()];
    for p in crate::providers::PROVIDERS {
        lines.push(format!("  {} — {}", p.id, p.display));
    }
    let custom = crate::providers::custom_provider_defs();
    if custom.is_empty() {
        lines.push("No custom providers. Add one with /provider add <id> <base_url>".to_string());
    } else {
        lines.push("Custom providers:".to_string());
        for p in custom {
            lines.push(format!(
                "  {} — {} [{}]",
                p.id,
                p.display,
                p.base_url.unwrap_or("no URL")
            ));
        }
    }
    CommandResponse {
        messages: lines,
        action: CommandAction::None,
    }
}

// ── Engine commands ─────────────────────────────────────────────────────────

/// `/engines [start|stop|install|models|pull|load|unload|remove] …`
fn handle_engines_subcommand(args: &[&str]) -> CommandResponse {
    let usage = || CommandResponse {
        messages: vec![
            "Usage: /engines — open the engines panel".to_string(),
            "       /engines start|stop|install <engine>".to_string(),
            "       /engines models <engine>".to_string(),
            "       /engines pull <engine> <model>".to_string(),
            "       /engines load|unload|remove <engine> <model>".to_string(),
            "Engines: ollama, exo, llamacpp, lmstudio, joshua".to_string(),
        ],
        action: CommandAction::None,
    };

    match args.first() {
        None => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowEngines,
        },
        Some(&"models") => match args.get(1) {
            Some(engine) => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::EngineModelList(engine.to_string()),
            },
            None => usage(),
        },
        Some(&"pull") => match (args.get(1), args.get(2)) {
            (Some(engine), Some(model)) => CommandResponse {
                messages: vec![format!("Pulling {} via {}…", model, engine)],
                action: CommandAction::EngineModelPull(engine.to_string(), model.to_string()),
            },
            _ => usage(),
        },
        Some(&action) => {
            if let Ok(kind) = action.parse::<EngineActionKind>() {
                match args.get(1) {
                    Some(engine) => CommandResponse {
                        messages: vec![format!("Requesting {} {}…", engine, kind)],
                        action: CommandAction::EngineAction(engine.to_string(), kind),
                    },
                    None => usage(),
                }
            } else if let Ok(kind) = action.parse::<ModelActionKind>() {
                match (args.get(1), args.get(2)) {
                    (Some(engine), Some(model)) => CommandResponse {
                        messages: vec![format!("Requesting {} {} for {}…", engine, kind, model)],
                        action: CommandAction::EngineModelAction(
                            engine.to_string(),
                            model.to_string(),
                            kind,
                        ),
                    },
                    _ => usage(),
                }
            } else {
                usage()
            }
        }
    }
}

fn handle_cron_subcommand(args: &[&str]) -> CommandResponse {
    let usage = || CommandResponse {
        messages: vec![
            "Usage: /cron — open the scheduled-jobs panel".to_string(),
            "       /cron add <name> | <schedule> | <message>".to_string(),
            "         schedule: 'at <ISO-8601>', 'every <N>[ms|s|m|h]', or a 5-field cron expr"
                .to_string(),
            "       /cron pause|resume|rm <job-id>".to_string(),
        ],
        action: CommandAction::None,
    };

    match args.first() {
        None | Some(&"list") => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowCron,
        },
        Some(&"add") => {
            // Fields are pipe-separated so name/schedule/message can
            // contain spaces: /cron add Standup | every 1h | Time for standup
            let rest = args[1..].join(" ");
            let fields: Vec<&str> = rest.split('|').map(str::trim).collect();
            match fields.as_slice() {
                [name, expr, payload]
                    if !name.is_empty() && !expr.is_empty() && !payload.is_empty() =>
                {
                    CommandResponse {
                        messages: vec![format!("Creating job '{}'…", name)],
                        action: CommandAction::CronAdd(
                            name.to_string(),
                            expr.to_string(),
                            payload.to_string(),
                        ),
                    }
                }
                _ => usage(),
            }
        }
        Some(&action @ ("pause" | "resume" | "rm" | "remove")) => match args.get(1) {
            Some(id) => {
                let kind = match action {
                    "pause" => CronActionKind::Pause,
                    "resume" => CronActionKind::Resume,
                    _ => CronActionKind::Remove,
                };
                CommandResponse {
                    messages: Vec::new(),
                    action: CommandAction::CronAction(id.to_string(), kind),
                }
            }
            None => usage(),
        },
        Some(_) => usage(),
    }
}

fn handle_memory_subcommand(args: &[&str]) -> CommandResponse {
    let usage = || CommandResponse {
        messages: vec![
            "Usage: /memory [query] — browse MEMORY.md entries".to_string(),
            "       /memory add [category ::] <content>".to_string(),
            "       /memory rm <entry-id>".to_string(),
            "       /memory history <query> — search HISTORY.md".to_string(),
        ],
        action: CommandAction::None,
    };

    match args.first() {
        None => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowMemory(None),
        },
        Some(&"add") => {
            let rest = args[1..].join(" ");
            if rest.trim().is_empty() {
                return usage();
            }
            let (category, content) = match rest.split_once("::") {
                Some((cat, content)) if !cat.trim().is_empty() && !content.trim().is_empty() => {
                    (Some(cat.trim().to_string()), content.trim().to_string())
                }
                _ => (None, rest.trim().to_string()),
            };
            CommandResponse {
                messages: Vec::new(),
                action: CommandAction::MemoryAdd(category, content),
            }
        }
        Some(&"rm" | &"remove") => match args.get(1) {
            Some(id) => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::MemoryDelete(id.to_string()),
            },
            None => usage(),
        },
        Some(&"history") => {
            let query = args[1..].join(" ");
            if query.trim().is_empty() {
                usage()
            } else {
                CommandResponse {
                    messages: Vec::new(),
                    action: CommandAction::HistorySearch(query),
                }
            }
        }
        Some(_) => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowMemory(Some(args.join(" "))),
        },
    }
}

fn handle_mcp_subcommand(args: &[&str]) -> CommandResponse {
    let usage = || CommandResponse {
        messages: vec![
            "Usage: /mcp — open the MCP servers panel".to_string(),
            "       /mcp connect <name> [command…] — connect (and persist) a stdio server"
                .to_string(),
            "       /mcp disconnect <name>".to_string(),
        ],
        action: CommandAction::None,
    };

    match args.first() {
        None | Some(&"list") | Some(&"status") => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowMcp,
        },
        Some(&"connect") => match args.get(1) {
            Some(name) => {
                let command = if args.len() > 2 {
                    Some(args[2..].join(" "))
                } else {
                    None
                };
                CommandResponse {
                    messages: vec![format!("Connecting MCP server '{}'…", name)],
                    action: CommandAction::McpConnect(name.to_string(), command),
                }
            }
            None => usage(),
        },
        Some(&"disconnect") => match args.get(1) {
            Some(name) => CommandResponse {
                messages: Vec::new(),
                action: CommandAction::McpDisconnect(name.to_string()),
            },
            None => usage(),
        },
        Some(_) => usage(),
    }
}

fn handle_channels_subcommand(args: &[&str]) -> CommandResponse {
    let usage = || CommandResponse {
        messages: vec![
            "Usage: /channels — open the messenger channels panel".to_string(),
            "       /channels pair|unpair <name> — enable/disable a configured messenger"
                .to_string(),
        ],
        action: CommandAction::None,
    };

    match args.first() {
        None | Some(&"list") | Some(&"status") => CommandResponse {
            messages: Vec::new(),
            action: CommandAction::ShowChannels,
        },
        Some(&action @ ("pair" | "unpair")) => match args.get(1) {
            Some(name) => {
                let kind = if action == "pair" {
                    ChannelPairActionKind::Pair
                } else {
                    ChannelPairActionKind::Unpair
                };
                CommandResponse {
                    messages: Vec::new(),
                    action: CommandAction::ChannelPair(name.to_string(), kind),
                }
            }
            None => usage(),
        },
        Some(_) => usage(),
    }
}

mod subcommands;
use subcommands::{handle_clawhub_subcommand, handle_skill_subcommand, handle_thread_subcommand};
