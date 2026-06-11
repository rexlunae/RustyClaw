//! Swarm management panel for the desktop UI.
//!
//! Displays active swarms, their agents and communication flows, and provides
//! controls to create, inspect, and stop swarms.  Each swarm renders as a
//! Bulma `Card` with `Tag` chips for status and metadata.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Card, CardContent, CardFooter, CardHeader,
    CardHeaderTitle, Tag, Tags,
};
use rustyclaw_view::SwarmData;

use super::{RcModal, tone_color};

/// Props for [`SwarmPanel`].
#[derive(Props, Clone, PartialEq)]
pub struct SwarmPanelProps {
    /// Currently known swarms.
    pub swarms: Vec<SwarmData>,
    /// Whether a swarm creation is in progress.
    pub creating: bool,
    /// Callbacks
    pub on_create: EventHandler<String>,
    pub on_stop: EventHandler<String>,
    pub on_close: EventHandler<()>,
    pub visible: bool,
}

/// Swarm management panel — shown as a modal.
#[component]
pub fn SwarmPanel(props: SwarmPanelProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let has_swarms = !props.swarms.is_empty();

    rsx! {
        RcModal {
            active: true,
            title: "🐝 Swarm Manager",
            width: 640,
            class: "swarm-panel",
            onclose: move |_| props.on_close.call(()),

            // Create button
            if !has_swarms {
                div { class: "swarm-empty",
                    p {
                        class: "swarm-empty-text",
                        "No swarms running. Create one from a built-in template."
                    }
                    Button {
                        color: BulmaColor::Primary,
                        loading: props.creating,
                        disabled: props.creating,
                        onclick: move |_| props.on_create.call("swarm".into()),
                        if props.creating {
                            "Creating…"
                        } else {
                            "🚀 Create Swarm"
                        }
                    }
                    p {
                        class: "swarm-hint",
                        "8 specialist agents: research, data, slides, docs, images, video & assistant"
                    }
                }
            }

            // Swarm list
            for swarm in &props.swarms {
                SwarmCard {
                    info: swarm.clone(),
                    on_stop: {
                        let name = swarm.name.clone();
                        move |_| props.on_stop.call(name.clone())
                    },
                }
            }

            // Create another button when swarms exist
            if has_swarms {
                Buttons { class: "swarm-actions",
                    Button {
                        color: BulmaColor::Light,
                        size: BulmaSize::Small,
                        loading: props.creating,
                        disabled: props.creating,
                        onclick: move |_| props.on_create.call("swarm".into()),
                        if props.creating {
                            "Creating…"
                        } else {
                            "+ New Swarm"
                        }
                    }
                }
            }
        }
    }
}

/// Props for a single swarm card.
#[derive(Props, Clone, PartialEq)]
struct SwarmCardProps {
    info: SwarmData,
    on_stop: EventHandler<()>,
}

/// Renders a single swarm with its agents and controls.
#[component]
fn SwarmCard(props: SwarmCardProps) -> Element {
    let info = &props.info;

    rsx! {
        Card { class: "swarm-card",
            CardHeader {
                CardHeaderTitle { class: "swarm-card-title",
                    span { class: "swarm-name", "{info.name}" }
                    Tag {
                        color: tone_color(info.status_tone()),
                        light: true,
                        rounded: true,
                        class: "rc-chip",
                        "{info.status}"
                    }
                }
            }
            CardContent {
                Tags { class: "swarm-card-meta",
                    Tag { rounded: true, class: "rc-chip", "🤖 {info.agents.len()} agents" }
                    Tag { rounded: true, class: "rc-chip", "📋 {info.tasks_routed} tasks" }
                    if info.uptime_secs > 0 {
                        Tag { rounded: true, class: "rc-chip", "⏱ {info.uptime_secs}s" }
                    }
                }

                if !info.description.is_empty() {
                    p { class: "swarm-card-desc", "{info.description}" }
                }

                div { class: "swarm-agents",
                    for agent in &info.agents {
                        div {
                            class: if agent.has_session { "swarm-agent is-active" } else { "swarm-agent" },
                            div { class: "swarm-agent-name",
                                span { class: "agent-role-icon",
                                    "{agent.role_icon()}"
                                }
                                span { "{agent.name}" }
                            }
                            span { class: "swarm-agent-desc", "{agent.description}" }
                        }
                    }
                }
            }

            if info.is_stoppable() {
                CardFooter { class: "swarm-card-footer",
                    Buttons { alignment: dioxus_bulma::prelude::ButtonsAlignment::Right,
                        Button {
                            color: BulmaColor::Danger,
                            size: BulmaSize::Small,
                            outlined: true,
                            onclick: move |_| props.on_stop.call(()),
                            "⏹ Stop"
                        }
                    }
                }
            }
        }
    }
}
