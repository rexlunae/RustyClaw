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
    /// Download media by ID (id, optional destination path)
    Download(String, Option<String>),
    /// Create a new thread
    ThreadNew(String),
    /// List threads (handled in TUI)
    ThreadList,
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

/// List of all known command names (without the / prefix).
/// Includes subcommand forms so tab-completion works for them.
pub fn command_names() -> Vec<String> {
    let mut names: Vec<String> = vec![
        "help".into(),
        "clear".into(),
        "download".into(),
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
    ];
    for p in providers::provider_ids() {
        names.push(format!("provider {}", p));
    }
    for m in providers::all_model_names() {
        names.push(format!("model {}", m));
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
        "enable-access" => {
            context.secrets_manager.set_agent_access(true);
            context.config.agent_access = true;
            let _ = context.config.save(None);
            CommandResponse {
                messages: vec!["Agent access to secrets enabled.".to_string()],
                action: CommandAction::None,
            }
        }
        "disable-access" => {
            context.secrets_manager.set_agent_access(false);
            context.config.agent_access = false;
            let _ = context.config.save(None);
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
                let list = providers::all_model_names().join(", ");
                CommandResponse {
                    messages: vec![
                        "Usage: /model <name>".to_string(),
                        format!("Known models: {}", list),
                    ],
                    action: CommandAction::None,
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

fn handle_skill_subcommand(parts: &[&str], context: &mut CommandContext<'_>) -> CommandResponse {
    match parts.first().copied() {
        Some("info") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill info <name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.skill_info(name) {
                Some(info) => CommandResponse {
                    messages: vec![info],
                    action: CommandAction::None,
                },
                None => CommandResponse {
                    messages: vec![format!("Skill '{}' not found.", name)],
                    action: CommandAction::None,
                },
            }
        }
        Some("remove") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill remove <name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.remove_skill(name) {
                Ok(()) => CommandResponse {
                    messages: vec![format!("Skill '{}' removed.", name)],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![e.to_string()],
                    action: CommandAction::None,
                },
            }
        }
        Some("search") => {
            let query = parts[1..].join(" ");
            if query.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill search <query>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.search_registry(&query) {
                Ok(results) => {
                    if results.is_empty() {
                        CommandResponse {
                            messages: vec![format!("No skills found matching '{}'.", query)],
                            action: CommandAction::None,
                        }
                    } else {
                        let has_local = results.iter().any(|r| r.version == "local");
                        let header = if has_local {
                            format!(
                                "{} local skill(s) matching '{}' (registry offline):",
                                results.len(),
                                query,
                            )
                        } else {
                            format!("{} result(s) for '{}':", results.len(), query,)
                        };
                        let mut msgs: Vec<String> = vec![header];
                        for r in &results {
                            msgs.push(format!(
                                "  • {} v{} by {} — {}",
                                r.name, r.version, r.author, r.description,
                            ));
                        }
                        CommandResponse {
                            messages: msgs,
                            action: CommandAction::None,
                        }
                    }
                }
                Err(e) => CommandResponse {
                    messages: vec![format!("Registry search failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("install") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill install <name> [version]".to_string()],
                    action: CommandAction::None,
                };
            }
            let version = parts.get(2).copied();
            match context.skill_manager.install_from_registry(name, version) {
                Ok(skill) => {
                    let _ = context.skill_manager.load_skills();
                    CommandResponse {
                        messages: vec![format!("Skill '{}' installed from ClawHub.", skill.name)],
                        action: CommandAction::None,
                    }
                }
                Err(e) => CommandResponse {
                    messages: vec![format!("Install failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("publish") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill publish <name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.publish_to_registry(name) {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Publish failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("link-secret") => {
            let skill = parts.get(1).copied().unwrap_or("");
            let secret = parts.get(2).copied().unwrap_or("");
            if skill.is_empty() || secret.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill link-secret <skill> <secret>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.link_secret(skill, secret) {
                Ok(_) => CommandResponse {
                    messages: vec![format!("Secret '{}' linked to skill '{}'.", secret, skill,)],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Link failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("unlink-secret") => {
            let skill = parts.get(1).copied().unwrap_or("");
            let secret = parts.get(2).copied().unwrap_or("");
            if skill.is_empty() || secret.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /skill unlink-secret <skill> <secret>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.unlink_secret(skill, secret) {
                Ok(_) => CommandResponse {
                    messages: vec![format!(
                        "Secret '{}' unlinked from skill '{}'.",
                        secret, skill,
                    )],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Unlink failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("create") => {
            // /skill create <name> <description>
            // Instructions are sent to the agent as a follow-up message
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec![
                        "Usage: /skill create <name> <one-line description>".to_string(),
                        "".to_string(),
                        "This creates an empty skill scaffold. To have the agent write a full"
                            .to_string(),
                        "skill from a prompt, just ask: \"Create a skill that deploys to S3\""
                            .to_string(),
                    ],
                    action: CommandAction::None,
                };
            }
            let description = if parts.len() > 2 {
                parts[2..].join(" ")
            } else {
                format!("A skill called {}", name)
            };
            let instructions = format!("# {}\n\nTODO: Add instructions for this skill.", name);
            match context
                .skill_manager
                .create_skill(name, &description, &instructions, None)
            {
                Ok(path) => CommandResponse {
                    messages: vec![
                        format!("✅ Skill '{}' created at {}", name, path.display()),
                        "Edit the SKILL.md to add instructions, or ask the agent to fill it in."
                            .to_string(),
                    ],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Create failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some(sub) => CommandResponse {
            messages: vec![
                format!("Unknown skill subcommand: {}", sub),
                "Usage: /skill info|remove|search|install|publish|create|link-secret|unlink-secret"
                    .to_string(),
            ],
            action: CommandAction::None,
        },
        None => CommandResponse {
            messages: vec![
                "Skill commands:".to_string(),
                "  /skill info <name>                 — Show skill details".to_string(),
                "  /skill remove <name>               — Remove a skill".to_string(),
                "  /skill search <query>              — Search ClawHub registry".to_string(),
                "  /skill install <name> [version]    — Install from ClawHub".to_string(),
                "  /skill publish <name>              — Publish to ClawHub".to_string(),
                "  /skill create <name> [description] — Create a new skill".to_string(),
                "  /skill link-secret <skill> <secret> — Link secret to skill".to_string(),
                "  /skill unlink-secret <skill> <secret> — Unlink secret".to_string(),
            ],
            action: CommandAction::None,
        },
    }
}

fn handle_thread_subcommand(parts: &[&str]) -> CommandResponse {
    match parts.first().copied() {
        Some("new") => {
            let label = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
            if label.is_empty() {
                CommandResponse {
                    messages: vec!["Usage: /thread new <label>".to_string()],
                    action: CommandAction::None,
                }
            } else {
                CommandResponse {
                    messages: vec![format!("Creating thread '{}'...", label)],
                    action: CommandAction::ThreadNew(label),
                }
            }
        }
        Some("list") | None => CommandResponse {
            messages: vec!["Press Tab to focus sidebar and view threads.".to_string()],
            action: CommandAction::ThreadList,
        },
        Some(sub) => CommandResponse {
            messages: vec![
                format!("Unknown thread subcommand: {}", sub),
                "Available: /thread new <label>, /thread list".to_string(),
            ],
            action: CommandAction::None,
        },
    }
}

fn handle_clawhub_subcommand(parts: &[&str], context: &mut CommandContext<'_>) -> CommandResponse {
    match parts.first().copied() {
        Some("auth") => match parts.get(1).copied() {
            Some("login") => {
                // Token-based login: /clawhub auth login <token>
                let token = parts.get(2).copied().unwrap_or("");
                if token.is_empty() {
                    return CommandResponse {
                        messages: vec![
                            "Usage: /clawhub auth login <api_token>".to_string(),
                            "Get your token at https://clawhub.ai/settings/tokens".to_string(),
                        ],
                        action: CommandAction::None,
                    };
                }
                match context.skill_manager.auth_token(token) {
                    Ok(resp) if resp.ok => {
                        // Store token in config
                        context.config.clawhub_token = Some(token.to_string());
                        let _ = context.config.save(None);
                        let url = context.skill_manager.registry_url().to_string();
                        context
                            .skill_manager
                            .set_registry(&url, Some(token.to_string()));
                        let user = resp.username.unwrap_or_else(|| "unknown".into());
                        CommandResponse {
                            messages: vec![format!("✓ Authenticated as '{}' on ClawHub.", user)],
                            action: CommandAction::None,
                        }
                    }
                    Ok(_) => CommandResponse {
                        messages: vec!["✗ Token is invalid.".to_string()],
                        action: CommandAction::None,
                    },
                    Err(e) => CommandResponse {
                        messages: vec![format!("✗ Auth failed: {}", e)],
                        action: CommandAction::None,
                    },
                }
            }
            Some("status") => match context.skill_manager.auth_status() {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Auth status check failed: {}", e)],
                    action: CommandAction::None,
                },
            },
            Some("logout") => {
                context.config.clawhub_token = None;
                let _ = context.config.save(None);
                let url = context.skill_manager.registry_url().to_string();
                context.skill_manager.set_registry(&url, None);
                CommandResponse {
                    messages: vec!["Logged out from ClawHub.".to_string()],
                    action: CommandAction::None,
                }
            }
            Some(sub) => CommandResponse {
                messages: vec![
                    format!("Unknown auth subcommand: {}", sub),
                    "Usage: /clawhub auth login|status|logout".to_string(),
                ],
                action: CommandAction::None,
            },
            None => CommandResponse {
                messages: vec![
                    "ClawHub auth commands:".to_string(),
                    "  /clawhub auth login <token>  — Authenticate with API token".to_string(),
                    "  /clawhub auth status         — Show auth status".to_string(),
                    "  /clawhub auth logout         — Remove stored credentials".to_string(),
                ],
                action: CommandAction::None,
            },
        },
        Some("search") => {
            let query = parts[1..].join(" ");
            if query.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /clawhub search <query>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.search_registry(&query) {
                Ok(results) => {
                    if results.is_empty() {
                        CommandResponse {
                            messages: vec![format!("No skills found matching '{}'.", query)],
                            action: CommandAction::None,
                        }
                    } else {
                        let mut msgs =
                            vec![format!("{} result(s) for '{}':", results.len(), query)];
                        for r in &results {
                            let dl = if r.downloads > 0 {
                                format!(" (↓{})", r.downloads)
                            } else {
                                String::new()
                            };
                            msgs.push(format!(
                                "  • {} v{} by {} — {}{}",
                                r.name, r.version, r.author, r.description, dl,
                            ));
                        }
                        msgs.push(String::new());
                        msgs.push("Install with: /clawhub install <name>".to_string());
                        CommandResponse {
                            messages: msgs,
                            action: CommandAction::None,
                        }
                    }
                }
                Err(e) => CommandResponse {
                    messages: vec![format!("Search failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("trending") => {
            let category = parts.get(1).copied();
            match context.skill_manager.trending(category, Some(15)) {
                Ok(entries) => {
                    if entries.is_empty() {
                        CommandResponse {
                            messages: vec!["No trending skills found.".to_string()],
                            action: CommandAction::None,
                        }
                    } else {
                        let header = match category {
                            Some(cat) => format!("Trending skills in '{}':", cat),
                            None => "Trending skills on ClawHub:".to_string(),
                        };
                        let mut msgs = vec![header];
                        for (i, e) in entries.iter().enumerate() {
                            msgs.push(format!(
                                "  {}. {} — {} (★{} ↓{})",
                                i + 1,
                                e.name,
                                e.description,
                                e.stars,
                                e.downloads,
                            ));
                        }
                        CommandResponse {
                            messages: msgs,
                            action: CommandAction::None,
                        }
                    }
                }
                Err(e) => CommandResponse {
                    messages: vec![format!("Failed to fetch trending: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("categories" | "cats") => match context.skill_manager.categories() {
            Ok(cats) => {
                if cats.is_empty() {
                    CommandResponse {
                        messages: vec!["No categories found.".to_string()],
                        action: CommandAction::None,
                    }
                } else {
                    let mut msgs = vec!["ClawHub skill categories:".to_string()];
                    for c in &cats {
                        let count = if c.count > 0 {
                            format!(" ({})", c.count)
                        } else {
                            String::new()
                        };
                        msgs.push(format!("  • {}{} — {}", c.name, count, c.description));
                    }
                    msgs.push(String::new());
                    msgs.push("Browse by category: /clawhub trending <category>".to_string());
                    CommandResponse {
                        messages: msgs,
                        action: CommandAction::None,
                    }
                }
            }
            Err(e) => CommandResponse {
                messages: vec![format!("Failed to fetch categories: {}", e)],
                action: CommandAction::None,
            },
        },
        Some("info") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /clawhub info <skill_name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.registry_info(name) {
                Ok(detail) => {
                    let mut msgs = vec![format!("{}  v{}", detail.name, detail.version)];
                    if !detail.description.is_empty() {
                        msgs.push(format!("  {}", detail.description));
                    }
                    if !detail.author.is_empty() {
                        msgs.push(format!("  Author: {}", detail.author));
                    }
                    if !detail.license.is_empty() {
                        msgs.push(format!("  License: {}", detail.license));
                    }
                    msgs.push(format!("  ★ {}  ↓ {}", detail.stars, detail.downloads));
                    if let Some(ref repo) = detail.repository {
                        msgs.push(format!("  Repo: {}", repo));
                    }
                    if !detail.categories.is_empty() {
                        msgs.push(format!("  Categories: {}", detail.categories.join(", ")));
                    }
                    if !detail.required_secrets.is_empty() {
                        msgs.push(format!(
                            "  Requires secrets: {}",
                            detail.required_secrets.join(", ")
                        ));
                    }
                    if !detail.updated_at.is_empty() {
                        msgs.push(format!("  Updated: {}", detail.updated_at));
                    }
                    CommandResponse {
                        messages: msgs,
                        action: CommandAction::None,
                    }
                }
                Err(e) => CommandResponse {
                    messages: vec![format!("Failed to fetch skill info: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("browse" | "open") => {
            let url = context.skill_manager.registry_url();
            // Try to open in default browser
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(url).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", url])
                .spawn();
            CommandResponse {
                messages: vec![format!("Opening {} in your browser…", url)],
                action: CommandAction::None,
            }
        }
        Some("profile" | "me") => match context.skill_manager.profile() {
            Ok(p) => {
                let mut msgs = vec![format!("ClawHub profile: {}", p.username)];
                if !p.display_name.is_empty() {
                    msgs.push(format!("  Name: {}", p.display_name));
                }
                if !p.bio.is_empty() {
                    msgs.push(format!("  Bio: {}", p.bio));
                }
                msgs.push(format!(
                    "  Published: {}  Starred: {}",
                    p.published_count, p.starred_count
                ));
                if !p.joined.is_empty() {
                    msgs.push(format!("  Joined: {}", p.joined));
                }
                CommandResponse {
                    messages: msgs,
                    action: CommandAction::None,
                }
            }
            Err(e) => CommandResponse {
                messages: vec![format!("Failed to fetch profile: {}", e)],
                action: CommandAction::None,
            },
        },
        Some("starred" | "stars") => match context.skill_manager.starred() {
            Ok(entries) => {
                if entries.is_empty() {
                    CommandResponse {
                        messages: vec![
                            "No starred skills. Star skills with: /clawhub star <name>".to_string(),
                        ],
                        action: CommandAction::None,
                    }
                } else {
                    let mut msgs = vec![format!("{} starred skill(s):", entries.len())];
                    for e in &entries {
                        msgs.push(format!(
                            "  ★ {} v{} by {} — {}",
                            e.name, e.version, e.author, e.description,
                        ));
                    }
                    CommandResponse {
                        messages: msgs,
                        action: CommandAction::None,
                    }
                }
            }
            Err(e) => CommandResponse {
                messages: vec![format!("Failed to fetch starred skills: {}", e)],
                action: CommandAction::None,
            },
        },
        Some("star") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /clawhub star <skill_name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.star(name) {
                Ok(msg) => CommandResponse {
                    messages: vec![format!("★ {}", msg)],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Star failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("unstar") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /clawhub unstar <skill_name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.unstar(name) {
                Ok(msg) => CommandResponse {
                    messages: vec![msg],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Unstar failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("install") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /clawhub install <name> [version]".to_string()],
                    action: CommandAction::None,
                };
            }
            let version = parts.get(2).copied();
            match context.skill_manager.install_from_registry(name, version) {
                Ok(skill) => {
                    let _ = context.skill_manager.load_skills();
                    CommandResponse {
                        messages: vec![format!("✓ Skill '{}' installed from ClawHub.", skill.name)],
                        action: CommandAction::None,
                    }
                }
                Err(e) => CommandResponse {
                    messages: vec![format!("Install failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some("publish") => {
            let name = parts.get(1).copied().unwrap_or("");
            if name.is_empty() {
                return CommandResponse {
                    messages: vec!["Usage: /clawhub publish <name>".to_string()],
                    action: CommandAction::None,
                };
            }
            match context.skill_manager.publish_to_registry(name) {
                Ok(msg) => CommandResponse {
                    messages: vec![format!("✓ {}", msg)],
                    action: CommandAction::None,
                },
                Err(e) => CommandResponse {
                    messages: vec![format!("Publish failed: {}", e)],
                    action: CommandAction::None,
                },
            }
        }
        Some(sub) => CommandResponse {
            messages: vec![
                format!("Unknown clawhub subcommand: {}", sub),
                "Type /clawhub for available commands.".to_string(),
            ],
            action: CommandAction::None,
        },
        None => CommandResponse {
            messages: vec![
                "ClawHub — Skill Registry".to_string(),
                format!("  Registry: {}", context.skill_manager.registry_url()),
                String::new(),
                "  /clawhub auth                — Authentication commands".to_string(),
                "  /clawhub search <query>      — Search for skills".to_string(),
                "  /clawhub trending [category] — Browse trending skills".to_string(),
                "  /clawhub categories          — List skill categories".to_string(),
                "  /clawhub info <name>         — Show skill details".to_string(),
                "  /clawhub browse              — Open ClawHub in browser".to_string(),
                "  /clawhub profile             — Show your profile".to_string(),
                "  /clawhub starred             — List your starred skills".to_string(),
                "  /clawhub star <name>         — Star a skill".to_string(),
                "  /clawhub unstar <name>       — Unstar a skill".to_string(),
                "  /clawhub install <name>      — Install a skill".to_string(),
                "  /clawhub publish <name>      — Publish a local skill".to_string(),
            ],
            action: CommandAction::None,
        },
    }
}
