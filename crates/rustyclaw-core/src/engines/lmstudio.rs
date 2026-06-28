//! LM Studio engine implementation (read-only).
//!
//! LM Studio manages its own lifecycle — we only detect status and list models.

use super::*;
use anyhow::Result;
use serde_json::Value;

/// LM Studio local inference engine (read-only integration).
pub struct LmStudioEngine;

impl LmStudioEngine {
    fn endpoint(cfg: &EngineConfig) -> String {
        cfg.endpoint.clone().unwrap_or_else(|| {
            let port = cfg.port.unwrap_or(1234);
            format!("http://127.0.0.1:{}", port)
        })
    }

    async fn api(endpoint: &str, path: &str) -> Result<String> {
        let url = format!("{}{}", endpoint, path);
        let resp = reqwest::get(&url).await?;
        if !resp.status().is_success() {
            anyhow::bail!("LM Studio API error ({})", resp.status());
        }
        Ok(resp.text().await?)
    }

    async fn is_running(endpoint: &str) -> bool {
        Self::api(endpoint, "/v1/models").await.is_ok()
    }
}

#[async_trait::async_trait]
impl LocalEngine for LmStudioEngine {
    fn id(&self) -> &str {
        "lmstudio"
    }

    fn display_name(&self) -> &str {
        "LM Studio"
    }

    fn default_endpoint(&self) -> &str {
        "http://127.0.0.1:1234"
    }

    async fn detect(&self) -> EnginePresence {
        // LM Studio is a GUI app — we can only detect if it's running
        EnginePresence {
            installed: true, // Assume installed if user configured it
            version: None,
            binary_path: None,
        }
    }

    async fn status(&self, cfg: &EngineConfig) -> EngineStatus {
        let presence = self.detect().await;
        let endpoint = Self::endpoint(cfg);

        let run_status = if Self::is_running(&endpoint).await {
            let available = match Self::api(&endpoint, "/v1/models").await {
                Ok(resp) => serde_json::from_str::<Value>(&resp)
                    .ok()
                    .and_then(|v| v.get("data")?.as_array().map(|a| a.len() as u32))
                    .unwrap_or(0),
                Err(_) => 0,
            };
            EngineRunStatus::Running {
                endpoint,
                loaded_models: available,
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
        anyhow::bail!("LM Studio must be installed manually from https://lmstudio.ai")
    }

    async fn start(&self, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("LM Studio manages its own lifecycle; start it from the app")
    }

    async fn stop(&self) -> Result<String> {
        anyhow::bail!("LM Studio manages its own lifecycle; stop it from the app")
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
                    loaded: true,
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
        anyhow::bail!("LM Studio manages model downloads through its own interface")
    }

    async fn remove(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("LM Studio manages model removal through its own interface")
    }

    async fn load(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("LM Studio manages model loading through its own interface")
    }

    async fn unload(&self, _model: &str, _cfg: &EngineConfig) -> Result<String> {
        anyhow::bail!("LM Studio manages model unloading through its own interface")
    }

    fn capabilities(&self) -> EngineCaps {
        EngineCaps::read_only()
    }
}
