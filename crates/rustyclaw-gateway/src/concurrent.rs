//! Concurrent model execution support.
//!
//! This module provides infrastructure for running multiple model requests
//! concurrently across different threads, allowing the TUI to remain responsive
//! while models are generating responses.
//!
//! Architecture:
//! - Each model request runs in its own spawned task
//! - Tasks send frames back via an mpsc channel
//! - The main loop selects between client messages and model responses
//! - Thread switching is allowed while models are running
//!
//! Both halves are wired: the connection handler spawns each chat turn with a
//! [`ChannelSink`], tracks it in [`ActiveTasks`], and keeps serving client
//! frames from its select loop while the turn runs.
#![allow(dead_code)]

use anyhow::Result;
use async_trait::async_trait;
use rustyclaw_core::gateway::protocol::frames::{ServerFrame, serialize_frame};
use rustyclaw_core::gateway::transport::TransportWriter;
use rustyclaw_core::threads::ThreadId;
use tokio::sync::mpsc;

/// A message from a spawned model task back to the main connection handler.
#[derive(Debug, Clone)]
pub enum ModelTaskMessage {
    /// A serialized frame to send to the client, on the stream the request
    /// arrived on — clients correlate a turn's frames by stream id, so the
    /// task's own id has to survive the trip through this channel.
    Frame { stream_id: u64, data: Vec<u8> },

    /// The model task completed successfully.
    /// The main loop should update thread state.
    Done {
        thread_id: ThreadId,
        /// Final assistant response text to add to thread history
        response: Option<String>,
    },

    /// The model task failed with an error
    Error {
        thread_id: ThreadId,
        message: String,
    },
}

/// Sender for model task messages.
pub type ModelTaskTx = mpsc::Sender<ModelTaskMessage>;

/// Receiver for model task messages.
pub type ModelTaskRx = mpsc::Receiver<ModelTaskMessage>;

/// Create a new model task channel.
pub fn channel() -> (ModelTaskTx, ModelTaskRx) {
    mpsc::channel(256)
}

/// A transport writer that sends frames through a channel.
///
/// This implements `TransportWriter` so it can be used with `send_frame`
/// and other functions that expect a writer, routing the frames back to
/// the main connection handler for dispatch.
pub struct ChannelSink {
    tx: ModelTaskTx,
    thread_id: ThreadId,
    stream_id: u64,
}

impl ChannelSink {
    pub fn new(tx: ModelTaskTx, thread_id: ThreadId, stream_id: u64) -> Self {
        Self {
            tx,
            thread_id,
            stream_id,
        }
    }

    /// Signal that the task completed successfully.
    pub async fn done(&self, response: Option<String>) {
        let _ = self
            .tx
            .send(ModelTaskMessage::Done {
                thread_id: self.thread_id,
                response,
            })
            .await;
    }

    /// Signal that the task failed.
    pub async fn error(&self, message: String) {
        let _ = self
            .tx
            .send(ModelTaskMessage::Error {
                thread_id: self.thread_id,
                message,
            })
            .await;
    }
}

#[async_trait]
impl TransportWriter for ChannelSink {
    async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
        self.tx
            .send(ModelTaskMessage::Frame {
                stream_id: self.stream_id,
                data,
            })
            .await
            .map_err(|_| anyhow::anyhow!("channel closed"))
    }

    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Tracks active model tasks per thread.
#[derive(Debug, Default)]
pub struct ActiveTasks {
    /// Map of thread ID to task handle (for cancellation)
    tasks: std::collections::HashMap<ThreadId, tokio::task::JoinHandle<()>>,
}

impl ActiveTasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new task for a thread.
    /// If there's already a task for this thread, it will be aborted.
    pub fn register(&mut self, thread_id: ThreadId, handle: tokio::task::JoinHandle<()>) {
        if let Some(old_handle) = self.tasks.insert(thread_id, handle) {
            old_handle.abort();
        }
    }

    /// Remove a task when it completes.
    pub fn remove(&mut self, thread_id: &ThreadId) {
        self.tasks.remove(thread_id);
    }

    /// Cancel a task for a specific thread.
    pub fn cancel(&mut self, thread_id: &ThreadId) -> bool {
        if let Some(handle) = self.tasks.remove(thread_id) {
            handle.abort();
            true
        } else {
            false
        }
    }

    /// Check if a thread has an active task.
    pub fn is_running(&self, thread_id: &ThreadId) -> bool {
        self.tasks.contains_key(thread_id)
    }

    /// Forget tasks that have already returned.
    ///
    /// A task is normally removed when its `Done` message is handled, but
    /// that message shares a channel with the turn's frames and the
    /// connection loop may read the client's *next* request first. Without
    /// this, the follow-up message would be refused as "already working" by
    /// a turn that has in fact finished.
    pub fn reap_finished(&mut self) {
        self.tasks.retain(|_, handle| !handle.is_finished());
    }

    /// Whether this thread's task has returned. Test-only view of the same
    /// state [`Self::reap_finished`] acts on.
    #[cfg(test)]
    pub fn is_finished_for_test(&self, thread_id: &ThreadId) -> bool {
        self.tasks.get(thread_id).is_some_and(|h| h.is_finished())
    }

    /// Get IDs of all threads with active tasks.
    pub fn running_threads(&self) -> Vec<ThreadId> {
        self.tasks.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A finished turn must not block the next message. The `Done` message
    /// that normally retires a task shares a channel with the turn's frames,
    /// so the connection loop can read the client's follow-up first — and
    /// would refuse it as "already working" without this reaping step.
    #[tokio::test]
    async fn a_finished_turn_stops_counting_as_running() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(7);
        let handle = tokio::spawn(async {});
        tasks.register(thread, handle);

        // Let the task run to completion without touching the channel the
        // connection loop would normally drain.
        tokio::task::yield_now().await;
        while !tasks.is_finished_for_test(&thread) {
            tokio::task::yield_now().await;
        }

        assert!(tasks.is_running(&thread), "not reaped yet");
        tasks.reap_finished();
        assert!(
            !tasks.is_running(&thread),
            "a finished turn must not hold the thread against the next message"
        );
    }
}
