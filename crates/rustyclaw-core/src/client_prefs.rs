//! Lightweight, shared client preferences.
//!
//! Both the desktop and TUI clients persist a small amount of state
//! (currently just the most recently-used gateway URL) so that the
//! connection dialog can pre-fill it on next launch. The file lives
//! at `~/.rustyclaw/client.json` and is intentionally simple JSON so
//! that any client can read/write it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default gateway URL used when no other URL is configured or saved.
pub const DEFAULT_GATEWAY_URL: &str = "ssh://127.0.0.1:2222";

fn prefs_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".rustyclaw").join("client.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ClientPrefs {
    #[serde(default)]
    gateway_url: Option<String>,
    #[serde(default)]
    gateway_config_toml: Option<String>,
}

fn load_prefs() -> Option<ClientPrefs> {
    let path = prefs_path()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_prefs(mut prefs: ClientPrefs) {
    let Some(path) = prefs_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if prefs
        .gateway_config_toml
        .as_ref()
        .is_some_and(|cfg| cfg.trim().is_empty())
    {
        prefs.gateway_config_toml = None;
    }

    if let Ok(bytes) = serde_json::to_vec_pretty(&prefs) {
        let _ = std::fs::write(&path, bytes);
    }
}

/// Read the previously-saved gateway URL, if any.
pub fn load_saved_gateway_url() -> Option<String> {
    load_prefs().and_then(|prefs| prefs.gateway_url)
}

/// Persist the gateway URL chosen by the user.
pub fn save_gateway_url(url: &str) {
    let mut prefs = load_prefs().unwrap_or_default();
    prefs.gateway_url = Some(url.to_string());
    save_prefs(prefs);
}

/// Read the previously-saved gateway config TOML payload, if any.
pub fn load_saved_gateway_config_toml() -> Option<String> {
    load_prefs().and_then(|prefs| prefs.gateway_config_toml)
}

/// Persist a gateway config TOML payload chosen by the user.
pub fn save_gateway_config_toml(config_toml: &str) {
    let mut prefs = load_prefs().unwrap_or_default();
    prefs.gateway_config_toml = Some(config_toml.to_string());
    save_prefs(prefs);
}
