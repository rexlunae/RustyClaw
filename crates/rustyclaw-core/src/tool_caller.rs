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
//! This is a [`tokio::task_local`], so it is visible to the wrapped future
//! and anything it awaits inline, and it is *not* visible inside
//! `spawn_blocking` or a freshly `tokio::spawn`ed task. Tools that need the
//! identity must therefore be async-native (as `execute_command` and
//! `process` are) rather than dispatched to the blocking pool. A tool that
//! reads [`current`] from the blocking pool sees `None`, which callers must
//! treat as "unidentified", never as "trusted".
//!
//! An absent identity is not an error. Direct CLI use and tests run without
//! one, and resources they create are simply unowned.

use std::future::Future;

tokio::task_local! {
    /// Identity of the caller whose tool call is currently running.
    static CURRENT_CALLER: String;
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

/// The current caller's identity, or `None` when running unidentified.
///
/// `None` means "no identity was established" — from a blocking-pool tool,
/// direct CLI use, or a test. Treat it as untrusted, not privileged.
pub fn current() -> Option<String> {
    CURRENT_CALLER.try_with(|id| id.clone()).ok()
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
    async fn opt_none_runs_unidentified() {
        with_caller_opt(None, async {
            assert_eq!(current(), None);
        })
        .await;
    }
}
