//! Periodic auto-fetch scheduler.
//!
//! Walks every active [`SyncSource`] on a global tick. Each source has its own
//! cursor, last-sync timestamp, dedup set, and daily request budget, so per-
//! source rate-limit behavior is independent of the global cadence. Modeled on
//! the design in <https://tinyhumans.gitbook.io/openhuman/features/obsidian-wiki/auto-fetch>.
//!
//! Sources are not coupled to RustyClaw channels — a channel can register
//! itself as a source if it has periodic-fetch semantics (Gmail-style label
//! pages, Slack channel history, etc.), but a source can also be a pure
//! file-system watcher, RSS poller, or scraper.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Why a sync was triggered. Sources may want to fetch more aggressively for
/// non-periodic events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncReason {
    /// Global scheduler tick.
    Periodic,
    /// External webhook (push) fired.
    Webhook,
    /// First-time connection setup.
    ConnectionCreated,
    /// User explicitly clicked "sync now".
    Manual,
}

/// A connected data source that the scheduler can poll.
#[async_trait]
pub trait SyncSource: Send + Sync {
    /// Stable family of the source (e.g. `"gmail"`, `"slack"`, `"github"`).
    fn toolkit(&self) -> &str;

    /// Per-connection identifier (e.g. account email, workspace id).
    fn connection_id(&self) -> &str;

    /// Minimum gap between successive `sync` calls. The global scheduler may
    /// run more often, but a source's `sync` is only invoked when this much
    /// time has elapsed since the last successful run.
    fn sync_interval(&self) -> Duration;

    /// Maximum syncs per UTC day. `None` means unbounded. The scheduler
    /// updates the bookkeeping; the source itself doesn't need to check.
    fn daily_budget(&self) -> Option<u32> {
        None
    }

    /// Fetch new data and ingest. Errors are logged and swallowed by the
    /// scheduler — they never panic out of the loop.
    async fn sync(&self, reason: SyncReason) -> Result<SyncOutcome, SyncError>;
}

/// Per-source bookkeeping persisted across restarts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncState {
    /// Provider-defined opaque cursor (last message id, page token, etc.).
    #[serde(default)]
    pub cursor: Option<String>,
    /// Last successful sync timestamp.
    #[serde(default)]
    pub last_sync: Option<DateTime<Utc>>,
    /// UTC day (`YYYY-MM-DD`) for which `daily_count` is valid.
    #[serde(default)]
    pub daily_window: Option<String>,
    /// Sync count within the current daily window.
    #[serde(default)]
    pub daily_count: u32,
    /// Total successful syncs over the lifetime of this connection.
    #[serde(default)]
    pub total_syncs: u64,
    /// Most recent error message, if the last sync failed.
    #[serde(default)]
    pub last_error: Option<String>,
}

impl SyncState {
    pub fn is_due(&self, interval: Duration) -> bool {
        match self.last_sync {
            None => true,
            Some(last) => {
                let elapsed = Utc::now()
                    .signed_duration_since(last)
                    .to_std()
                    .unwrap_or(Duration::ZERO);
                elapsed >= interval
            }
        }
    }

    pub fn budget_remaining(&self, budget: Option<u32>) -> bool {
        let Some(budget) = budget else {
            return true;
        };
        let today = today_utc();
        if self.daily_window.as_deref() != Some(&today) {
            return true;
        }
        self.daily_count < budget
    }

    pub(crate) fn record_success(&mut self, outcome: &SyncOutcome) {
        self.last_sync = Some(Utc::now());
        if let Some(c) = outcome.cursor.clone() {
            self.cursor = Some(c);
        }
        self.last_error = None;
        self.total_syncs = self.total_syncs.saturating_add(1);
        self.bump_daily_count();
    }

    pub(crate) fn record_failure(&mut self, err: &str) {
        self.last_error = Some(err.to_string());
        self.bump_daily_count();
    }

    fn bump_daily_count(&mut self) {
        let today = today_utc();
        if self.daily_window.as_deref() == Some(&today) {
            self.daily_count = self.daily_count.saturating_add(1);
        } else {
            self.daily_window = Some(today);
            self.daily_count = 1;
        }
    }
}

fn today_utc() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// What a successful sync returned.
#[derive(Debug, Clone, Default)]
pub struct SyncOutcome {
    /// New cursor to persist for next time.
    pub cursor: Option<String>,
    /// Items ingested (informational; e.g. number of new messages).
    pub items_ingested: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SyncError(pub String);

impl From<String> for SyncError {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for SyncError {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}
impl From<anyhow::Error> for SyncError {
    fn from(e: anyhow::Error) -> Self {
        Self(format!("{:#}", e))
    }
}

/// In-memory state store. For production use, wrap a database / KV backend
/// behind the [`StateBackend`] trait.
#[derive(Debug, Default)]
pub struct MemoryStateStore {
    inner: RwLock<HashMap<(String, String), SyncState>>,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Persistence interface for [`SyncState`]. Implement to plug in disk/SQLite/etc.
#[async_trait]
pub trait StateBackend: Send + Sync {
    async fn get(&self, toolkit: &str, connection_id: &str) -> SyncState;
    async fn put(&self, toolkit: &str, connection_id: &str, state: SyncState);
    /// Read all known (toolkit, connection_id, state) triples. Used for
    /// reporting / debug surfaces.
    async fn list(&self) -> Vec<(String, String, SyncState)> {
        Vec::new()
    }
}

#[async_trait]
impl StateBackend for MemoryStateStore {
    async fn get(&self, toolkit: &str, connection_id: &str) -> SyncState {
        let map = self.inner.read().await;
        map.get(&(toolkit.to_string(), connection_id.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    async fn put(&self, toolkit: &str, connection_id: &str, state: SyncState) {
        let mut map = self.inner.write().await;
        map.insert((toolkit.to_string(), connection_id.to_string()), state);
    }

    async fn list(&self) -> Vec<(String, String, SyncState)> {
        let map = self.inner.read().await;
        map.iter()
            .map(|((t, c), s)| (t.clone(), c.clone(), s.clone()))
            .collect()
    }
}

/// Global tick scheduler.
pub struct AutoFetchScheduler {
    sources: Vec<Arc<dyn SyncSource>>,
    state: Arc<dyn StateBackend>,
    tick: Duration,
}

impl AutoFetchScheduler {
    /// Default 20-minute global tick, matching OpenHuman's design choice.
    pub const DEFAULT_TICK: Duration = Duration::from_secs(20 * 60);

    pub fn new(state: Arc<dyn StateBackend>) -> Self {
        Self {
            sources: Vec::new(),
            state,
            tick: Self::DEFAULT_TICK,
        }
    }

    pub fn with_tick(mut self, tick: Duration) -> Self {
        self.tick = tick;
        self
    }

    pub fn register(&mut self, source: Arc<dyn SyncSource>) -> &mut Self {
        self.sources.push(source);
        self
    }

    /// Run the scheduler forever. Cancellation is via dropping the future.
    pub async fn run(self) {
        info!(
            sources = self.sources.len(),
            tick_secs = self.tick.as_secs(),
            "auto_fetch scheduler started"
        );
        let mut ticker = tokio::time::interval(self.tick);
        // Fire the first tick immediately rather than after `tick` elapses.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            self.tick_once(SyncReason::Periodic).await;
        }
    }

    /// Run one pass over all registered sources. Useful for tests and
    /// webhook-driven invocations.
    pub async fn tick_once(&self, reason: SyncReason) -> Vec<TickResult> {
        let mut results = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            let toolkit = source.toolkit().to_string();
            let conn = source.connection_id().to_string();
            let mut state = self.state.get(&toolkit, &conn).await;

            if reason == SyncReason::Periodic && !state.is_due(source.sync_interval()) {
                results.push(TickResult::skipped(&toolkit, &conn, "interval"));
                continue;
            }
            if !state.budget_remaining(source.daily_budget()) {
                results.push(TickResult::skipped(&toolkit, &conn, "budget"));
                continue;
            }

            debug!(toolkit, conn, ?reason, "auto_fetch syncing");
            match source.sync(reason).await {
                Ok(outcome) => {
                    state.record_success(&outcome);
                    self.state.put(&toolkit, &conn, state).await;
                    results.push(TickResult::ok(&toolkit, &conn, outcome.items_ingested));
                }
                Err(e) => {
                    warn!(toolkit, conn, error = %e.0, "auto_fetch sync failed");
                    state.record_failure(&e.0);
                    self.state.put(&toolkit, &conn, state).await;
                    results.push(TickResult::failed(&toolkit, &conn, &e.0));
                }
            }
        }
        results
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }
}

#[derive(Debug, Clone)]
pub struct TickResult {
    pub toolkit: String,
    pub connection_id: String,
    pub status: TickStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickStatus {
    Synced { items: usize },
    Skipped { reason: &'static str },
    Failed { error: String },
}

impl TickResult {
    fn ok(toolkit: &str, conn: &str, items: usize) -> Self {
        Self {
            toolkit: toolkit.to_string(),
            connection_id: conn.to_string(),
            status: TickStatus::Synced { items },
        }
    }
    fn skipped(toolkit: &str, conn: &str, reason: &'static str) -> Self {
        Self {
            toolkit: toolkit.to_string(),
            connection_id: conn.to_string(),
            status: TickStatus::Skipped { reason },
        }
    }
    fn failed(toolkit: &str, conn: &str, error: &str) -> Self {
        Self {
            toolkit: toolkit.to_string(),
            connection_id: conn.to_string(),
            status: TickStatus::Failed {
                error: error.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        toolkit: String,
        conn: String,
        interval: Duration,
        budget: Option<u32>,
        called: Arc<AtomicUsize>,
        fail_next: Arc<std::sync::Mutex<bool>>,
    }

    #[async_trait]
    impl SyncSource for Counting {
        fn toolkit(&self) -> &str {
            &self.toolkit
        }
        fn connection_id(&self) -> &str {
            &self.conn
        }
        fn sync_interval(&self) -> Duration {
            self.interval
        }
        fn daily_budget(&self) -> Option<u32> {
            self.budget
        }
        async fn sync(&self, _reason: SyncReason) -> Result<SyncOutcome, SyncError> {
            self.called.fetch_add(1, Ordering::SeqCst);
            let should_fail = {
                let mut g = self.fail_next.lock().unwrap();
                let v = *g;
                *g = false;
                v
            };
            if should_fail {
                return Err("boom".into());
            }
            Ok(SyncOutcome {
                cursor: Some("c1".into()),
                items_ingested: 3,
            })
        }
    }

    #[tokio::test]
    async fn tick_calls_each_source_once() {
        let store = Arc::new(MemoryStateStore::new());
        let called = Arc::new(AtomicUsize::new(0));
        let src: Arc<dyn SyncSource> = Arc::new(Counting {
            toolkit: "gmail".into(),
            conn: "user@example.com".into(),
            interval: Duration::from_secs(0),
            budget: None,
            called: Arc::clone(&called),
            fail_next: Arc::new(std::sync::Mutex::new(false)),
        });

        let mut sched = AutoFetchScheduler::new(store.clone());
        sched.register(src);

        let results = sched.tick_once(SyncReason::Manual).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].status, TickStatus::Synced { items: 3 }));
        assert_eq!(called.load(Ordering::SeqCst), 1);

        let state = store.get("gmail", "user@example.com").await;
        assert_eq!(state.cursor.as_deref(), Some("c1"));
        assert_eq!(state.total_syncs, 1);
    }

    #[tokio::test]
    async fn periodic_skips_when_not_due() {
        let store = Arc::new(MemoryStateStore::new());
        // Pre-populate state with last_sync = now.
        let s = SyncState {
            last_sync: Some(Utc::now()),
            ..Default::default()
        };
        store.put("slack", "ws1", s).await;

        let called = Arc::new(AtomicUsize::new(0));
        let src: Arc<dyn SyncSource> = Arc::new(Counting {
            toolkit: "slack".into(),
            conn: "ws1".into(),
            interval: Duration::from_secs(3600),
            budget: None,
            called: Arc::clone(&called),
            fail_next: Arc::new(std::sync::Mutex::new(false)),
        });
        let mut sched = AutoFetchScheduler::new(store);
        sched.register(src);

        let results = sched.tick_once(SyncReason::Periodic).await;
        assert!(matches!(
            results[0].status,
            TickStatus::Skipped { reason: "interval" }
        ));
        assert_eq!(called.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn manual_overrides_interval_skip() {
        let store = Arc::new(MemoryStateStore::new());
        let s = SyncState {
            last_sync: Some(Utc::now()),
            ..Default::default()
        };
        store.put("gh", "octocat", s).await;

        let called = Arc::new(AtomicUsize::new(0));
        let src: Arc<dyn SyncSource> = Arc::new(Counting {
            toolkit: "gh".into(),
            conn: "octocat".into(),
            interval: Duration::from_secs(3600),
            budget: None,
            called: Arc::clone(&called),
            fail_next: Arc::new(std::sync::Mutex::new(false)),
        });
        let mut sched = AutoFetchScheduler::new(store);
        sched.register(src);

        let results = sched.tick_once(SyncReason::Manual).await;
        assert!(matches!(results[0].status, TickStatus::Synced { .. }));
        assert_eq!(called.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn daily_budget_enforced() {
        let store = Arc::new(MemoryStateStore::new());
        let called = Arc::new(AtomicUsize::new(0));
        let src: Arc<dyn SyncSource> = Arc::new(Counting {
            toolkit: "rss".into(),
            conn: "feed1".into(),
            interval: Duration::from_secs(0),
            budget: Some(2),
            called: Arc::clone(&called),
            fail_next: Arc::new(std::sync::Mutex::new(false)),
        });
        let mut sched = AutoFetchScheduler::new(store);
        sched.register(src);

        for _ in 0..2 {
            let r = sched.tick_once(SyncReason::Manual).await;
            assert!(matches!(r[0].status, TickStatus::Synced { .. }));
        }
        let r = sched.tick_once(SyncReason::Manual).await;
        assert!(matches!(
            r[0].status,
            TickStatus::Skipped { reason: "budget" }
        ));
    }

    #[tokio::test]
    async fn errors_are_recorded_but_loop_continues() {
        let store = Arc::new(MemoryStateStore::new());
        let called = Arc::new(AtomicUsize::new(0));
        let fail_next = Arc::new(std::sync::Mutex::new(true));
        let src: Arc<dyn SyncSource> = Arc::new(Counting {
            toolkit: "github".into(),
            conn: "u".into(),
            interval: Duration::from_secs(0),
            budget: None,
            called: Arc::clone(&called),
            fail_next: Arc::clone(&fail_next),
        });
        let mut sched = AutoFetchScheduler::new(store.clone());
        sched.register(src);

        let r = sched.tick_once(SyncReason::Manual).await;
        assert!(matches!(r[0].status, TickStatus::Failed { .. }));

        let state = store.get("github", "u").await;
        assert_eq!(state.last_error.as_deref(), Some("boom"));

        // Next tick should still try.
        let r = sched.tick_once(SyncReason::Manual).await;
        assert!(matches!(r[0].status, TickStatus::Synced { .. }));
    }
}
