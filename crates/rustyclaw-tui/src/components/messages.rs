// ── Messages list ───────────────────────────────────────────────────────────
//
// Scrollable list anchored from the BOTTOM so that scroll_offset = 0 means
// "pinned to the latest messages" (like a chat window). Positive scroll_offset
// shifts the content downward, revealing older messages at the top.
//
// Layout: overflow: Hidden on an outer container, Position::Absolute with
// `bottom: -(scroll_offset)` on an inner container.

use iocraft::prelude::*;
use crate::components::message_bubble::MessageBubble;
use crate::types::DisplayMessage;

#[derive(Default, Props)]
pub struct MessagesProps {
    pub messages: Vec<DisplayMessage>,
    pub scroll_offset: i32,
}

#[component]
pub fn Messages(props: &MessagesProps) -> impl Into<AnyElement<'static>> {
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
                    element! {
                        MessageBubble(
                            key: i as u64,
                            role: msg.role,
                            content: msg.content.clone(),
                        )
                    }
                }))
            }
        }
    }
}
