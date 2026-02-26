// Ollama administration tools for RustyClaw.
//
// Provides full model lifecycle management via the ollama CLI and REST API:
// setup/install, pull/remove models, list/show/load/unload, running model
// status, and serve control.
//
// Provides both sync and async implementations.

use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

// ── Async implementations ───────────────────────────────────────────────────

/// `ollama_manage` — unified Ollama administration tool (async).
#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_ollama_manage_async(
    args: &Value,
    _workspace_dir: &Path,
) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    tracing::Span::current().record("action", action);
    debug!("Executing ollama_manage");

    match action {
        "setup" | "install" => {
            if is_ollama_installed_async().await {
                let version = sh_async("ollama --version")
                    .await
                    .unwrap_or_else(|_| "unknown".into());
                let running = if is_ollama_running_async().await {
                    "running"
                } else {
                    "stopped"
                };
                return Ok(format!(
                    "Ollama is already installed ({}). Server status: {}.",
                    version.trim(),
                    running,
                ));
            }
            let os = std::env::consts::OS;
            let install_result = match os {
                "macos" => sh_async("brew install ollama 2>&1").await,
                "linux" => sh_async("curl -fsSL https://ollama.com/install.sh | sh 2>&1").await,
                _ => Err(format!(
                    "Unsupported OS for automatic install: {}. Visit https://ollama.com/download",
                    os
                )),
            };
            match install_result {
                Ok(out) => Ok(format!("Ollama installed successfully.\n{}", out)),
                Err(e) => Err(format!("Failed to install Ollama: {}", e)),
            }
        }

        "serve" | "start" => {
            if !is_ollama_installed_async().await {
                return Err("Ollama is not installed. Run with action 'setup' first.".into());
            }
            if is_ollama_running_async().await {
                return Ok("Ollama server is already running.".into());
            }
            let os = std::env::consts::OS;
            match os {
                "macos" => {
                    let _ = sh_async("brew services start ollama 2>/dev/null || nohup ollama serve > /dev/null 2>&1 &").await;
                }
                _ => {
                    let _ = sh_async("nohup ollama serve > /dev/null 2>&1 &").await;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if is_ollama_running_async().await {
                Ok("Ollama server started.".into())
            } else {
                Ok("Ollama serve command issued. It may take a moment to start.".into())
            }
        }

        "stop" => {
            if !is_ollama_running_async().await {
                return Ok("Ollama server is not running.".into());
            }
            let os = std::env::consts::OS;
            match os {
                "macos" => sh_async("brew services stop ollama 2>/dev/null; pkill -f 'ollama serve' 2>/dev/null; echo 'Ollama server stopped.'").await,
                _ => sh_async("pkill -f 'ollama serve' 2>/dev/null; echo 'Ollama server stopped.'").await,
            }
        }

        "status" => {
            let installed = is_ollama_installed_async().await;
            let running = is_ollama_running_async().await;
            let version = if installed {
                sh_async("ollama --version")
                    .await
                    .unwrap_or_else(|_| "unknown".into())
            } else {
                "not installed".into()
            };
            let models = if running {
                match ollama_api_async("GET", "/api/tags", None).await {
                    Ok(resp) => {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                            let count = parsed
                                .get("models")
                                .and_then(|m| m.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            format!("{} model(s) available", count)
                        } else {
                            "unable to parse".into()
                        }
                    }
                    Err(_) => "unable to query".into(),
                }
            } else {
                "server not running".into()
            };
            let loaded = if running {
                match ollama_api_async("GET", "/api/ps", None).await {
                    Ok(resp) => {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                            let ms = parsed.get("models").and_then(|m| m.as_array());
                            match ms {
                                Some(arr) if !arr.is_empty() => {
                                    let names: Vec<String> = arr
                                        .iter()
                                        .filter_map(|m| {
                                            m.get("name")
                                                .and_then(|n| n.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        .collect();
                                    format!("loaded: {}", names.join(", "))
                                }
                                _ => "no models loaded in memory".into(),
                            }
                        } else {
                            "unable to parse".into()
                        }
                    }
                    Err(_) => "unable to query".into(),
                }
            } else {
                "n/a".into()
            };
            Ok(json!({
                "installed": installed,
                "running": running,
                "version": version.trim(),
                "models": models,
                "loaded": loaded,
            })
            .to_string())
        }

        "pull" | "add" | "download" => {
            let model = args.get("model").and_then(|v| v.as_str()).ok_or(
                "Missing required parameter: model (e.g. 'llama3.1', 'mistral', 'codellama')",
            )?;
            if !is_ollama_running_async().await {
                return Err(
                    "Ollama server is not running. Start it with action 'serve' first.".into(),
                );
            }
            sh_async(&format!("ollama pull {} 2>&1", model)).await
        }

        "rm" | "remove" | "delete" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_ollama_running_async().await {
                return Err("Ollama server is not running.".into());
            }
            sh_async(&format!("ollama rm {} 2>&1", model)).await
        }

        "list" | "ls" | "models" => {
            if !is_ollama_running_async().await {
                return sh_async("ollama list 2>&1").await;
            }
            match ollama_api_async("GET", "/api/tags", None).await {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        let models = parsed.get("models").and_then(|m| m.as_array());
                        match models {
                            Some(arr) if !arr.is_empty() => {
                                let mut lines = vec![
                                    "NAME                      SIZE       MODIFIED".to_string(),
                                ];
                                for m in arr {
                                    let name =
                                        m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                    let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                                    let size_str = if size > 1_000_000_000 {
                                        format!("{:.1} GB", size as f64 / 1e9)
                                    } else {
                                        format!("{:.0} MB", size as f64 / 1e6)
                                    };
                                    let modified = m
                                        .get("modified_at")
                                        .and_then(|d| d.as_str())
                                        .unwrap_or("?");
                                    lines.push(format!("{:<26}{:<11}{}", name, size_str, modified));
                                }
                                Ok(lines.join("\n"))
                            }
                            _ => {
                                Ok("No models downloaded. Use action 'pull' to download one."
                                    .into())
                            }
                        }
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(e),
            }
        }

        "show" | "info" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            sh_async(&format!("ollama show {} 2>&1", model)).await
        }

        "ps" | "running" | "loaded" => {
            if !is_ollama_running_async().await {
                return Err("Ollama server is not running.".into());
            }
            match ollama_api_async("GET", "/api/ps", None).await {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        let models = parsed.get("models").and_then(|m| m.as_array());
                        match models {
                            Some(arr) if !arr.is_empty() => {
                                let mut lines = vec![
                                    "NAME                      SIZE       PROCESSOR    EXPIRES"
                                        .to_string(),
                                ];
                                for m in arr {
                                    let name =
                                        m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                    let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                                    let size_str = format!("{:.0} MB", size as f64 / 1e6);
                                    let proc = m
                                        .get("size_vram")
                                        .and_then(|s| s.as_u64())
                                        .map(|v| if v > 0 { "GPU" } else { "CPU" })
                                        .unwrap_or("?");
                                    let expires =
                                        m.get("expires_at").and_then(|d| d.as_str()).unwrap_or("?");
                                    lines.push(format!(
                                        "{:<26}{:<11}{:<13}{}",
                                        name, size_str, proc, expires
                                    ));
                                }
                                Ok(lines.join("\n"))
                            }
                            _ => Ok("No models currently loaded in memory.".into()),
                        }
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(e),
            }
        }

        "load" | "warm" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_ollama_running_async().await {
                return Err("Ollama server is not running.".into());
            }
            let body = json!({
                "model": model,
                "prompt": "",
                "keep_alive": "10m"
            });
            match ollama_api_async("POST", "/api/generate", Some(&body)).await {
                Ok(_) => Ok(format!(
                    "Model '{}' loaded into memory (keep_alive: 10m).",
                    model
                )),
                Err(e) => Err(format!("Failed to load model '{}': {}", model, e)),
            }
        }

        "unload" | "evict" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_ollama_running_async().await {
                return Err("Ollama server is not running.".into());
            }
            let body = json!({
                "model": model,
                "prompt": "",
                "keep_alive": 0
            });
            match ollama_api_async("POST", "/api/generate", Some(&body)).await {
                Ok(_) => Ok(format!("Model '{}' unloaded from memory.", model)),
                Err(e) => Err(format!("Failed to unload model '{}': {}", model, e)),
            }
        }

        "copy" | "cp" => {
            let source = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model (source name)")?;
            let destination = args
                .get("destination")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: destination (new name)")?;
            sh_async(&format!("ollama cp {} {} 2>&1", source, destination)).await
        }

        _ => Err(format!(
            "Unknown ollama action: '{}'. Valid actions: setup, serve, stop, status, \
             pull, rm, list, show, ps, load, unload, copy.",
            action
        )),
    }
}

// ── Async helpers ───────────────────────────────────────────────────────────

/// Run a shell command asynchronously.
async fn sh_async(script: &str) -> Result<String, String> {
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .await
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }
    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

/// Hit the Ollama REST API asynchronously.
async fn ollama_api_async(
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<String, String> {
    let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let url = format!("{}{}", host, path);

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
        _ => return Err(format!("Unsupported HTTP method: {}", method)),
    };

    let response = request
        .send()
        .await
        .map_err(|e| format!("Ollama API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error = response.text().await.unwrap_or_default();
        return Err(format!("Ollama API error ({}): {}", status, error));
    }

    response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))
}

async fn is_ollama_installed_async() -> bool {
    tokio::process::Command::new("which")
        .arg("ollama")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn is_ollama_running_async() -> bool {
    ollama_api_async("GET", "/api/tags", None).await.is_ok()
}

// ── Sync implementations ────────────────────────────────────────────────────

/// Run a shell command, returning trimmed stdout or an error with stderr.
fn sh(script: &str) -> Result<String, String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Command exited with {}", output.status)
        } else {
            stderr
        });
    }
    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

/// Hit the Ollama REST API (default http://127.0.0.1:11434).
fn ollama_api(method: &str, path: &str, body: Option<&Value>) -> Result<String, String> {
    let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let url = format!("{}{}", host, path);

    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-s").arg("-S").arg("-X").arg(method);
    if let Some(b) = body {
        cmd.arg("-H").arg("Content-Type: application/json");
        cmd.arg("-d").arg(b.to_string());
    }
    cmd.arg(&url);

    let output = cmd.output().map_err(|e| format!("curl error: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("Ollama API request failed ({})", output.status)
        } else {
            format!("Ollama API error: {}", stderr)
        });
    }
    Ok(stdout)
}

fn is_ollama_installed() -> bool {
    std::process::Command::new("which")
        .arg("ollama")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_ollama_running() -> bool {
    ollama_api("GET", "/api/tags", None).is_ok()
}

/// `ollama_manage` — unified Ollama administration tool (sync).
pub fn exec_ollama_manage(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    match action {
        "setup" | "install" => {
            if is_ollama_installed() {
                let version = sh("ollama --version").unwrap_or_else(|_| "unknown".into());
                let running = if is_ollama_running() {
                    "running"
                } else {
                    "stopped"
                };
                return Ok(format!(
                    "Ollama is already installed ({}). Server status: {}.",
                    version.trim(),
                    running,
                ));
            }
            let os = std::env::consts::OS;
            let install_result = match os {
                "macos" => sh("brew install ollama 2>&1"),
                "linux" => sh("curl -fsSL https://ollama.com/install.sh | sh 2>&1"),
                _ => Err(format!(
                    "Unsupported OS for automatic install: {}. Visit https://ollama.com/download",
                    os
                )),
            };
            match install_result {
                Ok(out) => Ok(format!("Ollama installed successfully.\n{}", out)),
                Err(e) => Err(format!("Failed to install Ollama: {}", e)),
            }
        }

        "serve" | "start" => {
            if !is_ollama_installed() {
                return Err("Ollama is not installed. Run with action 'setup' first.".into());
            }
            if is_ollama_running() {
                return Ok("Ollama server is already running.".into());
            }
            let os = std::env::consts::OS;
            match os {
                "macos" => {
                    let _ = sh(
                        "brew services start ollama 2>/dev/null || nohup ollama serve > /dev/null 2>&1 &",
                    );
                }
                _ => {
                    let _ = sh("nohup ollama serve > /dev/null 2>&1 &");
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
            if is_ollama_running() {
                Ok("Ollama server started.".into())
            } else {
                Ok("Ollama serve command issued. It may take a moment to start.".into())
            }
        }

        "stop" => {
            if !is_ollama_running() {
                return Ok("Ollama server is not running.".into());
            }
            let os = std::env::consts::OS;
            match os {
                "macos" => sh(
                    "brew services stop ollama 2>/dev/null; pkill -f 'ollama serve' 2>/dev/null; echo 'Ollama server stopped.'",
                ),
                _ => sh("pkill -f 'ollama serve' 2>/dev/null; echo 'Ollama server stopped.'"),
            }
        }

        "status" => {
            let installed = is_ollama_installed();
            let running = is_ollama_running();
            let version = if installed {
                sh("ollama --version").unwrap_or_else(|_| "unknown".into())
            } else {
                "not installed".into()
            };
            let models = if running {
                match ollama_api("GET", "/api/tags", None) {
                    Ok(resp) => {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                            let count = parsed
                                .get("models")
                                .and_then(|m| m.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            format!("{} model(s) available", count)
                        } else {
                            "unable to parse".into()
                        }
                    }
                    Err(_) => "unable to query".into(),
                }
            } else {
                "server not running".into()
            };
            let loaded = if running {
                match ollama_api("GET", "/api/ps", None) {
                    Ok(resp) => {
                        if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                            let ms = parsed.get("models").and_then(|m| m.as_array());
                            match ms {
                                Some(arr) if !arr.is_empty() => {
                                    let names: Vec<String> = arr
                                        .iter()
                                        .filter_map(|m| {
                                            m.get("name")
                                                .and_then(|n| n.as_str())
                                                .map(|s| s.to_string())
                                        })
                                        .collect();
                                    format!("loaded: {}", names.join(", "))
                                }
                                _ => "no models loaded in memory".into(),
                            }
                        } else {
                            "unable to parse".into()
                        }
                    }
                    Err(_) => "unable to query".into(),
                }
            } else {
                "n/a".into()
            };
            Ok(json!({
                "installed": installed,
                "running": running,
                "version": version.trim(),
                "models": models,
                "loaded": loaded,
            })
            .to_string())
        }

        "pull" | "add" | "download" => {
            let model = args.get("model").and_then(|v| v.as_str()).ok_or(
                "Missing required parameter: model (e.g. 'llama3.1', 'mistral', 'codellama')",
            )?;
            if !is_ollama_running() {
                return Err(
                    "Ollama server is not running. Start it with action 'serve' first.".into(),
                );
            }
            sh(&format!("ollama pull {} 2>&1", model))
        }

        "rm" | "remove" | "delete" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_ollama_running() {
                return Err("Ollama server is not running.".into());
            }
            sh(&format!("ollama rm {} 2>&1", model))
        }

        "list" | "ls" | "models" => {
            if !is_ollama_running() {
                return sh("ollama list 2>&1");
            }
            match ollama_api("GET", "/api/tags", None) {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        let models = parsed.get("models").and_then(|m| m.as_array());
                        match models {
                            Some(arr) if !arr.is_empty() => {
                                let mut lines = vec![
                                    "NAME                      SIZE       MODIFIED".to_string(),
                                ];
                                for m in arr {
                                    let name =
                                        m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                    let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                                    let size_str = if size > 1_000_000_000 {
                                        format!("{:.1} GB", size as f64 / 1e9)
                                    } else {
                                        format!("{:.0} MB", size as f64 / 1e6)
                                    };
                                    let modified = m
                                        .get("modified_at")
                                        .and_then(|d| d.as_str())
                                        .unwrap_or("?");
                                    lines.push(format!("{:<26}{:<11}{}", name, size_str, modified));
                                }
                                Ok(lines.join("\n"))
                            }
                            _ => {
                                Ok("No models downloaded. Use action 'pull' to download one."
                                    .into())
                            }
                        }
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(e),
            }
        }

        "show" | "info" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            sh(&format!("ollama show {} 2>&1", model))
        }

        "ps" | "running" | "loaded" => {
            if !is_ollama_running() {
                return Err("Ollama server is not running.".into());
            }
            match ollama_api("GET", "/api/ps", None) {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        let models = parsed.get("models").and_then(|m| m.as_array());
                        match models {
                            Some(arr) if !arr.is_empty() => {
                                let mut lines = vec![
                                    "NAME                      SIZE       PROCESSOR    EXPIRES"
                                        .to_string(),
                                ];
                                for m in arr {
                                    let name =
                                        m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                    let size = m.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                                    let size_str = format!("{:.0} MB", size as f64 / 1e6);
                                    let proc = m
                                        .get("size_vram")
                                        .and_then(|s| s.as_u64())
                                        .map(|v| if v > 0 { "GPU" } else { "CPU" })
                                        .unwrap_or("?");
                                    let expires =
                                        m.get("expires_at").and_then(|d| d.as_str()).unwrap_or("?");
                                    lines.push(format!(
                                        "{:<26}{:<11}{:<13}{}",
                                        name, size_str, proc, expires
                                    ));
                                }
                                Ok(lines.join("\n"))
                            }
                            _ => Ok("No models currently loaded in memory.".into()),
                        }
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(e),
            }
        }

        "load" | "warm" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_ollama_running() {
                return Err("Ollama server is not running.".into());
            }
            let body = json!({
                "model": model,
                "prompt": "",
                "keep_alive": "10m"
            });
            match ollama_api("POST", "/api/generate", Some(&body)) {
                Ok(_) => Ok(format!(
                    "Model '{}' loaded into memory (keep_alive: 10m).",
                    model
                )),
                Err(e) => Err(format!("Failed to load model '{}': {}", model, e)),
            }
        }

        "unload" | "evict" => {
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_ollama_running() {
                return Err("Ollama server is not running.".into());
            }
            let body = json!({
                "model": model,
                "prompt": "",
                "keep_alive": 0
            });
            match ollama_api("POST", "/api/generate", Some(&body)) {
                Ok(_) => Ok(format!("Model '{}' unloaded from memory.", model)),
                Err(e) => Err(format!("Failed to unload model '{}': {}", model, e)),
            }
        }

        "copy" | "cp" => {
            let source = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model (source name)")?;
            let destination = args
                .get("destination")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: destination (new name)")?;
            sh(&format!("ollama cp {} {} 2>&1", source, destination))
        }

        _ => Err(format!(
            "Unknown ollama action: '{}'. Valid actions: setup, serve, stop, status, \
             pull, rm, list, show, ps, load, unload, copy.",
            action
        )),
    }
}
