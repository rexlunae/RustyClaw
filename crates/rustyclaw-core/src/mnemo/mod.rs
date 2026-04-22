//! Native memory coprocessor for persistent recall across sessions.
//!
//! Inspired by Mnemo Cortex, this module provides:
//! - SQLite + FTS5 storage for messages and summaries
//! - Background compaction via LLM summarization
//! - Context injection at agent bootstrap
//! - DAG-based summary lineage for expansion
//!
//! ## Trait Architecture
//!
//! Follows existing RustyClaw patterns:
//! - `MemoryStore` — storage abstraction (SQLite, in-memory, remote)
//! - `Summarizer` — summarization backend (LLM, deterministic)

mod compaction;
mod config;
mod schema;
mod sqlite_store;
mod summarizer;
#[cfg(test)]
mod tests;
mod traits;

pub use compaction::run_compaction;
pub use config::{MnemoConfig, SummarizationConfig};
pub use sqlite_store::SqliteMemoryStore;
pub use summarizer::{DeterministicSummarizer, LlmSummarizer};
pub use traits::{CompactionStats, MemoryEntry, MemoryHit, MemoryStore, Summarizer, SummaryKind};

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

/// Shared mnemo store for concurrent access.
pub type SharedMemoryStore = Arc<dyn MemoryStore>;

/// Create a shared memory store from configuration.
pub async fn create_memory_store(
    config: &MnemoConfig,
    settings_dir: &Path,
) -> Result<SharedMemoryStore> {
    let db_path = config
        .db_path
        .clone()
        .unwrap_or_else(|| settings_dir.join("mnemo.sqlite3"));

    let store = SqliteMemoryStore::open(&db_path, config.clone()).await?;
    Ok(Arc::new(store))
}

/// Create a summarizer based on configuration.
pub fn create_summarizer(config: &MnemoConfig) -> Arc<dyn Summarizer> {
    if config.summarization.use_main_model {
        // Will be wired to main provider later
        // For now, fall back to deterministic
        Arc::new(DeterministicSummarizer::new(
            config.summarization.truncate_chars,
            config.summarization.truncate_total,
        ))
    } else if let (Some(base_url), Some(model)) = (
        config.summarization.provider.as_ref(),
        config.summarization.model.as_ref(),
    ) {
        Arc::new(LlmSummarizer::new(
            base_url.clone(),
            model.clone(),
            None, // API key resolved elsewhere
        ))
    } else {
        Arc::new(DeterministicSummarizer::new(
            config.summarization.truncate_chars,
            config.summarization.truncate_total,
        ))
    }
}

/// Estimate token count for a string (rough heuristic: ~4 chars per token).
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 4.0).ceil() as usize
}

/// Generate context markdown for bootstrap injection.
pub fn generate_context_md(entries: &[MemoryEntry]) -> String {
    let mut lines = Vec::new();
    lines.push("# MNEMO CONTEXT".to_string());
    lines.push(String::new());

    for entry in entries {
        if entry.depth == 0 {
            lines.push(format!("## {} (msg #{})", entry.role, entry.id));
        } else {
            lines.push(format!("## Summary d{} #{}", entry.depth, entry.id));
        }
        lines.push(entry.content.clone());
        lines.push(String::new());
    }

    lines.join("\n")
}
