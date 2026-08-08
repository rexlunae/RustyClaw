// ── Auth dialog — TOTP code entry overlay ───────────────────────────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::AuthDialogData;

#[derive(Default, Props)]
pub struct AuthDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: AuthDialogData,
}

#[component]
pub fn AuthDialog(props: &AuthDialogProps) -> impl Into<AnyElement<'static>> {
    // Build the display: show typed digits + placeholder dots
    let typed = &props.data.code;
    let remaining = 6usize.saturating_sub(typed.len());
    let display = format!("{}{}", typed, "·".repeat(remaining),);

    let has_error = !props.data.error.is_empty();

    // TOTP codes rotate every 30 seconds. Show how much of the current
    // window is left so users don't submit a code that's about to expire —
    // the surrounding tick loop re-renders continuously, keeping this live.
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let remaining = 30 - (now_secs % 30);
    let countdown_color = if remaining <= 5 {
        theme::ERROR
    } else {
        theme::MUTED
    };
    let countdown_bar = format!(
        "{}{}  code expires in {remaining:>2}s",
        "▰".repeat(remaining as usize / 3),
        "▱".repeat(10 - remaining as usize / 3),
    );

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

                // Countdown for the current 30-second TOTP window
                View(
                    margin_top: 1,
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: countdown_bar,
                        color: countdown_color,
                    )
                }

                // Error message (if any)
                #(if has_error {
                    element! {
                        View(margin_top: 1) {
                            Text(content: props.data.error.clone(), color: theme::ERROR)
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
