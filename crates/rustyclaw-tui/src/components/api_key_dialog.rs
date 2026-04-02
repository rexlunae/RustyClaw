// ── API key input dialog — masked key entry with provider help ───────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct ApiKeyDialogProps {
    /// Display name of the provider (e.g. "Anthropic (Claude)").
    pub provider_display: String,
    /// Number of characters typed so far.
    pub input_len: usize,
    /// URL where the user can get an API key (shown as help).
    pub help_url: String,
    /// Short help text (e.g. "Get a key at console.anthropic.com").
    pub help_text: String,
}

#[component]
pub fn ApiKeyDialog(props: &ApiKeyDialogProps) -> impl Into<AnyElement<'static>> {
    let mask = if props.input_len == 0 {
        "·".repeat(20)
    } else {
        format!("{}{}", "•".repeat(props.input_len), "·".repeat(20usize.saturating_sub(props.input_len)))
    };

    let has_help = !props.help_text.is_empty();
    let has_url = !props.help_url.is_empty();

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 56,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::WARN,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                Text(
                    content: format!("🔑 API Key — {}", props.provider_display),
                    color: theme::WARN,
                    weight: Weight::Bold,
                )
                View(height: 1)

                // Help text
                #(if has_help {
                    element! {
                        Text(content: props.help_text.clone(), color: theme::MUTED)
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })
                #(if has_url {
                    element! {
                        Text(content: props.help_url.clone(), color: theme::INFO)
                    }.into_any()
                } else {
                    element! { View() }.into_any()
                })
                #(if has_help || has_url {
                    element! { View(height: 1) }.into_any()
                } else {
                    element! { View() }.into_any()
                })

                Text(content: "Paste or type your API key:", color: theme::TEXT)
                View(height: 1)

                // Masked input
                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: format!("  {}  ", mask),
                        color: theme::ACCENT_BRIGHT,
                        weight: Weight::Bold,
                    )
                }

                View(height: 1)
                Text(
                    content: "Enter ↩ submit  ·  Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
