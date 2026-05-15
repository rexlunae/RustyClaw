// ── ThreadTabs — Horizontal thread tab bar ─────────────────────────────────
//
// Replaces the thread list from the sidebar.  Uses shared view-layer types
// and minimal TUI-specific rendering.

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::TabBarData;

#[derive(Default, Props)]
pub struct ThreadTabsProps {
    pub data: TabBarData,
    pub focused: bool,
    pub selected: usize,
}

#[component]
pub fn ThreadTabs(props: &ThreadTabsProps) -> impl Into<AnyElement<'static>> {
    let total = props.data.len();
    let has_tabs = total > 0;

    // Compute the active tab index for highlighting.
    let active_idx = props.data.tabs.iter().position(|t| t.is_foreground);

    element! {
        View(
            flex_direction: FlexDirection::Row,
            width: 100pct,
            background_color: theme::BG_SURFACE,
        ) {
            #(if has_tabs {
                element! {
                    View(
                        flex_direction: FlexDirection::Row,
                        width: 100pct,
                    ) {
                        #(props.data.tabs.iter().enumerate().map(|(i, tab)| {
                            let is_active = active_idx == Some(i);
                            let is_selected = props.focused && i == props.selected;

                            let raw_label = tab.truncated_label(14);

                            // Prepend a pointer for active, a dot for inactive
                            let indicator = if is_active {
                                "▸".to_string()
                            } else {
                                "·".to_string()
                            };

                            let label_color = if is_active || is_selected {
                                theme::ACCENT
                            } else {
                                theme::TEXT_DIM
                            };

                            element! {
                                View(
                                    padding_left: 1,
                                    padding_right: 1,
                                ) {
                                    Text(
                                        content: format!(" {} {} ", indicator, raw_label),
                                        color: label_color,
                                        weight: if is_active || is_selected {
                                            Weight::Bold
                                        } else {
                                            Weight::Normal
                                        },
                                    )
                                }
                            }
                        }))
                    }
                }.into_any()
            } else {
                element! {
                    View(
                        flex_direction: FlexDirection::Row,
                        width: 100pct,
                        padding_left: 1,
                    ) {
                        Text(content: " No threads", color: theme::MUTED)
                    }
                }.into_any()
            })
        }
    }
}
