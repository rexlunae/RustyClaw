//! Joshua engine implementation.
//!
//! [Joshua](https://github.com/rexlunae/joshua) is a pure-Rust LLM inference
//! engine with an OpenAI-compatible server (`joshua serve`).  Unlike Ollama,
//! one Joshua process serves exactly one GGUF model, chosen at startup with
//! `--model <path>.gguf` (a `tokenizer.json` must sit beside the GGUF).
//!
//! RustyClaw therefore manages the model catalogue itself: models are GGUF
//! files in the engine's models directory (default
//! `~/.rustyclaw/models/joshua`), "loading" a model means (re)starting
//! `joshua serve` pointed at that file, and pulling downloads GGUF +
//! `tokenizer.json` from Hugging Face into the models directory.

use super::*;
use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Default port for RustyClaw-managed Joshua servers.  Joshua's own default
/// is 8080, but that collides with llama.cpp — so we always pass an explicit
/// `--addr` using this port instead.
pub const DEFAULT_PORT: u16 = 8331;

/// Joshua local inference engine (pure-Rust GGUF server).
pub struct JoshuaEngine;

impl JoshuaEngine {
    fn endpoint(cfg: &EngineConfig) -> String {
        cfg.endpoint.clone().unwrap_or_else(|| {
            let port = cfg.port.unwrap_or(DEFAULT_PORT);
            format!("http://127.0.0.1:{}", port)
        })
    }

    /// Models directory: configured `models_dir` or `~/.rustyclaw/models/joshua`.
    pub fn models_dir(cfg: &EngineConfig) -> PathBuf {
        cfg.models_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(default_models_dir)
    }

    async fn api(endpoint: &str, path: &str) -> Result<String> {
        let url = format!("{}{}", endpoint, path);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let response = client.get(&url).send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await.unwrap_or_default();
            anyhow::bail!("joshua API error ({}): {}", status, error);
        }
        Ok(response.text().await?)
    }

    async fn is_installed() -> bool {
        tokio::process::Command::new("which")
            .arg("joshua")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn is_running(endpoint: &str) -> bool {
        // /health is unauthenticated and cheap — the canonical liveness probe.
        Self::api(endpoint, "/health").await.is_ok()
    }

    /// Model ids reported by a running server (the loaded model's file stem).
    async fn loaded_model_ids(endpoint: &str) -> Vec<String> {
        let Ok(resp) = Self::api(endpoint, "/v1/models").await else {
            return Vec::new();
        };
        serde_json::from_str::<Value>(&resp)
            .ok()
            .and_then(|v| {
                v.get("data")?.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("id")?.as_str().map(String::from))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    async fn sh(script: &str) -> Result<String> {
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .output()
            .await?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() && stdout.is_empty() {
            anyhow::bail!(
                "{}",
                if stderr.is_empty() {
                    "Command failed".to_string()
                } else {
                    stderr
                }
            );
        }
        Ok(if stdout.is_empty() { stderr } else { stdout })
    }

    /// Start `joshua serve` for the given GGUF file.
    async fn spawn_server(cfg: &EngineConfig, model_path: &Path) -> Result<String> {
        let port = cfg.port.unwrap_or(DEFAULT_PORT);
        let mut cmd = format!(
            "nohup joshua serve --model '{}' --addr 127.0.0.1:{}",
            model_path.display(),
            port
        );
        for arg in &cfg.extra_args {
            cmd.push(' ');
            cmd.push_str(arg);
        }
        cmd.push_str(" > /dev/null 2>&1 &");
        let _ = Self::sh(&cmd).await;

        // GGUF loading is mmap-based and fast, but give it a moment.
        let endpoint = Self::endpoint(cfg);
        for _ in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if Self::is_running(&endpoint).await {
                return Ok(format!(
                    "joshua serving {} on {}",
                    model_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    endpoint
                ));
            }
        }
        Ok("joshua start command issued; the model may still be loading.".into())
    }
}

/// Default models directory for Joshua: `~/.rustyclaw/models/joshua`.
pub fn default_models_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".rustyclaw")
        .join("models")
        .join("joshua")
}

/// Scan a directory (recursively, one level of subdirectories) for GGUF files.
pub fn scan_gguf_models(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // HF downloads land in per-repo subdirectories.
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

/// Resolve which GGUF file to serve, in priority order:
/// 1. an explicit `--model <path>` in `extra_args`,
/// 2. the configured `default_model` (matched against file stems),
/// 3. the only GGUF in the models directory (errors if there are several).
pub fn resolve_model_path(cfg: &EngineConfig) -> Result<PathBuf> {
    if let Some(pos) = cfg
        .extra_args
        .iter()
        .position(|a| a == "--model" || a == "-m")
    {
        if let Some(path) = cfg.extra_args.get(pos + 1) {
            return Ok(PathBuf::from(path));
        }
    }

    let dir = JoshuaEngine::models_dir(cfg);
    let models = scan_gguf_models(&dir);
    if models.is_empty() {
        anyhow::bail!(
            "No GGUF models found in {}. Pull one first (e.g. a Hugging Face repo \
             containing a .gguf and tokenizer.json), or set models_dir in the \
             engine config.",
            dir.display()
        );
    }

    if let Some(want) = &cfg.default_model {
        for path in &models {
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string());
            if stem.as_deref() == Some(want.as_str())
                || path
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .as_deref()
                    == Some(want.as_str())
            {
                return Ok(path.clone());
            }
        }
        anyhow::bail!(
            "Configured default_model '{}' not found in {} (available: {})",
            want,
            dir.display(),
            models
                .iter()
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if models.len() == 1 {
        return Ok(models[0].clone());
    }
    anyhow::bail!(
        "Multiple GGUF models in {} — set default_model in the engine config or \
         load one explicitly (available: {})",
        dir.display(),
        models
            .iter()
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Find a GGUF whose file stem or file name matches `model`.
fn find_model_file(dir: &Path, model: &str) -> Option<PathBuf> {
    scan_gguf_models(dir).into_iter().find(|p| {
        p.file_stem().is_some_and(|s| s.to_string_lossy() == model)
            || p.file_name().is_some_and(|s| s.to_string_lossy() == model)
    })
}

#[async_trait::async_trait]
impl LocalEngine for JoshuaEngine {
    fn id(&self) -> &str {
        "joshua"
    }

    fn display_name(&self) -> &str {
        "Joshua"
    }

    fn default_endpoint(&self) -> &str {
        "http://127.0.0.1:8331"
    }

    async fn detect(&self) -> EnginePresence {
        let installed = Self::is_installed().await;
        let version = if installed {
            Self::sh("joshua --version 2>&1 | head -1").await.ok()
        } else {
            None
        };
        let binary_path = if installed {
            Self::sh("which joshua").await.ok()
        } else {
            None
        };
        EnginePresence {
            installed,
            version: version.map(|v| v.trim().to_string()),
            binary_path: binary_path.map(|p| p.trim().to_string()),
        }
    }

    async fn status(&self, cfg: &EngineConfig) -> EngineStatus {
        let presence = self.detect().await;
        let endpoint = Self::endpoint(cfg);
        let available = scan_gguf_models(&Self::models_dir(cfg)).len() as u32;

        let run_status = if Self::is_running(&endpoint).await {
            let loaded = Self::loaded_model_ids(&endpoint).await.len() as u32;
            EngineRunStatus::Running {
                endpoint,
                loaded_models: loaded,
                available_models: available.max(loaded),
            }
        } else {
            EngineRunStatus::Stopped
        };

        EngineStatus {
            presence,
            run_status,
        }
    }

    async fn install(&self, sink: Option<ProgressSink>) -> Result<String> {
        if Self::is_installed().await {
            return Ok("joshua is already installed.".into());
        }
        let has_cargo = Self::sh("which cargo").await.is_ok();
        if !has_cargo {
            anyhow::bail!(
                "Installing joshua requires a Rust toolchain (cargo). Install Rust from \
                 https://rustup.rs, or build joshua manually from \
                 https://github.com/rexlunae/joshua"
            );
        }
        // Stream the full build output (the view bounds the displayed tail).
        crate::engines::stream_shell(
            "cargo install --git https://github.com/rexlunae/joshua joshua 2>&1",
            "joshua",
            sink.as_ref(),
        )
        .await
    }

    async fn start(&self, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        if Self::is_running(&endpoint).await {
            return Ok("joshua is already running.".into());
        }
        let model_path = resolve_model_path(cfg)?;
        Self::spawn_server(cfg, &model_path).await
    }

    async fn stop(&self) -> Result<String> {
        Self::sh("pkill -f 'joshua serve' 2>/dev/null; echo 'stopped'").await
    }

    async fn list_models(&self, cfg: &EngineConfig) -> Result<Vec<LocalModel>> {
        let endpoint = Self::endpoint(cfg);
        let loaded_ids = if Self::is_running(&endpoint).await {
            Self::loaded_model_ids(&endpoint).await
        } else {
            Vec::new()
        };

        let dir = Self::models_dir(cfg);
        let mut models: Vec<LocalModel> = scan_gguf_models(&dir)
            .into_iter()
            .map(|path| {
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let modified_at = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs().to_string());
                LocalModel {
                    loaded: loaded_ids.iter().any(|id| id == &name),
                    name,
                    size_bytes,
                    quantization: parse_quantization(&path),
                    context_length: None,
                    vram_bytes: None,
                    family: None,
                    format: Some("gguf".into()),
                    modified_at,
                }
            })
            .collect();

        // A model may be loaded from outside the models dir (explicit path).
        for id in loaded_ids {
            if !models.iter().any(|m| m.name == id) {
                models.push(LocalModel {
                    name: id,
                    size_bytes: 0,
                    quantization: None,
                    context_length: None,
                    loaded: true,
                    vram_bytes: None,
                    family: None,
                    format: Some("gguf".into()),
                    modified_at: None,
                });
            }
        }

        Ok(models)
    }

    async fn pull(
        &self,
        model: &str,
        cfg: &EngineConfig,
        sink: Option<ProgressSink>,
    ) -> Result<String> {
        // `model` is a Hugging Face repo id (e.g. "Qwen/Qwen3-4B-GGUF").
        // Joshua needs both the .gguf and its tokenizer.json side by side,
        // so download the whole repo into a per-repo subdirectory.
        let dir = Self::models_dir(cfg);
        std::fs::create_dir_all(&dir)?;
        let target = dir.join(model.replace('/', "_"));

        if Self::sh("which huggingface-cli").await.is_err() {
            anyhow::bail!(
                "Pulling models for joshua requires huggingface-cli \
                 (pip install -U 'huggingface_hub[cli]'). Alternatively, place a \
                 .gguf and its tokenizer.json in {} manually.",
                dir.display()
            );
        }

        if let Some(ref tx) = sink {
            let _ = tx
                .send(PullProgress {
                    model: model.to_string(),
                    status: "downloading".into(),
                    percent: 0.0,
                    downloaded_bytes: 0,
                    total_bytes: 0,
                })
                .await;
        }

        let result = Self::sh(&format!(
            "huggingface-cli download '{}' --include '*.gguf' --include 'tokenizer.json' \
             --local-dir '{}' 2>&1 | tail -3",
            model,
            target.display()
        ))
        .await;

        if let Some(ref tx) = sink {
            let _ = tx
                .send(PullProgress {
                    model: model.to_string(),
                    status: if result.is_ok() {
                        "complete".into()
                    } else {
                        "failed".into()
                    },
                    percent: 100.0,
                    downloaded_bytes: 0,
                    total_bytes: 0,
                })
                .await;
        }

        result
    }

    async fn remove(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let dir = Self::models_dir(cfg);
        match find_model_file(&dir, model) {
            Some(path) => {
                std::fs::remove_file(&path)?;
                Ok(format!("Removed {}", path.display()))
            }
            None => anyhow::bail!("Model '{}' not found in {}", model, dir.display()),
        }
    }

    async fn load(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        // One process serves one model: "load" means restart on that model.
        let dir = Self::models_dir(cfg);
        let path = find_model_file(&dir, model)
            .ok_or_else(|| anyhow::anyhow!("Model '{}' not found in {}", model, dir.display()))?;

        let endpoint = Self::endpoint(cfg);
        if Self::is_running(&endpoint).await {
            let already = Self::loaded_model_ids(&endpoint).await;
            if already.iter().any(|id| id == model) {
                return Ok(format!("Model '{}' is already loaded", model));
            }
            let _ = self.stop().await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        Self::spawn_server(cfg, &path).await
    }

    async fn unload(&self, model: &str, _cfg: &EngineConfig) -> Result<String> {
        // One process serves one model: unloading stops the server.
        let _ = self.stop().await;
        Ok(format!("Model '{}' unloaded (joshua stopped)", model))
    }

    fn capabilities(&self) -> EngineCaps {
        EngineCaps::full()
    }
}

/// Extract a quantization tag (e.g. "Q4_K_M") from a GGUF filename.
fn parse_quantization(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().to_uppercase();
    for part in stem.split(['-', '_', '.']) {
        if part.starts_with('Q')
            && part.len() >= 2
            && part[1..2].chars().all(|c| c.is_ascii_digit())
        {
            // Re-find the full token in the original casing (Q4_K_M spans '_').
            if let Some(idx) = stem.find(part) {
                let tail = &stem[idx..];
                let token: String = tail
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                return Some(token);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_parsing() {
        assert_eq!(
            parse_quantization(Path::new("llama-3.2-3b-Q4_K_M.gguf")),
            Some("Q4_K_M".to_string())
        );
        assert_eq!(
            parse_quantization(Path::new("model-q8_0.gguf")),
            Some("Q8_0".to_string())
        );
        assert_eq!(parse_quantization(Path::new("plain-model.gguf")), None);
    }

    #[test]
    fn resolve_model_path_prefers_extra_args() {
        let cfg = EngineConfig {
            extra_args: vec!["--model".into(), "/tmp/foo.gguf".into()],
            ..Default::default()
        };
        assert_eq!(
            resolve_model_path(&cfg).unwrap(),
            PathBuf::from("/tmp/foo.gguf")
        );
    }

    #[test]
    fn resolve_model_path_scans_dir() {
        let dir = std::env::temp_dir().join(format!("joshua-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = EngineConfig {
            models_dir: Some(dir.to_string_lossy().to_string()),
            ..Default::default()
        };

        // Empty dir → helpful error.
        assert!(resolve_model_path(&cfg).is_err());

        // One model → picked automatically.
        std::fs::write(dir.join("tiny-Q4_0.gguf"), b"x").unwrap();
        assert_eq!(
            resolve_model_path(&cfg).unwrap(),
            dir.join("tiny-Q4_0.gguf")
        );

        // Two models, no default → error listing both.
        std::fs::write(dir.join("big-Q8_0.gguf"), b"x").unwrap();
        let err = resolve_model_path(&cfg).unwrap_err().to_string();
        assert!(err.contains("tiny-Q4_0") && err.contains("big-Q8_0"));

        // default_model selects by stem.
        let cfg_with_default = EngineConfig {
            default_model: Some("big-Q8_0".into()),
            ..cfg.clone()
        };
        assert_eq!(
            resolve_model_path(&cfg_with_default).unwrap(),
            dir.join("big-Q8_0.gguf")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
