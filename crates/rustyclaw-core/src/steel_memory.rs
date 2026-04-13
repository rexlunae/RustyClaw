//! Steel Memory integration for RustyClaw.
//!
//! Provides semantic vector search over agent memories using steel-memory's
//! embedding and SQLite-backed storage.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

use steel_memory_lib::{
    fastembed::{EmbeddingModel, InitOptions, TextEmbedding},
    storage::vector::VectorStorage,
    types::{Drawer, SearchResult as SteelSearchResult},
};

pub struct SteelMemoryIndex {
    db_path: PathBuf,
    palace_path: PathBuf,
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

// Helper functions for spawn_blocking with explicit return types
fn load_embedding_model() -> Result<TextEmbedding, String> {
    TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))
        .map_err(|e| format!("Failed to load embedding model: {}", e))
}

fn do_embed(embedding: Arc<Mutex<Option<TextEmbedding>>>, text: String) -> Result<Vec<f32>, String> {
    let mut guard = embedding.blocking_lock();
    let model = guard.as_mut().ok_or_else(|| "Embedding model not initialized".to_string())?;
    let mut embeddings = model
        .embed(vec![text.as_str()], None)
        .map_err(|e| format!("Embedding failed: {}", e))?;
    Ok(embeddings.remove(0))
}

fn do_search(db_path: PathBuf, query_vec: Vec<f32>, limit: usize) -> Result<Vec<SteelSearchResult>, String> {
    let storage = VectorStorage::new(&db_path)
        .map_err(|e| format!("Failed to open storage: {}", e))?;
    storage
        .search(&query_vec, limit, None, None)
        .map_err(|e| format!("Search failed: {}", e))
}

fn do_add_drawer(db_path: PathBuf, drawer: Drawer, vec: Vec<f32>) -> Result<(), String> {
    let storage = VectorStorage::new(&db_path)
        .map_err(|e| format!("Failed to open storage: {}", e))?;
    storage
        .add_drawer(&drawer, &vec)
        .map_err(|e| format!("Failed to add drawer: {}", e))
}

fn do_get_all(db_path: PathBuf) -> Result<Vec<Drawer>, String> {
    let storage = VectorStorage::new(&db_path)
        .map_err(|e| format!("Failed to open storage: {}", e))?;
    storage
        .get_all(None, None, usize::MAX)
        .map_err(|e| format!("Failed to count: {}", e))
}

impl SteelMemoryIndex {
    pub fn new(workspace: &Path) -> Result<Self, String> {
        let steel_dir = workspace.join(".steel-memory");
        std::fs::create_dir_all(&steel_dir)
            .map_err(|e| format!("Failed to create .steel-memory directory: {}", e))?;

        let db_path = steel_dir.join("palace.sqlite3");
        VectorStorage::new(&db_path)
            .map_err(|e| format!("Failed to initialize vector storage: {}", e))?;

        Ok(Self {
            db_path,
            palace_path: workspace.to_path_buf(),
            embedding: Arc::new(Mutex::new(None)),
        })
    }

    async fn ensure_embedding(&self) -> Result<(), String> {
        let mut guard: tokio::sync::MutexGuard<'_, Option<TextEmbedding>> = self.embedding.lock().await;
        if guard.is_none() {
            info!("Loading embedding model (AllMiniLML6V2)...");
            let join_result = tokio::task::spawn_blocking(load_embedding_model).await;
            let model: TextEmbedding = match join_result {
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
        
        let join_result = tokio::task::spawn_blocking(move || do_embed(embedding, text_owned)).await;
        match join_result {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(e) => Err(format!("Embedding task panicked: {}", e)),
        }
    }

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

        let join_result = tokio::task::spawn_blocking(move || do_search(db_path, query_vec, limit)).await;
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
        let join_result = tokio::task::spawn_blocking(move || do_add_drawer(db_path, drawer, vec)).await;
        match join_result {
            Ok(Ok(())) => {},
            Ok(Err(e)) => return Err(e),
            Err(e) => return Err(format!("Add task panicked: {}", e)),
        }

        debug!(id = %id, wing, room, "Added memory to steel-memory");
        Ok(id)
    }

    pub async fn index_workspace(&self) -> Result<usize, String> {
        info!(workspace = %self.palace_path.display(), "Indexing workspace memories");

        let mut count = 0;

        let memory_md = self.palace_path.join("MEMORY.md");
        if memory_md.exists() {
            count += self.index_file(&memory_md, "MEMORY.md", "memory", "long-term").await?;
        }

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

    async fn index_file(&self, path: &Path, relative_path: &str, wing: &str, room: &str) -> Result<usize, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", relative_path, e))?;

        let chunks = chunk_markdown(&content);
        let mut count = 0;

        for chunk in chunks {
            if chunk.trim().is_empty() {
                continue;
            }
            self.add_memory(&chunk, wing, room, Some(relative_path)).await?;
            count += 1;
        }

        debug!(path = %relative_path, chunks = count, "Indexed file");
        Ok(count)
    }

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
}

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
    use tempfile::TempDir;
    use std::fs;

    #[tokio::test]
    async fn test_basic_search() {
        let dir = TempDir::new().unwrap();
        let index = SteelMemoryIndex::new(dir.path()).unwrap();

        index.add_memory("I love programming in Rust", "preferences", "languages", None).await.unwrap();
        index.add_memory("Python is great for data science", "preferences", "languages", None).await.unwrap();
        index.add_memory("The sky is blue today", "observations", "weather", None).await.unwrap();

        let results = index.search("Rust programming", 5, None).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_index_workspace() {
        let dir = TempDir::new().unwrap();
        
        fs::write(dir.path().join("MEMORY.md"), "# Memory\n\nI like cats.").unwrap();
        fs::create_dir(dir.path().join("memory")).unwrap();
        fs::write(dir.path().join("memory/2026-04-13.md"), "# Today\n\nWent for a walk.").unwrap();

        let index = SteelMemoryIndex::new(dir.path()).unwrap();
        let count = index.index_workspace().await.unwrap();
        
        assert!(count >= 2);
        
        let results = index.search("cats", 5, None).await.unwrap();
        assert!(!results.is_empty());
    }
}
