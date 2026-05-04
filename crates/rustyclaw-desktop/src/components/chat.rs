//! Chat interface component.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

use crate::state::ChatMessage;

use super::message::MessageBubble;
use super::tool_call::ToolCallPanel;

/// Props for the Chat component.
#[derive(Props, Clone, PartialEq)]
pub struct ChatProps {
    /// Chat messages to display
    pub messages: Vec<ChatMessage>,
    /// Current input text
    pub input: String,
    /// Whether we're waiting for a response
    pub is_processing: bool,
    /// Whether the assistant is thinking
    pub is_thinking: bool,
    /// Callback when user submits a message
    pub on_submit: EventHandler<String>,
    /// Callback when input changes
    pub on_input_change: EventHandler<String>,
}

/// Main chat interface component.
#[component]
pub fn Chat(props: ChatProps) -> Element {
    let mut input_ref = use_signal(|| props.input.clone());

    // Update local input when prop changes
    use_effect(move || {
        input_ref.set(props.input.clone());
    });

    let on_submit = props.on_submit.clone();
    let is_processing = props.is_processing;

    let handle_submit = move |_| {
        let text = input_ref.read().trim().to_string();
        if !text.is_empty() && !is_processing {
            on_submit.call(text);
            input_ref.set(String::new());
        }
    };

    let handle_keypress = move |evt: KeyboardEvent| {
        if evt.key() == Key::Enter && !evt.modifiers().shift() {
            let text = input_ref.read().trim().to_string();
            if !text.is_empty() && !is_processing {
                on_submit.call(text);
                input_ref.set(String::new());
            }
        }
    };

    rsx! {
        div { class: "chat-container",
            style: "display: flex; flex-direction: column; height: 100%;",

            // Message list
            div { class: "message-list",
                style: "flex: 1; overflow-y: auto; padding: 1rem;",

                for msg in props.messages.iter() {
                    div { key: "{msg.id}",
                        MessageBubble {
                            role: msg.role.clone(),
                            content: msg.content.clone(),
                            is_streaming: msg.is_streaming,
                        }

                        // Tool calls for this message
                        for tool in msg.tool_calls.iter() {
                            ToolCallPanel {
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

                // Thinking indicator
                if props.is_thinking {
                    div { class: "thinking-indicator",
                        style: "padding: 0.5rem; color: #666; font-style: italic;",
                        Icon { size: BulmaSize::Small,
                            i { class: "fas fa-brain" }
                        }
                        " Thinking..."
                    }
                }
            }

            // Input area
            div { class: "input-area",
                style: "padding: 1rem; border-top: 1px solid #dbdbdb;",

                Field { addons: true,
                    Control { expanded: true,
                        textarea {
                            class: "textarea",
                            placeholder: "Type a message...",
                            value: "{input_ref}",
                            disabled: is_processing,
                            rows: 2,
                            onkeypress: handle_keypress,
                            oninput: move |evt| {
                                let value = evt.value();
                                input_ref.set(value.clone());
                                props.on_input_change.call(value);
                            },
                        }
                    }
                    Control {
                        Button {
                            id: "rustyclaw-chat-send",
                            color: BulmaColor::Primary,
                            loading: is_processing,
                            disabled: is_processing || input_ref.read().trim().is_empty(),
                            onclick: handle_submit,
                            Icon {
                                i { class: "fas fa-paper-plane" }
                            }
                        }
                    }
                }
            }
        }
    }
}
