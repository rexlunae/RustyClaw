// ── Logs dialog — general log viewer overlay ────────────────────────────────
//
// Opened with /logs; Esc closes. Shows log lines with source selection
// and optional follow/tail mode.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct LogsDialogProps {
    pub data: Option<rustyclaw_view::LogsPanelData>,
}

#[component]
pub fn LogsDialog(props: &LogsDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        let follow_indicator = if data.following { " [FOLLOW]" } else { "" };
        rows.push((
            "Source".into(),
            format!("{}{}", data.source.label(), follow_indicator),
        ));
        rows.push(("Lines".into(), data.line_count().to_string()));
        rows.push(("".into(), "".into()));

        if data.lines.is_empty() {
            rows.push(("".into(), "(no log entries)".into()));
        } else {
            // Show last 30 lines (or from scroll_offset)
            let start = if data.lines.len() > 30 {
                data.scroll_offset.min(data.lines.len().saturating_sub(30))
            } else {
                0
            };
            let end = (start + 30).min(data.lines.len());
            for line in &data.lines[start..end] {
                rows.push(("".into(), line.clone()));
            }
        }
    } else {
        rows.push(("".into(), "(logs not available)".into()));
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 80pct,
                max_height: 85pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::INFO,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                Text(
                    content: "Logs",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<10} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "f ", color: theme::ACCENT_BRIGHT)
                    Text(content: "follow  ", color: theme::MUTED)
                    Text(content: "s ", color: theme::ACCENT_BRIGHT)
                    Text(content: "switch source", color: theme::MUTED)
                }
            }
        }
    }
}
