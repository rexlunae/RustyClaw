//! (onboard submodule)

use std::io::{self, BufRead, Write};

use anyhow::{Context, Result};

use crate::prompts::{print_qr_code, prompt_line, whoami};
use rustyclaw_core::config::Config;
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::theme as t;

/// Perform OAuth device flow authentication and store the token.
pub(crate) fn perform_device_flow_auth(
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

/// Walk the user through TOTP 2FA enrollment.
pub(crate) fn setup_totp_enrollment(
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
pub(crate) fn verify_totp_loop(
    reader: &mut impl BufRead,
    secrets: &mut SecretsManager,
) -> Result<()> {
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
pub(crate) fn setup_agent_ssh_key(
    reader: &mut impl BufRead,
    secrets: &mut SecretsManager,
) -> Result<()> {
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
