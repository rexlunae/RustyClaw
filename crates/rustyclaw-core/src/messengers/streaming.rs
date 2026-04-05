//! Streaming support for messenger channels.
//!
//! Provides utilities for streaming model responses to messenger channels
//! in real-time, rather than waiting for the full response. Different
//! messengers support different streaming strategies:
//!
//! - **Edit-based**: Send an initial message, then edit it as tokens arrive
//!   (Telegram, Discord, Slack).
//! - **Chunked**: Send partial messages at intervals (IRC, generic).
//! - **Draft**: Use platform-specific draft/typing APIs (where available).

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::debug;

/// Streaming strategy for a messenger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamStrategy {
    /// Send initial message, then edit in place as tokens arrive.
    EditInPlace,
    /// Accumulate tokens and send chunks at intervals.
    Chunked,
    /// Wait for full response before sending (no streaming).
    BufferAll,
}

impl Default for StreamStrategy {
    fn default() -> Self {
        Self::EditInPlace
    }
}

/// Configuration for messenger streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamConfig {
    /// Whether streaming is enabled for this messenger.
    #[serde(default)]
    pub enabled: bool,

    /// Streaming strategy.
    #[serde(default)]
    pub strategy: StreamStrategy,

    /// Minimum interval between message edits (milliseconds).
    /// Prevents rate-limiting from too-frequent edits.
    #[serde(default = "default_edit_interval_ms")]
    pub edit_interval_ms: u64,

    /// For chunked strategy: minimum characters before sending a chunk.
    #[serde(default = "default_chunk_min_chars")]
    pub chunk_min_chars: usize,

    /// Maximum message length before splitting into multiple messages.
    #[serde(default = "default_max_message_len")]
    pub max_message_len: usize,

    /// Whether to show a typing indicator while generating.
    #[serde(default = "default_true")]
    pub show_typing: bool,

    /// Suffix to append while streaming is in progress (e.g., " ▌").
    #[serde(default = "default_cursor")]
    pub streaming_cursor: String,
}

fn default_edit_interval_ms() -> u64 {
    500
}

fn default_chunk_min_chars() -> usize {
    100
}

fn default_max_message_len() -> usize {
    4000
}

fn default_true() -> bool {
    true
}

fn default_cursor() -> String {
    " ▌".to_string()
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            strategy: StreamStrategy::default(),
            edit_interval_ms: default_edit_interval_ms(),
            chunk_min_chars: default_chunk_min_chars(),
            max_message_len: default_max_message_len(),
            show_typing: true,
            streaming_cursor: default_cursor(),
        }
    }
}

/// Buffer that accumulates streaming tokens and decides when to flush.
pub struct StreamBuffer {
    /// Accumulated text.
    content: String,
    /// Last time a flush (edit/send) was performed.
    last_flush: Instant,
    /// Configuration.
    config: StreamConfig,
    /// Number of flushes performed.
    flush_count: usize,
    /// Whether the stream is complete.
    done: bool,
}

impl StreamBuffer {
    /// Create a new stream buffer.
    pub fn new(config: StreamConfig) -> Self {
        Self {
            content: String::new(),
            last_flush: Instant::now(),
            config,
            flush_count: 0,
            done: false,
        }
    }

    /// Add a text chunk to the buffer.
    pub fn push(&mut self, text: &str) {
        self.content.push_str(text);
    }

    /// Mark the stream as complete.
    pub fn finish(&mut self) {
        self.done = true;
    }

    /// Check if a flush is needed based on strategy and timing.
    pub fn should_flush(&self) -> bool {
        if self.done {
            return true;
        }

        let elapsed = self.last_flush.elapsed();
        let interval = Duration::from_millis(self.config.edit_interval_ms);

        match self.config.strategy {
            StreamStrategy::EditInPlace => elapsed >= interval && !self.content.is_empty(),
            StreamStrategy::Chunked => {
                elapsed >= interval && self.content.len() >= self.config.chunk_min_chars
            }
            StreamStrategy::BufferAll => self.done,
        }
    }

    /// Get the current content to send/edit.
    ///
    /// For EditInPlace: returns full accumulated content with cursor.
    /// For Chunked: returns the pending chunk and clears the buffer.
    pub fn flush(&mut self) -> Option<FlushAction> {
        if self.content.is_empty() && !self.done {
            return None;
        }

        self.last_flush = Instant::now();
        self.flush_count += 1;

        let action = match self.config.strategy {
            StreamStrategy::EditInPlace => {
                let display_text = if self.done {
                    self.content.clone()
                } else {
                    format!("{}{}", self.content, self.config.streaming_cursor)
                };

                if self.flush_count == 1 {
                    FlushAction::SendNew(display_text)
                } else {
                    FlushAction::EditExisting(display_text)
                }
            }
            StreamStrategy::Chunked => {
                let chunk = std::mem::take(&mut self.content);
                if chunk.is_empty() {
                    return None;
                }
                FlushAction::SendNew(chunk)
            }
            StreamStrategy::BufferAll => {
                if self.done {
                    FlushAction::SendNew(std::mem::take(&mut self.content))
                } else {
                    return None;
                }
            }
        };

        debug!(
            strategy = ?self.config.strategy,
            flush_count = self.flush_count,
            done = self.done,
            "Stream buffer flushed"
        );

        Some(action)
    }

    /// Check if streaming is complete.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Get current content length.
    pub fn content_len(&self) -> usize {
        self.content.len()
    }

    /// Check if content exceeds max message length and needs splitting.
    pub fn needs_split(&self) -> bool {
        self.content.len() > self.config.max_message_len
    }

    /// Split content into message-sized chunks.
    pub fn split_content(&self) -> Vec<String> {
        let max_len = self.config.max_message_len;
        if self.content.len() <= max_len {
            return vec![self.content.clone()];
        }

        let mut chunks = Vec::new();
        let mut remaining = self.content.as_str();

        while !remaining.is_empty() {
            if remaining.len() <= max_len {
                chunks.push(remaining.to_string());
                break;
            }

            // Find a char boundary at or before max_len to avoid
            // panicking when max_len falls inside a multi-byte codepoint.
            let mut boundary = max_len;
            while boundary > 0 && !remaining.is_char_boundary(boundary) {
                boundary -= 1;
            }

            // Try to split at a newline or space within the boundary
            let split_at = remaining[..boundary]
                .rfind('\n')
                .or_else(|| remaining[..boundary].rfind(' '))
                .unwrap_or(boundary);

            chunks.push(remaining[..split_at].to_string());
            remaining = remaining[split_at..].trim_start();
        }

        chunks
    }
}

/// Action to perform after flushing the stream buffer.
#[derive(Debug, Clone)]
pub enum FlushAction {
    /// Send a new message.
    SendNew(String),
    /// Edit the previously sent message.
    EditExisting(String),
}

/// Get the recommended stream strategy for a messenger type.
pub fn recommended_strategy(messenger_type: &str) -> StreamStrategy {
    match messenger_type {
        "telegram" | "discord" | "slack" => StreamStrategy::EditInPlace,
        "irc" | "webhook" => StreamStrategy::Chunked,
        "msteams" | "googlechat" => StreamStrategy::EditInPlace,
        "imessage" => StreamStrategy::BufferAll,
        _ => StreamStrategy::BufferAll,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_config_defaults() {
        let config = StreamConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.strategy, StreamStrategy::EditInPlace);
        assert_eq!(config.edit_interval_ms, 500);
        assert!(config.show_typing);
    }

    #[test]
    fn test_stream_buffer_edit_in_place() {
        let config = StreamConfig {
            enabled: true,
            strategy: StreamStrategy::EditInPlace,
            edit_interval_ms: 0, // immediate
            ..Default::default()
        };

        let mut buf = StreamBuffer::new(config);
        buf.push("Hello ");
        buf.push("world");

        // First flush should be SendNew
        let action = buf.flush().unwrap();
        assert!(matches!(action, FlushAction::SendNew(_)));

        buf.push("!");
        // Subsequent flush should be EditExisting
        let action = buf.flush().unwrap();
        assert!(matches!(action, FlushAction::EditExisting(_)));
    }

    #[test]
    fn test_stream_buffer_chunked() {
        let config = StreamConfig {
            enabled: true,
            strategy: StreamStrategy::Chunked,
            edit_interval_ms: 0,
            chunk_min_chars: 5,
            ..Default::default()
        };

        let mut buf = StreamBuffer::new(config);
        buf.push("Hello");

        let action = buf.flush().unwrap();
        assert!(matches!(action, FlushAction::SendNew(ref s) if s == "Hello"));

        // Buffer should be cleared after chunked flush
        assert_eq!(buf.content_len(), 0);
    }

    #[test]
    fn test_stream_buffer_buffer_all() {
        let config = StreamConfig {
            enabled: true,
            strategy: StreamStrategy::BufferAll,
            ..Default::default()
        };

        let mut buf = StreamBuffer::new(config);
        buf.push("Hello ");
        buf.push("world");

        // Should not flush until done
        assert!(!buf.should_flush());

        buf.finish();
        assert!(buf.should_flush());

        let action = buf.flush().unwrap();
        assert!(matches!(action, FlushAction::SendNew(ref s) if s == "Hello world"));
    }

    #[test]
    fn test_stream_buffer_cursor() {
        let config = StreamConfig {
            enabled: true,
            strategy: StreamStrategy::EditInPlace,
            edit_interval_ms: 0,
            streaming_cursor: " ▌".to_string(),
            ..Default::default()
        };

        let mut buf = StreamBuffer::new(config);
        buf.push("typing...");

        let action = buf.flush().unwrap();
        if let FlushAction::SendNew(text) = action {
            assert!(text.ends_with(" ▌"));
        }

        buf.finish();
        buf.push(""); // trigger final state
        let action = buf.flush().unwrap();
        if let FlushAction::EditExisting(text) = action {
            assert!(!text.ends_with(" ▌"));
        }
    }

    #[test]
    fn test_split_content() {
        let config = StreamConfig {
            max_message_len: 10,
            ..Default::default()
        };

        let mut buf = StreamBuffer::new(config);
        buf.push("Hello world, this is a test");

        let chunks = buf.split_content();
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 10 || !chunk.contains(' '));
        }
    }

    #[test]
    fn test_recommended_strategy() {
        assert_eq!(
            recommended_strategy("telegram"),
            StreamStrategy::EditInPlace
        );
        assert_eq!(recommended_strategy("irc"), StreamStrategy::Chunked);
        assert_eq!(recommended_strategy("imessage"), StreamStrategy::BufferAll);
        assert_eq!(
            recommended_strategy("unknown"),
            StreamStrategy::BufferAll
        );
    }
}
