// ── Provider selector dialog — pick a model provider ────────────────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::ProviderSelectorData;

#[derive(Default, Props)]
pub struct ProviderSelectorDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: ProviderSelectorData,
}

#[component]
pub fn ProviderSelectorDialog(
    props: &ProviderSelectorDialogProps,
) -> impl Into<AnyElement<'static>> {
    let items: Vec<AnyElement> = props
        .data
        .providers
        .iter()
        .enumerate()
        .map(|(i, provider)| {
            let selected = i == props.data.cursor;
            let indicator = if selected { "▸ " } else { "  " };
            let color = if selected {
                theme::ACCENT_BRIGHT
            } else {
                theme::TEXT
            };

            element! {
                Text(
                    content: format!(
                        "{}{}{}",
                        indicator,
                        provider.display_name,
                        provider.auth_badge()
                    ),
                    color: color
                )
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
