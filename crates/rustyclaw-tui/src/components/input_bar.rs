// ── Input bar ───────────────────────────────────────────────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct InputBarProps {
    pub composer: rustyclaw_view::ComposerData,
    pub value: String,
    pub cursor_offset: usize,
    pub on_change: HandlerMut<'static, String>,
    pub on_submit: HandlerMut<'static, String>,
    pub gateway_icon: String,
    pub gateway_label: String,
    pub gateway_color: Option<Color>,
    /// When false, the TextInput won't capture keystrokes (e.g. dialog open).
    pub has_focus: bool,
}

#[component]
pub fn InputBar(props: &mut InputBarProps) -> impl Into<AnyElement<'static>> {
    let status_color = props.gateway_color.unwrap_or(theme::MUTED);
    let attachments = props.composer.attachments.clone();
    let cursor_offset = props.cursor_offset.min(props.value.len());
    let display_value = if props.has_focus {
        let mut rendered = String::with_capacity(props.value.len() + 1);
        rendered.push_str(&props.value[..cursor_offset]);
        rendered.push('▌');
        rendered.push_str(&props.value[cursor_offset..]);
        rendered
    } else {
        props.value.clone()
    };

    element! {
        View(
            width: 100pct,
            flex_direction: FlexDirection::Column,
            border_style: BorderStyle::Round,
            border_color: theme::ACCENT,
            border_edges: Edges::Top,
        ) {
            View(width: 100pct, height: 1, flex_direction: FlexDirection::Row) {
                Text(content: "❯ ", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                View(flex_grow: 1.0, height: 1, background_color: theme::BG_MAIN) {
                    Text(content: display_value, color: theme::TEXT)
                }
                View(padding_left: 1) {
                    Text(content: format!("{} {}", props.gateway_icon, props.gateway_label), color: status_color)
                }
            }
            #(if !attachments.is_empty() {
                element! {
                    View(width: 100pct, flex_direction: FlexDirection::Row, gap: 1u32, padding_left: 1) {
                        #(attachments.iter().map(|attachment| {
                            element! {
                                View(
                                    border_style: BorderStyle::Round,
                                    border_color: theme::ACCENT_DIM,
                                    padding_left: 1,
                                    padding_right: 1,
                                ) {
                                    Text(
                                        content: format!("{} {}", attachment.kind.icon(), attachment.display_name),
                                        color: theme::TEXT,
                                    )
                                }
                            }
                            .into_any()
                        }))
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
            View(height: 1)
            Text(
                content: if props.composer.has_attachments() {
                    "Attach via /attach file <path> or /attach dir <path> · /attach clear"
                } else {
                    "Press Enter to send · Shift+Enter for newline · /attach file <path> · /attach dir <path>"
                },
                color: theme::MUTED,
            )
        }
    }
}
