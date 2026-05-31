//! `/skill`, `/thread`, and `/clawhub` slash-subcommand handlers.

#![allow(unused_imports)]
use super::*;

pub(crate) fn handle_skill_subcommand(
    parts: &[&str],
    context: &mut CommandContext<'_>,
) -> CommandResponse {
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

pub(crate) fn handle_thread_subcommand(parts: &[&str]) -> CommandResponse {
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
        Some("close") => {
            let id_str = parts.get(1).copied().unwrap_or("");
            match id_str.parse::<u64>() {
                Ok(id) => CommandResponse {
                    messages: vec![format!("Closing thread {}...", id)],
                    action: CommandAction::ThreadClose(id),
                },
                Err(_) => CommandResponse {
                    messages: vec![
                        "Usage: /thread close <id>".to_string(),
                        "Get thread IDs from /thread list or sidebar.".to_string(),
                    ],
                    action: CommandAction::None,
                },
            }
        }
        Some("rename") => {
            let id_str = parts.get(1).copied().unwrap_or("");
            let new_label = parts.get(2..).map(|p| p.join(" ")).unwrap_or_default();
            match id_str.parse::<u64>() {
                Ok(id) if !new_label.is_empty() => CommandResponse {
                    messages: vec![format!("Renaming thread {} to '{}'...", id, new_label)],
                    action: CommandAction::ThreadRename(id, new_label),
                },
                _ => CommandResponse {
                    messages: vec![
                        "Usage: /thread rename <id> <new_label>".to_string(),
                        "Example: /thread rename 1234567890 Fix CSS bugs".to_string(),
                    ],
                    action: CommandAction::None,
                },
            }
        }
        Some("list") | None => CommandResponse {
            messages: vec!["Press Tab to focus sidebar and view threads.".to_string()],
            action: CommandAction::ThreadList,
        },
        Some("bg") | Some("background") => CommandResponse {
            messages: vec!["Backgrounding current thread…".to_string()],
            action: CommandAction::ThreadBackground,
        },
        Some("fg") | Some("foreground") => {
            let id_str = parts.get(1).copied().unwrap_or("");
            match id_str.parse::<u64>() {
                Ok(0) => CommandResponse {
                    messages: vec!["Thread ID 0 is reserved. Use a valid thread ID.".to_string()],
                    action: CommandAction::None,
                },
                Ok(id) => CommandResponse {
                    messages: vec![format!("Foregrounding thread {}…", id)],
                    action: CommandAction::ThreadForeground(id),
                },
                Err(_) => CommandResponse {
                    messages: vec![
                        "Usage: /thread fg <id>".to_string(),
                        "Get thread IDs from /thread list or sidebar.".to_string(),
                    ],
                    action: CommandAction::None,
                },
            }
        }
        Some(sub) => CommandResponse {
            messages: vec![
                format!("Unknown thread subcommand: {}", sub),
                "Available: new, list, close, rename, bg, fg".to_string(),
            ],
            action: CommandAction::None,
        },
    }
}

pub(crate) fn handle_clawhub_subcommand(
    parts: &[&str],
    context: &mut CommandContext<'_>,
) -> CommandResponse {
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
                        if let Err(e) = context.config.save(None) {
                            tracing::warn!("failed to persist config: {e}");
                        }
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
                if let Err(e) = context.config.save(None) {
                    tracing::warn!("failed to persist config: {e}");
                }
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
