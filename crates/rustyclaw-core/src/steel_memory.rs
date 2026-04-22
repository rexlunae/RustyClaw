//! Steel Memory integration for RustyClaw.
//!
//! Provides semantic vector search, knowledge graph, palace graph traversal,
//! AAAK dialect compression, and agent diary using steel-memory.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

use steel_memory_lib::{
    KnowledgeGraph, PalaceGraph, RoomNode, Triple, VectorStorage, compress_to_aaak,
    fastembed::{EmbeddingModel, InitOptions, TextEmbedding},
    types::{Drawer, SearchResult as SteelSearchResult},
};

/// Main interface to steel-memory for RustyClaw.
///
/// Provides:
/// - Semantic vector search over memories
/// - Knowledge graph (temporal RDF triples)
/// - Palace graph traversal (BFS across wings/rooms)
/// - AAAK dialect compression for context priming
/// - Agent diary for timestamped journal entries
pub struct SteelMemory {
    /// Path to vector storage database
    db_path: PathBuf,
    /// Path to knowledge graph database
    kg_path: PathBuf,
    /// Workspace root path
    palace_path: PathBuf,
    /// Embedding model (lazy loaded)
    embedding: Arc<Mutex<Option<TextEmbedding>>>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub content: String,
    pub path: String,
    pub wing: String,
    pub room: String,
    pub similarity: f32,
    pub id: String,
}

impl From<SteelSearchResult> for SearchResult {
    fn from(r: SteelSearchResult) -> Self {
        Self {
            content: r.drawer.content,
            path: r.drawer.source_file,
            wing: r.drawer.wing,
            room: r.drawer.room,
            similarity: r.similarity,
            id: r.drawer.id,
        }
    }
}

/// A knowledge graph triple for RustyClaw.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgTriple {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub confidence: f64,
    pub extracted_at: String,
}

impl From<Triple> for KgTriple {
    fn from(t: Triple) -> Self {
        Self {
            id: t.id,
            subject: t.subject,
            predicate: t.predicate,
            object: t.object,
            valid_from: t.valid_from,
            valid_to: t.valid_to,
            confidence: t.confidence,
            extracted_at: t.extracted_at,
        }
    }
}

/// A room node in the palace graph.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PalaceRoom {
    pub id: String,
    pub wing: String,
    pub room: String,
    pub count: usize,
}

impl From<RoomNode> for PalaceRoom {
    fn from(n: RoomNode) -> Self {
        Self {
            id: n.id,
            wing: n.wing,
            room: n.room,
            count: n.count,
        }
    }
}

/// Agent diary entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiaryEntry {
    pub id: String,
    pub agent: String,
    pub content: String,
    pub timestamp: String,
}

// Helper functions for spawn_blocking with explicit return types
fn load_embedding_model() -> Result<TextEmbedding, String> {
    TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
        .map_err(|e| format!("Failed to load embedding model: {}", e))
}

fn do_embed(
    embedding: Arc<Mutex<Option<TextEmbedding>>>,
    text: String,
) -> Result<Vec<f32>, String> {
    let mut guard = embedding.blocking_lock();
    let model = guard
        .as_mut()
        .ok_or_else(|| "Embedding model not initialized".to_string())?;
    let mut embeddings = model
        .embed(vec![text.as_str()], None)
        .map_err(|e| format!("Embedding failed: {}", e))?;
    Ok(embeddings.remove(0))
}

fn do_search(
    db_path: PathBuf,
    query_vec: Vec<f32>,
    limit: usize,
) -> Result<Vec<SteelSearchResult>, String> {
    let storage =
        VectorStorage::new(&db_path).map_err(|e| format!("Failed to open storage: {}", e))?;
    storage
        .search(&query_vec, limit, None, None)
        .map_err(|e| format!("Search failed: {}", e))
}

fn do_add_drawer(db_path: PathBuf, drawer: Drawer, vec: Vec<f32>) -> Result<(), String> {
    let storage =
        VectorStorage::new(&db_path).map_err(|e| format!("Failed to open storage: {}", e))?;
    storage
        .add_drawer(&drawer, &vec)
        .map_err(|e| format!("Failed to add drawer: {}", e))
}

fn do_get_all(db_path: PathBuf) -> Result<Vec<Drawer>, String> {
    let storage =
        VectorStorage::new(&db_path).map_err(|e| format!("Failed to open storage: {}", e))?;
    storage
        .get_all(None, None, usize::MAX)
        .map_err(|e| format!("Failed to get all: {}", e))
}

impl SteelMemory {
    /// Create a new SteelMemory instance for the given workspace.
    pub fn new(workspace: &Path) -> Result<Self, String> {
        let steel_dir = workspace.join(".steel-memory");
        std::fs::create_dir_all(&steel_dir)
            .map_err(|e| format!("Failed to create .steel-memory directory: {}", e))?;

        let db_path = steel_dir.join("palace.sqlite3");
        let kg_path = steel_dir.join("knowledge_graph.sqlite3");

        // Initialize vector storage
        VectorStorage::new(&db_path)
            .map_err(|e| format!("Failed to initialize vector storage: {}", e))?;

        // Initialize knowledge graph
        KnowledgeGraph::new(&kg_path)
            .map_err(|e| format!("Failed to initialize knowledge graph: {}", e))?;

        Ok(Self {
            db_path,
            kg_path,
            palace_path: workspace.to_path_buf(),
            embedding: Arc::new(Mutex::new(None)),
        })
    }

    async fn ensure_embedding(&self) -> Result<(), String> {
        let mut guard = self.embedding.lock().await;
        if guard.is_none() {
            info!("Loading embedding model (AllMiniLML6V2)...");
            let join_result = tokio::task::spawn_blocking(load_embedding_model).await;
            let model = match join_result {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(format!("Embedding task panicked: {}", e)),
            };
            *guard = Some(model);
            info!("Embedding model loaded");
        }
        Ok(())
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.ensure_embedding().await?;

        let embedding = self.embedding.clone();
        let text_owned = text.to_string();

        let join_result =
            tokio::task::spawn_blocking(move || do_embed(embedding, text_owned)).await;
        match join_result {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("Embedding task panicked: {}", e)),
        }
    }

    // =========================================================================
    // Vector Search (Semantic Memory)
    // =========================================================================

    /// Search memories semantically by query.
    pub async fn search(
        &self,
        query: &str,
        max_results: usize,
        min_score: Option<f32>,
    ) -> Result<Vec<SearchResult>, String> {
        debug!(query, max_results, "Searching steel-memory");

        let query_vec = self.embed(query).await?;
        let db_path = self.db_path.clone();
        let min_score = min_score.unwrap_or(0.3);
        let limit = max_results * 2;

        let join_result =
            tokio::task::spawn_blocking(move || do_search(db_path, query_vec, limit)).await;
        let results: Vec<SteelSearchResult> = match join_result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Search task panicked: {}", e)),
        };

        Ok(results
            .into_iter()
            .filter(|r| r.similarity >= min_score)
            .take(max_results)
            .map(SearchResult::from)
            .collect())
    }

    /// Add a memory to the palace.
    pub async fn add_memory(
        &self,
        content: &str,
        wing: &str,
        room: &str,
        source_file: Option<&str>,
    ) -> Result<String, String> {
        let vec = self.embed(content).await?;
        let id = uuid::Uuid::new_v4().to_string();

        let drawer = Drawer {
            id: id.clone(),
            content: content.to_string(),
            wing: wing.to_string(),
            room: room.to_string(),
            source_file: source_file.unwrap_or("rustyclaw").to_string(),
            source_mtime: 0,
            chunk_index: 0,
            added_by: "rustyclaw".to_string(),
            filed_at: chrono::Utc::now().to_rfc3339(),
            hall: String::new(),
            topic: String::new(),
            drawer_type: String::new(),
            agent: "rustyclaw".to_string(),
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            importance: 3.0,
        };

        let db_path = self.db_path.clone();
        let join_result =
            tokio::task::spawn_blocking(move || do_add_drawer(db_path, drawer, vec)).await;
        match join_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Add task panicked: {}", e)),
        }

        debug!(id = %id, wing, room, "Added memory to steel-memory");
        Ok(id)
    }

    /// Count total memories in the palace.
    pub async fn count(&self) -> Result<usize, String> {
        let db_path = self.db_path.clone();
        let join_result = tokio::task::spawn_blocking(move || do_get_all(db_path)).await;
        let drawers: Vec<Drawer> = match join_result {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Count task panicked: {}", e)),
        };
        Ok(drawers.len())
    }

    /// Index workspace memory files (MEMORY.md, memory/*.md).
    pub async fn index_workspace(&self) -> Result<usize, String> {
        info!(workspace = %self.palace_path.display(), "Indexing workspace memories");

        let mut count = 0;

        let memory_md = self.palace_path.join("MEMORY.md");
        if memory_md.exists() {
            count += self
                .index_file(&memory_md, "MEMORY.md", "memory", "long-term")
                .await?;
        }

        let memory_dir = self.palace_path.join("memory");
        if memory_dir.exists() && memory_dir.is_dir() {
            for entry in std::fs::read_dir(&memory_dir)
                .map_err(|e| format!("Failed to read memory dir: {}", e))?
            {
                let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md") {
                    let name = path.file_name().unwrap().to_string_lossy();
                    let relative = format!("memory/{}", name);

                    // Date files (YYYY-MM-DD.md) get their own room
                    let room = if name.len() == 13
                        && name
                            .chars()
                            .take(10)
                            .all(|c| c.is_ascii_digit() || c == '-')
                    {
                        name.trim_end_matches(".md").to_string()
                    } else {
                        "notes".to_string()
                    };

                    count += self.index_file(&path, &relative, "memory", &room).await?;
                }
            }
        }

        info!(count, "Indexed memory files");
        Ok(count)
    }

    async fn index_file(
        &self,
        path: &Path,
        relative_path: &str,
        wing: &str,
        room: &str,
    ) -> Result<usize, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", relative_path, e))?;

        let chunks = chunk_markdown(&content);
        let mut count = 0;

        for chunk in chunks {
            if chunk.trim().is_empty() {
                continue;
            }
            self.add_memory(&chunk, wing, room, Some(relative_path))
                .await?;
            count += 1;
        }

        debug!(path = %relative_path, chunks = count, "Indexed file");
        Ok(count)
    }

    // =========================================================================
    // Knowledge Graph (Temporal Triples)
    // =========================================================================

    /// Add a knowledge graph triple (subject-predicate-object).
    pub async fn kg_add(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: Option<f64>,
    ) -> Result<String, String> {
        let kg_path = self.kg_path.clone();
        let subject = subject.to_string();
        let predicate = predicate.to_string();
        let object = object.to_string();
        let confidence = confidence.unwrap_or(1.0);

        let join_result = tokio::task::spawn_blocking(move || {
            let kg =
                KnowledgeGraph::new(&kg_path).map_err(|e| format!("Failed to open KG: {}", e))?;
            let id = kg
                .add_triple(&subject, &predicate, &object, confidence, None, None)
                .map_err(|e| format!("Failed to add triple: {}", e))?;
            debug!(subject, predicate, object, "Added KG triple");
            Ok(id)
        })
        .await;

        match join_result {
            Ok(Ok(id)) => Ok(id),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("KG task panicked: {}", e)),
        }
    }

    /// Invalidate (soft-delete) a knowledge graph triple.
    pub async fn kg_invalidate(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<usize, String> {
        let kg_path = self.kg_path.clone();
        let subject = subject.to_string();
        let predicate = predicate.to_string();
        let object = object.to_string();

        let join_result = tokio::task::spawn_blocking(move || {
            let kg =
                KnowledgeGraph::new(&kg_path).map_err(|e| format!("Failed to open KG: {}", e))?;
            kg.invalidate_triple(&subject, &predicate, &object)
                .map_err(|e| format!("Failed to invalidate triple: {}", e))
        })
        .await;

        match join_result {
            Ok(Ok(n)) => Ok(n),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("KG task panicked: {}", e)),
        }
    }

    /// Query knowledge graph by entity.
    /// Direction: "outgoing", "incoming", or "both" (default).
    pub async fn kg_query(
        &self,
        entity: &str,
        direction: Option<&str>,
    ) -> Result<Vec<KgTriple>, String> {
        let kg_path = self.kg_path.clone();
        let entity = entity.to_string();
        let direction = direction.unwrap_or("both").to_string();

        let join_result = tokio::task::spawn_blocking(move || {
            let kg =
                KnowledgeGraph::new(&kg_path).map_err(|e| format!("Failed to open KG: {}", e))?;
            kg.query_entity(&entity, &direction)
                .map_err(|e| format!("Failed to query KG: {}", e))
        })
        .await;

        match join_result {
            Ok(Ok(triples)) => Ok(triples.into_iter().map(KgTriple::from).collect()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("KG task panicked: {}", e)),
        }
    }

    /// Get timeline of triples for an entity.
    pub async fn kg_timeline(
        &self,
        entity: &str,
        limit: Option<usize>,
    ) -> Result<Vec<KgTriple>, String> {
        let kg_path = self.kg_path.clone();
        let entity = entity.to_string();
        let limit = limit.unwrap_or(50);

        let join_result = tokio::task::spawn_blocking(move || {
            let kg =
                KnowledgeGraph::new(&kg_path).map_err(|e| format!("Failed to open KG: {}", e))?;
            kg.timeline(&entity, limit)
                .map_err(|e| format!("Failed to get timeline: {}", e))
        })
        .await;

        match join_result {
            Ok(Ok(triples)) => Ok(triples.into_iter().map(KgTriple::from).collect()),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("KG task panicked: {}", e)),
        }
    }

    /// Get knowledge graph statistics.
    pub async fn kg_stats(&self) -> Result<serde_json::Value, String> {
        let kg_path = self.kg_path.clone();

        let join_result = tokio::task::spawn_blocking(move || {
            let kg =
                KnowledgeGraph::new(&kg_path).map_err(|e| format!("Failed to open KG: {}", e))?;
            kg.stats()
                .map_err(|e| format!("Failed to get KG stats: {}", e))
        })
        .await;

        match join_result {
            Ok(Ok(stats)) => Ok(stats),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("KG task panicked: {}", e)),
        }
    }

    // =========================================================================
    // Palace Graph (Spatial Traversal)
    // =========================================================================

    /// Build the palace graph (all rooms with drawer counts).
    pub async fn palace_graph(&self) -> Result<Vec<PalaceRoom>, String> {
        let pg = PalaceGraph {
            db_path: self.db_path.clone(),
        };
        let nodes = pg
            .build_graph()
            .await
            .map_err(|e| format!("Failed to build palace graph: {}", e))?;
        Ok(nodes.into_iter().map(PalaceRoom::from).collect())
    }

    /// Traverse the palace graph from a starting room using BFS.
    pub async fn palace_traverse(
        &self,
        start_room: &str,
        max_hops: Option<usize>,
    ) -> Result<Vec<PalaceRoom>, String> {
        let pg = PalaceGraph {
            db_path: self.db_path.clone(),
        };
        let nodes = pg
            .traverse_graph(start_room, max_hops.unwrap_or(2))
            .await
            .map_err(|e| format!("Failed to traverse palace: {}", e))?;
        Ok(nodes.into_iter().map(PalaceRoom::from).collect())
    }

    /// Find tunnel rooms (rooms that exist in multiple wings).
    pub async fn palace_tunnels(
        &self,
        wing_a: Option<&str>,
        wing_b: Option<&str>,
    ) -> Result<Vec<PalaceRoom>, String> {
        let pg = PalaceGraph {
            db_path: self.db_path.clone(),
        };
        let nodes = pg
            .find_tunnels(wing_a, wing_b)
            .await
            .map_err(|e| format!("Failed to find tunnels: {}", e))?;
        Ok(nodes.into_iter().map(PalaceRoom::from).collect())
    }

    /// Get palace graph statistics.
    pub async fn palace_stats(&self) -> Result<serde_json::Value, String> {
        let pg = PalaceGraph {
            db_path: self.db_path.clone(),
        };
        pg.stats()
            .await
            .map_err(|e| format!("Failed to get palace stats: {}", e))
    }

    // =========================================================================
    // AAAK Dialect (Compressed Context)
    // =========================================================================

    /// Compress a memory to AAAK dialect for efficient context priming.
    pub fn compress_aaak(&self, content: &str, wing: &str, room: &str) -> String {
        let drawer = Drawer {
            id: String::new(),
            content: content.to_string(),
            wing: wing.to_string(),
            room: room.to_string(),
            source_file: String::new(),
            source_mtime: 0,
            chunk_index: 0,
            added_by: "rustyclaw".to_string(),
            filed_at: chrono::Utc::now().to_rfc3339(),
            hall: String::new(),
            topic: String::new(),
            drawer_type: String::new(),
            agent: "rustyclaw".to_string(),
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            importance: 3.0,
        };
        compress_to_aaak(&drawer)
    }

    /// Generate AAAK-compressed context for a wing (or all wings).
    pub async fn wake_up(&self, wing: Option<&str>) -> Result<String, String> {
        let db_path = self.db_path.clone();
        let wing_filter = wing.map(|s| s.to_string());

        let join_result = tokio::task::spawn_blocking(move || {
            let storage = VectorStorage::new(&db_path)
                .map_err(|e| format!("Failed to open storage: {}", e))?;
            storage
                .get_all(wing_filter.as_deref(), None, 100)
                .map_err(|e| format!("Failed to get drawers: {}", e))
        })
        .await;

        let drawers: Vec<Drawer> = match join_result {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Wake up task panicked: {}", e)),
        };

        let aaak_lines: Vec<String> = drawers.iter().map(|d| compress_to_aaak(d)).collect();

        Ok(aaak_lines.join("\n---\n"))
    }

    // =========================================================================
    // Agent Diary
    // =========================================================================

    /// Write a diary entry.
    pub async fn diary_write(&self, agent: &str, content: &str) -> Result<String, String> {
        // Store diary entries as memories in the "diary" wing
        let room = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let prefixed = format!("[{}] {}", agent, content);
        self.add_memory(&prefixed, "diary", &room, None).await
    }

    /// Read diary entries for an agent.
    pub async fn diary_read(
        &self,
        agent: &str,
        limit: Option<usize>,
    ) -> Result<Vec<DiaryEntry>, String> {
        let db_path = self.db_path.clone();
        let agent_prefix = format!("[{}]", agent);
        let limit = limit.unwrap_or(50);

        let join_result = tokio::task::spawn_blocking(move || {
            let storage = VectorStorage::new(&db_path)
                .map_err(|e| format!("Failed to open storage: {}", e))?;
            storage
                .get_all(Some("diary"), None, limit * 2)
                .map_err(|e| format!("Failed to read diary: {}", e))
        })
        .await;

        let drawers: Vec<Drawer> = match join_result {
            Ok(Ok(d)) => d,
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Diary read task panicked: {}", e)),
        };

        let entries: Vec<DiaryEntry> = drawers
            .into_iter()
            .filter(|d| d.content.starts_with(&agent_prefix))
            .take(limit)
            .map(|d| DiaryEntry {
                id: d.id,
                agent: agent.to_string(),
                content: d
                    .content
                    .trim_start_matches(&agent_prefix)
                    .trim()
                    .to_string(),
                timestamp: d.filed_at,
            })
            .collect();

        Ok(entries)
    }
}

// Legacy type alias for backwards compatibility
pub type SteelMemoryIndex = SteelMemory;

fn chunk_markdown(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut line_count = 0;

    for line in content.lines() {
        let is_heading = line.starts_with("## ") || line.starts_with("# ");

        if (is_heading || line_count >= 20) && !current_chunk.trim().is_empty() {
            chunks.push(current_chunk.trim().to_string());
            current_chunk = String::new();
            line_count = 0;
        }

        current_chunk.push_str(line);
        current_chunk.push('\n');
        line_count += 1;
    }

    if !current_chunk.trim().is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_basic_search() {
        let dir = TempDir::new().unwrap();
        let mem = SteelMemory::new(dir.path()).unwrap();

        mem.add_memory(
            "I love programming in Rust",
            "preferences",
            "languages",
            None,
        )
        .await
        .unwrap();
        mem.add_memory(
            "Python is great for data science",
            "preferences",
            "languages",
            None,
        )
        .await
        .unwrap();
        mem.add_memory("The sky is blue today", "observations", "weather", None)
            .await
            .unwrap();

        let results = mem.search("Rust programming", 5, None).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_index_workspace() {
        let dir = TempDir::new().unwrap();

        fs::write(dir.path().join("MEMORY.md"), "# Memory\n\nI like cats.").unwrap();
        fs::create_dir(dir.path().join("memory")).unwrap();
        fs::write(
            dir.path().join("memory/2026-04-13.md"),
            "# Today\n\nWent for a walk.",
        )
        .unwrap();

        let mem = SteelMemory::new(dir.path()).unwrap();
        let count = mem.index_workspace().await.unwrap();

        assert!(count >= 2);

        let results = mem.search("cats", 5, None).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_knowledge_graph() {
        let dir = TempDir::new().unwrap();
        let mem = SteelMemory::new(dir.path()).unwrap();

        // Add some triples
        mem.kg_add("Erica", "knows", "Rust", Some(0.9))
            .await
            .unwrap();
        mem.kg_add("Erica", "lives_in", "Colorado", None)
            .await
            .unwrap();
        mem.kg_add("Rust", "is_a", "programming_language", None)
            .await
            .unwrap();

        // Query outgoing
        let triples = mem.kg_query("Erica", Some("outgoing")).await.unwrap();
        assert_eq!(triples.len(), 2);

        // Query incoming
        let triples = mem.kg_query("Rust", Some("incoming")).await.unwrap();
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].subject, "erica");

        // Invalidate
        let count = mem
            .kg_invalidate("Erica", "lives_in", "Colorado")
            .await
            .unwrap();
        assert_eq!(count, 1);

        // Should now only have 1 valid triple for Erica
        let triples = mem.kg_query("Erica", Some("outgoing")).await.unwrap();
        assert_eq!(triples.len(), 1);

        // Stats
        let stats = mem.kg_stats().await.unwrap();
        assert!(stats["entities"].as_i64().unwrap() >= 3);
    }

    #[tokio::test]
    async fn test_palace_graph() {
        let dir = TempDir::new().unwrap();
        let mem = SteelMemory::new(dir.path()).unwrap();

        // Add memories to different wings/rooms
        mem.add_memory("Test 1", "wing_a", "room_1", None)
            .await
            .unwrap();
        mem.add_memory("Test 2", "wing_a", "room_2", None)
            .await
            .unwrap();
        mem.add_memory("Test 3", "wing_b", "room_1", None)
            .await
            .unwrap(); // Tunnel: room_1 in both wings

        // Build graph
        let rooms = mem.palace_graph().await.unwrap();
        assert_eq!(rooms.len(), 3);

        // Find tunnels
        let tunnels = mem.palace_tunnels(None, None).await.unwrap();
        assert!(!tunnels.is_empty());
        assert!(tunnels.iter().any(|t| t.room == "room_1"));

        // Stats
        let stats = mem.palace_stats().await.unwrap();
        assert_eq!(stats["wings"].as_i64().unwrap(), 2);
    }

    #[tokio::test]
    async fn test_aaak_compression() {
        let dir = TempDir::new().unwrap();
        let mem = SteelMemory::new(dir.path()).unwrap();

        let aaak = mem.compress_aaak(
            "I decided to switch from Python to Rust for performance",
            "decisions",
            "languages",
        );

        // AAAK format should contain wing|room|date|file on first line
        assert!(aaak.contains("decisions|languages|"));
        // Should detect emotions/flags
        assert!(aaak.contains("resolve") || aaak.contains("DEC"));
    }

    #[tokio::test]
    async fn test_diary() {
        let dir = TempDir::new().unwrap();
        let mem = SteelMemory::new(dir.path()).unwrap();

        // Write entries
        mem.diary_write("luthen", "Started working on RustyClaw")
            .await
            .unwrap();
        mem.diary_write("luthen", "Made good progress today")
            .await
            .unwrap();
        mem.diary_write("erskin", "Fixed a bug").await.unwrap();

        // Read Luthen's diary
        let entries = mem.diary_read("luthen", None).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].content.contains("progress") || entries[1].content.contains("progress"));

        // Read Erskin's diary
        let entries = mem.diary_read("erskin", None).await.unwrap();
        assert_eq!(entries.len(), 1);
    }
}
