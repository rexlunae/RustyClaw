//! `session_search` — find things said in past conversations.
//!
//! # What this searches, and why it is not sessions
//!
//! Issue #146 asks for a search "across all session transcripts". Taken
//! literally that would search [`crate::sessions::SessionManager`], which is
//! an in-memory registry with a hundred-message cap and no persistence: it
//! holds sub-agent run records for the life of the process and nothing else.
//! A search over it could not answer the issue's own example — "what did we
//! discuss about X last week?" — because last week's sessions no longer
//! exist anywhere to be searched.
//!
//! The durable record of a conversation is the *thread*, persisted to
//! `threads.json` with every message and timestamp. That is what this
//! searches. The tool keeps the issue's name because that is what someone
//! looking for it will type.
//!
//! # Scoring
//!
//! Term-frequency over the query's terms, with a bonus for matching all of
//! them and for matching a whole word rather than a fragment. Deliberately
//! not embeddings: those need the `semantic-memory` feature and a model on
//! disk, and a lexical search that always works beats a semantic one that
//! silently degrades to nothing when the model is missing.

use serde_json::Value;
use std::path::Path;

use crate::threads::{MessageRole, ThreadManager};
use crate::tools::error::ToolResult;

/// A message that matched, with enough around it to be read.
struct Hit {
    thread_id: u64,
    thread_label: String,
    role: MessageRole,
    content: String,
    timestamp: std::time::SystemTime,
    score: f64,
    /// Messages either side, when the caller asked for them.
    context: Vec<(MessageRole, String)>,
}

/// Split a query into lowercase terms worth matching on.
///
/// Single characters are dropped: they match nearly everything and would
/// dominate the ranking without saying anything about relevance.
fn terms_of(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 1)
        .map(str::to_lowercase)
        .collect()
}

/// How well one message answers the query. `0.0` means it does not.
///
/// Public within the crate so the tests can exercise the ranking directly
/// rather than asserting on formatted output.
pub(crate) fn score_message(content: &str, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let haystack = content.to_lowercase();
    let words: Vec<&str> = haystack
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let mut score = 0.0;
    let mut matched_terms = 0;
    for term in terms {
        let whole = words.iter().filter(|w| *w == term).count();
        let partial = words.iter().filter(|w| w.contains(term.as_str())).count() - whole;
        if whole + partial == 0 {
            continue;
        }
        matched_terms += 1;
        // A whole word is the stronger signal; a fragment still counts, so
        // "deploy" finds "deployment".
        score += whole as f64 + partial as f64 * 0.25;
    }
    if matched_terms == 0 {
        return 0.0;
    }
    // Matching every term is what separates "the thing they asked about"
    // from "a message that happens to share a common word".
    if matched_terms == terms.len() && terms.len() > 1 {
        score *= 2.0;
    }
    // Long messages accumulate hits by being long, not by being relevant.
    score / (1.0 + (words.len() as f64).ln())
}

/// Age of a message in whole days, for the `sinceDays` filter.
fn age_days(t: std::time::SystemTime) -> u64 {
    std::time::SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0)
}

/// Search persisted thread history.
/// Read the stored threads, whichever layout they are in.
///
/// `Ok(None)` means "nothing stored yet", which is not an error.
///
/// This originally read `threads.json` directly, which made the whole tool
/// answer "No conversation history yet." on every install that had run once.
/// Threads moved to a per-thread store in a `threads/` directory, and
/// `ThreadStore::load_or_migrate` renames the legacy file to
/// `threads.json.migrated` the first time it runs — so the path this looked
/// at was guaranteed to be absent exactly when there was history to find.
///
/// Read-only on purpose: `load_or_migrate` would be the obvious call and it
/// writes. A search is not a reason to rewrite state on disk, and running one
/// against a not-yet-migrated install should not decide the migration moment.
/// So the store is tried first and the legacy file second, without moving
/// anything.
fn load_history(legacy_path: &Path) -> Result<Option<ThreadManager>, String> {
    let store = crate::threads::ThreadStore::at_legacy_path(legacy_path);
    match store.load() {
        Ok(mgr) => return Ok(Some(mgr)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("Could not read conversation history: {e}")),
    }

    // No store directory: either a fresh install, or one that has not been
    // opened since the store landed and still has its legacy file.
    match ThreadManager::load_from_file(legacy_path) {
        Ok(mgr) => Ok(Some(mgr)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read conversation history: {e}")),
    }
}

pub fn exec_session_search(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    let include_context = args
        .get("includeContext")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let thread_filter = args.get("threadId").and_then(|v| v.as_u64());
    let since_days = args.get("sinceDays").and_then(|v| v.as_u64());

    let terms = terms_of(query);
    if terms.is_empty() {
        return Err(format!(
            "Query '{query}' has nothing to search for — single characters are \
             ignored because they match almost everything."
        )
        .into());
    }

    // The tool runs in core, which has no gateway to ask, so the transcript
    // is read from where the gateway persists it.
    let settings_dir = crate::runtime_ctx::get_settings_dir().ok_or_else(|| {
        "Conversation history is unavailable: no settings directory is \
         registered, so there is nothing to read threads from."
            .to_string()
    })?;
    let agent_id = crate::runtime_ctx::get_active_agent()
        .unwrap_or_else(|| crate::agents::MAIN_AGENT_ID.to_string());
    let path = settings_dir
        .join("agents")
        .join(&agent_id)
        .join("sessions")
        .join("threads.json");

    let manager = match load_history(&path)? {
        Some(mgr) => mgr,
        // Nothing stored yet is an empty history, not a failure: a fresh
        // install has nothing to search and should say so plainly.
        None => return Ok("No conversation history yet.".to_string()),
    };

    let mut hits: Vec<Hit> = Vec::new();
    for thread in manager.list() {
        if thread_filter.is_some_and(|want| want != thread.id.0) {
            continue;
        }
        let messages: Vec<_> = thread.messages.iter().collect();
        for (i, message) in messages.iter().enumerate() {
            // Tool chatter is not conversation, and matching it buries the
            // exchange the caller is looking for.
            if matches!(message.role, MessageRole::Tool | MessageRole::System) {
                continue;
            }
            if since_days.is_some_and(|days| age_days(message.timestamp) > days) {
                continue;
            }
            let score = score_message(&message.content, &terms);
            if score <= 0.0 {
                continue;
            }
            let context = if include_context {
                let start = i.saturating_sub(1);
                let end = (i + 2).min(messages.len());
                messages[start..end]
                    .iter()
                    .filter(|m| !std::ptr::eq(**m, *message))
                    .map(|m| (m.role.clone(), m.content.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            hits.push(Hit {
                thread_id: thread.id.0,
                thread_label: thread.label.clone(),
                role: message.role.clone(),
                content: message.content.clone(),
                timestamp: message.timestamp,
                score,
                context,
            });
        }
    }

    if hits.is_empty() {
        return Ok(format!("Nothing in past conversations matched '{query}'."));
    }

    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Ties go to the more recent message: when two are equally
            // relevant, the later one is usually the current state of things.
            .then_with(|| b.timestamp.cmp(&a.timestamp))
    });
    let total = hits.len();
    hits.truncate(limit);

    let mut out = format!(
        "{} match{} for '{}'{}:\n\n",
        total,
        if total == 1 { "" } else { "es" },
        query,
        if total > hits.len() {
            format!(" (showing {})", hits.len())
        } else {
            String::new()
        }
    );
    for hit in &hits {
        out.push_str(&format!(
            "#{} {} — {:?}, {} day(s) ago\n",
            hit.thread_id,
            hit.thread_label,
            hit.role,
            age_days(hit.timestamp)
        ));
        out.push_str(&format!(
            "  {}\n",
            crate::text::truncate_chars(&hit.content, 400)
        ));
        for (role, content) in &hit.context {
            out.push_str(&format!(
                "    ({:?}) {}\n",
                role,
                crate::text::truncate_chars(content, 200)
            ));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Parameters for `session_search`.
pub fn session_search_params() -> Vec<crate::tools::ToolParam> {
    use crate::tools::ToolParam;
    vec![
        ToolParam {
            name: "query".into(),
            description: "What to look for. Matched against the words of each message; \
                          single characters are ignored because they match nearly everything."
                .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "limit".into(),
            description: "Most results to return (default: 10). The total found is reported \
                          even when more were trimmed."
                .into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "includeContext".into(),
            description: "Also return the message either side of each hit, for when the \
                          match alone does not carry the meaning (default: false)."
                .into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "threadId".into(),
            description: "Search only this thread. Omit to search every thread.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "sinceDays".into(),
            description: "Ignore messages older than this many days.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(q: &str) -> Vec<String> {
        terms_of(q)
    }

    #[test]
    fn a_message_that_says_nothing_about_the_query_scores_zero() {
        assert_eq!(
            score_message("the cat sat on the mat", &terms("deploy")),
            0.0
        );
    }

    #[test]
    fn matching_every_term_beats_matching_one() {
        let both = score_message(
            "we should deploy the gateway tonight",
            &terms("deploy gateway"),
        );
        let one = score_message("we should deploy the cat tonight", &terms("deploy gateway"));
        assert!(both > one, "both={both} one={one}");
        assert!(one > 0.0, "a single term should still match");
    }

    #[test]
    fn a_whole_word_beats_a_fragment() {
        let whole = score_message("deploy now", &terms("deploy"));
        let fragment = score_message("deployment now", &terms("deploy"));
        assert!(whole > fragment, "whole={whole} fragment={fragment}");
        assert!(
            fragment > 0.0,
            "a fragment should still match, so 'deploy' finds 'deployment'"
        );
    }

    /// Otherwise the longest message in the history wins every search by
    /// containing everything.
    #[test]
    fn length_does_not_buy_relevance() {
        let terse = score_message("deploy the gateway", &terms("deploy gateway"));
        let padded = score_message(
            &format!("deploy the gateway {}", "and some other words ".repeat(50)),
            &terms("deploy gateway"),
        );
        assert!(terse > padded, "terse={terse} padded={padded}");
    }

    /// Single characters match almost everything, so they are not terms.
    #[test]
    fn single_characters_are_not_search_terms() {
        assert!(terms_of("a b c").is_empty());
        assert_eq!(terms_of("a deploy"), vec!["deploy".to_string()]);
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        assert_eq!(score_message("anything at all", &[]), 0.0);
    }
}

/// The loader, against both storage layouts.
///
/// These exist because the tool shipped reading `threads.json` directly and
/// therefore answered "No conversation history yet." on every install that had
/// migrated — a whole feature that never returned a result. Unit tests on the
/// scoring could not see that, because the scoring was fine.
#[cfg(test)]
mod load_history_tests {
    use super::load_history;
    use crate::threads::{MessageRole, ThreadStore};

    /// The regression: history persisted through the store must be found.
    /// Before the fix this returned `None`, because the store writes to
    /// `threads/` and nothing writes `threads.json` any more.
    #[test]
    fn history_written_by_the_store_is_found() {
        let dir = tempfile::tempdir().expect("temp dir");
        let legacy = dir.path().join("threads.json");

        let store = ThreadStore::at_legacy_path(&legacy);
        let mut manager = crate::threads::ThreadManager::new();
        let id = manager.create_thread("a thread");
        manager.add_message(id, MessageRole::User, "how do I deploy the gateway");
        store.persist(&mut manager).expect("store persists");

        assert!(!legacy.exists(), "the store must not write the legacy file");

        let loaded = load_history(&legacy)
            .expect("loading succeeds")
            .expect("the store's history is found");
        assert!(
            loaded
                .list()
                .iter()
                .any(|t| t.messages.iter().any(|m| m.content.contains("deploy"))),
            "the stored message did not come back"
        );
    }

    /// An install that has not been opened since the store landed still has
    /// its legacy file and no `threads/` directory. That history is still
    /// searchable.
    #[test]
    fn history_still_in_the_legacy_file_is_found() {
        let dir = tempfile::tempdir().expect("temp dir");
        let legacy = dir.path().join("threads.json");

        let mut manager = crate::threads::ThreadManager::new();
        let id = manager.create_thread("a thread");
        manager.add_message(id, MessageRole::User, "legacy message about deploys");
        manager.save_to_file(&legacy).expect("legacy file writes");

        let loaded = load_history(&legacy)
            .expect("loading succeeds")
            .expect("the legacy history is found");
        assert!(
            loaded
                .list()
                .iter()
                .any(|t| t.messages.iter().any(|m| m.content.contains("legacy"))),
            "the legacy message did not come back"
        );
    }

    /// Reading must not migrate. A search is not a reason to rewrite state on
    /// disk, and it should not be what decides when a migration happens.
    #[test]
    fn searching_does_not_move_or_rewrite_anything() {
        let dir = tempfile::tempdir().expect("temp dir");
        let legacy = dir.path().join("threads.json");

        let mut manager = crate::threads::ThreadManager::new();
        let id = manager.create_thread("a thread");
        manager.add_message(id, MessageRole::User, "hello");
        manager.save_to_file(&legacy).expect("legacy file writes");

        load_history(&legacy).expect("loading succeeds");

        assert!(legacy.exists(), "the legacy file was moved by a read");
        assert!(
            !dir.path().join("threads").exists(),
            "a read created the store directory"
        );
    }

    /// Neither layout present is "nothing yet", not an error.
    #[test]
    fn a_fresh_install_has_no_history_rather_than_an_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("threads.json");
        assert!(
            load_history(&missing)
                .expect("a fresh install is not an error")
                .is_none()
        );
    }
}
