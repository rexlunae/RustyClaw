//! Transient banner rows shown above the chat surface.
//!
//! Which banners appear for a given connection state / status message —
//! and which actions each banner offers — is client-independent, so the
//! decision lives here.  Clients render each [`BannerData`] with their
//! own notification widget and dispatch [`BannerActionKind`]s back to
//! their own handlers.

use rustyclaw_core::ui::ConnectionStatus;

use crate::tone::Tone;

/// What a banner action button should do when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BannerActionKind {
    /// Retry the gateway connection.
    Reconnect,
    /// Open the pairing dialog.
    PairGateway,
    /// Clear the transient status message.
    DismissStatus,
}

/// A single action button on a banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BannerAction {
    pub label: &'static str,
    pub kind: BannerActionKind,
}

/// One banner row: tone, text, and the actions it offers.
#[derive(Clone, Debug, PartialEq)]
pub struct BannerData {
    pub tone: Tone,
    pub icon: &'static str,
    pub text: String,
    pub actions: Vec<BannerAction>,
}

/// Build the list of banners to show for the current connection state
/// and transient status message, in display order.
pub fn build_banners(
    connection: &ConnectionStatus,
    status_message: Option<&str>,
) -> Vec<BannerData> {
    let mut banners = Vec::new();

    match connection {
        ConnectionStatus::Error(err) => banners.push(BannerData {
            tone: Tone::Danger,
            icon: "🚫",
            text: format!("Connection error: {err}"),
            actions: vec![
                BannerAction {
                    label: "↻ Retry",
                    kind: BannerActionKind::Reconnect,
                },
                BannerAction {
                    label: "Pair gateway",
                    kind: BannerActionKind::PairGateway,
                },
            ],
        }),
        ConnectionStatus::Connecting => banners.push(BannerData {
            tone: Tone::Info,
            icon: "🔄",
            text: "Connecting to gateway…".to_string(),
            actions: Vec::new(),
        }),
        _ => {}
    }

    if let Some(msg) = status_message.filter(|m| !m.is_empty()) {
        banners.push(BannerData {
            tone: Tone::Warning,
            icon: "",
            text: msg.to_string(),
            actions: vec![BannerAction {
                label: "Dismiss",
                kind: BannerActionKind::DismissStatus,
            }],
        });
    }

    banners
}
