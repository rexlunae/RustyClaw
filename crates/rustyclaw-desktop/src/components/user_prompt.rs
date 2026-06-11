//! User prompt dialog: structured input requested by the agent (ask_user tool).

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, Radio};
use rustyclaw_core::user_prompt_types::{PromptResponseValue, PromptType};
use rustyclaw_view::UserPromptData;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct UserPromptDialogProps {
    pub visible: bool,
    pub prompt_id: String,
    pub data: Option<UserPromptData>,
    pub on_respond: EventHandler<(String, PromptResponseValue)>,
    pub on_dismiss: EventHandler<String>,
}

#[component]
pub fn UserPromptDialog(props: UserPromptDialogProps) -> Element {
    let mut text_input = use_signal(String::new);
    let mut confirm_value = use_signal(|| true);
    let mut selected_index = use_signal(|| 0usize);

    if !props.visible {
        return rsx! {};
    }

    let prompt = match &props.data {
        Some(p) => p.clone(),
        None => return rsx! {},
    };

    let prompt_id = props.prompt_id.clone();
    let prompt_id_dismiss = props.prompt_id.clone();
    let prompt_id_footer = props.prompt_id.clone();

    rsx! {
        RcModal {
            active: true,
            title: "💬 {prompt.title}",
            width: 520,
            onclose: move |_| props.on_dismiss.call(prompt_id_dismiss.clone()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: {
                            let id = prompt_id_footer.clone();
                            move |_| props.on_dismiss.call(id.clone())
                        },
                        "Dismiss"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        onclick: {
                            let id = prompt_id.clone();
                            let prompt_type = prompt.prompt_type.clone();
                            move |_| {
                                let value = match &prompt_type {
                                    Some(PromptType::TextInput { .. }) => {
                                        PromptResponseValue::Text(text_input.read().clone())
                                    }
                                    Some(PromptType::Confirm { .. }) => {
                                        PromptResponseValue::Confirm(*confirm_value.read())
                                    }
                                    Some(PromptType::Select { options, .. }) => {
                                        let idx = *selected_index.read();
                                        let label = options
                                            .get(idx)
                                            .map(|o| o.label.clone())
                                            .unwrap_or_default();
                                        PromptResponseValue::Text(label)
                                    }
                                    _ => PromptResponseValue::Text(String::new()),
                                };
                                props.on_respond.call((id.clone(), value));
                            }
                        },
                        "Submit"
                    }
                }
            },

            if !prompt.description.is_empty() {
                p { class: "rc-dialog-lead", "{prompt.description}" }
            }

            {match &prompt.prompt_type {
                Some(PromptType::TextInput { placeholder, .. }) => {
                    let ph = placeholder.clone().unwrap_or_default();
                    rsx! {
                        Field {
                            Control {
                                input {
                                    class: "input",
                                    r#type: "text",
                                    placeholder: "{ph}",
                                    value: "{text_input}",
                                    autofocus: true,
                                    oninput: move |evt| text_input.set(evt.value()),
                                }
                            }
                        }
                    }
                }
                Some(PromptType::Confirm { default }) => {
                    let _ = *default;
                    rsx! {
                        Field { class: "rc-confirm-options",
                            Radio {
                                name: "confirm",
                                value: "yes",
                                checked: *confirm_value.read(),
                                onchange: move |_| confirm_value.set(true),
                                "Yes"
                            }
                            Radio {
                                name: "confirm",
                                value: "no",
                                checked: !*confirm_value.read(),
                                onchange: move |_| confirm_value.set(false),
                                "No"
                            }
                        }
                    }
                }
                Some(PromptType::Select { options, .. }) => {
                    rsx! {
                        div { class: "rc-select-options",
                            for (i, opt) in options.iter().enumerate() {
                                div {
                                    key: "{i}",
                                    class: if *selected_index.read() == i { "rc-select-option is-selected" } else { "rc-select-option" },
                                    onclick: move |_| selected_index.set(i),
                                    span { class: "rc-select-option-label", "{opt.label}" }
                                    if let Some(desc) = &opt.description {
                                        span { class: "rc-select-option-desc", "{desc}" }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => rsx! {
                    p { class: "rc-dialog-lead", "Unsupported prompt type." }
                },
            }}
        }
    }
}
