//! Sidebar component: brand, connection status, project→thread tree, footer.
//!
//! Rendered with bespoke markup rather than Bulma widgets: every row here
//! (project header, thread, footer action) shares one geometry and one set
//! of hover/active treatments, which Bulma's button/tag/menu defaults kept
//! pulling apart. See the "Sidebar" section of `assets/styles.css` — the
//! `--rc-row-*` tokens defined there are the single source of truth for row
//! height, padding, and radius.

use std::collections::HashSet;

use dioxus::prelude::*;

use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_view::{ProjectGroupData, SidebarItemData, SidebarTree, StatusBarData};

use super::tone_modifier;

/// Character budget for the inline project path. Sized to the expanded
/// sidebar width; the full path remains available as the row tooltip.
const PROJECT_PATH_MAX_CHARS: usize = 34;

/// The user's home directory, for collapsing project paths to `~`.
fn home_dir() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|h| !h.is_empty())
}

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
    // Thread actions.
    pub on_switch_thread: EventHandler<u64>,
    pub on_rename_thread: EventHandler<(u64, String)>,
    pub on_delete_thread: EventHandler<u64>,
    /// Create a new thread in the given project.
    pub on_new_thread_in: EventHandler<u64>,
    // Project actions.
    pub on_new_project: EventHandler<()>,
    pub on_switch_project: EventHandler<u64>,
    pub on_rename_project: EventHandler<(u64, String)>,
    pub on_delete_project: EventHandler<u64>,
    // Footer.
    pub on_pair: EventHandler<()>,
    pub on_secrets: EventHandler<()>,
    pub on_settings: EventHandler<()>,
    pub on_local_models: EventHandler<()>,
    /// The full project → thread tree.
    pub tree: SidebarTree,
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

            StatusChips {
                connection: props.connection.clone(),
                model: props.model.clone(),
                provider: props.provider.clone(),
                collapsed: collapsed,
            }

            ProjectsList {
                tree: props.tree.clone(),
                foreground_id: props.foreground_id,
                collapsed: collapsed,
                on_new_thread_in: props.on_new_thread_in,
                on_switch_thread: props.on_switch_thread,
                on_rename_thread: props.on_rename_thread,
                on_delete_thread: props.on_delete_thread,
                on_new_project: props.on_new_project,
                on_switch_project: props.on_switch_project,
                on_rename_project: props.on_rename_project,
                on_delete_project: props.on_delete_project,
            }

            FooterActions {
                collapsed: collapsed,
                on_pair: props.on_pair,
                on_secrets: props.on_secrets,
                on_settings: props.on_settings,
                on_local_models: props.on_local_models,
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
    let toggle_title = if props.collapsed {
        "Expand sidebar"
    } else {
        "Collapse sidebar"
    };

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
                title: "{toggle_title}",
                "aria-label": "{toggle_title}",
                onclick: move |_| props.on_toggle.call(()),
                span { class: "collapse-caret", "‹" }
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
    collapsed: bool,
}

/// Connection state and active model indicators.
///
/// Renders as a compact status block: a connection line (tone-coloured dot +
/// label, pulsing while pending) and a model line. Collapsed, it shrinks to a
/// single centred dot so connection state is never hidden.
#[component]
fn StatusChips(props: StatusChipsProps) -> Element {
    let label = StatusBarData::connection_label_static(&props.connection);
    let tone = StatusBarData::connection_tone_static(&props.connection);
    let pulse = StatusBarData::connection_is_pending_static(&props.connection);
    let err = StatusBarData::connection_error_static(&props.connection).map(str::to_owned);
    let tone_class = tone_modifier(tone);

    let dot_class = if pulse {
        format!("status-dot {tone_class} is-pulse")
    } else {
        format!("status-dot {tone_class}")
    };

    // Collapsed: a single dot, with the full state in the tooltip.
    if props.collapsed {
        let tip = match err.as_ref() {
            Some(e) => format!("{label} — {e}"),
            None => label.to_string(),
        };
        return rsx! {
            div { class: "sidebar-status is-collapsed", title: "{tip}",
                span { class: "{dot_class}" }
            }
        };
    }

    rsx! {
        div { class: "sidebar-status",
            div { class: "status-line", title: "{label}",
                span { class: "{dot_class}" }
                span { class: "status-label", "{label}" }
            }
            if let Some(err) = err.as_ref() {
                div { class: "status-line is-error", title: "{err}",
                    span { class: "status-glyph", "⚠" }
                    span { class: "status-label", "{err}" }
                }
            }
            if let Some(model) = props.model.as_ref() {
                div {
                    class: "status-line is-model",
                    title: match props.provider.as_ref() {
                        Some(p) => format!("{p} · {model}"),
                        None => model.clone(),
                    },
                    span { class: "status-glyph", "◈" }
                    span { class: "status-label",
                        if let Some(provider) = props.provider.as_ref() {
                            span { class: "status-provider", "{provider}" }
                            span { class: "status-sep", "·" }
                        }
                        "{model}"
                    }
                }
            }
        }
    }
}

// ── Projects list (two-level: projects → threads) ───────────────────────────

#[derive(Props, Clone, PartialEq)]
struct ProjectsListProps {
    tree: SidebarTree,
    foreground_id: Option<u64>,
    collapsed: bool,
    on_new_thread_in: EventHandler<u64>,
    on_switch_thread: EventHandler<u64>,
    on_rename_thread: EventHandler<(u64, String)>,
    on_delete_thread: EventHandler<u64>,
    on_new_project: EventHandler<()>,
    on_switch_project: EventHandler<u64>,
    on_rename_project: EventHandler<(u64, String)>,
    on_delete_project: EventHandler<u64>,
}

/// "New project" affordance and the scrollable list of project groups.
#[component]
fn ProjectsList(props: ProjectsListProps) -> Element {
    // Client-side collapse state for project groups (ids that are collapsed).
    let collapsed_projects = use_signal(HashSet::<u64>::new);
    let project_count = props.tree.groups.len();
    let is_empty = props.tree.groups.is_empty();

    rsx! {
        nav { class: "projects-menu",
            // Section header doubles as the "new project" affordance so the
            // list starts higher up and the control never scrolls away.
            div { class: "sidebar-section",
                if !props.collapsed {
                    span { class: "sidebar-section-label", "Projects" }
                    if project_count > 0 {
                        span { class: "sidebar-section-count", "{project_count}" }
                    }
                }
                button {
                    class: "sidebar-section-add",
                    title: "New project",
                    "aria-label": "New project",
                    onclick: move |_| props.on_new_project.call(()),
                    "＋"
                }
            }

            ul { class: "projects-list",
                for group in props.tree.groups.iter() {
                    ProjectGroup {
                        key: "{group.id}",
                        group: group.clone(),
                        foreground_id: props.foreground_id,
                        sidebar_collapsed: props.collapsed,
                        group_collapsed: collapsed_projects.read().contains(&group.id),
                        collapsed_projects: collapsed_projects,
                        on_new_thread_in: props.on_new_thread_in,
                        on_switch_thread: props.on_switch_thread,
                        on_rename_thread: props.on_rename_thread,
                        on_delete_thread: props.on_delete_thread,
                        on_switch_project: props.on_switch_project,
                        on_rename_project: props.on_rename_project,
                        on_delete_project: props.on_delete_project,
                    }
                }
                if is_empty && !props.collapsed {
                    li { class: "sidebar-empty-state",
                        span { class: "empty-glyph", "◇" }
                        span { class: "empty-title", "No projects yet" }
                        span { class: "empty-hint", "Use ＋ above to create one." }
                    }
                }
            }
        }
    }
}

// ── Project group (header + nested threads) ─────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct ProjectGroupProps {
    group: ProjectGroupData,
    foreground_id: Option<u64>,
    sidebar_collapsed: bool,
    group_collapsed: bool,
    collapsed_projects: Signal<HashSet<u64>>,
    on_new_thread_in: EventHandler<u64>,
    on_switch_thread: EventHandler<u64>,
    on_rename_thread: EventHandler<(u64, String)>,
    on_delete_thread: EventHandler<u64>,
    on_switch_project: EventHandler<u64>,
    on_rename_project: EventHandler<(u64, String)>,
    on_delete_project: EventHandler<u64>,
}

#[component]
fn ProjectGroup(props: ProjectGroupProps) -> Element {
    let project_id = props.group.id;
    // `display_name` never yields an empty string, so a blank or not-yet-known
    // project name still renders a readable label.
    let name = props.group.display_name().into_owned();
    let full_path = props.group.path.clone();
    // Shown inline under the name: `~`-collapsed and budgeted to the sidebar
    // width. The untruncated path stays in the row tooltip.
    let path = props
        .group
        .pretty_path(home_dir().as_deref(), PROJECT_PATH_MAX_CHARS)
        .into_owned();
    let count = props.group.threads.len();

    let mut header_class = String::from("project-header");
    if props.group.is_active {
        header_class.push_str(" is-active");
    }
    if props.group_collapsed {
        header_class.push_str(" is-folded");
    }

    // Tooltip carries the full, untruncated path.
    let header_title = if full_path.is_empty() {
        name.clone()
    } else {
        format!("{name}\n{full_path}")
    };

    let mut editing = use_signal(|| false);
    let mut edit_value = use_signal(String::new);
    let mut collapsed_projects = props.collapsed_projects;

    rsx! {
        li { class: "project-group",
            div {
                class: "{header_class}",
                title: "{header_title}",
                onclick: move |_| {
                    if !*editing.read() {
                        props.on_switch_project.call(project_id);
                    }
                },
                ondoubleclick: {
                    let name = name.clone();
                    move |evt| {
                        if !props.sidebar_collapsed {
                            evt.stop_propagation();
                            edit_value.set(name.clone());
                            editing.set(true);
                        }
                    }
                },
                button {
                    class: "project-chevron",
                    title: if props.group_collapsed { "Expand" } else { "Collapse" },
                    "aria-label": "Expand or collapse project",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        let mut set = collapsed_projects.write();
                        if !set.remove(&project_id) {
                            set.insert(project_id);
                        }
                    },
                    "▾"
                }
                if props.sidebar_collapsed {
                    // Collapsed rail: initial stands in for the name.
                    span { class: "project-initial",
                        "{name.chars().next().unwrap_or('#').to_uppercase()}"
                    }
                } else if *editing.read() {
                    input {
                        class: "input is-small rename-input",
                        r#type: "text",
                        value: "{edit_value}",
                        autofocus: true,
                        oninput: move |evt| edit_value.set(evt.value()),
                        onkeydown: {
                            let on_rename = props.on_rename_project;
                            move |evt: KeyboardEvent| {
                                if evt.key() == Key::Enter {
                                    let val = edit_value.read().trim().to_string();
                                    if !val.is_empty() {
                                        on_rename.call((project_id, val));
                                    }
                                    editing.set(false);
                                } else if evt.key() == Key::Escape {
                                    editing.set(false);
                                }
                            }
                        },
                        onfocusout: move |_| editing.set(false),
                    }
                } else {
                    div { class: "project-text",
                        span { class: "project-name", "{name}" }
                        if !path.is_empty() {
                            span { class: "project-path", "{path}" }
                        }
                    }
                    div { class: "row-actions",
                        button {
                            class: "row-action",
                            title: "New thread in this project",
                            "aria-label": "New thread in this project",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                props.on_new_thread_in.call(project_id);
                            },
                            "＋"
                        }
                        button {
                            class: "row-action is-danger",
                            title: "Delete project",
                            "aria-label": "Delete project",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                props.on_delete_project.call(project_id);
                            },
                            "✕"
                        }
                    }
                    span { class: "count-badge", "{count}" }
                }
            }

            // Collapsed to a rail, thread rows would be a column of anonymous
            // dots — the rail shows project initials only.
            if !props.group_collapsed && !props.sidebar_collapsed {
                ul { class: "sessions-list",
                    if props.group.threads.is_empty() {
                        li { class: "sidebar-empty-sessions", "No threads yet" }
                    } else {
                        for thread in props.group.threads.iter() {
                            SessionRow {
                                key: "{thread.id}",
                                thread: thread.clone(),
                                active: props.foreground_id == Some(thread.id),
                                collapsed: props.sidebar_collapsed,
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
        li {
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
                span { class: "session-dot" }
                if props.collapsed {
                    // Nothing else fits in the rail; the tooltip carries the
                    // label and the dot still marks the active thread.
                } else if *editing.read() {
                    input {
                        class: "input is-small rename-input",
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
                    div { class: "row-actions",
                        button {
                            class: "row-action is-danger",
                            title: "Delete thread",
                            "aria-label": "Delete thread",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                props.on_delete.call(());
                            },
                            "✕"
                        }
                    }
                    if count > 0 {
                        span { class: "count-badge", "{count}" }
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
    on_local_models: EventHandler<()>,
}

/// Pair, Secrets, Local models, and Settings buttons at the sidebar footer.
#[component]
fn FooterActions(props: FooterActionsProps) -> Element {
    let collapsed = props.collapsed;

    rsx! {
        div { class: "sidebar-footer",
            SidebarAction {
                icon: "🔗",
                label: "Pair gateway",
                collapsed: collapsed,
                on_click: props.on_pair,
            }
            SidebarAction {
                icon: "🔑",
                label: "Secrets",
                collapsed: collapsed,
                on_click: props.on_secrets,
            }
            SidebarAction {
                icon: "🧠",
                label: "Local models",
                collapsed: collapsed,
                on_click: props.on_local_models,
            }
            SidebarAction {
                icon: "⚙",
                label: "Settings",
                collapsed: collapsed,
                on_click: props.on_settings,
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SidebarActionProps {
    icon: &'static str,
    label: &'static str,
    collapsed: bool,
    on_click: EventHandler<()>,
}

/// One footer action row. Shares the row geometry of the project/thread rows
/// so the whole sidebar reads as a single system.
#[component]
fn SidebarAction(props: SidebarActionProps) -> Element {
    rsx! {
        button {
            class: "sidebar-action",
            title: "{props.label}",
            "aria-label": "{props.label}",
            onclick: move |_| props.on_click.call(()),
            span { class: "action-icon", "{props.icon}" }
            if !props.collapsed {
                span { class: "action-label", "{props.label}" }
            }
        }
    }
}
