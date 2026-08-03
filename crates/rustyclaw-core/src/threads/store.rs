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

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::{AgentThread, ThreadId, ThreadLogRecord, ThreadManager, ThreadStatus};

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
            share_context: m.share_context,
            memory_flushed: m.memory_flushed,
            open_turn: None,
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
                let _ = std::fs::remove_file(entry.path());
            }
        }
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
                    tracing::error!(
                        file = %entry.path().display(),
                        error = %e,
                        "Skipping unreadable thread metadata"
                    );
                    continue;
                }
            };
            let mut thread = AgentThread::from(meta);
            read_log_into(&self.log_path(thread.id), &mut thread);
            // A summary can only cover messages that are actually there —
            // a truncated log must not make context building skip the
            // whole conversation.
            thread.compacted_up_to = thread.compacted_up_to.min(thread.messages.len());
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

/// Append records to a log file as JSON lines, synced before returning so
/// an acknowledged message survives a crash.
fn append_records(path: &Path, records: &[ThreadLogRecord]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut buf = Vec::new();
    for record in records {
        serde_json::to_writer(&mut buf, record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        buf.push(b'\n');
    }
    file.write_all(&buf)?;
    file.flush()?;
    file.sync_all()
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

/// Write a small file atomically: temp sibling, fsync, rename.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

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

        let _ = std::fs::remove_dir_all(&dir);
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

        let _ = std::fs::remove_dir_all(&dir);
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

        let _ = std::fs::remove_dir_all(&dir);
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

        let _ = std::fs::remove_dir_all(&dir);
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

        let _ = std::fs::remove_dir_all(&dir);
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

        let _ = std::fs::remove_dir_all(&dir);
    }
}
