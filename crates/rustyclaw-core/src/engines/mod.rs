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

// ── Host-fit & pre-flight ───────────────────────────────────────────────────

/// Result of a host-fit check for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFitResult {
    /// Whether the model fits the host resources.
    pub fits: bool,
    /// Human-readable warning message (empty if fits).
    pub warning: String,
}

/// Check whether a model fits on the current host.
///
/// Compares the model's size against available VRAM/RAM. Returns a warning
/// string if the model likely won't fit — the UI should display it but not
/// block the user.
pub fn check_model_fit(model: &LocalModel) -> ModelFitResult {
    let host = crate::runtime_ctx::get_host();
    let Some(host) = host else {
        return ModelFitResult {
            fits: true,
            warning: String::new(),
        };
    };

    let vram = host.total_vram_bytes();
    let ram = host.total_memory_bytes;

    // Estimate memory needed: for inference, the model weights need to fit
    // in VRAM (GPU) or RAM (CPU). Use size_bytes as a rough proxy (actual
    // inference memory is ~1.2× model size for KV cache overhead).
    let model_mem = (model.size_bytes as f64 * 1.2) as u64;

    if vram > 0 {
        // GPU available — check VRAM fit.
        if model_mem > vram {
            let need_gb = model_mem as f64 / 1e9;
            let have_gb = vram as f64 / 1e9;
            return ModelFitResult {
                fits: false,
                warning: format!(
                    "Model needs ~{:.1} GB VRAM but host has {:.1} GB",
                    need_gb, have_gb
                ),
            };
        }
    } else {
        // CPU-only — check RAM fit (needs room for OS + inference).
        let available_for_model = ram.saturating_sub(4 * 1024 * 1024 * 1024); // reserve 4 GB
        if model_mem > available_for_model {
            let need_gb = model_mem as f64 / 1e9;
            let have_gb = ram as f64 / 1e9;
            return ModelFitResult {
                fits: false,
                warning: format!(
                    "Model needs ~{:.1} GB RAM but host has {:.1} GB total",
                    need_gb, have_gb
                ),
            };
        }
    }

    ModelFitResult {
        fits: true,
        warning: String::new(),
    }
}

/// Pre-flight check before pulling/downloading a model.
///
/// Checks available disk space against the expected download size.
/// Returns Ok(()) if there's enough space, or an error message.
pub fn preflight_disk_check(expected_bytes: u64) -> Result<()> {
    let host = crate::runtime_ctx::get_host();
    let Some(host) = host else {
        return Ok(());
    };

    // Check disk space: require expected_bytes + 10% buffer.
    let required = expected_bytes + (expected_bytes / 10);
    if host.disk_available_bytes < required {
        let need_gb = required as f64 / 1e9;
        let have_gb = host.disk_available_bytes as f64 / 1e9;
        anyhow::bail!(
            "Insufficient disk space: need ~{:.1} GB but only {:.1} GB available",
            need_gb,
            have_gb
        );
    }

    Ok(())
}

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

// ── Service integration ─────────────────────────────────────────────────────

/// Build `ServiceDef` entries for engines with `auto_start = true`.
///
/// The caller inserts these into the `ServicesConfig` so the existing service
/// manager handles lifecycle (restart, health-checks, logs) for free.
pub fn engine_service_defs(
    configs: &std::collections::HashMap<String, EngineConfig>,
) -> Vec<(String, crate::services::ServiceDef)> {
    let registry = EngineRegistry::new();
    let mut defs = Vec::new();

    for (id, cfg) in configs {
        if !cfg.auto_start || !cfg.enabled {
            continue;
        }
        // Only engines that can start are eligible.
        let Some(engine) = registry.get(id) else {
            continue;
        };
        if !engine.capabilities().can_start {
            continue;
        }

        let (command, args) = engine_start_command(id, cfg);
        let port = cfg.port.unwrap_or_else(|| default_port(id));

        let health_url = cfg
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", port));

        let svc = crate::services::ServiceDef {
            command,
            args,
            env: std::collections::HashMap::new(),
            cwd: None,
            service_type: crate::services::ServiceType::Http,
            restart: crate::services::RestartPolicy::OnFailure,
            auto_start: true,
            health_check: Some(crate::services::HealthCheck {
                method: crate::services::HealthMethod::HttpGet { url: health_url },
                interval_secs: 5,
                timeout_secs: 3,
                retries: 2,
            }),
            max_log_lines: 500,
        };
        defs.push((format!("engine-{}", id), svc));
    }

    defs
}

/// Determine the command+args to start an engine process.
fn engine_start_command(id: &str, cfg: &EngineConfig) -> (String, Vec<String>) {
    let mut args: Vec<String> = cfg.extra_args.clone();
    match id {
        "ollama" => {
            let cmd = "ollama".to_string();
            let mut a = vec!["serve".to_string()];
            a.extend(args);
            (cmd, a)
        }
        "exo" => {
            let cmd = "exo".to_string();
            // exo starts with no subcommand by default
            (cmd, args)
        }
        "llamacpp" => {
            let cmd = "llama-server".to_string();
            if let Some(port) = cfg.port {
                args.extend(["--port".to_string(), port.to_string()]);
            }
            if let Some(ref models_dir) = cfg.models_dir {
                args.extend(["--model-store".to_string(), models_dir.clone()]);
            }
            (cmd, args)
        }
        _ => ("echo".to_string(), vec!["unsupported-engine".to_string()]),
    }
}

/// Default port for each engine.
fn default_port(id: &str) -> u16 {
    match id {
        "ollama" => 11434,
        "exo" => 52415,
        "llamacpp" => 8080,
        "lmstudio" => 1234,
        _ => 8080,
    }
}
