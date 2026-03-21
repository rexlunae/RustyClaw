//! Chat interface component.

use dioxus::prelude::*;
use dioxus_bulma::prelude::*;

use crate::state::{ChatMessage, MessageRole};

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
    let mut message_list_ref = use_signal(|| None::<web_sys::Element>);
    
    // Update local input when prop changes
    use_effect(move || {
        input_ref.set(props.input.clone());
    });
    
    // Auto-scroll to bottom when new messages arrive
    use_effect(move || {
        let msg_count = props.messages.len();
        if msg_count > 0 {
            spawn(async move {
                if let Some(element) = &*message_list_ref.read() {
                    element.set_scroll_top(element.scroll_height());
                }
            });
        }
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
            div { 
                class: "message-list",
                style: "flex: 1; overflow-y: auto; padding: 1rem; scroll-behavior: smooth;",
                onmounted: move |evt| {
                    let element = evt.data.downcast::<web_sys::Element>().cloned();
                    message_list_ref.set(element);
                },
                
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
                        span { class: "icon is-small",
                            i { class: "fas fa-brain" }
                        }
                        " Thinking..."
                    }
                }
            }
            
            // Input area
            div { class: "input-area",
                style: "padding: 1rem; border-top: 1px solid #dbdbdb;",
                
                Field {
                    Field_addons { class: "is-expanded",
                        Control { class: "is-expanded",
                            textarea {
                                class: "textarea",
                                placeholder: if is_processing { "Please wait..." } else { "Type a message... (Enter to send, Shift+Enter for new line)" },
                                value: "{input_ref}",
                                disabled: is_processing,
                                rows: 3,
                                style: "resize: vertical; min-height: 80px;",
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
                                color: Color::Primary,
                                loading: is_processing,
                                disabled: is_processing || input_ref.read().trim().is_empty(),
                                onclick: handle_submit,
                                span { class: "icon is-small",
                                    i { class: "fas fa-paper-plane" }
                                }
                                span { "Send" }
                            }
                        }
                    }
                }
            }
        }
    }
}
