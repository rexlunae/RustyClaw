//! Interactive onboarding wizard.
//!
//! Mirrors the openclaw `onboard` command: walks the user through selecting a
//! model provider, storing an API key, picking a default model, and
//! initialising the SOUL.

use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};
use crossterm::terminal;

use rustyclaw_core::config::{Config, MessengerConfig, ModelProvider};
use rustyclaw_core::providers::PROVIDERS;
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::soul::{DEFAULT_SOUL_CONTENT, SoulManager};
use rustyclaw_core::theme as t;

// ── Public entry point ──────────────────────────────────────────────────────

/// Run the interactive onboarding wizard, mutating `config` in place and
/// storing secrets.  Returns `true` if the user completed onboarding.
pub fn run_onboard_wizard(
    config: &mut Config,
    secrets: &mut SecretsManager,
    reset: bool,
) -> Result<bool> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    println!();
    t::print_header("🦀  RustyClaw Onboarding  🦀");
    println!();

    // ── Optional reset ─────────────────────────────────────────────
    if reset {
        println!("{}\n", t::warn("Resetting configuration…"));
        *config = Config::default();
    }

    // ── 0. Safety acknowledgment ───────────────────────────────────
    println!(
        "{}",
        t::warn("⚠  Important: Please read before continuing.")
    );
    println!();
    println!(
        "  RustyClaw is an {}, meaning it can",
        t::accent_bright("agentic coding tool")
    );
    println!("  read, write, and execute code on your machine on your");
    println!("  behalf. Like any powerful tool, it should be used with");
    println!("  care and awareness.");
    println!();
    println!(
        "  • {} and modify files in your project",
        t::bold("It can create")
    );
    println!("  • {} commands in your terminal", t::bold("It can run"));
    println!(
        "  • {} with external APIs using your credentials",
        t::bold("It can interact")
    );
    println!();
    println!("  Always review actions before approving them, especially");
    println!("  in production environments. You are responsible for any");
    println!("  changes made by the tool.");
    println!();

    let ack = prompt_line(
        &mut reader,
        &format!(
            "{} ",
            t::accent("Do you acknowledge and wish to continue? [y/N]:")
        ),
    )?;
    if !ack.trim().eq_ignore_ascii_case("y") {
        println!();
        println!("  {}", t::muted("Onboarding cancelled."));
        println!();
        return Ok(false);
    }
    println!();

    // ── 0b. Name your agent ────────────────────────────────────────
    println!("{}", t::heading("Name your agent:"));
    println!();
    println!("  Give your RustyClaw agent a name. This appears in the TUI");
    println!("  title bar, authenticator app labels, and anywhere the");
    println!("  agent identifies itself.");
    println!();

    let current_name = if config.agent_name.is_empty() || config.agent_name == "RustyClaw" {
        None
    } else {
        Some(config.agent_name.clone())
    };
    let name_prompt = if let Some(ref current) = current_name {
        format!("Agent name [{}]: ", current)
    } else {
        "Agent name [RustyClaw]: ".to_string()
    };
    let name_input = prompt_line(&mut reader, &format!("{} ", t::accent(&name_prompt)))?;
    let name_input = name_input.trim().to_string();
    if !name_input.is_empty() {
        config.agent_name = name_input;
    } else if current_name.is_none() {
        config.agent_name = "RustyClaw".to_string();
    }
    println!(
        "  {}",
        t::icon_ok(&format!(
            "Agent name: {}",
            t::accent_bright(&config.agent_name)
        ))
    );
    println!();

    // ── 1. Secrets vault setup ─────────────────────────────────────
    let vault_path = config.credentials_dir().join("secrets.json");
    let key_path = config.credentials_dir().join("secrets.key");
    let vault_exists = vault_path.exists();

    // On reset, remove the old vault so we start fresh.
    if reset && vault_exists {
        let _ = std::fs::remove_file(&vault_path);
        let _ = std::fs::remove_file(&key_path);
        config.secrets_password_protected = false;
        config.totp_enabled = false;
        println!("  {}", t::icon_ok("Previous secrets vault removed."));
        println!();
    }

    let vault_exists = vault_path.exists();

    if !vault_exists {
        // ── First-time setup ───────────────────────────────────────
        println!("{}", t::heading("Secrets vault setup:"));
        println!();
        println!("  RustyClaw stores API keys, tokens, and other credentials");
        println!("  in an encrypted vault.  You can protect it with a");
        println!("  passphrase and optionally enable 2FA via an authenticator");
        println!("  app for an extra layer of security.");
        println!();

        // ── 1a. Optional password ──────────────────────────────────
        println!(
            "{}",
            t::bold("You can protect your secrets vault with a password.")
        );
        println!(
            "{}",
            t::muted("If you skip this, a key file will be generated instead.")
        );
        println!();
        println!(
            "  {}  If you set a password you will need to enter it every",
            t::warn("⚠")
        );
        println!("     time RustyClaw starts, including when the gateway is");
        println!("     launched.  Automated / unattended starts will not be");
        println!("     possible without the password.");
        println!();

        let pw = prompt_secret(
            &mut reader,
            &format!("{} ", t::accent("Vault password (leave blank to skip):")),
        )?;
        let pw = pw.trim().to_string();

        if pw.is_empty() {
            println!(
                "  {}",
                t::icon_ok("Using auto-generated key file (no password).")
            );
            config.secrets_password_protected = false;
        } else {
            loop {
                let confirm =
                    prompt_secret(&mut reader, &format!("{} ", t::accent("Confirm password:")))?;
                if confirm.trim() == pw {
                    secrets.set_password(pw.clone());
                    config.secrets_password_protected = true;
                    println!(
                        "  {}",
                        t::icon_ok("Secrets vault will be password-protected.")
                    );
                    break;
                }
                println!(
                    "  {}",
                    t::icon_warn("Passwords do not match — please try again.")
                );
            }
        }
        println!();

        // ── 1b. Optional TOTP 2FA ──────────────────────────────────
        setup_totp_enrollment(&mut reader, config, secrets, &config.agent_name.clone())?;

        // ── 1c. Agent SSH key ──────────────────────────────────────
        setup_agent_ssh_key(&mut reader, secrets)?;
    } else {
        // ── Existing vault — unlock it ─────────────────────────────
        // Determine the real key source: if the vault exists but no key
        // file is present, the vault *must* be password-protected even
        // if config doesn't reflect it (e.g. a previous run crashed
        // before saving the config).
        let needs_password =
            config.secrets_password_protected || (vault_path.exists() && !key_path.exists());

        if needs_password {
            if !config.secrets_password_protected {
                println!(
                    "  {}",
                    t::warn("Vault appears to be password-protected but config disagrees.")
                );
                println!(
                    "  {}",
                    t::muted("(This can happen if a previous onboard run was interrupted.)")
                );
                println!();
            }
            let pw = prompt_secret(
                &mut reader,
                &format!("{} ", t::accent("Enter vault password:")),
            )?;
            secrets.set_password(pw.trim().to_string());
            config.secrets_password_protected = true;
        }

        // Verify TOTP if enabled.
        if config.totp_enabled {
            verify_totp_loop(&mut reader, secrets)?;
        }

        // Offer to reconfigure vault security.
        println!("{}", t::heading("Secrets vault:"));
        println!();
        let pw_status = if config.secrets_password_protected {
            "password-protected"
        } else {
            "key-file (no password)"
        };
        let totp_status = if config.totp_enabled {
            "enabled"
        } else {
            "disabled"
        };
        println!("  Encryption : {}", t::info(pw_status));
        println!("  2FA (TOTP) : {}", t::info(totp_status));
        println!();

        let reconfig = prompt_line(
            &mut reader,
            &format!("{} ", t::accent("Reconfigure vault security? [y/N]:")),
        )?;

        if reconfig.trim().eq_ignore_ascii_case("y") {
            println!();

            // ── Change password ────────────────────────────────────
            println!("{}", t::bold("Change vault password:"));
            println!("{}", t::muted("Leave blank to keep current setting."));
            println!();

            let pw = prompt_secret(
                &mut reader,
                &format!("{} ", t::accent("New vault password (blank to keep):")),
            )?;
            let pw = pw.trim().to_string();

            if !pw.is_empty() {
                loop {
                    let confirm = prompt_secret(
                        &mut reader,
                        &format!("{} ", t::accent("Confirm password:")),
                    )?;
                    if confirm.trim() == pw {
                        // Re-encrypt the existing vault with the new password
                        // instead of just setting it (which would lose access
                        // to the vault encrypted under the old key source).
                        secrets
                            .change_password(pw.clone())
                            .context("Failed to re-encrypt vault with new password")?;
                        config.secrets_password_protected = true;
                        println!("  {}", t::icon_ok("Vault password updated."));
                        break;
                    }
                    println!(
                        "  {}",
                        t::icon_warn("Passwords do not match — please try again.")
                    );
                }
            } else {
                println!("  {}", t::muted("Keeping current password setting."));
            }
            println!();

            // ── 2FA reconfigure ────────────────────────────────────
            if config.totp_enabled {
                let disable = prompt_line(
                    &mut reader,
                    &format!("{} ", t::accent("Disable 2FA? [y/N]:")),
                )?;
                if disable.trim().eq_ignore_ascii_case("y") {
                    secrets.remove_totp()?;
                    config.totp_enabled = false;
                    println!("  {}", t::icon_ok("2FA disabled."));
                } else {
                    println!("  {}", t::muted("Keeping 2FA enabled."));
                }
            } else {
                setup_totp_enrollment(&mut reader, config, secrets, &config.agent_name.clone())?;
            }

            // ── Agent SSH key ──────────────────────────────────────
            setup_agent_ssh_key(&mut reader, secrets)?
        } else {
            println!("  {}", t::muted("Keeping current vault settings."));
        }
        println!();
    }

    // ── 2. Select model provider ───────────────────────────────────
    let provider_names: Vec<&str> = PROVIDERS.iter().map(|p| p.display).collect();
    let provider = match arrow_select(&provider_names, "Select a model provider:")? {
        Some(idx) => &PROVIDERS[idx],
        None => {
            println!("  {}", t::warn("Cancelled."));
            // Save any config changes made during vault setup before returning.
            config
                .ensure_dirs()
                .context("Failed to create directory structure")?;
            config.save(None)?;
            println!("  {}", t::muted("Partial config saved."));
            return Ok(false);
        }
    };

    println!();
    println!(
        "  {}",
        t::icon_ok(&format!("Selected: {}", t::accent_bright(provider.display)))
    );
    println!();

    // ── 3. Authentication ──────────────────────────────────────────
    use rustyclaw_core::providers::AuthMethod;

    if let Some(secret_key) = provider.secret_key {
        match provider.auth_method {
            AuthMethod::ApiKey => {
                // Standard API key authentication
                let existing = secrets.get_secret(secret_key, true)?;
                if existing.is_some() {
                    let reuse = prompt_line(
                        &mut reader,
                        &format!(
                            "{} ",
                            t::accent(&format!(
                                "An API key for {} is already stored. Keep it? [Y/n]:",
                                provider.display
                            ))
                        ),
                    )?;
                    if reuse.trim().eq_ignore_ascii_case("n") {
                        let key = prompt_secret(
                            &mut reader,
                            &format!("{} ", t::accent("Enter API key:")),
                        )?;
                        if key.trim().is_empty() {
                            println!(
                                "  {}",
                                t::icon_warn("No key entered — keeping existing key.")
                            );
                        } else {
                            secrets.store_secret(secret_key, key.trim())?;
                            println!("  {}", t::icon_ok("API key updated."));
                        }
                    } else {
                        println!("  {}", t::icon_ok("Keeping existing API key."));
                    }
                } else {
                    let key =
                        prompt_secret(&mut reader, &format!("{} ", t::accent("Enter API key:")))?;
                    if key.trim().is_empty() {
                        println!(
                            "  {}",
                            t::icon_warn("No key entered — you can add one later with:")
                        );
                        println!("      {}", t::accent_bright("rustyclaw onboard"));
                    } else {
                        secrets.store_secret(secret_key, key.trim())?;
                        println!("  {}", t::icon_ok("API key stored securely."));
                    }
                }
            }
            AuthMethod::DeviceFlow => {
                // OAuth device flow authentication
                if let Some(device_config) = provider.device_flow {
                    let existing = secrets.get_secret(secret_key, true)?;
                    if existing.is_some() {
                        let reuse = prompt_line(
                            &mut reader,
                            &format!(
                                "{} ",
                                t::accent(&format!(
                                    "An access token for {} is already stored. Keep it? [Y/n]:",
                                    provider.display
                                ))
                            ),
                        )?;
                        if !reuse.trim().eq_ignore_ascii_case("n") {
                            println!("  {}", t::icon_ok("Keeping existing access token."));
                            println!();
                            // Continue to model selection
                        } else {
                            // Re-authenticate with device flow
                            perform_device_flow_auth(
                                &mut reader,
                                provider.display,
                                device_config,
                                secret_key,
                                secrets,
                            )?;
                        }
                    } else {
                        // New authentication
                        perform_device_flow_auth(
                            &mut reader,
                            provider.display,
                            device_config,
                            secret_key,
                            secrets,
                        )?;
                    }
                } else {
                    println!("  {}", t::icon_warn("Device flow configuration missing."));
                }
            }
            AuthMethod::None => {
                // No authentication needed
            }
        }
        println!();
    }

    // ── 4. Base URL ────────────────────────────────────────────────
    // For custom/proxy providers: prompt for the full URL.
    // For local providers (Ollama, LM Studio, exo): show default and
    // allow override (e.g. non-standard ports).
    let needs_url_prompt = provider.id == "custom" || provider.id == "copilot-proxy";
    let is_local_provider =
        provider.id == "ollama" || provider.id == "lmstudio" || provider.id == "exo";
    let base_url: String = if needs_url_prompt {
        let prompt_text = if provider.id == "copilot-proxy" {
            "Copilot Proxy URL:"
        } else {
            "Base URL (OpenAI-compatible):"
        };
        let url = prompt_line(&mut reader, &format!("{} ", t::accent(prompt_text)))?;
        let url = url.trim().to_string();
        if url.is_empty() {
            println!(
                "  {}",
                t::icon_warn("No URL entered. You can set model.base_url in config.toml later.")
            );
            String::new()
        } else {
            println!("  {}", t::icon_ok(&format!("Base URL: {}", t::info(&url))));
            url
        }
    } else if is_local_provider {
        let default_url = provider.base_url.unwrap_or("http://localhost:8080/v1");
        println!("  {} Default: {}", t::muted("ℹ"), t::info(default_url));
        let url = prompt_line(
            &mut reader,
            &format!(
                "{} ",
                t::accent("Base URL (Enter for default, or type custom):")
            ),
        )?;
        let url = url.trim().to_string();
        if url.is_empty() {
            println!(
                "  {}",
                t::icon_ok(&format!("Using default: {}", t::info(default_url)))
            );
            default_url.to_string()
        } else {
            println!("  {}", t::icon_ok(&format!("Base URL: {}", t::info(&url))));
            url
        }
    } else {
        provider.base_url.unwrap_or("").to_string()
    };

    // ── 5. Select a model ──────────────────────────────────────────

    // Try to dynamically fetch models from the provider API.
    let api_key = provider
        .secret_key
        .and_then(|sk| secrets.get_secret(sk, true).ok().flatten());

    let fetched_models: Vec<String> = {
        let handle = tokio::runtime::Handle::current();
        let base_ref = if base_url.is_empty() {
            None
        } else {
            Some(base_url.as_str())
        };
        print!("  {} Fetching available models…", t::muted("⠋"));
        io::stdout().flush()?;
        let result = tokio::task::block_in_place(|| {
            handle.block_on(rustyclaw_core::providers::fetch_models(
                provider.id,
                api_key.as_deref(),
                base_ref,
            ))
        });
        // Clear the spinner line
        print!("\r{}\r", " ".repeat(50));
        io::stdout().flush()?;
        match result {
            Ok(models) => {
                println!(
                    "  {}",
                    t::icon_ok(&format!(
                        "Loaded {} models from {} API.",
                        models.len(),
                        provider.display,
                    ))
                );
                models
            }
            Err(_) => Vec::new(),
        }
    };

    // Use fetched models if available, otherwise fall back to static list.
    let available_models: Vec<String> = if !fetched_models.is_empty() {
        fetched_models
    } else {
        provider.models.iter().map(|s| s.to_string()).collect()
    };

    let model: String = if available_models.is_empty() {
        // No models available — ask for a model name.
        let m = prompt_line(&mut reader, &format!("{} ", t::accent("Model name:")))?;
        m.trim().to_string()
    } else {
        match arrow_select(&available_models, "Select a default model:")? {
            Some(idx) => available_models[idx].clone(),
            None => {
                println!("  {}", t::warn("Cancelled — no model selected."));
                String::new()
            }
        }
    };

    if !model.is_empty() {
        println!();
        println!(
            "  {}",
            t::icon_ok(&format!("Default model: {}", t::accent_bright(&model)))
        );
    }

    // ── 6. Initialize / update SOUL.md ─────────────────────────────
    println!();
    let soul_path = config.soul_path();

    // Only prompt if the file exists AND has been customised (differs from the
    // default template).  A previous `rustyclaw tui` run may have already
    // created the default SOUL.md — that shouldn't count as "already exists".
    let soul_customised = soul_path.exists()
        && std::fs::read_to_string(&soul_path)
            .map(|c| c != DEFAULT_SOUL_CONTENT)
            .unwrap_or(false);

    let init_soul = if soul_customised {
        let answer = prompt_line(
            &mut reader,
            &format!(
                "{} ",
                t::accent("SOUL.md has been customised. Reset to default? [y/N]:")
            ),
        )?;
        answer.trim().eq_ignore_ascii_case("y")
    } else {
        true
    };

    if init_soul {
        let mut soul = SoulManager::new(soul_path.clone());
        soul.load()?;
        println!(
            "  {}",
            t::icon_ok(&format!(
                "SOUL.md initialised at {}",
                t::info(&soul_path.display().to_string())
            ))
        );
    } else {
        println!("  {}", t::icon_ok("Keeping existing SOUL.md"));
    }

    // ── 7. Configure messengers ────────────────────────────────────
    println!();
    let configured_messengers = setup_messaging(&mut reader, config)?;

    // ── 8. Recommend additional skills ─────────────────────────────
    println!();
    setup_recommended_skills(&mut reader, config)?;

    // ── 9. Write config ────────────────────────────────────────
    config.model = Some(ModelProvider {
        provider: provider.id.to_string(),
        model: if model.is_empty() { None } else { Some(model) },
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
    });
    config.messengers = configured_messengers;

    // Ensure the full directory skeleton exists and save.
    config
        .ensure_dirs()
        .context("Failed to create directory structure")?;
    config.save(None)?;

    t::print_header("Onboarding complete! 🎉");
    println!(
        "  {}",
        t::icon_ok(&format!(
            "Config saved to {}",
            t::info(
                &config
                    .settings_dir
                    .join("config.toml")
                    .display()
                    .to_string()
            )
        ))
    );
    println!(
        "  Run {} to start the TUI.",
        t::accent_bright("`rustyclaw tui`")
    );
    println!();

    Ok(true)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Walk the user through messaging setup — native messengers first, then optional skill.
fn setup_messaging(reader: &mut impl BufRead, config: &Config) -> Result<Vec<MessengerConfig>> {
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

/// Recommended skills with descriptions (excludes messaging — handled separately).
struct RecommendedSkill {
    slug: &'static str,
    display: &'static str,
    description: &'static str,
    category: &'static str,
}

const RECOMMENDED_SKILLS: &[RecommendedSkill] = &[
    // Development
    RecommendedSkill {
        slug: "github",
        display: "GitHub",
        description: "GitHub operations via gh CLI — issues, PRs, CI, code review",
        category: "Development",
    },
    // Productivity
    RecommendedSkill {
        slug: "weather",
        display: "Weather",
        description: "Current weather and forecasts via wttr.in or Open-Meteo",
        category: "Productivity",
    },
    RecommendedSkill {
        slug: "bluesky",
        display: "Bluesky",
        description: "Bluesky social network — post, read timeline, follow, like, repost",
        category: "Social",
    },
];

/// Walk the user through installing recommended skills from ClawHub.
fn setup_recommended_skills(_reader: &mut impl BufRead, config: &Config) -> Result<()> {
    println!("{}", t::heading("Install recommended skills (optional):"));
    println!();
    println!("  Skills extend RustyClaw with new capabilities. These are");
    println!("  installed from ClawHub and don't require recompilation.");
    println!();

    // Check if clawhub CLI is available
    let clawhub_available = std::process::Command::new("clawhub")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !clawhub_available {
        println!("  {}", t::warn("clawhub CLI not found."));
        println!(
            "  Install it with: {}",
            t::accent_bright("npm install -g clawhub")
        );
        println!(
            "  Then run {} to add skills.",
            t::accent_bright("rustyclaw skills install <name>")
        );
        println!();
        return Ok(());
    }

    let skills_dir = config.workspace_dir().join("skills");

    // Build list of skills not yet installed
    let mut available: Vec<usize> = Vec::new();
    for (i, skill) in RECOMMENDED_SKILLS.iter().enumerate() {
        let skill_path = skills_dir.join(skill.slug);
        if !skill_path.exists() {
            available.push(i);
        }
    }

    if available.is_empty() {
        println!(
            "  {}",
            t::icon_ok("All recommended skills are already installed.")
        );
        println!();
        return Ok(());
    }

    // Group by category for display
    let mut by_category: std::collections::BTreeMap<&str, Vec<usize>> =
        std::collections::BTreeMap::new();
    for &i in &available {
        by_category
            .entry(RECOMMENDED_SKILLS[i].category)
            .or_default()
            .push(i);
    }

    // Display available skills
    println!("  {}", t::bold("Available skills:"));
    println!();
    for (category, indices) in &by_category {
        println!("  {}", t::accent(category));
        for &i in indices {
            let skill = &RECOMMENDED_SKILLS[i];
            println!(
                "    {} — {}",
                t::accent_bright(skill.display),
                t::muted(skill.description)
            );
        }
        println!();
    }

    // Let user select which to install
    let mut choices: Vec<String> = available
        .iter()
        .map(|&i| RECOMMENDED_SKILLS[i].display.to_string())
        .collect();
    choices.push("Install all recommended".to_string());
    choices.push("Skip — install none".to_string());

    let selected = match arrow_select(&choices, "Select skills to install:")? {
        None => return Ok(()),
        Some(idx) => idx,
    };

    println!();

    let to_install: Vec<usize> = if selected == choices.len() - 1 {
        // Skip
        println!(
            "  {}",
            t::muted("Skipping skill installation. You can install them later with:")
        );
        println!(
            "    {}",
            t::accent_bright("rustyclaw skills install <name>")
        );
        println!();
        return Ok(());
    } else if selected == choices.len() - 2 {
        // Install all
        available.clone()
    } else {
        // Single skill
        vec![available[selected]]
    };

    // Install selected skills
    for &i in &to_install {
        let skill = &RECOMMENDED_SKILLS[i];
        print!("  {} Installing {}...", t::muted("⠋"), skill.display);
        io::stdout().flush()?;

        let output = std::process::Command::new("clawhub")
            .args([
                "install",
                skill.slug,
                "--dir",
                &skills_dir.to_string_lossy(),
            ])
            .output();

        // Clear spinner
        print!("\r{}\r", " ".repeat(60));

        match output {
            Ok(out) if out.status.success() => {
                println!("  {}", t::icon_ok(&format!("Installed {}", skill.display)));
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                println!(
                    "  {}",
                    t::icon_warn(&format!(
                        "Failed to install {}: {}",
                        skill.display,
                        stderr.trim()
                    ))
                );
            }
            Err(e) => {
                println!(
                    "  {}",
                    t::icon_warn(&format!("Failed to install {}: {}", skill.display, e))
                );
            }
        }
    }

    println!();
    Ok(())
}

/// Perform OAuth device flow authentication and store the token.
fn perform_device_flow_auth(
    reader: &mut impl BufRead,
    provider_name: &str,
    device_config: &rustyclaw_core::providers::DeviceFlowConfig,
    secret_key: &str,
    secrets: &mut SecretsManager,
) -> Result<()> {
    println!(
        "{}",
        t::heading(&format!("Authenticating with {}...", provider_name))
    );
    println!();

    // Start the device flow
    let handle = tokio::runtime::Handle::current();
    let auth_response = tokio::task::block_in_place(|| {
        handle.block_on(rustyclaw_core::providers::start_device_flow(device_config))
    })
    .map_err(|e| anyhow::anyhow!(e))?;

    // Display the verification URL and code to the user
    println!("  {}", t::bold("Please complete the following steps:"));
    println!();
    println!(
        "  1. Visit: {}",
        t::accent_bright(&auth_response.verification_uri)
    );
    println!(
        "  2. Enter code: {}",
        t::accent_bright(&auth_response.user_code)
    );
    println!();
    println!(
        "  {}",
        t::muted(&format!(
            "Code expires in {} seconds",
            auth_response.expires_in
        ))
    );
    println!();

    // Wait for user to press Enter or type 'cancel'
    let response = prompt_line(
        reader,
        &format!(
            "{} ",
            t::accent("Press Enter after completing authorization (or type 'cancel'):")
        ),
    )?;
    if response.trim().eq_ignore_ascii_case("cancel") || response.trim().eq_ignore_ascii_case("c") {
        println!("  {}", t::muted("Authentication cancelled."));
        return Ok(());
    }

    // Poll for the token
    println!("  {}", t::muted("Waiting for authorization..."));

    // Use the server-provided interval, which is typically 5 seconds for GitHub.
    // This respects GitHub's rate limiting and follows OAuth 2.0 device flow best practices.
    let interval = std::time::Duration::from_secs(auth_response.interval);

    // Calculate max attempts based on expiration time and interval
    let max_attempts = (auth_response.expires_in / auth_response.interval).max(10);

    let mut token: Option<String> = None;
    for _attempt in 0..max_attempts {
        match tokio::task::block_in_place(|| {
            handle.block_on(rustyclaw_core::providers::poll_device_token(
                device_config,
                &auth_response.device_code,
            ))
        }) {
            Ok(Some(access_token)) => {
                token = Some(access_token);
                break;
            }
            Ok(None) => {
                // Still pending, wait and retry
                print!(".");
                io::stdout().flush()?;
                std::thread::sleep(interval);
            }
            Err(e) => {
                println!();
                println!(
                    "  {}",
                    t::icon_warn(&format!("Authentication failed: {}", e))
                );
                return Ok(());
            }
        }
    }
    println!();

    if let Some(access_token) = token {
        secrets.store_secret(secret_key, &access_token)?;
        println!(
            "  {}",
            t::icon_ok("Authentication successful! Token stored securely.")
        );
    } else {
        println!(
            "  {}",
            t::icon_warn("Authentication timed out. Please try again.")
        );
    }

    Ok(())
}

/// Render a QR code as compact Unicode art in the terminal.
///
/// Uses Unicode half-block characters (▀▄█ and space) so that each
/// character cell encodes two vertical "modules", giving a clean,
/// scannable result at roughly half the height of a naive render.
fn print_qr_code(data: &str) {
    use qrcode::QrCode;

    let code = match QrCode::new(data) {
        Ok(c) => c,
        Err(_) => {
            // Silently fall back — the URL is printed below anyway.
            return;
        }
    };

    let colors = code.to_colors();
    let width = code.width();

    // Quiet zone: 4 modules on each side (QR spec recommends 4).
    let qz = 4;
    let total_w = width + 2 * qz;

    // Collect rows with quiet-zone padding (false = light module).
    let mut rows: Vec<Vec<bool>> = Vec::new();
    for _ in 0..qz {
        rows.push(vec![false; total_w]);
    }
    for y in 0..width {
        let mut row = vec![false; qz];
        for x in 0..width {
            row.push(colors[y * width + x] == qrcode::Color::Dark);
        }
        row.resize(total_w, false);
        rows.push(row);
    }
    for _ in 0..qz {
        rows.push(vec![false; total_w]);
    }

    // Render two rows at a time using Unicode half-block characters.
    //
    // We use an INVERTED scheme so it works on dark terminal backgrounds:
    //   - Light module (false) → print a block character (appears as
    //     the foreground colour = white)
    //   - Dark  module (true)  → print a space (shows the background
    //     colour = dark/black)
    //
    // This way the QR's dark modules are dark and the light modules
    // (including the quiet zone) are bright — good contrast for scanners.
    //
    // Half-block ▀ means "top pixel on, bottom pixel off" (in the
    // inverted world: top is light, bottom is dark).
    let total_h = rows.len();
    let indent = "  ";
    for pair in (0..total_h).step_by(2) {
        print!("{}", indent);
        for x in 0..total_w {
            let top_dark = rows[pair][x];
            let bot_dark = if pair + 1 < total_h {
                rows[pair + 1][x]
            } else {
                false
            };
            // Invert: light → filled, dark → empty
            let ch = match (top_dark, bot_dark) {
                (false, false) => '█', // both light → full block
                (false, true) => '▀',  // top light, bottom dark → upper half
                (true, false) => '▄',  // top dark, bottom light → lower half
                (true, true) => ' ',   // both dark → space
            };
            print!("{}", ch);
        }
        println!();
    }
}

fn prompt_line(reader: &mut impl BufRead, prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    reader.read_line(&mut buf)?;
    Ok(buf
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}

/// Best-effort username for TOTP account labels.
fn whoami() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string())
}

/// Walk the user through TOTP 2FA enrollment.
fn setup_totp_enrollment(
    reader: &mut impl BufRead,
    config: &mut Config,
    secrets: &mut SecretsManager,
    agent_name: &str,
) -> Result<()> {
    println!("{}", t::bold("Two-factor authentication (optional):"));
    println!();
    println!("  You can add TOTP-based 2FA using any authenticator app");
    println!("  (Google Authenticator, Authy, 1Password, etc.).  This");
    println!("  adds a second layer of protection — you will need to");
    println!("  enter a 6-digit code each time you unlock the vault.");
    println!();

    let enable_2fa = prompt_line(
        reader,
        &format!(
            "{} ",
            t::accent("Enable 2FA with an authenticator app? [y/N]:")
        ),
    )?;

    if enable_2fa.trim().eq_ignore_ascii_case("y") {
        // Need the vault to exist before we can store the TOTP secret.
        // Force-create it now by storing a sentinel value.
        config
            .ensure_dirs()
            .context("Failed to create directory structure")?;
        secrets.store_secret("__init", "")?;
        secrets.delete_secret("__init")?;

        let account = whoami();
        let otpauth_url = secrets.setup_totp_with_issuer(&account, agent_name)?;

        println!();
        println!(
            "  {}",
            t::heading("Scan this QR code with your authenticator app:")
        );
        println!();
        print_qr_code(&otpauth_url);
        println!();
        println!(
            "  {}",
            t::muted("If you can't scan, enter the setup key manually.")
        );
        println!(
            "  {}",
            t::muted(&format!(
                "The key is the {} parameter in the URL below:",
                t::bold("secret")
            ))
        );
        println!();
        println!("  {}", t::accent_bright(&otpauth_url));
        println!();

        // Verify the user can produce a valid code before committing.
        loop {
            let code = prompt_line(
                reader,
                &format!("{} ", t::accent("Enter the 6-digit code to verify:")),
            )?;
            let code = code.trim();
            if code.is_empty() {
                println!("  {}", t::icon_warn("2FA setup cancelled."));
                secrets.remove_totp()?;
                break;
            }
            match secrets.verify_totp(code) {
                Ok(true) => {
                    config.totp_enabled = true;
                    println!(
                        "  {}",
                        t::icon_ok("2FA enabled — authenticator verified successfully.")
                    );
                    break;
                }
                Ok(false) => {
                    println!(
                        "  {}",
                        t::icon_warn("Invalid code. Please try again (or leave blank to cancel):")
                    );
                }
                Err(e) => {
                    println!(
                        "  {}",
                        t::icon_warn(&format!("Error verifying code: {}. 2FA not enabled.", e))
                    );
                    secrets.remove_totp()?;
                    break;
                }
            }
        }
    } else {
        println!("  {}", t::muted("Skipping 2FA. You can enable it later."));
    }
    println!();
    Ok(())
}

/// Prompt the user for a TOTP code in a retry loop.
fn verify_totp_loop(reader: &mut impl BufRead, secrets: &mut SecretsManager) -> Result<()> {
    loop {
        let code = prompt_line(reader, &format!("{} ", t::accent("Enter your 2FA code:")))?;
        match secrets.verify_totp(code.trim()) {
            Ok(true) => {
                println!("  {}", t::icon_ok("2FA verified."));
                return Ok(());
            }
            Ok(false) => {
                println!("  {}", t::icon_warn("Invalid code. Please try again:"));
            }
            Err(e) => {
                anyhow::bail!("2FA verification failed: {}", e);
            }
        }
    }
}

/// Offer to generate an Ed25519 SSH key for the agent.
fn setup_agent_ssh_key(reader: &mut impl BufRead, secrets: &mut SecretsManager) -> Result<()> {
    use rustyclaw_core::secrets::AccessPolicy;

    // Check if one already exists.
    let has_key = secrets
        .list_credentials()
        .iter()
        .any(|(n, _)| n == "rustyclaw_agent");

    if has_key {
        println!("{}", t::bold("Agent SSH key:"));
        println!(
            "  {}",
            t::icon_ok("SSH key already generated (stored in encrypted vault).")
        );
        let regen = prompt_line(
            reader,
            &format!("{} ", t::accent("Regenerate agent SSH key? [y/N]:")),
        )?;
        if !regen.trim().eq_ignore_ascii_case("y") {
            println!("  {}", t::muted("Keeping existing key."));
            println!();
            return Ok(());
        }
        secrets.delete_credential("rustyclaw_agent")?;
    }

    println!("{}", t::bold("Agent SSH key (optional):"));
    println!();
    println!("  RustyClaw can have its own Ed25519 SSH key, separate");
    println!("  from your personal keys.  This is useful for git");
    println!("  operations, deploy access, and remote connections");
    println!("  performed by the agent.");
    println!();

    let do_gen = prompt_line(
        reader,
        &format!("{} ", t::accent("Generate an agent SSH key? [y/N]:")),
    )?;

    if do_gen.trim().eq_ignore_ascii_case("y") {
        let comment = format!("rustyclaw-agent@{}", whoami());
        let pubkey =
            secrets.generate_ssh_key("rustyclaw_agent", &comment, AccessPolicy::WithApproval)?;
        println!();
        println!("  {}", t::icon_ok("Agent SSH key generated."));
        println!();
        println!("  {}", t::bold("Public key:"));
        println!("  {}", t::info(&pubkey));
        println!();
        println!("  Stored in the encrypted vault.");
        println!("  Add the public key above to your Git host or authorized_keys as needed.");
    } else {
        println!(
            "  {}",
            t::muted("Skipping agent SSH key. You can generate one later.")
        );
    }
    println!();
    Ok(())
}

/// Interactive arrow-key selector.
///
/// Renders a scrollable list of items with a `❯` marker and handles
/// Up / Down / Home / End / Enter / Esc / Ctrl-C navigation in raw mode.
/// Returns the selected index, or `None` if the user pressed Esc.
fn arrow_select(items: &[impl AsRef<str>], heading_text: &str) -> Result<Option<usize>> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
    use crossterm::{cursor, execute, terminal as ct};

    if items.is_empty() {
        return Ok(None);
    }

    let mut selected: usize = 0;
    let max_visible: usize = 14;

    // Print the heading (above the interactive region)
    println!("{}", t::heading(heading_text));
    println!();

    let visible_count = items.len().min(max_visible);
    // We render `visible_count` lines for the items + 1 hint line.
    let draw_height = visible_count + 1;

    // Pre-allocate the lines so we can overwrite them.
    for _ in 0..draw_height {
        println!();
    }

    let mut stdout = io::stdout();

    // Helper: draw the visible slice starting at `scroll_offset`.
    let draw = |stdout: &mut io::Stdout, selected: usize, scroll_offset: usize| -> io::Result<()> {
        // Move cursor up to the first item line.
        execute!(stdout, cursor::MoveUp(draw_height as u16))?;

        let end = (scroll_offset + max_visible).min(items.len());
        for i in scroll_offset..end {
            let label = items[i].as_ref();
            let line = if i == selected {
                format!("  {} {}", t::accent("❯"), t::accent_bright(label),)
            } else {
                format!("    {}", t::muted(label))
            };
            // Clear the line, print with \r\n (raw mode needs explicit CR).
            execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
            write!(stdout, "{}\r\n", line)?;
        }

        // Hint line
        execute!(stdout, ct::Clear(ct::ClearType::CurrentLine))?;
        if items.len() > max_visible {
            write!(
                stdout,
                "  {}\r\n",
                t::muted(&format!(
                    "{}/{} · ↑↓ navigate · Enter select · Esc cancel",
                    selected + 1,
                    items.len(),
                )),
            )?;
        } else {
            write!(
                stdout,
                "  {}\r\n",
                t::muted("↑↓ navigate · Enter select · Esc cancel")
            )?;
        }
        stdout.flush()
    };

    ct::enable_raw_mode()?;

    let result = (|| -> Result<Option<usize>> {
        let mut scroll_offset: usize = 0;

        // Initial draw
        draw(&mut stdout, selected, scroll_offset)?;

        loop {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        anyhow::bail!("Interrupted");
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        return Ok(None);
                    }
                    KeyCode::Enter => {
                        return Ok(Some(selected));
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if selected > 0 {
                            selected -= 1;
                            if selected < scroll_offset {
                                scroll_offset = selected;
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if selected + 1 < items.len() {
                            selected += 1;
                            if selected >= scroll_offset + max_visible {
                                scroll_offset = selected - max_visible + 1;
                            }
                        }
                    }
                    KeyCode::Home => {
                        selected = 0;
                        scroll_offset = 0;
                    }
                    KeyCode::End => {
                        selected = items.len().saturating_sub(1);
                        scroll_offset = items.len().saturating_sub(max_visible);
                    }
                    _ => continue,
                }

                draw(&mut stdout, selected, scroll_offset)?;
            }
        }
    })();

    // Always restore cooked mode.
    let _ = ct::disable_raw_mode();

    result
}

fn prompt_secret(_reader: &mut impl BufRead, prompt: &str) -> Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

    print!("{}", prompt);
    io::stdout().flush()?;

    // Enable raw mode to suppress echo and line buffering.
    terminal::enable_raw_mode()?;

    let result = (|| -> Result<String> {
        let mut buf = String::new();
        loop {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Enter => break,
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        anyhow::bail!("Interrupted");
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                    }
                    _ => {}
                }
            }
        }
        Ok(buf)
    })();

    // Always restore cooked mode, even on error.
    let _ = terminal::disable_raw_mode();
    // Print newline since Enter was consumed without echo.
    println!();

    result
}
