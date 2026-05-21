//! Vault unlock dialog: enter password to unlock the encrypted vault.

use dioxus::prelude::*;
use rustyclaw_view::VaultUnlockData;

#[derive(Props, Clone, PartialEq)]
pub struct VaultUnlockDialogProps {
    pub visible: bool,
    pub data: VaultUnlockData,
    pub on_submit: EventHandler<String>,
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn VaultUnlockDialog(props: VaultUnlockDialogProps) -> Element {
    let mut password = use_signal(String::new);

    if !props.visible {
        return rsx! {};
    }

    rsx! {
        div { class: "modal-backdrop",
            div {
                class: "modal",
                style: "max-width: 420px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🔐 Vault Locked" }
                    button {
                        class: "modal-close",
                        title: "Cancel",
                        onclick: move |_| props.on_cancel.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    p {
                        style: "color: var(--text-dim); margin-bottom: 12px;",
                        "Enter your vault password to unlock secrets."
                    }

                    if !props.data.error.is_empty() {
                        div {
                            style: "color: var(--error); margin-bottom: 8px; font-size: 0.9em;",
                            "⚠ {props.data.error}"
                        }
                    }

                    form {
                        onsubmit: move |_| {
                            let pw = password.read().clone();
                            if !pw.is_empty() {
                                props.on_submit.call(pw);
                                password.set(String::new());
                            }
                        },
                        div { class: "field",
                            input {
                                class: "input",
                                r#type: "password",
                                placeholder: "Vault password",
                                value: "{password}",
                                oninput: move |evt| password.set(evt.value()),
                            }
                        }
                        div {
                            style: "margin-top: 12px; display: flex; justify-content: flex-end; gap: 8px;",
                            button {
                                class: "btn btn-subtle",
                                r#type: "button",
                                onclick: move |_| props.on_cancel.call(()),
                                "Cancel"
                            }
                            button {
                                class: "btn btn-primary",
                                r#type: "submit",
                                "Unlock"
                            }
                        }
                    }
                }
            }
        }
    }
}
