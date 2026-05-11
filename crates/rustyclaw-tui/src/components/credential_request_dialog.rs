// ── Credential request dialog — gateway needs an API key ─────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct CredentialRequestDialogProps {
    /// The provider that needs a credential (e.g. "openai", "anthropic").
    pub provider: String,
    /// Human-readable message explaining what is needed.
    pub message: String,
    /// Length of the current input (masked as dots).
    pub input_len: usize,
}

#[component]
pub fn CredentialRequestDialog(
    props: &CredentialRequestDialogProps,
) -> impl Into<AnyElement<'static>> {
    let mask = "•".repeat(props.input_len);
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
                    content: format!("🔑 Credential Required — {}", props.provider),
                    color: theme::WARN,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Message
                Text(
                    content: props.message.clone(),
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
