//! One-call override of the credential-exfiltration guards (#418).
//!
//! The guards in [`super::helpers`] — the protected-path check on the
//! credentials directory and the exfiltration-pattern screen on commands —
//! are heuristics, and heuristics have false positives: `cat
//! ~/.rustyclaw/config.toml` while debugging is blocked the same as an
//! actual key grab. They used to be absolute; now the gateway may put a
//! block in front of the user, with the guard's reason and the exact call,
//! and re-run the tool once inside an override scope if the user approves.
//!
//! The scope rides on the async task, exactly like [`crate::tool_caller`]:
//! visible to the wrapped future and whatever it awaits, invisible to
//! spawned tasks, and carried onto the blocking pool by hand where the
//! synchronous tools run. It cannot leak past the one retried call — the
//! grant dies with the scope, so nothing the tool started keeps the
//! permission.
//!
//! Only the *user* path grants this. Headless callers (cron, triggers,
//! messengers, subagents) have no one to ask and never enter a scope, so
//! for them the guards stay absolute.

use std::cell::Cell;
use std::future::Future;

tokio::task_local! {
    /// Whether the current tool call runs with a user-granted override.
    static GRANTED: bool;
}

thread_local! {
    /// The grant carried onto a blocking-pool thread, where the task-local
    /// cannot reach.
    static BLOCKING_GRANTED: Cell<bool> = const { Cell::new(false) };
}

/// Run `fut` with the exfiltration guards overridden.
///
/// Callers must have shown the user what was blocked and why, and received
/// an explicit approval, before entering this scope.
pub async fn with_granted<F>(fut: F) -> F::Output
where
    F: Future,
{
    GRANTED.scope(true, fut).await
}

/// Run `f` on this thread with the grant carried across from the async
/// side — the blocking-pool counterpart of [`with_granted`], mirroring
/// [`crate::tool_caller::with_caller_blocking`].
///
/// The previous value is restored even if `f` panics: pool threads are
/// reused, and a leaked grant would silently disarm the guards for whoever
/// runs next.
pub fn with_granted_blocking<T>(granted: bool, f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0;
            BLOCKING_GRANTED.with(|slot| slot.set(previous));
        }
    }

    let previous = BLOCKING_GRANTED.with(|slot| slot.replace(granted));
    let _restore = Restore(previous);
    f()
}

/// Whether the current call holds a user-granted override.
pub fn is_granted() -> bool {
    GRANTED.try_with(|g| *g).unwrap_or(false) || BLOCKING_GRANTED.with(|slot| slot.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn absent_by_default() {
        assert!(!is_granted());
    }

    #[tokio::test]
    async fn granted_inside_a_scope_and_only_there() {
        with_granted(async {
            assert!(is_granted());
        })
        .await;
        assert!(!is_granted(), "the grant must die with its scope");
    }

    #[tokio::test]
    async fn the_grant_survives_the_trip_to_the_blocking_pool() {
        let seen = with_granted(async {
            let granted = is_granted();
            tokio::task::spawn_blocking(move || with_granted_blocking(granted, is_granted))
                .await
                .unwrap()
        })
        .await;
        assert!(seen);
    }

    #[test]
    fn a_blocking_grant_does_not_outlive_its_call() {
        with_granted_blocking(true, || assert!(is_granted()));
        assert!(
            !is_granted(),
            "pool threads are reused; leaks disarm guards"
        );
    }

    #[test]
    fn a_panicking_blocking_call_still_restores() {
        let panicked = std::panic::catch_unwind(|| {
            with_granted_blocking(true, || panic!("boom"));
        });
        assert!(panicked.is_err());
        assert!(!is_granted());
    }

    #[tokio::test]
    async fn a_spawned_task_does_not_inherit_the_grant() {
        // The override is for the one retried call — a background task the
        // tool spawns must not keep the permission after the call ends.
        let inherited =
            with_granted(async { tokio::spawn(async { is_granted() }).await.unwrap() }).await;
        assert!(!inherited);
    }
}
