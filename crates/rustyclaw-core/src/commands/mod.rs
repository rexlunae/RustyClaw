use crate::config::Config;
use crate::providers;
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    None,
    ClearMessages,
    Quit,
    /// Start (connect) the gateway
    GatewayStart,
    /// Stop (disconnect) the gateway
    GatewayStop,
    /// Restart the gateway connection
    GatewayRestart,
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
    /// Download media by ID (id, optional destination path)
    Download(String, Option<String>),
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
        "download".into(),
        "attach".into(),
        "attach file".into(),
        "attach dir".into(),
        "attach clear".into(),
        "enable-access".into(),
        "disable-access".into(),
        "onboard".into(),
        "reload-skills".into(),
        "gateway".into(),
        "gateway start".into(),
        "gateway stop".into(),
        "gateway restart".into(),
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
        "memory".into(),
        "analytics".into(),
        "logs".into(),
        "mcp".into(),
        "channels".into(),
        "approvals".into(),
        "engines".into(),
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
                "  /clear                   - Clear messages and conversation memory".to_string(),
                "  /download <id> [path]    - Download media attachment to file".to_string(),
                "  /attach file <path>      - Attach a file to the next prompt".to_string(),
                "  /attach dir <path>       - Attach a directory to the next prompt".to_string(),
                "  /attach clear            - Clear prompt attachments".to_string(),
                "  /enable-access           - Enable agent access to secrets".to_string(),
                "  /disable-access          - Disable agent access to secrets".to_string(),
                "  /onboard                 - Run setup wizard (use CLI: rustyclaw onboard)"
                    .to_string(),
                "  /reload-skills           - Reload skills".to_string(),
                "  /gateway                 - Show gateway connection status".to_string(),
                "  /gateway start           - Connect to the gateway".to_string(),
                "  /gateway stop            - Disconnect from the gateway".to_string(),
                "  /gateway restart         - Restart the gateway connection".to_string(),
                "  /reload                  - Reload gateway config (no restart)".to_string(),
                "  /provider <name>         - Change the AI provider".to_string(),
                "  /model <name>            - Change the AI model".to_string(),
                "  /skills                  - Show loaded skills".to_string(),
                "  /skill                   - Skill management (info/install/publish/link)"
                    .to_string(),
                "  /tools                   - Edit tool permissions (allow/deny/ask/skill)"
                    .to_string(),
                "  /secrets                 - Open the secrets vault".to_string(),
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
            ],
            action: CommandAction::None,
        },
        "clear" => CommandResponse {
            messages: vec!["Messages and conversation memory cleared.".to_string()],
            action: CommandAction::ClearMessages,
        },
        "download" => {
            if parts.len() < 2 {
                CommandResponse {
                    messages: vec![
                        "Usage: /download <media_id> [destination_path]".to_string(),
                        "Example: /download media_0001".to_string(),
                        "Example: /download media_0001 ~/Downloads/image.jpg".to_string(),
                    ],
                    action: CommandAction::None,
                }
            } else {
                let media_id = parts[1].to_string();
                let dest_path = parts.get(2).map(|s| s.to_string());
                CommandResponse {
                    messages: vec![format!("Downloading {}...", media_id)],
                    action: CommandAction::Download(media_id, dest_path),
                }
            }
        }
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
            Some("start") => CommandResponse {
                messages: vec!["Starting gateway connection…".to_string()],
                action: CommandAction::GatewayStart,
            },
            Some("stop") => CommandResponse {
                messages: vec!["Stopping gateway connection…".to_string()],
                action: CommandAction::GatewayStop,
            },
            Some("restart") => CommandResponse {
                messages: vec!["Restarting gateway connection…".to_string()],
                action: CommandAction::GatewayRestart,
            },
            Some(sub) => CommandResponse {
                messages: vec![
                    format!("Unknown gateway subcommand: {}", sub),
                    "Usage: /gateway start|stop|restart".to_string(),
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

mod subcommands;
use subcommands::{handle_clawhub_subcommand, handle_skill_subcommand, handle_thread_subcommand};
