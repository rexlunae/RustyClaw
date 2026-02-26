// ── Skills dialog — interactive skill list overlay ──────────────────────────

use crate::theme;
use iocraft::prelude::*;

#[derive(Debug, Clone, Default)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Default, Props)]
pub struct SkillsDialogProps {
    pub skills: Vec<SkillInfo>,
    pub selected: Option<usize>,
    pub scroll_offset: usize,
}

#[component]
pub fn SkillsDialog(props: &SkillsDialogProps) -> impl Into<AnyElement<'static>> {
    let count = props.skills.len();
    let enabled = props.skills.iter().filter(|s| s.enabled).count();
    let disabled = count - enabled;
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
                    content: "⚡ Skills",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )

                View(height: 1)

                // Summary
                View(flex_direction: FlexDirection::Row) {
                    Text(content: format!("{} skill{}  │  ", count, if count == 1 { "" } else { "s" }), color: theme::TEXT_DIM)
                    Text(content: format!("{} enabled  ", enabled), color: theme::SUCCESS)
                    Text(content: format!("{} disabled", disabled), color: theme::MUTED)
                }

                View(height: 1)

                // Skill list
                #(if props.skills.is_empty() {
                    element! {
                        Text(content: "  No skills loaded.", color: theme::MUTED)
                    }.into_any()
                } else {
                    element! {
                        View(
                            flex_direction: FlexDirection::Column,
                            width: 100pct,
                            overflow: Overflow::Hidden,
                        ) {
                            #(props.skills.iter().enumerate().skip(props.scroll_offset).take(20).map(|(i, s)| {
                                let is_selected = i == sel;
                                let icon = if s.enabled { "✓" } else { "✗" };
                                let desc = if s.description.is_empty() {
                                    "No description".to_string()
                                } else {
                                    s.description.clone()
                                };
                                let bg = if is_selected { Some(theme::ACCENT_BRIGHT) } else { None };
                                let pointer = if is_selected { "▸ " } else { "  " };
                                let fg = if is_selected {
                                    theme::BG_MAIN
                                } else if s.enabled {
                                    theme::ACCENT_BRIGHT
                                } else {
                                    theme::TEXT_DIM
                                };
                                let line = format!("{}{} {} — {}", pointer, icon, s.name, desc);
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
                    }.into_any()
                })

                View(height: 1)

                // Hint
                View(flex_direction: FlexDirection::Row) {
                    Text(content: "↑↓ ", color: theme::ACCENT_BRIGHT)
                    Text(content: "navigate  ", color: theme::MUTED)
                    Text(content: "Enter ", color: theme::ACCENT_BRIGHT)
                    Text(content: "toggle  ", color: theme::MUTED)
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close", color: theme::MUTED)
                }
            }
        }
    }
}
