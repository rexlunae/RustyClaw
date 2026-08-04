//! Async gateway client over the SSH stdio transport.
//!
//! This is the shared connection abstraction used by every RustyClaw client
//! (desktop, TUI, …). It owns the SSH transport plus a reader and writer task,
//! translating between the binary wire frames and the client-facing
//! [`GatewayCommand`]/[`GatewayEvent`] enums (via their `into_frame` /
//! `from_server_frame` conversions). Clients drive it purely through
//! [`GatewayCommand`]s and consume [`GatewayEvent`]s — they never touch the
//! wire protocol or stream-id bookkeeping directly.

use anyhow::{Context, Result, anyhow};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, mpsc};

use crate::gateway::client_types::{GatewayCommand, GatewayEvent};
use crate::gateway::protocol::event_log::{
    Direction, ProtocolEvent, ProtocolEventLog, default_log_path,
};
use crate::gateway::{ServerPayload, SshConnection, SshReader, SshWriter};

/// An event, and the thread whose turn produced it.
///
/// Turn-scoped frames — chunks, thinking, tool calls — carry no thread of
/// their own on the wire; they belong to the turn, and the turn's thread is
/// announced once on `StreamStart`. Resolving that here means every client
/// gets the attribution without reimplementing the stream bookkeeping, and
/// none of them can forget to.
///
/// `thread_id` is `None` for connection-scoped events (`Hello`,
/// `ThreadsUpdate`, transport errors) and for turns from a gateway too old to
/// announce one.
#[derive(Clone, Debug)]
pub struct ThreadEvent {
    pub thread_id: Option<u64>,
    pub event: GatewayEvent,
}

impl ThreadEvent {
    /// An event that belongs to the connection rather than to any turn.
    pub fn untargeted(event: GatewayEvent) -> Self {
        Self {
            thread_id: None,
            event,
        }
    }
}

/// Client for communicating with the RustyClaw gateway.
pub struct GatewayClient {
    /// Channel to send commands to the gateway.
    cmd_tx: mpsc::Sender<GatewayCommand>,
    /// Channel to receive events from the gateway.
    event_rx: Arc<Mutex<mpsc::Receiver<ThreadEvent>>>,
    /// Whether we're connected.
    connected: Arc<std::sync::atomic::AtomicBool>,
    /// The underlying SSH connection, held to keep the transport alive for the
    /// lifetime of the client.
    _connection: SshConnection,
}

impl GatewayClient {
    /// Connect to a gateway at the given URL, establishing the SSH transport
    /// and waiting for the gateway's first frame.
    ///
    /// Spawning the SSH subprocess alone proves nothing — it succeeds even
    /// when the host is unreachable or no gateway is running.  The handshake
    /// wait turns those cases into an `Err` here instead of a phantom
    /// "connected" state that never produces events.
    pub async fn connect(url: &str) -> Result<Self> {
        let (connection, writer, mut reader) = SshConnection::connect(url)
            .await
            .context("Failed to establish SSH transport")?;
        reader
            .wait_first_frame(crate::gateway::HANDSHAKE_TIMEOUT)
            .await
            .with_context(|| format!("Gateway at {} is not responding", url))?;
        Ok(Self::from_transport(connection, writer, reader, Some(url)))
    }

    /// Build a client over an already-established SSH transport.
    ///
    /// Clients that establish the SSH connection themselves (e.g. via an
    /// interactive connection dialog) can hand the transport parts here rather
    /// than reconnecting from a URL. `log_label` is recorded in the protocol
    /// event log as the connection target, if known.
    pub fn from_transport(
        connection: SshConnection,
        mut writer: SshWriter,
        mut reader: SshReader,
        log_label: Option<&str>,
    ) -> Self {
        // Channels for communication.
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<GatewayCommand>(32);
        let (event_tx, event_rx) = mpsc::channel::<ThreadEvent>(1024);

        let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let connected_clone = connected.clone();
        let next_stream_id = Arc::new(AtomicU64::new(1));
        let active_stream_id = Arc::new(AtomicU64::new(0));
        // Which thread each in-flight turn is running in, keyed by the
        // stream its request went out on. Written from both sides: the
        // sender seeds it from the thread the client named on `Chat`, and
        // the reader corrects it from `StreamStart`. Seeding matters
        // because a turn can ask for things *before* its stream opens — an
        // auth failure on the first model call raises a credential request
        // ahead of any `StreamStart` — and an unattributed request cannot
        // be retired when its turn ends. Plain mutex: every use is one map
        // operation with no await inside.
        let stream_threads: Arc<std::sync::Mutex<std::collections::HashMap<u64, u64>>> =
            Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        // Create protocol event log.
        let event_log = default_log_path()
            .map(ProtocolEventLog::new)
            .unwrap_or_else(ProtocolEventLog::noop);
        event_log.log(ProtocolEvent::Connection {
            message: format!("connecting to {}", log_label.unwrap_or("gateway")),
        });
        let event_log_tx = event_log.clone();
        let event_log_rx = event_log.clone();

        // ── Spawn task to handle outgoing commands ─────────────────────
        let event_tx_clone = event_tx.clone();
        let next_stream_id_tx = next_stream_id.clone();
        let active_stream_id_tx = active_stream_id.clone();
        let stream_threads_tx = stream_threads.clone();
        let connected_tx = connected.clone();
        // Lets the writer observe the reader's death. The sender lives in the
        // reader task and nothing is ever sent on it — dropping it when that
        // task ends is the signal.
        let (reader_gone_tx, mut reader_gone_rx) = tokio::sync::watch::channel(());
        tokio::spawn(async move {
            loop {
                let cmd = tokio::select! {
                    received = cmd_rx.recv() => match received {
                        Some(cmd) => cmd,
                        None => break,
                    },
                    // The reader is gone, so nothing more will arrive and
                    // nothing sent could be answered. Leaving matters beyond
                    // this task: it holds the second sender on the event
                    // channel, so parking here forever would hold that channel
                    // open, and consumers waiting on it stay alive holding the
                    // client — which keeps the SSH child running for as long
                    // as the process lives, once per reconnect.
                    _ = reader_gone_rx.changed() => break,
                };
                let stream_id = match &cmd {
                    GatewayCommand::Chat { thread_id, .. } => {
                        let id = next_stream_id_tx.fetch_add(2, Ordering::Relaxed);
                        active_stream_id_tx.store(id, Ordering::Relaxed);
                        if let Some(thread) = thread_id {
                            stream_threads_tx
                                .lock()
                                .expect("stream thread map poisoned")
                                .insert(id, *thread);
                        }
                        id
                    }
                    GatewayCommand::Cancel { .. } => active_stream_id_tx.load(Ordering::Relaxed),
                    _ => 0,
                };

                let frame = cmd.into_frame();
                let frame_type_name = format!("{:?}", frame.frame_type);

                // Log before sending.
                event_log_tx.log_frame(Direction::Sent, &frame_type_name, stream_id, 0);

                if let Err(err) = writer.send_frame(stream_id, &frame).await {
                    event_log_tx.log(ProtocolEvent::EncodeError {
                        frame_type: frame_type_name.clone(),
                        error: err.to_string(),
                    });
                    let _ = event_tx_clone
                        .send(ThreadEvent::untargeted(GatewayEvent::Disconnected {
                            reason: Some(err.to_string()),
                        }))
                        .await;
                    break;
                }
            }

            // This task owns the only receiver for `cmd_tx`, so once it ends
            // every later `send` fails — the connection is write-dead even
            // when the reader is still delivering frames. Saying so here is
            // what stops that from being invisible: the flag used to be
            // cleared only by the reader, so a client whose writer had died
            // still reported itself connected while silently dropping every
            // request the user made.
            connected_tx.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        // ── Spawn task to handle incoming messages ─────────────────────
        let active_stream_id_rx = active_stream_id.clone();
        let stream_threads_rx = stream_threads;
        tokio::spawn(async move {
            // Held for this task's lifetime, never sent on: dropping it when
            // the reader ends is what releases the writer (see its select).
            let _reader_gone_tx = reader_gone_tx;
            // Streaming stats for the event log.
            let mut stream_chunk_count: u32 = 0;
            let mut stream_total_bytes: usize = 0;
            // Turn streams whose close-out has already been forwarded.
            // Stream ids are never reused (the sender allocates a fresh
            // one per Chat), so a *turn-scoped* frame arriving on one of
            // these is a displaced turn's leftover output, drained from
            // the gateway's channel after the abort. By then the thread
            // mapping is gone, and an unattributed frame renders into
            // whatever conversation the user is looking at.
            //
            // Only turn-scoped frames are dropped. The dispatch tail
            // legitimately sends connection-scoped frames on the turn's
            // stream *after* its close-out — above all the completion
            // snapshot (`ThreadMessages`), which is how a background
            // turn's transcript reaches the client at all. Those carry
            // their own thread ids and need no stream attribution.
            let mut closed_streams: std::collections::HashSet<u64> =
                std::collections::HashSet::new();
            fn turn_scoped(payload: &ServerPayload) -> bool {
                matches!(
                    payload,
                    ServerPayload::StreamStart { .. }
                        | ServerPayload::Chunk { .. }
                        | ServerPayload::ThinkingStart
                        | ServerPayload::ThinkingDelta { .. }
                        | ServerPayload::ThinkingEnd
                        | ServerPayload::ToolCall { .. }
                        | ServerPayload::ToolResult { .. }
                        | ServerPayload::ToolResultMedia { .. }
                        | ServerPayload::ToolOutputStart { .. }
                        | ServerPayload::ToolOutputDelta { .. }
                        | ServerPayload::ToolOutputEnd { .. }
                        | ServerPayload::ToolStatus { .. }
                        | ServerPayload::ResponseDone { .. }
                        | ServerPayload::ToolApprovalRequest { .. }
                        | ServerPayload::UserPromptRequest { .. }
                        | ServerPayload::CredentialRequest { .. }
                        | ServerPayload::DeviceFlowStart { .. }
                        | ServerPayload::DeviceFlowComplete
                        | ServerPayload::DomQuery { .. }
                )
            }

            loop {
                match reader.recv_wire().await {
                    Ok(Some(envelope)) => {
                        if closed_streams.contains(&envelope.stream_id)
                            && turn_scoped(&envelope.frame.payload)
                        {
                            event_log_rx.log_frame(
                                Direction::Received,
                                &format!(
                                    "{:?} (dropped: stream closed)",
                                    envelope.frame.frame_type
                                ),
                                envelope.stream_id,
                                0,
                            );
                            continue;
                        }
                        let len = 0; // wire already consumed, len not available here
                        let frame_type_name = format!("{:?}", envelope.frame.frame_type);
                        event_log_rx.log_frame(
                            Direction::Received,
                            &frame_type_name,
                            envelope.stream_id,
                            len,
                        );

                        // Track streaming progress.
                        match &envelope.frame.payload {
                            ServerPayload::StreamStart { .. } => {
                                stream_chunk_count = 0;
                                stream_total_bytes = 0;
                                event_log_rx.log_streaming("started");
                            }
                            ServerPayload::Chunk { delta } => {
                                stream_chunk_count += 1;
                                stream_total_bytes += delta.len();
                            }
                            ServerPayload::ResponseDone { .. } => {
                                event_log_rx.log_streaming(&format!(
                                    "done chunks={} chars={}",
                                    stream_chunk_count, stream_total_bytes,
                                ));
                            }
                            _ => {}
                        }

                        if matches!(envelope.frame.payload, ServerPayload::ResponseDone { .. }) {
                            let active = active_stream_id_rx.load(Ordering::Relaxed);
                            if active == envelope.stream_id {
                                active_stream_id_rx.store(0, Ordering::Relaxed);
                            }
                        }

                        // Attribute the frame to a thread before anyone
                        // sees it. Every frame of a turn shares the stream id
                        // its request went out on, and `StreamStart` says
                        // which thread that stream is running in — so the
                        // mapping is learnable here, once, instead of in each
                        // client. Without it a chunk is just text, and two
                        // turns' text interleaves into one message.
                        let stream_id = envelope.stream_id;
                        if let ServerPayload::StreamStart {
                            thread_id: Some(thread),
                        } = &envelope.frame.payload
                        {
                            // The gateway's announcement corrects the seed:
                            // the client's guess is wrong when it named
                            // nothing and a thread was elected.
                            stream_threads_rx
                                .lock()
                                .expect("stream thread map poisoned")
                                .insert(stream_id, *thread);
                        }
                        let thread_id = stream_threads_rx
                            .lock()
                            .expect("stream thread map poisoned")
                            .get(&stream_id)
                            .copied();
                        let closing =
                            matches!(envelope.frame.payload, ServerPayload::ResponseDone { .. });
                        let announced = envelope.frame.frame_type;
                        if let Some(event) = GatewayEvent::from_server_frame(envelope.frame) {
                            if event_tx
                                .send(ThreadEvent { thread_id, event })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        } else {
                            // Some payloads carry nothing a client acts on, so
                            // this is not always wrong — but it is the last
                            // place a frame can disappear, and it used to do so
                            // without a word. A frame whose type promised a
                            // reply and whose payload had decoded into
                            // something else went missing here, and the only
                            // visible symptom was a view that never filled in.
                            tracing::debug!(
                                frame_type = ?announced,
                                stream_id = envelope.stream_id,
                                "Received frame carried no client event"
                            );
                        }
                        // Released only after its close-out has been stamped,
                        // so the frame that ends the turn still names it.
                        // A stream the map was tracking is a turn's, and a
                        // turn's stream carries nothing after its close-out
                        // — so it is closed for good, and later arrivals
                        // are dropped above. Streams the map never knew
                        // (an old gateway that attributes nothing) keep
                        // the old behaviour.
                        if closing
                            && stream_threads_rx
                                .lock()
                                .expect("stream thread map poisoned")
                                .remove(&stream_id)
                                .is_some()
                        {
                            closed_streams.insert(stream_id);
                        }
                    }
                    Ok(None) => {
                        // EOF — drain stderr for diagnostic info.
                        let ssh_err = reader.drain_stderr().await;
                        let reason = crate::gateway::parse_ssh_error(&ssh_err);
                        let _ = event_tx
                            .send(ThreadEvent::untargeted(GatewayEvent::Disconnected {
                                reason: Some(reason),
                            }))
                            .await;
                        break;
                    }
                    Err(err) => {
                        event_log_rx.log_decode_error(Direction::Received, 0, &err.to_string());
                        let _ = event_tx
                            .send(ThreadEvent::untargeted(GatewayEvent::Error {
                                message: format!("Protocol error: {}", err),
                            }))
                            .await;
                        // A decode error is fatal — the frame stream cannot
                        // be resynced, and this loop ends here, so no
                        // close-out will ever arrive for the turns still
                        // tracked. Without a Disconnected the UI keeps its
                        // spinner and gates the composer forever.
                        let _ = event_tx
                            .send(ThreadEvent::untargeted(GatewayEvent::Disconnected {
                                reason: Some(format!("Protocol error: {}", err)),
                            }))
                            .await;
                        break;
                    }
                }
            }

            connected_clone.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        Self {
            cmd_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            connected,
            _connection: connection,
        }
    }

    /// Send a command to the gateway.
    pub async fn send(&self, cmd: GatewayCommand) -> Result<()> {
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|_| anyhow!("Failed to send command"))
    }

    /// Receive the next event from the gateway (blocks until one arrives).
    pub async fn recv(&self) -> Option<ThreadEvent> {
        let mut rx = self.event_rx.lock().await;
        rx.recv().await
    }

    /// Drain all currently-buffered events without blocking.
    pub async fn drain_available(&self) -> Vec<ThreadEvent> {
        let mut rx = self.event_rx.lock().await;
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Check if connected.
    pub fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Send a chat message on the gateway's current thread.
    pub async fn chat(&self, message: String) -> Result<()> {
        self.send(GatewayCommand::Chat {
            message,
            thread_id: None,
        })
        .await
    }

    /// Send a chat message, naming the thread it belongs to. `None` leaves
    /// the choice to the gateway, for callers that have no thread of their
    /// own yet — the first message of a fresh session, a headless one-shot.
    pub async fn chat_in_thread(&self, message: String, thread_id: Option<u64>) -> Result<()> {
        self.send(GatewayCommand::Chat { message, thread_id }).await
    }

    /// Authenticate with TOTP code.
    pub async fn authenticate(&self, code: String) -> Result<()> {
        self.send(GatewayCommand::Auth { code }).await
    }

    /// Unlock the vault.
    pub async fn unlock_vault(&self, password: String) -> Result<()> {
        self.send(GatewayCommand::VaultUnlock { password }).await
    }

    /// Approve or deny a tool call.
    pub async fn respond_tool_approval(&self, id: String, approved: bool) -> Result<()> {
        self.send(GatewayCommand::ToolApprove { id, approved })
            .await
    }
}

// The writer task owns the only receiver for the command channel, so its
// death is total: every later request fails. These tests pin that such a
// connection stops claiming to be connected — the reader half staying
// healthy is exactly the case that used to hide it.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Spawn a child whose stdin is closed but whose stdout stays open: the
    /// write side breaks while the read side is still perfectly healthy.
    fn half_open_child() -> tokio::process::Child {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg("exec 0<&-; sleep 30")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawning a shell should succeed")
    }

    /// A connection whose writer has died must report itself disconnected.
    ///
    /// The flag used to be cleared only by the reader task, so a client that
    /// could no longer send anything still answered "connected" for as long
    /// as frames kept arriving. Nothing retried, nothing reconnected, and
    /// every request the user made was dropped on the floor in silence.
    #[tokio::test]
    async fn a_write_dead_connection_stops_claiming_to_be_connected() {
        let (conn, writer, reader) =
            SshConnection::from_child(half_open_child()).expect("splitting the child");
        let client = GatewayClient::from_transport(conn, writer, reader, None);
        assert!(client.is_connected(), "a fresh client starts connected");

        // Writing to the closed read end fails; the writer task ends. The
        // reader is still blocked on a live stdout the whole time, so only
        // the writer's own bookkeeping can report this.
        for _ in 0..100 {
            if !client.is_connected() {
                break;
            }
            let _ = client.send(GatewayCommand::ThreadList).await;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        assert!(
            !client.is_connected(),
            "a client that can no longer send must not report itself connected"
        );
    }

    /// And once it is write-dead, `send` says so rather than succeeding.
    ///
    /// This is the error the history-request call sites discard: a request
    /// that never left the process leaves the view waiting for a reply that
    /// cannot come, which is what an empty thread with a populated sidebar
    /// row actually is.
    #[tokio::test]
    async fn sending_on_a_write_dead_connection_reports_the_failure() {
        let (conn, writer, reader) =
            SshConnection::from_child(half_open_child()).expect("splitting the child");
        let client = GatewayClient::from_transport(conn, writer, reader, None);

        let mut last = Ok(());
        for _ in 0..100 {
            last = client.send(GatewayCommand::ThreadList).await;
            if last.is_err() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        assert!(
            last.is_err(),
            "a send on a dead command channel must surface an error, not vanish"
        );
    }

    /// Spawn a child whose stdout is closed but whose stdin stays open: the
    /// read side ends while the write side is still perfectly healthy — the
    /// mirror of `half_open_child`.
    ///
    /// stderr is closed alongside stdout because the EOF path drains it for a
    /// diagnostic reason; a child holding it open would stall the reader there
    /// rather than at the thing under test.
    fn read_dead_child() -> tokio::process::Child {
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg("exec 1>&- 2>&-; sleep 30")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawning a shell should succeed")
    }

    /// A connection whose reader has died must end its event stream.
    ///
    /// The writer parks on the command channel until the last client is
    /// dropped, and it holds the second sender on the event channel — so
    /// unless the reader's death releases it, the stream never ends. That is
    /// not merely an idle task: consumers waiting on the stream hold the
    /// client, the client holds the SSH child, and the child outlives the
    /// connection it belonged to, once per reconnect.
    #[tokio::test]
    async fn a_read_dead_connection_ends_its_event_stream() {
        let (conn, writer, reader) =
            SshConnection::from_child(read_dead_child()).expect("splitting the child");
        let client = GatewayClient::from_transport(conn, writer, reader, None);

        // The reader announces the disconnect before it goes, so drain rather
        // than expecting `None` first.
        let ended = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while client.recv().await.is_some() {}
        })
        .await;

        assert!(
            ended.is_ok(),
            "a client whose reader has died must end its event stream, \
             not park its consumers on it forever"
        );
    }
}
