//! (onboard submodule)

use std::io::{self, BufRead, Write};

use anyhow::Result;

use crate::prompts::{arrow_select, prompt_line, prompt_secret};
use rustyclaw_core::config::{Config, MessengerConfig};
use rustyclaw_core::theme as t;

/// Walk the user through messaging setup — native messengers first, then optional skill.
pub(crate) fn setup_messaging(
    reader: &mut impl BufRead,
    config: &Config,
) -> Result<Vec<MessengerConfig>> {
    println!("{}", t::heading("📱 Messaging Setup"));
    println!();
    println!("  RustyClaw can send and receive messages through various");
    println!("  platforms. Configure the ones you want to use.");
    println!();

    // ── Part 1: Native messengers ──────────────────────────────────────────

    /// Available messenger definitions for onboarding.
    struct MessengerDef {
        id: &'static str,
        display: &'static str,
        description: &'static str,
        secret_label: &'static str,
    }

    const MESSENGERS: &[MessengerDef] = &[
        MessengerDef {
            id: "telegram",
            display: "Telegram",
            description: "Bot via @BotFather",
            secret_label: "Bot token (from @BotFather)",
        },
        MessengerDef {
            id: "discord",
            display: "Discord",
            description: "Bot via Discord Developer Portal",
            secret_label: "Bot token",
        },
        MessengerDef {
            id: "matrix",
            display: "Matrix",
            description: "Any Matrix homeserver (Element, etc.)",
            secret_label: "Access token or password",
        },
        MessengerDef {
            id: "webhook",
            display: "Webhook",
            description: "HTTP endpoint for custom integrations",
            secret_label: "Webhook URL",
        },
    ];

    let mut configured_messengers: Vec<MessengerConfig> = Vec::new();

    // Allow the user to pick multiple messengers in a loop.
    let mut remaining: Vec<usize> = (0..MESSENGERS.len()).collect();

    println!("  {}", t::bold("Built-in messengers:"));
    for def in MESSENGERS {
        println!(
            "    {} — {}",
            t::accent_bright(def.display),
            t::muted(def.description)
        );
    }
    println!();

    loop {
        if remaining.is_empty() {
            break;
        }

        let mut choices: Vec<String> = remaining
            .iter()
            .map(|&i| format!("{} — {}", MESSENGERS[i].display, MESSENGERS[i].description))
            .collect();
        choices.push("Done — continue to next step".to_string());

        let heading = if configured_messengers.is_empty() {
            "Select a messenger to configure:"
        } else {
            "Add another messenger?"
        };

        let choice_refs: Vec<&str> = choices.iter().map(|s| s.as_str()).collect();
        match arrow_select(&choice_refs, heading)? {
            None => break,
            Some(idx) if idx == choices.len() - 1 => break,
            Some(pick) => {
                let orig_idx = remaining[pick];
                let def = &MESSENGERS[orig_idx];
                println!();

                // Special handling for Matrix (needs homeserver + user_id)
                if def.id == "matrix" {
                    println!("  {}", t::accent("Matrix configuration:"));
                    println!();

                    let homeserver = prompt_line(
                        reader,
                        &format!(
                            "  {} ",
                            t::accent("Homeserver URL (e.g., https://matrix.org):")
                        ),
                    )?;
                    let homeserver = homeserver.trim().to_string();

                    if homeserver.is_empty() {
                        println!(
                            "  {}",
                            t::icon_warn("No homeserver entered — skipping Matrix.")
                        );
                        remaining.remove(pick);
                        println!();
                        continue;
                    }

                    let user_id = prompt_line(
                        reader,
                        &format!("  {} ", t::accent("User ID (e.g., @bot:matrix.org):")),
                    )?;
                    let user_id = user_id.trim().to_string();

                    let token = prompt_secret(
                        reader,
                        &format!(
                            "  {} ",
                            t::accent("Access token (or leave empty for password):")
                        ),
                    )?;
                    let token = token.trim().to_string();

                    let password = if token.is_empty() {
                        let pwd = prompt_secret(reader, &format!("  {} ", t::accent("Password:")))?;
                        Some(pwd.trim().to_string())
                    } else {
                        None
                    };

                    if token.is_empty() && password.as_ref().map(|p| p.is_empty()).unwrap_or(true) {
                        println!(
                            "  {}",
                            t::icon_warn("No credentials entered — skipping Matrix.")
                        );
                    } else {
                        let mut mc = MessengerConfig {
                            name: "matrix".to_string(),
                            messenger_type: "matrix".to_string(),
                            enabled: true,
                            homeserver: Some(homeserver),
                            user_id: Some(user_id),
                            ..Default::default()
                        };
                        if !token.is_empty() {
                            mc.access_token = Some(token);
                        }
                        if let Some(pwd) = password {
                            if !pwd.is_empty() {
                                mc.password = Some(pwd);
                            }
                        }
                        configured_messengers.push(mc);
                        println!("  {}", t::icon_ok("Matrix configured."));
                    }
                } else {
                    // Standard token-based messenger
                    let token = prompt_secret(
                        reader,
                        &format!(
                            "{} ",
                            t::accent(&format!("{} — {}:", def.display, def.secret_label))
                        ),
                    )?;
                    let token = token.trim().to_string();

                    if token.is_empty() {
                        println!(
                            "  {}",
                            t::icon_warn(&format!("No token entered — skipping {}.", def.display,))
                        );
                    } else {
                        let mut mc = MessengerConfig {
                            name: def.id.to_string(),
                            messenger_type: def.id.to_string(),
                            enabled: true,
                            ..Default::default()
                        };

                        // Store appropriately based on type
                        match def.id {
                            "telegram" | "discord" => mc.token = Some(token),
                            "webhook" => mc.webhook_url = Some(token),
                            _ => mc.token = Some(token),
                        }

                        configured_messengers.push(mc);
                        println!("  {}", t::icon_ok(&format!("{} configured.", def.display,)));
                    }
                }

                remaining.remove(pick);
                println!();
            }
        }
    }

    if configured_messengers.is_empty() {
        println!("  {}", t::muted("No native messengers configured."));
    } else {
        let names: Vec<&str> = configured_messengers
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        println!(
            "  {}",
            t::icon_ok(&format!("Messengers configured: {}", names.join(", "),))
        );
    }

    // ── Part 2: Additional platforms via claw-me-maybe ─────────────────────
    println!();
    println!("{}", t::heading("📱 Additional Platforms (Optional)"));
    println!();
    println!(
        "  Want {} or other platforms?",
        t::accent_bright("Signal, iMessage, Slack, Instagram, LinkedIn")
    );
    println!();
    println!(
        "  The {} skill connects to 15+ platforms",
        t::accent_bright("claw-me-maybe")
    );
    println!(
        "  via {} — a free unified messaging app.",
        t::accent_bright("Beeper")
    );
    println!();

    let choices = &[
        "Yes — set up additional platforms",
        "No — I'm good with what I have",
    ];

    let selected = match arrow_select(choices, "Set up additional platforms?")? {
        Some(idx) => idx,
        None => return Ok(configured_messengers),
    };

    if selected == 1 {
        println!();
        println!(
            "  {}",
            t::muted("You can install claw-me-maybe later with:")
        );
        println!("    {}", t::accent_bright("clawhub install claw-me-maybe"));
        println!();
        return Ok(configured_messengers);
    }

    // Set up Beeper + claw-me-maybe
    println!();
    println!("{}", t::heading("Step 1: Install Beeper"));
    println!();
    println!("  Beeper connects all your chat accounts in one app.");
    println!("  RustyClaw talks to Beeper's local API.");
    println!();
    println!(
        "  {} {}",
        t::bold("Download:"),
        t::accent_bright("https://www.beeper.com/download")
    );
    println!();

    let beeper_ready = prompt_line(
        reader,
        &format!(
            "{} ",
            t::accent("Press Enter once Beeper is installed (or 's' to skip):")
        ),
    )?;

    if beeper_ready.trim().eq_ignore_ascii_case("s") {
        println!();
        println!(
            "  {}",
            t::muted("Skipping. Install claw-me-maybe later when ready.")
        );
        return Ok(configured_messengers);
    }

    println!();
    println!("{}", t::heading("Step 2: Enable Beeper Desktop API"));
    println!();
    println!("  In Beeper:");
    println!(
        "    1. Open {} → {}",
        t::bold("Settings"),
        t::bold("Developers")
    );
    println!(
        "    2. Toggle {} ON",
        t::accent_bright("\"Beeper Desktop API\"")
    );
    println!();

    let _ = prompt_line(
        reader,
        &format!(
            "{} ",
            t::accent("Press Enter once enabled (or 's' to skip):")
        ),
    )?;

    // Install claw-me-maybe skill
    println!();
    println!("{}", t::heading("Step 3: Install Skill"));
    println!();

    let clawhub_available = std::process::Command::new("clawhub")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !clawhub_available {
        println!("  {}", t::warn("clawhub CLI not found."));
        println!();
        println!("  Install it with:");
        println!("    {}", t::accent_bright("npm install -g clawhub"));
        println!();
        println!("  Then install the skill:");
        println!("    {}", t::accent_bright("clawhub install claw-me-maybe"));
        return Ok(configured_messengers);
    }

    let skills_dir = config.workspace_dir().join("skills");
    let skill_path = skills_dir.join("claw-me-maybe");

    if skill_path.exists() {
        println!(
            "  {}",
            t::icon_ok("claw-me-maybe skill is already installed!")
        );
    } else {
        print!("  {} Installing claw-me-maybe...", t::muted("⠋"));
        io::stdout().flush()?;

        let output = std::process::Command::new("clawhub")
            .args([
                "install",
                "claw-me-maybe",
                "--dir",
                &skills_dir.to_string_lossy(),
            ])
            .output();

        print!("\r{}\r", " ".repeat(50));

        match output {
            Ok(out) if out.status.success() => {
                println!("  {}", t::icon_ok("claw-me-maybe installed!"));
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                println!(
                    "  {}",
                    t::icon_warn(&format!("Installation failed: {}", stderr.trim()))
                );
                println!(
                    "  Try manually: {}",
                    t::accent_bright("clawhub install claw-me-maybe")
                );
            }
            Err(e) => {
                println!("  {}", t::icon_warn(&format!("Installation failed: {}", e)));
            }
        }
    }

    println!();
    println!("  {}", t::icon_ok("Additional platforms ready via Beeper!"));
    println!();

    Ok(configured_messengers)
}
