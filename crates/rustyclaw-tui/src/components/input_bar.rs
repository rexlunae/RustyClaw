// ── Input bar ───────────────────────────────────────────────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct InputBarProps {
    pub value: String,
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

    element! {
        View(
            width: 100pct,
            height: 3,
            flex_direction: FlexDirection::Column,
            border_style: BorderStyle::Round,
            border_color: theme::ACCENT,
            border_edges: Edges::Top,
        ) {
            View(width: 100pct, height: 1, flex_direction: FlexDirection::Row) {
                Text(content: "❯ ", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                View(flex_grow: 1.0, height: 1, background_color: theme::BG_MAIN) {
                    TextInput(
                        has_focus: props.has_focus,
                        value: props.value.clone(),
                        on_change: props.on_change.take(),
                        color: theme::TEXT,
                    )
                }
                View(padding_left: 1) {
                    Text(content: format!("{} {}", props.gateway_icon, props.gateway_label), color: status_color)
                }
            }
        }
    }
}
