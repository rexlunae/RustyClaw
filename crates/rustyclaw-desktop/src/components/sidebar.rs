//! Sidebar component: brand, connection chip, sessions list, footer actions.
//!
//! Each visual section is a dedicated Dioxus component for clarity and
//! reusability:
//!   - [`BrandHeader`] — logo, agent name, collapse toggle
//!   - [`StatusChips`] — connection and model indicators
//!   - [`SessionsList`] — scrollable thread list
//!   - [`SessionRow`] — individual thread entry with label + description
//!   - [`FooterActions`] — pair / settings buttons

use dioxus::prelude::*;

use rustyclaw_core::ui::{ConnectionStatus, ThreadInfo};

// ── Public top-level component ──────────────────────────────────────────────

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

/// Sidebar component.
#[component]
pub fn Sidebar(props: SidebarProps) -> Element {
    let collapsed = props.collapsed;
    let aside_class = if collapsed {
        "sidebar is-collapsed"
    } else {
        "sidebar"
    };

    rsx! {
        aside { class: "{aside_class}",
            BrandHeader {
                agent_name: props.agent_name.clone(),
                collapsed: collapsed,
                on_toggle: props.on_toggle_collapse,
            }

            if !collapsed {
                StatusChips {
                    connection: props.connection.clone(),
                    model: props.model.clone(),
                    provider: props.provider.clone(),
                }
            }

            SessionsList {
                threads: props.threads.clone(),
                foreground_id: props.foreground_id,
                collapsed: collapsed,
                on_new_thread: props.on_new_thread,
                on_switch_thread: props.on_switch_thread,
            }

            FooterActions {
                collapsed: collapsed,
                on_pair: props.on_pair,
                on_settings: props.on_settings,
            }
        }
    }
}

// ── Brand header ────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct BrandHeaderProps {
    agent_name: Option<String>,
    collapsed: bool,
    on_toggle: EventHandler<()>,
}

/// Logo, agent name, and collapse toggle.
#[component]
fn BrandHeader(props: BrandHeaderProps) -> Element {
    rsx! {
        div { class: "sidebar-brand",
            span { class: "brand-mark", "🦞" }
            if !props.collapsed {
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
                title: if props.collapsed { "Expand sidebar" } else { "Collapse sidebar" },
                onclick: move |_| props.on_toggle.call(()),
                if props.collapsed { "›" } else { "‹" }
            }
        }
    }
}

// ── Status chips ────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct StatusChipsProps {
    connection: ConnectionStatus,
    model: Option<String>,
    provider: Option<String>,
}

/// Connection state and active model indicators.
#[component]
fn StatusChips(props: StatusChipsProps) -> Element {
    let (conn_info, conn_err) = classify(&props.connection);
    let conn_chip_class = format!("chip {}", conn_info.cls);

    rsx! {
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
}

/// Connection status classification for chip rendering.
#[derive(Clone, Copy)]
struct ConnInfo {
    label: &'static str,
    cls: &'static str,
}

fn classify(status: &ConnectionStatus) -> (ConnInfo, Option<String>) {
    match status {
        ConnectionStatus::Disconnected => (
            ConnInfo { label: "Disconnected", cls: "is-warn" },
            None,
        ),
        ConnectionStatus::Connecting => (
            ConnInfo { label: "Connecting…", cls: "is-info is-pulse" },
            None,
        ),
        ConnectionStatus::Connected => (
            ConnInfo { label: "Connected", cls: "is-success" },
            None,
        ),
        ConnectionStatus::Authenticating => (
            ConnInfo { label: "Authenticating…", cls: "is-info is-pulse" },
            None,
        ),
        ConnectionStatus::Authenticated => (
            ConnInfo { label: "Ready", cls: "is-success" },
            None,
        ),
        ConnectionStatus::Error(e) => (
            ConnInfo { label: "Error", cls: "is-danger" },
            Some(e.clone()),
        ),
    }
}

// ── Sessions list ───────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct SessionsListProps {
    threads: Vec<ThreadInfo>,
    foreground_id: Option<u64>,
    collapsed: bool,
    on_new_thread: EventHandler<()>,
    on_switch_thread: EventHandler<u64>,
}

/// "New session" button and scrollable thread list.
#[component]
fn SessionsList(props: SessionsListProps) -> Element {
    rsx! {
        div { class: "sidebar-section-label", "Sessions" }
        button {
            class: "sidebar-action is-primary",
            onclick: move |_| props.on_new_thread.call(()),
            title: "New session",
            span { class: "icon-only", "＋" }
            if !props.collapsed { span { "New session" } }
        }

        div { class: "sessions-list",
            if props.threads.is_empty() {
                if !props.collapsed {
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
                        collapsed: props.collapsed,
                        on_click: {
                            let id = thread.id;
                            let cb = props.on_switch_thread;
                            move |_| cb.call(id)
                        }
                    }
                }
            }
        }
    }
}

// ── Session row ─────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct SessionRowProps {
    thread: ThreadInfo,
    active: bool,
    collapsed: bool,
    on_click: EventHandler<()>,
}

/// A single thread entry: icon, label, optional description, message count.
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
    let description = props.thread.description.clone();
    let title_text = if let Some(desc) = description.as_deref() {
        format!("{label}\n{desc}")
    } else {
        label.clone()
    };

    rsx! {
        div {
            class: "{class}",
            title: "{title_text}",
            onclick: move |_| props.on_click.call(()),
            span { class: "session-icon", "💬" }
            if !props.collapsed {
                div { class: "session-text",
                    span { class: "session-label", "{label}" }
                    if let Some(desc) = description.as_deref() {
                        span { class: "session-description", "{desc}" }
                    }
                }
                if count > 0 {
                    span { class: "session-count", "{count}" }
                }
            }
        }
    }
}

// ── Footer actions ──────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct FooterActionsProps {
    collapsed: bool,
    on_pair: EventHandler<()>,
    on_settings: EventHandler<()>,
}

/// Pair and Settings buttons at the bottom of the sidebar.
#[component]
fn FooterActions(props: FooterActionsProps) -> Element {
    rsx! {
        div { class: "sidebar-footer",
            button {
                class: "sidebar-action",
                onclick: move |_| props.on_pair.call(()),
                title: "Pair with gateway",
                span { class: "icon-only", "🔗" }
                if !props.collapsed { span { "Pair gateway" } }
            }
            button {
                class: "sidebar-action",
                onclick: move |_| props.on_settings.call(()),
                title: "Settings",
                span { class: "icon-only", "⚙" }
                if !props.collapsed { span { "Settings" } }
            }
        }
    }
}
