//! Pre-compaction memory flush.
//!
//! Triggers a silent agent turn before compaction to persist durable memories.
//! This prevents important context from being lost when conversation history
//! is compacted to fit within the model's context window.

use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for pre-compaction memory flush.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFlushConfig {
    /// Enable pre-compaction memory flush.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Trigger flush when this many tokens remain before hard limit.
    /// Default: 4000 tokens before compaction threshold.
    #[serde(default = "default_soft_threshold")]
    pub soft_threshold_tokens: usize,

    /// System prompt for flush turn.
    #[serde(default = "default_flush_system_prompt")]
    pub system_prompt: String,

    /// User prompt for flush turn.
    #[serde(default = "default_flush_user_prompt")]
    pub user_prompt: String,
}

fn default_true() -> bool {
    true
}

fn default_soft_threshold() -> usize {
    4000
}

fn default_flush_system_prompt() -> String {
    "Pre-compaction memory flush. Session context is approaching limits. \
     Store any durable memories now (use memory/YYYY-MM-DD.md; create memory/ if needed). \
     IMPORTANT: If the file already exists, APPEND new content only — do not overwrite existing entries. \
     If nothing important needs to be stored, reply with NO_REPLY."
        .to_string()
}

fn default_flush_user_prompt() -> String {
    "Write any lasting notes or context to memory files before compaction. \
     Reply with NO_REPLY if nothing needs to be stored."
        .to_string()
}

impl Default for MemoryFlushConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            soft_threshold_tokens: default_soft_threshold(),
            system_prompt: default_flush_system_prompt(),
            user_prompt: default_flush_user_prompt(),
        }
    }
}

/// Memory flush controller.
///
/// Tracks whether a flush has been triggered in the current compaction cycle
/// and provides methods to check if a flush is needed.
pub struct MemoryFlush {
    config: MemoryFlushConfig,
    /// Track whether we've flushed this compaction cycle.
    flushed_this_cycle: bool,
}

impl MemoryFlush {
    /// Create a new memory flush controller.
    pub fn new(config: MemoryFlushConfig) -> Self {
        Self {
            config,
            flushed_this_cycle: false,
        }
    }

    /// Check if we should trigger a flush based on token count.
    ///
    /// Returns `true` if:
    /// - Memory flush is enabled
    /// - We haven't already flushed this cycle
    /// - Current token count exceeds the soft threshold
    pub fn should_flush(
        &self,
        current_tokens: usize,
        max_tokens: usize,
        compaction_threshold: f64,
    ) -> bool {
        if !self.config.enabled || self.flushed_this_cycle {
            return false;
        }

        // Calculate the threshold where we should flush
        // (slightly before compaction would trigger)
        let compaction_point = (max_tokens as f64 * compaction_threshold) as usize;
        let flush_point = compaction_point.saturating_sub(self.config.soft_threshold_tokens);

        current_tokens >= flush_point
    }

    /// Build the flush messages to inject.
    ///
    /// Returns (system_message, user_message) with date/time substituted.
    pub fn build_flush_messages(&self) -> (String, String) {
        let date = Local::now().format("%Y-%m-%d").to_string();
        let time = Utc::now().format("%H:%M UTC").to_string();

        let system = format!(
            "{}\nCurrent time: {}. Today's date: {}.",
            self.config.system_prompt, time, date
        );

        let user = self.config.user_prompt.replace("YYYY-MM-DD", &date);

        (system, user)
    }

    /// Mark that we've flushed this cycle.
    pub fn mark_flushed(&mut self) {
        self.flushed_this_cycle = true;
    }

    /// Reset for a new compaction cycle.
    pub fn reset_cycle(&mut self) {
        self.flushed_this_cycle = false;
    }

    /// Check if flush is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_flush_at_threshold() {
        let config = MemoryFlushConfig::default();
        let flush = MemoryFlush::new(config);

        // 100k max, 0.75 compaction threshold = 75k compaction point
        // 75k - 4k soft threshold = 71k flush point
        assert!(!flush.should_flush(70000, 100000, 0.75));
        assert!(flush.should_flush(71000, 100000, 0.75));
        assert!(flush.should_flush(75000, 100000, 0.75));
    }

    #[test]
    fn test_flush_only_once_per_cycle() {
        let config = MemoryFlushConfig::default();
        let mut flush = MemoryFlush::new(config);

        assert!(flush.should_flush(75000, 100000, 0.75));
        flush.mark_flushed();
        assert!(!flush.should_flush(75000, 100000, 0.75));

        flush.reset_cycle();
        assert!(flush.should_flush(75000, 100000, 0.75));
    }

    #[test]
    fn test_disabled_flush() {
        let config = MemoryFlushConfig {
            enabled: false,
            ..Default::default()
        };
        let flush = MemoryFlush::new(config);

        assert!(!flush.should_flush(100000, 100000, 0.75));
    }

    #[test]
    fn test_build_flush_messages() {
        let config = MemoryFlushConfig::default();
        let flush = MemoryFlush::new(config);

        let (system, user) = flush.build_flush_messages();

        assert!(system.contains("Pre-compaction memory flush"));
        assert!(system.contains("UTC"));
        assert!(!user.contains("YYYY-MM-DD")); // Should be substituted
    }
}
