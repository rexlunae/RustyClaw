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
    /// "blocked on I/O", "paused", "exited", …).
    pub state: Option<String>,
    /// Whether the user paused this process via [`control`].
    pub paused: bool,
    /// Whether the process still exists as something worth waiting on.
    ///
    /// False once it has gone from the process table, and false for a
    /// zombie — which has already exited and is only waiting to be
    /// reaped. An entry outlives its process by however long the exec
    /// loop takes to notice, so a registered pid is not by itself
    /// evidence that anything is still running.
    pub alive: bool,
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
        let (cpu_percent, memory_bytes, state, alive) = match proc_info {
            Some(p) if !is_finished(p.status()) => (
                // The first refresh has no prior measurement to diff
                // against, so its CPU value is meaningless — hide it.
                (!first_sample).then(|| p.cpu_usage()),
                Some(p.memory()),
                Some(if entry.paused {
                    "paused".to_string()
                } else {
                    state_label(p.status()).to_string()
                }),
                true,
            ),
            // Gone from the process table, or still in it only as a
            // zombie. Report it as finished rather than as a process with
            // no stats: the difference is what stops a caller presenting
            // a dead pid as something it is still waiting on.
            _ => (None, None, Some("exited".to_string()), false),
        };
        out.push(ExecProcessStatus {
            pid,
            command: entry.command.clone(),
            elapsed_ms: entry.started.elapsed().as_millis() as u64,
            cpu_percent,
            memory_bytes,
            state,
            paused: entry.paused,
            alive,
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
        let mut reg = registry()
            .lock()
            .map_err(|_| "process registry lock poisoned".to_string())?;
        if !reg.entries.contains_key(&pid) {
            return Err(format!("no controllable process with pid {pid}"));
        }
        // Registration alone is not enough. An entry outlives its process
        // by however long the exec loop takes to reap it, and the OS is
        // free to hand that number straight to something else — so
        // signalling on the strength of a stale entry can hit a process
        // this registry never admitted. Confirm it is still there.
        let target = Pid::from_u32(pid);
        reg.system
            .refresh_processes(ProcessesToUpdate::Some(&[target]), true);
        let live = reg
            .system
            .process(target)
            .is_some_and(|p| !is_finished(p.status()));
        if !live {
            return Err(format!("process {pid} has already exited"));
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

/// Whether a scheduler state means the process is over.
///
/// A zombie has already exited — it lingers in the table only until its
/// parent reaps it — so it counts as finished even though it is still
/// listed, which is what makes "present in the process table" the wrong
/// liveness test on its own.
fn is_finished(status: sysinfo::ProcessStatus) -> bool {
    matches!(
        status,
        sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead
    )
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
    use crate::ignore::Ignore;

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

    /// A registered pid whose process is gone reports as finished rather
    /// than as a live process with no stats.
    ///
    /// The entry outlives the process — the exec loop removes it only when
    /// its wait ends — so this is the window in which a status line could
    /// go on naming a pid that no longer existed.
    #[test]
    fn a_registered_pid_that_is_gone_reads_as_exited() {
        let dead = u32::MAX - 21;
        let _guard = register(dead, "already over");

        let sampled = sample_active()
            .into_iter()
            .find(|s| s.pid == dead)
            .expect("a registered pid is still sampled");
        assert!(!sampled.alive, "a pid with no process is not alive");
        assert_eq!(sampled.state.as_deref(), Some("exited"));
        assert_eq!(sampled.cpu_percent, None);
        assert_eq!(sampled.memory_bytes, None);
    }

    /// Being registered is not a licence to signal: the process must still
    /// be there. Otherwise a stale entry lets a control frame land on
    /// whatever the OS has since given the number to.
    #[test]
    fn control_refuses_a_registered_pid_that_has_exited() {
        let dead = u32::MAX - 23;
        let _guard = register(dead, "already over");

        let err = control(dead, ProcessControlAction::Kill).unwrap_err();
        assert!(err.contains("already exited"), "got: {err}");
    }

    /// A child that exited but has not been reaped is a zombie: still in
    /// the process table, but nothing to wait on.
    #[cfg(unix)]
    #[test]
    fn an_unreaped_child_reads_as_exited_not_running() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id();
        let _guard = register(pid, "true");

        // Deliberately not reaped yet, so the kernel keeps the entry as a
        // zombie — the state this is about.
        let mut sampled = None;
        for _ in 0..200 {
            let s = sample_active()
                .into_iter()
                .find(|s| s.pid == pid)
                .expect("registered pid is sampled");
            if !s.alive {
                sampled = Some(s);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let sampled = sampled.expect("an exited child should stop reading as alive");
        assert_eq!(sampled.state.as_deref(), Some("exited"));

        child.wait().ignore();
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
        child.wait().ignore();
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
