//! Component data for the command palette (Ctrl+Shift+P / Cmd+K style).
//!
//! Generalizes the slash-command menu into a full command palette with
//! fuzzy matching, categories, and shortcut display.

/// A single palette action/entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteEntry {
    /// Unique action identifier (e.g. "open.cron", "toggle.dark-mode").
    pub id: String,
    /// Display label.
    pub label: String,
    /// Category for grouping.
    pub category: String,
    /// Keyboard shortcut (if assigned).
    pub shortcut: Option<String>,
    /// Optional description.
    pub description: Option<String>,
}

/// Full state for the command palette.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CommandPaletteData {
    pub open: bool,
    pub query: String,
    pub entries: Vec<PaletteEntry>,
    pub filtered: Vec<usize>,
    pub selected: Option<usize>,
}

impl CommandPaletteData {
    /// Filter entries by the current query (fuzzy match on label).
    pub fn update_filter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.entries.len()).collect();
        } else {
            let q = self.query.to_lowercase();
            self.filtered = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.label.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected = if self.filtered.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Get the selected entry.
    pub fn selected_entry(&self) -> Option<&PaletteEntry> {
        self.selected
            .and_then(|si| self.filtered.get(si))
            .and_then(|&idx| self.entries.get(idx))
    }

    pub fn select_next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let max = self.filtered.len() - 1;
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur >= max { 0 } else { cur + 1 });
    }

    pub fn select_prev(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let max = self.filtered.len() - 1;
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur == 0 { max } else { cur - 1 });
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.query.clear();
            self.update_filter();
        }
    }
}

/// Keyboard shortcut remapping entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutMapping {
    /// Action ID (same as `PaletteEntry.id`).
    pub action: String,
    /// Default shortcut (display label like "Ctrl+K").
    pub default_shortcut: String,
    /// User-assigned override (if any).
    pub user_shortcut: Option<String>,
}

/// Zoom level state.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoomState {
    /// Current zoom factor (1.0 = 100%).
    pub factor: f32,
    /// Minimum zoom.
    pub min: f32,
    /// Maximum zoom.
    pub max: f32,
    /// Step size.
    pub step: f32,
}

impl Default for ZoomState {
    fn default() -> Self {
        Self {
            factor: 1.0,
            min: 0.5,
            max: 3.0,
            step: 0.1,
        }
    }
}

impl ZoomState {
    pub fn zoom_in(&mut self) {
        self.factor = (self.factor + self.step).min(self.max);
    }

    pub fn zoom_out(&mut self) {
        self.factor = (self.factor - self.step).max(self.min);
    }

    pub fn reset(&mut self) {
        self.factor = 1.0;
    }

    /// Display as percentage (e.g. "120%").
    pub fn display(&self) -> String {
        format!("{}%", (self.factor * 100.0).round() as u32)
    }
}
