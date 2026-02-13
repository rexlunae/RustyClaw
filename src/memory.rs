//! Memory search and retrieval for RustyClaw.
//!
//! Provides semantic-like search over `MEMORY.md` and `memory/*.md` files.
//! Current implementation uses keyword/BM25-style matching; embeddings can be
//! added later for true semantic search.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// A chunk of text from a memory file with metadata.
#[derive(Debug, Clone)]
pub struct MemoryChunk {
    /// Source file path (relative to workspace).
    pub path: String,
    /// Starting line number (1-indexed).
    pub start_line: usize,
    /// Ending line number (1-indexed, inclusive).
    pub end_line: usize,
    /// The text content of this chunk.
    pub text: String,
}

/// A search result with relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching chunk.
    pub chunk: MemoryChunk,
    /// Relevance score (higher is better).
    pub score: f64,
}

/// Memory search index.
pub struct MemoryIndex {
    /// All indexed chunks.
    chunks: Vec<MemoryChunk>,
    /// Inverted index: term -> chunk indices.
    term_index: HashMap<String, Vec<usize>>,
    /// Document frequency for each term.
    doc_freq: HashMap<String, usize>,
    /// Total number of chunks.
    total_docs: usize,
}

impl MemoryIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            term_index: HashMap::new(),
            doc_freq: HashMap::new(),
            total_docs: 0,
        }
    }

    /// Index all memory files in a workspace.
    pub fn index_workspace(workspace: &Path) -> Result<Self, String> {
        let mut index = Self::new();
        
        // Index MEMORY.md if it exists
        let memory_md = workspace.join("MEMORY.md");
        if memory_md.exists() {
            index.index_file(&memory_md, "MEMORY.md")?;
        }
        
        // Index memory/*.md
        let memory_dir = workspace.join("memory");
        if memory_dir.exists() && memory_dir.is_dir() {
            index.index_directory(&memory_dir, "memory")?;
        }
        
        // Build inverted index
        index.build_inverted_index();
        
        Ok(index)
    }

    /// Index a single file.
    fn index_file(&mut self, path: &Path, relative_path: &str) -> Result<(), String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", relative_path, e))?;
        
        // Split into chunks (~400 tokens target, roughly 300-400 words)
        // For simplicity, we chunk by paragraphs or heading sections
        let chunks = self.chunk_content(&content, relative_path);
        self.chunks.extend(chunks);
        
        Ok(())
    }

    /// Index a directory recursively.
    fn index_directory(&mut self, dir: &Path, relative_prefix: &str) -> Result<(), String> {
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("Failed to read directory {}: {}", relative_prefix, e))?;
        
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let relative = format!("{}/{}", relative_prefix, name);
            
            if path.is_file() && name.ends_with(".md") {
                self.index_file(&path, &relative)?;
            } else if path.is_dir() && !name.starts_with('.') {
                self.index_directory(&path, &relative)?;
            }
        }
        
        Ok(())
    }

    /// Chunk content into searchable pieces.
    fn chunk_content(&self, content: &str, path: &str) -> Vec<MemoryChunk> {
        let mut chunks = Vec::new();
        let lines: Vec<&str> = content.lines().collect();
        
        if lines.is_empty() {
            return chunks;
        }
        
        // Chunk by sections (## headings) or every ~20 lines
        let mut current_chunk = String::new();
        let mut chunk_start = 1;
        let mut line_count = 0;
        
        for (i, line) in lines.iter().enumerate() {
            let line_num = i + 1;
            
            // Check if this is a heading that should start a new chunk
            let is_heading = line.starts_with("## ") || line.starts_with("# ");
            
            // Start new chunk on heading or every ~20 lines (if we have content)
            if (is_heading || line_count >= 20) && !current_chunk.trim().is_empty() {
                chunks.push(MemoryChunk {
                    path: path.to_string(),
                    start_line: chunk_start,
                    end_line: line_num - 1,
                    text: current_chunk.trim().to_string(),
                });
                current_chunk = String::new();
                chunk_start = line_num;
                line_count = 0;
            }
            
            current_chunk.push_str(line);
            current_chunk.push('\n');
            line_count += 1;
        }
        
        // Don't forget the last chunk
        if !current_chunk.trim().is_empty() {
            chunks.push(MemoryChunk {
                path: path.to_string(),
                start_line: chunk_start,
                end_line: lines.len(),
                text: current_chunk.trim().to_string(),
            });
        }
        
        chunks
    }

    /// Build the inverted index for BM25 search.
    fn build_inverted_index(&mut self) {
        self.term_index.clear();
        self.doc_freq.clear();
        self.total_docs = self.chunks.len();
        
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let terms = tokenize(&chunk.text);
            let unique_terms: std::collections::HashSet<_> = terms.iter().collect();
            
            for term in unique_terms {
                self.term_index
                    .entry(term.clone())
                    .or_default()
                    .push(idx);
                
                *self.doc_freq.entry(term.clone()).or_insert(0) += 1;
            }
        }
    }

    /// Search the index using BM25-style scoring.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<SearchResult> {
        let query_terms = tokenize(query);
        
        if query_terms.is_empty() || self.chunks.is_empty() {
            return Vec::new();
        }
        
        // Score each chunk
        let mut scores: Vec<(usize, f64)> = Vec::new();
        
        for (idx, _chunk) in self.chunks.iter().enumerate() {
            let score = self.bm25_score(idx, &query_terms);
            if score > 0.0 {
                scores.push((idx, score));
            }
        }
        
        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        
        // Return top results
        scores
            .into_iter()
            .take(max_results)
            .map(|(idx, score)| SearchResult {
                chunk: self.chunks[idx].clone(),
                score,
            })
            .collect()
    }

    /// Calculate BM25 score for a chunk.
    fn bm25_score(&self, chunk_idx: usize, query_terms: &[String]) -> f64 {
        const K1: f64 = 1.2;
        const B: f64 = 0.75;
        
        let chunk = &self.chunks[chunk_idx];
        let chunk_terms = tokenize(&chunk.text);
        let doc_len = chunk_terms.len() as f64;
        
        // Calculate average document length
        let avg_doc_len = self.chunks.iter()
            .map(|c| tokenize(&c.text).len())
            .sum::<usize>() as f64 / self.total_docs.max(1) as f64;
        
        let mut score = 0.0;
        
        for term in query_terms {
            let tf = chunk_terms.iter().filter(|t| *t == term).count() as f64;
            let df = *self.doc_freq.get(term).unwrap_or(&0) as f64;
            
            if tf > 0.0 && df > 0.0 {
                // IDF component
                let idf = ((self.total_docs as f64 - df + 0.5) / (df + 0.5) + 1.0).ln();
                
                // TF component with length normalization
                let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * (doc_len / avg_doc_len)));
                
                score += idf * tf_norm;
            }
        }
        
        score
    }
}

impl Default for MemoryIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Tokenize text into lowercase terms for indexing/searching.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|s| s.len() >= 2) // Skip very short tokens
        .map(|s| s.to_string())
        .collect()
}

/// Read specific lines from a memory file.
pub fn read_memory_file(
    workspace: &Path,
    relative_path: &str,
    from_line: Option<usize>,
    num_lines: Option<usize>,
) -> Result<String, String> {
    // Validate path is within memory scope
    if !is_valid_memory_path(relative_path) {
        return Err(format!(
            "Path '{}' is not a valid memory file. Must be MEMORY.md or memory/*.md",
            relative_path
        ));
    }
    
    let full_path = workspace.join(relative_path);
    
    if !full_path.exists() {
        return Err(format!("Memory file not found: {}", relative_path));
    }
    
    let content = fs::read_to_string(&full_path)
        .map_err(|e| format!("Failed to read {}: {}", relative_path, e))?;
    
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    
    // Handle line range
    let start = from_line.unwrap_or(1).saturating_sub(1); // Convert to 0-indexed
    let count = num_lines.unwrap_or(total_lines);
    
    if start >= total_lines {
        return Ok(String::new());
    }
    
    let end = (start + count).min(total_lines);
    let selected: Vec<&str> = lines[start..end].to_vec();
    
    Ok(selected.join("\n"))
}

/// Check if a path is a valid memory file path.
fn is_valid_memory_path(path: &str) -> bool {
    // Must be MEMORY.md or within memory/ directory
    if path == "MEMORY.md" {
        return true;
    }
    
    if path.starts_with("memory/") && path.ends_with(".md") {
        // Check for path traversal
        !path.contains("..") && !path.contains("//")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_workspace() -> TempDir {
        let dir = TempDir::new().unwrap();
        
        // Create MEMORY.md
        fs::write(
            dir.path().join("MEMORY.md"),
            "# Long-term Memory\n\n## Preferences\nUser prefers dark mode.\nFavorite color is blue.\n\n## Projects\nWorking on RustyClaw.\n"
        ).unwrap();
        
        // Create memory directory
        fs::create_dir(dir.path().join("memory")).unwrap();
        
        // Create daily note
        fs::write(
            dir.path().join("memory/2026-02-12.md"),
            "# 2026-02-12\n\n## Morning\nStarted implementing memory tools.\n\n## Afternoon\nWorking on BM25 search.\n"
        ).unwrap();
        
        dir
    }

    #[test]
    fn test_index_workspace() {
        let workspace = setup_test_workspace();
        let index = MemoryIndex::index_workspace(workspace.path()).unwrap();
        
        assert!(!index.chunks.is_empty());
        assert!(index.total_docs > 0);
    }

    #[test]
    fn test_search_finds_relevant() {
        let workspace = setup_test_workspace();
        let index = MemoryIndex::index_workspace(workspace.path()).unwrap();
        
        let results = index.search("dark mode", 5);
        assert!(!results.is_empty());
        assert!(results[0].chunk.text.contains("dark mode"));
    }

    #[test]
    fn test_search_empty_query() {
        let workspace = setup_test_workspace();
        let index = MemoryIndex::index_workspace(workspace.path()).unwrap();
        
        let results = index.search("", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_read_memory_file() {
        let workspace = setup_test_workspace();
        
        let content = read_memory_file(workspace.path(), "MEMORY.md", None, None).unwrap();
        assert!(content.contains("Long-term Memory"));
    }

    #[test]
    fn test_read_memory_file_with_range() {
        let workspace = setup_test_workspace();
        
        let content = read_memory_file(workspace.path(), "MEMORY.md", Some(3), Some(2)).unwrap();
        // Line 3-4 should be "## Preferences" and the next line
        assert!(!content.is_empty());
    }

    #[test]
    fn test_read_memory_file_invalid_path() {
        let workspace = setup_test_workspace();
        
        let result = read_memory_file(workspace.path(), "../etc/passwd", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_valid_memory_paths() {
        assert!(is_valid_memory_path("MEMORY.md"));
        assert!(is_valid_memory_path("memory/2026-02-12.md"));
        assert!(is_valid_memory_path("memory/notes/work.md"));
        
        assert!(!is_valid_memory_path("../secret.md"));
        assert!(!is_valid_memory_path("memory/../../../etc/passwd"));
        assert!(!is_valid_memory_path("src/main.rs"));
        assert!(!is_valid_memory_path("memory/file.txt"));
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello, World! This is a TEST.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Single-char tokens should be filtered
        assert!(!tokens.contains(&"a".to_string()));
    }
}
