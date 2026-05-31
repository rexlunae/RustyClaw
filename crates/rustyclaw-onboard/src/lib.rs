//! Interactive onboarding wizard.
//!
//! Mirrors the openclaw `onboard` command: walks the user through selecting a
//! model provider, storing an API key, picking a default model, and
//! initialising the SOUL.

use std::io::{self, Write};

use anyhow::{Context, Result};

use rustyclaw_core::config::{Config, ModelProvider};
use rustyclaw_core::providers::PROVIDERS;
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::soul::{DEFAULT_SOUL_CONTENT, SoulManager};
use rustyclaw_core::theme as t;

mod messaging;
mod prompts;
mod security;
mod skills;

use messaging::setup_messaging;
use prompts::{arrow_select, fuzzy_select, prompt_line, prompt_secret};
use security::{
    perform_device_flow_auth, setup_agent_ssh_key, setup_totp_enrollment, verify_totp_loop,
};
use skills::setup_recommended_skills;

// ── Public entry point ──────────────────────────────────────────────────────

/// Onboard arguments from CLI (optional).
pub struct OnboardArgs {
    pub openrouter_api_key: Option<String>,
    // Add other API key fields as needed
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
    pub xai_api_key: Option<String>,
    pub reset: bool,
    pub non_interactive: bool,
}

/// Run the interactive onboarding wizard, mutating `config` in place and
/// storing secrets.  Returns `true` if the user completed onboarding.
pub fn run_onboard_wizard(
    config: &mut Config,
    secrets: &mut SecretsManager,
    args: Option<OnboardArgs>,
) -> Result<bool> {
    let reset = args.as_ref().map(|a| a.reset).unwrap_or(false);
    let non_interactive = args.as_ref().map(|a| a.non_interactive).unwrap_or(false);
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
    if !non_interactive {
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
    }

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
    let provider = if let Some(ref args) = args {
        // Check for auto-selection based on API key flags
        if args.openrouter_api_key.is_some() {
            // Auto-select OpenRouter
            println!(
                "  {}",
                t::icon_ok("Auto-selecting OpenRouter provider based on --openrouter-api-key flag")
            );
            PROVIDERS.iter().find(|p| p.id == "openrouter").unwrap()
        } else if args.anthropic_api_key.is_some() {
            // Auto-select Anthropic
            println!(
                "  {}",
                t::icon_ok("Auto-selecting Anthropic provider based on --anthropic-api-key flag")
            );
            PROVIDERS.iter().find(|p| p.id == "anthropic").unwrap()
        } else if args.openai_api_key.is_some() {
            // Auto-select OpenAI
            println!(
                "  {}",
                t::icon_ok("Auto-selecting OpenAI provider based on --openai-api-key flag")
            );
            PROVIDERS.iter().find(|p| p.id == "openai").unwrap()
        } else if args.gemini_api_key.is_some() {
            // Auto-select Google
            println!(
                "  {}",
                t::icon_ok("Auto-selecting Google provider based on --gemini-api-key flag")
            );
            PROVIDERS.iter().find(|p| p.id == "google").unwrap()
        } else if args.xai_api_key.is_some() {
            // Auto-select xAI
            println!(
                "  {}",
                t::icon_ok("Auto-selecting xAI provider based on --xai-api-key flag")
            );
            PROVIDERS.iter().find(|p| p.id == "xai").unwrap()
        } else {
            // No API key provided, show interactive selection
            let provider_names: Vec<&str> = PROVIDERS.iter().map(|p| p.display).collect();
            match arrow_select(&provider_names, "Select a model provider:")? {
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
            }
        }
    } else {
        // No args provided, show interactive selection
        let provider_names: Vec<&str> = PROVIDERS.iter().map(|p| p.display).collect();
        match arrow_select(&provider_names, "Select a model provider:")? {
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
                // Check if API key was provided via CLI args first
                let provided_key = if let Some(ref args) = args {
                    match provider.id {
                        "openrouter" => args.openrouter_api_key.as_ref(),
                        "anthropic" => args.anthropic_api_key.as_ref(),
                        "openai" => args.openai_api_key.as_ref(),
                        "google" => args.gemini_api_key.as_ref(),
                        "xai" => args.xai_api_key.as_ref(),
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some(key) = provided_key {
                    // Store the provided API key
                    secrets.store_secret(secret_key, key)?;
                    println!("  {}", t::icon_ok("API key stored securely."));
                } else {
                    // Standard API key authentication flow
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
                        let key = prompt_secret(
                            &mut reader,
                            &format!("{} ", t::accent("Enter API key:")),
                        )?;
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
            AuthMethod::OptionalApiKey => {
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
                            &format!(
                                "{} ",
                                t::accent("Enter API key (or leave blank to remove):")
                            ),
                        )?;
                        if key.trim().is_empty() {
                            secrets.delete_secret(secret_key)?;
                            println!(
                                "  {}",
                                t::icon_ok("Key removed — will connect without authentication.")
                            );
                        } else {
                            secrets.store_secret(secret_key, key.trim())?;
                            println!("  {}", t::icon_ok("API key updated."));
                        }
                    } else {
                        println!("  {}", t::icon_ok("Keeping existing API key."));
                    }
                } else {
                    let key = prompt_secret(
                        &mut reader,
                        &format!(
                            "{} ",
                            t::accent("Enter API key (optional — press Enter to skip):")
                        ),
                    )?;
                    if key.trim().is_empty() {
                        println!(
                            "  {}",
                            t::icon_ok("No key — connecting without authentication.")
                        );
                    } else {
                        secrets.store_secret(secret_key, key.trim())?;
                        println!("  {}", t::icon_ok("API key stored securely."));
                    }
                }
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
    } else if available_models.len() > 20 {
        // Large list — use fuzzy search for better UX
        match fuzzy_select(
            &available_models,
            "Select a default model (type to filter):",
        )? {
            Some(idx) => available_models[idx].clone(),
            None => {
                println!("  {}", t::warn("Cancelled — no model selected."));
                String::new()
            }
        }
    } else {
        // Small list — use simple arrow select
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
