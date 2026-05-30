//! Summarization abstraction for the memory-tree.
//!
//! memory-tree owns the trait because no upstream crate provides one that
//! fits its needs. The trait is intentionally narrow: callers hand it a slice
//! of [`SummaryEntry`]s and a [`SummaryKind`] and get back a single string.
//! Implementations can be deterministic (for tests), call an LLM, or stub out
//! to anything else.

use crate::error::Result;
use crate::store::StoredChunk;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// Kind of summary being generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryKind {
    /// Summarizing raw chunks into an L1 summary.
    Leaf,
    /// Summarizing L_n summaries into an L_{n+1} summary.
    Condensed,
}

/// One unit of input to the summarizer. Decoupled from [`StoredChunk`] so the
/// trait can also be called on the output of an earlier summarization round
/// (where there is no underlying chunk).
#[derive(Debug, Clone)]
pub struct SummaryEntry {
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl SummaryEntry {
    pub fn from_chunk(c: &StoredChunk) -> Self {
        Self {
            role: "source".to_string(),
            content: c.content.clone(),
            created_at: c.created_at,
        }
    }
}

#[async_trait]
pub trait Summarizer: Send + Sync {
    /// A human-readable name. Used in logs and debug surfaces.
    fn name(&self) -> &str;

    /// Summarize a slice of entries into a single string.
    async fn summarize(&self, entries: &[SummaryEntry], kind: SummaryKind) -> Result<String>;
}

/// Deterministic fallback summarizer — concatenates inputs with head/tail
/// truncation per item. Useful for tests, and as a safe default when no LLM
/// is configured.
pub struct ConcatSummarizer {
    pub head_chars: usize,
    pub tail_chars: usize,
}

impl Default for ConcatSummarizer {
    fn default() -> Self {
        Self {
            head_chars: 2_000,
            tail_chars: 500,
        }
    }
}

#[async_trait]
impl Summarizer for ConcatSummarizer {
    fn name(&self) -> &str {
        "concat"
    }

    async fn summarize(&self, entries: &[SummaryEntry], kind: SummaryKind) -> Result<String> {
        let mut out = format!("[{:?}] {} items\n\n", kind, entries.len());
        for (i, e) in entries.iter().enumerate() {
            out.push_str(&format!("--- item {} ({}) ---\n", i, e.role));
            if e.content.chars().count() > self.head_chars + self.tail_chars {
                let head: String = e.content.chars().take(self.head_chars).collect();
                let tail: String = e
                    .content
                    .chars()
                    .skip(e.content.chars().count() - self.tail_chars)
                    .collect();
                out.push_str(&head);
                out.push_str("\n… [middle elided]\n");
                out.push_str(&tail);
            } else {
                out.push_str(&e.content);
            }
            out.push_str("\n\n");
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concat_summarizer_emits_header() {
        let s = ConcatSummarizer::default();
        let entries = vec![
            SummaryEntry {
                role: "source".into(),
                content: "alpha".into(),
                created_at: Utc::now(),
            },
            SummaryEntry {
                role: "source".into(),
                content: "beta".into(),
                created_at: Utc::now(),
            },
        ];
        let out = s.summarize(&entries, SummaryKind::Leaf).await.unwrap();
        assert!(out.contains("2 items"));
        assert!(out.contains("--- item 0 (source) ---"));
        assert!(out.contains("alpha"));
    }
}
