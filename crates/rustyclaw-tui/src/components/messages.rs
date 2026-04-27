// ── Messages list ───────────────────────────────────────────────────────────
//
// Scrollable list anchored from the BOTTOM so that scroll_offset = 0 means
// "pinned to the latest messages" (like a chat window). Positive scroll_offset
// shifts the content downward, revealing older messages at the top.
//
// Layout: overflow: Hidden on an outer container, Position::Absolute with
// `bottom: -(scroll_offset)` on an inner container.

use crate::components::message_bubble::MessageBubble;
use crate::theme;
use crate::types::DisplayMessage;
use iocraft::prelude::*;

/// Braille spinner frames for smooth animation.
const SPINNER_FRAMES: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];

#[derive(Default, Props)]
pub struct MessagesProps {
    pub messages: Vec<DisplayMessage>,
    pub scroll_offset: i32,
    /// Whether the model is currently streaming/thinking.
    pub streaming: bool,
    /// Tick counter for spinner animation.
    pub spinner_tick: usize,
    /// Elapsed time string (e.g., "2.3s").
    pub elapsed: String,
    /// Custom name to display for assistant messages.
    pub assistant_name: Option<String>,
}

#[component]
pub fn Messages(props: &MessagesProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER_FRAMES[props.spinner_tick % SPINNER_FRAMES.len()];
    let assistant_name = props.assistant_name.clone();

    // Find the index of the most recent warning/error that carries
    // extended details, so we can show the "Ctrl-D for details" hint
    // only on that bubble (older ones have already scrolled out of
    // focus and would just be visual noise).
    let latest_details_idx: Option<usize> = props
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| {
            matches!(
                m.role,
                rustyclaw_core::types::MessageRole::Warning
                    | rustyclaw_core::types::MessageRole::Error
            ) && m.details.is_some()
        })
        .map(|(i, _)| i);

    element! {
        View(
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            overflow: Overflow::Hidden,
            width: 100pct,
        ) {
            View(
                flex_direction: FlexDirection::Column,
                width: 100pct,
                position: Position::Absolute,
                bottom: -(props.scroll_offset),
            ) {
                #(props.messages.iter().enumerate().map(|(i, msg)| {
                    let name = assistant_name.clone();
                    let has_details = latest_details_idx == Some(i);
                    element! {
                        MessageBubble(
                            key: i as u64,
                            role: msg.role,
                            content: msg.content.clone(),
                            assistant_name: name,
                            has_details: has_details,
                        )
                    }
                }))

                // Streaming indicator at the bottom
                #(if props.streaming {
                    element! {
                        View(
                            flex_direction: FlexDirection::Row,
                            padding_left: 2,
                            padding_top: 1,
                        ) {
                            Text(
                                content: format!("{} ", spinner),
                                color: theme::ACCENT,
                            )
                            Text(
                                content: format!("Thinking… {}", props.elapsed),
                                color: theme::MUTED,
                            )
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })
            }
        }
    }
}
