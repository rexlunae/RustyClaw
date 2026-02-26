// npm / Node.js package management tool for RustyClaw.
//
// Provides both sync and async implementations.

use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

// ── Async implementations ───────────────────────────────────────────────────

/// `npm_manage` — unified Node.js / npm administration tool (async).
#[instrument(skip(args, workspace_dir), fields(action))]
pub async fn exec_npm_manage_async(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: action")?;

    tracing::Span::current().record("action", action);
    debug!("Executing npm_manage");

    match action {
        "setup" | "install-node" => {
            if is_node_installed_async(workspace_dir).await
                && is_npm_installed_async(workspace_dir).await
            {
                let node_v = sh_in_async(workspace_dir, "node --version 2>&1")
                    .await
                    .unwrap_or_else(|_| "unknown".into());
                let npm_v = sh_in_async(workspace_dir, "npm --version 2>&1")
                    .await
                    .unwrap_or_else(|_| "unknown".into());
                return Ok(format!(
                    "Node.js ({}) and npm ({}) are already installed.",
                    node_v.trim(),
                    npm_v.trim()
                ));
            }
            let os = std::env::consts::OS;
            match os {
                "macos" => {
                    let result = sh_async("command -v brew >/dev/null 2>&1 && brew install node 2>&1").await;
                    if result.is_ok() { return result; }
                    sh_async("curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash 2>&1 && \
                        export NVM_DIR=\"$HOME/.nvm\" && . \"$NVM_DIR/nvm.sh\" && nvm install --lts 2>&1").await
                }
                "linux" => {
                    sh_async("curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash 2>&1 && \
                        export NVM_DIR=\"$HOME/.nvm\" && . \"$NVM_DIR/nvm.sh\" && nvm install --lts 2>&1").await
                }
                _ => Err(format!("Unsupported OS: {}", os)),
            }
        }

        "version" | "versions" => {
            let node_v = sh_in_async(workspace_dir, "node --version 2>&1")
                .await
                .unwrap_or_else(|_| "not installed".into());
            let npm_v = sh_in_async(workspace_dir, "npm --version 2>&1")
                .await
                .unwrap_or_else(|_| "not installed".into());
            let npx_v = sh_in_async(workspace_dir, "npx --version 2>&1")
                .await
                .unwrap_or_else(|_| "not installed".into());
            Ok(format!(
                "node: {}\nnpm:  {}\nnpx:  {}",
                node_v.trim(),
                npm_v.trim(),
                npx_v.trim()
            ))
        }

        "init" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let yes = args.get("yes").and_then(|v| v.as_bool()).unwrap_or(true);
            let cmd = if yes {
                "npm init -y 2>&1"
            } else {
                "npm init 2>&1"
            };
            sh_in_async(workspace_dir, cmd).await
        }

        "npm-install" | "add" | "i" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let packages: Vec<String> =
                if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                    vec![pkg.to_string()]
                } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                    pkgs.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    vec![]
                };

            let mut cmd = String::from("npm install");
            if !packages.is_empty() {
                cmd.push(' ');
                cmd.push_str(&packages.join(" "));
            }
            if args.get("dev").and_then(|v| v.as_bool()).unwrap_or(false) {
                cmd.push_str(" --save-dev");
            }
            if args
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                cmd.push_str(" -g");
            }
            cmd.push_str(" 2>&1");
            sh_in_async(workspace_dir, &cmd).await
        }

        "uninstall" | "remove" | "rm" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let packages: Vec<String> =
                if let Some(pkg) = args.get("package").and_then(|v| v.as_str()) {
                    vec![pkg.to_string()]
                } else if let Some(pkgs) = args.get("packages").and_then(|v| v.as_array()) {
                    pkgs.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    return Err("Missing package or packages.".into());
                };
            if packages.is_empty() {
                return Err("No packages specified.".into());
            }
            let global = if args
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                " -g"
            } else {
                ""
            };
            sh_in_async(
                workspace_dir,
                &format!("npm uninstall {}{} 2>&1", packages.join(" "), global),
            )
            .await
        }

        "list" | "ls" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(0);
            let global = if args
                .get("global")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                " -g"
            } else {
                ""
            };
            sh_in_async(
                workspace_dir,
                &format!("npm list --depth={}{} 2>&1", depth, global),
            )
            .await
        }

        "outdated" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            sh_in_async(workspace_dir, "npm outdated 2>&1 || true").await
        }

        "update" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let pkg = args.get("package").and_then(|v| v.as_str());
            let cmd = match pkg {
                Some(p) => format!("npm update {} 2>&1", p),
                None => "npm update 2>&1".to_string(),
            };
            sh_in_async(workspace_dir, &cmd).await
        }

        "run" | "run-script" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let script = args
                .get("script")
                .and_then(|v| v.as_str())
                .ok_or("Missing script")?;
            let extra = args.get("args").and_then(|v| v.as_str()).unwrap_or("");
            let cmd = if extra.is_empty() {
                format!("npm run {} 2>&1", script)
            } else {
                format!("npm run {} -- {} 2>&1", script, extra)
            };
            sh_in_async(workspace_dir, &cmd).await
        }

        "start" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            sh_in_async(workspace_dir, "npm start 2>&1").await
        }

        "build" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            sh_in_async(workspace_dir, "npm run build 2>&1").await
        }

        "test" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            sh_in_async(workspace_dir, "npm test 2>&1").await
        }

        "npx" | "exec" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or("Missing command")?;
            sh_in_async(workspace_dir, &format!("npx -y {} 2>&1", command)).await
        }

        "audit" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let fix = args.get("fix").and_then(|v| v.as_bool()).unwrap_or(false);
            let cmd = if fix {
                "npm audit fix 2>&1"
            } else {
                "npm audit 2>&1 || true"
            };
            sh_in_async(workspace_dir, cmd).await
        }

        "cache-clean" | "cache" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            sh_in_async(workspace_dir, "npm cache clean --force 2>&1").await
        }

        "info" | "view" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let pkg = args
                .get("package")
                .and_then(|v| v.as_str())
                .ok_or("Missing package")?;
            sh_in_async(workspace_dir, &format!("npm info {} 2>&1", pkg)).await
        }

        "search" => {
            if !is_npm_installed_async(workspace_dir).await {
                return Err("npm is not installed.".into());
            }
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("Missing query")?;
            sh_in_async(workspace_dir, &format!("npm search {} 2>&1", query)).await
        }

        "status" => {
            let node_installed = is_node_installed_async(workspace_dir).await;
            let npm_installed = is_npm_installed_async(workspace_dir).await;
            let node_v = if node_installed {
                sh_in_async(workspace_dir, "node --version 2>&1")
                    .await
                    .unwrap_or_else(|_| "unknown".into())
            } else {
                "not installed".into()
            };
            let npm_v = if npm_installed {
                sh_in_async(workspace_dir, "npm --version 2>&1")
                    .await
                    .unwrap_or_else(|_| "unknown".into())
            } else {
                "not installed".into()
            };

            let pkg_json_path = workspace_dir.join("package.json");
            let has_pkg_json = tokio::fs::try_exists(&pkg_json_path).await.unwrap_or(false);
            let has_node_modules = tokio::fs::try_exists(workspace_dir.join("node_modules"))
                .await
                .unwrap_or(false);
            let has_lock = tokio::fs::try_exists(workspace_dir.join("package-lock.json"))
                .await
                .unwrap_or(false)
                || tokio::fs::try_exists(workspace_dir.join("yarn.lock"))
                    .await
                    .unwrap_or(false)
                || tokio::fs::try_exists(workspace_dir.join("pnpm-lock.yaml"))
                    .await
                    .unwrap_or(false);

            let scripts = if has_pkg_json {
                match tokio::fs::read_to_string(&pkg_json_path).await {
                    Ok(content) => {
                        if let Ok(pkg) = serde_json::from_str::<Value>(&content) {
                            if let Some(s) = pkg.get("scripts").and_then(|s| s.as_object()) {
                                s.keys().cloned().collect::<Vec<_>>().join(", ")
                            } else {
                                "none".into()
                            }
                        } else {
                            "parse error".into()
                        }
                    }
                    Err(_) => "read error".into(),
                }
            } else {
                "n/a".into()
            };

            Ok(json!({
                "node": node_v.trim(),
                "npm": npm_v.trim(),
                "package_json": has_pkg_json,
                "node_modules": has_node_modules,
                "lock_file": has_lock,
                "scripts": scripts,
            })
            .to_string())
        }

        _ => Err(format!(
            "Unknown npm action: '{}'. Valid: setup, version, init, npm-install, uninstall, list, outdated, update, run, start, build, test, npx, audit, cache-clean, info, search, status.",
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
    let preamble = r#"
        export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
        [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" 2>/dev/null
        command -v fnm >/dev/null 2>&1 && eval "$(fnm env --use-on-cd)" 2>/dev/null
    "#;
    let full = format!("{}\n{}", preamble.trim(), script);

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&full)
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

async fn is_node_installed_async(workspace_dir: &Path) -> bool {
    sh_in_async(workspace_dir, "command -v node >/dev/null 2>&1 && echo yes")
        .await
        .map(|s| s.contains("yes"))
        .unwrap_or(false)
}

async fn is_npm_installed_async(workspace_dir: &Path) -> bool {
    sh_in_async(workspace_dir, "command -v npm >/dev/null 2>&1 && echo yes")
        .await
        .map(|s| s.contains("yes"))
        .unwrap_or(false)
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
    let preamble = r#"
        export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
        [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh" 2>/dev/null
        command -v fnm >/dev/null 2>&1 && eval "$(fnm env --use-on-cd)" 2>/dev/null
    "#;
    let full = format!("{}\n{}", preamble.trim(), script);
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&full)
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

fn is_node_installed(workspace_dir: &Path) -> bool {
    sh_in(workspace_dir, "command -v node >/dev/null 2>&1 && echo yes")
        .map(|s| s.contains("yes"))
        .unwrap_or(false)
}

fn is_npm_installed(workspace_dir: &Path) -> bool {
    sh_in(workspace_dir, "command -v npm >/dev/null 2>&1 && echo yes")
        .map(|s| s.contains("yes"))
        .unwrap_or(false)
}

/// `npm_manage` — unified Node.js / npm administration tool (sync).
pub fn exec_npm_manage(args: &Value, workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("Missing action")?;

    match action {
        "setup" | "install-node" => {
            if is_node_installed(workspace_dir) && is_npm_installed(workspace_dir) {
                let node_v = sh_in(workspace_dir, "node --version 2>&1")
                    .unwrap_or_else(|_| "unknown".into());
                let npm_v =
                    sh_in(workspace_dir, "npm --version 2>&1").unwrap_or_else(|_| "unknown".into());
                return Ok(format!(
                    "Node.js ({}) and npm ({}) are already installed.",
                    node_v.trim(),
                    npm_v.trim()
                ));
            }
            let os = std::env::consts::OS;
            match os {
                "macos" => {
                    let result = sh("command -v brew >/dev/null 2>&1 && brew install node 2>&1");
                    if result.is_ok() {
                        return result;
                    }
                    sh(
                        "curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash 2>&1 && \
                        export NVM_DIR=\"$HOME/.nvm\" && . \"$NVM_DIR/nvm.sh\" && nvm install --lts 2>&1",
                    )
                }
                "linux" => sh(
                    "curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash 2>&1 && \
                    export NVM_DIR=\"$HOME/.nvm\" && . \"$NVM_DIR/nvm.sh\" && nvm install --lts 2>&1",
                ),
                _ => Err(format!("Unsupported OS: {}", os)),
            }
        }
        "version" | "versions" => {
            let node_v = sh_in(workspace_dir, "node --version 2>&1")
                .unwrap_or_else(|_| "not installed".into());
            let npm_v = sh_in(workspace_dir, "npm --version 2>&1")
                .unwrap_or_else(|_| "not installed".into());
            let npx_v = sh_in(workspace_dir, "npx --version 2>&1")
                .unwrap_or_else(|_| "not installed".into());
            Ok(format!(
                "node: {}\nnpm:  {}\nnpx:  {}",
                node_v.trim(),
                npm_v.trim(),
                npx_v.trim()
            ))
        }
        "status" => {
            let node_installed = is_node_installed(workspace_dir);
            let npm_installed = is_npm_installed(workspace_dir);
            Ok(
                json!({ "node_installed": node_installed, "npm_installed": npm_installed })
                    .to_string(),
            )
        }
        _ => Err(format!(
            "Sync execution not fully supported for '{}'. Use async dispatch.",
            action
        )),
    }
}
