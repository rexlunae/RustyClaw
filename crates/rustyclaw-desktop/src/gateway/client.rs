//! WebSocket client for gateway communication.

use anyhow::{anyhow, Result};
use futures::{SinkExt, StreamExt};
use rustyclaw_core::gateway::{ChatMessage, ClientFrame, ClientPayload, ServerFrame, ServerPayload, StatusType};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

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
        let url = Url::parse(url)?;
        let (ws_stream, _) = connect_async(url.as_str()).await?;
        let (mut write, mut read) = ws_stream.split();
        
        // Channels for communication
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<GatewayCommand>(32);
        let (event_tx, event_rx) = mpsc::channel::<GatewayEvent>(64);
        
        let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let connected_clone = connected.clone();
        
        // Spawn task to handle outgoing commands
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let frame = command_to_frame(cmd);
                let data = bincode::serde::encode_to_vec(&frame, bincode::config::standard())
                    .unwrap_or_default();
                
                if write.send(Message::Binary(data.into())).await.is_err() {
                    let _ = event_tx_clone.send(GatewayEvent::Disconnected {
                        reason: Some("Send failed".into()),
                    }).await;
                    break;
                }
            }
        });
        
        // Spawn task to handle incoming messages
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Binary(data)) => {
                        if let Ok((frame, _)) = bincode::serde::decode_from_slice::<ServerFrame, _>(
                            &data,
                            bincode::config::standard(),
                        ) {
                            if let Some(event) = frame_to_event(frame) {
                                if event_tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(Message::Text(text)) => {
                        // Try JSON fallback
                        if let Ok(frame) = serde_json::from_str::<ServerFrame>(&text) {
                            if let Some(event) = frame_to_event(frame) {
                                if event_tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => {
                        let _ = event_tx.send(GatewayEvent::Disconnected { reason: None }).await;
                        break;
                    }
                    Err(e) => {
                        let _ = event_tx.send(GatewayEvent::Disconnected {
                            reason: Some(e.to_string()),
                        }).await;
                        break;
                    }
                    _ => {}
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
    
    /// Receive the next event from the gateway.
    pub async fn recv(&self) -> Option<GatewayEvent> {
        let mut rx = self.event_rx.lock().await;
        rx.recv().await
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
    pub async fn authenticate(&self, code: String) -> Result<()> {
        self.send(GatewayCommand::Auth { code }).await
    }
    
    /// Unlock the vault.
    pub async fn unlock_vault(&self, password: String) -> Result<()> {
        self.send(GatewayCommand::VaultUnlock { password }).await
    }
    
    /// Approve or deny a tool call.
    pub async fn respond_tool_approval(&self, id: String, approved: bool) -> Result<()> {
        self.send(GatewayCommand::ToolApprove { id, approved }).await
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
            payload: ClientPayload::ToolApprovalResponse {
                id,
                approved,
            },
        },
        GatewayCommand::ThreadSwitch { thread_id } => ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch { thread_id },
        },
        GatewayCommand::ThreadCreate { label } => ClientFrame {
            frame_type: ClientFrameType::ThreadCreate,
            payload: ClientPayload::ThreadCreate { label: label.unwrap_or_default() },
        },
        GatewayCommand::SecretsList => ClientFrame {
            frame_type: ClientFrameType::SecretsList,
            payload: ClientPayload::SecretsList,
        },
        GatewayCommand::Cancel => ClientFrame {
            frame_type: ClientFrameType::Cancel,
            payload: ClientPayload::Empty,
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
        ServerPayload::Error { message, .. } => Some(GatewayEvent::Error { message }),
        ServerPayload::Info { message } => Some(GatewayEvent::Info { message }),
        _ => None,
    }
}
