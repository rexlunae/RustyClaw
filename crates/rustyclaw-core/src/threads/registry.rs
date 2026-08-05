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
use std::sync::{Arc, LazyLock, Mutex, Weak};

use super::{ThreadManager, ThreadStore};

/// A manager shared by everything working against one store.
pub type SharedThreadMgr = Arc<tokio::sync::Mutex<ThreadManager>>;

/// Weak, so a store nobody is using is dropped rather than held for the
/// life of the process.
///
/// A manager is not a handle: `ThreadStore::load` reads every thread's log
/// into memory, so an entry is an agent's entire conversation history. The
/// per-connection managers this replaced were at least dropped on
/// disconnect; keeping strong references here would have meant a
/// long-running gateway retaining every agent anyone had ever opened.
///
/// Weakness also makes "is anyone using this store" exact, and exactly the
/// right question. It is not "does a window have it open" — a turn spawned
/// before an agent switch outlives the session that started it and goes on
/// persisting through the manager it captured. Whoever holds an `Arc` is a
/// user, whatever they are, and that is what `strong_count` reports.
static MANAGERS: LazyLock<Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<ThreadManager>>>>> =
    LazyLock::new(Default::default);

/// The manager for the store at `threads_path`, loading it from disk when
/// nobody currently holds one and returning the live one when somebody does.
pub fn manager_for(threads_path: &Path) -> SharedThreadMgr {
    let mut managers = MANAGERS.lock().expect("thread manager registry poisoned");
    if let Some(live) = managers.get(threads_path).and_then(Weak::upgrade) {
        return live;
    }
    let manager = Arc::new(tokio::sync::Mutex::new(ThreadStore::load_or_migrate(
        threads_path,
    )));
    managers.insert(threads_path.to_path_buf(), Arc::downgrade(&manager));
    manager
}

/// Whether anything is still working against a store under `dir`.
///
/// Prunes dead entries while it is here — nothing else would, and they cost
/// a path each.
pub fn store_in_use_under(dir: &Path) -> bool {
    let mut managers = MANAGERS.lock().expect("thread manager registry poisoned");
    managers.retain(|_, weak| weak.strong_count() > 0);
    managers.keys().any(|path| path.starts_with(dir))
}

/// Remove `dir` from disk, but only while nothing is working against a
/// store inside it. Returns whether it was removed.
///
/// One operation rather than check-then-remove-then-forget, because the gaps
/// between those are usable: a connection calling [`manager_for`] after the
/// check gets a live manager for a store that is being deleted, and its next
/// persist recreates the directory. Worse, once the entry has been dropped
/// from the map, the *next* caller builds a second manager for the same
/// path — two authorities for one store, which is the condition this module
/// exists to remove.
///
/// So the lock is held across all three. The filesystem work happens under
/// it, which is not free, but a delete is rare and the alternative is a
/// window that cannot be closed from outside.
pub fn remove_store_dir_if_unused(dir: &Path) -> std::io::Result<bool> {
    let mut managers = MANAGERS.lock().expect("thread manager registry poisoned");
    managers.retain(|_, weak| weak.strong_count() > 0);
    if managers.keys().any(|path| path.starts_with(dir)) {
        return Ok(false);
    }
    std::fs::remove_dir_all(dir)?;
    // Only dead entries can be under `dir` now, but they would otherwise
    // linger until something happened to prune them.
    managers.retain(|path, _| !path.starts_with(dir));
    Ok(true)
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

    /// A store nobody holds is released, and reloaded on next use.
    ///
    /// The entry is weak precisely so an agent's whole message history does
    /// not sit in memory for the life of the process once its windows have
    /// gone.
    #[tokio::test]
    async fn a_store_nobody_holds_is_released() {
        let dir = temp_dir("released");
        let path = dir.join("threads.json");

        let first = manager_for(&path);
        first.lock().await.create_chat("while it was held");
        assert!(store_in_use_under(&dir), "someone is holding it");

        drop(first);
        assert!(
            !store_in_use_under(&dir),
            "with nothing holding it, the store is no longer in use"
        );

        // Reloading is from disk, so unpersisted work is gone — the same as
        // the per-connection managers this replaced.
        let second = manager_for(&path);
        assert!(
            !second
                .lock()
                .await
                .list()
                .iter()
                .any(|t| t.label == "while it was held")
        );
        forget_managers_under(&dir);
    }

    /// "In use" means anyone holding the manager, not just an open window.
    ///
    /// A turn spawned before an agent switch outlives the session that
    /// started it and keeps persisting through the manager it captured, so
    /// asking about windows would answer the wrong question.
    #[tokio::test]
    async fn a_holder_that_is_not_a_window_still_counts_as_in_use() {
        let dir = temp_dir("holder");
        let agent = dir.join("agents/busy");
        let path = agent.join("sessions/threads.json");

        // Stands in for a running turn: it holds the manager and nothing
        // else does.
        let running_turn = manager_for(&path);

        assert!(
            store_in_use_under(&agent),
            "a running turn holding the manager keeps the store in use"
        );
        drop(running_turn);
        assert!(!store_in_use_under(&agent));
        forget_managers_under(&dir);
    }
}
