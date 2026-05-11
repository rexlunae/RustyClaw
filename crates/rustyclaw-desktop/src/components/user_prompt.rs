//! User prompt dialog: structured input requested by the agent (ask_user tool).

use dioxus::prelude::*;
use rustyclaw_core::user_prompt_types::{PromptResponseValue, PromptType, UserPrompt};

#[derive(Props, Clone, PartialEq)]
pub struct UserPromptDialogProps {
    pub visible: bool,
    pub prompt: Option<UserPrompt>,
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

    let prompt = match &props.prompt {
        Some(p) => p.clone(),
        None => return rsx! {},
    };

    let prompt_id = prompt.id.clone();
    let prompt_id_dismiss = prompt.id.clone();

    rsx! {
        div { class: "modal-backdrop",
            onclick: move |_| props.on_dismiss.call(prompt_id_dismiss.clone()),

            div {
                class: "modal",
                style: "max-width: 520px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "💬 {prompt.title}" }
                    button {
                        class: "modal-close",
                        title: "Dismiss",
                        onclick: {
                            let id = prompt_id.clone();
                            move |_| props.on_dismiss.call(id.clone())
                        },
                        "✕"
                    }
                }

                div { class: "modal-body",
                    if let Some(desc) = &prompt.description {
                        p {
                            style: "color: var(--text-dim); margin-bottom: 12px;",
                            "{desc}"
                        }
                    }

                    {match &prompt.prompt_type {
                        PromptType::TextInput { placeholder, .. } => {
                            let ph = placeholder.clone().unwrap_or_default();
                            rsx! {
                                div { class: "field",
                                    input {
                                        class: "input",
                                        r#type: "text",
                                        placeholder: "{ph}",
                                        value: "{text_input}",
                                        oninput: move |evt| text_input.set(evt.value()),
                                    }
                                }
                            }
                        }
                        PromptType::Confirm { default } => {
                            let _ = *default;
                            rsx! {
                                div {
                                    style: "display: flex; gap: 12px;",
                                    label {
                                        style: "display: flex; align-items: center; gap: 6px; cursor: pointer;",
                                        input {
                                            r#type: "radio",
                                            name: "confirm",
                                            checked: *confirm_value.read(),
                                            onchange: move |_| confirm_value.set(true),
                                        }
                                        "Yes"
                                    }
                                    label {
                                        style: "display: flex; align-items: center; gap: 6px; cursor: pointer;",
                                        input {
                                            r#type: "radio",
                                            name: "confirm",
                                            checked: !*confirm_value.read(),
                                            onchange: move |_| confirm_value.set(false),
                                        }
                                        "No"
                                    }
                                }
                            }
                        }
                        PromptType::Select { options, .. } => {
                            rsx! {
                                div {
                                    style: "display: flex; flex-direction: column; gap: 4px; max-height: 300px; overflow: auto;",
                                    for (i, opt) in options.iter().enumerate() {
                                        {
                                            let bg = if *selected_index.read() == i { "var(--accent-dim)" } else { "transparent" };
                                            let item_style = format!("padding: 8px 12px; border-radius: 6px; cursor: pointer; background: {};", bg);
                                            rsx! {
                                        div {
                                            key: "{i}",
                                            style: "{item_style}",
                                            onclick: move |_| selected_index.set(i),
                                            span {
                                                style: "font-weight: 500;",
                                                "{opt.label}"
                                            }
                                            if let Some(desc) = &opt.description {
                                                span {
                                                    style: "color: var(--text-dim); margin-left: 8px; font-size: 0.9em;",
                                                    "{desc}"
                                                }
                                            }
                                        }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => rsx! {
                            p { style: "color: var(--text-dim);", "Unsupported prompt type." }
                        },
                    }}
                }

                div { class: "modal-foot",
                    button {
                        class: "btn btn-subtle",
                        onclick: {
                            let id = prompt_id.clone();
                            move |_| props.on_dismiss.call(id.clone())
                        },
                        "Dismiss"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: {
                            let id = prompt_id.clone();
                            let prompt_type = prompt.prompt_type.clone();
                            move |_| {
                                let value = match &prompt_type {
                                    PromptType::TextInput { .. } => {
                                        PromptResponseValue::Text(text_input.read().clone())
                                    }
                                    PromptType::Confirm { .. } => {
                                        PromptResponseValue::Confirm(*confirm_value.read())
                                    }
                                    PromptType::Select { options, .. } => {
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
            }
        }
    }
}
