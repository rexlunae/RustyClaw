//! Connection dialog shown at startup so the user can choose which
//! gateway to connect to. While a connection attempt is in flight the
//! dialog shows a spinner; on failure the error is displayed inline
//! and the user can edit the URL and retry.

use dioxus::prelude::*;
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

    let submit = move |_| {
        let v = url.read().trim().to_string();
        if !v.is_empty() {
            props.on_connect.call(v);
        }
    };

    rsx! {
        div { class: "modal-backdrop",
            // Don't allow click-outside-to-dismiss while a connect
            // attempt is in flight — keeps the user from accidentally
            // losing the spinner.
            onclick: move |_| {
                if !is_connecting {
                    props.on_cancel.call(());
                }
            },

            div {
                class: "modal",
                style: "max-width: 460px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "Connect to gateway" }
                }

                div { class: "modal-body",
                    div { class: "settings-section",
                        div { class: "field",
                            span { class: "field-label", "Gateway URL" }
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
                            span { class: "field-help",
                                "RustyClaw connects to your gateway over SSH (default port 2222)."
                            }
                        }

                        if is_connecting {
                            div {
                                class: "connection-status connection-status-connecting",
                                style: "display: flex; align-items: center; gap: 0.5rem; margin-top: 0.75rem;",
                                span { class: "icon spin", "↻" }
                                span { "Connecting to {trimmed_url}…" }
                            }
                        }

                        if let Some(err) = error_text {
                            div {
                                class: "connection-status connection-status-error",
                                style: "margin-top: 0.75rem; color: var(--rc-danger, #ff6b6b);",
                                "🚫 {err}"
                            }
                        }
                    }
                }

                div { class: "modal-foot",
                    button {
                        class: "btn btn-subtle",
                        disabled: is_connecting,
                        onclick: move |_| props.on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-primary",
                        disabled: !can_connect,
                        onclick: submit,
                        if is_connecting { "Connecting…" } else { "Connect" }
                    }
                }
            }
        }
    }
}
