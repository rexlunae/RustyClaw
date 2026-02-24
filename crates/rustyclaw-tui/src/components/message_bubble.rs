// ── Message bubble ──────────────────────────────────────────────────────────

use iocraft::prelude::*;
use rustyclaw_core::types::MessageRole;
use crate::theme;

#[derive(Default, Props)]
pub struct MessageBubbleProps {
    pub role: Option<MessageRole>,
    pub content: String,
}

#[component]
pub fn MessageBubble(props: &MessageBubbleProps) -> impl Into<AnyElement<'static>> {
    let role = props.role.unwrap_or(MessageRole::System);
    let fg = theme::role_color(&role);
    let bg = theme::role_bg(&role);
    let border = theme::role_border(&role);

    let icon = role.icon();
    let label = match role {
        MessageRole::User => "You",
        MessageRole::Assistant => "Assistant",
        MessageRole::Info => "Info",
        MessageRole::Success => "Success",
        MessageRole::Warning => "Warning",
        MessageRole::Error => "Error",
        MessageRole::System => "System",
        MessageRole::ToolCall => "Tool Call",
        MessageRole::ToolResult => "Tool Result",
        MessageRole::Thinking => "Thinking",
    };

    let display = if role == MessageRole::Thinking && props.content.len() > 120 {
        format!("{}…", &props.content[..120])
    } else {
        props.content.clone()
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
            Text(content: format!("{} {}", icon, label), color: border, weight: Weight::Bold)
            Text(content: display, color: fg, wrap: TextWrap::Wrap)
        }
    }
}
