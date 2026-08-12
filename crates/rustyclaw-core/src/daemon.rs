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

use crate::ignore::Ignore;
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
    crate::persist::write_atomically(&path, pid.to_string().as_bytes())
        .with_context(|| format!("Failed to write PID file {}", path.display()))
}

/// Read the stored PID, if the file exists and is valid.
pub fn read_pid(settings_dir: &Path) -> Option<u32> {
    let path = pid_path(settings_dir);
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Remove the PID file, but only while it still names `pid`.
///
/// The record says which process *is* the gateway, so only that process may
/// retract it. Removing unconditionally meant a gateway that exited for any
/// reason deleted whatever record it found — including a healthy gateway's.
/// A second one started against an occupied port did exactly that: it
/// overwrote the record on the way in and deleted it on the way out, and the
/// gateway still serving connections became invisible to `gateway status`
/// and unreachable by `gateway stop`.
pub fn remove_pid(settings_dir: &Path, pid: u32) {
    let path = pid_path(settings_dir);
    match read_pid(settings_dir) {
        Some(recorded) if recorded == pid => fs::remove_file(&path).ignore(),
        Some(recorded) => tracing::warn!(
            path = %path.display(),
            recorded,
            ours = pid,
            "Leaving the PID file alone: it names another process"
        ),
        // Already gone, or unreadable — either way there is nothing of ours
        // to retract.
        None => {}
    }
}

/// Check whether a process with the given PID is alive.
///
/// A zombie does not count. On Unix an exited child stays in the process
/// table until its parent reaps it, and [`start`] spawns the gateway with
/// `Command::spawn` and drops the `Child` without ever waiting. That is
/// harmless for the CLI, which exits moments later and hands the child to
/// init — but the desktop client calls [`start`] too and then stays running
/// for hours. Asking `sysinfo` whether the PID exists finds the zombie and
/// says yes, so a gateway that died at startup would keep reading as
/// `Running`: the status badge lies, `start` refuses because "it is already
/// running", and `stop` sits through its full two-second wait for a process
/// that is already gone.
///
/// `Dead` is included for completeness; Linux effectively never reports it,
/// but a process in either state is one no client can talk to.
pub fn is_process_alive(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
    );
    match sys.process(Pid::from_u32(pid)) {
        Some(process) => !matches!(
            process.status(),
            sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
        ),
        None => false,
    }
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
/// executable.  We pass `--ssh-listen` and any config/settings-dir
/// flags, then redirect stdout/stderr to the log file.
pub fn start(
    settings_dir: &Path,
    ssh_listen: &str,
    extra_args: &[String],
    model_api_key: Option<&str>,
    vault_password: Option<&str>,
    tls_cert: Option<&Path>,
    tls_key: Option<&Path>,
    log_level: Option<&str>,
) -> Result<u32> {
    // If already running, bail.
    if let DaemonStatus::Running { pid } = status(settings_dir) {
        anyhow::bail!("Gateway is already running (PID {})", pid);
    }

    // Clean up a stale PID file. Reached only when `status` said the
    // recorded process is not running, so whatever is in the file is the
    // stale record this is entitled to clear.
    if let Some(stale) = read_pid(settings_dir) {
        remove_pid(settings_dir, stale);
    }

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
        .arg("--ssh-listen")
        .arg(ssh_listen)
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

    // Set the log level for the gateway process via RUST_LOG environment variable.
    if let Some(level) = log_level {
        cmd.env("RUST_LOG", level);
        cmd.env("RUSTYCLAW_LOG", level);
    }

    // Pass TLS certificate and key paths for WSS support.
    if let Some(cert) = tls_cert {
        cmd.arg("--tls-cert").arg(cert);
    }
    if let Some(key) = tls_key {
        cmd.arg("--tls-key").arg(key);
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

/// Run the `rustyclaw-gateway` binary in the foreground.
///
/// Unlike [`start`], this does not detach or redirect output: the gateway
/// inherits the current terminal (so it can prompt for the vault password and
/// stream logs to the console) and this call blocks until the gateway exits.
///
/// `args` are passed through verbatim after the `run` subcommand (e.g.
/// `--bind`, `--port`, `--listen`). TLS paths and the settings dir are added
/// automatically, mirroring [`start`].
pub fn run_foreground(
    settings_dir: &Path,
    args: &[String],
    tls_cert: Option<&Path>,
    tls_key: Option<&Path>,
    log_level: Option<&str>,
) -> Result<std::process::ExitStatus> {
    let gateway_bin = resolve_gateway_binary()?;

    let mut cmd = Command::new(&gateway_bin);
    cmd.arg("run").arg("--settings-dir").arg(settings_dir);

    // Set the log level for the gateway process via RUST_LOG environment variable.
    if let Some(level) = log_level {
        cmd.env("RUST_LOG", level);
        cmd.env("RUSTYCLAW_LOG", level);
    }

    for a in args {
        cmd.arg(a);
    }

    if let Some(cert) = tls_cert {
        cmd.arg("--tls-cert").arg(cert);
    }
    if let Some(key) = tls_key {
        cmd.arg("--tls-key").arg(key);
    }

    // Foreground: inherit stdio and wait for the gateway to exit.
    cmd.status()
        .with_context(|| format!("Failed to run {}", gateway_bin.display()))
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
                    remove_pid(settings_dir, pid);
                    return Ok(StopResult::Stopped { pid });
                }
            }
            // Process still alive after 2s — it may be shutting down slowly.
            // Remove PID file anyway; the OS will finish cleanup.
            remove_pid(settings_dir, pid);
            Ok(StopResult::Stopped { pid })
        }
        DaemonStatus::Stale { pid } => {
            remove_pid(settings_dir, pid);
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
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo_pid]), true);
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

/// Find a sibling RustyClaw binary by name.  Checks:
/// 1. Next to the current executable (same directory).
/// 2. On `$PATH` via the `which` crate (cross-platform).
///
/// `name` is the base name without an extension (e.g. `"rustyclaw-gateway"`,
/// `"rustyclaw-tui"`); `.exe` is appended automatically on Windows.
pub fn resolve_binary(name: &str) -> Result<PathBuf> {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };

    // 1. Same directory as the running binary.
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let candidate = dir.join(&file_name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    // 2. On $PATH.
    if let Ok(path) = which::which(&file_name) {
        return Ok(path);
    }

    anyhow::bail!(
        "Could not find the `{name}` binary.\n\
         Make sure it is installed or built (`cargo build`) and on your PATH."
    )
}

/// Find the gateway binary (see [`resolve_binary`]).
fn resolve_gateway_binary() -> Result<PathBuf> {
    resolve_binary("rustyclaw-gateway")
}

/// Resolve a sibling binary by `name` and run it in the foreground, passing
/// `args` verbatim. Inherits stdio (so terminal/GUI clients work) and blocks
/// until the process exits.
pub fn run_foreground_named(name: &str, args: &[String]) -> Result<std::process::ExitStatus> {
    let bin = resolve_binary(name)?;
    Command::new(&bin)
        .args(args)
        .status()
        .with_context(|| format!("Failed to run {}", bin.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An exited-but-unreaped child is not a running gateway.
    ///
    /// This is the desktop client's case, not the CLI's: `start` drops the
    /// `Child` without waiting, and a long-lived parent therefore accumulates
    /// zombies. `sysinfo` still finds the PID, so the naive existence check
    /// reported a dead gateway as `Running` — the panel showed a stale PID,
    /// Start stayed disabled, and Stop waited out two seconds for nothing.
    ///
    /// Unix-only because zombies are: Windows drops an exited process from
    /// the table outright, so there is nothing to misread.
    #[cfg(unix)]
    #[test]
    fn a_zombie_child_does_not_count_as_alive() {
        // Deliberately not waited on until the assertions are done — the
        // unreaped exited child is the condition under test.
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("spawning /bin/sh");
        let pid = child.id();

        // Wait for the zombie state rather than assuming a duration: the PID
        // has to still be in the table (unreaped) *and* reported as exited.
        let mut became_zombie = false;
        for _ in 0..200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let mut sys = System::new();
            sys.refresh_processes(
                sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
                true,
            );
            let zombie = match sys.process(Pid::from_u32(pid)) {
                Some(p) => matches!(p.status(), sysinfo::ProcessStatus::Zombie),
                None => false,
            };
            if zombie {
                became_zombie = true;
                break;
            }
        }
        assert!(
            became_zombie,
            "child never reached the zombie state, so this test proves nothing"
        );

        assert!(
            !is_process_alive(pid),
            "an exited child still in the process table is not a running gateway"
        );

        // A zombie is `Stale`, not `Running`, which is what lets `start`
        // clear the record and `stop` skip its two-second wait loop.
        let dir = tempfile::tempdir().unwrap();
        write_pid(dir.path(), pid).unwrap();
        assert!(matches!(status(dir.path()), DaemonStatus::Stale { .. }));

        // Reap, so the test does not leave the zombie behind for whatever
        // runs next in this process.
        child.wait().expect("reaping the child");
    }

    /// A process that really is running still reads as alive — the new guard
    /// must not have turned the check into a constant `false`.
    #[test]
    fn our_own_process_counts_as_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    /// The record is a claim to *be* the gateway, so only the process it
    /// names may retract it. A gateway that exits for any reason used to
    /// delete whatever record it found — and a second one started against an
    /// occupied port overwrote the record on the way in and deleted it on the
    /// way out, leaving the gateway still serving connections invisible to
    /// `gateway status` and unreachable by `gateway stop`.
    #[test]
    fn remove_pid_leaves_another_process_record_alone() {
        let dir = tempfile::tempdir().unwrap();
        write_pid(dir.path(), 4242).unwrap();

        remove_pid(dir.path(), 9999);

        assert_eq!(
            read_pid(dir.path()),
            Some(4242),
            "a record we did not write is not ours to delete"
        );
    }

    #[test]
    fn remove_pid_retracts_our_own_record() {
        let dir = tempfile::tempdir().unwrap();
        write_pid(dir.path(), 4242).unwrap();

        remove_pid(dir.path(), 4242);

        assert_eq!(read_pid(dir.path()), None);
        assert!(!pid_path(dir.path()).exists());
    }

    /// No file is not a failure: a run that bailed before claiming the record
    /// still calls this on the way out.
    #[test]
    fn remove_pid_on_a_missing_file_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        remove_pid(dir.path(), 4242);
        assert!(!pid_path(dir.path()).exists());
    }
}
