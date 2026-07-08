//! Inline user prompt: structured input requested by the agent (`ask_user`
//! tool), rendered as a card in the chat stream instead of a modal overlay.
//!
//! The card takes the composer's place at the bottom of the transcript while
//! the agent is waiting for an answer, so the question is always visible in
//! context. It supports all five prompt types: free text, yes/no confirm,
//! single select (radio), multi select (checkboxes), and multi-field forms.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons};
use rustyclaw_core::user_prompt_types::{PromptResponseValue, PromptType, UserPrompt};

#[derive(Props, Clone, PartialEq)]
pub struct UserPromptCardProps {
    pub prompt: UserPrompt,
    pub on_respond: EventHandler<(String, PromptResponseValue)>,
    pub on_dismiss: EventHandler<String>,
}

/// Inline question card. Mount it keyed by `prompt.id` so per-prompt input
/// state resets whenever a new question arrives.
#[component]
pub fn UserPromptCard(props: UserPromptCardProps) -> Element {
    let prompt = props.prompt.clone();

    // Per-type input state, initialised from the prompt's defaults.
    let text_default = match &prompt.prompt_type {
        PromptType::TextInput { default, .. } => default.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let mut text_input = use_signal(move || text_default);

    let selected_default = match &prompt.prompt_type {
        PromptType::Select { default, .. } => default.unwrap_or(0),
        _ => 0,
    };
    let mut selected_index = use_signal(move || selected_default);

    let checked_default = match &prompt.prompt_type {
        PromptType::MultiSelect { options, defaults } => {
            let mut checked = vec![false; options.len()];
            for &i in defaults {
                if let Some(slot) = checked.get_mut(i) {
                    *slot = true;
                }
            }
            checked
        }
        _ => Vec::new(),
    };
    let mut checked = use_signal(move || checked_default);

    let form_defaults = match &prompt.prompt_type {
        PromptType::Form { fields } => fields
            .iter()
            .map(|f| f.default.clone().unwrap_or_default())
            .collect(),
        _ => Vec::new(),
    };
    let mut form_values = use_signal(move || form_defaults);

    // Shared submit: build the typed response for the current prompt type.
    let prompt_for_submit = prompt.clone();
    let on_respond = props.on_respond;
    let submit = move || {
        let value = match &prompt_for_submit.prompt_type {
            PromptType::TextInput { .. } => PromptResponseValue::Text(text_input.read().clone()),
            // Confirm answers via its own Yes/No buttons; this path is
            // never wired up for it, but stay total rather than panic.
            PromptType::Confirm { default } => PromptResponseValue::Confirm(*default),
            PromptType::Select { options, .. } => {
                let label = options
                    .get(*selected_index.read())
                    .map(|o| o.label.clone())
                    .unwrap_or_default();
                PromptResponseValue::Selected(vec![label])
            }
            PromptType::MultiSelect { options, .. } => {
                let checked = checked.read();
                let labels = options
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| checked.get(*i).copied().unwrap_or(false))
                    .map(|(_, o)| o.label.clone())
                    .collect();
                PromptResponseValue::Selected(labels)
            }
            PromptType::Form { fields } => {
                let values = form_values.read();
                PromptResponseValue::Form(
                    fields
                        .iter()
                        .enumerate()
                        .map(|(i, f)| (f.name.clone(), values.get(i).cloned().unwrap_or_default()))
                        .collect(),
                )
            }
        };
        on_respond.call((prompt_for_submit.id.clone(), value));
    };
    let submit_for_enter = submit.clone();
    let submit_for_click = submit.clone();

    // A form is submittable once every required field has a value.
    let form_complete = match &prompt.prompt_type {
        PromptType::Form { fields } => {
            let values = form_values.read();
            fields
                .iter()
                .enumerate()
                .all(|(i, f)| !f.required || values.get(i).is_some_and(|v| !v.trim().is_empty()))
        }
        _ => true,
    };

    let is_confirm = matches!(prompt.prompt_type, PromptType::Confirm { .. });
    let prompt_id_confirm = prompt.id.clone();
    let prompt_id_dismiss = prompt.id.clone();
    let on_dismiss = props.on_dismiss;

    rsx! {
        div { class: "rc-inline-prompt",
            div { class: "rc-inline-prompt-header",
                span { class: "rc-inline-prompt-icon", "💬" }
                span { class: "rc-inline-prompt-title", "{prompt.title}" }
            }
            if let Some(description) = prompt.description.as_deref().filter(|d| !d.is_empty()) {
                p { class: "rc-inline-prompt-desc", "{description}" }
            }

            {match &prompt.prompt_type {
                PromptType::TextInput { placeholder, .. } => {
                    let ph = placeholder.clone().unwrap_or_default();
                    rsx! {
                        input {
                            class: "input rc-inline-prompt-input",
                            r#type: "text",
                            placeholder: "{ph}",
                            value: "{text_input}",
                            autofocus: true,
                            oninput: move |evt| text_input.set(evt.value()),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter {
                                    evt.prevent_default();
                                    submit_for_enter();
                                }
                            },
                        }
                    }
                }
                PromptType::Confirm { .. } => {
                    // Yes/No answer immediately on click; no separate submit.
                    let id_yes = prompt_id_confirm.clone();
                    let id_no = prompt_id_confirm.clone();
                    rsx! {
                        Buttons { class: "rc-inline-prompt-confirm",
                            Button {
                                color: BulmaColor::Primary,
                                onclick: move |_| on_respond
                                    .call((id_yes.clone(), PromptResponseValue::Confirm(true))),
                                "Yes"
                            }
                            Button {
                                color: BulmaColor::Light,
                                onclick: move |_| on_respond
                                    .call((id_no.clone(), PromptResponseValue::Confirm(false))),
                                "No"
                            }
                        }
                    }
                }
                PromptType::Select { options, .. } => rsx! {
                    div { class: "rc-select-options",
                        for (i, opt) in options.iter().enumerate() {
                            label {
                                key: "{i}",
                                class: if *selected_index.read() == i {
                                    "rc-select-option is-selected"
                                } else {
                                    "rc-select-option"
                                },
                                input {
                                    r#type: "radio",
                                    name: "rc-prompt-{prompt.id}",
                                    checked: *selected_index.read() == i,
                                    onchange: move |_| selected_index.set(i),
                                }
                                span { class: "rc-select-option-label", "{opt.label}" }
                                if let Some(desc) = &opt.description {
                                    span { class: "rc-select-option-desc", "{desc}" }
                                }
                            }
                        }
                    }
                },
                PromptType::MultiSelect { options, .. } => rsx! {
                    div { class: "rc-select-options",
                        for (i, opt) in options.iter().enumerate() {
                            label {
                                key: "{i}",
                                class: if checked.read().get(i).copied().unwrap_or(false) {
                                    "rc-select-option is-selected"
                                } else {
                                    "rc-select-option"
                                },
                                input {
                                    r#type: "checkbox",
                                    checked: checked.read().get(i).copied().unwrap_or(false),
                                    onchange: move |_| {
                                        let mut c = checked.write();
                                        if let Some(slot) = c.get_mut(i) {
                                            *slot = !*slot;
                                        }
                                    },
                                }
                                span { class: "rc-select-option-label", "{opt.label}" }
                                if let Some(desc) = &opt.description {
                                    span { class: "rc-select-option-desc", "{desc}" }
                                }
                            }
                        }
                    }
                },
                PromptType::Form { fields } => rsx! {
                    div { class: "rc-inline-prompt-form",
                        for (i, field) in fields.iter().enumerate() {
                            div { key: "{i}", class: "field",
                                label { class: "label rc-inline-prompt-field-label",
                                    "{field.label}"
                                    if field.required {
                                        span { class: "rc-inline-prompt-required", " *" }
                                    }
                                }
                                input {
                                    class: "input rc-inline-prompt-input",
                                    r#type: "text",
                                    placeholder: field.placeholder.clone().unwrap_or_default(),
                                    value: form_values.read().get(i).cloned().unwrap_or_default(),
                                    autofocus: i == 0,
                                    oninput: move |evt| {
                                        let mut v = form_values.write();
                                        if let Some(slot) = v.get_mut(i) {
                                            *slot = evt.value();
                                        }
                                    },
                                }
                            }
                        }
                    }
                },
            }}

            div { class: "rc-inline-prompt-actions",
                Button {
                    size: BulmaSize::Small,
                    color: BulmaColor::Ghost,
                    class: "rc-inline-prompt-dismiss",
                    onclick: move |_| on_dismiss.call(prompt_id_dismiss.clone()),
                    "Dismiss"
                }
                if !is_confirm {
                    Button {
                        color: BulmaColor::Primary,
                        disabled: !form_complete,
                        onclick: move |_| submit_for_click(),
                        "Answer"
                    }
                }
            }
        }
    }
}
