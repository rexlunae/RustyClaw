// uv tool integration for RustyClaw.
//
// uv is an ultra-fast Python package manager written in Rust by Astral.
// This tool provides a unified interface for installing uv, creating
// virtual environments, and managing Python dependencies.
// All pip-related commands are venv-aware: if a .venv (or venv/env/.env)
// exists in the workspace, it is automatically activated before running.

use serde_json::Value;
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

/// Run a shell command in `dir`, activating the venv if one exists.
fn sh_in(dir: &Path, script: &str) -> Result<String, String> {
    let full_script = if let Some(venv) = find_venv(dir) {
        format!(
            "source '{}' && {}",
            venv.join("bin/activate").display(),
            script,
        )
    } else {
        script.to_string()
    };

    let output = Command::new("sh")
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
    Command::new("which")
        .arg("uv")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── Tool executor ───────────────────────────────────────────────────────────

/// `uv_manage` — unified Python dependency management via uv.
pub fn exec_uv_manage(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    match action {
        // ── setup / install uv itself ───────────────────────────
        "setup" | "install" => {
            if is_uv_installed() {
                let version = sh("uv --version 2>&1").unwrap_or_else(|_| "unknown".into());
                return Ok(format!("uv is already installed ({}).", version.trim()));
            }
            sh("curl -LsSf https://astral.sh/uv/install.sh | sh 2>&1")
        }

        // ── version ─────────────────────────────────────────────
        "version" => {
            if !is_uv_installed() {
                return Err("uv is not installed. Run with action 'setup' first.".into());
            }
            sh("uv --version 2>&1")
        }

        // ── venv (create a virtual environment) ─────────────────
        "venv" | "create-venv" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or(".venv");
            let python = args.get("python").and_then(|v| v.as_str());
            let mut cmd = format!("uv venv {} 2>&1", name);
            if let Some(py) = python {
                cmd = format!("uv venv {} --python {} 2>&1", name, py);
            }
            sh_in(workspace_dir, &cmd)
        }

        // ── pip install ─────────────────────────────────────────
        "pip-install" | "add" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            // Accept either a single "package" or an array of "packages"
            let packages: Vec<String> = if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                vec![pkg.to_string()]
            } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                pkgs.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            } else {
                return Err("Missing required parameter: package (string) or packages (array).".into());
            };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            let pkg_str = packages.join(" ");
            sh_in(workspace_dir, &format!("uv pip install {} 2>&1", pkg_str))
        }

        // ── pip uninstall ───────────────────────────────────────
        "pip-uninstall" | "remove" | "rm" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let packages: Vec<String> = if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                vec![pkg.to_string()]
            } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                pkgs.iter().filter_map(|v| v.as_str().map(String::from)).collect()
            } else {
                return Err("Missing required parameter: package or packages.".into());
            };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            let pkg_str = packages.join(" ");
            sh_in(workspace_dir, &format!("uv pip uninstall {} -y 2>&1", pkg_str))
        }

        // ── pip list ────────────────────────────────────────────
        "pip-list" | "list" | "ls" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            sh_in(workspace_dir, "uv pip list 2>&1")
        }

        // ── pip freeze ──────────────────────────────────────────
        "pip-freeze" | "freeze" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            sh_in(workspace_dir, "uv pip freeze 2>&1")
        }

        // ── sync (install from requirements/pyproject) ──────────
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

        // ── run (execute a command in the uv-managed env) ───────
        "run" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let command = args.get("command").and_then(|v| v.as_str())
                .ok_or("Missing required parameter: command")?;
            sh_in(workspace_dir, &format!("uv run {} 2>&1", command))
        }

        // ── python (install a specific Python version) ──────────
        "python" | "install-python" => {
            if !is_uv_installed() {
                return Err("uv is not installed.".into());
            }
            let version = args.get("version").and_then(|v| v.as_str())
                .ok_or("Missing required parameter: version (e.g. '3.12', '3.11.6')")?;
            sh(&format!("uv python install {} 2>&1", version))
        }

        // ── init (create a new Python project) ──────────────────
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
            "Unknown uv action: '{}'. Valid actions: setup, version, venv, \
             pip-install, pip-uninstall, pip-list, pip-freeze, sync, run, python, init.",
            action
        )),
    }
}
