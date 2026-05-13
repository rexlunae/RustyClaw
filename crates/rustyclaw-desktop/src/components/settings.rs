//! Settings dialog: theme, gateway URL, model/provider selection, manual reconnect.

use dioxus::prelude::*;
use rustyclaw_core::providers;

use crate::state::Theme;

/// (provider_id, model_id) pair emitted when the user selects a new model.
pub type ModelSelection = (String, String);

#[derive(Props, Clone, PartialEq)]
pub struct SettingsDialogProps {
    pub visible: bool,
    pub theme: Theme,
    pub gateway_url: String,
    pub current_provider: Option<String>,
    pub current_model: Option<String>,
    pub on_theme_change: EventHandler<Theme>,
    pub on_gateway_url_change: EventHandler<String>,
    pub on_reconnect: EventHandler<()>,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SettingsDialog(props: SettingsDialogProps) -> Element {
    // Hooks must be called unconditionally on every render and in the same
    // order. Declare all signals before the visibility guard so the hook
    // index stays stable when the dialog is opened/closed.
    let mut url = use_signal(|| props.gateway_url.clone());
    let mut selected_provider = use_signal(|| props.current_provider.clone().unwrap_or_default());
    let mut selected_model = use_signal(|| props.current_model.clone().unwrap_or_default());

    if !props.visible {
        return rsx! {};
    }

    let dark_class = if props.theme == Theme::Dark {
        "is-active"
    } else {
        ""
    };
    let light_class = if props.theme == Theme::Light {
        "is-active"
    } else {
        ""
    };

    let provider_list = providers::provider_ids();
    let current_provider_id = selected_provider.read().clone();
    let models_for_current = providers::models_for_provider(&current_provider_id);

    rsx! {
        div { class: "modal-backdrop",
            onclick: move |_| props.on_close.call(()),

            div {
                class: "modal",
                style: "max-width: 480px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "Settings" }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| props.on_close.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    // Model / Provider
                    div { class: "settings-section",
                        div { class: "settings-section-title", "Model" }
                        div { class: "field",
                            span { class: "field-label", "Provider" }
                            select {
                                class: "input",
                                value: "{selected_provider}",
                                onchange: {
                                    let on_model_change = props.on_model_change;
                                    move |evt: Event<FormData>| {
                                        let prov = evt.value();
                                        selected_provider.set(prov.clone());
                                        let models = providers::models_for_provider(&prov);
                                        let first_model = models.first().copied().unwrap_or("").to_string();
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
                            }
                        }
                        div { class: "field",
                            span { class: "field-label", "Model" }
                            select {
                                class: "input",
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

                    // Appearance
                    div { class: "settings-section",
                        div { class: "settings-section-title", "Appearance" }
                        div { class: "field",
                            span { class: "field-label", "Theme" }
                            div { class: "theme-toggle",
                                button {
                                    class: "{dark_class}",
                                    onclick: move |_| props.on_theme_change.call(Theme::Dark),
                                    "Dark"
                                }
                                button {
                                    class: "{light_class}",
                                    onclick: move |_| props.on_theme_change.call(Theme::Light),
                                    "Light"
                                }
                            }
                        }
                    }

                    // Connection
                    div { class: "settings-section",
                        div { class: "settings-section-title", "Connection" }
                        div { class: "field",
                            span { class: "field-label", "Gateway URL" }
                            input {
                                class: "input",
                                r#type: "text",
                                value: "{url}",
                                placeholder: "ssh://127.0.0.1:2222",
                                oninput: move |evt| {
                                    let v = evt.value();
                                    url.set(v.clone());
                                    props.on_gateway_url_change.call(v);
                                }
                            }
                            span { class: "field-help",
                                "RustyClaw connects to your gateway over SSH."
                            }
                        }
                        div {
                            style: "margin-top: 10px; display: flex; justify-content: flex-end;",
                            button {
                                class: "btn btn-subtle btn-sm",
                                onclick: move |_| props.on_reconnect.call(()),
                                "Reconnect"
                            }
                        }
                    }
                }

                div { class: "modal-foot",
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| props.on_close.call(()),
                        "Done"
                    }
                }
            }
        }
    }
}
