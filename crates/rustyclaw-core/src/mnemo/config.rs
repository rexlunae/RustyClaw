//! Mnemo configuration.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for the memory coprocessor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MnemoConfig {
    /// Whether mnemo is enabled.
    #[serde(default)]
    pub enabled: bool,
    
    /// Path to the SQLite database.
    /// Default: `<settings_dir>/mnemo.sqlite3`
    #[serde(default)]
    pub db_path: Option<PathBuf>,
    
    /// Number of recent messages to keep verbatim (not summarized).
    /// Default: 6
    #[serde(default = "default_fresh_tail")]
    pub fresh_tail_messages: usize,
    
    /// Number of messages per leaf compaction chunk.
    /// Default: 8
    #[serde(default = "default_leaf_chunk")]
    pub leaf_chunk_size: usize,
    
    /// Number of summaries per condensed compaction chunk.
    /// Default: 4
    #[serde(default = "default_condensed_chunk")]
    pub condensed_chunk_size: usize,
    
    /// Trigger compaction when context items exceed this count.
    /// Default: 24
    #[serde(default = "default_threshold")]
    pub threshold_items: usize,
    
    /// Summarization configuration.
    #[serde(default)]
    pub summarization: SummarizationConfig,
}

impl Default for MnemoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: None,
            fresh_tail_messages: default_fresh_tail(),
            leaf_chunk_size: default_leaf_chunk(),
            condensed_chunk_size: default_condensed_chunk(),
            threshold_items: default_threshold(),
            summarization: SummarizationConfig::default(),
        }
    }
}

/// Summarization backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizationConfig {
    /// Whether to use the main model for summarization.
    /// If true, uses the same provider/model as the agent.
    /// If false, uses the configured provider/model below.
    #[serde(default = "default_use_main_model")]
    pub use_main_model: bool,
    
    /// Provider for summarization (if not using main model).
    /// Default: "openrouter"
    #[serde(default)]
    pub provider: Option<String>,
    
    /// Model for summarization (if not using main model).
    /// Default: "google/gemini-2.5-flash"
    #[serde(default)]
    pub model: Option<String>,
    
    /// Fallback strategy when LLM is unavailable.
    /// Options: "truncate", "disabled"
    /// Default: "truncate"
    #[serde(default = "default_fallback")]
    pub fallback: String,
    
    /// Max characters per message in truncate fallback.
    #[serde(default = "default_truncate_chars")]
    pub truncate_chars: usize,
    
    /// Max total characters in truncate fallback.
    #[serde(default = "default_truncate_total")]
    pub truncate_total: usize,
}

impl Default for SummarizationConfig {
    fn default() -> Self {
        Self {
            use_main_model: true,
            provider: None,
            model: None,
            fallback: default_fallback(),
            truncate_chars: default_truncate_chars(),
            truncate_total: default_truncate_total(),
        }
    }
}

fn default_fresh_tail() -> usize { 6 }
fn default_leaf_chunk() -> usize { 8 }
fn default_condensed_chunk() -> usize { 4 }
fn default_threshold() -> usize { 24 }
fn default_use_main_model() -> bool { true }
fn default_fallback() -> String { "truncate".to_string() }
fn default_truncate_chars() -> usize { 180 }
fn default_truncate_total() -> usize { 900 }
