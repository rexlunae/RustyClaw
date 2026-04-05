// ── Pairing dialog — SSH key pairing overlay ────────────────────────────────
//
//! Multi-step wizard for SSH gateway pairing:
//! 1. ShowKey — Display public key and fingerprint
//! 2. EnterGateway — Input gateway host:port
//! 3. Connecting — Connection in progress
//! 4. Complete — Pairing successful

use crate::theme;
use iocraft::prelude::*;

/// Steps in the pairing flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairingStep {
    #[default]
    ShowKey,
    EnterGateway,
    Connecting,
    Complete,
}

/// Input fields in the gateway entry step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairingField {
    #[default]
    Host,
    Port,
}

#[derive(Default, Props)]
pub struct PairingDialogProps {
    /// Current step in the pairing flow.
    pub step: PairingStep,
    /// The client's public key in OpenSSH format.
    pub public_key: String,
    /// The key fingerprint (SHA256:...).
    pub fingerprint: String,
    /// ASCII art fingerprint visualization.
    pub fingerprint_art: String,
    /// QR code ASCII art (optional).
    pub qr_ascii: String,
    /// Gateway host input.
    pub gateway_host: String,
    /// Gateway port input.
    pub gateway_port: String,
    /// Which input field is active.
    pub active_field: PairingField,
    /// Error message to display.
    pub error: String,
    /// Success message.
    pub success: String,
}

#[component]
pub fn PairingDialog(props: &PairingDialogProps) -> impl Into<AnyElement<'static>> {
    let title = match props.step {
        PairingStep::ShowKey => "🔐 Pair with Gateway — Step 1/2",
        PairingStep::EnterGateway => "🔐 Pair with Gateway — Step 2/2",
        PairingStep::Connecting => "🔐 Connecting...",
        PairingStep::Complete => "✅ Pairing Complete",
    };

    element! {
        // Full-screen overlay
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            // Dialog box
            View(
                width: 72,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::ACCENT_BRIGHT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                // Title
                Text(
                    content: title,
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Step-specific content
                #(match props.step {
                    PairingStep::ShowKey => {
                        let el: AnyElement<'static> = render_show_key(props).into();
                        el
                    },
                    PairingStep::EnterGateway => {
                        let el: AnyElement<'static> = render_enter_gateway(props).into();
                        el
                    },
                    PairingStep::Connecting => {
                        let el: AnyElement<'static> = render_connecting(props).into();
                        el
                    },
                    PairingStep::Complete => {
                        let el: AnyElement<'static> = render_complete(props).into();
                        el
                    },
                })
            }
        }
    }
}

fn render_show_key(props: &PairingDialogProps) -> impl Into<AnyElement<'static>> {
    let has_qr = !props.qr_ascii.is_empty();
    let visual = if has_qr {
        props.qr_ascii.clone()
    } else {
        props.fingerprint_art.clone()
    };
    let visual_title = if has_qr { "QR Code" } else { "Key Art" };

    element! {
        View(flex_direction: FlexDirection::Column) {
            // Instructions
            Text(
                content: "Copy your public key and add it to the gateway's",
                color: theme::TEXT,
            )
            Text(
                content: "~/.rustyclaw/authorized_clients",
                color: theme::ACCENT_BRIGHT,
                weight: Weight::Bold,
            )

            View(height: 1)

            // Public key box
            View(
                border_style: BorderStyle::Single,
                border_color: theme::MUTED,
                padding_left: 1,
                padding_right: 1,
            ) {
                Text(
                    content: format!(" Public Key "),
                    color: theme::MUTED,
                )
            }
            View(
                padding_left: 1,
                padding_right: 1,
            ) {
                // Truncate key for display if too long
                Text(
                    content: truncate_key(&props.public_key, 66),
                    color: theme::TEXT,
                )
            }

            View(height: 1)

            // Fingerprint
            View(flex_direction: FlexDirection::Row) {
                Text(content: "Fingerprint: ", color: theme::MUTED)
                Text(content: props.fingerprint.clone(), color: theme::ACCENT)
            }

            View(height: 1)

            // Visual (fingerprint art or QR)
            View(
                border_style: BorderStyle::Single,
                border_color: theme::MUTED,
                padding_left: 1,
                padding_right: 1,
            ) {
                Text(
                    content: format!(" {} ", visual_title),
                    color: theme::MUTED,
                )
            }
            View(
                padding_left: 1,
                align_items: AlignItems::Center,
            ) {
                Text(content: visual, color: theme::TEXT)
            }

            View(height: 1)

            // Help text
            View(flex_direction: FlexDirection::Row, justify_content: JustifyContent::Center) {
                Text(content: "[Enter]", color: theme::ACCENT)
                Text(content: " Next  ", color: theme::MUTED)
                Text(content: "[Esc]", color: theme::ACCENT)
                Text(content: " Cancel", color: theme::MUTED)
            }
        }
    }
}

fn render_enter_gateway(props: &PairingDialogProps) -> impl Into<AnyElement<'static>> {
    let host_focused = props.active_field == PairingField::Host;
    let port_focused = props.active_field == PairingField::Port;
    let has_error = !props.error.is_empty();

    element! {
        View(flex_direction: FlexDirection::Column) {
            // Instructions
            Text(
                content: "Enter the gateway's SSH address:",
                color: theme::TEXT,
            )

            View(height: 1)

            // Host input
            View(
                border_style: BorderStyle::Single,
                border_color: if host_focused { theme::ACCENT_BRIGHT } else { theme::MUTED },
                padding_left: 1,
                padding_right: 1,
            ) {
                Text(
                    content: " Host ",
                    color: if host_focused { theme::ACCENT_BRIGHT } else { theme::MUTED },
                )
            }
            View(padding_left: 2) {
                Text(
                    content: if props.gateway_host.is_empty() {
                        "example.com".to_string()
                    } else {
                        props.gateway_host.clone()
                    },
                    color: if props.gateway_host.is_empty() { theme::MUTED } else { theme::TEXT },
                )
            }

            View(height: 1)

            // Port input
            View(
                border_style: BorderStyle::Single,
                border_color: if port_focused { theme::ACCENT_BRIGHT } else { theme::MUTED },
                padding_left: 1,
                padding_right: 1,
            ) {
                Text(
                    content: " Port ",
                    color: if port_focused { theme::ACCENT_BRIGHT } else { theme::MUTED },
                )
            }
            View(padding_left: 2) {
                Text(
                    content: props.gateway_port.clone(),
                    color: theme::TEXT,
                )
            }

            View(height: 1)

            // Error message
            #(if has_error {
                element! {
                    Text(content: props.error.clone(), color: theme::ERROR)
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            View(height: 1)

            // Help text
            View(flex_direction: FlexDirection::Row, justify_content: JustifyContent::Center) {
                Text(content: "[Tab]", color: theme::ACCENT)
                Text(content: " Switch  ", color: theme::MUTED)
                Text(content: "[Enter]", color: theme::ACCENT)
                Text(content: " Connect  ", color: theme::MUTED)
                Text(content: "[Esc]", color: theme::ACCENT)
                Text(content: " Back", color: theme::MUTED)
            }
        }
    }
}

fn render_connecting(props: &PairingDialogProps) -> impl Into<AnyElement<'static>> {
    let address = format!("{}:{}", props.gateway_host, props.gateway_port);

    element! {
        View(
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding_top: 2,
            padding_bottom: 2,
        ) {
            Text(
                content: "Connecting to gateway...",
                color: theme::TEXT,
                weight: Weight::Bold,
            )

            View(height: 1)

            Text(content: address, color: theme::ACCENT)
        }
    }
}

fn render_complete(props: &PairingDialogProps) -> impl Into<AnyElement<'static>> {
    let message = if props.success.is_empty() {
        "Pairing successful!".to_string()
    } else {
        props.success.clone()
    };

    element! {
        View(
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding_top: 2,
            padding_bottom: 2,
        ) {
            Text(
                content: "✓",
                color: theme::SUCCESS,
                weight: Weight::Bold,
            )

            View(height: 1)

            Text(
                content: message,
                color: theme::SUCCESS,
                weight: Weight::Bold,
            )

            View(height: 2)

            View(flex_direction: FlexDirection::Row) {
                Text(content: "[Enter]", color: theme::ACCENT)
                Text(content: " Close", color: theme::MUTED)
            }
        }
    }
}

/// Truncate a key for display, showing start and end.
fn truncate_key(key: &str, max_len: usize) -> String {
    if key.len() <= max_len {
        key.to_string()
    } else {
        let half = (max_len - 3) / 2;
        format!("{}...{}", &key[..half], &key[key.len() - half..])
    }
}
