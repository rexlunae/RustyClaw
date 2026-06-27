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

use rustyclaw_view::anyhow::Result;
use rustyclaw_view::{tokio, tracing, url};
use std::sync::mpsc as sync_mpsc;

use rustyclaw_core::commands::{CommandContext, CommandResponse, handle_command};
use rustyclaw_core::config::Config;
use rustyclaw_core::gateway::{GatewayClient, GatewayCommand};
use rustyclaw_core::secrets::SecretsManager;
use rustyclaw_core::skills::SkillManager;
use rustyclaw_core::soul::SoulManager;
use rustyclaw_view::{PromptAttachment, build_prompt_with_attachments};

use crate::gateway_client;

use super::GwEvent;
use super::command_action::handle_command_action;
use super::tui_component;
use super::tui_component::TuiRoot;

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
    /// User responded to a credential request
    CredentialResponse {
        id: String,
        dismissed: bool,
        value: Option<String>,
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
                let _ = gw_tx.send(GwEvent::error(format!("SSH connection failed: {}", e)));
                let _ = gw_tx.send(GwEvent::Disconnected(format!("Failed to connect: {}", e)));
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
            while let Some(event) = client_reader.recv().await {
                if let Some(ev) = gateway_client::gateway_event_to_gw_event(event) {
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

        loop {
            // Poll user_rx (non-blocking on tokio side)
            match user_rx.try_recv() {
                Ok(UserInput::Chat(text)) => {
                    let prompt = build_prompt_with_attachments(&text, &prompt_attachments);
                    prompt_attachments.clear();
                    let _ = gw_tx.send(GwEvent::PromptAttachmentsChanged {
                        attachments: prompt_attachments.clone(),
                    });
                    let _ = client.send(GatewayCommand::Chat { message: prompt }).await;
                }
                Ok(UserInput::AuthResponse(code)) => {
                    let _ = client.send(GatewayCommand::Auth { code }).await;
                }
                Ok(UserInput::ToolApprovalResponse { id, approved }) => {
                    let _ = client
                        .send(GatewayCommand::ToolApprove { id, approved })
                        .await;
                }
                Ok(UserInput::VaultUnlock(password)) => {
                    // Unlock locally so /secrets can read the vault
                    secrets_manager.set_password(password.clone());
                    let _ = client.send(GatewayCommand::VaultUnlock { password }).await;
                }
                Ok(UserInput::UserPromptResponse {
                    id,
                    dismissed,
                    value,
                }) => {
                    let _ = client
                        .send(GatewayCommand::UserPromptResponse {
                            id,
                            dismissed,
                            value,
                        })
                        .await;
                }
                Ok(UserInput::CredentialResponse {
                    id,
                    dismissed,
                    value,
                }) => {
                    let _ = client
                        .send(GatewayCommand::CredentialResponse {
                            id,
                            dismissed,
                            value,
                        })
                        .await;
                }
                Ok(UserInput::CancelCurrentRequest) => {
                    let _ = client.send(GatewayCommand::Cancel).await;
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
                        let _ = skill_manager.set_skill_enabled(&name, new_enabled);
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
                            rustyclaw_view::ToolPermInfoData {
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
                    let _ = client
                        .send(GatewayCommand::SecretsSetPolicy {
                            name,
                            policy: next_policy.to_string(),
                            skills: vec![],
                        })
                        .await;
                }
                Ok(UserInput::DeleteSecret { name }) => {
                    let _ = client
                        .send(GatewayCommand::SecretsDeleteCredential { name })
                        .await;
                }
                Ok(UserInput::AddSecret { name, value }) => {
                    let _ = client
                        .send(GatewayCommand::SecretsStore { key: name, value })
                        .await;
                }
                Ok(UserInput::RefreshSecrets) => {
                    let _ = client.send(GatewayCommand::SecretsList).await;
                }
                Ok(UserInput::RefreshTasks) => {
                    let _ = client
                        .send(GatewayCommand::TasksRequest { session: None })
                        .await;
                }
                Ok(UserInput::ThreadSwitch(thread_id)) => {
                    let _ = client
                        .send(GatewayCommand::ThreadSwitch { thread_id })
                        .await;
                }
                Ok(UserInput::RequestThreadHistory(thread_id)) => {
                    let _ = client
                        .send(GatewayCommand::ThreadHistoryRequest { thread_id })
                        .await;
                }
                Ok(UserInput::RefreshThreads) => {
                    let _ = client.send(GatewayCommand::ThreadList).await;
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
                                    base_url: config
                                        .model
                                        .as_ref()
                                        .and_then(|m| m.base_url.clone()),
                                });
                                let _ = config.save(None);
                                // Reload gateway
                                let _ = client.send(GatewayCommand::Reload).await;
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
                                        base_url: config
                                            .model
                                            .as_ref()
                                            .and_then(|m| m.base_url.clone()),
                                    });
                                    let _ = config.save(None);
                                    let _ = client.send(GatewayCommand::Reload).await;
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
                                    let _ = client.send(GatewayCommand::Reload).await;
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
                    let _ = client.send(GatewayCommand::Reload).await;
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
                        let _ = client.send(GatewayCommand::Reload).await;
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
