//! Input bar: the model selector + text area + send/cancel button + directory selector.
//!
//! Analogue of the TUI's `components/input_bar.rs`.  Sub-components:
//!   - [`InputBar`] — public composite (attachments + ModelBar + textarea + button)
//!   - [`DirectorySelectorBar`] — working directory selector (Bulma dropdown)
//!   - [`ModelBar`] — provider/model selects
//!
//! The local text value is kept in a `Signal<String>` so that typing
//! updates are snappy and only the submit/cancel actions cross the
//! component boundary.  The composer textarea stays a native element
//! (Bulma's `Textarea` component has no `onkeydown`, which we need for
//! Enter-to-send).

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Dropdown, DropdownItem, DropdownMenu, DropdownTrigger, Help,
    Select, Tag, Tags,
};
use rustyclaw_core::providers;
use rustyclaw_view::{BottomBarData, ComposerData};

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
    let send_title = bottom_bar.composer.send_button_title();

    rsx! {
        div { class: "composer-wrap",
            div { class: "composer",
                textarea {
                    class: "textarea composer-input",
                    placeholder: ComposerData::PLACEHOLDER,
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
                span { title: "{send_title}",
                    Button {
                        color: if is_processing { BulmaColor::Danger } else { BulmaColor::Primary },
                        rounded: true,
                        class: "composer-send",
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
            }

            div { class: "composer-bottom-row",
                if !attachments.is_empty() {
                    Tags { class: "composer-attachments",
                        for attachment in attachments.clone() {
                            span {
                                key: "{attachment.path}",
                                title: "{attachment.path}",
                                Tag {
                                    rounded: true,
                                    delete: true,
                                    class: "composer-attachment-chip",
                                    ondelete: {
                                        let path = attachment.path.clone();
                                        let on_remove = props.on_remove_attachment;
                                        move |_| on_remove.call(path.clone())
                                    },
                                    span { class: "composer-attachment-icon", "{attachment.kind.icon()}" }
                                    span { class: "composer-attachment-name", "{attachment.display_name}" }
                                }
                            }
                        }
                        Button {
                            color: BulmaColor::Ghost,
                            size: BulmaSize::Small,
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
                Button {
                    color: BulmaColor::Ghost,
                    size: BulmaSize::Small,
                    onclick: move |_| props.on_add_file_attachment.call(()),
                    "Add file"
                }
                Button {
                    color: BulmaColor::Ghost,
                    size: BulmaSize::Small,
                    onclick: move |_| props.on_add_directory_attachment.call(()),
                    "Add dir"
                }
                DirectorySelectorBar {
                    state: bottom_bar.directory_selector.clone(),
                    on_toggle: props.on_toggle_directory_selector,
                    on_select: props.on_select_directory,
                }
            }

            div { class: "composer-hint", {ComposerData::HINT} }
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
    let display = state.current_display.clone().unwrap_or_else(|| {
        state
            .current_path
            .clone()
            .unwrap_or_else(|| "No directory".to_string())
    });

    let arrow = if state.is_expanded { "▾" } else { "▸" };
    let directories = state.available_directories.clone();

    rsx! {
        div { class: "directory-selector-bar",
            Dropdown {
                active: state.is_expanded,
                up: true,
                class: "directory-selector",
                DropdownTrigger {
                    onclick: move |_| props.on_toggle.call(()),
                    Button {
                        size: BulmaSize::Small,
                        class: "directory-selector-toggle",
                        span { class: "directory-selector-label", "Dir" }
                        span { class: "directory-path", "{display}" }
                        span { class: "directory-arrow", "{arrow}" }
                    }
                }
                DropdownMenu { class: "directory-selector-menu",
                    for dir in directories.into_iter() {
                        DropdownItem {
                            key: "{dir.path}",
                            active: dir.is_selected,
                            onclick: {
                                let path = dir.path.clone();
                                let on_select = props.on_select;
                                move |_| on_select.call(path.clone())
                            },
                            "{dir.display_name}"
                        }
                    }
                    DropdownItem {
                        class: "directory-item-other",
                        onclick: move |_| {
                            props.on_select.call(DIRECTORY_OTHER_SENTINEL.to_string())
                        },
                        "Other…"
                    }
                }
            }

            if let Some(err) = &state.error {
                Help {
                    color: BulmaColor::Danger,
                    class: "directory-selector-error",
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
    let current_model = props.current_model.clone().unwrap_or_else(|| {
        models_for_provider
            .first()
            .copied()
            .unwrap_or("")
            .to_string()
    });

    let mut provider_options: Vec<String> =
        provider_list.iter().map(|p| (*p).to_string()).collect();
    if !current_provider.is_empty() && !provider_options.iter().any(|p| p == &current_provider) {
        provider_options.insert(0, current_provider.clone());
    }
    let mut model_options: Vec<String> = models_for_provider
        .iter()
        .map(|m| (*m).to_string())
        .collect();
    if !current_model.is_empty() && !model_options.iter().any(|m| m == &current_model) {
        model_options.insert(0, current_model.clone());
    }

    rsx! {
        div { class: "model-bar",
            Select {
                size: BulmaSize::Small,
                value: current_provider.clone(),
                class: "model-bar-select",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let on_add_provider = props.on_add_provider;
                    let selected_model = current_model.clone();
                    move |evt: FormEvent| {
                        let prov = evt.value();
                        if prov == ADD_PROVIDER_SENTINEL {
                            on_add_provider.call(());
                            return;
                        }
                        let models = providers::models_for_provider(&prov);
                        let next_model = if !selected_model.is_empty()
                            && models.contains(&selected_model.as_str())
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

            Select {
                size: BulmaSize::Small,
                value: current_model.clone(),
                disabled: current_provider.is_empty(),
                class: "model-bar-select",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let selected_provider = current_provider.clone();
                    move |evt: FormEvent| {
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
