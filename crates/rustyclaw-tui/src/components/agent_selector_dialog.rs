// ── Agent selector dialog — switch between agents in this installation ──────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::AgentSelectorData;

#[derive(Default, Props)]
pub struct AgentSelectorDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: AgentSelectorData,
}

#[component]
pub fn AgentSelectorDialog(props: &AgentSelectorDialogProps) -> impl Into<AnyElement<'static>> {
    let items: Vec<AnyElement> = props
        .data
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let selected = i == props.data.cursor;
            let active = agent.id == props.data.active_id;
            let indicator = if selected { "▸ " } else { "  " };
            let badge = if active { " (active)" } else { "" };
            let color = if selected {
                theme::ACCENT_BRIGHT
            } else if active {
                theme::ACCENT
            } else {
                theme::TEXT
            };
            let desc = agent
                .description
                .as_deref()
                .map(|d| format!(" — {}", d))
                .unwrap_or_default();

            element! {
                Text(
                    content: format!("{}{}{}{}", indicator, agent.name, badge, desc),
                    color: color,
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
                width: 60,
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
                    content: "🤖 Switch Agent",
                    color: theme::ACCENT_BRIGHT,
                    weight: Weight::Bold,
                )
                View(height: 1)
                #(items)
                View(height: 1)
                Text(
                    content: "↑↓ navigate  ·  Enter switch  ·  Esc cancel",
                    color: theme::MUTED,
                )
                Text(
                    content: "/agents new <name> creates an agent",
                    color: theme::MUTED,
                )
            }
        }
    }
}
