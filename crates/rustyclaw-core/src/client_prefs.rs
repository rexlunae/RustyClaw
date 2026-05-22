//! Lightweight, shared client preferences.
//!
//! Both the desktop and TUI clients persist a small amount of state
//! (currently just the most recently-used gateway URL) so that the
//! connection dialog can pre-fill it on next launch. The file lives
//! at `~/.rustyclaw/client.json` and is intentionally simple JSON so
//! that any client can read/write it.

use std::path::PathBuf;

/// Default gateway URL used when no other URL is configured or saved.
pub const DEFAULT_GATEWAY_URL: &str = "ssh://127.0.0.1:2222";

fn prefs_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".rustyclaw").join("client.json"))
}

/// Read the previously-saved gateway URL, if any.
pub fn load_saved_gateway_url() -> Option<String> {
    let path = prefs_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("gateway_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Persist the gateway URL chosen by the user.
pub fn save_gateway_url(url: &str) {
    let Some(path) = prefs_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Preserve any other fields that may live in the file.
    let mut value: serde_json::Value = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "gateway_url".to_string(),
            serde_json::Value::String(url.to_string()),
        );
    } else {
        value = serde_json::json!({ "gateway_url": url });
    }

    if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
        let _ = std::fs::write(&path, bytes);
    }
}
