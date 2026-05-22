// ── Credential request dialog — gateway needs an API key ─────────────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::CredentialRequestData;

#[derive(Default, Props)]
pub struct CredentialRequestDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: CredentialRequestData,
}

#[component]
pub fn CredentialRequestDialog(
    props: &CredentialRequestDialogProps,
) -> impl Into<AnyElement<'static>> {
    let mask = props.data.masked_input();
    let cursor = "▏";

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70,
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
                    content: format!("🔑 Credential Required — {}", props.data.provider),
                    color: theme::WARN,
                    weight: Weight::Bold,
                )

                View(height: 1)

                Text(
                    content: format!("Requested secret: {}", props.data.secret_name),
                    color: theme::MUTED,
                    wrap: TextWrap::Wrap,
                )

                View(height: 1)

                // Message
                Text(
                    content: props.data.message.clone(),
                    color: theme::TEXT,
                    wrap: TextWrap::Wrap,
                )

                View(height: 1)

                // Masked input field
                Text(content: "Enter API key:", color: theme::MUTED)
                View(
                    border_style: BorderStyle::Round,
                    border_color: theme::ACCENT,
                    padding_left: 1,
                    padding_right: 1,
                ) {
                    Text(
                        content: format!("{}{}", mask, cursor),
                        color: theme::TEXT,
                    )
                }

                View(height: 1)

                // Hint
                Text(
                    content: "Enter to submit · Esc to dismiss",
                    color: theme::MUTED,
                )
            }
        }
    }
}
