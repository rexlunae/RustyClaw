// ── Message bubble ──────────────────────────────────────────────────────────

use crate::markdown;
use crate::theme;
use iocraft::prelude::*;
use rustyclaw_core::types::MessageRole;
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
    let content = &props.data.content;
    let fg = theme::role_color(role);
    let bg = theme::role_bg(role);
    let border = theme::role_border(role);

    let icon = role.icon();
    let label = match role {
        MessageRole::User => "You".to_string(),
        MessageRole::Assistant => props
            .data
            .agent_name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Assistant".to_string()),
        MessageRole::Info => "Info".to_string(),
        MessageRole::Success => "Success".to_string(),
        MessageRole::Warning => "Warning".to_string(),
        MessageRole::Error => "Error".to_string(),
        MessageRole::System => "System".to_string(),
        MessageRole::ToolCall => "Tool Call".to_string(),
        MessageRole::ToolResult => "Tool Result".to_string(),
        MessageRole::Thinking => "Thinking".to_string(),
    };

    // Render markdown for assistant messages, plain text for others
    let display = if *role == MessageRole::Thinking && content.len() > 120 {
        format!("{}…", &content[..120])
    } else if *role == MessageRole::Assistant {
        markdown::render_ansi(content)
    } else {
        content.clone()
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
