//! Gateway daemon management — PID file, spawn, stop, status.
//!
//! The `gateway start` command spawns the `rustyclaw-gateway` binary as a
//! detached background process, writes a PID file to
//! `<settings_dir>/gateway.pid`, and stores the log path alongside it.
//!
//! `gateway stop` reads that PID file and terminates the process.
//! `gateway restart` does stop-then-start.
//! `gateway status` checks if the recorded PID is still alive.
//!
//! All process management uses `sysinfo` and `which` for cross-platform
//! support (macOS, Linux, Windows) with no `cfg(unix)` gates.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use sysinfo::{Pid, Signal, System};

// ── PID file helpers ────────────────────────────────────────────────────────

/// Returns the path to the PID file: `<settings_dir>/gateway.pid`.
pub fn pid_path(settings_dir: &Path) -> PathBuf {
    settings_dir.join("gateway.pid")
}

/// Returns the path to the gateway log file: `<settings_dir>/logs/gateway.log`.
pub fn log_path(settings_dir: &Path) -> PathBuf {
    settings_dir.join("logs").join("gateway.log")
}

/// Write a PID to the PID file.
pub fn write_pid(settings_dir: &Path, pid: u32) -> Result<()> {
    let path = pid_path(settings_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, pid.to_string())
        .with_context(|| format!("Failed to write PID file {}", path.display()))
}

/// Read the stored PID, if the file exists and is valid.
pub fn read_pid(settings_dir: &Path) -> Option<u32> {
    let path = pid_path(settings_dir);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Remove the PID file.
pub fn remove_pid(settings_dir: &Path) {
    let path = pid_path(settings_dir);
    let _ = fs::remove_file(&path);
}

/// Check whether a process with the given PID is alive.
pub fn is_process_alive(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
    );
    sys.process(Pid::from_u32(pid)).is_some()
}

// ── High-level daemon operations ────────────────────────────────────────────

/// Status of the gateway daemon.
#[derive(Debug, Clone)]
pub enum DaemonStatus {
    /// Running with the given PID.
    Running { pid: u32 },
    /// PID file exists but the process is dead.
    Stale { pid: u32 },
    /// No PID file — not running.
    Stopped,
}

/// Check the current daemon status.
pub fn status(settings_dir: &Path) -> DaemonStatus {
    match read_pid(settings_dir) {
        Some(pid) => {
            if is_process_alive(pid) {
                DaemonStatus::Running { pid }
            } else {
                DaemonStatus::Stale { pid }
            }
        }
        None => DaemonStatus::Stopped,
    }
}

/// Spawn the `rustyclaw-gateway` binary as a background process.
///
/// The gateway binary is expected to be on `$PATH` or next to the current
/// executable.  We pass `--port`, `--bind`, and any config/settings-dir
/// flags, then redirect stdout/stderr to the log file.
pub fn start(
    settings_dir: &Path,
    port: u16,
    bind: &str,
    extra_args: &[String],
    model_api_key: Option<&str>,
    vault_password: Option<&str>,
) -> Result<u32> {
    // If already running, bail.
    if let DaemonStatus::Running { pid } = status(settings_dir) {
        anyhow::bail!("Gateway is already running (PID {})", pid);
    }

    // Clean up stale PID file.
    remove_pid(settings_dir);

    // Resolve gateway binary path — look next to our own binary first.
    let gateway_bin = resolve_gateway_binary()?;

    // Ensure log directory exists.
    let log = log_path(settings_dir);
    if let Some(parent) = log.parent() {
        fs::create_dir_all(parent)?;
    }

    let log_file = fs::File::create(&log)
        .with_context(|| format!("Failed to create gateway log at {}", log.display()))?;
    let log_stderr = log_file
        .try_clone()
        .context("Failed to clone log file handle")?;

    let mut cmd = Command::new(&gateway_bin);
    cmd.arg("run")
        .arg("--port")
        .arg(port.to_string())
        .arg("--bind")
        .arg(bind)
        .arg("--settings-dir")
        .arg(settings_dir)
        .stdout(log_file)
        .stderr(log_stderr);

    // Pass the model API key via an environment variable so the gateway
    // never needs direct access to the secrets vault.
    if let Some(key) = model_api_key {
        cmd.env("RUSTYCLAW_MODEL_API_KEY", key);
    }

    // Pass the vault password so the gateway can unlock the secrets vault.
    if let Some(pw) = vault_password {
        cmd.env("RUSTYCLAW_VAULT_PASSWORD", pw);
    }

    for a in extra_args {
        cmd.arg(a);
    }

    // Platform-specific detach so the child survives our exit.
    detach_child(&mut cmd);

    let child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn {}", gateway_bin.display()))?;

    let pid = child.id();
    write_pid(settings_dir, pid)?;

    Ok(pid)
}

/// Stop a running gateway by terminating the process.
pub fn stop(settings_dir: &Path) -> Result<StopResult> {
    match status(settings_dir) {
        DaemonStatus::Running { pid } => {
            kill_process(pid)?;
            // Wait briefly for the process to exit.
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if !is_process_alive(pid) {
                    remove_pid(settings_dir);
                    return Ok(StopResult::Stopped { pid });
                }
            }
            // Process still alive after 2s — it may be shutting down slowly.
            // Remove PID file anyway; the OS will finish cleanup.
            remove_pid(settings_dir);
            Ok(StopResult::Stopped { pid })
        }
        DaemonStatus::Stale { pid } => {
            remove_pid(settings_dir);
            Ok(StopResult::WasStale { pid })
        }
        DaemonStatus::Stopped => Ok(StopResult::WasNotRunning),
    }
}

#[derive(Debug)]
pub enum StopResult {
    Stopped { pid: u32 },
    WasStale { pid: u32 },
    WasNotRunning,
}

/// Terminate a process by PID using `sysinfo`.
/// Sends SIGTERM on Unix, TerminateProcess on Windows.
fn kill_process(pid: u32) -> Result<()> {
    let sysinfo_pid = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo_pid]),
        true,
    );
    let process = sys
        .process(sysinfo_pid)
        .context(format!("Process {} not found", pid))?;

    if !process.kill_with(Signal::Term).unwrap_or(false) {
        // Fallback: hard kill if graceful signal unsupported (e.g. Windows
        // doesn't have SIGTERM — kill_with(Term) returns false).
        process.kill();
    }
    Ok(())
}

/// Configure a `Command` to detach the child from the parent session.
#[cfg(unix)]
fn detach_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Create a new process group so the child isn't killed when the
    // parent's terminal closes.
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach_child(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP (0x200) | DETACHED_PROCESS (0x08)
    cmd.creation_flags(0x0000_0208);
}

#[cfg(not(any(unix, windows)))]
fn detach_child(_cmd: &mut Command) {
    // No detach on unknown platforms — the child may be tied to our terminal.
}

/// Find the gateway binary.  Checks:
/// 1. Next to the current executable (same directory).
/// 2. On `$PATH` via the `which` crate (cross-platform).
fn resolve_gateway_binary() -> Result<PathBuf> {
    let name = if cfg!(windows) {
        "rustyclaw-gateway.exe"
    } else {
        "rustyclaw-gateway"
    };

    // 1. Same directory as the running binary.
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    // 2. On $PATH.
    if let Ok(path) = which::which(name) {
        return Ok(path);
    }

    anyhow::bail!(
        "Could not find the `rustyclaw-gateway` binary.\n\
         Make sure it is installed or built (`cargo build`) and on your PATH."
    )
}
