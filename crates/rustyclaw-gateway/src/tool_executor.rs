//! Tool execution helper — extracted from dispatch_text_message for DRY.
//!
//! This module provides a unified entry point for executing tools, handling
//! the different tool types (user prompts, secrets, skills, standard tools).

use rustyclaw_core::tools;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{SharedSkillManager, SharedVault};
use crate::secrets_handler;
use crate::skills_handler;

// ── Rate limiting ──────────────────────────────────────────────────────────

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
    pub fn check(&mut self, name: &str) -> Result<(), RateLimitError> {
        let now = Instant::now();
        let cutoff = now - Duration::from_millis(self.window_ms);

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
            return Err(RateLimitError::Exceeded {
                tool: name.to_string(),
                count: count + 1,
                window_ms: self.window_ms,
            });
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

/// Why a tool call was rejected before execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RateLimitError {
    /// The per-tool call budget for the window was exhausted.
    #[error("Rate limit exceeded: '{tool}' called {count} times in {window_ms}ms window")]
    Exceeded {
        tool: String,
        count: usize,
        window_ms: u64,
    },
    /// The limiter's lock was poisoned by a panicking task.
    #[error("Rate limiter poisoned")]
    Poisoned,
}

/// Check the rate limiter for a tool call.
pub fn check_rate_limit(name: &str) -> Result<(), RateLimitError> {
    rate_limiter()
        .lock()
        .map_err(|_| RateLimitError::Poisoned)
        .and_then(|mut limiter| limiter.check(name))
}

// ── Repeated malformed-call protection ─────────────────────────────────────

const MALFORMED_CALL_WINDOW: Duration = Duration::from_secs(30);
const MALFORMED_CALL_LIMIT: usize = 3;

#[derive(Clone)]
struct MalformedCallRecord {
    count: usize,
    last_seen: Instant,
    explanation: String,
}

fn malformed_call_tracker() -> &'static Mutex<HashMap<u64, MalformedCallRecord>> {
    static TRACKER: std::sync::OnceLock<Mutex<HashMap<u64, MalformedCallRecord>>> =
        std::sync::OnceLock::new();
    TRACKER.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Hash a call without retaining argument values, which may contain secrets.
fn call_fingerprint(name: &str, arguments: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    arguments.to_string().hash(&mut hasher);
    hasher.finish()
}

/// Return the prior explanation when an identical malformed call should be suppressed.
fn repeated_malformed_call(name: &str, arguments: &Value) -> Option<String> {
    let fingerprint = call_fingerprint(name, arguments);
    let now = Instant::now();
    let mut tracker = malformed_call_tracker().lock().ok()?;
    tracker.retain(|_, record| now.duration_since(record.last_seen) <= MALFORMED_CALL_WINDOW);
    let record = tracker.get(&fingerprint)?;
    if record.count < MALFORMED_CALL_LIMIT {
        return None;
    }

    Some(format!(
        "{}. This identical malformed call has already failed {} times in the last {} seconds, so it was suppressed without executing again. Change the arguments before retrying.",
        record.explanation,
        record.count,
        MALFORMED_CALL_WINDOW.as_secs(),
    ))
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Describe argument names and types without exposing their values.
fn argument_shape(arguments: &Value) -> String {
    match arguments {
        Value::Object(map) if map.is_empty() => "an empty object".to_string(),
        Value::Object(map) => {
            let mut fields: Vec<String> = map
                .iter()
                .map(|(name, value)| format!("{name}: {}", json_type(value)))
                .collect();
            fields.sort();
            format!("fields {{{}}}", fields.join(", "))
        }
        other => format!("a {} value", json_type(other)),
    }
}

fn missing_parameter(error: &str) -> Option<&str> {
    error
        .strip_prefix("Missing required parameter: ")
        .map(|name| name.trim_end_matches('.'))
}

/// Turn terse validation failures into actionable model- and user-facing explanations.
fn explain_tool_error(name: &str, arguments: &Value, error: &str) -> String {
    let Some(parameter) = missing_parameter(error) else {
        return format!("Tool `{name}` failed: {error}");
    };

    let expected = if name == "execute_command" && parameter == "command" {
        "Call it with a non-empty string, for example: execute_command(command=\"pwd\")."
            .to_string()
    } else {
        format!("Include `{parameter}` with the type required by the tool schema.")
    };

    format!(
        "Tool `{name}` could not run because required parameter `{parameter}` was missing or had the wrong type. It received {}. {expected} Do not retry with the same arguments.",
        argument_shape(arguments),
    )
}

fn record_malformed_failure(name: &str, arguments: &Value, error: &str) -> usize {
    if missing_parameter(error).is_none() {
        return 1;
    }

    let fingerprint = call_fingerprint(name, arguments);
    let explanation = explain_tool_error(name, arguments, error);
    let now = Instant::now();
    let Ok(mut tracker) = malformed_call_tracker().lock() else {
        return 1;
    };
    tracker.retain(|_, record| now.duration_since(record.last_seen) <= MALFORMED_CALL_WINDOW);
    let record = tracker.entry(fingerprint).or_insert(MalformedCallRecord {
        count: 0,
        last_seen: now,
        explanation,
    });
    record.count += 1;
    record.last_seen = now;
    record.count
}

/// Execute a tool by name, routing to the appropriate handler.
///
/// Tools that produce incremental output (currently `execute_command`)
/// stream it into `output` as it arrives; other tools ignore the sink.
///
/// Returns `(output_text, is_error)`.
pub async fn execute_tool_by_type(
    name: &str,
    arguments: &Value,
    workspace_dir: &Path,
    vault: &SharedVault,
    skill_mgr: &SharedSkillManager,
    output: Option<tools::ToolOutputSink>,
) -> (String, bool) {
    if let Some(explanation) = repeated_malformed_call(name, arguments) {
        tracing::warn!(tool = name, "Suppressing repeated malformed tool call");
        return (explanation, true);
    }

    // Apply rate limiting before executing any tool.
    if let Err(err) = check_rate_limit(name) {
        tracing::warn!(tool = name, "Rate limit hit");
        return (err.to_string(), true);
    }

    let result = if tools::is_secrets_tool(name) {
        secrets_handler::execute_secrets_tool(name, arguments, vault).await
    } else if tools::is_skill_tool(name) {
        skills_handler::execute_skill_tool(name, arguments, skill_mgr).await
    } else {
        tools::execute_tool_streaming(name, arguments, workspace_dir, output).await
    };

    match result {
        Ok(text) => (text, false),
        Err(err) => {
            let raw_error = err.to_string();
            let explanation = explain_tool_error(name, arguments, &raw_error);
            let count = record_malformed_failure(name, arguments, &raw_error);
            if missing_parameter(&raw_error).is_some() {
                tracing::warn!(
                    tool = name,
                    repeated_count = count,
                    argument_shape = %argument_shape(arguments),
                    "Tool call rejected because a required argument was missing or invalid"
                );
            }
            (explanation, true)
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

/// Detect a response that only acknowledges a pre-compaction memory flush
/// ("Memory saved to memory/… Ready to resume after compaction.") without
/// answering the user's actual message. Used by the dispatch loop to
/// auto-resume once after a flush instead of ending the turn on the
/// acknowledgement.
pub fn is_flush_acknowledgement(response_text: &str) -> bool {
    let lower = response_text.to_lowercase();
    const ACK_PATTERNS: &[&str] = &[
        "memory saved",
        "saved to memory",
        "memory flush",
        "ready to resume",
        "resume after compaction",
        "before compaction",
    ];
    ACK_PATTERNS.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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

    #[test]
    fn test_flush_acknowledgement_detection() {
        assert!(is_flush_acknowledgement(
            "Memory saved to memory/2026-07-01.md — appended a snapshot. \
             Ready to resume after compaction."
        ));
        assert!(is_flush_acknowledgement(
            "I've completed the memory flush and stored the notes."
        ));
        assert!(!is_flush_acknowledgement(
            "The banner is rendered in banner.rs; here's the fix you asked for."
        ));
    }

    #[test]
    fn missing_command_error_is_actionable_and_does_not_leak_values() {
        let args = json!({"working_dir": "/secret/project", "timeout_secs": 10});
        let explanation = explain_tool_error(
            "execute_command",
            &args,
            "Missing required parameter: command",
        );

        assert!(explanation.contains("required parameter `command`"));
        assert!(explanation.contains("execute_command(command=\"pwd\")"));
        assert!(explanation.contains("working_dir: string"));
        assert!(!explanation.contains("/secret/project"));
    }

    #[test]
    fn non_validation_errors_gain_tool_context() {
        let explanation = explain_tool_error("read_file", &json!({}), "File not found");
        assert_eq!(explanation, "Tool `read_file` failed: File not found");
    }

    #[test]
    fn argument_shape_lists_names_and_types_only() {
        let shape = argument_shape(&json!({"command": "echo secret", "background": true}));
        assert_eq!(shape, "fields {background: boolean, command: string}");
        assert!(!shape.contains("secret"));
    }
}
