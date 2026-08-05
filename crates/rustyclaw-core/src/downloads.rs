//! Registry of file downloads started by the agent.
//!
//! A download is not a tool call that returns a value — it is a thing that
//! outlives the call that started it. `web_fetch` with a destination returns
//! as soon as the transfer is registered, so the turn is not held open for a
//! file that may take minutes, and the transfer reports on itself from here.
//!
//! Two consumers watch this registry, and they want different things:
//!
//! * The **downloads panel** wants every change, so a progress bar moves.
//! * The **agent** wants only terminal ones, because being told a file
//!   finished is worth a turn and being told it is 40% through is not.
//!
//! Both read the same broadcast; the difference is which events they act on.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Identifier for one transfer, unique for the life of the process.
pub type DownloadId = String;

/// Who started a transfer.
///
/// This registry is process-global, but a completion is not: waking "the
/// agent" is meaningless in a gateway serving several of them, and thread ids
/// restart low in every agent's store — so a bare thread id would name a
/// conversation belonging to whichever agent asked first.
///
/// The agent id is what disambiguates it, and the connection id is not a
/// substitute for two reasons that pull in opposite directions. One connection
/// can change agents wholesale — `handle_agent_switch` replaces the session
/// and its thread store — so a connection is too *broad*: thread 3 means a
/// different conversation before and after. And one agent outlives its
/// connections, so a connection is also too *narrow*: reconnect and the
/// transfers you started become invisible and uncancellable, which is the
/// opposite of what a background download in a daemon should do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOrigin {
    /// The agent that ran the tool. This is what ownership is decided by.
    pub agent: String,
    /// The connection it ran on. Kept for routing live updates only — never
    /// for deciding who a transfer belongs to.
    pub connection: u64,
    /// The conversation the tool call belonged to, when there was one.
    /// Meaningful only together with `agent`.
    pub thread: Option<u64>,
}

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

/// Mint an id for a connection, so transfers it starts can be told from
/// another connection's.
pub fn next_connection_id() -> u64 {
    NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed)
}

tokio::task_local! {
    static ORIGIN: DownloadOrigin;
}

/// Run a turn with its origin attached, so tools it calls do not each have to
/// be handed one.
///
/// A task-local rather than a parameter: `web_fetch` reaches this through
/// `execute_tool`, which dispatches forty-odd tools none of the rest of which
/// care, and a task-local is scoped to the turn — unlike a thread-local, two
/// turns sharing a worker thread cannot read each other's.
pub async fn with_origin<F>(origin: DownloadOrigin, f: F) -> F::Output
where
    F: std::future::Future,
{
    ORIGIN.scope(origin, f).await
}

/// The origin of the turn currently running, if this is running inside one.
///
/// `None` outside a turn — the CLI's one-shot paths, and tests — which simply
/// means nothing gets woken.
pub fn current_origin() -> Option<DownloadOrigin> {
    ORIGIN.try_with(|o| o.clone()).ok()
}

/// Where a transfer has got to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadStatus {
    /// Bytes are still arriving.
    Running,
    /// The file is complete and closed.
    Complete,
    /// The transfer stopped early. Carries why, because "it failed" is not
    /// something the user or the agent can act on.
    Failed { error: String },
    /// Stopped deliberately, from the panel.
    Cancelled,
}

impl DownloadStatus {
    /// Whether this status ends the transfer.
    ///
    /// The agent is woken on these and only these — progress is the panel's
    /// business.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, DownloadStatus::Running)
    }
}

/// One transfer, as both the panel and the agent see it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: DownloadId,
    pub url: String,
    /// Where the bytes are being written.
    pub dest: PathBuf,
    /// Total size when the server declared one. Absent is normal — chunked
    /// responses do not carry a length — and means the panel shows progress
    /// without a percentage rather than a wrong percentage.
    pub total_bytes: Option<u64>,
    pub received_bytes: u64,
    pub status: DownloadStatus,
    pub started_ms: u64,
    /// When it reached a terminal status.
    pub finished_ms: Option<u64>,
    /// Which agent, connection and conversation started it. `None` when
    /// nothing was listening — a transfer still gets tracked, it just wakes
    /// nobody.
    pub origin: Option<DownloadOrigin>,
}

impl Download {
    /// Progress as a fraction, when the size is known.
    pub fn fraction(&self) -> Option<f32> {
        match self.total_bytes {
            Some(total) if total > 0 => Some((self.received_bytes as f32 / total as f32).min(1.0)),
            _ => None,
        }
    }

    /// Whether `agent` should be woken for this event.
    ///
    /// Both halves matter and neither is obvious from the call site, which is
    /// why the rule lives here with its tests. Progress must not wake anyone
    /// — a large file would otherwise start a turn every quarter-megabyte —
    /// and an untagged transfer belongs to no agent, so a gateway serving two
    /// of them must not let either claim it.
    pub fn wakes(&self, agent: &str) -> bool {
        self.status.is_terminal() && self.belongs_to(agent)
    }

    /// Whether `agent` is the one that started this transfer.
    ///
    /// What the panel filters on. Progress belongs in a panel and not in a
    /// turn, so this is the weaker of the two tests — but it is the same
    /// ownership rule, and the panel must not show a client another agent's
    /// URLs and destination paths, including after a switch on the same
    /// connection.
    pub fn belongs_to(&self, agent: &str) -> bool {
        self.origin.as_ref().map(|o| o.agent.as_str()) == Some(agent)
    }

    /// How this transfer ended, in a sentence.
    ///
    /// Lives here rather than in whoever renders it because the same words go
    /// to two audiences that must not disagree: the panel shows it, and it is
    /// what the agent is told when the transfer wakes it.
    pub fn summary(&self) -> String {
        match &self.status {
            DownloadStatus::Running => format!(
                "Downloading {} to {} ({} so far)",
                self.url,
                self.dest.display(),
                human_bytes(self.received_bytes),
            ),
            DownloadStatus::Complete => format!(
                "Finished downloading {} to {} ({})",
                self.url,
                self.dest.display(),
                human_bytes(self.received_bytes),
            ),
            // The reason is carried through verbatim: "the download failed"
            // tells the agent nothing it can act on, and it is the agent that
            // has to decide whether to retry, pick another URL, or stop.
            DownloadStatus::Failed { error } => format!(
                "Download of {} to {} failed after {}: {}",
                self.url,
                self.dest.display(),
                human_bytes(self.received_bytes),
                error,
            ),
            DownloadStatus::Cancelled => format!(
                "Download of {} to {} was cancelled after {}",
                self.url,
                self.dest.display(),
                human_bytes(self.received_bytes),
            ),
        }
    }
}

/// Bytes at a size a person can read.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// What changed, for whoever is watching.
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    /// A transfer was registered, made progress, or ended. Carries the whole
    /// record rather than a delta: consumers redraw or narrate from it, and
    /// neither wants to reassemble state from fragments.
    Changed(Download),
    /// Transfers were dropped from the registry.
    ///
    /// A separate variant because a removal is not a state a `Download` can
    /// be in — it no longer exists — and because the two consumers must treat
    /// it differently: a panel re-reads the list, and the agent is told
    /// nothing. Forgetting a finished transfer is bookkeeping, not news.
    ///
    /// Carries the agent so a panel can tell whether it was its list that
    /// changed, and the ids so anything keyed by them can drop its entries.
    Removed { agent: String, ids: Vec<DownloadId> },
}

/// The set of transfers this process knows about.
#[derive(Debug, Default)]
pub struct DownloadManager {
    downloads: HashMap<DownloadId, Download>,
    next_id: u64,
}

impl DownloadManager {
    /// An empty registry. The process one is reached through
    /// [`download_manager`] instead; this is for tests that want their own.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a transfer that is about to start.
    ///
    /// The origin is passed in rather than read from the ambient task, so the
    /// registry stays a plain data structure and tests can name an origin
    /// without a runtime. Callers inside a turn pass [`current_origin`].
    pub fn register(
        &mut self,
        url: String,
        dest: PathBuf,
        total_bytes: Option<u64>,
        origin: Option<DownloadOrigin>,
    ) -> Download {
        self.next_id += 1;
        let download = Download {
            id: format!("dl_{}", self.next_id),
            url,
            dest,
            total_bytes,
            received_bytes: 0,
            status: DownloadStatus::Running,
            started_ms: now_ms(),
            finished_ms: None,
            origin,
        };
        self.downloads.insert(download.id.clone(), download.clone());
        download
    }

    /// Record bytes received. Returns the updated record, or `None` if the id
    /// is unknown or the transfer has already ended — a late chunk must not
    /// resurrect something the panel has shown as finished.
    pub fn advance(&mut self, id: &str, received_bytes: u64) -> Option<Download> {
        let d = self.downloads.get_mut(id)?;
        if d.status.is_terminal() {
            return None;
        }
        d.received_bytes = received_bytes;
        Some(d.clone())
    }

    /// Move a transfer to a terminal status. Returns `None` if it was already
    /// terminal, so a cancel racing a completion cannot produce two endings —
    /// which for the agent would mean being woken twice for one file.
    pub fn finish(&mut self, id: &str, status: DownloadStatus) -> Option<Download> {
        let d = self.downloads.get_mut(id)?;
        if d.status.is_terminal() {
            return None;
        }
        d.status = status;
        d.finished_ms = Some(now_ms());
        Some(d.clone())
    }

    /// One transfer by id, or `None` once it has been cleared from the list.
    pub fn get(&self, id: &str) -> Option<&Download> {
        self.downloads.get(id)
    }

    /// Every transfer, newest first.
    pub fn list(&self) -> Vec<Download> {
        let mut all: Vec<Download> = self.downloads.values().cloned().collect();
        all.sort_by_key(|d| std::cmp::Reverse(d.started_ms));
        all
    }

    /// Every transfer `agent` started, newest first.
    ///
    /// What a client's panel is shown. A gateway can be serving several
    /// agents, and one agent's files are not another's to see, cancel, or
    /// learn the paths of.
    pub fn list_for(&self, agent: &str) -> Vec<Download> {
        let mut all: Vec<Download> = self
            .downloads
            .values()
            .filter(|d| d.belongs_to(agent))
            .cloned()
            .collect();
        all.sort_by_key(|d| std::cmp::Reverse(d.started_ms));
        all
    }

    /// Stop a running transfer on `agent`'s behalf.
    ///
    /// Returns the cancelled record, or `None` if the id is unknown, the
    /// transfer has already ended, or it belongs to another agent. The
    /// ownership check is here rather than at the call site because it is the
    /// same rule as [`Download::wakes`] and [`Self::list_for`]: ids are
    /// process-wide, so a client that guessed one could otherwise stop a
    /// download it was never shown.
    pub fn cancel(&mut self, id: &str, agent: &str) -> Option<Download> {
        let d = self.downloads.get(id)?;
        if !d.belongs_to(agent) {
            return None;
        }
        self.finish(id, DownloadStatus::Cancelled)
    }

    /// Drop `agent`'s finished transfers, leaving its running ones — and every
    /// other agent's — alone.
    ///
    /// Returns the ids that went, rather than a count. Two things outside this
    /// module are keyed by transfer id and have no other way to learn an id
    /// stopped meaning anything — the panels on *other* connections, which
    /// would otherwise keep rendering entries the registry has forgotten, and
    /// the gateway's announcement claim set, which would otherwise grow for
    /// the life of the process.
    pub fn clear_finished_for(&mut self, agent: &str) -> Vec<DownloadId> {
        let going: Vec<DownloadId> = self
            .downloads
            .values()
            .filter(|d| d.status.is_terminal() && d.belongs_to(agent))
            .map(|d| d.id.clone())
            .collect();
        self.downloads.retain(|id, _| !going.contains(id));
        going
    }

    /// Drop finished transfers, leaving running ones alone. Returns how many
    /// went.
    pub fn clear_finished(&mut self) -> usize {
        let before = self.downloads.len();
        self.downloads.retain(|_, d| !d.status.is_terminal());
        before - self.downloads.len()
    }
}

/// Thread-safe download manager.
pub type SharedDownloadManager = Arc<Mutex<DownloadManager>>;

static DOWNLOAD_MANAGER: OnceLock<SharedDownloadManager> = OnceLock::new();
static DOWNLOAD_EVENTS: OnceLock<tokio::sync::broadcast::Sender<DownloadEvent>> = OnceLock::new();

/// The global download manager.
pub fn download_manager() -> &'static SharedDownloadManager {
    DOWNLOAD_MANAGER.get_or_init(|| Arc::new(Mutex::new(DownloadManager::new())))
}

/// Lock the registry, recovering it if a thread panicked while holding it.
///
/// Poison is recovered rather than propagated, because of what this data is
/// and what the callers do with a failure. A `PoisonError` says a thread
/// panicked mid-update; it does not say the map is inconsistent, and there is
/// no invariant spanning two entries that a panic could leave half-applied —
/// every operation here is one insert or one record's fields.
///
/// Both alternatives are worse, and the first is worse in a way that is easy
/// to miss. A caller that maps a failed lock to `None` cannot tell it from
/// "this transfer already ended", which is precisely the cancel signal the
/// write loop watches for: one poisoned lock would make every running
/// transfer look cancelled, stopping it mid-write *without* marking it
/// terminal, so it would sit in the panel as Running forever — uncancellable,
/// unclearable, and never waking the agent that was waiting on it. A caller
/// that surfaces the error instead breaks the panel permanently, since poison
/// never clears once set.
///
/// Recovering leaves `None` with exactly one meaning and keeps the "exactly
/// one ending" invariant true.
pub fn lock_registry() -> std::sync::MutexGuard<'static, DownloadManager> {
    download_manager()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Subscribe to transfer changes.
///
/// Lagging is survivable by construction: every event carries the whole
/// record, so a consumer that misses one is at worst briefly stale and is
/// corrected by the next. A dropped *terminal* event would cost the agent a
/// notification, which is why the capacity is generous relative to how often
/// these can be produced.
pub fn subscribe() -> tokio::sync::broadcast::Receiver<DownloadEvent> {
    events().subscribe()
}

fn events() -> &'static tokio::sync::broadcast::Sender<DownloadEvent> {
    DOWNLOAD_EVENTS.get_or_init(|| tokio::sync::broadcast::channel(256).0)
}

/// Announce a change. Sending with no subscribers is not a failure — a
/// headless run has nobody watching and the registry is still the record.
pub fn announce(download: Download) {
    let _ = events().send(DownloadEvent::Changed(download));
}

/// Tell every watcher that `agent`'s transfers named by `ids` are gone.
///
/// Separate from [`announce`] because the audiences differ: this reaches the
/// panels and must not reach the agent. Sending nothing when nothing went
/// keeps a no-op clear from redrawing every window.
pub fn announce_removed(agent: &str, ids: Vec<DownloadId>) {
    if ids.is_empty() {
        return;
    }
    let _ = events().send(DownloadEvent::Removed {
        agent: agent.to_string(),
        ids,
    });
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> DownloadManager {
        DownloadManager::new()
    }

    #[test]
    fn a_registered_download_starts_running_with_nothing_received() {
        let mut m = mgr();
        let d = m.register(
            "https://e/f.bin".into(),
            PathBuf::from("/tmp/f.bin"),
            Some(10),
            None,
        );
        assert_eq!(d.status, DownloadStatus::Running);
        assert_eq!(d.received_bytes, 0);
        assert!(d.finished_ms.is_none());
    }

    /// A file can only end once.
    ///
    /// Cancelling from the panel can land at the same moment the last chunk
    /// arrives. Both paths call `finish`, and the agent is woken per terminal
    /// event — so without this the user gets told twice about one file, once
    /// with the wrong ending.
    #[test]
    fn a_download_reaches_a_terminal_status_only_once() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/f.bin".into(),
                PathBuf::from("/tmp/f.bin"),
                None,
                None,
            )
            .id;

        assert!(m.finish(&id, DownloadStatus::Complete).is_some());
        assert!(
            m.finish(&id, DownloadStatus::Cancelled).is_none(),
            "a second ending must be refused"
        );
        assert_eq!(
            m.get(&id).map(|d| d.status.clone()),
            Some(DownloadStatus::Complete)
        );
    }

    /// A chunk arriving after the end does not reopen the transfer.
    #[test]
    fn progress_after_the_end_is_ignored() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/f.bin".into(),
                PathBuf::from("/tmp/f.bin"),
                Some(10),
                None,
            )
            .id;
        m.finish(&id, DownloadStatus::Complete);

        assert!(m.advance(&id, 5).is_none());
        assert_eq!(
            m.get(&id).map(|d| d.status.clone()),
            Some(DownloadStatus::Complete)
        );
    }

    #[test]
    fn progress_is_a_fraction_only_when_the_size_is_known() {
        let mut m = mgr();
        let sized = m
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                Some(200),
                None,
            )
            .id;
        let unsized_ = m
            .register("https://e/b".into(), PathBuf::from("/tmp/b"), None, None)
            .id;

        m.advance(&sized, 50);
        assert_eq!(m.get(&sized).and_then(|d| d.fraction()), Some(0.25));
        m.advance(&unsized_, 50);
        assert_eq!(
            m.get(&unsized_).and_then(|d| d.fraction()),
            None,
            "a chunked response has no percentage, and inventing one is worse than omitting it"
        );
    }

    /// A server that under-reports its length must not produce >100%.
    #[test]
    fn progress_past_the_declared_size_is_clamped() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                Some(100),
                None,
            )
            .id;
        m.advance(&id, 150);
        assert_eq!(m.get(&id).and_then(|d| d.fraction()), Some(1.0));
    }

    #[tokio::test]
    async fn a_turn_lends_its_origin_to_the_tools_it_calls() {
        assert!(
            current_origin().is_none(),
            "outside a turn there is nobody to wake"
        );
        let origin = DownloadOrigin {
            agent: "researcher".into(),
            connection: 7,
            thread: Some(3),
        };
        let seen = with_origin(origin.clone(), async { current_origin() }).await;
        assert_eq!(seen, Some(origin));
    }

    /// Why the origin is stamped at registration and not at completion.
    ///
    /// The transfer runs in a task the tool spawns, and a spawned task starts
    /// with an empty task-local set. Reading the origin from there would find
    /// nothing, and every completion would wake nobody.
    #[tokio::test]
    async fn a_spawned_transfer_cannot_read_the_turns_origin() {
        let origin = DownloadOrigin {
            agent: "researcher".into(),
            connection: 7,
            thread: Some(3),
        };
        let from_spawned = with_origin(origin, async {
            tokio::spawn(async { current_origin() })
                .await
                .expect("task panicked")
        })
        .await;
        assert_eq!(from_spawned, None);
    }

    /// Only the agent that started a transfer hears about it.
    ///
    /// Every connection in the process reads the same broadcast. Without this
    /// filter, a second agent would be told about a file it never asked for —
    /// and would be woken into a conversation of its own to discuss it.
    #[test]
    fn a_completion_wakes_only_the_agent_that_started_it() {
        let mut m = mgr();
        let mine = m
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                None,
                Some(DownloadOrigin {
                    agent: "researcher".into(),
                    connection: 1,
                    thread: Some(4),
                }),
            )
            .id;
        let finished = m
            .finish(&mine, DownloadStatus::Complete)
            .expect("first ending");

        assert!(finished.wakes("researcher"));
        assert!(
            !finished.wakes("archivist"),
            "another agent must not be woken"
        );
    }

    /// A quarter-megabyte of progress is not worth a turn.
    #[test]
    fn progress_never_wakes_the_agent() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                Some(1_000_000),
                Some(DownloadOrigin {
                    agent: "researcher".into(),
                    connection: 1,
                    thread: Some(4),
                }),
            )
            .id;
        let ticked = m.advance(&id, 262_144).expect("still running");
        assert!(!ticked.wakes("researcher"));
    }

    /// A transfer nobody owns wakes nobody, rather than whoever asks first.
    #[test]
    fn an_untagged_transfer_belongs_to_no_agent() {
        let mut m = mgr();
        let id = m
            .register("https://e/a".into(), PathBuf::from("/tmp/a"), None, None)
            .id;
        let finished = m
            .finish(&id, DownloadStatus::Complete)
            .expect("first ending");
        assert!(!finished.wakes("researcher"));
        assert!(!finished.wakes("archivist"));
    }

    /// A failure is as much worth waking for as a success — more so, since it
    /// is the case the agent has to do something about. The reason travels
    /// with it.
    #[test]
    fn a_failure_wakes_the_agent_and_says_why() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/a.bin".into(),
                PathBuf::from("/tmp/a.bin"),
                None,
                Some(DownloadOrigin {
                    agent: "researcher".into(),
                    connection: 1,
                    thread: Some(4),
                }),
            )
            .id;
        let failed = m
            .finish(
                &id,
                DownloadStatus::Failed {
                    error: "connection reset".into(),
                },
            )
            .expect("first ending");

        assert!(failed.wakes("researcher"));
        assert!(
            failed.summary().contains("connection reset"),
            "the agent has to know what went wrong to decide whether to retry: {}",
            failed.summary()
        );
    }

    #[test]
    fn sizes_are_reported_in_units_a_person_reads() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn connection_ids_are_never_reused() {
        let a = next_connection_id();
        let b = next_connection_id();
        assert_ne!(a, b);
    }

    /// One connection can change agents wholesale — `handle_agent_switch`
    /// swaps the session and its thread store — so the connection is not what
    /// a transfer belongs to. Were it, the panel would go on showing the
    /// previous agent's URLs and destination paths after a switch, and a
    /// completion would be filed against whatever conversation now happens to
    /// hold the remembered thread number.
    #[test]
    fn a_switch_on_one_connection_does_not_inherit_the_previous_agents_transfers() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/private.tar".into(),
                PathBuf::from("/tmp/private.tar"),
                None,
                Some(DownloadOrigin {
                    agent: "researcher".into(),
                    connection: 9,
                    thread: Some(3),
                }),
            )
            .id;

        // Same connection, different agent — the case the connection id cannot
        // tell apart.
        assert!(
            m.list_for("archivist").is_empty(),
            "the switched-to agent must not see the previous agent's transfers"
        );
        assert!(
            m.cancel(&id, "archivist").is_none(),
            "nor be able to stop them"
        );
        assert_eq!(m.list_for("researcher").len(), 1);
    }

    /// The other half of the same rule: an agent outlives the connection it
    /// was reached on. After a reconnect the client gets a fresh connection id,
    /// and a transfer scoped to the old one would be invisible, uncancellable
    /// and unclearable for the life of the process.
    #[test]
    fn a_reconnect_still_owns_the_transfers_it_started() {
        let mut m = mgr();
        let id = m
            .register(
                "https://e/big.iso".into(),
                PathBuf::from("/tmp/big.iso"),
                None,
                Some(DownloadOrigin {
                    agent: "researcher".into(),
                    connection: 1,
                    thread: Some(2),
                }),
            )
            .id;

        // Reconnected: same agent, new connection id. Ownership must survive.
        assert_eq!(m.list_for("researcher").len(), 1);
        assert!(
            m.cancel(&id, "researcher").is_some(),
            "a reconnected client must still be able to stop its own transfer"
        );
        assert_eq!(m.clear_finished_for("researcher"), vec![id.clone()]);
        assert!(m.get(&id).is_none());
    }

    /// A panic anywhere else must not wedge every running transfer.
    ///
    /// The write loop reads `advance` returning `None` as "cancelled from the
    /// panel" and stops writing. If a poisoned lock also produced `None` the
    /// two would be indistinguishable: the transfer would stop mid-file
    /// *without* being marked terminal, so it would sit in the panel as
    /// Running forever — never cleared, never cancelled, never waking the
    /// agent waiting on it. Recovering the guard is what keeps `None` meaning
    /// one thing.
    #[test]
    fn a_poisoned_registry_still_serves_the_write_loop() {
        // Poison the real global registry: a thread panicking while holding
        // the lock is exactly the situation being reproduced.
        let poisoner = std::thread::spawn(|| {
            let _guard = download_manager().lock().expect("first lock");
            panic!("deliberate panic while holding the registry lock");
        });
        assert!(poisoner.join().is_err(), "the helper thread should panic");
        assert!(
            download_manager().lock().is_err(),
            "the lock really is poisoned, so the rest of this test means something"
        );

        // The write loop's path must still work through it.
        let id = lock_registry()
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                None,
                Some(DownloadOrigin {
                    agent: "researcher".into(),
                    connection: 1,
                    thread: Some(2),
                }),
            )
            .id;
        assert!(
            lock_registry().advance(&id, 128).is_some(),
            "a poisoned lock must not look like a cancel"
        );
        let finished = lock_registry().finish(&id, DownloadStatus::Complete);
        assert!(
            finished.is_some(),
            "the transfer must still reach exactly one ending"
        );
        // Leave the global registry as it was found.
        lock_registry().clear_finished();
    }

    /// Clearing has to tell the ids that went, not just how many.
    ///
    /// Two things outside this module are keyed by transfer id and have no
    /// other way to learn an id has stopped meaning anything: the panels on
    /// other connections, which would keep rendering entries the registry has
    /// forgotten, and the gateway's announcement claim set, which would
    /// otherwise keep an entry per download for the life of the process.
    #[test]
    fn clearing_names_the_transfers_that_went() {
        let mut m = mgr();
        let origin = Some(DownloadOrigin {
            agent: "researcher".into(),
            connection: 1,
            thread: Some(2),
        });
        let done = m
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                None,
                origin.clone(),
            )
            .id;
        let running = m
            .register("https://e/b".into(), PathBuf::from("/tmp/b"), None, origin)
            .id;
        // Another agent's finished transfer, which must not be swept up.
        let theirs = m
            .register(
                "https://e/c".into(),
                PathBuf::from("/tmp/c"),
                None,
                Some(DownloadOrigin {
                    agent: "archivist".into(),
                    connection: 2,
                    thread: Some(1),
                }),
            )
            .id;
        m.finish(&done, DownloadStatus::Complete);
        m.finish(&theirs, DownloadStatus::Complete);

        assert_eq!(m.clear_finished_for("researcher"), vec![done.clone()]);
        assert!(m.get(&done).is_none());
        assert!(m.get(&running).is_some(), "a running transfer stays");
        assert!(
            m.get(&theirs).is_some(),
            "another agent's finished transfer is not this agent's to clear"
        );

        // A second clear finds nothing, so nothing is announced and no panel
        // redraws for a no-op.
        assert!(m.clear_finished_for("researcher").is_empty());
    }

    #[test]
    fn clearing_finished_leaves_running_transfers_alone() {
        let mut m = mgr();
        let done = m
            .register("https://e/a".into(), PathBuf::from("/tmp/a"), None, None)
            .id;
        let running = m
            .register("https://e/b".into(), PathBuf::from("/tmp/b"), None, None)
            .id;
        m.finish(&done, DownloadStatus::Complete);

        assert_eq!(m.clear_finished(), 1);
        assert!(m.get(&done).is_none());
        assert!(m.get(&running).is_some());
    }
}
