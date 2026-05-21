// ── Status bar ──────────────────────────────────────────────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct StatusBarProps {
    pub hint: String,
    pub surface: rustyclaw_view::ChatSurfaceData,
    pub soul_name: String,
    pub model_label: String,
}

#[component]
pub fn StatusBar(props: &StatusBarProps) -> impl Into<AnyElement<'static>> {
    let right_text = if props.surface.is_streaming || props.surface.is_thinking {
        let ch = theme::SPINNER[props.surface.spinner_tick % theme::SPINNER.len()];
        format!("{} {}", ch, props.surface.status_hint_text(&props.hint))
    } else {
        props.surface.status_hint_text(&props.hint)
    };

    let right_color = if props.surface.is_streaming || props.surface.is_thinking {
        theme::ACCENT
    } else {
        theme::MUTED
    };

    let model_text = if props.model_label.is_empty() {
        "(no model)".to_string()
    } else {
        props.model_label.clone()
    };
    let model_color = if props.model_label.is_empty() {
        theme::WARN
    } else {
        theme::INFO
    };

    element! {
        View(
            width: 100pct,
            height: 1,
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            padding_left: 1,
            padding_right: 1,
        ) {
            View(flex_direction: FlexDirection::Row) {
                Text(content: "🦀 ", color: theme::ACCENT)
                Text(content: &props.soul_name, color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                Text(content: format!(" v{}", env!("CARGO_PKG_VERSION")), color: theme::MUTED)
                Text(content: " · ", color: theme::MUTED)
                Text(content: model_text, color: model_color)
            }
            Text(content: right_text, color: right_color)
        }
    }
}
