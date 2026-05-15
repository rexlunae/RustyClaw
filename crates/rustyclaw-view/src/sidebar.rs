//! Component data for the thread sidebar.
//!
//! The sidebar lists active threads/sessions and lets the user
//! switch between them.  This module provides the shared data
//! type for each sidebar item.

/// Data for a single item in the thread sidebar.
///
/// Rendered as a row showing the thread label, message count,
/// and an indicator for the currently active thread.
#[derive(Clone, Debug, PartialEq)]
pub struct SidebarItemData {
    /// Thread / session ID.
    pub id: u64,

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
}

impl From<&rustyclaw_core::ui::ThreadInfo> for SidebarItemData {
    fn from(t: &rustyclaw_core::ui::ThreadInfo) -> Self {
        Self {
            id: t.id,
            label: t.label.clone(),
            description: t.description.clone(),
            status: t.status.clone(),
            is_foreground: t.is_foreground,
            message_count: t.message_count,
        }
    }
}
