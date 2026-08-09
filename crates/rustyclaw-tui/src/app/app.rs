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

use rustyclaw_core::ignore::Ignore;
use rustyclaw_view::anyhow::Result;
use rustyclaw_view::{tokio, tracing, url};
use std::sync::mpsc as sync_mpsc;

use rustyclaw_core::commands::{CommandContext, CommandResponse, handle_command};
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{EngineActionKind, GatewayClient, GatewayCommand, SessionOrigin};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_core::soul::SoulManager;
use rustyclaw_view::{PromptAttachment, build_prompt_with_attachments};

use crate::gateway_client;

use super::GwEvent;
use super::command_action::handle_command_action;
use super::tui_component;
use super::tui_component::TuiRoot;
use rustyclaw_view::anyhow::Context;

/// Messages from the iocraft render component back to tokio.
#[derive(Debug, Clone, strum::IntoStaticStr)]
pub(crate) enum UserInput {
    /// The text, and the thread the user typed it into. The id travels with
    /// the message so the gateway files it where the user was looking, not
    /// wherever its foreground has drifted to by the time the frame lands.
    Chat {
        text: String,
        thread_id: Option<u64>,
    },
    /// Stop the turn running in `thread_id`. Named, because the gateway can
    /// no longer resolve "the current one" once turns run per thread.
    CancelCurrentRequest {
        thread_id: Option<u64>,
    },
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
    /// User responded to a credential request
    CredentialResponse {
        id: String,
        dismissed: bool,
        value: Option<String>,
    },
    /// Pause/resume/stop/kill the process behind the running tool call.
    ProcessControl {
        pid: u32,
        action: rustyclaw_core::exec_status::ProcessControlAction,
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
    /// Reveal a credential's values. `code` carries a TOTP code when the
    /// gateway has asked for one; the first attempt sends `None`.
    PeekSecret {
        name: String,
        code: Option<String>,
    },
    /// Re-request secrets list from gateway (after a mutation)
    RefreshSecrets,
    /// Re-request a gateway panel's data (after a mutation)
    RefreshPanel(crate::app::PanelKind),
    /// A messenger-setup mutation or refresh, forwarded to the gateway.
    ///
    /// Carries the command rather than re-deriving it here: the messengers
    /// dialog builds several different frames (save, delete, migrate, route
    /// save, route delete) and re-encoding each as a `UserInput` variant would
    /// duplicate the whole surface for no gain.
    MessengerCommand(rustyclaw_core::gateway::client_types::GatewayCommand),
    /// Request current task list from gateway
    RefreshTasks,
    /// Request current thread list from gateway
    RefreshThreads,
    /// Switch to a different thread
    ThreadSwitch(u64),
    /// Switch the connection's active agent (from the agent selector dialog)
    AgentSwitch(String),
    /// Request the gateway-persisted history for a thread (cross-session/client).
    RequestThreadHistory(u64),
    /// Hatching name entered - save personalised SOUL.md
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
    /// Search the Hugging Face Hub for repo ids matching a partial model
    /// name (progressive autocomplete for `/engines pull|files|search`).
    FetchHubModelCompletions {
        query: String,
        gguf_only: bool,
    },
    /// Cancel the current provider-flow dialog
    CancelProviderFlow,
    /// Engines panel: select an engine (fetch its model list)
    EngineSelect(String),
    /// Engines panel: lifecycle action (install/start/stop)
    EngineAction {
        engine: String,
        action: EngineActionKind,
    },
    /// Engines panel: refresh the engine list
    EngineRefresh,
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
    skip_connection_dialog: bool,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::locked(config.credentials_dir());
        Self::build(config, secrets_manager)
    }

    pub fn set_deferred_vault_password(&mut self, password: String) {
        self.deferred_vault_password = Some(password);
    }

    /// Skip the interactive connection dialog and connect directly using
    /// the configured / saved / default gateway URL.
    pub fn set_skip_connection_dialog(&mut self, skip: bool) {
        self.skip_connection_dialog = skip;
    }

    fn build(config: Config, mut secrets_manager: SecretsManager) -> Result<Self> {
        if !config.use_secrets {
            secrets_manager.set_agent_access(false);
        } else {
            secrets_manager.set_agent_access(config.agent_access);
        }

        // Neither failure is fatal — the app runs with no skills and an empty
        // soul — but both leave the user staring at a feature that looks empty
        // for no stated reason, so the reason goes to the log.
        let skills_dirs = config.skills_dirs();
        let mut skill_manager = SkillManager::with_dirs(skills_dirs);
        if let Err(e) = skill_manager.load_skills() {
            tracing::warn!("starting with no skills loaded: {}", e);
        }

        let soul_path = config.soul_path();
        let mut soul_manager = SoulManager::new(soul_path);
        if let Err(e) = soul_manager.load() {
            tracing::warn!("starting with no SOUL.md loaded: {}", e);
        }

        Ok(Self {
            config,
            secrets_manager,
            skill_manager,
            soul_manager,
            deferred_vault_password: None,
            skip_connection_dialog: false,
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

        let gateway_url_explicit = self.config.gateway_url.clone();
        let skip_dialog = self.skip_connection_dialog;

        // ── Show the connection dialog (or skip when --url/config provided)
        //    and establish the SSH transport before iocraft takes over. ──
        let conn_result = match crate::connection_dialog::prompt_and_connect(
            gateway_url_explicit.clone(),
            skip_dialog,
        )
        .await
        {
            Ok(Some(r)) => r,
            Ok(None) => {
                // User cancelled the dialog — exit cleanly.
                return Ok(());
            }
            Err(e) => {
                crate::app::events::emit(
                    &gw_tx,
                    GwEvent::error(format!("SSH connection failed: {}", e)),
                );
                crate::app::events::emit(
                    &gw_tx,
                    GwEvent::Disconnected(format!("Failed to connect: {}", e)),
                );
                return Ok(());
            }
        };

        let gateway_url = conn_result.url.clone();

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

        // ── Build the shared gateway client over the dialog's transport ──
        // The connection dialog already established the SSH transport, so we
        // hand its parts to the shared core client rather than reconnecting.
        let client = std::sync::Arc::new(GatewayClient::from_transport(
            conn_result.connection,
            conn_result.writer,
            conn_result.reader,
            Some(gateway_url.as_str()),
        ));

        // Reader task: drain shared GatewayEvents from the client and adapt
        // them into the TUI's UI events. Wire-frame parsing and EOF/error →
        // Disconnected mapping all live in the core client now.
        let gw_tx_conn = gw_tx.clone();
        let client_reader = client.clone();
        let _reader_handle = tokio::spawn(async move {
            while let Some(threaded) = client_reader.recv().await {
                if let Some(ev) =
                    gateway_client::gateway_event_to_gw_event(threaded.thread_id, threaded.event)
                {
                    if gw_tx_conn.send(ev).is_err() {
                        break;
                    }
                }
            }
        });

        // ── Spawn the iocraft render on a blocking thread ───────────────
        // Stash the channels in statics so the component can grab them on
        // first render (via use_const). This avoids ownership issues with
        // iocraft props.
        *tui_component::CHANNEL_RX.lock().unwrap() = Some(gw_rx);
        *tui_component::CHANNEL_TX.lock().unwrap() = Some(user_tx);

        // Point prompt-history persistence at this profile's settings dir
        // before the component's first render loads it.
        crate::input_history::init(self.config.settings_dir.join("input_history"));

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
        // Stream-id assignment and active-stream tracking (for Cancel) now
        // live inside the shared gateway client.
        let mut prompt_attachments: Vec<PromptAttachment> = Vec::new();
        let config = &mut self.config;
        let secrets_manager = &mut self.secrets_manager;
        let skill_manager = &mut self.skill_manager;

        // Monotonic sequence for Hub autocomplete requests: each keystroke
        // bumps it, and an in-flight request only runs its search if it is
        // still the newest after a short debounce sleep. This collapses
        // rapid typing into one Hub API call for the final prefix.
        let hub_fetch_seq = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

        loop {
            // Poll user_rx (non-blocking on tokio side)
            match user_rx.try_recv() {
                Ok(UserInput::Chat { text, thread_id }) => {
                    let prompt = build_prompt_with_attachments(&text, &prompt_attachments);
                    prompt_attachments.clear();
                    crate::app::events::emit(
                        &gw_tx,
                        GwEvent::PromptAttachmentsChanged {
                            attachments: prompt_attachments.clone(),
                        },
                    );
                    client
                        .send(GatewayCommand::Chat {
                            message: prompt,
                            thread_id,
                            client_kind: Some(SessionOrigin::Tui),
                        })
                        .await
                        .context("sending Chat")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::AuthResponse(code)) => {
                    client
                        .send(GatewayCommand::Auth { code })
                        .await
                        .context("sending Auth")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::ToolApprovalResponse { id, approved }) => {
                    client
                        .send(GatewayCommand::ToolApprove { id, approved })
                        .await
                        .context("sending ToolApprove")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::VaultUnlock(password)) => {
                    // Unlock locally so /secrets can read the vault
                    secrets_manager.set_password(password.clone());
                    client
                        .send(GatewayCommand::VaultUnlock { password })
                        .await
                        .context("sending VaultUnlock")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::UserPromptResponse {
                    id,
                    dismissed,
                    value,
                }) => {
                    client
                        .send(GatewayCommand::UserPromptResponse {
                            id,
                            dismissed,
                            value,
                        })
                        .await
                        .context("sending UserPromptResponse")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::CredentialResponse {
                    id,
                    dismissed,
                    value,
                }) => {
                    client
                        .send(GatewayCommand::CredentialResponse {
                            id,
                            dismissed,
                            value,
                        })
                        .await
                        .context("sending CredentialResponse")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::CancelCurrentRequest { thread_id }) => {
                    client
                        .send(GatewayCommand::Cancel { thread_id })
                        .await
                        .context("sending Cancel")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::ProcessControl { pid, action }) => {
                    client
                        .send(GatewayCommand::ProcessControl { pid, action })
                        .await
                        .context("sending ProcessControl")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::AssistantResponse(text)) => {
                    let _ = text;
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
                                gw_tx2
                                    .send(GwEvent::ModelCompletionsLoaded { provider, models })
                                    .ignore();
                            }
                            Err(e) => {
                                gw_tx2
                                    .send(GwEvent::Warning {
                                        summary: format!(
                                            "Failed to load model completions: {:#}",
                                            e
                                        ),
                                        details: Some(
                                            rustyclaw_core::error_details::render_extended(&e),
                                        ),
                                    })
                                    .ignore();
                            }
                        }
                    });
                }
                Ok(UserInput::FetchHubModelCompletions { query, gguf_only }) => {
                    use std::sync::atomic::Ordering;
                    let seq = hub_fetch_seq.fetch_add(1, Ordering::SeqCst) + 1;
                    let seq_ref = hub_fetch_seq.clone();
                    let gw_tx2 = gw_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        if seq_ref.load(Ordering::SeqCst) != seq {
                            // A newer keystroke superseded this request.
                            return;
                        }
                        // Autocomplete is best-effort: a failed search just
                        // means no suggestions, never an error toast.
                        let models =
                            rustyclaw_core::engines::hub::search_models(&query, gguf_only, 10)
                                .await
                                .map(|models| models.into_iter().map(|m| m.id).collect())
                                .unwrap_or_default();
                        gw_tx2
                            .send(GwEvent::HubModelCompletionsLoaded { query, models })
                            .ignore();
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
                        crate::app::events::emit(&gw_tx, GwEvent::Info(msg.clone()));
                    }
                    if handle_command_action(
                        resp.action,
                        &client,
                        &gw_tx,
                        &mut prompt_attachments,
                        config,
                        secrets_manager,
                        skill_manager,
                    )
                    .await?
                    {
                        break;
                    }
                }
                Ok(UserInput::ToggleSkill { name }) => {
                    if let Some(skill) = skill_manager.get_skills().iter().find(|s| s.name == name)
                    {
                        let new_enabled = !skill.enabled;
                        skill_manager.set_skill_enabled(&name, new_enabled).ignore();
                        // Re-send updated skills list
                        let skills_list: Vec<_> = skill_manager
                            .get_skills()
                            .iter()
                            .map(|s| rustyclaw_view::SkillInfoData {
                                name: s.name.clone(),
                                description: s.description.clone().unwrap_or_default(),
                                enabled: s.enabled,
                            })
                            .collect();
                        crate::app::events::emit(
                            &gw_tx,
                            GwEvent::ShowSkills {
                                skills: skills_list,
                            },
                        );
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
                    crate::app::events::persist_config(config, &gw_tx);
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
                            rustyclaw_view::ToolPermInfoData {
                                name: tn.to_string(),
                                permission: perm.badge().to_string(),
                                summary: rustyclaw_core::tools::tool_summary(tn).to_string(),
                            }
                        })
                        .collect();
                    crate::app::events::emit(&gw_tx, GwEvent::ShowToolPerms { tools });
                }
                Ok(UserInput::CycleSecretPolicy {
                    name,
                    current_policy,
                }) => {
                    // Cycle OPEN → ASK → AUTH → SKILL → OPEN, then translate
                    // to the wire vocabulary of SecretsSetPolicy.
                    use rustyclaw_core::secrets::AccessPolicy;
                    let next_policy = match AccessPolicy::from_badge(&current_policy)
                        .map(|p| p.cycled())
                        .unwrap_or_default()
                    {
                        AccessPolicy::Always => "always",
                        AccessPolicy::WithApproval => "ask",
                        AccessPolicy::WithAuth => "auth",
                        AccessPolicy::SkillOnly(_) => "skill_only",
                        // Trigger-scoping is set via secrets_link_trigger, not
                        // the interactive cycle (which never yields it — see
                        // AccessPolicy::cycled); cycle it back to OPEN.
                        AccessPolicy::TriggerOnly(_) => "always",
                    };
                    client
                        .send(GatewayCommand::SecretsSetPolicy {
                            name,
                            policy: next_policy.to_string(),
                            skills: vec![],
                        })
                        .await
                        .context("sending SecretsSetPolicy")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::DeleteSecret { name }) => {
                    client
                        .send(GatewayCommand::SecretsDeleteCredential { name })
                        .await
                        .context("sending SecretsDeleteCredential")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::AddSecret { name, value }) => {
                    client
                        .send(GatewayCommand::SecretsStore { key: name, value })
                        .await
                        .context("sending SecretsStore")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::PeekSecret { name, code }) => {
                    client
                        .send(GatewayCommand::SecretsPeek { name, code })
                        .await
                        .context("sending SecretsPeek")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::RefreshSecrets) => {
                    client
                        .send(GatewayCommand::SecretsList)
                        .await
                        .context("sending SecretsList")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::RefreshPanel(panel)) => {
                    let cmd = match panel {
                        crate::app::PanelKind::Cron => GatewayCommand::CronList,
                        crate::app::PanelKind::Memory => GatewayCommand::MemoryList {
                            query: None,
                            limit: None,
                        },
                        crate::app::PanelKind::Mcp => GatewayCommand::McpList,
                        crate::app::PanelKind::Channels => GatewayCommand::ChannelStatus,
                    };
                    client
                        .send(cmd)
                        .await
                        .context("sending cmd")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::MessengerCommand(cmd)) => {
                    client
                        .send(cmd)
                        .await
                        .context("sending cmd")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::RefreshTasks) => {
                    client
                        .send(GatewayCommand::TasksRequest { session: None })
                        .await
                        .context("sending TasksRequest")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::ThreadSwitch(thread_id)) => {
                    client
                        .send(GatewayCommand::ThreadSwitch { thread_id })
                        .await
                        .context("sending ThreadSwitch")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::AgentSwitch(agent_id)) => {
                    client
                        .send(GatewayCommand::AgentSwitch { agent_id })
                        .await
                        .context("sending AgentSwitch")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::RequestThreadHistory(thread_id)) => {
                    // A dropped error here reads as an empty thread: the view
                    // is waiting for a reply to a request that never went out.
                    if let Err(e) = client
                        .send(GatewayCommand::ThreadHistoryRequest { thread_id })
                        .await
                    {
                        tracing::error!(
                            thread_id,
                            error = %e,
                            "Thread history request failed to send"
                        );
                    }
                }
                Ok(UserInput::RefreshThreads) => {
                    client
                        .send(GatewayCommand::ThreadList)
                        .await
                        .context("sending ThreadList")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::HatchingComplete(payload)) => {
                    // Parse "name\tpersonality" or just "name"
                    let (name, personality) = if let Some((n, p)) = payload.split_once('\t') {
                        (n.trim().to_string(), Some(p.trim().to_string()))
                    } else {
                        (payload.trim().to_string(), None)
                    };
                    let soul_path = config.soul_path();
                    // Build personalised SOUL.md: heading with name, then optional
                    // personality section, then the default template body.
                    let default_body = rustyclaw_core::soul::DEFAULT_SOUL_CONTENT
                        .trim_start_matches("# SOUL.md - Who You Are")
                        .trim_start_matches('\n');
                    let content = if let Some(ref p) = personality {
                        format!("# {}\n\n## Personality\n\n{}\n\n{}", name, p, default_body)
                    } else {
                        format!("# {}\n\n{}", name, default_body)
                    };
                    if let Err(e) = std::fs::write(&soul_path, &content) {
                        tracing::warn!("Failed to write SOUL.md: {}", e);
                    } else {
                        tracing::debug!("Saved SOUL.md for agent {:?} to {:?}", name, soul_path);
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
                                    base_url:
                                        rustyclaw_core::providers::base_url_override_for_switch(
                                            &provider_id,
                                            config.model.as_ref().map(|m| m.provider.as_str()),
                                            config.model.as_ref().and_then(|m| m.base_url.clone()),
                                        ),
                                });
                                crate::app::events::persist_config(config, &gw_tx);
                                // Reload gateway
                                client
                                    .send(GatewayCommand::Reload)
                                    .await
                                    .context("sending Reload")
                                    .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                                // Trigger model selector (show loading)
                                let display = def.display.to_string();
                                let pid = provider_id.clone();
                                crate::app::events::emit(
                                    &gw_tx,
                                    GwEvent::FetchModelsLoading {
                                        provider: pid.clone(),
                                        provider_display: display.clone(),
                                    },
                                );
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
                                            gw_tx2
                                                .send(GwEvent::ShowModelSelector {
                                                    provider: pid,
                                                    provider_display: display,
                                                    models,
                                                })
                                                .ignore();
                                        }
                                        Err(e) => {
                                            gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to fetch models: {:#}", e),
                                                details: Some(
                                                    rustyclaw_core::error_details::render_extended(
                                                        &e,
                                                    ),
                                                ),
                                            }).ignore();
                                        }
                                    }
                                });
                            }
                            rustyclaw_core::providers::AuthMethod::ApiKey
                            | rustyclaw_core::providers::AuthMethod::OptionalApiKey => {
                                // Check if we already have a key stored
                                let has_key = def.secret_key.and_then(|sk| {
                                    secrets_manager
                                        .get_secret(sk, true)
                                        .ok()
                                        .flatten()
                                        .or_else(|| std::env::var(sk).ok())
                                });
                                let is_optional = def.auth_method
                                    == rustyclaw_core::providers::AuthMethod::OptionalApiKey;
                                if has_key.is_some() || is_optional {
                                    // Key exists, or key is optional — set provider and fetch models
                                    let existing_model =
                                        config.model.as_ref().and_then(|m| m.model.clone());
                                    config.model = Some(rustyclaw_core::config::ModelProvider {
                                        provider: provider_id.clone(),
                                        model: existing_model,
                                        base_url:
                                            rustyclaw_core::providers::base_url_override_for_switch(
                                                &provider_id,
                                                config.model.as_ref().map(|m| m.provider.as_str()),
                                                config
                                                    .model
                                                    .as_ref()
                                                    .and_then(|m| m.base_url.clone()),
                                            ),
                                    });
                                    crate::app::events::persist_config(config, &gw_tx);
                                    client
                                        .send(GatewayCommand::Reload)
                                        .await
                                        .context("sending Reload")
                                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                                    let display = def.display.to_string();
                                    let pid = provider_id.clone();
                                    let key = has_key;
                                    crate::app::events::emit(
                                        &gw_tx,
                                        GwEvent::FetchModelsLoading {
                                            provider: pid.clone(),
                                            provider_display: display.clone(),
                                        },
                                    );
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
                                                gw_tx2
                                                    .send(GwEvent::ShowModelSelector {
                                                        provider: pid,
                                                        provider_display: display,
                                                        models,
                                                    })
                                                    .ignore();
                                            }
                                            Err(e) => {
                                                gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to fetch models: {:#}", e),
                                                details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                            }).ignore();
                                            }
                                        }
                                    });
                                } else {
                                    // No key — prompt for one
                                    crate::app::events::emit(
                                        &gw_tx,
                                        GwEvent::PromptApiKey {
                                            provider: provider_id.clone(),
                                            provider_display: def.display.to_string(),
                                            help_url: def.help_url.unwrap_or("").to_string(),
                                            help_text: def.help_text.unwrap_or("").to_string(),
                                        },
                                    );
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
                                        base_url:
                                            rustyclaw_core::providers::base_url_override_for_switch(
                                                &provider_id,
                                                config.model.as_ref().map(|m| m.provider.as_str()),
                                                config
                                                    .model
                                                    .as_ref()
                                                    .and_then(|m| m.base_url.clone()),
                                            ),
                                    });
                                    crate::app::events::persist_config(config, &gw_tx);
                                    client
                                        .send(GatewayCommand::Reload)
                                        .await
                                        .context("sending Reload")
                                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                                    let display = def.display.to_string();
                                    let pid = provider_id.clone();
                                    let token = has_token;
                                    crate::app::events::emit(
                                        &gw_tx,
                                        GwEvent::FetchModelsLoading {
                                            provider: pid.clone(),
                                            provider_display: display.clone(),
                                        },
                                    );
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
                                                gw_tx2
                                                    .send(GwEvent::ShowModelSelector {
                                                        provider: pid,
                                                        provider_display: display,
                                                        models,
                                                    })
                                                    .ignore();
                                            }
                                            Err(e) => {
                                                gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to fetch models: {:#}", e),
                                                details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                            }).ignore();
                                            }
                                        }
                                    });
                                } else {
                                    // No token — start device flow
                                    if let Some(df_config) = def.device_flow {
                                        let pid = provider_id.clone();
                                        let display = def.display.to_string();
                                        let gw_tx2 = gw_tx.clone();
                                        crate::app::events::emit(
                                            &gw_tx,
                                            GwEvent::Info(format!(
                                                "Starting device flow for {}…",
                                                display
                                            )),
                                        );
                                        tokio::spawn(async move {
                                            match rustyclaw_core::providers::start_device_flow(
                                                df_config,
                                            )
                                            .await
                                            {
                                                Ok(auth_resp) => {
                                                    // A client-local flow,
                                                    // owned by no turn.
                                                    gw_tx2
                                                        .send(GwEvent::DeviceFlowCode {
                                                            owner:
                                                                crate::app::DeviceFlowOwner::Local,
                                                            provider: pid.clone(),
                                                            url: auth_resp.verification_uri.clone(),
                                                            code: auth_resp.user_code.clone(),
                                                        })
                                                        .ignore();
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
                                                            gw_tx2
                                                                .send(GwEvent::DeviceFlowDone(
                                                                crate::app::DeviceFlowOwner::Local,
                                                            )).ignore();
                                                            gw_tx2.send(GwEvent::error(
                                                                "Device flow timed out — please try again.".to_string(),
                                                            )).ignore();
                                                            break;
                                                        }
                                                        match rustyclaw_core::providers::poll_device_token(
                                                            df_config, &auth_resp.device_code,
                                                        ).await {
                                                            Ok(Some(token)) => {
                                                                gw_tx2.send(GwEvent::DeviceFlowDone(crate::app::DeviceFlowOwner::Local)).ignore();
                                                                gw_tx2.send(GwEvent::Success(format!(
                                                                    "✓ {} authenticated!", display
                                                                ))).ignore();
                                                                gw_tx2.send(GwEvent::DeviceFlowToken {
                                                                    provider: pid.clone(),
                                                                    token,
                                                                }).ignore();
                                                                break;
                                                            }
                                                            Ok(None) => {
                                                                // Still pending — continue polling
                                                            }
                                                            Err(e) => {
                                                                gw_tx2.send(GwEvent::DeviceFlowDone(crate::app::DeviceFlowOwner::Local)).ignore();
                                                                gw_tx2.send(GwEvent::Error {
                                                                    summary: format!("Device flow failed: {:#}", e),
                                                                    details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                                                }).ignore();
                                                                break;
                                                            }
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    gw_tx2.send(GwEvent::Error {
                                                summary: format!("Failed to start device flow: {:#}", e),
                                                details: Some(rustyclaw_core::error_details::render_extended(&e)),
                                            }).ignore();
                                                }
                                            }
                                        });
                                    } else {
                                        crate::app::events::emit(
                                            &gw_tx,
                                            GwEvent::error(
                                                "Device flow not configured for this provider."
                                                    .to_string(),
                                            ),
                                        );
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
                            crate::app::events::emit(
                                &gw_tx,
                                GwEvent::Success(format!(
                                    "✓ API key for {} stored securely.",
                                    display,
                                )),
                            );
                        }
                        Err(e) => {
                            crate::app::events::emit(
                                &gw_tx,
                                GwEvent::warning(format!(
                                    "Failed to store API key: {}. Key is set for this session only.",
                                    e,
                                )),
                            );
                        }
                    }
                    // Update config with the new provider
                    let existing_model = config.model.as_ref().and_then(|m| m.model.clone());
                    config.model = Some(rustyclaw_core::config::ModelProvider {
                        provider: provider.clone(),
                        model: existing_model,
                        base_url: rustyclaw_core::providers::base_url_override_for_switch(
                            &provider,
                            config.model.as_ref().map(|m| m.provider.as_str()),
                            config.model.as_ref().and_then(|m| m.base_url.clone()),
                        ),
                    });
                    crate::app::events::persist_config(config, &gw_tx);
                    // Reload gateway
                    client
                        .send(GatewayCommand::Reload)
                        .await
                        .context("sending Reload")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                    // Now fetch models
                    let pid = provider.clone();
                    crate::app::events::emit(
                        &gw_tx,
                        GwEvent::FetchModelsLoading {
                            provider: pid.clone(),
                            provider_display: display.clone(),
                        },
                    );
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
                                gw_tx2
                                    .send(GwEvent::ShowModelSelector {
                                        provider: pid,
                                        provider_display: display,
                                        models,
                                    })
                                    .ignore();
                            }
                            Err(e) => {
                                gw_tx2
                                    .send(GwEvent::Error {
                                        summary: format!("Failed to fetch models: {:#}", e),
                                        details: Some(
                                            rustyclaw_core::error_details::render_extended(&e),
                                        ),
                                    })
                                    .ignore();
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
                        crate::app::events::emit(
                            &gw_tx,
                            GwEvent::error(format!("Failed to save config: {}", e)),
                        );
                    } else {
                        let display =
                            rustyclaw_core::providers::display_name_for_provider(&provider);
                        crate::app::events::emit(
                            &gw_tx,
                            GwEvent::Info(format!(
                                "Model set to {} / {}. Reloading gateway…",
                                display, model,
                            )),
                        );
                        // Reload gateway so the new provider + model take effect
                        client
                            .send(GatewayCommand::Reload)
                            .await
                            .context("sending Reload")
                            .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                    }
                }
                Ok(UserInput::CancelProviderFlow) => {
                    // User cancelled — nothing to do
                }
                Ok(UserInput::EngineSelect(engine)) => {
                    client
                        .send(GatewayCommand::EngineModelList { engine })
                        .await
                        .context("sending EngineModelList")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::EngineAction { engine, action }) => {
                    client
                        .send(GatewayCommand::EngineAction { engine, action })
                        .await
                        .context("sending EngineAction")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                    // Refresh the list so status changes show up.
                    client
                        .send(GatewayCommand::EngineList)
                        .await
                        .context("sending EngineList")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
                }
                Ok(UserInput::EngineRefresh) => {
                    client
                        .send(GatewayCommand::EngineList)
                        .await
                        .context("sending EngineList")
                        .unwrap_or_else(|e| crate::app::events::report(&gw_tx, e));
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
                                gw_tx_pair
                                    .send(GwEvent::PairingSuccess { gateway_name })
                                    .ignore();
                            }
                            Err(e) => {
                                gw_tx_pair
                                    .send(GwEvent::PairingError(e.to_string()))
                                    .ignore();
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
        render_handle.await.ignore();
        Ok(())
    }
}
