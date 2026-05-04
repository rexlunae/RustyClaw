//! Sidebar component for sessions and settings.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

use crate::state::{ConnectionStatus, ThreadInfo};

/// Props for Sidebar.
#[derive(Props, Clone, PartialEq)]
pub struct SidebarProps {
    /// Connection status
    pub connection: ConnectionStatus,
    /// Agent name
    pub agent_name: Option<String>,
    /// Current model
    pub model: Option<String>,
    /// Current provider
    pub provider: Option<String>,
    /// Active threads
    pub threads: Vec<ThreadInfo>,
    /// Current foreground thread ID
    pub foreground_id: Option<u64>,
    /// Callback for new thread
    pub on_new_thread: EventHandler<()>,
    /// Callback for switching threads
    pub on_switch_thread: EventHandler<u64>,
    /// Callback for settings
    pub on_settings: EventHandler<()>,
}

/// Sidebar component.
#[component]
pub fn Sidebar(props: SidebarProps) -> Element {
    let connection_color = match &props.connection {
        ConnectionStatus::Disconnected => "has-text-grey",
        ConnectionStatus::Connecting => "has-text-warning",
        ConnectionStatus::Connected | ConnectionStatus::Authenticated => "has-text-success",
        ConnectionStatus::Authenticating => "has-text-info",
        ConnectionStatus::Error(_) => "has-text-danger",
    };

    let connection_icon = match &props.connection {
        ConnectionStatus::Disconnected => "fa-plug",
        ConnectionStatus::Connecting | ConnectionStatus::Authenticating => "fa-spinner fa-spin",
        ConnectionStatus::Connected | ConnectionStatus::Authenticated => "fa-check-circle",
        ConnectionStatus::Error(_) => "fa-exclamation-circle",
    };

    let connection_text = match &props.connection {
        ConnectionStatus::Disconnected => "Disconnected".to_string(),
        ConnectionStatus::Connecting => "Connecting...".to_string(),
        ConnectionStatus::Connected => "Connected".to_string(),
        ConnectionStatus::Authenticating => "Authenticating...".to_string(),
        ConnectionStatus::Authenticated => "Ready".to_string(),
        ConnectionStatus::Error(e) => format!("Error: {}", e),
    };

    rsx! {
        aside {
            class: "sidebar",
            style: "display: flex; flex-direction: column; padding: 1rem; height: 100%;",

            Menu {
                style: "display: flex; flex-direction: column; flex: 1; min-height: 0;",

                // Agent header
                div { class: "sidebar-header", style: "margin-bottom: 1rem;",
                    MenuLabel {
                        span { class: "icon-text",
                            Icon { i { class: "fas fa-robot" } }
                            span {
                                if let Some(name) = &props.agent_name {
                                    "{name}"
                                } else {
                                    "RustyClaw"
                                }
                            }
                        }
                    }

                    // Connection status
                    p { class: "is-size-7 {connection_color}",
                        Icon { class: "is-small".to_string(), i { class: "fas {connection_icon}" } }
                        " {connection_text}"
                    }

                    // Model info
                    if let Some(model) = &props.model {
                        p { class: "is-size-7 has-text-grey",
                            Icon { class: "is-small".to_string(), i { class: "fas fa-brain" } }
                            " {model}"
                        }
                    }
                }

                // Sessions section
                MenuLabel { "Sessions" }

                Button {
                    color: BulmaColor::Primary,
                    size: BulmaSize::Small,
                    fullwidth: true,
                    onclick: move |_| props.on_new_thread.call(()),
                    Icon { class: "is-small".to_string(), i { class: "fas fa-plus" } }
                    span { "New Session" }
                }

                MenuList {
                    style: "flex: 1; overflow-y: auto; margin-top: 0.5rem;",

                    for thread in props.threads.iter() {
                        // Use raw <li>/<a> here so we can attach an `is-active`
                        // class for the current thread; MenuItem doesn't expose
                        // a typed `active` prop in dioxus-bulma 0.7.3.
                        li { key: "{thread.id}",
                            a {
                                class: if props.foreground_id == Some(thread.id) { "is-active" } else { "" },
                                style: "display: flex; align-items: center;",
                                onclick: {
                                    let thread_id = thread.id;
                                    move |_| props.on_switch_thread.call(thread_id)
                                },

                                Icon { class: "is-small".to_string(), i { class: "fas fa-comments" } }
                                span { style: "margin-left: 0.25rem; flex: 1;",
                                    if let Some(label) = &thread.label {
                                        "{label}"
                                    } else {
                                        "Session #{thread.id}"
                                    }
                                }
                                Tag {
                                    size: BulmaSize::Small,
                                    rounded: true,
                                    style: "margin-left: auto;",
                                    "{thread.message_count}"
                                }
                            }
                        }
                    }
                }
            }

            // Footer with settings
            div { class: "sidebar-footer",
                style: "margin-top: auto; padding-top: 1rem; border-top: 1px solid #dbdbdb;",

                Button {
                    color: BulmaColor::Light,
                    size: BulmaSize::Small,
                    fullwidth: true,
                    onclick: move |_| props.on_settings.call(()),
                    Icon { class: "is-small".to_string(), i { class: "fas fa-cog" } }
                    span { "Settings" }
                }
            }
        }
    }
}
