//! Connection dialog shown at startup so the user can choose which
//! gateway to connect to. Shows the connection history for quick
//! reconnects, lets the user pin a default and enable auto-connect at
//! startup. While a connection attempt is in flight the dialog shows a
//! spinner; on failure the error is displayed inline and the user can
//! edit the URL and retry.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Checkbox, Control, Delete, Field, FieldLabel, Help,
    Notification,
};
use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_view::ConnectionDialogData;

use super::{RcModal, copy_to_clipboard};

#[derive(Props, Clone, PartialEq)]
pub struct ConnectionDialogProps {
    /// Whether the dialog is rendered.
    pub visible: bool,
    /// Currently-configured gateway URL (pre-fills the input).
    pub gateway_url: String,
    /// Current connection status. Used to drive the spinner and
    /// inline error message.
    pub status: ConnectionStatus,
    /// Recent connections, default marker, and the autoconnect setting.
    pub data: ConnectionDialogData,
    /// User clicked "Connect" or a recent connection. The string is the
    /// (trimmed) URL.
    pub on_connect: EventHandler<String>,
    /// Toggle a URL as the startup default: `(url, is_default)`.
    pub on_set_default: EventHandler<(String, bool)>,
    /// Remove a URL from the history.
    pub on_remove: EventHandler<String>,
    /// Toggle auto-connect to the default at startup.
    pub on_toggle_autoconnect: EventHandler<bool>,
    /// User dismissed the dialog without connecting.
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn ConnectionDialog(props: ConnectionDialogProps) -> Element {
    let mut url = use_signal(|| props.gateway_url.clone());

    if !props.visible {
        return rsx! {};
    }

    let is_connecting = matches!(
        props.status,
        ConnectionStatus::Connecting | ConnectionStatus::Authenticating
    );
    let error_text = match &props.status {
        ConnectionStatus::Error(msg) => Some(msg.clone()),
        _ => None,
    };

    let trimmed_url = url.read().trim().to_string();
    let can_connect = !is_connecting && !trimmed_url.is_empty();
    let has_default = props.data.has_default();
    let autoconnect = props.data.autoconnect_on_startup;

    let submit = move |_| {
        let v = url.read().trim().to_string();
        if !v.is_empty() {
            props.on_connect.call(v);
        }
    };

    rsx! {
        RcModal {
            active: true,
            title: "Connect to gateway",
            width: 480,
            // Don't allow dismissal while a connect attempt is in flight —
            // keeps the user from accidentally losing the spinner.
            closable: !is_connecting,
            onclose: move |_| props.on_cancel.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        disabled: is_connecting,
                        onclick: move |_| props.on_cancel.call(()),
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        loading: is_connecting,
                        disabled: !can_connect,
                        onclick: submit,
                        if is_connecting { "Connecting…" } else { "Connect" }
                    }
                }
            },

            Field {
                FieldLabel { "Gateway URL" }
                Control {
                    input {
                        class: "input",
                        r#type: "text",
                        value: "{url}",
                        placeholder: "ssh://127.0.0.1:2222",
                        disabled: is_connecting,
                        oninput: move |evt| url.set(evt.value()),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter && can_connect {
                                let v = url.read().trim().to_string();
                                if !v.is_empty() {
                                    props.on_connect.call(v);
                                }
                            }
                        },
                    }
                }
                Help { "RustyClaw connects to your gateway over SSH (default port 2222)." }
            }

            // ── Recent connections ──────────────────────────────────────
            if !props.data.recent.is_empty() {
                div { class: "recent-connections",
                    FieldLabel { "Recent connections" }
                    for option in props.data.recent.iter().cloned() {
                        div {
                            key: "{option.url}",
                            class: if option.is_default { "recent-connection-row is-default" } else { "recent-connection-row" },
                            // Click to quick-reconnect.
                            onclick: {
                                let connect_url = option.url.clone();
                                let on_connect = props.on_connect;
                                move |_| {
                                    if !is_connecting {
                                        url.set(connect_url.clone());
                                        on_connect.call(connect_url.clone());
                                    }
                                }
                            },
                            span { class: "recent-connection-label", title: "{option.url}",
                                "{option.display_label()}"
                            }
                            if option.is_default {
                                span { class: "recent-connection-default-tag", "default" }
                            }
                            span { title: if option.is_default { "Unset default" } else { "Set as default" },
                                Button {
                                    color: BulmaColor::Ghost,
                                    size: BulmaSize::Small,
                                    class: "recent-connection-star",
                                    onclick: {
                                        let u = option.url.clone();
                                        let is_default = option.is_default;
                                        let on_set_default = props.on_set_default;
                                        move |evt: MouseEvent| {
                                            evt.stop_propagation();
                                            on_set_default.call((u.clone(), !is_default));
                                        }
                                    },
                                    if option.is_default { "★" } else { "☆" }
                                }
                            }
                            Delete {
                                size: BulmaSize::Small,
                                class: "recent-connection-remove",
                                onclick: {
                                    let u = option.url.clone();
                                    let on_remove = props.on_remove;
                                    move |evt: MouseEvent| {
                                        evt.stop_propagation();
                                        on_remove.call(u.clone());
                                    }
                                },
                            }
                        }
                    }

                    Field { class: "autoconnect-field",
                        Checkbox {
                            checked: autoconnect,
                            disabled: !has_default,
                            onchange: move |_| props.on_toggle_autoconnect.call(!autoconnect),
                            " Connect to the default automatically at startup"
                        }
                        if !has_default {
                            Help { "Pick a default connection (☆) to enable auto-connect." }
                        }
                    }
                }
            }

            if is_connecting {
                div { class: "connection-status connection-status-connecting",
                    span { class: "icon spin", "↻" }
                    span { "Connecting to {trimmed_url}…" }
                }
            }

            if let Some(err) = error_text {
                Notification {
                    color: BulmaColor::Danger,
                    light: true,
                    class: "connection-status-error",
                    span { class: "connection-error-text", "🚫 {err}" }
                    Button {
                        color: BulmaColor::Ghost,
                        size: BulmaSize::Small,
                        class: "connection-error-copy",
                        onclick: {
                            let text = err.clone();
                            move |_| copy_to_clipboard(text.clone())
                        },
                        "⎘ Copy"
                    }
                }
            }
        }
    }
}
