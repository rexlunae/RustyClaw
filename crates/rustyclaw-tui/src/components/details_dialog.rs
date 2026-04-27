// ── Details dialog — extended error/warning details overlay ─────────────────
//
// Used to surface the structured fields attached to provider /
// model-fetch / device-flow errors (URL, status, redacted headers,
// body excerpt, full cause chain) when the user presses the details
// keybind on a warning or error toast.  The body is rendered as a
// scrollable bordered popup; PgUp/PgDn scroll, Esc closes.

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct DetailsDialogProps {
    /// Multi-line details text (already rendered by
    /// [`rustyclaw_core::error_details::render_extended`]).
    pub details: String,
    /// Whether this is for an error (vs. a warning) — drives the title
    /// and accent colour.
    pub is_error: bool,
    /// Vertical scroll offset, in lines.
    pub scroll_offset: usize,
}

#[component]
pub fn DetailsDialog(props: &DetailsDialogProps) -> impl Into<AnyElement<'static>> {
    let title = if props.is_error {
        "✖ Error details"
    } else {
        "⚠ Warning details"
    };
    let accent = if props.is_error {
        theme::ERROR
    } else {
        theme::WARN
    };

    let lines: Vec<String> = props.details.lines().map(|s| s.to_string()).collect();
    let total = lines.len();
    let visible: Vec<String> = if total == 0 {
        vec!["(no extended details)".to_string()]
    } else {
        lines.iter().skip(props.scroll_offset).cloned().collect()
    };

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 80pct,
                max_height: 80pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: accent,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                Text(
                    content: title.to_string(),
                    color: accent,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(visible.into_iter().enumerate().map(|(i, line)| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(content: line, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                // Scroll position + hint
                View(flex_direction: FlexDirection::Row) {
                    Text(
                        content: if total == 0 {
                            "no content".to_string()
                        } else {
                            format!(
                                "line {}/{}  ",
                                (props.scroll_offset + 1).min(total),
                                total,
                            )
                        },
                        color: theme::MUTED,
                    )
                    Text(content: "PgUp/PgDn ", color: theme::ACCENT_BRIGHT)
                    Text(content: "scroll  ", color: theme::MUTED)
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close", color: theme::MUTED)
                }
            }
        }
    }
}
