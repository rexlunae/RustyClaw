//! Core memory storage and summarization traits.
//!
//! Follows existing RustyClaw patterns (RuntimeAdapter, Observer, Messenger).

use anyhow::Result;
use async_trait::async_trait;
use std::time::Duration;

/// A single memory entry (message or summary).
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    /// Unique identifier.
    pub id: i64,
    /// Message role ("user", "assistant", "system").
    pub role: String,
    /// Content text.
    pub content: String,
    /// Estimated token count.
    pub token_count: usize,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: i64,
    /// Summary depth: 0 = raw message, 1+ = summary level.
    pub depth: u8,
}

/// Search result with relevance score.
#[derive(Debug, Clone)]
pub struct MemoryHit {
    /// The matching entry.
    pub entry: MemoryEntry,
    /// Relevance score (higher is better).
    pub score: f32,
    /// Highlighted snippet for display.
    pub snippet: String,
}

/// Statistics from a compaction pass.
#[derive(Debug, Default)]
pub struct CompactionStats {
    /// Number of messages compacted into summaries.
    pub messages_compacted: usize,
    /// Number of new summaries created.
    pub summaries_created: usize,
    /// Estimated tokens saved.
    pub tokens_saved: usize,
    /// Time taken for compaction.
    pub duration: Duration,
}

/// Core memory storage trait.
///
/// Implementations must be `Send + Sync` for sharing across async tasks.
/// The trait abstracts over storage backends (SQLite, in-memory, remote)
/// while ensuring consistent behavior for compaction and retrieval.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Return the human-readable name of this memory backend.
    fn name(&self) -> &str;

    /// Ingest a new message into the store.
    ///
    /// Called synchronously on the message processing path.
    /// Implementations should avoid blocking I/O where possible.
    async fn ingest(&self, role: &str, content: &str, token_count: usize) -> Result<i64>;

    /// Full-text search across all stored messages and summaries.
    ///
    /// Returns up to `limit` results ordered by relevance score.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryHit>>;

    /// Generate context string for agent bootstrap.
    ///
    /// Returns a formatted string of recent messages and summaries,
    /// respecting `max_tokens` budget. Used for system prompt injection.
    async fn get_context(&self, max_tokens: usize) -> Result<String>;

    /// Get recent context entries for the agent.
    ///
    /// Returns the most recent entries up to `max_tokens` budget.
    async fn get_context_entries(&self, max_tokens: usize) -> Result<Vec<MemoryEntry>>;

    /// Run compaction if thresholds are exceeded.
    ///
    /// Compresses older messages into summaries using the configured
    /// summarization backend. Safe to call frequently; returns early
    /// if no compaction needed.
    async fn compact(&self, summarizer: &dyn Summarizer) -> Result<CompactionStats>;

    /// Return total message count (including compacted).
    async fn message_count(&self) -> Result<usize>;

    /// Return total summary count.
    async fn summary_count(&self) -> Result<usize>;

    /// Flush any buffered data to persistent storage.
    async fn flush(&self) -> Result<()> {
        Ok(())
    }
}

/// Kind of summary being generated.
#[derive(Debug, Clone, Copy)]
pub enum SummaryKind {
    /// Summarizing raw messages into a leaf summary.
    Leaf,
    /// Summarizing summaries into a higher-level condensed summary.
    Condensed,
}

/// Summarization backend for memory compaction.
///
/// Implementations call LLM APIs or use deterministic fallbacks.
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Return the backend name (e.g., "openrouter", "deterministic").
    fn name(&self) -> &str;

    /// Summarize a batch of entries into a single summary string.
    async fn summarize(&self, entries: &[MemoryEntry], kind: SummaryKind) -> Result<String>;
}
