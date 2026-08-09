//! Who is running the current tool call.
//!
//! Tool functions take `(args, workspace_dir)` and nothing else, so they have
//! no way to ask which conversation, sub-agent, or scheduled job invoked
//! them. That is fine for stateless tools, but anything that owns a resource
//! across calls — a backgrounded process, say — needs to know, or every
//! caller can reach every other caller's resources.
//!
//! Rather than thread an identity parameter through every tool signature,
//! the identity rides on the async task. Callers wrap their tool execution in
//! [`with_caller`]; tools read [`current`].
//!
//! # Scope and limits
//!
//! The identity rides on a [`tokio::task_local`], so it is visible to the
//! wrapped future and anything it awaits inline, and *not* to a freshly
//! `tokio::spawn`ed task — which is the point: a sub-agent gets its own
//! identity rather than inheriting its parent's.
//!
//! A task-local is also invisible inside `spawn_blocking`, where every
//! synchronous tool runs. That would leave most of the registry unidentified,
//! so the blocking dispatch carries the identity across on a thread-local via
//! [`with_caller_blocking`], and [`current`] reads whichever is set. The
//! thread-local is restored on the way out, because blocking-pool threads are
//! reused and a leaked identity would hand the next tool call someone else's
//! ownership.
//!
//! An absent identity is not an error. Direct CLI use and tests run without
//! one, and resources they create are simply unowned.

use std::cell::RefCell;
use std::future::Future;

tokio::task_local! {
    /// Identity of the caller whose tool call is currently running.
    static CURRENT_CALLER: String;
}

thread_local! {
    /// Identity carried onto a blocking-pool thread by
    /// [`with_caller_blocking`], where the task-local cannot reach.
    static BLOCKING_CALLER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Run `fut` with `id` as the ambient caller identity.
///
/// `id` must be stable for the lifetime of whatever the caller owns and
/// distinct between callers — a thread id, sub-agent session key, or job
/// name. Two callers sharing an id can reach each other's resources.
pub async fn with_caller<F>(id: impl Into<String>, fut: F) -> F::Output
where
    F: Future,
{
    CURRENT_CALLER.scope(id.into(), fut).await
}

/// Run `fut` with `id` as the caller identity when one is available,
/// otherwise unchanged.
///
/// Convenience for the common call site that holds an `Option` identity and
/// would otherwise duplicate itself across both branches.
pub async fn with_caller_opt<F>(id: Option<String>, fut: F) -> F::Output
where
    F: Future,
{
    match id {
        Some(id) => with_caller(id, fut).await,
        None => fut.await,
    }
}

/// Run `f` on this thread with `id` as the ambient caller identity.
///
/// The blocking-pool counterpart to [`with_caller`], for dispatching a
/// synchronous tool: pass what [`current`] returned on the async side and the
/// tool sees the same identity it would have seen inline.
///
/// The previous value is restored even if `f` panics, so a thread returning
/// to the pool never carries an identity into the next call.
pub fn with_caller_blocking<T>(id: Option<String>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            BLOCKING_CALLER.with(|slot| *slot.borrow_mut() = previous);
        }
    }

    // `id` of `None` clears rather than leaves: an unidentified call must not
    // inherit whatever the previous call on this thread was.
    let previous = BLOCKING_CALLER.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), id));
    let _restore = Restore(previous);
    f()
}

/// The current caller's identity, or `None` when running unidentified.
///
/// `None` means "no identity was established" — direct CLI use, a test, or a
/// call site that never set one. Treat it as untrusted, not privileged.
pub fn current() -> Option<String> {
    CURRENT_CALLER
        .try_with(|id| id.clone())
        .ok()
        .or_else(|| BLOCKING_CALLER.with(|slot| slot.borrow().clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn absent_outside_a_scope() {
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn visible_inside_a_scope() {
        with_caller("thread:7", async {
            assert_eq!(current().as_deref(), Some("thread:7"));
        })
        .await;
    }

    #[tokio::test]
    async fn does_not_leak_past_the_scope() {
        with_caller("thread:7", async {}).await;
        assert_eq!(current(), None, "identity must not outlive its scope");
    }

    #[tokio::test]
    async fn nested_scopes_shadow() {
        with_caller("outer", async {
            assert_eq!(current().as_deref(), Some("outer"));
            with_caller("inner", async {
                assert_eq!(current().as_deref(), Some("inner"));
            })
            .await;
            // The outer identity is restored, so a sub-agent's scope cannot
            // leave its parent running as the sub-agent.
            assert_eq!(current().as_deref(), Some("outer"));
        })
        .await;
    }

    #[tokio::test]
    async fn concurrent_tasks_keep_separate_identities() {
        // The whole point of a task-local over a global: two turns running
        // at once must not see each other's identity.
        let a = tokio::spawn(with_caller("thread:1", async {
            tokio::task::yield_now().await;
            current()
        }));
        let b = tokio::spawn(with_caller("thread:2", async {
            tokio::task::yield_now().await;
            current()
        }));
        assert_eq!(a.await.unwrap().as_deref(), Some("thread:1"));
        assert_eq!(b.await.unwrap().as_deref(), Some("thread:2"));
    }

    #[tokio::test]
    async fn identity_survives_the_trip_to_the_blocking_pool() {
        // The path every synchronous tool takes. Without the hand-off the
        // identity vanishes and ownership checks silently pass everyone.
        let seen = with_caller("thread:9", async {
            let caller = current();
            tokio::task::spawn_blocking(move || with_caller_blocking(caller, current))
                .await
                .unwrap()
        })
        .await;
        assert_eq!(seen.as_deref(), Some("thread:9"));
    }

    #[test]
    fn a_blocking_identity_does_not_outlive_its_call() {
        // Blocking-pool threads are reused; a leaked identity would hand the
        // next tool call someone else's ownership.
        with_caller_blocking(Some("thread:1".into()), || {
            assert_eq!(current().as_deref(), Some("thread:1"));
        });
        assert_eq!(current(), None);
    }

    #[test]
    fn an_unidentified_blocking_call_does_not_inherit() {
        with_caller_blocking(Some("thread:1".into()), || {
            with_caller_blocking(None, || {
                assert_eq!(current(), None, "must not inherit the outer identity");
            });
            assert_eq!(current().as_deref(), Some("thread:1"));
        });
    }

    #[test]
    fn a_panicking_blocking_call_still_restores() {
        let panicked = std::panic::catch_unwind(|| {
            with_caller_blocking(Some("thread:1".into()), || panic!("boom"));
        });
        assert!(panicked.is_err());
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn opt_none_runs_unidentified() {
        with_caller_opt(None, async {
            assert_eq!(current(), None);
        })
        .await;
    }
}
