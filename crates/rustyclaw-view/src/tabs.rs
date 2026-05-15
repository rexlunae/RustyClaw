//! Component data for the thread tab bar.
//!
//! Tabs replace the old thread list in the sidebar.  Each tab
//! represents one chat session.  The tab bar renders horizontally
//! above the message area.

use std::borrow::Cow;

/// Data for a single tab in the thread tab bar.
///
/// Tabs display the session label, a close button, and an active
/// indicator.  Methods centralise display formatting so both
/// desktop and TUI derive the same strings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabItemData {
    /// Thread / session ID.
    pub id: u64,

    /// Optional user-assigned label.
    pub label: Option<String>,

    /// Whether this is the currently active (foreground) tab.
    pub is_foreground: bool,

    /// Number of messages — shown as a muted counter.
    pub message_count: usize,
}

impl TabItemData {
    /// The resolved display label.
    ///
    /// Uses the user-assigned label when present, otherwise falls
    /// back to `"Session #{id}"`.
    pub fn display_label(&self) -> Cow<'_, str> {
        self.label
            .as_deref()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Owned(format!("Session #{}", self.id)))
    }

    /// Label truncated to at most `max_chars` character boundaries.
    ///
    /// Appends `"…"` when truncation occurs.
    pub fn truncated_label(&self, max_chars: usize) -> Cow<'_, str> {
        let label = self.display_label();
        if label.chars().count() > max_chars {
            let truncated: String = label.chars().take(max_chars.saturating_sub(1)).collect();
            Cow::Owned(format!("{}…", truncated))
        } else {
            label
        }
    }

    /// Whether this tab can be closed (always true for tabs
    /// beyond the first, to prevent closing the last session).
    pub fn closeable(&self, total_tabs: usize) -> bool {
        total_tabs > 1
    }
}

impl From<&rustyclaw_core::ui::ThreadInfo> for TabItemData {
    fn from(t: &rustyclaw_core::ui::ThreadInfo) -> Self {
        Self {
            id: t.id,
            label: t.label.clone(),
            is_foreground: t.is_foreground,
            message_count: t.message_count,
        }
    }
}

/// Data for the full tab bar.
///
/// Wraps the ordered list of tabs and the foreground thread ID.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabBarData {
    /// Ordered list of tabs for rendering.
    pub tabs: Vec<TabItemData>,

    /// The currently foreground (active) thread ID.
    pub foreground_id: u64,
}

impl TabBarData {
    /// Create from a list of `ThreadInfo` references.
    pub fn from_threads(threads: &[rustyclaw_core::ui::ThreadInfo]) -> Self {
        let tabs: Vec<TabItemData> = threads.iter().map(TabItemData::from).collect();
        let fg = tabs.iter().find(|t| t.is_foreground).map(|t| t.id).unwrap_or(0);
        Self {
            tabs,
            foreground_id: fg,
        }
    }

    /// The active tab, if any.
    pub fn active_tab(&self) -> Option<&TabItemData> {
        self.tabs.iter().find(|t| t.is_foreground)
    }

    /// The active tab's label, or a fallback.
    pub fn active_label(&self) -> Cow<'_, str> {
        self.active_tab()
            .map(|t| t.display_label())
            .unwrap_or_else(|| Cow::Borrowed("No session"))
    }

    /// Number of tabs.
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}
