//! Memory Tree — hierarchical summary-tree memory store for AI agents.
//!
//! # Scope
//!
//! memory-tree handles **external data ingest**: emails, scrapes, ingested
//! documents — keyed by `(source, source_id)`, with a leaf lifecycle
//! (`pending_extraction → admitted → buffered → sealed`). It is *not* a
//! conversation memory; for the agent's own chat history use the
//! `steel_memory` integration in `rustyclaw-core`.
//!
//! The [`Summarizer`] trait is owned by this crate; implementations can be
//! deterministic (see [`ConcatSummarizer`]) or call an LLM. Steel Memory is
//! orthogonal — it provides vector embeddings and a knowledge graph, and can
//! be wired alongside memory-tree as a secondary index over the same chunks.
//!
//! # Pipeline
//!
//! 1. **Ingest** canonicalizes input to markdown and chunks it (≤3k-tokens
//!    each, deterministic SHA-256 IDs).
//! 2. **Score** runs cheap heuristics so low-signal chrome is dropped before
//!    it touches the model.
//! 3. **Buffer** appends admitted chunks to a per-source L0 buffer.
//! 4. **Seal** compresses a full buffer into an L1 summary via the
//!    [`Summarizer`]. The cascade up to L2, topic trees, and the global
//!    daily digest is scaffolded but not materialized in this initial cut.
//! 5. **Retrieve** searches chunks and summaries via SQLite FTS5; scope can
//!    be the whole store or a single source.
//!
//! The store lives in a single SQLite file (`chunks.db` by convention) and
//! survives process restarts.

pub mod chunker;
pub mod error;
pub mod indexer;
pub mod queue;
pub mod retrieval;
pub mod score;
pub mod store;
pub mod summarizer;
pub mod trees;

pub use chunker::{Chunk, ChunkerOptions, chunk, chunk_with};
pub use error::{MemoryTreeError, Result};
pub use indexer::{Indexer, NoopIndexer, SemanticHit};
pub use queue::{Job, JobKind, Queue};
pub use retrieval::{HitKind, Retrieval, Scope, SearchHit};
pub use score::fast_score;
pub use store::{LeafStatus, Store, StoredChunk, Summary};
pub use summarizer::{ConcatSummarizer, SummaryEntry, Summarizer, SummaryKind};
pub use trees::{SourceTree, TreeOptions};

use std::path::Path;
use std::sync::Arc;

/// Facade tying the pieces together with sensible defaults.
pub struct MemoryTree {
    store: Arc<Store>,
    queue: Queue,
    tree: SourceTree,
    retrieval: Retrieval,
}

impl MemoryTree {
    /// Open a store at the given path with the supplied summarizer. Path may
    /// be `:memory:` to use an in-memory store (testing).
    pub fn open(path: &Path, summarizer: Arc<dyn Summarizer>) -> Result<Self> {
        let store = Arc::new(if path == Path::new(":memory:") {
            Store::in_memory()?
        } else {
            Store::open(path)?
        });
        let queue = Queue::new(Arc::clone(&store));
        let tree = SourceTree::new(Arc::clone(&store), summarizer, TreeOptions::default());
        let retrieval = Retrieval::new(Arc::clone(&store));
        Ok(Self {
            store,
            queue,
            tree,
            retrieval,
        })
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }
    pub fn queue(&self) -> &Queue {
        &self.queue
    }
    pub fn tree(&self) -> &SourceTree {
        &self.tree
    }
    pub fn retrieval(&self) -> &Retrieval {
        &self.retrieval
    }

    /// Ingest a canonicalized markdown document under `source`. Inserts
    /// chunks, scores them, and enqueues extraction follow-up jobs.
    ///
    /// `source` is a stable identifier for the source (e.g. `gmail/inbox`,
    /// `slack/eng-discuss`). `source_id` is the document id within that
    /// source (e.g. message id).
    pub fn ingest(
        &self,
        source: &str,
        source_id: &str,
        content: &str,
    ) -> Result<IngestReport> {
        let chunks = chunk(source_id, content);
        let scores: Vec<f64> = chunks.iter().map(fast_score).collect();
        let inserted = self.store.insert_chunks(source, &chunks, &scores)?;

        let mut enqueued = 0usize;
        for c in &chunks {
            let dedupe = format!("extract:{}", c.id);
            let payload = serde_json::json!({"source": source, "chunk_id": c.id});
            if self
                .queue
                .enqueue(JobKind::ExtractChunk, payload, Some(&dedupe))?
                .is_some()
            {
                enqueued += 1;
            }
        }
        Ok(IngestReport {
            chunks_seen: chunks.len(),
            chunks_inserted: inserted,
            jobs_enqueued: enqueued,
        })
    }

    /// Drain one job from the queue and process it. Returns `Ok(true)` if a
    /// job was processed, `Ok(false)` if the queue was empty.
    pub async fn process_one(&self) -> Result<bool> {
        let Some(job) = self.queue.reserve()? else {
            return Ok(false);
        };
        match self.process_job(&job).await {
            Ok(()) => {
                self.queue.complete(job.id)?;
                Ok(true)
            }
            Err(e) => {
                let msg = format!("{}", e);
                let _ = self.queue.fail(job.id, &msg, None);
                Err(e)
            }
        }
    }

    async fn process_job(&self, job: &Job) -> Result<()> {
        match job.kind {
            JobKind::ExtractChunk => {
                let chunk_id = job
                    .payload
                    .get("chunk_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        MemoryTreeError::InvalidInput("missing chunk_id".into())
                    })?;
                let status = self.tree.extract_and_buffer(chunk_id).await?;
                if status == LeafStatus::Buffered {
                    // Did this buffer just become full? Enqueue a seal.
                    let source = job
                        .payload
                        .get("source")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            MemoryTreeError::InvalidInput("missing source".into())
                        })?;
                    if self.tree.ready_to_seal(source)? {
                        let payload = serde_json::json!({"source": source});
                        let dedupe = format!("seal:{}", source);
                        let _ = self
                            .queue
                            .enqueue(JobKind::Seal, payload, Some(&dedupe))?;
                    }
                }
            }
            JobKind::Seal => {
                let source = job
                    .payload
                    .get("source")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| MemoryTreeError::InvalidInput("missing source".into()))?;
                self.tree.seal_buffer(source).await?;
            }
            // Not yet implemented in this initial cut.
            JobKind::AppendBuffer
            | JobKind::TopicRoute
            | JobKind::DigestDaily
            | JobKind::FlushStale => {}
        }
        Ok(())
    }

    pub fn search(&self, query: &str, scope: &Scope, limit: usize) -> Result<Vec<SearchHit>> {
        self.retrieval.search(query, scope, limit)
    }
}

#[derive(Debug, Clone)]
pub struct IngestReport {
    pub chunks_seen: usize,
    pub chunks_inserted: usize,
    pub jobs_enqueued: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ingest_then_search_finds_chunk() {
        let mt = MemoryTree::open(
            Path::new(":memory:"),
            Arc::new(ConcatSummarizer::default()),
        )
        .unwrap();
        let report = mt
            .ingest(
                "notes/test",
                "doc-1",
                "The quick brown fox jumps over the lazy dog.",
            )
            .unwrap();
        assert_eq!(report.chunks_inserted, 1);
        assert_eq!(report.jobs_enqueued, 1);

        let hits = mt
            .search("brown fox", &Scope::Global, 10)
            .unwrap();
        assert!(!hits.is_empty());
    }

    #[tokio::test]
    async fn end_to_end_ingest_extract_seal() {
        let mt = MemoryTree::open(
            Path::new(":memory:"),
            Arc::new(ConcatSummarizer::default()),
        )
        .unwrap();
        // Ingest enough small docs to trigger a seal (default threshold = 8).
        for i in 0..8 {
            let body = format!(
                "Document number {}. It has a heading\n# Title\nand a useful body about topics like alpha, beta, and gamma.\n\n## Subhead\nMore content goes here. Enough to clear the length penalty.",
                i
            );
            mt.ingest("gmail/inbox", &format!("msg-{}", i), &body).unwrap();
        }
        // Drain extract jobs first, then a seal will land on the queue.
        let mut processed = 0;
        while mt.process_one().await.unwrap() {
            processed += 1;
            if processed > 50 {
                panic!("too many jobs — runaway loop");
            }
        }
        // At least one summary should have been written by the seal job.
        assert!(mt.store.count_summaries().unwrap() >= 1);
    }
}
