// ── Status bar ──────────────────────────────────────────────────────────────

use iocraft::prelude::*;
use crate::theme;

#[derive(Default, Props)]
pub struct StatusBarProps {
    pub hint: String,
    pub streaming: bool,
    pub elapsed: String,
    pub spinner_tick: usize,
    pub soul_name: String,
    pub model_label: String,
}

#[component]
pub fn StatusBar(props: &StatusBarProps) -> impl Into<AnyElement<'static>> {
    let right_text = if props.streaming {
        let ch = theme::SPINNER[props.spinner_tick % theme::SPINNER.len()];
        format!("{} Streaming response {}", ch, props.elapsed)
    } else if props.hint.is_empty() {
        "Ctrl+C quit · /help commands · ↑↓ scroll".to_string()
    } else {
        props.hint.clone()
    };

    let right_color = if props.streaming { theme::ACCENT } else { theme::MUTED };

    let model_text = if props.model_label.is_empty() {
        "(no model)".to_string()
    } else {
        props.model_label.clone()
    };
    let model_color = if props.model_label.is_empty() { theme::WARN } else { theme::INFO };

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
