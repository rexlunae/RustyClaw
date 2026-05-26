//! Lightweight, shared client preferences.
//!
//! Both the desktop and TUI clients persist a small amount of state
//! (gateway connection preferences) so clients can pre-fill and/or
//! bypass the connection dialog on next launch. The file lives
//! at `~/.rustyclaw/client.json` and is intentionally simple JSON so
//! that any client can read/write it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default gateway URL used when no other URL is configured or saved.
pub const DEFAULT_GATEWAY_URL: &str = "ssh://127.0.0.1:2222";

fn default_true() -> bool {
    true
}

/// One configured gateway connection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientConnection {
    /// Optional display label.
    #[serde(default)]
    pub name: Option<String>,
    /// Gateway URL.
    pub url: String,
    /// Whether startup auto-connect should attempt this connection.
    #[serde(default = "default_true")]
    pub auto_connect: bool,
    /// Whether this connection is part of the startup default set.
    #[serde(default = "default_true")]
    pub default_on_startup: bool,
}

impl ClientConnection {
    fn normalized(self) -> Option<Self> {
        let url = self.url.trim().to_string();
        if url.is_empty() {
            return None;
        }
        Some(Self {
            name: self.name.and_then(|n| {
                let trimmed = n.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }),
            url,
            auto_connect: self.auto_connect,
            default_on_startup: self.default_on_startup,
        })
    }
}

/// Shared client preferences persisted in `~/.rustyclaw/client.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ClientPreferences {
    /// When true, clients should skip the connection dialog at startup.
    #[serde(default)]
    pub bypass_connection_dialog: bool,
    /// Configured gateway connections.
    #[serde(default)]
    pub connections: Vec<ClientConnection>,
}

fn prefs_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".rustyclaw").join("client.json"))
}

fn parse_preferences(value: &serde_json::Value) -> ClientPreferences {
    let bypass_connection_dialog = value
        .get("bypass_connection_dialog")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut connections = value
        .get("connections")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| serde_json::from_value::<ClientConnection>(entry.clone()).ok())
                .filter_map(ClientConnection::normalized)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Backward compatibility: migrate legacy singular gateway URL.
    if connections.is_empty()
        && let Some(legacy_url) = value.get("gateway_url").and_then(|v| v.as_str())
    {
        let legacy = ClientConnection {
            name: None,
            url: legacy_url.to_string(),
            auto_connect: true,
            default_on_startup: true,
        };
        if let Some(conn) = legacy.normalized() {
            connections.push(conn);
        }
    }

    ClientPreferences {
        bypass_connection_dialog,
        connections,
    }
}

fn read_raw_prefs() -> Option<serde_json::Value> {
    let path = prefs_path()?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Load all persisted client preferences.
pub fn load_client_preferences() -> ClientPreferences {
    read_raw_prefs()
        .as_ref()
        .map(parse_preferences)
        .unwrap_or_default()
}

/// Persist all client preferences.
pub fn save_client_preferences(prefs: &ClientPreferences) {
    let Some(path) = prefs_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut value = read_raw_prefs().unwrap_or_else(|| serde_json::json!({}));
    if !value.is_object() {
        value = serde_json::json!({});
    }

    let mut normalized_connections = prefs
        .connections
        .clone()
        .into_iter()
        .filter_map(ClientConnection::normalized)
        .collect::<Vec<_>>();

    // Keep URLs unique while preserving order.
    let mut seen = std::collections::HashSet::new();
    normalized_connections.retain(|conn| seen.insert(conn.url.clone()));

    let legacy_gateway_url = normalized_connections
        .iter()
        .find(|conn| conn.default_on_startup)
        .or_else(|| normalized_connections.first())
        .map(|conn| conn.url.clone());

    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "bypass_connection_dialog".to_string(),
            serde_json::Value::Bool(prefs.bypass_connection_dialog),
        );
        obj.insert(
            "connections".to_string(),
            serde_json::to_value(&normalized_connections).unwrap_or_else(|_| serde_json::json!([])),
        );
        match legacy_gateway_url {
            Some(url) => {
                obj.insert("gateway_url".to_string(), serde_json::Value::String(url));
            }
            None => {
                obj.remove("gateway_url");
            }
        }
    }

    if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
        let _ = std::fs::write(&path, bytes);
    }
}

/// Read the previously-saved startup gateway URL, if any.
pub fn load_saved_gateway_url() -> Option<String> {
    let prefs = load_client_preferences();
    prefs
        .connections
        .iter()
        .find(|conn| conn.default_on_startup)
        .or_else(|| prefs.connections.first())
        .map(|conn| conn.url.clone())
}

/// Persist the gateway URL chosen by the user.
pub fn save_gateway_url(url: &str) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return;
    }

    let mut prefs = load_client_preferences();
    let mut found = false;
    for conn in &mut prefs.connections {
        if conn.url == trimmed {
            conn.default_on_startup = true;
            found = true;
        } else {
            conn.default_on_startup = false;
        }
    }
    if !found {
        for conn in &mut prefs.connections {
            conn.default_on_startup = false;
        }
        prefs.connections.push(ClientConnection {
            name: None,
            url: trimmed.to_string(),
            auto_connect: true,
            default_on_startup: true,
        });
    }
    save_client_preferences(&prefs);
}

/// Whether client startup should bypass the interactive connection dialog.
pub fn should_bypass_connection_dialog() -> bool {
    load_client_preferences().bypass_connection_dialog
}

/// Startup default connections, in order.
pub fn load_default_startup_gateway_urls() -> Vec<String> {
    let prefs = load_client_preferences();
    let defaults = prefs
        .connections
        .iter()
        .filter(|conn| conn.default_on_startup)
        .map(|conn| conn.url.clone())
        .collect::<Vec<_>>();
    if !defaults.is_empty() {
        defaults
    } else {
        prefs
            .connections
            .iter()
            .map(|conn| conn.url.clone())
            .collect()
    }
}

/// Auto-connect candidates, in startup order.
pub fn load_auto_connect_gateway_urls() -> Vec<String> {
    let prefs = load_client_preferences();
    let startup_defaults = if prefs.connections.iter().any(|conn| conn.default_on_startup) {
        prefs
            .connections
            .iter()
            .filter(|conn| conn.default_on_startup)
            .collect::<Vec<_>>()
    } else {
        prefs.connections.iter().collect::<Vec<_>>()
    };

    let defaults_auto = startup_defaults
        .iter()
        .filter(|conn| conn.auto_connect)
        .map(|conn| conn.url.clone())
        .collect::<Vec<_>>();
    if !defaults_auto.is_empty() {
        return defaults_auto;
    }

    prefs
        .connections
        .iter()
        .filter(|conn| conn.auto_connect)
        .map(|conn| conn.url.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_gateway_url_into_connection() {
        let value = serde_json::json!({
            "gateway_url": "ssh://legacy:2222"
        });
        let prefs = parse_preferences(&value);
        assert_eq!(prefs.connections.len(), 1);
        assert_eq!(prefs.connections[0].url, "ssh://legacy:2222");
        assert!(prefs.connections[0].auto_connect);
        assert!(prefs.connections[0].default_on_startup);
    }

    #[test]
    fn parse_connections_and_bypass_setting() {
        let value = serde_json::json!({
            "bypass_connection_dialog": true,
            "connections": [
                {"name": "primary", "url": " ssh://one:2222 ", "auto_connect": true, "default_on_startup": true},
                {"url": "ssh://two:2222", "auto_connect": false, "default_on_startup": true},
                {"url": "   "}
            ]
        });
        let prefs = parse_preferences(&value);
        assert!(prefs.bypass_connection_dialog);
        assert_eq!(prefs.connections.len(), 2);
        assert_eq!(prefs.connections[0].url, "ssh://one:2222");
        assert_eq!(prefs.connections[0].name.as_deref(), Some("primary"));
        assert!(!prefs.connections[1].auto_connect);
    }

    #[test]
    fn save_gateway_promotes_selected_url_to_default() {
        let mut prefs = ClientPreferences {
            bypass_connection_dialog: false,
            connections: vec![
                ClientConnection {
                    name: None,
                    url: "ssh://a:2222".to_string(),
                    auto_connect: true,
                    default_on_startup: true,
                },
                ClientConnection {
                    name: None,
                    url: "ssh://b:2222".to_string(),
                    auto_connect: false,
                    default_on_startup: false,
                },
            ],
        };

        // Simulate the core behavior without touching the filesystem.
        let selected = "ssh://b:2222";
        let mut found = false;
        for conn in &mut prefs.connections {
            if conn.url == selected {
                conn.default_on_startup = true;
                found = true;
            } else {
                conn.default_on_startup = false;
            }
        }
        if !found {
            for conn in &mut prefs.connections {
                conn.default_on_startup = false;
            }
            prefs.connections.push(ClientConnection {
                name: None,
                url: selected.to_string(),
                auto_connect: true,
                default_on_startup: true,
            });
        }

        assert_eq!(
            prefs
                .connections
                .iter()
                .filter(|conn| conn.default_on_startup)
                .count(),
            1
        );
        assert_eq!(
            prefs
                .connections
                .iter()
                .find(|conn| conn.default_on_startup)
                .map(|conn| conn.url.as_str()),
            Some("ssh://b:2222")
        );
    }
}
