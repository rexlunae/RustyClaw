//! Steel Memory integration for RustyClaw.
//!
//! Provides semantic vector search over agent memories using steel-memory's
//! embedding and SQLite-backed storage.
//!
//! This replaces the BM25-based keyword search with true semantic search,
//! delivering much better recall for natural language queries.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

use steel_memory::{
    fastembed::{EmbeddingModel, InitOptions, TextEmbedding},
    storage::vector::VectorStorage,
    types::{Drawer, SearchResult as SteelSearchResult},
};

/// A semantic memory index using steel-memory's vector storage.
pub struct SteelMemoryIndex {
    /// Path to the vector database
    db_path: PathBuf,
    /// Path to the palace/workspace directory
    palace_path: PathBuf,
    /// Embedding model (lazy initialized)
    embedding: Arc<Mutex<Option<TextEmbedding>>>,
}

/// A search result from steel-memory.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching memory content
    pub content: String,
    /// Source file path
    pub path: String,
    /// Wing (category) of the memory
    pub wing: String,
    /// Room (subcategory) of the memory
    pub room: String,
    /// Similarity score (0.0 to 1.0)
    pub similarity: f32,
    /// Drawer ID for reference
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

impl SteelMemoryIndex {
    /// Create a new steel-memory index for a workspace.
    ///
    /// The database will be stored at `workspace/.steel-memory/palace.sqlite3`.
    pub fn new(workspace: &Path) -> Result<Self, String> {
        let steel_dir = workspace.join(".steel-memory");
        std::fs::create_dir_all(&steel_dir)
            .map_err(|e| format!("Failed to create .steel-memory directory: {}", e))?;

        let db_path = steel_dir.join("palace.sqlite3");

        // Initialize storage to create tables
        VectorStorage::new(&db_path)
            .map_err(|e| format!("Failed to initialize vector storage: {}", e))?;

        Ok(Self {
            db_path,
            palace_path: workspace.to_path_buf(),
            embedding: Arc::new(Mutex::new(None)),
        })
    }

    /// Ensure the embedding model is loaded.
    async fn ensure_embedding(&self) -> Result<(), String> {
        let mut guard: tokio::sync::MutexGuard<'_, Option<TextEmbedding>> = self.embedding.lock().await;
        if guard.is_none() {
            info!("Loading embedding model (AllMiniLML6V2)...");
            let model = tokio::task::spawn_blocking(|| -> Result<TextEmbedding, String> {
                TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
                    .map_err(|e| format!("Failed to load embedding model: {}", e))
            })
            .await
            .map_err(|e| format!("Embedding task failed: {}", e))??;
            *guard = Some(model);
            info!("Embedding model loaded");
        }
        Ok(())
    }

    /// Embed text into a vector.
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.ensure_embedding().await?;
        
        let embedding = self.embedding.clone();
        let text_owned = text.to_string();
        
        let result = tokio::task::spawn_blocking(move || -> Result<Vec<f32>, String> {
            let mut guard = embedding.blocking_lock();
            let model = guard.as_mut().ok_or_else(|| "Embedding model not initialized".to_string())?;
            let mut embeddings = model
                .embed(vec![text_owned.as_str()], None)
                .map_err(|e| format!("Embedding failed: {}", e))?;
            Ok(embeddings.remove(0))
        })
        .await
        .map_err(|e| format!("Embedding task failed: {}", e))??;
        
        Ok(result)
    }

    /// Search for memories matching a query.
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

        let results = tokio::task::spawn_blocking(move || -> Result<Vec<SteelSearchResult>, String> {
            let storage = VectorStorage::new(&db_path)
                .map_err(|e| format!("Failed to open storage: {}", e))?;
            storage
                .search(&query_vec, max_results * 2, None, None) // Over-fetch to filter
                .map_err(|e| format!("Search failed: {}", e))
        })
        .await
        .map_err(|e| format!("Search task failed: {}", e))??;

        Ok(results
            .into_iter()
            .filter(|r| r.similarity >= min_score)
            .take(max_results)
            .map(SearchResult::from)
            .collect())
    }

    /// Add a memory to the index.
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
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let storage = VectorStorage::new(&db_path)
                .map_err(|e| format!("Failed to open storage: {}", e))?;
            storage
                .add_drawer(&drawer, &vec)
                .map_err(|e| format!("Failed to add drawer: {}", e))
        })
        .await
        .map_err(|e| format!("Add task failed: {}", e))??;

        debug!(id = %id, wing, room, "Added memory to steel-memory");
        Ok(id)
    }

    /// Index workspace memory files (MEMORY.md, memory/*.md).
    ///
    /// This reads markdown files and chunks them into the vector database,
    /// replacing the BM25 index with semantic embeddings.
    pub async fn index_workspace(&self) -> Result<usize, String> {
        info!(workspace = %self.palace_path.display(), "Indexing workspace memories");

        let mut count = 0;

        // Index MEMORY.md
        let memory_md = self.palace_path.join("MEMORY.md");
        if memory_md.exists() {
            count += self.index_file(&memory_md, "MEMORY.md", "memory", "long-term").await?;
        }

        // Index memory/*.md
        let memory_dir = self.palace_path.join("memory");
        if memory_dir.exists() && memory_dir.is_dir() {
            for entry in std::fs::read_dir(&memory_dir)
                .map_err(|e| format!("Failed to read memory dir: {}", e))?
            {
                let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    let name = path.file_name().unwrap().to_string_lossy();
                    let relative = format!("memory/{}", name);
                    
                    // Use date as room if filename is YYYY-MM-DD.md
                    let room = if name.len() == 13 && name.chars().take(10).all(|c| c.is_ascii_digit() || c == '-') {
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

    /// Index a single markdown file.
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
            
            // Add memory (deduplication can be added later with content hashing)
            self.add_memory(&chunk, wing, room, Some(relative_path)).await?;
            count += 1;
        }

        debug!(path = %relative_path, chunks = count, "Indexed file");
        Ok(count)
    }

    /// Get total number of memories.
    pub async fn count(&self) -> Result<usize, String> {
        let db_path = self.db_path.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<Vec<Drawer>, String> {
            let storage = VectorStorage::new(&db_path)
                .map_err(|e| format!("Failed to open storage: {}", e))?;
            storage
                .get_all(None, None, usize::MAX)
                .map_err(|e| format!("Failed to count: {}", e))
        })
        .await
        .map_err(|e| format!("Count task failed: {}", e))??;
        
        Ok(result.len())
    }
}

/// Chunk markdown content into sections.
fn chunk_markdown(content: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut line_count = 0;

    for line in content.lines() {
        let is_heading = line.starts_with("## ") || line.starts_with("# ");

        // Start new chunk on heading or every ~20 lines
        if (is_heading || line_count >= 20) && !current_chunk.trim().is_empty() {
            chunks.push(current_chunk.trim().to_string());
            current_chunk = String::new();
            line_count = 0;
        }

        current_chunk.push_str(line);
        current_chunk.push('\n');
        line_count += 1;
    }

    // Don't forget the last chunk
    if !current_chunk.trim().is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[tokio::test]
    async fn test_basic_search() {
        let dir = TempDir::new().unwrap();
        let index = SteelMemoryIndex::new(dir.path()).unwrap();

        // Add some memories
        index.add_memory("I love programming in Rust", "preferences", "languages", None).await.unwrap();
        index.add_memory("Python is great for data science", "preferences", "languages", None).await.unwrap();
        index.add_memory("The sky is blue today", "observations", "weather", None).await.unwrap();

        // Search for programming
        let results = index.search("Rust programming", 5, None).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_index_workspace() {
        let dir = TempDir::new().unwrap();
        
        // Create test files
        fs::write(dir.path().join("MEMORY.md"), "# Memory\n\nI like cats.").unwrap();
        fs::create_dir(dir.path().join("memory")).unwrap();
        fs::write(dir.path().join("memory/2026-04-13.md"), "# Today\n\nWent for a walk.").unwrap();

        let index = SteelMemoryIndex::new(dir.path()).unwrap();
        let count = index.index_workspace().await.unwrap();
        
        assert!(count >= 2);
        
        // Search should find results
        let results = index.search("cats", 5, None).await.unwrap();
        assert!(!results.is_empty());
    }
}
