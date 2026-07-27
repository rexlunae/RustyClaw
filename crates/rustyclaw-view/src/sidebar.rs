//! Component data for the thread sidebar.
//!
//! The sidebar lists active threads/sessions and lets the user
//! switch between them.  This module provides the shared data
//! type for each sidebar item.

use std::borrow::Cow;

/// Data for a single item in the thread sidebar.
///
/// Rendered as a row showing the thread label, message count,
/// and an indicator for the currently active thread.
///
/// Methods on this struct centralise label formatting so both
/// the desktop (Dioxus) and TUI (iocraft) derive the same display
/// strings without duplicating fallback logic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidebarItemData {
    /// Thread / session ID.
    pub id: u64,

    /// Project this thread belongs to (0 = the active project).
    pub project_id: u64,

    /// Optional user-assigned label.
    pub label: Option<String>,

    /// Optional auto-generated description.
    pub description: Option<String>,

    /// Status string (e.g. "active", "idle").
    pub status: String,

    /// Whether this is the currently foregrounded thread.
    pub is_foreground: bool,

    /// Number of messages in the thread.
    pub message_count: usize,

    /// Working-directory override, or `None` when the thread inherits its
    /// project's directory.
    pub working_dir: Option<String>,
}

impl SidebarItemData {
    /// The resolved display label.
    ///
    /// Uses the user-assigned label when present, otherwise falls back
    /// to `"Session #{id}"`.
    pub fn display_label(&self) -> Cow<'_, str> {
        self.label
            .as_deref()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(format!("Session #{}", self.id)))
    }

    /// Label truncated to at most `max_chars` character boundaries.
    ///
    /// Uses [`char`] counting so multi-byte CJK / emoji characters
    /// aren't split.  Appends `"…"` when truncation occurs.
    pub fn truncated_label(&self, max_chars: usize) -> Cow<'_, str> {
        let label = self.display_label();
        if label.chars().count() > max_chars {
            let truncated: String = label.chars().take(max_chars.saturating_sub(1)).collect();
            Cow::Owned(format!("{}…", truncated))
        } else {
            label
        }
    }

    /// Full description text, truncated like [`truncated_label`](Self::truncated_label).
    pub fn truncated_description(&self, max_chars: usize) -> Cow<'_, str> {
        let Some(desc) = self.description.as_deref() else {
            return Cow::Borrowed("");
        };
        if desc.chars().count() > max_chars {
            let truncated: String = desc.chars().take(max_chars.saturating_sub(1)).collect();
            Cow::Owned(format!("{}…", truncated))
        } else {
            Cow::Borrowed(desc)
        }
    }

    /// The full title tooltip text (label + description joined by newline).
    pub fn title_text(&self) -> String {
        let label = self.display_label();
        match self.description.as_deref() {
            Some(desc) if !desc.is_empty() => format!("{label}\n{desc}"),
            _ => label.into_owned(),
        }
    }

    /// A brief status indicator character for display.
    ///
    /// Maps common status strings to recognisable symbols:
    /// - `"active"` / `"foreground"` → `"●"`
    /// - `"idle"` → `"○"`
    /// - `"error"` / `"failed"` → `"✕"`
    /// - Anything else → `"·"`
    pub fn status_dot(&self) -> &'static str {
        match self.status.as_str() {
            "active" | "foreground" => "●",
            "idle" => "○",
            "error" | "failed" => "✕",
            _ => "·",
        }
    }
}

impl From<&rustyclaw_core::ui::ThreadInfo> for SidebarItemData {
    fn from(t: &rustyclaw_core::ui::ThreadInfo) -> Self {
        Self {
            id: t.id,
            project_id: t.project_id,
            label: t.label.clone(),
            description: t.description.clone(),
            status: t.status.clone(),
            is_foreground: t.is_foreground,
            message_count: t.message_count,
            working_dir: t.working_dir.clone(),
        }
    }
}

/// A project group in the two-level sidebar: a project header plus the
/// threads that belong to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectGroupData {
    /// Project ID.
    pub id: u64,
    /// Project name (the sidebar header label).
    pub name: String,
    /// Working directory (shown as a subtitle / tooltip).
    pub path: String,
    /// Whether this is the active project.
    pub is_active: bool,
    /// Threads belonging to this project, in display order.
    pub threads: Vec<SidebarItemData>,
}

/// Name used for a synthesized group when the project list hasn't arrived yet.
pub const FALLBACK_PROJECT_NAME: &str = "Workspace";

impl ProjectGroupData {
    /// The resolved display name, never empty.
    ///
    /// Projects can carry a blank name (an empty rename, or a group
    /// synthesized before the gateway's project list arrives). Falls back to
    /// the final path segment, then to [`FALLBACK_PROJECT_NAME`], so the
    /// sidebar always has a label to render.
    pub fn display_name(&self) -> Cow<'_, str> {
        if !self.name.trim().is_empty() {
            return Cow::Borrowed(self.name.trim());
        }
        let tail = self
            .path
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .find(|seg| !seg.is_empty());
        match tail {
            Some(seg) => Cow::Borrowed(seg),
            None => Cow::Borrowed(FALLBACK_PROJECT_NAME),
        }
    }

    /// A display path: `home` collapsed to `~`, then truncated from the left.
    ///
    /// Truncating in Rust rather than with CSS is deliberate. The CSS trick
    /// for a leading ellipsis (`direction: rtl`) reorders the bidi-neutral
    /// leading separator, rendering `/home/x` as `home/x/`. The sidebar is a
    /// fixed width, so a character budget is a fine substitute.
    ///
    /// Pass the user's home directory (`None` to skip the `~` step).
    pub fn pretty_path(&self, home: Option<&str>, max_chars: usize) -> Cow<'_, str> {
        if self.path.is_empty() {
            return Cow::Borrowed("");
        }

        // Collapse the home prefix, but only on a path-segment boundary so
        // `/home/user-old` isn't mangled into `~-old`.
        let shortened = home
            .map(|h| h.trim_end_matches(['/', '\\']))
            .filter(|h| !h.is_empty())
            .and_then(|h| {
                let rest = self.path.strip_prefix(h)?;
                if rest.is_empty() {
                    Some(Cow::Borrowed("~"))
                } else if rest.starts_with(['/', '\\']) {
                    Some(Cow::Owned(format!("~{rest}")))
                } else {
                    None
                }
            })
            .unwrap_or(Cow::Borrowed(self.path.as_str()));

        if shortened.chars().count() <= max_chars {
            return shortened;
        }

        let tail: String = {
            let chars: Vec<char> = shortened.chars().collect();
            chars[chars.len().saturating_sub(max_chars.saturating_sub(1))..]
                .iter()
                .collect()
        };
        Cow::Owned(format!("…{tail}"))
    }

    /// Path truncated from the left (keeping the tail) to `max_chars`.
    pub fn truncated_path(&self, max_chars: usize) -> Cow<'_, str> {
        if self.path.chars().count() > max_chars {
            let tail: String = self
                .path
                .chars()
                .rev()
                .take(max_chars.saturating_sub(1))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Cow::Owned(format!("…{tail}"))
        } else {
            Cow::Borrowed(&self.path)
        }
    }
}

/// The full two-level sidebar tree: projects, each with their threads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SidebarTree {
    pub groups: Vec<ProjectGroupData>,
    /// The active project's ID (its group renders expanded/highlighted).
    pub active_project_id: u64,
}

impl SidebarTree {
    /// Build the tree from the project list, thread list, and active project.
    ///
    /// Threads are bucketed by `project_id`. A thread whose `project_id` is 0
    /// or doesn't match any known project (e.g. an ephemeral task/sub-agent)
    /// is placed under the active project so it's never dropped. Project order
    /// follows `projects`; threads keep their incoming order within a group.
    pub fn build(
        projects: &[rustyclaw_core::ui::ProjectInfo],
        threads: &[rustyclaw_core::ui::ThreadInfo],
        active_project_id: u64,
    ) -> Self {
        let items = threads.iter().map(SidebarItemData::from).collect();
        Self::from_items(projects, items, active_project_id)
    }

    /// The project a thread displays under: its own `project_id` when that is
    /// non-zero and known, otherwise the active project. Single source of truth
    /// for orphan/ephemeral-thread placement, shared by the tree builder and
    /// flat (TUI) clients so every renderer groups threads identically.
    pub fn effective_project_id(
        project_id: u64,
        known: &std::collections::HashSet<u64>,
        active_project_id: u64,
    ) -> u64 {
        if project_id != 0 && known.contains(&project_id) {
            project_id
        } else {
            active_project_id
        }
    }

    /// Bucket already-converted [`SidebarItemData`] into project groups.
    ///
    /// Project order follows `projects`; threads keep their incoming order
    /// within a group. Orphan/ephemeral threads (see [`effective_project_id`])
    /// land under the active project so they're never dropped.
    ///
    /// When `projects` is empty — the window between connecting and the
    /// gateway's first `ProjectsUpdate` — a single fallback group is
    /// synthesized so threads still render instead of vanishing.
    ///
    /// [`effective_project_id`]: SidebarTree::effective_project_id
    pub fn from_items(
        projects: &[rustyclaw_core::ui::ProjectInfo],
        items: Vec<SidebarItemData>,
        active_project_id: u64,
    ) -> Self {
        use std::collections::HashSet;
        let known: HashSet<u64> = projects.iter().map(|p| p.id).collect();

        let mut groups: Vec<ProjectGroupData> = projects
            .iter()
            .map(|p| ProjectGroupData {
                id: p.id,
                name: p.name.clone(),
                path: p.path.clone(),
                is_active: p.id == active_project_id,
                threads: Vec::new(),
            })
            .collect();

        // No project list yet: keep the threads visible under a placeholder
        // group rather than dropping them on the floor.
        if groups.is_empty() && !items.is_empty() {
            groups.push(ProjectGroupData {
                id: active_project_id,
                name: FALLBACK_PROJECT_NAME.to_string(),
                path: String::new(),
                is_active: true,
                threads: Vec::new(),
            });
        }

        for item in items {
            let target = Self::effective_project_id(item.project_id, &known, active_project_id);
            // `target` may name a project that isn't in `groups` (empty list →
            // synthesized group). Fall back to the first group so no thread is
            // silently dropped.
            let slot = groups
                .iter()
                .position(|g| g.id == target)
                .or(if groups.is_empty() { None } else { Some(0) });
            if let Some(idx) = slot {
                groups[idx].threads.push(item);
            }
        }

        Self {
            groups,
            active_project_id,
        }
    }

    /// Flatten the tree into a single project-grouped list, rewriting each
    /// item's `project_id` to its effective group so flat renderers can insert
    /// a header whenever it changes. Order matches the rendered tree, so a flat
    /// selection index lines up with what the user sees.
    pub fn into_flat_items(self) -> Vec<SidebarItemData> {
        self.groups
            .into_iter()
            .flat_map(|g| {
                let gid = g.id;
                g.threads.into_iter().map(move |mut t| {
                    t.project_id = gid;
                    t
                })
            })
            .collect()
    }

    /// Total thread count across all groups.
    pub fn thread_count(&self) -> usize {
        self.groups.iter().map(|g| g.threads.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::ui::{ProjectInfo, ThreadInfo};

    fn thread(id: u64, project_id: u64) -> ThreadInfo {
        ThreadInfo {
            id,
            project_id,
            label: Some(format!("t{id}")),
            description: None,
            status: "active".into(),
            is_foreground: false,
            message_count: 0,
            working_dir: None,
        }
    }

    #[test]
    fn groups_threads_by_project_and_buckets_orphans() {
        let projects = vec![
            ProjectInfo {
                id: 1,
                name: "Default".into(),
                path: "/ws".into(),
            },
            ProjectInfo {
                id: 2,
                name: "Side".into(),
                path: "/side".into(),
            },
        ];
        // t10 → project 2, t11 → project 1, t12 → unknown (0) → active (2).
        let threads = vec![thread(10, 2), thread(11, 1), thread(12, 0)];
        let tree = SidebarTree::build(&projects, &threads, 2);

        assert_eq!(tree.groups.len(), 2);
        let p1 = tree.groups.iter().find(|g| g.id == 1).unwrap();
        let p2 = tree.groups.iter().find(|g| g.id == 2).unwrap();
        assert!(p2.is_active);
        assert_eq!(p1.threads.len(), 1);
        assert_eq!(
            p2.threads.len(),
            2,
            "orphan thread lands under active project"
        );
        assert_eq!(tree.thread_count(), 3);
    }

    #[test]
    fn threads_survive_an_empty_project_list() {
        // The window between connecting and the first ProjectsUpdate: no
        // projects known yet, but threads must still render.
        let threads = vec![thread(10, 0), thread(11, 7)];
        let tree = SidebarTree::build(&[], &threads, 1);

        assert_eq!(tree.groups.len(), 1, "a fallback group is synthesized");
        assert_eq!(tree.thread_count(), 2, "no thread is dropped");
        assert_eq!(tree.groups[0].display_name(), FALLBACK_PROJECT_NAME);
    }

    #[test]
    fn empty_project_list_with_no_threads_stays_empty() {
        let tree = SidebarTree::build(&[], &[], 1);
        assert!(tree.is_empty(), "nothing to show, no placeholder group");
    }

    #[test]
    fn display_name_falls_back_to_path_tail_then_placeholder() {
        let named = ProjectGroupData {
            name: "Api Server".into(),
            path: "/home/dev/api".into(),
            ..Default::default()
        };
        assert_eq!(named.display_name(), "Api Server");

        // Blank name → last path segment, with a trailing separator ignored.
        let blank = ProjectGroupData {
            name: "   ".into(),
            path: "/home/dev/api/".into(),
            ..Default::default()
        };
        assert_eq!(blank.display_name(), "api");

        let windows = ProjectGroupData {
            name: String::new(),
            path: r"C:\Users\dev\proj".into(),
            ..Default::default()
        };
        assert_eq!(windows.display_name(), "proj");

        // Nothing to go on at all.
        let bare = ProjectGroupData::default();
        assert_eq!(bare.display_name(), FALLBACK_PROJECT_NAME);
    }

    #[test]
    fn pretty_path_collapses_home_and_truncates_from_the_left() {
        let g = |path: &str| ProjectGroupData {
            path: path.into(),
            ..Default::default()
        };

        // Home collapses to ~ on a segment boundary.
        assert_eq!(
            g("/home/user/src/RustyClaw").pretty_path(Some("/home/user"), 40),
            "~/src/RustyClaw"
        );
        // A trailing separator on the supplied home is tolerated.
        assert_eq!(
            g("/home/user/src/app").pretty_path(Some("/home/user/"), 40),
            "~/src/app"
        );
        // The home directory itself.
        assert_eq!(g("/home/user").pretty_path(Some("/home/user"), 40), "~");
        // A sibling that merely shares a prefix must not be mangled.
        assert_eq!(
            g("/home/user-old/src").pretty_path(Some("/home/user"), 40),
            "/home/user-old/src"
        );
        // No home supplied → untouched.
        assert_eq!(g("/srv/app").pretty_path(None, 40), "/srv/app");

        // Over budget: ellipsis on the left, meaningful tail kept.
        let long = g("/home/user/tmp/very/deeply/nested/scratch");
        let out = long.pretty_path(Some("/home/user"), 20);
        assert!(out.starts_with('…'), "ellipsis leads: {out}");
        assert!(out.ends_with("scratch"), "tail is kept: {out}");
        assert_eq!(out.chars().count(), 20, "respects the budget: {out}");

        // Empty path stays empty rather than becoming a bare ellipsis.
        assert_eq!(g("").pretty_path(Some("/home/user"), 20), "");
    }
}
