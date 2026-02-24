// Exo AI administration tools for RustyClaw.
//
// Exo (https://github.com/exo-explore/exo) is a distributed AI cluster
// framework that pools multiple devices into a single inference cluster.
//
// IMPORTANT: exo must be run from a cloned source checkout using `uv run exo`.
// It is NOT a simple pip package.  The setup process is:
//   1. git clone https://github.com/exo-explore/exo
//   2. cd exo/dashboard && npm install && npm run build
//   3. uv run exo   (from the repo root)
//
// API reference: http://localhost:52415
//   GET  /models | /v1/models     — list available models
//   GET  /state                   — cluster state + active instances
//   GET  /node_id                 — this node's ID
//   POST /instance                — create a model instance (load a model)
//   GET  /instance/previews?model_id=...  — preview placements
//   DELETE /instance/{id}         — unload instance
//   POST /v1/chat/completions     — OpenAI-compatible inference

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Run a shell one-liner and return combined stdout (+ stderr if non-empty).
fn sh(script: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(script)
        .output()
        .map_err(|e| format!("shell error: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        // Always report failure — even when there's partial stdout.
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Command exited with {}", output.status)
        };
        return Err(detail);
    }
    if !stderr.is_empty() && !stdout.is_empty() {
        Ok(format!("{}\n[stderr] {}", stdout, stderr))
    } else if !stdout.is_empty() {
        Ok(stdout)
    } else {
        Ok(stderr)
    }
}

/// Format a byte count into a human-readable string (e.g. "1.23 GB").
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.2} TB", b / TB)
    } else if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// Extract download progress from the `/state` JSON and return a formatted summary.
fn parse_downloads_from_state(state: &Value) -> String {
    let downloads = match state.get("downloads") {
        Some(d) => d,
        None => return "No download information available.".into(),
    };

    let mut pending: Vec<String> = Vec::new();
    let mut completed: Vec<String> = Vec::new();

    // downloads is keyed by node ID, each value is an array of download entries
    if let Some(obj) = downloads.as_object() {
        for (_node_id, entries) in obj {
            if let Some(arr) = entries.as_array() {
                for entry in arr {
                    if let Some(dp) = entry.get("DownloadPending") {
                        let model_id = dp
                            .pointer("/shardMetadata/PipelineShardMetadata/modelCard/modelId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let downloaded = dp
                            .pointer("/downloaded/inBytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let total = dp
                            .pointer("/total/inBytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1);
                        let pct = if total > 0 {
                            (downloaded as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        // Visual progress bar
                        let filled = (pct / 5.0) as usize; // 20 chars wide
                        let bar: String = format!(
                            "{}{}",
                            "█".repeat(filled),
                            "░".repeat(20_usize.saturating_sub(filled))
                        );
                        pending.push(format!(
                            "  ⏳ {} [{bar}] {:.1}%  ({} / {})",
                            model_id,
                            pct,
                            format_bytes(downloaded),
                            format_bytes(total),
                        ));
                    } else if let Some(dc) = entry.get("DownloadCompleted") {
                        let model_id = dc
                            .pointer("/shardMetadata/PipelineShardMetadata/modelCard/modelId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let total = dc
                            .pointer("/total/inBytes")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        completed.push(format!(
                            "  ✅ {} ({})",
                            model_id,
                            format_bytes(total),
                        ));
                    }
                }
            }
        }
    }

    if pending.is_empty() && completed.is_empty() {
        return "No downloads in progress or completed.".into();
    }

    let mut out = Vec::new();
    if !completed.is_empty() {
        out.push(format!("Completed ({}):", completed.len()));
        out.extend(completed);
    }
    // Sort pending: actively downloading (downloaded > 0) first, then by name
    pending.sort_by(|a, b| {
        // Extract percentage for sorting: higher % first
        let pct_a = a.split(']').next().unwrap_or("").matches('█').count();
        let pct_b = b.split(']').next().unwrap_or("").matches('█').count();
        pct_b.cmp(&pct_a).then_with(|| a.cmp(b))
    });
    if !pending.is_empty() {
        out.push(format!("Pending/Downloading ({}):", pending.len()));
        out.extend(pending);
    }
    out.join("\n")
}

/// Extract runner errors from the `/state` JSON.
fn parse_runner_errors(state: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(runners) = state.get("runners").and_then(|r| r.as_object()) {
        for (runner_id, info) in runners {
            if let Some(failed) = info.get("RunnerFailed") {
                let msg = failed
                    .get("errorMessage")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                errors.push(format!("  ⚠ Runner {}: {}", &runner_id[..8.min(runner_id.len())], msg));
            }
        }
    }
    errors
}

/// Extract active instances from the `/state` JSON.
fn parse_instances(state: &Value) -> Vec<String> {
    let mut instances = Vec::new();
    if let Some(inst_map) = state.get("instances").and_then(|i| i.as_object()) {
        for (id, info) in inst_map {
            // Try to get model ID from the instance
            let model_id = info
                .pointer("/MlxRingInstance/shardAssignments/modelId")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            instances.push(format!("  🟢 {} (instance: {})", model_id, &id[..8.min(id.len())]));
        }
    }
    instances
}

/// Extract node identity info from the `/state` JSON.
fn parse_node_info(state: &Value) -> Vec<String> {
    let mut nodes = Vec::new();
    if let Some(identities) = state.get("nodeIdentities").and_then(|n| n.as_object()) {
        for (node_id, info) in identities {
            let name = info.get("friendlyName").and_then(|v| v.as_str()).unwrap_or("unknown");
            let chip = info.get("chipId").and_then(|v| v.as_str()).unwrap_or("");
            let short_id = &node_id[..12.min(node_id.len())];

            // Get memory info if available
            let mem_info = state
                .pointer(&format!("/nodeMemory/{}", node_id))
                .map(|m| {
                    let ram_total = m.pointer("/ramTotal/inBytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    let ram_avail = m.pointer("/ramAvailable/inBytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    format!(" — RAM: {} / {}", format_bytes(ram_avail), format_bytes(ram_total))
                })
                .unwrap_or_default();

            let disk_info = state
                .pointer(&format!("/nodeDisk/{}", node_id))
                .map(|d| {
                    let disk_avail = d.pointer("/available/inBytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    let disk_total = d.pointer("/total/inBytes").and_then(|v| v.as_u64()).unwrap_or(0);
                    format!(" — Disk: {} / {}", format_bytes(disk_avail), format_bytes(disk_total))
                })
                .unwrap_or_default();

            nodes.push(format!(
                "  📱 {} ({}) [{}…]{}{}",
                name, chip, short_id, mem_info, disk_info
            ));
        }
    }
    nodes
}

/// Default directory where the exo repo is cloned.
fn exo_repo_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".rustyclaw").join("exo")
}

/// Try to locate an exo source checkout.
/// Checks `~/.rustyclaw/exo/` first, then `~/exo/`.
fn find_exo_repo() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = PathBuf::from(home);

    let candidates = [
        home.join(".rustyclaw").join("exo"),
        home.join("exo"),
    ];
    for p in &candidates {
        if p.join("pyproject.toml").exists() && p.join("src").join("exo").exists() {
            return Some(p.clone());
        }
    }
    None
}

/// Log file for the background exo process.
fn exo_log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let log_dir = PathBuf::from(home).join(".rustyclaw");
    let _ = std::fs::create_dir_all(&log_dir);
    log_dir.join("exo.log")
}

/// Check if the exo source repo has been cloned (at any known location).
fn is_exo_cloned() -> bool {
    find_exo_repo().is_some()
}

/// Check whether `uv` is available on PATH.
fn is_uv_installed() -> bool {
    Command::new("which")
        .arg("uv")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if `node` (Node.js) is available on PATH.
fn is_node_installed() -> bool {
    Command::new("which")
        .arg("node")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if the dashboard has been built.
fn is_dashboard_built() -> bool {
    find_exo_repo()
        .map(|r| r.join("dashboard").join("build").exists())
        .unwrap_or(false)
}

/// Search for the `exo` binary in known locations.
///
/// Search order:
///   1. `<repo>/.venv/bin/exo` for each known repo location
///   2. `which exo` (system PATH)
fn find_exo_bin() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let home = PathBuf::from(home);

    // Check venvs inside known repo locations
    let venv_candidates = [
        home.join(".rustyclaw").join("exo").join(".venv").join("bin").join("exo"),
        home.join("exo").join(".venv").join("bin").join("exo"),
    ];
    for p in &venv_candidates {
        if p.exists() {
            return Some(p.clone());
        }
    }

    // Fall back to PATH
    if let Ok(out) = Command::new("which").arg("exo").output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

/// Check if exo is installed anywhere we can find it.
fn is_exo_installed() -> bool {
    find_exo_bin().is_some()
}

/// Check if exo is currently running.
/// Looks for the Python exo process or `uv run exo`, then falls back to
/// probing the API.
fn is_exo_running() -> bool {
    let proc_check = Command::new("sh")
        .arg("-c")
        .arg("pgrep -f '[e]xo\\.main' >/dev/null 2>&1 || pgrep -f '[u]v run exo' >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if proc_check {
        return true;
    }
    exo_api("GET", "/node_id").is_ok()
}

/// Make an HTTP request to the local exo API (default port 52415).
fn exo_api(method: &str, path: &str) -> Result<String, String> {
    exo_api_port(method, path, 52415, None)
}

/// Make an HTTP request to the exo API on a specific port with optional body.
fn exo_api_port(
    method: &str,
    path: &str,
    port: u64,
    body: Option<&str>,
) -> Result<String, String> {
    let url = format!("http://localhost:{}{}", port, path);
    let mut script = match method.to_uppercase().as_str() {
        "GET" => format!("curl -sf --max-time 5 '{}'", url),
        "POST" => {
            if let Some(b) = body {
                format!(
                    "curl -sf --max-time 30 -X POST -H 'Content-Type: application/json' -d '{}' '{}'",
                    b.replace('\'', "'\\''"),
                    url
                )
            } else {
                format!("curl -sf --max-time 30 -X POST '{}'", url)
            }
        }
        "DELETE" => format!("curl -sf --max-time 10 -X DELETE '{}'", url),
        _ => format!("curl -sf --max-time 5 -X {} '{}'", method, url),
    };
    script.push_str(" 2>/dev/null");
    sh(&script)
}

// ── Main dispatch ───────────────────────────────────────────────────────────

/// Execute an exo management action.
///
/// This is callable both as a tool (by the agent) and via the `/exo` slash
/// command.  The `workspace_dir` parameter is the RustyClaw workspace root
/// (used for context, but exo itself lives in `~/.rustyclaw/exo/`).
pub fn exec_exo_manage(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("status")
        .to_lowercase();

    let port = args
        .get("port")
        .and_then(|v| v.as_u64())
        .unwrap_or(52415);

    match action.as_str() {
        // ── setup (clone repo, install prereqs, build dashboard) ────
        "setup" | "install" => {
            let mut steps: Vec<String> = Vec::new();

            // 1. Check for uv
            if !is_uv_installed() {
                match sh("curl -LsSf https://astral.sh/uv/install.sh | sh 2>&1") {
                    Ok(msg) => steps.push(format!("✓ uv installed: {}", msg)),
                    Err(e) => return Err(format!("Failed to install uv: {}", e)),
                }
            } else {
                steps.push("✓ uv already installed".into());
            }

            // 2. Check for node/npm
            if !is_node_installed() {
                steps.push("⚠ Node.js not found. Install via: brew install node".into());
            } else {
                steps.push("✓ Node.js available".into());
            }

            // 3. Locate or clone the exo repo
            //    Checks ~/exo/ first, then ~/.rustyclaw/exo/.
            let repo = if let Some(existing) = find_exo_repo() {
                // Pull latest
                match sh(&format!(
                    "cd '{}' && git pull --ff-only 2>&1",
                    existing.display()
                )) {
                    Ok(msg) => steps.push(format!("✓ exo repo ({}) updated: {}", existing.display(), msg)),
                    Err(_) => steps.push(format!("✓ exo repo found at {} (pull skipped)", existing.display())),
                }
                existing
            } else {
                let target = exo_repo_dir();
                let _ = std::fs::create_dir_all(target.parent().unwrap_or(Path::new("/tmp")));
                match sh(&format!(
                    "git clone https://github.com/exo-explore/exo '{}' 2>&1",
                    target.display()
                )) {
                    Ok(msg) => steps.push(format!("✓ Cloned exo repo: {}", msg)),
                    Err(e) => return Err(format!("Failed to clone exo: {}", e)),
                }
                target
            };

            // 4. Ensure Apple Metal Toolchain is available (macOS only)
            //    mlx compiles Metal shader kernels and needs the `metal`
            //    compiler from the toolchain.
            //    `xcrun -f metal` can find the path even when the toolchain
            //    isn't mounted, so we actually try to run `xcrun metal --version`.
            #[cfg(target_os = "macos")]
            {
                let metal_works = Command::new("xcrun")
                    .args(["metal", "--version"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if !metal_works {
                    // Try to install; may need sudo for the mount step.
                    steps.push("⏳ Installing Metal Toolchain (needed by mlx)…".into());
                    match sh("xcodebuild -downloadComponent MetalToolchain 2>&1") {
                        Ok(msg) => {
                            // Verify it actually works now
                            let ok = Command::new("xcrun")
                                .args(["metal", "--version"])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status()
                                .map(|s| s.success())
                                .unwrap_or(false);
                            if ok {
                                steps.push("✓ Metal Toolchain installed".into());
                            } else {
                                steps.push(format!(
                                    "⚠ Metal Toolchain downloaded but cannot mount (permissions).\n  \
                                     Run manually: sudo xcodebuild -downloadComponent MetalToolchain\n  \
                                     Then re-run /exo setup.\n  Output: {}",
                                    msg
                                ));
                                return Ok(steps.join("\n"));
                            }
                        }
                        Err(e) => {
                            steps.push(format!(
                                "⚠ Metal Toolchain install failed: {}\n  \
                                 Try manually: sudo xcodebuild -downloadComponent MetalToolchain",
                                e
                            ));
                            return Ok(steps.join("\n"));
                        }
                    }
                } else {
                    steps.push("✓ Metal Toolchain available".into());
                }
            }

            // 5. Install exo and its dependencies
            //    If a .venv exists in the repo, install there; otherwise
            //    install into system Python.
            {
                let venv_dir = repo.join(".venv");
                let pip_cmd = if venv_dir.exists() {
                    format!(
                        "cd '{}' && VIRTUAL_ENV='{}' uv pip install -e . 2>&1",
                        repo.display(),
                        venv_dir.display()
                    )
                } else {
                    format!(
                        "cd '{}' && uv pip install --system -e . 2>&1",
                        repo.display()
                    )
                };
                match sh(&pip_cmd) {
                    Ok(_) => steps.push("✓ exo installed".into()),
                    Err(e) => return Err(format!("Failed to install exo: {}", e)),
                }
            }

            // 6. Build the dashboard (requires npm)
            if is_node_installed() {
                let dashboard_dir = repo.join("dashboard");
                if dashboard_dir.join("package.json").exists() {
                    match sh(&format!(
                        "cd '{}' && npm install --no-fund --no-audit 2>&1 && npm run build 2>&1",
                        dashboard_dir.display()
                    )) {
                        Ok(_) => steps.push("✓ Dashboard built".into()),
                        Err(e) => steps.push(format!("⚠ Dashboard build failed: {}", e)),
                    }
                } else {
                    steps.push("⚠ dashboard/package.json not found".into());
                }
            } else {
                steps.push("⚠ Skipping dashboard build (Node.js required)".into());
            }

            // 7. Verify exo can be invoked
            let exo_bin = match find_exo_bin() {
                Some(p) => p,
                None => {
                    steps.push("⚠ exo binary not found after install".into());
                    return Ok(steps.join("\n"));
                }
            };
            let verify = Command::new(&exo_bin)
                .arg("--help")
                .current_dir(&repo)
                .output();
            match verify {
                Ok(out) if out.status.success() => {
                    let preview: String = String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" | ");
                    steps.push(format!("✓ exo verified: {}", preview));
                }
                Ok(out) => {
                    let combined: String = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let tail: String = combined
                        .lines()
                        .filter(|l: &&str| !l.trim().is_empty())
                        .rev()
                        .take(5)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    steps.push(format!("⚠ exo --help failed:\n{}", tail));
                }
                Err(e) => steps.push(format!("⚠ exo binary not found: {}", e)),
            }

            Ok(steps.join("\n"))
        }

        // ── start (launch exo cluster node) ─────────────────────────
        "start" | "run" | "serve" => {
            if !is_exo_cloned() {
                return Err(
                    "exo is not set up. Run with action 'setup' first to clone and build.".into(),
                );
            }
            if is_exo_running() {
                return Ok("exo is already running.".into());
            }

            let repo = find_exo_repo().ok_or(
                "exo repo not found. Run with action 'setup' first.".to_string(),
            )?;
            let log_path = exo_log_path();
            let exo_bin = find_exo_bin().ok_or(
                "exo binary not found. Run with action 'setup' first.".to_string(),
            )?;

            // Build the command using the discovered binary
            let mut cmd_parts: Vec<String> = vec![exo_bin.to_string_lossy().into()];

            if port != 52415 {
                cmd_parts.push("--api-port".into());
                cmd_parts.push(port.to_string());
            }
            if args.get("no_worker").and_then(|v| v.as_bool()).unwrap_or(false) {
                cmd_parts.push("--no-worker".into());
            }
            if args.get("offline").and_then(|v| v.as_bool()).unwrap_or(false) {
                cmd_parts.push("--offline".into());
            }
            if args.get("verbose").and_then(|v| v.as_bool()).unwrap_or(false) {
                cmd_parts.push("-v".into());
            }

            // Launch as a detached background process, logging to file.
            let log_file = std::fs::File::create(&log_path)
                .map_err(|e| format!("Cannot create log file {}: {}", log_path.display(), e))?;
            let log_err = log_file
                .try_clone()
                .map_err(|e| format!("Cannot clone log handle: {}", e))?;

            let mut cmd = Command::new(&cmd_parts[0]);
            cmd.args(&cmd_parts[1..])
                .current_dir(&repo)
                .stdin(std::process::Stdio::null())
                .stdout(log_file)
                .stderr(log_err);

            // setsid so the child survives the parent exit
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                unsafe {
                    cmd.pre_exec(|| {
                        libc::setsid();
                        Ok(())
                    });
                }
            }

            cmd.spawn()
                .map_err(|e| format!("Failed to spawn exo: {}", e))?;

            // Poll for startup (up to ~15 seconds)
            for i in 0..15 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if is_exo_running() {
                    return Ok(format!(
                        "exo cluster node started (after ~{}s). Dashboard & API at \
                         http://localhost:{}. Log: {}",
                        i + 1,
                        port,
                        log_path.display()
                    ));
                }
            }

            // Process didn't show up — read the log to report what went wrong.
            let tail = std::fs::read_to_string(&log_path).unwrap_or_default();
            let last_lines: String = tail
                .lines()
                .rev()
                .take(40)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            Err(format!(
                "exo failed to start within 15 seconds. Log ({}):\n{}",
                log_path.display(),
                if last_lines.is_empty() {
                    "(empty — process may have crashed immediately)".into()
                } else {
                    last_lines
                }
            ))
        }

        // ── stop ────────────────────────────────────────────────────
        "stop" => {
            if !is_exo_running() {
                return Ok("exo is not running.".into());
            }
            sh("pkill -INT -f '[e]xo\\.main' 2>/dev/null; \
                pkill -INT -f '[u]v run exo' 2>/dev/null; \
                sleep 1; \
                pkill -f '[e]xo\\.main' 2>/dev/null; \
                pkill -f '[u]v run exo' 2>/dev/null; \
                echo 'exo stopped.'")
        }

        // ── status ──────────────────────────────────────────────────
        "status" => {
            let cloned = is_exo_cloned();
            let installed = is_exo_installed();
            let running = is_exo_running();
            let repo_path = find_exo_repo().unwrap_or_else(|| exo_repo_dir());
            let exo_bin = find_exo_bin()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".into());
            let dashboard_built = is_dashboard_built();

            let mut out: Vec<String> = Vec::new();
            out.push(format!("═══ exo status ═══"));
            out.push(format!("Repo cloned:      {}", if cloned { "✓" } else { "✗" }));
            out.push(format!("Binary installed:  {}", if installed { "✓" } else { "✗" }));
            out.push(format!("Binary path:       {}", exo_bin));
            out.push(format!("Repo path:         {}", repo_path.display()));
            out.push(format!("Dashboard built:   {}", if dashboard_built { "✓" } else { "✗" }));
            out.push(format!("Running:           {}", if running { "✓ yes" } else { "✗ no" }));
            out.push(format!("API port:          {}", port));

            if running {
                let node_id = exo_api_port("GET", "/node_id", port, None)
                    .unwrap_or_else(|_| "unknown".into());
                out.push(format!("Node ID:           {}", node_id));

                // Parse the full state for summaries
                if let Ok(state_raw) = exo_api_port("GET", "/state", port, None) {
                    if let Ok(state) = serde_json::from_str::<Value>(&state_raw) {
                        // Nodes
                        let nodes = parse_node_info(&state);
                        if !nodes.is_empty() {
                            out.push(String::new());
                            out.push("Cluster nodes:".into());
                            out.extend(nodes);
                        }
                        // Instances
                        let instances = parse_instances(&state);
                        if !instances.is_empty() {
                            out.push(String::new());
                            out.push("Active instances:".into());
                            out.extend(instances);
                        }
                        // Runner errors
                        let errors = parse_runner_errors(&state);
                        if !errors.is_empty() {
                            out.push(String::new());
                            out.push("Runner errors:".into());
                            out.extend(errors);
                        }
                        // Downloads summary
                        let dl_summary = parse_downloads_from_state(&state);
                        if dl_summary != "No downloads in progress or completed." {
                            out.push(String::new());
                            out.push("Downloads:".into());
                            // Indent the summary lines
                            for line in dl_summary.lines() {
                                out.push(format!("  {}", line));
                            }
                        }
                    }
                }
            }

            Ok(out.join("\n"))
        }

        // ── models (list available models via API) ──────────────────
        "models" | "list" | "ls" => {
            if !is_exo_running() {
                return Err("exo is not running. Start it with action 'start' first.".into());
            }
            match exo_api_port("GET", "/models", port, None) {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        Ok(serde_json::to_string_pretty(&parsed).unwrap_or(resp))
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(format!("Failed to list models: {}", e)),
            }
        }

        // ── state (cluster state including instances) ───────────────
        "state" | "topology" | "peers" | "cluster" => {
            if !is_exo_running() {
                return Err("exo is not running.".into());
            }
            match exo_api_port("GET", "/state", port, None) {
                Ok(resp) => {
                    if let Ok(state) = serde_json::from_str::<Value>(&resp) {
                        let mut out: Vec<String> = Vec::new();
                        out.push("═══ exo cluster state ═══".into());

                        // Nodes
                        let nodes = parse_node_info(&state);
                        if !nodes.is_empty() {
                            out.push(String::new());
                            out.push("Nodes:".into());
                            out.extend(nodes);
                        }

                        // Topology
                        if let Some(topo) = state.get("topology") {
                            let node_count = topo
                                .get("nodes")
                                .and_then(|n| n.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            out.push(format!("\nTopology: {} node(s)", node_count));
                        }

                        // Active instances
                        let instances = parse_instances(&state);
                        if !instances.is_empty() {
                            out.push(String::new());
                            out.push("Active instances:".into());
                            out.extend(instances);
                        }

                        // Runner errors
                        let errors = parse_runner_errors(&state);
                        if !errors.is_empty() {
                            out.push(String::new());
                            out.push("Runner errors:".into());
                            out.extend(errors);
                        }

                        // Active tasks
                        if let Some(tasks) = state.get("tasks").and_then(|t| t.as_object()) {
                            let active: Vec<String> = tasks
                                .iter()
                                .filter_map(|(id, task)| {
                                    // Look for task type and status
                                    if let Some(dl) = task.get("DownloadModel") {
                                        let status = dl.get("taskStatus")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        let model = dl
                                            .pointer("/shardMetadata/PipelineShardMetadata/modelCard/modelId")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        Some(format!("  📥 Download {} — status: {} (task: {}…)", model, status, &id[..8.min(id.len())]))
                                    } else if let Some(cr) = task.get("CreateRunner") {
                                        let status = cr.get("taskStatus")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown");
                                        Some(format!("  🔧 CreateRunner — status: {} (task: {}…)", status, &id[..8.min(id.len())]))
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if !active.is_empty() {
                                out.push(String::new());
                                out.push("Tasks:".into());
                                out.extend(active);
                            }
                        }

                        // Downloads
                        let dl_summary = parse_downloads_from_state(&state);
                        out.push(String::new());
                        out.push("Downloads:".into());
                        for line in dl_summary.lines() {
                            out.push(format!("  {}", line));
                        }

                        Ok(out.join("\n"))
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(format!("Failed to get cluster state: {}", e)),
            }
        }

        // ── downloads (show download progress) ──────────────────────
        "downloads" | "progress" | "dl" => {
            if !is_exo_running() {
                return Err("exo is not running. Start it with action 'start' first.".into());
            }
            match exo_api_port("GET", "/state", port, None) {
                Ok(resp) => {
                    if let Ok(state) = serde_json::from_str::<Value>(&resp) {
                        let mut out: Vec<String> = Vec::new();
                        out.push("═══ exo downloads ═══".into());
                        out.push(parse_downloads_from_state(&state));

                        // Also show runner errors since they often relate to
                        // downloaded models failing to load
                        let errors = parse_runner_errors(&state);
                        if !errors.is_empty() {
                            out.push(String::new());
                            out.push("Runner errors (may affect loaded models):".into());
                            out.extend(errors);
                        }

                        Ok(out.join("\n"))
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(format!("Failed to query state: {}", e)),
            }
        }

        // ── preview (preview instance placements for a model) ───────
        "preview" | "placements" => {
            if !is_exo_running() {
                return Err("exo is not running.".into());
            }
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model (e.g. 'llama-3.2-1b')")?;

            let path = format!("/instance/previews?model_id={}", model);
            match exo_api_port("GET", &path, port, None) {
                Ok(resp) => {
                    if let Ok(parsed) = serde_json::from_str::<Value>(&resp) {
                        Ok(serde_json::to_string_pretty(&parsed).unwrap_or(resp))
                    } else {
                        Ok(resp)
                    }
                }
                Err(e) => Err(format!(
                    "Failed to preview placements for '{}': {}",
                    model, e
                )),
            }
        }

        // ── load (create a model instance) ──────────────────────────
        "load" | "add" | "download" | "pull" | "create-instance" => {
            if !is_exo_running() {
                return Err("exo is not running. Start it with action 'start' first.".into());
            }
            let model = args
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: model (e.g. 'llama-3.2-1b')")?;

            // First get a valid placement preview
            let preview_path = format!("/instance/previews?model_id={}", model);
            let previews_resp = exo_api_port("GET", &preview_path, port, None)
                .map_err(|e| format!("Failed to get placements for '{}': {}", model, e))?;

            let previews: Value = serde_json::from_str(&previews_resp)
                .map_err(|e| format!("Invalid preview response: {}", e))?;

            // Pick the first valid placement (no error)
            let instance = previews
                .get("previews")
                .and_then(|p| p.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|p| p.get("error").map_or(true, |e| e.is_null()))
                })
                .and_then(|p| p.get("instance"))
                .ok_or(format!(
                    "No valid placement found for '{}'. The model may be too large \
                     for available memory, or the model ID may be invalid.",
                    model
                ))?;

            // Create the instance
            let body = json!({ "instance": instance }).to_string();
            match exo_api_port("POST", "/instance", port, Some(&body)) {
                Ok(resp) => Ok(format!("Model '{}' instance created:\n{}", model, resp)),
                Err(e) => Err(format!(
                    "Failed to create instance for '{}': {}",
                    model, e
                )),
            }
        }

        // ── unload (delete a model instance) ────────────────────────
        "unload" | "remove" | "rm" | "delete" | "delete-instance" => {
            if !is_exo_running() {
                return Err("exo is not running.".into());
            }

            if let Some(instance_id) = args.get("instance_id").and_then(|v| v.as_str()) {
                let path = format!("/instance/{}", instance_id);
                match exo_api_port("DELETE", &path, port, None) {
                    Ok(resp) => Ok(format!("Instance '{}' deleted: {}", instance_id, resp)),
                    Err(e) => Err(format!(
                        "Failed to delete instance '{}': {}",
                        instance_id, e
                    )),
                }
            } else if let Some(model) = args.get("model").and_then(|v| v.as_str()) {
                let state_resp = exo_api_port("GET", "/state", port, None)
                    .map_err(|e| format!("Failed to query state: {}", e))?;
                Ok(format!(
                    "To unload model '{}', provide the instance_id from the cluster \
                     state. Current state:\n{}",
                    model, state_resp
                ))
            } else {
                Err("Missing required parameter: instance_id or model.".into())
            }
        }

        // ── update (git pull the exo repo) ──────────────────────────
        "update" | "upgrade" => {
            if !is_exo_cloned() {
                return Err("exo is not set up. Run with action 'setup' first.".into());
            }
            let repo = find_exo_repo().ok_or(
                "exo repo not found. Run with action 'setup' first.".to_string(),
            )?;
            let mut results = Vec::new();

            match sh(&format!("cd '{}' && git pull 2>&1", repo.display())) {
                Ok(msg) => results.push(format!("✓ git pull: {}", msg)),
                Err(e) => results.push(format!("⚠ git pull failed: {}", e)),
            }

            if is_node_installed() {
                let dashboard_dir = repo.join("dashboard");
                if dashboard_dir.join("package.json").exists() {
                    match sh(&format!(
                        "cd '{}' && npm install --no-fund --no-audit 2>&1 && npm run build 2>&1",
                        dashboard_dir.display()
                    )) {
                        Ok(_) => results.push("✓ Dashboard rebuilt".into()),
                        Err(e) => results.push(format!("⚠ Dashboard build failed: {}", e)),
                    }
                }
            }

            Ok(results.join("\n"))
        }

        // ── log (show recent log output) ────────────────────────────
        "log" | "logs" => {
            let log_path = exo_log_path();
            match std::fs::read_to_string(&log_path) {
                Ok(content) => {
                    let n = args
                        .get("lines")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(50) as usize;
                    let lines: Vec<&str> = content.lines().collect();
                    let start = if lines.len() > n { lines.len() - n } else { 0 };
                    Ok(lines[start..].join("\n"))
                }
                Err(_) => Ok(format!("No log file found at {}", log_path.display())),
            }
        }

        _ => Err(format!(
            "Unknown exo action: '{}'. Valid actions: setup, start, stop, status, \
             models, state, downloads, preview, load, unload, update, log.",
            action
        )),
    }
}
