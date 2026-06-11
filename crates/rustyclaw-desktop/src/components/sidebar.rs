//! Sidebar component: brand, connection chip, project→thread tree, footer.
//!
//! Rendered with Bulma `Menu`/`Tag` components; the project/thread rows
//! are custom rows inside the menu list because they carry inline rename
//! inputs and per-row actions.

use std::collections::HashSet;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Delete, Menu, MenuLabel, MenuList, Tag, Tags,
};

use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_view::{ProjectGroupData, SidebarItemData, SidebarTree, StatusBarData};

use super::tone_color;

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

            if !collapsed {
                StatusChips {
                    connection: props.connection.clone(),
                    model: props.model.clone(),
                    provider: props.provider.clone(),
                }
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
            Button {
                color: BulmaColor::Ghost,
                size: BulmaSize::Small,
                class: "sidebar-collapse-btn",
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
    let label = StatusBarData::connection_label_static(&props.connection);
    let tone = StatusBarData::connection_tone_static(&props.connection);
    let pulse = StatusBarData::connection_is_pending_static(&props.connection);
    let chip_class = if pulse { "rc-chip is-pulse" } else { "rc-chip" };
    let err = StatusBarData::connection_error_static(&props.connection).map(str::to_owned);

    rsx! {
        Tags { class: "sidebar-status",
            Tag {
                color: tone_color(tone),
                light: true,
                rounded: true,
                class: chip_class,
                span { class: "dot" }
                span { "{label}" }
            }
            if let Some(err) = err.as_ref() {
                span { title: "{err}",
                    Tag {
                        color: BulmaColor::Danger,
                        light: true,
                        rounded: true,
                        class: "rc-chip rc-chip-error",
                        "⚠ {err}"
                    }
                }
            }
            if let Some(model) = props.model.as_ref() {
                Tag {
                    rounded: true,
                    class: "rc-chip",
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

/// "New project" button and the scrollable list of project groups.
#[component]
fn ProjectsList(props: ProjectsListProps) -> Element {
    // Client-side collapse state for project groups (ids that are collapsed).
    let collapsed_projects = use_signal(HashSet::<u64>::new);

    rsx! {
        Menu { class: "projects-menu",
            MenuLabel { class: "sidebar-section-label", "Projects" }
            Button {
                color: BulmaColor::Primary,
                size: BulmaSize::Small,
                outlined: true,
                fullwidth: true,
                class: "sidebar-new-project",
                onclick: move |_| props.on_new_project.call(()),
                span { class: "icon-only", "＋" }
                if !props.collapsed { span { "New project" } }
            }

            MenuList { class: "projects-list",
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
    let header_class = if props.group.is_active {
        "project-header is-active"
    } else {
        "project-header"
    };
    let project_id = props.group.id;
    let name = props.group.name.clone();
    let path = props.group.path.clone();
    let count = props.group.threads.len();
    let chevron = if props.group_collapsed { "▸" } else { "▾" };

    let mut editing = use_signal(|| false);
    let mut edit_value = use_signal(String::new);
    let mut collapsed_projects = props.collapsed_projects;

    rsx! {
        li { class: "project-group",
            div {
                class: "{header_class}",
                title: "{path}",
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
                    title: "Expand/collapse",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        let mut set = collapsed_projects.write();
                        if !set.remove(&project_id) {
                            set.insert(project_id);
                        }
                    },
                    "{chevron}"
                }
                if !props.sidebar_collapsed {
                    if *editing.read() {
                        input {
                            class: "input is-small session-rename-input",
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
                        span { class: "project-name", "{name}" }
                        Tag {
                            size: BulmaSize::Small,
                            rounded: true,
                            class: "project-count",
                            "{count}"
                        }
                        button {
                            class: "project-add-btn",
                            title: "New thread in this project",
                            onclick: move |evt| {
                                evt.stop_propagation();
                                props.on_new_thread_in.call(project_id);
                            },
                            "＋"
                        }
                        Delete {
                            size: BulmaSize::Small,
                            class: "project-delete-btn",
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                props.on_delete_project.call(project_id);
                            },
                        }
                    }
                }
            }

            if !props.group_collapsed {
                ul { class: "sessions-list is-nested",
                    if props.group.threads.is_empty() {
                        if !props.sidebar_collapsed {
                            li { class: "sidebar-empty-sessions",
                                "No threads yet."
                            }
                        }
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
                span { class: "session-icon", "💬" }
                if !props.collapsed {
                    if *editing.read() {
                        input {
                            class: "input is-small session-rename-input",
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
                            Tag {
                                size: BulmaSize::Small,
                                rounded: true,
                                class: "session-count",
                                "{count}"
                            }
                        }
                        Delete {
                            size: BulmaSize::Small,
                            class: "session-delete-btn",
                            onclick: move |evt: MouseEvent| {
                                evt.stop_propagation();
                                props.on_delete.call(());
                            },
                        }
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
            Button {
                color: BulmaColor::Ghost,
                size: BulmaSize::Small,
                fullwidth: true,
                class: "sidebar-action",
                onclick: move |_| props.on_pair.call(()),
                span { class: "icon-only", "🔗" }
                if !props.collapsed { span { "Pair gateway" } }
            }
            Button {
                color: BulmaColor::Ghost,
                size: BulmaSize::Small,
                fullwidth: true,
                class: "sidebar-action",
                onclick: move |_| props.on_secrets.call(()),
                span { class: "icon-only", "🔑" }
                if !props.collapsed { span { "Secrets" } }
            }
            Button {
                color: BulmaColor::Ghost,
                size: BulmaSize::Small,
                fullwidth: true,
                class: "sidebar-action",
                onclick: move |_| props.on_settings.call(()),
                span { class: "icon-only", "⚙" }
                if !props.collapsed { span { "Settings" } }
            }
        }
    }
}
