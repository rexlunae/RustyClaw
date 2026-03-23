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
            class: "menu sidebar",
            style: "width: 250px; padding: 1rem; background: #f5f5f5; border-right: 1px solid #dbdbdb; height: 100%; display: flex; flex-direction: column;",
            
            // Agent header
            div { class: "sidebar-header",
                style: "margin-bottom: 1rem;",
                
                p { class: "menu-label",
                    span { class: "icon-text",
                        span { class: "icon",
                            i { class: "fas fa-robot" }
                        }
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
                    span { class: "icon is-small",
                        i { class: "fas {connection_icon}" }
                    }
                    " {connection_text}"
                }
                
                // Model info
                if let Some(model) = &props.model {
                    p { class: "is-size-7 has-text-grey",
                        span { class: "icon is-small",
                            i { class: "fas fa-brain" }
                        }
                        " {model}"
                    }
                }
            }
            
            // Threads/Sessions
            p { class: "menu-label", "Sessions" }
            
            Button {
                color: Color::Primary,
                size: Size::Small,
                fullwidth: true,
                onclick: move |_| props.on_new_thread.call(()),
                
                span { class: "icon is-small",
                    i { class: "fas fa-plus" }
                }
                span { "New Session" }
            }
            
            ul { class: "menu-list",
                style: "flex: 1; overflow-y: auto; margin-top: 0.5rem;",
                
                for thread in props.threads.iter() {
                    li { key: "{thread.id}",
                        a { 
                            class: if props.foreground_id == Some(thread.id) { "is-active" } else { "" },
                            onclick: {
                                let thread_id = thread.id;
                                move |_| props.on_switch_thread.call(thread_id)
                            },
                            
                            span { class: "icon is-small",
                                i { class: "fas fa-comments" }
                            }
                            span {
                                if let Some(label) = &thread.label {
                                    "{label}"
                                } else {
                                    "Session #{thread.id}"
                                }
                            }
                            span { class: "tag is-small is-rounded",
                                style: "margin-left: auto;",
                                "{thread.message_count}"
                            }
                        }
                    }
                }
            }
            
            // Footer with settings
            div { class: "sidebar-footer",
                style: "margin-top: auto; padding-top: 1rem; border-top: 1px solid #dbdbdb;",
                
                Button {
                    color: Color::Light,
                    size: Size::Small,
                    fullwidth: true,
                    onclick: move |_| props.on_settings.call(()),
                    
                    span { class: "icon is-small",
                        i { class: "fas fa-cog" }
                    }
                    span { "Settings" }
                }
            }
        }
    }
}
