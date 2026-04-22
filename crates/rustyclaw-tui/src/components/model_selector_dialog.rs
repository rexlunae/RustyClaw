// ── Model selector dialog — pick a model from the fetched list ───────────────

use crate::theme;
use iocraft::prelude::*;

pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Default, Props)]
pub struct ModelSelectorDialogProps {
    /// Provider display name.
    pub provider_display: String,
    /// Available model names.
    pub models: Vec<String>,
    /// Currently highlighted index.
    pub cursor: usize,
    /// Whether we're still loading models.
    pub loading: bool,
    /// Spinner tick for loading animation.
    pub spinner_tick: usize,
}

#[component]
pub fn ModelSelectorDialog(props: &ModelSelectorDialogProps) -> impl Into<AnyElement<'static>> {
    let loading = props.loading;

    // Show at most 15 models around cursor to keep the dialog compact
    let max_visible = 15usize;
    let total = props.models.len();
    let (start, end) = if total <= max_visible {
        (0, total)
    } else {
        let half = max_visible / 2;
        let start = props.cursor.saturating_sub(half);
        let end = (start + max_visible).min(total);
        let start = if end == total {
            total.saturating_sub(max_visible)
        } else {
            start
        };
        (start, end)
    };

    let items: Vec<AnyElement> = if loading {
        let spinner = SPINNER[props.spinner_tick % SPINNER.len()];
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
        props.models[start..end]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let real_i = start + i;
                let selected = real_i == props.cursor;
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
    let scroll_hint = if total > max_visible {
        format!("  ({}/{})", props.cursor + 1, total)
    } else {
        String::new()
    };

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
                    content: format!("📦 Select Model — {}{}", props.provider_display, scroll_hint),
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
