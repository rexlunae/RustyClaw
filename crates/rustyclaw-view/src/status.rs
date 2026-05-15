//! Component data for the status bar.
//!
//! The status bar shows connection state, current model/provider,
//! and streaming progress at the bottom of the client window.

use rustyclaw_core::ui::ConnectionStatus;

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
