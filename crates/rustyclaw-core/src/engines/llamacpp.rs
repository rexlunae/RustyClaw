//! llama.cpp (`llama-server`) engine implementation.
//!
//! Supports the new Ollama-style model management endpoints in llama.cpp:
//! - `/v1/models` for discovery
//! - `/models/load` and `/models/unload` for hot-swapping
//! - `-hf user/model[:quant]` for downloading from Hugging Face

use super::*;
use crate::ignore::Ignore;
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

    /// `(model name, port)` for every `llama-server` process running on the
    /// host — including ones started manually outside RustyClaw.  Best-effort
    /// (Linux); empty elsewhere.
    async fn running_servers() -> Vec<(String, Option<u16>)> {
        let lines = crate::engines::running_server_cmdlines("llama-server").await;
        crate::engines::parse_server_cmdlines(&lines, &["--model", "-m"], &["--port"], 8080)
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
        let configured_port = cfg.port.unwrap_or(8080);
        let detected = Self::running_servers().await;

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
        } else if let Some((_, port)) = detected
            .iter()
            .find(|(_, port)| *port == Some(configured_port))
        {
            // A llama-server on the engine's own configured port is running
            // (e.g. started manually outside RustyClaw): report it so the
            // UI's lifecycle gating stays tied to this engine's server.
            let detected_endpoint = port
                .map(|port| format!("http://127.0.0.1:{}", port))
                .unwrap_or(endpoint);
            let loaded = detected.len() as u32;
            EngineRunStatus::Running {
                endpoint: detected_endpoint,
                loaded_models: loaded,
                available_models: loaded,
            }
        } else {
            // llama-server processes on *other* ports belong to someone
            // else: this engine is not running, and reporting Running would
            // hide the Start button while Stop could not touch that server.
            EngineRunStatus::Stopped
        };

        EngineStatus {
            presence,
            run_status,
        }
    }

    async fn install(&self, sink: Option<ProgressSink>) -> Result<String> {
        if Self::is_installed().await {
            return Ok("llama-server is already installed.".into());
        }
        let os = std::env::consts::OS;
        let script = match os {
            "macos" => "brew install llama.cpp 2>&1".to_string(),
            "linux" => {
                // Download prebuilt from GitHub releases
                let arch = std::env::consts::ARCH;
                let triple = match arch {
                    "x86_64" => "ubuntu-x64",
                    "aarch64" => "ubuntu-arm64",
                    _ => anyhow::bail!("Unsupported architecture: {}", arch),
                };
                format!(
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
                )
            }
            _ => anyhow::bail!(
                "Unsupported OS: {}. Install llama-server manually from https://github.com/ggml-org/llama.cpp",
                os
            ),
        };
        crate::engines::stream_shell(&script, "llamacpp", sink.as_ref()).await
    }

    async fn start(&self, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);
        if Self::is_running(&endpoint).await {
            return Ok("llama-server is already running.".into());
        }
        let port = cfg.port.unwrap_or(8080);
        let mut cmd = format!("nohup llama-server --port {}", port);
        if let Some(ref dir) = cfg.models_dir {
            cmd.push_str(&format!(" --models-dir {}", sh_quote(dir)));
        }
        // The typed context window applies to manual starts too (it already
        // applies to auto-start via `engine_start_command`).
        if let Some(ctx) = cfg.context_length {
            cmd.push_str(&format!(" --ctx-size {}", ctx));
        }
        for arg in &cfg.extra_args {
            cmd.push(' ');
            // extra_args are client-supplied strings reaching a `sh -c`
            // command line — quote them so metacharacters stay inert.
            cmd.push_str(&crate::engines::sh_quote(arg));
        }
        cmd.push_str(" > /dev/null 2>&1 &");
        Self::sh(&cmd).await.ignore();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if Self::is_running(&endpoint).await {
            Ok("llama-server started.".into())
        } else {
            Ok("llama-server start command issued; may take a moment.".into())
        }
    }

    /// `cfg` is only read inside the Linux block (process inspection); the
    /// non-Linux fallback ignores it.
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    async fn stop(&self, cfg: &EngineConfig) -> Result<String> {
        // Scoped to the configured port: `pkill -f 'llama-server'` would
        // also kill servers started manually on other ports.  Report what
        // actually happened instead of always claiming success.  The pattern
        // terminates the digit run (`( |$)`), so a short port such as 1234
        // cannot match a server running on 12345.
        #[cfg(target_os = "linux")]
        {
            let port = cfg.port.unwrap_or(8080);
            let pattern = format!("llama-server .*--port {}( |$)", port);
            let running = crate::engines::running_server_cmdlines(&pattern)
                .await
                .iter()
                .any(|line| line.contains("llama-server"));
            if !running {
                return Ok(format!(
                    "no llama-server is running on port {} (nothing to stop)",
                    port
                ));
            }
            return Self::sh(&format!(
                "pkill -f '{}' 2>/dev/null; echo 'stopped'",
                pattern
            ))
            .await;
        }
        // No process inspection on this platform: fall back to stopping
        // every llama-server rather than claiming success while one runs.
        #[cfg(not(target_os = "linux"))]
        {
            Self::sh("pkill -f 'llama-server' 2>/dev/null; echo 'stopped'").await
        }
    }

    async fn list_models(&self, cfg: &EngineConfig) -> Result<Vec<LocalModel>> {
        // A running llama-server reports only the model it is serving.  For
        // "what is available locally" we merge in every GGUF in the models
        // directory (configured `models_dir`, or the llama.cpp cache dir),
        // so models on disk show up even before the server is started.
        let mut names: Vec<String> = Vec::new();
        let mut loaded: Vec<String> = Vec::new();
        let endpoint = Self::endpoint(cfg);
        if let Ok(resp) = Self::api(&endpoint, "GET", "/v1/models", None).await {
            if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                if let Some(arr) = parsed.get("data").and_then(|d| d.as_array()) {
                    for m in arr {
                        if let Some(id) = m.get("id").and_then(|n| n.as_str()) {
                            names.push(id.to_string());
                            loaded.push(id.to_string());
                        }
                    }
                }
            }
        }
        // Mark models served by running llama-server processes (even ones
        // started outside RustyClaw) as loaded.
        for (name, _) in Self::running_servers().await {
            if !loaded.iter().any(|l| l == &name) {
                loaded.push(name);
            }
        }

        let models_dir = cfg.models_dir.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("llama.cpp")
                .to_string_lossy()
                .to_string()
        });
        // (name, path) for every GGUF on disk, deduped against the API list.
        let mut on_disk: Vec<(String, std::path::PathBuf)> = Vec::new();
        for path in crate::engines::scan_gguf_models(std::path::Path::new(&models_dir)) {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            if !names.iter().any(|n| n == &name) {
                names.push(name.clone());
                on_disk.push((name, path));
            }
        }
        // A model served by a running llama-server may live outside the
        // scanned models dir (e.g. started manually with an explicit
        // `--model /elsewhere/foo.gguf` while the API is unreachable from
        // the configured endpoint).  It is loaded, so it must appear in the
        // list — like Joshua's list_models, surface any loaded id that the
        // scan did not produce.
        for name in &loaded {
            if !names.iter().any(|n| n == name) {
                names.push(name.clone());
            }
        }

        Ok(names
            .into_iter()
            .map(|name| {
                let is_loaded = loaded.iter().any(|l| l == &name);
                let (size_bytes, modified_at) = on_disk
                    .iter()
                    .find(|(n, _)| n == &name)
                    .and_then(|(_, p)| std::fs::metadata(p).ok())
                    .map(|m| (m.len(), m.modified().ok()))
                    .unwrap_or((0, None));
                let modified_at = modified_at
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs().to_string());
                LocalModel {
                    name,
                    size_bytes,
                    quantization: None,
                    context_length: None,
                    loaded: is_loaded,
                    vram_bytes: None,
                    family: None,
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
        // llama.cpp pulls via CLI: llama-server -hf user/model[:quant]
        // For a standalone pull, use the huggingface-cli or llama-cli
        let models_dir = cfg.models_dir.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("llama.cpp")
                .to_string_lossy()
                .to_string()
        });

        // Use the Hugging Face CLI (`hf` or legacy `huggingface-cli`).
        let Some(hf) = crate::engines::downloaders::hf_cli().await else {
            anyhow::bail!("{}", crate::engines::downloaders::HF_CLI_MISSING_HINT);
        };

        // Fuzzy names are resolved to a repo id by searching the Hub.
        let (repo, note) = crate::engines::hub::resolve_for_pull(model, true).await?;
        let model = repo.as_str();

        if let Some(ref tx) = sink {
            tx.send(PullProgress {
                model: model.to_string(),
                status: note.unwrap_or_else(|| "downloading".into()),
                percent: 0.0,
                downloaded_bytes: 0,
                total_bytes: 0,
            })
            .await
            .ignore();
        }

        let result = Self::sh(&format!(
            "{} download {} --local-dir {} 2>&1",
            hf,
            sh_quote(model),
            sh_quote(&models_dir)
        ))
        .await;

        if let Some(ref tx) = sink {
            tx.send(PullProgress {
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
            .await
            .ignore();
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
        // `list_models` names on-disk GGUFs by their file stem (no .gguf
        // extension, possibly inside a per-repo subdirectory), so resolve
        // the name back to the actual scanned path before deleting — a bare
        // `rm -f {dir}/{model}` would point at nothing and silently report
        // success while the file stays on disk.
        let dir = std::path::Path::new(&models_dir);
        let matched = crate::engines::scan_gguf_models(dir).into_iter().find(|p| {
            p.file_stem().is_some_and(|s| s.to_string_lossy() == model)
                || p.file_name().is_some_and(|s| s.to_string_lossy() == model)
        });
        let Some(path) = matched else {
            anyhow::bail!(
                "Model '{}' not found in {} (available: {})",
                model,
                models_dir,
                crate::engines::scan_gguf_models(dir)
                    .iter()
                    .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        };
        std::fs::remove_file(&path)?;
        Ok(format!("Removed {}", path.display()))
    }

    async fn load(&self, model: &str, cfg: &EngineConfig) -> Result<String> {
        let endpoint = Self::endpoint(cfg);

        // Per-model knobs: an explicit `--ctx-size` in extra_args (the
        // gateway's per-load override rides here) wins; otherwise the
        // persisted typed context window applies.
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
        if ctx_size.is_none() {
            ctx_size = cfg.context_length;
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
