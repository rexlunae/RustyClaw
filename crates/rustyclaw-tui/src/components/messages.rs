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
                    // An assistant turn that only carries tool calls has empty
                    // text — don't render an empty bubble box (which would also
                    // show an action bar). Render just the tool panels instead.
                    let show_bubble = !(msg.content.trim().is_empty() && !msg.tool_calls.is_empty());
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
