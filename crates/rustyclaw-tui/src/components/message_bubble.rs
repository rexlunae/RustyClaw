// ── Message bubble ──────────────────────────────────────────────────────────

use crate::markdown;
use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::MessageBubbleData;

#[derive(Default, Props)]
pub struct MessageBubbleProps {
    /// Shared component data from `rustyclaw-view`.
    ///
    /// This is the single source of truth — role, content, streaming
    /// status, agent name, and details flag all come from here.
    pub data: MessageBubbleData,
}

#[component]
pub fn MessageBubble(props: &MessageBubbleProps) -> impl Into<AnyElement<'static>> {
    let role = &props.data.role;
    let fg = theme::role_color(role);
    let bg = theme::role_bg(role);
    let border = theme::role_border(role);

    // Display logic uses the shared methods:
    //   - label / icon come from the data struct
    //   - markdown rendering is renderer-specific (TUI → ANSI)
    //   - thinking truncation is handled by display_content()
    let display = if props.data.should_render_markdown() {
        markdown::render_ansi(&props.data.content)
    } else {
        props.data.display_content().to_string()
    };

    element! {
        View(
            width: 100pct,
            margin_bottom: 1,
            flex_direction: FlexDirection::Column,
            background_color: bg,
            border_style: BorderStyle::Round,
            border_color: border,
            border_edges: Edges::Left,
            padding_left: 1,
            padding_right: 1,
        ) {
            Text(
                content: format!("{} {}", props.data.icon(), props.data.display_name()),
                color: border,
                weight: Weight::Bold,
            )
            Text(content: display, color: fg, wrap: TextWrap::Wrap)
            #(if props.data.has_details {
                element! {
                    Text(
                        content: "↵ press Ctrl-D for details".to_string(),
                        color: theme::TEXT_DIM,
                    )
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
        }
    }
}
