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
            label: t.label.clone(),
            description: t.description.clone(),
            status: t.status.clone(),
            is_foreground: t.is_foreground,
            message_count: t.message_count,
        }
    }
}
