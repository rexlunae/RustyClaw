// ── Messages list ───────────────────────────────────────────────────────────
//
// Scrollable list anchored from the BOTTOM so that scroll_offset = 0 means
// "pinned to the latest messages" (like a chat window). Positive scroll_offset
// shifts the content downward, revealing older messages at the top.
//
// Layout: overflow: Hidden on an outer container, Position::Absolute with
// `bottom: -(scroll_offset)` on an inner container.

use crate::components::message::MessageBubble;
use crate::components::tool_call::ToolCallPanel;
use crate::theme;
use crate::types::DisplayMessage;
use iocraft::prelude::*;
use rustyclaw_view::latest_details_index;

/// Braille spinner frames for smooth animation.
const SPINNER_FRAMES: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];

#[derive(Default, Props)]
pub struct MessagesProps {
    pub messages: Vec<DisplayMessage>,
    pub scroll_offset: i32,
    pub surface: rustyclaw_view::ChatSurfaceData,
    /// Custom name to display for assistant messages.
    pub assistant_name: Option<String>,
    pub selected_idx: Option<usize>,
}

#[component]
pub fn Messages(props: &MessagesProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER_FRAMES[props.surface.spinner_tick % SPINNER_FRAMES.len()];
    let assistant_name = props.assistant_name.clone();

    // Find the index of the most recent warning/error that carries
    // extended details, so we can show the "Ctrl-D for details" hint
    // only on that bubble (older ones have already scrolled out of
    // focus and would just be visual noise).
    let latest_details_idx: Option<usize> = latest_details_index(&props.messages);
    let msg_count = props.messages.len();

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
                    let bubble_data = msg.to_bubble_data(name, has_details);
                    // An assistant turn with empty text is just activity scaffolding
                    // (tool calls land underneath, or the model is mid-work). Don't
                    // render a full empty bubble + action bar. If it carries tool
                    // calls, the panels alone are enough; otherwise show a slim
                    // one-line activity indicator so there's still a heartbeat.
                    let is_empty_turn = msg.content.trim().is_empty();
                    let is_last = i + 1 == msg_count;
                    let show_bubble = !is_empty_turn;
                    // Only the live tail shows a heartbeat; stale empty turns in
                    // history are dropped so they don't leave a frozen "working…".
                    let show_activity = is_empty_turn && msg.tool_calls.is_empty() && is_last && (props.surface.is_streaming || props.surface.is_thinking);
                    element! {
                        View(
                            key: i as u64,
                            flex_direction: FlexDirection::Column,
                            width: 100pct,
                        ) {
                            #(if show_bubble {
                                element! {
                                    MessageBubble(
                                        data: bubble_data,
                                        is_selected: props.selected_idx == Some(i),
                                    )
                                }.into_any()
                            } else if show_activity {
                                element! {
                                    View(padding_left: 2, margin_bottom: 1) {
                                        Text(
                                            content: format!("{} working…", spinner),
                                            color: theme::MUTED,
                                        )
                                    }
                                }.into_any()
                            } else {
                                element! { View() }.into_any()
                            })
                            #(msg.tool_calls.iter().enumerate().map(|(ti, tool)| {
                                element! {
                                    ToolCallPanel(
                                        key: ((i as u64) << 32) | (ti as u64),
                                        data: tool.clone(),
                                    )
                                }
                            }))
                        }
                    }
                }))

                // Streaming indicator at the bottom
                #(if props.surface.is_streaming || props.surface.is_thinking {
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
                                content: format!(
                                    "Thinking… {}",
                                    props.surface.elapsed.as_deref().unwrap_or("")
                                ),
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
