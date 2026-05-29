//! SSH stdio client for gateway communication.

use anyhow::{Context, Result, anyhow};
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::gateway::{ServerPayload, SshConnection};
use rustyclaw_core::gateway::protocol::event_log::{
    Direction, ProtocolEventLog, default_log_path,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, mpsc};

/// Client for communicating with the RustyClaw gateway.
pub struct GatewayClient {
    /// Channel to send commands to the gateway
    cmd_tx: mpsc::Sender<GatewayCommand>,
    /// Channel to receive events from the gateway
    event_rx: Arc<Mutex<mpsc::Receiver<GatewayEvent>>>,
    /// Whether we're connected
    connected: Arc<std::sync::atomic::AtomicBool>,
}

impl GatewayClient {
    /// Connect to a gateway at the given URL.
    pub async fn connect(url: &str) -> Result<Self> {
        // ── Use shared SSH transport from rustyclaw_core ────────────────
        let (_connection, mut writer, mut reader) =
            SshConnection::connect(url).await.context("Failed to establish SSH transport")?;

        // Channels for communication
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<GatewayCommand>(32);
        let (event_tx, event_rx) = mpsc::channel::<GatewayEvent>(1024);

        let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let connected_clone = connected.clone();
        let next_stream_id = Arc::new(AtomicU64::new(1));
        let active_stream_id = Arc::new(AtomicU64::new(0));

        // Create protocol event log
        let event_log = default_log_path()
            .map(ProtocolEventLog::new)
            .unwrap_or_else(ProtocolEventLog::noop);
        event_log.log(
            rustyclaw_core::gateway::protocol::event_log::ProtocolEvent::Connection {
                message: format!("connecting to {}", url),
            },
        );
        let event_log_tx = event_log.clone();
        let event_log_rx = event_log.clone();

        // ── Spawn task to handle outgoing commands ─────────────────────
        let event_tx_clone = event_tx.clone();
        let next_stream_id_tx = next_stream_id.clone();
        let active_stream_id_tx = active_stream_id.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let stream_id = match &cmd {
                    GatewayCommand::Chat { .. } => {
                        let id = next_stream_id_tx.fetch_add(2, Ordering::Relaxed);
                        active_stream_id_tx.store(id, Ordering::Relaxed);
                        id
                    }
                    GatewayCommand::Cancel => active_stream_id_tx.load(Ordering::Relaxed),
                    _ => 0,
                };

                let frame = cmd.into_frame();
                let frame_type_name = format!("{:?}", frame.frame_type);

                // Log before sending
                event_log_tx.log_frame(Direction::Sent, &frame_type_name, stream_id, 0);

                if let Err(err) = writer.send_frame(stream_id, &frame).await {
                    event_log_tx.log(
                        rustyclaw_core::gateway::protocol::event_log::ProtocolEvent::EncodeError {
                            frame_type: frame_type_name.clone(),
                            error: err.to_string(),
                        },
                    );
                    let _ = event_tx_clone
                        .send(GatewayEvent::Disconnected {
                            reason: Some(err.to_string()),
                        })
                        .await;
                    break;
                }
            }
        });

        // ── Spawn task to handle incoming messages ─────────────────────
        let active_stream_id_rx = active_stream_id.clone();
        tokio::spawn(async move {
            // Streaming stats for the event log.
            let mut stream_chunk_count: u32 = 0;
            let mut stream_total_bytes: usize = 0;

            loop {
                match reader.recv_wire().await {
                    Ok(Some(envelope)) => {
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
                            ServerPayload::StreamStart => {
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

                        if matches!(
                            envelope.frame.payload,
                            ServerPayload::ResponseDone { .. }
                        ) {
                            let active = active_stream_id_rx.load(Ordering::Relaxed);
                            if active == envelope.stream_id {
                                active_stream_id_rx.store(0, Ordering::Relaxed);
                            }
                        }

                        if let Some(event) = GatewayEvent::from_server_frame(envelope.frame) {
                            if event_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        // EOF — drain stderr for diagnostic info
                        let ssh_err = reader.drain_stderr().await;
                        let reason = ssh_err
                            .lines()
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .find(|line| {
                                line.contains("Permission denied")
                                    || line.contains("Host key verification failed")
                                    || line.contains("Connection refused")
                                    || line.contains("Connection timed out")
                                    || line.contains("No route to host")
                                    || line.contains("Could not resolve hostname")
                                    || line.contains("kex_exchange_identification")
                            })
                            .map(str::to_string)
                            .or_else(|| {
                                ssh_err
                                    .lines()
                                    .map(str::trim)
                                    .rfind(|line| !line.is_empty())
                                    .map(str::to_string)
                            })
                            .unwrap_or_else(|| "SSH connection closed".to_string());
                        let _ = event_tx
                            .send(GatewayEvent::Disconnected {
                                reason: Some(reason),
                            })
                            .await;
                        break;
                    }
                    Err(err) => {
                        event_log_rx.log_decode_error(Direction::Received, 0, &err.to_string());
                        let _ = event_tx
                            .send(GatewayEvent::Error {
                                message: format!("Protocol error: {}", err),
                            })
                            .await;
                        break;
                    }
                }
            }

            connected_clone.store(false, std::sync::atomic::Ordering::SeqCst);
        });

        Ok(Self {
            cmd_tx,
            event_rx: Arc::new(Mutex::new(event_rx)),
            connected,
        })
    }

    /// Send a command to the gateway.
    pub async fn send(&self, cmd: GatewayCommand) -> Result<()> {
        self.cmd_tx
            .send(cmd)
            .await
            .map_err(|_| anyhow!("Failed to send command"))
    }

    /// Receive the next event from the gateway (blocks until one arrives).
    pub async fn recv(&self) -> Option<GatewayEvent> {
        let mut rx = self.event_rx.lock().await;
        rx.recv().await
    }

    /// Drain all currently-buffered events without blocking.
    pub async fn drain_available(&self) -> Vec<GatewayEvent> {
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

    /// Send a chat message.
    pub async fn chat(&self, message: String) -> Result<()> {
        self.send(GatewayCommand::Chat { message }).await
    }

    /// Authenticate with TOTP code.
    #[allow(dead_code)]
    pub async fn authenticate(&self, code: String) -> Result<()> {
        self.send(GatewayCommand::Auth { code }).await
    }

    /// Unlock the vault.
    #[allow(dead_code)]
    pub async fn unlock_vault(&self, password: String) -> Result<()> {
        self.send(GatewayCommand::VaultUnlock { password }).await
    }

    /// Approve or deny a tool call.
    #[allow(dead_code)]
    pub async fn respond_tool_approval(&self, id: String, approved: bool) -> Result<()> {
        self.send(GatewayCommand::ToolApprove { id, approved })
            .await
    }
}
