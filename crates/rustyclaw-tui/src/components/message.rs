// ── Message bubble ──────────────────────────────────────────────────────────

use crate::markdown;
use crate::theme;
use iocraft::prelude::*;
use rustyclaw_core::types::MessageRole;
use rustyclaw_view::MessageBubbleData;

#[derive(Default, Props)]
pub struct MessageBubbleProps {
    /// Shared component data from `rustyclaw-view`.
    pub data: MessageBubbleData,
    /// Whether this bubble is the currently selected one.
    pub is_selected: bool,
}

#[component]
pub fn MessageBubble(props: &MessageBubbleProps) -> impl Into<AnyElement<'static>> {
    let role = &props.data.role;
    let fg = theme::role_color(role);
    let bg = theme::role_bg(role);
    let border = theme::role_border(role);

    // content_for_render handles collapsed previews for every role,
    // including the one-line gist for folded Thinking blocks (and the
    // full reasoning text when expanded).
    let render_content = props.data.content_for_render();
    let display = if props.data.should_render_markdown() {
        markdown::render_ansi(render_content.as_ref())
    } else {
        render_content.into_owned()
    };

    // Show the action bar only for assistant messages; it is not useful
    // (and wastes render cycles) on short user/system/info messages.
    let show_actions = !props.data.is_streaming && props.data.role == MessageRole::Assistant;
    let action_color = if props.is_selected {
        theme::MUTED
    } else {
        theme::TEXT_DIM
    };
    let expand_label = if props.data.collapsed {
        "expand"
    } else {
        "collapse"
    };
    let action_bar = format!("[Ctrl+E] {}  [Ctrl+Y] copy  [Ctrl+S] save", expand_label);

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
                content: format!("{} {}", props.data.icon(), props.data.header_label()),
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
            #(if show_actions {
                element! {
                    Text(
                        content: action_bar,
                        color: action_color,
                    )
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
        }
    }
}
