//! `rustyclaw refresh-token` — re-import OAuth/session tokens from an
//! existing OpenClaw installation into the RustyClaw vault.

use anyhow::{Context, Result};
use clap::Args;
use rustyclaw_core::config::Config;
use std::path::PathBuf;

use crate::commands::shared::open_secrets;

/// Arguments for `rustyclaw refresh-token`.
#[derive(Debug, Args)]
pub struct RefreshTokenArgs {
    /// Path to the OpenClaw directory (default: ~/.openclaw)
    #[arg(long, value_name = "PATH")]
    pub openclaw_dir: Option<String>,
    /// Restart the gateway after refreshing
    #[arg(long)]
    pub restart: bool,
}

pub(crate) fn run_refresh_token(args: &RefreshTokenArgs, config: &mut Config) -> Result<()> {
    use colored::Colorize;
    use std::fs;

    let openclaw_dir = args
        .openclaw_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".openclaw"));

    let token_file = openclaw_dir.join("credentials/github-copilot.token.json");

    if !token_file.exists() {
        anyhow::bail!(
            "GitHub Copilot token file not found at: {}\nRun `openclaw onboard` first to authenticate.",
            token_file.display()
        );
    }

    let content = fs::read_to_string(&token_file)
        .with_context(|| format!("Failed to read {}", token_file.display()))?;

    let json: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "Failed to parse token file as JSON")?;

    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Token file missing 'token' field"))?;

    let expires_at = json
        .get("expiresAt")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow::anyhow!("Token file missing 'expiresAt' field"))?;

    // expiresAt is in milliseconds, convert to seconds
    let expires_at_secs = expires_at / 1000;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    if expires_at_secs <= now + 60 {
        anyhow::bail!("OpenClaw token has expired. Run `openclaw onboard` to re-authenticate.");
    }

    let hours_left = (expires_at_secs - now) / 3600;
    let mins_left = ((expires_at_secs - now) % 3600) / 60;

    // Store in RustyClaw's vault
    let mut secrets = open_secrets(config)?;
    let session_data = serde_json::json!({
        "session_token": token,
        "expires_at": expires_at_secs,
    });
    secrets.store_secret("GITHUB_COPILOT_SESSION", &session_data.to_string())?;

    println!(
        "{} Imported GitHub Copilot session token (~{}h {}m remaining)",
        "✓".green(),
        hours_left,
        mins_left
    );

    if args.restart {
        println!("{}", "Restarting gateway...".cyan());
        // Send SIGHUP to gateway if running
        let pid_file = config.settings_dir.join("gateway.pid");
        if pid_file.exists() {
            if let Ok(pid_str) = fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid_str.trim().parse::<i32>() {
                    #[cfg(unix)]
                    {
                        unsafe {
                            libc::kill(pid, libc::SIGHUP);
                        }
                        println!(
                            "{} Sent reload signal to gateway (pid {})",
                            "✓".green(),
                            pid
                        );
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = pid; // Suppress unused warning
                        println!("{}", "Signal sending not supported on Windows. Please restart gateway manually.".yellow());
                    }
                }
            }
        } else {
            println!("{}", "Gateway not running (no pid file).".yellow());
        }
    } else {
        println!(
            "{}",
            "Restart the gateway to use the new token: rustyclaw gateway restart".dimmed()
        );
    }

    Ok(())
}
