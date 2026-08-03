//! Thread manager — manages all agent threads.

use super::{AgentThread, MessageRole, ThreadEvent, ThreadId, ThreadInfo, ThreadStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{RwLock, broadcast};
use tracing::debug;

/// Configuration for thread management.
#[derive(Debug, Clone)]
pub struct ThreadManagerConfig {
    /// How long to keep completed ephemeral threads before cleanup
    pub ephemeral_retention: Duration,

    /// Maximum messages before compaction is triggered
    pub compaction_threshold: usize,

    /// How many recent messages to keep after compaction
    pub messages_after_compaction: usize,
}

impl Default for ThreadManagerConfig {
    fn default() -> Self {
        Self {
            ephemeral_retention: Duration::from_secs(300), // 5 minutes
            compaction_threshold: 20,
            messages_after_compaction: 5,
        }
    }
}

/// Manages all agent threads with event emission.
pub struct ThreadManager {
    /// All threads by ID
    threads: HashMap<ThreadId, AgentThread>,

    /// Currently foregrounded thread
    foreground_id: Option<ThreadId>,

    /// Event broadcast channel
    events_tx: broadcast::Sender<ThreadEvent>,

    /// Configuration
    config: ThreadManagerConfig,
}

impl ThreadManager {
    /// Create a new thread manager.
    pub fn new() -> Self {
        Self::with_config(ThreadManagerConfig::default())
    }

    /// Create with custom config.
    pub fn with_config(config: ThreadManagerConfig) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self {
            threads: HashMap::new(),
            foreground_id: None,
            events_tx,
            config,
        }
    }

    /// Subscribe to thread events.
    pub fn subscribe(&self) -> broadcast::Receiver<ThreadEvent> {
        self.events_tx.subscribe()
    }

    /// Build a manager from already-loaded threads — the store's loader.
    /// Callers still need [`Self::ensure_foreground`] afterwards.
    pub fn from_parts(threads: Vec<AgentThread>, foreground_id: Option<ThreadId>) -> Self {
        let (events_tx, _) = broadcast::channel(256);
        let mut mgr = Self {
            threads: HashMap::new(),
            foreground_id,
            events_tx,
            config: ThreadManagerConfig::default(),
        };
        for thread in threads {
            ThreadId::reserve_above(thread.id.0);
            mgr.threads.insert(thread.id, thread);
        }
        mgr
    }

    /// Every thread, mutably — the store drains pending log records
    /// through this.
    pub fn threads_mut(&mut self) -> impl Iterator<Item = &mut AgentThread> {
        self.threads.values_mut()
    }

    /// Record that a turn began in a thread. Until the matching
    /// [`Self::end_turn`] writes the stop indicator, the thread is open —
    /// what clients render as streaming, and what a restarted gateway
    /// resumes.
    pub fn begin_turn(&mut self, id: ThreadId) {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.begin_turn();
        }
    }

    /// Write a thread's stop indicator. `ok` is false for errors and
    /// cancellations. Threads with no open turn are left alone.
    pub fn end_turn(&mut self, id: ThreadId, ok: bool) {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.end_turn(ok);
        }
    }

    /// The threads whose last turn has no stop indicator — interrupted
    /// mid-answer by whatever ended the process that ran them.
    pub fn open_threads(&self) -> Vec<ThreadId> {
        let mut open: Vec<&AgentThread> = self.threads.values().filter(|t| t.is_open()).collect();
        open.sort_by_key(|t| t.created_at);
        open.iter().map(|t| t.id).collect()
    }

    // ── Thread Creation ─────────────────────────────────────────────────────

    /// Create a new chat thread and make it foreground.
    pub fn create_chat(&mut self, label: impl Into<String>) -> ThreadId {
        let mut thread = AgentThread::new_chat(label);
        thread.is_foreground = true;
        let id = thread.id;

        // Background the old foreground
        if let Some(old_fg) = self.foreground_id {
            if let Some(t) = self.threads.get_mut(&old_fg) {
                t.is_foreground = false;
            }
        }

        self.foreground_id = Some(id);
        let info = ThreadInfo::from(&thread);
        self.threads.insert(id, thread);

        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id: None,
        });

        id
    }

    /// Create a foreground chat thread belonging to a specific project.
    pub fn create_chat_in(
        &mut self,
        project_id: crate::projects::ProjectId,
        label: impl Into<String>,
    ) -> ThreadId {
        let id = self.create_chat(label);
        if let Some(t) = self.threads.get_mut(&id) {
            t.project_id = project_id;
        }
        id
    }

    /// All threads belonging to a project.
    pub fn threads_for(&self, project_id: crate::projects::ProjectId) -> Vec<&AgentThread> {
        let mut v: Vec<&AgentThread> = self
            .threads
            .values()
            .filter(|t| t.project_id == project_id)
            .collect();
        v.sort_by_key(|t| t.created_at);
        v
    }

    /// Reassign a thread to a different project. Returns false if unknown.
    pub fn set_project(&mut self, id: ThreadId, project_id: crate::projects::ProjectId) -> bool {
        if let Some(t) = self.threads.get_mut(&id) {
            t.project_id = project_id;
            true
        } else {
            false
        }
    }

    /// Set (or with `None`, clear) a thread's working-directory override.
    ///
    /// Clearing it makes the thread inherit its project's directory again —
    /// see [`ProjectManager::effective_dir_for`](crate::projects::ProjectManager::effective_dir_for).
    /// Returns false if the thread is unknown.
    pub fn set_working_dir(&mut self, id: ThreadId, dir: Option<std::path::PathBuf>) -> bool {
        if let Some(t) = self.threads.get_mut(&id) {
            t.working_dir = dir;
            true
        } else {
            false
        }
    }

    /// Create a sub-agent thread.
    pub fn create_subagent(
        &mut self,
        label: impl Into<String>,
        agent_id: impl Into<String>,
        task: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> ThreadId {
        let thread = AgentThread::new_subagent(label, agent_id, task, parent_id);
        let id = thread.id;
        let info = ThreadInfo::from(&thread);

        self.threads.insert(id, thread);

        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id,
        });

        id
    }

    /// Create a background thread.
    pub fn create_background(
        &mut self,
        label: impl Into<String>,
        purpose: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> ThreadId {
        let thread = AgentThread::new_background(label, purpose, parent_id);
        let id = thread.id;
        let info = ThreadInfo::from(&thread);

        self.threads.insert(id, thread);

        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id,
        });

        id
    }

    /// Create a task thread.
    pub fn create_task(
        &mut self,
        label: impl Into<String>,
        action: impl Into<String>,
        parent_id: Option<ThreadId>,
    ) -> ThreadId {
        let thread = AgentThread::new_task(label, action, parent_id);
        let id = thread.id;
        let info = ThreadInfo::from(&thread);

        self.threads.insert(id, thread);

        self.emit(ThreadEvent::Created {
            thread: info,
            parent_id,
        });

        id
    }

    // ── Thread Access ───────────────────────────────────────────────────────

    /// Get a thread by ID.
    pub fn get(&self, id: ThreadId) -> Option<&AgentThread> {
        self.threads.get(&id)
    }

    /// Get a mutable thread by ID.
    pub fn get_mut(&mut self, id: ThreadId) -> Option<&mut AgentThread> {
        self.threads.get_mut(&id)
    }

    /// Get the foreground thread.
    pub fn foreground(&self) -> Option<&AgentThread> {
        self.foreground_id.and_then(|id| self.threads.get(&id))
    }

    /// Get the foreground thread mutably.
    pub fn foreground_mut(&mut self) -> Option<&mut AgentThread> {
        self.foreground_id.and_then(|id| self.threads.get_mut(&id))
    }

    /// Get foreground thread ID.
    pub fn foreground_id(&self) -> Option<ThreadId> {
        self.foreground_id
    }

    /// List all threads.
    pub fn list(&self) -> Vec<&AgentThread> {
        self.threads.values().collect()
    }

    /// List thread info for sidebar display.
    pub fn list_info(&self) -> Vec<ThreadInfo> {
        self.threads.values().map(Into::into).collect()
    }

    // ── Thread Updates ──────────────────────────────────────────────────────

    /// Set description for a thread.
    pub fn set_description(&mut self, id: ThreadId, description: impl Into<String>) {
        let desc = description.into();
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.set_description(&desc);
            self.emit(ThreadEvent::DescriptionChanged {
                thread_id: id,
                description: desc,
            });
        }
    }

    /// Set description for the foreground thread.
    pub fn set_foreground_description(&mut self, description: impl Into<String>) {
        if let Some(id) = self.foreground_id {
            self.set_description(id, description);
        }
    }

    /// Update thread status.
    pub fn set_status(&mut self, id: ThreadId, status: ThreadStatus) {
        if let Some(thread) = self.threads.get_mut(&id) {
            let old_status = thread.status.clone();
            thread.set_status(status.clone());
            self.emit(ThreadEvent::StatusChanged {
                thread_id: id,
                old_status,
                new_status: status,
            });
        }
    }

    /// Switch foreground to a different thread.
    pub fn switch_foreground(&mut self, id: ThreadId) -> bool {
        if !self.threads.contains_key(&id) {
            return false;
        }

        let old_fg = self.foreground_id;

        // Background the old foreground
        if let Some(old_id) = old_fg {
            if let Some(t) = self.threads.get_mut(&old_id) {
                t.is_foreground = false;
            }
        }

        // Foreground the new one
        if let Some(t) = self.threads.get_mut(&id) {
            t.is_foreground = true;
        }

        self.foreground_id = Some(id);

        self.emit(ThreadEvent::Foregrounded {
            thread_id: id,
            previous_foreground: old_fg,
        });

        true
    }

    /// Clear the foreground — no thread is active (background all).
    pub fn clear_foreground(&mut self) {
        if let Some(old_id) = self.foreground_id {
            if let Some(t) = self.threads.get_mut(&old_id) {
                t.is_foreground = false;
            }
        }
        self.foreground_id = None;
    }

    /// Ensure *some* thread holds the foreground, electing the most recently
    /// active interactive thread when the current one is missing or dangling.
    /// Returns the foreground id, or `None` when there are no threads at all.
    ///
    /// A manager with threads but no foreground is a dead end for clients: the
    /// thread list they receive carries `foreground_id: None`, so they have no
    /// thread to request history for and render an empty transcript even
    /// though every thread's messages are sitting right here. That state is
    /// reachable — closing the foreground thread, or the "background the
    /// current thread" sentinel — and it is persisted, so it survives
    /// restarts. Electing a replacement keeps the invariant.
    pub fn ensure_foreground(&mut self) -> Option<ThreadId> {
        if let Some(id) = self.foreground_id
            && self.threads.contains_key(&id)
        {
            return Some(id);
        }
        // Prefer interactive (chat) threads; among those, the one the user
        // touched last. Fall back to any thread so a manager holding only
        // sub-agent/task threads still has a foreground.
        let elected = self
            .threads
            .values()
            .max_by_key(|t| (t.kind.is_interactive(), t.last_activity))
            .map(|t| t.id)?;
        self.switch_foreground(elected);
        Some(elected)
    }

    /// Rename a thread.
    pub fn rename(&mut self, id: ThreadId, new_label: impl Into<String>) -> bool {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.label = new_label.into();
            thread.last_activity = SystemTime::now();
            true
        } else {
            false
        }
    }

    /// Find the best matching thread for a given message content.
    /// Returns the thread ID if a good match is found, None otherwise.
    pub fn find_best_match(&self, content: &str) -> Option<ThreadId> {
        let content_lower = content.to_lowercase();

        // Look for threads where label or description matches content keywords
        for thread in self.threads.values() {
            // Skip the current foreground
            if thread.is_foreground {
                continue;
            }

            // Check if thread label is mentioned in content
            if content_lower.contains(&thread.label.to_lowercase()) {
                return Some(thread.id);
            }

            // Check if description keywords match
            if let Some(desc) = &thread.description {
                let desc_words: Vec<&str> =
                    desc.split_whitespace().filter(|w| w.len() > 3).collect();
                let matches = desc_words
                    .iter()
                    .filter(|w| content_lower.contains(&w.to_lowercase()))
                    .count();
                if matches >= 2 {
                    return Some(thread.id);
                }
            }
        }

        None
    }

    /// Mark a thread as completed.
    pub fn complete(&mut self, id: ThreadId, summary: Option<String>, result: Option<String>) {
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.complete(summary.clone(), result.clone());
            self.emit(ThreadEvent::Completed {
                thread_id: id,
                summary,
                result,
            });
        }
    }

    /// Mark a thread as failed.
    pub fn fail(&mut self, id: ThreadId, error: impl Into<String>) {
        let err = error.into();
        if let Some(thread) = self.threads.get_mut(&id) {
            thread.fail(&err);
            self.emit(ThreadEvent::Failed {
                thread_id: id,
                error: err,
            });
        }
    }

    /// Add a message to a thread.
    pub fn add_message(&mut self, id: ThreadId, role: MessageRole, content: impl Into<String>) {
        let message_count = if let Some(thread) = self.threads.get_mut(&id) {
            thread.add_message(role, content);
            thread.messages.len()
        } else {
            return;
        };

        self.emit(ThreadEvent::MessageAdded {
            thread_id: id,
            message_count,
        });
    }

    /// Add a message to the foreground thread.
    pub fn add_foreground_message(&mut self, role: MessageRole, content: impl Into<String>) {
        if let Some(id) = self.foreground_id {
            self.add_message(id, role, content);
        }
    }

    // ── Thread Removal ──────────────────────────────────────────────────────

    /// Remove a thread.
    ///
    /// Removing the foreground thread hands the foreground to another one
    /// rather than leaving the manager with none — see [`Self::ensure_foreground`].
    pub fn remove(&mut self, id: ThreadId) -> Option<AgentThread> {
        let thread = self.threads.remove(&id);
        if thread.is_some() {
            if self.foreground_id == Some(id) {
                self.foreground_id = None;
                self.ensure_foreground();
            }
            self.emit(ThreadEvent::Removed { thread_id: id });
        }
        thread
    }

    /// Clean up old ephemeral threads.
    pub fn cleanup_ephemeral(&mut self) {
        let now = SystemTime::now();
        let retention = self.config.ephemeral_retention;

        let to_remove: Vec<ThreadId> = self
            .threads
            .iter()
            .filter(|(_, t)| {
                t.kind.is_ephemeral()
                    && t.status.is_terminal()
                    && now
                        .duration_since(t.last_activity)
                        .map(|d| d > retention)
                        .unwrap_or(false)
            })
            .map(|(id, _)| *id)
            .collect();

        for id in to_remove {
            self.remove(id);
        }
    }

    // ── Context Building ────────────────────────────────────────────────────

    /// Build global context from all threads that share context.
    pub fn build_global_context(&self) -> String {
        let mut context = String::new();

        for thread in self.threads.values() {
            if !thread.share_context || thread.is_foreground {
                continue;
            }

            // Include summary or recent info for backgrounded threads
            if let Some(summary) = &thread.compact_summary {
                context.push_str(&format!(
                    "## {} ({})\n{}\n\n",
                    thread.label,
                    thread.kind.display_name(),
                    summary
                ));
            } else if !thread.messages.is_empty() {
                let recent: Vec<_> = thread.messages.iter().rev().take(2).collect();
                context.push_str(&format!(
                    "## {} ({}) - {} messages\n",
                    thread.label,
                    thread.kind.display_name(),
                    thread.messages.len()
                ));
                for msg in recent.into_iter().rev() {
                    context.push_str(&format!(
                        "{:?}: {}\n",
                        msg.role,
                        &msg.content[..msg.content.len().min(100)]
                    ));
                }
                context.push('\n');
            }
        }

        context
    }

    // ── Persistence ─────────────────────────────────────────────────────────

    /// Save threads to a file.
    ///
    /// Writes to a sibling temporary file and renames it into place. Every
    /// thread's entire message history lives in this one document, so a
    /// direct `write` that fails partway — a full disk, a killed process —
    /// would leave truncated JSON behind. `load_from_file` cannot parse that,
    /// and `load_or_default` responds by starting a fresh manager and
    /// overwriting the file, which turns one failed write into the loss of
    /// every thread. `rename` is atomic on both Unix and Windows, so readers
    /// see either the previous file or the complete new one.
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let state = PersistentState {
            threads: self.threads.values().cloned().collect(),
            foreground_id: self.foreground_id,
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let tmp = path.with_extension("json.tmp");
        // Flush and fsync before the rename: a rename that lands ahead of the
        // data would publish an empty file after a crash.
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(json.as_bytes())?;
            file.flush()?;
            file.sync_all()?;
        }
        match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Don't leave the temporary behind on failure.
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// Load threads from a file.
    pub fn load_from_file(path: &Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let state: PersistentState = serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let (events_tx, _) = broadcast::channel(256);
        let mut mgr = Self {
            threads: HashMap::new(),
            foreground_id: state.foreground_id,
            events_tx,
            config: ThreadManagerConfig::default(),
        };

        for thread in state.threads {
            // Restored ids were minted by a previous process, whose counter
            // died with it. Reserve above them or the next `create_chat` will
            // mint an id that is already taken and overwrite that thread.
            ThreadId::reserve_above(thread.id.0);
            mgr.threads.insert(thread.id, thread);
        }

        // A persisted `foreground_id` can be null (the "background the current
        // thread" sentinel) or point at a thread that was since closed. Either
        // way clients would get `foreground_id: None` and never ask for any
        // history, so re-elect one here.
        mgr.ensure_foreground();

        Ok(mgr)
    }

    /// Load from file or create with default chat thread.
    pub fn load_or_default(path: &Path) -> Self {
        match Self::load_from_file(path) {
            Ok(mgr) => {
                debug!("Loaded {} threads from {:?}", mgr.threads.len(), path);
                mgr
            }
            Err(e) => {
                debug!("Creating new thread manager (load failed: {})", e);
                let mut mgr = Self::new();
                mgr.create_chat("Main");
                if let Err(save_err) = mgr.save_to_file(path) {
                    debug!(
                        "Failed to persist default thread manager to {:?}: {}",
                        path, save_err
                    );
                }
                mgr
            }
        }
    }

    // ── Backwards Compatibility ─────────────────────────────────────────────
    // These methods match the old tasks::ThreadManager API for easier migration.

    /// Alias for create_chat (old API compatibility).
    pub fn create_thread(&mut self, label: impl Into<String>) -> ThreadId {
        self.create_chat(label)
    }

    /// Alias for switch_foreground that returns old foreground ID (old API compatibility).
    pub fn switch_to(&mut self, id: ThreadId) -> Option<ThreadId> {
        let old_fg = self.foreground_id;
        if self.switch_foreground(id) {
            old_fg
        } else {
            None
        }
    }

    /// Get a thread by ID (compatibility - already exists as get()).
    pub fn get_by_id(&self, id: ThreadId) -> Option<&AgentThread> {
        self.get(id)
    }

    /// Get a mutable thread by ID (compatibility - already exists as get_mut()).
    pub fn get_by_id_mut(&mut self, id: ThreadId) -> Option<&mut AgentThread> {
        self.get_mut(id)
    }

    // ── Internal ────────────────────────────────────────────────────────────

    fn emit(&self, event: ThreadEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.events_tx.send(event);
    }
}

impl Default for ThreadManager {
    fn default() -> Self {
        Self::new()
    }
}

/// State for persistence.
#[derive(Debug, Serialize, Deserialize)]
struct PersistentState {
    threads: Vec<AgentThread>,
    foreground_id: Option<ThreadId>,
}

/// Shared thread manager type.
pub type SharedThreadManager = Arc<RwLock<ThreadManager>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threads::ThreadKind;

    #[test]
    fn test_create_chat_thread() {
        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Test Chat");

        assert!(mgr.get(id).is_some());
        assert_eq!(mgr.foreground_id(), Some(id));
        assert!(mgr.get(id).unwrap().is_foreground);
    }

    #[test]
    fn test_create_subagent() {
        let mut mgr = ThreadManager::new();
        let parent = mgr.create_chat("Main");
        let child = mgr.create_subagent("Worker", "gpt-4", "Do the thing", Some(parent));

        let thread = mgr.get(child).unwrap();
        assert!(matches!(thread.kind, ThreadKind::SubAgent { .. }));
        assert_eq!(thread.parent_id, Some(parent));
    }

    #[test]
    fn test_switch_foreground() {
        let mut mgr = ThreadManager::new();
        let id1 = mgr.create_chat("Chat 1");
        let id2 = mgr.create_chat("Chat 2");

        assert_eq!(mgr.foreground_id(), Some(id2));
        assert!(!mgr.get(id1).unwrap().is_foreground);

        mgr.switch_foreground(id1);
        assert_eq!(mgr.foreground_id(), Some(id1));
        assert!(mgr.get(id1).unwrap().is_foreground);
        assert!(!mgr.get(id2).unwrap().is_foreground);
    }

    #[test]
    fn test_set_description() {
        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Test");

        mgr.set_description(id, "Working on taxes");
        assert_eq!(
            mgr.get(id).unwrap().description.as_deref(),
            Some("Working on taxes")
        );
    }

    #[test]
    fn test_complete_and_fail() {
        let mut mgr = ThreadManager::new();

        let task1 = mgr.create_task("Task 1", "Do thing", None);
        mgr.complete(task1, Some("Done!".into()), Some("result data".into()));
        assert!(mgr.get(task1).unwrap().status.is_terminal());
        assert_eq!(
            mgr.get(task1).unwrap().result.as_deref(),
            Some("result data")
        );

        let task2 = mgr.create_task("Task 2", "Do other thing", None);
        mgr.fail(task2, "Something went wrong");
        assert!(matches!(
            mgr.get(task2).unwrap().status,
            ThreadStatus::Failed { .. }
        ));
    }

    #[test]
    fn test_list_info() {
        let mut mgr = ThreadManager::new();
        mgr.create_chat("Chat");
        mgr.create_subagent("Worker", "gpt-4", "task", None);

        let info = mgr.list_info();
        assert_eq!(info.len(), 2);
    }

    /// Save a manager to a temp file and read it back, the way a restarted
    /// gateway does.
    fn round_trip(mgr: &ThreadManager, name: &str) -> ThreadManager {
        let path = std::env::temp_dir().join(format!(
            "rustyclaw-threads-{}-{name}.json",
            std::process::id()
        ));
        mgr.save_to_file(&path).expect("save threads");
        let loaded = ThreadManager::load_from_file(&path).expect("load threads");
        let _ = std::fs::remove_file(&path);
        loaded
    }

    /// Regression: thread ids outlive the process that minted them. A
    /// restarted gateway must not hand a restored thread's id to a new
    /// thread — doing so overwrites the restored thread and loses its
    /// entire message history.
    #[test]
    fn new_threads_never_collide_with_restored_ids() {
        let mut mgr = ThreadManager::new();
        let kept = mgr.create_chat("Has history");
        mgr.add_message(kept, MessageRole::User, "hello");
        mgr.add_message(kept, MessageRole::Assistant, "hi there");

        let mut restored = round_trip(&mgr, "id-collision");
        let fresh = restored.create_chat("Brand new");

        assert_ne!(fresh, kept, "a new thread reused a restored thread's id");
        assert_eq!(restored.list().len(), 2);
        assert_eq!(
            restored.get(kept).map(|t| t.messages.len()),
            Some(2),
            "the restored thread's history was overwritten"
        );
    }

    /// Regression: a manager that holds threads must always name a
    /// foreground. Clients drive history loading off `foreground_id`, so a
    /// `None` persisted by "background the current thread" leaves every
    /// client with an empty transcript on the next start.
    #[test]
    fn loading_reelects_a_foreground_when_none_was_persisted() {
        let mut mgr = ThreadManager::new();
        let chat = mgr.create_chat("Only chat");
        mgr.add_message(chat, MessageRole::User, "hello");
        mgr.clear_foreground();
        assert_eq!(mgr.foreground_id(), None);

        let restored = round_trip(&mgr, "no-foreground");
        assert_eq!(restored.foreground_id(), Some(chat));
        assert_eq!(restored.foreground().map(|t| t.messages.len()), Some(1));
    }

    /// Regression: the save must be atomic. Thread history is one document,
    /// so a partial write leaves unparseable JSON — and `load_or_default`
    /// answers a parse failure by starting fresh and overwriting the file,
    /// turning one bad write into the loss of every thread. A pre-existing
    /// truncated temp file must also not be mistaken for the real thing.
    #[test]
    fn saving_never_leaves_a_partial_file_behind() {
        let dir = std::env::temp_dir().join(format!("rustyclaw-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("threads.json");

        let mut mgr = ThreadManager::new();
        let chat = mgr.create_chat("Lengthy");
        for i in 0..500 {
            mgr.add_message(chat, MessageRole::User, format!("message {i}"));
        }
        mgr.save_to_file(&path).expect("save");

        // Leftover junk at the temp path must not affect the real file.
        std::fs::write(path.with_extension("json.tmp"), "{ truncated").unwrap();
        mgr.save_to_file(&path).expect("save over stale temp");

        let restored = ThreadManager::load_from_file(&path).expect("still parseable");
        assert_eq!(
            restored.get(chat).map(|t| t.messages.len()),
            Some(500),
            "every message survives the round trip"
        );
        // The temp file is not left lying around next to the real one.
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temporary file is renamed away, not orphaned"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compaction summarises the model's context; it must never destroy
    /// the conversation. The old implementation popped the summarised
    /// messages off the thread — and the thread's messages are the
    /// transcript every client loads, so switching threads or crossing
    /// the context threshold truncated the visible conversation to its
    /// last few messages, permanently.
    #[test]
    fn compaction_keeps_the_transcript_whole() {
        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Long chat");
        for i in 0..10 {
            mgr.add_message(id, MessageRole::User, format!("message {i}"));
        }

        let thread = mgr.get_mut(id).unwrap();
        thread.apply_compaction_keeping("what came before".into(), 3);

        assert_eq!(
            thread.messages.len(),
            10,
            "every message is still in the record"
        );
        assert_eq!(thread.compacted_up_to, 7);
        let ctx = thread.build_context();
        assert!(
            ctx.contains("what came before") && ctx.contains("message 9"),
            "context is summary plus the uncompacted tail"
        );
        assert!(
            !ctx.contains("message 0"),
            "summarised messages stay out of the model context"
        );
        // Re-compacting covers only the tail, seeded with the prior summary.
        let prompt = thread.compaction_prompt();
        assert!(prompt.contains("what came before"));
        assert!(!prompt.contains("message 0"));
    }

    /// The elected foreground is an interactive thread, not a sub-agent or
    /// task thread that happens to have been active more recently.
    #[test]
    fn foreground_election_prefers_chat_threads() {
        let mut mgr = ThreadManager::new();
        let chat = mgr.create_chat("Chat");
        mgr.create_subagent("Worker", "gpt-4", "task", Some(chat));
        mgr.clear_foreground();

        assert_eq!(mgr.ensure_foreground(), Some(chat));
    }

    /// Closing the foreground thread hands the foreground to another thread
    /// rather than leaving the manager (and every client) with none.
    #[test]
    fn removing_the_foreground_elects_a_replacement() {
        let mut mgr = ThreadManager::new();
        let first = mgr.create_chat("First");
        let second = mgr.create_chat("Second");
        assert_eq!(mgr.foreground_id(), Some(second));

        mgr.remove(second);
        assert_eq!(mgr.foreground_id(), Some(first));
        assert!(mgr.get(first).unwrap().is_foreground);

        // The last thread going away is the one case with nothing to elect.
        mgr.remove(first);
        assert_eq!(mgr.foreground_id(), None);
    }

    /// A working-directory override survives a save/load round trip, and
    /// clearing it really clears it rather than leaving the old path behind.
    #[test]
    fn working_dir_override_round_trips_and_clears() {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-threads-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("threads.json");

        let mut mgr = ThreadManager::new();
        let id = mgr.create_chat("Pinned");
        assert!(mgr.set_working_dir(id, Some("/tmp/worktree".into())));
        mgr.save_to_file(&path).unwrap();

        let reloaded = ThreadManager::load_from_file(&path).unwrap();
        assert_eq!(
            reloaded.get(id).unwrap().working_dir,
            Some(std::path::PathBuf::from("/tmp/worktree"))
        );

        let mut mgr = reloaded;
        assert!(mgr.set_working_dir(id, None));
        mgr.save_to_file(&path).unwrap();
        let reloaded = ThreadManager::load_from_file(&path).unwrap();
        assert_eq!(reloaded.get(id).unwrap().working_dir, None);

        // Unknown threads report failure rather than silently doing nothing.
        let mut mgr = reloaded;
        assert!(!mgr.set_working_dir(ThreadId(9_999), Some("/tmp/x".into())));

        std::fs::remove_dir_all(&dir).ok();
    }
}
