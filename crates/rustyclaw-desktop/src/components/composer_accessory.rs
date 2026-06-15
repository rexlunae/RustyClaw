//! Composer accessory controls injected into `ChatSurface`'s input area via its
//! `input_accessory` slot: the provider/model selector and the working-directory
//! selector. These are RustyClaw-specific affordances the generic chat crate
//! intentionally doesn't know about.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Dropdown, DropdownItem, DropdownMenu, DropdownTrigger, Help,
    Select,
};
use rustyclaw_core::providers;

/// (provider_id, model_id) pair emitted when the user changes model.
pub type ModelSelection = (String, String);

/// Sentinel selected from the directory menu to open a native folder picker.
pub const DIRECTORY_OTHER_SENTINEL: &str = "__directory_other__";

/// Props for [`ComposerAccessory`].
#[derive(Props, Clone, PartialEq)]
pub struct ComposerAccessoryProps {
    pub current_provider: Option<String>,
    pub current_model: Option<String>,
    pub directory_selector: rustyclaw_view::DirectorySelectorState,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_add_provider: EventHandler<()>,
    pub on_toggle_directory_selector: EventHandler<()>,
    pub on_select_directory: EventHandler<String>,
}

/// The model bar + directory selector, rendered as one row inside the composer.
#[component]
pub fn ComposerAccessory(props: ComposerAccessoryProps) -> Element {
    rsx! {
        ModelBar {
            current_provider: props.current_provider.clone(),
            current_model: props.current_model.clone(),
            on_model_change: props.on_model_change,
            on_add_provider: props.on_add_provider,
        }
        DirectorySelectorBar {
            state: props.directory_selector.clone(),
            on_toggle: props.on_toggle_directory_selector,
            on_select: props.on_select_directory,
        }
    }
}

// ── Directory selector bar ──────────────────────────────────────────────────

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

// ── Model bar (provider / model selector) ────────────────────────────────────

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
