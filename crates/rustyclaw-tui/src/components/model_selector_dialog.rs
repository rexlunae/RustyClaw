// ── Model selector dialog — pick a model from the fetched list ───────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::ModelSelectorData;

pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Default, Props)]
pub struct ModelSelectorDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: ModelSelectorData,
}

#[component]
pub fn ModelSelectorDialog(props: &ModelSelectorDialogProps) -> impl Into<AnyElement<'static>> {
    let loading = props.data.loading;
    let max_visible = 15usize;
    let total = props.data.models.len();
    let (start, end) = props.data.visible_window(max_visible);

    let items: Vec<AnyElement> = if loading {
        let spinner = SPINNER[props.data.spinner_tick % SPINNER.len()];
        vec![
            element! {
                Text(
                    content: format!("{} Fetching models…", spinner),
                    color: theme::MUTED,
                )
            }
            .into_any(),
        ]
    } else if total == 0 {
        vec![
            element! {
                Text(content: "No models found.", color: theme::MUTED)
            }
            .into_any(),
        ]
    } else {
        props.data.models[start..end]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let real_i = start + i;
                let selected = real_i == props.data.cursor;
                let indicator = if selected { "▸ " } else { "  " };
                let color = if selected {
                    theme::ACCENT_BRIGHT
                } else {
                    theme::TEXT
                };
                element! {
                    Text(content: format!("{}{}", indicator, name), color: color)
                }
                .into_any()
            })
            .collect()
    };

    // Scroll indicator
    let scroll_hint = props.data.scroll_hint(max_visible);

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
                border_color: theme::ACCENT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                Text(
                    content: format!(
                        "📦 Select Model — {}{}",
                        props.data.provider_display, scroll_hint
                    ),
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )
                View(height: 1)
                #(items)
                View(height: 1)
                Text(
                    content: if loading { "Esc cancel" } else { "↑↓ navigate  ·  Enter select  ·  Esc cancel" },
                    color: theme::MUTED,
                )
            }
        }
    }
}
