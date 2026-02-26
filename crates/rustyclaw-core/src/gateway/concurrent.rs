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

use crate::tasks::TaskId;
use futures_util::Sink;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// A message from a spawned model task back to the main connection handler.
#[derive(Debug, Clone)]
pub enum ModelTaskMessage {
    /// A raw WebSocket message to send to the client
    RawMessage(Message),
    
    /// The model task completed successfully.
    /// The main loop should update thread state.
    Done {
        thread_id: TaskId,
        /// Final assistant response text to add to thread history
        response: Option<String>,
    },
    
    /// The model task failed with an error
    Error {
        thread_id: TaskId,
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

/// A sink that sends WebSocket messages through a channel.
/// 
/// This implements `Sink<Message>` so it can be used with `send_frame` and other
/// functions that expect a WebSocket writer.
pub struct ChannelSink {
    tx: ModelTaskTx,
    thread_id: TaskId,
}

impl ChannelSink {
    pub fn new(tx: ModelTaskTx, thread_id: TaskId) -> Self {
        Self { tx, thread_id }
    }
    
    /// Signal that the task completed successfully.
    pub async fn done(&self, response: Option<String>) {
        let _ = self.tx.send(ModelTaskMessage::Done {
            thread_id: self.thread_id,
            response,
        }).await;
    }
    
    /// Signal that the task failed.
    pub async fn error(&self, message: String) {
        let _ = self.tx.send(ModelTaskMessage::Error {
            thread_id: self.thread_id,
            message,
        }).await;
    }
}

impl Sink<Message> for ChannelSink {
    type Error = mpsc::error::SendError<ModelTaskMessage>;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Channel is always ready (bounded but non-blocking poll)
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        // Use try_send to avoid blocking
        self.tx.try_send(ModelTaskMessage::RawMessage(item))
            .map_err(|e| mpsc::error::SendError(match e {
                mpsc::error::TrySendError::Full(m) => m,
                mpsc::error::TrySendError::Closed(m) => m,
            }))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Tracks active model tasks per thread.
#[derive(Debug, Default)]
pub struct ActiveTasks {
    /// Map of thread ID to task handle (for cancellation)
    tasks: std::collections::HashMap<TaskId, tokio::task::JoinHandle<()>>,
}

impl ActiveTasks {
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Register a new task for a thread.
    /// If there's already a task for this thread, it will be aborted.
    pub fn register(&mut self, thread_id: TaskId, handle: tokio::task::JoinHandle<()>) {
        if let Some(old_handle) = self.tasks.insert(thread_id, handle) {
            old_handle.abort();
        }
    }
    
    /// Remove a task when it completes.
    pub fn remove(&mut self, thread_id: &TaskId) {
        self.tasks.remove(thread_id);
    }
    
    /// Cancel a task for a specific thread.
    pub fn cancel(&mut self, thread_id: &TaskId) -> bool {
        if let Some(handle) = self.tasks.remove(thread_id) {
            handle.abort();
            true
        } else {
            false
        }
    }
    
    /// Check if a thread has an active task.
    pub fn is_running(&self, thread_id: &TaskId) -> bool {
        self.tasks.contains_key(thread_id)
    }
    
    /// Get IDs of all threads with active tasks.
    pub fn running_threads(&self) -> Vec<TaskId> {
        self.tasks.keys().copied().collect()
    }
}
