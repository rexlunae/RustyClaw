//! Chat surface: message list with empty state, composer, thinking indicator.
//!
//! Sub-components:
//!   - [`EmptyState`] — starter prompts when no messages exist
//!   - [`ThinkingIndicator`] — animated "Thinking…" row
//!   - [`ProcessingIndicator`] — hourglass shown before streaming starts
//!   - [`Composer`] — model selector bar + textarea + send/cancel button
//!   - [`ModelBar`] — provider/model dropdowns above the input
//!   - [`StreamingProgress`] — live chunk/byte counter during streaming

use dioxus::prelude::*;
use rustyclaw_core::providers;

use rustyclaw_core::ui::{ChatMessage, StreamingState};

use super::message::MessageBubble;
use super::tool_call::ToolCallPanel;

/// (provider_id, model_id) pair emitted when the user changes model.
pub type ModelSelection = (String, String);

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

    // Auto-scroll: a "sticky to bottom" approach.  A MutationObserver on
    // the stable `.chat` parent scrolls `.messages` to the end whenever
    // new content arrives — but only if the user hasn't scrolled away.
    // A scroll-event listener on `.messages` re-evaluates the sticky
    // flag so the user can disengage (scroll up) and re-engage (scroll
    // back to the bottom).
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
                                    data: rustyclaw_view::MessageBubbleData::from_chat_message(
                                        msg,
                                        props.agent_name.clone(),
                                    ),
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
                            ThinkingIndicator {
                                agent_name: props.agent_name.clone(),
                            }
                        }

                        if props.is_streaming {
                            StreamingProgress {
                                chunks: props.streaming_chunks,
                                bytes: props.streaming_bytes,
                            }
                        } else if props.is_processing {
                            ProcessingIndicator {}
                        }
                    }
                }
            }

            Composer {
                input: input_ref,
                is_processing: is_processing,
                current_provider: props.current_provider.clone(),
                current_model: props.current_model.clone(),
                on_send: move |_| {
                    let mut s = send_now;
                    s();
                },
                on_cancel: props.on_cancel,
                on_input_change: props.on_input_change,
                on_model_change: props.on_model_change,
                on_add_provider: props.on_add_provider,
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

// ── Thinking indicator ──────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct ThinkingIndicatorProps {
    agent_name: Option<String>,
}

/// Animated "Thinking..." row shown while the model is processing.
#[component]
fn ThinkingIndicator(props: ThinkingIndicatorProps) -> Element {
    rsx! {
        div { class: "msg-row is-assistant",
            div { class: "msg-avatar", "🦞" }
            div { class: "msg-body",
                div { class: "msg-header",
                    span { class: "msg-name",
                        {props.agent_name.as_deref().unwrap_or("Assistant")}
                    }
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

// ── Processing indicator ───────────────────────────────────────────────────

/// Simple "Processing…" indicator shown after submit but before streaming.
#[component]
fn ProcessingIndicator() -> Element {
    rsx! {
        div { class: "streaming-progress",
            span { class: "streaming-progress-icon", "⏳" }
            span { class: "streaming-progress-text", "Processing…" }
        }
    }
}

// ── Composer ───────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct ComposerProps {
    input: Signal<String>,
    is_processing: bool,
    current_provider: Option<String>,
    current_model: Option<String>,
    on_send: EventHandler<()>,
    on_cancel: EventHandler<()>,
    on_input_change: EventHandler<String>,
    on_model_change: EventHandler<ModelSelection>,
    on_add_provider: EventHandler<()>,
}

/// Model selector bar + message input area with send/cancel button.
#[component]
fn Composer(props: ComposerProps) -> Element {
    let mut input_ref = props.input;
    let is_processing = props.is_processing;
    let on_send = props.on_send;

    rsx! {
        div { class: "composer-wrap",
            ModelBar {
                current_provider: props.current_provider.clone(),
                current_model: props.current_model.clone(),
                on_model_change: props.on_model_change,
                on_add_provider: props.on_add_provider,
            }
            div { class: "composer",
                textarea {
                    placeholder: "Message RustyClaw…",
                    rows: "1",
                    value: "{input_ref}",
                    disabled: is_processing,
                    onkeydown: move |evt: KeyboardEvent| {
                        if evt.key() == Key::Enter && !evt.modifiers().shift() {
                            evt.prevent_default();
                            on_send.call(());
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
                    title: if is_processing { "Cancel request" } else { "Send (Enter)" },
                    disabled: !is_processing && input_ref.read().trim().is_empty(),
                    onclick: move |_| {
                        if is_processing {
                            props.on_cancel.call(());
                        } else {
                            on_send.call(());
                        }
                    },
                    if is_processing { "×" } else { "↑" }
                }
            }
            div { class: "composer-hint",
                "Press Enter to send · Shift + Enter for newline"
            }
        }
    }
}

// ── Streaming progress indicator ───────────────────────────────────────────

// ── Model bar (provider / model selector above composer) ─────────────

#[derive(Props, Clone, PartialEq)]
struct ModelBarProps {
    current_provider: Option<String>,
    current_model: Option<String>,
    on_model_change: EventHandler<ModelSelection>,
    on_add_provider: EventHandler<()>,
}

/// Sentinel value used for the "Add provider…" menu entry.
const ADD_PROVIDER_SENTINEL: &str = "__add_provider__";

#[component]
fn ModelBar(props: ModelBarProps) -> Element {
    let mut selected_provider =
        use_signal(|| props.current_provider.clone().unwrap_or_default());
    let mut selected_model =
        use_signal(|| props.current_model.clone().unwrap_or_default());

    // Keep signals in sync when parent props change (e.g. gateway reports
    // the active provider/model after reconnection).
    {
        let pp = props.current_provider.clone();
        let pm = props.current_model.clone();
        use_effect(move || {
            if let Some(ref p) = pp {
                if *selected_provider.read() != *p {
                    selected_provider.set(p.clone());
                }
            }
            if let Some(ref m) = pm {
                if *selected_model.read() != *m {
                    selected_model.set(m.clone());
                }
            }
        });
    }

    let provider_list = providers::provider_ids();
    let current_provider_id = selected_provider.read().clone();
    let models_for_current = providers::models_for_provider(&current_provider_id);

    rsx! {
        div { class: "model-bar",
            select {
                class: "model-bar-select",
                value: "{selected_provider}",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let on_add_provider = props.on_add_provider;
                    move |evt: Event<FormData>| {
                        let prov = evt.value();
                        if prov == ADD_PROVIDER_SENTINEL {
                            on_add_provider.call(());
                            return;
                        }
                        selected_provider.set(prov.clone());
                        let models = providers::models_for_provider(&prov);
                        let first_model =
                            models.first().copied().unwrap_or("").to_string();
                        selected_model.set(first_model.clone());
                        on_model_change.call((prov, first_model));
                    }
                },
                for pid in provider_list.iter() {
                    option {
                        value: "{pid}",
                        selected: *pid == current_provider_id.as_str(),
                        "{providers::display_name_for_provider(pid)}"
                    }
                }
                option { disabled: true, "─────────────" }
                option {
                    value: "{ADD_PROVIDER_SENTINEL}",
                    "Add provider\u{2026}"
                }
            }

            select {
                class: "model-bar-select",
                value: "{selected_model}",
                onchange: {
                    let on_model_change = props.on_model_change;
                    let provider_signal = selected_provider;
                    move |evt: Event<FormData>| {
                        let mdl = evt.value();
                        selected_model.set(mdl.clone());
                        let prov = provider_signal.read().clone();
                        on_model_change.call((prov, mdl));
                    }
                },
                for mid in models_for_current.iter() {
                    option {
                        value: "{mid}",
                        selected: *mid == selected_model.read().as_str(),
                        "{mid}"
                    }
                }
            }
        }
    }
}

// ── Streaming progress ──────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct StreamingProgressProps {
    chunks: u32,
    bytes: usize,
}

#[component]
fn StreamingProgress(props: StreamingProgressProps) -> Element {
    let state = StreamingState {
        is_streaming: true,
        is_thinking: false,
        chunks: props.chunks,
        bytes: props.bytes,
        start_time: None,
    };
    let label = state.progress_summary();

    rsx! {
        div { class: "streaming-progress",
            span { class: "streaming-progress-icon streaming-pulse" }
            span { class: "streaming-progress-text", "{label}" }
        }
    }
}
