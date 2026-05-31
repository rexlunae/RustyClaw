//! GitHub Copilot session bootstrap.
//!
//! Copilot providers need a short-lived *session token* (exchanged from the
//! long-lived OAuth token) before any model call. These helpers resolve a
//! usable [`CopilotSession`] from the vault, an OpenClaw import, or the OAuth
//! token in the model context, and are used both at gateway startup and on
//! live model/config changes.

use std::sync::Arc;

use tracing::{debug, info, warn};

use rustyclaw_core::gateway::CopilotSession;
use rustyclaw_core::providers as crate_providers;
use rustyclaw_core::secrets::SecretsManager;

use crate::SharedVault;

/// Try to import a fresh GitHub Copilot token from OpenClaw's credential store.
///
/// Reads `~/.openclaw/credentials/github-copilot.token.json`; if it holds a
/// still-valid session token, caches it in our vault and returns a session.
fn try_import_openclaw_token(vault: &mut SecretsManager) -> Option<CopilotSession> {
    let openclaw_dir = dirs::home_dir()?.join(".openclaw");
    let token_file = openclaw_dir.join("credentials/github-copilot.token.json");

    if !token_file.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&token_file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;

    let token = json.get("token")?.as_str()?;
    let expires_at_ms = json.get("expiresAt")?.as_i64()?;
    let expires_at = expires_at_ms / 1000; // Convert ms to seconds

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let remaining = expires_at - now;
    if remaining <= 60 {
        debug!("OpenClaw token also expired");
        return None;
    }

    info!(
        remaining_hours = remaining / 3600,
        "Auto-imported fresh token from OpenClaw"
    );

    // Store in our vault for next time
    let session_data = serde_json::json!({
        "session_token": token,
        "expires_at": expires_at,
    });
    if let Err(e) = vault.store_secret("GITHUB_COPILOT_SESSION", &session_data.to_string()) {
        warn!(error = %e, "Failed to cache imported token in vault");
    }

    Some(CopilotSession::from_session_token(
        token.to_string(),
        expires_at,
    ))
}

/// Initialize a CopilotSession if the provider requires one.
///
/// Checks multiple sources in order:
/// 1. Imported session token from vault (GITHUB_COPILOT_SESSION)
/// 2. Fresh token from OpenClaw (~/.openclaw/credentials/github-copilot.token.json)
/// 3. OAuth token from model context
pub(crate) async fn init_copilot_session(
    provider: &str,
    api_key: Option<&str>,
    vault: &SharedVault,
) -> Option<Arc<CopilotSession>> {
    if !crate_providers::needs_copilot_session(provider) {
        return None;
    }

    let mut vault_guard = vault.lock().await;
    let session_result = vault_guard.get_secret("GITHUB_COPILOT_SESSION", true);

    let mut session_from_import = match &session_result {
        Ok(Some(json_str)) => {
            debug!("Found GITHUB_COPILOT_SESSION in vault");
            serde_json::from_str::<serde_json::Value>(json_str)
                .ok()
                .and_then(|json| {
                    let token = json.get("session_token")?.as_str()?.to_string();
                    let expires_at = json.get("expires_at")?.as_i64()?;

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;

                    let remaining = expires_at - now;
                    debug!(remaining_seconds = remaining, "Session expiry check");

                    if remaining > 60 {
                        Some(CopilotSession::from_session_token(token, expires_at))
                    } else {
                        debug!("Session expired or expiring soon");
                        None
                    }
                })
        }
        Ok(None) => {
            debug!("GITHUB_COPILOT_SESSION not found in vault");
            None
        }
        Err(e) => {
            warn!(error = %e, "Failed to read GITHUB_COPILOT_SESSION");
            None
        }
    };

    // If vault session is expired/missing, try to auto-import from OpenClaw
    if session_from_import.is_none() {
        if let Some(session) = try_import_openclaw_token(&mut vault_guard) {
            session_from_import = Some(session);
        }
    }
    drop(vault_guard);

    if let Some(session) = session_from_import {
        debug!("Using imported session token");
        Some(Arc::new(session))
    } else if let Some(oauth) = api_key {
        debug!("Falling back to OAuth token");
        Some(Arc::new(CopilotSession::new(oauth.to_string())))
    } else {
        warn!("No OAuth token available for Copilot provider");
        None
    }
}
