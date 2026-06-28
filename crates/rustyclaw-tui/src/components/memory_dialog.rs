// ── Memory dialog — memory browser overlay ──────────────────────────────────
//
// Opened with /memory; Esc closes. Shows searchable memory entries
// with category labels and relevance scores.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct MemoryDialogProps {
    pub data: Option<rustyclaw_view::MemoryPanelData>,
}

#[component]
pub fn MemoryDialog(props: &MemoryDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.entries.is_empty() {
            rows.push(("".into(), "(no memory entries)".into()));
        } else {
            rows.push(("Entries".into(), format!("{} total", data.count())));
            if !data.search_query.is_empty() {
                rows.push(("Search".into(), data.search_query.clone()));
            }
            rows.push(("".into(), "".into()));

            for (i, entry) in data.entries.iter().enumerate() {
                let marker = if Some(i) == data.selected { ">" } else { " " };
                let score = entry
                    .score_display()
                    .map(|s| format!(" ({})", s))
                    .unwrap_or_default();

                rows.push((
                    format!("{} [{}]{}", marker, entry.category_label(), score),
                    entry.preview(60).to_string(),
                ));
            }
        }

        if let Some(ref status) = data.status {
            rows.push(("".into(), "".into()));
            rows.push(("Status".into(), status.clone()));
        }
    } else {
        rows.push(("".into(), "(memory system not initialised)".into()));
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
                max_height: 80pct,
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
                    content: "Memory Browser",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<16} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "/ ", color: theme::ACCENT_BRIGHT)
                    Text(content: "search  ", color: theme::MUTED)
                    Text(content: "e ", color: theme::ACCENT_BRIGHT)
                    Text(content: "edit  ", color: theme::MUTED)
                    Text(content: "d ", color: theme::ACCENT_BRIGHT)
                    Text(content: "delete", color: theme::MUTED)
                }
            }
        }
    }
}
