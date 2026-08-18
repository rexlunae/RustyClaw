//! Composer accessory controls injected into `ChatSurface`'s input area via its
//! `input_accessory` slot: the provider/model selector and the working-directory
//! selector. These are RustyClaw-specific affordances the generic chat crate
//! intentionally doesn't know about.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Help, Select};
use rustyclaw_core::providers;
use std::collections::HashMap;

/// (provider_id, model_id) pair emitted when the user changes model.
pub type ModelSelection = (String, String);

/// Sentinel selected from the directory menu to open a native folder picker.
pub const DIRECTORY_OTHER_SENTINEL: &str = "__directory_other__";

/// Props for [`ComposerAccessory`].
#[derive(Props, Clone, PartialEq)]
pub struct ComposerAccessoryProps {
    pub current_provider: Option<String>,
    pub current_model: Option<String>,
    /// Live model lists fetched from provider APIs, keyed by provider id.
    pub provider_models: HashMap<String, Vec<String>>,
    /// Live "loaded/running" model ids per provider (a subset of
    /// `provider_models`); the picker marks those models as running.
    pub provider_loaded: HashMap<String, Vec<String>>,
    pub directory_selector: rustyclaw_view::DirectorySelectorState,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_add_provider: EventHandler<()>,
    pub on_select_directory: EventHandler<String>,
}

/// The model bar + directory selector, rendered as one row inside the composer.
#[component]
pub fn ComposerAccessory(props: ComposerAccessoryProps) -> Element {
    rsx! {
        ModelBar {
            current_provider: props.current_provider.clone(),
            current_model: props.current_model.clone(),
            provider_models: props.provider_models.clone(),
            provider_loaded: props.provider_loaded.clone(),
            on_model_change: props.on_model_change,
            on_add_provider: props.on_add_provider,
        }
        DirectorySelectorBar {
            state: props.directory_selector.clone(),
            on_select: props.on_select_directory,
        }
    }
}

// ── Directory selector bar ──────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct DirectorySelectorBarProps {
    state: rustyclaw_view::DirectorySelectorState,
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

    rsx! {
        div { class: "directory-selector-bar",
            Button {
                size: BulmaSize::Small,
                class: "directory-selector-toggle",
                onclick: move |_| {
                    props.on_select.call(DIRECTORY_OTHER_SENTINEL.to_string())
                },
                span { class: "directory-selector-label", "Dir" }
                span { class: "directory-path", "{display}" }
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
    /// Live model lists fetched from provider APIs, keyed by provider id.
    provider_models: HashMap<String, Vec<String>>,
    /// Live "loaded/running" model ids per provider.
    provider_loaded: HashMap<String, Vec<String>>,
    on_model_change: EventHandler<ModelSelection>,
    on_add_provider: EventHandler<()>,
}

/// Resolve the model list for a provider: prefer the live list fetched from
/// the provider API, falling back to the static catalogue entry.
fn resolve_models(provider_models: &HashMap<String, Vec<String>>, provider: &str) -> Vec<String> {
    match provider_models.get(provider) {
        Some(live) if !live.is_empty() => live.clone(),
        _ => providers::models_for_provider(provider)
            .iter()
            .map(|m| (*m).to_string())
            .collect(),
    }
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
    let models_for_provider = resolve_models(&props.provider_models, &provider_for_models);
    let current_model = props
        .current_model
        .clone()
        .unwrap_or_else(|| models_for_provider.first().cloned().unwrap_or_default());

    let mut provider_options: Vec<String> =
        provider_list.iter().map(|p| (*p).to_string()).collect();
    if !current_provider.is_empty() && !provider_options.iter().any(|p| p == &current_provider) {
        provider_options.insert(0, current_provider.clone());
    }
    let mut model_options: Vec<String> = models_for_provider.clone();
    if !current_model.is_empty() && !model_options.iter().any(|m| m == &current_model) {
        model_options.insert(0, current_model.clone());
    }
    // Which of the listed models are loaded/running on the local engine, so
    // the picker can say so instead of showing a flat list of names.
    let loaded_set: std::collections::HashSet<&String> = props
        .provider_loaded
        .get(&provider_for_models)
        .map(|v| v.iter().collect())
        .unwrap_or_default();

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
                    let provider_models = props.provider_models.clone();
                    move |evt: FormEvent| {
                        let prov = evt.value();
                        if prov == ADD_PROVIDER_SENTINEL {
                            on_add_provider.call(());
                            return;
                        }
                        let models = resolve_models(&provider_models, &prov);
                        let next_model = if !selected_model.is_empty()
                            && models.iter().any(|m| m == &selected_model)
                        {
                            selected_model.clone()
                        } else {
                            models.first().cloned().unwrap_or_default()
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
                        if loaded_set.contains(mid) { "{mid} ● running" } else { "{mid}" }
                    }
                }
            }
        }
    }
}
