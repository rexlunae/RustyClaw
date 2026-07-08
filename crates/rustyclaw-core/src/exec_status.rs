//! Live status registry for foreground exec child processes.
//!
//! While `execute_command` waits on a child process, the child registers
//! itself here so the gateway can sample its CPU usage and scheduler state
//! (running, sleeping, blocked on I/O, paused, …) and stream that to
//! clients, and so clients can control it (pause/resume/stop/kill).
//!
//! Only processes registered here can be signalled through [`control`] —
//! the registry doubles as an allowlist so a client frame can never
//! signal an arbitrary PID on the host.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};

// ── Control actions ─────────────────────────────────────────────────────────

/// Control actions a client can apply to a running exec process.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display, strum::EnumString,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ProcessControlAction {
    /// Suspend the process (SIGSTOP). The exec timeout clock is frozen
    /// while paused so a paused process cannot time out.
    Pause,
    /// Resume a paused process (SIGCONT).
    Resume,
    /// Ask the process to terminate gracefully (SIGTERM).
    Stop,
    /// Force-kill the process (SIGKILL).
    Kill,
}

// ── Status snapshots ────────────────────────────────────────────────────────

/// A point-in-time snapshot of one registered exec process.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecProcessStatus {
    pub pid: u32,
    pub command: String,
    /// Time since the process was spawned.
    pub elapsed_ms: u64,
    /// CPU usage as a percentage of one core (may exceed 100 on
    /// multi-threaded work). None on the first sample, before a
    /// usage delta exists.
    pub cpu_percent: Option<f32>,
    /// Resident memory in bytes.
    pub memory_bytes: Option<u64>,
    /// Human-readable scheduler state ("running", "sleeping",
    /// "blocked on I/O", "paused", …).
    pub state: Option<String>,
    /// Whether the user paused this process via [`control`].
    pub paused: bool,
}

// ── Registry internals ──────────────────────────────────────────────────────

struct Entry {
    command: String,
    started: Instant,
    paused: bool,
}

struct Registry {
    entries: HashMap<u32, Entry>,
    /// Persistent System so successive refreshes yield real CPU deltas.
    system: System,
    /// PIDs that have been sampled at least once (their next sample has
    /// a meaningful CPU percentage).
    sampled_once: HashMap<u32, ()>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            entries: HashMap::new(),
            system: System::new(),
            sampled_once: HashMap::new(),
        })
    })
}

/// RAII guard returned by [`register`]; dropping it removes the process
/// from the registry (and thus from status sampling and control).
pub struct ExecGuard {
    pid: u32,
}

impl Drop for ExecGuard {
    fn drop(&mut self) {
        if let Ok(mut reg) = registry().lock() {
            reg.entries.remove(&self.pid);
            reg.sampled_once.remove(&self.pid);
        }
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Register a foreground exec child for status sampling and control.
pub fn register(pid: u32, command: &str) -> ExecGuard {
    if let Ok(mut reg) = registry().lock() {
        reg.entries.insert(
            pid,
            Entry {
                command: command.to_string(),
                started: Instant::now(),
                paused: false,
            },
        );
    }
    ExecGuard { pid }
}

/// Whether the user paused this process. Exec loops freeze their
/// timeout/yield deadlines while this returns true.
pub fn is_paused(pid: u32) -> bool {
    registry()
        .lock()
        .ok()
        .and_then(|reg| reg.entries.get(&pid).map(|e| e.paused))
        .unwrap_or(false)
}

/// Sample all registered processes: refresh their CPU/memory/state via
/// sysinfo and return a snapshot per live entry.
pub fn sample_active() -> Vec<ExecProcessStatus> {
    let Ok(mut reg) = registry().lock() else {
        return Vec::new();
    };
    if reg.entries.is_empty() {
        return Vec::new();
    }

    let pids: Vec<Pid> = reg.entries.keys().map(|&p| Pid::from_u32(p)).collect();
    reg.system
        .refresh_processes(ProcessesToUpdate::Some(&pids), true);

    let mut out = Vec::with_capacity(reg.entries.len());
    let Registry {
        entries,
        system,
        sampled_once,
    } = &mut *reg;
    for (&pid, entry) in entries.iter() {
        let proc_info = system.process(Pid::from_u32(pid));
        let first_sample = sampled_once.insert(pid, ()).is_none();
        let (cpu_percent, memory_bytes, state) = match proc_info {
            Some(p) => (
                // The first refresh has no prior measurement to diff
                // against, so its CPU value is meaningless — hide it.
                (!first_sample).then(|| p.cpu_usage()),
                Some(p.memory()),
                Some(if entry.paused {
                    "paused".to_string()
                } else {
                    state_label(p.status()).to_string()
                }),
            ),
            None => (None, None, None),
        };
        out.push(ExecProcessStatus {
            pid,
            command: entry.command.clone(),
            elapsed_ms: entry.started.elapsed().as_millis() as u64,
            cpu_percent,
            memory_bytes,
            state,
            paused: entry.paused,
        });
    }
    out.sort_by_key(|s| s.elapsed_ms);
    out
}

/// Apply a control action to a registered exec process.
///
/// Returns a short human-readable confirmation, or an error string if the
/// PID is not registered or the signal could not be delivered.
pub fn control(pid: u32, action: ProcessControlAction) -> Result<String, String> {
    // Verify registration first — this is the safety boundary that stops
    // a client frame from signalling arbitrary host processes.
    {
        let reg = registry()
            .lock()
            .map_err(|_| "process registry lock poisoned".to_string())?;
        if !reg.entries.contains_key(&pid) {
            return Err(format!("no controllable process with pid {pid}"));
        }
    }

    send_signal(pid, action)?;

    if let Ok(mut reg) = registry().lock() {
        if let Some(entry) = reg.entries.get_mut(&pid) {
            match action {
                ProcessControlAction::Pause => entry.paused = true,
                ProcessControlAction::Resume => entry.paused = false,
                ProcessControlAction::Stop | ProcessControlAction::Kill => {}
            }
        }
    }

    Ok(match action {
        ProcessControlAction::Pause => format!("paused process {pid}"),
        ProcessControlAction::Resume => format!("resumed process {pid}"),
        ProcessControlAction::Stop => format!("sent SIGTERM to process {pid}"),
        ProcessControlAction::Kill => format!("killed process {pid}"),
    })
}

// ── Platform signal delivery ────────────────────────────────────────────────

#[cfg(unix)]
fn send_signal(pid: u32, action: ProcessControlAction) -> Result<(), String> {
    let sig = match action {
        ProcessControlAction::Pause => libc::SIGSTOP,
        ProcessControlAction::Resume => libc::SIGCONT,
        ProcessControlAction::Stop => libc::SIGTERM,
        ProcessControlAction::Kill => libc::SIGKILL,
    };
    // Exec children are spawned as process-group leaders, so signal the
    // whole group to reach the `sh -c` child's own children. Fall back to
    // the single PID for processes not leading a group.
    // SAFETY: `pid` is a valid OS process ID obtained from `Child::id()`;
    // on all supported platforms PIDs fit in `i32`. `sig` is a valid
    // signal constant from `libc`. Negating the PID signals the process
    // group, which is safe even if the group does not exist (returns -1).
    let group = unsafe { libc::kill(-(pid as i32), sig) };
    if group == 0 {
        return Ok(());
    }
    // SAFETY: same invariants as above; here we signal the single process.
    let single = unsafe { libc::kill(pid as i32, sig) };
    if single == 0 {
        return Ok(());
    }
    Err(format!(
        "failed to signal process {pid}: {}",
        std::io::Error::last_os_error()
    ))
}

#[cfg(not(unix))]
fn send_signal(pid: u32, action: ProcessControlAction) -> Result<(), String> {
    // Windows has no SIGSTOP/SIGCONT/SIGTERM equivalents that sysinfo can
    // deliver; only hard kill is supported.
    match action {
        ProcessControlAction::Kill | ProcessControlAction::Stop => {
            let mut sys = System::new();
            let target = Pid::from_u32(pid);
            sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
            match sys.process(target) {
                Some(p) if p.kill() => Ok(()),
                Some(_) => Err(format!("failed to kill process {pid}")),
                None => Err(format!("process {pid} not found")),
            }
        }
        ProcessControlAction::Pause | ProcessControlAction::Resume => {
            Err("pause/resume is not supported on this platform".to_string())
        }
    }
}

/// Map a sysinfo scheduler state to a short human-readable label.
fn state_label(status: sysinfo::ProcessStatus) -> &'static str {
    use sysinfo::ProcessStatus::*;
    match status {
        Run => "running",
        Sleep => "sleeping",
        Idle => "idle",
        Stop | Suspended => "paused",
        Zombie => "zombie",
        UninterruptibleDiskSleep => "blocked on I/O",
        LockBlocked => "blocked on lock",
        Parked => "parked",
        Tracing => "traced",
        Dead => "dead",
        Wakekill | Waking => "waking",
        Unknown(_) => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_drop_removes_entry() {
        let guard = register(u32::MAX - 7, "sleep 100");
        assert!(
            sample_active().iter().any(|s| s.pid == u32::MAX - 7),
            "registered pid should appear in samples"
        );
        drop(guard);
        assert!(
            !sample_active().iter().any(|s| s.pid == u32::MAX - 7),
            "dropped guard should remove the entry"
        );
    }

    #[test]
    fn control_rejects_unregistered_pid() {
        let err = control(u32::MAX - 13, ProcessControlAction::Kill).unwrap_err();
        assert!(err.contains("no controllable process"), "got: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn pause_and_resume_real_child() {
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        let guard = register(pid, "sleep 30");

        control(pid, ProcessControlAction::Pause).expect("pause");
        assert!(is_paused(pid));
        let paused = sample_active()
            .into_iter()
            .find(|s| s.pid == pid)
            .expect("sampled");
        assert_eq!(paused.state.as_deref(), Some("paused"));

        control(pid, ProcessControlAction::Resume).expect("resume");
        assert!(!is_paused(pid));

        control(pid, ProcessControlAction::Kill).expect("kill");
        drop(guard);
        // Reap the child so the test process doesn't leave a zombie.
        let mut child = child;
        let _ = child.wait();
    }

    #[test]
    fn action_wire_format_is_snake_case() {
        assert_eq!(ProcessControlAction::Pause.to_string(), "pause");
        assert_eq!(
            "kill".parse::<ProcessControlAction>().unwrap(),
            ProcessControlAction::Kill
        );
    }
}
