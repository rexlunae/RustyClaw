//! Event pattern matching for routine triggers.
//!
//! Provides regex-based pattern matching to trigger routines based on
//! agent responses, tool outputs, or system events.

use anyhow::{Context as AnyhowContext, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Event matcher for pattern-based routine triggers.
pub struct EventMatcher {
    pattern: Regex,
    description: Option<String>,
}

impl EventMatcher {
    /// Create a new event matcher from a regex pattern.
    ///
    /// Patterns use standard Rust regex syntax.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Match any mention of "error" or "failed"
    /// let matcher = EventMatcher::new(r"(?i)(error|failed)")?;
    ///
    /// // Match GitHub issue/PR numbers
    /// let matcher = EventMatcher::new(r"#\d+")?;
    ///
    /// // Match deployment keywords
    /// let matcher = EventMatcher::new(r"(?i)(deploy|release|ship)")?;
    /// ```
    pub fn new(pattern: &str) -> Result<Self> {
        let regex = Regex::new(pattern)
            .with_context(|| format!("Invalid regex pattern: {}", pattern))?;

        Ok(Self {
            pattern: regex,
            description: None,
        })
    }

    /// Create a new event matcher with a description.
    pub fn new_with_description(pattern: &str, description: String) -> Result<Self> {
        let mut matcher = Self::new(pattern)?;
        matcher.description = Some(description);
        Ok(matcher)
    }

    /// Check if the given text matches the pattern.
    pub fn matches(&self, text: &str) -> bool {
        self.pattern.is_match(text)
    }

    /// Find all matches in the text.
    ///
    /// Returns a vector of matched strings.
    pub fn find_matches<'a>(&self, text: &'a str) -> Vec<&'a str> {
        self.pattern
            .find_iter(text)
            .map(|m| m.as_str())
            .collect()
    }

    /// Get the pattern string.
    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    /// Get the description if set.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Validate a regex pattern without creating a matcher.
    pub fn validate(pattern: &str) -> Result<()> {
        Regex::new(pattern)
            .with_context(|| format!("Invalid regex pattern: {}", pattern))?;
        Ok(())
    }
}

/// Event context for matching against routines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Event type (e.g., "agent_response", "tool_output", "system")
    pub event_type: String,
    /// The text content to match against
    pub content: String,
    /// Optional metadata (tool name, session ID, etc.)
    pub metadata: Option<serde_json::Value>,
}

impl Event {
    /// Create a new event.
    pub fn new(event_type: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            event_type: event_type.into(),
            content: content.into(),
            metadata: None,
        }
    }

    /// Add metadata to the event.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Create an agent response event.
    pub fn agent_response(content: impl Into<String>) -> Self {
        Self::new("agent_response", content)
    }

    /// Create a tool output event.
    pub fn tool_output(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        let mut event = Self::new("tool_output", content);
        event.metadata = Some(serde_json::json!({
            "tool_name": tool_name.into()
        }));
        event
    }

    /// Create a system event.
    pub fn system_event(content: impl Into<String>) -> Self {
        Self::new("system", content)
    }
}

/// Event dispatcher that manages multiple event matchers.
pub struct EventDispatcher {
    matchers: Vec<(String, EventMatcher)>, // (routine_id, matcher)
}

impl EventDispatcher {
    /// Create a new event dispatcher.
    pub fn new() -> Self {
        Self {
            matchers: Vec::new(),
        }
    }

    /// Register a matcher for a routine.
    pub fn register(&mut self, routine_id: String, matcher: EventMatcher) {
        self.matchers.push((routine_id, matcher));
    }

    /// Unregister all matchers for a routine.
    pub fn unregister(&mut self, routine_id: &str) {
        self.matchers.retain(|(id, _)| id != routine_id);
    }

    /// Find all routines that match the given event.
    ///
    /// Returns a vector of routine IDs that should be triggered.
    pub fn dispatch(&self, event: &Event) -> Vec<String> {
        self.matchers
            .iter()
            .filter_map(|(routine_id, matcher)| {
                if matcher.matches(&event.content) {
                    Some(routine_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get the number of registered matchers.
    pub fn matcher_count(&self) -> usize {
        self.matchers.len()
    }

    /// Clear all matchers.
    pub fn clear(&mut self) {
        self.matchers.clear();
    }
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_match() {
        let matcher = EventMatcher::new(r"error").unwrap();
        assert!(matcher.matches("An error occurred"));
        assert!(!matcher.matches("Everything is fine"));
    }

    #[test]
    fn test_case_insensitive() {
        let matcher = EventMatcher::new(r"(?i)error").unwrap();
        assert!(matcher.matches("ERROR"));
        assert!(matcher.matches("Error"));
        assert!(matcher.matches("error"));
    }

    #[test]
    fn test_complex_pattern() {
        let matcher = EventMatcher::new(r"(?i)(deploy|release|ship)\s+v?\d+\.\d+").unwrap();
        assert!(matcher.matches("Ready to deploy v1.5"));
        assert!(matcher.matches("Ship 2.0 tomorrow"));
        assert!(matcher.matches("Release 1.2.3 is live"));
        assert!(!matcher.matches("deploy soon"));
    }

    #[test]
    fn test_find_matches() {
        let matcher = EventMatcher::new(r"#\d+").unwrap();
        let text = "Fixed issues #123 and #456";
        let matches = matcher.find_matches(text);
        assert_eq!(matches, vec!["#123", "#456"]);
    }

    #[test]
    fn test_invalid_pattern() {
        let result = EventMatcher::new(r"[invalid(regex");
        assert!(result.is_err());
    }

    #[test]
    fn test_validation() {
        assert!(EventMatcher::validate(r"\d+").is_ok());
        assert!(EventMatcher::validate(r"[invalid").is_err());
    }

    #[test]
    fn test_event_creation() {
        let event = Event::agent_response("Hello world");
        assert_eq!(event.event_type, "agent_response");
        assert_eq!(event.content, "Hello world");

        let event = Event::tool_output("web_fetch", "Page loaded");
        assert_eq!(event.event_type, "tool_output");
        assert!(event.metadata.is_some());
    }

    #[test]
    fn test_dispatcher() {
        let mut dispatcher = EventDispatcher::new();

        let error_matcher = EventMatcher::new(r"(?i)error").unwrap();
        let deploy_matcher = EventMatcher::new(r"(?i)deploy").unwrap();

        dispatcher.register("routine-1".to_string(), error_matcher);
        dispatcher.register("routine-2".to_string(), deploy_matcher);

        assert_eq!(dispatcher.matcher_count(), 2);

        // Test error event
        let event = Event::agent_response("An error occurred");
        let triggered = dispatcher.dispatch(&event);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0], "routine-1");

        // Test deploy event
        let event = Event::agent_response("Ready to deploy v1.0");
        let triggered = dispatcher.dispatch(&event);
        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0], "routine-2");

        // Test unregister
        dispatcher.unregister("routine-1");
        assert_eq!(dispatcher.matcher_count(), 1);
    }

    #[test]
    fn test_multiple_matches() {
        let mut dispatcher = EventDispatcher::new();

        let error_matcher = EventMatcher::new(r"(?i)error").unwrap();
        let critical_matcher = EventMatcher::new(r"(?i)critical").unwrap();

        dispatcher.register("routine-1".to_string(), error_matcher);
        dispatcher.register("routine-2".to_string(), critical_matcher);

        // Both matchers should trigger
        let event = Event::agent_response("CRITICAL ERROR detected");
        let triggered = dispatcher.dispatch(&event);
        assert_eq!(triggered.len(), 2);
        assert!(triggered.contains(&"routine-1".to_string()));
        assert!(triggered.contains(&"routine-2".to_string()));
    }

    #[test]
    fn test_no_matches() {
        let mut dispatcher = EventDispatcher::new();
        let matcher = EventMatcher::new(r"error").unwrap();
        dispatcher.register("routine-1".to_string(), matcher);

        let event = Event::agent_response("Everything is fine");
        let triggered = dispatcher.dispatch(&event);
        assert_eq!(triggered.len(), 0);
    }

    #[test]
    fn test_matcher_with_description() {
        let matcher = EventMatcher::new_with_description(
            r"(?i)error",
            "Triggers on any error message".to_string(),
        )
        .unwrap();

        assert_eq!(matcher.description(), Some("Triggers on any error message"));
        assert!(matcher.matches("An error occurred"));
    }

    #[test]
    fn test_event_metadata() {
        let event = Event::tool_output("web_fetch", "Page loaded")
            .with_metadata(serde_json::json!({
                "url": "https://example.com",
                "status": 200
            }));

        assert!(event.metadata.is_some());
        let metadata = event.metadata.unwrap();
        assert_eq!(metadata["url"], "https://example.com");
        assert_eq!(metadata["status"], 200);
    }
}
