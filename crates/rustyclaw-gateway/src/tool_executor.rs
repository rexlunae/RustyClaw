//! Tool execution helper — extracted from dispatch_text_message for DRY.
//!
//! This module provides a unified entry point for executing tools, handling
//! the different tool types (user prompts, secrets, skills, standard tools).

use rustyclaw_core::tools;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

use super::{SharedSkillManager, SharedVault};
use crate::secrets_handler;
use crate::skills_handler;

// ── Rate limiting ───────────────────────────────────────────────────────────

/// Simple sliding-window rate limiter for tool execution.
///
/// Tracks calls per tool name within a configurable window.  When the
/// limit is exceeded the tool is rejected with an error message, preventing
/// runaway tool loops or abuse through repeated expensive calls.
///
/// The limiter is **global** (all sessions share one instance) and uses
/// a coarse-grained mutex — contention is negligible given tool calls
/// are serialised by the model anyway.
pub struct ToolRateLimiter {
    window_ms: u64,
    max_calls: usize,
    buckets: VecDeque<(String, Instant)>,
}

impl ToolRateLimiter {
    /// Create a new limiter.
    ///
    /// `window_ms` — sliding window duration.
    /// `max_calls`  — maximum tool invocations in the window **per tool name**.
    pub fn new(window_ms: u64, max_calls: usize) -> Self {
        Self {
            window_ms,
            max_calls,
            buckets: VecDeque::new(),
        }
    }

    /// Check whether a tool call of `name` is allowed.
    /// If denied, returns an error message.
    pub fn check(&mut self, name: &str) -> Result<(), String> {
        let now = Instant::now();
        let cutoff = now - std::time::Duration::from_millis(self.window_ms);

        // Drop stale entries
        while let Some(front) = self.buckets.front() {
            if front.1 < cutoff {
                self.buckets.pop_front();
            } else {
                break;
            }
        }

        // Count current-call-name entries in the window
        let count = self.buckets.iter().filter(|(n, _)| n == name).count();
        if count >= self.max_calls {
            return Err(format!(
                "Rate limit exceeded: '{}' called {} times in {}ms window",
                name,
                count + 1,
                self.window_ms,
            ));
        }

        self.buckets.push_back((name.to_string(), now));
        Ok(())
    }
}

/// Global rate limiter instance (initialised lazily on first use).
fn rate_limiter() -> &'static Mutex<ToolRateLimiter> {
    static LIMITER: std::sync::OnceLock<Mutex<ToolRateLimiter>> = std::sync::OnceLock::new();
    LIMITER.get_or_init(|| {
        let cfg = std::env::var("RUSTYCLAW_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(60);
        Mutex::new(ToolRateLimiter::new(30_000, cfg))
    })
}

/// Check the rate limiter for a tool call.  If denied, return the error text.
pub fn check_rate_limit(name: &str) -> Result<(), String> {
    rate_limiter()
        .lock()
        .map_err(|e| format!("Rate limiter poisoned: {}", e))
        .and_then(|mut limiter| limiter.check(name))
}

/// Execute a tool by name, routing to the appropriate handler.
///
/// Returns `(output_text, is_error)`.
pub async fn execute_tool_by_type(
    name: &str,
    arguments: &Value,
    workspace_dir: &Path,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
) -> (String, bool) {
    // Apply rate limiting before executing any tool.
    if let Err(err) = check_rate_limit(name) {
        tracing::warn!(tool = name, "Rate limit hit");
        return (err, true);
    }

    if tools::is_secrets_tool(name) {
        match secrets_handler::execute_secrets_tool(name, arguments, vault).await {
            Ok(text) => (text, false),
            Err(err) => (err.to_string(), true),
        }
    } else if tools::is_skill_tool(name) {
        match skills_handler::execute_skill_tool(name, arguments, skill_mgr).await {
            Ok(text) => (text, false),
            Err(err) => (err.to_string(), true),
        }
    } else {
        match tools::execute_tool(name, arguments, workspace_dir).await {
            Ok(text) => (text, false),
            Err(err) => (err, true),
        }
    }
}

/// Check if a short response suggests incomplete intent that should be continued.
///
/// Returns true if the model appears to have stated intent without making a tool call.
pub fn should_auto_continue(
    response_text: &str,
    consecutive_continues: usize,
    max_continues: usize,
) -> bool {
    // Only consider continuation for short responses
    if response_text.len() >= 500 || consecutive_continues >= max_continues {
        return false;
    }

    // Check only the tail of the response for intent patterns
    let tail = if response_text.len() > 200 {
        &response_text[response_text.len() - 200..]
    } else {
        response_text
    };

    const INTENT_PATTERNS: &[&str] = &[
        "Let me ",
        "I'll ",
        "I will ",
        "Now let me ",
        "Let's ",
        "Now I'll ",
        "I need to ",
        "First, let me ",
        "First let me ",
    ];

    // Phrases that look like intent but are actually polite closers
    const NON_ACTION_PHRASES: &[&str] = &[
        "let me know",
        "i'll help",
        "i'll guide",
        "i'll be happy",
        "i'll be glad",
        "i'll do my best",
        "i'll try my best",
        "i'll assist",
        "let's get started",
        "let's begin",
        "let me help",
    ];

    let text_lower = response_text.to_lowercase();
    let has_exclusion = NON_ACTION_PHRASES.iter().any(|p| text_lower.contains(p));

    if has_exclusion {
        return false;
    }

    let text_suggests_action = INTENT_PATTERNS.iter().any(|p| tail.contains(p));
    let ends_with_continuation = tail.trim_end().ends_with(':');

    text_suggests_action || ends_with_continuation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_continue_intent_patterns() {
        assert!(should_auto_continue("Let me check the file.", 0, 2));
        assert!(should_auto_continue("I'll read it now.", 0, 2));
        assert!(should_auto_continue("Here are the results:", 0, 2));
    }

    #[test]
    fn test_should_not_continue_exclusions() {
        assert!(!should_auto_continue("Let me know if you need help.", 0, 2));
        assert!(!should_auto_continue("I'll be happy to assist.", 0, 2));
    }

    #[test]
    fn test_should_not_continue_long_response() {
        let long_text = "x".repeat(600);
        assert!(!should_auto_continue(&long_text, 0, 2));
    }

    #[test]
    fn test_should_not_continue_max_reached() {
        assert!(!should_auto_continue("Let me check.", 2, 2));
    }
}
