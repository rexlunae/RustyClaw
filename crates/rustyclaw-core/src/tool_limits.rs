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
}

impl Default for ToolLimitsConfig {
    fn default() -> Self {
        Self {
            window_secs: default_window_secs(),
            default_max_calls: default_max_calls(),
            per_tool: default_per_tool(),
            max_background_processes: default_max_background_processes(),
            max_subagents: default_max_subagents(),
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
    fn zero_concurrency_limit_disables_the_ceiling() {
        let _guard = guard();
        install(ToolLimitsConfig {
            max_background_processes: 0,
            max_subagents: 0,
            ..Default::default()
        });
        assert!(check_background_process(9_999).is_ok());
        assert!(check_subagent(9_999).is_ok());
        install(ToolLimitsConfig::default());
    }
}
