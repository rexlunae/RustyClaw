// ── Vault unlock dialog — password entry overlay ────────────────────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::VaultUnlockData;

#[derive(Default, Props)]
pub struct VaultUnlockDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: VaultUnlockData,
}

#[component]
pub fn VaultUnlockDialog(props: &VaultUnlockDialogProps) -> impl Into<AnyElement<'static>> {
    let dots = props.data.masked_password();
    let cursor = if props.data.password_len == 0 {
        "▏"
    } else {
        ""
    };
    let has_error = !props.data.error.is_empty();

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 48,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::WARN,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                // Title
                Text(
                    content: "🔒 Vault Locked",
                    color: theme::WARN,
                    weight: Weight::Bold,
                )

                View(height: 1)

                Text(
                    content: "Enter your vault password:",
                    color: theme::TEXT,
                )

                View(height: 1)

                // Password display (masked)
                View(
                    flex_direction: FlexDirection::Row,
                    border_style: BorderStyle::Single,
                    border_color: theme::ACCENT,
                    padding_left: 1,
                    padding_right: 1,
                ) {
                    Text(
                        content: format!("{}{}", dots, cursor),
                        color: theme::ACCENT_BRIGHT,
                    )
                }

                // Error message
                #(if has_error {
                    element! {
                        View(margin_top: 1) {
                            Text(content: props.data.error.clone(), color: theme::ERROR)
                        }
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                View(height: 1)

                Text(
                    content: "Enter ↩ submit  ·  Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
