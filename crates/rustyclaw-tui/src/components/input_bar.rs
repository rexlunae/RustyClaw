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

    // Split the value into [before-cursor][cursor-cell][after-cursor] so the
    // cursor renders as an inverted-color overlay on the underlying glyph
    // instead of inserting a new character (which would shift the rest of
    // the line every time the cursor moved).
    let (before_cursor, cursor_char, after_cursor) = if props.has_focus {
        let before = props.value[..cursor_offset].to_string();
        let mut chars = props.value[cursor_offset..].chars();
        match chars.next() {
            Some(ch) => (before, ch.to_string(), chars.as_str().to_string()),
            None => (before, " ".to_string(), String::new()),
        }
    } else {
        (props.value.clone(), String::new(), String::new())
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
                View(flex_grow: 1.0, height: 1, background_color: theme::BG_MAIN, flex_direction: FlexDirection::Row) {
                    Text(content: before_cursor, color: theme::TEXT)
                    #(if props.has_focus {
                        element! {
                            View(background_color: theme::TEXT) {
                                Text(
                                    content: cursor_char,
                                    color: theme::BG_MAIN,
                                )
                            }
                        }.into_any()
                    } else {
                        element! { View() }.into_any()
                    })
                    Text(content: after_cursor, color: theme::TEXT)
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
