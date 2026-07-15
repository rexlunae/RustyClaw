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

/// Like [`build_slash_completions`], additionally appending Hugging Face
/// Hub completion entries (full command lines built by
/// [`HubCompletionContext::entries`]).
///
/// Hub entries bypass the `starts_with` filter: Hub search matches
/// substrings (typing `qwen3` matches `Qwen/Qwen3-4B-GGUF`), so a hub
/// entry usually does *not* literally extend what was typed.
pub fn build_slash_completions_with_hub(
    provider: &str,
    live_models: Option<&[String]>,
    hub_entries: &[String],
    partial: &str,
) -> Vec<String> {
    let mut out = build_slash_completions(provider, live_models, partial);
    for entry in hub_entries {
        if !out.contains(entry) {
            out.push(entry.clone());
        }
    }
    out
}

// ── Hugging Face Hub model-name completion ──────────────────────────────────

/// A slash-command position where Hugging Face Hub model-name completion
/// applies, parsed from the current input by [`hub_completion_context`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubCompletionContext {
    /// The command text before the model argument (no leading slash),
    /// e.g. `engines pull llamacpp `. Prepended to a repo id to form a
    /// full completion entry.
    pub prefix: String,
    /// The partial model text to search the Hub for. May be empty, in
    /// which case the most-downloaded models are suggested.
    pub query: String,
    /// Restrict the search to repos that ship GGUF files (what the local
    /// inference engines can actually run).
    pub gguf_only: bool,
}

impl HubCompletionContext {
    /// Turn Hub repo ids into full completion entries for the menu.
    pub fn entries(&self, repo_ids: &[String]) -> Vec<String> {
        repo_ids
            .iter()
            .map(|id| format!("{}{}", self.prefix, id))
            .collect()
    }

    /// Completion entries from the last-loaded Hub results.
    ///
    /// Used as-is when the cached results answer exactly this query;
    /// otherwise the stale set is narrowed by case-insensitive substring
    /// (mirroring how Hub search matches) so the dropdown updates on
    /// every keystroke while the fresh search is still in flight.
    pub fn entries_from_cache(
        &self,
        loaded_query: Option<&str>,
        loaded_ids: &[String],
    ) -> Vec<String> {
        if loaded_query == Some(self.query.as_str()) {
            return self.entries(loaded_ids);
        }
        let needle = self.query.to_lowercase();
        let narrowed: Vec<String> = loaded_ids
            .iter()
            .filter(|id| id.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        self.entries(&narrowed)
    }
}

/// Engines whose `pull` argument is a Hugging Face repo id.
/// (Ollama pulls from its own registry; Exo and LM Studio can't pull.)
fn hub_pull_engine_gguf_only(engine: &str) -> Option<bool> {
    match engine {
        // The HF downloader can pull any repo, GGUF or not.
        "huggingface" => Some(false),
        // Inference engines only run GGUF files.
        "llamacpp" | "joshua" => Some(true),
        _ => None,
    }
}

/// Detect whether the current `/{partial}` input is typing a Hugging Face
/// model name, and if so what to search for.
///
/// Recognized positions:
/// - `engines pull <engine> <partial-model>` (HF-backed engines only)
/// - `engines files <partial-repo>`
/// - `engines search <query>`
pub fn hub_completion_context(partial: &str) -> Option<HubCompletionContext> {
    let rest = partial
        .strip_prefix("engines ")
        .or_else(|| partial.strip_prefix("engine "))?;

    let (query, gguf_only) = if let Some(after) = rest.strip_prefix("pull ") {
        // The engine arg must be complete (a space typed after it).
        let (engine, query) = after.split_once(' ')?;
        let gguf_only = hub_pull_engine_gguf_only(engine)?;
        (query, gguf_only)
    } else if let Some(query) = rest.strip_prefix("files ") {
        (query, true)
    } else {
        (rest.strip_prefix("search ")?, true)
    };

    // Repo ids never contain spaces; a second word means the user has
    // moved past the model argument (search queries may span words).
    if query.contains(' ') && !rest.starts_with("search ") {
        return None;
    }

    Some(HubCompletionContext {
        prefix: partial[..partial.len() - query.len()].to_string(),
        query: query.to_string(),
        gguf_only,
    })
}

#[cfg(test)]
mod hub_completion_tests {
    use super::*;

    #[test]
    fn pull_context_requires_hf_backed_engine() {
        let ctx = hub_completion_context("engines pull llamacpp qwen3").unwrap();
        assert_eq!(ctx.prefix, "engines pull llamacpp ");
        assert_eq!(ctx.query, "qwen3");
        assert!(ctx.gguf_only);

        let ctx = hub_completion_context("engines pull huggingface qwen3").unwrap();
        assert!(!ctx.gguf_only);

        // Ollama pulls from its own registry — no Hub completion.
        assert!(hub_completion_context("engines pull ollama qwen3").is_none());
        // Engine name still being typed.
        assert!(hub_completion_context("engines pull llamacpp").is_none());
    }

    #[test]
    fn pull_context_with_empty_query_suggests_top_models() {
        let ctx = hub_completion_context("engines pull joshua ").unwrap();
        assert_eq!(ctx.query, "");
        assert_eq!(ctx.prefix, "engines pull joshua ");
    }

    #[test]
    fn files_and_search_contexts() {
        let ctx = hub_completion_context("engines files qwen").unwrap();
        assert_eq!(ctx.prefix, "engines files ");
        assert!(ctx.gguf_only);

        // Search queries may span words.
        let ctx = hub_completion_context("engines search qwen 4b").unwrap();
        assert_eq!(ctx.query, "qwen 4b");

        // Singular alias works too.
        assert!(hub_completion_context("engine files qwen").is_some());
    }

    #[test]
    fn non_engine_commands_have_no_context() {
        assert!(hub_completion_context("model gpt").is_none());
        assert!(hub_completion_context("engines start ollama").is_none());
        // A second word past the model arg ends the completion position.
        assert!(hub_completion_context("engines pull llamacpp qwen extra").is_none());
    }

    #[test]
    fn entries_prepend_the_typed_prefix() {
        let ctx = hub_completion_context("engines pull llamacpp qwen3").unwrap();
        let entries = ctx.entries(&["Qwen/Qwen3-4B-GGUF".to_string()]);
        assert_eq!(entries, vec!["engines pull llamacpp Qwen/Qwen3-4B-GGUF"]);
    }

    #[test]
    fn cache_is_used_exactly_or_narrowed_by_substring() {
        let ctx = hub_completion_context("engines pull llamacpp qwen3").unwrap();
        let cached = vec![
            "Qwen/Qwen3-4B-GGUF".to_string(),
            "TheBloke/Llama-2-7B-GGUF".to_string(),
        ];
        // Exact query match → cached set verbatim.
        assert_eq!(
            ctx.entries_from_cache(Some("qwen3"), &cached).len(),
            cached.len()
        );
        // Stale cache (from a shorter query) → narrowed by substring.
        assert_eq!(
            ctx.entries_from_cache(Some("q"), &cached),
            vec!["engines pull llamacpp Qwen/Qwen3-4B-GGUF"]
        );
    }

    #[test]
    fn hub_entries_bypass_the_prefix_filter() {
        let hub = vec!["engines pull llamacpp Qwen/Qwen3-4B-GGUF".to_string()];
        let out =
            build_slash_completions_with_hub("openrouter", None, &hub, "engines pull llamacpp qw");
        assert!(out.contains(&hub[0]));
    }
}
