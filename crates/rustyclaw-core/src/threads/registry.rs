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
///
/// The load happens *outside* the registry lock. `ThreadStore::load` reads
/// every thread's log into memory — see the note on [`MANAGERS`] — and this
/// runs on every connect and every agent switch, so holding the lock across
/// it would make one agent's history the thing every other window waits on
/// to open an unrelated agent. Same hazard the deletion path is shaped
/// around, and it deserves the same treatment.
///
/// The empty manager goes into the map *locked*, which is what keeps the
/// one-manager-per-path invariant while the lock is not held: a concurrent
/// caller finds this entry rather than building a second authority for the
/// path, and gets an `Arc` it cannot read until the load finishes and the
/// guard drops. It waits on the manager's own mutex — the one it would have
/// to take to use it regardless — instead of on the registry.
pub fn manager_for(threads_path: &Path) -> SharedThreadMgr {
    let (manager, mut loading) = {
        let mut managers = MANAGERS.lock().expect("thread manager registry poisoned");
        if let Some(live) = managers.get(threads_path).and_then(Weak::upgrade) {
            return live;
        }
        let manager = Arc::new(tokio::sync::Mutex::new(ThreadManager::new()));
        // Fresh and unpublished, so this cannot fail — and `try_lock_owned`
        // rather than `blocking_lock`, which panics inside a runtime.
        let loading = manager
            .clone()
            .try_lock_owned()
            .expect("a just-created manager cannot already be locked");
        managers.insert(threads_path.to_path_buf(), Arc::downgrade(&manager));
        (manager, loading)
    };
    *loading = ThreadStore::load_or_migrate(threads_path);
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
/// So the lock is held across all three — but only across the *cheap*
/// three. `dir` is the whole agent folder, workspace included, and that can
/// be a checkout with a build tree in it; a recursive delete of one is not
/// the sort of work to do while every other window in the process waits to
/// open an unrelated agent.
///
/// The rename is what makes the two separable. Renaming is O(1) and, once
/// done, `dir` no longer names anything: a [`manager_for`] arriving a moment
/// later loads a fresh empty store rather than reviving the doomed one, and
/// there is still never a second manager for a path that has one. Only the
/// bulk removal happens outside the lock, against a path nothing can reach
/// by name.
///
/// The caller still waits for the delete, so a directory this returns `true`
/// for really is gone. (The `remove_dir_all` remains a blocking call on
/// whatever thread invoked it — one thread, no longer every one of them.)
///
/// The rename is also the point of no return, and the two failure paths
/// differ because of it. Before it, an error means nothing happened and the
/// agent is still there, so it is reported. After it, the agent is already
/// detached — gone from `AgentRegistry::list`, its managers forgotten — and
/// a failure to erase the bytes does not put it back. Reporting *that* as a
/// failed deletion would tell the user their agent survived while they watch
/// it disappear; it is a reclaim that did not finish, and it is logged and
/// swept, not raised.
pub fn remove_store_dir_if_unused(dir: &Path) -> std::io::Result<bool> {
    let doomed = {
        let mut managers = MANAGERS.lock().expect("thread manager registry poisoned");
        managers.retain(|_, weak| weak.strong_count() > 0);
        if managers.keys().any(|path| path.starts_with(dir)) {
            return Ok(false);
        }
        // Only dead entries can be under `dir` now, but they would otherwise
        // linger until something happened to prune them.
        managers.retain(|path, _| !path.starts_with(dir));
        match rename_aside(dir) {
            Ok(doomed) => doomed,
            // Nothing to gain by reporting a rename failure as the outcome
            // when the delete itself may well work: fall back to the
            // in-place removal, lock and all. Rare path, correct either way.
            Err(_) => {
                std::fs::remove_dir_all(dir)?;
                return Ok(true);
            }
        }
    };
    if let Err(error) = std::fs::remove_dir_all(&doomed) {
        tracing::warn!(
            path = %doomed.display(),
            %error,
            "Could not finish erasing a deleted agent's files — the agent is \
             gone, but its data is still on disk and will be swept on a later \
             delete"
        );
    }
    if let Some(parent) = doomed.parent() {
        sweep_abandoned_deletes(parent);
    }
    Ok(true)
}

/// Re-attempt any removals that were left half-done.
///
/// A scratch directory outlives its delete whenever the removal failed or
/// the process died holding one, and nothing else would ever look at it
/// again — the next delete of the same agent picks a fresh number rather
/// than colliding with it. Retrying here keeps that from being permanent
/// without needing a startup hook: the code that makes the litter is the
/// code that collects it.
///
/// Best-effort throughout. Two deletes running at once can reach for the
/// same leftover, and the loser gets a `NotFound` it has no reason to care
/// about — they were both trying to erase the same doomed bytes.
fn sweep_abandoned_deletes(agents_root: &Path) {
    let Ok(entries) = std::fs::read_dir(agents_root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') && name.contains(".deleting.") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Move `dir` out of the way, returning where it went.
///
/// The name keeps the result from being mistaken for an agent, which it
/// otherwise would be: a manifest-bearing directory sitting in the agents
/// root is exactly what `AgentRegistry::list` goes looking for. Two things
/// independently rule it out — the leading dot, which `list` skips before it
/// checks for a manifest, and the interior dots, which make the id fail
/// `is_valid_agent_id` and so give `AgentRegistry::manifest` nothing to
/// return. Either alone would do; a rename to some tidy `<name>_old` would
/// satisfy neither.
///
/// The counter keeps concurrent deletes apart within a process, and the
/// `exists` check steps over anything a previous run died holding.
fn rename_aside(dir: &Path) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let invalid = |what| std::io::Error::new(std::io::ErrorKind::InvalidInput, what);
    let parent = dir.parent().ok_or_else(|| invalid("no parent directory"))?;
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("directory has no usable name"))?;

    for _ in 0..64 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(".{name}.deleting.{n}"));
        if candidate.exists() {
            continue;
        }
        std::fs::rename(dir, &candidate)?;
        return Ok(candidate);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free scratch name for the directory being deleted",
    ))
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

    /// A delete leaves the directory gone, not merely renamed.
    ///
    /// The removal moved out from under the registry lock, so the thing to
    /// pin is that it still finishes before the call returns: a caller told
    /// `true` has had the data deleted, and no scratch directory is left
    /// behind in the agents root.
    #[test]
    fn a_completed_delete_leaves_nothing_behind() {
        let dir = temp_dir("swept");
        let agents_root = dir.join("agents");
        let agent = agents_root.join("doomed");
        std::fs::create_dir_all(agent.join("workspace/nested")).unwrap();
        std::fs::write(agent.join("agent.toml"), "id = 'doomed'").unwrap();
        std::fs::write(agent.join("workspace/nested/file.txt"), "payload").unwrap();

        assert!(remove_store_dir_if_unused(&agent).unwrap());
        assert!(!agent.exists(), "the agent directory should be gone");

        let leftovers: Vec<_> = std::fs::read_dir(&agents_root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(
            leftovers.is_empty(),
            "the scratch directory should not outlive the delete; found {leftovers:?}"
        );
        forget_managers_under(&dir);
    }

    /// The directory a delete moves aside cannot be read back as an agent.
    ///
    /// While the removal runs — and for good, if the process dies during it
    /// — the scratch directory is a manifest-bearing folder sitting in the
    /// agents root, which is the whole definition of an agent as far as
    /// `AgentRegistry::list` is concerned save for one thing: the leading
    /// dot. So the name comes from `rename_aside` itself rather than being
    /// written out here, or the test would only be checking that `list`
    /// skips dotfiles and not that this code produces one.
    #[test]
    fn the_directory_a_delete_moves_aside_is_not_an_agent() {
        let dir = temp_dir("moved-aside");
        let agents_root = dir.join("agents");
        let agent = agents_root.join("doomed");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(agent.join("agent.toml"), "name = \"Doomed\"").unwrap();

        let registry = crate::agents::AgentRegistry::new(&dir, "Main");
        let ids = || -> Vec<String> { registry.list().into_iter().map(|a| a.id).collect() };

        // Establish that it really is listable to begin with. Without this
        // the assertion after the rename holds for any reason at all — a
        // manifest that failed to parse would satisfy it just as well.
        assert!(
            ids().iter().any(|id| id == "doomed"),
            "the fixture should be a real agent before it is deleted; got {:?}",
            ids()
        );

        let moved = rename_aside(&agent).unwrap();
        assert!(
            moved.join("agent.toml").exists(),
            "it still looks like an agent from the inside"
        );

        let listed = ids();
        assert_eq!(
            listed,
            vec![crate::agents::MAIN_AGENT_ID.to_string()],
            "a directory mid-delete should not appear as an agent; found {listed:?}"
        );

        // And a leftover does not block a later delete of the same id.
        std::fs::create_dir_all(&agent).unwrap();
        assert!(remove_store_dir_if_unused(&agent).unwrap());
        assert!(!agent.exists());
        forget_managers_under(&dir);
    }

    /// What is on disk is in the manager by the time a caller can read it.
    ///
    /// The load moved out from under the registry lock, so the entry now
    /// goes into the map *before* it holds anything — empty, and locked
    /// until the read finishes. That is what keeps a concurrent caller from
    /// building a second authority for the path, but it is also a way to
    /// hand out a manager that never got filled: forget the write-back and
    /// every agent silently opens with no history.
    #[tokio::test]
    async fn a_store_is_loaded_before_the_manager_is_readable() {
        let dir = temp_dir("loaded");
        let path = dir.join("threads.json");

        // Put something on disk, then let go of everything holding it.
        {
            let seeded = manager_for(&path);
            let mut tm = seeded.lock().await;
            tm.create_chat("written before the reload");
            ThreadStore::at_legacy_path(&path).persist(&mut tm).unwrap();
        }
        forget_managers_under(&dir);

        let reopened = manager_for(&path);
        assert!(
            reopened
                .lock()
                .await
                .list()
                .iter()
                .any(|t| t.label == "written before the reload"),
            "a manager handed out before its load finished would look empty"
        );
        forget_managers_under(&dir);
    }

    /// Files left behind by a delete that could not finish are collected.
    ///
    /// A removal that fails partway — or a process killed holding one —
    /// strands the moved-aside directory, and nothing would ever look at it
    /// again: the next delete of the same agent picks a fresh number rather
    /// than colliding with it. Without a sweep that is permanent, and the
    /// disk the user was reclaiming stays spent.
    #[test]
    fn files_stranded_by_a_failed_delete_are_swept_by_the_next_one() {
        let dir = temp_dir("stranded");
        let agents_root = dir.join("agents");

        // What a delete that died mid-removal leaves behind.
        let stranded = agents_root.join(".earlier.deleting.7");
        std::fs::create_dir_all(stranded.join("workspace")).unwrap();
        std::fs::write(stranded.join("workspace/big.bin"), "bytes").unwrap();

        let agent = agents_root.join("unrelated");
        std::fs::create_dir_all(&agent).unwrap();
        assert!(remove_store_dir_if_unused(&agent).unwrap());

        assert!(
            !stranded.exists(),
            "an abandoned delete should be finished off by the next one"
        );
        forget_managers_under(&dir);
    }
}
