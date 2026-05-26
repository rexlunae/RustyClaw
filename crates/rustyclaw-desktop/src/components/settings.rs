//! Settings dialog: theme, gateway URL, provider credentials, manual reconnect.

use dioxus::prelude::*;
use rustyclaw_core::providers;

use crate::state::Theme;

/// Emitted when the user saves an API key for a provider.
pub type CredentialUpdate = (String, String); // (provider_id, api_key)

#[derive(Props, Clone, PartialEq)]
pub struct SettingsDialogProps {
    pub visible: bool,
    pub theme: Theme,
    pub gateway_url: String,
    pub gateway_config_toml: String,
    pub on_theme_change: EventHandler<Theme>,
    pub on_gateway_url_change: EventHandler<String>,
    pub on_gateway_config_change: EventHandler<String>,
    pub on_apply_gateway_config: EventHandler<String>,
    pub on_reconnect: EventHandler<()>,
    pub on_setup_totp: EventHandler<()>,
    pub on_credential_save: EventHandler<CredentialUpdate>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SettingsDialog(props: SettingsDialogProps) -> Element {
    let mut url = use_signal(|| props.gateway_url.clone());
    let mut gateway_config_toml = use_signal(|| props.gateway_config_toml.clone());
    let mut editing_provider: Signal<Option<String>> = use_signal(|| None);
    let mut key_input = use_signal(String::new);

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

    let provider_defs: Vec<_> = providers::provider_ids()
        .iter()
        .filter_map(|id| providers::provider_by_id(id).map(|def| (id.to_string(), def)))
        .collect();

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
                        "\u{2715}"
                    }
                }

                div { class: "modal-body",
                    // Provider credentials
                    div { class: "settings-section",
                        div { class: "settings-section-title", "Provider Credentials" }
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
                                        div { class: "field provider-cred-row",
                                            div {
                                                class: "provider-cred-info",
                                                span { class: "field-label", "{display}" }
                                                span {
                                                    class: "field-help",
                                                    style: "margin-left: 8px;",
                                                    "({hint})"
                                                }
                                            }
                                            if is_editing {
                                                div { class: "provider-cred-edit",
                                                    input {
                                                        class: "input",
                                                        r#type: "password",
                                                        placeholder: "API key",
                                                        value: "{key_input}",
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
                                                    button {
                                                        class: "btn btn-primary btn-sm",
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
                                                    button {
                                                        class: "btn btn-ghost btn-sm",
                                                        onclick: move |_| {
                                                            editing_provider.set(None);
                                                            key_input.set(String::new());
                                                        },
                                                        "Cancel"
                                                    }
                                                }
                                                if !help.is_empty() {
                                                    span {
                                                        class: "field-help",
                                                        style: "display: block; margin-top: 4px;",
                                                        "{help}"
                                                    }
                                                }
                                            } else {
                                                button {
                                                    class: "btn btn-subtle btn-sm",
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
                        div { class: "field",
                            span { class: "field-label", "Gateway Config (TOML)" }
                            textarea {
                                class: "input",
                                style: "min-height: 140px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace;",
                                value: "{gateway_config_toml}",
                                placeholder: "# Optional: full gateway config TOML pushed via this client",
                                oninput: move |evt| {
                                    let v = evt.value();
                                    gateway_config_toml.set(v.clone());
                                    props.on_gateway_config_change.call(v);
                                }
                            }
                            span { class: "field-help",
                                "Apply full gateway config updates (messengers, ports, tool permissions, etc.)."
                            }
                        }
                        div {
                            style: "margin-top: 10px; display: flex; justify-content: flex-end; gap: 8px;",
                            button {
                                class: "btn btn-ghost btn-sm",
                                onclick: move |_| props.on_setup_totp.call(()),
                                "Setup TOTP key"
                            }
                            button {
                                class: "btn btn-primary btn-sm",
                                onclick: move |_| {
                                    props
                                        .on_apply_gateway_config
                                        .call(gateway_config_toml.read().to_string());
                                },
                                "Apply config"
                            }
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
