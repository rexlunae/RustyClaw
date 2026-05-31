//! `clawhub` command: browse/auth/install skills from the ClawHub registry.

use anyhow::Result;
use clap::{Args, Subcommand};

use rustyclaw_core::config::Config;
use rustyclaw_core::skills::SkillManager;

#[derive(Debug, Args)]
pub(crate) struct ClawHubCommands {
    #[command(subcommand)]
    command: Option<ClawHubSub>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ClawHubSub {
    /// Authenticate with ClawHub
    #[command(subcommand)]
    Auth(ClawHubAuthCommands),

    /// Search skills on ClawHub
    Search {
        /// Search query
        #[arg(value_name = "QUERY", trailing_var_arg = true)]
        query: Vec<String>,
    },

    /// Show trending / popular skills
    Trending {
        /// Filter by category
        #[arg(value_name = "CATEGORY")]
        category: Option<String>,
        /// Max results to show
        #[arg(long, short = 'n', default_value_t = 15)]
        limit: usize,
    },

    /// List skill categories
    Categories,

    /// Show detailed info about a registry skill
    Info {
        /// Skill name
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Open ClawHub in your browser
    Browse,

    /// Show your ClawHub profile
    Profile,

    /// List your starred skills
    Starred,

    /// Star a skill
    Star {
        /// Skill name to star
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Unstar a skill
    Unstar {
        /// Skill name to unstar
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Install a skill from ClawHub
    Install {
        /// Skill name
        #[arg(value_name = "NAME")]
        name: String,
        /// Version to install (default: latest)
        #[arg(value_name = "VERSION")]
        version: Option<String>,
    },

    /// Publish a local skill to ClawHub
    Publish {
        /// Skill name to publish
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ClawHubAuthCommands {
    /// Authenticate with an API token
    Login {
        /// API token from https://clawhub.ai/settings/tokens
        #[arg(value_name = "TOKEN")]
        token: String,
    },
    /// Show authentication status
    Status,
    /// Remove stored credentials
    Logout,
}

/// Run a `clawhub` subcommand.
pub(crate) fn run(args: ClawHubCommands, config: &mut Config) -> Result<()> {
    use rustyclaw_core::theme as t;

    // Use consolidated skills_dirs from config
    let skills_dirs = config.skills_dirs();

    let mut sm = SkillManager::with_dirs(skills_dirs);
    sm.load_skills()?;
    if let Some(url) = config.clawhub_url.as_deref() {
        sm.set_registry(url, config.clawhub_token.clone());
    } else if let Some(ref token) = config.clawhub_token {
        let url = sm.registry_url().to_string();
        sm.set_registry(&url, Some(token.clone()));
    }

    match args.command {
        None => {
            // No subcommand: show overview
            println!("{}", t::accent_bright("ClawHub — Skill Registry"));
            println!("  Registry: {}", t::info(sm.registry_url()));
            match sm.auth_status() {
                Ok(status) => println!("  Auth: {}", status),
                Err(_) => println!("  Auth: {}", t::muted("unknown")),
            }
            println!();
            println!(
                "  {} search <query>        Search for skills",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} trending [category]   Browse trending skills",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} categories            List skill categories",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} info <name>           Show skill details",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} browse                Open ClawHub in browser",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} auth login <token>    Authenticate",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} profile               Show your profile",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} starred                List starred skills",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} install <name>        Install a skill",
                t::muted("rustyclaw clawhub")
            );
            println!(
                "  {} publish <name>        Publish a skill",
                t::muted("rustyclaw clawhub")
            );
        }
        Some(ClawHubSub::Auth(auth_cmd)) => match auth_cmd {
            ClawHubAuthCommands::Login { token } => match sm.auth_token(&token) {
                Ok(resp) if resp.ok => {
                    config.clawhub_token = Some(token);
                    config.save(None)?;
                    let user = resp.username.unwrap_or_else(|| "unknown".into());
                    println!(
                        "{}",
                        t::icon_ok(&format!("Authenticated as '{}' on ClawHub.", user))
                    );
                }
                Ok(_) => {
                    println!("{}", t::icon_fail("Token is invalid."));
                    std::process::exit(1);
                }
                Err(e) => {
                    println!("{}", t::icon_fail(&format!("Auth failed: {}", e)));
                    std::process::exit(1);
                }
            },
            ClawHubAuthCommands::Status => match sm.auth_status() {
                Ok(msg) => println!("{}", msg),
                Err(e) => println!(
                    "{}",
                    t::icon_fail(&format!("Auth status check failed: {}", e))
                ),
            },
            ClawHubAuthCommands::Logout => {
                config.clawhub_token = None;
                config.save(None)?;
                println!("{}", t::icon_ok("Logged out from ClawHub."));
            }
        },
        Some(ClawHubSub::Search { query }) => {
            let q = query.join(" ");
            if q.is_empty() {
                println!(
                    "{}",
                    t::icon_fail("Usage: rustyclaw clawhub search <query>")
                );
                std::process::exit(1);
            }
            match sm.search_registry(&q) {
                Ok(results) => {
                    if results.is_empty() {
                        println!(
                            "{}",
                            t::muted(&format!("No skills found matching '{}'.", q))
                        );
                    } else {
                        println!("{} result(s) for '{}':", results.len(), q);
                        for r in &results {
                            let dl = if r.downloads > 0 {
                                format!(" (↓{})", r.downloads)
                            } else {
                                String::new()
                            };
                            println!(
                                "  {} {} v{} by {} — {}{}",
                                t::icon_ok(""),
                                r.name,
                                r.version,
                                r.author,
                                r.description,
                                dl,
                            );
                        }
                        println!();
                        println!(
                            "Install with: {} install <name>",
                            t::muted("rustyclaw clawhub")
                        );
                    }
                }
                Err(e) => {
                    println!("{}", t::icon_fail(&format!("Search failed: {}", e)));
                    std::process::exit(1);
                }
            }
        }
        Some(ClawHubSub::Trending { category, limit }) => {
            match sm.trending(category.as_deref(), Some(limit)) {
                Ok(entries) => {
                    if entries.is_empty() {
                        println!("{}", t::muted("No trending skills found."));
                    } else {
                        let header = match &category {
                            Some(cat) => format!("Trending skills in '{}':", cat),
                            None => "Trending skills on ClawHub:".into(),
                        };
                        println!("{}", t::accent_bright(&header));
                        for (i, e) in entries.iter().enumerate() {
                            println!(
                                "  {}. {} — {} (★{} ↓{})",
                                i + 1,
                                e.name,
                                e.description,
                                e.stars,
                                e.downloads,
                            );
                        }
                    }
                }
                Err(e) => {
                    println!(
                        "{}",
                        t::icon_fail(&format!("Failed to fetch trending: {}", e))
                    );
                    std::process::exit(1);
                }
            }
        }
        Some(ClawHubSub::Categories) => match sm.categories() {
            Ok(cats) => {
                if cats.is_empty() {
                    println!("{}", t::muted("No categories found."));
                } else {
                    println!("{}", t::accent_bright("ClawHub skill categories:"));
                    for c in &cats {
                        let count = if c.count > 0 {
                            format!(" ({})", c.count)
                        } else {
                            String::new()
                        };
                        println!("  • {}{} — {}", c.name, count, c.description);
                    }
                    println!();
                    println!(
                        "Browse by category: {} trending <category>",
                        t::muted("rustyclaw clawhub")
                    );
                }
            }
            Err(e) => {
                println!(
                    "{}",
                    t::icon_fail(&format!("Failed to fetch categories: {}", e))
                );
                std::process::exit(1);
            }
        },
        Some(ClawHubSub::Info { name }) => match sm.registry_info(&name) {
            Ok(detail) => {
                println!(
                    "{}",
                    t::accent_bright(&format!("{}  v{}", detail.name, detail.version))
                );
                if !detail.description.is_empty() {
                    println!("  {}", detail.description);
                }
                if !detail.author.is_empty() {
                    println!("  Author: {}", detail.author);
                }
                if !detail.license.is_empty() {
                    println!("  License: {}", detail.license);
                }
                println!("  ★ {}  ↓ {}", detail.stars, detail.downloads);
                if let Some(ref repo) = detail.repository {
                    println!("  Repo: {}", repo);
                }
                if !detail.categories.is_empty() {
                    println!("  Categories: {}", detail.categories.join(", "));
                }
                if !detail.required_secrets.is_empty() {
                    println!("  Requires secrets: {}", detail.required_secrets.join(", "));
                }
                if !detail.updated_at.is_empty() {
                    println!("  Updated: {}", detail.updated_at);
                }
            }
            Err(e) => {
                println!(
                    "{}",
                    t::icon_fail(&format!("Failed to fetch skill info: {}", e))
                );
                std::process::exit(1);
            }
        },
        Some(ClawHubSub::Browse) => {
            let url = sm.registry_url().to_string();
            println!("Opening {} …", t::info(&url));
            #[cfg(target_os = "macos")]
            let _ = std::process::Command::new("open").arg(&url).spawn();
            #[cfg(target_os = "linux")]
            let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", &url])
                .spawn();
        }
        Some(ClawHubSub::Profile) => match sm.profile() {
            Ok(p) => {
                println!(
                    "{}",
                    t::accent_bright(&format!("ClawHub profile: {}", p.username))
                );
                if !p.display_name.is_empty() {
                    println!("  Name: {}", p.display_name);
                }
                if !p.bio.is_empty() {
                    println!("  Bio: {}", p.bio);
                }
                println!(
                    "  Published: {}  Starred: {}",
                    p.published_count, p.starred_count
                );
                if !p.joined.is_empty() {
                    println!("  Joined: {}", p.joined);
                }
            }
            Err(e) => {
                println!(
                    "{}",
                    t::icon_fail(&format!("Failed to fetch profile: {}", e))
                );
                std::process::exit(1);
            }
        },
        Some(ClawHubSub::Starred) => match sm.starred() {
            Ok(entries) => {
                if entries.is_empty() {
                    println!("{}", t::muted("No starred skills."));
                    println!(
                        "Star skills with: {} star <name>",
                        t::muted("rustyclaw clawhub")
                    );
                } else {
                    println!("{} starred skill(s):", entries.len());
                    for e in &entries {
                        println!(
                            "  ★ {} v{} by {} — {}",
                            e.name, e.version, e.author, e.description,
                        );
                    }
                }
            }
            Err(e) => {
                println!(
                    "{}",
                    t::icon_fail(&format!("Failed to fetch starred: {}", e))
                );
                std::process::exit(1);
            }
        },
        Some(ClawHubSub::Star { name }) => match sm.star(&name) {
            Ok(msg) => println!("{}", t::icon_ok(&msg)),
            Err(e) => {
                println!("{}", t::icon_fail(&format!("Star failed: {}", e)));
                std::process::exit(1);
            }
        },
        Some(ClawHubSub::Unstar { name }) => match sm.unstar(&name) {
            Ok(msg) => println!("{}", t::icon_ok(&msg)),
            Err(e) => {
                println!("{}", t::icon_fail(&format!("Unstar failed: {}", e)));
                std::process::exit(1);
            }
        },
        Some(ClawHubSub::Install { name, version }) => {
            match sm.install_from_registry(&name, version.as_deref()) {
                Ok(skill) => {
                    println!(
                        "{}",
                        t::icon_ok(&format!("Skill '{}' installed from ClawHub.", skill.name))
                    );
                }
                Err(e) => {
                    println!("{}", t::icon_fail(&format!("Install failed: {}", e)));
                    std::process::exit(1);
                }
            }
        }
        Some(ClawHubSub::Publish { name }) => match sm.publish_to_registry(&name) {
            Ok(msg) => println!("{}", t::icon_ok(&msg)),
            Err(e) => {
                println!("{}", t::icon_fail(&format!("Publish failed: {}", e)));
                std::process::exit(1);
            }
        },
    }
    Ok(())
}
