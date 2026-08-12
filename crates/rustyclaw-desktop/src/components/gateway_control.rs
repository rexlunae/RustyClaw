//! Gateway control panel — local daemon lifecycle plus remote reload.
//!
//! The panel is split in two on purpose. The top half is about the gateway
//! daemon *on this machine*: the process named by the PID file under the
//! settings directory, which start/stop/restart act on whether or not this
//! client is connected to anything. The bottom half is about the gateway this
//! client is *talking to*, which may be somewhere else entirely; the only
//! lifecycle command the protocol carries is `reload`, and that is the only
//! button there.
//!
//! Keeping them visually separate is the point. A single "Stop" button under
//! a heading naming a remote host would read as stopping that host.

use dioxus::prelude::*;
use dioxus_bulma::components::{Title, TitleSize};
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Notification};
use rustyclaw_view::{GatewayControlData, PendingAction};

use super::RcModal;

/// What the user asked the panel to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayControlCommand {
    /// Start the local daemon.
    Start,
    /// Stop the local daemon (also clears a stale PID file).
    Stop,
    /// Stop then start the local daemon.
    Restart,
    /// Ask the connected gateway to re-read its configuration.
    Reload,
}

#[derive(Props, Clone, PartialEq)]
pub struct GatewayControlDialogProps {
    /// Whether the dialog is rendered.
    pub visible: bool,
    /// Status of the local daemon and the connected gateway.
    pub data: GatewayControlData,
    /// User pressed one of the action buttons.
    pub on_command: EventHandler<GatewayControlCommand>,
    /// User dismissed the dialog.
    pub on_close: EventHandler<()>,
}

#[component]
pub fn GatewayControlDialog(props: GatewayControlDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let d = props.data.clone();
    let status_label = d.local.label();
    let status_class = d.local.status_class();
    let scope_note = d.scope_note();

    rsx! {
        RcModal {
            active: true,
            title: "Gateway",
            width: 560,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Close"
                    }
                }
            },

            // ── Local daemon ────────────────────────────────────────────
            div { class: "gateway-control-section",
                Title { size: TitleSize::Is6, class: "gateway-control-title", "Gateway on this machine" }

                table { class: "table is-narrow is-fullwidth",
                    tbody {
                        tr {
                            td { strong { "Status" } }
                            td {
                                span { class: "tag {status_class}", "{status_label}" }
                            }
                        }
                        tr {
                            td { strong { "SSH listen" } }
                            td {
                                if d.ssh_listen.is_empty() {
                                    span { class: "has-text-grey", "—" }
                                } else {
                                    "{d.ssh_listen}"
                                }
                            }
                        }
                        tr {
                            td { strong { "Log" } }
                            td {
                                if d.log_path.is_empty() {
                                    span { class: "has-text-grey", "—" }
                                } else {
                                    code { class: "gateway-control-log", "{d.log_path}" }
                                }
                            }
                        }
                    }
                }

                Buttons {
                    Button {
                        color: BulmaColor::Success,
                        disabled: !d.can_start(),
                        loading: d.is_pending(PendingAction::Start),
                        onclick: move |_| props.on_command.call(GatewayControlCommand::Start),
                        "Start"
                    }
                    Button {
                        color: BulmaColor::Danger,
                        disabled: !d.can_stop(),
                        loading: d.is_pending(PendingAction::Stop),
                        onclick: move |_| props.on_command.call(GatewayControlCommand::Stop),
                        "Stop"
                    }
                    Button {
                        color: BulmaColor::Warning,
                        disabled: !d.can_restart(),
                        loading: d.is_pending(PendingAction::Restart),
                        onclick: move |_| props.on_command.call(GatewayControlCommand::Restart),
                        "Restart"
                    }
                }

                // The panel never asks for the vault password, so a daemon it
                // starts comes up locked. Saying so here beats letting the
                // user discover it when the agent cannot reach a credential.
                if d.vault_password_protected {
                    p { class: "has-text-grey is-size-7 mt-2",
                        "The secrets vault is password-protected. A gateway started from here "
                        "comes up with the vault locked — you'll be asked to unlock it after connecting."
                    }
                }

                if let Some(ref outcome) = d.last_action {
                    Notification {
                        color: if outcome.ok { BulmaColor::Success } else { BulmaColor::Danger },
                        light: true,
                        class: "gateway-control-outcome",
                        "{outcome.message}"
                    }
                }
            }

            // ── Connected gateway ───────────────────────────────────────
            div { class: "gateway-control-section mt-4",
                Title { size: TitleSize::Is6, class: "gateway-control-title", "Connected gateway" }

                table { class: "table is-narrow is-fullwidth",
                    tbody {
                        tr {
                            td { strong { "URL" } }
                            td {
                                if d.url.is_empty() {
                                    span { class: "has-text-grey", "not configured" }
                                } else {
                                    code { "{d.url}" }
                                }
                            }
                        }
                        tr {
                            td { strong { "Session" } }
                            td {
                                if d.connected {
                                    span { class: "tag is-success", "connected" }
                                } else {
                                    span { class: "tag is-light", "disconnected" }
                                }
                            }
                        }
                    }
                }

                Buttons {
                    Button {
                        color: BulmaColor::Info,
                        disabled: !d.can_reload(),
                        onclick: move |_| props.on_command.call(GatewayControlCommand::Reload),
                        "Reload configuration"
                    }
                }
                if !d.connected {
                    p { class: "has-text-grey is-size-7",
                        "Reload runs over the gateway session, so it needs a connection."
                    }
                }

                p { class: "has-text-grey is-size-7 mt-2", "{scope_note}" }
            }
        }
    }
}
