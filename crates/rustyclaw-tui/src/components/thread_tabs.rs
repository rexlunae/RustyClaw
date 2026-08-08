// ── ThreadTabs — vertical two-level project → thread sidebar ────────────────
//
// The tab bar was replaced by a single left sidebar: projects are headers and
// their threads are listed (indented) beneath. `threads` is assumed to be
// grouped by project (contiguous, project order) and `selected` indexes into
// it directly, so keyboard navigation stays a simple flat index.

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_core::ui::ProjectInfo;
use rustyclaw_view::SidebarItemData;

#[derive(Default, Props)]
pub struct ThreadTabsProps {
    /// Threads in project-grouped order (each `project_id` is its effective
    /// group), so a header can be emitted whenever it changes.
    pub threads: Vec<SidebarItemData>,
    pub projects: Vec<ProjectInfo>,
    pub active_project_id: u64,
    pub focused: bool,
    pub selected: usize,
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
            pinned: bool,
            needs_input: bool,
        },
        Thread {
            label: String,
            icon: &'static str,
            active: bool,
            selected: bool,
            pinned: bool,
            needs_input: bool,
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
                pinned: props
                    .projects
                    .iter()
                    .any(|p| p.id == t.project_id && p.pinned),
                needs_input: props
                    .threads
                    .iter()
                    .filter(|s| s.project_id == t.project_id)
                    .any(SidebarItemData::needs_input),
            });
            last_project = Some(t.project_id);
        }
        rows.push(Row::Thread {
            label: t.truncated_label(20).into_owned(),
            icon: t.state.icon(),
            active: t.is_foreground,
            selected: props.focused && idx == props.selected,
            pinned: t.pinned,
            needs_input: t.needs_input(),
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
                    Row::Header { name, active, pinned, needs_input } => element! {
                        View(margin_top: 1) {
                            Text(
                                content: format!(
                                    "{} {}{}",
                                    if active { "▾" } else { "▸" },
                                    if pinned { "📌 " } else { "" },
                                    name,
                                ),
                                color: if needs_input {
                                    theme::WARN
                                } else if active {
                                    theme::ACCENT
                                } else {
                                    theme::TEXT_DIM
                                },
                                weight: Weight::Bold,
                            )
                        }
                    }.into_any(),
                    Row::Thread { label, icon, active, selected, pinned, needs_input } => {
                        // The state icon leads (▶ working, ❓ asking, ○ ready,
                        // …); the row colour still marks the foreground. A
                        // thread parked on the user goes warning-coloured; a
                        // pinned one gets the pin marker.
                        let color = if needs_input {
                            theme::WARN
                        } else if active || selected {
                            theme::ACCENT
                        } else {
                            theme::TEXT_DIM
                        };
                        element! {
                            View(padding_left: 2) {
                                Text(
                                    content: format!(
                                        "{} {}{}",
                                        icon,
                                        if pinned { "📌 " } else { "" },
                                        label,
                                    ),
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
            View(flex_grow: 1.0_f32)
        }
    }
}
