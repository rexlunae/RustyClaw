//! Connection-dialog data: recent connections, default, and autoconnect.

use rustyclaw_core::client_prefs::ClientPreferences;

/// One row in the connection dialog's recent-connections list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionOption {
    /// Gateway URL.
    pub url: String,
    /// Optional display label.
    pub name: Option<String>,
    /// Whether this is the startup default.
    pub is_default: bool,
}

impl ConnectionOption {
    /// Text shown for this row: the label when set, otherwise the URL.
    pub fn display_label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.url)
    }
}

/// Everything the connection dialog needs beyond the URL input itself.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ConnectionDialogData {
    /// Recent connections, most recent first.
    pub recent: Vec<ConnectionOption>,
    /// Whether startup auto-connects to the default (skipping this dialog).
    pub autoconnect_on_startup: bool,
}

impl ConnectionDialogData {
    /// Build the dialog data from the persisted client preferences.
    pub fn from_preferences(prefs: &ClientPreferences) -> Self {
        Self {
            recent: prefs
                .connections
                .iter()
                .map(|conn| ConnectionOption {
                    url: conn.url.clone(),
                    name: conn.name.clone(),
                    is_default: conn.default_on_startup,
                })
                .collect(),
            autoconnect_on_startup: prefs.bypass_connection_dialog,
        }
    }

    /// Reload from disk.
    pub fn load() -> Self {
        Self::from_preferences(&rustyclaw_core::client_prefs::load_client_preferences())
    }

    /// Whether a default connection exists (autoconnect requires one).
    pub fn has_default(&self) -> bool {
        self.recent.iter().any(|c| c.is_default)
    }
}
