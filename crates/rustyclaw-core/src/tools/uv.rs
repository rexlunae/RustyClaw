// uv tool integration for RustyClaw.
//
// Provides both sync and async implementations.

use serde_json::Value;
use std::path::{Path, PathBuf};
use tracing::{debug, instrument};

// ── Async implementations ───────────────────────────────────────────────────

/// `uv_manage` — unified Python dependency management via uv (async).
#[instrument(skip(args, workspace_dir), fields(action))]
pub async fn exec_uv_manage_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    tracing::Span::current().record("action", action);
    debug!("Executing uv_manage");

    match action {
        "setup" | "install" => {
            if is_uv_installed_async().await {
                let version = sh_async("uv --version 2>&1")
                    .await
                    .unwrap_or_else(|_| "unknown".into());
                return Ok(format!("uv is already installed ({}).", version.trim()));
            }
            sh_async("curl -LsSf https://astral.sh/uv/install.sh | sh 2>&1").await
        }

        "version" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            sh_async("uv --version 2>&1").await
        }

        "venv" | "create-venv" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or(".venv");
            let python = args.get("python").and_then(|v| v.as_str());
            let cmd = if let Some(py) = python {
                format!("uv venv {} --python {} 2>&1", name, py)
            } else {
                format!("uv venv {} 2>&1", name)
            };
            sh_in_async(workspace_dir, &cmd).await
        }

        "pip-install" | "add" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let packages: Vec<String> =
                if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                    vec![pkg.to_string()]
                } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                    pkgs.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    return Err("Missing required parameter: package or packages.".into());
                };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            sh_in_async(
                workspace_dir,
                &format!("uv pip install {} 2>&1", packages.join(" ")),
            )
            .await
        }

        "pip-uninstall" | "remove" | "rm" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let packages: Vec<String> =
                if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                    vec![pkg.to_string()]
                } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                    pkgs.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    return Err("Missing required parameter: package or packages.".into());
                };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            sh_in_async(
                workspace_dir,
                &format!("uv pip uninstall {} -y 2>&1", packages.join(" ")),
            )
            .await
        }

        "pip-list" | "list" | "ls" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            sh_in_async(workspace_dir, "uv pip list 2>&1").await
        }

        "pip-freeze" | "freeze" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            sh_in_async(workspace_dir, "uv pip freeze 2>&1").await
        }

        "sync" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let file = args.get("file").and_then(|v| v.as_str());
            let cmd = if let Some(f) = file {
                format!("uv pip sync {} 2>&1", f)
            } else {
                "uv pip sync requirements.txt 2>&1".to_string()
            };
            sh_in_async(workspace_dir, &cmd).await
        }

        "run" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: command")?;
            sh_in_async(workspace_dir, &format!("uv run {} 2>&1", command)).await
        }

        "python" | "install-python" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let version = args
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: version")?;
            sh_async(&format!("uv python install {} 2>&1", version)).await
        }

        "init" => {
            if !is_uv_installed_async().await {
                return Err("uv is not installed.".into());
            }
            let name = args.get("name").and_then(|v| v.as_str());
            let cmd = if let Some(n) = name {
                format!("uv init {} 2>&1", n)
            } else {
                "uv init 2>&1".to_string()
            };
            sh_in_async(workspace_dir, &cmd).await
        }

        _ => Err(format!(
            "Unknown uv action: '{}'. Valid: setup, version, venv, pip-install, pip-uninstall, pip-list, pip-freeze, sync, run, python, init.",
            action
        )),
    }
}

// ── Async helpers ───────────────────────────────────────────────────────────

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

async fn sh_in_async(dir: &Path, script: &str) -> Result<String, String> {
    let full_script = if let Some(venv) = find_venv(dir) {
        format!(
            "source '{}' && {}",
            venv.join("bin/activate").display(),
            script
        )
    } else {
        script.to_string()
    };

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&full_script)
        .current_dir(dir)
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

async fn is_uv_installed_async() -> bool {
    tokio::process::Command::new("which")
        .arg("uv")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Shared helpers ──────────────────────────────────────────────────────────

fn find_venv(workspace_dir: &Path) -> Option<PathBuf> {
    for name in &[".venv", "venv", "env", ".env"] {
        let candidate = workspace_dir.join(name);
        if candidate.join("bin/activate").exists() {
            return Some(candidate);
        }
    }
    if let Ok(v) = std::env::var("VIRTUAL_ENV") {
        let p = PathBuf::from(&v);
        if p.join("bin/activate").exists() {
            return Some(p);
        }
    }
    None
}

// ── Sync implementations ────────────────────────────────────────────────────

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

fn sh_in(dir: &Path, script: &str) -> Result<String, String> {
    let full_script = if let Some(venv) = find_venv(dir) {
        format!(
            "source '{}' && {}",
            venv.join("bin/activate").display(),
            script
        )
    } else {
        script.to_string()
    };

    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&full_script)
        .current_dir(dir)
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

fn is_uv_installed() -> bool {
    std::process::Command::new("which")
        .arg("uv")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `uv_manage` — unified Python dependency management via uv (sync).
pub fn exec_uv_manage(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    match action {
        "setup" | "install" => {
            if is_uv_installed() {
                let version = sh("uv --version 2>&1").unwrap_or_else(|_| "unknown".into());
                return Ok(format!("uv is already installed ({}).", version.trim()));
            }
            sh("curl -LsSf https://astral.sh/uv/install.sh | sh 2>&1")
        }

        "version" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            sh("uv --version 2>&1")
        }

        "venv" | "create-venv" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or(".venv");
            let python = args.get("python").and_then(|v| v.as_str());
            let cmd = if let Some(py) = python {
                format!("uv venv {} --python {} 2>&1", name, py)
            } else {
                format!("uv venv {} 2>&1", name)
            };
            sh_in(workspace_dir, &cmd)
        }

        "pip-install" | "add" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let packages: Vec<String> =
                if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                    vec![pkg.to_string()]
                } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                    pkgs.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    return Err("Missing required parameter: package or packages.".into());
                };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            sh_in(
                workspace_dir,
                &format!("uv pip install {} 2>&1", packages.join(" ")),
            )
        }

        "pip-uninstall" | "remove" | "rm" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let packages: Vec<String> =
                if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                    vec![pkg.to_string()]
                } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                    pkgs.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    return Err("Missing required parameter: package or packages.".into());
                };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            sh_in(
                workspace_dir,
                &format!("uv pip uninstall {} -y 2>&1", packages.join(" ")),
            )
        }

        "pip-list" | "list" | "ls" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            sh_in(workspace_dir, "uv pip list 2>&1")
        }

        "pip-freeze" | "freeze" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            sh_in(workspace_dir, "uv pip freeze 2>&1")
        }

        "sync" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let file = args.get("file").and_then(|v| v.as_str());
            let cmd = if let Some(f) = file {
                format!("uv pip sync {} 2>&1", f)
            } else {
                "uv pip sync requirements.txt 2>&1".to_string()
            };
            sh_in(workspace_dir, &cmd)
        }

        "run" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: command")?;
            sh_in(workspace_dir, &format!("uv run {} 2>&1", command))
        }

        "python" | "install-python" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let version = args
                .get("version")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: version")?;
            sh(&format!("uv python install {} 2>&1", version))
        }

        "init" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let name = args.get("name").and_then(|v| v.as_str());
            let cmd = if let Some(n) = name {
                format!("uv init {} 2>&1", n)
            } else {
                "uv init 2>&1".to_string()
            };
            sh_in(workspace_dir, &cmd)
        }

        _ => Err(format!(
            "Unknown uv action: '{}'. Valid: setup, version, venv, pip-install, pip-uninstall, pip-list, pip-freeze, sync, run, python, init.",
            action
        )),
    }
}
