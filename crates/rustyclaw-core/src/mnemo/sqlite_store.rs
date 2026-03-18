//! SQLite-backed memory store implementation.

use super::config::MnemoConfig;
use super::schema::{CURRENT_VERSION, SCHEMA, VERSION_CHECK};
use super::traits::{CompactionStats, MemoryEntry, MemoryHit, MemoryStore, SummaryKind, Summarizer};
use super::estimate_tokens;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// SQLite-backed memory store with FTS5 support.
pub struct SqliteMemoryStore {
    conn: Mutex<rusqlite::Connection>,
    config: MnemoConfig,
    /// Current conversation ID (set after first ingest).
    conversation_id: Mutex<Option<i64>>,
}

impl SqliteMemoryStore {
    /// Open or create a memory database at the given path.
    pub async fn open(path: &Path, config: MnemoConfig) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create mnemo directory: {:?}", parent))?;
        }

        // Open SQLite with WAL mode for better concurrency
        let conn = rusqlite::Connection::open(path)
            .with_context(|| format!("Failed to open mnemo database: {:?}", path))?;

        // Enable WAL mode
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        // Create schema
        conn.execute_batch(VERSION_CHECK)?;
        conn.execute_batch(SCHEMA)?;

        // Check/update version
        let version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if version < CURRENT_VERSION {
            conn.execute(
                "INSERT OR REPLACE INTO schema_version (version) VALUES (?)",
                [CURRENT_VERSION],
            )?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
            config,
            conversation_id: Mutex::new(None),
        })
    }

    /// Get or create the default conversation.
    fn ensure_conversation(&self) -> Result<i64> {
        let mut conv_id = self.conversation_id.lock().unwrap();
        if let Some(id) = *conv_id {
            return Ok(id);
        }

        let conn = self.conn.lock().unwrap();

        // Try to find existing
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM conversations WHERE agent_id = 'default' AND session_id = 'main'",
                [],
                |row| row.get(0),
            )
            .ok();

        let id = if let Some(id) = existing {
            id
        } else {
            conn.execute(
                "INSERT INTO conversations (agent_id, session_id) VALUES ('default', 'main')",
                [],
            )?;
            conn.last_insert_rowid()
        };

        *conv_id = Some(id);
        Ok(id)
    }

    /// Get compaction candidates (oldest non-fresh context items).
    fn get_compaction_candidates(&self, count: usize) -> Result<Vec<MemoryEntry>> {
        let conv_id = self.ensure_conversation()?;
        let conn = self.conn.lock().unwrap();

        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM context_items WHERE conversation_id = ?",
            [conv_id],
            |row| row.get(0),
        )?;

        let fresh_tail = self.config.fresh_tail_messages;
        if (total as usize) <= fresh_tail {
            return Ok(Vec::new());
        }

        let available = (total as usize) - fresh_tail;
        let to_get = count.min(available);

        let mut stmt = conn.prepare(
            "SELECT item_type, ref_id FROM context_items 
             WHERE conversation_id = ? 
             ORDER BY position ASC
             LIMIT ?",
        )?;

        let items: Vec<(String, i64)> = stmt
            .query_map(rusqlite::params![conv_id, to_get as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut entries = Vec::new();
        for (item_type, ref_id) in items {
            match item_type.as_str() {
                "message" => {
                    if let Ok(entry) = self.get_message_entry(&conn, ref_id) {
                        entries.push(entry);
                    }
                }
                "summary" => {
                    if let Ok(entry) = self.get_summary_entry(&conn, ref_id) {
                        entries.push(entry);
                    }
                }
                _ => {}
            }
        }

        Ok(entries)
    }

    fn get_message_entry(&self, conn: &rusqlite::Connection, id: i64) -> Result<MemoryEntry> {
        conn.query_row(
            "SELECT id, role, content, token_count, created_at FROM messages WHERE id = ?",
            [id],
            |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    token_count: row.get::<_, i32>(3)? as usize,
                    timestamp: row.get(4)?,
                    depth: 0,
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("Failed to get message {}: {}", id, e))
    }

    fn get_summary_entry(&self, conn: &rusqlite::Connection, id: i64) -> Result<MemoryEntry> {
        conn.query_row(
            "SELECT id, depth, content, token_count, created_at FROM summaries WHERE id = ?",
            [id],
            |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    role: "summary".to_string(),
                    content: row.get(2)?,
                    token_count: row.get::<_, i32>(3)? as usize,
                    timestamp: row.get(4)?,
                    depth: row.get::<_, i32>(1)? as u8,
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("Failed to get summary {}: {}", id, e))
    }

    /// Create a summary from messages.
    fn create_summary_from_messages(
        &self,
        message_ids: &[i64],
        summary_content: &str,
    ) -> Result<i64> {
        let conv_id = self.ensure_conversation()?;
        let conn = self.conn.lock().unwrap();
        let token_count = estimate_tokens(summary_content) as i32;

        // Insert summary at depth 0 (leaf)
        conn.execute(
            "INSERT INTO summaries (conversation_id, depth, content, token_count) VALUES (?, 0, ?, ?)",
            rusqlite::params![conv_id, summary_content, token_count],
        )?;

        let summary_id = conn.last_insert_rowid();

        // Link to source messages
        for &msg_id in message_ids {
            conn.execute(
                "INSERT INTO summary_messages (summary_id, message_id) VALUES (?, ?)",
                [summary_id, msg_id],
            )?;
        }

        // Remove source messages from context, add summary
        let mut positions_to_remove = Vec::new();
        for &msg_id in message_ids {
            let pos: Option<i64> = conn
                .query_row(
                    "SELECT position FROM context_items WHERE conversation_id = ? AND item_type = 'message' AND ref_id = ?",
                    rusqlite::params![conv_id, msg_id],
                    |row| row.get(0),
                )
                .ok();
            if let Some(p) = pos {
                positions_to_remove.push(p);
            }
        }

        // Delete old context items
        for &msg_id in message_ids {
            conn.execute(
                "DELETE FROM context_items WHERE conversation_id = ? AND item_type = 'message' AND ref_id = ?",
                rusqlite::params![conv_id, msg_id],
            )?;
        }

        // Insert summary at the lowest removed position
        if let Some(&min_pos) = positions_to_remove.iter().min() {
            conn.execute(
                "INSERT INTO context_items (conversation_id, item_type, ref_id, position) VALUES (?, 'summary', ?, ?)",
                rusqlite::params![conv_id, summary_id, min_pos],
            )?;
        }

        Ok(summary_id)
    }

    /// Check if compaction is needed.
    fn needs_compaction(&self) -> Result<bool> {
        let conv_id = self.ensure_conversation()?;
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM context_items WHERE conversation_id = ?",
            [conv_id],
            |row| row.get(0),
        )?;
        Ok(count as usize > self.config.threshold_items)
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    fn name(&self) -> &str {
        "sqlite"
    }

    async fn ingest(&self, role: &str, content: &str, token_count: usize) -> Result<i64> {
        let conv_id = self.ensure_conversation()?;
        let conn = self.conn.lock().unwrap();

        // Get next sequence number
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE conversation_id = ?",
            [conv_id],
            |row| row.get(0),
        )?;

        // Insert message
        conn.execute(
            "INSERT INTO messages (conversation_id, role, content, seq, token_count) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![conv_id, role, content, seq, token_count as i32],
        )?;

        let msg_id = conn.last_insert_rowid();

        // Add to context items
        let position: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), 0) + 1 FROM context_items WHERE conversation_id = ?",
            [conv_id],
            |row| row.get(0),
        )?;

        conn.execute(
            "INSERT INTO context_items (conversation_id, item_type, ref_id, position) VALUES (?, 'message', ?, ?)",
            rusqlite::params![conv_id, msg_id, position],
        )?;

        // Update conversation timestamp
        conn.execute(
            "UPDATE conversations SET updated_at = strftime('%s', 'now') WHERE id = ?",
            [conv_id],
        )?;

        Ok(msg_id)
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryHit>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT m.id, m.role, m.content, m.token_count, m.created_at
             FROM messages m
             JOIN messages_fts fts ON m.id = fts.rowid
             WHERE messages_fts MATCH ?
             ORDER BY rank
             LIMIT ?",
        )?;

        let hits: Vec<MemoryHit> = stmt
            .query_map(rusqlite::params![query, limit as i64], |row| {
                let content: String = row.get(2)?;
                Ok(MemoryHit {
                    entry: MemoryEntry {
                        id: row.get(0)?,
                        role: row.get(1)?,
                        content: content.clone(),
                        token_count: row.get::<_, i32>(3)? as usize,
                        timestamp: row.get(4)?,
                        depth: 0,
                    },
                    score: 1.0, // FTS5 doesn't expose raw scores easily
                    snippet: content,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(hits)
    }

    async fn get_context(&self, max_tokens: usize) -> Result<String> {
        let entries = self.get_context_entries(max_tokens).await?;
        Ok(super::generate_context_md(&entries))
    }

    async fn get_context_entries(&self, max_tokens: usize) -> Result<Vec<MemoryEntry>> {
        let conv_id = self.ensure_conversation()?;
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT item_type, ref_id FROM context_items 
             WHERE conversation_id = ? 
             ORDER BY position ASC",
        )?;

        let items: Vec<(String, i64)> = stmt
            .query_map([conv_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut entries = Vec::new();
        let mut total_tokens = 0;

        for (item_type, ref_id) in items {
            let entry = match item_type.as_str() {
                "message" => self.get_message_entry(&conn, ref_id).ok(),
                "summary" => self.get_summary_entry(&conn, ref_id).ok(),
                _ => None,
            };

            if let Some(e) = entry {
                if total_tokens + e.token_count > max_tokens && !entries.is_empty() {
                    break;
                }
                total_tokens += e.token_count;
                entries.push(e);
            }
        }

        Ok(entries)
    }

    async fn compact(&self, summarizer: &dyn Summarizer) -> Result<CompactionStats> {
        let start = Instant::now();
        let mut stats = CompactionStats::default();

        if !self.needs_compaction()? {
            return Ok(stats);
        }

        // Get candidates for leaf compaction
        let candidates = self.get_compaction_candidates(self.config.leaf_chunk_size)?;

        // Only compact messages at depth 0
        let messages: Vec<&MemoryEntry> = candidates.iter().filter(|e| e.depth == 0).collect();

        if messages.len() >= self.config.leaf_chunk_size {
            let chunk: Vec<_> = messages
                .iter()
                .take(self.config.leaf_chunk_size)
                .cloned()
                .cloned()
                .collect();

            let message_ids: Vec<i64> = chunk.iter().map(|e| e.id).collect();
            let tokens_before: usize = chunk.iter().map(|e| e.token_count).sum();

            let summary_text = summarizer.summarize(&chunk, SummaryKind::Leaf).await?;
            self.create_summary_from_messages(&message_ids, &summary_text)?;

            stats.messages_compacted = chunk.len();
            stats.summaries_created = 1;
            stats.tokens_saved = tokens_before.saturating_sub(estimate_tokens(&summary_text));
        }

        stats.duration = start.elapsed();
        Ok(stats)
    }

    async fn message_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    async fn summary_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM summaries", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    async fn flush(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}
