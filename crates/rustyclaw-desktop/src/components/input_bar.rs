//! Input bar: the model selector + text area + send/cancel button + directory selector.
//!
//! Analogue of the TUI's `components/input_bar.rs`, extracted from
//! `chat.rs` during the Phase D structural alignment.  Sub-components:
//!   - [`InputBar`] — public composite (DirectoryBar + ModelBar + textarea + button)
//!   - [`DirectorySelectorBar`] — working directory selector
//!   - [`ModelBar`] — provider/model dropdowns
//!
//! The local text value is kept in a `Signal<String>` so that typing
//! updates are snappy and only the submit/cancel actions cross the
//! component boundary.

use dioxus::prelude::*;
use rustyclaw_core::providers;
use rustyclaw_view::BottomBarData;

use super::messages::ModelSelection;

/// Props for [`InputBar`].
#[derive(Props, Clone, PartialEq)]
pub struct InputBarProps {
    pub input: Signal<String>,
    pub bottom_bar: BottomBarData,
    pub on_send: EventHandler<()>,
    pub on_cancel: EventHandler<()>,
    pub on_input_change: EventHandler<String>,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_add_provider: EventHandler<()>,
    pub on_add_file_attachment: EventHandler<()>,
    pub on_add_directory_attachment: EventHandler<()>,
    pub on_clear_attachments: EventHandler<()>,
    pub on_remove_attachment: EventHandler<String>,
    pub on_toggle_directory_selector: EventHandler<()>,
    pub on_select_directory: EventHandler<String>,
}

/// Model selector bar + message input area with send/cancel button + directory selector.
#[component]
pub fn InputBar(props: InputBarProps) -> Element {
    let mut input_ref = props.input;
    let bottom_bar = &props.bottom_bar;
    let is_processing = bottom_bar.composer.is_processing;
    let on_send = props.on_send;
    let attachments = bottom_bar.composer.attachments.clone();

    rsx! {
        div { class: "composer-wrap",
            div { class: "composer",
                textarea {
                    placeholder: "Message RustyClaw…",
                    rows: "1",
                    value: "{input_ref}",
                    disabled: is_processing,
                    onkeydown: move |evt: KeyboardEvent| {
                        if evt.key() == Key::Enter && !evt.modifiers().shift() {
                            evt.prevent_default();
                            on_send.call(());
                        }
                    },
                    oninput: move |evt| {
                        let value = evt.value();
                        input_ref.set(value.clone());
                        props.on_input_change.call(value);
                    }
                }
                button {
                    class: "composer-send",
                    title: if is_processing { "Cancel request" } else { "Send (Enter)" },
                    disabled: !is_processing && input_ref.read().trim().is_empty(),
                    onclick: move |_| {
                        if is_processing {
                            props.on_cancel.call(());
                        } else {
                            on_send.call(());
                        }
                    },
                    if is_processing { "×" } else { "↑" }
                }
            }

            div { class: "composer-bottom-row",
                if !attachments.is_empty() {
                    div { class: "composer-attachments",
                        for attachment in attachments.clone() {
                            div {
                                key: "{attachment.path}",
                                class: "composer-attachment-chip",
                                title: "{attachment.path}",
                                span { class: "composer-attachment-icon", "{attachment.kind.icon()}" }
                                span { class: "composer-attachment-name", "{attachment.display_name}" }
                                button {
                                    class: "composer-attachment-remove",
                                    title: "Remove attachment",
                                    onclick: move |_| props.on_remove_attachment.call(attachment.path.clone()),
                                    "⊗"
                                }
                            }
                        }
                        button {
                            class: "btn btn-subtle btn-sm",
                            title: "Clear attached files and directories",
                            onclick: move |_| props.on_clear_attachments.call(()),
                            "Clear"
                        }
                    }
                }
                ModelBar {
                    current_provider: bottom_bar.composer.current_provider.clone(),
                    current_model: bottom_bar.composer.current_model.clone(),
                    on_model_change: props.on_model_change,
                    on_add_provider: props.on_add_provider,
                }
                button {
                    class: "btn btn-subtle btn-sm",
                    title: "Attach a file to the next prompt",
                    onclick: move |_| props.on_add_file_attachment.call(()),
                    "Add file"
                }
                button {
                    class: "btn btn-subtle btn-sm",
                    title: "Attach a directory to the next prompt",
                    onclick: move |_| props.on_add_directory_attachment.call(()),
                    "Add dir"
                }
                DirectorySelectorBar {
                    state: bottom_bar.directory_selector.clone(),
                    on_toggle: props.on_toggle_directory_selector,
                    on_select: props.on_select_directory,
                }
            }

            div { class: "composer-hint",
                "Press Enter to send · Shift + Enter for newline · Attachments are included in the next prompt"
            }
        }
    }
}

// ── Directory selector bar ──────────────────────────────────────────────────

const DIRECTORY_OTHER_SENTINEL: &str = "__directory_other__";

#[derive(Props, Clone, PartialEq)]
struct DirectorySelectorBarProps {
    state: rustyclaw_view::DirectorySelectorState,
    on_toggle: EventHandler<()>,
    on_select: EventHandler<String>,
}

#[component]
fn DirectorySelectorBar(props: DirectorySelectorBarProps) -> Element {
    let state = props.state.clone();
    let display = state
        .current_display
        .clone()
        .unwrap_or_else(|| state.current_path.clone().unwrap_or_else(|| "No directory".to_string()));

    let arrow = if state.is_expanded { "v" } else { ">" };

    rsx! {
        div { class: "directory-selector-bar",
            button {
                class: "directory-selector-toggle",
                title: "Change working directory",
                onclick: move |_| props.on_toggle.call(()),
                span { class: "directory-selector-label", "Dir" }
                span { class: "directory-path", "{display}" }
                span { class: "directory-arrow", "{arrow}" }
            }

            if state.is_expanded && !state.available_directories.is_empty() {
                div { class: "directory-selector-menu",
                    for dir in state.available_directories.clone().into_iter() {
                        button {
                            class: if dir.is_selected { 
                                "directory-item is-selected" 
                            } else { 
                                "directory-item" 
                            },
                            onclick: move |_| {
                                props.on_select.call(dir.path.clone())
                            },
                            "{dir.display_name}"
                        }
                    }
                    button {
                        class: "directory-item directory-item-other",
                        onclick: move |_| {
                            props.on_select.call(DIRECTORY_OTHER_SENTINEL.to_string())
                        },
                        "Other…"
                    }
                }
            }

            if let Some(err) = &state.error {
                div { class: "directory-selector-error",
                    "⚠ {err}"
                }
            }
        }
    }
}

// ── Model bar (provider / model selector above composer) ─────────────────────

/// Sentinel value used for the "Add provider…" menu entry.
const ADD_PROVIDER_SENTINEL: &str = "__add_provider__";

#[derive(Props, Clone, PartialEq)]
struct ModelBarProps {
    current_provider: Option<String>,
    current_model: Option<String>,
    on_model_change: EventHandler<ModelSelection>,
    on_add_provider: EventHandler<()>,
}

fn normalize_provider_id(id: &str) -> &str {
    match id {
        "copilot" | "github_copilot" | "githubcopilot" => "github-copilot",
        other => other,
    }
}

#[component]
fn ModelBar(props: ModelBarProps) -> Element {
    let provider_list = providers::provider_ids();
    let current_provider = props
        .current_provider
        .clone()
        .map(|p| normalize_provider_id(&p).to_string())
        .unwrap_or_default();
    let provider_for_models = if current_provider.is_empty() {
        provider_list.first().copied().unwrap_or("").to_string()
    } else {
        current_provider.clone()
    };
    let models_for_provider = providers::models_for_provider(&provider_for_models);
    let current_model = props
        .current_model
        .clone()
        .unwrap_or_else(|| models_for_provider.first().copied().unwrap_or("").to_string());

    let mut provider_options: Vec<String> = provider_list.iter().map(|p| (*p).to_string()).collect();
    if !current_provider.is_empty()
        && !provider_options.iter().any(|p| p == &current_provider)
    {
        provider_options.insert(0, current_provider.clone());
    }
    let mut model_options: Vec<String> = models_for_provider
        .iter()
        .map(|m| (*m).to_string())
        .collect();
    if !current_model.is_empty() && !model_options.iter().any(|m| m == &current_model)
    {
        model_options.insert(0, current_model.clone());
    }

    rsx! {
        div { class: "model-bar",
            select {
                class: "model-bar-select",
                value: "{current_provider}",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let on_add_provider = props.on_add_provider;
                    let selected_model = current_model.clone();
                    move |evt: Event<FormData>| {
                        let prov = evt.value();
                        if prov == ADD_PROVIDER_SENTINEL {
                            on_add_provider.call(());
                            return;
                        }
                        let models = providers::models_for_provider(&prov);
                        let next_model = if !selected_model.is_empty()
                            && models.iter().any(|m| *m == selected_model.as_str())
                        {
                            selected_model.clone()
                        } else {
                            models.first().copied().unwrap_or("").to_string()
                        };
                        if !prov.is_empty() {
                            on_model_change.call((prov, next_model));
                        }
                    }
                },
                option {
                    value: "",
                    selected: current_provider.is_empty(),
                    disabled: true,
                    "Select provider"
                }
                for pid in provider_options.iter() {
                    option {
                        value: "{pid}",
                        selected: *pid == current_provider,
                        "{providers::display_name_for_provider(pid)}"
                    }
                }
                option { disabled: true, "─────────────" }
                option {
                    value: "{ADD_PROVIDER_SENTINEL}",
                    "Add provider\u{2026}"
                }
            }

            select {
                class: "model-bar-select",
                value: "{current_model}",
                disabled: current_provider.is_empty(),
                onchange: {
                    let on_model_change = props.on_model_change;
                    let selected_provider = current_provider.clone();
                    move |evt: Event<FormData>| {
                        let mdl = evt.value();
                        let prov = selected_provider.clone();
                        if !prov.is_empty() {
                            on_model_change.call((prov, mdl));
                        }
                    }
                },
                if model_options.is_empty() {
                    option {
                        value: "",
                        selected: true,
                        disabled: true,
                        "No models"
                    }
                }
                for mid in model_options.iter() {
                    option {
                        value: "{mid}",
                        selected: *mid == current_model,
                        "{mid}"
                    }
                }
            }
        }
    }
}
