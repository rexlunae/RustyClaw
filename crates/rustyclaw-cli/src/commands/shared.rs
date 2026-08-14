//! Helpers shared across CLI command modules — secrets-vault access and
//! interactive password prompting.

use anyhow::{Context, Result};
use rustyclaw_core::config::Config;
use rustyclaw_core::secrets::SecretsManager;

/// Whether opening this installation's vault needs a password.
///
/// The config flag records what onboarding chose; the vault on disk is the
/// authority, and the two disagree whenever the config has been replaced by
/// defaults (a hand-edited file, or `Config::load` quarantining a torn one).
/// Asking the flag alone left an encrypted vault with no prompt anywhere —
/// not here, and not in the gateway, which applies the same rule.
fn needs_vault_password(config: &Config) -> bool {
    config.secrets_password_protected || SecretsManager::requires_password(config.credentials_dir())
}

/// Extract the vault password for the gateway daemon.
///
/// If the vault is password-protected, prompt the user for it.  The
/// password will be passed to the daemon via an environment variable
/// so it can open the secrets vault on startup.
pub(crate) fn extract_vault_password(config: &Config) -> Option<String> {
    if !needs_vault_password(config) {
        return None;
    }
    match prompt_password(&format!(
        "{} Vault password (for gateway): ",
        rustyclaw_core::theme::info("🔑"),
    )) {
        Ok(pw) if !pw.is_empty() => Some(pw),
        _ => None,
    }
}

/// Open the secrets vault, prompting for a password and TOTP if required.
///
/// NOTE: Under the new security model, TOTP is only verified by the
/// gateway at WebSocket connect time.  The CLI `open_secrets` is only
/// used during onboarding and ad-hoc CLI vault access.
pub(crate) fn open_secrets(config: &Config) -> Result<SecretsManager> {
    let mut manager = if needs_vault_password(config) {
        let pw = prompt_password("Enter secrets vault password: ")?;
        SecretsManager::with_password(config.credentials_dir(), pw)
    } else {
        SecretsManager::new(config.credentials_dir())
    };

    // If TOTP 2FA is enabled, verify before returning. Asked of the vault
    // for the same reason as the password above: the enrolled secret is the
    // authority and the config flag is only what onboarding chose.
    if manager.totp_required(config.totp_enabled) {
        loop {
            let code = prompt_password("Enter your 2FA code: ")?;
            match manager.verify_totp(code.trim()) {
                Ok(true) => break,
                Ok(false) => {
                    eprintln!("Invalid code. Please try again.");
                }
                Err(e) => {
                    anyhow::bail!("2FA verification failed: {}", e);
                }
            }
        }
    }

    Ok(manager)
}

pub(crate) fn prompt_password(prompt: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{}", prompt);
    io::stdout().flush()?;
    let input = rpassword::read_password().context("Failed to read password")?;
    Ok(input.trim().to_string())
}
