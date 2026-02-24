// ── Sidebar ─────────────────────────────────────────────────────────────────

use iocraft::prelude::*;
use crate::theme;

#[derive(Default, Props)]
pub struct SidebarProps {
    pub gateway_label: String,
    pub task_text: String,
    pub streaming: bool,
    pub elapsed: String,
}

#[component]
pub fn Sidebar(props: &SidebarProps) -> impl Into<AnyElement<'static>> {
    element! {
        View(
            width: 24,
            height: 100pct,
            flex_direction: FlexDirection::Column,
            border_style: BorderStyle::Round,
            border_color: theme::MUTED,
            border_edges: Edges::Left,
            padding_left: 1,
            padding_right: 1,
        ) {
            // Session
            Text(content: " Session", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
            View(margin_top: 1) {
                Text(content: format!("Status: {}", props.gateway_label), color: theme::TEXT_DIM)
            }

            // Tasks
            View(margin_top: 1) {
                Text(content: " Tasks", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
            }
            View(margin_top: 1) {
                #(if props.streaming {
                    element! {
                        View(flex_direction: FlexDirection::Row) {
                            Text(content: "⠋ ", color: theme::ACCENT)
                            Text(content: format!("Streaming {}", props.elapsed), color: theme::TEXT_DIM)
                        }
                    }.into_any()
                } else {
                    element! {
                        Text(content: &props.task_text, color: theme::MUTED)
                    }.into_any()
                })
            }
        }
    }
}
