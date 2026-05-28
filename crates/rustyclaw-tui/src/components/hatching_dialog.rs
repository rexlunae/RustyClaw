// ── Hatching dialog — first-run agent setup ─────────────────────────────────
//
// Shown on first launch when SOUL.md has not yet been customised.
// One screen with two fields: agent name (required) and a brief personality
// description (optional).  Tab switches focus; Enter on either field confirms.

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::HatchingDialogData;

#[derive(Default, Props)]
pub struct HatchingDialogProps {
    pub data: HatchingDialogData,
}

fn field_with_cursor(text: &str, focused: bool) -> String {
    if focused {
        if text.is_empty() {
            "█".to_string()
        } else {
            format!("{}█", text)
        }
    } else {
        text.to_string()
    }
}

#[component]
pub fn HatchingDialog(props: &HatchingDialogProps) -> impl Into<AnyElement<'static>> {
    let name_focused = props.data.name_focused();
    let name_text = field_with_cursor(&props.data.name_input, name_focused);
    let personality_text = field_with_cursor(&props.data.personality_input, !name_focused);

    let name_border_color = if name_focused {
        theme::ACCENT
    } else {
        theme::MUTED
    };
    let personality_border_color = if !name_focused {
        theme::ACCENT
    } else {
        theme::MUTED
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
                width: 62,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::ACCENT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                Text(
                    content: "🥚  Set up your agent",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Name field
                Text(content: "Name  (required)", color: theme::TEXT)
                View(height: 0)
                View(
                    width: 100pct,
                    border_style: BorderStyle::Single,
                    border_color: name_border_color,
                    padding_left: 1,
                    padding_right: 1,
                ) {
                    Text(content: name_text, color: theme::TEXT)
                }

                View(height: 1)

                // Personality field
                Text(content: "Personality  (optional)", color: theme::TEXT)
                View(height: 0)
                View(
                    width: 100pct,
                    border_style: BorderStyle::Single,
                    border_color: personality_border_color,
                    padding_left: 1,
                    padding_right: 1,
                    padding_top: 0,
                    padding_bottom: 0,
                ) {
                    Text(
                        content: personality_text,
                        color: theme::TEXT,
                        wrap: TextWrap::Wrap,
                    )
                }

                View(height: 1)

                Text(
                    content: "Tab to switch field · Enter to confirm · Esc to skip",
                    color: theme::MUTED,
                )
            }
        }
    }
}
