// ── Sidebar ─────────────────────────────────────────────────────────────────

use crate::action::ThreadInfo;
use crate::theme;
use iocraft::prelude::*;

/// Braille spinner frames for smooth animation.
const SPINNER_FRAMES: [char; 8] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧'];

#[derive(Default, Props)]
pub struct SidebarProps {
    pub gateway_label: String,
    pub task_text: String,
    pub streaming: bool,
    pub elapsed: String,
    pub spinner_tick: usize,
    pub threads: Vec<ThreadInfo>,
    pub focused: bool,
    pub selected: usize,
}

#[component]
pub fn Sidebar(props: &SidebarProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER_FRAMES[props.spinner_tick % SPINNER_FRAMES.len()];
    let has_threads = !props.threads.is_empty();

    // Border color reflects focus state
    let border_color = if props.focused {
        theme::ACCENT
    } else {
        theme::MUTED
    };

    element! {
        View(
            width: 24,
            height: 100pct,
            flex_direction: FlexDirection::Column,
            border_style: BorderStyle::Round,
            border_color: border_color,
            border_edges: Edges::Left,
            padding_left: 1,
            padding_right: 1,
        ) {
            // Session
            Text(content: " Session", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
            View(margin_top: 1) {
                Text(content: format!("Status: {}", props.gateway_label), color: theme::TEXT_DIM)
            }

            // Streaming indicator
            #(if props.streaming {
                element! {
                    View(margin_top: 1, flex_direction: FlexDirection::Row) {
                        Text(content: format!("{} ", spinner), color: theme::ACCENT)
                        Text(content: format!("Streaming {}", props.elapsed), color: theme::TEXT_DIM)
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // Unified threads section (includes tasks)
            #(if has_threads {
                element! {
                    View(margin_top: 1, flex_direction: FlexDirection::Column) {
                        Text(content: " Threads", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                        View(margin_top: 1, flex_direction: FlexDirection::Column) {
                            #(props.threads.iter().enumerate().take(10).map(|(i, thread)| {
                                let is_selected = props.focused && i == props.selected;

                                // Use structured status_icon from gateway if available,
                                // otherwise fall back to string matching
                                let status_icon = if is_selected {
                                    "▸".to_string()
                                } else if let Some(ref icon) = thread.status_icon {
                                    icon.clone()
                                } else {
                                    match thread.status.as_deref() {
                                        Some("Running") => "▶",
                                        Some("Pending") => "◯",
                                        Some("Completed") => "✓",
                                        Some("Failed") => "✗",
                                        Some("Cancelled") => "⊘",
                                        Some("Paused") => "⏸",
                                        None if thread.is_foreground => "★",
                                        None if thread.has_summary => "⌁",
                                        _ => " ",
                                    }.to_string()
                                };

                                // Truncate label to fit sidebar width
                                let label = if thread.label.len() > 16 {
                                    format!("{}…", &thread.label[..15])
                                } else {
                                    thread.label.clone()
                                };

                                // Build description line if present
                                let desc = thread.description.as_ref().map(|d| {
                                    if d.len() > 20 {
                                        format!("{}…", &d[..19])
                                    } else {
                                        d.clone()
                                    }
                                });

                                element! {
                                    View(key: i as u64, flex_direction: FlexDirection::Column) {
                                        View(flex_direction: FlexDirection::Row) {
                                            Text(
                                                content: status_icon,
                                                color: if thread.is_foreground || is_selected { theme::ACCENT } else { theme::MUTED },
                                            )
                                            Text(
                                                content: format!(" {}", label),
                                                color: if is_selected { theme::TEXT } else { theme::TEXT_DIM },
                                                weight: if is_selected { Weight::Bold } else { Weight::Normal },
                                            )
                                        }
                                        #(if let Some(ref d) = desc {
                                            element! {
                                                Text(
                                                    content: format!("  {}", d),
                                                    color: theme::MUTED,
                                                )
                                            }.into_any()
                                        } else {
                                            element! { View() }.into_any()
                                        })
                                    }
                                }
                            }))
                            #(if props.threads.len() > 10 {
                                element! {
                                    Text(
                                        content: format!("  +{} more", props.threads.len() - 10),
                                        color: theme::MUTED,
                                    )
                                }.into_any()
                            } else {
                                element! { View() }.into_any()
                            })
                        }
                    }
                }.into_any()
            } else {
                element! {
                    View(margin_top: 1, flex_direction: FlexDirection::Column) {
                        Text(content: " Threads", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                        View(margin_top: 1) {
                            Text(content: &props.task_text, color: theme::MUTED)
                        }
                    }
                }.into_any()
            })

            // Spacer
            View(flex_grow: 1.0)
        }
    }
}
