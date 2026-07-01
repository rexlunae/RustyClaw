//! Chat surface — renders the conversation with the `dioxus-genai-chat`
//! `ChatSurface` component (embedded mode), plus RustyClaw's empty state and the
//! composer accessory (model + working-directory selectors).
//!
//! This component owns the local input value (`Signal<String>`) for snappy
//! typing and the auto-scroll behaviour; the transcript and attachment chips
//! are rendered by the crate. The send/stop buttons are rendered by us through
//! the crate's `input_accessory` slot so they sit *inside* the input box (the
//! crate's own send/stop buttons render outside it, so they stay disabled).

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button};
use dioxus_genai_chat::{ChatControls, ChatSurface, ContextEvent};
use rustyclaw_core::ui::ChatMessage;

use super::composer_accessory::{ComposerAccessory, ModelSelection};
use crate::chat_transcript::{to_context_items, to_transcript};

/// Props for [`Chat`].
#[derive(Props, Clone, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub surface: rustyclaw_view::ChatSurfaceData,
    pub bottom_bar: rustyclaw_view::BottomBarData,
    pub agent_name: Option<String>,
    pub on_submit: EventHandler<String>,
    pub on_cancel: EventHandler<()>,
    pub on_input_change: EventHandler<String>,
    pub on_model_change: EventHandler<ModelSelection>,
    pub on_add_provider: EventHandler<()>,
    pub on_add_file_attachment: EventHandler<()>,
    pub on_add_directory_attachment: EventHandler<()>,
    pub on_remove_attachment: EventHandler<String>,
    pub on_select_directory: EventHandler<String>,
}

/// The chat surface: empty-state when there are no messages, otherwise the
/// `ChatSurface` transcript + composer.
#[component]
pub fn Chat(props: ChatProps) -> Element {
    let mut input_ref = use_signal(|| props.input.clone());

    // Keep the local input in sync when the parent clears it (e.g. after submit).
    {
        let parent = props.input.clone();
        use_effect(move || {
            if parent.is_empty() && !input_ref.read().is_empty() {
                input_ref.set(String::new());
            }
        });
    }

    // Auto-scroll: keep the transcript pinned to the bottom as content streams
    // in, unless the user has scrolled up. Observes the embedded surface.
    use_effect(move || {
        document::eval(
            r#"
            (function() {
                if (window.__rcAutoScroll) return;
                window.__rcStick = true;
                var el = document.querySelector('.chat-scroll');
                if (!el) return;
                el.addEventListener('scroll', function() {
                    var gap = el.scrollHeight - el.scrollTop - el.clientHeight;
                    window.__rcStick = gap < 80;
                }, { passive: true });
                new MutationObserver(function() {
                    if (window.__rcStick) { el.scrollTop = el.scrollHeight; }
                }).observe(el, { childList: true, subtree: true, characterData: true });
                el.scrollTop = el.scrollHeight;
                window.__rcAutoScroll = true;
            })();
        "#,
        );
    });

    let is_processing = props.surface.is_processing;
    let has_messages = !props.messages.is_empty();

    // The crate renders its send/stop buttons *below* the input box, so keep
    // them off; ours live in the accessory row inside the box instead.
    let controls = ChatControls {
        show_input: true,
        show_send_button: false,
        show_stop_button: false,
        show_retry_button: false,
        show_clear_button: false,
        input_enabled: !is_processing,
        placeholder: rustyclaw_view::ComposerData::PLACEHOLDER.to_string(),
        allow_file_attachments: true,
        allow_directory_context: true,
        attach_files_label: "📎 Add file".to_string(),
        add_directory_label: "📁 Add dir".to_string(),
        allow_document_selection: false,
    };

    let on_submit = props.on_submit;
    let on_input_change = props.on_input_change;
    let mut send = move |text: String| {
        let text = text.trim().to_string();
        if !text.is_empty() && !is_processing {
            on_submit.call(text);
            input_ref.set(String::new());
            on_input_change.call(String::new());
        }
    };

    let on_cancel = props.on_cancel;
    let accessory = rsx! {
        ComposerAccessory {
            current_provider: props.bottom_bar.composer.current_provider.clone(),
            current_model: props.bottom_bar.composer.current_model.clone(),
            directory_selector: props.bottom_bar.directory_selector.clone(),
            on_model_change: props.on_model_change,
            on_add_provider: props.on_add_provider,
            on_select_directory: props.on_select_directory,
        }
        if is_processing {
            Button {
                size: BulmaSize::Small,
                color: BulmaColor::Warning,
                outlined: true,
                class: "composer-stop",
                onclick: move |_| on_cancel.call(()),
                "Stop"
            }
        } else {
            Button {
                size: BulmaSize::Small,
                color: BulmaColor::Primary,
                class: "composer-send",
                disabled: input_ref().trim().is_empty(),
                onclick: move |_| send(input_ref()),
                "Send"
            }
        }
    };

    rsx! {
        div { class: "chat",
            div { class: "chat-scroll",
                if !has_messages && !props.surface.is_thinking {
                    EmptyState {
                        agent_name: props.agent_name.clone(),
                        on_pick: move |prompt: String| {
                            input_ref.set(prompt.clone());
                            on_input_change.call(prompt);
                        },
                    }
                }
                ChatSurface {
                    embedded: true,
                    transcript: to_transcript(&props.messages, &props.surface),
                    controls,
                    input: input_ref(),
                    attachments: to_context_items(&props.bottom_bar.composer.attachments),
                    input_accessory: accessory,
                    on_input: move |value: String| {
                        input_ref.set(value.clone());
                        on_input_change.call(value);
                    },
                    on_send: send,
                    on_stop: move |_| props.on_cancel.call(()),
                    on_context: move |event: ContextEvent| match event {
                        ContextEvent::AddFilesRequested => props.on_add_file_attachment.call(()),
                        ContextEvent::AddDirectoryRequested => props.on_add_directory_attachment.call(()),
                        ContextEvent::Remove(id) => props.on_remove_attachment.call(id),
                    },
                }
            }
        }
    }
}

// ── Empty state ─────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct EmptyStateProps {
    agent_name: Option<String>,
    on_pick: EventHandler<String>,
}

#[component]
fn EmptyState(props: EmptyStateProps) -> Element {
    use dioxus_bulma::components::{Subtitle, Title, TitleSize};
    use dioxus_bulma::prelude::BulmaBox;

    let data = rustyclaw_view::EmptyStateData {
        agent_name: props.agent_name,
    };

    rsx! {
        div { class: "empty-state",
            div { class: "empty-state-card",
                div { class: "empty-state-mark", "🦞" }
                Title { size: TitleSize::Is3, "{data.greeting()}" }
                Subtitle { size: TitleSize::Is6, "{data.subtitle()}" }
                div { class: "starter-grid",
                    for starter in data.starters().iter() {
                        BulmaBox {
                            key: "{starter.title}",
                            class: "starter-card",
                            onclick: {
                                let p = starter.prompt.to_string();
                                let cb = props.on_pick;
                                move |_| cb.call(p.clone())
                            },
                            div { class: "starter-title",
                                span { "{starter.icon}" }
                                span { "{starter.title}" }
                            }
                            div { class: "starter-sub", "{starter.prompt}" }
                        }
                    }
                }
            }
        }
    }
}
