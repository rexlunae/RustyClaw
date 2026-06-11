//! Credential request dialog: gateway needs an API key or token from the user.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, Tag};
use rustyclaw_view::CredentialRequestData;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct CredentialRequestDialogProps {
    pub visible: bool,
    pub id: String,
    pub data: CredentialRequestData,
    pub on_submit: EventHandler<(String, String)>,
    pub on_dismiss: EventHandler<String>,
}

#[component]
pub fn CredentialRequestDialog(props: CredentialRequestDialogProps) -> Element {
    let mut input_value = use_signal(String::new);

    if !props.visible {
        return rsx! {};
    }

    let id_submit = props.id.clone();
    let id_dismiss = props.id.clone();
    let id_close = props.id.clone();

    let submit = {
        let on_submit = props.on_submit;
        move || {
            let val = input_value.read().clone();
            if !val.is_empty() {
                on_submit.call((id_submit.clone(), val));
                input_value.set(String::new());
            }
        }
    };
    let mut submit_btn = submit.clone();
    let mut submit_key = submit;

    rsx! {
        RcModal {
            active: true,
            title: "🔑 Credential Required",
            width: 480,
            onclose: move |_| props.on_dismiss.call(id_close.clone()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: {
                            let id = id_dismiss.clone();
                            move |_| props.on_dismiss.call(id.clone())
                        },
                        "Skip"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| submit_btn(),
                        "Save & Continue"
                    }
                }
            },

            div { class: "rc-cred-provider",
                Tag {
                    color: BulmaColor::Primary,
                    light: true,
                    "{props.data.provider}"
                }
                span { class: "rc-cred-secret", "({props.data.secret_name})" }
            }

            p { class: "rc-dialog-lead", "{props.data.message}" }

            Field {
                Control {
                    input {
                        class: "input",
                        r#type: "password",
                        placeholder: "Enter API key or token",
                        value: "{input_value}",
                        autofocus: true,
                        oninput: move |evt| input_value.set(evt.value()),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter {
                                evt.prevent_default();
                                submit_key();
                            }
                        },
                    }
                }
            }
        }
    }
}
