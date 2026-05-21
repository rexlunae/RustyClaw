// ── Sidebar ─────────────────────────────────────────────────────────────────
//
// Simplified sidebar: status info + spacer. The thread list has been
// replaced by a horizontal ThreadTabs component in the main area.

use crate::theme;
use iocraft::prelude::*;

/// Braille spinner frames for smooth animation.
const SPINNER_FRAMES: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];

#[derive(Default, Props)]
pub struct SidebarProps {
    pub gateway_label: String,
    pub surface: rustyclaw_view::ChatSurfaceData,
}

#[component]
pub fn Sidebar(props: &SidebarProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER_FRAMES[props.surface.spinner_tick % SPINNER_FRAMES.len()];

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
            View(margin_top: 1) {
                Text(content: format!("Task: {}", props.surface.task_label()), color: theme::TEXT_DIM)
            }

            // Streaming indicator
            #(if props.surface.is_streaming || props.surface.is_thinking {
                element! {
                    View(margin_top: 1, flex_direction: FlexDirection::Row) {
                        Text(content: format!("{} ", spinner), color: theme::ACCENT)
                        Text(
                            content: format!(
                                "Streaming {}",
                                props.surface.elapsed.as_deref().unwrap_or("")
                            ),
                            color: theme::TEXT_DIM,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // Spacer
            View(flex_grow: 1.0)
        }
    }
}
