// ── ThreadTabs — vertical two-level project → thread sidebar ────────────────
//
// The tab bar was replaced by a single left sidebar: projects are headers and
// their threads are listed (indented) beneath. `threads` is assumed to be
// grouped by project (contiguous, project order) and `selected` indexes into
// it directly, so keyboard navigation stays a simple flat index.

use crate::action::ThreadInfo;
use crate::theme;
use iocraft::prelude::*;
use rustyclaw_core::ui::ProjectInfo;

#[derive(Default, Props)]
pub struct ThreadTabsProps {
    pub threads: Vec<ThreadInfo>,
    pub projects: Vec<ProjectInfo>,
    pub active_project_id: u64,
    pub focused: bool,
    pub selected: usize,
}

/// Truncate to at most `max` chars, appending `…` when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    } else {
        s.to_string()
    }
}

#[component]
pub fn ThreadTabs(props: &ThreadTabsProps) -> impl Into<AnyElement<'static>> {
    let has_threads = !props.threads.is_empty();

    // Build the rendered rows: a project header whenever the project changes,
    // then each thread (carrying its flat index for selection highlighting).
    enum Row {
        Header {
            name: String,
            active: bool,
        },
        Thread {
            label: String,
            active: bool,
            selected: bool,
        },
    }

    let project_name = |id: u64| -> String {
        props
            .projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Project".to_string())
    };

    let mut rows: Vec<Row> = Vec::new();
    let mut last_project: Option<u64> = None;
    for (idx, t) in props.threads.iter().enumerate() {
        if last_project != Some(t.project_id) {
            rows.push(Row::Header {
                name: project_name(t.project_id),
                active: t.project_id == props.active_project_id,
            });
            last_project = Some(t.project_id);
        }
        rows.push(Row::Thread {
            label: truncate(t.label.as_str(), 20),
            active: t.is_foreground,
            selected: props.focused && idx == props.selected,
        });
    }

    element! {
        View(
            width: 26,
            height: 100pct,
            flex_direction: FlexDirection::Column,
            border_style: BorderStyle::Round,
            border_color: if props.focused { theme::ACCENT } else { theme::MUTED },
            border_edges: Edges::Right,
            padding_left: 1,
            padding_right: 1,
        ) {
            Text(content: " Projects", color: theme::ACCENT_BRIGHT, weight: Weight::Bold)
            #(if has_threads {
                rows.into_iter().map(|row| match row {
                    Row::Header { name, active } => element! {
                        View(margin_top: 1) {
                            Text(
                                content: format!("{} {}", if active { "▾" } else { "▸" }, name),
                                color: if active { theme::ACCENT } else { theme::TEXT_DIM },
                                weight: Weight::Bold,
                            )
                        }
                    }.into_any(),
                    Row::Thread { label, active, selected } => {
                        let indicator = if active { "▸" } else { "·" };
                        let color = if active || selected { theme::ACCENT } else { theme::TEXT_DIM };
                        element! {
                            View(padding_left: 2) {
                                Text(
                                    content: format!("{} {}", indicator, label),
                                    color: color,
                                    weight: if active || selected { Weight::Bold } else { Weight::Normal },
                                )
                            }
                        }.into_any()
                    }
                }).collect::<Vec<_>>()
            } else {
                vec![element! {
                    View(margin_top: 1) {
                        Text(content: "No threads", color: theme::MUTED)
                    }
                }.into_any()]
            })
            View(flex_grow: 1.0)
        }
    }
}
