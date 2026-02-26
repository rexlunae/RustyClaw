// ── Tool permissions dialog — interactive tool permission control ────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Debug, Clone, Default)]
pub struct ToolPermInfo {
    pub name: String,
    pub permission: String,
    pub summary: String,
}

#[derive(Default, Props)]
pub struct ToolPermsDialogProps {
    pub tools: Vec<ToolPermInfo>,
    pub selected: Option<usize>,
    pub scroll_offset: usize,
}

#[component]
pub fn ToolPermsDialog(props: &ToolPermsDialogProps) -> impl Into<AnyElement<'static>> {
    let total = props.tools.len();
    let allowed = props
        .tools
        .iter()
        .filter(|t| t.permission == "ALLOW")
        .count();
    let denied = props
        .tools
        .iter()
        .filter(|t| t.permission == "DENY")
        .count();
    let ask = props.tools.iter().filter(|t| t.permission == "ASK").count();
    let sel = props.selected.unwrap_or(0);

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70pct,
                max_height: 80pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::ACCENT_BRIGHT,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                // Title
                Text(
                    content: "🔧 Tool Permissions",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Summary
                View(flex_direction: FlexDirection::Row) {
                    Text(content: format!("{} tools  │  ", total), color: theme::TEXT_DIM)
                    Text(content: format!("{} allow  ", allowed), color: theme::SUCCESS)
                    Text(content: format!("{} ask  ", ask), color: theme::WARN)
                    Text(content: format!("{} deny", denied), color: theme::ERROR)
                }

                View(height: 1)

                // Tool list
                View(
                    flex_direction: FlexDirection::Column,
                    width: 100pct,
                    overflow: Overflow::Hidden,
                ) {
                    #(props.tools.iter().enumerate().skip(props.scroll_offset).take(20).map(|(i, t)| {
                        let is_selected = i == sel;
                        let bg = if is_selected { Some(theme::ACCENT_BRIGHT) } else { None };
                        let pointer = if is_selected { "▸ " } else { "  " };
                        let fg = if is_selected { theme::BG_MAIN } else { theme::TEXT };
                        let line = format!("{} {:5}  {} — {}", pointer, t.permission, t.name, t.summary);
                        element! {
                            View(
                                key: i as u64,
                                width: 100pct,
                                background_color: bg.unwrap_or(Color::Reset),
                            ) {
                                Text(content: line, color: fg, wrap: TextWrap::NoWrap)
                            }
                        }
                    }))
                }

                View(height: 1)

                // Hint
                View(flex_direction: FlexDirection::Row) {
                    Text(content: "↑↓ ", color: theme::ACCENT_BRIGHT)
                    Text(content: "navigate  ", color: theme::MUTED)
                    Text(content: "Enter ", color: theme::ACCENT_BRIGHT)
                    Text(content: "cycle permission  ", color: theme::MUTED)
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close", color: theme::MUTED)
                }
            }
        }
    }
}
