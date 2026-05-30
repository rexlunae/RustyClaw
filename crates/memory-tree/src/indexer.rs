//! Optional secondary index hook.
//!
//! Memory-tree's primary index is SQLite FTS5. For semantic search, a caller
//! can supply an [`Indexer`] that gets called on every admitted chunk —
//! typically a wrapper around a vector store. The trait lives here, but
//! production implementations live in their host crate (e.g.
//! `rustyclaw_core::steel_memory_indexer`) to avoid forcing the dep on every
//! memory-tree consumer.
//!
//! `Indexer::index` is called inside the buffer-append path so a slow
//! implementation will throttle ingest. If embeddings are expensive,
//! implementations should spawn their own background work and return promptly.

use crate::error::Result;
use crate::store::StoredChunk;
use async_trait::async_trait;

#[async_trait]
pub trait Indexer: Send + Sync {
    /// Identifier for logs / debug surfaces.
    fn name(&self) -> &str;

    /// Index a chunk that's just been admitted (passed scoring) and is about
    /// to enter the buffer. Failure is logged by the caller and does not abort
    /// ingest — the FTS5 path remains the primary index.
    async fn index_chunk(&self, source: &str, chunk: &StoredChunk) -> Result<()>;

    /// Optional: perform a semantic search. Returning `None` means "not
    /// supported"; the caller will fall back to FTS5-only.
    async fn semantic_search(
        &self,
        _query: &str,
        _source: Option<&str>,
        _limit: usize,
    ) -> Result<Option<Vec<SemanticHit>>> {
        Ok(None)
    }
}

/// One hit from a semantic search.
#[derive(Debug, Clone)]
pub struct SemanticHit {
    /// The chunk id the indexer associates with this hit.
    pub chunk_id: String,
    pub source: String,
    pub snippet: String,
    /// Similarity in `[0, 1]`. Higher is better.
    pub similarity: f32,
}

/// No-op default. Constructed via `Default::default()` so callers can opt in
/// without holding a real implementation.
#[derive(Debug, Default)]
pub struct NoopIndexer;

#[async_trait]
impl Indexer for NoopIndexer {
    fn name(&self) -> &str {
        "noop"
    }
    async fn index_chunk(&self, _source: &str, _chunk: &StoredChunk) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Arc<AtomicUsize>);

    #[async_trait]
    impl Indexer for Counting {
        fn name(&self) -> &str {
            "counting"
        }
        async fn index_chunk(&self, _source: &str, _chunk: &StoredChunk) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn mk_chunk(id: &str) -> StoredChunk {
        StoredChunk {
            id: id.into(),
            source: "src".into(),
            source_id: "doc".into(),
            index: 0,
            content: "body".into(),
            status: crate::store::LeafStatus::Admitted,
            fast_score: 1.0,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn noop_indexer_is_a_no_op() {
        let i = NoopIndexer;
        assert!(i.index_chunk("src", &mk_chunk("x")).await.is_ok());
        assert!(i.semantic_search("q", None, 5).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn indexer_called_per_chunk() {
        let n = Arc::new(AtomicUsize::new(0));
        let i = Counting(Arc::clone(&n));
        i.index_chunk("s", &mk_chunk("a")).await.unwrap();
        i.index_chunk("s", &mk_chunk("b")).await.unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }
}
