//! Sidebar component: brand, connection chip, footer actions.
//!
//! Thread switching moved to the top tab bar.  The sidebar now
//! serves as a compact brand + status + action panel.

use dioxus::prelude::*;

use rustyclaw_core::ui::ConnectionStatus;

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
    pub on_pair: EventHandler<()>,
    pub on_secrets: EventHandler<()>,
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
