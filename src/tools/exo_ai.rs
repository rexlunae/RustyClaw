// Exo AI administration tools for RustyClaw.
//
// Exo is a distributed AI cluster framework that lets you pool multiple
// devices (Macs, Linux boxes, etc.) into a single inference cluster.
// It is installed via pip/uv as a Python package.  The exo CLI provides
// cluster management, model downloading, and inference serving.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Helpers ─────────────────────────────────────────────────────────────────

fn sh(script: &str) -> Result<String, String> {
    let output = Command::new("sh")
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

/// Locate a Python virtual environment relative to the workspace dir.
/// Checks: .venv, venv, env, .env — returns the first that has a bin/activate.
fn find_venv(workspace_dir: &Path) -> Option<PathBuf> {
    for name in &[".venv", "venv", "env", ".env"] {
        let candidate = workspace_dir.join(name);
        if candidate.join("bin/activate").exists() {
            return Some(candidate);
        }
    }
    // Also honour an explicit VIRTUAL_ENV from the environment.
    if let Ok(v) = std::env::var("VIRTUAL_ENV") {
        let p = PathBuf::from(&v);
        if p.join("bin/activate").exists() {
            return Some(p);
        }
    }
    None
}

/// Run a shell command with the venv activated (if one exists in workspace_dir).
/// Falls back to a plain shell if no venv is found.
fn venv_sh(workspace_dir: &Path, script: &str) -> Result<String, String> {
    let full_script = if let Some(venv) = find_venv(workspace_dir) {
        format!(
            "source '{}' && {}",
            venv.join("bin/activate").display(),
            script
        )
    } else {
        script.to_string()
    };

    let output = Command::new("sh")
        .arg("-c")
        .arg(&full_script)
        .current_dir(workspace_dir)
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

/// Check whether `exo` is on PATH or inside the workspace venv.
fn is_exo_installed(workspace_dir: &Path) -> bool {
    // Check the venv first
    if let Some(venv) = find_venv(workspace_dir) {
        if venv.join("bin/exo").exists() {
            return true;
        }
    }
    // Fall back to global PATH
    Command::new("which")
        .arg("exo")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_uv_installed() -> bool {
    Command::new("which")
        .arg("uv")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if exo is currently running by looking for the process.
fn is_exo_running() -> bool {
    Command::new("pgrep")
        .arg("-f")
        .arg("exo")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Hit the exo REST API (default http://localhost:52415).
fn exo_api(method: &str, path: &str) -> Result<String, String> {
    let host = std::env::var("EXO_API_HOST").unwrap_or_else(|_| "http://localhost:52415".into());
    let url = format!("{}{}", host, path);

    let output = Command::new("curl")
        .arg("-s").arg("-S")
        .arg("-X").arg(method)
        .arg("--max-time").arg("5")
        .arg(&url)
        .output()
        .map_err(|e| format!("curl error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() && stdout.is_empty() {
        return Err(if stderr.is_empty() {
            format!("exo API request failed ({})", output.status)
        } else {
            format!("exo API error: {}", stderr)
        });
    }
    Ok(stdout)
}

// ── Tool executor ───────────────────────────────────────────────────────────

/// `exo_manage` — unified exo AI cluster administration tool.
pub fn exec_exo_manage(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    match action {
        // ── setup / install ─────────────────────────────────────
        "setup" | "install" => {
            if is_exo_installed(workspace_dir) {
                let version = venv_sh(workspace_dir, "exo --version 2>&1")
                    .unwrap_or_else(|_| "unknown".into());
                return Ok(format!("exo is already installed ({}).", version.trim()));
            }

            // Ensure we have a venv; create one if missing.
            let venv = find_venv(workspace_dir);
            if venv.is_none() {
                // Try to create one with uv, then plain python
                let venv_dir = workspace_dir.join(".venv");
                let created = if is_uv_installed() {
                    sh(&format!(
                        "cd '{}' && uv venv '{}' 2>&1",
                        workspace_dir.display(),
                        venv_dir.display()
                    ))
                } else {
                    sh(&format!(
                        "python3 -m venv '{}' 2>&1 || python -m venv '{}' 2>&1",
                        venv_dir.display(),
                        venv_dir.display()
                    ))
                };
                if let Err(e) = created {
                    return Err(format!(
                        "No virtual environment found and failed to create one: {}",
                        e
                    ));
                }
            }

            // Now install exo inside the venv
            let install_result = if is_uv_installed() {
                venv_sh(workspace_dir, "uv pip install exo-ai 2>&1")
            } else {
                venv_sh(workspace_dir, "pip install exo-ai 2>&1")
            };

            match install_result {
                Ok(msg) => {
                    // Verify it landed
                    if is_exo_installed(workspace_dir) {
                        Ok(format!(
                            "exo installed successfully into venv at {}.\n{}",
                            find_venv(workspace_dir)
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "(venv)".into()),
                            msg
                        ))
                    } else {
                        Ok(format!(
                            "Install command completed but exo binary not found. Output:\n{}",
                            msg
                        ))
                    }
                }
                Err(e) => Err(format!("Failed to install exo-ai: {}", e)),
            }
        }

        // ── start (launch exo cluster node) ─────────────────────
        "start" | "run" | "serve" => {
            if !is_exo_installed(workspace_dir) {
                return Err("exo is not installed. Run with action 'setup' first.".into());
            }
            if is_exo_running() {
                return Ok("exo is already running.".into());
            }
            // Build the exo command with optional parameters
            let mut cmd_parts = vec!["nohup exo run".to_string()];

            if let Some(model) = args.get("model").and_then(|v| v.as_str()) {
                cmd_parts.push(format!("--model-id {}", model));
            }
            if let Some(port) = args.get("port").and_then(|v| v.as_u64()) {
                cmd_parts.push(format!("--chatgpt-api-port {}", port));
            }
            if let Some(discovery) = args.get("discovery").and_then(|v| v.as_str()) {
                cmd_parts.push(format!("--discovery-module {}", discovery));
            }

            cmd_parts.push("> /dev/null 2>&1 &".to_string());
            let _ = venv_sh(workspace_dir, &cmd_parts.join(" "));

            std::thread::sleep(std::time::Duration::from_secs(3));
            if is_exo_running() {
                Ok("exo cluster node started. Peers will be discovered automatically.".into())
            } else {
                Ok("exo start command issued. It may take a moment to initialize.".into())
            }
        }

        // ── stop ────────────────────────────────────────────────
        "stop" => {
            if !is_exo_running() {
                return Ok("exo is not running.".into());
            }
            sh("pkill -f 'exo run' 2>/dev/null; pkill -f 'exo serve' 2>/dev/null; echo 'exo stopped.'")
        }

        // ── status ──────────────────────────────────────────────
        "status" => {
            let installed = is_exo_installed(workspace_dir);
            let running = is_exo_running();
            let version = if installed {
                venv_sh(workspace_dir, "exo --version 2>&1")
                    .unwrap_or_else(|_| "unknown".into())
            } else {
                "not installed".into()
            };

            let venv_info = find_venv(workspace_dir)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "none".into());

            let topology = if running {
                exo_api("GET", "/v1/topology").unwrap_or_else(|_| "unable to query".into())
            } else {
                "n/a".into()
            };

            let models = if running {
                exo_api("GET", "/v1/models").unwrap_or_else(|_| "unable to query".into())
            } else {
                "n/a".into()
            };

            Ok(json!({
                "installed": installed,
                "running": running,
                "version": version.trim(),
                "venv": venv_info,
                "topology": topology,
                "models": models,
            }).to_string())
        }

        // ── topology (view cluster peers) ───────────────────────
        "topology" | "peers" | "cluster" => {
            if !is_exo_running() {
                return Err("exo is not running. Start it with action 'start' first.".into());
            }
            match exo_api("GET", "/v1/topology") {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        Ok(serde_json::to_string_pretty(&parsed).unwrap_or(resp))
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(format!("Failed to get topology: {}", e)),
            }
        }

        // ── models (list available models) ──────────────────────
        "models" | "list" | "ls" => {
            if !is_exo_running() {
                return Err("exo is not running.".into());
            }
            match exo_api("GET", "/v1/models") {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
                            let mut lines = vec!["Available models:".to_string()];
                            for m in data {
                                let id = m.get("id").and_then(|i| i.as_str()).unwrap_or("?");
                                lines.push(format!("  • {}", id));
                            }
                            if lines.len() == 1 {
                                lines.push("  (none)".to_string());
                            }
                            Ok(lines.join("\n"))
                        } else {
                            Ok(serde_json::to_string_pretty(&parsed).unwrap_or(resp))
                        }
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(format!("Failed to list models: {}", e)),
            }
        }

        // ── download (pre-download a model) ─────────────────────
        "download" | "pull" | "add" => {
            let model = args.get("model").and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model (e.g. 'llama-3.1-8b', 'mistral-7b')")?;
            if !is_exo_installed(workspace_dir) {
                return Err("exo is not installed.".into());
            }
            venv_sh(workspace_dir, &format!("exo download {} 2>&1", model))
        }

        // ── remove (delete a downloaded model) ──────────────────
        "remove" | "rm" | "delete" => {
            let model = args.get("model").and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model")?;
            if !is_exo_installed(workspace_dir) {
                return Err("exo is not installed.".into());
            }
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
            let cache_dir = format!("{}/.cache/huggingface/hub", home);
            let result = sh(&format!(
                "find {} -maxdepth 1 -name '*{}*' -type d 2>/dev/null",
                cache_dir,
                model.replace('/', "--")
            ));
            match result {
                Ok(dirs) if !dirs.is_empty() => {
                    for dir in dirs.lines() {
                        let _ = sh(&format!("rm -rf '{}' 2>&1", dir));
                    }
                    Ok(format!("Model '{}' removed from cache.", model))
                }
                _ => Err(format!("Model '{}' not found in cache at {}.", model, cache_dir)),
            }
        }

        _ => Err(format!(
            "Unknown exo action: '{}'. Valid actions: setup, start, stop, status, \
             topology, models, download, remove.",
            action
        )),
    }
}
