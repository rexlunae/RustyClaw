// ── Command menu ────────────────────────────────────────────────────────────
//
// Floating completion popup for `/` slash commands. Rendered just above the
// input bar with the list of matching commands and a highlighted selection.

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct CommandMenuProps {
    /// The filtered list of matching command names (without the `/` prefix).
    pub completions: Vec<String>,
    /// Index of the currently highlighted entry (None ⇒ nothing selected).
    pub selected: Option<usize>,
}

#[component]
pub fn CommandMenu(props: &CommandMenuProps) -> impl Into<AnyElement<'static>> {
    if props.completions.is_empty() {
        return element! { View() }.into_any();
    }

    let max_rows = props.completions.len().min(12) as u32;

    element! {
        View(
            width: 100pct,
            flex_direction: FlexDirection::Column,
            max_height: max_rows + 2, // rows + top/bottom border
            border_style: BorderStyle::Round,
            border_color: theme::ACCENT,
            background_color: theme::BG_SURFACE,
        ) {
            #(props.completions.iter().enumerate().take(max_rows as usize).map(|(i, cmd)| {
                let is_selected = props.selected == Some(i);
                let bg = if is_selected { theme::ACCENT_DIM } else { theme::BG_SURFACE };
                let fg = if is_selected { theme::ACCENT_BRIGHT } else { theme::TEXT };
                element! {
                    View(
                        key: i as u64,
                        width: 100pct,
                        background_color: bg,
                        padding_left: 1,
                    ) {
                        Text(
                            content: format!("/{}", cmd),
                            color: fg,
                        )
                    }
                }
            }))
        }
    }
    .into_any()
}
