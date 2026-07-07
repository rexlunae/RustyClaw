//! Settings dialog: theme, gateway URL, provider credentials, custom
//! providers, manual reconnect.

use dioxus::prelude::*;
use dioxus_bulma::components::{Title, TitleSize};
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Control, Field, FieldLabel, Help,
};
use rustyclaw_core::providers::{self, ApiFormat, CustomProviderConfig};

use super::RcModal;
use crate::state::Theme;

/// Emitted when the user saves an API key for a provider.
pub type CredentialUpdate = (String, String); // (provider_id, api_key)

#[derive(Props, Clone, PartialEq)]
pub struct SettingsDialogProps {
    pub visible: bool,
    pub theme: Theme,
    pub gateway_url: String,
    /// User-defined providers from the config, shown with remove buttons.
    pub custom_providers: Vec<CustomProviderConfig>,
    pub on_theme_change: EventHandler<Theme>,
    pub on_gateway_url_change: EventHandler<String>,
    pub on_reconnect: EventHandler<()>,
    pub on_credential_save: EventHandler<CredentialUpdate>,
    /// Add (or replace, by id) a custom provider.
    pub on_custom_provider_add: EventHandler<CustomProviderConfig>,
    /// Remove a custom provider by id.
    pub on_custom_provider_remove: EventHandler<String>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SettingsDialog(props: SettingsDialogProps) -> Element {
    let mut url = use_signal(|| props.gateway_url.clone());
    let mut editing_provider: Signal<Option<String>> = use_signal(|| None);
    let mut key_input = use_signal(String::new);

    // Custom-provider add form state.
    let mut adding_custom = use_signal(|| false);
    let mut custom_id = use_signal(String::new);
    let mut custom_name = use_signal(String::new);
    let mut custom_url = use_signal(String::new);
    let mut custom_format = use_signal(|| ApiFormat::OpenAi);
    let mut custom_key_secret = use_signal(String::new);
    let mut custom_models = use_signal(String::new);
    let mut custom_error: Signal<Option<String>> = use_signal(|| None);

    if !props.visible {
        return rsx! {};
    }

    let mut reset_custom_form = move || {
        adding_custom.set(false);
        custom_id.set(String::new());
        custom_name.set(String::new());
        custom_url.set(String::new());
        custom_format.set(ApiFormat::OpenAi);
        custom_key_secret.set(String::new());
        custom_models.set(String::new());
        custom_error.set(None);
    };

    let on_custom_save = {
        let on_add = props.on_custom_provider_add;
        move |_| {
            let name = custom_name.read().trim().to_string();
            let key = custom_key_secret.read().trim().to_string();
            let cfg = CustomProviderConfig {
                id: custom_id.read().trim().to_string(),
                display_name: (!name.is_empty()).then_some(name),
                base_url: custom_url.read().trim().to_string(),
                api_format: *custom_format.read(),
                api_key_secret: (!key.is_empty()).then_some(key),
                models: custom_models
                    .read()
                    .split(',')
                    .map(|m| m.trim().to_string())
                    .filter(|m| !m.is_empty())
                    .collect(),
            };
            match cfg.validate() {
                Ok(()) => {
                    on_add.call(cfg);
                    reset_custom_form();
                }
                Err(e) => custom_error.set(Some(e.to_string())),
            }
        }
    };

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

            // Custom providers (local / self-hosted model servers)
            div { class: "settings-section",
                Title { size: TitleSize::Is6, class: "settings-section-title", "Custom Providers" }
                if props.custom_providers.is_empty() && !adding_custom() {
                    p { class: "help", "No custom providers configured. Add a local or self-hosted OpenAI-compatible server (vLLM, llama.cpp, …)." }
                }
                for cp in props.custom_providers.iter() {
                    {
                        let id = cp.id.clone();
                        let display = cp.display_name.clone().unwrap_or_else(|| cp.id.clone());
                        let format = cp.api_format.display();
                        let base_url = cp.base_url.clone();
                        let on_remove = props.on_custom_provider_remove;
                        rsx! {
                            Field { class: "provider-cred-row", key: "{id}",
                                div { class: "provider-cred-info",
                                    FieldLabel { class: "provider-cred-name", "{display}" }
                                    span { class: "provider-cred-hint", "({format}) {base_url}" }
                                }
                                Button {
                                    color: BulmaColor::Danger,
                                    outlined: true,
                                    size: BulmaSize::Small,
                                    onclick: move |_| on_remove.call(id.clone()),
                                    "Remove"
                                }
                            }
                        }
                    }
                }
                if adding_custom() {
                    div { class: "custom-provider-form",
                        Field {
                            FieldLabel { "ID" }
                            Control {
                                input {
                                    class: "input is-small",
                                    r#type: "text",
                                    placeholder: "my-vllm",
                                    value: "{custom_id}",
                                    autofocus: true,
                                    oninput: move |evt| custom_id.set(evt.value()),
                                }
                            }
                            Help { "Lowercase letters, digits, - and _; must not collide with a built-in provider." }
                        }
                        Field {
                            FieldLabel { "Display name (optional)" }
                            Control {
                                input {
                                    class: "input is-small",
                                    r#type: "text",
                                    placeholder: "My vLLM box",
                                    value: "{custom_name}",
                                    oninput: move |evt| custom_name.set(evt.value()),
                                }
                            }
                        }
                        Field {
                            FieldLabel { "Base URL" }
                            Control {
                                input {
                                    class: "input is-small",
                                    r#type: "text",
                                    placeholder: "http://192.168.1.50:8000/v1",
                                    value: "{custom_url}",
                                    oninput: move |evt| custom_url.set(evt.value()),
                                }
                            }
                        }
                        Field {
                            FieldLabel { "API format" }
                            Control {
                                Buttons { addons: true,
                                    for (format, label) in [
                                        (ApiFormat::OpenAi, "OpenAI"),
                                        (ApiFormat::Anthropic, "Anthropic"),
                                        (ApiFormat::Gemini, "Gemini"),
                                        (ApiFormat::Xai, "xAI"),
                                    ] {
                                        Button {
                                            color: if *custom_format.read() == format { BulmaColor::Primary } else { BulmaColor::Light },
                                            size: BulmaSize::Small,
                                            onclick: move |_| custom_format.set(format),
                                            "{label}"
                                        }
                                    }
                                }
                            }
                        }
                        Field {
                            FieldLabel { "API key secret (optional)" }
                            Control {
                                input {
                                    class: "input is-small",
                                    r#type: "text",
                                    placeholder: "MY_VLLM_API_KEY",
                                    value: "{custom_key_secret}",
                                    oninput: move |evt| custom_key_secret.set(evt.value()),
                                }
                            }
                            Help { "Vault entry or environment variable holding the key; leave empty for keyless local servers." }
                        }
                        Field {
                            FieldLabel { "Models (optional, comma-separated)" }
                            Control {
                                input {
                                    class: "input is-small",
                                    r#type: "text",
                                    placeholder: "qwen3-coder-30b, llama-3.3-70b",
                                    value: "{custom_models}",
                                    oninput: move |evt| custom_models.set(evt.value()),
                                }
                            }
                        }
                        if let Some(err) = custom_error.read().clone() {
                            p { class: "help is-danger", "{err}" }
                        }
                        Buttons {
                            Button {
                                color: BulmaColor::Primary,
                                size: BulmaSize::Small,
                                onclick: on_custom_save,
                                "Save Provider"
                            }
                            Button {
                                color: BulmaColor::Ghost,
                                size: BulmaSize::Small,
                                onclick: move |_| reset_custom_form(),
                                "Cancel"
                            }
                        }
                    }
                } else {
                    Buttons {
                        Button {
                            color: BulmaColor::Primary,
                            outlined: true,
                            size: BulmaSize::Small,
                            onclick: move |_| adding_custom.set(true),
                            "+ Add Custom Provider"
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
