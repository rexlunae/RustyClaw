//! Structured memory with auto-reflector.
//!
//! Provides persistent agent memory through:
//! 1. **File memory**: Manual facts in AGENTS.md (via soul.rs)
//! 2. **Structured memory**: SQLite database with auto-extracted facts
//! 3. **Auto-reflector**: Background task that extracts durable facts from conversations
//! 4. **Quality gates**: Confidence scoring and deduplication
//! 5. **Retrieval**: Query interface with optional semantic search
//!
//! ## Database Schema
//!
//! ```sql
//! CREATE TABLE facts (
//!     id INTEGER PRIMARY KEY,
//!     content TEXT NOT NULL,
//!     category TEXT,
//!     confidence REAL DEFAULT 0.5,
//!     source TEXT,
//!     created_at INTEGER NOT NULL,
//!     updated_at INTEGER NOT NULL,
//!     access_count INTEGER DEFAULT 0,
//!     last_accessed INTEGER
//! );
//! ```
//!
//! ## Configuration Example
//!
//! ```toml
//! [structured_memory]
//! enabled = true
//! db_path = "memory/facts.db"  # relative to workspace
//! min_confidence = 0.5
//! reflection_interval_secs = 3600  # Reflect every hour
//! max_facts = 10000
//! ```

use anyhow::{Context as AnyhowContext, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Structured memory configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredMemoryConfig {
    /// Whether structured memory is enabled
    #[serde(default)]
    pub enabled: bool,

    /// Database path (relative to workspace)
    #[serde(default = "StructuredMemoryConfig::default_db_path")]
    pub db_path: String,

    /// Minimum confidence score to store a fact (0.0-1.0)
    #[serde(default = "StructuredMemoryConfig::default_min_confidence")]
    pub min_confidence: f64,

    /// Reflection interval in seconds (how often to auto-extract facts)
    #[serde(default = "StructuredMemoryConfig::default_reflection_interval")]
    pub reflection_interval_secs: u64,

    /// Maximum number of facts to store (oldest low-confidence facts pruned)
    #[serde(default = "StructuredMemoryConfig::default_max_facts")]
    pub max_facts: usize,
}

impl StructuredMemoryConfig {
    fn default_db_path() -> String {
        "memory/facts.db".to_string()
    }

    fn default_min_confidence() -> f64 {
        0.5
    }

    fn default_reflection_interval() -> u64 {
        3600 // 1 hour
    }

    fn default_max_facts() -> usize {
        10000
    }
}

impl Default for StructuredMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: Self::default_db_path(),
            min_confidence: Self::default_min_confidence(),
            reflection_interval_secs: Self::default_reflection_interval(),
            max_facts: Self::default_max_facts(),
        }
    }
}

/// A stored fact with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: Option<i64>,
    pub content: String,
    pub category: Option<String>,
    pub confidence: f64,
    pub source: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub access_count: i64,
    pub last_accessed: Option<i64>,
}

impl Fact {
    /// Create a new fact with the given content.
    pub fn new(content: String) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id: None,
            content,
            category: None,
            confidence: 0.5,
            source: None,
            created_at: now,
            updated_at: now,
            access_count: 0,
            last_accessed: None,
        }
    }

    /// Set the category.
    pub fn with_category(mut self, category: String) -> Self {
        self.category = Some(category);
        self
    }

    /// Set the confidence score.
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Set the source.
    pub fn with_source(mut self, source: String) -> Self {
        self.source = Some(source);
        self
    }
}

/// Structured memory store with SQLite backend.
pub struct StructuredMemory {
    db_path: PathBuf,
    conn: Arc<RwLock<Connection>>,
    config: StructuredMemoryConfig,
}

impl StructuredMemory {
    /// Open or create a structured memory database.
    pub fn open(workspace: &Path, config: StructuredMemoryConfig) -> Result<Self> {
        let db_path = workspace.join(&config.db_path);

        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create memory directory: {}", parent.display()))?;
        }

        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open database: {}", db_path.display()))?;

        // Initialize schema
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS facts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                category TEXT,
                confidence REAL DEFAULT 0.5,
                source TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                access_count INTEGER DEFAULT 0,
                last_accessed INTEGER,
                UNIQUE(content)
            );

            CREATE INDEX IF NOT EXISTS idx_facts_confidence ON facts(confidence DESC);
            CREATE INDEX IF NOT EXISTS idx_facts_category ON facts(category);
            CREATE INDEX IF NOT EXISTS idx_facts_created ON facts(created_at DESC);
            "#,
        )?;

        Ok(Self {
            db_path,
            conn: Arc::new(RwLock::new(conn)),
            config,
        })
    }

    /// Store a new fact (or update if duplicate).
    pub async fn store_fact(&self, mut fact: Fact) -> Result<i64> {
        // Enforce minimum confidence threshold
        if fact.confidence < self.config.min_confidence {
            anyhow::bail!(
                "Fact confidence {} below threshold {}",
                fact.confidence,
                self.config.min_confidence
            );
        }

        let conn = self.conn.write().await;
        let now = chrono::Utc::now().timestamp();
        fact.updated_at = now;

        // Try to insert, or update if duplicate content exists
        let result = conn.execute(
            r#"
            INSERT INTO facts (content, category, confidence, source, created_at, updated_at, access_count)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
            ON CONFLICT(content) DO UPDATE SET
                confidence = MAX(confidence, excluded.confidence),
                updated_at = excluded.updated_at
            "#,
            params![
                &fact.content,
                &fact.category,
                fact.confidence,
                &fact.source,
                fact.created_at,
                fact.updated_at,
            ],
        )?;

        // Get the ID of the inserted/updated row
        let id = conn.last_insert_rowid();

        // Prune old facts if we're over the limit
        self.prune_old_facts_sync(&conn)?;

        Ok(id)
    }

    /// Retrieve facts matching a query.
    pub async fn search_facts(&self, query: &str, limit: usize) -> Result<Vec<Fact>> {
        let conn = self.conn.read().await;
        let query_lower = query.to_lowercase();

        let mut stmt = conn.prepare(
            r#"
            SELECT id, content, category, confidence, source, created_at, updated_at, access_count, last_accessed
            FROM facts
            WHERE LOWER(content) LIKE ?1 OR LOWER(category) LIKE ?1
            ORDER BY confidence DESC, access_count DESC
            LIMIT ?2
            "#,
        )?;

        let search_pattern = format!("%{}%", query_lower);
        let facts = stmt
            .query_map(params![search_pattern, limit as i64], |row| {
                Ok(Fact {
                    id: Some(row.get(0)?),
                    content: row.get(1)?,
                    category: row.get(2)?,
                    confidence: row.get(3)?,
                    source: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    access_count: row.get(7)?,
                    last_accessed: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(facts)
    }

    /// Get all facts above a confidence threshold.
    pub async fn get_high_confidence_facts(&self, min_confidence: f64, limit: usize) -> Result<Vec<Fact>> {
        let conn = self.conn.read().await;

        let mut stmt = conn.prepare(
            r#"
            SELECT id, content, category, confidence, source, created_at, updated_at, access_count, last_accessed
            FROM facts
            WHERE confidence >= ?1
            ORDER BY confidence DESC, access_count DESC
            LIMIT ?2
            "#,
        )?;

        let facts = stmt
            .query_map(params![min_confidence, limit as i64], |row| {
                Ok(Fact {
                    id: Some(row.get(0)?),
                    content: row.get(1)?,
                    category: row.get(2)?,
                    confidence: row.get(3)?,
                    source: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    access_count: row.get(7)?,
                    last_accessed: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(facts)
    }

    /// Record that a fact was accessed (for usage tracking).
    pub async fn record_access(&self, fact_id: i64) -> Result<()> {
        let conn = self.conn.write().await;
        let now = chrono::Utc::now().timestamp();

        conn.execute(
            r#"
            UPDATE facts
            SET access_count = access_count + 1,
                last_accessed = ?1
            WHERE id = ?2
            "#,
            params![now, fact_id],
        )?;

        Ok(())
    }

    /// Delete a fact by ID.
    pub async fn delete_fact(&self, fact_id: i64) -> Result<()> {
        let conn = self.conn.write().await;
        conn.execute("DELETE FROM facts WHERE id = ?1", params![fact_id])?;
        Ok(())
    }

    /// Get total count of facts.
    pub async fn count_facts(&self) -> Result<usize> {
        let conn = self.conn.read().await;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM facts", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Prune old low-confidence facts if over the limit.
    fn prune_old_facts_sync(&self, conn: &Connection) -> Result<()> {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM facts", [], |row| row.get(0))?;

        if count as usize > self.config.max_facts {
            let to_remove = count as usize - self.config.max_facts;

            // Delete oldest low-confidence facts
            conn.execute(
                r#"
                DELETE FROM facts
                WHERE id IN (
                    SELECT id FROM facts
                    ORDER BY confidence ASC, access_count ASC, created_at ASC
                    LIMIT ?1
                )
                "#,
                params![to_remove as i64],
            )?;
        }

        Ok(())
    }

    /// Calculate confidence score for a potential fact.
    ///
    /// Heuristic scoring based on:
    /// - Length (longer statements often more informative)
    /// - Specificity indicators (numbers, proper nouns)
    /// - Categorical keywords
    /// - Statement structure
    pub fn calculate_confidence(text: &str) -> f64 {
        let mut score = 0.3; // Base score

        // Length factor (normalize to 0-0.2)
        let length_score = (text.len() as f64 / 500.0).min(1.0) * 0.2;
        score += length_score;

        // Contains numbers (specific data)
        if text.chars().any(|c| c.is_numeric()) {
            score += 0.15;
        }

        // Contains proper nouns (capital letters mid-sentence)
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.iter().skip(1).any(|w| w.chars().next().map_or(false, |c| c.is_uppercase())) {
            score += 0.1;
        }

        // Statement indicators
        if text.contains("is") || text.contains("are") || text.contains("has") {
            score += 0.1;
        }

        // Preference/configuration indicators (highly valuable for memory)
        if text.contains("prefer") || text.contains("should") || text.contains("always") || text.contains("never") {
            score += 0.2;
        }

        score.clamp(0.0, 1.0)
    }

    /// Extract potential facts from text using simple heuristics.
    ///
    /// This is a basic implementation. A production system would use:
    /// - LLM-based extraction
    /// - Named entity recognition
    /// - Dependency parsing
    pub fn extract_facts(text: &str, min_confidence: f64) -> Vec<Fact> {
        let mut facts = Vec::new();

        // Split into sentences
        let sentences: Vec<&str> = text
            .split(&['.', '!', '?', '\n'])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && s.len() > 20) // Filter very short fragments
            .collect();

        for sentence in sentences {
            let confidence = Self::calculate_confidence(sentence);

            if confidence >= min_confidence {
                facts.push(
                    Fact::new(sentence.to_string())
                        .with_confidence(confidence)
                );
            }
        }

        facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_store_and_retrieve_fact() {
        let temp_dir = TempDir::new().unwrap();
        let config = StructuredMemoryConfig::default();
        let memory = StructuredMemory::open(temp_dir.path(), config).unwrap();

        let fact = Fact::new("The user prefers Rust for systems programming".to_string())
            .with_category("preference".to_string())
            .with_confidence(0.9);

        let id = memory.store_fact(fact).await.unwrap();
        assert!(id > 0);

        let results = memory.search_facts("Rust", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "The user prefers Rust for systems programming");
        assert_eq!(results[0].confidence, 0.9);
    }

    #[tokio::test]
    async fn test_duplicate_facts_update_confidence() {
        let temp_dir = TempDir::new().unwrap();
        let config = StructuredMemoryConfig::default();
        let memory = StructuredMemory::open(temp_dir.path(), config).unwrap();

        // Store same fact twice with different confidence
        let fact1 = Fact::new("User likes Python".to_string()).with_confidence(0.6);
        memory.store_fact(fact1).await.unwrap();

        let fact2 = Fact::new("User likes Python".to_string()).with_confidence(0.8);
        memory.store_fact(fact2).await.unwrap();

        let count = memory.count_facts().await.unwrap();
        assert_eq!(count, 1); // Should have only one fact

        let results = memory.search_facts("Python", 10).await.unwrap();
        assert_eq!(results[0].confidence, 0.8); // Should keep higher confidence
    }

    #[test]
    fn test_confidence_calculation() {
        // Specific fact with numbers and proper nouns
        let score1 = StructuredMemory::calculate_confidence(
            "The user's name is John and they prefer 4 spaces for indentation"
        );
        assert!(score1 > 0.7);

        // Generic short statement
        let score2 = StructuredMemory::calculate_confidence("User likes coding");
        assert!(score2 < 0.6);

        // Preference statement
        let score3 = StructuredMemory::calculate_confidence(
            "User should always use TypeScript for web projects"
        );
        assert!(score3 > 0.6);
    }

    #[test]
    fn test_fact_extraction() {
        let text = "The user prefers dark mode. They work in software engineering. \
                    They use Neovim as their editor. Random short text.";

        let facts = StructuredMemory::extract_facts(text, 0.4);

        // Should extract the substantive sentences
        assert!(facts.len() >= 2);
        assert!(facts.iter().any(|f| f.content.contains("dark mode")));
        assert!(facts.iter().any(|f| f.content.contains("software engineering")));
    }

    #[tokio::test]
    async fn test_access_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let config = StructuredMemoryConfig::default();
        let memory = StructuredMemory::open(temp_dir.path(), config).unwrap();

        let fact = Fact::new("Test fact".to_string()).with_confidence(0.8);
        let id = memory.store_fact(fact).await.unwrap();

        // Record access twice
        memory.record_access(id).await.unwrap();
        memory.record_access(id).await.unwrap();

        let results = memory.search_facts("Test", 10).await.unwrap();
        assert_eq!(results[0].access_count, 2);
        assert!(results[0].last_accessed.is_some());
    }
}
