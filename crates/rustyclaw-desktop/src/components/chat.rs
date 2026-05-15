//! Chat surface — composite of [`Messages`] and [`InputBar`].
//!
//! This component owns the message list and input bar, keeping the
//! local text value in a `Signal<String>` and coordinating auto-scroll
//! via a Dioxus document `eval`.  It delegates rendering to the
//! extracted sub-components [`Messages`] and [`InputBar`], which mirror
//! the TUI's `components/messages.rs` and `components/input_bar.rs`.

use dioxus::prelude::*;
use rustyclaw_core::ui::ChatMessage;

use super::input_bar::InputBar;
use super::messages::{Messages, ModelSelection};

/// Props for [`Chat`].
#[derive(Props, Clone, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub is_processing: bool,
    pub is_thinking: bool,
    pub is_streaming: bool,
    pub streaming_chunks: u32,
    pub streaming_bytes: usize,
    pub agent_name: Option<String>,
    pub current_provider: Option<String>,
    pub current_model: Option<String>,
    pub on_submit: EventHandler<String>,
    pub on_cancel: EventHandler<()>,
    pub on_input_change: EventHandler<String>,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_add_provider: EventHandler<()>,
}

/// Composite of the message list and the input bar.
///
/// Owns the local input signal and auto-scroll logic that the two
/// sub-components share.
#[component]
pub fn Chat(props: ChatProps) -> Element {
    let mut input_ref = use_signal(|| props.input.clone());

    // Keep local input in sync if the parent's value resets (e.g. cleared
    // after submit). Not blindly mirrored so the user's typing isn't clobbered.
    {
        let parent = props.input.clone();
        use_effect(move || {
            if parent.is_empty() && !input_ref.read().is_empty() {
                input_ref.set(String::new());
            }
        });
    }

    // Auto-scroll: MutationObserver-based sticky-scroll on `.messages`.
    use_effect(move || {
        document::eval(
            r#"
            (function() {
                if (window.__rcAutoScroll) return;
                window.__rcStick = true;
                var currentEl = null;

                function attach(el) {
                    if (el === currentEl) return;
                    if (currentEl) currentEl.removeEventListener('scroll', onScroll);
                    currentEl = el;
                    if (!el) return;
                    el.addEventListener('scroll', onScroll, { passive: true });
                    el.scrollTop = el.scrollHeight;
                }

                function onScroll() {
                    if (!currentEl) return;
                    var gap = currentEl.scrollHeight - currentEl.scrollTop - currentEl.clientHeight;
                    window.__rcStick = gap < 80;
                }

                var chat = document.querySelector('.chat');
                if (!chat) return;
                new MutationObserver(function() {
                    var el = chat.querySelector('.messages');
                    attach(el);
                    if (el && window.__rcStick) {
                        el.scrollTop = el.scrollHeight;
                    }
                }).observe(chat, { childList: true, subtree: true, characterData: true });

                window.__rcAutoScroll = true;
            })();
        "#,
        );
    });

    let on_submit = props.on_submit;
    let is_processing = props.is_processing;

    let mut send_now = move || {
        let text = input_ref.read().trim().to_string();
        if !text.is_empty() && !is_processing {
            on_submit.call(text);
            input_ref.set(String::new());
        }
    };

    rsx! {
        div { class: "chat",
            Messages {
                messages: props.messages.clone(),
                is_processing: props.is_processing,
                is_thinking: props.is_thinking,
                is_streaming: props.is_streaming,
                streaming_chunks: props.streaming_chunks,
                streaming_bytes: props.streaming_bytes,
                agent_name: props.agent_name.clone(),
                on_starter_pick: {
                    let on_input_change = props.on_input_change;
                    move |prompt: String| {
                        input_ref.set(prompt.clone());
                        on_input_change.call(prompt);
                    }
                },
            }

            InputBar {
                input: input_ref,
                is_processing: is_processing,
                current_provider: props.current_provider.clone(),
                current_model: props.current_model.clone(),
                on_send: move |_| send_now(),
                on_cancel: props.on_cancel,
                on_input_change: props.on_input_change,
                on_model_change: props.on_model_change,
                on_add_provider: props.on_add_provider,
            }
        }
    }
}
