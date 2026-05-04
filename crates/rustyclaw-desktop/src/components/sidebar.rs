//! Sidebar component: brand, connection chip, sessions list, footer actions.

use dioxus::prelude::*;

use crate::state::{ConnectionStatus, ThreadInfo};

/// Props for [`Sidebar`].
#[derive(Props, Clone, PartialEq)]
pub struct SidebarProps {
    pub connection: ConnectionStatus,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub threads: Vec<ThreadInfo>,
    pub foreground_id: Option<u64>,
    pub collapsed: bool,
    pub on_toggle_collapse: EventHandler<()>,
    pub on_new_thread: EventHandler<()>,
    pub on_switch_thread: EventHandler<u64>,
    pub on_pair: EventHandler<()>,
    pub on_settings: EventHandler<()>,
}

/// Status chip describing the connection to the gateway.
#[derive(Clone, Copy)]
struct ConnInfo {
    label: &'static str,
    cls: &'static str,
}

fn classify(status: &ConnectionStatus) -> (ConnInfo, Option<String>) {
    match status {
        ConnectionStatus::Disconnected => (
            ConnInfo {
                label: "Disconnected",
                cls: "is-warn",
            },
            None,
        ),
        ConnectionStatus::Connecting => (
            ConnInfo {
                label: "Connecting…",
                cls: "is-info is-pulse",
            },
            None,
        ),
        ConnectionStatus::Connected => (
            ConnInfo {
                label: "Connected",
                cls: "is-success",
            },
            None,
        ),
        ConnectionStatus::Authenticating => (
            ConnInfo {
                label: "Authenticating…",
                cls: "is-info is-pulse",
            },
            None,
        ),
        ConnectionStatus::Authenticated => (
            ConnInfo {
                label: "Ready",
                cls: "is-success",
            },
            None,
        ),
        ConnectionStatus::Error(e) => (
            ConnInfo {
                label: "Error",
                cls: "is-danger",
            },
            Some(e.clone()),
        ),
    }
}

/// Sidebar component.
#[component]
pub fn Sidebar(props: SidebarProps) -> Element {
    let collapsed = props.collapsed;
    let aside_class = if collapsed {
        "sidebar is-collapsed"
    } else {
        "sidebar"
    };

    let (conn_info, conn_err) = classify(&props.connection);
    let conn_chip_class = format!("chip {}", conn_info.cls);

    rsx! {
        aside { class: "{aside_class}",
            // Brand row
            div { class: "sidebar-brand",
                span { class: "brand-mark", "🦞" }
                if !collapsed {
                    div { class: "brand-text",
                        span { class: "brand-name",
                            if let Some(name) = props.agent_name.as_ref() {
                                "{name}"
                            } else {
                                "RustyClaw"
                            }
                        }
                        span { class: "brand-sub",
                            if props.agent_name.is_some() {
                                "RustyClaw Agent"
                            } else {
                                "Agent OS"
                            }
                        }
                    }
                }
                button {
                    class: "sidebar-collapse-btn",
                    title: if collapsed { "Expand sidebar" } else { "Collapse sidebar" },
                    onclick: move |_| props.on_toggle_collapse.call(()),
                    if collapsed {
                        "›"
                    } else {
                        "‹"
                    }
                }
            }

            // Connection / model chips
            if !collapsed {
                div { class: "sidebar-status",
                    span { class: "{conn_chip_class}",
                        span { class: "dot" }
                        span { "{conn_info.label}" }
                    }
                    if let Some(err) = conn_err.as_ref() {
                        span {
                            class: "chip is-danger",
                            title: "{err}",
                            "⚠ {err}"
                        }
                    }
                    if let Some(model) = props.model.as_ref() {
                        span { class: "chip",
                            "🧠 ",
                            if let Some(provider) = props.provider.as_ref() {
                                "{provider} · {model}"
                            } else {
                                "{model}"
                            }
                        }
                    }
                }
            }

            // Sessions
            div { class: "sidebar-section-label", "Sessions" }
            button {
                class: "sidebar-action is-primary",
                onclick: move |_| props.on_new_thread.call(()),
                title: "New session",
                span { class: "icon-only", "＋" }
                if !collapsed { span { "New session" } }
            }

            div { class: "sessions-list",
                if props.threads.is_empty() {
                    if !collapsed {
                        div { class: "sidebar-empty-sessions",
                            "No sessions yet — start one to begin chatting."
                        }
                    }
                } else {
                    for thread in props.threads.iter() {
                        SessionRow {
                            key: "{thread.id}",
                            thread: thread.clone(),
                            active: props.foreground_id == Some(thread.id),
                            collapsed: collapsed,
                            on_click: {
                                let id = thread.id;
                                let cb = props.on_switch_thread;
                                move |_| cb.call(id)
                            }
                        }
                    }
                }
            }

            // Footer actions
            div { class: "sidebar-footer",
                button {
                    class: "sidebar-action",
                    onclick: move |_| props.on_pair.call(()),
                    title: "Pair with gateway",
                    span { class: "icon-only", "🔗" }
                    if !collapsed { span { "Pair gateway" } }
                }
                button {
                    class: "sidebar-action",
                    onclick: move |_| props.on_settings.call(()),
                    title: "Settings",
                    span { class: "icon-only", "⚙" }
                    if !collapsed { span { "Settings" } }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SessionRowProps {
    thread: ThreadInfo,
    active: bool,
    collapsed: bool,
    on_click: EventHandler<()>,
}

#[component]
fn SessionRow(props: SessionRowProps) -> Element {
    let class = if props.active {
        "session-row is-active"
    } else {
        "session-row"
    };
    let label = props
        .thread
        .label
        .clone()
        .unwrap_or_else(|| format!("Session #{}", props.thread.id));
    let count = props.thread.message_count;

    rsx! {
        div {
            class: "{class}",
            title: "{label}",
            onclick: move |_| props.on_click.call(()),
            span { class: "session-icon", "💬" }
            if !props.collapsed {
                span { class: "session-label", "{label}" }
                if count > 0 {
                    span { class: "session-count", "{count}" }
                }
            }
        }
    }
}
