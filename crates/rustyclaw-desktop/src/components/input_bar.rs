//! Input bar: the model selector + text area + send/cancel button.
//!
//! Analogue of the TUI's `components/input_bar.rs`, extracted from
//! `chat.rs` during the Phase D structural alignment.  Sub-components:
//!   - [`InputBar`] — public composite (ModelBar + textarea + button)
//!   - [`ModelBar`] — provider/model dropdowns
//!
//! The local text value is kept in a `Signal<String>` so that typing
//! updates are snappy and only the submit/cancel actions cross the
//! component boundary.

use dioxus::prelude::*;
use rustyclaw_core::providers;

use super::messages::ModelSelection;

/// Props for [`InputBar`].
#[derive(Props, Clone, PartialEq)]
pub struct InputBarProps {
    pub input: Signal<String>,
    pub is_processing: bool,
    pub current_provider: Option<String>,
    pub current_model: Option<String>,
    pub on_send: EventHandler<()>,
    pub on_cancel: EventHandler<()>,
    pub on_input_change: EventHandler<String>,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_add_provider: EventHandler<()>,
}

/// Model selector bar + message input area with send/cancel button.
#[component]
pub fn InputBar(props: InputBarProps) -> Element {
    let mut input_ref = props.input;
    let is_processing = props.is_processing;
    let on_send = props.on_send;

    rsx! {
        div { class: "composer-wrap",
            ModelBar {
                current_provider: props.current_provider.clone(),
                current_model: props.current_model.clone(),
                on_model_change: props.on_model_change,
                on_add_provider: props.on_add_provider,
            }
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
            div { class: "composer-hint",
                "Press Enter to send · Shift + Enter for newline"
            }
        }
    }
}

// ── Model bar (provider / model selector above composer) ─────────────

/// Sentinel value used for the "Add provider…" menu entry.
const ADD_PROVIDER_SENTINEL: &str = "__add_provider__";

#[derive(Props, Clone, PartialEq)]
struct ModelBarProps {
    current_provider: Option<String>,
    current_model: Option<String>,
    on_model_change: EventHandler<ModelSelection>,
    on_add_provider: EventHandler<()>,
}

#[component]
fn ModelBar(props: ModelBarProps) -> Element {
    let mut selected_provider =
        use_signal(|| props.current_provider.clone().unwrap_or_default());
    let mut selected_model =
        use_signal(|| props.current_model.clone().unwrap_or_default());

    // Keep signals in sync when parent props change.
    {
        let pp = props.current_provider.clone();
        let pm = props.current_model.clone();
        use_effect(move || {
            if let Some(ref p) = pp {
                if *selected_provider.read() != *p {
                    selected_provider.set(p.clone());
                }
            }
            if let Some(ref m) = pm {
                if *selected_model.read() != *m {
                    selected_model.set(m.clone());
                }
            }
        });
    }

    let provider_list = providers::provider_ids();
    let current_provider_id = selected_provider.read().clone();
    let models_for_current = providers::models_for_provider(&current_provider_id);

    rsx! {
        div { class: "model-bar",
            select {
                class: "model-bar-select",
                value: "{selected_provider}",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let on_add_provider = props.on_add_provider;
                    move |evt: Event<FormData>| {
                        let prov = evt.value();
                        if prov == ADD_PROVIDER_SENTINEL {
                            on_add_provider.call(());
                            return;
                        }
                        selected_provider.set(prov.clone());
                        let models = providers::models_for_provider(&prov);
                        let first_model =
                            models.first().copied().unwrap_or("").to_string();
                        selected_model.set(first_model.clone());
                        on_model_change.call((prov, first_model));
                    }
                },
                for pid in provider_list.iter() {
                    option {
                        value: "{pid}",
                        selected: *pid == current_provider_id.as_str(),
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
                value: "{selected_model}",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let provider_signal = selected_provider;
                    move |evt: Event<FormData>| {
                        let mdl = evt.value();
                        selected_model.set(mdl.clone());
                        let prov = provider_signal.read().clone();
                        on_model_change.call((prov, mdl));
                    }
                },
                for mid in models_for_current.iter() {
                    option {
                        value: "{mid}",
                        selected: *mid == selected_model.read().as_str(),
                        "{mid}"
                    }
                }
            }
        }
    }
}
