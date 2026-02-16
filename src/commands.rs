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
    /// Reload gateway configuration
    GatewayReload,
    /// Download media by ID (id, optional destination path)
    Download(String, Option<String>),
    /// Toggle elevated (sudo) mode for execute_command
    SetElevated(bool),
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
        "skill info".into(),
        "skill remove".into(),
        "skill search".into(),
        "skill install".into(),
        "skill publish".into(),
        "skill link-secret".into(),
        "skill unlink-secret".into(),
        "secrets".into(),
        "elevated".into(),
        "elevated on".into(),
        "elevated off".into(),
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
        "help" => CommandResponse {
            messages: vec![
                "Available commands:".to_string(),
                "  /help                    - Show this help".to_string(),
                "  /clear                   - Clear messages and conversation memory".to_string(),
                "  /download <id> [path]    - Download media attachment to file".to_string(),
                "  /enable-access           - Enable agent access to secrets".to_string(),
                "  /disable-access          - Disable agent access to secrets".to_string(),
                "  /onboard                 - Run setup wizard (use CLI: rustyclaw onboard)".to_string(),
                "  /reload-skills           - Reload skills".to_string(),
                "  /gateway                 - Show gateway connection status".to_string(),
                "  /gateway start           - Connect to the gateway".to_string(),
                "  /gateway stop            - Disconnect from the gateway".to_string(),
                "  /gateway restart         - Restart the gateway connection".to_string(),
                "  /reload                  - Reload gateway config (no restart)".to_string(),
                "  /provider <name>         - Change the AI provider".to_string(),
                "  /model <name>            - Change the AI model".to_string(),
                "  /skills                  - Show loaded skills".to_string(),
                "  /skill                   - Skill management (info/install/publish/link)".to_string(),
                "  /secrets                 - Open the secrets vault".to_string(),
                "  /elevated <on|off>       - Toggle elevated (sudo) mode for commands".to_string(),
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
        },
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
            None => {
                CommandResponse {
                    messages: Vec::new(),
                    action: CommandAction::ShowProviderSelector,
                }
            }
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
        "elevated" => {
            if parts.len() < 2 {
                CommandResponse {
                    messages: vec![
                        "Usage: /elevated <on|off>".to_string(),
                        "Enable or disable elevated (sudo) mode for execute_command".to_string(),
                    ],
                    action: CommandAction::None,
                }
            } else {
                match parts[1] {
                    "on" => CommandResponse {
                        messages: vec!["⚠️  Elevated mode enabled. Commands will run with sudo.".to_string()],
                        action: CommandAction::SetElevated(true),
                    },
                    "off" => CommandResponse {
                        messages: vec!["✓ Elevated mode disabled.".to_string()],
                        action: CommandAction::SetElevated(false),
                    },
                    _ => CommandResponse {
                        messages: vec![
                            format!("Invalid option: {}", parts[1]),
                            "Usage: /elevated <on|off>".to_string(),
                        ],
                        action: CommandAction::None,
                    },
                }
            }
        },
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
                            format!(
                                "{} result(s) for '{}':",
                                results.len(),
                                query,
                            )
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
                    messages: vec![format!(
                        "Secret '{}' linked to skill '{}'.",
                        secret, skill,
                    )],
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
        Some(sub) => CommandResponse {
            messages: vec![
                format!("Unknown skill subcommand: {}", sub),
                "Usage: /skill info|remove|search|install|publish|link-secret|unlink-secret".to_string(),
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
                "  /skill link-secret <skill> <secret> — Link secret to skill".to_string(),
                "  /skill unlink-secret <skill> <secret> — Unlink secret".to_string(),
            ],
            action: CommandAction::None,
        },
    }
}
