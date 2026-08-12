//! Budgets on how fast and how much a caller may use tools.
//!
//! Authentication decides *whether* a client may call tools; this decides how
//! hard it may lean on them. Two different failure modes are covered:
//!
//! * **Rate** — a model stuck in a loop can call `execute_command` thousands of
//!   times a second and fork-bomb the host, or hammer an external service
//!   through `web_fetch`. A sliding window per caller and tool stops that
//!   without touching normal work.
//! * **Concurrency** — a rate limit still permits an unbounded *accumulation*
//!   of long-lived resources: background processes and sub-agents each start
//!   quickly and then persist. Those get their own ceilings.
//!
//! Budgets are per caller (see [`crate::tool_caller`]), so one runaway
//! conversation cannot spend everyone else's allowance. Unidentified callers
//! share a single bucket rather than getting a free pass each.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Bucket used when a caller runs without an identity. Shared rather than
/// per-call, so unidentified traffic cannot dodge the limit by being
/// anonymous.
const ANONYMOUS_CALLER: &str = "\0anonymous";

fn default_window_secs() -> u64 {
    30
}

fn default_max_calls() -> usize {
    60
}

fn default_max_background_processes() -> usize {
    16
}

fn default_max_subagents() -> usize {
    8
}

fn default_max_background_sessions() -> usize {
    32
}

fn default_max_rounds_per_minute() -> usize {
    60
}

/// Per-tool call ceilings applied on top of [`ToolLimitsConfig::default_max_calls`].
///
/// Only tools whose cost lands somewhere other than this process are listed:
/// spawning shells, reaching external services, and starting sub-agents. They
/// are deliberately well above what real work needs — the point is to stop a
/// loop, not to pace a user. Anything absent uses the default.
fn default_per_tool() -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("execute_command".to_string(), 40),
        ("web_fetch".to_string(), 30),
        ("web_search".to_string(), 15),
        ("sessions_spawn".to_string(), 10),
        ("subagent_run".to_string(), 10),
    ])
}

/// How much tool use one caller is allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolLimitsConfig {
    /// Sliding window, in seconds, over which calls are counted.
    #[serde(default = "default_window_secs")]
    pub window_secs: u64,

    /// Calls per tool per caller per window for tools with no override.
    #[serde(default = "default_max_calls")]
    pub default_max_calls: usize,

    /// Per-tool overrides of [`Self::default_max_calls`].
    #[serde(default = "default_per_tool")]
    pub per_tool: BTreeMap<String, usize>,

    /// Background processes one caller may have running at once.
    #[serde(default = "default_max_background_processes")]
    pub max_background_processes: usize,

    /// Sub-agents one caller may have running at once.
    #[serde(default = "default_max_subagents")]
    pub max_subagents: usize,

    /// Background sessions running anywhere in this process at once.
    ///
    /// [`Self::max_subagents`] bounds a caller's *breadth*, not the depth of
    /// the tree: a spawned run is a caller in its own right, so eight can
    /// each start eight. This ceiling is what actually bounds the total.
    #[serde(default = "default_max_background_sessions")]
    pub max_background_sessions: usize,

    /// Model-call rounds per minute one turn's tool loop may sustain.
    ///
    /// This is a *pace*, not a ceiling: a turn over the rate is delayed
    /// until the sliding window has room, never stopped. It replaces the
    /// old absolute round cap, which killed legitimate long-running tasks
    /// at an arbitrary count while doing nothing to bound how *fast* a
    /// runaway loop burned until it got there. A genuine round includes a
    /// full model round-trip, so real work rarely sustains even one round
    /// per second; a degenerate loop spinning on instant responses is held
    /// to this rate, visibly, where the user can read what it is doing and
    /// cancel. 0 disables pacing.
    #[serde(default = "default_max_rounds_per_minute")]
    pub max_rounds_per_minute: usize,
}

impl Default for ToolLimitsConfig {
    fn default() -> Self {
        Self {
            window_secs: default_window_secs(),
            default_max_calls: default_max_calls(),
            per_tool: default_per_tool(),
            max_background_processes: default_max_background_processes(),
            max_subagents: default_max_subagents(),
            max_background_sessions: default_max_background_sessions(),
            max_rounds_per_minute: default_max_rounds_per_minute(),
        }
    }
}

impl ToolLimitsConfig {
    /// The call ceiling that applies to `tool`.
    pub fn max_calls_for(&self, tool: &str) -> usize {
        self.per_tool
            .get(tool)
            .copied()
            .unwrap_or(self.default_max_calls)
    }
}

/// Why a tool call was refused before it ran.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitError {
    /// The caller's window budget for this tool is spent.
    #[error(
        "Rate limit exceeded: '{tool}' called {limit} times in the last {window_secs}s. \
         Wait for the window to clear, or raise tool_limits in config."
    )]
    RateExceeded {
        tool: String,
        limit: usize,
        window_secs: u64,
    },

    /// The caller already holds the maximum number of background processes.
    #[error(
        "Too many background processes: {running} already running (limit {limit}). \
         Finish or remove one with the process tool, or raise \
         tool_limits.max_background_processes in config."
    )]
    TooManyProcesses { running: usize, limit: usize },

    /// The caller already has the maximum number of sub-agents running.
    #[error(
        "Too many sub-agents: {running} already running (limit {limit}). \
         Wait for one to finish, or raise tool_limits.max_subagents in config."
    )]
    TooManySubagents { running: usize, limit: usize },

    /// This process is already running as many background sessions as it may.
    #[error(
        "Too many background sessions: {running} already running across all callers \
         (limit {limit}). Wait for one to finish or stop one with sessions_kill, or \
         raise tool_limits.max_background_sessions in config."
    )]
    TooManyBackgroundSessions { running: usize, limit: usize },
}

/// Installed limits. Set once by the gateway at startup; defaults apply until
/// then so direct CLI use and tests are still bounded.
fn config_cell() -> &'static Mutex<ToolLimitsConfig> {
    static CONFIG: OnceLock<Mutex<ToolLimitsConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| Mutex::new(ToolLimitsConfig::default()))
}

/// Replace the active limits.
pub fn install(config: ToolLimitsConfig) {
    if let Ok(mut slot) = config_cell().lock() {
        *slot = config;
    }
}

/// A snapshot of the active limits.
pub fn config() -> ToolLimitsConfig {
    config_cell().lock().map(|c| c.clone()).unwrap_or_default()
}

/// Call timestamps per `(caller, tool)`.
#[derive(Default)]
struct Windows {
    calls: HashMap<(String, String), VecDeque<Instant>>,
}

fn windows() -> &'static Mutex<Windows> {
    static WINDOWS: OnceLock<Mutex<Windows>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(Windows::default()))
}

/// Record a call by `caller` to `tool`, or refuse it.
///
/// A refusal does not consume budget: the call never happened, so charging for
/// it would extend the lockout every time the model retried.
///
/// A poisoned lock allows the call. This is a budget, not an access control —
/// the caller has already been authenticated and authorised — so failing open
/// keeps a panic elsewhere from bricking every tool in the process.
pub fn check_rate(caller: Option<&str>, tool: &str) -> Result<(), LimitError> {
    let cfg = config();
    let limit = cfg.max_calls_for(tool);
    if limit == 0 {
        // Explicitly disabled rather than "no calls allowed": a zero here is
        // how a user turns the limit off for one tool.
        return Ok(());
    }
    let window = Duration::from_secs(cfg.window_secs);

    let Ok(mut w) = windows().lock() else {
        return Ok(());
    };

    let now = Instant::now();
    let key = (
        caller.unwrap_or(ANONYMOUS_CALLER).to_string(),
        tool.to_string(),
    );
    let entry = w.calls.entry(key).or_default();

    while entry
        .front()
        .is_some_and(|t| now.duration_since(*t) >= window)
    {
        entry.pop_front();
    }

    if entry.len() >= limit {
        return Err(LimitError::RateExceeded {
            tool: tool.to_string(),
            limit,
            window_secs: cfg.window_secs,
        });
    }

    entry.push_back(now);

    // Drop buckets that have gone quiet, so a long-lived gateway does not
    // accumulate one entry per (caller, tool) pair it has ever seen.
    if w.calls.len() > 512 {
        w.calls.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < window);
            !times.is_empty()
        });
    }

    Ok(())
}

/// Refuse a new background process when the caller is already at its ceiling.
pub fn check_background_process(running: usize) -> Result<(), LimitError> {
    let limit = config().max_background_processes;
    if limit == 0 || running < limit {
        return Ok(());
    }
    Err(LimitError::TooManyProcesses { running, limit })
}

/// Refuse a new sub-agent when the caller is already at its ceiling.
pub fn check_subagent(running: usize) -> Result<(), LimitError> {
    let limit = config().max_subagents;
    if limit == 0 || running < limit {
        return Ok(());
    }
    Err(LimitError::TooManySubagents { running, limit })
}

/// Refuse a new background session when the process is already at its ceiling.
///
/// Unlike the per-caller checks, this one counts everything: a spawned run
/// spawning its own runs is a fresh caller with a fresh budget, so nothing
/// per-caller can bound the tree.
pub fn check_background_session(running: usize) -> Result<(), LimitError> {
    let limit = config().max_background_sessions;
    if limit == 0 || running < limit {
        return Ok(());
    }
    Err(LimitError::TooManyBackgroundSessions { running, limit })
}

// ── Round pacing ────────────────────────────────────────────────────────────

/// The sliding window [`RoundPacer`] paces over.
const ROUND_WINDOW: Duration = Duration::from_secs(60);

/// Paces one turn's agentic tool loop by rate instead of stopping it at a
/// count.
///
/// The loop used to end at an absolute round cap, which cut off legitimate
/// long-running tasks at an arbitrary number while doing nothing to bound
/// how fast a runaway loop burned on the way there. This inverts that: any
/// number of rounds is allowed, but a turn that exceeds
/// [`ToolLimitsConfig::max_rounds_per_minute`] waits for the window to open
/// rather than starting the next round. A runaway loop is thereby held to a
/// bounded, visible burn rate — slow enough to read and cancel — and a long
/// task simply keeps going.
///
/// One pacer per turn: the rate describes a single loop, and two concurrent
/// turns sharing a window would throttle each other for working at all.
pub struct RoundPacer {
    max_per_minute: usize,
    rounds: VecDeque<Instant>,
}

impl RoundPacer {
    /// A pacer honouring the installed [`ToolLimitsConfig`].
    pub fn from_config() -> Self {
        Self::new(config().max_rounds_per_minute)
    }

    /// A pacer allowing `max_per_minute` rounds per sliding minute.
    /// 0 disables pacing.
    pub fn new(max_per_minute: usize) -> Self {
        Self {
            max_per_minute,
            rounds: VecDeque::new(),
        }
    }

    /// Ask to start a round at `now`.
    ///
    /// `None` admits the round and records it. `Some(wait)` means the window
    /// is full: the caller should wait roughly that long and ask again —
    /// nothing is recorded for a refused ask, so waiting never extends the
    /// wait. Callers should re-ask in a loop rather than trusting one wait
    /// to be exact.
    pub fn admit(&mut self, now: Instant) -> Option<Duration> {
        if self.max_per_minute == 0 {
            return None;
        }
        while self
            .rounds
            .front()
            .is_some_and(|t| now.duration_since(*t) >= ROUND_WINDOW)
        {
            self.rounds.pop_front();
        }
        if self.rounds.len() >= self.max_per_minute {
            // Room opens when the oldest recorded round ages out.
            let oldest = *self.rounds.front().expect("len >= max >= 1");
            return Some(ROUND_WINDOW.saturating_sub(now.duration_since(oldest)));
        }
        self.rounds.push_back(now);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `install` writes process-wide state and the test harness runs tests in
    /// parallel, so they take turns. Each also uses tool names of its own, so
    /// the call windows stay independent even though the config does not.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cfg_with(tool: &str, max: usize) -> ToolLimitsConfig {
        let mut cfg = ToolLimitsConfig::default();
        cfg.per_tool.insert(tool.to_string(), max);
        cfg
    }

    #[test]
    fn per_tool_override_beats_the_default() {
        let cfg = cfg_with("execute_command", 3);
        assert_eq!(cfg.max_calls_for("execute_command"), 3);
        assert_eq!(cfg.max_calls_for("read_file"), cfg.default_max_calls);
    }

    #[test]
    fn budget_is_spent_then_refused() {
        let _guard = guard();
        install(cfg_with("t_spend", 2));
        assert!(check_rate(Some("caller:a"), "t_spend").is_ok());
        assert!(check_rate(Some("caller:a"), "t_spend").is_ok());
        let refused = check_rate(Some("caller:a"), "t_spend");
        assert!(
            matches!(refused, Err(LimitError::RateExceeded { .. })),
            "third call must be refused, got {refused:?}"
        );
        install(ToolLimitsConfig::default());
    }

    #[test]
    fn callers_do_not_share_a_budget() {
        let _guard = guard();
        install(cfg_with("t_isolate", 1));
        assert!(check_rate(Some("caller:one"), "t_isolate").is_ok());
        // The point of keying by caller: one conversation exhausting its
        // budget must not refuse another's first call.
        assert!(
            check_rate(Some("caller:two"), "t_isolate").is_ok(),
            "a second caller must have its own budget"
        );
        assert!(check_rate(Some("caller:one"), "t_isolate").is_err());
        install(ToolLimitsConfig::default());
    }

    #[test]
    fn tools_do_not_share_a_budget() {
        let _guard = guard();
        install(cfg_with("t_toolsep", 1));
        assert!(check_rate(Some("caller:c"), "t_toolsep").is_ok());
        assert!(check_rate(Some("caller:c"), "t_toolsep").is_err());
        // A different tool is a different bucket.
        assert!(check_rate(Some("caller:c"), "t_toolsep_other").is_ok());
        install(ToolLimitsConfig::default());
    }

    #[test]
    fn unidentified_callers_share_one_bucket() {
        let _guard = guard();
        install(cfg_with("t_anon", 1));
        assert!(check_rate(None, "t_anon").is_ok());
        assert!(
            check_rate(None, "t_anon").is_err(),
            "anonymity must not hand out a fresh budget per call"
        );
        install(ToolLimitsConfig::default());
    }

    #[test]
    fn a_refusal_does_not_consume_budget() {
        let _guard = guard();
        install(cfg_with("t_nocharge", 1));
        assert!(check_rate(Some("caller:d"), "t_nocharge").is_ok());
        for _ in 0..5 {
            assert!(check_rate(Some("caller:d"), "t_nocharge").is_err());
        }
        // Five refusals must not have extended the lockout: exactly one call
        // is still on record, so the window clears when *it* ages out.
        let held = windows()
            .lock()
            .map(|w| {
                w.calls
                    .get(&("caller:d".to_string(), "t_nocharge".to_string()))
                    .map(|q| q.len())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        assert_eq!(held, 1, "refused calls must not be recorded");
        install(ToolLimitsConfig::default());
    }

    #[test]
    fn a_zero_limit_disables_the_check() {
        let _guard = guard();
        install(cfg_with("t_off", 0));
        for _ in 0..100 {
            assert!(check_rate(Some("caller:e"), "t_off").is_ok());
        }
        install(ToolLimitsConfig::default());
    }

    #[test]
    fn concurrency_ceilings_refuse_at_the_limit() {
        let _guard = guard();
        let cfg = ToolLimitsConfig {
            max_background_processes: 2,
            max_subagents: 1,
            ..Default::default()
        };
        install(cfg);

        assert!(check_background_process(1).is_ok());
        assert!(matches!(
            check_background_process(2),
            Err(LimitError::TooManyProcesses { .. })
        ));
        assert!(check_subagent(0).is_ok());
        assert!(matches!(
            check_subagent(1),
            Err(LimitError::TooManySubagents { .. })
        ));

        install(ToolLimitsConfig::default());
    }

    #[test]
    fn the_background_session_ceiling_bounds_the_whole_tree() {
        // The per-caller sub-agent cap cannot bound depth: each spawned run is
        // a caller with its own fresh allowance. This one counts everything.
        let _guard = guard();
        install(ToolLimitsConfig {
            max_background_sessions: 3,
            ..Default::default()
        });

        assert!(check_background_session(2).is_ok());
        assert!(matches!(
            check_background_session(3),
            Err(LimitError::TooManyBackgroundSessions {
                running: 3,
                limit: 3
            })
        ));

        install(ToolLimitsConfig::default());
    }

    /// Under the rate, rounds are admitted immediately — pacing must be
    /// invisible to a loop doing ordinary work.
    #[test]
    fn rounds_under_the_rate_are_never_delayed() {
        let mut pacer = RoundPacer::new(10);
        let start = Instant::now();
        for i in 0..10 {
            assert_eq!(
                pacer.admit(start + Duration::from_secs(i)),
                None,
                "round {i} is within the rate and must not wait"
            );
        }
    }

    /// Over the rate, the pacer asks for a wait — and the wait is until the
    /// oldest round leaves the window, not forever: this is a pace, not a
    /// stop. That distinction is the whole point of replacing the absolute
    /// round cap that killed long-running tasks.
    #[test]
    fn a_full_window_waits_for_room_and_then_admits() {
        let mut pacer = RoundPacer::new(3);
        let start = Instant::now();
        for i in 0..3 {
            assert_eq!(pacer.admit(start + Duration::from_millis(i)), None);
        }

        let asked_at = start + Duration::from_secs(1);
        let wait = pacer
            .admit(asked_at)
            .expect("a full window must ask for a wait");
        // The oldest round was at `start`; it ages out at start + 60s.
        assert_eq!(wait, Duration::from_secs(59));

        // Asking again after the wait must admit — a caller that keeps
        // waiting and re-asking always eventually proceeds.
        assert_eq!(pacer.admit(asked_at + wait), None);
    }

    /// A refused ask must not be recorded: a loop that politely waits and
    /// re-asks would otherwise push its own admission further away each
    /// time it asked.
    #[test]
    fn waiting_does_not_extend_the_wait() {
        let mut pacer = RoundPacer::new(1);
        let start = Instant::now();
        assert_eq!(pacer.admit(start), None);

        let first = pacer.admit(start + Duration::from_secs(1)).expect("full");
        let second = pacer.admit(start + Duration::from_secs(2)).expect("full");
        assert!(
            second < first,
            "the wait must shrink while the caller waits: {first:?} then {second:?}"
        );
    }

    /// Zero disables pacing entirely, matching every other limit here.
    #[test]
    fn a_zero_rate_disables_pacing() {
        let mut pacer = RoundPacer::new(0);
        let now = Instant::now();
        for _ in 0..1000 {
            assert_eq!(pacer.admit(now), None);
        }
    }

    #[test]
    fn zero_concurrency_limit_disables_the_ceiling() {
        let _guard = guard();
        install(ToolLimitsConfig {
            max_background_processes: 0,
            max_subagents: 0,
            max_background_sessions: 0,
            ..Default::default()
        });
        assert!(check_background_process(9_999).is_ok());
        assert!(check_subagent(9_999).is_ok());
        assert!(check_background_session(9_999).is_ok());
        install(ToolLimitsConfig::default());
    }
}
