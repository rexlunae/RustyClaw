//! Search and drill-down over chunks and summaries.

use crate::error::Result;
use crate::store::Store;
use rusqlite::params;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// All sources.
    Global,
    /// Restrict to a single source identifier (e.g. `"gmail/inbox"`).
    Source(String),
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub kind: HitKind,
    pub id: String,
    pub source: String,
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitKind {
    Chunk,
    Summary { level: u8 },
}

pub struct Retrieval {
    store: Arc<Store>,
}

impl Retrieval {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// FTS search across both chunks and summaries. Returns hits sorted by
    /// best (lowest) FTS rank; summaries are preferred when their rank ties.
    pub fn search(&self, query: &str, scope: &Scope, limit: usize) -> Result<Vec<SearchHit>> {
        let mut hits = Vec::new();
        let fts_query = escape_fts(query);

        self.store.with_conn(|c| {
            // Chunks.
            let (sql, source_filter) = match scope {
                Scope::Global => (
                    "SELECT cf.id, cf.source, snippet(chunk_fts, -1, '[', ']', '…', 12), rank
                     FROM chunk_fts cf
                     WHERE chunk_fts MATCH ?1
                     ORDER BY rank LIMIT ?2"
                        .to_string(),
                    None,
                ),
                Scope::Source(s) => (
                    "SELECT cf.id, cf.source, snippet(chunk_fts, -1, '[', ']', '…', 12), rank
                     FROM chunk_fts cf
                     WHERE chunk_fts MATCH ?1 AND cf.source = ?3
                     ORDER BY rank LIMIT ?2"
                        .to_string(),
                    Some(s.clone()),
                ),
            };
            let mut stmt = c.prepare(&sql)?;
            let make_hit_chunk = |id: String, source: String, snippet: String, rank: f64| {
                SearchHit {
                    kind: HitKind::Chunk,
                    id,
                    source,
                    snippet,
                    score: -rank, // lower rank = better; invert so higher score = better
                }
            };
            let rows = if let Some(s) = source_filter {
                stmt.query_map(params![&fts_query, limit as i64, s], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<std::result::Result<Vec<(String, String, String, f64)>, _>>()?
            } else {
                stmt.query_map(params![&fts_query, limit as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<std::result::Result<Vec<(String, String, String, f64)>, _>>()?
            };
            for (id, source, snippet, rank) in rows {
                hits.push(make_hit_chunk(id, source, snippet, rank));
            }

            // Summaries.
            let (sql2, src2) = match scope {
                Scope::Global => (
                    "SELECT sf.id, sf.source, sf.level, snippet(summary_fts, -1, '[', ']', '…', 12), rank
                     FROM summary_fts sf
                     WHERE summary_fts MATCH ?1
                     ORDER BY rank LIMIT ?2"
                        .to_string(),
                    None,
                ),
                Scope::Source(s) => (
                    "SELECT sf.id, sf.source, sf.level, snippet(summary_fts, -1, '[', ']', '…', 12), rank
                     FROM summary_fts sf
                     WHERE summary_fts MATCH ?1 AND sf.source = ?3
                     ORDER BY rank LIMIT ?2"
                        .to_string(),
                    Some(s.clone()),
                ),
            };
            let mut stmt2 = c.prepare(&sql2)?;
            let rows2 = if let Some(s) = src2 {
                stmt2
                    .query_map(params![&fts_query, limit as i64, s], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get(1)?,
                            r.get::<_, i64>(2)?,
                            r.get(3)?,
                            r.get(4)?,
                        ))
                    })?
                    .collect::<std::result::Result<
                        Vec<(i64, String, i64, String, f64)>,
                        _,
                    >>()?
            } else {
                stmt2
                    .query_map(params![&fts_query, limit as i64], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get(1)?,
                            r.get::<_, i64>(2)?,
                            r.get(3)?,
                            r.get(4)?,
                        ))
                    })?
                    .collect::<std::result::Result<
                        Vec<(i64, String, i64, String, f64)>,
                        _,
                    >>()?
            };
            for (id, source, level, snippet, rank) in rows2 {
                hits.push(SearchHit {
                    kind: HitKind::Summary {
                        level: level as u8,
                    },
                    id: id.to_string(),
                    source,
                    snippet,
                    score: -rank + 0.001, // tiny preference for summaries on tie
                });
            }
            Ok(())
        })?;

        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        Ok(hits)
    }
}

/// Escape a user-supplied query for FTS5. We don't expose FTS operators
/// because they have surprising behavior (`AND`, `NOT`, `*`). Anything
/// non-alphanumeric becomes a literal phrase boundary.
fn escape_fts(q: &str) -> String {
    // Wrap each token in quotes and join.
    let tokens: Vec<String> = q
        .split_whitespace()
        .map(|t| t.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t))
        .collect();
    if tokens.is_empty() {
        // Match nothing rather than crash.
        return "\"\"".to_string();
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::chunk;

    #[test]
    fn search_finds_chunks() {
        let store = Arc::new(Store::in_memory().unwrap());
        let chunks = chunk("doc-1", "The quick brown fox jumps over the lazy dog.");
        store.insert_chunks("notes/test", &chunks, &[0.5]).unwrap();

        let r = Retrieval::new(Arc::clone(&store));
        let hits = r.search("brown fox", &Scope::Global, 10).unwrap();
        assert!(!hits.is_empty());
        assert!(matches!(hits[0].kind, HitKind::Chunk));
        assert!(hits[0].snippet.contains("brown") || hits[0].snippet.contains("fox"));
    }

    #[test]
    fn source_scope_filters() {
        let store = Arc::new(Store::in_memory().unwrap());
        store
            .insert_chunks(
                "gmail",
                &chunk("m1", "alpha beta gamma"),
                &[0.5],
            )
            .unwrap();
        store
            .insert_chunks(
                "slack",
                &chunk("m2", "alpha beta gamma"),
                &[0.5],
            )
            .unwrap();
        let r = Retrieval::new(Arc::clone(&store));
        let hits = r
            .search("alpha", &Scope::Source("gmail".into()), 10)
            .unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|h| h.source == "gmail"));
    }
}
