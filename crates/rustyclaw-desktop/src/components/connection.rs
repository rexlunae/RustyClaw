//! Connection dialog shown at startup so the user can choose which
//! gateway to connect to. While a connection attempt is in flight the
//! dialog shows a spinner; on failure the error is displayed inline
//! and the user can edit the URL and retry.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help, Notification,
};
use rustyclaw_core::ui::ConnectionStatus;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct ConnectionDialogProps {
    /// Whether the dialog is rendered.
    pub visible: bool,
    /// Currently-configured gateway URL (pre-fills the input).
    pub gateway_url: String,
    /// Current connection status. Used to drive the spinner and
    /// inline error message.
    pub status: ConnectionStatus,
    /// User clicked "Connect". The string is the (trimmed) URL.
    pub on_connect: EventHandler<String>,
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
            width: 460,
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
                    "🚫 {err}"
                }
            }
        }
    }
}
