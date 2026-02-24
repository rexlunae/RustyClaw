// ── User prompt dialog — agent asks user a question ─────────────────────────

use iocraft::prelude::*;
use crate::theme;

#[derive(Default, Props)]
pub struct UserPromptDialogProps {
    /// The question title from the agent.
    pub title: String,
    /// Optional longer description.
    pub description: String,
    /// Current user input text.
    pub input: String,
}

#[component]
pub fn UserPromptDialog(props: &UserPromptDialogProps) -> impl Into<AnyElement<'static>> {
    let has_desc = !props.description.is_empty();
    let cursor = "▏";

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 60,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::ACCENT_BRIGHT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                // Title
                Text(
                    content: "❓ Agent Question",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Question text
                Text(
                    content: props.title.clone(),
                    color: theme::TEXT,
                    weight: Weight::Bold,
                )

                // Description (if any)
                #(if has_desc {
                    element! {
                        View(margin_top: 1) {
                            Text(
                                content: props.description.clone(),
                                color: theme::MUTED,
                            )
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                View(height: 1)

                // Input field
                Text(
                    content: "Your answer:",
                    color: theme::MUTED,
                )
                View(
                    flex_direction: FlexDirection::Row,
                    border_style: BorderStyle::Single,
                    border_color: theme::ACCENT,
                    padding_left: 1,
                    padding_right: 1,
                    min_height: 1u32,
                ) {
                    Text(
                        content: format!("{}{}", props.input, cursor),
                        color: theme::TEXT,
                    )
                }

                View(height: 1)

                // Hint
                Text(
                    content: "Enter ↩ submit  ·  Esc dismiss",
                    color: theme::MUTED,
                )
            }
        }
    }
}
