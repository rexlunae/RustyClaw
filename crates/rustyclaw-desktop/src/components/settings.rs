//! Settings dialog: theme, gateway URL, manual reconnect.

use dioxus::prelude::*;

use crate::state::Theme;

#[derive(Props, Clone, PartialEq)]
pub struct SettingsDialogProps {
    pub visible: bool,
    pub theme: Theme,
    pub gateway_url: String,
    pub on_theme_change: EventHandler<Theme>,
    pub on_gateway_url_change: EventHandler<String>,
    pub on_reconnect: EventHandler<()>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SettingsDialog(props: SettingsDialogProps) -> Element {
    // Hooks must be called unconditionally on every render and in the same
    // order. Declare all signals before the visibility guard so the hook
    // index stays stable when the dialog is opened/closed.
    let mut url = use_signal(|| props.gateway_url.clone());

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

    rsx! {
        div { class: "modal-backdrop",
            onclick: move |_| props.on_close.call(()),

            div {
                class: "modal",
                style: "max-width: 480px;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "⚙ Settings" }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| props.on_close.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    // Appearance
                    div { class: "settings-section",
                        div { class: "settings-section-title", "Appearance" }
                        div { class: "field",
                            span { class: "field-label", "Theme" }
                            div { class: "theme-toggle",
                                button {
                                    class: "{dark_class}",
                                    onclick: move |_| props.on_theme_change.call(Theme::Dark),
                                    "🌙 Dark"
                                }
                                button {
                                    class: "{light_class}",
                                    onclick: move |_| props.on_theme_change.call(Theme::Light),
                                    "☀ Light"
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
                                "↻ Reconnect"
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
