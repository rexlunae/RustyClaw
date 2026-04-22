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

use crate::gateway::protocol::frames::{ServerFrame, serialize_frame};
use crate::gateway::transport::TransportWriter;
use crate::threads::ThreadId;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

/// A message from a spawned model task back to the main connection handler.
#[derive(Debug, Clone)]
pub enum ModelTaskMessage {
    /// A serialized frame to send to the client.
    Frame(Vec<u8>),

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
}

impl ChannelSink {
    pub fn new(tx: ModelTaskTx, thread_id: ThreadId) -> Self {
        Self { tx, thread_id }
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
    async fn send(&mut self, frame: &ServerFrame) -> Result<()> {
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
        self.tx
            .send(ModelTaskMessage::Frame(data))
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

    /// Get IDs of all threads with active tasks.
    pub fn running_threads(&self) -> Vec<ThreadId> {
        self.tasks.keys().copied().collect()
    }
}
