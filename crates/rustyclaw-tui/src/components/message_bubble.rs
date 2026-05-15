// ── Message bubble ──────────────────────────────────────────────────────────

use crate::markdown;
use crate::theme;
use iocraft::prelude::*;
use rustyclaw_core::types::MessageRole;
use rustyclaw_view::MessageBubbleData;

#[derive(Default, Props)]
pub struct MessageBubbleProps {
    /// Shared component data from rustyclaw-view.
    ///
    /// Provides role, content, streaming status and agent name
    /// from a single source of truth shared by all clients.
    pub data: Option<MessageBubbleData>,

    /// Legacy: fallback content when data is not provided.
    /// Kept to ease incremental migration in call sites.
    pub content: String,

    /// Legacy: fallback role.
    pub role: Option<MessageRole>,

    /// Legacy: fallback agent display name for assistant messages.
    pub assistant_name: Option<String>,

    /// True when this message has extended structured details that
    /// can be opened with Ctrl-D.  When set, a small hint is shown
    /// after the content.
    pub has_details: bool,
}

#[component]
pub fn MessageBubble(props: &MessageBubbleProps) -> impl Into<AnyElement<'static>> {
    // Resolve role, content, and agent name from shared data (preferred)
    // or fall back to legacy individual props for incremental migration.
    let role = props
        .data
        .as_ref()
        .map(|d| &d.role)
        .copied()
        .or(props.role)
        .unwrap_or(MessageRole::System);
    let content = props
        .data
        .as_ref()
        .map(|d| d.content.as_str())
        .unwrap_or(&props.content);
    let assistant_name = props
        .data
        .as_ref()
        .and_then(|d| d.agent_name.as_deref())
        .map(|s| s.to_string())
        .or_else(|| props.assistant_name.clone());
    let fg = theme::role_color(&role);
    let bg = theme::role_bg(&role);
    let border = theme::role_border(&role);

    let icon = role.icon();
    let label = match role {
        MessageRole::User => "You".to_string(),
        MessageRole::Assistant => assistant_name
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
    let display = if role == MessageRole::Thinking && content.len() > 120 {
        format!("{}…", &content[..120])
    } else if role == MessageRole::Assistant {
        markdown::render_ansi(content)
    } else {
        content.to_string()
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
            #(if props.has_details {
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
