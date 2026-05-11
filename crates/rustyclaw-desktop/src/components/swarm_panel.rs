//! Swarm management panel for the desktop UI.
//!
//! Displays active swarms, their agents and communication flows, and provides
//! controls to create, inspect, and stop swarms.

use dioxus::prelude::*;

/// Summary of an agent within a swarm (view-model for the UI).
#[derive(Clone, Debug, PartialEq)]
pub struct SwarmAgentInfo {
    pub id: String,
    pub name: String,
    pub role: String,
    pub description: String,
    pub has_session: bool,
}

/// Summary of a swarm instance (view-model for the UI).
#[derive(Clone, Debug, PartialEq)]
pub struct SwarmInfo {
    pub name: String,
    pub status: String,
    pub description: String,
    pub agents: Vec<SwarmAgentInfo>,
    pub tasks_routed: u64,
    pub uptime_secs: u64,
}

/// Props for [`SwarmPanel`].
#[derive(Props, Clone, PartialEq)]
pub struct SwarmPanelProps {
    /// Currently known swarms.
    pub swarms: Vec<SwarmInfo>,
    /// Whether a swarm creation is in progress.
    pub creating: bool,
    /// Callbacks
    pub on_create: EventHandler<String>,
    pub on_stop: EventHandler<String>,
    pub on_close: EventHandler<()>,
    pub visible: bool,
}

/// Swarm management panel — shown as a slide-over or modal.
#[component]
pub fn SwarmPanel(props: SwarmPanelProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let has_swarms = !props.swarms.is_empty();

    rsx! {
        div { class: "modal-backdrop",
            onclick: move |_| props.on_close.call(()),

            div {
                class: "modal swarm-panel",
                style: "max-width: 640px; max-height: 80vh; overflow-y: auto;",
                onclick: move |evt| evt.stop_propagation(),

                div { class: "modal-head",
                    span { class: "modal-title", "🐝 Swarm Manager" }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| props.on_close.call(()),
                        "✕"
                    }
                }

                div { class: "modal-body",
                    // Create button
                    if !has_swarms {
                        div { class: "swarm-empty",
                            p {
                                class: "swarm-empty-text",
                                "No swarms running. Create one from a built-in template."
                            }
                            button {
                                class: "btn btn-primary",
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
                        div { class: "swarm-actions",
                            button {
                                class: "btn btn-subtle btn-sm",
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
    }
}

/// Props for a single swarm card.
#[derive(Props, Clone, PartialEq)]
struct SwarmCardProps {
    info: SwarmInfo,
    on_stop: EventHandler<()>,
}

/// Renders a single swarm with its agents and controls.
#[component]
fn SwarmCard(props: SwarmCardProps) -> Element {
    let info = &props.info;
    let status_class = match info.status.as_str() {
        "Running" => "chip is-success",
        "Idle" => "chip is-info",
        "Paused" => "chip is-warn",
        "Stopped" => "chip is-muted",
        _ => "chip is-danger",
    };

    rsx! {
        div { class: "swarm-card",
            div { class: "swarm-card-header",
                div { class: "swarm-card-title",
                    span { class: "swarm-name", "{info.name}" }
                    span { class: "{status_class}", "{info.status}" }
                }
                div { class: "swarm-card-meta",
                    span { class: "chip", "🤖 {info.agents.len()} agents" }
                    span { class: "chip", "📋 {info.tasks_routed} tasks" }
                    if info.uptime_secs > 0 {
                        span { class: "chip", "⏱ {info.uptime_secs}s" }
                    }
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
                                match agent.role.as_str() {
                                    "Orchestrator" => "🎯",
                                    "Virtual Assistant" => "💼",
                                    "Deep Research" => "🔬",
                                    "Data Analyst" => "📊",
                                    "Slides" => "📽",
                                    "Docs" => "📄",
                                    "Image Generation" => "🎨",
                                    "Video Generation" => "🎬",
                                    _ => "🤖",
                                }
                            }
                            span { "{agent.name}" }
                        }
                        span { class: "swarm-agent-desc", "{agent.description}" }
                    }
                }
            }

            if info.status == "Running" || info.status == "Idle" || info.status == "Paused" {
                div { class: "swarm-card-footer",
                    button {
                        class: "btn btn-danger btn-sm",
                        onclick: move |_| props.on_stop.call(()),
                        "⏹ Stop"
                    }
                }
            }
        }
    }
}
