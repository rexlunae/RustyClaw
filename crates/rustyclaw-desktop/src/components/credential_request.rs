//! Credential request dialog: gateway needs an API key or token from the user.

use dioxus::prelude::*;
use rustyclaw_view::CredentialRequestData;

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

    rsx! {
        div { class: "modal-backdrop",
            onclick: {
                let id = id_dismiss.clone();
                move |_| props.on_dismiss.call(id.clone())
            },

            div {
                class: "modal",
                style: "max-width: 480px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🔑 Credential Required" }
                    button {
                        class: "modal-close",
                        title: "Dismiss",
                        onclick: {
                            let id = id_dismiss.clone();
                            move |_| props.on_dismiss.call(id.clone())
                        },
                        "✕"
                    }
                }

                div { class: "modal-body",
                    div {
                        style: "margin-bottom: 12px;",
                        span {
                            style: "color: var(--accent-bright); font-weight: bold;",
                            "{props.data.provider}"
                        }
                        span {
                            style: "color: var(--text-dim); margin-left: 8px;",
                            "({props.data.secret_name})"
                        }
                    }

                    p {
                        style: "color: var(--text-dim); margin-bottom: 12px;",
                        "{props.data.message}"
                    }

                    form {
                        onsubmit: {
                            let id = id_submit.clone();
                            move |_| {
                                let val = input_value.read().clone();
                                if !val.is_empty() {
                                    props.on_submit.call((id.clone(), val));
                                    input_value.set(String::new());
                                }
                            }
                        },
                        div { class: "field",
                            input {
                                class: "input",
                                r#type: "password",
                                placeholder: "Enter API key or token",
                                value: "{input_value}",
                                oninput: move |evt| input_value.set(evt.value()),
                            }
                        }
                        div {
                            style: "margin-top: 12px; display: flex; justify-content: flex-end; gap: 8px;",
                            button {
                                class: "btn btn-subtle",
                                r#type: "button",
                                onclick: {
                                    let id = id_dismiss;
                                    move |_| props.on_dismiss.call(id.clone())
                                },
                                "Skip"
                            }
                            button {
                                class: "btn btn-primary",
                                r#type: "submit",
                                "Save & Continue"
                            }
                        }
                    }
                }
            }
        }
    }
}
