//! `rustyclaw import` — migrate configuration and credentials from an
//! existing OpenClaw installation.

use anyhow::{Context, Result};
use rustyclaw_core::config::Config;
use rustyclaw_core::providers;
use rustyclaw_core::secrets::SecretsManager;

use crate::ImportArgs;

pub(crate) fn run_import(args: &ImportArgs, config: &mut Config) -> Result<()> {
    use colored::Colorize;
    use rpassword::read_password;
    use std::fs;
    use std::io::{BufRead, Write};
    use std::path::PathBuf;

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();

    let home = dirs::home_dir().context("Could not determine home directory")?;
    let source_dir = args
        .source
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".openclaw"));

    if !source_dir.exists() {
        anyhow::bail!(
            "OpenClaw directory not found: {}\n\
             Specify the path with: rustyclaw import /path/to/.openclaw",
            source_dir.display()
        );
    }

    let target_dir = args
        .target
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".rustyclaw"));

    println!();
    println!("{}", "━".repeat(60).dimmed());
    println!("{}", "  RustyClaw Import Wizard".cyan().bold());
    println!("{}", "━".repeat(60).dimmed());
    println!();
    println!(
        "  {} {}",
        "From:".bold(),
        source_dir.display().to_string().yellow()
    );
    println!(
        "  {}   {}",
        "To:".bold(),
        target_dir.display().to_string().green()
    );

    if args.dry_run {
        println!();
        println!("{}", "  (dry run — no changes will be made)".dimmed());
    }

    // ── Detect what's available to import ───────────────────────────────
    let source_workspace = source_dir.join("workspace");
    let source_credentials = source_dir.join("credentials");
    let openclaw_config_path = source_dir.join("openclaw.json");

    let has_workspace = source_workspace.exists();
    let has_credentials = source_credentials.exists();
    let has_config = openclaw_config_path.exists();

    println!();
    println!("{}", "  Available to import:".bold());
    if has_config {
        println!("    {} Configuration (model, settings)", "•".cyan());
    }
    if has_workspace {
        println!(
            "    {} Workspace files (SOUL.md, memory/, etc.)",
            "•".cyan()
        );
    }
    if has_credentials {
        println!("    {} API credentials", "•".cyan());
    }
    println!();

    // ── Confirm import ──────────────────────────────────────────────────
    print!("{} ", "Proceed with import? [Y/n]:".cyan());
    std::io::stdout().flush()?;
    let mut response = String::new();
    reader.read_line(&mut response)?;
    if response.trim().eq_ignore_ascii_case("n") {
        println!("  {}", "Import cancelled.".yellow());
        return Ok(());
    }

    // Create target directories
    let target_workspace = target_dir.join("workspace");
    let target_credentials = target_dir.join("credentials");

    if !args.dry_run {
        fs::create_dir_all(&target_dir).context("Failed to create target directory")?;
        fs::create_dir_all(&target_workspace).context("Failed to create workspace directory")?;
        fs::create_dir_all(&target_credentials)
            .context("Failed to create credentials directory")?;
    }

    let mut imported_count = 0;
    let mut skipped_count = 0;

    // ── Import configuration ────────────────────────────────────────────
    if has_config {
        println!();
        println!("{}", "━".repeat(60).dimmed());
        println!("{}", "Configuration".cyan().bold());
        println!("{}", "━".repeat(60).dimmed());

        if let Ok(content) = fs::read_to_string(&openclaw_config_path) {
            if let Ok(oc_config) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(model_str) = oc_config
                    .pointer("/agents/defaults/model/primary")
                    .and_then(|v| v.as_str())
                {
                    let parts: Vec<&str> = model_str.splitn(2, '/').collect();
                    if parts.len() == 2 {
                        config.model = Some(rustyclaw_core::config::ModelProvider {
                            provider: parts[0].to_string(),
                            model: Some(parts[1].to_string()),
                            base_url: None,
                        });
                        println!("  {} Model: {}", "✓".green(), model_str.cyan());
                        imported_count += 1;
                    }
                }
            }
        }
    }

    // ── Import workspace files ──────────────────────────────────────────
    if has_workspace {
        println!();
        println!("{}", "━".repeat(60).dimmed());
        println!("{}", "Workspace Files".cyan().bold());
        println!("{}", "━".repeat(60).dimmed());

        print!(
            "{} ",
            "Import workspace files (SOUL.md, AGENTS.md, memory/, etc.)? [Y/n]:".cyan()
        );
        std::io::stdout().flush()?;
        let mut response = String::new();
        reader.read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("n") {
            let workspace_files = [
                "SOUL.md",
                "AGENTS.md",
                "TOOLS.md",
                "USER.md",
                "IDENTITY.md",
                "HEARTBEAT.md",
                "MEMORY.md",
            ];

            for file in &workspace_files {
                let src = source_workspace.join(file);
                let dst = target_workspace.join(file);

                if src.exists() {
                    if dst.exists() && !args.force {
                        println!(
                            "  {} {} (exists, use --force to overwrite)",
                            "⊘".yellow(),
                            file
                        );
                        skipped_count += 1;
                    } else {
                        if !args.dry_run {
                            fs::copy(&src, &dst)
                                .with_context(|| format!("Failed to copy {}", file))?;
                        }
                        println!("  {} {}", "✓".green(), file);
                        imported_count += 1;
                    }
                }
            }

            // Import memory/ directory
            let src_memory = source_workspace.join("memory");
            let dst_memory = target_workspace.join("memory");
            if src_memory.exists() && src_memory.is_dir() {
                if !args.dry_run {
                    fs::create_dir_all(&dst_memory)?;
                }

                let mut memory_count = 0;
                for entry in fs::read_dir(&src_memory)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() {
                        let file_name = path.file_name().unwrap();
                        let dst_file = dst_memory.join(file_name);

                        if dst_file.exists() && !args.force {
                            skipped_count += 1;
                        } else {
                            if !args.dry_run {
                                fs::copy(&path, &dst_file)?;
                            }
                            memory_count += 1;
                        }
                    }
                }
                if memory_count > 0 {
                    println!("  {} memory/ ({} files)", "✓".green(), memory_count);
                    imported_count += memory_count;
                }
            }

            // Import skills/ directory (recursive)
            let src_skills = source_workspace.join("skills");
            let dst_skills = target_workspace.join("skills");
            if src_skills.exists() && src_skills.is_dir() {
                if !args.dry_run {
                    fs::create_dir_all(&dst_skills)?;
                }

                let mut skills_count = 0;
                fn copy_dir_recursive(
                    src: &std::path::Path,
                    dst: &std::path::Path,
                    dry_run: bool,
                    force: bool,
                    count: &mut usize,
                    skipped: &mut usize,
                ) -> Result<()> {
                    if !dry_run {
                        fs::create_dir_all(dst)?;
                    }
                    for entry in fs::read_dir(src)? {
                        let entry = entry?;
                        let path = entry.path();
                        let file_name = path.file_name().unwrap();
                        let dst_path = dst.join(file_name);

                        if path.is_dir() {
                            copy_dir_recursive(&path, &dst_path, dry_run, force, count, skipped)?;
                        } else {
                            if dst_path.exists() && !force {
                                *skipped += 1;
                            } else {
                                if !dry_run {
                                    fs::copy(&path, &dst_path)?;
                                }
                                *count += 1;
                            }
                        }
                    }
                    Ok(())
                }

                copy_dir_recursive(
                    &src_skills,
                    &dst_skills,
                    args.dry_run,
                    args.force,
                    &mut skills_count,
                    &mut skipped_count,
                )?;

                if skills_count > 0 {
                    println!("  {} skills/ ({} files)", "✓".green(), skills_count);
                    imported_count += skills_count;
                }
            }

            // Extract agent name from IDENTITY.md as default, then prompt
            let mut default_name = String::new();
            let identity_path = source_workspace.join("IDENTITY.md");
            if identity_path.exists() {
                if let Ok(content) = fs::read_to_string(&identity_path) {
                    // Look for "- **Name:** <name>" pattern
                    for line in content.lines() {
                        let line = line.trim();
                        if line.starts_with("- **Name:**") || line.starts_with("**Name:**") {
                            if let Some(name) = line.split(":**").nth(1) {
                                let name = name.trim();
                                if !name.is_empty() {
                                    default_name = name.to_string();
                                }
                            }
                            break;
                        }
                    }
                }
            }

            // Prompt for agent name with default from IDENTITY.md
            println!();
            if default_name.is_empty() {
                print!("{} ", "Agent name:".cyan());
            } else {
                print!("{} ", format!("Agent name [{}]:", default_name).cyan());
            }
            std::io::stdout().flush()?;
            let mut name_input = String::new();
            reader.read_line(&mut name_input)?;
            let name_input = name_input.trim();

            if name_input.is_empty() && !default_name.is_empty() {
                config.agent_name = default_name.clone();
                println!("  {} Agent name: {}", "✓".green(), default_name.cyan());
            } else if !name_input.is_empty() {
                config.agent_name = name_input.to_string();
                println!("  {} Agent name: {}", "✓".green(), name_input.cyan());
            } else {
                println!("  {}", "Using default agent name.".dimmed());
            }
        } else {
            println!("  {}", "Skipping workspace files.".dimmed());
        }
    }

    // ── Credentials import ──────────────────────────────────────────────
    if has_credentials {
        println!();
        println!("{}", "━".repeat(60).dimmed());
        println!("{}", "Credentials".cyan().bold());
        println!("{}", "━".repeat(60).dimmed());

        // List available credentials
        let credential_files = [
            ("github-copilot.token.json", "GitHub Copilot"),
            ("anthropic.key", "Anthropic"),
            ("openai.key", "OpenAI"),
            ("openrouter.key", "OpenRouter"),
            ("opencode.key", "OpenCode Zen"),
            ("gemini.key", "Gemini"),
            ("xai.key", "xAI"),
        ];

        let mut found_creds: Vec<(&str, &str)> = Vec::new();
        for (file, name) in &credential_files {
            if source_credentials.join(file).exists() {
                found_creds.push((file, name));
            }
        }

        if found_creds.is_empty() {
            println!("  {}", "No credentials found to import.".dimmed());
        } else {
            println!("  Found credentials:");
            for (_, name) in &found_creds {
                println!("    {} {}", "•".cyan(), name);
            }
            println!();

            print!("{} ", "Import these credentials? [Y/n]:".cyan());
            std::io::stdout().flush()?;
            let mut response = String::new();
            reader.read_line(&mut response)?;

            if !response.trim().eq_ignore_ascii_case("n") {
                // Need to release stdin lock before password prompt
                drop(reader);

                // ── Vault security setup ────────────────────────────────
                println!();
                println!("{}", "━".repeat(60).dimmed());
                println!("{}", "Vault Security Setup".cyan().bold());
                println!("{}", "━".repeat(60).dimmed());
                println!();
                println!("  Your credentials will be stored in an encrypted vault.");
                println!("  You can add a password for additional security.");
                println!();
                println!(
                    "  {}  With a password, you'll need to enter it each time",
                    "⚠".yellow()
                );
                println!("     you start the agent. Without one, an auto-generated");
                println!("     key file protects the vault instead.");
                println!();

                let mut secrets = SecretsManager::new(&target_credentials);

                // Password setup
                print!("{} ", "Vault password (leave blank to skip):".cyan());
                std::io::stdout().flush()?;
                let password = read_password().unwrap_or_default();

                if password.trim().is_empty() {
                    println!("  {}", "✓ Using auto-generated key file.".green());
                    config.secrets_password_protected = false;
                } else {
                    print!("{} ", "Confirm password:".cyan());
                    std::io::stdout().flush()?;
                    let confirm = read_password().unwrap_or_default();

                    if password != confirm {
                        anyhow::bail!("Passwords do not match. Import cancelled.");
                    }

                    secrets.set_password(password);
                    config.secrets_password_protected = true;
                    println!("  {}", "✓ Vault will be password-protected.".green());
                }

                // Re-acquire stdin for TOTP setup
                let stdin = std::io::stdin();
                let mut reader = stdin.lock();

                // TOTP setup
                println!();
                println!("{}", "Two-Factor Authentication (optional)".cyan().bold());
                println!();
                println!("  Add TOTP 2FA using any authenticator app.");
                println!();

                print!("{} ", "Enable 2FA? [y/N]:".cyan());
                std::io::stdout().flush()?;
                let mut response = String::new();
                reader.read_line(&mut response)?;

                if response.trim().eq_ignore_ascii_case("y") {
                    // Initialize vault
                    if !args.dry_run {
                        secrets.store_secret("__init", "")?;
                        secrets.delete_secret("__init")?;
                    }

                    let account = std::env::var("USER")
                        .or_else(|_| std::env::var("USERNAME"))
                        .unwrap_or_else(|_| "user".to_string());
                    let agent_name = config.agent_name.clone();

                    if !args.dry_run {
                        let otpauth_url = secrets.setup_totp_with_issuer(&account, &agent_name)?;

                        println!();
                        println!("  {}", "Scan this QR code:".bold());
                        println!();
                        print_qr_code_import(&otpauth_url);
                        println!();
                        println!("  {}", otpauth_url.dimmed());
                        println!();

                        loop {
                            print!("{} ", "Enter 6-digit code to verify:".cyan());
                            std::io::stdout().flush()?;
                            let mut code = String::new();
                            reader.read_line(&mut code)?;
                            let code = code.trim();

                            if code.is_empty() {
                                println!("  {}", "⚠ 2FA setup cancelled.".yellow());
                                secrets.remove_totp()?;
                                break;
                            }

                            match secrets.verify_totp(code) {
                                Ok(true) => {
                                    config.totp_enabled = true;
                                    println!("  {}", "✓ 2FA enabled.".green());
                                    break;
                                }
                                Ok(false) => {
                                    println!("  {}", "⚠ Invalid code. Try again:".yellow());
                                }
                                Err(e) => {
                                    println!(
                                        "  {}",
                                        format!("⚠ Error: {}. 2FA not enabled.", e).yellow()
                                    );
                                    secrets.remove_totp()?;
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    println!("  {}", "Skipping 2FA.".dimmed());
                }

                // Now import the credentials
                println!();
                println!("{}", "Importing credentials...".cyan());

                let secret_map = [
                    ("github-copilot.token.json", "GITHUB_COPILOT_TOKEN"),
                    ("anthropic.key", "ANTHROPIC_API_KEY"),
                    ("openai.key", "OPENAI_API_KEY"),
                    ("openrouter.key", "OPENROUTER_API_KEY"),
                    ("opencode.key", "OPENCODE_API_KEY"),
                    ("gemini.key", "GEMINI_API_KEY"),
                    ("xai.key", "XAI_API_KEY"),
                ];

                for (file, secret_name) in &secret_map {
                    // GitHub Copilot: try to import the session token directly
                    if *file == "github-copilot.token.json" {
                        let src = source_credentials.join(file);
                        if src.exists() {
                            if let Ok(content) = fs::read_to_string(&src) {
                                if let Ok(json) =
                                    serde_json::from_str::<serde_json::Value>(&content)
                                {
                                    let token = json.get("token").and_then(|v| v.as_str());
                                    let expires_at = json.get("expiresAt").and_then(|v| v.as_i64());

                                    if let (Some(token), Some(expires_at)) = (token, expires_at) {
                                        // expiresAt is in milliseconds, convert to seconds
                                        let expires_at_secs = expires_at / 1000;
                                        let now = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs()
                                            as i64;

                                        if expires_at_secs > now + 300 {
                                            // Token still valid for at least 5 minutes
                                            // Store as JSON with token and expiration
                                            let session_data = serde_json::json!({
                                                "session_token": token,
                                                "expires_at": expires_at_secs,
                                            });
                                            if !args.dry_run {
                                                secrets.store_secret(
                                                    "GITHUB_COPILOT_SESSION",
                                                    &session_data.to_string(),
                                                )?;
                                            }
                                            let hours_left = (expires_at_secs - now) / 3600;
                                            println!(
                                                "  {} {} (session token, ~{}h remaining)",
                                                "✓".green(),
                                                secret_name,
                                                hours_left
                                            );
                                            imported_count += 1;
                                            continue;
                                        } else {
                                            println!(
                                                "  {} {} (session expired, needs re-auth)",
                                                "⊘".yellow(),
                                                secret_name
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        // Fall through to re-auth prompt below
                        skipped_count += 1;
                        continue;
                    }

                    let src = source_credentials.join(file);
                    if src.exists() {
                        if let Ok(content) = fs::read_to_string(&src) {
                            let token = if file.ends_with(".json") {
                                serde_json::from_str::<serde_json::Value>(&content)
                                    .ok()
                                    .and_then(|json| {
                                        json.get("access_token")
                                            .or_else(|| json.get("token"))
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .or_else(|| Some(content.trim().to_string()))
                            } else {
                                Some(content.trim().to_string())
                            };

                            if let Some(token) = token {
                                if !token.is_empty() && !args.dry_run {
                                    secrets.store_secret(secret_name, &token)?;
                                    println!("  {} {}", "✓".green(), secret_name);
                                    imported_count += 1;
                                }
                            }
                        }
                    }
                }

                // Prompt for GitHub Copilot re-authentication
                println!();
                println!("{}", "GitHub Copilot Authentication".cyan().bold());
                println!("  OpenClaw stores session tokens that can't be migrated.");
                println!("  You'll need to re-authenticate with GitHub.");
                println!();
                print!("{} ", "Authenticate with GitHub Copilot now? [Y/n]:".cyan());
                std::io::stdout().flush()?;
                let mut response = String::new();
                reader.read_line(&mut response)?;

                if !response.trim().eq_ignore_ascii_case("n") {
                    // Re-use the device flow auth
                    use providers::GITHUB_COPILOT_DEVICE_FLOW;
                    let device_config = &GITHUB_COPILOT_DEVICE_FLOW;

                    println!();
                    println!("{}", "Starting GitHub device flow...".cyan());

                    let handle = tokio::runtime::Handle::current();
                    match tokio::task::block_in_place(|| {
                        handle.block_on(providers::start_device_flow(device_config))
                    }) {
                        Ok(auth_response) => {
                            println!();
                            println!("  {}", "Please complete the following steps:".bold());
                            println!();
                            println!("  1. Visit: {}", auth_response.verification_uri.cyan());
                            println!("  2. Enter code: {}", auth_response.user_code.cyan().bold());
                            println!();

                            print!(
                                "{} ",
                                "Press Enter after completing authorization (or type 'cancel'):"
                                    .cyan()
                            );
                            std::io::stdout().flush()?;
                            let mut response = String::new();
                            reader.read_line(&mut response)?;

                            if !response.trim().eq_ignore_ascii_case("cancel")
                                && !response.trim().eq_ignore_ascii_case("c")
                            {
                                println!("  {}", "Waiting for authorization...".dimmed());

                                let interval =
                                    std::time::Duration::from_secs(auth_response.interval);
                                let max_attempts =
                                    (auth_response.expires_in / auth_response.interval).max(10);

                                let mut token: Option<String> = None;
                                for _attempt in 0..max_attempts {
                                    match tokio::task::block_in_place(|| {
                                        handle.block_on(providers::poll_device_token(
                                            device_config,
                                            &auth_response.device_code,
                                        ))
                                    }) {
                                        Ok(Some(access_token)) => {
                                            token = Some(access_token);
                                            break;
                                        }
                                        Ok(None) => {
                                            print!(".");
                                            std::io::stdout().flush()?;
                                            std::thread::sleep(interval);
                                        }
                                        Err(e) => {
                                            println!();
                                            println!(
                                                "  {}",
                                                format!("⚠ Authentication failed: {}", e).yellow()
                                            );
                                            break;
                                        }
                                    }
                                }
                                println!();

                                if let Some(access_token) = token {
                                    if !args.dry_run {
                                        secrets
                                            .store_secret("GITHUB_COPILOT_TOKEN", &access_token)?;
                                    }
                                    println!("  {}", "✓ GitHub Copilot authenticated!".green());
                                    imported_count += 1;
                                } else {
                                    println!("  {}", "⚠ Authentication timed out.".yellow());
                                }
                            } else {
                                println!("  {}", "Skipping GitHub Copilot.".dimmed());
                            }
                        }
                        Err(e) => {
                            println!(
                                "  {}",
                                format!("⚠ Failed to start device flow: {}", e).yellow()
                            );
                        }
                    }
                } else {
                    println!("  {}", "Skipping GitHub Copilot.".dimmed());
                }
            } else {
                println!("  {}", "Skipping credentials.".dimmed());
            }
        }
    }

    // ── Save config ─────────────────────────────────────────────────────
    if !args.dry_run {
        config.settings_dir = target_dir.clone();
        config.workspace_dir = Some(target_workspace.clone());
        config.credentials_dir = Some(target_credentials);
        config.save(Some(target_dir.join("config.toml")))?;
    }

    // ── Summary ─────────────────────────────────────────────────────────
    println!();
    println!("{}", "━".repeat(60).dimmed());
    println!(
        "{} Import complete: {} items imported, {} skipped",
        "✓".green().bold(),
        imported_count.to_string().green(),
        skipped_count.to_string().yellow()
    );

    if config.secrets_password_protected {
        println!("  🔒 Vault is password-protected");
    }
    if config.totp_enabled {
        println!("  🔐 2FA is enabled");
    }

    if args.dry_run {
        println!();
        println!("{}", "Run without --dry-run to apply changes.".dimmed());
    } else {
        println!();
        println!(
            "{} Saved to {}",
            "✓".green(),
            target_dir.join("config.toml").display()
        );
        println!();
        println!(
            "{} Run {} to launch your agent!",
            "→".cyan(),
            "rustyclaw tui".green()
        );
    }

    Ok(())
}

/// Print a QR code to the terminal (simplified version for import)
fn print_qr_code_import(data: &str) {
    use qrcode::{QrCode, render::unicode};

    if let Ok(code) = QrCode::new(data.as_bytes()) {
        let image = code
            .render::<unicode::Dense1x2>()
            .dark_color(unicode::Dense1x2::Light)
            .light_color(unicode::Dense1x2::Dark)
            .build();
        for line in image.lines() {
            println!("    {}", line);
        }
    } else {
        println!("  (Could not generate QR code)");
    }
}
