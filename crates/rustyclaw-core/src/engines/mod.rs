//! Local inference engine management.
//!
//! Provides a common [`LocalEngine`] trait that unifies lifecycle control
//! (detect/install/start/stop) and model management (list/pull/remove/load/unload)
//! across Ollama, Exo, llama.cpp, and LM Studio.

pub mod exo;
pub mod llamacpp;
pub mod lmstudio;
pub mod ollama;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Core types ──────────────────────────────────────────────────────────────

/// What the engine binary/process looks like on this host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnginePresence {
    /// Whether the engine binary is detected on the system.
    pub installed: bool,
    /// Engine version string, if detected.
    pub version: Option<String>,
    /// Absolute path to the binary, if found.
    pub binary_path: Option<String>,
}

/// Runtime status of a local engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EngineRunStatus {
    /// Not running.
    #[default]
    Stopped,
    /// Running and healthy.
    Running {
        endpoint: String,
        loaded_models: u32,
        available_models: u32,
    },
    /// Running but not responding to health checks.
    Unhealthy { endpoint: String, error: String },
}

/// Full status snapshot for an engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineStatus {
    pub presence: EnginePresence,
    pub run_status: EngineRunStatus,
}

/// Per-engine configuration (stored in Config.engines).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Whether this engine is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Override endpoint URL (default per engine).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Port override (default per engine).
    #[serde(default)]
    pub port: Option<u16>,
    /// Custom models directory.
    #[serde(default)]
    pub models_dir: Option<String>,
    /// Start the engine automatically when the gateway starts.
    #[serde(default)]
    pub auto_start: bool,
    /// Extra CLI arguments for the engine process.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: None,
            port: None,
            models_dir: None,
            auto_start: false,
            extra_args: Vec::new(),
        }
    }
}

/// Capability flags for an engine — determines which actions the UI enables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCaps {
    pub can_install: bool,
    pub can_start: bool,
    pub can_stop: bool,
    pub can_pull: bool,
    pub can_remove: bool,
    pub can_load: bool,
    pub can_unload: bool,
}

impl EngineCaps {
    /// Full control (Ollama, llama.cpp).
    pub fn full() -> Self {
        Self {
            can_install: true,
            can_start: true,
            can_stop: true,
            can_pull: true,
            can_remove: true,
            can_load: true,
            can_unload: true,
        }
    }

    /// Read-only + lifecycle (Exo — can start/stop but pull is via its own UI).
    pub fn lifecycle_only() -> Self {
        Self {
            can_install: true,
            can_start: true,
            can_stop: true,
            can_pull: false,
            can_remove: false,
            can_load: true,
            can_unload: true,
        }
    }

    /// Status and list only (LM Studio — manages its own lifecycle).
    pub fn read_only() -> Self {
        Self {
            can_install: false,
            can_start: false,
            can_stop: false,
            can_pull: false,
            can_remove: false,
            can_load: false,
            can_unload: false,
        }
    }
}

/// A local model as reported by an engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    /// Model name/tag (e.g. "llama3.1:8b-instruct-q4_K_M").
    pub name: String,
    /// Size on disk in bytes.
    pub size_bytes: u64,
    /// Quantization info (e.g. "Q4_K_M").
    pub quantization: Option<String>,
    /// Context window size (if known).
    pub context_length: Option<u32>,
    /// Whether this model is currently loaded in memory.
    pub loaded: bool,
    /// VRAM usage in bytes (if loaded).
    pub vram_bytes: Option<u64>,
    /// Model family/architecture.
    pub family: Option<String>,
    /// Model format (e.g. "gguf", "safetensors").
    pub format: Option<String>,
    /// Last modified timestamp.
    pub modified_at: Option<String>,
}

impl LocalModel {
    /// Human-readable size.
    pub fn size_display(&self) -> String {
        if self.size_bytes >= 1_000_000_000 {
            format!("{:.1} GB", self.size_bytes as f64 / 1e9)
        } else {
            format!("{:.0} MB", self.size_bytes as f64 / 1e6)
        }
    }
}

/// Progress update for streamed operations (pull/install).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullProgress {
    pub model: String,
    pub status: String,
    pub percent: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

impl fmt::Display for PullProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {:.0}% ({}/{})",
            self.model,
            self.percent,
            format_bytes(self.downloaded_bytes),
            format_bytes(self.total_bytes),
        )
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes >= 1_000_000 {
        format!("{:.0} MB", bytes as f64 / 1e6)
    } else {
        format!("{:.0} KB", bytes as f64 / 1e3)
    }
}

/// Channel for streaming progress updates.
pub type ProgressSink = tokio::sync::mpsc::Sender<PullProgress>;

// ── Trait ───────────────────────────────────────────────────────────────────

/// Common interface for local inference engines.
#[async_trait::async_trait]
pub trait LocalEngine: Send + Sync {
    /// Short identifier: "ollama", "exo", "llamacpp", "lmstudio".
    fn id(&self) -> &str;

    /// Human-friendly display name.
    fn display_name(&self) -> &str;

    /// Default endpoint URL.
    fn default_endpoint(&self) -> &str;

    /// Detect whether the engine is installed.
    async fn detect(&self) -> EnginePresence;

    /// Get full runtime status.
    async fn status(&self, cfg: &EngineConfig) -> EngineStatus;

    /// Install the engine (if not already present).
    async fn install(&self, sink: Option<ProgressSink>) -> Result<String>;

    /// Start the engine process.
    async fn start(&self, cfg: &EngineConfig) -> Result<String>;

    /// Stop the engine process.
    async fn stop(&self) -> Result<String>;

    /// List models available to this engine.
    async fn list_models(&self, cfg: &EngineConfig) -> Result<Vec<LocalModel>>;

    /// Pull/download a model (streamed progress).
    async fn pull(
        &self,
        model: &str,
        cfg: &EngineConfig,
        sink: Option<ProgressSink>,
    ) -> Result<String>;

    /// Remove a model from disk.
    async fn remove(&self, model: &str, cfg: &EngineConfig) -> Result<String>;

    /// Load a model into memory (GPU/CPU).
    async fn load(&self, model: &str, cfg: &EngineConfig) -> Result<String>;

    /// Unload a model from memory.
    async fn unload(&self, model: &str, cfg: &EngineConfig) -> Result<String>;

    /// What this engine supports.
    fn capabilities(&self) -> EngineCaps;
}

// ── Registry ────────────────────────────────────────────────────────────────

/// Registry of all known local engines.
pub struct EngineRegistry {
    engines: Vec<Box<dyn LocalEngine>>,
}

impl EngineRegistry {
    /// Create a registry with all built-in engines.
    pub fn new() -> Self {
        Self {
            engines: vec![
                Box::new(ollama::OllamaEngine),
                Box::new(exo::ExoEngine),
                Box::new(llamacpp::LlamaCppEngine),
                Box::new(lmstudio::LmStudioEngine),
            ],
        }
    }

    /// Get all engines.
    pub fn all(&self) -> &[Box<dyn LocalEngine>] {
        &self.engines
    }

    /// Look up an engine by id.
    pub fn get(&self, id: &str) -> Option<&dyn LocalEngine> {
        self.engines.iter().find(|e| e.id() == id).map(|e| &**e)
    }
}

impl Default for EngineRegistry {
    fn default() -> Self {
        Self::new()
    }
}
