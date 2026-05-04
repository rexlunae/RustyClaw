//! SSH stdio client for gateway communication.

use anyhow::{Context, Result, anyhow};
use rustyclaw_core::gateway::{
    ChatMessage, ClientFrame, ClientPayload, ServerFrame, ServerPayload, StatusType,
    deserialize_frame, serialize_frame,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
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
        if url.scheme() != "ssh" {
            anyhow::bail!(
                "Unsupported gateway URL scheme '{}'; expected ssh://",
                url.scheme()
            );
        }

        let host = url.host_str().unwrap_or("localhost").to_string();
        let port = url.port();
        let user = if url.username().is_empty() {
            None
        } else {
            Some(url.username().to_string())
        };

        // Channels for communication
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<GatewayCommand>(32);
        let (event_tx, event_rx) = mpsc::channel::<GatewayEvent>(64);

        let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let connected_clone = connected.clone();

        let client_key_path = rustyclaw_core::pairing::ClientKeyPair::load_or_generate(None)
            .map(|_| rustyclaw_core::pairing::default_client_key_path())
            .context("Failed to load/generate client key")?;

        let mut cmd = tokio::process::Command::new("ssh");
        cmd.arg("-T");
        cmd.arg("-o").arg("PreferredAuthentications=publickey");
        cmd.arg("-o").arg("IdentitiesOnly=yes");
        cmd.arg("-i").arg(&client_key_path);

        let known_hosts_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("rustyclaw")
            .join("known_hosts");
        cmd.arg("-o")
            .arg(format!("UserKnownHostsFile={}", known_hosts_path.display()));
        cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
        cmd.arg("-o").arg("BatchMode=yes");
        if let Some(port) = port {
            cmd.arg("-p").arg(port.to_string());
        }
        let target = if let Some(user) = &user {
            format!("{}@{}", user, host)
        } else {
            host.clone()
        };
        cmd.arg(&target);
        cmd.arg("rustyclaw-gateway").arg("--ssh-stdio");
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("Failed to spawn ssh")?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("SSH stdin unavailable"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("SSH stdout unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("SSH stderr unavailable"))?;

        // Spawn task to handle outgoing commands
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                let frame = command_to_frame(cmd);
                let data = match serialize_frame(&frame) {
                    Ok(data) => data,
                    Err(err) => {
                        let _ = event_tx_clone
                            .send(GatewayEvent::Error {
                                message: format!("Failed to encode frame: {}", err),
                            })
                            .await;
                        break;
                    }
                };

                let len = data.len() as u32;
                let write_result: Result<(), std::io::Error> = async {
                    stdin.write_all(&len.to_be_bytes()).await?;
                    stdin.write_all(&data).await?;
                    stdin.flush().await?;
                    Ok(())
                }
                .await;

                if write_result.is_err() {
                    let _ = event_tx_clone
                        .send(GatewayEvent::Disconnected {
                            reason: Some("Send failed".into()),
                        })
                        .await;
                    break;
                }
            }
        });

        // Spawn task to handle incoming messages
        tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            loop {
                match stdout.read_exact(&mut len_buf).await {
                    Ok(_) => {
                        let len = u32::from_be_bytes(len_buf) as usize;
                        if len > 16 * 1024 * 1024 {
                            let _ = event_tx
                                .send(GatewayEvent::Error {
                                    message: "SSH frame too large".into(),
                                })
                                .await;
                            break;
                        }

                        let mut frame_buf = vec![0u8; len];
                        let read_result: Result<(), std::io::Error> = async {
                            stdout.read_exact(&mut frame_buf).await?;
                            Ok(())
                        }
                        .await;
                        if read_result.is_err() {
                            let _ = event_tx
                                .send(GatewayEvent::Disconnected {
                                    reason: Some("SSH read error".into()),
                                })
                                .await;
                            break;
                        }

                        match deserialize_frame::<ServerFrame>(&frame_buf) {
                            Ok(frame) => {
                                if let Some(event) = frame_to_event(frame)
                                    && event_tx.send(event).await.is_err()
                                {
                                    break;
                                }
                            }
                            Err(err) => {
                                let _ = event_tx
                                    .send(GatewayEvent::Error {
                                        message: format!("Protocol error: {}", err),
                                    })
                                    .await;
                            }
                        }
                    }
                    Err(_) => {
                        let mut stderr_buf = Vec::new();
                        let _ = stderr.read_to_end(&mut stderr_buf).await;
                        let ssh_err = String::from_utf8_lossy(&stderr_buf);
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
                }
            }

            let _ = child.wait().await;
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
