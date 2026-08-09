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
use rustyclaw_core::gateway::protocol::frames::{ServerFrame, ServerPayload, serialize_frame};
use rustyclaw_core::gateway::transport::TransportWriter;
use rustyclaw_core::ignore::Ignore;
use rustyclaw_core::threads::ThreadId;
use tokio::sync::mpsc;

/// A message from a spawned model task back to the main connection handler.
#[derive(Debug, Clone)]
pub enum ModelTaskMessage {
    /// A serialized frame to send to the client, on the stream the request
    /// arrived on — clients correlate a turn's frames by stream id, so the
    /// task's own id has to survive the trip through this channel. The turn
    /// id travels with it because the loop may drain a turn's frames after
    /// the next turn has already started, and "whose frame is this" then
    /// stops being answerable from context.
    Frame {
        stream_id: u64,
        turn_id: u64,
        data: Vec<u8>,
    },

    /// The model task completed successfully.
    /// The main loop should update thread state.
    Done {
        thread_id: ThreadId,
        /// The stream the turn's request arrived on. The backstop close-out
        /// must go out on the same stream as every other frame of the turn —
        /// clients correlate a turn's frames by stream id, and a close-out
        /// on the control stream leaves their per-stream bookkeeping for
        /// the turn unreleased.
        stream_id: u64,
        /// Whether the turn already sent its own `ResponseDone`. Every turn
        /// must end with exactly one close-out — clients retire their
        /// in-flight tracking on it — and the paths that can end a turn are
        /// many. The sink watches, and the loop makes up the difference, so
        /// a future early return cannot leak a stuck spinner by forgetting.
        closed_out: bool,
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
        /// See [`ModelTaskMessage::Done::stream_id`].
        stream_id: u64,
        turn_id: u64,
        message: String,
        /// See [`ModelTaskMessage::Done::closed_out`].
        closed_out: bool,
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
    /// The turn's key in the task registry. This is `ThreadId(0)` when there
    /// was no thread to elect — a local bookkeeping handle, not a thread.
    thread_id: ThreadId,
    /// The thread to name on the wire, which is *not* the same thing.
    /// Resolved once, here, so no frame can leak the sentinel by omission.
    announced_thread: Option<u64>,
    /// Whether a `ResponseDone` has passed through this sink.
    sent_response_done: bool,
    turn_id: u64,
    stream_id: u64,
}

impl ChannelSink {
    pub fn new(tx: ModelTaskTx, thread_id: ThreadId, turn_id: u64, stream_id: u64) -> Self {
        Self {
            tx,
            thread_id,
            // Zero is the wire sentinel for "no thread is focused", and a
            // client that adopts it as this turn's thread will match it
            // against nothing — dropping the whole reply on the floor.
            // A turn with no thread announces no thread.
            announced_thread: Some(thread_id.0).filter(|id| *id != 0),
            sent_response_done: false,
            turn_id,
            stream_id,
        }
    }

    /// The thread this sink names on turn-scoped frames.
    pub fn announced_thread(&self) -> Option<u64> {
        self.announced_thread
    }

    /// Signal that the task completed successfully.
    pub async fn done(&self, response: Option<String>) {
        self.tx
            .send(ModelTaskMessage::Done {
                thread_id: self.thread_id,
                stream_id: self.stream_id,
                closed_out: self.sent_response_done,
                turn_id: self.turn_id,
                response,
            })
            .await
            .ignore();
    }

    /// Signal that the task failed.
    pub async fn error(&self, message: String) {
        self.tx
            .send(ModelTaskMessage::Error {
                thread_id: self.thread_id,
                stream_id: self.stream_id,
                turn_id: self.turn_id,
                message,
                closed_out: self.sent_response_done,
            })
            .await
            .ignore();
    }
}

#[async_trait]
impl TransportWriter for ChannelSink {
    async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
        // Stamp the turn's thread onto the frames that open and close it.
        // The layers that emit these — the provider streaming loop, the
        // dispatch tail — have no thread context, and this writer is the one
        // object that does: it exists per turn and was handed the thread the
        // connection loop settled on. Doing it here means no caller has to
        // remember, and none can name a different thread by mistake.
        let stamped;
        let frame = match &frame.payload {
            ServerPayload::StreamStart { .. } => {
                stamped = ServerFrame {
                    frame_type: frame.frame_type,
                    payload: ServerPayload::StreamStart {
                        thread_id: self.announced_thread,
                    },
                };
                &stamped
            }
            ServerPayload::ResponseDone { ok, .. } => {
                self.sent_response_done = true;
                stamped = ServerFrame {
                    frame_type: frame.frame_type,
                    payload: ServerPayload::ResponseDone {
                        ok: *ok,
                        thread_id: self.announced_thread,
                    },
                };
                &stamped
            }
            _ => frame,
        };
        let data = serialize_frame(frame).map_err(|e| anyhow::anyhow!(e))?;
        self.tx
            .send(ModelTaskMessage::Frame {
                stream_id: self.stream_id,
                turn_id: self.turn_id,
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
    /// The stream the turn's frames travel on, kept so a displaced turn's
    /// close-out can go out where clients are tracking it.
    stream_id: u64,
    handle: tokio::task::JoinHandle<()>,
    cancel: crate::ToolCancelFlag,
    /// Direction the user added after this turn started, waiting to be read
    /// at the turn's next round.
    steers: SteerQueue,
}

/// Messages the user sent while a turn was already running.
///
/// A turn used to be a closed loop: once it started, nothing the user typed
/// could reach it until it finished. This is the side channel that lets them
/// redirect work in progress instead of waiting to correct it afterwards.
///
/// A plain mutex rather than a channel because both ends want the *set* of
/// pending messages, not a stream: the reader task appends, and the turn
/// takes everything at once between rounds.
pub type SteerQueue = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

/// Append one message, reporting whether it landed.
///
/// A poisoned lock drops it and says so, so the caller can tell the user it
/// did not land rather than leaving them to wonder why nothing changed.
fn push_steer(queue: &SteerQueue, text: String) -> bool {
    match queue.lock() {
        Ok(mut q) => {
            q.push(text);
            true
        }
        Err(_) => false,
    }
}

/// Take everything queued, leaving the queue empty.
///
/// A poisoned lock yields nothing rather than failing the turn: losing a
/// steering message is bad, but killing the turn the user was trying to
/// steer is worse.
pub fn drain_steers(queue: &SteerQueue) -> Vec<String> {
    queue
        .lock()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
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
    /// If there's already a task for this thread, it will be aborted —
    /// though the Chat handler displaces it explicitly first, so it can
    /// close the old turn out to clients before the new one exists.
    pub fn register(
        &mut self,
        thread_id: ThreadId,
        turn_id: u64,
        stream_id: u64,
        handle: tokio::task::JoinHandle<()>,
        cancel: crate::ToolCancelFlag,
        steers: SteerQueue,
    ) {
        if let Some(old) = self.tasks.insert(
            thread_id,
            RunningTurn {
                id: turn_id,
                stream_id,
                handle,
                cancel,
                steers,
            },
        ) {
            old.handle.abort();
        }
    }

    /// Abort and remove the turn running for a thread, returning the stream
    /// its close-out belongs on.
    ///
    /// An aborted turn dies at its next await — often the very wait for a
    /// tool approval or `ask_user` answer — and never reaches its own
    /// close-out. Nothing downstream speaks for it: no `ToolResult` for
    /// the request it left on the user's screen, no `ResponseDone` for the
    /// client's in-flight tracking. The caller must send that close-out,
    /// which is why the stream comes back. A turn that already finished
    /// returns `None` — its completion message is queued and will close
    /// the turn out on its own, and a second close-out would end the
    /// *next* turn's tracking in clients' eyes.
    pub fn displace(&mut self, thread_id: &ThreadId) -> Option<u64> {
        let turn = self.tasks.remove(thread_id)?;
        let was_running = !turn.handle.is_finished();
        turn.handle.abort();
        was_running.then_some(turn.stream_id)
    }

    /// Retire a turn once its completion message is handled — but only if it
    /// is still the turn registered for that thread. A turn that finished
    /// and was already reaped may have been replaced by the next one, and
    /// removing that replacement would leave a running turn nothing can
    /// stop and nothing counts as busy.
    ///
    /// Returns whether this completion still speaks for the thread — the
    /// caller's licence to close the turn's marker. Three cases:
    ///
    /// - The matching turn is registered: retire it, licence granted.
    /// - A *different* turn is registered: the thread already moved on,
    ///   and this stale completion must not write the new turn's stop
    ///   indicator. No licence.
    /// - *Nothing* is registered: the turn finished and `reap_finished`
    ///   (which runs for every incoming Chat frame, on all threads) swept
    ///   its entry before this completion drained. The completion is
    ///   still the thread's last word, and refusing it would leave the
    ///   start marker open forever — reported as Streaming to every
    ///   client, and re-run as a crashed turn on the next start.
    ///   Licence granted.
    pub fn remove_if(&mut self, thread_id: &ThreadId, turn_id: u64) -> bool {
        match self.tasks.get(thread_id) {
            Some(turn) if turn.id == turn_id => {
                self.tasks.remove(thread_id);
                true
            }
            Some(_) => false,
            None => true,
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

    /// Hand `text` to this thread's running turn, to be read at its next
    /// round. Returns whether there was a turn to hand it to.
    pub fn queue_steer(&self, thread_id: &ThreadId, text: String) -> bool {
        match self.tasks.get(thread_id) {
            Some(turn) => push_steer(&turn.steers, text),
            None => false,
        }
    }

    /// Hand `text` to the only running turn, if exactly one is running.
    ///
    /// Mirrors [`request_cancel_sole`](Self::request_cancel_sole): a client
    /// that names no thread means the turn it can see, which is unambiguous
    /// only while one is running. With several, this declines rather than
    /// steering the wrong conversation — which here would put words into a
    /// transcript the user was not looking at.
    pub fn queue_steer_sole(&self, text: String) -> bool {
        match self.tasks.values().next() {
            Some(turn) if self.tasks.len() == 1 => push_steer(&turn.steers, text),
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

/// Stop every turn when the registry goes away.
///
/// The connection handler returns through `?` from a dozen writer-backed
/// calls — a client vanishing mid-write is the ordinary case — and those
/// exits do not run the explicit `abort_all`. Dropping a `JoinHandle`
/// detaches its task rather than ending it, so without this a turn would
/// outlive its connection: still calling the model, still holding the
/// thread manager and vault, and, if parked on an `ask_user` question,
/// sitting there for the full five-minute wait with nobody left to answer.
impl Drop for ActiveTasks {
    fn drop(&mut self) {
        for turn in self.tasks.values() {
            turn.handle.abort();
        }
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

    fn queue() -> SteerQueue {
        Arc::new(std::sync::Mutex::new(Vec::new()))
    }

    /// Open a turn through a sink and read back the thread it announced.
    async fn announced_thread_of(turn_thread: ThreadId) -> Option<u64> {
        let (tx, mut rx) = channel();
        let mut sink = ChannelSink::new(tx, turn_thread, 1, 7);
        rustyclaw_core::gateway::protocol::server::send_stream_start(&mut sink, None)
            .await
            .expect("the sink should accept the frame");
        let ModelTaskMessage::Frame { data, .. } = rx.recv().await.expect("a frame should arrive")
        else {
            panic!("Expected a Frame message");
        };
        let frame: ServerFrame =
            rustyclaw_core::gateway::protocol::frames::deserialize_frame(&data)
                .expect("the frame should decode");
        match frame.payload {
            ServerPayload::StreamStart { thread_id } => thread_id,
            other => panic!("Expected StreamStart, got {other:?}"),
        }
    }

    /// A steer reaches the turn running for the thread it names, and the
    /// turn takes it whole at its next round.
    #[tokio::test]
    async fn a_steer_reaches_the_named_turn() {
        let mut tasks = ActiveTasks::new();
        let (a, b) = (ThreadId(1), ThreadId(2));
        let (qa, qb) = (queue(), queue());
        tasks.register(
            a,
            1,
            1,
            tokio::spawn(std::future::pending()),
            flag(),
            qa.clone(),
        );
        tasks.register(
            b,
            2,
            2,
            tokio::spawn(std::future::pending()),
            flag(),
            qb.clone(),
        );

        assert!(tasks.queue_steer(&a, "actually use sqlite".into()));
        assert_eq!(drain_steers(&qa), vec!["actually use sqlite".to_string()]);
        assert!(
            drain_steers(&qb).is_empty(),
            "the other conversation must not receive it"
        );
    }

    /// Draining empties the queue, so the next round does not re-read
    /// direction the model has already been given.
    #[tokio::test]
    async fn draining_consumes_the_queue() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(3);
        let q = queue();
        tasks.register(
            thread,
            1,
            1,
            tokio::spawn(std::future::pending()),
            flag(),
            q.clone(),
        );

        tasks.queue_steer(&thread, "first".into());
        tasks.queue_steer(&thread, "second".into());
        // Order matters: these are sentences the user typed in sequence.
        assert_eq!(
            drain_steers(&q),
            vec!["first".to_string(), "second".to_string()]
        );
        assert!(drain_steers(&q).is_empty(), "a drained queue stays drained");
    }

    /// Steering a thread with nothing running reports that it did not land,
    /// rather than silently dropping the user's words.
    #[tokio::test]
    async fn steering_an_idle_thread_reports_failure() {
        let tasks = ActiveTasks::new();
        assert!(!tasks.queue_steer(&ThreadId(9), "hello".into()));
    }

    /// An unaddressed steer goes to the only running turn, and declines when
    /// that is ambiguous — the same rule Stop follows, and for a sharper
    /// reason: guessing here types the user's words into a conversation they
    /// were not looking at.
    #[tokio::test]
    async fn an_unaddressed_steer_needs_exactly_one_turn() {
        let mut tasks = ActiveTasks::new();
        assert!(
            !tasks.queue_steer_sole("nobody home".into()),
            "no turns means nothing to steer"
        );

        let q = queue();
        tasks.register(
            ThreadId(1),
            1,
            1,
            tokio::spawn(std::future::pending()),
            flag(),
            q.clone(),
        );
        assert!(tasks.queue_steer_sole("the only one".into()));
        assert_eq!(drain_steers(&q), vec!["the only one".to_string()]);

        tasks.register(
            ThreadId(2),
            2,
            2,
            tokio::spawn(std::future::pending()),
            flag(),
            queue(),
        );
        assert!(
            !tasks.queue_steer_sole("which one?".into()),
            "with two running turns this must decline rather than guess"
        );
        assert!(drain_steers(&q).is_empty(), "and must not have picked one");
    }

    /// A turn with no thread announces no thread.
    ///
    /// `ThreadId(0)` is the registry key for a turn started when there was
    /// nothing to elect — every thread closed — and it is also the wire
    /// sentinel for "no thread is focused". Stamping it onto the boundary
    /// frames handed the desktop a thread it could never be showing, so it
    /// matched the whole turn against nothing and dropped the reply: text,
    /// tool calls, the lot, silently. The key stays local; the wire gets
    /// `None`, which clients already read as "apply this to the view".
    #[tokio::test]
    async fn a_turn_with_no_thread_announces_no_thread() {
        assert_eq!(
            announced_thread_of(ThreadId(0)).await,
            None,
            "the placeholder key must not reach the client"
        );
        assert_eq!(
            announced_thread_of(ThreadId(12)).await,
            Some(12),
            "a real thread is still named"
        );
    }

    /// A displaced turn hands back its stream so its close-out can be sent
    /// on the displaced turn's behalf — aborted mid-wait, it will never
    /// send its own, and the approval or question box it left on screen
    /// has no other retirement. A turn that already finished returns
    /// nothing: its queued completion message closes it out, and a second
    /// close-out would end the next turn's tracking.
    #[tokio::test]
    async fn displacing_a_running_turn_returns_its_stream() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(1);
        tasks.register(
            thread,
            1,
            41,
            tokio::spawn(std::future::pending()),
            flag(),
            queue(),
        );
        assert_eq!(
            tasks.displace(&thread),
            Some(41),
            "a running turn needs its close-out sent for it"
        );
        assert_eq!(tasks.displace(&thread), None, "nothing is left to displace");

        tasks.register(thread, 2, 42, tokio::spawn(async {}), flag(), queue());
        while !tasks.is_finished_for_test(&thread) {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            tasks.displace(&thread),
            None,
            "a finished turn closes itself out through its completion message"
        );
    }

    /// A stale completion has no licence to close the current turn's
    /// marker — but a merely *reaped* one keeps its licence. `remove_if`
    /// answers "does this completion still speak for the thread?": false
    /// only when a different turn took the thread over, whose marker the
    /// stale drain must not close. When nothing is registered — the turn
    /// finished and `reap_finished` swept it before its completion
    /// drained — the completion is still the last word, and refusing it
    /// would leave the thread's start marker open forever.
    #[tokio::test]
    async fn a_stale_completion_gets_no_licence() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(1);
        tasks.register(
            thread,
            1,
            1,
            tokio::spawn(std::future::pending()),
            flag(),
            queue(),
        );
        // The thread's next turn displaces the first and registers itself.
        tasks.displace(&thread);
        tasks.register(
            thread,
            2,
            3,
            tokio::spawn(std::future::pending()),
            flag(),
            queue(),
        );

        assert!(
            !tasks.remove_if(&thread, 1),
            "the displaced turn's completion must not speak for the thread"
        );
        assert!(
            tasks.remove_if(&thread, 2),
            "the registered turn's own completion may close it"
        );
    }

    /// The reap-then-drain ordering: a Chat frame for *any* thread reaps
    /// every finished turn before their completions drain. The drained
    /// completion then finds nothing registered — and must still close
    /// its turn's marker, because nothing else ever will.
    #[tokio::test]
    async fn a_reaped_completion_keeps_its_licence() {
        let mut tasks = ActiveTasks::new();
        let thread = ThreadId(1);
        tasks.register(thread, 1, 1, tokio::spawn(async {}), flag(), queue());
        while !tasks.is_finished_for_test(&thread) {
            tokio::task::yield_now().await;
        }
        // A Chat frame for some other thread sweeps the finished entry…
        tasks.reap_finished();
        // …and only then does the finished turn's completion drain.
        assert!(
            tasks.remove_if(&thread, 1),
            "a reaped completion is still the thread's last word and must \
             close its marker"
        );
    }

    /// Stop applies to the turn it was aimed at. One flag per connection
    /// would cancel every running turn and would be cleared by the next
    /// turn to start, throwing away a Stop the user had just pressed.
    #[tokio::test]
    async fn stopping_one_turn_leaves_the_others_running() {
        let mut tasks = ActiveTasks::new();
        let (a, b) = (ThreadId(1), ThreadId(2));
        let (flag_a, flag_b) = (flag(), flag());
        tasks.register(
            a,
            1,
            1,
            tokio::spawn(std::future::pending()),
            flag_a.clone(),
            queue(),
        );
        tasks.register(
            b,
            2,
            2,
            tokio::spawn(std::future::pending()),
            flag_b.clone(),
            queue(),
        );

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
        tasks.register(ThreadId(3), 3, 3, tokio::spawn(async {}), flag(), queue());
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
        tasks.register(thread, 1, 1, tokio::spawn(async {}), flag(), queue());
        while !tasks.is_finished_for_test(&thread) {
            tokio::task::yield_now().await;
        }
        tasks.reap_finished();

        // Turn B starts on the same thread.
        let b_cancel = flag();
        tasks.register(
            thread,
            2,
            2,
            tokio::spawn(std::future::pending()),
            b_cancel.clone(),
            queue(),
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
        tasks.register(ThreadId(1), 1, 1, handle, flag(), queue());

        tasks.abort_all();

        assert!(tasks.running_threads().is_empty());
        // Give the runtime a chance to process the abort.
        tokio::task::yield_now().await;
    }

    /// Every exit from the connection handler must stop its turns, not just
    /// the two that call `abort_all` — a write failure returns through `?`
    /// with the registry merely dropped, and a dropped `JoinHandle` detaches
    /// its task rather than ending it. The task here holds a sender, so the
    /// channel closing is proof it was actually dropped.
    #[tokio::test]
    async fn dropping_the_registry_stops_its_turns() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
        let handle = tokio::spawn(async move {
            let _tx = tx;
            std::future::pending::<()>().await;
        });

        let mut tasks = ActiveTasks::new();
        tasks.register(ThreadId(1), 1, 1, handle, flag(), queue());
        drop(tasks);

        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("a turn left running after its connection ended");
        assert!(closed.is_none(), "the task should be gone, not detached");
    }

    /// Only one turn may be registered at a time. A finished-but-not-yet-
    /// returned turn on another thread would otherwise sit alongside the new
    /// one, and Stop declines when it cannot tell which was meant.
    #[tokio::test]
    async fn starting_a_turn_displaces_a_finished_one_on_another_thread() {
        let mut tasks = ActiveTasks::new();
        let stale = flag();
        tasks.register(ThreadId(1), 1, 1, tokio::spawn(async {}), stale, queue());

        // What the spawn path does before registering.
        tasks.abort_all();
        let fresh = flag();
        tasks.register(
            ThreadId(2),
            2,
            2,
            tokio::spawn(std::future::pending()),
            fresh.clone(),
            queue(),
        );

        assert_eq!(tasks.running_threads().len(), 1);
        assert!(
            tasks.request_cancel_sole(),
            "Stop must not be left with an ambiguous choice"
        );
        assert!(fresh.load(Ordering::Relaxed));
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
            1,
            tokio::spawn(std::future::pending()),
            only.clone(),
            queue(),
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
            2,
            tokio::spawn(std::future::pending()),
            second.clone(),
            queue(),
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
        tasks.register(thread, 1, 1, handle, flag(), queue());

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
