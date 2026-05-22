//! Bridge that lets `memory-tree` use `steel-memory` as its secondary
//! (semantic) index.
//!
//! `memory-tree` defines the [`Indexer`](memory_tree::Indexer) trait but
//! deliberately does not depend on `steel-memory`. This module supplies the
//! implementation in the one crate where both are visible.
//!
//! # Mapping
//!
//! memory-tree's `(source, source_id)` is mapped onto steel-memory's
//! `(wing, room, source_file)`:
//!
//! | memory-tree | steel-memory  |
//! |-------------|---------------|
//! | `source`    | `room`        |
//! | constant    | `wing` = `"ingest"` |
//! | `source_id` | `source_file` |
//!
//! All ingested chunks land in a fixed `"ingest"` wing so they're easy to
//! sweep separately from chat-derived memories.

use crate::steel_memory::SteelMemory;
use async_trait::async_trait;
use memory_tree::{Indexer, MemoryTreeError, Result as MtResult, SemanticHit, StoredChunk};
use std::sync::Arc;

/// Wing every ingested chunk lives in. Stable so semantic search can scope.
pub const INGEST_WING: &str = "ingest";

/// `Indexer` impl that mirrors every admitted chunk into steel-memory.
pub struct SteelMemoryIndexer {
    steel: Arc<SteelMemory>,
}

impl SteelMemoryIndexer {
    pub fn new(steel: Arc<SteelMemory>) -> Self {
        Self { steel }
    }
}

#[async_trait]
impl Indexer for SteelMemoryIndexer {
    fn name(&self) -> &str {
        "steel_memory"
    }

    async fn index_chunk(&self, source: &str, chunk: &StoredChunk) -> MtResult<()> {
        self.steel
            .add_memory(&chunk.content, INGEST_WING, source, Some(&chunk.source_id))
            .await
            .map(|_id| ())
            .map_err(MemoryTreeError::Summarizer)
    }

    async fn semantic_search(
        &self,
        query: &str,
        source: Option<&str>,
        limit: usize,
    ) -> MtResult<Option<Vec<SemanticHit>>> {
        let results = self
            .steel
            .search(query, limit * 2, None)
            .await
            .map_err(MemoryTreeError::Summarizer)?;
        let hits: Vec<SemanticHit> = results
            .into_iter()
            .filter(|r| r.wing == INGEST_WING)
            .filter(|r| match source {
                Some(s) => r.room == s,
                None => true,
            })
            .take(limit)
            .map(|r| SemanticHit {
                chunk_id: r.id,
                source: r.room,
                snippet: r.content,
                similarity: r.similarity,
            })
            .collect();
        Ok(Some(hits))
    }
}
