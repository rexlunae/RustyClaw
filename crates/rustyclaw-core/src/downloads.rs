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
/// agent" is meaningless in a gateway serving several connections, and thread
/// ids restart low in every agent's store — so a bare thread id would name a
/// conversation belonging to whichever agent asked first. The connection id
/// is what makes the pair unambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOrigin {
    /// The connection whose agent ran the tool. Unique per process.
    pub connection: u64,
    /// The conversation the tool call belonged to, when there was one.
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
    ORIGIN.try_with(|o| *o).ok()
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
    /// Which connection and conversation started it. `None` when nothing was
    /// listening — a transfer still gets tracked, it just wakes nobody.
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

    /// Whether this event is one the agent on `connection` should be woken
    /// for.
    ///
    /// Both halves matter and neither is obvious from the call site, which is
    /// why the rule lives here with its tests. Progress must not wake anyone
    /// — a large file would otherwise start a turn every quarter-megabyte —
    /// and an untagged transfer belongs to no connection, so a gateway
    /// serving two agents must not let either claim it.
    pub fn wakes(&self, connection: u64) -> bool {
        self.status.is_terminal() && self.origin.map(|o| o.connection) == Some(connection)
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
}

/// The set of transfers this process knows about.
#[derive(Debug, Default)]
pub struct DownloadManager {
    downloads: HashMap<DownloadId, Download>,
    next_id: u64,
}

impl DownloadManager {
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

    pub fn get(&self, id: &str) -> Option<&Download> {
        self.downloads.get(id)
    }

    /// Every transfer, newest first.
    pub fn list(&self) -> Vec<Download> {
        let mut all: Vec<Download> = self.downloads.values().cloned().collect();
        all.sort_by_key(|d| std::cmp::Reverse(d.started_ms));
        all
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
            connection: 7,
            thread: Some(3),
        };
        let seen = with_origin(origin, async { current_origin() }).await;
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

    /// Only the connection that started a transfer hears about it.
    ///
    /// Every connection in the process reads the same broadcast. Without this
    /// filter, a second agent would be told about a file it never asked for —
    /// and would be woken into a conversation of its own to discuss it.
    #[test]
    fn a_completion_wakes_only_the_connection_that_started_it() {
        let mut m = mgr();
        let mine = m
            .register(
                "https://e/a".into(),
                PathBuf::from("/tmp/a"),
                None,
                Some(DownloadOrigin {
                    connection: 1,
                    thread: Some(4),
                }),
            )
            .id;
        let finished = m
            .finish(&mine, DownloadStatus::Complete)
            .expect("first ending");

        assert!(finished.wakes(1));
        assert!(!finished.wakes(2), "another connection must not be woken");
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
                    connection: 1,
                    thread: Some(4),
                }),
            )
            .id;
        let ticked = m.advance(&id, 262_144).expect("still running");
        assert!(!ticked.wakes(1));
    }

    /// A transfer nobody owns wakes nobody, rather than whoever asks first.
    #[test]
    fn an_untagged_transfer_belongs_to_no_connection() {
        let mut m = mgr();
        let id = m
            .register("https://e/a".into(), PathBuf::from("/tmp/a"), None, None)
            .id;
        let finished = m
            .finish(&id, DownloadStatus::Complete)
            .expect("first ending");
        assert!(!finished.wakes(1));
        assert!(!finished.wakes(2));
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

        assert!(failed.wakes(1));
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
