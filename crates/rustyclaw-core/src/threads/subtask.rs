//! Subtask abstraction — async spawn/join integrated with ThreadManager.
//!
//! This module provides the unified mechanism for running concurrent work:
//! - Subagents spawned by the main agent (joinable, return a result)
//! - Background tasks (long-running, may not return)
//! - One-shot tasks (quick async work that returns a result)
//!
//! All subtasks:
//! - Run as tokio tasks
//! - Create a thread entry in ThreadManager on spawn
//! - Update thread status on completion/failure
//! - Support cancellation via CancellationToken
//! - Support agent-settable descriptions (shown in sidebar)

use super::{SharedThreadManager, ThreadId, ThreadStatus};
use std::future::Future;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

/// Result of a completed subtask.
#[derive(Debug, Clone)]
pub enum SubtaskResult {
    /// Task completed successfully with an optional string result.
    Ok(Option<String>),
    /// Task failed with an error message.
    Err(String),
    /// Task was cancelled.
    Cancelled,
}

/// Handle to a running subtask. Allows joining, cancelling, and updating status.
///
/// The type parameter `T` is the return type of the async function.
/// For subagents that return string results, use `SubtaskHandle<String>`.
pub struct SubtaskHandle<T: Send + 'static> {
    /// The thread ID in the ThreadManager.
    pub thread_id: ThreadId,

    /// Cancellation token — cancel the subtask by calling `.cancel()`.
    cancel_token: CancellationToken,

    /// Oneshot receiver for the result.
    result_rx: Option<oneshot::Receiver<Result<T, String>>>,

    /// Shared thread manager for status updates.
    thread_mgr: SharedThreadManager,

    /// The underlying tokio JoinHandle (for abort).
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl<T: Send + 'static> SubtaskHandle<T> {
    /// Wait for the subtask to complete and return its result.
    ///
    /// This consumes the handle. After joining, the thread status is updated
    /// to Completed or Failed.
    pub async fn join(mut self) -> Result<T, String> {
        let rx = self
            .result_rx
            .take()
            .ok_or_else(|| "SubtaskHandle already joined".to_string())?;

        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // Sender dropped — task panicked or was aborted
                Err("Subtask channel closed unexpectedly".to_string())
            }
        }
    }

    /// Cancel the subtask.
    ///
    /// This signals the cancellation token. The subtask's async function
    /// should check `token.is_cancelled()` or use `token.cancelled().await`.
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    /// Check if the subtask has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Update the description shown in the sidebar for this subtask.
    pub async fn set_description(&self, description: impl Into<String>) {
        let mut mgr = self.thread_mgr.write().await;
        mgr.set_description(self.thread_id, description);
    }

    /// Update the status of the subtask's thread.
    pub async fn set_status(&self, status: ThreadStatus) {
        let mut mgr = self.thread_mgr.write().await;
        mgr.set_status(self.thread_id, status);
    }

    /// Get the cancellation token (for passing to subtask internals).
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }
}

impl<T: Send + 'static> Drop for SubtaskHandle<T> {
    fn drop(&mut self) {
        // If the handle is dropped without joining, cancel the subtask
        if self.result_rx.is_some() {
            self.cancel_token.cancel();
            if let Some(handle) = self.join_handle.take() {
                handle.abort();
            }
        }
    }
}

/// Options for spawning a subtask.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    /// Label shown in the sidebar.
    pub label: String,
    /// Initial description of what the subtask is doing.
    pub description: Option<String>,
    /// Parent thread that spawned this subtask (if any).
    pub parent_id: Option<ThreadId>,
}

impl SpawnOptions {
    /// Create spawn options with just a label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            parent_id: None,
        }
    }

    /// Set the initial description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the parent thread.
    pub fn with_parent(mut self, parent_id: ThreadId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }
}

/// Spawn a subagent as an async subtask.
///
/// The `task_fn` receives a `CancellationToken` and a `SharedThreadManager`.
/// It should check the token periodically and return a result.
///
/// Returns a `SubtaskHandle` that can be joined to get the result.
///
/// # Example
/// ```ignore
/// let handle = spawn_subagent(
///     thread_mgr.clone(),
///     SpawnOptions::new("Research Task")
///         .with_description("Searching for information"),
///     |token, mgr| async move {
///         // Do async work, checking token.is_cancelled()
///         Ok("result data".to_string())
///     },
/// ).await;
///
/// // Later, join to get the result
/// let result = handle.join().await;
/// ```
pub async fn spawn_subagent<F, Fut, T>(
    thread_mgr: SharedThreadManager,
    options: SpawnOptions,
    task_fn: F,
) -> SubtaskHandle<T>
where
    F: FnOnce(CancellationToken, SharedThreadManager) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let cancel_token = CancellationToken::new();
    let (result_tx, result_rx) = oneshot::channel();

    // Create thread entry in ThreadManager
    let thread_id = {
        let mut mgr = thread_mgr.write().await;
        let id = mgr.create_subagent(
            &options.label,
            "subtask",
            options.description.as_deref().unwrap_or(&options.label),
            options.parent_id,
        );
        if let Some(desc) = &options.description {
            mgr.set_description(id, desc);
        }
        id
    };

    debug!(thread_id = %thread_id, label = %options.label, "Spawning subagent subtask");

    // Spawn the tokio task
    let token = cancel_token.clone();
    let mgr = thread_mgr.clone();
    let tid = thread_id;

    let join_handle = tokio::spawn(async move {
        let result = tokio::select! {
            _ = token.cancelled() => {
                Err("Cancelled".to_string())
            }
            res = task_fn(token.clone(), mgr.clone()) => {
                res
            }
        };

        // Update thread status based on result
        {
            let mut mgr_guard = mgr.write().await;
            match &result {
                Ok(_) => {
                    mgr_guard.complete(tid, Some("Completed".to_string()), None);
                    debug!(thread_id = %tid, "Subagent subtask completed");
                }
                Err(e) if e == "Cancelled" => {
                    mgr_guard.set_status(tid, ThreadStatus::Cancelled);
                    debug!(thread_id = %tid, "Subagent subtask cancelled");
                }
                Err(e) => {
                    mgr_guard.fail(tid, e);
                    warn!(thread_id = %tid, error = %e, "Subagent subtask failed");
                }
            }
        }

        // Send result through oneshot channel
        let _ = result_tx.send(result);
    });

    SubtaskHandle {
        thread_id,
        cancel_token,
        result_rx: Some(result_rx),
        thread_mgr,
        join_handle: Some(join_handle),
    }
}

/// Spawn a one-shot background task.
///
/// Similar to `spawn_subagent` but creates a Task thread instead of SubAgent.
/// Best for quick async work that returns a result.
pub async fn spawn_task<F, Fut, T>(
    thread_mgr: SharedThreadManager,
    options: SpawnOptions,
    task_fn: F,
) -> SubtaskHandle<T>
where
    F: FnOnce(CancellationToken, SharedThreadManager) -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let cancel_token = CancellationToken::new();
    let (result_tx, result_rx) = oneshot::channel();

    // Create thread entry in ThreadManager
    let thread_id = {
        let mut mgr = thread_mgr.write().await;
        let id = mgr.create_task(
            &options.label,
            options.description.as_deref().unwrap_or(&options.label),
            options.parent_id,
        );
        if let Some(desc) = &options.description {
            mgr.set_description(id, desc);
        }
        id
    };

    debug!(thread_id = %thread_id, label = %options.label, "Spawning one-shot task");

    let token = cancel_token.clone();
    let mgr = thread_mgr.clone();
    let tid = thread_id;

    let join_handle = tokio::spawn(async move {
        let result = tokio::select! {
            _ = token.cancelled() => {
                Err("Cancelled".to_string())
            }
            res = task_fn(token.clone(), mgr.clone()) => {
                res
            }
        };

        // Update thread status
        {
            let mut mgr_guard = mgr.write().await;
            match &result {
                Ok(_) => {
                    mgr_guard.complete(tid, Some("Completed".to_string()), None);
                    debug!(thread_id = %tid, "Task completed");
                }
                Err(e) if e == "Cancelled" => {
                    mgr_guard.set_status(tid, ThreadStatus::Cancelled);
                    debug!(thread_id = %tid, "Task cancelled");
                }
                Err(e) => {
                    mgr_guard.fail(tid, e);
                    warn!(thread_id = %tid, error = %e, "Task failed");
                }
            }
        }

        let _ = result_tx.send(result);
    });

    SubtaskHandle {
        thread_id,
        cancel_token,
        result_rx: Some(result_rx),
        thread_mgr,
        join_handle: Some(join_handle),
    }
}

/// Spawn a long-running background thread.
///
/// Unlike subagents and tasks, background threads don't have a natural
/// return value. They run until cancelled.
pub async fn spawn_background<F, Fut>(
    thread_mgr: SharedThreadManager,
    options: SpawnOptions,
    task_fn: F,
) -> SubtaskHandle<()>
where
    F: FnOnce(CancellationToken, SharedThreadManager) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let cancel_token = CancellationToken::new();
    let (result_tx, result_rx) = oneshot::channel();

    let thread_id = {
        let mut mgr = thread_mgr.write().await;
        let id = mgr.create_background(
            &options.label,
            options.description.as_deref().unwrap_or(&options.label),
            options.parent_id,
        );
        if let Some(desc) = &options.description {
            mgr.set_description(id, desc);
        }
        id
    };

    debug!(thread_id = %thread_id, label = %options.label, "Spawning background thread");

    let token = cancel_token.clone();
    let mgr = thread_mgr.clone();
    let tid = thread_id;

    let join_handle = tokio::spawn(async move {
        let result = tokio::select! {
            _ = token.cancelled() => {
                Err("Cancelled".to_string())
            }
            res = task_fn(token.clone(), mgr.clone()) => {
                res
            }
        };

        {
            let mut mgr_guard = mgr.write().await;
            match &result {
                Ok(()) => {
                    mgr_guard.complete(tid, Some("Finished".to_string()), None);
                    debug!(thread_id = %tid, "Background thread finished");
                }
                Err(e) if e == "Cancelled" => {
                    mgr_guard.set_status(tid, ThreadStatus::Cancelled);
                    debug!(thread_id = %tid, "Background thread cancelled");
                }
                Err(e) => {
                    mgr_guard.fail(tid, e);
                    warn!(thread_id = %tid, error = %e, "Background thread failed");
                }
            }
        }

        let _ = result_tx.send(result);
    });

    SubtaskHandle {
        thread_id,
        cancel_token,
        result_rx: Some(result_rx),
        thread_mgr,
        join_handle: Some(join_handle),
    }
}

/// Registry of active subtask handles for a session.
///
/// This is used by the gateway to track all running subtasks and
/// allow the agent to list, join, or cancel them.
pub struct SubtaskRegistry {
    handles: std::collections::HashMap<ThreadId, RegistryEntry>,
}

/// An entry in the subtask registry.
struct RegistryEntry {
    cancel_token: CancellationToken,
    join_handle: tokio::task::JoinHandle<()>,
    label: String,
}

impl SubtaskRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            handles: std::collections::HashMap::new(),
        }
    }

    /// Register a subtask. Call this after spawning.
    pub fn register<T: Send + 'static>(
        &mut self,
        handle: &SubtaskHandle<T>,
        label: impl Into<String>,
    ) {
        // We can't move the join_handle out of SubtaskHandle, so we just
        // store the cancel token for cancellation purposes
        self.handles.insert(
            handle.thread_id,
            RegistryEntry {
                cancel_token: handle.cancel_token.clone(),
                join_handle: tokio::spawn(async {}), // placeholder
                label: label.into(),
            },
        );
    }

    /// Cancel a subtask by thread ID.
    pub fn cancel(&mut self, thread_id: &ThreadId) -> bool {
        if let Some(entry) = self.handles.remove(thread_id) {
            entry.cancel_token.cancel();
            entry.join_handle.abort();
            true
        } else {
            false
        }
    }

    /// Cancel all subtasks.
    pub fn cancel_all(&mut self) {
        for (_, entry) in self.handles.drain() {
            entry.cancel_token.cancel();
            entry.join_handle.abort();
        }
    }

    /// List active subtask thread IDs with labels.
    pub fn list(&self) -> Vec<(ThreadId, String)> {
        self.handles
            .iter()
            .map(|(id, entry)| (*id, entry.label.clone()))
            .collect()
    }

    /// Remove a completed subtask from the registry.
    pub fn remove(&mut self, thread_id: &ThreadId) {
        self.handles.remove(thread_id);
    }

    /// Get the number of active subtasks.
    pub fn count(&self) -> usize {
        self.handles.len()
    }
}

impl Default for SubtaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn make_thread_mgr() -> SharedThreadManager {
        Arc::new(RwLock::new(super::super::ThreadManager::new()))
    }

    #[tokio::test]
    async fn test_spawn_and_join_subagent() {
        let mgr = make_thread_mgr();

        let handle = spawn_subagent(
            mgr.clone(),
            SpawnOptions::new("Test Subagent").with_description("Doing work"),
            |_token, _mgr| async move { Ok("result!".to_string()) },
        )
        .await;

        let thread_id = handle.thread_id;
        let result = handle.join().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "result!");

        // Thread should be marked as completed
        let mgr_guard = mgr.read().await;
        let thread = mgr_guard.get(thread_id).unwrap();
        assert!(thread.status.is_terminal());
    }

    #[tokio::test]
    async fn test_spawn_and_cancel() {
        let mgr = make_thread_mgr();

        let handle: SubtaskHandle<String> = spawn_subagent(
            mgr.clone(),
            SpawnOptions::new("Cancellable"),
            |token, _mgr| async move {
                // Wait until cancelled
                token.cancelled().await;
                Err("Cancelled".to_string())
            },
        )
        .await;

        let thread_id = handle.thread_id;

        // Cancel it
        handle.cancel();

        let result = handle.join().await;
        assert!(result.is_err());

        // Give tokio a moment to process the status update
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mgr_guard = mgr.read().await;
        let thread = mgr_guard.get(thread_id).unwrap();
        assert!(thread.status.is_terminal());
    }

    #[tokio::test]
    async fn test_spawn_task() {
        let mgr = make_thread_mgr();

        let handle = spawn_task(
            mgr.clone(),
            SpawnOptions::new("Quick Task"),
            |_token, _mgr| async move { Ok(42i64) },
        )
        .await;

        let result = handle.join().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_spawn_with_description_update() {
        let mgr = make_thread_mgr();

        let handle = spawn_subagent(
            mgr.clone(),
            SpawnOptions::new("Descriptive Task"),
            |_token, _mgr| async move {
                // Simulate work
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Ok("done".to_string())
            },
        )
        .await;

        // Update description while running
        handle.set_description("Phase 2: Processing").await;

        let thread_id = handle.thread_id;
        {
            let mgr_guard = mgr.read().await;
            let thread = mgr_guard.get(thread_id).unwrap();
            assert_eq!(
                thread.description.as_deref(),
                Some("Phase 2: Processing")
            );
        }

        let _ = handle.join().await;
    }

    #[tokio::test]
    async fn test_subtask_failure() {
        let mgr = make_thread_mgr();

        let handle = spawn_subagent(
            mgr.clone(),
            SpawnOptions::new("Failing Task"),
            |_token, _mgr| async move {
                Err::<String, _>("something went wrong".to_string())
            },
        )
        .await;

        let thread_id = handle.thread_id;
        let result = handle.join().await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "something went wrong");

        // Thread should be marked as failed
        let mgr_guard = mgr.read().await;
        let thread = mgr_guard.get(thread_id).unwrap();
        assert!(matches!(thread.status, ThreadStatus::Failed { .. }));
    }

    #[test]
    fn test_subtask_registry() {
        let registry = SubtaskRegistry::new();
        assert_eq!(registry.count(), 0);
        assert!(registry.list().is_empty());
    }
}
