//! llama.cpp (`llama-server`) engine implementation.
//!
//! Supports the new Ollama-style model management endpoints in llama.cpp:
//! - `/v1/models` for discovery
//! - `/models/load` and `/models/unload` for hot-swapping
//! - `-hf user/model[:quant]` for downloading from Hugging Face

use super::*;
use anyhow::Result;
use serde_json::Value;

/// llama.cpp local inference engine.
pub struct LlamaCppEngine;

impl LlamaCppEngine {
    fn endpoint(cfg: &EngineConfig) -> String {
        cfg.endpoint.clone().unwrap_or_else(|| {
            let port = cfg.port.unwrap_or(8080);
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
            _ => anyhow::bail!("Unsupported method: {}", method),
        };

        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await.unwrap_or_default();
            anyhow::bail!("llama-server API error ({}): {}", status, error);
        }
        Ok(response.text().await?)
    }

    async fn is_installed() -> bool {
        tokio::process::Command::new("which")
            .arg("llama-server")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn is_running(endpoint: &str) -> bool {
        Self::api(endpoint, "GET", "/health", None).await.is_ok()
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
impl LocalEngine for LlamaCppEngine {
    fn id(&self) -> &str {
        "llamacpp"
    }

    fn display_name(&self) -> &str {
        "llama.cpp"
    }

    fn default_endpoint(&self) -> &str {
        "http://127.0.0.1:8080"
    }

    async fn detect(&self) -> EnginePresence {
        let installed = Self::is_installed().await;
        let version = if installed {
            Self::sh("llama-server --version 2>&1 | head -1").await.ok()
        } else {
            None
        };
        let binary_path = if installed {
            Self::sh("which llama-server").await.ok()
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
            let available = match Self::api(&endpoint, "GET", "/v1/models", None).await {
                Ok(resp) => serde_json::from_str::<Value>(&resp)
                    .ok()
                    .and_then(|v| v.get("data")?.as_array().map(|a| a.len() as u32))
                    .unwrap_or(0),
                Err(_) => 0,
            };
            EngineRunStatus::Running {
                endpoint,
                loaded_models: available, // llama-server only shows loaded models
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
            return Ok("llama-server is already installed.".into());
        }
        let os = std::env::consts::OS;
        match os {
            "macos" => Self::sh("brew install llama.cpp 2>&1").await,
            "linux" => {
                // Download prebuilt from GitHub releases
                let arch = std::env::consts::ARCH;
                let triple = match arch {
                    "x86_64" => "ubuntu-x64",
                    "aarch64" => "ubuntu-arm64",
                    _ => anyhow::bail!("Unsupported architecture: {}", arch),
                };
                Self::sh(&format!(
                    concat!(
                        "LATEST=$(curl -sL https://api.github.com/repos/ggml-org/llama.cpp/releases/latest | grep tag_name | cut -d'\"' -f4) && ",
                        "curl -L -o /tmp/llama-server.zip \"https://github.com/ggml-org/llama.cpp/releases/download/${{LATEST}}/llama-${{LATEST}}-bin-{}.zip\" && ",
                        "unzip -o /tmp/llama-server.zip -d /tmp/llama-cpp && ",
                        "sudo cp /tmp/llama-cpp/*/bin/llama-server /usr/local/bin/ && ",
                        "chmod +x /usr/local/bin/llama-server && ",
                        "rm -rf /tmp/llama-server.zip /tmp/llama-cpp && ",
                        "echo 'llama-server installed to /usr/local/bin/'"
                    ),
                    triple
                ))
                .await
            }
            _ => anyhow::bail!(
                "Unsupported OS: {}. Install llama-server manually from https://github.com/ggml-org/llama.cpp",
                os
            ),
        }
    }

    async fn start(&self, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        if Self::is_running(&endpoint).await {
            return Ok("llama-server is already running.".into());
        }
        let port = cfg.port.unwrap_or(8080);
        let mut cmd = format!("nohup llama-server --port {}", port);
        if let Some(ref dir) = cfg.models_dir {
            cmd.push_str(&format!(" --models-dir '{}'", dir));
        }
        for arg in &cfg.extra_args {
            cmd.push(' ');
            cmd.push_str(arg);
        }
        cmd.push_str(" > /dev/null 2>&1 &");
        let _ = Self::sh(&cmd).await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if Self::is_running(&endpoint).await {
            Ok("llama-server started.".into())
        } else {
            Ok("llama-server start command issued; may take a moment.".into())
        }
    }

    async fn stop(&self) -> Result<String> {
        Self::sh("pkill -f 'llama-server' 2>/dev/null; echo 'stopped'").await
    }

    async fn list_models(&self, cfg: &EngineConfig) -> Result<Vec<LocalModel>> {
        let endpoint = Self::endpoint(cfg);
        let resp = Self::api(&endpoint, "GET", "/v1/models", None).await?;
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
                    loaded: true, // if listed by llama-server, it's loaded
                    vram_bytes: None,
                    family: None,
                    format: Some("gguf".into()),
                    modified_at: None,
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
        // llama.cpp pulls via CLI: llama-server -hf user/model[:quant]
        // For a standalone pull, use the huggingface-cli or llama-cli
        let models_dir = cfg.models_dir.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("llama.cpp")
                .to_string_lossy()
                .to_string()
        });

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

        // Use huggingface-cli if available, otherwise curl
        let result = Self::sh(&format!(
            "huggingface-cli download {} --local-dir '{}' 2>&1 || curl -L -o '{}/{}' 'https://huggingface.co/{}/resolve/main/*.gguf' 2>&1",
            model, models_dir, models_dir, model.replace('/', "_"), model
        ))
        .await;

        if let Some(ref tx) = sink {
            let _ = tx
                .send(PullProgress {
                    model: model.to_string(),
                    status: "complete".into(),
                    percent: 100.0,
                    downloaded_bytes: 0,
                    total_bytes: 0,
                })
                .await;
        }

        result
    }

    async fn remove(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let models_dir = cfg.models_dir.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("llama.cpp")
                .to_string_lossy()
                .to_string()
        });
        Self::sh(&format!("rm -f '{}/{}' 2>&1", models_dir, model)).await
    }

    async fn load(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);

        // P6: Extract per-model knobs from extra_args.
        let mut ctx_size: Option<u32> = None;
        let mut i = 0;
        while i < cfg.extra_args.len() {
            if cfg.extra_args[i] == "--ctx-size" {
                if let Some(val) = cfg.extra_args.get(i + 1) {
                    ctx_size = val.parse().ok();
                }
                i += 2;
            } else {
                i += 1;
            }
        }

        let mut body = serde_json::json!({ "model": model });
        if let Some(n) = ctx_size {
            body["n_ctx"] = serde_json::json!(n);
        }

        Self::api(&endpoint, "POST", "/models/load", Some(&body)).await?;
        Ok(format!("Model '{}' loaded", model))
    }

    async fn unload(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        let body = serde_json::json!({ "model": model });
        Self::api(&endpoint, "POST", "/models/unload", Some(&body)).await?;
        Ok(format!("Model '{}' unloaded", model))
    }

    fn capabilities(&self) -> EngineCaps {
        EngineCaps::full()
    }
}
