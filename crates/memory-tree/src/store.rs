//! SQLite-backed persistence: chunks, leaves, summaries, and the job queue.

use crate::chunker::Chunk;
use crate::error::{MemoryTreeError, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

/// Lifecycle of a chunk in the tree pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafStatus {
    PendingExtraction,
    Admitted,
    Buffered,
    Sealed,
    Dropped,
}

impl LeafStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::PendingExtraction => "pending_extraction",
            Self::Admitted => "admitted",
            Self::Buffered => "buffered",
            Self::Sealed => "sealed",
            Self::Dropped => "dropped",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending_extraction" => Self::PendingExtraction,
            "admitted" => Self::Admitted,
            "buffered" => Self::Buffered,
            "sealed" => Self::Sealed,
            "dropped" => Self::Dropped,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub index: usize,
    pub content: String,
    pub status: LeafStatus,
    pub fast_score: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub id: i64,
    pub source: String,
    pub level: u8,
    pub content: String,
    pub child_chunk_ids: Vec<String>,
    pub child_summary_ids: Vec<i64>,
    pub created_at: DateTime<Utc>,
}

impl Store {
    /// Open (or create) a memory-tree store at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// In-memory store, for tests.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                source_id TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                status TEXT NOT NULL,
                fast_score REAL NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS chunks_by_source ON chunks(source);
            CREATE INDEX IF NOT EXISTS chunks_by_status ON chunks(status);

            CREATE TABLE IF NOT EXISTS summaries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                level INTEGER NOT NULL,
                content TEXT NOT NULL,
                child_chunk_ids TEXT NOT NULL,
                child_summary_ids TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS summaries_by_source ON summaries(source);
            CREATE INDEX IF NOT EXISTS summaries_by_level ON summaries(level);

            CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                dedupe_key TEXT,
                lease_until TEXT,
                attempts INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                scheduled_at TEXT NOT NULL,
                completed_at TEXT,
                last_error TEXT
            );
            CREATE INDEX IF NOT EXISTS jobs_due ON jobs(completed_at, scheduled_at);
            CREATE UNIQUE INDEX IF NOT EXISTS jobs_dedupe ON jobs(dedupe_key)
                WHERE dedupe_key IS NOT NULL AND completed_at IS NULL;

            -- Full-text search over chunks and summaries.
            CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
                id UNINDEXED, source UNINDEXED, content
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS summary_fts USING fts5(
                id UNINDEXED, source UNINDEXED, level UNINDEXED, content
            );
            "#,
        )?;
        Ok(())
    }

    pub fn insert_chunks(
        &self,
        source: &str,
        chunks: &[Chunk],
        fast_scores: &[f64],
    ) -> Result<usize> {
        if chunks.is_empty() {
            return Ok(0);
        }
        if chunks.len() != fast_scores.len() {
            return Err(MemoryTreeError::InvalidInput(
                "chunks and fast_scores length mismatch".into(),
            ));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = Utc::now().to_rfc3339();
        let mut inserted = 0usize;
        {
            let mut insert = tx.prepare(
                "INSERT OR IGNORE INTO chunks
                    (id, source, source_id, chunk_index, content, status, fast_score, created_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let mut fts =
                tx.prepare("INSERT INTO chunk_fts (id, source, content) VALUES (?1, ?2, ?3)")?;
            for (c, score) in chunks.iter().zip(fast_scores.iter()) {
                let n = insert.execute(params![
                    &c.id,
                    source,
                    &c.source_id,
                    c.index as i64,
                    &c.content,
                    LeafStatus::PendingExtraction.as_str(),
                    score,
                    &now,
                ])?;
                if n > 0 {
                    fts.execute(params![&c.id, source, &c.content])?;
                    inserted += 1;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn set_status(&self, chunk_id: &str, status: LeafStatus) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE chunks SET status = ?1 WHERE id = ?2",
            params![status.as_str(), chunk_id],
        )?;
        Ok(())
    }

    pub fn get_chunk(&self, chunk_id: &str) -> Result<Option<StoredChunk>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT id, source, source_id, chunk_index, content, status, fast_score, created_at
                 FROM chunks WHERE id = ?1",
                params![chunk_id],
                |r| {
                    Ok(StoredChunk {
                        id: r.get(0)?,
                        source: r.get(1)?,
                        source_id: r.get(2)?,
                        index: r.get::<_, i64>(3)? as usize,
                        content: r.get(4)?,
                        status: LeafStatus::parse(&r.get::<_, String>(5)?)
                            .unwrap_or(LeafStatus::PendingExtraction),
                        fast_score: r.get(6)?,
                        created_at: r.get::<_, String>(7)?.parse::<DateTime<Utc>>().map_err(
                            |e| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    7,
                                    rusqlite::types::Type::Text,
                                    Box::new(e),
                                )
                            },
                        )?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    pub fn buffered_chunks(&self, source: &str) -> Result<Vec<StoredChunk>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source, source_id, chunk_index, content, status, fast_score, created_at
             FROM chunks
             WHERE source = ?1 AND status = 'buffered'
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(params![source], |r| {
                Ok(StoredChunk {
                    id: r.get(0)?,
                    source: r.get(1)?,
                    source_id: r.get(2)?,
                    index: r.get::<_, i64>(3)? as usize,
                    content: r.get(4)?,
                    status: LeafStatus::parse(&r.get::<_, String>(5)?)
                        .unwrap_or(LeafStatus::Buffered),
                    fast_score: r.get(6)?,
                    created_at: r
                        .get::<_, String>(7)?
                        .parse::<DateTime<Utc>>()
                        .map_err(|e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                7,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        })?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_summary(&self, summary: &Summary) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let child_chunks = serde_json::to_string(&summary.child_chunk_ids)?;
        let child_sums = serde_json::to_string(&summary.child_summary_ids)?;
        let now = summary.created_at.to_rfc3339();
        conn.execute(
            "INSERT INTO summaries (source, level, content, child_chunk_ids, child_summary_ids, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![&summary.source, summary.level as i64, &summary.content, child_chunks, child_sums, &now],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO summary_fts (id, source, level, content) VALUES (?1, ?2, ?3, ?4)",
            params![id, &summary.source, summary.level as i64, &summary.content],
        )?;
        Ok(id)
    }

    pub fn count_chunks(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn count_summaries(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM summaries", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Run a closure with a locked connection. Escape hatch for the queue
    /// module without leaking rusqlite types in the public API.
    pub(crate) fn with_conn<R>(&self, f: impl FnOnce(&mut Connection) -> Result<R>) -> Result<R> {
        let mut conn = self.conn.lock().unwrap();
        f(&mut conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::chunk;

    #[test]
    fn insert_chunk_then_read() {
        let store = Store::in_memory().unwrap();
        let chunks = chunk("doc-1", "Hello world");
        let n = store.insert_chunks("gmail/inbox", &chunks, &[0.5]).unwrap();
        assert_eq!(n, 1);
        let got = store.get_chunk(&chunks[0].id).unwrap().unwrap();
        assert_eq!(got.content, "Hello world");
        assert_eq!(got.status, LeafStatus::PendingExtraction);
        assert_eq!(got.fast_score, 0.5);
    }

    #[test]
    fn duplicate_chunk_ids_are_idempotent() {
        let store = Store::in_memory().unwrap();
        let chunks = chunk("doc-1", "Hello world");
        let _ = store.insert_chunks("s", &chunks, &[0.0]).unwrap();
        let n = store.insert_chunks("s", &chunks, &[0.0]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(store.count_chunks().unwrap(), 1);
    }

    #[test]
    fn status_transitions_persist() {
        let store = Store::in_memory().unwrap();
        let chunks = chunk("doc-1", "Hello world");
        store.insert_chunks("s", &chunks, &[0.0]).unwrap();
        store
            .set_status(&chunks[0].id, LeafStatus::Admitted)
            .unwrap();
        let got = store.get_chunk(&chunks[0].id).unwrap().unwrap();
        assert_eq!(got.status, LeafStatus::Admitted);
    }

    #[test]
    fn summary_insert_and_count() {
        let store = Store::in_memory().unwrap();
        let s = Summary {
            id: 0,
            source: "s".into(),
            level: 1,
            content: "summary text".into(),
            child_chunk_ids: vec!["a".into()],
            child_summary_ids: vec![],
            created_at: Utc::now(),
        };
        let id = store.insert_summary(&s).unwrap();
        assert!(id > 0);
        assert_eq!(store.count_summaries().unwrap(), 1);
    }
}
