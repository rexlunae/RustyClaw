//! Source tree mechanics: L0 buffer fills with admitted leaves; when it
//! reaches `seal_threshold`, a `seal` job summarizes the buffer into an L1
//! summary node. Topic and global trees are designed in the same shape but
//! aren't materialized in this initial implementation.

use crate::error::Result;
use crate::indexer::Indexer;
use crate::store::{LeafStatus, Store, Summary};
use crate::summarizer::{Summarizer, SummaryEntry, SummaryKind};
use chrono::Utc;
use std::sync::Arc;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct TreeOptions {
    /// Number of admitted leaves required to trigger an L0 → L1 seal.
    pub seal_threshold: usize,
    /// Minimum fast_score to admit a chunk into the buffer (else dropped).
    pub admit_threshold: f64,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self {
            seal_threshold: 8,
            admit_threshold: 0.25,
        }
    }
}

pub struct SourceTree {
    store: Arc<Store>,
    summarizer: Arc<dyn Summarizer>,
    indexer: Option<Arc<dyn Indexer>>,
    opts: TreeOptions,
}

impl SourceTree {
    pub fn new(store: Arc<Store>, summarizer: Arc<dyn Summarizer>, opts: TreeOptions) -> Self {
        Self {
            store,
            summarizer,
            indexer: None,
            opts,
        }
    }

    /// Attach an optional secondary [`Indexer`] (e.g. a vector store). Called
    /// for every admitted chunk on its way into the buffer.
    pub fn with_indexer(mut self, indexer: Arc<dyn Indexer>) -> Self {
        self.indexer = Some(indexer);
        self
    }

    /// Take a chunk through extract → admit/drop → buffer.
    pub async fn extract_and_buffer(&self, chunk_id: &str) -> Result<LeafStatus> {
        let Some(c) = self.store.get_chunk(chunk_id)? else {
            return Err(crate::error::MemoryTreeError::InvalidInput(format!(
                "no such chunk {}",
                chunk_id
            )));
        };
        if c.fast_score < self.opts.admit_threshold {
            self.store.set_status(chunk_id, LeafStatus::Dropped)?;
            debug!(chunk_id, score = c.fast_score, "chunk dropped");
            return Ok(LeafStatus::Dropped);
        }
        self.store.set_status(chunk_id, LeafStatus::Buffered)?;

        if let Some(indexer) = &self.indexer {
            if let Err(e) = indexer.index_chunk(&c.source, &c).await {
                warn!(
                    chunk_id,
                    indexer = indexer.name(),
                    error = %e,
                    "secondary indexer failed; primary FTS5 index unaffected"
                );
            }
        }
        Ok(LeafStatus::Buffered)
    }

    /// True when the source's L0 buffer is full enough to seal.
    pub fn ready_to_seal(&self, source: &str) -> Result<bool> {
        let buffered = self.store.buffered_chunks(source)?;
        Ok(buffered.len() >= self.opts.seal_threshold)
    }

    /// Seal the buffer into an L1 summary. Idempotent at the leaf level — all
    /// participating chunks are marked `sealed`, and the summary is stored
    /// with their ids in `child_chunk_ids`.
    pub async fn seal_buffer(&self, source: &str) -> Result<Option<i64>> {
        let buffered = self.store.buffered_chunks(source)?;
        if buffered.is_empty() {
            return Ok(None);
        }
        let entries: Vec<SummaryEntry> = buffered.iter().map(SummaryEntry::from_chunk).collect();
        let summary_text = self
            .summarizer
            .summarize(&entries, SummaryKind::Leaf)
            .await?;
        let summary = Summary {
            id: 0,
            source: source.to_string(),
            level: 1,
            content: summary_text,
            child_chunk_ids: buffered.iter().map(|c| c.id.clone()).collect(),
            child_summary_ids: vec![],
            created_at: Utc::now(),
        };
        let id = self.store.insert_summary(&summary)?;
        for c in &buffered {
            self.store.set_status(&c.id, LeafStatus::Sealed)?;
        }
        Ok(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::chunk;
    use crate::summarizer::ConcatSummarizer;

    fn tree() -> (Arc<Store>, SourceTree) {
        let store = Arc::new(Store::in_memory().unwrap());
        let tree = SourceTree::new(
            Arc::clone(&store),
            Arc::new(ConcatSummarizer::default()),
            TreeOptions {
                seal_threshold: 3,
                admit_threshold: 0.0,
            },
        );
        (store, tree)
    }

    #[tokio::test]
    async fn end_to_end_buffer_then_seal() {
        let (store, tree) = tree();
        for i in 0..3 {
            let chunks = chunk(&format!("msg-{}", i), &format!("Body {}", i));
            store.insert_chunks("gmail/inbox", &chunks, &[0.7]).unwrap();
            tree.extract_and_buffer(&chunks[0].id).await.unwrap();
        }
        assert!(tree.ready_to_seal("gmail/inbox").unwrap());
        let id = tree.seal_buffer("gmail/inbox").await.unwrap();
        assert!(id.is_some());
        assert_eq!(store.count_summaries().unwrap(), 1);
    }

    #[tokio::test]
    async fn low_score_chunks_dropped_not_buffered() {
        let store = Arc::new(Store::in_memory().unwrap());
        let tree = SourceTree::new(
            Arc::clone(&store),
            Arc::new(ConcatSummarizer::default()),
            TreeOptions {
                seal_threshold: 1,
                admit_threshold: 0.5,
            },
        );
        let chunks = chunk("m", "body");
        store.insert_chunks("s", &chunks, &[0.1]).unwrap();
        let status = tree.extract_and_buffer(&chunks[0].id).await.unwrap();
        assert_eq!(status, LeafStatus::Dropped);
        assert!(!tree.ready_to_seal("s").unwrap());
    }

    #[tokio::test]
    async fn indexer_called_on_admitted_chunks() {
        use crate::indexer::Indexer;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(Arc<AtomicUsize>);
        #[async_trait::async_trait]
        impl Indexer for Counting {
            fn name(&self) -> &str {
                "counting"
            }
            async fn index_chunk(
                &self,
                _source: &str,
                _chunk: &crate::store::StoredChunk,
            ) -> Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        let n = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(Store::in_memory().unwrap());
        let tree = SourceTree::new(
            Arc::clone(&store),
            Arc::new(ConcatSummarizer::default()),
            TreeOptions {
                seal_threshold: 10,
                admit_threshold: 0.0,
            },
        )
        .with_indexer(Arc::new(Counting(Arc::clone(&n))));

        let chunks = chunk("doc", "alpha beta gamma");
        store.insert_chunks("src", &chunks, &[0.7]).unwrap();
        tree.extract_and_buffer(&chunks[0].id).await.unwrap();

        assert_eq!(n.load(Ordering::SeqCst), 1);
    }
}
