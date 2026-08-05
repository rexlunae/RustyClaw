//! One live [`ThreadManager`] per store on disk.
//!
//! A manager is not a view of a store — it is the *authority* for one.
//! [`ThreadStore::persist`] acts on that authority: it deletes the files of
//! every thread the manager it is handed does not contain, so that a thread
//! closed in memory does not rise from the dead on the next load. That is
//! only safe while exactly one manager speaks for a store.
//!
//! Loading a fresh manager per connection broke it. Two windows on one agent
//! each held a snapshot taken when they connected, and the first write after
//! the other created a thread removed that thread from disk — not a narrow
//! race, but anything the other window had done since it opened.
//!
//! So managers live here, keyed by the store's path. The path is what a
//! manager is the authority *for*: two agents have separate directories, and
//! so do two installations pointed at different settings directories, which
//! keying by agent id would have collapsed into one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use super::{ThreadManager, ThreadStore};

/// A manager shared by everything working against one store.
pub type SharedThreadMgr = Arc<tokio::sync::Mutex<ThreadManager>>;

static MANAGERS: LazyLock<Mutex<HashMap<PathBuf, SharedThreadMgr>>> =
    LazyLock::new(Default::default);

/// The manager for the store at `threads_path`, loading it from disk on
/// first use and returning the same one to every later caller.
pub fn manager_for(threads_path: &Path) -> SharedThreadMgr {
    MANAGERS
        .lock()
        .expect("thread manager registry poisoned")
        .entry(threads_path.to_path_buf())
        .or_insert_with(|| {
            Arc::new(tokio::sync::Mutex::new(ThreadStore::load_or_migrate(
                threads_path,
            )))
        })
        .clone()
}

/// Forget every manager whose store lives under `dir`.
///
/// Holding managers for the life of the process is what makes one of them
/// the authority, but a directory can be deleted out from under it. Without
/// this, an agent recreated under a deleted one's id is handed the dead
/// manager, and its first write puts the deleted agent's conversations back
/// on disk.
///
/// By directory rather than by exact file, and called from
/// `AgentRegistry::delete` rather than from the handlers that ask for a
/// deletion: an agent can be removed by a client frame, by the `agents_delete`
/// tool, or by swarm teardown, and all three go through that one function.
/// Hooking the callers instead means every future deletion path has to
/// remember to do this, and the failure is silent.
pub fn forget_managers_under(dir: &Path) {
    MANAGERS
        .lock()
        .expect("thread manager registry poisoned")
        .retain(|path, _| !path.starts_with(dir));
}

/// Counts of live sessions per store, so a store nobody is using can be
/// told apart from one that is open somewhere.
static OPEN_SESSIONS: LazyLock<Mutex<HashMap<PathBuf, usize>>> = LazyLock::new(Default::default);

/// A live user of the store at `threads_path`. Releases on drop.
///
/// Evicting a manager removes it from the table, but anything already
/// holding the `Arc` keeps writing through it — and `ThreadStore::persist`
/// recreates the store directory, so a deleted agent's conversations come
/// back and a later agent under the same id loads a *second* manager beside
/// the stale one. Exactly the two-authorities condition this module exists
/// to prevent, reached from the other direction.
///
/// Counting live users is what lets deletion refuse instead.
pub struct StoreSession {
    path: PathBuf,
}

impl Drop for StoreSession {
    fn drop(&mut self) {
        let mut open = OPEN_SESSIONS
            .lock()
            .expect("thread manager registry poisoned");
        if let Some(count) = open.get_mut(&self.path) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.path);
            }
        }
    }
}

/// Register a live user of a store for as long as the returned value lives.
pub fn open_session(threads_path: &Path) -> StoreSession {
    *OPEN_SESSIONS
        .lock()
        .expect("thread manager registry poisoned")
        .entry(threads_path.to_path_buf())
        .or_insert(0) += 1;
    StoreSession {
        path: threads_path.to_path_buf(),
    }
}

/// How many live users any store under `dir` has.
pub fn sessions_open_under(dir: &Path) -> usize {
    OPEN_SESSIONS
        .lock()
        .expect("thread manager registry poisoned")
        .iter()
        .filter(|(path, _)| path.starts_with(dir))
        .map(|(_, count)| *count)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustyclaw-mgr-registry-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The same store hands back the same manager, so there is only ever one
    /// authority to reconcile from.
    #[tokio::test]
    async fn one_manager_per_store() {
        let dir = temp_dir("same");
        let path = dir.join("threads.json");

        let a = manager_for(&path);
        a.lock().await.create_chat("written through a");
        let b = manager_for(&path);

        // A fresh store comes with a default thread, so this asks whether
        // the second caller sees the *first caller's* work rather than
        // counting.
        assert!(
            b.lock()
                .await
                .list()
                .iter()
                .any(|t| t.label == "written through a"),
            "a second caller must see the first's work, not a stale reload"
        );
        assert!(Arc::ptr_eq(&a, &b));
        forget_managers_under(&dir);
    }

    /// A different store is a different authority.
    #[tokio::test]
    async fn separate_stores_do_not_share() {
        let dir = temp_dir("separate");
        let one = manager_for(&dir.join("a/threads.json"));
        let two = manager_for(&dir.join("b/threads.json"));
        one.lock().await.create_chat("only in a");
        assert!(
            !two.lock()
                .await
                .list()
                .iter()
                .any(|t| t.label == "only in a")
        );
        forget_managers_under(&dir);
    }

    /// Deleting the directory forgets what was cached for it, so the next
    /// caller loads from disk rather than inheriting the dead agent's
    /// conversations.
    #[tokio::test]
    async fn forgetting_a_directory_drops_its_managers() {
        let dir = temp_dir("forget");
        let agent = dir.join("agents/researcher");
        let path = agent.join("sessions/threads.json");

        let before = manager_for(&path);
        before.lock().await.create_chat("secret plans");

        forget_managers_under(&agent);

        let after = manager_for(&path);
        assert!(
            !Arc::ptr_eq(&before, &after),
            "the cached manager should not have survived its directory"
        );
        assert!(
            !after
                .lock()
                .await
                .list()
                .iter()
                .any(|t| t.label == "secret plans"),
            "a recreated agent must not inherit the deleted one's threads"
        );
        forget_managers_under(&dir);
    }
}
