// ── Tools dialog — tool enable/disable configuration overlay ────────────────
//
// Opened with /tools; Esc closes. Shows tools grouped by category with
// enable/disable toggles and policy badges.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct ToolsDialogProps {
    pub data: Option<rustyclaw_view::ToolConfigPanelData>,
}

#[component]
pub fn ToolsDialog(props: &ToolsDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.tools.is_empty() {
            rows.push(("".into(), "(no tools registered)".into()));
        } else {
            rows.push((
                "Summary".into(),
                format!(
                    "{} enabled / {} total",
                    data.enabled_count(),
                    data.total_count(),
                ),
            ));
            rows.push(("".into(), "".into()));

            let filtered = data.filtered_tools();
            for (i, tool) in filtered.iter().enumerate() {
                let marker = if Some(i) == data.selected { ">" } else { " " };
                let toggle = if tool.enabled { "[ON]" } else { "[--]" };

                rows.push((
                    format!("{} {} {}", marker, toggle, tool.name),
                    format!(
                        "{} {} [{}]",
                        tool.category_icon(),
                        tool.category,
                        tool.policy
                    ),
                ));
            }
        }
    } else {
        rows.push(("".into(), "(tool registry not available)".into()));
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70pct,
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
                    content: "Tool Configuration",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<24} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "Space ", color: theme::ACCENT_BRIGHT)
                    Text(content: "toggle  ", color: theme::MUTED)
                    Text(content: "p ", color: theme::ACCENT_BRIGHT)
                    Text(content: "cycle policy", color: theme::MUTED)
                }
            }
        }
    }
}
