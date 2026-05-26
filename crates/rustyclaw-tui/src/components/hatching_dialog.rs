// ── Hatching dialog — first-run agent naming ────────────────────────────────
//
// Shown on first launch when SOUL.md has not yet been customised.
// A single prompt asks the user to name their agent; pressing Enter
// saves a personalised SOUL.md and dismisses the dialog.

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct HatchingDialogProps {
    /// Text typed so far for the agent name.
    pub name_input: String,
}

#[component]
pub fn HatchingDialog(props: &HatchingDialogProps) -> impl Into<AnyElement<'static>> {
    let cursor = if props.name_input.is_empty() {
        "█".to_string()
    } else {
        format!("{}█", props.name_input)
    };

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            background_color: theme::BG_MAIN,
        ) {
            View(
                width: 60,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::ACCENT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                align_items: AlignItems::Center,
            ) {
                Text(
                    content: "🥚  First run — name your agent",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                Text(
                    content: "What should your agent be called?",
                    color: theme::TEXT,
                )

                View(height: 1)

                // Name input with cursor
                View(
                    width: 100pct,
                    border_style: BorderStyle::Single,
                    border_color: theme::ACCENT,
                    padding_left: 1,
                    padding_right: 1,
                ) {
                    Text(content: cursor, color: theme::TEXT)
                }

                View(height: 1)

                Text(
                    content: "Enter to confirm · Esc to skip",
                    color: theme::MUTED,
                )
            }
        }
    }
}

