//! Shared slash-command menu modelling.

/// Shared data for a slash-command completion menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandMenuData {
    pub completions: Vec<String>,
    pub selected: Option<usize>,
}

impl CommandMenuData {
    pub fn is_open(&self) -> bool {
        !self.completions.is_empty()
    }

    /// Clear all completions and reset selection.
    pub fn clear(&mut self) {
        self.completions.clear();
        self.selected = None;
    }

    /// Return the selected completion formatted for the input box.
    ///
    /// The returned string includes the leading slash, e.g. `/model gpt-4o`.
    pub fn selected_input_value(&self) -> Option<String> {
        let idx = self.selected?;
        self.completions.get(idx).map(|cmd| format!("/{cmd}"))
    }

    /// Move selection to the next completion (wrapping).
    ///
    /// Returns the selected completion text after moving.
    pub fn select_next(&mut self) -> Option<String> {
        let len = self.completions.len();
        if len == 0 {
            self.selected = None;
            return None;
        }

        let next = match self.selected {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.selected = Some(next);
        self.completions.get(next).cloned()
    }

    /// Move selection forward and return the formatted input value.
    pub fn select_next_input_value(&mut self) -> Option<String> {
        self.select_next()?;
        self.selected_input_value()
    }

    /// Move selection to the previous completion (wrapping).
    ///
    /// Returns the selected completion text after moving.
    pub fn select_prev(&mut self) -> Option<String> {
        let len = self.completions.len();
        if len == 0 {
            self.selected = None;
            return None;
        }

        let prev = match self.selected {
            Some(0) | None => len.saturating_sub(1),
            Some(i) => i - 1,
        };
        self.selected = Some(prev);
        self.completions.get(prev).cloned()
    }

    /// Move selection backward and return the formatted input value.
    pub fn select_prev_input_value(&mut self) -> Option<String> {
        self.select_prev()?;
        self.selected_input_value()
    }

    /// Compute the visible completion window around the selected row.
    pub fn visible_window(&self, max_visible: usize) -> (usize, usize, usize) {
        let total = self.completions.len();
        if total == 0 {
            return (0, 0, 0);
        }

        let selected = self.selected.unwrap_or(0).min(total.saturating_sub(1));
        let start = if total <= max_visible {
            0
        } else {
            let half = max_visible / 2;
            let start = selected.saturating_sub(half);
            let end = (start + max_visible).min(total);
            if end == total {
                total.saturating_sub(max_visible)
            } else {
                start
            }
        };
        let end = (start + max_visible).min(total);
        (start, end, selected)
    }
}

/// Build slash-command completions for a `/{partial}` input.
///
/// Merges static command names with optionally fetched model IDs and
/// returns only entries that match the current partial input.
pub fn build_slash_completions(
    provider: &str,
    live_models: Option<&[String]>,
    partial: &str,
) -> Vec<String> {
    let mut names = rustyclaw_core::commands::command_names_for_provider(provider);
    if let Some(live) = live_models {
        let mut seen: std::collections::HashSet<String> = names.iter().cloned().collect();
        for model in live {
            let entry = format!("model {}", model);
            if seen.insert(entry.clone()) {
                names.push(entry);
            }
        }
    }

    names
        .into_iter()
        .filter(|c| c.starts_with(partial))
        .collect()
}