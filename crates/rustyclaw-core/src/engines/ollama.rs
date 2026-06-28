//! Ollama engine implementation.
//!
//! Wraps the existing `tools/ollama.rs` logic behind the [`LocalEngine`] trait.

use super::*;
use anyhow::Result;
use serde_json::Value;

/// Ollama local inference engine.
pub struct OllamaEngine;

impl OllamaEngine {
    fn endpoint(cfg: &EngineConfig) -> String {
        cfg.endpoint.clone().unwrap_or_else(|| {
            let port = cfg.port.unwrap_or(11434);
            format!("http://127.0.0.1:{}", port)
        })
    }

    async fn api(endpoint: &str, method: &str, path: &str, body: Option<&Value>) -> Result<String> {
        let url = format!("{}{}", endpoint, path);
        let client = reqwest::Client::new();
        let request = match method {
            "GET" => client.get(&url),
            "POST" => {
                let mut req = client.post(&url);
                if let Some(b) = body {
                    req = req.header("Content-Type", "application/json").json(b);
                }
                req
            }
            "DELETE" => client.delete(&url),
            _ => anyhow::bail!("Unsupported method: {}", method),
        };

        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error ({}): {}", status, error);
        }
        Ok(response.text().await?)
    }

    async fn is_installed() -> bool {
        tokio::process::Command::new("which")
            .arg("ollama")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn is_running(endpoint: &str) -> bool {
        Self::api(endpoint, "GET", "/api/tags", None).await.is_ok()
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
}

#[async_trait::async_trait]
impl LocalEngine for OllamaEngine {
    fn id(&self) -> &str {
        "ollama"
    }

    fn display_name(&self) -> &str {
        "Ollama"
    }

    fn default_endpoint(&self) -> &str {
        "http://127.0.0.1:11434"
    }

    async fn detect(&self) -> EnginePresence {
        let installed = Self::is_installed().await;
        let version = if installed {
            Self::sh("ollama --version").await.ok()
        } else {
            None
        };
        let binary_path = if installed {
            Self::sh("which ollama").await.ok()
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

        let run_status = if !presence.installed {
            EngineRunStatus::Stopped
        } else if Self::is_running(&endpoint).await {
            let (available, loaded) = match Self::api(&endpoint, "GET", "/api/tags", None).await {
                Ok(resp) => {
                    let available = serde_json::from_str::<Value>(&resp)
                        .ok()
                        .and_then(|v| v.get("models")?.as_array().map(|a| a.len() as u32))
                        .unwrap_or(0);
                    let loaded = match Self::api(&endpoint, "GET", "/api/ps", None).await {
                        Ok(ps) => serde_json::from_str::<Value>(&ps)
                            .ok()
                            .and_then(|v| v.get("models")?.as_array().map(|a| a.len() as u32))
                            .unwrap_or(0),
                        Err(_) => 0,
                    };
                    (available, loaded)
                }
                Err(_) => (0, 0),
            };
            EngineRunStatus::Running {
                endpoint,
                loaded_models: loaded,
                available_models: available,
            }
        } else {
            EngineRunStatus::Stopped
        };

        EngineStatus {
            presence,
            run_status,
        }
    }

    async fn install(&self, _sink: Option<ProgressSink>) -> Result<String> {
        if Self::is_installed().await {
            return Ok("Ollama is already installed.".into());
        }
        let os = std::env::consts::OS;
        match os {
            "macos" => Self::sh("brew install ollama 2>&1").await,
            "linux" => Self::sh("curl -fsSL https://ollama.com/install.sh | sh 2>&1").await,
            _ => anyhow::bail!(
                "Unsupported OS for automatic install: {}. Visit https://ollama.com/download",
                os
            ),
        }
    }

    async fn start(&self, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        if Self::is_running(&endpoint).await {
            return Ok("Ollama server is already running.".into());
        }
        let os = std::env::consts::OS;
        match os {
            "macos" => {
                let _ = Self::sh("brew services start ollama 2>/dev/null || nohup ollama serve > /dev/null 2>&1 &").await;
            }
            _ => {
                let _ = Self::sh("nohup ollama serve > /dev/null 2>&1 &").await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if Self::is_running(&endpoint).await {
            Ok("Ollama server started.".into())
        } else {
            Ok("Ollama serve command issued; may take a moment to start.".into())
        }
    }

    async fn stop(&self) -> Result<String> {
        let os = std::env::consts::OS;
        match os {
            "macos" => Self::sh("brew services stop ollama 2>/dev/null; pkill -f 'ollama serve' 2>/dev/null; echo 'stopped'").await,
            _ => Self::sh("pkill -f 'ollama serve' 2>/dev/null; echo 'stopped'").await,
        }
    }

    async fn list_models(&self, cfg: &EngineConfig) -> Result<Vec<LocalModel>> {
        let endpoint = Self::endpoint(cfg);
        let resp = Self::api(&endpoint, "GET", "/api/tags", None).await?;
        let parsed: Value = serde_json::from_str(&resp)?;
        let models = parsed
            .get("models")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(models
            .iter()
            .map(|m| {
                let name = m
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("?")
                    .to_string();
                let size_bytes = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                let family = m
                    .get("details")
                    .and_then(|d| d.get("family"))
                    .and_then(|f| f.as_str())
                    .map(|s| s.to_string());
                let quantization = m
                    .get("details")
                    .and_then(|d| d.get("quantization_level"))
                    .and_then(|q| q.as_str())
                    .map(|s| s.to_string());
                let modified_at = m
                    .get("modified_at")
                    .and_then(|d| d.as_str())
                    .map(|s| s.to_string());
                LocalModel {
                    name,
                    size_bytes,
                    quantization,
                    context_length: None,
                    loaded: false,
                    vram_bytes: None,
                    family,
                    format: Some("gguf".into()),
                    modified_at,
                }
            })
            .collect())
    }

    async fn pull(
        &self,
        model: &str,
        cfg: &EngineConfig,
        sink: Option<ProgressSink>,
    ) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        let url = format!("{}/api/pull", endpoint);
        let client = reqwest::Client::new();
        let body = serde_json::json!({ "name": model, "stream": true });

        let response = client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Pull failed: {}", response.text().await?);
        }

        // Stream NDJSON progress
        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;
        let mut last_status = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);
            for line in text.lines() {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    let status = v
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let total = v.get("total").and_then(|t| t.as_u64()).unwrap_or(0);
                    let completed = v.get("completed").and_then(|c| c.as_u64()).unwrap_or(0);
                    let pct = if total > 0 {
                        (completed as f32 / total as f32) * 100.0
                    } else {
                        0.0
                    };
                    last_status = status.clone();
                    if let Some(ref tx) = sink {
                        let _ = tx
                            .send(PullProgress {
                                model: model.to_string(),
                                status,
                                percent: pct,
                                downloaded_bytes: completed,
                                total_bytes: total,
                            })
                            .await;
                    }
                }
            }
        }
        Ok(format!("Pull complete: {} ({})", model, last_status))
    }

    async fn remove(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        let body = serde_json::json!({ "name": model });
        Self::api(&endpoint, "DELETE", "/api/delete", Some(&body)).await?;
        Ok(format!("Removed model '{}'", model))
    }

    async fn load(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);

        // P6: Extract per-model knobs from extra_args.
        let mut options = serde_json::Map::new();
        for arg in &cfg.extra_args {
            if let Some(val) = arg.strip_prefix("--num-ctx=") {
                if let Ok(n) = val.parse::<u32>() {
                    options.insert("num_ctx".to_string(), serde_json::Value::Number(n.into()));
                }
            }
        }

        let mut body = serde_json::json!({
            "model": model,
            "prompt": "",
            "keep_alive": "10m"
        });
        if !options.is_empty() {
            body["options"] = serde_json::Value::Object(options);
        }

        Self::api(&endpoint, "POST", "/api/generate", Some(&body)).await?;
        Ok(format!("Model '{}' loaded (keep_alive: 10m)", model))
    }

    async fn unload(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        let body = serde_json::json!({
            "model": model,
            "prompt": "",
            "keep_alive": 0
        });
        Self::api(&endpoint, "POST", "/api/generate", Some(&body)).await?;
        Ok(format!("Model '{}' unloaded", model))
    }

    fn capabilities(&self) -> EngineCaps {
        EngineCaps::full()
    }
}
