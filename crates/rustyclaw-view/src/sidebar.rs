//! Component data for the thread sidebar.
//!
//! The sidebar lists active threads/sessions and lets the user
//! switch between them.  This module provides the shared data
//! type for each sidebar item.

use std::borrow::Cow;
use std::path::PathBuf;

/// Lifecycle state of a thread's turn, for the sidebar state icon.
///
/// The desktop resolves this per thread by layering client-side knowledge
/// (a locally-known in-flight turn, a prompt the agent is parked on) over
/// the gateway's status string; the TUI derives it from the status alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadState {
    /// No conversation yet, or the status is unknown.
    #[default]
    Idle,
    /// The agent is running a turn in this thread.
    Working,
    /// The agent is parked waiting on the user (tool approval, question,
    /// credential request, sign-in).
    Asking,
    /// Turn finished cleanly; ready for the next prompt.
    Ready,
    /// The turn is paused (backgrounded or waiting).
    Paused,
    /// The task/sub-agent turn finished with a result.
    Completed,
    /// The turn ended in an error.
    Error,
    /// The turn was cancelled.
    Cancelled,
}

impl ThreadState {
    /// Map a gateway status string to a state.
    ///
    /// The gateway's `ThreadInfo.status` is one of `"Streaming"` (an open
    /// turn), `"Ready"` (an interactive thread at rest), or the
    /// `ThreadStatus` display text (`"Completed"`, `"Failed: …"`,
    /// `"Waiting: …"`, …).
    pub fn from_status(status: &str) -> Self {
        match status {
            "Streaming" | "Active" | "Running" => Self::Working,
            "Ready" => Self::Ready,
            "Paused" => Self::Paused,
            "Completed" => Self::Completed,
            "Cancelled" => Self::Cancelled,
            s if s.starts_with("Running") => Self::Working,
            s if s.starts_with("Waiting") => Self::Asking,
            s if s.starts_with("Failed") => Self::Error,
            _ => Self::Idle,
        }
    }

    /// A single glyph for display.
    pub fn icon(&self) -> &'static str {
        match self {
            Self::Idle => "·",
            Self::Working => "▶",
            Self::Asking => "❓",
            Self::Ready => "○",
            Self::Paused => "⏸",
            Self::Completed => "✓",
            Self::Error => "✕",
            Self::Cancelled => "⊘",
        }
    }

    /// Short human-readable label (tooltips / accessibility).
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::Asking => "Asking",
            Self::Ready => "Ready",
            Self::Paused => "Paused",
            Self::Completed => "Completed",
            Self::Error => "Error",
            Self::Cancelled => "Cancelled",
        }
    }

    /// CSS modifier class for the state icon (`is-working`, `is-asking`, …).
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Asking => "asking",
            Self::Ready => "ready",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

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

    /// Resolved lifecycle state for the row's state icon.
    pub state: ThreadState,

    /// Whether this is the currently foregrounded thread.
    pub is_foreground: bool,

    /// Number of messages in the thread.
    pub message_count: usize,

    /// Working-directory override, or `None` when the thread inherits its
    /// project's directory.
    pub working_dir: Option<PathBuf>,
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

    /// The full title tooltip text (label + description joined by newline,
    /// with the resolved state as the last line).
    pub fn title_text(&self) -> String {
        let state = format!("[{}]", self.state.label());
        match self.description.as_deref() {
            Some(desc) if !desc.is_empty() => {
                format!("{}\n{}\n{}", self.display_label(), desc, state)
            }
            _ => format!("{}\n{}", self.display_label(), state),
        }
    }

    /// A brief status indicator character for display.
    ///
    /// Legacy helper kept for callers that render a plain dot; new
    /// renderers should use [`ThreadState::icon`] on [`Self::state`]
    /// instead, which carries the same information with colour/animation
    /// hooks.
    pub fn status_dot(&self) -> &'static str {
        match self.state {
            ThreadState::Working => "●",
            ThreadState::Asking => "?",
            ThreadState::Ready => "○",
            ThreadState::Completed => "✓",
            ThreadState::Error => "✕",
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
            state: ThreadState::from_status(&t.status),
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
    pub path: PathBuf,
    /// Whether this is the active project.
    pub is_active: bool,
    /// Threads belonging to this project, in display order.
    pub threads: Vec<SidebarItemData>,
}

/// Name used for a synthesized group when the project list hasn't arrived yet.
pub const FALLBACK_PROJECT_NAME: &str = "Workspace";

// The path helpers below split on both `/` and `\` rather than using
// `std::path`'s component logic, and that is deliberate: the gateway may run
// on a different platform than the client, so a Windows path routinely has to
// render in a Unix sidebar (and vice versa). `Path::file_name` would treat
// `C:\Users\dev\proj` as one long segment on Unix.

/// The last non-empty path segment, ignoring any trailing separator.
fn path_tail(path: &str) -> Option<&str> {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|seg| !seg.is_empty())
}

/// Keep the rightmost `max_chars` characters, marking the cut with a leading
/// ellipsis. Counts characters, not bytes, so multi-byte paths aren't split.
fn truncate_from_left(path: &str, max_chars: usize) -> Cow<'_, str> {
    if path.chars().count() <= max_chars {
        return Cow::Borrowed(path);
    }
    let chars: Vec<char> = path.chars().collect();
    let tail: String = chars[chars.len().saturating_sub(max_chars.saturating_sub(1))..]
        .iter()
        .collect();
    Cow::Owned(format!("…{tail}"))
}

/// `home` collapsed to `~`, then truncated from the left to `max_chars`.
fn pretty_path_str<'a>(path: &'a str, home: Option<&str>, max_chars: usize) -> Cow<'a, str> {
    if path.is_empty() {
        return Cow::Borrowed("");
    }

    // Collapse the home prefix, but only on a path-segment boundary so
    // `/home/user-old` isn't mangled into `~-old`.
    let shortened = home
        .map(|h| h.trim_end_matches(['/', '\\']))
        .filter(|h| !h.is_empty())
        .and_then(|h| {
            let rest = path.strip_prefix(h)?;
            if rest.is_empty() {
                Some(Cow::Borrowed("~"))
            } else if rest.starts_with(['/', '\\']) {
                Some(Cow::Owned(format!("~{rest}")))
            } else {
                None
            }
        })
        .unwrap_or(Cow::Borrowed(path));

    match shortened {
        Cow::Borrowed(s) => truncate_from_left(s, max_chars),
        Cow::Owned(s) => Cow::Owned(truncate_from_left(&s, max_chars).into_owned()),
    }
}

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
        match self.path_text() {
            Cow::Borrowed(p) => match path_tail(p) {
                Some(seg) => Cow::Borrowed(seg),
                None => Cow::Borrowed(FALLBACK_PROJECT_NAME),
            },
            Cow::Owned(p) => match path_tail(&p) {
                Some(seg) => Cow::Owned(seg.to_string()),
                None => Cow::Borrowed(FALLBACK_PROJECT_NAME),
            },
        }
    }

    /// The path as display text.
    ///
    /// Lossy for a path that isn't valid UTF-8 — but only here, at the point
    /// of rendering, where the alternative is showing the user nothing. The
    /// stored [`path`](Self::path) keeps the original bytes.
    fn path_text(&self) -> Cow<'_, str> {
        self.path.to_string_lossy()
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
        match self.path_text() {
            Cow::Borrowed(p) => pretty_path_str(p, home, max_chars),
            Cow::Owned(p) => Cow::Owned(pretty_path_str(&p, home, max_chars).into_owned()),
        }
    }

    /// Path truncated from the left (keeping the tail) to `max_chars`.
    pub fn truncated_path(&self, max_chars: usize) -> Cow<'_, str> {
        match self.path_text() {
            Cow::Borrowed(p) => truncate_from_left(p, max_chars),
            Cow::Owned(p) => Cow::Owned(truncate_from_left(&p, max_chars).into_owned()),
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
                path: PathBuf::new(),
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
    fn from_status_maps_gateway_statuses() {
        use ThreadState::*;
        assert_eq!(ThreadState::from_status("Streaming"), Working);
        assert_eq!(ThreadState::from_status("Active"), Working);
        assert_eq!(ThreadState::from_status("Running"), Working);
        assert_eq!(
            ThreadState::from_status("Running: downloading model"),
            Working
        );
        assert_eq!(ThreadState::from_status("Ready"), Ready);
        assert_eq!(ThreadState::from_status("Waiting: pick a tool"), Asking);
        assert_eq!(ThreadState::from_status("Paused"), Paused);
        assert_eq!(ThreadState::from_status("Completed"), Completed);
        assert_eq!(ThreadState::from_status("Failed: boom"), Error);
        assert_eq!(ThreadState::from_status("Cancelled"), Cancelled);
        assert_eq!(ThreadState::from_status("something-else"), Idle);
    }

    #[test]
    fn state_icon_has_a_glyph_and_label_for_every_state() {
        use ThreadState::*;
        for (state, icon, label) in [
            (Idle, "·", "Idle"),
            (Working, "▶", "Working"),
            (Asking, "❓", "Asking"),
            (Ready, "○", "Ready"),
            (Paused, "⏸", "Paused"),
            (Completed, "✓", "Completed"),
            (Error, "✕", "Error"),
            (Cancelled, "⊘", "Cancelled"),
        ] {
            assert_eq!(state.icon(), icon, "icon for {label}");
            assert_eq!(state.label(), label);
            assert!(!state.css_class().is_empty());
        }
    }

    #[test]
    fn thread_info_derives_state_from_status() {
        let mut t = thread(1, 0);
        t.status = "Streaming".into();
        assert_eq!(SidebarItemData::from(&t).state, ThreadState::Working);

        t.status = "Ready".into();
        assert_eq!(SidebarItemData::from(&t).state, ThreadState::Ready);
    }

    #[test]
    fn title_text_appends_the_state_label() {
        let mut t = thread(1, 0);
        t.status = "Streaming".into();
        let item = SidebarItemData::from(&t);
        let title = item.title_text();
        assert!(
            title.contains("[Working]"),
            "tooltip names the state: {title}"
        );
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
