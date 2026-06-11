//! Vault unlock dialog: enter password to unlock the encrypted vault.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, Help};
use rustyclaw_view::VaultUnlockData;

use super::RcModal;

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

    let submit = move || {
        let pw = password.read().clone();
        if !pw.is_empty() {
            props.on_submit.call(pw);
            password.set(String::new());
        }
    };
    let mut submit_btn = submit;
    let mut submit_key = submit;

    rsx! {
        RcModal {
            active: true,
            title: "🔐 Vault Locked",
            width: 420,
            onclose: move |_| props.on_cancel.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| props.on_cancel.call(()),
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| submit_btn(),
                        "Unlock"
                    }
                }
            },

            p { class: "rc-dialog-lead", "Enter your vault password to unlock secrets." }

            Field {
                Control {
                    input {
                        class: "input",
                        r#type: "password",
                        placeholder: "Vault password",
                        value: "{password}",
                        autofocus: true,
                        oninput: move |evt| password.set(evt.value()),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter {
                                evt.prevent_default();
                                submit_key();
                            }
                        },
                    }
                }
                if !props.data.error.is_empty() {
                    Help { color: BulmaColor::Danger, "⚠ {props.data.error}" }
                }
            }
        }
    }
}
