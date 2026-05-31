//! Connection dialog shown at startup so the user can choose which
//! gateway to connect to. While a connection attempt is in flight the
//! dialog shows a spinner; on failure the error is displayed inline
//! and the user can edit the URL and retry.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, Button, Control, Field, FieldLabel, Help, Modal, ModalCard, ModalCardBody,
    ModalCardFoot, ModalCardHead, Notification,
};
use rustyclaw_core::ui::ConnectionStatus;

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

    let on_connect = props.on_connect;
    let on_cancel = props.on_cancel;
    let cancel = move |_| {
        if !is_connecting {
            on_cancel.call(());
        }
    };
    let submit = move |_| {
        let v = url.read().trim().to_string();
        if !v.is_empty() {
            on_connect.call(v);
        }
    };

    rsx! {
        Modal { active: true, onclose: cancel,
            ModalCard { class: "rc-modal-narrow",
                ModalCardHead { onclose: cancel,
                    p { class: "modal-card-title", "Connect to gateway" }
                }
                ModalCardBody {
                    Field {
                        FieldLabel { "Gateway URL" }
                        Control {
                            // Raw input: dioxus-bulma's `Input` has no onkeydown,
                            // and we need Enter-to-connect. Bulma's `.input` class
                            // (themed) still applies.
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
                                            on_connect.call(v);
                                        }
                                    }
                                },
                            }
                        }
                        Help {
                            "RustyClaw connects to your gateway over SSH (default port 2222)."
                        }
                    }

                    if is_connecting {
                        Notification { color: BulmaColor::Info, class: "is-light",
                            "Connecting to {trimmed_url}…"
                        }
                    }

                    if let Some(err) = error_text {
                        Notification { color: BulmaColor::Danger, "🚫 {err}" }
                    }
                }
                ModalCardFoot {
                    Button { disabled: is_connecting, onclick: cancel, "Cancel" }
                    Button { color: BulmaColor::Primary, disabled: !can_connect, onclick: submit,
                        if is_connecting { "Connecting…" } else { "Connect" }
                    }
                }
            }
        }
    }
}
