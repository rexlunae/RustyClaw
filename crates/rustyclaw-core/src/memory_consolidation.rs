//! Two-layer memory consolidation, inspired by nanobot.
//!
//! This module implements LLM-driven memory consolidation with two layers:
//! - **MEMORY.md**: Long-term facts, curated by the LLM
//! - **HISTORY.md**: Grep-searchable timestamped log
//!
//! The LLM calls `save_memory` to consolidate conversation history, deciding
//! what facts to keep in MEMORY.md and what to log in HISTORY.md.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Result of a memory consolidation operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// Whether consolidation was performed.
    pub performed: bool,
    /// Number of messages consolidated.
    pub messages_consolidated: usize,
    /// New MEMORY.md size in bytes.
    pub memory_size: usize,
    /// New HISTORY.md size in bytes.
    pub history_size: usize,
    /// Error message if consolidation failed.
    pub error: Option<String>,
}

/// Configuration for memory consolidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationConfig {
    /// Enable automatic consolidation.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Trigger consolidation after this many messages.
    #[serde(default = "default_message_threshold")]
    pub message_threshold: usize,

    /// Maximum size for MEMORY.md in bytes before warning.
    #[serde(default = "default_memory_max_size")]
    pub memory_max_size: usize,

    /// Path to MEMORY.md (relative to workspace).
    #[serde(default = "default_memory_path")]
    pub memory_path: String,

    /// Path to HISTORY.md (relative to workspace).
    #[serde(default = "default_history_path")]
    pub history_path: String,
}

fn default_true() -> bool {
    true
}

fn default_message_threshold() -> usize {
    20
}

fn default_memory_max_size() -> usize {
    50 * 1024 // 50KB
}

fn default_memory_path() -> String {
    "MEMORY.md".to_string()
}

fn default_history_path() -> String {
    "HISTORY.md".to_string()
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            message_threshold: default_message_threshold(),
            memory_max_size: default_memory_max_size(),
            memory_path: default_memory_path(),
            history_path: default_history_path(),
        }
    }
}

/// Memory consolidation controller.
///
/// Manages the two-layer memory system:
/// - MEMORY.md: Long-term facts (LLM-maintained)
/// - HISTORY.md: Timestamped log (append-only)
pub struct MemoryConsolidation {
    config: ConsolidationConfig,
    /// Messages since last consolidation.
    messages_since_consolidation: usize,
}

impl MemoryConsolidation {
    /// Create a new consolidation controller.
    pub fn new(config: ConsolidationConfig) -> Self {
        Self {
            config,
            messages_since_consolidation: 0,
        }
    }

    /// Check if consolidation should be triggered.
    pub fn should_consolidate(&self) -> bool {
        self.config.enabled && self.messages_since_consolidation >= self.config.message_threshold
    }

    /// Increment the message counter.
    pub fn record_message(&mut self) {
        self.messages_since_consolidation += 1;
    }

    /// Reset the message counter after consolidation.
    pub fn reset_counter(&mut self) {
        self.messages_since_consolidation = 0;
    }

    /// Get the current message count.
    pub fn message_count(&self) -> usize {
        self.messages_since_consolidation
    }

    /// Save a history entry (append to HISTORY.md).
    ///
    /// This is called by the `save_memory` tool to log timestamped entries.
    pub fn append_history(&self, workspace: &Path, entry: &str) -> Result<usize, String> {
        let history_path = workspace.join(&self.config.history_path);

        // Create parent directories if needed
        if let Some(parent) = history_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let timestamp = Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();
        let formatted = format!("\n[{}] {}\n", timestamp, entry.trim());

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&history_path)
            .map_err(|e| format!("Failed to open HISTORY.md: {}", e))?;

        file.write_all(formatted.as_bytes())
            .map_err(|e| format!("Failed to write to HISTORY.md: {}", e))?;

        let metadata = fs::metadata(&history_path)
            .map_err(|e| format!("Failed to read HISTORY.md metadata: {}", e))?;

        Ok(metadata.len() as usize)
    }

    /// Update MEMORY.md (full replacement).
    ///
    /// This is called by the `save_memory` tool with the LLM's curated content.
    pub fn update_memory(&self, workspace: &Path, content: &str) -> Result<usize, String> {
        let memory_path = workspace.join(&self.config.memory_path);

        // Create parent directories if needed
        if let Some(parent) = memory_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        fs::write(&memory_path, content)
            .map_err(|e| format!("Failed to write MEMORY.md: {}", e))?;

        let size = content.len();

        if size > self.config.memory_max_size {
            eprintln!(
                "Warning: MEMORY.md is {} bytes, exceeds recommended max of {} bytes",
                size, self.config.memory_max_size
            );
        }

        Ok(size)
    }

    /// Read current MEMORY.md content.
    pub fn read_memory(&self, workspace: &Path) -> Result<String, String> {
        let memory_path = workspace.join(&self.config.memory_path);

        if !memory_path.exists() {
            return Ok(String::new());
        }

        fs::read_to_string(&memory_path).map_err(|e| format!("Failed to read MEMORY.md: {}", e))
    }

    /// Read current HISTORY.md content.
    pub fn read_history(&self, workspace: &Path) -> Result<String, String> {
        let history_path = workspace.join(&self.config.history_path);

        if !history_path.exists() {
            return Ok(String::new());
        }

        fs::read_to_string(&history_path).map_err(|e| format!("Failed to read HISTORY.md: {}", e))
    }

    /// Search HISTORY.md using grep-style pattern matching.
    pub fn search_history(
        &self,
        workspace: &Path,
        pattern: &str,
        max_results: usize,
    ) -> Result<Vec<HistoryEntry>, String> {
        let history = self.read_history(workspace)?;
        let pattern_lower = pattern.to_lowercase();

        let mut results = Vec::new();
        let mut current_entry: Option<HistoryEntry> = None;

        for line in history.lines() {
            // Check if this is a new entry (starts with timestamp)
            if line.starts_with('[') && line.contains(']') {
                // Save previous entry if it matched
                if let Some(entry) = current_entry.take() {
                    if entry.text.to_lowercase().contains(&pattern_lower) {
                        results.push(entry);
                        if results.len() >= max_results {
                            break;
                        }
                    }
                }

                // Parse new entry
                if let Some(end_bracket) = line.find(']') {
                    let timestamp_str = &line[1..end_bracket];
                    let text = line[end_bracket + 1..].trim().to_string();

                    current_entry = Some(HistoryEntry {
                        timestamp: timestamp_str.to_string(),
                        text,
                    });
                }
            } else if let Some(ref mut entry) = current_entry {
                // Continuation of current entry
                entry.text.push('\n');
                entry.text.push_str(line);
            }
        }

        // Don't forget the last entry
        if let Some(entry) = current_entry {
            if entry.text.to_lowercase().contains(&pattern_lower) && results.len() < max_results {
                results.push(entry);
            }
        }

        Ok(results)
    }

    /// Get configuration reference.
    pub fn config(&self) -> &ConsolidationConfig {
        &self.config
    }
}

/// A single entry from HISTORY.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Timestamp string from the entry.
    pub timestamp: String,
    /// Entry text content.
    pub text: String,
}

/// Arguments for the save_memory tool.
///
/// The LLM provides both pieces in a single call:
/// - `history_entry`: A summary to append to HISTORY.md
/// - `memory_update`: The full new content for MEMORY.md (or None to skip)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveMemoryArgs {
    /// Timestamped summary to append to HISTORY.md.
    pub history_entry: String,

    /// Full updated MEMORY.md content (optional).
    /// If provided, replaces the entire file.
    /// If None, MEMORY.md is not modified.
    #[serde(default)]
    pub memory_update: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_append_history() {
        let dir = tempdir().unwrap();
        let config = ConsolidationConfig::default();
        let consolidation = MemoryConsolidation::new(config);

        let size = consolidation
            .append_history(dir.path(), "Test entry 1")
            .unwrap();
        assert!(size > 0);

        let size2 = consolidation
            .append_history(dir.path(), "Test entry 2")
            .unwrap();
        assert!(size2 > size);

        let history = consolidation.read_history(dir.path()).unwrap();
        assert!(history.contains("Test entry 1"));
        assert!(history.contains("Test entry 2"));
    }

    #[test]
    fn test_update_memory() {
        let dir = tempdir().unwrap();
        let config = ConsolidationConfig::default();
        let consolidation = MemoryConsolidation::new(config);

        let content = "# Memory\n\nSome important facts.";
        let size = consolidation.update_memory(dir.path(), content).unwrap();
        assert_eq!(size, content.len());

        let read_back = consolidation.read_memory(dir.path()).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_search_history() {
        let dir = tempdir().unwrap();
        let config = ConsolidationConfig::default();
        let consolidation = MemoryConsolidation::new(config);

        consolidation
            .append_history(dir.path(), "Meeting with Alice about project")
            .unwrap();
        consolidation
            .append_history(dir.path(), "Fixed bug in parser")
            .unwrap();
        consolidation
            .append_history(dir.path(), "Called Alice, discussed timeline")
            .unwrap();

        let results = consolidation
            .search_history(dir.path(), "Alice", 10)
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_consolidation_threshold() {
        let config = ConsolidationConfig {
            message_threshold: 5,
            ..Default::default()
        };
        let mut consolidation = MemoryConsolidation::new(config);

        for _ in 0..4 {
            consolidation.record_message();
            assert!(!consolidation.should_consolidate());
        }

        consolidation.record_message();
        assert!(consolidation.should_consolidate());

        consolidation.reset_counter();
        assert!(!consolidation.should_consolidate());
    }
}
