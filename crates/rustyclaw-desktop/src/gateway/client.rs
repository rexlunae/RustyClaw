//! SSH stdio client for gateway communication.

use anyhow::{Context, Result, anyhow};
use rustyclaw_core::gateway::{
    ChatMessage, ClientFrame, ClientPayload, ServerFrame, ServerPayload, SshConnection,
    StatusType,
};
use rustyclaw_core::gateway::protocol::event_log::{
    Direction, ProtocolEventLog, default_log_path,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, mpsc};

use super::protocol::{GatewayCommand, GatewayEvent, ThreadInfoDto};

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

                let frame = command_to_frame(cmd);
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

                        if let Some(event) = frame_to_event(envelope.frame) {
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

/// Convert a gateway command to a client frame.
fn command_to_frame(cmd: GatewayCommand) -> ClientFrame {
    use rustyclaw_core::gateway::ClientFrameType;

    match cmd {
        GatewayCommand::Chat { message } => ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", &message)],
            },
        },
        GatewayCommand::Auth { code } => ClientFrame {
            frame_type: ClientFrameType::AuthResponse,
            payload: ClientPayload::AuthResponse { code },
        },
        GatewayCommand::VaultUnlock { password } => ClientFrame {
            frame_type: ClientFrameType::UnlockVault,
            payload: ClientPayload::UnlockVault { password },
        },
        GatewayCommand::ToolApprove { id, approved } => ClientFrame {
            frame_type: ClientFrameType::ToolApprovalResponse,
            payload: ClientPayload::ToolApprovalResponse { id, approved },
        },
        GatewayCommand::ThreadSwitch { thread_id } => ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch { thread_id },
        },
        GatewayCommand::ThreadCreate { label } => ClientFrame {
            frame_type: ClientFrameType::ThreadCreate,
            payload: ClientPayload::ThreadCreate {
                label: label.unwrap_or_default(),
            },
        },
        GatewayCommand::ThreadList => ClientFrame {
            frame_type: ClientFrameType::ThreadList,
            payload: ClientPayload::ThreadList,
        },
        GatewayCommand::ThreadClose { thread_id } => ClientFrame {
            frame_type: ClientFrameType::ThreadClose,
            payload: ClientPayload::ThreadClose { thread_id },
        },
        GatewayCommand::UserPromptResponse {
            id,
            dismissed,
            value,
        } => ClientFrame {
            frame_type: ClientFrameType::UserPromptResponse,
            payload: ClientPayload::UserPromptResponse {
                id,
                dismissed,
                value,
            },
        },
        GatewayCommand::CredentialResponse {
            id,
            dismissed,
            value,
        } => ClientFrame {
            frame_type: ClientFrameType::CredentialResponse,
            payload: ClientPayload::CredentialResponse {
                id,
                dismissed,
                value,
            },
        },
        GatewayCommand::SecretsList => ClientFrame {
            frame_type: ClientFrameType::SecretsList,
            payload: ClientPayload::SecretsList,
        },
        GatewayCommand::Cancel => ClientFrame {
            frame_type: ClientFrameType::Cancel,
            payload: ClientPayload::Empty,
        },
        GatewayCommand::ModelSwitch { provider, model } => ClientFrame {
            frame_type: ClientFrameType::ModelSwitch,
            payload: ClientPayload::ModelSwitch { provider, model },
        },
        GatewayCommand::DomQueryResponse {
            id,
            result,
            is_error,
        } => ClientFrame {
            frame_type: ClientFrameType::DomQueryResponse,
            payload: ClientPayload::DomQueryResponse {
                id,
                result,
                is_error,
            },
        },
        GatewayCommand::SetAgentName { name } => ClientFrame {
            frame_type: ClientFrameType::SetAgentName,
            payload: ClientPayload::SetAgentName { name },
        },
        GatewayCommand::SetWorkingDirectory { path } => ClientFrame {
            frame_type: ClientFrameType::SetWorkingDirectory,
            payload: ClientPayload::SetWorkingDirectory { path },
        },
        GatewayCommand::SecretsStore { key, value } => ClientFrame {
            frame_type: ClientFrameType::SecretsStore,
            payload: ClientPayload::SecretsStore { key, value },
        },
        GatewayCommand::ThreadRename {
            thread_id,
            new_label,
        } => ClientFrame {
            frame_type: ClientFrameType::ThreadRename,
            payload: ClientPayload::ThreadRename {
                thread_id,
                new_label,
            },
        },
        GatewayCommand::SecretsDelete { key } => ClientFrame {
            frame_type: ClientFrameType::SecretsDelete,
            payload: ClientPayload::SecretsDelete { key },
        },
        GatewayCommand::SecretsSetPolicy { name, policy, skills } => ClientFrame {
            frame_type: ClientFrameType::SecretsSetPolicy,
            payload: ClientPayload::SecretsSetPolicy { name, policy, skills },
        },
    }
}

/// Convert a server frame to a gateway event.
fn frame_to_event(frame: ServerFrame) -> Option<GatewayEvent> {
    match frame.payload {
        ServerPayload::Hello {
            agent,
            vault_locked,
            provider,
            model,
            ..
        } => Some(GatewayEvent::Connected {
            agent: Some(agent),
            vault_locked,
            provider,
            model,
        }),
        ServerPayload::Status { status, detail } => match status {
            StatusType::ModelReady => Some(GatewayEvent::ModelReady { model: detail }),
            StatusType::ModelError => Some(GatewayEvent::ModelError { message: detail }),
            StatusType::VaultLocked => Some(GatewayEvent::VaultLocked),
            _ => Some(GatewayEvent::Info { message: detail }),
        },
        ServerPayload::AuthChallenge { .. } => Some(GatewayEvent::AuthRequired),
        ServerPayload::AuthResult { ok, message, retry } => {
            if ok {
                Some(GatewayEvent::AuthSuccess)
            } else {
                Some(GatewayEvent::AuthFailed {
                    message: message.unwrap_or_default(),
                    retry: retry.unwrap_or(false),
                })
            }
        }
        ServerPayload::VaultUnlocked { ok, message } => {
            if ok {
                Some(GatewayEvent::VaultUnlocked)
            } else {
                Some(GatewayEvent::Error {
                    message: message.unwrap_or_else(|| "Failed to unlock vault".into()),
                })
            }
        }
        ServerPayload::StreamStart => Some(GatewayEvent::StreamStart),
        ServerPayload::ThinkingStart => Some(GatewayEvent::ThinkingStart),
        ServerPayload::ThinkingEnd => Some(GatewayEvent::ThinkingEnd),
        ServerPayload::Chunk { delta } => Some(GatewayEvent::Chunk { delta }),
        ServerPayload::ResponseDone { .. } => Some(GatewayEvent::ResponseDone),
        ServerPayload::ToolCall {
            id,
            name,
            arguments,
        } => Some(GatewayEvent::ToolCall {
            id,
            name,
            arguments,
        }),
        ServerPayload::ToolResult {
            id,
            name,
            result,
            is_error,
        } => Some(GatewayEvent::ToolResult {
            id,
            name,
            result,
            is_error,
        }),
        ServerPayload::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => Some(GatewayEvent::ToolApprovalRequest {
            id,
            name,
            arguments,
        }),
        ServerPayload::UserPromptRequest { id, prompt } => {
            Some(GatewayEvent::UserPromptRequest { id, prompt })
        }
        ServerPayload::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => Some(GatewayEvent::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        }),
        ServerPayload::DeviceFlowStart { url, code, message } => {
            Some(GatewayEvent::DeviceFlowStart { url, code, message })
        }
        ServerPayload::DeviceFlowComplete => Some(GatewayEvent::DeviceFlowComplete),
        ServerPayload::ThreadsUpdate {
            threads,
            foreground_id,
        } => Some(GatewayEvent::ThreadsUpdate {
            threads: threads
                .into_iter()
                .map(|t| ThreadInfoDto {
                    id: t.id,
                    label: Some(t.label),
                    description: t.description,
                    status: t.status.unwrap_or_default(),
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                })
                .collect(),
            foreground_id,
        }),
        ServerPayload::SecretsListResult { ok, entries } => {
            Some(GatewayEvent::SecretsListResult {
                ok,
                entries: entries.into_iter().map(Into::into).collect(),
            })
        }
        ServerPayload::SecretsStoreResult { ok, message } => {
            Some(GatewayEvent::SecretsStoreResult { ok, message })
        }
        ServerPayload::SecretsDeleteResult { ok, message } => {
            Some(GatewayEvent::SecretsDeleteResult { ok, message })
        }
        ServerPayload::SecretsSetPolicyResult { ok, message } => {
            Some(GatewayEvent::SecretsSetPolicyResult { ok, message })
        }
        ServerPayload::Error { message, .. } => Some(GatewayEvent::Error { message }),
        ServerPayload::Info { message } => Some(GatewayEvent::Info { message }),
        ServerPayload::DomQuery { id, js } => Some(GatewayEvent::DomQuery { id, js }),
        _ => None,
    }
}
