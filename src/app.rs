use anyhow::Result;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use ratatui::prelude::*;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::action::Action;
use crate::commands::{handle_command, CommandAction, CommandContext};
use crate::config::Config;
use crate::gateway::{run_gateway, GatewayOptions};
use crate::pages::hatching::Hatching;
use crate::pages::home::Home;
use crate::pages::Page;
use crate::panes::footer::FooterPane;
use crate::panes::header::HeaderPane;
use crate::panes::{GatewayStatus, InputMode, Pane, PaneState};
use crate::providers;
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;
use crate::soul::SoulManager;
use crate::tui::{Event, EventResponse, Tui};

/// Type alias for the client-side WebSocket write half.
type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// Phase of the API-key dialog overlay.
#[derive(Debug, Clone, PartialEq)]
enum ApiKeyDialogPhase {
    /// Prompting the user to enter an API key (text is masked)
    EnterKey,
    /// Asking whether to store the entered key permanently
    ConfirmStore,
}

/// Spinner state shown while fetching models from a provider API.
struct FetchModelsLoading {
    /// Display name of the provider
    display: String,
    /// Tick counter for the spinner animation
    tick: usize,
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// State for the model-selector dialog overlay.
struct ModelSelectorState {
    /// Provider this selection is for
    provider: String,
    /// Display name
    display: String,
    /// Available model names
    models: Vec<String>,
    /// Currently highlighted index
    selected: usize,
    /// Scroll offset when the list is longer than the dialog
    scroll_offset: usize,
}

/// State for the provider-selector dialog overlay.
struct ProviderSelectorState {
    /// Provider entries: (id, display)
    providers: Vec<(String, String)>,
    /// Currently highlighted index
    selected: usize,
    /// Scroll offset
    scroll_offset: usize,
}

/// Which option is highlighted in the credential-management dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CredDialogOption {
    ViewSecret,
    CopySecret,
    ChangePolicy,
    ToggleDisable,
    Delete,
    SetupTotp,
    Cancel,
}

/// State for the credential-management dialog overlay.
struct CredentialDialogState {
    /// Vault key name of the credential
    name: String,
    /// Whether the credential is currently disabled
    disabled: bool,
    /// Whether 2FA is currently configured for the vault
    has_totp: bool,
    /// Current access policy of the credential
    current_policy: crate::secrets::AccessPolicy,
    /// Currently highlighted menu option
    selected: CredDialogOption,
}

/// Which policy option is highlighted in the policy-picker dialog.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PolicyPickerOption {
    Open,
    Ask,
    Auth,
    Skill,
}

/// Phase of the SKILL-policy sub-flow inside the policy picker.
#[derive(Debug, Clone, PartialEq)]
enum PolicyPickerPhase {
    /// Selecting among OPEN / ASK / AUTH / SKILL
    Selecting,
    /// Editing the skill name list (comma-separated)
    EditingSkills { input: String },
}

/// State for the access-policy picker dialog overlay.
struct PolicyPickerState {
    /// Vault key name of the credential
    cred_name: String,
    /// Currently highlighted policy option
    selected: PolicyPickerOption,
    /// Current dialog phase
    phase: PolicyPickerPhase,
}

/// Phase of the TOTP setup dialog.
#[derive(Debug, Clone, PartialEq)]
enum TotpDialogPhase {
    /// Show the otpauth URL and ask user to enter TOTP code to verify
    ShowUri { uri: String, input: String },
    /// TOTP is already set up — offer to remove it
    AlreadyConfigured,
    /// Verification succeeded
    Verified,
    /// Verification failed — let the user retry
    Failed { uri: String, input: String },
}

/// State for the 2FA (TOTP) setup dialog overlay.
struct TotpDialogState {
    phase: TotpDialogPhase,
}

/// State for the secret-viewer dialog overlay.
struct SecretViewerState {
    /// Vault key name of the credential
    name: String,
    /// Decrypted (label, value) pairs
    fields: Vec<(String, String)>,
    /// Whether the values are currently revealed (unmasked)
    revealed: bool,
    /// Which field is highlighted (for copying)
    selected: usize,
    /// Scroll offset when the list is longer than the dialog
    #[allow(dead_code)]
    scroll_offset: usize,
    /// Transient status message (e.g. "Copied!")
    status: Option<String>,
}

/// Shared state that is separate from the UI components so we can borrow both
/// independently.
struct SharedState {
    config: Config,
    messages: Vec<String>,
    input_mode: InputMode,
    secrets_manager: SecretsManager,
    skill_manager: SkillManager,
    soul_manager: SoulManager,
    gateway_status: GatewayStatus,
    /// Animated loading line shown at the bottom of the messages list.
    loading_line: Option<String>,
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
    /// Handle for the in-process gateway server task (if running)
    gateway_task: Option<JoinHandle<()>>,
    /// Token used to cancel the gateway server task
    gateway_cancel: Option<CancellationToken>,
    /// Write half of the WebSocket client connection to the gateway
    ws_sink: Option<WsSink>,
    /// Handle for the background WebSocket reader task
    reader_task: Option<JoinHandle<()>>,
    /// Whether the skills dialog overlay is visible
    show_skills_dialog: bool,
    /// API-key dialog state
    api_key_dialog: Option<ApiKeyDialogState>,
    /// Model-selector dialog state
    model_selector: Option<ModelSelectorState>,
    /// Loading spinner shown while fetching models
    fetch_loading: Option<FetchModelsLoading>,
    /// Loading spinner shown during device flow authentication
    device_flow_loading: Option<FetchModelsLoading>,
    /// Provider-selector dialog state
    provider_selector: Option<ProviderSelectorState>,
    /// Credential-management dialog state
    credential_dialog: Option<CredentialDialogState>,
    /// 2FA (TOTP) setup dialog state
    totp_dialog: Option<TotpDialogState>,
    /// Secret viewer dialog state
    secret_viewer: Option<SecretViewerState>,
    /// Policy-picker dialog state
    policy_picker: Option<PolicyPickerState>,
    /// Hatching page (shown on first run)
    hatching_page: Option<Hatching>,
    /// Whether we're currently showing the hatching animation
    showing_hatching: bool,
}

/// State for the API-key input dialog overlay.
struct ApiKeyDialogState {
    /// Which provider this key is for
    provider: String,
    /// Display name for the provider
    display: String,
    /// Name of the secret key (e.g. "ANTHROPIC_API_KEY")
    #[allow(dead_code)]
    secret_key: String,
    /// Current input buffer (the API key being typed)
    input: String,
    /// Which phase the dialog is in
    phase: ApiKeyDialogPhase,
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

    fn build(config: Config, mut secrets_manager: SecretsManager) -> Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();

        // Initialise managers
        if !config.use_secrets {
            secrets_manager.set_agent_access(false);
        } else {
            secrets_manager.set_agent_access(config.agent_access);
        }

        let skills_dir = config.skills_dir();
        let mut skill_manager = SkillManager::new(skills_dir);
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

        let gateway_status = GatewayStatus::Disconnected;

        let state = SharedState {
            config,
            messages: vec!["Welcome to RustyClaw! Type /help for commands.".to_string()],
            input_mode: InputMode::Normal,
            secrets_manager,
            skill_manager,
            soul_manager,
            gateway_status,
            loading_line: None,
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
            gateway_task: None,
            gateway_cancel: None,
            ws_sink: None,
            reader_task: None,
            show_skills_dialog: false,
            api_key_dialog: None,
            model_selector: None,
            fetch_loading: None,
            device_flow_loading: None,
            provider_selector: None,
            credential_dialog: None,
            totp_dialog: None,
            secret_viewer: None,
            policy_picker: None,
            hatching_page,
            showing_hatching,
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
                        // While loading, Esc cancels the active async operation
                        else if self.fetch_loading.is_some() || self.device_flow_loading.is_some() {
                            if let Event::Key(key) = &event {
                                if key.code == crossterm::event::KeyCode::Esc {
                                    if self.device_flow_loading.is_some() {
                                        self.device_flow_loading = None;
                                        self.state.loading_line = None;
                                        self.state.messages.push(
                                            "Device flow authentication cancelled.".to_string(),
                                        );
                                    } else {
                                        self.fetch_loading = None;
                                        self.state.loading_line = None;
                                        self.state.messages.push(
                                            "Model fetch cancelled.".to_string(),
                                        );
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
                // Parse the gateway JSON envelope and extract the payload.
                // The gateway wraps responses as {"type":"response","received":"…"}.
                let payload = serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|v| {
                        // Skip non-response frames (e.g. the initial "hello")
                        if v.get("type").and_then(|t| t.as_str()) == Some("response") {
                            v.get("received").and_then(|r| r.as_str()).map(String::from)
                        } else {
                            None
                        }
                    });

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
                self.state.messages.push(format!("◀ {}", display));
                // Auto-scroll
                return Ok(Some(Action::Update));
            }
            Action::GatewayDisconnected(reason) => {
                self.state.gateway_status = GatewayStatus::Disconnected;
                self.state.messages.push(format!("Gateway disconnected: {}", reason));
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
                }
                // Fall through so panes also get Tick
            }
            Action::ShowSkills => {
                self.show_skills_dialog = !self.show_skills_dialog;
                return Ok(None);
            }
            Action::ShowProviderSelector => {
                self.open_provider_selector();
                return Ok(None);
            }
            Action::SetProvider(provider) => {
                let provider = provider.clone();
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
                    self.state.messages.push(format!("Failed to save config: {}", e));
                } else {
                    self.state.messages.push(format!("Provider set to {}.", provider));
                }
                // Check auth method and proceed accordingly
                let def = providers::provider_by_id(&provider);
                let auth_method = def.map(|d| d.auth_method)
                    .unwrap_or(providers::AuthMethod::ApiKey);

                match auth_method {
                    providers::AuthMethod::DeviceFlow => {
                        if let Some(secret_key) = providers::secret_key_for_provider(&provider) {
                            match self.state.secrets_manager.get_secret(secret_key, true) {
                                Ok(Some(_)) => {
                                    self.state.messages.push(format!(
                                        "✓ Access token for {} is already stored.",
                                        providers::display_name_for_provider(&provider),
                                    ));
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
                                    self.state.messages.push(format!(
                                        "✓ API key for {} is already stored.",
                                        providers::display_name_for_provider(&provider),
                                    ));
                                    return Ok(Some(Action::FetchModels(provider)));
                                }
                                _ => {
                                    return Ok(Some(Action::PromptApiKey(provider)));
                                }
                            }
                        }
                    }
                    providers::AuthMethod::None => {
                        self.state.messages.push(format!(
                            "{} does not require authentication.",
                            providers::display_name_for_provider(&provider),
                        ));
                        return Ok(Some(Action::FetchModels(provider)));
                    }
                }
                return Ok(None);
            }
            Action::PromptApiKey(provider) => {
                return Ok(self.open_api_key_dialog(provider.clone()));
            }
            Action::ConfirmStoreSecret { provider, key } => {
                return self.handle_confirm_store_secret(provider.clone(), key.clone());
            }
            Action::FetchModels(provider) => {
                self.spawn_fetch_models(provider.clone());
                return Ok(None);
            }
            Action::FetchModelsFailed(msg) => {
                self.fetch_loading = None;
                self.state.loading_line = None;
                self.state.messages.push(msg.clone());
                return Ok(Some(Action::Update));
            }
            Action::ShowModelSelector { provider, models } => {
                self.fetch_loading = None;
                self.state.loading_line = None;
                self.open_model_selector(provider.clone(), models.clone());
                return Ok(None);
            }
            Action::StartDeviceFlow(provider) => {
                self.spawn_device_flow(provider.clone());
                return Ok(None);
            }
            Action::DeviceFlowCodeReady { url, code } => {
                self.state.messages.push(format!(
                    "Open this URL in your browser:",
                ));
                self.state.messages.push(format!(
                    "  ➜  {}", url,
                ));
                self.state.messages.push(format!(
                    "Then enter this code:  {}", code,
                ));
                return Ok(Some(Action::Update));
            }
            Action::DeviceFlowAuthenticated { provider, token } => {
                self.device_flow_loading = None;
                self.state.loading_line = None;
                let secret_key = providers::secret_key_for_provider(provider)
                    .unwrap_or("COPILOT_TOKEN");
                let display = providers::display_name_for_provider(provider).to_string();
                match self.state.secrets_manager.store_secret(secret_key, token) {
                    Ok(()) => {
                        self.state.messages.push(format!(
                            "✓ {} authenticated successfully. Token stored.",
                            display,
                        ));
                    }
                    Err(e) => {
                        self.state.messages.push(format!(
                            "Failed to store token: {}. Token set for this session only.",
                            e,
                        ));
                    }
                }
                // Proceed to model selection
                return Ok(Some(Action::FetchModels(provider.clone())));
            }
            Action::DeviceFlowFailed(msg) => {
                self.device_flow_loading = None;
                self.state.loading_line = None;
                self.state.messages.push(msg.clone());
                return Ok(Some(Action::Update));
            }
            Action::ShowCredentialDialog { name, disabled, .. } => {
                let has_totp = self.state.secrets_manager.has_totp();
                // Look up the actual current policy from the vault metadata
                let current_policy = self.state.secrets_manager
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
                            self.state.messages.push(format!(
                                "Failed to set up 2FA: {}", e,
                            ));
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
            Action::BeginHatchingExchange | Action::HatchingSendMessage(_) => {
                // Build a structured chat request with full conversation history
                // and send it to the gateway for model completion.
                let messages = self.hatching_page.as_ref()
                    .map(|h| h.chat_messages());
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
                    self.state.messages.push(format!("Failed to save SOUL.md: {}", e));
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
                    for msg in response.messages {
                        self.state.messages.push(msg);
                    }
                }
                CommandAction::GatewayStart => {
                    for msg in response.messages {
                        self.state.messages.push(msg);
                    }
                    return Ok(Some(Action::ReconnectGateway));
                }
                CommandAction::GatewayStop => {
                    for msg in response.messages {
                        self.state.messages.push(msg);
                    }
                    return Ok(Some(Action::DisconnectGateway));
                }
                CommandAction::GatewayRestart => {
                    for msg in response.messages {
                        self.state.messages.push(msg);
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
                    self.state.messages.push(format!(
                        "Gateway: {}  Status: {}",
                        url_display,
                        self.state.gateway_status.label()
                    ));
                }
                CommandAction::SetProvider(ref provider) => {
                    for msg in &response.messages {
                        self.state.messages.push(msg.clone());
                    }
                    return Ok(Some(Action::SetProvider(provider.clone())));
                }
                CommandAction::SetModel(ref model) => {
                    for msg in &response.messages {
                        self.state.messages.push(msg.clone());
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
                        self.state.messages.push(format!("Failed to save config: {}", e));
                    } else {
                        self.state.messages.push(format!("Model set to {}.", model));
                    }
                }
                CommandAction::ShowSkills => {
                    return Ok(Some(Action::ShowSkills));
                }
                CommandAction::ShowProviderSelector => {
                    return Ok(Some(Action::ShowProviderSelector));
                }
                CommandAction::None => {
                    for msg in response.messages {
                        self.state.messages.push(msg);
                    }
                }
            }

            Ok(Some(Action::TimedStatusLine(text, 3)))
        } else {
            // It's a plain prompt
            self.state.messages.push(format!("▶ {}", text));
            if self.state.gateway_status == GatewayStatus::Connected && self.ws_sink.is_some() {
                return Ok(Some(Action::SendToGateway(text)));
            }
            self.state
                .messages
                .push("(Gateway not connected — use /gateway start)".to_string());
            Ok(Some(Action::Update))
        }
    }

    /// Start the gateway server in-process, then connect to it as a client.
    async fn start_gateway(&mut self) {
        const DEFAULT_GATEWAY_URL: &str = "ws://127.0.0.1:9001";

        let url = self
            .state
            .config
            .gateway_url
            .clone()
            .unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string());

        // If already running, report and return.
        if self.gateway_task.is_some() {
            self.state
                .messages
                .push("Gateway is already running.".to_string());
            return;
        }

        self.state.gateway_status = GatewayStatus::Connecting;
        self.state
            .messages
            .push(format!("Starting gateway on {}…", url));

        // Resolve model context from config + secrets before spawning.
        let model_ctx = crate::gateway::ModelContext::resolve(
            &self.state.config,
            &mut self.state.secrets_manager,
        )
        .ok();

        // Spawn the gateway server as a background task.
        let cancel = CancellationToken::new();
        let cancel_child = cancel.clone();
        let config_clone = self.state.config.clone();
        let listen_url = url.clone();
        let handle = tokio::spawn(async move {
            let opts = GatewayOptions {
                listen: listen_url,
            };
            if let Err(err) = run_gateway(config_clone, opts, model_ctx, cancel_child).await {
                eprintln!("Gateway server error: {}", err);
            }
        });
        self.gateway_task = Some(handle);
        self.gateway_cancel = Some(cancel);

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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
                    .push(format!("Connected to gateway {}", url));

                // Spawn a background task that reads from the gateway and
                // forwards messages into the TUI event loop via action_tx.
                let tx = self.action_tx.clone();
                self.reader_task = Some(tokio::spawn(async move {
                    Self::gateway_reader_loop(stream, tx).await;
                }));
            }
            Err(err) => {
                self.state.gateway_status = GatewayStatus::Error;
                self.state
                    .messages
                    .push(format!("Gateway connection failed: {}", err));
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

    /// Build a structured JSON chat request for the hatching exchange.
    ///
    /// The request includes the full conversation history (system prompt +
    /// all user/assistant turns).  The gateway owns the model configuration
    /// and API credentials — resolved at startup from config + secrets vault
    /// — so the client only needs to send the messages.
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

    /// Send a text message to the gateway over the open WebSocket connection.
    async fn send_to_gateway(&mut self, text: String) {
        if let Some(ref mut sink) = self.ws_sink {
            match sink.send(Message::Text(text.into())).await {
                Ok(()) => {}
                Err(err) => {
                    self.state.messages.push(format!("Send failed: {}", err));
                    self.state.gateway_status = GatewayStatus::Error;
                    self.ws_sink = None;
                }
            }
        } else {
            self.state
                .messages
                .push("Cannot send: gateway not connected.".to_string());
        }
    }

    /// Stop the gateway: close the client connection and cancel the server task.
    async fn stop_gateway(&mut self) {
        let was_running = self.gateway_task.is_some() || self.ws_sink.is_some();

        // Abort the reader task first so it doesn't fire a disconnect action.
        if let Some(handle) = self.reader_task.take() {
            handle.abort();
        }

        // Close the client-side WebSocket gracefully.
        if let Some(mut sink) = self.ws_sink.take() {
            let _ = sink.send(Message::Close(None)).await;
            let _ = sink.close().await;
        }

        // Cancel the server task.
        if let Some(cancel) = self.gateway_cancel.take() {
            cancel.cancel();
        }
        if let Some(handle) = self.gateway_task.take() {
            // Give it a moment to wind down; don't block forever.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                handle,
            )
            .await;
        }

        if was_running {
            self.state.gateway_status = GatewayStatus::Disconnected;
            self.state
                .messages
                .push("Gateway stopped.".to_string());
        } else {
            self.state
                .messages
                .push("Gateway is not running.".to_string());
        }
    }

    /// Restart: stop, let the TUI render the disconnect, then reconnect.
    ///
    /// We stop synchronously so the status flips to Disconnected immediately,
    /// then schedule ReconnectGateway via the action channel after a short
    /// delay so the event loop renders at least one frame showing the
    /// intermediate state before the connection attempt begins.
    async fn restart_gateway(&mut self) {
        self.stop_gateway().await;

        // Schedule the reconnect after a brief pause so the render loop can
        // show the Disconnected status before we start connecting again.
        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = tx.send(Action::ReconnectGateway);
        });
    }

    fn draw(&mut self, tui: &mut Tui) -> Result<()> {
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

            // API key dialog overlay
            if let Some(ref dialog) = self.api_key_dialog {
                Self::draw_api_key_dialog(frame, area, dialog);
            }

            // Provider selector dialog overlay
            if let Some(ref selector) = self.provider_selector {
                Self::draw_provider_selector_dialog(frame, area, selector);
            }

            // Model selector dialog overlay
            if let Some(ref selector) = self.model_selector {
                Self::draw_model_selector_dialog(frame, area, selector);
            }

            // Credential management dialog overlay
            if let Some(ref dialog) = self.credential_dialog {
                Self::draw_credential_dialog(frame, area, dialog);
            }

            // Policy picker dialog overlay
            if let Some(ref picker) = self.policy_picker {
                Self::draw_policy_picker(frame, area, picker);
            }

            // TOTP setup dialog overlay
            if let Some(ref dialog) = self.totp_dialog {
                Self::draw_totp_dialog(frame, area, dialog);
            }

            // Secret viewer dialog overlay
            if let Some(ref viewer) = self.secret_viewer {
                Self::draw_secret_viewer(frame, area, viewer);
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
        let dialog_h = ((skills.len() as u16) + 4).min(area.height.saturating_sub(4)).max(6);
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
                    .title(Span::styled(
                        " Skills ",
                        tp::title_focused(),
                    ))
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

    // ── API-key dialog ──────────────────────────────────────────────────────

    /// Open the API-key input dialog for the given provider.
    fn open_api_key_dialog(&mut self, provider: String) -> Option<Action> {
        let secret_key = match providers::secret_key_for_provider(&provider) {
            Some(k) => k.to_string(),
            None => return None, // shouldn't happen, but just in case
        };
        let display = providers::display_name_for_provider(&provider).to_string();
        self.state.messages.push(format!(
            "No API key found for {}. Please enter one below.",
            display,
        ));
        self.api_key_dialog = Some(ApiKeyDialogState {
            provider,
            display,
            secret_key,
            input: String::new(),
            phase: ApiKeyDialogPhase::EnterKey,
        });
        None
    }

    /// Handle key events when the API key dialog is open.
    fn handle_api_key_dialog_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        use crossterm::event::KeyCode;

        // Take the dialog state so we can mutate it without borrowing self
        let Some(mut dialog) = self.api_key_dialog.take() else {
            return Action::Noop;
        };

        match dialog.phase {
            ApiKeyDialogPhase::EnterKey => match code {
                KeyCode::Esc => {
                    self.state
                        .messages
                        .push("API key entry cancelled.".to_string());
                    // dialog is already taken — dropped
                    return Action::Noop;
                }
                KeyCode::Enter => {
                    if dialog.input.is_empty() {
                        self.state.messages.push(
                            "No key entered — you can add one later with /provider."
                                .to_string(),
                        );
                        return Action::Noop;
                    }
                    // Move to confirmation phase
                    dialog.phase = ApiKeyDialogPhase::ConfirmStore;
                    self.api_key_dialog = Some(dialog);
                    return Action::Noop;
                }
                KeyCode::Backspace => {
                    dialog.input.pop();
                    self.api_key_dialog = Some(dialog);
                    return Action::Noop;
                }
                KeyCode::Char(c) => {
                    dialog.input.push(c);
                    self.api_key_dialog = Some(dialog);
                    return Action::Noop;
                }
                _ => {
                    self.api_key_dialog = Some(dialog);
                    return Action::Noop;
                }
            },
            ApiKeyDialogPhase::ConfirmStore => match code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    // Store it
                    let provider = dialog.provider.clone();
                    let key = dialog.input.clone();
                    return Action::ConfirmStoreSecret { provider, key };
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    // Use the key for this session but don't store
                    self.state.messages.push(format!(
                        "✓ API key for {} set for this session (not stored).",
                        dialog.display,
                    ));
                    // Proceed to model selection
                    return Action::FetchModels(dialog.provider.clone());
                }
                _ => {
                    self.api_key_dialog = Some(dialog);
                    return Action::Noop;
                }
            },
        }
    }

    /// Store the API key in the secrets vault after user confirmation.
    fn handle_confirm_store_secret(
        &mut self,
        provider: String,
        key: String,
    ) -> Result<Option<Action>> {
        let secret_key = providers::secret_key_for_provider(&provider)
            .unwrap_or("API_KEY");
        let display = providers::display_name_for_provider(&provider).to_string();

        match self.state.secrets_manager.store_secret(&secret_key, &key) {
            Ok(()) => {
                self.state.messages.push(format!(
                    "✓ API key for {} stored securely.",
                    display,
                ));
            }
            Err(e) => {
                self.state.messages.push(format!(
                    "Failed to store API key: {}. Key is set for this session only.",
                    e,
                ));
            }
        }
        // After storing the key, proceed to model selection
        Ok(Some(Action::FetchModels(provider)))
    }

    /// Draw a centered API-key dialog overlay.
    fn draw_api_key_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        dialog: &ApiKeyDialogState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{Block, Borders, Clear, Paragraph};

        let dialog_w = 56.min(area.width.saturating_sub(4));
        let dialog_h = 7_u16.min(area.height.saturating_sub(4)).max(5);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        // Clear the background behind the dialog
        frame.render_widget(Clear, dialog_area);

        let title = format!(" {} API Key ", dialog.display);
        let block = Block::default()
            .title(Span::styled(&title, tp::title_focused()))
            .title_bottom(
                Line::from(Span::styled(
                    " Esc to cancel ",
                    Style::default().fg(tp::MUTED),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(tp::focused_border())
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        match dialog.phase {
            ApiKeyDialogPhase::EnterKey => {
                // Label
                let label = Line::from(Span::styled(
                    format!(" Enter your {} API key:", dialog.display),
                    Style::default().fg(tp::TEXT),
                ));
                if inner.height >= 1 {
                    frame.render_widget(
                        Paragraph::new(label),
                        Rect::new(inner.x, inner.y, inner.width, 1),
                    );
                }

                // Masked input
                if inner.height >= 3 {
                    let masked: String = "•".repeat(dialog.input.len());
                    let input_area = Rect::new(inner.x + 1, inner.y + 2, inner.width.saturating_sub(2), 1);
                    let prompt = Line::from(vec![
                        Span::styled("❯ ", Style::default().fg(tp::ACCENT)),
                        Span::styled(&masked, Style::default().fg(tp::TEXT)),
                    ]);
                    frame.render_widget(Paragraph::new(prompt), input_area);

                    // Show cursor
                    frame.set_cursor_position((
                        input_area.x + 2 + masked.len() as u16,
                        input_area.y,
                    ));
                }
            }
            ApiKeyDialogPhase::ConfirmStore => {
                // Show key length hint
                let hint = Line::from(Span::styled(
                    format!(" Key entered ({} chars).", dialog.input.len()),
                    Style::default().fg(tp::SUCCESS),
                ));
                if inner.height >= 1 {
                    frame.render_widget(
                        Paragraph::new(hint),
                        Rect::new(inner.x, inner.y, inner.width, 1),
                    );
                }

                // Store question
                if inner.height >= 3 {
                    let question = Line::from(vec![
                        Span::styled(
                            " Store permanently in secrets vault? ",
                            Style::default().fg(tp::TEXT),
                        ),
                        Span::styled(
                            "[Y/n]",
                            Style::default()
                                .fg(tp::ACCENT_BRIGHT)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]);
                    frame.render_widget(
                        Paragraph::new(question),
                        Rect::new(inner.x, inner.y + 2, inner.width, 1),
                    );
                }
            }
        }
    }

    // ── Model selector dialog ───────────────────────────────────────────────

    /// Spawn a background task to fetch models and send the result back
    /// via the action channel.  Shows an inline loading line in the meantime.
    fn spawn_fetch_models(&mut self, provider: String) {
        let display = providers::display_name_for_provider(&provider).to_string();
        self.state.messages.push(format!(
            "Fetching available models for {}…",
            display,
        ));

        // Show the inline loading line under the chat log
        let spinner = SPINNER_FRAMES[0];
        self.state.loading_line = Some(format!(
            "  {} Fetching models from {}…",
            spinner, display,
        ));
        self.fetch_loading = Some(FetchModelsLoading {
            display: display.clone(),
            tick: 0,
        });

        // Gather what we need for the background task
        let api_key = providers::secret_key_for_provider(&provider)
            .and_then(|sk| {
                self.state
                    .secrets_manager
                    .get_secret(sk, true)
                    .ok()
                    .flatten()
            });

        let base_url = self
            .state
            .config
            .model
            .as_ref()
            .and_then(|m| m.base_url.clone());

        let tx = self.action_tx.clone();
        let provider_clone = provider.clone();

        tokio::spawn(async move {
            match providers::fetch_models(
                &provider_clone,
                api_key.as_deref(),
                base_url.as_deref(),
            )
            .await
            {
                Ok(models) => {
                    let _ = tx.send(Action::ShowModelSelector {
                        provider: provider_clone,
                        models,
                    });
                }
                Err(err) => {
                    let _ = tx.send(Action::FetchModelsFailed(err));
                }
            }
        });
    }

    /// Draw a centered loading spinner overlay.

    // ── Device flow authentication ──────────────────────────────────────────

    /// Spawn a background task to perform OAuth device flow authentication.
    /// Shows the verification URL and user code as messages, then polls for
    /// the token in the background.
    fn spawn_device_flow(&mut self, provider: String) {
        let def = match providers::provider_by_id(&provider) {
            Some(d) => d,
            None => {
                self.state.messages.push(format!("Unknown provider: {}", provider));
                return;
            }
        };

        let device_config = match def.device_flow {
            Some(cfg) => cfg,
            None => {
                self.state.messages.push(format!(
                    "{} does not support device flow authentication.",
                    def.display,
                ));
                return;
            }
        };

        let display = def.display.to_string();
        self.state.messages.push(format!(
            "Authenticating with {}…",
            display,
        ));

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
        // All fields of DeviceFlowConfig are &'static str, so we can just
        // copy the reference to the static config into the spawned task.
        let device_cfg: &'static providers::DeviceFlowConfig = device_config;

        tokio::spawn(async move {
            // Step 1: Start the device flow
            let auth = match providers::start_device_flow(device_cfg).await {
                Ok(a) => a,
                Err(e) => {
                    let _ = tx.send(Action::DeviceFlowFailed(format!(
                        "Failed to start device flow: {}", e,
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
                            "Authentication failed: {}", e,
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

    /// Open the model selector dialog with the given list.
    fn open_model_selector(&mut self, provider: String, models: Vec<String>) {
        let display = providers::display_name_for_provider(&provider).to_string();
        self.model_selector = Some(ModelSelectorState {
            provider,
            display,
            models,
            selected: 0,
            scroll_offset: 0,
        });
    }

    /// Handle key events when the model selector dialog is open.
    fn handle_model_selector_key(&mut self, code: crossterm::event::KeyCode) -> Action {
        use crossterm::event::KeyCode;

        let Some(mut sel) = self.model_selector.take() else {
            return Action::Noop;
        };

        // Maximum visible rows in the dialog body
        const MAX_VISIBLE: usize = 14;

        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state
                    .messages
                    .push("Model selection cancelled.".to_string());
                return Action::Noop;
            }
            KeyCode::Enter => {
                if let Some(model_name) = sel.models.get(sel.selected).cloned() {
                    // Save the selected model
                    let model_cfg =
                        self.state.config.model.get_or_insert_with(|| {
                            crate::config::ModelProvider {
                                provider: sel.provider.clone(),
                                model: None,
                                base_url: None,
                            }
                        });
                    model_cfg.model = Some(model_name.clone());
                    if let Err(e) = self.state.config.save(None) {
                        self.state
                            .messages
                            .push(format!("Failed to save config: {}", e));
                    } else {
                        self.state.messages.push(format!(
                            "✓ Model set to {}.",
                            model_name,
                        ));
                    }
                }
                return Action::Update;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if sel.selected > 0 {
                    sel.selected -= 1;
                    if sel.selected < sel.scroll_offset {
                        sel.scroll_offset = sel.selected;
                    }
                }
                self.model_selector = Some(sel);
                return Action::Noop;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if sel.selected + 1 < sel.models.len() {
                    sel.selected += 1;
                    if sel.selected >= sel.scroll_offset + MAX_VISIBLE {
                        sel.scroll_offset = sel.selected - MAX_VISIBLE + 1;
                    }
                }
                self.model_selector = Some(sel);
                return Action::Noop;
            }
            KeyCode::Home => {
                sel.selected = 0;
                sel.scroll_offset = 0;
                self.model_selector = Some(sel);
                return Action::Noop;
            }
            KeyCode::End => {
                sel.selected = sel.models.len().saturating_sub(1);
                sel.scroll_offset = sel
                    .models
                    .len()
                    .saturating_sub(MAX_VISIBLE);
                self.model_selector = Some(sel);
                return Action::Noop;
            }
            _ => {
                self.model_selector = Some(sel);
                return Action::Noop;
            }
        }
    }

    /// Draw a centered model-selector dialog overlay.
    fn draw_model_selector_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        sel: &ModelSelectorState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

        const MAX_VISIBLE: usize = 14;

        let dialog_w = 60.min(area.width.saturating_sub(4));
        let visible_count = sel.models.len().min(MAX_VISIBLE);
        // +4 for border (2) + title line + hint line
        let dialog_h = ((visible_count as u16) + 4)
            .min(area.height.saturating_sub(4))
            .max(6);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        // Clear the background behind the dialog
        frame.render_widget(Clear, dialog_area);

        let title = format!(" Select a {} model ", sel.display);
        let hint = if sel.models.len() > MAX_VISIBLE {
            format!(
                " {}/{} · ↑↓ navigate · Enter select · Esc cancel ",
                sel.selected + 1,
                sel.models.len(),
            )
        } else {
            " ↑↓ navigate · Enter select · Esc cancel ".to_string()
        };

        let block = Block::default()
            .title(Span::styled(&title, tp::title_focused()))
            .title_bottom(
                Line::from(Span::styled(
                    &hint,
                    Style::default().fg(tp::MUTED),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(tp::focused_border())
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let end = (sel.scroll_offset + MAX_VISIBLE).min(sel.models.len());
        let visible_models = &sel.models[sel.scroll_offset..end];

        let items: Vec<ListItem> = visible_models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let abs_idx = sel.scroll_offset + i;
                let is_selected = abs_idx == sel.selected;
                let (marker, style) = if is_selected {
                    (
                        "❯ ",
                        Style::default()
                            .fg(tp::ACCENT_BRIGHT)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", Style::default().fg(tp::TEXT))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(marker, Style::default().fg(tp::ACCENT)),
                    Span::styled(model.as_str(), style),
                ]))
            })
            .collect();

        let list =
            List::new(items).style(Style::default().fg(tp::TEXT));

        frame.render_widget(list, inner);
    }

    // ── Provider selector dialog ──────────────────────────────

    /// Open the provider-selector dialog populated from the shared
    /// provider registry.
    fn open_provider_selector(&mut self) {
        let providers: Vec<(String, String)> = providers::PROVIDERS
            .iter()
            .map(|p| (p.id.to_string(), p.display.to_string()))
            .collect();
        self.provider_selector = Some(ProviderSelectorState {
            providers,
            selected: 0,
            scroll_offset: 0,
        });
    }

    /// Handle key events when the provider selector dialog is open.
    fn handle_provider_selector_key(
        &mut self,
        code: crossterm::event::KeyCode,
    ) -> Action {
        use crossterm::event::KeyCode;

        let Some(mut sel) = self.provider_selector.take() else {
            return Action::Noop;
        };

        const MAX_VISIBLE: usize = 14;

        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.state
                    .messages
                    .push("Provider selection cancelled.".to_string());
                return Action::Noop;
            }
            KeyCode::Enter => {
                if let Some((id, display)) =
                    sel.providers.get(sel.selected).cloned()
                {
                    self.state.messages.push(format!(
                        "Switching provider to {}\u{2026}",
                        display,
                    ));
                    return Action::SetProvider(id);
                }
                return Action::Noop;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if sel.selected > 0 {
                    sel.selected -= 1;
                    if sel.selected < sel.scroll_offset {
                        sel.scroll_offset = sel.selected;
                    }
                }
                self.provider_selector = Some(sel);
                return Action::Noop;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if sel.selected + 1 < sel.providers.len() {
                    sel.selected += 1;
                    if sel.selected >= sel.scroll_offset + MAX_VISIBLE {
                        sel.scroll_offset =
                            sel.selected - MAX_VISIBLE + 1;
                    }
                }
                self.provider_selector = Some(sel);
                return Action::Noop;
            }
            KeyCode::Home => {
                sel.selected = 0;
                sel.scroll_offset = 0;
                self.provider_selector = Some(sel);
                return Action::Noop;
            }
            KeyCode::End => {
                sel.selected =
                    sel.providers.len().saturating_sub(1);
                sel.scroll_offset =
                    sel.providers.len().saturating_sub(MAX_VISIBLE);
                self.provider_selector = Some(sel);
                return Action::Noop;
            }
            _ => {
                self.provider_selector = Some(sel);
                return Action::Noop;
            }
        }
    }

    /// Draw a centered provider-selector dialog overlay.
    fn draw_provider_selector_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        sel: &ProviderSelectorState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{
            Block, Borders, Clear, List, ListItem,
        };

        const MAX_VISIBLE: usize = 14;

        let dialog_w = 50.min(area.width.saturating_sub(4));
        let visible_count = sel.providers.len().min(MAX_VISIBLE);
        let dialog_h = ((visible_count as u16) + 4)
            .min(area.height.saturating_sub(4))
            .max(6);
        let x =
            area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y =
            area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let title = " Select a provider ";
        let hint = if sel.providers.len() > MAX_VISIBLE {
            format!(
                " {}/{} · ↑↓ navigate · Enter select · Esc cancel ",
                sel.selected + 1,
                sel.providers.len(),
            )
        } else {
            " ↑↓ navigate · Enter select · Esc cancel ".to_string()
        };

        let block = Block::default()
            .title(Span::styled(title, tp::title_focused()))
            .title_bottom(
                Line::from(Span::styled(
                    &hint,
                    Style::default().fg(tp::MUTED),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(tp::focused_border())
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let end = (sel.scroll_offset + MAX_VISIBLE)
            .min(sel.providers.len());
        let visible = &sel.providers[sel.scroll_offset..end];

        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, (_id, display))| {
                let abs_idx = sel.scroll_offset + i;
                let is_selected = abs_idx == sel.selected;
                let (marker, style) = if is_selected {
                    (
                        "❯ ",
                        Style::default()
                            .fg(tp::ACCENT_BRIGHT)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", Style::default().fg(tp::TEXT))
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        marker,
                        Style::default().fg(tp::ACCENT),
                    ),
                    Span::styled(display.as_str(), style),
                ]))
            })
            .collect();

        let list =
            List::new(items).style(Style::default().fg(tp::TEXT));

        frame.render_widget(list, inner);
    }

    // ── Credential management dialog ──────────────────────────

    /// Handle key events when the credential dialog is open.
    fn handle_credential_dialog_key(
        &mut self,
        code: crossterm::event::KeyCode,
    ) -> Action {
        use crossterm::event::KeyCode;

        let Some(mut dlg) = self.credential_dialog.take() else {
            return Action::Noop;
        };

        let options = [
            CredDialogOption::ViewSecret,
            CredDialogOption::CopySecret,
            CredDialogOption::ChangePolicy,
            CredDialogOption::ToggleDisable,
            CredDialogOption::Delete,
            CredDialogOption::SetupTotp,
            CredDialogOption::Cancel,
        ];

        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // Close without action
                return Action::Noop;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let cur = options.iter().position(|o| *o == dlg.selected).unwrap_or(0);
                let next = if cur == 0 { options.len() - 1 } else { cur - 1 };
                dlg.selected = options[next];
                self.credential_dialog = Some(dlg);
                return Action::Noop;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = options.iter().position(|o| *o == dlg.selected).unwrap_or(0);
                let next = (cur + 1) % options.len();
                dlg.selected = options[next];
                self.credential_dialog = Some(dlg);
                return Action::Noop;
            }
            KeyCode::Enter => {
                match dlg.selected {
                    CredDialogOption::ViewSecret => {
                        match self.state.secrets_manager.peek_credential_display(&dlg.name) {
                            Ok(fields) => {
                                self.secret_viewer = Some(SecretViewerState {
                                    name: dlg.name.clone(),
                                    fields,
                                    revealed: false,
                                    selected: 0,
                                    scroll_offset: 0,
                                    status: None,
                                });
                            }
                            Err(e) => {
                                self.state.messages.push(format!(
                                    "Failed to read secret: {}", e,
                                ));
                            }
                        }
                        return Action::Noop;
                    }
                    CredDialogOption::CopySecret => {
                        match self.state.secrets_manager.peek_credential_display(&dlg.name) {
                            Ok(fields) => {
                                // Copy the first (or only) value to clipboard.
                                let text = if fields.len() == 1 {
                                    fields[0].1.clone()
                                } else {
                                    // Multi-field: join as "Label: Value" lines.
                                    fields.iter()
                                        .map(|(k, v)| format!("{}: {}", k, v))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                };
                                match Self::copy_to_clipboard(&text) {
                                    Ok(()) => {
                                        self.state.messages.push(format!(
                                            "Credential '{}' copied to clipboard.", dlg.name,
                                        ));
                                    }
                                    Err(e) => {
                                        self.state.messages.push(format!(
                                            "Failed to copy: {}", e,
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                self.state.messages.push(format!(
                                    "Failed to read secret: {}", e,
                                ));
                            }
                        }
                        return Action::Update;
                    }
                    CredDialogOption::ChangePolicy => {
                        // Determine the currently selected policy picker option
                        let selected = match &dlg.current_policy {
                            crate::secrets::AccessPolicy::Always => PolicyPickerOption::Open,
                            crate::secrets::AccessPolicy::WithApproval => PolicyPickerOption::Ask,
                            crate::secrets::AccessPolicy::WithAuth => PolicyPickerOption::Auth,
                            crate::secrets::AccessPolicy::SkillOnly(_) => PolicyPickerOption::Skill,
                        };
                        self.policy_picker = Some(PolicyPickerState {
                            cred_name: dlg.name.clone(),
                            selected,
                            phase: PolicyPickerPhase::Selecting,
                        });
                        return Action::Noop;
                    }
                    CredDialogOption::ToggleDisable => {
                        let new_state = !dlg.disabled;
                        match self.state.secrets_manager.set_credential_disabled(&dlg.name, new_state) {
                            Ok(()) => {
                                let verb = if new_state { "disabled" } else { "enabled" };
                                self.state.messages.push(format!(
                                    "Credential '{}' {}.", dlg.name, verb,
                                ));
                            }
                            Err(e) => {
                                self.state.messages.push(format!(
                                    "Failed to update credential: {}", e,
                                ));
                            }
                        }
                    }
                    CredDialogOption::Delete => {
                        // For legacy bare keys, also delete the raw key
                        let meta_key = format!("cred:{}", dlg.name);
                        let is_legacy = self.state.secrets_manager
                            .get_secret(&meta_key, true)
                            .ok()
                            .flatten()
                            .is_none();

                        if is_legacy {
                            let _ = self.state.secrets_manager.delete_secret(&dlg.name);
                        }
                        match self.state.secrets_manager.delete_credential(&dlg.name) {
                            Ok(()) => {
                                self.state.messages.push(format!(
                                    "Credential '{}' deleted.", dlg.name,
                                ));
                            }
                            Err(e) => {
                                self.state.messages.push(format!(
                                    "Failed to delete credential: {}", e,
                                ));
                            }
                        }
                    }
                    CredDialogOption::SetupTotp => {
                        return Action::ShowTotpSetup;
                    }
                    CredDialogOption::Cancel => {
                        // Close
                    }
                }
                return Action::Update;
            }
            _ => {
                self.credential_dialog = Some(dlg);
                return Action::Noop;
            }
        }
    }

    /// Draw a centered credential-management dialog overlay.
    fn draw_credential_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        dlg: &CredentialDialogState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{
            Block, Borders, Clear, List, ListItem,
        };

        let dialog_w = 50.min(area.width.saturating_sub(4));
        let dialog_h = 14u16.min(area.height.saturating_sub(4)).max(9);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let title = format!(" {} ", dlg.name);
        let hint = " ↑↓ navigate · Enter select · Esc cancel ";

        let block = Block::default()
            .title(Span::styled(&title, tp::title_focused()))
            .title_bottom(
                Line::from(Span::styled(
                    hint,
                    Style::default().fg(tp::MUTED),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(tp::focused_border())
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let toggle_label = if dlg.disabled {
            "  Enable credential"
        } else {
            "  Disable credential"
        };

        let totp_label = if dlg.has_totp {
            "🔒 Manage 2FA (TOTP)"
        } else {
            "🔒 Set up 2FA (TOTP)"
        };

        let policy_label = format!("🛡 Change policy [{}]", dlg.current_policy.badge());

        let menu_items: Vec<(String, CredDialogOption)> = vec![
            ("👁 View secret".to_string(), CredDialogOption::ViewSecret),
            ("📋 Copy to clipboard".to_string(), CredDialogOption::CopySecret),
            (policy_label, CredDialogOption::ChangePolicy),
            (toggle_label.to_string(), CredDialogOption::ToggleDisable),
            ("  Delete credential".to_string(), CredDialogOption::Delete),
            (totp_label.to_string(), CredDialogOption::SetupTotp),
            ("  Cancel".to_string(), CredDialogOption::Cancel),
        ];

        let items: Vec<ListItem> = menu_items
            .iter()
            .map(|(label, opt)| {
                let is_selected = *opt == dlg.selected;
                let (marker, style) = if is_selected {
                    (
                        "❯ ",
                        Style::default()
                            .fg(tp::ACCENT_BRIGHT)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ("  ", Style::default().fg(tp::TEXT))
                };

                // Colour the delete option red when highlighted
                let final_style = if *opt == CredDialogOption::Delete && is_selected {
                    style.fg(tp::ERROR)
                } else {
                    style
                };

                ListItem::new(Line::from(vec![
                    Span::styled(marker, Style::default().fg(tp::ACCENT)),
                    Span::styled(label.as_str(), final_style),
                ]))
            })
            .collect();

        let list = List::new(items).style(Style::default().fg(tp::TEXT));
        frame.render_widget(list, inner);
    }

    // ── Policy picker dialog ───────────────────────────────────

    /// Handle key events when the policy-picker dialog is open.
    fn handle_policy_picker_key(
        &mut self,
        code: crossterm::event::KeyCode,
    ) -> Action {
        use crossterm::event::KeyCode;

        let Some(mut picker) = self.policy_picker.take() else {
            return Action::Noop;
        };

        match picker.phase {
            PolicyPickerPhase::Selecting => {
                let options = [
                    PolicyPickerOption::Open,
                    PolicyPickerOption::Ask,
                    PolicyPickerOption::Auth,
                    PolicyPickerOption::Skill,
                ];

                match code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        // Cancel — go back to credential dialog
                        return Action::Noop;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let cur = options.iter().position(|o| *o == picker.selected).unwrap_or(0);
                        let next = if cur == 0 { options.len() - 1 } else { cur - 1 };
                        picker.selected = options[next];
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let cur = options.iter().position(|o| *o == picker.selected).unwrap_or(0);
                        let next = (cur + 1) % options.len();
                        picker.selected = options[next];
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                    KeyCode::Enter => {
                        match picker.selected {
                            PolicyPickerOption::Open => {
                                match self.state.secrets_manager.set_credential_policy(
                                    &picker.cred_name,
                                    crate::secrets::AccessPolicy::Always,
                                ) {
                                    Ok(()) => {
                                        self.state.messages.push(format!(
                                            "Policy for '{}' set to OPEN.", picker.cred_name,
                                        ));
                                    }
                                    Err(e) => {
                                        self.state.messages.push(format!(
                                            "Failed to set policy: {}", e,
                                        ));
                                    }
                                }
                                return Action::Update;
                            }
                            PolicyPickerOption::Ask => {
                                match self.state.secrets_manager.set_credential_policy(
                                    &picker.cred_name,
                                    crate::secrets::AccessPolicy::WithApproval,
                                ) {
                                    Ok(()) => {
                                        self.state.messages.push(format!(
                                            "Policy for '{}' set to ASK.", picker.cred_name,
                                        ));
                                    }
                                    Err(e) => {
                                        self.state.messages.push(format!(
                                            "Failed to set policy: {}", e,
                                        ));
                                    }
                                }
                                return Action::Update;
                            }
                            PolicyPickerOption::Auth => {
                                match self.state.secrets_manager.set_credential_policy(
                                    &picker.cred_name,
                                    crate::secrets::AccessPolicy::WithAuth,
                                ) {
                                    Ok(()) => {
                                        self.state.messages.push(format!(
                                            "Policy for '{}' set to AUTH.", picker.cred_name,
                                        ));
                                    }
                                    Err(e) => {
                                        self.state.messages.push(format!(
                                            "Failed to set policy: {}", e,
                                        ));
                                    }
                                }
                                return Action::Update;
                            }
                            PolicyPickerOption::Skill => {
                                // Transition to skill name input phase
                                picker.phase = PolicyPickerPhase::EditingSkills {
                                    input: String::new(),
                                };
                                self.policy_picker = Some(picker);
                                return Action::Noop;
                            }
                        }
                    }
                    _ => {
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                }
            }
            PolicyPickerPhase::EditingSkills { ref mut input } => {
                match code {
                    KeyCode::Esc => {
                        // Go back to the selection phase
                        picker.phase = PolicyPickerPhase::Selecting;
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                    KeyCode::Enter => {
                        let skills: Vec<String> = input
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();

                        match self.state.secrets_manager.set_credential_policy(
                            &picker.cred_name,
                            crate::secrets::AccessPolicy::SkillOnly(skills.clone()),
                        ) {
                            Ok(()) => {
                                if skills.is_empty() {
                                    self.state.messages.push(format!(
                                        "Policy for '{}' set to SKILL (locked — no skills).",
                                        picker.cred_name,
                                    ));
                                } else {
                                    self.state.messages.push(format!(
                                        "Policy for '{}' set to SKILL ({}).",
                                        picker.cred_name,
                                        skills.join(", "),
                                    ));
                                }
                            }
                            Err(e) => {
                                self.state.messages.push(format!(
                                    "Failed to set policy: {}", e,
                                ));
                            }
                        }
                        return Action::Update;
                    }
                    _ => {
                        self.policy_picker = Some(picker);
                        return Action::Noop;
                    }
                }
            }
        }
    }

    /// Draw a centered policy-picker dialog overlay.
    fn draw_policy_picker(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        picker: &PolicyPickerState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{
            Block, Borders, Clear, List, ListItem, Paragraph,
        };

        let dialog_w = 52.min(area.width.saturating_sub(4));
        let dialog_h = match picker.phase {
            PolicyPickerPhase::Selecting => 12u16,
            PolicyPickerPhase::EditingSkills { .. } => 8u16,
        }
        .min(area.height.saturating_sub(4))
        .max(8);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        match picker.phase {
            PolicyPickerPhase::Selecting => {
                let title = format!(" {} — Access Policy ", picker.cred_name);
                let hint = " ↑↓ navigate · Enter select · Esc cancel ";

                let block = Block::default()
                    .title(Span::styled(&title, tp::title_focused()))
                    .title_bottom(
                        Line::from(Span::styled(hint, Style::default().fg(tp::MUTED)))
                            .right_aligned(),
                    )
                    .borders(Borders::ALL)
                    .border_style(tp::focused_border())
                    .border_type(ratatui::widgets::BorderType::Rounded);

                let inner = block.inner(dialog_area);
                frame.render_widget(block, dialog_area);

                let options: Vec<(PolicyPickerOption, &str, &str, Color)> = vec![
                    (PolicyPickerOption::Open, "OPEN", "  Agent can read anytime", tp::SUCCESS),
                    (PolicyPickerOption::Ask, "ASK", "   Agent asks per use", tp::WARN),
                    (PolicyPickerOption::Auth, "AUTH", "  Re-authenticate each time", tp::ERROR),
                    (PolicyPickerOption::Skill, "SKILL", " Only named skills may read", tp::INFO),
                ];

                let items: Vec<ListItem> = options
                    .iter()
                    .map(|(opt, badge_text, desc, badge_color)| {
                        let is_selected = *opt == picker.selected;
                        let marker = if is_selected { "❯ " } else { "  " };

                        let badge = Span::styled(
                            format!(" {} ", badge_text),
                            Style::default()
                                .fg(Color::Rgb(0x1E, 0x1C, 0x1A))
                                .bg(*badge_color),
                        );

                        let desc_style = if is_selected {
                            Style::default()
                                .fg(tp::ACCENT_BRIGHT)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(tp::TEXT_DIM)
                        };

                        ListItem::new(Line::from(vec![
                            Span::styled(marker, Style::default().fg(tp::ACCENT)),
                            badge,
                            Span::styled(*desc, desc_style),
                        ]))
                    })
                    .collect();

                // Render with a blank row at top for padding
                let mut all_items = vec![ListItem::new("")];
                all_items.extend(items);

                let list = List::new(all_items);
                frame.render_widget(list, inner);
            }
            PolicyPickerPhase::EditingSkills { ref input } => {
                let title = format!(" {} — SKILL Policy ", picker.cred_name);
                let hint = " Enter confirm · Esc back ";

                let block = Block::default()
                    .title(Span::styled(&title, tp::title_focused()))
                    .title_bottom(
                        Line::from(Span::styled(hint, Style::default().fg(tp::MUTED)))
                            .right_aligned(),
                    )
                    .borders(Borders::ALL)
                    .border_style(tp::focused_border())
                    .border_type(ratatui::widgets::BorderType::Rounded);

                let inner = block.inner(dialog_area);
                frame.render_widget(block, dialog_area);

                let prompt_text = vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        " Enter skill names (comma-separated):",
                        Style::default().fg(tp::TEXT_DIM),
                    )),
                    Line::from(Span::styled(
                        " Leave empty to lock the credential.",
                        Style::default().fg(tp::MUTED),
                    )),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(" > ", Style::default().fg(tp::ACCENT)),
                        Span::styled(
                            format!("{}_", input),
                            Style::default().fg(tp::TEXT).add_modifier(Modifier::BOLD),
                        ),
                    ]),
                ];

                let paragraph = Paragraph::new(prompt_text);
                frame.render_widget(paragraph, inner);
            }
        }
    }

    // ── TOTP setup dialog ─────────────────────────────────────

    /// Handle key events when the TOTP dialog is open.
    fn handle_totp_dialog_key(
        &mut self,
        code: crossterm::event::KeyCode,
    ) -> Action {
        use crossterm::event::KeyCode;

        let Some(mut dlg) = self.totp_dialog.take() else {
            return Action::Noop;
        };

        match dlg.phase {
            TotpDialogPhase::ShowUri { ref mut uri, ref mut input } |
            TotpDialogPhase::Failed { ref mut uri, ref mut input } => {
                match code {
                    KeyCode::Esc => {
                        // Cancel — remove the TOTP secret we just set up
                        let _ = self.state.secrets_manager.remove_totp();
                        return Action::Noop;
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() && input.len() < 6 => {
                        input.push(c);
                        self.totp_dialog = Some(dlg);
                        return Action::Noop;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.totp_dialog = Some(dlg);
                        return Action::Noop;
                    }
                    KeyCode::Enter => {
                        if input.len() == 6 {
                            match self.state.secrets_manager.verify_totp(input) {
                                Ok(true) => {
                                    self.state.config.totp_enabled = true;
                                    let _ = self.state.config.save(None);
                                    self.state.messages.push(
                                        "✓ 2FA configured successfully.".to_string(),
                                    );
                                    self.totp_dialog = Some(TotpDialogState {
                                        phase: TotpDialogPhase::Verified,
                                    });
                                    return Action::Noop;
                                }
                                Ok(false) => {
                                    let saved_uri = uri.clone();
                                    self.totp_dialog = Some(TotpDialogState {
                                        phase: TotpDialogPhase::Failed {
                                            uri: saved_uri,
                                            input: String::new(),
                                        },
                                    });
                                    return Action::Noop;
                                }
                                Err(e) => {
                                    self.state.messages.push(format!(
                                        "TOTP verification error: {}", e,
                                    ));
                                    let _ = self.state.secrets_manager.remove_totp();
                                    return Action::Noop;
                                }
                            }
                        }
                        self.totp_dialog = Some(dlg);
                        return Action::Noop;
                    }
                    _ => {
                        self.totp_dialog = Some(dlg);
                        return Action::Noop;
                    }
                }
            }
            TotpDialogPhase::AlreadyConfigured => {
                match code {
                    KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                        // Keep 2FA
                        return Action::Noop;
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        // Remove 2FA
                        match self.state.secrets_manager.remove_totp() {
                            Ok(()) => {
                                self.state.config.totp_enabled = false;
                                let _ = self.state.config.save(None);
                                self.state.messages.push(
                                    "2FA has been removed.".to_string(),
                                );
                            }
                            Err(e) => {
                                self.state.messages.push(format!(
                                    "Failed to remove 2FA: {}", e,
                                ));
                            }
                        }
                        return Action::Noop;
                    }
                    _ => {
                        self.totp_dialog = Some(dlg);
                        return Action::Noop;
                    }
                }
            }
            TotpDialogPhase::Verified => {
                // Any key closes
                return Action::Noop;
            }
        }
    }

    /// Draw a centered TOTP setup dialog overlay.
    fn draw_totp_dialog(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        dlg: &TotpDialogState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{
            Block, Borders, Clear, Paragraph, Wrap,
        };

        let dialog_w = 56.min(area.width.saturating_sub(4));
        let dialog_h = 12u16.min(area.height.saturating_sub(4)).max(8);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let (title, lines, hint): (&str, Vec<Line>, &str) = match &dlg.phase {
            TotpDialogPhase::ShowUri { uri, input } => {
                let masked: String = "*".repeat(input.len())
                    + &"_".repeat(6 - input.len());
                (
                    " Set up 2FA ",
                    vec![
                        Line::from(Span::styled(
                            "Add this URI to your authenticator app:",
                            Style::default().fg(tp::TEXT),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            uri.as_str(),
                            Style::default().fg(tp::ACCENT_BRIGHT),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Enter the 6-digit code to verify:",
                            Style::default().fg(tp::TEXT),
                        )),
                        Line::from(Span::styled(
                            format!("  Code: [{}]", masked),
                            Style::default().fg(tp::WARN).add_modifier(Modifier::BOLD),
                        )),
                    ],
                    " Enter code · Esc cancel ",
                )
            }
            TotpDialogPhase::Failed { input, .. } => {
                let masked: String = "*".repeat(input.len())
                    + &"_".repeat(6 - input.len());
                (
                    " 2FA Verification ",
                    vec![
                        Line::from(Span::styled(
                            "✗ Code invalid — please try again.",
                            Style::default().fg(tp::ERROR).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            format!("  Code: [{}]", masked),
                            Style::default().fg(tp::WARN).add_modifier(Modifier::BOLD),
                        )),
                    ],
                    " Enter code · Esc cancel ",
                )
            }
            TotpDialogPhase::AlreadyConfigured => {
                (
                    " 2FA Active ",
                    vec![
                        Line::from(Span::styled(
                            "Two-factor authentication is already configured.",
                            Style::default().fg(tp::SUCCESS),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Remove 2FA? (y/n)",
                            Style::default().fg(tp::WARN),
                        )),
                    ],
                    " y remove · n/Esc keep ",
                )
            }
            TotpDialogPhase::Verified => {
                (
                    " 2FA Configured ",
                    vec![
                        Line::from(Span::styled(
                            "✓ Two-factor authentication is now active.",
                            Style::default().fg(tp::SUCCESS).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Credentials with AUTH policy will require TOTP.",
                            Style::default().fg(tp::TEXT_DIM),
                        )),
                    ],
                    " Press any key to close ",
                )
            }
        };

        let block = Block::default()
            .title(Span::styled(title, tp::title_focused()))
            .title_bottom(
                Line::from(Span::styled(
                    hint,
                    Style::default().fg(tp::MUTED),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(tp::focused_border())
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let text = ratatui::text::Text::from(lines);
        let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }

    // ── Clipboard helper ──────────────────────────────────────

    /// Copy text to the system clipboard using platform-native tools.
    fn copy_to_clipboard(text: &str) -> Result<()> {
        use anyhow::Context;
        use std::io::Write;
        use std::process::{Command, Stdio};

        #[cfg(target_os = "macos")]
        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to launch pbcopy")?;

        #[cfg(target_os = "linux")]
        let mut child = {
            // Try xclip first, fall back to xsel
            Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(Stdio::piped())
                .spawn()
                .or_else(|_| {
                    Command::new("xsel")
                        .arg("--clipboard")
                        .stdin(Stdio::piped())
                        .spawn()
                })
                .context("Failed to launch xclip or xsel")?
        };

        #[cfg(target_os = "windows")]
        let mut child = Command::new("clip")
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to launch clip.exe")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())
                .context("Failed to write to clipboard process")?;
        }
        child.wait().context("Clipboard process failed")?;
        Ok(())
    }

    // ── Secret viewer dialog ──────────────────────────────────

    /// Handle key events when the secret viewer dialog is open.
    fn handle_secret_viewer_key(
        &mut self,
        code: crossterm::event::KeyCode,
    ) -> Action {
        use crossterm::event::KeyCode;

        let Some(mut viewer) = self.secret_viewer.take() else {
            return Action::Noop;
        };

        // Clear transient status on any keypress
        viewer.status = None;

        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                // Close viewer
                return Action::Noop;
            }
            KeyCode::Char('r') => {
                // Toggle reveal/mask
                viewer.revealed = !viewer.revealed;
                self.secret_viewer = Some(viewer);
                return Action::Noop;
            }
            KeyCode::Char('c') => {
                // Copy the selected field value to clipboard
                if let Some((_label, value)) = viewer.fields.get(viewer.selected) {
                    match Self::copy_to_clipboard(value) {
                        Ok(()) => {
                            viewer.status = Some("Copied!".to_string());
                        }
                        Err(e) => {
                            viewer.status = Some(format!("Copy failed: {}", e));
                        }
                    }
                }
                self.secret_viewer = Some(viewer);
                return Action::Noop;
            }
            KeyCode::Char('a') => {
                // Copy all fields to clipboard
                let text = viewer.fields.iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect::<Vec<_>>()
                    .join("\n");
                match Self::copy_to_clipboard(&text) {
                    Ok(()) => {
                        viewer.status = Some("All fields copied!".to_string());
                    }
                    Err(e) => {
                        viewer.status = Some(format!("Copy failed: {}", e));
                    }
                }
                self.secret_viewer = Some(viewer);
                return Action::Noop;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if viewer.selected > 0 {
                    viewer.selected -= 1;
                }
                self.secret_viewer = Some(viewer);
                return Action::Noop;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if viewer.selected + 1 < viewer.fields.len() {
                    viewer.selected += 1;
                }
                self.secret_viewer = Some(viewer);
                return Action::Noop;
            }
            _ => {
                self.secret_viewer = Some(viewer);
                return Action::Noop;
            }
        }
    }

    /// Draw a centered secret-viewer dialog overlay.
    fn draw_secret_viewer(
        frame: &mut ratatui::Frame<'_>,
        area: Rect,
        viewer: &SecretViewerState,
    ) {
        use crate::theme::tui_palette as tp;
        use ratatui::widgets::{
            Block, Borders, Clear, List, ListItem,
        };

        // Size: width up to 70, height = fields + header/status/hint + borders
        let dialog_w = 70u16.min(area.width.saturating_sub(4));
        let content_lines = viewer.fields.len() as u16 + 3; // fields + blank + status + blank
        let dialog_h = (content_lines + 3)
            .min(area.height.saturating_sub(4))
            .max(8);
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        frame.render_widget(Clear, dialog_area);

        let title = format!(" {} ", viewer.name);
        let reveal_hint = if viewer.revealed { "r:hide" } else { "r:reveal" };
        let hint = format!(
            " ↑↓ select · c copy · a copy all · {} · Esc close ",
            reveal_hint,
        );

        let block = Block::default()
            .title(Span::styled(&title, tp::title_focused()))
            .title_bottom(
                Line::from(Span::styled(
                    &hint,
                    Style::default().fg(tp::MUTED),
                ))
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_style(tp::focused_border())
            .border_type(ratatui::widgets::BorderType::Rounded);

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let max_label_w = viewer.fields.iter()
            .map(|(k, _)| k.len())
            .max()
            .unwrap_or(0);

        let mut items: Vec<ListItem> = Vec::new();

        for (i, (label, value)) in viewer.fields.iter().enumerate() {
            let is_selected = i == viewer.selected;
            let display_value = if viewer.revealed {
                value.clone()
            } else {
                "•".repeat(value.len().min(32))
            };

            // Truncate to fit dialog width
            let avail = (dialog_w as usize).saturating_sub(max_label_w + 8);
            let truncated = if display_value.len() > avail {
                format!("{}…", &display_value[..avail.saturating_sub(1)])
            } else {
                display_value
            };

            let (marker, label_style, val_style) = if is_selected {
                (
                    "❯ ",
                    Style::default()
                        .fg(tp::ACCENT_BRIGHT)
                        .add_modifier(Modifier::BOLD),
                    Style::default()
                        .fg(tp::TEXT)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    "  ",
                    Style::default().fg(tp::TEXT_DIM),
                    Style::default().fg(tp::TEXT),
                )
            };

            items.push(ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(tp::ACCENT)),
                Span::styled(
                    format!("{:>width$}: ", label, width = max_label_w),
                    label_style,
                ),
                Span::styled(truncated, val_style),
            ])));
        }

        // Status line
        if let Some(ref status) = viewer.status {
            items.push(ListItem::new(""));
            items.push(ListItem::new(Line::from(Span::styled(
                format!("  {}", status),
                Style::default().fg(tp::SUCCESS).add_modifier(Modifier::BOLD),
            ))));
        }

        let list = List::new(items).style(Style::default().fg(tp::TEXT));
        frame.render_widget(list, inner);
    }
}
