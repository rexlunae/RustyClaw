// ── Sidebar ─────────────────────────────────────────────────────────────────

use crate::action::{TaskInfo, ThreadInfo};
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
    pub tasks: Vec<TaskInfo>,
    pub threads: Vec<ThreadInfo>,
    pub focused: bool,
    pub selected: usize,
}

#[component]
pub fn Sidebar(props: &SidebarProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER_FRAMES[props.spinner_tick % SPINNER_FRAMES.len()];
    let has_tasks = !props.tasks.is_empty();
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

            // Threads section (when available)
            #(if has_threads {
                element! {
                    View(margin_top: 1, flex_direction: FlexDirection::Column) {
                        Text(content: " Threads", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
                        View(margin_top: 1, flex_direction: FlexDirection::Column) {
                            #(props.threads.iter().enumerate().take(8).map(|(i, thread)| {
                                let is_selected = props.focused && i == props.selected;
                                let fg_marker = if thread.is_foreground { "★ " } else { "  " };
                                let summary_marker = if thread.has_summary { "⌁" } else { "" };
                                // Truncate label to fit
                                let label = if thread.label.len() > 15 {
                                    format!("{}…", &thread.label[..14])
                                } else {
                                    thread.label.clone()
                                };
                                element! {
                                    View(key: i as u64, flex_direction: FlexDirection::Row) {
                                        Text(
                                            content: if is_selected { "▸" } else { fg_marker }.to_string(),
                                            color: if thread.is_foreground || is_selected { theme::ACCENT } else { theme::MUTED },
                                        )
                                        Text(
                                            content: format!("{}{}", label, summary_marker),
                                            color: if is_selected { theme::TEXT } else { theme::TEXT_DIM },
                                            weight: if is_selected { Weight::Bold } else { Weight::Normal },
                                        )
                                    }
                                }
                            }))
                            #(if props.threads.len() > 8 {
                                element! {
                                    Text(
                                        content: format!("  +{} more", props.threads.len() - 8),
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
                element! { View() }.into_any()
            })

            // Tasks section
            View(margin_top: 1) {
                Text(content: " Tasks", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
            }

            // Show streaming indicator or task list
            View(margin_top: 1, flex_direction: FlexDirection::Column) {
                #(if props.streaming {
                    element! {
                        View(flex_direction: FlexDirection::Row) {
                            Text(content: format!("{} ", spinner), color: theme::ACCENT)
                            Text(content: format!("Streaming {}", props.elapsed), color: theme::TEXT_DIM)
                        }
                    }.into_any()
                } else if has_tasks {
                    element! {
                        View(flex_direction: FlexDirection::Column) {
                            #(props.tasks.iter().take(5).enumerate().map(|(i, task)| {
                                let status_icon = match task.status.as_str() {
                                    "Running" => "▶",
                                    "Pending" => "◯",
                                    "Completed" => "✓",
                                    "Failed" => "✗",
                                    "Cancelled" => "⊘",
                                    "Paused" => "⏸",
                                    _ => "•",
                                };
                                let fg_marker = if task.is_foreground { "★" } else { "" };
                                let desc = task.description.as_deref().unwrap_or(&task.label);
                                // Truncate description to fit sidebar
                                let truncated = if desc.len() > 18 {
                                    format!("{}…", &desc[..17])
                                } else {
                                    desc.to_string()
                                };
                                element! {
                                    View(key: i as u64, flex_direction: FlexDirection::Row) {
                                        Text(
                                            content: format!("{}{} ", status_icon, fg_marker),
                                            color: if task.is_foreground { theme::ACCENT } else { theme::MUTED },
                                        )
                                        Text(
                                            content: truncated,
                                            color: if task.is_foreground { theme::TEXT } else { theme::TEXT_DIM },
                                        )
                                    }
                                }
                            }))
                            #(if props.tasks.len() > 5 {
                                element! {
                                    Text(
                                        content: format!("  +{} more", props.tasks.len() - 5),
                                        color: theme::MUTED,
                                    )
                                }.into_any()
                            } else {
                                element! { View() }.into_any()
                            })
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
