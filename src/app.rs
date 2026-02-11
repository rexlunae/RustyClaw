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
use crate::pages::home::Home;
use crate::pages::Page;
use crate::panes::footer::FooterPane;
use crate::panes::header::HeaderPane;
use crate::panes::{GatewayStatus, InputMode, Pane, PaneState};
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;
use crate::soul::SoulManager;
use crate::tui::{Event, EventResponse, Tui};

/// Type alias for the client-side WebSocket write half.
type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

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
        }

        let skills_dir = config.skills_dir();
        let mut skill_manager = SkillManager::new(skills_dir);
        let _ = skill_manager.load_skills();

        let soul_path = config.soul_path();
        let mut soul_manager = SoulManager::new(soul_path);
        let _ = soul_manager.load();

        // Build pages
        let mut home = Home::new()?;
        home.register_action_handler(action_tx.clone())?;
        let pages: Vec<Box<dyn Page>> = vec![Box::new(home)];

        let gateway_status = if config.gateway_url.is_some() {
            GatewayStatus::Disconnected
        } else {
            GatewayStatus::Unconfigured
        };

        let state = SharedState {
            config,
            messages: vec!["Welcome to RustyClaw! Type /help for commands.".to_string()],
            input_mode: InputMode::Normal,
            secrets_manager,
            skill_manager,
            soul_manager,
            gateway_status,
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
        }
        self.pages[self.active_page].focus()?;

        // Auto-start gateway if configured
        if self.state.config.gateway_url.is_some() {
            self.start_gateway().await;
        }

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
            Action::InputSubmit(ref text) => {
                return self.handle_input_submit(text.clone());
            }
            Action::ReconnectGateway => {
                self.start_gateway().await;
                return Ok(None);
            }
            Action::DisconnectGateway => {
                self.stop_gateway().await;
                return Ok(None);
            }
            Action::RestartGateway => {
                self.restart_gateway().await;
                return Ok(None);
            }
            Action::SendToGateway(ref text) => {
                self.send_to_gateway(text.clone()).await;
                return Ok(None);
            }
            Action::GatewayMessage(ref text) => {
                self.state.messages.push(format!("◀ {}", text));
                // Auto-scroll
                return Ok(Some(Action::Update));
            }
            Action::GatewayDisconnected(ref reason) => {
                self.state.gateway_status = GatewayStatus::Disconnected;
                self.state.messages.push(format!("Gateway disconnected: {}", reason));
                self.ws_sink = None;
                self.reader_task = None;
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
        let url = match &self.state.config.gateway_url {
            Some(u) => u.clone(),
            None => {
                self.state.gateway_status = GatewayStatus::Unconfigured;
                self.state
                    .messages
                    .push("No gateway URL configured. Use --gateway ws://... or set gateway_url in config.toml".to_string());
                return;
            }
        };

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

        // Spawn the gateway server as a background task.
        let cancel = CancellationToken::new();
        let cancel_child = cancel.clone();
        let config_clone = self.state.config.clone();
        let listen_url = url.clone();
        let handle = tokio::spawn(async move {
            let opts = GatewayOptions {
                listen: listen_url,
            };
            if let Err(err) = run_gateway(config_clone, opts, cancel_child).await {
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
                    let _ = tx.send(Action::GatewayMessage(text));
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
            match sink.send(Message::Text(text)).await {
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

    /// Restart: stop then start.
    async fn restart_gateway(&mut self) {
        self.stop_gateway().await;
        self.start_gateway().await;
    }

    fn draw(&mut self, tui: &mut Tui) -> Result<()> {
        tui.draw(|frame| {
            let area = frame.size();

            // Layout: header (1 row), body (fill), footer (2 rows: status + input)
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .split(area);

            let ps = PaneState {
                config: &self.state.config,
                secrets_manager: &mut self.state.secrets_manager,
                skill_manager: &mut self.state.skill_manager,
                soul_manager: &self.state.soul_manager,
                messages: &mut self.state.messages,
                input_mode: self.state.input_mode,
                gateway_status: self.state.gateway_status,
            };

            let _ = self.header.draw(frame, chunks[0], &ps);
            let _ = self.pages[self.active_page].draw(frame, chunks[1], &ps);
            let _ = self.footer.draw(frame, chunks[2], &ps);
        })?;
        Ok(())
    }
}
