use crate::action::Action;
use crate::commands::{handle_command, CommandAction, CommandContext};
use crate::config::Config;
use crate::dialogs::{
    self, ApiKeyDialogState, AuthPromptState, CredDialogOption, CredentialDialogState,
    FetchModelsLoading, ModelSelectorState, PolicyPickerState, ProviderSelectorState,
    SecretViewerState, TotpDialogPhase, TotpDialogState, VaultUnlockPromptState, SPINNER_FRAMES,
};
use crate::gateway::{ChatMessage, ClientFrame, ClientFrameType, ClientPayload};
use crate::pages::hatching::Hatching;
use crate::pages::home::Home;
use crate::pages::Page;
use crate::panes::footer::FooterPane;
use crate::panes::header::HeaderPane;
use crate::panes::{DisplayMessage, GatewayStatus, InputMode, Pane, PaneState};
use crate::providers;
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;
use crate::soul::SoulManager;
use crate::tui::{Event, EventResponse, Tui};

use anyhow::Result;
use ratatui::prelude::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::handlers::gateway::WsSink as AppWsSink;

type WsSink = AppWsSink;

pub struct App {
    pub state: crate::app::state::SharedState,
    pages: Vec<Box<dyn Page>>,
    active_page: usize,
    header: HeaderPane,
    footer: FooterPane,
    pub should_quit: bool,
    pub should_suspend: bool,
    #[allow(dead_code)]
    pub action_tx: mpsc::UnboundedSender<Action>,
    pub action_rx: mpsc::UnboundedReceiver<Action>,
    pub ws_sink: Option<WsSink>,
    pub reader_task: Option<JoinHandle<()>>,
    pub show_skills_dialog: bool,
    pub show_secrets_dialog: bool,
    pub secrets_scroll: usize,
    pub api_key_dialog: Option<ApiKeyDialogState>,
    pub model_selector: Option<ModelSelectorState>,
    pub fetch_loading: Option<FetchModelsLoading>,
    pub device_flow_loading: Option<FetchModelsLoading>,
    pub chat_loading_tick: Option<usize>,
    pub streaming_response: Option<String>,
    pub provider_selector: Option<ProviderSelectorState>,
    pub credential_dialog: Option<CredentialDialogState>,
    pub totp_dialog: Option<TotpDialogState>,
    pub auth_prompt: Option<AuthPromptState>,
    pub vault_unlock_prompt: Option<VaultUnlockPromptState>,
    pub secret_viewer: Option<SecretViewerState>,
    pub policy_picker: Option<PolicyPickerState>,
    pub hatching_page: Option<Hatching>,
    pub showing_hatching: bool,
    pub deferred_vault_password: Option<String>,
    pub cached_secrets: Vec<serde_json::Value>,
    pub pending_secret_key: Option<String>,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::locked(config.credentials_dir());
        Self::build(config, secrets_manager)
    }

    pub fn with_password(config: Config, password: String) -> Result<Self> {
        let creds_dir = config.credentials_dir();
        let mut app = Self::build(config, SecretsManager::locked(creds_dir))?;
        app.deferred_vault_password = Some(password);
        Ok(app)
    }

    pub fn new_locked(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::locked(config.credentials_dir());
        Self::build(config, secrets_manager)
    }

    pub fn set_deferred_vault_password(&mut self, password: String) {
        self.deferred_vault_password = Some(password);
    }

    fn build(config: Config, mut secrets_manager: SecretsManager) -> Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();

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

        let needs_hatching = soul_manager.needs_hatching();

        let _ = soul_manager.load();

        let mut home = Home::new()?;
        home.register_action_handler(action_tx.clone())?;
        let pages: Vec<Box<dyn Page>> = vec![Box::new(home)];

        let (hatching_page, showing_hatching) = if needs_hatching {
            let agent_name = config.agent_name.clone();
            (Some(Hatching::new(&agent_name)?), true)
        } else {
            (None, false)
        };

        let history_path = Self::history_path(&config);
        let conversation_history = Self::load_history(&history_path, &soul_manager, &skill_manager);

        let mut messages =
            vec![DisplayMessage::info("Welcome to RustyClaw! Type /help for commands.")];
        let prior_turns: usize = conversation_history
            .iter()
            .filter(|m| m.role != "system")
            .count();
        if prior_turns > 0 {
            messages.push(DisplayMessage::info(format!(
                "Restored {} turns from previous conversation. Use /clear to start fresh.",
                prior_turns,
            )));
            for msg in &conversation_history {
                match msg.role.as_str() {
                    "user" => messages.push(DisplayMessage::user(&msg.content)),
                    "assistant" => messages.push(DisplayMessage::assistant(&msg.content)),
                    _ => {}
                }
            }
        }

        let gateway_status = GatewayStatus::Disconnected;

        let state = crate::app::state::SharedState {
            config,
            messages,
            conversation_history,
            input_mode: InputMode::Normal,
            secrets_manager,
            skill_manager,
            soul_manager,
            gateway_status,
            loading_line: None,
            streaming_started: None,
        };

        Ok(Self {
            state,
            pages,
            active_page: 0,
            header: HeaderPane::new(),
            footer: FooterPane::new(),
            should_quit: false,
            should_suspend: false,
            action_tx,
            action_rx,
            ws_sink: None,
            reader_task: None,
            show_skills_dialog: false,
            show_secrets_dialog: false,
            secrets_scroll: 0,
            api_key_dialog: None,
            model_selector: None,
            fetch_loading: None,
            device_flow_loading: None,
            chat_loading_tick: None,
            streaming_response: None,
            provider_selector: None,
            credential_dialog: None,
            totp_dialog: None,
            auth_prompt: None,
            vault_unlock_prompt: None,
            secret_viewer: None,
            policy_picker: None,
            hatching_page,
            showing_hatching,
            deferred_vault_password: None,
            cached_secrets: Vec::new(),
            pending_secret_key: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut tui = Tui::new()?;
        tui.enter()?;

        {
            let ps = self.state.pane_state();
            for page in &mut self.pages {
                page.init(&ps)?;
            }
            if let Some(ref mut hatching) = self.hatching_page {
                hatching.init(&ps)?;
            }
        }
        self.pages[self.active_page].focus()?;

        self.start_gateway().await;

        loop {
            if let Some(event) = tui.next().await {
                let mut action = match &event {
                    Event::Render => None,
                    Event::Tick => Some(Action::Tick),
                    Event::Resize(w, h) => Some(Action::Resize(*w, *h)),
                    Event::Quit => Some(Action::Quit),
                    _ => {
                        if self.showing_hatching {
                            if let Some(ref mut hatching) = self.hatching_page {
                                let mut ps = self.state.pane_state();
                                match hatching.handle_events(event.clone(), &mut ps)? {
                                    Some(EventResponse::Stop(a)) => Some(a),
                                    Some(EventResponse::Continue(_)) => None,
                                    None => None,
                                }
                            } else {
                                None
                            }
                        } else if self.streaming_response.is_some() {
                            if let Event::Key(key) = &event {
                                if key.code == crossterm::event::KeyCode::Esc {
                                    self.send_cancel().await;
                                    self.state.messages.push(DisplayMessage::info(
                                        "Cancelling tool loop…",
                                    ));
                                    continue;
                                }
                            }
                            None
                        } else if self.fetch_loading.is_some() || self.device_flow_loading.is_some() {
                            if let Event::Key(key) = &event {
                                if key.code == crossterm::event::KeyCode::Esc {
                                    if self.device_flow_loading.is_some() {
                                        self.device_flow_loading = None;
                                        self.state.loading_line = None;
                                        self.state.messages.push(DisplayMessage::info(
                                            "Device flow authentication cancelled.",
                                        ));
                                    } else {
                                        self.fetch_loading = None;
                                        self.state.loading_line = None;
                                        self.state
                                            .messages
                                            .push(DisplayMessage::info("Model fetch cancelled."));
                                    }
                                    continue;
                                }
                            }
                            None
                        } else if self.api_key_dialog.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_api_key_dialog_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.provider_selector.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_provider_selector_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.model_selector.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_model_selector_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.policy_picker.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_policy_picker_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.secret_viewer.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_secret_viewer_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.credential_dialog.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_credential_dialog_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.totp_dialog.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_totp_dialog_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.auth_prompt.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_auth_prompt_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.vault_unlock_prompt.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_vault_unlock_prompt_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        } else if self.show_skills_dialog {
                            if let Event::Key(key) = &event {
                                match key.code {
                                    crossterm::event::KeyCode::Esc
                                    | crossterm::event::KeyCode::Enter
                                    | crossterm::event::KeyCode::Char('q') => {
                                        self.show_skills_dialog = false;
                                        Some(Action::Noop)
                                    }
                                    _ => Some(Action::Noop),
                                }
                            } else {
                                None
                            }
                        } else if self.show_secrets_dialog {
                            if let Event::Key(key) = &event {
                                match key.code {
                                    crossterm::event::KeyCode::Esc
                                    | crossterm::event::KeyCode::Char('q') => {
                                        self.show_secrets_dialog = false;
                                        Some(Action::Noop)
                                    }
                                    crossterm::event::KeyCode::Char('j')
                                    | crossterm::event::KeyCode::Down => {
                                        let max = self.cached_secrets.len().saturating_sub(1);
                                        if self.secrets_scroll < max {
                                            self.secrets_scroll += 1;
                                        }
                                        Some(Action::Noop)
                                    }
                                    crossterm::event::KeyCode::Char('k')
                                    | crossterm::event::KeyCode::Up => {
                                        self.secrets_scroll = self.secrets_scroll.saturating_sub(1);
                                        Some(Action::Noop)
                                    }
                                    crossterm::event::KeyCode::Enter => {
                                        if let Some(entry) = self.cached_secrets.get(self.secrets_scroll) {
                                            let name = entry.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                            let disabled = entry.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false);
                                            let policy = entry.get("policy")
                                                .and_then(|p| serde_json::from_value::<crate::secrets::AccessPolicy>(p.clone()).ok())
                                                .map(|p| p.badge().to_string())
                                                .unwrap_or_default();
                                            self.show_secrets_dialog = false;
                                            Some(Action::ShowCredentialDialog {
                                                name,
                                                disabled,
                                                policy,
                                            })
                                        } else {
                                            Some(Action::Noop)
                                        }
                                    }
                                    _ => Some(Action::Noop),
                                }
                            } else {
                                None
                            }
                        } else {
                            let mut ps = self.state.pane_state();
                            match self.footer.handle_events(event.clone(), &mut ps)? {
                                Some(EventResponse::Stop(a)) => {
                                    self.state.input_mode = ps.input_mode;
                                    Some(a)
                                }
                                _ => {
                                    self.state.input_mode = ps.input_mode;
                                    let mut ps2 = self.state.pane_state();
                                    match self.pages[self.active_page]
                                        .handle_events(event.clone(), &mut ps2)?
                                    {
                                        Some(EventResponse::Stop(a)) => Some(a),
                                        Some(EventResponse::Continue(_)) => None,
                                        None => None,
                                    }
                                }
                            }
                        }
                    }
                };

                while let Some(act) = action {
                    action = self.dispatch_action(act).await?;
                }

                while let Ok(act) = self.action_rx.try_recv() {
                    let mut a = Some(act);
                    while let Some(act) = a {
                        a = self.dispatch_action(act).await?;
                    }
                }

                if matches!(event, Event::Render) {
                    self.draw(&mut tui)?;
                }

                if self.should_quit {
                    tui.stop()?;
                    break;
                }
            }
        }

        tui.exit()?;
        Ok(())
    }

    pub async fn dispatch_action(&mut self, action: Action) -> Result<Option<Action>> {
        match &action {
            Action::Quit => {
                self.should_quit = true;
                return Ok(None);
            }
            Action::Suspend => {
                self.should_suspend = true;
                return Ok(None);
            }
            Action::Resume => {
                self.should_suspend = false;
                return Ok(None);
            }
            Action::InputSubmit(text) => {
                return self.handle_input_submit(text.clone());
            }
            Action::ReconnectGateway => {
                self.start_gateway().await;
                return Ok(Some(Action::Update));
            }
            Action::DisconnectGateway => {
                self.stop_gateway().await;
                return Ok(Some(Action::Update));
            }
            Action::RestartGateway => {
                self.restart_gateway().await;
                return Ok(Some(Action::Update));
            }
            Action::SendToGateway(text) => {
                self.send_to_gateway(text.clone()).await;
                return Ok(None);
            }
            Action::GatewayAuthChallenge => {
                self.auth_prompt = Some(AuthPromptState {
                    input: String::new(),
                });
                return Ok(Some(Action::Update));
            }
            Action::GatewayAuthResponse(code) => {
                let frame = ClientFrame {
                    frame_type: ClientFrameType::AuthResponse,
                    payload: ClientPayload::AuthResponse { code: code.clone() },
                };
                self.send_frame(frame).await;
                self.state
                    .messages
                    .push(DisplayMessage::info("Sent authentication code…"));
                return Ok(Some(Action::Update));
            }
            Action::GatewayVaultLocked => {
                self.vault_unlock_prompt = Some(VaultUnlockPromptState {
                    input: String::new(),
                });
                return Ok(Some(Action::Update));
            }
            Action::GatewayUnlockVault(password) => {
                let frame = serde_json::json!({
                    "type": "unlock_vault",
                    "password": password,
                });
                return Ok(Some(Action::SendToGateway(frame.to_string())));
            }
            Action::GatewayDisconnected(reason) => {
                self.state.gateway_status = GatewayStatus::Disconnected;
                self.chat_loading_tick = None;
                self.state.loading_line = None;
                self.streaming_response = None;
                self.state.streaming_started = None;
                self.state
                    .messages
                    .push(DisplayMessage::warning(format!(
                        "Gateway disconnected: {}",
                        reason
                    )));
                self.ws_sink = None;
                self.reader_task = None;
                return Ok(Some(Action::Update));
            }
            Action::Tick => {
                if self.showing_hatching {
                    if let Some(ref mut hatching) = self.hatching_page {
                        let mut ps = self.state.pane_state();
                        if let Ok(Some(hatching_action)) = hatching.update(Action::Tick, &mut ps) {
                            return Ok(Some(hatching_action));
                        }
                    }
                }

                if let Some(ref mut loading) = self.fetch_loading {
                    loading.tick += 1;
                    let spinner = SPINNER_FRAMES[loading.tick % SPINNER_FRAMES.len()];
                    self.state.loading_line = Some(format!(
                        "  {} Fetching models from {}…",
                        spinner, loading.display,
                    ));
                } else if let Some(ref mut loading) = self.device_flow_loading {
                    loading.tick += 1;
                    let spinner = SPINNER_FRAMES[loading.tick % SPINNER_FRAMES.len()];
                    self.state.loading_line = Some(format!(
                        "  {} Waiting for {} authorization…",
                        spinner, loading.display,
                    ));
                } else if let Some(ref mut tick) = self.chat_loading_tick {
                    *tick += 1;
                    let spinner = SPINNER_FRAMES[*tick % SPINNER_FRAMES.len()];
                    self.state.loading_line =
                        Some(format!("  {} Waiting for model response\u{2026}", spinner,));
                }
            }
            Action::ShowSkills => {
                self.show_skills_dialog = !self.show_skills_dialog;
                return Ok(None);
            }
            Action::ShowSecrets => {
                self.show_secrets_dialog = !self.show_secrets_dialog;
                self.secrets_scroll = 0;
                return Ok(None);
            }
            Action::ShowProviderSelector => {
                self.provider_selector = Some(dialogs::open_provider_selector());
                return Ok(None);
            }
            Action::SetProvider(provider) => {
                return self.handle_set_provider(provider.clone());
            }
            Action::PromptApiKey(provider) => {
                self.api_key_dialog =
                    dialogs::open_api_key_dialog(provider, &mut self.state.messages);
                return Ok(None);
            }
            Action::ConfirmStoreSecret { provider, key } => {
                let secret_key = providers::secret_key_for_provider(provider).unwrap_or("API_KEY");
                let display = providers::display_name_for_provider(provider).to_string();
                let frame = serde_json::json!({
                    "type": "secrets_store",
                    "key": secret_key,
                    "value": key,
                });
                return Ok(Some(Action::SendToGateway(frame.to_string())));
            }
            Action::FetchModels(provider) => {
                dialogs::spawn_fetch_models(
                    provider,
                    &mut self.state.secrets_manager,
                    self.state.config.model.as_ref(),
                    &mut self.state.messages,
                    &mut self.state.loading_line,
                    &mut self.fetch_loading,
                    self.action_tx.clone(),
                );
                return Ok(None);
            }
            Action::FetchModelsFailed(msg) => {
                self.fetch_loading = None;
                self.state.loading_line = None;
                self.state.messages.push(DisplayMessage::error(msg));
                return Ok(Some(Action::Update));
            }
            Action::ShowModelSelector { provider, models } => {
                self.fetch_loading = None;
                self.state.loading_line = None;
                self.model_selector = Some(dialogs::open_model_selector(provider, models.clone()));
                return Ok(None);
            }
            Action::StartDeviceFlow(provider) => {
                self.spawn_device_flow(provider.clone());
                return Ok(None);
            }
            Action::DeviceFlowCodeReady { url, code } => {
                self.state
                    .messages
                    .push(DisplayMessage::info("Open this URL in your browser:"));
                self.state
                    .messages
                    .push(DisplayMessage::info(format!("  ➜  {}", url)));
                self.state
                    .messages
                    .push(DisplayMessage::info(format!("Then enter this code:  {}", code)));
                return Ok(Some(Action::Update));
            }
            Action::DeviceFlowAuthenticated { provider, token } => {
                self.device_flow_loading = None;
                self.state.loading_line = None;
                let secret_key =
                    providers::secret_key_for_provider(provider).unwrap_or("COPILOT_TOKEN");
                let display = providers::display_name_for_provider(provider).to_string();
                let frame = serde_json::json!({
                    "type": "secrets_store",
                    "key": secret_key,
                    "value": token,
                });
                self.state.messages.push(DisplayMessage::success(format!(
                    "{} authenticated successfully. Storing token…",
                    display,
                )));
                return Ok(Some(Action::SendToGateway(frame.to_string())));
            }
            Action::DeviceFlowFailed(msg) => {
                self.device_flow_loading = None;
                self.state.loading_line = None;
                self.state.messages.push(DisplayMessage::error(msg));
                return Ok(Some(Action::Update));
            }
            Action::ShowCredentialDialog { name, disabled, .. } => {
                let has_totp = self.state.config.totp_enabled;
                let current_policy = self.cached_secrets.iter()
                    .find(|e| e.get("name").and_then(|n| n.as_str()) == Some(name))
                    .and_then(|e| e.get("policy"))
                    .and_then(|p| serde_json::from_value::<crate::secrets::AccessPolicy>(p.clone()).ok())
                    .unwrap_or_default();
                self.credential_dialog = Some(CredentialDialogState {
                    name: name.clone(),
                    disabled: *disabled,
                    has_totp,
                    current_policy,
                    selected: CredDialogOption::ToggleDisable,
                });
                return Ok(None);
            }
            Action::ShowTotpSetup => {
                if self.state.config.totp_enabled {
                    self.totp_dialog = Some(TotpDialogState {
                        phase: TotpDialogPhase::AlreadyConfigured,
                    });
                } else {
                    self.send_secrets_setup_totp().await;
                }
                return Ok(None);
            }
            Action::CloseHatching => {
                self.showing_hatching = false;
                self.hatching_page = None;
                return Ok(None);
            }
            Action::BeginHatchingExchange => {
                let messages = self.hatching_page.as_ref().map(|h| h.chat_messages());
                if let Some(msgs) = messages {
                    self.send_chat(msgs).await;
                }
                return Ok(Some(Action::Update));
            }
            Action::FinishHatching(soul_content) => {
                let content = soul_content.clone();
                if let Err(e) = self.state.soul_manager.set_content(content) {
                    self.state
                        .messages
                        .push(DisplayMessage::error(format!("Failed to save SOUL.md: {}", e)));
                }
                self.showing_hatching = false;
                self.hatching_page = None;
                return Ok(Some(Action::Update));
            }
            // ── Forward binary-protocol frame actions to handle_action ──
            //
            // When the client receives binary WebSocket frames, the reader
            // loop decodes them into specific Action variants (GatewayChunk,
            // GatewayResponseDone, etc.) and sends them here directly.
            // Route them to handle_action() in the gateway handler, which
            // contains the actual processing logic for these frame types.
            Action::GatewayChunk(_)
            | Action::GatewayResponseDone
            | Action::GatewayStreamStart
            | Action::GatewayThinkingStart
            | Action::GatewayThinkingDelta
            | Action::GatewayThinkingEnd
            | Action::GatewayToolCall { .. }
            | Action::GatewayToolResult { .. }
            | Action::GatewayAuthenticated
            | Action::GatewayVaultUnlocked
            | Action::Info(_)
            | Action::Success(_)
            | Action::Warning(_)
            | Action::Error(_)
            | Action::SecretsListResult { .. }
            | Action::SecretsGetResult { .. }
            | Action::SecretsStoreResult { .. }
            | Action::SecretsPeekResult { .. }
            | Action::SecretsSetPolicyResult { .. }
            | Action::SecretsSetDisabledResult { .. }
            | Action::SecretsDeleteCredentialResult { .. }
            | Action::SecretsHasTotpResult { .. }
            | Action::SecretsSetupTotpResult { .. }
            | Action::SecretsVerifyTotpResult { .. }
            | Action::SecretsRemoveTotpResult { .. } => {
                return self.handle_action(action).await;
            }
            _ => {}
        }

        {
            let mut ps = self.state.pane_state();
            self.header.update(action.clone(), &mut ps)?;
            self.state.input_mode = ps.input_mode;
        }

        let footer_follow = {
            let mut ps = self.state.pane_state();
            let r = self.footer.update(action.clone(), &mut ps)?;
            self.state.input_mode = ps.input_mode;
            r
        };

        let page_follow = {
            let mut ps = self.state.pane_state();
            let r = self.pages[self.active_page].update(action, &mut ps)?;
            self.state.input_mode = ps.input_mode;
            r
        };

        Ok(footer_follow.or(page_follow))
    }

    pub fn handle_input_submit(&mut self, text: String) -> Result<Option<Action>> {
        if text.is_empty() {
            return Ok(None);
        }

        if text.starts_with('/') {
            let mut context = CommandContext {
                secrets_manager: &mut self.state.secrets_manager,
                skill_manager: &mut self.state.skill_manager,
                config: &mut self.state.config,
            };

            let response = handle_command(&text, &mut context);

            match response.action {
                CommandAction::Quit => {
                    self.should_quit = true;
                    return Ok(None);
                }
                CommandAction::ClearMessages => {
                    self.state.messages.clear();
                    self.clear_history();
                    for msg in response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                }
                CommandAction::GatewayStart => {
                    for msg in response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                    return Ok(Some(Action::ReconnectGateway));
                }
                CommandAction::GatewayStop => {
                    for msg in response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                    return Ok(Some(Action::DisconnectGateway));
                }
                CommandAction::GatewayRestart => {
                    for msg in response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                    return Ok(Some(Action::RestartGateway));
                }
                CommandAction::GatewayInfo => {
                    let url_display = self
                        .state
                        .config
                        .gateway_url
                        .as_deref()
                        .unwrap_or("(none)");
                    self.state.messages.push(DisplayMessage::system(format!(
                        "Gateway: {}  Status: {}",
                        url_display,
                        self.state.gateway_status.label()
                    )));
                }
                CommandAction::GatewayReload => {
                    for msg in response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                    let frame = serde_json::json!({ "type": "reload" });
                    return Ok(Some(Action::SendToGateway(frame.to_string())));
                }
                CommandAction::SetProvider(ref provider) => {
                    for msg in &response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                    return Ok(Some(Action::SetProvider(provider.clone())));
                }
                CommandAction::SetModel(ref model) => {
                    for msg in &response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                    let model_cfg = self.state.config.model.get_or_insert_with(|| {
                        crate::config::ModelProvider {
                            provider: "anthropic".into(),
                            model: None,
                            base_url: None,
                        }
                    });
                    model_cfg.model = Some(model.clone());
                    if let Err(e) = self.state.config.save(None) {
                        self.state
                            .messages
                            .push(DisplayMessage::error(format!("Failed to save config: {}", e)));
                    } else {
                        self.state
                            .messages
                            .push(DisplayMessage::success(format!("Model set to {}.", model)));
                    }
                    return Ok(Some(Action::RestartGateway));
                }
                CommandAction::ShowSkills => {
                    return Ok(Some(Action::ShowSkills));
                }
                CommandAction::ShowSecrets => {
                    return Ok(Some(Action::ShowSecrets));
                }
                CommandAction::ShowProviderSelector => {
                    return Ok(Some(Action::ShowProviderSelector));
                }
                CommandAction::Download(ref media_id, ref dest_path) => {
                    let media_ref = self.find_media_ref(media_id);
                    match media_ref {
                        Some(m) => {
                            match self.download_media(&m, dest_path.as_deref()) {
                                Ok(path) => {
                                    self.state.messages.push(DisplayMessage::info(
                                        format!("Downloaded to: {}", path)
                                    ));
                                }
                                Err(e) => {
                                    self.state.messages.push(DisplayMessage::error(
                                        format!("Download failed: {}", e)
                                    ));
                                }
                            }
                        }
                        None => {
                            self.state.messages.push(DisplayMessage::error(
                                format!("Media not found: {}", media_id)
                            ));
                        }
                    }
                }
                CommandAction::None => {
                    for msg in response.messages {
                        self.state.messages.push(DisplayMessage::info(msg));
                    }
                }
            }

            Ok(Some(Action::TimedStatusLine(text, 3)))
        } else {
            self.state.messages.push(DisplayMessage::user(&text));
            if matches!(
                self.state.gateway_status,
                GatewayStatus::Connected | GatewayStatus::ModelReady
            ) && self.ws_sink.is_some()
            {
                self.state.conversation_history.push(ChatMessage::text("user", &text));
                self.save_history();

                let chat_json = serde_json::json!({
                    "type": "chat",
                    "messages": self.state.conversation_history,
                })
                .to_string();
                self.chat_loading_tick = Some(0);
                let spinner = SPINNER_FRAMES[0];
                self.state.loading_line =
                    Some(format!("  {} Waiting for model response\u{2026}", spinner,));
                return Ok(Some(Action::SendToGateway(chat_json)));
            }
            self.state
                .messages
                .push(DisplayMessage::warning("Gateway not connected — use /gateway start"));
            Ok(Some(Action::Update))
        }
    }

    pub fn history_path(config: &Config) -> std::path::PathBuf {
        config
            .settings_dir
            .join("conversations")
            .join("current.json")
    }

    fn system_message(soul: &SoulManager, skill_manager: &SkillManager) -> Option<ChatMessage> {
        let mut content = String::new();

        if let Some(soul_text) = soul.get_content() {
            content.push_str(soul_text);
        }

        let skills_context = skill_manager.generate_prompt_context();
        if !skills_context.is_empty() {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&skills_context);
        }

        if content.is_empty() {
            None
        } else {
            Some(ChatMessage::text("system", &content))
        }
    }

    fn load_history(
        path: &std::path::Path,
        soul: &SoulManager,
        skill_manager: &SkillManager,
    ) -> Vec<ChatMessage> {
        let mut history = Vec::new();

        if let Some(sys) = Self::system_message(soul, skill_manager) {
            history.push(sys);
        }

        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(turns) = serde_json::from_str::<Vec<ChatMessage>>(&data) {
                history.extend(turns);
            }
        }

        history
    }

    pub fn save_history(&self) {
        let path = Self::history_path(&self.state.config);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let turns: Vec<&ChatMessage> = self
            .state
            .conversation_history
            .iter()
            .filter(|m| m.role != "system")
            .collect();

        if let Ok(json) = serde_json::to_string_pretty(&turns) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn clear_history(&mut self) {
        self.state.conversation_history.clear();
        if let Some(sys) =
            Self::system_message(&self.state.soul_manager, &self.state.skill_manager)
        {
            self.state.conversation_history.push(sys);
        }
        let path = Self::history_path(&self.state.config);
        let _ = std::fs::remove_file(&path);
    }

    fn find_media_ref(&self, media_id: &str) -> Option<crate::gateway::MediaRef> {
        for msg in &self.state.conversation_history {
            if let Some(media) = &msg.media {
                for m in media {
                    if m.id == media_id {
                        return Some(m.clone());
                    }
                }
            }
        }
        None
    }

    fn download_media(
        &self,
        media_ref: &crate::gateway::MediaRef,
        dest_path: Option<&str>,
    ) -> Result<String, String> {
        let dest = if let Some(path) = dest_path {
            let path = shellexpand::tilde(path);
            std::path::PathBuf::from(path.as_ref())
        } else {
            let downloads = dirs::download_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let filename = media_ref.filename.as_deref().unwrap_or(&media_ref.id);
            downloads.join(filename)
        };

        if let Some(local_path) = &media_ref.local_path {
            let src = std::path::Path::new(local_path);
            if src.exists() {
                std::fs::copy(src, &dest)
                    .map_err(|e| format!("Failed to copy: {}", e))?;
                return Ok(dest.to_string_lossy().to_string());
            }
        }

        if let Some(url) = &media_ref.url {
            let response = reqwest::blocking::get(url)
                .map_err(|e| format!("Failed to download: {}", e))?;

            let bytes = response.bytes()
                .map_err(|e| format!("Failed to read response: {}", e))?;

            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }

            std::fs::write(&dest, &bytes)
                .map_err(|e| format!("Failed to write file: {}", e))?;

            return Ok(dest.to_string_lossy().to_string());
        }

        Err("No local cache or URL available".to_string())
    }

    fn build_hatching_chat_request(
        &mut self,
        messages: Vec<crate::gateway::ChatMessage>,
    ) -> Vec<crate::gateway::ChatMessage> {
        messages
    }

    pub fn draw(&mut self, tui: &mut Tui) -> Result<()> {
        tui.draw(|frame| {
            let area = frame.area();

            let ps = PaneState {
                config: &self.state.config,
                secrets_manager: &mut self.state.secrets_manager,
                skill_manager: &mut self.state.skill_manager,
                soul_manager: &self.state.soul_manager,
                messages: &mut self.state.messages,
                input_mode: self.state.input_mode,
                gateway_status: self.state.gateway_status,
                loading_line: self.state.loading_line.clone(),
                streaming_started: self.state.streaming_started,
            };

            if self.showing_hatching {
                if let Some(ref mut hatching) = self.hatching_page {
                    let _ = hatching.draw(frame, area, &ps);
                }
                return;
            }

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .split(area);

            let _ = self.header.draw(frame, chunks[0], &ps);
            let _ = self.pages[self.active_page].draw(frame, chunks[1], &ps);
            let _ = self.footer.draw(frame, chunks[2], &ps);

            if self.show_skills_dialog {
                Self::draw_skills_dialog(frame, area, &ps);
            }

            if self.show_secrets_dialog {
                Self::draw_secrets_dialog(frame, area, &self.cached_secrets, &self.state.config, self.secrets_scroll);
            }

            if let Some(ref dialog) = self.api_key_dialog {
                dialogs::draw_api_key_dialog(frame, area, dialog);
            }

            if let Some(ref selector) = self.provider_selector {
                dialogs::draw_provider_selector_dialog(frame, area, selector);
            }

            if let Some(ref selector) = self.model_selector {
                dialogs::draw_model_selector_dialog(frame, area, selector);
            }

            if let Some(ref dialog) = self.credential_dialog {
                dialogs::draw_credential_dialog(frame, area, dialog);
            }

            if let Some(ref picker) = self.policy_picker {
                dialogs::draw_policy_picker(frame, area, picker);
            }

            if let Some(ref dialog) = self.totp_dialog {
                dialogs::draw_totp_dialog(frame, area, dialog);
            }

            if let Some(ref prompt) = self.auth_prompt {
                dialogs::draw_auth_prompt(frame, area, prompt);
            }

            if let Some(ref prompt) = self.vault_unlock_prompt {
                dialogs::draw_vault_unlock_prompt(frame, area, prompt);
            }

            if let Some(ref viewer) = self.secret_viewer {
                dialogs::draw_secret_viewer(frame, area, viewer);
            }
        })?;
        Ok(())
    }

    fn draw_skills_dialog(frame: &mut ratatui::Frame<'_>, area: Rect, state: &PaneState<'_>) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

        let skills = state.skill_manager.get_skills();
        let dialog_w = 60.min(area.width.saturating_sub(4));
        let dialog_h = ((skills.len() as u16) + 4)
            .min(area.height.saturating_sub(4))
            .max(6);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let items: Vec<ListItem> = skills
            .iter()
            .map(|s| {
                let (icon, icon_style) = if s.enabled {
                    ("✓", Style::default().fg(tp::SUCCESS))
                } else {
                    ("✗", Style::default().fg(tp::MUTED))
                };
                let name_style = if s.enabled {
                    Style::default().fg(tp::ACCENT_BRIGHT)
                } else {
                    Style::default().fg(tp::TEXT_DIM)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", icon), icon_style),
                    Span::styled(&s.name, name_style),
                    Span::styled(
                        format!(" — {}", s.description.as_deref().unwrap_or("No description")),
                        Style::default().fg(tp::MUTED),
                    ),
                ]))
            })
            .collect();

        let empty_msg = if skills.is_empty() {
            vec![ListItem::new(Span::styled(
                "  No skills loaded. Place .md files in your skills/ directory.",
                Style::default().fg(tp::TEXT_DIM),
            ))]
        } else {
            vec![]
        };

        let all_items = if items.is_empty() { empty_msg } else { items };

        let list = List::new(all_items)
            .block(
                Block::default()
                    .title(Span::styled(" Skills ", tp::title_focused()))
                    .title_bottom(
                        Line::from(Span::styled(
                            " Esc to close ",
                            Style::default().fg(tp::MUTED),
                        ))
                        .right_aligned(),
                    )
                    .borders(Borders::ALL)
                    .border_style(tp::focused_border())
                    .border_type(ratatui::widgets::BorderType::Rounded),
            )
            .style(Style::default().fg(tp::TEXT));

        frame.render_widget(list, dialog_area);
    }

    fn draw_secrets_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        cached_secrets: &[serde_json::Value],
        config: &Config,
        scroll_offset: usize,
    ) {
        use crate::secrets::AccessPolicy;
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

        let agent_access = config.agent_access;
        let has_totp = config.totp_enabled;

        let dialog_w = 70.min(area.width.saturating_sub(4));
        let dialog_h = ((cached_secrets.len() as u16) + 8)
            .min(area.height.saturating_sub(4))
            .max(8);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let mut items: Vec<ListItem> = Vec::new();

        let (access_label, access_style) = if agent_access {
            ("Enabled", Style::default().fg(tp::SUCCESS))
        } else {
            ("Disabled", Style::default().fg(tp::WARN))
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(" Agent Access: ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(access_label, access_style),
            Span::styled(
                format!(
                    "  │  {} cred{}",
                    cached_secrets.len(),
                    if cached_secrets.len() == 1 { "" } else { "s" }
                ),
                Style::default().fg(tp::TEXT_DIM),
            ),
            Span::styled("  │  2FA: ", Style::default().fg(tp::TEXT_DIM)),
            if has_totp {
                Span::styled("On", Style::default().fg(tp::SUCCESS))
            } else {
                Span::styled("Off", Style::default().fg(tp::MUTED))
            },
        ])));

        items.push(ListItem::new(""));

        if cached_secrets.is_empty() {
            items.push(ListItem::new(Span::styled(
                "  No credentials stored.",
                Style::default()
                    .fg(tp::MUTED)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            for (i, entry_val) in cached_secrets.iter().enumerate() {
                let highlight = i == scroll_offset;
                let name = entry_val.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let is_disabled = entry_val.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false);
                let label = entry_val.get("label").and_then(|l| l.as_str()).unwrap_or(name);
                let kind_str = entry_val.get("kind").and_then(|k| k.as_str()).unwrap_or("ApiKey");
                let description = entry_val.get("description").and_then(|d| d.as_str()).unwrap_or("");

                let policy: AccessPolicy = entry_val.get("policy")
                    .and_then(|p| serde_json::from_value(p.clone()).ok())
                    .unwrap_or_default();

                let row_style = if highlight {
                    tp::selected()
                } else if is_disabled {
                    Style::default().add_modifier(Modifier::DIM | Modifier::CROSSED_OUT)
                } else {
                    Style::default()
                };

                let icon = match kind_str {
                    "ApiKey" => "🔑",
                    "Credential" => "🪪",
                    "Token" => "🎟",
                    "SshKey" => "🔐",
                    "Certificate" => "📜",
                    _ => "🔑",
                };
                let kind_label = format!(" {:10} ", kind_str);

                let badge = if is_disabled {
                    Span::styled(
                        " OFF ",
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(tp::MUTED),
                    )
                } else {
                    let (badge_label, color) = match &policy {
                        AccessPolicy::Always => (" OPEN ", tp::SUCCESS),
                        AccessPolicy::WithApproval => (" ASK ", tp::WARN),
                        AccessPolicy::WithAuth => (" AUTH ", tp::ERROR),
                        AccessPolicy::SkillOnly(skills) if skills.is_empty() => {
                            (" LOCK ", tp::MUTED)
                        }
                        AccessPolicy::SkillOnly(_) => (" SKILL ", tp::INFO),
                    };
                    Span::styled(
                        badge_label,
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(color),
                    )
                };

                let detail = if description.is_empty() {
                    format!(" {}", name)
                } else {
                    format!(" {} — {}", name, description)
                };

                let label_style = if is_disabled {
                    Style::default()
                        .fg(tp::MUTED)
                        .add_modifier(Modifier::CROSSED_OUT)
                        .patch(row_style)
                } else {
                    Style::default().fg(tp::TEXT).patch(row_style)
                };
                let kind_style = if is_disabled {
                    Style::default().fg(tp::MUTED).patch(row_style)
                } else {
                    Style::default().fg(tp::ACCENT_BRIGHT).patch(row_style)
                };

                items.push(ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", icon), row_style),
                    Span::styled(kind_label, kind_style),
                    badge,
                    Span::styled(" ", row_style),
                    Span::styled(label.to_string(), label_style),
                    Span::styled(detail, Style::default().fg(tp::TEXT_DIM).patch(row_style)),
                ])));
            }
        }

        items.push(ListItem::new(""));

        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                " OPEN ",
                Style::default()
                    .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                    .bg(tp::SUCCESS),
            ),
            Span::styled(" anytime ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(
                " ASK ",
                Style::default()
                    .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                    .bg(tp::WARN),
            ),
            Span::styled(" per-use ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(
                " AUTH ",
                Style::default()
                    .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                    .bg(tp::ERROR),
            ),
            Span::styled(" re-auth ", Style::default().fg(tp::TEXT_DIM)),
            Span::styled(
                " SKILL ",
                Style::default()
                    .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                    .bg(tp::INFO),
            ),
            Span::styled(" skill-gated", Style::default().fg(tp::TEXT_DIM)),
        ])));

        let list = List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(" Secrets Vault ", tp::title_focused()))
                    .title_bottom(
                        Line::from(Span::styled(
                            " j/k↕  Enter→manage  Esc→close ",
                            Style::default().fg(tp::MUTED),
                        ))
                        .right_aligned(),
                    )
                    .borders(Borders::ALL)
                    .border_style(tp::focused_border())
                    .border_type(ratatui::widgets::BorderType::Rounded),
            )
            .style(Style::default().fg(tp::TEXT));

        frame.render_widget(list, dialog_area);
    }
}
