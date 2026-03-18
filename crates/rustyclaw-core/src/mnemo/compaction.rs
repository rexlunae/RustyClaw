//! Background compaction runner.

use super::traits::{MemoryStore, Summarizer};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

/// Run compaction in a background loop.
///
/// This function spawns a task that periodically checks and runs compaction.
pub async fn run_compaction(
    store: Arc<dyn MemoryStore>,
    summarizer: Arc<dyn Summarizer>,
    interval: Duration,
) -> Result<()> {
    loop {
        tokio::time::sleep(interval).await;
        
        match store.compact(summarizer.as_ref()).await {
            Ok(stats) => {
                if stats.summaries_created > 0 {
                    tracing::info!(
                        store = store.name(),
                        messages_compacted = stats.messages_compacted,
                        summaries_created = stats.summaries_created,
                        tokens_saved = stats.tokens_saved,
                        duration_ms = stats.duration.as_millis() as u64,
                        "Memory compaction completed"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    store = store.name(),
                    error = %e,
                    "Memory compaction failed"
                );
            }
        }
    }
}
