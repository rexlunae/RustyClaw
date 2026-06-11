//! Component data for the status bar.
//!
//! The status bar shows connection state, current model/provider,
//! and streaming progress at the bottom of the client window.

use rustyclaw_core::ui::ConnectionStatus;

use crate::tone::Tone;

/// Everything the status bar needs to render.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StatusBarData {
    /// Current connection status to the gateway.
    pub connection: ConnectionStatus,

    /// Active model identifier.
    pub model: Option<String>,

    /// Active provider identifier.
    pub provider: Option<String>,

    /// Human-readable streaming summary (if streaming).
    pub streaming_summary: Option<String>,

    /// Whether a response is being processed.
    pub is_processing: bool,
}

impl StatusBarData {
    /// Human-readable label for the current connection state.
    ///
    /// Maps each [`ConnectionStatus`] variant to a short string
    /// suitable for display in a status bar or sidebar chip.
    pub fn connection_label(&self) -> &'static str {
        Self::connection_label_static(&self.connection)
    }

    /// CSS-like class for the connection chip colouring.
    ///
    /// Returns `"is-success"`, `"is-warn"`, `"is-danger"`, or `"is-info"`.
    pub fn connection_class(&self) -> &'static str {
        Self::connection_class_static(&self.connection)
    }

    /// The error message from an error connection state, if any.
    pub fn connection_error(&self) -> Option<&str> {
        match &self.connection {
            ConnectionStatus::Error(e) => Some(e.as_str()),
            _ => None,
        }
    }

    /// Static version of [`connection_label`] that takes a reference.
    pub fn connection_label_static(status: &ConnectionStatus) -> &'static str {
        match status {
            ConnectionStatus::Disconnected => "Disconnected",
            ConnectionStatus::Connecting => "Connecting…",
            ConnectionStatus::Connected => "Connected",
            ConnectionStatus::Authenticating => "Authenticating…",
            ConnectionStatus::Authenticated => "Ready",
            ConnectionStatus::Error(_) => "Error",
        }
    }

    /// Static version of [`connection_class`] that takes a reference.
    pub fn connection_class_static(status: &ConnectionStatus) -> &'static str {
        match status {
            ConnectionStatus::Disconnected => "is-warn",
            ConnectionStatus::Connecting => "is-info",
            ConnectionStatus::Connected => "is-success",
            ConnectionStatus::Authenticating => "is-info",
            ConnectionStatus::Authenticated => "is-success",
            ConnectionStatus::Error(_) => "is-danger",
        }
    }

    /// Semantic tone for the connection chip.
    pub fn connection_tone(&self) -> Tone {
        Self::connection_tone_static(&self.connection)
    }

    /// Static version of [`connection_tone`](Self::connection_tone).
    pub fn connection_tone_static(status: &ConnectionStatus) -> Tone {
        match status {
            ConnectionStatus::Disconnected => Tone::Warning,
            ConnectionStatus::Connecting | ConnectionStatus::Authenticating => Tone::Info,
            ConnectionStatus::Connected | ConnectionStatus::Authenticated => Tone::Success,
            ConnectionStatus::Error(_) => Tone::Danger,
        }
    }

    /// Whether the connection chip should pulse (a transition is in flight).
    pub fn connection_is_pending_static(status: &ConnectionStatus) -> bool {
        matches!(
            status,
            ConnectionStatus::Connecting | ConnectionStatus::Authenticating
        )
    }

    /// Static version of [`connection_error`] that takes a reference.
    pub fn connection_error_static(status: &ConnectionStatus) -> Option<&str> {
        match status {
            ConnectionStatus::Error(e) => Some(e.as_str()),
            _ => None,
        }
    }

    /// Whether the client is in an active connected state.
    ///
    /// Returns `true` for Connected, Authenticated, or Authenticating.
    pub fn is_connected(&self) -> bool {
        matches!(
            &self.connection,
            ConnectionStatus::Connected
                | ConnectionStatus::Authenticated
                | ConnectionStatus::Authenticating
        )
    }

    /// Formatted model/provider string, e.g. `"openrouter · gpt-4o"`.
    pub fn model_display(&self) -> String {
        match (&self.provider, &self.model) {
            (Some(p), Some(m)) => format!("{p} · {m}"),
            (Some(p), None) => p.clone(),
            (None, Some(m)) => m.clone(),
            (None, None) => "(no model)".into(),
        }
    }
}
