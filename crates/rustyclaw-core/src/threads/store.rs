//! Per-thread persistent storage.
//!
//! Each thread is stored on its own, so persisting new activity appends to
//! that thread's log instead of rewriting every conversation the agent has
//! ever had:
//!
//! ```text
//! <state>/threads/
//!   state.json            – the foreground pointer
//!   <id>.meta.json        – one thread's metadata (small, rewritten whole)
//!   <id>.log.jsonl        – one thread's event log (append-only)
//! ```
//!
//! The log is the record: messages in chronological order, interleaved with
//! explicit turn markers ([`ThreadLogRecord`]). A `TurnStarted` with no
//! `TurnEnded` after it means the turn never finished — the process died
//! mid-answer — and the loader reports the thread as open so the gateway
//! can resume it. Nothing stores "streaming" as a flag that someone has to
//! remember to clear.
//!
//! The old format — every thread serialized into one `threads.json`,
//! rewritten in full at every persistence point — is migrated on first
//! load and the legacy file renamed out of the way.

use crate::ignore::Ignore;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::{AgentThread, ThreadId, ThreadLogRecord, ThreadManager, ThreadStatus};

/// A thread's identity as [`ThreadStore::peek`] reports it — enough to list
/// it and to route to it, without the weight (or side effects) of a full
/// load.
#[derive(Clone, Debug)]
pub struct ThreadSummary {
    /// Thread id within its agent's store.
    pub id: u64,
    /// Label as the sidebar shows it.
    pub label: String,
    /// Effective working directory, when one was set.
    pub working_dir: Option<PathBuf>,
}

/// A thread's metadata, exactly as written to `<id>.meta.json` — everything
/// but the message log, which lives in the sibling `.log.jsonl`.
#[derive(Debug, Serialize, Deserialize)]
struct ThreadMeta {
    id: ThreadId,
    #[serde(default)]
    project_id: crate::projects::ProjectId,
    kind: super::ThreadKind,
    label: String,
    description: Option<String>,
    status: ThreadStatus,
    parent_id: Option<ThreadId>,
    created_at: SystemTime,
    last_activity: SystemTime,
    is_foreground: bool,
    compact_summary: Option<String>,
    #[serde(default)]
    compacted_up_to: usize,
    #[serde(default)]
    working_dir: Option<PathBuf>,
    result: Option<String>,
    #[serde(default)]
    pinned: bool,
    share_context: bool,
    #[serde(default)]
    memory_flushed: bool,
}

impl From<&AgentThread> for ThreadMeta {
    fn from(t: &AgentThread) -> Self {
        Self {
            id: t.id,
            project_id: t.project_id,
            kind: t.kind.clone(),
            label: t.label.clone(),
            description: t.description.clone(),
            status: t.status.clone(),
            parent_id: t.parent_id,
            created_at: t.created_at,
            last_activity: t.last_activity,
            is_foreground: t.is_foreground,
            compact_summary: t.compact_summary.clone(),
            compacted_up_to: t.compacted_up_to,
            working_dir: t.working_dir.clone(),
            result: t.result.clone(),
            pinned: t.pinned,
            share_context: t.share_context,
            memory_flushed: t.memory_flushed,
        }
    }
}

impl From<ThreadMeta> for AgentThread {
    fn from(m: ThreadMeta) -> Self {
        Self {
            id: m.id,
            project_id: m.project_id,
            kind: m.kind,
            label: m.label,
            description: m.description,
            status: m.status,
            parent_id: m.parent_id,
            created_at: m.created_at,
            last_activity: m.last_activity,
            is_foreground: m.is_foreground,
            messages: std::collections::VecDeque::new(),
            compact_summary: m.compact_summary,
            compacted_up_to: m.compacted_up_to,
            working_dir: m.working_dir,
            result: m.result,
            pinned: m.pinned,
            share_context: m.share_context,
            memory_flushed: m.memory_flushed,
            open_turn: None,
            compacting: false,
            pending_log: Vec::new(),
        }
    }
}

/// The store-level state that is not any one thread's: the foreground.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreState {
    foreground_id: Option<ThreadId>,
}

/// Per-thread storage rooted at a `threads/` directory.
pub struct ThreadStore {
    root: PathBuf,
}

impl ThreadStore {
    /// The store that replaces a legacy `threads.json` path: a `threads/`
    /// directory next to it. Every call site that used to pass the file
    /// path keeps passing it, and the store derives its root here — one
    /// definition, no caller can point the two layouts at different homes.
    pub fn at_legacy_path(threads_json: &Path) -> Self {
        let dir = threads_json.parent().unwrap_or(Path::new("."));
        Self {
            root: dir.join("threads"),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.root.join("state.json")
    }

    fn meta_path(&self, id: ThreadId) -> PathBuf {
        self.root.join(format!("{}.meta.json", id.0))
    }

    fn log_path(&self, id: ThreadId) -> PathBuf {
        self.root.join(format!("{}.log.jsonl", id.0))
    }

    /// Persist the manager: append each thread's pending log records, write
    /// its (small) metadata, record the foreground, and remove the files of
    /// threads that no longer exist.
    ///
    /// Appending is the point: the old single-file layout rewrote every
    /// thread's entire history to say one message was added. Here a message
    /// costs one line in one file, and history already on disk is never
    /// touched again.
    pub fn persist(&self, tm: &mut ThreadManager) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;

        let mut live = std::collections::HashSet::new();
        let foreground_id = tm.foreground_id();
        for thread in tm.threads_mut() {
            live.insert(thread.id);
            let log_path = self.log_path(thread.id);
            // A thread whose log vanished (or was never written — a freshly
            // migrated or restored manager) gets it rebuilt whole; everyone
            // else appends only what happened since the last persist.
            if !log_path.exists() && !thread.messages.is_empty() {
                let records: Vec<ThreadLogRecord> = thread
                    .messages
                    .iter()
                    .cloned()
                    .map(ThreadLogRecord::Message)
                    .collect();
                append_records(&log_path, &records)?;
                // The pending log holds a suffix of `messages` (plus
                // markers); the rebuild wrote all messages already, so keep
                // only the markers to avoid duplicating those messages.
                thread
                    .pending_log
                    .retain(|r| !matches!(r, ThreadLogRecord::Message(_)));
            }
            if !thread.pending_log.is_empty() {
                append_records(&log_path, &thread.pending_log)?;
                thread.pending_log.clear();
            }
            write_atomically(
                &self.meta_path(thread.id),
                &serde_json::to_vec_pretty(&ThreadMeta::from(&*thread))
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            )?;
        }

        write_atomically(
            &self.state_path(),
            &serde_json::to_vec_pretty(&StoreState { foreground_id })
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        )?;

        // A closed thread's files go with it — otherwise it rises from the
        // dead on the next load.
        for entry in std::fs::read_dir(&self.root)?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = name
                .strip_suffix(".meta.json")
                .or_else(|| name.strip_suffix(".log.jsonl"))
                .and_then(|stem| stem.parse::<u64>().ok())
            else {
                continue;
            };
            if !live.contains(&ThreadId(id)) {
                std::fs::remove_file(entry.path()).ignore();
            }
        }
        Ok(())
    }

    /// Rewrite a thread's whole log file from its in-memory state.
    ///
    /// The normal write path is append-only — one line per new record, so
    /// history already on disk is never touched. Deleting a message can't
    /// be expressed as an append, so this replaces the file: every
    /// remaining message, plus the open-turn marker if the thread is still
    /// streaming. The pending log is drained (its messages are all in
    /// `thread.messages`, and its markers are re-derived), so the next
    /// `persist` appends only genuinely new records.
    pub fn rewrite_thread_log(&self, thread: &mut AgentThread) -> std::io::Result<()> {
        let mut records: Vec<ThreadLogRecord> = thread
            .messages
            .iter()
            .cloned()
            .map(ThreadLogRecord::Message)
            .collect();
        if let Some(at) = thread.open_turn {
            records.push(ThreadLogRecord::TurnStarted { at });
        }
        let mut content = String::new();
        for record in &records {
            content.push_str(
                &serde_json::to_string(record)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            );
            content.push('\n');
        }
        write_atomically(&self.log_path(thread.id), content.as_bytes())?;
        thread.pending_log.clear();
        Ok(())
    }

    /// Load the store into a manager. Threads whose log ends inside a turn
    /// — a `TurnStarted` with no stop indicator — come back open, exactly
    /// as they were left.
    pub fn load(&self) -> std::io::Result<ThreadManager> {
        let mut threads = Vec::new();
        for entry in std::fs::read_dir(&self.root)?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.ends_with(".meta.json") {
                continue;
            }
            let meta: ThreadMeta = match std::fs::read_to_string(entry.path())
                .map_err(anyhow_io)
                .and_then(|s| serde_json::from_str(&s).map_err(anyhow_io))
            {
                Ok(meta) => meta,
                Err(e) => {
                    // Skipping used to be quietly fatal: a thread absent
                    // from the loaded manager is one the next persist's
                    // reaper deletes the files of — a transient read error
                    // became a permanently deleted conversation. The log
                    // is still the full transcript, so quarantine the bad
                    // metadata and rebuild the thread around the log.
                    let Some(id) = name
                        .strip_suffix(".meta.json")
                        .and_then(|stem| stem.parse::<u64>().ok())
                        .map(ThreadId)
                    else {
                        continue;
                    };
                    tracing::error!(
                        file = %entry.path().display(),
                        error = %e,
                        "Thread metadata unreadable; rebuilding the thread from its log"
                    );
                    crate::persist::quarantine(&entry.path(), &e.to_string());
                    fallback_meta(id)
                }
            };
            let mut thread = AgentThread::from(meta);
            read_log_into(&self.log_path(thread.id), &mut thread);
            // A summary can only cover messages that are actually there —
            // a truncated log must not make context building skip the
            // whole conversation.
            thread.compacted_up_to = thread.compacted_up_to.min(thread.messages.len());
            // Messages persisted before stable ids existed load without
            // one; assign them now so every message in a loaded thread is
            // addressable (delete-by-id etc.) this session.
            for message in thread.messages.iter_mut() {
                if message.id.is_none() {
                    message.id = Some(uuid::Uuid::new_v4().to_string());
                }
            }
            ThreadId::reserve_above(thread.id.0);
            threads.push(thread);
        }

        let state: StoreState = std::fs::read_to_string(self.state_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut mgr = ThreadManager::from_parts(threads, state.foreground_id);
        mgr.ensure_foreground();
        Ok(mgr)
    }

    /// Enumerate a store's threads without loading logs, creating anything,
    /// or touching the process-wide id counter.
    ///
    /// `None` means the agent has no store at all. Unlike
    /// [`load_or_migrate`](Self::load_or_migrate), looking is not an event:
    /// no "Main" thread is materialised for an agent nobody has opened, no
    /// migration rewrites the store, and the global id floor is left alone —
    /// this is the one path that reads *other* agents' stores, and reserving
    /// from here would inflate every new thread's id to the
    /// installation-wide maximum.
    pub fn peek(threads_json: &Path) -> Option<Vec<ThreadSummary>> {
        let store = Self::at_legacy_path(threads_json);
        if store.state_path().exists() {
            let mut out = Vec::new();
            for entry in std::fs::read_dir(&store.root).ok()?.flatten() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if !name.ends_with(".meta.json") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let Ok(meta) = serde_json::from_str::<ThreadMeta>(&text) else {
                    continue;
                };
                out.push(ThreadSummary {
                    id: meta.id.0,
                    label: meta.label,
                    working_dir: meta.working_dir,
                });
            }
            return Some(out);
        }
        if threads_json.exists() {
            // Not yet migrated. Reading the legacy file does reserve ids —
            // a transitional wrinkle that disappears on the first real load,
            // which migrates the store — but it does not rewrite anything.
            let mgr = ThreadManager::load_from_file(threads_json).ok()?;
            return Some(
                mgr.list()
                    .iter()
                    .map(|t| ThreadSummary {
                        id: t.id.0,
                        label: t.label.clone(),
                        working_dir: t.working_dir.clone(),
                    })
                    .collect(),
            );
        }
        None
    }

    /// Load the per-thread store, migrating a legacy `threads.json` on the
    /// way if this is the first run since the layout changed; start fresh
    /// with a default chat thread when there is nothing to load at all.
    pub fn load_or_migrate(threads_json: &Path) -> ThreadManager {
        let store = Self::at_legacy_path(threads_json);
        if store.state_path().exists() {
            match store.load() {
                Ok(mgr) => return mgr,
                Err(e) => {
                    tracing::error!(
                        root = %store.root.display(),
                        error = %e,
                        "Failed to load thread store; starting fresh"
                    );
                }
            }
        } else if threads_json.exists() {
            match ThreadManager::load_from_file(threads_json) {
                Ok(mut mgr) => {
                    // Write the new layout, then move the legacy file out
                    // of the load path — kept, not deleted, so a failed
                    // migration can be retried by hand.
                    if let Err(e) = store.persist(&mut mgr) {
                        tracing::error!(
                            error = %e,
                            "Failed to write migrated thread store; keeping legacy file"
                        );
                    } else {
                        let backup = threads_json.with_extension("json.migrated");
                        if let Err(e) = std::fs::rename(threads_json, &backup) {
                            tracing::warn!(error = %e, "Could not rename legacy threads.json");
                        }
                        tracing::info!(
                            root = %store.root.display(),
                            threads = mgr.list().len(),
                            "Migrated threads.json to per-thread storage"
                        );
                    }
                    return mgr;
                }
                Err(e) => {
                    tracing::error!(
                        file = %threads_json.display(),
                        error = %e,
                        "Failed to load legacy threads.json; starting fresh"
                    );
                }
            }
        }

        let mut mgr = ThreadManager::new();
        mgr.create_chat("Main");
        if let Err(e) = store.persist(&mut mgr) {
            tracing::error!(error = %e, "Failed to persist fresh thread store");
        }
        mgr
    }
}

fn anyhow_io<E: std::error::Error + Send + Sync + 'static>(e: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

/// The metadata a thread gets when its own metadata cannot be read: enough
/// for the transcript in the log to load, display, and — critically —
/// count as live, so the persist reaper does not delete it. The next
/// persist writes a healthy replacement.
fn fallback_meta(id: ThreadId) -> ThreadMeta {
    let now = SystemTime::now();
    ThreadMeta {
        id,
        project_id: Default::default(),
        kind: super::ThreadKind::Chat,
        label: format!("Recovered thread {}", id.0),
        description: None,
        status: ThreadStatus::Active,
        parent_id: None,
        created_at: now,
        last_activity: now,
        is_foreground: false,
        compact_summary: None,
        compacted_up_to: 0,
        working_dir: None,
        result: None,
        pinned: false,
        share_context: true,
        memory_flushed: false,
    }
}

/// Append records to a log file as JSON lines, synced before returning so
/// an acknowledged message survives a crash.
fn append_records(path: &Path, records: &[ThreadLogRecord]) -> std::io::Result<()> {
    let mut buf = Vec::new();
    for record in records {
        serde_json::to_writer(&mut buf, record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        buf.push(b'\n');
    }
    crate::persist::append_durably(path, &buf)
}

/// Replay a thread's log: messages in order, turn markers folding into the
/// open/closed state. A line that does not parse ends the replay — with an
/// append-only log the torn line is the tail a crash left behind, and
/// everything before it is intact.
fn read_log_into(path: &Path, thread: &mut AgentThread) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<ThreadLogRecord>(line) {
            Ok(ThreadLogRecord::Message(message)) => thread.messages.push_back(message),
            Ok(ThreadLogRecord::TurnStarted { at }) => thread.open_turn = Some(at),
            Ok(ThreadLogRecord::TurnEnded { .. }) => thread.open_turn = None,
            Err(e) => {
                tracing::warn!(
                    file = %path.display(),
                    line = idx + 1,
                    error = %e,
                    "Thread log ends in a torn record; keeping everything before it"
                );
                break;
            }
        }
    }
}

use crate::persist::write_atomically;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threads::MessageRole;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-store-{}-{}-{name}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rand_suffix() -> u64 {
        use std::hash::{BuildHasher, Hasher};
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    }

    /// Everything round-trips: messages in order, metadata, foreground.
    #[test]
    fn a_manager_round_trips_through_the_store() {
        let dir = temp_root("round-trip");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let a = mgr.create_chat("Alpha");
        let b = mgr.create_chat("Beta");
        mgr.add_message(a, MessageRole::User, "first");
        mgr.add_message(a, MessageRole::Assistant, "second");
        mgr.add_message(b, MessageRole::User, "other conversation");
        mgr.switch_foreground(a);
        store.persist(&mut mgr).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.foreground_id(), Some(a));
        let thread_a = loaded.get(a).unwrap();
        assert_eq!(
            thread_a
                .messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"],
            "messages come back whole and in order"
        );
        assert_eq!(thread_a.label, "Alpha");
        assert_eq!(loaded.get(b).unwrap().messages.len(), 1);

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// Persisting twice appends — the second persist must not duplicate
    /// what the first already wrote.
    #[test]
    fn repeated_persists_append_without_duplicating() {
        let dir = temp_root("append");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Chat");
        mgr.add_message(id, MessageRole::User, "one");
        store.persist(&mut mgr).unwrap();
        mgr.add_message(id, MessageRole::Assistant, "two");
        store.persist(&mut mgr).unwrap();
        // A persist with nothing new writes nothing new.
        store.persist(&mut mgr).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(
            loaded
                .get(id)
                .unwrap()
                .messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// An open turn — started, never ended — survives the round trip as
    /// open. This is the stop-indicator contract: the gateway that died
    /// mid-answer left no `TurnEnded`, and the next start must see the
    /// thread as still streaming so it can resume the turn.
    #[test]
    fn a_turn_without_a_stop_indicator_loads_as_open() {
        let dir = temp_root("open-turn");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Chat");
        mgr.add_message(id, MessageRole::User, "do the thing");
        mgr.begin_turn(id);
        store.persist(&mut mgr).unwrap();

        let loaded = store.load().unwrap();
        assert!(
            loaded.get(id).unwrap().is_open(),
            "no stop indicator means the turn is still open"
        );

        // The stop indicator closes it.
        let mut mgr = loaded;
        mgr.end_turn(id, true);
        store.persist(&mut mgr).unwrap();
        let loaded = store.load().unwrap();
        assert!(!loaded.get(id).unwrap().is_open());

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// A torn tail — the half-written line a crash leaves — costs exactly
    /// that line, not the file. The old single-document layout lost every
    /// thread to one bad write.
    #[test]
    fn a_torn_log_tail_keeps_everything_before_it() {
        let dir = temp_root("torn");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Chat");
        mgr.add_message(id, MessageRole::User, "kept");
        mgr.add_message(id, MessageRole::Assistant, "also kept");
        store.persist(&mut mgr).unwrap();

        use std::io::Write;
        let log = dir.join("threads").join(format!("{}.log.jsonl", id.0));
        let mut f = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(b"{\"kind\":\"message\",\"trunc").unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.get(id).unwrap().messages.len(),
            2,
            "the torn record is dropped, the intact history is kept"
        );

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// The legacy single-file layout migrates on first load: same threads,
    /// same messages, and the old file moved aside rather than deleted.
    #[test]
    fn a_legacy_threads_json_migrates_to_the_store() {
        let dir = temp_root("migrate");
        let legacy = dir.join("threads.json");

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Old world");
        mgr.add_message(id, MessageRole::User, "from before");
        mgr.save_to_file(&legacy).unwrap();

        let migrated = ThreadStore::load_or_migrate(&legacy);
        assert_eq!(
            migrated.get(id).map(|t| t.messages.len()),
            Some(1),
            "history survives the migration"
        );
        assert!(!legacy.exists(), "the legacy file is moved out of the way");
        assert!(legacy.with_extension("json.migrated").exists());

        // The next load comes from the store, not the (absent) legacy file.
        let reloaded = ThreadStore::load_or_migrate(&legacy);
        assert_eq!(reloaded.get(id).map(|t| t.messages.len()), Some(1));

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// Closing a thread removes its files; it must not come back on the
    /// next load.
    #[test]
    fn a_removed_thread_stays_removed() {
        let dir = temp_root("remove");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let keep = mgr.create_chat("Keep");
        let drop_ = mgr.create_chat("Drop");
        mgr.add_message(drop_, MessageRole::User, "doomed");
        store.persist(&mut mgr).unwrap();

        mgr.remove(drop_);
        store.persist(&mut mgr).unwrap();

        let loaded = store.load().unwrap();
        assert!(loaded.get(keep).is_some());
        assert!(loaded.get(drop_).is_none(), "closed threads stay closed");

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// `persist` reconciles: it is not safe to call with a manager that is no
    /// longer the authority on which threads exist.
    ///
    /// This is what makes persisting from a detached background task
    /// dangerous. The deletion itself is wanted — a closed thread must not
    /// rise from the dead on the next load — but it means a *stale* manager
    /// silently destroys threads created since it was built. Pinned here so
    /// the sharp edge is visible from the store rather than only from the
    /// callers that have to avoid it.
    #[test]
    fn persist_deletes_threads_the_manager_does_not_know_about() {
        let dir = temp_root("stale-persist");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);
        let root = dir.join("threads");

        // What was on disk when an earlier connection built its manager.
        let mut first = ThreadManager::new();
        let kept = first.create_chat("kept");
        first.add_message(kept, MessageRole::User, "hello");
        store.persist(&mut first).expect("first persist");

        // A task spawned by that connection still holds this snapshot.
        let mut stale = store.load().expect("load snapshot");
        assert!(stale.get(kept).is_some());

        // A later connection loads afresh and adds a thread.
        let mut later = store.load().expect("load again");
        let created_later = later.create_chat("created later");
        store.persist(&mut later).expect("later persist");
        assert!(root.join(format!("{}.meta.json", created_later.0)).exists());

        // The stale snapshot writes, and reconciliation takes the newer
        // thread with it — it was never in this manager to begin with.
        store.persist(&mut stale).expect("stale persist");
        assert!(
            !root.join(format!("{}.meta.json", created_later.0)).exists(),
            "a write from a stale manager deleted a thread it had never seen"
        );
        assert!(root.join(format!("{}.meta.json", kept.0)).exists());
    }
    /// A thread whose metadata is corrupt used to be skipped on load — and
    /// a skipped thread is one the next persist's reaper deletes the files
    /// of, so a transient read error became a permanently deleted
    /// conversation. The transcript must instead come back from the log,
    /// the bad metadata must be quarantined, and a subsequent persist must
    /// leave the log alone.
    #[test]
    fn corrupt_metadata_recovers_the_thread_from_its_log() {
        let dir = temp_root("meta-recovery");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Precious");
        mgr.add_message(id, MessageRole::User, "irreplaceable");
        mgr.add_message(id, MessageRole::Assistant, "history");
        store.persist(&mut mgr).unwrap();

        // Corrupt the metadata; leave the log intact.
        let meta_path = dir.join("threads").join(format!("{}.meta.json", id.0));
        std::fs::write(&meta_path, b"{ torn").unwrap();

        let mut loaded = store.load().unwrap();
        let thread = loaded.get(id).expect("thread must survive bad metadata");
        assert_eq!(
            thread
                .messages
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>(),
            vec!["irreplaceable", "history"],
            "the transcript comes back from the log"
        );

        // The reaper must treat the recovered thread as live.
        store.persist(&mut loaded).unwrap();
        let log_path = dir.join("threads").join(format!("{}.log.jsonl", id.0));
        assert!(
            log_path.exists(),
            "persist must not reap the recovered thread"
        );
        // And the rewritten metadata is healthy again.
        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.get(id).unwrap().messages.len(), 2);

        // The torn metadata was kept for inspection.
        let quarantined = std::fs::read_dir(dir.join("threads"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .count();
        assert_eq!(quarantined, 1);

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// A deleted message must stay deleted across a reload: `remove_message`
    /// drops it from memory and the pending log, and `rewrite_thread_log`
    /// rewrites the append-only log without it.
    #[test]
    fn deleting_a_message_rewrites_the_log() {
        let dir = temp_root("delete-rewrite");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Trimmer");
        {
            let thread = mgr.get_mut(id).unwrap();
            thread.add_message_with_id(Some("keep-1".into()), MessageRole::User, "keep me");
            thread.add_message_with_id(Some("gone-1".into()), MessageRole::Assistant, "delete me");
        }
        store.persist(&mut mgr).unwrap();

        let mut loaded = store.load().unwrap();
        let thread = loaded.get_mut(id).unwrap();
        let removed = thread
            .remove_message("gone-1")
            .expect("the message exists");
        assert_eq!(removed.content, "delete me");
        store.rewrite_thread_log(thread).unwrap();

        let reloaded = store.load().unwrap();
        let thread = reloaded.get(id).unwrap();
        assert_eq!(thread.messages.len(), 1, "only the kept message remains");
        assert_eq!(thread.messages[0].id.as_deref(), Some("keep-1"));
        assert_eq!(thread.messages[0].content, "keep me");

        std::fs::remove_dir_all(&dir).ignore();
    }

    /// `rewrite_thread_log` must not drop an open-turn marker: a thread with
    /// a live turn on screen has one, and deleting an older message while
    /// the turn streams must leave the marker (and the thread) open.
    #[test]
    fn rewrite_thread_log_keeps_an_open_turn() {
        let dir = temp_root("rewrite-open-turn");
        let legacy = dir.join("threads.json");
        let store = ThreadStore::at_legacy_path(&legacy);

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Open");
        mgr.add_message(id, MessageRole::User, "prompt");
        mgr.begin_turn(id); // open turn marker
        mgr.add_message(id, MessageRole::Assistant, "partial");

        store.persist(&mut mgr).unwrap();
        let mut loaded = store.load().unwrap();
        let thread = loaded.get_mut(id).unwrap();
        assert!(
            thread.open_turn.is_some(),
            "the open turn survived the round trip"
        );
        store.rewrite_thread_log(thread).unwrap();

        let reloaded = store.load().unwrap();
        let thread = reloaded.get(id).unwrap();
        assert_eq!(thread.messages.len(), 2);
        assert!(
            thread.open_turn.is_some(),
            "rewrite preserved the open turn"
        );

        std::fs::remove_dir_all(&dir).ignore();
    }
}
