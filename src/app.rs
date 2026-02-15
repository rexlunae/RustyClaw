use std::path::PathBuf;
use std::io::Write;
use std::fs::OpenOptions;

use anyhow::Result;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use ratatui::prelude::*;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::action::Action;
use crate::commands::{handle_command, CommandAction, CommandContext};
use crate::config::Config;
use crate::daemon;
use crate::dialogs::{
    self, ApiKeyDialogState, AuthPromptState, CredDialogOption, CredentialDialogState,
    FetchModelsLoading, ModelSelectorState, PolicyPickerState, ProviderSelectorState,
    SecretViewerState, TotpDialogPhase, TotpDialogState, VaultUnlockPromptState, SPINNER_FRAMES,
};
use crate::gateway::ChatMessage;
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

/// Debug log to file (avoids TUI interference)
fn debug_log(msg: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/rustyclaw-tui.log")
    {
        let _ = writeln!(file, "[{}] {}", chrono::Utc::now().format("%H:%M:%S%.3f"), msg);
    }
}

/// Type alias for the client-side WebSocket write half.
type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Shared state that is separate from the UI components so we can borrow both
/// independently.
struct SharedState {
    config: Config,
    messages: Vec<DisplayMessage>,
    /// Wire-format conversation history sent to the model each turn.
    /// Includes system prompt (SOUL.md) and all user/assistant messages.
    conversation_history: Vec<ChatMessage>,
    input_mode: InputMode,
    secrets_manager: SecretsManager,
    skill_manager: SkillManager,
    soul_manager: SoulManager,
    gateway_status: GatewayStatus,
    /// Animated loading line shown at the bottom of the messages list.
    loading_line: Option<String>,
    /// When streaming started (for elapsed time display in footer).
    streaming_started: Option<std::time::Instant>,
}

impl SharedState {
    fn pane_state(&mut self) -> PaneState<'_> {
        PaneState {
            config: &self.config,
            secrets_manager: &mut self.secrets_manager,
            skill_manager: &mut self.skill_manager,
            soul_manager: &self.soul_manager,
            messages: &mut self.messages,
            input_mode: self.input_mode,
            gateway_status: self.gateway_status,
            loading_line: self.loading_line.clone(),
            streaming_started: self.streaming_started,
        }
    }
}

pub struct App {
    state: SharedState,
    pages: Vec<Box<dyn Page>>,
    active_page: usize,
    header: HeaderPane,
    footer: FooterPane,
    should_quit: bool,
    should_suspend: bool,
    #[allow(dead_code)]
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,
    /// Write half of the WebSocket client connection to the gateway
    ws_sink: Option<WsSink>,
    /// Handle for the background WebSocket reader task
    reader_task: Option<JoinHandle<()>>,
    /// Whether the skills dialog overlay is visible
    show_skills_dialog: bool,
    /// Whether the secrets dialog overlay is visible
    show_secrets_dialog: bool,
    /// Scroll offset in the secrets dialog
    secrets_scroll: usize,
    /// API-key dialog state
    api_key_dialog: Option<ApiKeyDialogState>,
    /// Model-selector dialog state
    model_selector: Option<ModelSelectorState>,
    /// Loading spinner shown while fetching models
    fetch_loading: Option<FetchModelsLoading>,
    /// Loading spinner shown during device flow authentication
    device_flow_loading: Option<FetchModelsLoading>,
    /// Tick counter for spinner while waiting for model response
    chat_loading_tick: Option<usize>,
    /// Accumulates streaming chunks from the gateway before finalising
    streaming_response: Option<String>,
    /// Provider-selector dialog state
    provider_selector: Option<ProviderSelectorState>,
    /// Credential-management dialog state
    credential_dialog: Option<CredentialDialogState>,
    /// 2FA (TOTP) setup dialog state
    totp_dialog: Option<TotpDialogState>,
    /// Gateway auth prompt (TOTP code entry for connecting)
    auth_prompt: Option<AuthPromptState>,
    /// Gateway vault unlock prompt (password entry)
    vault_unlock_prompt: Option<VaultUnlockPromptState>,
    /// Secret viewer dialog state
    secret_viewer: Option<SecretViewerState>,
    /// Policy-picker dialog state
    policy_picker: Option<PolicyPickerState>,
    /// Hatching page (shown on first run)
    hatching_page: Option<Hatching>,
    /// Whether we're currently showing the hatching animation
    showing_hatching: bool,
    /// Password to forward to gateway after connecting (from --password flag)
    deferred_vault_password: Option<String>,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::new(config.credentials_dir());
        Self::build(config, secrets_manager)
    }

    /// Create the app with a password-protected secrets vault.
    pub fn with_password(config: Config, password: String) -> Result<Self> {
        let secrets_manager = SecretsManager::with_password(config.credentials_dir(), password);
        Self::build(config, secrets_manager)
    }

    /// Create the app with the local vault in a locked state.
    ///
    /// Used when the vault is password-protected but no password was
    /// given on the CLI.  The local SecretsManager will not attempt
    /// to decrypt the vault, avoiding repeated (expensive) failures
    /// on every render frame.
    pub fn new_locked(config: Config) -> Result<Self> {
        let secrets_manager = SecretsManager::locked(config.credentials_dir());
        Self::build(config, secrets_manager)
    }

    /// Set a vault password to be sent to the gateway after connecting.
    /// Used when --password is passed on the command line.
    pub fn set_deferred_vault_password(&mut self, password: String) {
        self.deferred_vault_password = Some(password);
    }

    fn build(config: Config, mut secrets_manager: SecretsManager) -> Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();

        // Initialise managers
        if !config.use_secrets {
            secrets_manager.set_agent_access(false);
        } else {
            secrets_manager.set_agent_access(config.agent_access);
        }

        // Build skill directories list (highest precedence last):
        // 1. Bundled OpenClaw skills (if available)
        // 2. User OpenClaw skills (~/.openclaw/workspace/skills)
        // 3. User RustyClaw skills (~/.rustyclaw/workspace/skills)
        // 4. Configured skills_dir (if different)
        let mut skills_dirs = Vec::new();

        // OpenClaw bundled skills (npm global install)
        let openclaw_bundled = PathBuf::from("/usr/lib/node_modules/openclaw/skills");
        if openclaw_bundled.exists() {
            skills_dirs.push(openclaw_bundled);
        }

        // OpenClaw user skills
        if let Some(home) = dirs::home_dir() {
            let openclaw_user = home.join(".openclaw/workspace/skills");
            if openclaw_user.exists() {
                skills_dirs.push(openclaw_user);
            }
        }

        // RustyClaw user skills (primary)
        let rustyclaw_skills = config.skills_dir();
        skills_dirs.push(rustyclaw_skills);

        let mut skill_manager = SkillManager::with_dirs(skills_dirs);
        let _ = skill_manager.load_skills();

        let soul_path = config.soul_path();
        let mut soul_manager = SoulManager::new(soul_path);

        // Check if we need to show the hatching animation
        let needs_hatching = soul_manager.needs_hatching();

        // Load or create SOUL
        let _ = soul_manager.load();

        // Build pages
        let mut home = Home::new()?;
        home.register_action_handler(action_tx.clone())?;
        let pages: Vec<Box<dyn Page>> = vec![Box::new(home)];

        // Create hatching page if needed
        let (hatching_page, showing_hatching) = if needs_hatching {
            let agent_name = config.agent_name.clone();
            (Some(Hatching::new(&agent_name)?), true)
        } else {
            (None, false)
        };

        // Load persisted conversation history (if any).
        let history_path = Self::history_path(&config);
        let conversation_history = Self::load_history(&history_path, &soul_manager, &skill_manager);

        // Replay previous conversation turns into the display messages
        // so the user can see their past context.
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
                    _ => {} // skip system prompt
                }
            }
        }

        let gateway_status = GatewayStatus::Disconnected;

        let state = SharedState {
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
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut tui = Tui::new()?;
        tui.enter()?;

        // Init pages
        {
            let ps = self.state.pane_state();
            for page in &mut self.pages {
                page.init(&ps)?;
            }
            // Init hatching page if present
            if let Some(ref mut hatching) = self.hatching_page {
                hatching.init(&ps)?;
            }
        }
        self.pages[self.active_page].focus()?;

        // Auto-start gateway (uses configured URL or defaults to ws://127.0.0.1:9001)
        self.start_gateway().await;

        loop {
            // Pull the next TUI event (key, mouse, tick, render, etc.)
            if let Some(event) = tui.next().await {
                // Determine the action from the event
                let mut action = match &event {
                    Event::Render => None,
                    Event::Tick => Some(Action::Tick),
                    Event::Resize(w, h) => Some(Action::Resize(*w, *h)),
                    Event::Quit => Some(Action::Quit),
                    _ => {
                        // If showing hatching animation, intercept all events for it
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
                        }
                        // While streaming, Esc cancels the tool loop
                        else if self.streaming_response.is_some() {
                            if let Event::Key(key) = &event {
                                if key.code == crossterm::event::KeyCode::Esc {
                                    // Send cancel message to gateway
                                    let cancel_msg = serde_json::json!({ "type": "cancel" });
                                    self.send_to_gateway(cancel_msg.to_string()).await;
                                    self.state.messages.push(DisplayMessage::info(
                                        "Cancelling tool loop…",
                                    ));
                                    continue;
                                }
                            }
                            None
                        }
                        // While loading, Esc cancels the active async operation
                        else if self.fetch_loading.is_some() || self.device_flow_loading.is_some() {
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
                                    // Consume the Esc so it doesn't propagate
                                    continue;
                                }
                            }
                            None
                        }
                        // If the API key dialog is open, intercept keys for it
                        else if self.api_key_dialog.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_api_key_dialog_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the provider selector is open, intercept keys for it
                        else if self.provider_selector.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_provider_selector_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the model selector is open, intercept keys for it
                        else if self.model_selector.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_model_selector_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the policy picker is open, intercept keys for it
                        else if self.policy_picker.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_policy_picker_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the secret viewer is open, intercept keys for it
                        else if self.secret_viewer.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_secret_viewer_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the credential dialog is open, intercept keys for it
                        else if self.credential_dialog.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_credential_dialog_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the TOTP setup dialog is open, intercept keys for it
                        else if self.totp_dialog.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_totp_dialog_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the gateway auth prompt is open, intercept keys for it
                        else if self.auth_prompt.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_auth_prompt_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the vault unlock prompt is open, intercept keys for it
                        else if self.vault_unlock_prompt.is_some() {
                            if let Event::Key(key) = &event {
                                let action = self.handle_vault_unlock_prompt_key(key.code);
                                Some(action)
                            } else {
                                None
                            }
                        }
                        // If the skills dialog is open, intercept keys to close it
                        else if self.show_skills_dialog {
                            if let Event::Key(key) = &event {
                                match key.code {
                                    crossterm::event::KeyCode::Esc
                                    | crossterm::event::KeyCode::Enter
                                    | crossterm::event::KeyCode::Char('q') => {
                                        self.show_skills_dialog = false;
                                        Some(Action::Noop)
                                    }
                                    _ => Some(Action::Noop), // swallow all other keys
                                }
                            } else {
                                None
                            }
                        }
                        // If the secrets dialog is open, handle navigation or close
                        else if self.show_secrets_dialog {
                            if let Event::Key(key) = &event {
                                match key.code {
                                    crossterm::event::KeyCode::Esc
                                    | crossterm::event::KeyCode::Char('q') => {
                                        self.show_secrets_dialog = false;
                                        Some(Action::Noop)
                                    }
                                    crossterm::event::KeyCode::Char('j')
                                    | crossterm::event::KeyCode::Down => {
                                        let creds = self.state.secrets_manager.list_all_entries();
                                        let max = creds.len().saturating_sub(1);
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
                                        let creds = self.state.secrets_manager.list_all_entries();
                                        if let Some((name, entry)) = creds.get(self.secrets_scroll)
                                        {
                                            self.show_secrets_dialog = false;
                                            Some(Action::ShowCredentialDialog {
                                                name: name.clone(),
                                                disabled: entry.disabled,
                                                policy: entry.policy.badge().to_string(),
                                            })
                                        } else {
                                            Some(Action::Noop)
                                        }
                                    }
                                    _ => Some(Action::Noop), // swallow
                                }
                            } else {
                                None
                            }
                        } else {
                            let mut ps = self.state.pane_state();
                            // Footer (input bar) always gets first chance at key events.
                            // In Normal mode it returns None for keys it doesn't consume,
                            // letting the active page handle navigation.
                            match self.footer.handle_events(event.clone(), &mut ps)? {
                                Some(EventResponse::Stop(a)) => {
                                    self.state.input_mode = ps.input_mode;
                                    Some(a)
                                }
                                _ => {
                                    self.state.input_mode = ps.input_mode;
                                    // Pass to the active page for navigation keys
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

                // Process the action (and any chained follow-up actions)
                while let Some(act) = action {
                    action = self.dispatch_action(act).await?;
                }

                // Drain the mpsc channel (pages may have sent actions via tx)
                while let Ok(act) = self.action_rx.try_recv() {
                    let mut a = Some(act);
                    while let Some(act) = a {
                        a = self.dispatch_action(act).await?;
                    }
                }

                // Render
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

    /// Dispatch a single action to header, footer, and the active page.
    /// Returns an optional follow-up action.
    async fn dispatch_action(&mut self, action: Action) -> Result<Option<Action>> {
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
            Action::GatewayMessage(text) => {
                return self.handle_gateway_message(text);
            }
            Action::GatewayAuthChallenge => {
                // Open the TOTP code entry dialog.
                self.auth_prompt = Some(AuthPromptState {
                    input: String::new(),
                });
                return Ok(Some(Action::Update));
            }
            Action::GatewayAuthResponse(code) => {
                // Send the TOTP code to the gateway as an auth_response frame.
                let frame = serde_json::json!({
                    "type": "auth_response",
                    "code": code,
                });
                self.send_to_gateway(frame.to_string()).await;
                self.state
                    .messages
                    .push(DisplayMessage::info("Sent authentication code…"));
                return Ok(Some(Action::Update));
            }
            Action::GatewayVaultLocked => {
                // Open the vault password entry dialog.
                self.vault_unlock_prompt = Some(VaultUnlockPromptState {
                    input: String::new(),
                });
                return Ok(Some(Action::Update));
            }
            Action::GatewayUnlockVault(password) => {
                // Send the password to the gateway as an unlock_vault frame.
                let frame = serde_json::json!({
                    "type": "unlock_vault",
                    "password": password,
                });
                self.send_to_gateway(frame.to_string()).await;
                self.state
                    .messages
                    .push(DisplayMessage::info("Sent vault unlock request…"));
                return Ok(Some(Action::Update));
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
                // Advance hatching animation if active
                if self.showing_hatching {
                    if let Some(ref mut hatching) = self.hatching_page {
                        let mut ps = self.state.pane_state();
                        if let Ok(Some(hatching_action)) = hatching.update(Action::Tick, &mut ps) {
                            // Propagate actions from hatching (e.g. BeginHatchingExchange)
                            return Ok(Some(hatching_action));
                        }
                    }
                }

                // Advance the inline loading line
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
                // Fall through so panes also get Tick
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
                let action = dialogs::handle_confirm_store_secret(
                    provider,
                    key,
                    &mut self.state.secrets_manager,
                    &mut self.state.messages,
                );
                return Ok(action);
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
                match self.state.secrets_manager.store_secret(secret_key, token) {
                    Ok(()) => {
                        self.state.messages.push(DisplayMessage::success(format!(
                            "{} authenticated successfully. Token stored.",
                            display,
                        )));
                    }
                    Err(e) => {
                        self.state.messages.push(DisplayMessage::error(format!(
                            "Failed to store token: {}. Token set for this session only.",
                            e,
                        )));
                    }
                }
                // Proceed to model selection
                return Ok(Some(Action::FetchModels(provider.clone())));
            }
            Action::DeviceFlowFailed(msg) => {
                self.device_flow_loading = None;
                self.state.loading_line = None;
                self.state.messages.push(DisplayMessage::error(msg));
                return Ok(Some(Action::Update));
            }
            Action::ShowCredentialDialog { name, disabled, .. } => {
                let has_totp = self.state.secrets_manager.has_totp();
                // Look up the actual current policy from the vault metadata
                let current_policy = self
                    .state
                    .secrets_manager
                    .list_all_entries()
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, entry)| entry.policy.clone())
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
                if self.state.secrets_manager.has_totp() {
                    self.totp_dialog = Some(TotpDialogState {
                        phase: TotpDialogPhase::AlreadyConfigured,
                    });
                } else {
                    match self.state.secrets_manager.setup_totp("rustyclaw") {
                        Ok(uri) => {
                            self.totp_dialog = Some(TotpDialogState {
                                phase: TotpDialogPhase::ShowUri {
                                    uri,
                                    input: String::new(),
                                },
                            });
                        }
                        Err(e) => {
                            self.state
                                .messages
                                .push(DisplayMessage::error(format!("Failed to set up 2FA: {}", e)));
                        }
                    }
                }
                return Ok(None);
            }
            Action::CloseHatching => {
                self.showing_hatching = false;
                self.hatching_page = None;
                return Ok(None);
            }
            Action::BeginHatchingExchange => {
                // Build a structured chat request and send it to the gateway
                let messages = self.hatching_page.as_ref().map(|h| h.chat_messages());
                if let Some(msgs) = messages {
                    let chat_json = self.build_hatching_chat_request(msgs);
                    self.send_to_gateway(chat_json).await;
                }
                return Ok(Some(Action::Update));
            }
            Action::FinishHatching(soul_content) => {
                // Save the generated identity as SOUL.md
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
            _ => {}
        }

        // Update header
        {
            let mut ps = self.state.pane_state();
            self.header.update(action.clone(), &mut ps)?;
            self.state.input_mode = ps.input_mode;
        }

        // Update footer
        let footer_follow = {
            let mut ps = self.state.pane_state();
            let r = self.footer.update(action.clone(), &mut ps)?;
            self.state.input_mode = ps.input_mode;
            r
        };

        // Update active page
        let page_follow = {
            let mut ps = self.state.pane_state();
            let r = self.pages[self.active_page].update(action, &mut ps)?;
            self.state.input_mode = ps.input_mode;
            r
        };

        Ok(footer_follow.or(page_follow))
    }

    /// Handle gateway messages received from the WebSocket.
    fn handle_gateway_message(&mut self, text: &str) -> Result<Option<Action>> {
        // Parse the gateway JSON envelope.
        let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
        let frame_type = parsed
            .as_ref()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()));

        // Debug: log all incoming frames
        debug_log(&format!("Received frame: type={:?}, len={}", frame_type, text.len()));

        // ── Handle status frames from the gateway ────────────
        if frame_type == Some("status") {
            let status = parsed
                .as_ref()
                .and_then(|v| v.get("status").and_then(|s| s.as_str()))
                .unwrap_or("");
            let detail = parsed
                .as_ref()
                .and_then(|v| v.get("detail").and_then(|d| d.as_str()))
                .unwrap_or("");

            match status {
                "model_configured" => {
                    self.state
                        .messages
                        .push(DisplayMessage::info(format!("Model: {}", detail)));
                }
                "credentials_loaded" => {
                    self.state.messages.push(DisplayMessage::info(detail));
                }
                "credentials_missing" => {
                    self.state.gateway_status = GatewayStatus::ModelError;
                    self.state.messages.push(DisplayMessage::warning(detail));
                }
                "model_connecting" => {
                    self.state.messages.push(DisplayMessage::info(detail));
                }
                "model_ready" => {
                    self.state.gateway_status = GatewayStatus::ModelReady;
                    self.state.messages.push(DisplayMessage::success(detail));
                }
                "model_error" => {
                    self.state.gateway_status = GatewayStatus::ModelError;
                    self.state.messages.push(DisplayMessage::error(detail));
                }
                "no_model" => {
                    self.state.messages.push(DisplayMessage::warning(detail));
                }
                "vault_locked" => {
                    // If we have a deferred password (from --password flag),
                    // forward it automatically instead of prompting.
                    if let Some(pw) = self.deferred_vault_password.take() {
                        return Ok(Some(Action::GatewayUnlockVault(pw)));
                    }
                    self.state.gateway_status = GatewayStatus::VaultLocked;
                    self.state.messages.push(DisplayMessage::warning(
                        "Gateway vault is locked — enter password to unlock.",
                    ));
                    return Ok(Some(Action::GatewayVaultLocked));
                }
                _ => {
                    self.state
                        .messages
                        .push(DisplayMessage::system(format!("[{}] {}", status, detail)));
                }
            }
            return Ok(Some(Action::Update));
        }

        // ── Handle auth challenge from gateway ───────────────
        if frame_type == Some("auth_challenge") {
            self.state.gateway_status = GatewayStatus::AuthRequired;
            self.state
                .messages
                .push(DisplayMessage::warning("Gateway requires 2FA authentication."));
            return Ok(Some(Action::GatewayAuthChallenge));
        }

        // ── Handle auth result from gateway ──────────────────
        if frame_type == Some("auth_result") {
            let ok = parsed
                .as_ref()
                .and_then(|v| v.get("ok").and_then(|o| o.as_bool()))
                .unwrap_or(false);
            let retry = parsed
                .as_ref()
                .and_then(|v| v.get("retry").and_then(|r| r.as_bool()))
                .unwrap_or(false);

            if ok {
                self.state
                    .messages
                    .push(DisplayMessage::success("Authenticated with gateway."));
                // The hello + status frames will follow from the
                // gateway, so keep the Connecting status.
                self.state.gateway_status = GatewayStatus::Connected;
            } else if retry {
                // Wrong code but gateway allows retry — show message and re-prompt
                let msg = parsed
                    .as_ref()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("Invalid code. Try again.");
                self.state.messages.push(DisplayMessage::warning(msg));
                // Re-trigger the auth challenge to show the TOTP prompt again
                return Ok(Some(Action::GatewayAuthChallenge));
            } else {
                let msg = parsed
                    .as_ref()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("Authentication failed.");
                self.state.messages.push(DisplayMessage::error(msg));
                self.state.gateway_status = GatewayStatus::Error;
            }
            return Ok(Some(Action::Update));
        }

        // ── Handle auth lockout from gateway ─────────────────
        if frame_type == Some("auth_locked") {
            let msg = parsed
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                .unwrap_or("Too many failed attempts.");
            self.state.messages.push(DisplayMessage::error(msg));
            self.state.gateway_status = GatewayStatus::Error;
            return Ok(Some(Action::Update));
        }

        // ── Handle vault unlock result ───────────────────────
        if frame_type == Some("vault_unlocked") {
            let ok = parsed
                .as_ref()
                .and_then(|v| v.get("ok").and_then(|o| o.as_bool()))
                .unwrap_or(false);
            if ok {
                self.state
                    .messages
                    .push(DisplayMessage::success("Gateway vault unlocked."));
                self.state.gateway_status = GatewayStatus::Connected;
            } else {
                let msg = parsed
                    .as_ref()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("Failed to unlock vault.");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            return Ok(Some(Action::Update));
        }

        // ── Handle tool-call frames ──────────────────────────
        if frame_type == Some("tool_call") {
            let name = parsed
                .as_ref()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()))
                .unwrap_or("unknown");
            let arguments = parsed
                .as_ref()
                .and_then(|v| v.get("arguments").and_then(|a| a.as_str()))
                .unwrap_or("{}");
            self.state
                .messages
                .push(DisplayMessage::tool_call(format!("{name}({arguments})")));
            return Ok(Some(Action::Update));
        }

        // ── Handle tool-result frames ────────────────────────
        if frame_type == Some("tool_result") {
            let name = parsed
                .as_ref()
                .and_then(|v| v.get("name").and_then(|n| n.as_str()))
                .unwrap_or("unknown");
            let result = parsed
                .as_ref()
                .and_then(|v| v.get("result").and_then(|r| r.as_str()))
                .unwrap_or("");
            let is_error = parsed
                .as_ref()
                .and_then(|v| v.get("is_error").and_then(|e| e.as_bool()))
                .unwrap_or(false);
            let prefix = if is_error { "⚠ " } else { "" };
            // Truncate long results for the TUI display.
            let display_result = if result.len() > 2000 {
                format!("{}{}…({} bytes)", prefix, &result[..2000], result.len())
            } else {
                format!("{prefix}{result}")
            };
            self.state
                .messages
                .push(DisplayMessage::tool_result(format!("{name}: {display_result}")));
            return Ok(Some(Action::Update));
        }

        // ── Handle debug frames (suppress or log) ────────────
        if frame_type == Some("debug") {
            // Debug frames are internal — don't display to user
            // Could optionally log to a debug pane or file in the future
            return Ok(Some(Action::Update));
        }

        // ── Handle info/notification frames ──────────────────
        if frame_type == Some("info") {
            let message = parsed
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                .unwrap_or("");
            if !message.is_empty() {
                self.state.messages.push(DisplayMessage::info(message));
            }
            return Ok(Some(Action::Update));
        }

        // ── Handle stream start (API connected, waiting for response) ──
        if frame_type == Some("stream_start") {
            // Show that we're connected and waiting for the model
            self.state.loading_line = Some("⏳ Waiting for response...".to_string());
            self.state.streaming_started = Some(std::time::Instant::now());
            return Ok(Some(Action::Update));
        }

        // ── Handle extended thinking frames (Anthropic) ──────
        if frame_type == Some("thinking_start") {
            // Show thinking indicator in the loading line
            self.state.loading_line = Some("🤔 Thinking...".to_string());
            self.state.streaming_started = Some(std::time::Instant::now());
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("thinking_delta") {
            // Update thinking indicator with elapsed time
            // The actual thinking content is available in delta but we don't display it
            // to keep the UI clean. The loading_line shows we're still working.
            if let Some(started) = self.state.streaming_started {
                let elapsed = started.elapsed().as_secs();
                self.state.loading_line = Some(format!("🤔 Thinking... ({}s)", elapsed));
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("thinking_end") {
            // Thinking complete — clear indicator, text streaming will begin
            self.state.loading_line = None;
            // Keep streaming_started for the text phase timing
            return Ok(Some(Action::Update));
        }

        // ── Handle streaming chunk frames ────────────────────
        if frame_type == Some("chunk") {
            let delta = parsed
                .as_ref()
                .and_then(|v| v.get("delta").and_then(|d| d.as_str()))
                .unwrap_or("");

            debug_log(&format!("Received chunk: {} chars, delta='{}'", delta.len(), &delta[..delta.len().min(50)]));

            if self.streaming_response.is_none() {
                // First chunk — clear the loading spinner and start accumulating.
                debug_log("First chunk - initializing streaming response");
                self.state.loading_line = None;
                self.streaming_response = Some(String::new());
                self.state.streaming_started = Some(std::time::Instant::now());
                // Only push a placeholder message if NOT in hatching mode.
                if !self.showing_hatching {
                    self.state.messages.push(DisplayMessage::assistant(""));
                }
            }

            if let Some(ref mut buf) = self.streaming_response {
                buf.push_str(delta);
                debug_log(&format!("Buffer now {} chars", buf.len()));

                // During hatching, just accumulate — don't push to messages.
                if !self.showing_hatching {
                    // Update the last assistant message with accumulated text.
                    if let Some(last) = self.state.messages.last_mut() {
                        last.update_content(buf.clone());
                        debug_log(&format!("Updated last message to {} chars", last.content.len()));
                    } else {
                        debug_log("WARNING: No last message to update!");
                    }
                } else {
                    debug_log("Hatching mode - not updating messages");
                }
            }

            return Ok(Some(Action::Update));
        }

        // ── Handle streaming-done sentinel ───────────────────
        if frame_type == Some("response_done") {
            self.chat_loading_tick = None;
            self.state.loading_line = None;
            self.state.streaming_started = None;

            if let Some(buf) = self.streaming_response.take() {
                debug_log(&format!("response_done: buf len={}, showing_hatching={}", buf.len(), self.showing_hatching));
                
                // During hatching, deliver the full accumulated text
                // to the hatching page instead of the messages pane.
                if self.showing_hatching {
                    if !buf.is_empty() {
                        if let Some(ref mut hatching) = self.hatching_page {
                            let mut ps = self.state.pane_state();
                            let _ = hatching.update(Action::HatchingResponse(buf), &mut ps);
                        }
                    }
                    return Ok(Some(Action::Update));
                }

                // Trim trailing whitespace from the final message.
                let trimmed = buf.trim_end().to_string();
                debug_log(&format!("response_done: trimmed len={}, messages count={}", trimmed.len(), self.state.messages.len()));
                
                if let Some(last) = self.state.messages.last_mut() {
                    debug_log(&format!("response_done: last message role={:?}", last.role));
                    if matches!(last.role, crate::panes::MessageRole::Assistant) {
                        last.content = trimmed.clone();
                        debug_log(&format!("response_done: set content to {} chars", trimmed.len()));
                    }
                }

                // Record the assistant turn in conversation history
                // and persist to disk so future sessions remember.
                if !trimmed.is_empty() {
                    self.state.conversation_history.push(ChatMessage::text("assistant", &trimmed));
                    self.save_history();
                }
            } else {
                debug_log("response_done: no streaming_response buffer!");
            }
            return Ok(Some(Action::Update));
        }

        // ── Extract chat response payload ────────────────────
        let payload = parsed.as_ref().and_then(|v| {
            if v.get("type").and_then(|t| t.as_str()) == Some("response") {
                v.get("received").and_then(|r| r.as_str()).map(String::from)
            } else {
                None
            }
        });

        // ── Handle error frames from the gateway ─────────────
        let is_error_frame = frame_type == Some("error");
        let error_message = if is_error_frame {
            parsed
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                .map(String::from)
        } else {
            None
        };

        if is_error_frame {
            self.chat_loading_tick = None;
            self.state.loading_line = None;
            self.streaming_response = None;
            self.state.streaming_started = None;
            let msg = error_message.unwrap_or_else(|| "Unknown gateway error".to_string());
            self.state.messages.push(DisplayMessage::error(msg));
            return Ok(Some(Action::Update));
        }

        // Route gateway messages to hatching during the exchange
        if self.showing_hatching {
            if let Some(content) = payload {
                if let Some(ref mut hatching) = self.hatching_page {
                    let mut ps = self.state.pane_state();
                    let _ = hatching.update(Action::HatchingResponse(content), &mut ps);
                }
            }
            return Ok(Some(Action::Update));
        }

        // Normal (non-hatching) messages pane display
        let display = payload.as_deref().unwrap_or(text);

        // Clear chat loading spinner
        self.chat_loading_tick = None;
        self.state.loading_line = None;

        // Keep the full response as a single message — the messages
        // pane renderer handles multi-line content natively.
        self.state.messages.push(DisplayMessage::assistant(display));
        // Auto-scroll
        Ok(Some(Action::Update))
    }

    /// Handle SetProvider action.
    fn handle_set_provider(&mut self, provider: String) -> Result<Option<Action>> {
        // Save provider to config
        let model_cfg = self.state.config.model.get_or_insert_with(|| {
            crate::config::ModelProvider {
                provider: String::new(),
                model: None,
                base_url: None,
            }
        });
        model_cfg.provider = provider.clone();
        if let Some(url) = providers::base_url_for_provider(&provider) {
            model_cfg.base_url = Some(url.to_string());
        }
        if let Err(e) = self.state.config.save(None) {
            self.state
                .messages
                .push(DisplayMessage::error(format!("Failed to save config: {}", e)));
        } else {
            self.state
                .messages
                .push(DisplayMessage::success(format!("Provider set to {}.", provider)));
        }
        // Check auth method and proceed accordingly
        let def = providers::provider_by_id(&provider);
        let auth_method = def
            .map(|d| d.auth_method)
            .unwrap_or(providers::AuthMethod::ApiKey);

        match auth_method {
            providers::AuthMethod::DeviceFlow => {
                if let Some(secret_key) = providers::secret_key_for_provider(&provider) {
                    match self.state.secrets_manager.get_secret(secret_key, true) {
                        Ok(Some(_)) => {
                            self.state.messages.push(DisplayMessage::success(format!(
                                "Access token for {} is already stored.",
                                providers::display_name_for_provider(&provider),
                            )));
                            return Ok(Some(Action::FetchModels(provider)));
                        }
                        _ => {
                            return Ok(Some(Action::StartDeviceFlow(provider)));
                        }
                    }
                }
            }
            providers::AuthMethod::ApiKey => {
                if let Some(secret_key) = providers::secret_key_for_provider(&provider) {
                    match self.state.secrets_manager.get_secret(secret_key, true) {
                        Ok(Some(_)) => {
                            self.state.messages.push(DisplayMessage::success(format!(
                                "API key for {} is already stored.",
                                providers::display_name_for_provider(&provider),
                            )));
                            return Ok(Some(Action::FetchModels(provider)));
                        }
                        _ => {
                            return Ok(Some(Action::PromptApiKey(provider)));
                        }
                    }
                }
            }
            providers::AuthMethod::None => {
                self.state.messages.push(DisplayMessage::info(format!(
                    "{} does not require authentication.",
                    providers::display_name_for_provider(&provider),
                )));
                return Ok(Some(Action::FetchModels(provider)));
            }
        }
        Ok(None)
    }

    /// Process submitted input — either a /command or a plain prompt.
    fn handle_input_submit(&mut self, text: String) -> Result<Option<Action>> {
        if text.is_empty() {
            return Ok(None);
        }

        if text.starts_with('/') {
            // It's a command
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
                    // Restart the gateway so it picks up the new model.
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
                    // Find the media in conversation history
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
            // It's a plain prompt — wrap it in a chat request envelope so
            // the gateway recognises it as a model call rather than echoing
            // the raw text back.
            self.state.messages.push(DisplayMessage::user(&text));
            if matches!(
                self.state.gateway_status,
                GatewayStatus::Connected | GatewayStatus::ModelReady
            ) && self.ws_sink.is_some()
            {
                // Append the new user turn to the running conversation history.
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

    // ── Dialog key handlers ─────────────────────────────────────────────────

    /// Handle key events when the API key dialog is open.
    fn handle_api_key_dialog_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(dialog) = self.api_key_dialog.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_api_key_dialog_key(dialog, code, &mut self.state.messages);
        self.api_key_dialog = new_state;
        action
    }

    /// Handle key events when the provider selector dialog is open.
    fn handle_provider_selector_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(sel) = self.provider_selector.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_provider_selector_key(sel, code, &mut self.state.messages);
        self.provider_selector = new_state;
        action
    }

    /// Handle key events when the model selector dialog is open.
    fn handle_model_selector_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(sel) = self.model_selector.take() else {
            return Action::Noop;
        };
        // Create a no-op save function - we'll save after the dialog function returns
        let save_fn = || -> Result<(), String> { Ok(()) };
        let (new_state, action) = dialogs::handle_model_selector_key(
            sel,
            code,
            &mut self.state.config.model,
            &mut self.state.messages,
            save_fn,
        );
        // Now save if Enter was pressed (action is RestartGateway)
        if matches!(action, Action::RestartGateway) {
            if let Err(e) = self.state.config.save(None) {
                // Replace the success message with an error
                if let Some(msg) = self.state.messages.last_mut() {
                    *msg = DisplayMessage::error(format!("Failed to save config: {}", e));
                }
            }
        }
        self.model_selector = new_state;
        action
    }

    /// Handle key events when the policy picker dialog is open.
    fn handle_policy_picker_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(picker) = self.policy_picker.take() else {
            return Action::Noop;
        };
        let (new_state, action) = dialogs::handle_policy_picker_key(
            picker,
            code,
            &mut self.state.secrets_manager,
            &mut self.state.messages,
        );
        self.policy_picker = new_state;
        action
    }

    /// Handle key events when the secret viewer dialog is open.
    fn handle_secret_viewer_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(viewer) = self.secret_viewer.take() else {
            return Action::Noop;
        };
        let (new_state, action) = dialogs::handle_secret_viewer_key(viewer, code);
        self.secret_viewer = new_state;
        action
    }

    /// Handle key events when the credential dialog is open.
    fn handle_credential_dialog_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(dlg) = self.credential_dialog.take() else {
            return Action::Noop;
        };
        let (new_dlg, new_picker, new_viewer, action): (
            Option<dialogs::CredentialDialogState>,
            Option<dialogs::PolicyPickerState>,
            Option<dialogs::SecretViewerState>,
            Action,
        ) = dialogs::handle_credential_dialog_key(
            dlg,
            code,
            &mut self.state.secrets_manager,
            &mut self.state.messages,
        );
        self.credential_dialog = new_dlg;
        if new_picker.is_some() {
            self.policy_picker = new_picker;
        }
        if new_viewer.is_some() {
            self.secret_viewer = new_viewer;
        }
        action
    }

    /// Handle key events when the TOTP dialog is open.
    fn handle_totp_dialog_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(dlg) = self.totp_dialog.take() else {
            return Action::Noop;
        };
        let (new_state, action) = dialogs::handle_totp_dialog_key(
            dlg,
            code,
            &mut self.state.secrets_manager,
            &mut self.state.config,
            &mut self.state.messages,
        );
        self.totp_dialog = new_state;
        action
    }

    /// Handle key events when the gateway auth prompt is open.
    fn handle_auth_prompt_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(prompt) = self.auth_prompt.take() else {
            return Action::Noop;
        };
        let (new_state, action) = dialogs::handle_auth_prompt_key(
            prompt,
            code,
            &mut self.state.messages,
            &mut self.state.gateway_status,
        );
        self.auth_prompt = new_state;
        action
    }

    /// Handle key events when the vault unlock prompt is open.
    fn handle_vault_unlock_prompt_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        let Some(prompt) = self.vault_unlock_prompt.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_vault_unlock_prompt_key(prompt, code, &mut self.state.messages);
        self.vault_unlock_prompt = new_state;
        action
    }

    // ── Gateway connection ──────────────────────────────────────────────────

    /// Ensure the gateway daemon is running, then connect to it.
    async fn start_gateway(&mut self) {
        let (port, bind) = Self::gateway_defaults(&self.state.config);
        let url = self
            .state
            .config
            .gateway_url
            .clone()
            .unwrap_or_else(|| format!("ws://127.0.0.1:{}", port));

        // If we already have an open WebSocket, nothing to do.
        if self.ws_sink.is_some() {
            self.state
                .messages
                .push(DisplayMessage::info("Already connected to gateway."));
            return;
        }

        self.state.gateway_status = GatewayStatus::Connecting;

        // Start the daemon if it isn't running yet.
        match daemon::status(&self.state.config.settings_dir) {
            daemon::DaemonStatus::Running { pid } => {
                self.state.messages.push(DisplayMessage::info(format!(
                    "Gateway daemon already running (PID {}).",
                    pid,
                )));
            }
            _ => {
                // Save config first so the daemon reads current values.
                if let Err(e) = self.state.config.save(None) {
                    self.state.messages.push(DisplayMessage::warning(format!(
                        "Warning: could not save config: {}",
                        e,
                    )));
                }

                let api_key = self.extract_model_api_key();
                let vault_password = self.extract_vault_password();

                self.state.messages.push(DisplayMessage::info(format!(
                    "Starting gateway daemon on {}…",
                    url,
                )));
                match daemon::start(
                    &self.state.config.settings_dir,
                    port,
                    bind,
                    &[],
                    api_key.as_deref(),
                    vault_password.as_deref(),
                ) {
                    Ok(pid) => {
                        self.state.messages.push(DisplayMessage::success(format!(
                            "Gateway daemon started (PID {}).",
                            pid,
                        )));
                    }
                    Err(e) => {
                        self.state.gateway_status = GatewayStatus::Error;
                        self.state
                            .messages
                            .push(DisplayMessage::error(format!("Failed to start gateway: {}", e)));
                        return;
                    }
                }
                // Give the daemon a moment to bind.
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }

        // Connect as a WebSocket client.
        self.connect_to_gateway(&url).await;
    }

    /// Connect the TUI as a WebSocket client to a running gateway.
    async fn connect_to_gateway(&mut self, url: &str) {
        self.state.gateway_status = GatewayStatus::Connecting;
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws_stream, _)) => {
                let (sink, stream) = ws_stream.split();
                self.ws_sink = Some(sink);

                self.state.gateway_status = GatewayStatus::Connected;
                self.state
                    .messages
                    .push(DisplayMessage::success(format!("Connected to gateway {}", url)));

                // Spawn a background task that reads from the gateway and
                // forwards messages into the TUI event loop via action_tx.
                let tx = self.action_tx.clone();
                self.reader_task = Some(tokio::spawn(async move {
                    Self::gateway_reader_loop(stream, tx).await;
                }));
            }
            Err(err) => {
                self.state.gateway_status = GatewayStatus::Error;
                self.state.messages.push(DisplayMessage::error(format!(
                    "Gateway connection failed: {}",
                    err
                )));
            }
        }
    }

    /// Background loop: reads messages from the gateway WebSocket stream and
    /// sends them as actions into the TUI event loop.
    async fn gateway_reader_loop(
        mut stream: futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        tx: mpsc::UnboundedSender<Action>,
    ) {
        while let Some(result) = stream.next().await {
            match result {
                Ok(Message::Text(text)) => {
                    let _ = tx.send(Action::GatewayMessage(text.to_string()));
                }
                Ok(Message::Close(_)) => {
                    let _ = tx.send(Action::GatewayDisconnected(
                        "server sent close frame".to_string(),
                    ));
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                    // handled automatically by tungstenite
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = tx.send(Action::GatewayDisconnected(format!("{}", err)));
                    break;
                }
            }
        }
    }

    /// Send a text message to the gateway over the open WebSocket connection.
    async fn send_to_gateway(&mut self, text: String) {
        if let Some(ref mut sink) = self.ws_sink {
            match sink.send(Message::Text(text.into())).await {
                Ok(()) => {}
                Err(err) => {
                    self.chat_loading_tick = None;
                    self.state.loading_line = None;
                    self.streaming_response = None;
                    self.state.streaming_started = None;
                    self.state
                        .messages
                        .push(DisplayMessage::error(format!("Send failed: {}", err)));
                    self.state.gateway_status = GatewayStatus::Error;
                    self.ws_sink = None;
                }
            }
        } else {
            self.state
                .messages
                .push(DisplayMessage::warning("Cannot send: gateway not connected."));
        }
    }

    /// Stop the gateway: disconnect the client and stop the daemon.
    async fn stop_gateway(&mut self) {
        let had_connection = self.ws_sink.is_some();

        // Abort the reader task first so it doesn't fire a disconnect action.
        if let Some(handle) = self.reader_task.take() {
            handle.abort();
        }

        // Close the client-side WebSocket gracefully.
        if let Some(mut sink) = self.ws_sink.take() {
            let _ = sink.send(Message::Close(None)).await;
            let _ = sink.close().await;
        }

        // Stop the daemon process.
        let daemon_stopped = match daemon::stop(&self.state.config.settings_dir) {
            Ok(daemon::StopResult::Stopped { pid }) => {
                self.state.messages.push(DisplayMessage::info(format!(
                    "Gateway daemon stopped (was PID {}).",
                    pid,
                )));
                true
            }
            Ok(daemon::StopResult::WasStale { pid }) => {
                self.state.messages.push(DisplayMessage::warning(format!(
                    "Cleaned up stale PID file (PID {}).",
                    pid,
                )));
                false
            }
            Ok(daemon::StopResult::WasNotRunning) => false,
            Err(e) => {
                self.state.messages.push(DisplayMessage::warning(format!(
                    "Warning: could not stop daemon: {}",
                    e,
                )));
                false
            }
        };

        if had_connection || daemon_stopped {
            self.state.gateway_status = GatewayStatus::Disconnected;
            if !daemon_stopped {
                self.state
                    .messages
                    .push(DisplayMessage::info("Disconnected from gateway."));
            }
        } else {
            self.state
                .messages
                .push(DisplayMessage::info("Gateway is not running."));
        }
    }

    /// Restart: stop the daemon, start a fresh one, reconnect.
    async fn restart_gateway(&mut self) {
        self.stop_gateway().await;

        // Brief pause so the OS releases the port and the TUI can
        // render the Disconnected status.
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = tx.send(Action::ReconnectGateway);
        });
    }

    /// Extract the API key for the currently configured model provider
    /// from the secrets vault.
    fn extract_model_api_key(&mut self) -> Option<String> {
        let provider_id = self
            .state
            .config
            .model
            .as_ref()
            .map(|m| m.provider.as_str())?;
        let key_name = providers::secret_key_for_provider(provider_id)?;
        self.state
            .secrets_manager
            .get_secret(key_name, true)
            .ok()
            .flatten()
    }

    /// Extract the vault password to pass to the gateway daemon.
    fn extract_vault_password(&self) -> Option<String> {
        if !self.state.config.secrets_password_protected {
            return None;
        }
        self.state
            .secrets_manager
            .password()
            .map(|s| s.to_string())
    }

    /// Parse port and bind mode from the config's gateway URL.
    fn gateway_defaults(config: &Config) -> (u16, &'static str) {
        if let Some(url) = &config.gateway_url {
            if let Ok(parsed) = url::Url::parse(url) {
                let port = parsed.port().unwrap_or(9001);
                let host = parsed.host_str().unwrap_or("127.0.0.1");
                let bind = if host == "0.0.0.0" { "lan" } else { "loopback" };
                return (port, bind);
            }
        }
        (9001, "loopback")
    }

    // ── Device flow authentication ──────────────────────────────────────────

    /// Spawn a background task to perform OAuth device flow authentication.
    fn spawn_device_flow(&mut self, provider: String) {
        let def = match providers::provider_by_id(&provider) {
            Some(d) => d,
            None => {
                self.state
                    .messages
                    .push(DisplayMessage::error(format!("Unknown provider: {}", provider)));
                return;
            }
        };

        let device_config = match def.device_flow {
            Some(cfg) => cfg,
            None => {
                self.state.messages.push(DisplayMessage::warning(format!(
                    "{} does not support device flow authentication.",
                    def.display,
                )));
                return;
            }
        };

        let display = def.display.to_string();
        self.state.messages.push(DisplayMessage::info(format!(
            "Authenticating with {}…",
            display,
        )));

        // Show the inline loading line
        let spinner = SPINNER_FRAMES[0];
        self.state.loading_line = Some(format!(
            "  {} Starting {} authentication…",
            spinner, display,
        ));
        self.device_flow_loading = Some(FetchModelsLoading {
            display: display.clone(),
            tick: 0,
        });

        let tx = self.action_tx.clone();
        let provider_clone = provider.clone();
        let device_cfg: &'static providers::DeviceFlowConfig = device_config;

        tokio::spawn(async move {
            // Step 1: Start the device flow
            let auth = match providers::start_device_flow(device_cfg).await {
                Ok(a) => a,
                Err(e) => {
                    let _ = tx.send(Action::DeviceFlowFailed(format!(
                        "Failed to start device flow: {}",
                        e,
                    )));
                    return;
                }
            };

            // Step 2: Show the URL and code to the user via messages
            let _ = tx.send(Action::DeviceFlowCodeReady {
                url: auth.verification_uri.clone(),
                code: auth.user_code.clone(),
            });

            // Step 3: Poll for the token
            let interval = std::time::Duration::from_secs(auth.interval.max(5));
            let max_attempts = (auth.expires_in / interval.as_secs()).max(10);

            for _ in 0..max_attempts {
                tokio::time::sleep(interval).await;

                match providers::poll_device_token(device_cfg, &auth.device_code).await {
                    Ok(Some(token)) => {
                        let _ = tx.send(Action::DeviceFlowAuthenticated {
                            provider: provider_clone,
                            token,
                        });
                        return;
                    }
                    Ok(None) => {
                        // Still pending — keep polling
                    }
                    Err(e) => {
                        let _ = tx.send(Action::DeviceFlowFailed(format!(
                            "Authentication failed: {}",
                            e,
                        )));
                        return;
                    }
                }
            }

            let _ = tx.send(Action::DeviceFlowFailed(
                "Authentication timed out. Please try again with /provider.".to_string(),
            ));
        });
    }

    // ── Conversation history helpers ────────────────────────────────────────

    /// Path to the persisted conversation history file.
    fn history_path(config: &Config) -> std::path::PathBuf {
        config
            .settings_dir
            .join("conversations")
            .join("current.json")
    }

    /// Build the system prompt from SOUL.md.
    fn system_message(soul: &SoulManager, skill_manager: &SkillManager) -> Option<ChatMessage> {
        let mut content = String::new();

        // Add SOUL.md content
        if let Some(soul_text) = soul.get_content() {
            content.push_str(soul_text);
        }

        // Add skills context
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

    /// Load conversation history from disk, prepending the system prompt.
    fn load_history(
        path: &std::path::Path,
        soul: &SoulManager,
        skill_manager: &SkillManager,
    ) -> Vec<ChatMessage> {
        let mut history = Vec::new();

        // Always lead with the system prompt.
        if let Some(sys) = Self::system_message(soul, skill_manager) {
            history.push(sys);
        }

        // Append persisted turns (if the file exists).
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(turns) = serde_json::from_str::<Vec<ChatMessage>>(&data) {
                history.extend(turns);
            }
        }

        history
    }

    /// Save the user/assistant turns (without the system prompt) to disk.
    fn save_history(&self) {
        let path = Self::history_path(&self.state.config);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Strip the system prompt — we always regenerate it from SOUL.md.
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

    /// Clear conversation history (in-memory and on-disk) and re-seed
    /// the system prompt.
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

    /// Find a MediaRef in conversation history by ID.
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

    /// Download media to a file.
    fn download_media(
        &self,
        media_ref: &crate::gateway::MediaRef,
        dest_path: Option<&str>,
    ) -> Result<String, String> {
        // Determine destination path
        let dest = if let Some(path) = dest_path {
            let path = shellexpand::tilde(path);
            std::path::PathBuf::from(path.as_ref())
        } else {
            // Default to Downloads or current directory
            let downloads = dirs::download_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let filename = media_ref.filename.as_deref().unwrap_or(&media_ref.id);
            downloads.join(filename)
        };

        // Try local cache first
        if let Some(local_path) = &media_ref.local_path {
            let src = std::path::Path::new(local_path);
            if src.exists() {
                std::fs::copy(src, &dest)
                    .map_err(|e| format!("Failed to copy: {}", e))?;
                return Ok(dest.to_string_lossy().to_string());
            }
        }

        // Try downloading from URL
        if let Some(url) = &media_ref.url {
            // Use blocking reqwest for simplicity in sync context
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

    /// Build a structured JSON chat request for the hatching exchange.
    fn build_hatching_chat_request(
        &mut self,
        messages: Vec<crate::gateway::ChatMessage>,
    ) -> String {
        serde_json::json!({
            "type": "chat",
            "messages": messages,
        })
        .to_string()
    }

    // ── Drawing ─────────────────────────────────────────────────────────────

    fn draw(&mut self, tui: &mut Tui) -> Result<()> {
        tui.draw(|frame| {
            let area = frame.area();

            let mut ps = PaneState {
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

            // If showing hatching, render it fullscreen and skip everything else
            if self.showing_hatching {
                if let Some(ref mut hatching) = self.hatching_page {
                    let _ = hatching.draw(frame, area, &ps);
                }
                return;
            }

            // Layout: header (3 rows), body (fill), footer (2 rows: status + input)
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

            // Skills dialog overlay
            if self.show_skills_dialog {
                Self::draw_skills_dialog(frame, area, &ps);
            }

            // Secrets dialog overlay
            if self.show_secrets_dialog {
                Self::draw_secrets_dialog(frame, area, &mut ps, self.secrets_scroll);
            }

            // API key dialog overlay
            if let Some(ref dialog) = self.api_key_dialog {
                dialogs::draw_api_key_dialog(frame, area, dialog);
            }

            // Provider selector dialog overlay
            if let Some(ref selector) = self.provider_selector {
                dialogs::draw_provider_selector_dialog(frame, area, selector);
            }

            // Model selector dialog overlay
            if let Some(ref selector) = self.model_selector {
                dialogs::draw_model_selector_dialog(frame, area, selector);
            }

            // Credential management dialog overlay
            if let Some(ref dialog) = self.credential_dialog {
                dialogs::draw_credential_dialog(frame, area, dialog);
            }

            // Policy picker dialog overlay
            if let Some(ref picker) = self.policy_picker {
                dialogs::draw_policy_picker(frame, area, picker);
            }

            // TOTP setup dialog overlay
            if let Some(ref dialog) = self.totp_dialog {
                dialogs::draw_totp_dialog(frame, area, dialog);
            }

            // Gateway auth prompt overlay (TOTP code entry)
            if let Some(ref prompt) = self.auth_prompt {
                dialogs::draw_auth_prompt(frame, area, prompt);
            }

            // Vault unlock prompt overlay (password entry)
            if let Some(ref prompt) = self.vault_unlock_prompt {
                dialogs::draw_vault_unlock_prompt(frame, area, prompt);
            }

            // Secret viewer dialog overlay
            if let Some(ref viewer) = self.secret_viewer {
                dialogs::draw_secret_viewer(frame, area, viewer);
            }
        })?;
        Ok(())
    }

    /// Draw a centered skills dialog overlay.
    fn draw_skills_dialog(frame: &mut ratatui::Frame<'_>, area: Rect, state: &PaneState<'_>) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

        let skills = state.skill_manager.get_skills();

        // Size the dialog: width ~60 or 80% of screen, height = skills + 4 (border + header + hint)
        let dialog_w = 60.min(area.width.saturating_sub(4));
        let dialog_h = ((skills.len() as u16) + 4)
            .min(area.height.saturating_sub(4))
            .max(6);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        // Clear the background behind the dialog
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

    /// Draw a centered secrets vault dialog overlay.
    fn draw_secrets_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        state: &mut PaneState<'_>,
        scroll_offset: usize,
    ) {
        use crate::secrets::AccessPolicy;
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

        let creds = state.secrets_manager.list_all_entries();
        let agent_access = state.secrets_manager.has_agent_access();
        let has_totp = state.secrets_manager.has_totp();

        // Size: 70 cols or 90% width, height = creds + header lines + border
        let dialog_w = 70.min(area.width.saturating_sub(4));
        let dialog_h = ((creds.len() as u16) + 8)
            .min(area.height.saturating_sub(4))
            .max(8);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let mut items: Vec<ListItem> = Vec::new();

        // ── Header: agent-access status ─────────────────────────────
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
                    creds.len(),
                    if creds.len() == 1 { "" } else { "s" }
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

        // ── Credential rows ─────────────────────────────────────────
        if creds.is_empty() {
            items.push(ListItem::new(Span::styled(
                "  No credentials stored.",
                Style::default()
                    .fg(tp::MUTED)
                    .add_modifier(Modifier::ITALIC),
            )));
        } else {
            for (i, (name, entry)) in creds.iter().enumerate() {
                let highlight = i == scroll_offset;
                let is_disabled = entry.disabled;

                let row_style = if highlight {
                    tp::selected()
                } else if is_disabled {
                    Style::default().add_modifier(Modifier::DIM | Modifier::CROSSED_OUT)
                } else {
                    Style::default()
                };

                let icon = entry.kind.icon();
                let kind_label = format!(" {:10} ", entry.kind.to_string());

                let badge = if is_disabled {
                    Span::styled(
                        " OFF ",
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(tp::MUTED),
                    )
                } else {
                    let (label, color) = match &entry.policy {
                        AccessPolicy::Always => (" OPEN ", tp::SUCCESS),
                        AccessPolicy::WithApproval => (" ASK ", tp::WARN),
                        AccessPolicy::WithAuth => (" AUTH ", tp::ERROR),
                        AccessPolicy::SkillOnly(skills) if skills.is_empty() => {
                            (" LOCK ", tp::MUTED)
                        }
                        AccessPolicy::SkillOnly(_) => (" SKILL ", tp::INFO),
                    };
                    Span::styled(
                        label,
                        Style::default()
                            .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                            .bg(color),
                    )
                };

                let desc = entry.description.as_deref().unwrap_or("");
                let detail = if desc.is_empty() {
                    format!(" {}", name)
                } else {
                    format!(" {} — {}", name, desc)
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
                    Span::styled(&entry.label, label_style),
                    Span::styled(detail, Style::default().fg(tp::TEXT_DIM).patch(row_style)),
                ])));
            }
        }

        items.push(ListItem::new(""));

        // ── Legend ───────────────────────────────────────────────────
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
