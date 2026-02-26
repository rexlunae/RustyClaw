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

// ── Channel message types ───────────────────────────────────────────────────

/// Events pushed from the gateway reader into the iocraft render component.
#[derive(Debug, Clone)]
pub(crate) enum GwEvent {
    Disconnected(String),
    AuthChallenge,
    Authenticated,
    ModelReady(String),
    Info(String),
    Success(String),
    Warning(String),
    Error(String),
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
        let soul_name = self
            .soul_manager
            .get_content()
            .and_then(|c: &str| {
                c.lines()
                    .find(|l: &&str| l.starts_with("# "))
                    .map(|l: &str| l.trim_start_matches("# ").to_string())
            })
            .unwrap_or_else(|| "RustyClaw".to_string());

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
            .unwrap_or_else(|| "ws://127.0.0.1:9001".to_string());

        let hint = "Ctrl+C quit · /help commands · ↑↓ scroll".to_string();

        // ── Connect to gateway ──────────────────────────────────────────
        let gw_tx_conn = gw_tx.clone();
        let gateway_url_clone = gateway_url.clone();

        // Use a oneshot for the write-half of the WS connection.
        type WsSink = futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            tokio_tungstenite::tungstenite::Message,
        >;

        let (sink_tx, sink_rx) = tokio::sync::oneshot::channel::<WsSink>();

        let _reader_handle = tokio::spawn(async move {
            use futures_util::StreamExt;
            use tokio_tungstenite::connect_async;

            match connect_async(&gateway_url_clone).await {
                Ok((ws, _)) => {
                    let (write, mut read) = StreamExt::split(ws);
                    let _ = sink_tx.send(write);
                    // Don't report Connected yet — wait for auth flow.
                    // The gateway will send AuthChallenge or Hello+Status frames.

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                                match deserialize_frame::<ServerFrame>(&data) {
                                    Ok(frame) => {
                                        // Check for ModelReady status before action conversion
                                        // since it maps to a generic Success action otherwise.
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
                                            let _ = gw_tx_conn.send(GwEvent::ModelReady(detail.clone()));
                                        }
                                        } else {
                                            let fa = gateway_client::server_frame_to_action(&frame);
                                            if let Some(action) = fa.action {
                                                let ev = action_to_gw_event(&action);
                                                if let Some(ev) = ev {
                                                    let _ = gw_tx_conn.send(ev);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[rustyclaw] Failed to deserialize server frame ({} bytes): {}",
                                            data.len(),
                                            e
                                        );
                                        let _ = gw_tx_conn.send(GwEvent::Error(format!(
                                            "Protocol error: failed to deserialize frame ({}). Gateway/TUI version mismatch?",
                                            e
                                        )));
                                    }
                                }
                            }
                            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                                let _ = gw_tx_conn.send(GwEvent::Disconnected("closed".into()));
                                break;
                            }
                            Err(e) => {
                                let _ = gw_tx_conn.send(GwEvent::Disconnected(e.to_string()));
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    drop(sink_tx);
                    let _ = gw_tx_conn
                        .send(GwEvent::Error(format!("Gateway connection failed: {}", e)));
                    let _ = gw_tx_conn.send(GwEvent::Disconnected(e.to_string()));
                }
            }
        });

        // Try to get the write-half.
        let mut ws_sink: Option<WsSink> = match sink_rx.await {
            Ok(s) => Some(s),
            Err(_) => None,
        };

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
                    hint: hint,
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
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::Chat,
                            payload: ClientPayload::Chat {
                                messages: conversation.clone(),
                            },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::AuthResponse(code)) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::AuthResponse,
                            payload: ClientPayload::AuthResponse { code },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::ToolApprovalResponse { id, approved }) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ToolApprovalResponse,
                            payload: ClientPayload::ToolApprovalResponse { id, approved },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::VaultUnlock(password)) => {
                    // Unlock locally so /secrets can read the vault
                    secrets_manager.set_password(password.clone());
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::UnlockVault,
                            payload: ClientPayload::UnlockVault { password },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::UserPromptResponse {
                    id,
                    dismissed,
                    value,
                }) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::UserPromptResponse,
                            payload: ClientPayload::UserPromptResponse {
                                id,
                                dismissed,
                                value,
                            },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::AssistantResponse(text)) => {
                    // Feed the completed assistant response into the conversation
                    // so subsequent Chat frames include the full history.
                    conversation.push(ChatMessage::text("assistant", &text));
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
                            if let Some(ref mut sink) = ws_sink {
                                use futures_util::SinkExt;
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::SecretsList,
                                    payload: ClientPayload::SecretsList,
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink
                                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                                            data.into(),
                                        ))
                                        .await;
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
                            if let Some(ref mut sink) = ws_sink {
                                use futures_util::SinkExt;
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadCreate,
                                    payload: ClientPayload::ThreadCreate { label },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink
                                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                                            data.into(),
                                        ))
                                        .await;
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
                            if let Some(ref mut sink) = ws_sink {
                                use futures_util::SinkExt;
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadClose,
                                    payload: ClientPayload::ThreadClose { thread_id: id },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink
                                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                                            data.into(),
                                        ))
                                        .await;
                                }
                            }
                        }
                        CommandAction::ThreadRename(id, new_label) => {
                            // Send thread rename to gateway
                            if let Some(ref mut sink) = ws_sink {
                                use futures_util::SinkExt;
                                let frame = ClientFrame {
                                    frame_type: ClientFrameType::ThreadRename,
                                    payload: ClientPayload::ThreadRename {
                                        thread_id: id,
                                        new_label,
                                    },
                                };
                                if let Ok(data) = serialize_frame(&frame) {
                                    let _ = sink
                                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                                            data.into(),
                                        ))
                                        .await;
                                }
                            }
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
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsSetPolicy,
                            payload: ClientPayload::SecretsSetPolicy {
                                name,
                                policy: next_policy.to_string(),
                                skills: vec![],
                            },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::DeleteSecret { name }) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsDeleteCredential,
                            payload: ClientPayload::SecretsDeleteCredential { name },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::AddSecret { name, value }) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsStore,
                            payload: ClientPayload::SecretsStore { key: name, value },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::RefreshSecrets) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::SecretsList,
                            payload: ClientPayload::SecretsList,
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::RefreshTasks) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::TasksRequest,
                            payload: ClientPayload::TasksRequest { session: None },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::ThreadSwitch(thread_id)) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ThreadSwitch,
                            payload: ClientPayload::ThreadSwitch { thread_id },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::RefreshThreads) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ThreadList,
                            payload: ClientPayload::ThreadList,
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
                }
                Ok(UserInput::ThreadCreate(label)) => {
                    if let Some(ref mut sink) = ws_sink {
                        use futures_util::SinkExt;
                        let frame = ClientFrame {
                            frame_type: ClientFrameType::ThreadCreate,
                            payload: ClientPayload::ThreadCreate { label },
                        };
                        if let Ok(data) = serialize_frame(&frame) {
                            let _ = sink
                                .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
                                .await;
                        }
                    }
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
        Action::Warning(s) => Some(GwEvent::Warning(s.clone())),
        Action::Error(s) => Some(GwEvent::Error(s.clone())),

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
                Some(GwEvent::Error(format!(
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
                Some(GwEvent::Error(
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
                Some(GwEvent::Error(
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
                Some(GwEvent::Error(format!(
                    "Failed to {} credential {}",
                    action_word, cred_name
                )))
            }
        }
        Action::SecretsDeleteCredentialResult { ok, cred_name } => {
            if *ok {
                Some(GwEvent::RefreshSecrets)
            } else {
                Some(GwEvent::Error(format!(
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
                Some(GwEvent::Error(
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
                Some(GwEvent::Error("TOTP verification failed".into()))
            }
        }
        Action::SecretsRemoveTotpResult { ok } => {
            if *ok {
                Some(GwEvent::Success("TOTP removed".into()))
            } else {
                Some(GwEvent::Error("TOTP removal failed".into()))
            }
        }

        // ── Actions that are UI-only (no gateway frame) — show if relevant ──
        Action::ToolCommandDone { message, is_error } => {
            if *is_error {
                Some(GwEvent::Error(message.clone()))
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
        other => Some(GwEvent::Warning(format!("Unhandled event: {}", other))),
    }
}

// ── The iocraft TUI root component ──────────────────────────────────────────

mod tui_component {
    use iocraft::prelude::*;
    use std::sync::mpsc as sync_mpsc;
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    use crate::components::root::Root;
    use crate::theme;
    use crate::types::DisplayMessage;

    use super::{GwEvent, UserInput};

    #[derive(Default, Props)]
    pub struct TuiRootProps {
        pub soul_name: String,
        pub model_label: String,
        pub hint: String,
    }

    // ── Static channels ─────────────────────────────────────────────────
    pub(super) static CHANNEL_RX: StdMutex<Option<sync_mpsc::Receiver<GwEvent>>> =
        StdMutex::new(None);
    pub(super) static CHANNEL_TX: StdMutex<Option<sync_mpsc::Sender<UserInput>>> =
        StdMutex::new(None);

    #[component]
    pub fn TuiRoot(props: &TuiRootProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
        let (width, height) = hooks.use_terminal_size();
        let mut system = hooks.use_context_mut::<SystemContext>();

        // ── Local UI state ──────────────────────────────────────────────
        let mut messages: State<Vec<DisplayMessage>> = hooks.use_state(Vec::new);
        let mut input_value = hooks.use_state(|| String::new());
        let mut gw_status = hooks.use_state(|| rustyclaw_core::types::GatewayStatus::Connecting);
        let mut streaming = hooks.use_state(|| false);
        let mut stream_start: State<Option<Instant>> = hooks.use_state(|| None);
        let mut elapsed = hooks.use_state(|| String::new());
        let mut scroll_offset = hooks.use_state(|| 0i32);
        let mut spinner_tick = hooks.use_state(|| 0usize);
        let mut should_quit = hooks.use_state(|| false);
        let mut streaming_buf = hooks.use_state(|| String::new());

        // ── Auth dialog state ───────────────────────────────────────────
        let mut show_auth_dialog = hooks.use_state(|| false);
        let mut auth_code = hooks.use_state(|| String::new());
        let mut auth_error = hooks.use_state(|| String::new());

        // ── Tool approval dialog state ──────────────────────────────────
        let mut show_tool_approval = hooks.use_state(|| false);
        let mut tool_approval_id = hooks.use_state(|| String::new());
        let mut tool_approval_name = hooks.use_state(|| String::new());
        let mut tool_approval_args = hooks.use_state(|| String::new());
        let mut tool_approval_selected = hooks.use_state(|| true); // true = Allow

        // ── Vault unlock dialog state ───────────────────────────────────
        let mut show_vault_unlock = hooks.use_state(|| false);
        let mut vault_password = hooks.use_state(|| String::new());
        let mut vault_error = hooks.use_state(|| String::new());

        // ── User prompt dialog state ────────────────────────────────────
        let mut show_user_prompt = hooks.use_state(|| false);
        let mut user_prompt_id = hooks.use_state(|| String::new());
        let mut user_prompt_title = hooks.use_state(|| String::new());
        let mut user_prompt_desc = hooks.use_state(|| String::new());
        let mut user_prompt_input = hooks.use_state(|| String::new());
        let mut user_prompt_type: State<Option<rustyclaw_core::user_prompt_types::PromptType>> =
            hooks.use_state(|| None);
        let mut user_prompt_selected = hooks.use_state(|| 0usize);

        // ── Thread state (unified tasks + threads) ───────────────────────
        let mut threads: State<Vec<crate::action::ThreadInfo>> = hooks.use_state(Vec::new);
        let mut sidebar_focused = hooks.use_state(|| false);
        let mut sidebar_selected = hooks.use_state(|| 0usize);

        // ── Command menu (slash-command completions) ────────────────────
        let mut command_completions: State<Vec<String>> = hooks.use_state(Vec::new);
        let mut command_selected: State<Option<usize>> = hooks.use_state(|| None);

        // ── Info dialog state (secrets / skills / tool permissions) ──────
        let mut show_secrets_dialog = hooks.use_state(|| false);
        let mut secrets_dialog_data: State<Vec<crate::components::secrets_dialog::SecretInfo>> =
            hooks.use_state(Vec::new);
        let mut secrets_agent_access = hooks.use_state(|| false);
        let mut secrets_has_totp = hooks.use_state(|| false);
        let mut secrets_selected: State<Option<usize>> = hooks.use_state(|| Some(0));
        let mut secrets_scroll_offset = hooks.use_state(|| 0usize);
        // Add-secret inline input: 0 = off, 1 = entering name, 2 = entering value
        let mut secrets_add_step = hooks.use_state(|| 0u8);
        let mut secrets_add_name = hooks.use_state(|| String::new());
        let mut secrets_add_value = hooks.use_state(|| String::new());

        let mut show_skills_dialog = hooks.use_state(|| false);
        let mut skills_dialog_data: State<Vec<crate::components::skills_dialog::SkillInfo>> =
            hooks.use_state(Vec::new);
        let mut skills_selected: State<Option<usize>> = hooks.use_state(|| Some(0));

        let mut show_tool_perms_dialog = hooks.use_state(|| false);
        let mut tool_perms_dialog_data: State<
            Vec<crate::components::tool_perms_dialog::ToolPermInfo>,
        > = hooks.use_state(Vec::new);
        let mut tool_perms_selected: State<Option<usize>> = hooks.use_state(|| Some(0));

        // Scroll offsets for interactive dialogs
        let mut skills_scroll_offset = hooks.use_state(|| 0usize);
        let mut tool_perms_scroll_offset = hooks.use_state(|| 0usize);

        // ── Channel access ──────────────────────────────────────────────
        let gw_rx: Arc<StdMutex<Option<sync_mpsc::Receiver<GwEvent>>>> =
            hooks.use_const(|| Arc::new(StdMutex::new(CHANNEL_RX.lock().unwrap().take())));
        let user_tx: Arc<StdMutex<Option<sync_mpsc::Sender<UserInput>>>> =
            hooks.use_const(|| Arc::new(StdMutex::new(CHANNEL_TX.lock().unwrap().take())));

        // ── Poll gateway channel on a timer ─────────────────────────────
        hooks.use_future({
            let rx_handle = Arc::clone(&gw_rx);
            let tx_for_history = Arc::clone(&user_tx);
            async move {
                loop {
                    smol::Timer::after(Duration::from_millis(30)).await;

                    if let Ok(guard) = rx_handle.lock() {
                        if let Some(ref rx) = *guard {
                            while let Ok(ev) = rx.try_recv() {
                                match ev {
                                    GwEvent::AuthChallenge => {
                                        // Gateway wants TOTP — show the dialog
                                        gw_status.set(rustyclaw_core::types::GatewayStatus::AuthRequired);
                                        show_auth_dialog.set(true);
                                        auth_code.set(String::new());
                                        auth_error.set(String::new());
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::info("Authentication required — enter TOTP code"));
                                        messages.set(m);
                                    }
                                    GwEvent::Disconnected(reason) => {
                                        gw_status.set(rustyclaw_core::types::GatewayStatus::Disconnected);
                                        show_auth_dialog.set(false);
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::warning(format!("Disconnected: {}", reason)));
                                        messages.set(m);
                                    }
                                    GwEvent::Authenticated => {
                                        gw_status.set(rustyclaw_core::types::GatewayStatus::Connected);
                                        show_auth_dialog.set(false);
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::success("Authenticated"));
                                        messages.set(m);
                                        // Request initial thread list
                                        if let Ok(guard) = tx_for_history.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send(UserInput::RefreshThreads);
                                            }
                                        }
                                    }
                                    GwEvent::Info(s) => {
                                        // Check for "Model ready" or similar to upgrade status
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::info(s));
                                        messages.set(m);
                                    }
                                    GwEvent::Success(s) => {
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::success(s));
                                        messages.set(m);
                                    }
                                    GwEvent::Warning(s) => {
                                        // If auth dialog is open, treat warnings as auth retries
                                        if show_auth_dialog.get() {
                                            auth_error.set(s.clone());
                                            auth_code.set(String::new());
                                        }
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::warning(s));
                                        messages.set(m);
                                    }
                                    GwEvent::Error(s) => {
                                        // Auth errors close the dialog
                                        if show_auth_dialog.get() {
                                            show_auth_dialog.set(false);
                                            auth_code.set(String::new());
                                            auth_error.set(String::new());
                                        }
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::error(s));
                                        messages.set(m);
                                    }
                                    GwEvent::StreamStart => {
                                        streaming.set(true);
                                        // Keep the earlier start time if we already
                                        // began timing on user submit.
                                        if stream_start.get().is_none() {
                                            stream_start.set(Some(Instant::now()));
                                        }
                                        streaming_buf.set(String::new());
                                    }
                                    GwEvent::Chunk(text) => {
                                        let mut buf = streaming_buf.read().clone();
                                        buf.push_str(&text);
                                        streaming_buf.set(buf);

                                        let mut m = messages.read().clone();
                                        if let Some(last) = m.last_mut() {
                                            if last.role == rustyclaw_core::types::MessageRole::Assistant {
                                                last.append(&text);
                                            } else {
                                                m.push(DisplayMessage::assistant(&text));
                                            }
                                        } else {
                                            m.push(DisplayMessage::assistant(&text));
                                        }
                                        messages.set(m);
                                    }
                                    GwEvent::ResponseDone => {
                                        // Capture the accumulated assistant text and
                                        // send it back to the tokio loop so it gets
                                        // appended to the conversation history.
                                        let completed_text = streaming_buf.read().clone();
                                        if !completed_text.is_empty() {
                                            if let Ok(guard) = tx_for_history.lock() {
                                                if let Some(ref tx) = *guard {
                                                    let _ = tx.send(UserInput::AssistantResponse(completed_text));
                                                }
                                            }
                                        }
                                        streaming.set(false);
                                        stream_start.set(None);
                                        elapsed.set(String::new());
                                        streaming_buf.set(String::new());
                                        // Refresh task list after response
                                        if let Ok(guard) = tx_for_history.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send(UserInput::RefreshTasks);
                                            }
                                        }
                                    }
                                    GwEvent::ThinkingStart => {
                                        // Thinking is a form of streaming — show spinner
                                        streaming.set(true);
                                        if stream_start.get().is_none() {
                                            stream_start.set(Some(Instant::now()));
                                        }
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::thinking("Thinking…"));
                                        messages.set(m);
                                    }
                                    GwEvent::ThinkingDelta => {
                                        // Thinking is ongoing — keep spinner alive
                                    }
                                    GwEvent::ThinkingEnd => {
                                        // Thinking done, but streaming may continue
                                        // with chunks. Don't clear streaming here.
                                    }
                                    GwEvent::ModelReady(detail) => {
                                        gw_status.set(rustyclaw_core::types::GatewayStatus::ModelReady);
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::success(detail));
                                        messages.set(m);
                                    }
                                    GwEvent::ToolCall { name, arguments } => {
                                        let msg = if name == "ask_user" {
                                            // Don't show raw JSON args for ask_user — the dialog handles it
                                            format!("🔧 {} — preparing question…", name)
                                        } else {
                                            // Pretty-print JSON arguments if possible
                                            let pretty = serde_json::from_str::<serde_json::Value>(&arguments)
                                                .ok()
                                                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                                                .unwrap_or(arguments);
                                            format!("🔧 {}\n{}", name, pretty)
                                        };
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::tool_call(msg));
                                        messages.set(m);
                                    }
                                    GwEvent::ToolResult { result } => {
                                        let preview = if result.len() > 200 {
                                            format!("{}…", &result[..200])
                                        } else {
                                            result
                                        };
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::tool_result(preview));
                                        messages.set(m);
                                    }
                                    GwEvent::ToolApprovalRequest { id, name, arguments } => {
                                        // Show tool approval dialog
                                        tool_approval_id.set(id);
                                        tool_approval_name.set(name.clone());
                                        tool_approval_args.set(arguments.clone());
                                        tool_approval_selected.set(true);
                                        show_tool_approval.set(true);
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::system(format!(
                                            "🔐 Tool approval required: {} — press Enter to allow, Esc to deny",
                                            name,
                                        )));
                                        messages.set(m);
                                    }
                                    GwEvent::UserPromptRequest(prompt) => {
                                        // Show user prompt dialog
                                        user_prompt_id.set(prompt.id.clone());
                                        user_prompt_title.set(prompt.title.clone());
                                        user_prompt_desc.set(
                                            prompt.description.clone().unwrap_or_default(),
                                        );
                                        user_prompt_input.set(String::new());
                                        user_prompt_type.set(Some(prompt.prompt_type.clone()));
                                        // Set default selection based on prompt type
                                        let default_sel = match &prompt.prompt_type {
                                            rustyclaw_core::user_prompt_types::PromptType::Select { default, .. } => {
                                                default.unwrap_or(0)
                                            }
                                            rustyclaw_core::user_prompt_types::PromptType::Confirm { default } => {
                                                if *default { 0 } else { 1 }
                                            }
                                            _ => 0,
                                        };
                                        user_prompt_selected.set(default_sel);
                                        show_user_prompt.set(true);

                                        // Build informative message based on prompt type
                                        let hint = match &prompt.prompt_type {
                                            rustyclaw_core::user_prompt_types::PromptType::Select { options, .. } => {
                                                let opt_list: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
                                                format!("Options: {}", opt_list.join(", "))
                                            }
                                            rustyclaw_core::user_prompt_types::PromptType::Confirm { .. } => {
                                                "Yes/No".to_string()
                                            }
                                            _ => "Type your answer".to_string(),
                                        };
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::system(format!(
                                            "❓ Agent asks: {} — {}",
                                            prompt.title, hint,
                                        )));
                                        if let Some(desc) = &prompt.description {
                                            if !desc.is_empty() {
                                                m.push(DisplayMessage::info(desc.clone()));
                                            }
                                        }
                                        messages.set(m);
                                    }
                                    GwEvent::VaultLocked => {
                                        gw_status.set(rustyclaw_core::types::GatewayStatus::VaultLocked);
                                        show_vault_unlock.set(true);
                                        vault_password.set(String::new());
                                        vault_error.set(String::new());
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::warning(
                                            "🔒 Vault is locked — enter password to unlock".to_string(),
                                        ));
                                        messages.set(m);
                                    }
                                    GwEvent::VaultUnlocked => {
                                        show_vault_unlock.set(false);
                                        vault_password.set(String::new());
                                        vault_error.set(String::new());
                                        let mut m = messages.read().clone();
                                        m.push(DisplayMessage::success("🔓 Vault unlocked".to_string()));
                                        messages.set(m);
                                    }
                                    GwEvent::ShowSecrets { secrets, agent_access, has_totp } => {
                                        secrets_dialog_data.set(secrets);
                                        secrets_agent_access.set(agent_access);
                                        secrets_has_totp.set(has_totp);
                                        if !show_secrets_dialog.get() {
                                            // First open — reset selection and scroll
                                            secrets_selected.set(Some(0));
                                            secrets_scroll_offset.set(0);
                                            secrets_add_step.set(0);
                                        }
                                        show_secrets_dialog.set(true);
                                    }
                                    GwEvent::ShowSkills { skills } => {
                                        skills_dialog_data.set(skills);
                                        if !show_skills_dialog.get() {
                                            // First open — reset selection and scroll
                                            skills_selected.set(Some(0));
                                            skills_scroll_offset.set(0);
                                        }
                                        show_skills_dialog.set(true);
                                    }
                                    GwEvent::ShowToolPerms { tools } => {
                                        tool_perms_dialog_data.set(tools);
                                        if !show_tool_perms_dialog.get() {
                                            // First open — reset selection and scroll
                                            tool_perms_selected.set(Some(0));
                                            tool_perms_scroll_offset.set(0);
                                        }
                                        show_tool_perms_dialog.set(true);
                                    }
                                    GwEvent::RefreshSecrets => {
                                        // Gateway mutation succeeded — re-fetch list
                                        if let Ok(guard) = tx_for_history.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send(UserInput::RefreshSecrets);
                                            }
                                        }
                                    }
                                    GwEvent::ThreadsUpdate {
                                        threads: thread_list,
                                        foreground_id: _,
                                    } => {
                                        threads.set(thread_list);
                                        // Update sidebar_selected to stay in bounds
                                        let count = threads.read().len();
                                        if count > 0 && sidebar_selected.get() >= count {
                                            sidebar_selected.set(count - 1);
                                        }
                                    }
                                    GwEvent::ThreadSwitched {
                                        thread_id,
                                        context_summary,
                                    } => {
                                        // Clear messages for the new thread
                                        let mut m = Vec::new();
                                        m.push(DisplayMessage::info(format!(
                                            "Switched to thread (id: {})",
                                            thread_id
                                        )));
                                        // Show context summary if available
                                        if let Some(summary) = context_summary {
                                            m.push(DisplayMessage::assistant(format!(
                                                "[Previous context]\n\n{}",
                                                summary
                                            )));
                                        }
                                        messages.set(m);
                                        // Unfocus sidebar after switch
                                        sidebar_focused.set(false);
                                    }
                                }
                            }
                        }
                    }

                    // Update spinner and elapsed timer
                    spinner_tick.set(spinner_tick.get().wrapping_add(1));
                    if let Some(start) = stream_start.get() {
                        let d = start.elapsed();
                        let secs = d.as_secs();
                        elapsed.set(if secs >= 60 {
                            format!("{}m {:02}s", secs / 60, secs % 60)
                        } else {
                            format!("{}.{}s", secs, d.subsec_millis() / 100)
                        });
                    }
                }
            }
        });

        // ── Keyboard handling ───────────────────────────────────────────
        let tx_for_keys = Arc::clone(&user_tx);
        hooks.use_terminal_events({
            move |event| match event {
                TerminalEvent::Key(KeyEvent { code, kind, modifiers, .. })
                    if kind != KeyEventKind::Release =>
                {
                    // ── Auth dialog has focus when visible ───────────
                    if show_auth_dialog.get() {
                        match code {
                            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                                should_quit.set(true);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::Quit);
                                    }
                                }
                            }
                            KeyCode::Esc => {
                                // Cancel auth dialog
                                show_auth_dialog.set(false);
                                auth_code.set(String::new());
                                auth_error.set(String::new());
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::info("Authentication cancelled."));
                                messages.set(m);
                                gw_status.set(rustyclaw_core::types::GatewayStatus::Disconnected);
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                let mut code_val = auth_code.read().clone();
                                if code_val.len() < 6 {
                                    code_val.push(c);
                                    auth_code.set(code_val);
                                }
                            }
                            KeyCode::Backspace => {
                                let mut code_val = auth_code.read().clone();
                                code_val.pop();
                                auth_code.set(code_val);
                            }
                            KeyCode::Enter => {
                                let code_val = auth_code.read().clone();
                                if code_val.len() == 6 {
                                    // Submit the TOTP code — keep dialog open
                                    // until Authenticated/Error arrives
                                    auth_code.set(String::new());
                                    auth_error.set("Verifying…".to_string());
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::AuthResponse(code_val));
                                        }
                                    }
                                }
                                // If < 6 digits, ignore Enter
                            }
                            _ => {}
                        }
                        return;
                    }

                    // ── Tool approval dialog ────────────────────────
                    if show_tool_approval.get() {
                        match code {
                            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                                should_quit.set(true);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::Quit);
                                    }
                                }
                            }
                            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                                // Toggle between Allow / Deny
                                tool_approval_selected.set(!tool_approval_selected.get());
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                // Quick-approve
                                let id = tool_approval_id.read().clone();
                                show_tool_approval.set(false);
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::success(format!(
                                    "✓ Approved: {}", &*tool_approval_name.read()
                                )));
                                messages.set(m);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::ToolApprovalResponse {
                                            id,
                                            approved: true,
                                        });
                                    }
                                }
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                // Deny
                                let id = tool_approval_id.read().clone();
                                show_tool_approval.set(false);
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::warning(format!(
                                    "✗ Denied: {}", &*tool_approval_name.read()
                                )));
                                messages.set(m);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::ToolApprovalResponse {
                                            id,
                                            approved: false,
                                        });
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                let id = tool_approval_id.read().clone();
                                let approved = tool_approval_selected.get();
                                show_tool_approval.set(false);
                                let mut m = messages.read().clone();
                                if approved {
                                    m.push(DisplayMessage::success(format!(
                                        "✓ Approved: {}", &*tool_approval_name.read()
                                    )));
                                } else {
                                    m.push(DisplayMessage::warning(format!(
                                        "✗ Denied: {}", &*tool_approval_name.read()
                                    )));
                                }
                                messages.set(m);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::ToolApprovalResponse {
                                            id,
                                            approved,
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                        return;
                    }

                    // ── Vault unlock dialog ─────────────────────────
                    if show_vault_unlock.get() {
                        match code {
                            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                                should_quit.set(true);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::Quit);
                                    }
                                }
                            }
                            KeyCode::Esc => {
                                show_vault_unlock.set(false);
                                vault_password.set(String::new());
                                vault_error.set(String::new());
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::info("Vault unlock cancelled."));
                                messages.set(m);
                            }
                            KeyCode::Char(c) => {
                                let mut pw = vault_password.read().clone();
                                pw.push(c);
                                vault_password.set(pw);
                            }
                            KeyCode::Backspace => {
                                let mut pw = vault_password.read().clone();
                                pw.pop();
                                vault_password.set(pw);
                            }
                            KeyCode::Enter => {
                                let pw = vault_password.read().clone();
                                if !pw.is_empty() {
                                    vault_password.set(String::new());
                                    vault_error.set("Unlocking…".to_string());
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::VaultUnlock(pw));
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                        return;
                    }

                    // ── User prompt dialog ──────────────────────────
                    if show_user_prompt.get() {
                        let prompt_type = user_prompt_type.read().clone();
                        match code {
                            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                                should_quit.set(true);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::Quit);
                                    }
                                }
                            }
                            KeyCode::Esc => {
                                let id = user_prompt_id.read().clone();
                                show_user_prompt.set(false);
                                user_prompt_input.set(String::new());
                                user_prompt_type.set(None);
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::info("Prompt dismissed."));
                                messages.set(m);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::UserPromptResponse {
                                            id,
                                            dismissed: true,
                                            value: rustyclaw_core::user_prompt_types::PromptResponseValue::Text(String::new()),
                                        });
                                    }
                                }
                            }
                            // Navigation for Select/MultiSelect
                            KeyCode::Up | KeyCode::Char('k') => {
                                if let Some(ref pt) = prompt_type {
                                    match pt {
                                        rustyclaw_core::user_prompt_types::PromptType::Select { options: _, .. } |
                                        rustyclaw_core::user_prompt_types::PromptType::MultiSelect { options: _, .. } => {
                                            let current = user_prompt_selected.get();
                                            if current > 0 {
                                                user_prompt_selected.set(current - 1);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if let Some(ref pt) = prompt_type {
                                    match pt {
                                        rustyclaw_core::user_prompt_types::PromptType::Select { options, .. } |
                                        rustyclaw_core::user_prompt_types::PromptType::MultiSelect { options, .. } => {
                                            let current = user_prompt_selected.get();
                                            if current + 1 < options.len() {
                                                user_prompt_selected.set(current + 1);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            // Left/Right for Confirm
                            KeyCode::Left | KeyCode::Right => {
                                if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) = prompt_type {
                                    let current = user_prompt_selected.get();
                                    user_prompt_selected.set(if current == 0 { 1 } else { 0 });
                                }
                            }
                            // Y/N shortcuts for Confirm
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) = prompt_type {
                                    user_prompt_selected.set(0); // Yes
                                } else {
                                    // Normal text input
                                    let mut input = user_prompt_input.read().clone();
                                    input.push(if code == KeyCode::Char('Y') { 'Y' } else { 'y' });
                                    user_prompt_input.set(input);
                                }
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') => {
                                if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) = prompt_type {
                                    user_prompt_selected.set(1); // No
                                } else {
                                    // Normal text input
                                    let mut input = user_prompt_input.read().clone();
                                    input.push(if code == KeyCode::Char('N') { 'N' } else { 'n' });
                                    user_prompt_input.set(input);
                                }
                            }
                            KeyCode::Char(c) => {
                                // Only for TextInput types
                                if matches!(prompt_type, None | Some(rustyclaw_core::user_prompt_types::PromptType::TextInput { .. }) | Some(rustyclaw_core::user_prompt_types::PromptType::Form { .. })) {
                                    let mut input = user_prompt_input.read().clone();
                                    input.push(c);
                                    user_prompt_input.set(input);
                                }
                            }
                            KeyCode::Backspace => {
                                let mut input = user_prompt_input.read().clone();
                                input.pop();
                                user_prompt_input.set(input);
                            }
                            KeyCode::Enter => {
                                let id = user_prompt_id.read().clone();
                                let input = user_prompt_input.read().clone();
                                let selected = user_prompt_selected.get();
                                show_user_prompt.set(false);
                                user_prompt_input.set(String::new());
                                user_prompt_type.set(None);

                                // Build response based on prompt type
                                let (value, display) = match &prompt_type {
                                    Some(rustyclaw_core::user_prompt_types::PromptType::Select { options, .. }) => {
                                        let label = options.get(selected).map(|o| o.label.clone()).unwrap_or_default();
                                        (rustyclaw_core::user_prompt_types::PromptResponseValue::Selected(vec![label.clone()]), format!("→ {}", label))
                                    }
                                    Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) => {
                                        let yes = selected == 0;
                                        (rustyclaw_core::user_prompt_types::PromptResponseValue::Confirm(yes), format!("→ {}", if yes { "Yes" } else { "No" }))
                                    }
                                    Some(rustyclaw_core::user_prompt_types::PromptType::MultiSelect { options, .. }) => {
                                        // TODO: track multiple selections properly
                                        let label = options.get(selected).map(|o| o.label.clone()).unwrap_or_default();
                                        (rustyclaw_core::user_prompt_types::PromptResponseValue::Selected(vec![label.clone()]), format!("→ {}", label))
                                    }
                                    _ => {
                                        (rustyclaw_core::user_prompt_types::PromptResponseValue::Text(input.clone()), format!("→ {}", input))
                                    }
                                };

                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::user(display));
                                messages.set(m);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::UserPromptResponse {
                                            id,
                                            dismissed: false,
                                            value,
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                        return;
                    }

                    // ── Normal mode keyboard ────────────────────────
                    // Info dialogs: Esc to close, Up/Down to navigate, Enter to act
                    if show_skills_dialog.get() {
                        const VISIBLE_ROWS: usize = 20;
                        match code {
                            KeyCode::Esc => {
                                show_skills_dialog.set(false);
                            }
                            KeyCode::Up => {
                                let cur = skills_selected.get().unwrap_or(0);
                                let len = skills_dialog_data.read().len();
                                if len > 0 {
                                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                                    skills_selected.set(Some(next));
                                    // Adjust scroll offset
                                    let so = skills_scroll_offset.get();
                                    if next < so {
                                        skills_scroll_offset.set(next);
                                    } else if next >= so + VISIBLE_ROWS {
                                        skills_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                                    }
                                }
                            }
                            KeyCode::Down => {
                                let cur = skills_selected.get().unwrap_or(0);
                                let len = skills_dialog_data.read().len();
                                if len > 0 {
                                    let next = (cur + 1) % len;
                                    skills_selected.set(Some(next));
                                    // Adjust scroll offset
                                    let so = skills_scroll_offset.get();
                                    if next < so {
                                        skills_scroll_offset.set(next);
                                    } else if next >= so + VISIBLE_ROWS {
                                        skills_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                let idx = skills_selected.get().unwrap_or(0);
                                let data = skills_dialog_data.read();
                                if let Some(skill) = data.get(idx) {
                                    let name = skill.name.clone();
                                    drop(data);
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::ToggleSkill { name });
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                        return;
                    }
                    if show_tool_perms_dialog.get() {
                        const VISIBLE_ROWS: usize = 20;
                        match code {
                            KeyCode::Esc => {
                                show_tool_perms_dialog.set(false);
                            }
                            KeyCode::Up => {
                                let cur = tool_perms_selected.get().unwrap_or(0);
                                let len = tool_perms_dialog_data.read().len();
                                if len > 0 {
                                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                                    tool_perms_selected.set(Some(next));
                                    let so = tool_perms_scroll_offset.get();
                                    if next < so {
                                        tool_perms_scroll_offset.set(next);
                                    } else if next >= so + VISIBLE_ROWS {
                                        tool_perms_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                                    }
                                }
                            }
                            KeyCode::Down => {
                                let cur = tool_perms_selected.get().unwrap_or(0);
                                let len = tool_perms_dialog_data.read().len();
                                if len > 0 {
                                    let next = (cur + 1) % len;
                                    tool_perms_selected.set(Some(next));
                                    let so = tool_perms_scroll_offset.get();
                                    if next < so {
                                        tool_perms_scroll_offset.set(next);
                                    } else if next >= so + VISIBLE_ROWS {
                                        tool_perms_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                let idx = tool_perms_selected.get().unwrap_or(0);
                                let data = tool_perms_dialog_data.read();
                                if let Some(tool) = data.get(idx) {
                                    let name = tool.name.clone();
                                    drop(data);
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::CycleToolPermission { name });
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                        return;
                    }
                    if show_secrets_dialog.get() {
                        const VISIBLE_ROWS: usize = 20;
                        // Add-secret inline input mode
                        let add_step = secrets_add_step.get();
                        if add_step > 0 {
                            match code {
                                KeyCode::Esc => {
                                    secrets_add_step.set(0);
                                    secrets_add_name.set(String::new());
                                    secrets_add_value.set(String::new());
                                }
                                KeyCode::Enter => {
                                    if add_step == 1 {
                                        // Name entered, move to value
                                        if !secrets_add_name.read().trim().is_empty() {
                                            secrets_add_step.set(2);
                                        }
                                    } else {
                                        // Value entered, submit
                                        let name = secrets_add_name.read().trim().to_string();
                                        let value = secrets_add_value.read().clone();
                                        if !name.is_empty() && !value.is_empty() {
                                            if let Ok(guard) = tx_for_keys.lock() {
                                                if let Some(ref tx) = *guard {
                                                    let _ = tx.send(UserInput::AddSecret { name, value });
                                                }
                                            }
                                        }
                                        secrets_add_step.set(0);
                                        secrets_add_name.set(String::new());
                                        secrets_add_value.set(String::new());
                                    }
                                }
                                KeyCode::Backspace => {
                                    if add_step == 1 {
                                        let mut s = secrets_add_name.read().clone();
                                        s.pop();
                                        secrets_add_name.set(s);
                                    } else {
                                        let mut s = secrets_add_value.read().clone();
                                        s.pop();
                                        secrets_add_value.set(s);
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if add_step == 1 {
                                        let mut s = secrets_add_name.read().clone();
                                        s.push(c);
                                        secrets_add_name.set(s);
                                    } else {
                                        let mut s = secrets_add_value.read().clone();
                                        s.push(c);
                                        secrets_add_value.set(s);
                                    }
                                }
                                _ => {}
                            }
                            return;
                        }
                        // Normal secrets dialog navigation
                        match code {
                            KeyCode::Esc => {
                                show_secrets_dialog.set(false);
                            }
                            KeyCode::Up => {
                                let cur = secrets_selected.get().unwrap_or(0);
                                let len = secrets_dialog_data.read().len();
                                if len > 0 {
                                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                                    secrets_selected.set(Some(next));
                                    let so = secrets_scroll_offset.get();
                                    if next < so {
                                        secrets_scroll_offset.set(next);
                                    } else if next >= so + VISIBLE_ROWS {
                                        secrets_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                                    }
                                }
                            }
                            KeyCode::Down => {
                                let cur = secrets_selected.get().unwrap_or(0);
                                let len = secrets_dialog_data.read().len();
                                if len > 0 {
                                    let next = (cur + 1) % len;
                                    secrets_selected.set(Some(next));
                                    let so = secrets_scroll_offset.get();
                                    if next < so {
                                        secrets_scroll_offset.set(next);
                                    } else if next >= so + VISIBLE_ROWS {
                                        secrets_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                // Cycle permission policy
                                let idx = secrets_selected.get().unwrap_or(0);
                                let data = secrets_dialog_data.read();
                                if let Some(secret) = data.get(idx) {
                                    let name = secret.name.clone();
                                    let policy = secret.policy.clone();
                                    drop(data);
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::CycleSecretPolicy { name, current_policy: policy });
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('d') | KeyCode::Delete => {
                                // Delete selected secret
                                let idx = secrets_selected.get().unwrap_or(0);
                                let data = secrets_dialog_data.read();
                                if let Some(secret) = data.get(idx) {
                                    let name = secret.name.clone();
                                    drop(data);
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::DeleteSecret { name });
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('a') => {
                                // Start add-secret inline input
                                secrets_add_step.set(1);
                                secrets_add_name.set(String::new());
                                secrets_add_value.set(String::new());
                            }
                            _ => {}
                        }
                        return;
                    }

                    // Command menu intercepts when visible
                    let menu_open = !command_completions.read().is_empty();

                    match code {
                        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                            should_quit.set(true);
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::Quit);
                                }
                            }
                        }
                        KeyCode::Tab if menu_open => {
                            // Cycle forward through completions
                            let completions = command_completions.read().clone();
                            let new_idx = match command_selected.get() {
                                Some(i) => (i + 1) % completions.len(),
                                None => 0,
                            };
                            command_selected.set(Some(new_idx));
                            // Apply the selected completion into the input
                            if let Some(cmd) = completions.get(new_idx) {
                                input_value.set(format!("/{}", cmd));
                            }
                        }
                        KeyCode::BackTab if menu_open => {
                            // Cycle backward through completions
                            let completions = command_completions.read().clone();
                            let new_idx = match command_selected.get() {
                                Some(0) | None => completions.len().saturating_sub(1),
                                Some(i) => i - 1,
                            };
                            command_selected.set(Some(new_idx));
                            if let Some(cmd) = completions.get(new_idx) {
                                input_value.set(format!("/{}", cmd));
                            }
                        }
                        KeyCode::Up if menu_open => {
                            // Navigate up through completions
                            let completions = command_completions.read().clone();
                            let new_idx = match command_selected.get() {
                                Some(0) | None => completions.len().saturating_sub(1),
                                Some(i) => i - 1,
                            };
                            command_selected.set(Some(new_idx));
                            if let Some(cmd) = completions.get(new_idx) {
                                input_value.set(format!("/{}", cmd));
                            }
                        }
                        KeyCode::Down if menu_open => {
                            // Navigate down through completions
                            let completions = command_completions.read().clone();
                            let new_idx = match command_selected.get() {
                                Some(i) => (i + 1) % completions.len(),
                                None => 0,
                            };
                            command_selected.set(Some(new_idx));
                            if let Some(cmd) = completions.get(new_idx) {
                                input_value.set(format!("/{}", cmd));
                            }
                        }
                        KeyCode::Esc if menu_open => {
                            // Close the command menu
                            command_completions.set(Vec::new());
                            command_selected.set(None);
                        }
                        KeyCode::Enter if sidebar_focused.get() => {
                            let thread_list = threads.read().clone();
                            if let Some(thread) = thread_list.get(sidebar_selected.get()) {
                                // Send thread switch request
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::ThreadSwitch(thread.id));
                                    }
                                }
                            }
                            // Return focus to input after selection
                            sidebar_focused.set(false);
                        }
                        KeyCode::Enter => {
                            let val = input_value.to_string();
                            if !val.is_empty() {
                                input_value.set(String::new());
                                // Close command menu
                                command_completions.set(Vec::new());
                                command_selected.set(None);
                                // Snap to bottom so user sees their message + response
                                scroll_offset.set(0);
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        if val.starts_with('/') {
                                            let _ = tx.send(UserInput::Command(
                                                val.trim_start_matches('/').to_string(),
                                            ));
                                        } else {
                                            let mut m = messages.read().clone();
                                            m.push(DisplayMessage::user(&val));
                                            messages.set(m);
                                            // Start the spinner immediately so the user
                                            // sees feedback while waiting for the model.
                                            streaming.set(true);
                                            stream_start.set(Some(Instant::now()));
                                            let _ = tx.send(UserInput::Chat(val));
                                        }
                                    }
                                }
                            }
                        }
                        // Tab toggles sidebar focus when command menu is not open
                        KeyCode::Tab if !menu_open => {
                            sidebar_focused.set(!sidebar_focused.get());
                        }
                        // Sidebar navigation when focused
                        KeyCode::Up if sidebar_focused.get() => {
                            let thread_count = threads.read().len();
                            if thread_count > 0 {
                                let current = sidebar_selected.get();
                                sidebar_selected.set(current.saturating_sub(1));
                            }
                        }
                        KeyCode::Down if sidebar_focused.get() => {
                            let thread_count = threads.read().len();
                            if thread_count > 0 {
                                let current = sidebar_selected.get();
                                sidebar_selected.set((current + 1).min(thread_count - 1));
                            }
                        }
                        KeyCode::Esc if sidebar_focused.get() => {
                            // Escape returns focus to input
                            sidebar_focused.set(false);
                        }
                        KeyCode::Up => {
                            scroll_offset.set(scroll_offset.get() + 1);
                        }
                        KeyCode::Down => {
                            scroll_offset.set((scroll_offset.get() - 1).max(0));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        });

        if should_quit.get() {
            system.exit();
        }

        // Auto-scroll to bottom when streaming
        if streaming.get() {
            scroll_offset.set(0);
        }

        // Gateway display
        let status = gw_status.get();
        let gw_icon = theme::gateway_icon(&status).to_string();
        let gw_label = status.label().to_string();
        let gw_color = Some(theme::gateway_color(&status));

        element! {
            Root(
                width: width,
                height: height,
                soul_name: props.soul_name.clone(),
                model_label: props.model_label.clone(),
                gateway_icon: gw_icon,
                gateway_label: gw_label,
                gateway_color: gw_color,
                messages: messages.read().clone(),
                scroll_offset: scroll_offset.get(),
                command_completions: command_completions.read().clone(),
                command_selected: command_selected.get(),
                input_value: input_value.to_string(),
                input_has_focus: !show_auth_dialog.get()
                    && !show_tool_approval.get()
                    && !show_vault_unlock.get()
                    && !show_user_prompt.get()
                    && !show_secrets_dialog.get()
                    && !show_skills_dialog.get()
                    && !show_tool_perms_dialog.get()
                    && !sidebar_focused.get(),
                on_change: move |new_val: String| {
                    input_value.set(new_val.clone());
                    // Update slash-command completions
                    if let Some(partial) = new_val.strip_prefix('/') {
                        let names = rustyclaw_core::commands::command_names();
                        let filtered: Vec<String> = names
                            .into_iter()
                            .filter(|c: &String| c.starts_with(partial))
                            .collect();
                        if filtered.is_empty() {
                            command_completions.set(Vec::new());
                            command_selected.set(None);
                        } else {
                            command_completions.set(filtered);
                            command_selected.set(None);
                        }
                    } else {
                        command_completions.set(Vec::new());
                        command_selected.set(None);
                    }
                },
                on_submit: move |_val: String| {
                    // Submit handled by Enter key above
                },
                task_text: if streaming.get() { "Streaming…".to_string() } else { "Idle".to_string() },
                streaming: streaming.get(),
                elapsed: elapsed.to_string(),
                threads: threads.read().clone(),
                sidebar_focused: sidebar_focused.get(),
                sidebar_selected: sidebar_selected.get(),
                hint: props.hint.clone(),
                spinner_tick: spinner_tick.get(),
                show_auth_dialog: show_auth_dialog.get(),
                auth_code: auth_code.read().clone(),
                auth_error: auth_error.read().clone(),
                show_tool_approval: show_tool_approval.get(),
                tool_approval_name: tool_approval_name.read().clone(),
                tool_approval_args: tool_approval_args.read().clone(),
                tool_approval_selected: tool_approval_selected.get(),
                show_vault_unlock: show_vault_unlock.get(),
                vault_password_len: vault_password.read().len(),
                vault_error: vault_error.read().clone(),
                show_user_prompt: show_user_prompt.get(),
                user_prompt_title: user_prompt_title.read().clone(),
                user_prompt_desc: user_prompt_desc.read().clone(),
                user_prompt_input: user_prompt_input.read().clone(),
                user_prompt_type: user_prompt_type.read().clone(),
                user_prompt_selected: user_prompt_selected.get(),
                show_secrets_dialog: show_secrets_dialog.get(),
                secrets_data: secrets_dialog_data.read().clone(),
                secrets_agent_access: secrets_agent_access.get(),
                secrets_has_totp: secrets_has_totp.get(),
                secrets_selected: secrets_selected.get(),
                secrets_scroll_offset: secrets_scroll_offset.get(),
                secrets_add_step: secrets_add_step.get(),
                secrets_add_name: secrets_add_name.read().clone(),
                secrets_add_value: secrets_add_value.read().clone(),
                show_skills_dialog: show_skills_dialog.get(),
                skills_data: skills_dialog_data.read().clone(),
                skills_selected: skills_selected.get(),
                skills_scroll_offset: skills_scroll_offset.get(),
                show_tool_perms_dialog: show_tool_perms_dialog.get(),
                tool_perms_data: tool_perms_dialog_data.read().clone(),
                tool_perms_selected: tool_perms_selected.get(),
                tool_perms_scroll_offset: tool_perms_scroll_offset.get(),
            )
        }
    }
}

// Re-export the component so element!() can find it
use tui_component::TuiRoot;
