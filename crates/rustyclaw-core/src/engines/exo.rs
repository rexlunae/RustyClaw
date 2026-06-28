//! Exo engine implementation.
//!
//! Wraps the existing `tools/exo_ai/` logic behind the [`LocalEngine`] trait.

use super::*;
use anyhow::Result;
use serde_json::Value;

/// Exo distributed inference engine.
pub struct ExoEngine;

impl ExoEngine {
    fn endpoint(cfg: &EngineConfig) -> String {
        cfg.endpoint.clone().unwrap_or_else(|| {
            let port = cfg.port.unwrap_or(52415);
            format!("http://127.0.0.1:{}", port)
        })
    }

    async fn api(endpoint: &str, path: &str) -> Result<String> {
        let url = format!("{}{}", endpoint, path);
        let resp = reqwest::get(&url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("Exo API error ({})", resp.status());
        }
        Ok(resp.text().await?)
    }

    async fn is_installed() -> bool {
        tokio::process::Command::new("which")
            .arg("exo")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn is_running(endpoint: &str) -> bool {
        Self::api(endpoint, "/v1/models").await.is_ok()
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
impl LocalEngine for ExoEngine {
    fn id(&self) -> &str {
        "exo"
    }

    fn display_name(&self) -> &str {
        "Exo"
    }

    fn default_endpoint(&self) -> &str {
        "http://127.0.0.1:52415"
    }

    async fn detect(&self) -> EnginePresence {
        let installed = Self::is_installed().await;
        let version = if installed {
            Self::sh("exo --version 2>/dev/null || echo unknown")
                .await
                .ok()
        } else {
            None
        };
        let binary_path = if installed {
            Self::sh("which exo").await.ok()
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
            let available = match Self::api(&endpoint, "/v1/models").await {
                Ok(resp) => serde_json::from_str::<Value>(&resp)
                    .ok()
                    .and_then(|v| v.get("data")?.as_array().map(|a| a.len() as u32))
                    .unwrap_or(0),
                Err(_) => 0,
            };
            EngineRunStatus::Running {
                endpoint,
                loaded_models: 0,
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
        // Exo requires uv + git clone; delegate to the existing tool logic
        Self::sh(
            "pip install exo 2>&1 || (curl -LsSf https://astral.sh/uv/install.sh | sh && uv pip install exo 2>&1)",
        )
        .await
    }

    async fn start(&self, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        if Self::is_running(&endpoint).await {
            return Ok("Exo is already running.".into());
        }
        let port = cfg.port.unwrap_or(52415);
        let _ = Self::sh(&format!(
            "nohup exo --chatgpt-api-port {} > /dev/null 2>&1 &",
            port
        ))
        .await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if Self::is_running(&endpoint).await {
            Ok("Exo started.".into())
        } else {
            Ok("Exo start command issued; may take a moment.".into())
        }
    }

    async fn stop(&self) -> Result<String> {
        Self::sh("pkill -f 'exo' 2>/dev/null; echo 'stopped'").await
    }

    async fn list_models(&self, cfg: &EngineConfig) -> Result<Vec<LocalModel>> {
        let endpoint = Self::endpoint(cfg);
        let resp = Self::api(&endpoint, "/v1/models").await?;
        let parsed: Value = serde_json::from_str(&resp)?;
        let models = parsed
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(models
            .iter()
            .map(|m| {
                let name = m
                    .get("id")
                    .and_then(|n| n.as_str())
                    .unwrap_or("?")
                    .to_string();
                LocalModel {
                    name,
                    size_bytes: 0,
                    quantization: None,
                    context_length: None,
                    loaded: false,
                    vram_bytes: None,
                    family: None,
                    format: None,
                    modified_at: None,
                }
            })
            .collect())
    }

    async fn pull(
        &self,
        _model: &str,
        _cfg: &EngineConfig,
        _sink: Option<ProgressSink>,
    ) -> Result<String> {
        anyhow::bail!("Exo does not support pull from the API; use its own download interface")
    }

    async fn remove(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("Exo does not support model removal from the API")
    }

    async fn load(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1
        });
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/chat/completions", endpoint))
            .json(&body)
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(format!("Model '{}' loaded via warmup request", model))
        } else {
            anyhow::bail!("Failed to load '{}': {}", model, resp.status())
        }
    }

    async fn unload(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("Exo does not support explicit model unload")
    }

    fn capabilities(&self) -> EngineCaps {
        EngineCaps::lifecycle_only()
    }
}
