//! Chat surface: message list with empty state, composer, thinking indicator.

use dioxus::prelude::*;

use crate::state::ChatMessage;

use super::message::MessageBubble;
use super::tool_call::ToolCallPanel;

/// Suggested starter prompts shown on the empty state.
const STARTERS: &[(&str, &str, &str)] = &[
    (
        "🔍",
        "Explore the system",
        "What can you do? List your available tools and capabilities.",
    ),
    (
        "📚",
        "Summarise a topic",
        "Give me a concise overview of how RustyClaw secures agent runtimes.",
    ),
    (
        "🛠️",
        "Run a quick task",
        "Help me draft a TOML config for connecting to a local Ollama provider.",
    ),
    (
        "🧠",
        "Think out loud",
        "Walk me through how you'd debug a failing tool call step by step.",
    ),
];

/// Props for [`Chat`].
#[derive(Props, Clone, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub is_processing: bool,
    pub is_thinking: bool,
    pub agent_name: Option<String>,
    pub on_submit: EventHandler<String>,
    pub on_input_change: EventHandler<String>,
}

#[component]
pub fn Chat(props: ChatProps) -> Element {
    let mut input_ref = use_signal(|| props.input.clone());

    // Keep local input in sync if the parent's value resets (e.g. cleared
    // after submit). We can't blindly mirror props.input on every change
    // because that would clobber what the user is typing.
    {
        let parent = props.input.clone();
        use_effect(move || {
            if parent.is_empty() && !input_ref.read().is_empty() {
                input_ref.set(String::new());
            }
        });
    }

    let on_submit = props.on_submit;
    let is_processing = props.is_processing;
    let has_messages = !props.messages.is_empty();

    let send_now = move || {
        let text = input_ref.read().trim().to_string();
        if !text.is_empty() && !is_processing {
            on_submit.call(text);
            input_ref.set(String::new());
        }
    };

    rsx! {
        div { class: "chat",
            // Message list (or empty state)
            if !has_messages && !props.is_thinking {
                EmptyState {
                    agent_name: props.agent_name.clone(),
                    on_pick: move |prompt: String| {
                        input_ref.set(prompt.clone());
                        props.on_input_change.call(prompt);
                    }
                }
            } else {
                div { class: "messages",
                    div { class: "messages-inner",
                        for msg in props.messages.iter() {
                            div { key: "{msg.id}",
                                MessageBubble {
                                    role: msg.role.clone(),
                                    content: msg.content.clone(),
                                    timestamp: msg.timestamp,
                                    is_streaming: msg.is_streaming,
                                }
                                for tool in msg.tool_calls.iter() {
                                    ToolCallPanel {
                                        key: "{tool.id}",
                                        id: tool.id.clone(),
                                        name: tool.name.clone(),
                                        arguments: tool.arguments.clone(),
                                        result: tool.result.clone(),
                                        is_error: tool.is_error,
                                        collapsed: tool.collapsed,
                                    }
                                }
                            }
                        }

                        if props.is_thinking {
                            div { class: "msg-row is-assistant",
                                div { class: "msg-avatar", "🦞" }
                                div { class: "msg-body",
                                    div { class: "msg-header",
                                        span { class: "msg-name", "Assistant" }
                                    }
                                    div { class: "thinking",
                                        span { "Thinking" }
                                        span { class: "thinking-dots",
                                            span {} span {} span {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Composer
            div { class: "composer-wrap",
                div { class: "composer",
                    textarea {
                        placeholder: "Message RustyClaw…",
                        rows: "1",
                        value: "{input_ref}",
                        disabled: is_processing,
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter && !evt.modifiers().shift() {
                                evt.prevent_default();
                                let mut s = send_now;
                                s();
                            }
                        },
                        oninput: move |evt| {
                            let value = evt.value();
                            input_ref.set(value.clone());
                            props.on_input_change.call(value);
                        }
                    }
                    button {
                        class: "composer-send",
                        title: "Send (Enter)",
                        disabled: is_processing || input_ref.read().trim().is_empty(),
                        onclick: move |_| {
                            let mut s = send_now;
                            s();
                        },
                        if is_processing { "…" } else { "↑" }
                    }
                }
                div { class: "composer-hint",
                    "Press Enter to send · Shift + Enter for newline"
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct EmptyStateProps {
    agent_name: Option<String>,
    on_pick: EventHandler<String>,
}

#[component]
fn EmptyState(props: EmptyStateProps) -> Element {
    let greeting = match props.agent_name.as_deref() {
        Some(name) if !name.is_empty() => format!("How can {name} help you today?"),
        _ => "How can I help you today?".to_string(),
    };

    rsx! {
        div { class: "empty-state",
            div { class: "empty-state-card",
                div { class: "empty-state-mark", "🦞" }
                h2 { "{greeting}" }
                p { "Pick a starter or just type below to begin." }
                div { class: "starter-grid",
                    for (icon, title, prompt) in STARTERS.iter() {
                        button {
                            key: "{title}",
                            class: "starter-card",
                            onclick: {
                                let p = (*prompt).to_string();
                                let cb = props.on_pick;
                                move |_| cb.call(p.clone())
                            },
                            div { class: "starter-title",
                                span { "{icon}" }
                                span { "{title}" }
                            }
                            div { class: "starter-sub", "{prompt}" }
                        }
                    }
                }
            }
        }
    }
}
