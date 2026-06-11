//! Settings dialog: theme, gateway URL, provider credentials, manual reconnect.

use dioxus::prelude::*;
use dioxus_bulma::components::{Title, TitleSize};
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Control, Field, FieldLabel, Help,
};
use rustyclaw_core::providers;

use super::RcModal;
use crate::state::Theme;

/// Emitted when the user saves an API key for a provider.
pub type CredentialUpdate = (String, String); // (provider_id, api_key)

#[derive(Props, Clone, PartialEq)]
pub struct SettingsDialogProps {
    pub visible: bool,
    pub theme: Theme,
    pub gateway_url: String,
    pub on_theme_change: EventHandler<Theme>,
    pub on_gateway_url_change: EventHandler<String>,
    pub on_reconnect: EventHandler<()>,
    pub on_credential_save: EventHandler<CredentialUpdate>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SettingsDialog(props: SettingsDialogProps) -> Element {
    let mut url = use_signal(|| props.gateway_url.clone());
    let mut editing_provider: Signal<Option<String>> = use_signal(|| None);
    let mut key_input = use_signal(String::new);

    if !props.visible {
        return rsx! {};
    }

    let provider_defs: Vec<_> = providers::provider_ids()
        .iter()
        .filter_map(|id| providers::provider_by_id(id).map(|def| (id.to_string(), def)))
        .collect();

    let is_dark = props.theme == Theme::Dark;

    rsx! {
        RcModal {
            active: true,
            title: "Settings",
            width: 520,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Done"
                    }
                }
            },

            // Provider credentials
            div { class: "settings-section",
                Title { size: TitleSize::Is6, class: "settings-section-title", "Provider Credentials" }
                for (pid, def) in provider_defs.iter() {
                    {
                        let needs_key = def.secret_key.is_some()
                            && def.auth_method != providers::AuthMethod::None
                            && def.auth_method != providers::AuthMethod::DeviceFlow;
                        let is_editing = editing_provider
                            .read()
                            .as_deref() == Some(pid.as_str());

                        if needs_key {
                            let pid_clone = pid.clone();
                            let display = def.display.to_string();
                            let is_optional = def.auth_method
                                == providers::AuthMethod::OptionalApiKey;
                            let hint = if is_optional {
                                "optional"
                            } else {
                                "required"
                            };
                            let help = def.help_text.unwrap_or("");

                            rsx! {
                                Field { class: "provider-cred-row",
                                    div { class: "provider-cred-info",
                                        FieldLabel { class: "provider-cred-name", "{display}" }
                                        span { class: "provider-cred-hint", "({hint})" }
                                    }
                                    if is_editing {
                                        Field { addons: true, class: "provider-cred-edit",
                                            Control { expanded: true,
                                                input {
                                                    class: "input is-small",
                                                    r#type: "password",
                                                    placeholder: "API key",
                                                    value: "{key_input}",
                                                    autofocus: true,
                                                    oninput: move |evt| {
                                                        key_input.set(evt.value());
                                                    },
                                                    onkeydown: {
                                                        let pid2 = pid_clone.clone();
                                                        let on_save = props.on_credential_save;
                                                        move |evt: KeyboardEvent| {
                                                            if evt.key() == Key::Enter {
                                                                let k = key_input.read().trim().to_string();
                                                                if !k.is_empty() {
                                                                    on_save.call((pid2.clone(), k));
                                                                }
                                                                editing_provider.set(None);
                                                                key_input.set(String::new());
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Control {
                                                Button {
                                                    color: BulmaColor::Primary,
                                                    size: BulmaSize::Small,
                                                    onclick: {
                                                        let pid3 = pid_clone.clone();
                                                        let on_save = props.on_credential_save;
                                                        move |_| {
                                                            let k = key_input.read().trim().to_string();
                                                            if !k.is_empty() {
                                                                on_save.call((pid3.clone(), k));
                                                            }
                                                            editing_provider.set(None);
                                                            key_input.set(String::new());
                                                        }
                                                    },
                                                    "Save"
                                                }
                                            }
                                            Control {
                                                Button {
                                                    color: BulmaColor::Ghost,
                                                    size: BulmaSize::Small,
                                                    onclick: move |_| {
                                                        editing_provider.set(None);
                                                        key_input.set(String::new());
                                                    },
                                                    "Cancel"
                                                }
                                            }
                                        }
                                        if !help.is_empty() {
                                            Help { "{help}" }
                                        }
                                    } else {
                                        Button {
                                            color: BulmaColor::Light,
                                            size: BulmaSize::Small,
                                            onclick: {
                                                let pid4 = pid_clone.clone();
                                                move |_| {
                                                    editing_provider.set(Some(pid4.clone()));
                                                    key_input.set(String::new());
                                                }
                                            },
                                            "Set key"
                                        }
                                    }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }

            // Appearance
            div { class: "settings-section",
                Title { size: TitleSize::Is6, class: "settings-section-title", "Appearance" }
                Field {
                    FieldLabel { "Theme" }
                    Control {
                        Buttons { addons: true, class: "theme-toggle",
                            Button {
                                color: if is_dark { BulmaColor::Primary } else { BulmaColor::Light },
                                size: BulmaSize::Small,
                                class: if is_dark { "is-selected" } else { "" },
                                onclick: move |_| props.on_theme_change.call(Theme::Dark),
                                "Dark"
                            }
                            Button {
                                color: if !is_dark { BulmaColor::Primary } else { BulmaColor::Light },
                                size: BulmaSize::Small,
                                class: if !is_dark { "is-selected" } else { "" },
                                onclick: move |_| props.on_theme_change.call(Theme::Light),
                                "Light"
                            }
                        }
                    }
                }
            }

            // Connection
            div { class: "settings-section",
                Title { size: TitleSize::Is6, class: "settings-section-title", "Connection" }
                Field {
                    FieldLabel { "Gateway URL" }
                    Control {
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
                    }
                    Help { "RustyClaw connects to your gateway over SSH." }
                }
                Buttons { alignment: dioxus_bulma::prelude::ButtonsAlignment::Right,
                    Button {
                        color: BulmaColor::Light,
                        size: BulmaSize::Small,
                        onclick: move |_| props.on_reconnect.call(()),
                        "Reconnect"
                    }
                }
            }
        }
    }
}
