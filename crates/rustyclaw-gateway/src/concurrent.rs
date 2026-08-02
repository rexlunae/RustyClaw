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
        /// Which turn this is from. A turn's completion travels the same
        /// channel as its frames, so the loop can read the client's next
        /// message — and start the *next* turn on this thread — before
        /// draining it. Without this id, retiring "the turn for thread N"
        /// would retire whichever turn is registered now, not the one that
        /// finished.
        turn_id: u64,
        /// Final assistant response text to add to thread history
        response: Option<String>,
    },

    /// The model task failed with an error
    Error {
        thread_id: ThreadId,
        turn_id: u64,
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
    turn_id: u64,
    stream_id: u64,
}

impl ChannelSink {
    pub fn new(tx: ModelTaskTx, thread_id: ThreadId, turn_id: u64, stream_id: u64) -> Self {
        Self {
            tx,
            thread_id,
            turn_id,
            stream_id,
        }
    }

    /// Signal that the task completed successfully.
    pub async fn done(&self, response: Option<String>) {
        let _ = self
            .tx
            .send(ModelTaskMessage::Done {
                thread_id: self.thread_id,
                turn_id: self.turn_id,
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
                turn_id: self.turn_id,
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

/// One running turn: the task itself, and the flag that stops it.
#[derive(Debug)]
struct RunningTurn {
    /// Distinguishes this turn from an earlier one on the same thread whose
    /// completion message has not been drained yet.
    id: u64,
    handle: tokio::task::JoinHandle<()>,
    cancel: crate::ToolCancelFlag,
}

/// Tracks active model tasks per thread.
///
/// Each turn owns its cancel flag rather than sharing one per connection.
/// With turns for different threads able to run at once, a single flag
/// would mean a Stop aimed at one turn stopped the others, and starting a
/// turn would clear a Stop the user had just pressed for another.
#[derive(Debug, Default)]
pub struct ActiveTasks {
    /// Map of thread ID to the turn running for it.
    tasks: std::collections::HashMap<ThreadId, RunningTurn>,
}

impl ActiveTasks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new task for a thread, with the flag that cancels it.
    /// If there's already a task for this thread, it will be aborted.
    pub fn register(
        &mut self,
        thread_id: ThreadId,
        turn_id: u64,
        handle: tokio::task::JoinHandle<()>,
        cancel: crate::ToolCancelFlag,
    ) {
        if let Some(old) = self.tasks.insert(
            thread_id,
            RunningTurn {
                id: turn_id,
                handle,
                cancel,
            },
        ) {
            old.handle.abort();
        }
    }

    /// Retire a turn once its completion message is handled — but only if it
    /// is still the turn registered for that thread. A turn that finished
    /// and was already reaped may have been replaced by the next one, and
    /// removing that replacement would leave a running turn nothing can
    /// stop and nothing counts as busy.
    pub fn remove_if(&mut self, thread_id: &ThreadId, turn_id: u64) {
        if self.tasks.get(thread_id).is_some_and(|t| t.id == turn_id) {
            self.tasks.remove(thread_id);
        }
    }

    /// Cancel a task for a specific thread.
    pub fn cancel(&mut self, thread_id: &ThreadId) -> bool {
        if let Some(turn) = self.tasks.remove(thread_id) {
            turn.handle.abort();
            true
        } else {
            false
        }
    }

    /// Ask this thread's turn to stop at its next cancellation check, which
    /// lets it unwind and report rather than being torn down mid-write.
    /// Returns whether there was a turn to ask.
    pub fn request_cancel(&self, thread_id: &ThreadId) -> bool {
        match self.tasks.get(thread_id) {
            Some(turn) => {
                turn.cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Ask the only running turn to stop, if exactly one is running.
    ///
    /// Stop is aimed at the thread on screen, but a user who switched away
    /// from a working thread still means "stop that" — the client shows the
    /// button for as long as anything is in flight. With one turn running
    /// there is nothing to be ambiguous about; with several, this declines
    /// rather than guessing.
    pub fn request_cancel_sole(&self) -> bool {
        match self.tasks.values().next() {
            Some(turn) if self.tasks.len() == 1 => {
                turn.cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }

    /// Stop every running turn outright.
    ///
    /// Dropping a `JoinHandle` detaches its task rather than ending it, so a
    /// turn left behind on shutdown would keep making model calls and then
    /// block forever writing into a frame channel nobody drains — holding
    /// its share of the connection's state alive with it.
    pub fn abort_all(&mut self) {
        for (_, turn) in self.tasks.drain() {
            turn.handle.abort();
        }
    }

    /// Ask every running turn to stop. Used when the client goes away.
    pub fn request_cancel_all(&self) {
        for turn in self.tasks.values() {
            turn.cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
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
        self.tasks.retain(|_, turn| !turn.handle.is_finished());
    }

    /// Whether this thread's task has returned. Test-only view of the same
    /// state [`Self::reap_finished`] acts on.
    #[cfg(test)]
    pub fn is_finished_for_test(&self, thread_id: &ThreadId) -> bool {
        self.tasks
            .get(thread_id)
            .is_some_and(|turn| turn.handle.is_finished())
    }

    /// Get IDs of all threads with active tasks.
    pub fn running_threads(&self) -> Vec<ThreadId> {
        self.tasks.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn flag() -> crate::ToolCancelFlag {
        Arc::new(AtomicBool::new(false))
    }

    /// Stop applies to the turn it was aimed at. One flag per connection
    /// would cancel every running turn and would be cleared by the next
    /// turn to start, throwing away a Stop the user had just pressed.
    #[tokio::test]
    async fn stopping_one_turn_leaves_the_others_running() {
        let mut tasks = ActiveTasks::new();
        let (a, b) = (ThreadId(1), ThreadId(2));
        let (flag_a, flag_b) = (flag(), flag());
        tasks.register(a, 1, tokio::spawn(std::future::pending()), flag_a.clone());
        tasks.register(b, 2, tokio::spawn(std::future::pending()), flag_b.clone());

        assert!(tasks.request_cancel(&a));
        assert!(
            flag_a.load(Ordering::Relaxed),
            "the target must be asked to stop"
        );
        assert!(
            !flag_b.load(Ordering::Relaxed),
            "another thread's turn must keep running"
        );

        // Starting a third turn cannot disturb the Stop already recorded.
        tasks.register(ThreadId(3), 3, tokio::spawn(async {}), flag());
        assert!(flag_a.load(Ordering::Relaxed));

        assert!(
            !tasks.request_cancel(&ThreadId(9)),
            "a thread with no turn has nothing to stop"
        );

        tasks.request_cancel_all();
        assert!(
            flag_b.load(Ordering::Relaxed),
            "disconnect stops everything"
        );
    }

    /// The completion of a finished turn must not retire the turn that
    /// replaced it. `Done` travels the same channel as the turn's frames,
    /// so the connection loop can read the client's follow-up — and start
    /// the next turn — before draining it. Removing by thread alone would
    /// leave that new turn untracked: Stop could not reach it, and the
    /// busy guard would let a third turn start alongside it.
    #[tokio::test]
    async fn a_finished_turns_completion_does_not_retire_its_successor() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(7);

        // Turn A finishes and is reaped when the follow-up message arrives.
        tasks.register(thread, 1, tokio::spawn(async {}), flag());
        while !tasks.is_finished_for_test(&thread) {
            tokio::task::yield_now().await;
        }
        tasks.reap_finished();

        // Turn B starts on the same thread.
        let b_cancel = flag();
        tasks.register(
            thread,
            2,
            tokio::spawn(std::future::pending()),
            b_cancel.clone(),
        );

        // Only now does A's queued completion get drained.
        tasks.remove_if(&thread, 1);

        assert!(
            tasks.is_running(&thread),
            "the new turn must survive the old turn's completion"
        );
        assert!(tasks.request_cancel(&thread), "Stop must still reach it");
        assert!(b_cancel.load(Ordering::Relaxed));

        tasks.remove_if(&thread, 2);
        assert!(!tasks.is_running(&thread), "its own completion retires it");
    }

    /// Shutdown must end running turns, not detach them. A dropped
    /// `JoinHandle` leaves the task alive with nothing draining its
    /// frames, so it would block forever holding connection state.
    #[tokio::test]
    async fn shutdown_ends_running_turns() {
        let mut tasks = ActiveTasks::new();
        let handle = tokio::spawn(std::future::pending::<()>());
        let watch = tokio::spawn(async {});
        drop(watch);
        tasks.register(ThreadId(1), 1, handle, flag());

        tasks.abort_all();

        assert!(tasks.running_threads().is_empty());
        // Give the runtime a chance to process the abort.
        tokio::task::yield_now().await;
    }

    /// Stop still reaches the turn when the user has navigated to a thread
    /// that is not the one working — as long as there is only one.
    #[tokio::test]
    async fn stop_falls_back_to_the_sole_running_turn() {
        let mut tasks = ActiveTasks::new();
        let only = flag();
        tasks.register(
            ThreadId(1),
            1,
            tokio::spawn(std::future::pending()),
            only.clone(),
        );

        assert!(
            !tasks.request_cancel(&ThreadId(2)),
            "the thread on screen has no turn of its own"
        );
        assert!(tasks.request_cancel_sole());
        assert!(only.load(Ordering::Relaxed));

        // With more than one running, guessing would be wrong.
        let second = flag();
        tasks.register(
            ThreadId(2),
            2,
            tokio::spawn(std::future::pending()),
            second.clone(),
        );
        assert!(!tasks.request_cancel_sole());
        assert!(!second.load(Ordering::Relaxed));
    }

    /// A finished turn must not block the next message. The `Done` message
    /// that normally retires a task shares a channel with the turn's frames,
    /// so the connection loop can read the client's follow-up first — and
    /// would refuse it as "already working" without this reaping step.
    #[tokio::test]
    async fn a_finished_turn_stops_counting_as_running() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(7);
        let handle = tokio::spawn(async {});
        tasks.register(thread, 1, handle, flag());

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
