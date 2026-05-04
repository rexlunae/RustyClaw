// ── App — clean iocraft TUI ─────────────────────────────────────────────────
//
// Architecture:
//
//   CLI (tokio) ──▶ App::run() ──▶ spawns gateway reader (tokio task)
//                                  spawns iocraft render  (blocking thread)
//
//   Gateway events flow through  std::sync::mpsc::Receiver<GwEvent>
//   User input flows through     std::sync::mpsc::Sender<UserInput>
//
//   The iocraft component owns ALL UI state and runs entirely on smol.
//   No Arc<Mutex<_>> shared state — just channels.

use anyhow::Result;
use std::sync::mpsc as sync_mpsc;

use rustyclaw_core::commands::{CommandAction, CommandContext, CommandResponse, handle_command};
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{
    ChatMessage, ClientFrame, ClientFrameType, ClientPayload, ServerFrame, deserialize_frame,
    serialize_frame,
};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_core::soul::SoulManager;

use crate::gateway_client;

use super::tui_component;
use super::tui_component::TuiRoot;

// ── Channel message types ───────────────────────────────────────────────────

/// Events pushed from the gateway reader into the iocraft render component.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum GwEvent {
    Disconnected(String),
    AuthChallenge,
    Authenticated,
    ModelReady(String),
    /// Gateway reloaded config — update model label in status bar
    ModelReloaded {
        provider: String,
        model: String,
    },
    Info(String),
    Success(String),
    /// A non-fatal warning.  `details` carries the multi-line extended
    /// representation (URL, status, redacted headers, body excerpt,
    /// full error chain) for the TUI's "details" dialog when the
    /// underlying error has structured fields attached.
    Warning {
        summary: String,
        details: Option<String>,
    },
    /// An error.  Same shape as [`Warning`], including optional
    /// extended details.
    Error {
        summary: String,
        details: Option<String>,
    },
    StreamStart,
    Chunk(String),
    ResponseDone,
    ThinkingStart,
    ThinkingDelta,
    ThinkingEnd,
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolResult {
        result: String,
    },
    /// Gateway requests user approval for a tool call (Ask mode)
    ToolApprovalRequest {
        id: String,
        name: String,
        arguments: String,
    },
    /// Gateway requests structured user input (ask_user tool)
    UserPromptRequest(rustyclaw_core::user_prompt_types::UserPrompt),
    /// Vault is locked — user needs to provide password
    VaultLocked,
    /// Vault was successfully unlocked
    VaultUnlocked,
    /// Show secrets info dialog
    ShowSecrets {
        secrets: Vec<crate::components::secrets_dialog::SecretInfo>,
        agent_access: bool,
        has_totp: bool,
    },
    /// Show skills info dialog
    ShowSkills {
        skills: Vec<crate::components::skills_dialog::SkillInfo>,
    },
    /// Show tool permissions info dialog
    ShowToolPerms {
        tools: Vec<crate::components::tool_perms_dialog::ToolPermInfo>,
    },
    /// A secrets mutation succeeded — re-fetch the list from the gateway
    RefreshSecrets,
    /// Thread list update from gateway (unified tasks + threads)
    ThreadsUpdate {
        threads: Vec<crate::action::ThreadInfo>,
        #[allow(dead_code)]
        foreground_id: Option<u64>,
    },
    /// Thread switch confirmed — clear messages and show context
    ThreadSwitched {
        thread_id: u64,
        context_summary: Option<String>,
    },
    /// Hatching identity generated
    HatchingResponse(String),
    /// Open the provider selector dialog
    ShowProviderSelector {
        providers: Vec<String>,
        provider_ids: Vec<String>,
        auth_hints: Vec<String>,
    },
    /// Open the API key input dialog
    PromptApiKey {
        provider: String,
        provider_display: String,
        help_url: String,
        help_text: String,
    },
    /// Show the device flow verification dialog
    DeviceFlowCode {
        provider: String,
        url: String,
        code: String,
    },
    /// Device flow completed — dismiss dialog and store token
    DeviceFlowDone,
    /// Device flow succeeded — store token and proceed to model selection
    DeviceFlowToken {
        provider: String,
        token: String,
    },
    /// Open the model selector dialog
    ShowModelSelector {
        provider: String,
        provider_display: String,
        models: Vec<String>,
    },
    /// Live model IDs loaded for slash-command autocomplete.
    ModelCompletionsLoaded {
        provider: String,
        models: Vec<String>,
    },
    /// Model fetch is in progress (show loading spinner)
    FetchModelsLoading {
        provider: String,
        provider_display: String,
    },
    /// SSH pairing connection succeeded
    PairingSuccess {
        gateway_name: String,
    },
    /// SSH pairing connection failed
    PairingError(String),
}

impl GwEvent {
    /// Warning event with no extended details.
    pub fn warning(summary: impl Into<String>) -> Self {
        GwEvent::Warning {
            summary: summary.into(),
            details: None,
        }
    }
    /// Warning event from an `anyhow_tracing::Error`.  The error's
    /// `Display` form becomes the toast summary (rendered with `{:#}`
    /// to include the cause chain), and the structured fields are
    /// rendered into a multi-line "details" string for the
    /// details-dialog keybind.
    #[allow(dead_code)]
    pub fn warning_from_err(err: &anyhow_tracing::Error) -> Self {
        GwEvent::Warning {
            summary: format!("{:#}", err),
            details: Some(rustyclaw_core::error_details::render_extended(err)),
        }
    }
    /// Error event with no extended details.
    pub fn error(summary: impl Into<String>) -> Self {
        GwEvent::Error {
            summary: summary.into(),
            details: None,
        }
    }
    /// Error event from an `anyhow_tracing::Error`.  See [`Self::warning_from_err`].
    pub fn error_from_err(err: &anyhow_tracing::Error) -> Self {
        GwEvent::Error {
            summary: format!("{:#}", err),
            details: Some(rustyclaw_core::error_details::render_extended(err)),
        }
    }
}

/// Messages from the iocraft render component back to tokio.
#[derive(Debug, Clone)]
pub(crate) enum UserInput {
    Chat(String),
    Command(String),
    AuthResponse(String),
    /// User approved or denied a tool call
    ToolApprovalResponse {
        id: String,
        approved: bool,
    },
    /// User submitted vault password
    VaultUnlock(String),
    /// User responded to a structured prompt
    UserPromptResponse {
        id: String,
        dismissed: bool,
        value: rustyclaw_core::user_prompt_types::PromptResponseValue,
    },
    /// Cancel the active model/tool run.
    CancelCurrentRequest,
    /// Feed back the completed assistant response for conversation history tracking.
    AssistantResponse(String),
    /// Toggle a skill's enabled state
    ToggleSkill {
        name: String,
    },
    /// Cycle a tool's permission level (Allow → Ask → Deny → SkillOnly → Allow)
    CycleToolPermission {
        name: String,
    },
    /// Cycle a secret's access policy (OPEN → ASK → AUTH → SKILL)
    CycleSecretPolicy {
        name: String,
        current_policy: String,
    },
    /// Delete a secret credential
    DeleteSecret {
        name: String,
    },
    /// Add a new secret (API key)
    AddSecret {
        name: String,
        value: String,
    },
    /// Re-request secrets list from gateway (after a mutation)
    RefreshSecrets,
    /// Request current task list from gateway
    RefreshTasks,
    /// Request current thread list from gateway
    RefreshThreads,
    /// Switch to a different thread
    ThreadSwitch(u64),
    /// Create a new thread
    #[allow(dead_code)]
    ThreadCreate(String),
    /// Request identity generation for hatching
    HatchingRequest,
    /// Hatching response received - save to SOUL.md
    HatchingComplete(String),
    /// User selected a provider from the selector dialog
    SelectProvider(String),
    /// User submitted an API key in the dialog
    SubmitApiKey {
        provider: String,
        key: String,
    },
    /// User selected a model from the selector dialog
    SelectModel {
        provider: String,
        model: String,
    },
    /// Load live model IDs for slash-command autocomplete.
    FetchModelCompletions {
        provider: String,
    },
    /// Cancel the current provider-flow dialog
    CancelProviderFlow,
    /// Initiate SSH pairing connection
    PairingConnect {
        host: String,
        port: u16,
        public_key: String,
    },
    Quit,
}

// ── App ─────────────────────────────────────────────────────────────────────

pub struct App {
    config: Config,
    secrets_manager: SecretsManager,
    skill_manager: SkillManager,
    soul_manager: SoulManager,
    deferred_vault_password: Option<String>,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::locked(config.credentials_dir());
        Self::build(config, secrets_manager)
    }

    pub fn with_password(config: Config, password: String) -> Result<Self> {
        let mut app = Self::new(config)?;
        app.deferred_vault_password = Some(password);
        Ok(app)
    }

    pub fn new_locked(config: Config) -> Result<Self> {
        Self::new(config)
    }

    pub fn set_deferred_vault_password(&mut self, password: String) {
        self.deferred_vault_password = Some(password);
    }

    fn build(config: Config, mut secrets_manager: SecretsManager) -> Result<Self> {
        if !config.use_secrets {
            secrets_manager.set_agent_access(false);
        } else {
            secrets_manager.set_agent_access(config.agent_access);
        }

        let skills_dirs = config.skills_dirs();
        let mut skill_manager = SkillManager::with_dirs(skills_dirs);
        let _ = skill_manager.load_skills();

        let soul_path = config.soul_path();
        let mut soul_manager = SoulManager::new(soul_path);
        let _ = soul_manager.load();

        Ok(Self {
            config,
            secrets_manager,
            skill_manager,
            soul_manager,
            deferred_vault_password: None,
        })
    }

    /// Run the TUI — this takes over the terminal.
    pub async fn run(&mut self) -> Result<()> {
        // Apply deferred vault password if one was provided at startup
        if let Some(pw) = self.deferred_vault_password.take() {
            self.secrets_manager.set_password(pw);
        }

        // Channels: gateway → UI
        let (gw_tx, gw_rx) = sync_mpsc::channel::<GwEvent>();
        // Channels: UI → tokio (for sending chat to gateway)
        let (user_tx, user_rx) = sync_mpsc::channel::<UserInput>();

        // ── Gather static info for the component ────────────────────────
        // Use the configured agent_name — no need to parse SOUL.md
        let soul_name = self.config.agent_name.clone();

        // Check if soul needs hatching (first run or default content)
        let needs_hatching = self.soul_manager.needs_hatching();

        let provider = self
            .config
            .model
            .as_ref()
            .map(|m| m.provider.clone())
            .unwrap_or_default();

        let model = self
            .config
            .model
            .as_ref()
            .and_then(|m| m.model.clone())
            .unwrap_or_default();

        let model_label = if provider.is_empty() {
            String::new()
        } else if model.is_empty() {
            provider.clone()
        } else {
            format!("{} / {}", provider, model)
        };

        let gateway_url = self
            .config
            .gateway_url
            .clone()
            .unwrap_or_else(|| "ssh://127.0.0.1:2222".to_string());

        let hint = "Ctrl+C quit · Esc cancel run · /help commands · ↑↓ scroll".to_string();

        // Extract host/port from gateway_url for pre-filling the pairing dialog.
        let (pairing_default_host, pairing_default_port) =
            if let Ok(parsed) = url::Url::parse(&gateway_url) {
                let h = parsed.host_str().unwrap_or("").to_string();
                let p = parsed
                    .port()
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "2222".to_string());
                (h, p)
            } else {
                (String::new(), "2222".to_string())
            };

        // ── Connect to gateway ──────────────────────────────────────────
        let gw_tx_conn = gw_tx.clone();
        let gateway_url_clone = gateway_url.clone();

        // ── Gateway sink abstraction ────────────────────────────────────
        //
        // SSH-only transport: the TUI spawns `ssh user@host
        // rustyclaw-gateway --ssh-stdio` and communicates over the
        // subprocess's stdin/stdout using length-prefixed binary frames.

        enum GatewaySink {
            Ssh(tokio::process::ChildStdin),
        }

        impl GatewaySink {
            async fn send_binary(&mut self, data: Vec<u8>) -> Result<()> {
                match self {
                    GatewaySink::Ssh(stdin) => {
                        use tokio::io::AsyncWriteExt;
                        let len = data.len() as u32;
                        stdin.write_all(&len.to_be_bytes()).await?;
                        stdin.write_all(&data).await?;
                        stdin.flush().await?;
                        Ok(())
                    }
                }
            }
        }

        let (sink_tx, sink_rx) = tokio::sync::oneshot::channel::<GatewaySink>();

        // ── SSH transport ───────────────────────────────────────────
        //
        // Parse ssh://[user@]host[:port] and spawn the system SSH client.
        // The gateway runs in --ssh-stdio mode on the remote end.
        let gw_tx_ssh = gw_tx_conn.clone();
        let _reader_handle = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;

            let parsed = match url::Url::parse(&gateway_url_clone) {
                Ok(u) => u,
                Err(e) => {
                    let _ = gw_tx_ssh.send(GwEvent::error(format!("Invalid SSH URL: {}", e)));
                    let _ = gw_tx_ssh.send(GwEvent::Disconnected(e.to_string()));
                    return;
                }
            };

            let host = parsed.host_str().unwrap_or("localhost").to_string();
            let port = if parsed.port().is_some() {
                parsed.port()
            } else {
                None
            };
            let user = if parsed.username().is_empty() {
                None
            } else {
                Some(parsed.username().to_string())
            };

            // Ensure we have a RustyClaw client identity and use it for SSH auth.
            // This aligns runtime connection auth with the pairing key used by the gateway.
            let client_key_path =
                match rustyclaw_core::pairing::ClientKeyPair::load_or_generate(None) {
                    Ok(_) => rustyclaw_core::pairing::default_client_key_path(),
                    Err(e) => {
                        let _ = gw_tx_ssh.send(GwEvent::error(format!(
                            "Failed to load/generate client key: {}",
                            e
                        )));
                        let _ =
                            gw_tx_ssh.send(GwEvent::Disconnected("SSH auth setup failed".into()));
                        return;
                    }
                };

            // Build the SSH command
            let mut cmd = tokio::process::Command::new("ssh");
            cmd.arg("-T"); // no pseudo-terminal
            cmd.arg("-o").arg("PreferredAuthentications=publickey");
            cmd.arg("-o").arg("IdentitiesOnly=yes");
            cmd.arg("-i").arg(&client_key_path);
            // Use a dedicated known-hosts file so we don't pollute ~/.ssh/known_hosts
            // and can manage gateway fingerprints independently.
            let known_hosts_path = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("rustyclaw")
                .join("known_hosts");
            cmd.arg("-o")
                .arg(format!("UserKnownHostsFile={}", known_hosts_path.display()));
            // Accept new host keys on first connect; reject changed keys.
            // This matches the pairing model: pair once, trust forever.
            cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
            // Never prompt for anything — fail fast instead of blocking the TUI.
            cmd.arg("-o").arg("BatchMode=yes");
            if let Some(p) = port {
                cmd.arg("-p").arg(p.to_string());
            }
            let target = if let Some(u) = &user {
                format!("{}@{}", u, host)
            } else {
                host.clone()
            };
            cmd.arg(&target);
            cmd.arg("rustyclaw-gateway").arg("--ssh-stdio");

            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            match cmd.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take().unwrap();
                    let mut stdout = child.stdout.take().unwrap();
                    let mut stderr = child.stderr.take().unwrap();

                    // Send the write-half to the main loop
                    let _ = sink_tx.send(GatewaySink::Ssh(stdin));

                    // Read length-prefixed frames from stdout
                    let mut len_buf = [0u8; 4];
                    loop {
                        match stdout.read_exact(&mut len_buf).await {
                            Ok(_) => {
                                let len = u32::from_be_bytes(len_buf) as usize;
                                if len > 16 * 1024 * 1024 {
                                    let _ = gw_tx_ssh.send(GwEvent::error("SSH frame too large"));
                                    break;
                                }
                                let mut frame_buf = vec![0u8; len];
                                if stdout.read_exact(&mut frame_buf).await.is_err() {
                                    let _ = gw_tx_ssh
                                        .send(GwEvent::Disconnected("SSH read error".into()));
                                    break;
                                }
                                match deserialize_frame::<ServerFrame>(&frame_buf) {
                                    Ok(frame) => {
                                        let is_model_ready = matches!(
                                            &frame.payload,
                                            rustyclaw_core::gateway::ServerPayload::Status {
                                                status:
                                                    rustyclaw_core::gateway::StatusType::ModelReady,
                                                ..
                                            }
                                        );
                                        if is_model_ready {
                                            if let rustyclaw_core::gateway::ServerPayload::Status { detail, .. } = &frame.payload {
                                                let _ = gw_tx_ssh.send(GwEvent::ModelReady(detail.clone()));
                                            }
                                        } else {
                                            let fa = gateway_client::server_frame_to_action(&frame);
                                            if let Some(action) = fa.action {
                                                let ev = action_to_gw_event(&action);
                                                if let Some(ev) = ev {
                                                    let _ = gw_tx_ssh.send(ev);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = gw_tx_ssh
                                            .send(GwEvent::error(format!("Protocol error: {}", e)));
                                    }
                                }
                            }
                            Err(_) => {
                                // Drain stderr to surface the real SSH error message.
                                use tokio::io::AsyncReadExt as _;
                                let mut stderr_buf = Vec::new();
                                let _ = stderr.read_to_end(&mut stderr_buf).await;
                                let ssh_err = String::from_utf8_lossy(&stderr_buf);
                                let msg = if let Some(line) = ssh_err
                                    .lines()
                                    .map(str::trim)
                                    .filter(|l| !l.is_empty())
                                    .filter(|l| !l.starts_with("**"))
                                    .find(|l| {
                                        l.contains("Permission denied")
                                            || l.contains("Host key verification failed")
                                            || l.contains("Connection refused")
                                            || l.contains("Connection timed out")
                                            || l.contains("No route to host")
                                            || l.contains("Could not resolve hostname")
                                            || l.contains("kex_exchange_identification")
                                    })
                                    .or_else(|| {
                                        ssh_err
                                            .lines()
                                            .map(str::trim)
                                            .rfind(|l| !l.is_empty() && !l.starts_with("**"))
                                    }) {
                                    if line.contains("Permission denied") {
                                        "SSH authentication failed: this client key is not authorized on the gateway. Press Ctrl+P to pair, then retry.".to_string()
                                    } else if line.contains("Host key verification failed") {
                                        "Host key verification failed: the gateway host key changed. Re-pair or update ~/.config/rustyclaw/known_hosts.".to_string()
                                    } else {
                                        line.to_string()
                                    }
                                } else {
                                    "SSH connection closed".to_string()
                                };
                                let _ = gw_tx_ssh.send(GwEvent::Disconnected(msg));
                                break;
                            }
                        }
                    }

                    // Wait for the child process to exit
                    let _ = child.wait().await;
                }
                Err(e) => {
                    drop(sink_tx);
                    let _ = gw_tx_ssh.send(GwEvent::error(format!("Failed to spawn ssh: {}", e)));
                    let _ = gw_tx_ssh.send(GwEvent::Disconnected(e.to_string()));
                }
            }
        });

        // Try to get the write-half.
        let mut gw_sink: Option<GatewaySink> = sink_rx.await.ok();

        // ── Spawn the iocraft render on a blocking thread ───────────────
        // Stash the channels in statics so the component can grab them on
        // first render (via use_const). This avoids ownership issues with
        // iocraft props.
        *tui_component::CHANNEL_RX.lock().unwrap() = Some(gw_rx);
        *tui_component::CHANNEL_TX.lock().unwrap() = Some(user_tx);

        let render_handle = tokio::task::spawn_blocking(move || {
            use iocraft::prelude::*;
            smol::block_on(
                element!(TuiRoot(
                    soul_name: soul_name,
                    model_label: model_label,
                    provider_id: provider.clone(),
                    hint: hint,
                    needs_hatching: needs_hatching,
                    gateway_host: pairing_default_host,
                    gateway_port: pairing_default_port,
                ))
                .fullscreen()
                .disable_mouse_capture(),
            )
        });

        // ── Tokio loop: handle UserInput from UI ────────────────────────
        let mut conversation: Vec<ChatMessage> = Vec::new();
        let config = &mut self.config;
        let secrets_manager = &mut self.secrets_manager;
        let skill_manager = &mut self.skill_manager;

        loop {
            // Poll user_rx (non-blocking on tokio side)
            match user_rx.try_recv() {
                Ok(UserInput::Chat(text)) => {
                    conversation.push(ChatMessage::text("user", &text));
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::Chat,
                            payload: ClientPayload::Chat {
                                messages: conversation.clone(),
                            },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::AuthResponse(code)) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::AuthResponse,
                            payload: ClientPayload::AuthResponse { code },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::ToolApprovalResponse { id, approved }) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ToolApprovalResponse,
                            payload: ClientPayload::ToolApprovalResponse { id, approved },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::VaultUnlock(password)) => {
                    // Unlock locally so /secrets can read the vault
                    secrets_manager.set_password(password.clone());
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::UnlockVault,
                            payload: ClientPayload::UnlockVault { password },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::UserPromptResponse {
                    id,
                    dismissed,
                    value,
                }) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::UserPromptResponse,
                            payload: ClientPayload::UserPromptResponse {
                                id,
                                dismissed,
                                value,
                            },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::CancelCurrentRequest) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::Cancel,
                            payload: ClientPayload::Empty,
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::AssistantResponse(text)) => {
                    // Feed the completed assistant response into the conversation
                    // so subsequent Chat frames include the full history.
                    conversation.push(ChatMessage::text("assistant", &text));
                }
                Ok(UserInput::FetchModelCompletions { provider }) => {
                    let base_url = config.model.as_ref().and_then(|m| m.base_url.clone());
                    let api_key = rustyclaw_core::providers::secret_key_for_provider(&provider)
                        .and_then(|key_name| {
                            secrets_manager
                                .get_secret(key_name, true)
                                .ok()
                                .flatten()
                                .or_else(|| std::env::var(key_name).ok())
                        });
                    let gw_tx2 = gw_tx.clone();
                    tokio::spawn(async move {
                        match rustyclaw_core::providers::fetch_models(
                            &provider,
                            api_key.as_deref(),
                            base_url.as_deref(),
                        )
                        .await
                        {
                            Ok(models) => {
                                let _ = gw_tx2
                                    .send(GwEvent::ModelCompletionsLoaded { provider, models });
                            }
                            Err(e) => {
                                let _ = gw_tx2.send(GwEvent::Warning {
                                    summary: format!("Failed to load model completions: {:#}", e),
                                    details: Some(rustyclaw_core::error_details::render_extended(
                                        &e,
                                    )),
                                });
                            }
                        }
                    });
                }
                Ok(UserInput::Command(cmd)) => {
                    let mut ctx = CommandContext {
                        config,
                        secrets_manager,
                        skill_manager,
                    };
                    let resp: CommandResponse = handle_command(&cmd, &mut ctx);
                    // Send feedback to UI via gateway channel
                    for msg in &resp.messages {
                        let _ = gw_tx.send(GwEvent::Info(msg.clone()));
                    }
                    match resp.action {
                        CommandAction::Quit => break,
                        CommandAction::ShowSecrets => {
                            // Request secrets list from the gateway daemon
                            // (secrets live in the gateway's vault, not locally).
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::SecretsList,
                                    payload: ClientPayload::SecretsList,
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                            }
                        }
                        CommandAction::ShowSkills => {
                            let skills_list: Vec<_> = skill_manager
                                .get_skills()
                                .iter()
                                .map(|s| crate::components::skills_dialog::SkillInfo {
                                    name: s.name.clone(),
                                    description: s.description.clone().unwrap_or_default(),
                                    enabled: s.enabled,
                                })
                                .collect();
                            let _ = gw_tx.send(GwEvent::ShowSkills {
                                skills: skills_list,
                            });
                        }
                        CommandAction::ShowToolPermissions => {
                            let tool_names = rustyclaw_core::tools::all_tool_names();
                            let tools: Vec<_> = tool_names
                                .iter()
                                .map(|name| {
                                    let perm = config
                                        .tool_permissions
                                        .get(*name)
                                        .cloned()
                                        .unwrap_or_default();
                                    crate::components::tool_perms_dialog::ToolPermInfo {
                                        name: name.to_string(),
                                        permission: perm.badge().to_string(),
                                        summary: rustyclaw_core::tools::tool_summary(name)
                                            .to_string(),
                                    }
                                })
                                .collect();
                            let _ = gw_tx.send(GwEvent::ShowToolPerms { tools });
                        }
                        CommandAction::ThreadNew(label) => {
                            // Send thread create to gateway
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadCreate,
                                    payload: ClientPayload::ThreadCreate { label },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                            }
                        }
                        CommandAction::ThreadList => {
                            // Focus sidebar to show threads
                            let _ = gw_tx.send(GwEvent::Info(
                                "Press Tab to focus sidebar and navigate threads.".to_string(),
                            ));
                        }
                        CommandAction::ThreadClose(id) => {
                            // Send thread close to gateway
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadClose,
                                    payload: ClientPayload::ThreadClose { thread_id: id },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                            }
                        }
                        CommandAction::ThreadRename(id, new_label) => {
                            // Send thread rename to gateway
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadRename,
                                    payload: ClientPayload::ThreadRename {
                                        thread_id: id,
                                        new_label,
                                    },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                            }
                        }
                        CommandAction::ThreadBackground => {
                            // Background the current foreground thread by switching
                            // to thread_id 0 (sentinel: no foreground thread).
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadSwitch,
                                    payload: ClientPayload::ThreadSwitch { thread_id: 0 },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                                let _ = gw_tx.send(GwEvent::Info(
                                    "Current thread backgrounded. Use /thread fg <id> or sidebar to switch.".to_string(),
                                ));
                            }
                        }
                        CommandAction::ThreadForeground(id) => {
                            // Foreground a thread by ID — reuse ThreadSwitch
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadSwitch,
                                    payload: ClientPayload::ThreadSwitch { thread_id: id },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                            }
                        }
                        CommandAction::SetModel(model_name) => {
                            // /model only changes the model, never the provider.
                            // The model name is used exactly as entered — on
                            // OpenRouter, IDs like "anthropic/claude-opus-4-20250514"
                            // include a provider prefix that is part of the model ID,
                            // not a directive to switch providers.  Use /provider to
                            // change providers.
                            let existing_provider = config
                                .model
                                .as_ref()
                                .map(|m| m.provider.clone())
                                .unwrap_or_else(|| "openrouter".to_string());

                            // Update config — keep the current provider, only change model
                            config.model = Some(rustyclaw_core::config::ModelProvider {
                                provider: existing_provider,
                                model: Some(model_name.clone()),
                                base_url: config.model.as_ref().and_then(|m| m.base_url.clone()),
                            });

                            // Save config and tell the gateway to reload so the
                            // new model takes effect immediately (no restart needed).
                            if let Err(e) = config.save(None) {
                                let _ = gw_tx
                                    .send(GwEvent::error(format!("Failed to save config: {}", e)));
                            } else {
                                let _ = gw_tx.send(GwEvent::Info(format!(
                                    "Model set to {}. Reloading gateway…",
                                    model_name
                                )));
                                // Send Reload frame so the gateway picks up the new config
                                if let Some(ref mut sink) = gw_sink {
                                    let frame = ClientFrame {
                                        frame_type: ClientFrameType::Reload,
                                        payload: ClientPayload::Reload,
                                    };
                                    if let Ok(data) = serialize_frame(&frame) {
                                        let _ = sink.send_binary(data).await;
                                    }
                                }
                            }
                        }
                        CommandAction::SetProvider(provider_name) => {
                            // Update config with new provider, keep existing model
                            let existing_model =
                                config.model.as_ref().and_then(|m| m.model.clone());
                            config.model = Some(rustyclaw_core::config::ModelProvider {
                                provider: provider_name.clone(),
                                model: existing_model,
                                base_url: config.model.as_ref().and_then(|m| m.base_url.clone()),
                            });

                            // Save config and tell the gateway to reload
                            if let Err(e) = config.save(None) {
                                let _ = gw_tx
                                    .send(GwEvent::error(format!("Failed to save config: {}", e)));
                            } else {
                                let _ = gw_tx.send(GwEvent::Info(format!(
                                    "Provider set to {}. Reloading gateway…",
                                    provider_name
                                )));
                                if let Some(ref mut sink) = gw_sink {
                                    let frame = ClientFrame {
                                        frame_type: ClientFrameType::Reload,
                                        payload: ClientPayload::Reload,
                                    };
                                    if let Ok(data) = serialize_frame(&frame) {
                                        let _ = sink.send_binary(data).await;
                                    }
                                }
                            }
                        }
                        CommandAction::GatewayReload => {
                            // Send Reload frame to the gateway
                            if let Some(ref mut sink) = gw_sink {
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::Reload,
                                    payload: ClientPayload::Reload,
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink.send_binary(data).await;
                                }
                            }
                        }
                        CommandAction::FetchModels => {
                            // Spawn an async task to fetch the live model list
                            // from the provider API and send results back via
                            // the GwEvent channel.
                            let provider_id = config
                                .model
                                .as_ref()
                                .map(|m| m.provider.clone())
                                .unwrap_or_default();
                            let base_url = config.model.as_ref().and_then(|m| m.base_url.clone());
                            // Read the API key: try the encrypted vault first
                            // (where onboarding stores it), then fall back to
                            // environment variables.
                            let api_key =
                                rustyclaw_core::providers::secret_key_for_provider(&provider_id)
                                    .and_then(|key_name| {
                                        secrets_manager
                                            .get_secret(key_name, true)
                                            .ok()
                                            .flatten()
                                            .or_else(|| std::env::var(key_name).ok())
                                    });

                            let gw_tx2 = gw_tx.clone();
                            tokio::spawn(async move {
                                match rustyclaw_core::providers::fetch_models_detailed(
                                    &provider_id,
                                    api_key.as_deref(),
                                    base_url.as_deref(),
                                )
                                .await
                                {
                                    Ok(models) => {
                                        let count = models.len();
                                        let display =
                                            rustyclaw_core::providers::display_name_for_provider(
                                                &provider_id,
                                            );
                                        let _ = gw_tx2.send(GwEvent::Info(format!(
                                            "{} models from {}:",
                                            count, display,
                                        )));
                                        // Show models in batches to avoid
                                        // flooding the channel.
                                        let lines: Vec<String> =
                                            models.iter().map(|m| m.display_line()).collect();
                                        for chunk in lines.chunks(20) {
                                            let _ = gw_tx2.send(GwEvent::Info(chunk.join("\n")));
                                        }
                                        let _ = gw_tx2.send(GwEvent::Info(
                                            "Tip: /model <id> to switch".to_string(),
                                        ));
                                    }
                                    Err(e) => {
                                        let _ = gw_tx2.send(GwEvent::error_from_err(&e));
                                    }
                                }
                            });
                        }
                        CommandAction::ShowProviderSelector => {
                            // Build the provider list and send it to the UI
                            let providers: Vec<String> = rustyclaw_core::providers::PROVIDERS
                                .iter()
                                .map(|p| p.display.to_string())
                                .collect();
                            let ids: Vec<String> = rustyclaw_core::providers::PROVIDERS
                                .iter()
                                .map(|p| p.id.to_string())
                                .collect();
                            let hints: Vec<String> = rustyclaw_core::providers::PROVIDERS
                                .iter()
                                .map(|p| match p.auth_method {
                                    rustyclaw_core::providers::AuthMethod::ApiKey => {
                                        "apikey".to_string()
                                    }
                                    rustyclaw_core::providers::AuthMethod::DeviceFlow => {
                                        "deviceflow".to_string()
                                    }
                                    rustyclaw_core::providers::AuthMethod::None => {
                                        "none".to_string()
                                    }
                                })
                                .collect();
                            let _ = gw_tx.send(GwEvent::ShowProviderSelector {
                                providers,
                                provider_ids: ids,
                                auth_hints: hints,
                            });
                        }
                        _ => {}
                    }
                }
                Ok(UserInput::ToggleSkill { name }) => {
                    if let Some(skill) = skill_manager.get_skills().iter().find(|s| s.name == name)
                    {
                        let new_enabled = !skill.enabled;
                        let _ = skill_manager.set_skill_enabled(&name, new_enabled);
                        // Re-send updated skills list
                        let skills_list: Vec<_> = skill_manager
                            .get_skills()
                            .iter()
                            .map(|s| crate::components::skills_dialog::SkillInfo {
                                name: s.name.clone(),
                                description: s.description.clone().unwrap_or_default(),
                                enabled: s.enabled,
                            })
                            .collect();
                        let _ = gw_tx.send(GwEvent::ShowSkills {
                            skills: skills_list,
                        });
                    }
                }
                Ok(UserInput::CycleToolPermission { name }) => {
                    let current = config
                        .tool_permissions
                        .get(&name)
                        .cloned()
                        .unwrap_or_default();
                    let next = current.cycle();
                    config.tool_permissions.insert(name.clone(), next);
                    let _ = config.save(None);
                    // Re-send updated tool perms list
                    let tool_names = rustyclaw_core::tools::all_tool_names();
                    let tools: Vec<_> = tool_names
                        .iter()
                        .map(|tn| {
                            let perm = config
                                .tool_permissions
                                .get(*tn)
                                .cloned()
                                .unwrap_or_default();
                            crate::components::tool_perms_dialog::ToolPermInfo {
                                name: tn.to_string(),
                                permission: perm.badge().to_string(),
                                summary: rustyclaw_core::tools::tool_summary(tn).to_string(),
                            }
                        })
                        .collect();
                    let _ = gw_tx.send(GwEvent::ShowToolPerms { tools });
                }
                Ok(UserInput::CycleSecretPolicy {
                    name,
                    current_policy,
                }) => {
                    // Cycle OPEN → ASK → AUTH → SKILL → OPEN
                    let next_policy = match current_policy.as_str() {
                        "OPEN" => "ask",
                        "ASK" => "auth",
                        "AUTH" => "skill_only",
                        "SKILL" => "always",
                        _ => "ask",
                    };
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsSetPolicy,
                            payload: ClientPayload::SecretsSetPolicy {
                                name,
                                policy: next_policy.to_string(),
                                skills: vec![],
                            },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::DeleteSecret { name }) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsDeleteCredential,
                            payload: ClientPayload::SecretsDeleteCredential { name },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::AddSecret { name, value }) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsStore,
                            payload: ClientPayload::SecretsStore { key: name, value },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::RefreshSecrets) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsList,
                            payload: ClientPayload::SecretsList,
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::RefreshTasks) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::TasksRequest,
                            payload: ClientPayload::TasksRequest { session: None },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::ThreadSwitch(thread_id)) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ThreadSwitch,
                            payload: ClientPayload::ThreadSwitch { thread_id },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::RefreshThreads) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ThreadList,
                            payload: ClientPayload::ThreadList,
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::ThreadCreate(label)) => {
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ThreadCreate,
                            payload: ClientPayload::ThreadCreate { label },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::HatchingRequest) => {
                    // Send hatching prompt to gateway as a special chat
                    if let Some(ref mut sink) = gw_sink {
                        let hatching_prompt = crate::components::hatching_dialog::HATCHING_PROMPT;
                        let messages = vec![
                            ChatMessage::text("system", hatching_prompt),
                            ChatMessage::text("user", "Generate my identity."),
                        ];
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::Chat,
                            payload: ClientPayload::Chat { messages },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                }
                Ok(UserInput::HatchingComplete(identity)) => {
                    // Save identity to SOUL.md
                    let soul_path = config.soul_path();
                    if let Err(e) = std::fs::write(&soul_path, &identity) {
                        tracing::warn!("Failed to write SOUL.md: {}", e);
                    } else {
                        tracing::info!("Saved hatched identity to {:?}", soul_path);
                    }
                }
                Ok(UserInput::SelectProvider(provider_id)) => {
                    // User picked a provider from the selector dialog.
                    // Check if auth is needed and route accordingly.
                    let def = rustyclaw_core::providers::provider_by_id(&provider_id);
                    if let Some(def) = def {
                        match def.auth_method {
                            rustyclaw_core::providers::AuthMethod::None => {
                                // No auth needed — go straight to model fetch.
                                // Update config first.
                                let existing_model =
                                    config.model.as_ref().and_then(|m| m.model.clone());
                                config.model = Some(rustyclaw_core::config::ModelProvider {
                                    provider: provider_id.clone(),
                                    model: existing_model,
                                    base_url: config
                                        .model
                                        .as_ref()
                                        .and_then(|m| m.base_url.clone()),
                                });
                                let _ = config.save(None);
                                // Reload gateway
                                if let Some(ref mut sink) = gw_sink {
                                    let frame = ClientFrame {
                                        frame_type: ClientFrameType::Reload,
                                        payload: ClientPayload::Reload,
                                    };
                                    if let Ok(data) = serialize_frame(&frame) {
                                        let _ = sink.send_binary(data).await;
                                    }
                                }
                                // Trigger model selector (show loading)
                                let display = def.display.to_string();
                                let pid = provider_id.clone();
                                let _ = gw_tx.send(GwEvent::FetchModelsLoading {
                                    provider: pid.clone(),
                                    provider_display: display.clone(),
                                });
                                let gw_tx2 = gw_tx.clone();
                                let base = config.model.as_ref().and_then(|m| m.base_url.clone());
                                tokio::spawn(async move {
                                    match rustyclaw_core::providers::fetch_models(
                                        &pid,
                                        None,
                                        base.as_deref(),
                                    )
                                    .await
                                    {
                                        Ok(models) => {
                                            let _ = gw_tx2.send(GwEvent::ShowModelSelector {
                                                provider: pid,
                                                provider_display: display,
                                                models,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to fetch models: {:#}", e),
                                                details: Some(
                                                    rustyclaw_core::error_details::render_extended(
                                                        &e,
                                                    ),
                                                ),
                                            });
                                        }
                                    }
                                });
                            }
                            rustyclaw_core::providers::AuthMethod::ApiKey => {
                                // Check if we already have a key stored
                                let has_key = def.secret_key.and_then(|sk| {
                                    secrets_manager
                                        .get_secret(sk, true)
                                        .ok()
                                        .flatten()
                                        .or_else(|| std::env::var(sk).ok())
                                });
                                if has_key.is_some() {
                                    // Key exists — set provider and fetch models
                                    let existing_model =
                                        config.model.as_ref().and_then(|m| m.model.clone());
                                    config.model = Some(rustyclaw_core::config::ModelProvider {
                                        provider: provider_id.clone(),
                                        model: existing_model,
                                        base_url: config
                                            .model
                                            .as_ref()
                                            .and_then(|m| m.base_url.clone()),
                                    });
                                    let _ = config.save(None);
                                    if let Some(ref mut sink) = gw_sink {
                                        let frame = ClientFrame {
                                            frame_type: ClientFrameType::Reload,
                                            payload: ClientPayload::Reload,
                                        };
                                        if let Ok(data) = serialize_frame(&frame) {
                                            let _ = sink.send_binary(data).await;
                                        }
                                    }
                                    let display = def.display.to_string();
                                    let pid = provider_id.clone();
                                    let key = has_key;
                                    let _ = gw_tx.send(GwEvent::FetchModelsLoading {
                                        provider: pid.clone(),
                                        provider_display: display.clone(),
                                    });
                                    let gw_tx2 = gw_tx.clone();
                                    let base =
                                        config.model.as_ref().and_then(|m| m.base_url.clone());
                                    tokio::spawn(async move {
                                        match rustyclaw_core::providers::fetch_models(
                                            &pid,
                                            key.as_deref(),
                                            base.as_deref(),
                                        )
                                        .await
                                        {
                                            Ok(models) => {
                                                let _ = gw_tx2.send(GwEvent::ShowModelSelector {
                                                    provider: pid,
                                                    provider_display: display,
                                                    models,
                                                });
                                            }
                                            Err(e) => {
                                                let _ = gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to fetch models: {:#}", e),
                                                details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                            });
                                            }
                                        }
                                    });
                                } else {
                                    // No key — prompt for one
                                    let _ = gw_tx.send(GwEvent::PromptApiKey {
                                        provider: provider_id.clone(),
                                        provider_display: def.display.to_string(),
                                        help_url: def.help_url.unwrap_or("").to_string(),
                                        help_text: def.help_text.unwrap_or("").to_string(),
                                    });
                                }
                            }
                            rustyclaw_core::providers::AuthMethod::DeviceFlow => {
                                // Check if we already have a token stored
                                let has_token = def.secret_key.and_then(|sk| {
                                    secrets_manager
                                        .get_secret(sk, true)
                                        .ok()
                                        .flatten()
                                        .or_else(|| std::env::var(sk).ok())
                                });
                                if has_token.is_some() {
                                    // Token exists — set provider and fetch models
                                    let existing_model =
                                        config.model.as_ref().and_then(|m| m.model.clone());
                                    config.model = Some(rustyclaw_core::config::ModelProvider {
                                        provider: provider_id.clone(),
                                        model: existing_model,
                                        base_url: config
                                            .model
                                            .as_ref()
                                            .and_then(|m| m.base_url.clone()),
                                    });
                                    let _ = config.save(None);
                                    if let Some(ref mut sink) = gw_sink {
                                        let frame = ClientFrame {
                                            frame_type: ClientFrameType::Reload,
                                            payload: ClientPayload::Reload,
                                        };
                                        if let Ok(data) = serialize_frame(&frame) {
                                            let _ = sink.send_binary(data).await;
                                        }
                                    }
                                    let display = def.display.to_string();
                                    let pid = provider_id.clone();
                                    let token = has_token;
                                    let _ = gw_tx.send(GwEvent::FetchModelsLoading {
                                        provider: pid.clone(),
                                        provider_display: display.clone(),
                                    });
                                    let gw_tx2 = gw_tx.clone();
                                    let base =
                                        config.model.as_ref().and_then(|m| m.base_url.clone());
                                    tokio::spawn(async move {
                                        match rustyclaw_core::providers::fetch_models(
                                            &pid,
                                            token.as_deref(),
                                            base.as_deref(),
                                        )
                                        .await
                                        {
                                            Ok(models) => {
                                                let _ = gw_tx2.send(GwEvent::ShowModelSelector {
                                                    provider: pid,
                                                    provider_display: display,
                                                    models,
                                                });
                                            }
                                            Err(e) => {
                                                let _ = gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to fetch models: {:#}", e),
                                                details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                            });
                                            }
                                        }
                                    });
                                } else {
                                    // No token — start device flow
                                    if let Some(df_config) = def.device_flow {
                                        let pid = provider_id.clone();
                                        let display = def.display.to_string();
                                        let gw_tx2 = gw_tx.clone();
                                        let _ = gw_tx.send(GwEvent::Info(format!(
                                            "Starting device flow for {}…",
                                            display
                                        )));
                                        tokio::spawn(async move {
                                            match rustyclaw_core::providers::start_device_flow(
                                                df_config,
                                            )
                                            .await
                                            {
                                                Ok(auth_resp) => {
                                                    let _ = gw_tx2.send(GwEvent::DeviceFlowCode {
                                                        provider: pid.clone(),
                                                        url: auth_resp.verification_uri.clone(),
                                                        code: auth_resp.user_code.clone(),
                                                    });
                                                    // Poll for the token with the interval from the response
                                                    let interval = std::time::Duration::from_secs(
                                                        auth_resp.interval.max(5),
                                                    );
                                                    let deadline = tokio::time::Instant::now()
                                                        + std::time::Duration::from_secs(
                                                            auth_resp.expires_in,
                                                        );
                                                    loop {
                                                        tokio::time::sleep(interval).await;
                                                        if tokio::time::Instant::now() >= deadline {
                                                            let _ = gw_tx2
                                                                .send(GwEvent::DeviceFlowDone);
                                                            let _ = gw_tx2.send(GwEvent::error(
                                                                "Device flow timed out — please try again.".to_string(),
                                                            ));
                                                            break;
                                                        }
                                                        match rustyclaw_core::providers::poll_device_token(
                                                            df_config, &auth_resp.device_code,
                                                        ).await {
                                                            Ok(Some(token)) => {
                                                                let _ = gw_tx2.send(GwEvent::DeviceFlowDone);
                                                                let _ = gw_tx2.send(GwEvent::Success(format!(
                                                                    "✓ {} authenticated!", display
                                                                )));
                                                                let _ = gw_tx2.send(GwEvent::DeviceFlowToken {
                                                                    provider: pid.clone(),
                                                                    token,
                                                                });
                                                                break;
                                                            }
                                                            Ok(None) => {
                                                                // Still pending — continue polling
                                                            }
                                                            Err(e) => {
                                                                let _ = gw_tx2.send(GwEvent::DeviceFlowDone);
                                                                let _ = gw_tx2.send(GwEvent::Error {
                                                                    summary: format!("Device flow failed: {:#}", e),
                                                                    details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                                                });
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    let _ = gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to start device flow: {:#}", e),
                                                details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                            });
                                                }
                                            }
                                        });
                                    } else {
                                        let _ = gw_tx.send(GwEvent::error(
                                            "Device flow not configured for this provider."
                                                .to_string(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(UserInput::SubmitApiKey { provider, key }) => {
                    // Store the API key in the secrets vault
                    let secret_key_name =
                        rustyclaw_core::providers::secret_key_for_provider(&provider)
                            .unwrap_or("API_KEY");
                    let display =
                        rustyclaw_core::providers::display_name_for_provider(&provider).to_string();
                    match secrets_manager.store_secret(secret_key_name, &key) {
                        Ok(()) => {
                            let _ = gw_tx.send(GwEvent::Success(format!(
                                "✓ API key for {} stored securely.",
                                display,
                            )));
                        }
                        Err(e) => {
                            let _ = gw_tx.send(GwEvent::warning(format!(
                                "Failed to store API key: {}. Key is set for this session only.",
                                e,
                            )));
                        }
                    }
                    // Update config with the new provider
                    let existing_model = config.model.as_ref().and_then(|m| m.model.clone());
                    config.model = Some(rustyclaw_core::config::ModelProvider {
                        provider: provider.clone(),
                        model: existing_model,
                        base_url: config.model.as_ref().and_then(|m| m.base_url.clone()),
                    });
                    let _ = config.save(None);
                    // Reload gateway
                    if let Some(ref mut sink) = gw_sink {
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::Reload,
                            payload: ClientPayload::Reload,
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink.send_binary(data).await;
                        }
                    }
                    // Now fetch models
                    let pid = provider.clone();
                    let _ = gw_tx.send(GwEvent::FetchModelsLoading {
                        provider: pid.clone(),
                        provider_display: display.clone(),
                    });
                    let gw_tx2 = gw_tx.clone();
                    let api_key = Some(key);
                    let base = config.model.as_ref().and_then(|m| m.base_url.clone());
                    tokio::spawn(async move {
                        match rustyclaw_core::providers::fetch_models(
                            &pid,
                            api_key.as_deref(),
                            base.as_deref(),
                        )
                        .await
                        {
                            Ok(models) => {
                                let _ = gw_tx2.send(GwEvent::ShowModelSelector {
                                    provider: pid,
                                    provider_display: display,
                                    models,
                                });
                            }
                            Err(e) => {
                                let _ = gw_tx2.send(GwEvent::Error {
                                    summary: format!("Failed to fetch models: {:#}", e),
                                    details: Some(rustyclaw_core::error_details::render_extended(
                                        &e,
                                    )),
                                });
                            }
                        }
                    });
                }
                Ok(UserInput::SelectModel { provider, model }) => {
                    // Update config with the selected model
                    config.model = Some(rustyclaw_core::config::ModelProvider {
                        provider: provider.clone(),
                        model: Some(model.clone()),
                        base_url: config.model.as_ref().and_then(|m| m.base_url.clone()),
                    });
                    if let Err(e) = config.save(None) {
                        let _ = gw_tx.send(GwEvent::error(format!("Failed to save config: {}", e)));
                    } else {
                        let display =
                            rustyclaw_core::providers::display_name_for_provider(&provider);
                        let _ = gw_tx.send(GwEvent::Info(format!(
                            "Model set to {} / {}. Reloading gateway…",
                            display, model,
                        )));
                        // Reload gateway so the new provider + model take effect
                        if let Some(ref mut sink) = gw_sink {
                            let frame = ClientFrame {
                                frame_type: ClientFrameType::Reload,
                                payload: ClientPayload::Reload,
                            };
                            if let Ok(data) = serialize_frame(&frame) {
                                let _ = sink.send_binary(data).await;
                            }
                        }
                    }
                }
                Ok(UserInput::CancelProviderFlow) => {
                    // User cancelled — nothing to do
                }
                #[allow(unused_variables)]
                Ok(UserInput::PairingConnect {
                    host,
                    port,
                    public_key,
                }) => {
                    // Initiate SSH connection for pairing
                    let gw_tx_pair = gw_tx.clone();
                    tokio::spawn(async move {
                        match crate::pairing::connect_and_pair(&host, port, &public_key).await {
                            Ok(gateway_name) => {
                                let _ = gw_tx_pair.send(GwEvent::PairingSuccess { gateway_name });
                            }
                            Err(e) => {
                                let _ = gw_tx_pair.send(GwEvent::PairingError(e.to_string()));
                            }
                        }
                    });
                }
                Ok(UserInput::Quit) => break,
                Err(sync_mpsc::TryRecvError::Empty) => {}
                Err(sync_mpsc::TryRecvError::Disconnected) => break,
            }

            // Small sleep to avoid busy-spinning
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
        }

        // Wait for render thread to finish
        let _ = render_handle.await;
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Map an Action enum value to a GwEvent.
///
/// Every Action that `server_frame_to_action()` can produce MUST be handled
/// here — either with a dedicated GwEvent variant or by converting to an
/// Info/Success/Warning/Error message so the user always sees feedback.
fn action_to_gw_event(action: &crate::action::Action) -> Option<GwEvent> {
    use crate::action::Action;
    match action {
        // ── Gateway lifecycle ───────────────────────────────────────────
        Action::GatewayAuthChallenge => Some(GwEvent::AuthChallenge),
        Action::GatewayAuthenticated => Some(GwEvent::Authenticated),
        Action::GatewayDisconnected(s) => Some(GwEvent::Disconnected(s.clone())),
        Action::GatewayVaultLocked => Some(GwEvent::VaultLocked),
        Action::GatewayVaultUnlocked => Some(GwEvent::VaultUnlocked),
        Action::GatewayReloaded { provider, model } => Some(GwEvent::ModelReloaded {
            provider: provider.clone(),
            model: model.clone(),
        }),

        // ── Streaming ───────────────────────────────────────────────────
        Action::GatewayStreamStart => Some(GwEvent::StreamStart),
        Action::GatewayChunk(t) => Some(GwEvent::Chunk(t.clone())),
        Action::GatewayResponseDone => Some(GwEvent::ResponseDone),
        Action::GatewayThinkingStart => Some(GwEvent::ThinkingStart),
        Action::GatewayThinkingDelta => Some(GwEvent::ThinkingDelta),
        Action::GatewayThinkingEnd => Some(GwEvent::ThinkingEnd),

        // ── Tool calls and results ──────────────────────────────────────
        Action::GatewayToolCall {
            name, arguments, ..
        } => Some(GwEvent::ToolCall {
            name: name.clone(),
            arguments: arguments.clone(),
        }),
        Action::GatewayToolResult { result, .. } => Some(GwEvent::ToolResult {
            result: result.clone(),
        }),

        // ── Interactive: tool approval ──────────────────────────────────
        Action::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => Some(GwEvent::ToolApprovalRequest {
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        }),

        // ── Interactive: user prompt ────────────────────────────────────
        Action::UserPromptRequest(prompt) => Some(GwEvent::UserPromptRequest(prompt.clone())),

        // ── Tasks ───────────────────────────────────────────────────────

        // ── Threads ─────────────────────────────────────────────────────
        Action::ThreadsUpdate {
            threads,
            foreground_id,
        } => Some(GwEvent::ThreadsUpdate {
            threads: threads.clone(),
            foreground_id: *foreground_id,
        }),
        Action::ThreadSwitched {
            thread_id,
            context_summary,
        } => Some(GwEvent::ThreadSwitched {
            thread_id: *thread_id,
            context_summary: context_summary.clone(),
        }),

        // ── Generic messages ────────────────────────────────────────────
        Action::Info(s) => Some(GwEvent::Info(s.clone())),
        Action::Success(s) => Some(GwEvent::Success(s.clone())),
        Action::Warning(s) => Some(GwEvent::warning(s.clone())),
        Action::Error(s) => Some(GwEvent::error(s.clone())),

        // ── Secrets results — show as info/success/error messages ───────
        Action::SecretsListResult { entries } => {
            let secrets: Vec<crate::components::secrets_dialog::SecretInfo> = entries
                .iter()
                .map(|e| crate::components::secrets_dialog::SecretInfo {
                    name: e.name.clone(),
                    label: e.label.clone(),
                    kind: e.kind.clone(),
                    policy: e.policy.clone(),
                    disabled: e.disabled,
                })
                .collect();
            Some(GwEvent::ShowSecrets {
                secrets,
                agent_access: false,
                has_totp: false,
            })
        }
        Action::SecretsStoreResult { ok, message } => {
            if *ok {
                Some(GwEvent::RefreshSecrets)
            } else {
                Some(GwEvent::error(format!(
                    "Failed to store secret: {}",
                    message
                )))
            }
        }
        Action::SecretsGetResult { key, value } => {
            let display = value.as_deref().unwrap_or("(not found)");
            Some(GwEvent::Info(format!("Secret [{}]: {}", key, display)))
        }
        Action::SecretsPeekResult {
            name,
            ok,
            fields,
            message,
        } => {
            if *ok {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("  {}: {}", k, v))
                    .collect();
                Some(GwEvent::Info(format!(
                    "Credential [{}]:\n{}",
                    name,
                    field_strs.join("\n")
                )))
            } else {
                Some(GwEvent::error(
                    message
                        .clone()
                        .unwrap_or_else(|| format!("Failed to peek {}", name)),
                ))
            }
        }
        Action::SecretsSetPolicyResult { ok, message } => {
            if *ok {
                Some(GwEvent::RefreshSecrets)
            } else {
                Some(GwEvent::error(
                    message
                        .clone()
                        .unwrap_or_else(|| "Failed to update policy".into()),
                ))
            }
        }
        Action::SecretsSetDisabledResult {
            ok,
            cred_name,
            disabled,
        } => {
            let action_word = if *disabled { "disabled" } else { "enabled" };
            if *ok {
                Some(GwEvent::Success(format!(
                    "Credential {} {}",
                    cred_name, action_word
                )))
            } else {
                Some(GwEvent::error(format!(
                    "Failed to {} credential {}",
                    action_word, cred_name
                )))
            }
        }
        Action::SecretsDeleteCredentialResult { ok, cred_name } => {
            if *ok {
                Some(GwEvent::RefreshSecrets)
            } else {
                Some(GwEvent::error(format!(
                    "Failed to delete credential {}",
                    cred_name
                )))
            }
        }
        Action::SecretsHasTotpResult { has_totp } => Some(GwEvent::Info(if *has_totp {
            "TOTP is configured".into()
        } else {
            "TOTP is not configured".into()
        })),
        Action::SecretsSetupTotpResult { ok, uri, message } => {
            if *ok {
                Some(GwEvent::Success(format!(
                    "TOTP setup complete{}",
                    uri.as_ref()
                        .map(|u| format!(" — URI: {}", u))
                        .unwrap_or_default()
                )))
            } else {
                Some(GwEvent::error(
                    message
                        .clone()
                        .unwrap_or_else(|| "TOTP setup failed".into()),
                ))
            }
        }
        Action::SecretsVerifyTotpResult { ok } => {
            if *ok {
                Some(GwEvent::Success("TOTP verified".into()))
            } else {
                Some(GwEvent::error("TOTP verification failed"))
            }
        }
        Action::SecretsRemoveTotpResult { ok } => {
            if *ok {
                Some(GwEvent::Success("TOTP removed".into()))
            } else {
                Some(GwEvent::error("TOTP removal failed"))
            }
        }

        // ── Actions that are UI-only (no gateway frame) — show if relevant ──
        Action::ToolCommandDone { message, is_error } => {
            if *is_error {
                Some(GwEvent::error(message.clone()))
            } else {
                Some(GwEvent::Success(message.clone()))
            }
        }

        // ── Actions that the TUI doesn't originate from gateway ─────────
        // These are internal UI or CLI-only actions. If they somehow arrive
        // here, show them so nothing is ever silent.
        Action::HatchingResponse(s) => Some(GwEvent::Info(format!("Hatching: {}", s))),
        Action::FinishHatching(s) => Some(GwEvent::Success(format!("Hatching complete: {}", s))),

        // ── Catch-all: NEVER silently drop ──────────────────────────────
        // Any action not explicitly handled above is shown as a warning
        // so the user always knows something happened.
        other => Some(GwEvent::warning(format!("Unhandled event: {}", other))),
    }
}
