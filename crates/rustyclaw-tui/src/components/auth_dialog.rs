// ── Auth dialog — TOTP code entry overlay ───────────────────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct AuthDialogProps {
    /// The digits entered so far (0–6 characters).
    pub code: String,
    /// Optional error/retry message to display.
    pub error: String,
}

#[component]
pub fn AuthDialog(props: &AuthDialogProps) -> impl Into<AnyElement<'static>> {
    // Build the display: show typed digits + placeholder dots
    let typed = &props.code;
    let remaining = 6usize.saturating_sub(typed.len());
    let display = format!("{}{}", typed, "·".repeat(remaining),);

    let has_error = !props.error.is_empty();

    element! {
        // Full-screen overlay with semi-transparent feel
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            // Dialog box
            View(
                width: 48,
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
                    content: "🔑 Gateway Authentication",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                // Spacer
                View(height: 1)

                // Label
                Text(
                    content: "Enter your 6-digit TOTP code:",
                    color: theme::TEXT,
                )

                // Spacer
                View(height: 1)

                // Code display
                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: format!("  {}  ", display),
                        color: theme::ACCENT_BRIGHT,
                        weight: Weight::Bold,
                    )
                }

                // Error message (if any)
                #(if has_error {
                    element! {
                        View(margin_top: 1) {
                            Text(content: props.error.clone(), color: theme::ERROR)
                        }
                    }.into_any()
                } else {
                    element! {
                        View()
                    }.into_any()
                })

                // Spacer
                View(height: 1)

                // Hint
                Text(
                    content: "Enter ↩ submit  ·  Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
