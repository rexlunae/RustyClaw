//! Sidebar component: brand, connection chip, session list, footer actions.
//!
//! The sidebar shows brand info, connection status, the list of chat sessions
//! with rename/delete UI, and footer action buttons.

use dioxus::prelude::*;

use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_view::SidebarItemData;

// ── Public top-level component ──────────────────────────────────────────────

/// Props for [`Sidebar`].
#[derive(Props, Clone, PartialEq)]
pub struct SidebarProps {
    pub connection: ConnectionStatus,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub collapsed: bool,
    pub on_toggle_collapse: EventHandler<()>,
    pub on_new_thread: EventHandler<()>,
    pub on_switch_thread: EventHandler<u64>,
    pub on_rename_thread: EventHandler<(u64, String)>,
    pub on_delete_thread: EventHandler<u64>,
    pub on_pair: EventHandler<()>,
    pub on_secrets: EventHandler<()>,
    pub on_settings: EventHandler<()>,
    pub threads: Vec<SidebarItemData>,
    pub foreground_id: Option<u64>,
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
                on_rename_thread: props.on_rename_thread,
                on_delete_thread: props.on_delete_thread,
            }

            FooterActions {
                collapsed: collapsed,
                on_pair: props.on_pair,
                on_secrets: props.on_secrets,
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
    let label = rustyclaw_view::StatusBarData::connection_label_static(&props.connection);
    let base_class = rustyclaw_view::StatusBarData::connection_class_static(&props.connection);
    let pulse = matches!(
        &props.connection,
        ConnectionStatus::Connecting | ConnectionStatus::Authenticating
    );
    let cls = if pulse {
        format!("chip {} is-pulse", base_class)
    } else {
        format!("chip {}", base_class)
    };
    let err = match &props.connection {
        ConnectionStatus::Error(e) => Some(e.clone()),
        _ => None,
    };

    rsx! {
        div { class: "sidebar-status",
            span { class: "{cls}",
                span { class: "dot" }
                span { "{label}" }
            }
            if let Some(err) = err.as_ref() {
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

// ── Sessions list ───────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct SessionsListProps {
    threads: Vec<SidebarItemData>,
    foreground_id: Option<u64>,
    collapsed: bool,
    on_new_thread: EventHandler<()>,
    on_switch_thread: EventHandler<u64>,
    on_rename_thread: EventHandler<(u64, String)>,
    on_delete_thread: EventHandler<u64>,
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
                        },
                        on_rename: props.on_rename_thread,
                        on_delete: {
                            let id = thread.id;
                            let cb = props.on_delete_thread;
                            move |_| cb.call(id)
                        },
                    }
                }
            }
        }
    }
}

// ── Session row ─────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct SessionRowProps {
    thread: SidebarItemData,
    active: bool,
    collapsed: bool,
    on_click: EventHandler<()>,
    on_rename: EventHandler<(u64, String)>,
    on_delete: EventHandler<()>,
}

/// A single thread entry: icon, label, optional description, message count,
/// with double-click-to-rename and a delete button on hover.
#[component]
fn SessionRow(props: SessionRowProps) -> Element {
    let class = if props.active {
        "session-row is-active"
    } else {
        "session-row"
    };
    let label = props.thread.display_label().into_owned();
    let count = props.thread.message_count;
    let description = props.thread.description.clone();
    let title_text = props.thread.title_text();

    let mut editing = use_signal(|| false);
    let mut edit_value = use_signal(String::new);
    let thread_id = props.thread.id;

    rsx! {
        div {
            class: "{class}",
            title: "{title_text}",
            onclick: move |_| {
                if !*editing.read() {
                    props.on_click.call(());
                }
            },
            ondoubleclick: move |evt| {
                if !props.collapsed {
                    evt.stop_propagation();
                    edit_value.set(label.clone());
                    editing.set(true);
                }
            },
            span { class: "session-icon", "💬" }
            if !props.collapsed {
                if *editing.read() {
                    input {
                        class: "session-rename-input",
                        r#type: "text",
                        value: "{edit_value}",
                        autofocus: true,
                        oninput: move |evt| {
                            edit_value.set(evt.value());
                        },
                        onkeydown: {
                            let on_rename = props.on_rename;
                            move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter {
                                    let val = edit_value.read().trim().to_string();
                                    if !val.is_empty() {
                                        on_rename.call((thread_id, val));
                                    }
                                    editing.set(false);
                                } else if evt.key() == Key::Escape {
                                    editing.set(false);
                                }
                            }
                        },
                        onfocusout: {
                            let on_rename = props.on_rename;
                            move |_| {
                                let val = edit_value.read().trim().to_string();
                                if !val.is_empty() {
                                    on_rename.call((thread_id, val));
                                }
                                editing.set(false);
                            }
                        },
                    }
                } else {
                    div { class: "session-text",
                        span { class: "session-label", "{label}" }
                        if let Some(desc) = description.as_deref() {
                            span { class: "session-description", "{desc}" }
                        }
                    }
                    if count > 0 {
                        span { class: "session-count", "{count}" }
                    }
                    button {
                        class: "session-delete-btn",
                        title: "Delete thread",
                        onclick: move |evt| {
                            evt.stop_propagation();
                            props.on_delete.call(());
                        },
                        "✕"
                    }
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
    on_secrets: EventHandler<()>,
    on_settings: EventHandler<()>,
}

/// Pair, Secrets, and Settings buttons at the bottom of the sidebar.
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
                onclick: move |_| props.on_secrets.call(()),
                title: "Secrets Vault",
                span { class: "icon-only", "🔑" }
                if !props.collapsed { span { "Secrets" } }
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
