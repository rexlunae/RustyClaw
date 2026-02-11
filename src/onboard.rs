//! Interactive onboarding wizard.
//!
//! Mirrors the openclaw `onboard` command: walks the user through selecting a
//! model provider, storing an API key, picking a default model, and
//! initialising the SOUL.

use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};

use crate::config::{Config, ModelProvider};
use crate::secrets::SecretsManager;
use crate::soul::{SoulManager, DEFAULT_SOUL_CONTENT};

// ── Provider catalogue ──────────────────────────────────────────────────────

/// A provider definition with its secret key name and available models.
struct ProviderDef {
    id: &'static str,
    display: &'static str,
    secret_key: Option<&'static str>,
    base_url: Option<&'static str>,
    models: &'static [&'static str],
}

const PROVIDERS: &[ProviderDef] = &[
    ProviderDef {
        id: "anthropic",
        display: "Anthropic (Claude)",
        secret_key: Some("ANTHROPIC_API_KEY"),
        base_url: Some("https://api.anthropic.com"),
        models: &[
            "claude-opus-4-20250514",
            "claude-sonnet-4-20250514",
            "claude-haiku-4-20250514",
        ],
    },
    ProviderDef {
        id: "openai",
        display: "OpenAI (GPT / o-series)",
        secret_key: Some("OPENAI_API_KEY"),
        base_url: Some("https://api.openai.com/v1"),
        models: &[
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-nano",
            "o3",
            "o4-mini",
        ],
    },
    ProviderDef {
        id: "google",
        display: "Google (Gemini)",
        secret_key: Some("GEMINI_API_KEY"),
        base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
        models: &[
            "gemini-2.5-pro",
            "gemini-2.5-flash",
            "gemini-2.0-flash",
        ],
    },
    ProviderDef {
        id: "xai",
        display: "xAI (Grok)",
        secret_key: Some("XAI_API_KEY"),
        base_url: Some("https://api.x.ai/v1"),
        models: &["grok-3", "grok-3-mini"],
    },
    ProviderDef {
        id: "openrouter",
        display: "OpenRouter",
        secret_key: Some("OPENROUTER_API_KEY"),
        base_url: Some("https://openrouter.ai/api/v1"),
        models: &[
            "anthropic/claude-opus-4-20250514",
            "anthropic/claude-sonnet-4-20250514",
            "openai/gpt-4.1",
            "google/gemini-2.5-pro",
        ],
    },
    ProviderDef {
        id: "ollama",
        display: "Ollama (local)",
        secret_key: None,
        base_url: Some("http://localhost:11434/v1"),
        models: &[
            "llama3.1",
            "mistral",
            "codellama",
            "deepseek-coder",
        ],
    },
    ProviderDef {
        id: "custom",
        display: "Custom / OpenAI-compatible endpoint",
        secret_key: Some("CUSTOM_API_KEY"),
        base_url: None, // will prompt
        models: &[],
    },
];

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
    println!("┌──────────────────────────────────────────┐");
    println!("│      🦀  RustyClaw Onboarding  🦀       │");
    println!("└──────────────────────────────────────────┘");
    println!();

    // ── Optional reset ─────────────────────────────────────────────
    if reset {
        println!("Resetting configuration…");
        *config = Config::default();
    }

    // ── 1. Select model provider ───────────────────────────────────
    println!("Select a model provider:");
    println!();
    for (i, p) in PROVIDERS.iter().enumerate() {
        println!("  {}. {}", i + 1, p.display);
    }
    println!();

    let provider = loop {
        let choice = prompt_line(&mut reader, &format!("Provider [1-{}]: ", PROVIDERS.len()))?;
        if let Ok(n) = choice.trim().parse::<usize>() {
            if n >= 1 && n <= PROVIDERS.len() {
                break &PROVIDERS[n - 1];
            }
        }
        println!("  Please enter a number between 1 and {}.", PROVIDERS.len());
    };

    println!();
    println!("  ✓ Selected: {}", provider.display);
    println!();

    // ── 1b. Optional password for secrets vault ────────────────────
    let vault_exists = config.credentials_dir().join("secrets.json").exists();
    if !vault_exists {
        println!("You can protect your secrets vault with a password.");
        println!("If you skip this, a key file will be generated instead.");
        println!();
        println!("  ⚠  If you set a password you will need to enter it every");
        println!("     time RustyClaw starts, including when the gateway is");
        println!("     launched.  Automated / unattended starts will not be");
        println!("     possible without the password.");
        println!();

        let pw = prompt_secret(&mut reader, "Vault password (leave blank to skip): ")?;
        let pw = pw.trim().to_string();

        if pw.is_empty() {
            println!("  ✓ Using auto-generated key file (no password).");
            config.secrets_password_protected = false;
        } else {
            let confirm = prompt_secret(&mut reader, "Confirm password: ")?;
            if confirm.trim() != pw {
                println!("  ⚠ Passwords do not match — falling back to key file.");
                config.secrets_password_protected = false;
            } else {
                secrets.set_password(pw);
                config.secrets_password_protected = true;
                println!("  ✓ Secrets vault will be password-protected.");
            }
        }
        println!();
    } else if config.secrets_password_protected {
        // Vault already exists with a password — make sure SecretsManager has it.
        let pw = prompt_secret(&mut reader, "Enter vault password: ")?;
        secrets.set_password(pw.trim().to_string());
        println!();
    }

    // ── 2. API key ─────────────────────────────────────────────────
    if let Some(secret_key) = provider.secret_key {
        // Check if we already have one stored.
        let existing = secrets.get_secret(secret_key, true)?;
        if let Some(_) = &existing {
            let reuse = prompt_line(
                &mut reader,
                &format!("An API key for {} is already stored. Keep it? [Y/n]: ", provider.display),
            )?;
            if reuse.trim().eq_ignore_ascii_case("n") {
                let key = prompt_secret(&mut reader, "Enter API key: ")?;
                if key.trim().is_empty() {
                    println!("  ⚠ No key entered — keeping existing key.");
                } else {
                    secrets.store_secret(secret_key, key.trim())?;
                    println!("  ✓ API key updated.");
                }
            } else {
                println!("  ✓ Keeping existing API key.");
            }
        } else {
            let key = prompt_secret(&mut reader, "Enter API key: ")?;
            if key.trim().is_empty() {
                println!("  ⚠ No key entered — you can add one later with:");
                println!("      rustyclaw onboard");
            } else {
                secrets.store_secret(secret_key, key.trim())?;
                println!("  ✓ API key stored securely.");
            }
        }
        println!();
    }

    // ── 3. Base URL (only for custom) ──────────────────────────────
    let base_url: String = if provider.id == "custom" {
        let url = prompt_line(&mut reader, "Base URL (OpenAI-compatible): ")?;
        let url = url.trim().to_string();
        if url.is_empty() {
            println!("  ⚠ No URL entered. You can set model.base_url in config.toml later.");
            String::new()
        } else {
            println!("  ✓ Base URL: {}", url);
            url
        }
    } else {
        provider.base_url.unwrap_or("").to_string()
    };

    // ── 4. Select a model ──────────────────────────────────────────
    let model: String = if provider.models.is_empty() {
        // Custom provider — ask for a model name.
        let m = prompt_line(&mut reader, "Model name: ")?;
        m.trim().to_string()
    } else {
        println!("Select a default model:");
        println!();
        for (i, m) in provider.models.iter().enumerate() {
            println!("  {}. {}", i + 1, m);
        }
        println!();

        loop {
            let choice = prompt_line(
                &mut reader,
                &format!("Model [1-{}]: ", provider.models.len()),
            )?;
            if let Ok(n) = choice.trim().parse::<usize>() {
                if n >= 1 && n <= provider.models.len() {
                    break provider.models[n - 1].to_string();
                }
            }
            println!(
                "  Please enter a number between 1 and {}.",
                provider.models.len()
            );
        }
    };

    if !model.is_empty() {
        println!();
        println!("  ✓ Default model: {}", model);
    }

    // ── 5. Initialize / update SOUL.md ─────────────────────────────
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
            "SOUL.md has been customised. Reset to default? [y/N]: ",
        )?;
        answer.trim().eq_ignore_ascii_case("y")
    } else {
        true
    };

    if init_soul {
        let mut soul = SoulManager::new(soul_path.clone());
        soul.load()?;
        println!("  ✓ SOUL.md initialised at {}", soul_path.display());
    } else {
        println!("  ✓ Keeping existing SOUL.md");
    }

    // ── 6. Write config ────────────────────────────────────────────
    config.model = Some(ModelProvider {
        provider: provider.id.to_string(),
        model: if model.is_empty() {
            None
        } else {
            Some(model)
        },
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
    });

    // Ensure the full directory skeleton exists and save.
    config.ensure_dirs()
        .context("Failed to create directory structure")?;
    config.save(None)?;

    println!();
    println!("┌──────────────────────────────────────────┐");
    println!("│        Onboarding complete! 🎉           │");
    println!("└──────────────────────────────────────────┘");
    println!();
    println!(
        "Config saved to {}",
        config.settings_dir.join("config.toml").display()
    );
    println!("Run `rustyclaw tui` to start the TUI.");
    println!();

    Ok(true)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn prompt_line(reader: &mut impl BufRead, prompt: &str) -> Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;
    let mut buf = String::new();
    reader.read_line(&mut buf)?;
    Ok(buf.trim_end_matches('\n').trim_end_matches('\r').to_string())
}

fn prompt_secret(reader: &mut impl BufRead, prompt: &str) -> Result<String> {
    // If stdin is a tty we could disable echo, but for simplicity we just
    // read a normal line.  A future improvement can use `rpassword`.
    prompt_line(reader, prompt)
}
