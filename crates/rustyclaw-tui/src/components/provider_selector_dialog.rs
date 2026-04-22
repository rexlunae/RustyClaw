// ── Provider selector dialog — pick a model provider ────────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct ProviderSelectorDialogProps {
    /// Display names of available providers.
    pub providers: Vec<String>,
    /// Provider IDs (parallel to `providers`).
    pub provider_ids: Vec<String>,
    /// Auth methods — "apikey", "deviceflow", "none" (parallel).
    pub auth_hints: Vec<String>,
    /// Currently highlighted index.
    pub cursor: usize,
}

#[component]
pub fn ProviderSelectorDialog(
    props: &ProviderSelectorDialogProps,
) -> impl Into<AnyElement<'static>> {
    let items: Vec<AnyElement> = props
        .providers
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let selected = i == props.cursor;
            let indicator = if selected { "▸ " } else { "  " };
            let color = if selected {
                theme::ACCENT_BRIGHT
            } else {
                theme::TEXT
            };
            // Show auth hint badge
            let badge = props
                .auth_hints
                .get(i)
                .map(|h| match h.as_str() {
                    "apikey" => " 🔑",
                    "deviceflow" => " 🔗",
                    "none" => " ✓",
                    _ => "",
                })
                .unwrap_or("");

            element! {
                Text(content: format!("{}{}{}", indicator, name, badge), color: color)
            }
            .into_any()
        })
        .collect();

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 52,
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
                    content: "🌐 Select Provider",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )
                View(height: 1)
                #(items)
                View(height: 1)
                Text(
                    content: "↑↓ navigate  ·  Enter select  ·  Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
