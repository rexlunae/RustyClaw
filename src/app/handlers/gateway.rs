use crate::action::Action;
use crate::app::App;
use crate::config::Config;
use crate::daemon;
use crate::dialogs::{FetchModelsLoading, SecretViewerState, SPINNER_FRAMES};
use crate::gateway::{
    ClientFrame, ClientFrameType, ClientPayload, ChatMessage, deserialize_frame, serialize_frame,
    ServerFrame,
};
use crate::pages::Page;
use crate::panes::DisplayMessage;
use crate::providers;
use anyhow::Result;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use futures_util::stream::SplitStream;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::debug;

pub type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

impl App {
    pub async fn start_gateway(&mut self) {
        let (port, bind) = Self::gateway_defaults(&self.state.config);
        let url = self
            .state
            .config
            .gateway_url
            .clone()
            .unwrap_or_else(|| format!("ws://127.0.0.1:{}", port));

        if self.ws_sink.is_some() {
            self.state
                .messages
                .push(DisplayMessage::info("Already connected to gateway."));
            return;
        }

        self.state.gateway_status = crate::panes::GatewayStatus::Connecting;

        match daemon::status(&self.state.config.settings_dir) {
            daemon::DaemonStatus::Running { pid } => {
                self.state.messages.push(DisplayMessage::info(format!(
                    "Gateway daemon already running (PID {}).",
                    pid,
                )));
            }
            _ => {
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
                    self.state.config.tls_cert.as_deref(),
                    self.state.config.tls_key.as_deref(),
                ) {
                    Ok(pid) => {
                        self.state.messages.push(DisplayMessage::success(format!(
                            "Gateway daemon started (PID {}).",
                            pid,
                        )));
                    }
                    Err(e) => {
                        self.state.gateway_status = crate::panes::GatewayStatus::Error;
                        self.state
                            .messages
                            .push(DisplayMessage::error(format!("Failed to start gateway: {}", e)));
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
        }

        self.connect_to_gateway(&url).await;
    }

    pub async fn connect_to_gateway(&mut self, url: &str) {
        self.state.gateway_status = crate::panes::GatewayStatus::Connecting;
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws_stream, _)) => {
                let (sink, stream) = ws_stream.split();
                self.ws_sink = Some(sink);

                self.state.gateway_status = crate::panes::GatewayStatus::Connected;
                self.state
                    .messages
                    .push(DisplayMessage::success(format!("Connected to gateway {}", url)));

                let tx = self.action_tx.clone();
                self.reader_task = Some(tokio::spawn(async move {
                    Self::gateway_reader_loop(stream, tx).await;
                }));

                self.request_secrets_list().await;
            }
            Err(err) => {
                self.state.gateway_status = crate::panes::GatewayStatus::Error;
                self.state.messages.push(DisplayMessage::error(format!(
                    "Gateway connection failed: {}",
                    err
                )));
            }
        }
    }

    pub async fn gateway_reader_loop(
        mut stream: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
        tx: mpsc::UnboundedSender<Action>,
    ) {
        while let Some(result) = stream.next().await {
            match result {
                Ok(Message::Binary(data)) => {
                    // Deserialize binary ServerFrame and dispatch directly
                    match deserialize_frame::<ServerFrame>(&data) {
                        Ok(frame) => {
                            let frame_action = crate::gateway::server_frame_to_action(&frame);
                            if let Some(action) = frame_action.action {
                                let _ = tx.send(action);
                            }
                        }
                        Err(_) => {
                            // Silently ignore malformed binary frames
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    let _ = tx.send(Action::GatewayDisconnected(
                        "server sent close frame".to_string(),
                    ));
                    break;
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(_) => {}
                Err(err) => {
                    let _ = tx.send(Action::GatewayDisconnected(format!("{}", err)));
                    break;
                }
            }
        }
    }

    pub async fn send_to_gateway(&mut self, text: String) {
        // Always send as bincode-serialized binary frame
        use crate::gateway::protocol::types::ChatMessage;
        let frame = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
            let msg_type = val.get("type").and_then(|t| t.as_str());
            match msg_type {
                Some("unlock_vault") => {
                    let password = val.get("password").and_then(|p| p.as_str()).unwrap_or("");
                    ClientFrame {
                        frame_type: ClientFrameType::UnlockVault,
                        payload: ClientPayload::UnlockVault { password: password.into() },
                    }
                }
                Some("reload") => {
                    ClientFrame {
                        frame_type: ClientFrameType::Reload,
                        payload: ClientPayload::Reload,
                    }
                }
                Some("secrets_list") => {
                    ClientFrame {
                        frame_type: ClientFrameType::SecretsList,
                        payload: ClientPayload::SecretsList,
                    }
                }
                Some("secrets_store") => {
                    let key = val.get("key").and_then(|k| k.as_str()).unwrap_or("");
                    let value = val.get("value").and_then(|v| v.as_str()).unwrap_or("");
                    ClientFrame {
                        frame_type: ClientFrameType::SecretsStore,
                        payload: ClientPayload::SecretsStore { key: key.into(), value: value.into() },
                    }
                }
                Some("secrets_get") => {
                    let key = val.get("key").and_then(|k| k.as_str()).unwrap_or("");
                    ClientFrame {
                        frame_type: ClientFrameType::SecretsGet,
                        payload: ClientPayload::SecretsGet { key: key.into() },
                    }
                }
                Some("secrets_delete") => {
                    let key = val.get("key").and_then(|k| k.as_str()).unwrap_or("");
                    ClientFrame {
                        frame_type: ClientFrameType::SecretsDelete,
                        payload: ClientPayload::SecretsDelete { key: key.into() },
                    }
                }
                Some("secrets_setup_totp") => {
                    ClientFrame {
                        frame_type: ClientFrameType::SecretsSetupTotp,
                        payload: ClientPayload::SecretsSetupTotp,
                    }
                }
                Some("cancel") => {
                    ClientFrame {
                        frame_type: ClientFrameType::Cancel,
                        payload: ClientPayload::Empty,
                    }
                }
                // Fallback: treat as chat if unknown type
                _ => {
                    ClientFrame {
                        frame_type: ClientFrameType::Chat,
                        payload: ClientPayload::Chat { messages: vec![ChatMessage::text("user", &text)] },
                    }
                }
            }
        } else {
            // Not JSON: treat as chat
            ClientFrame {
                frame_type: ClientFrameType::Chat,
                payload: ClientPayload::Chat { messages: vec![ChatMessage::text("user", &text)] },
            }
        };
        self.send_frame(frame).await;
    }

    /// Send a typed frame to the gateway using bincode serialization.
    pub async fn send_frame(&mut self, frame: ClientFrame) {
        if let Some(ref mut sink) = self.ws_sink {
            let bytes = match serialize_frame(&frame) {
                Ok(b) => {
                    b
                }
                Err(err) => {
                    self.state.messages.push(DisplayMessage::error(format!(
                        "Failed to serialize frame: {}", err
                    )));
                    return;
                }
            };
            match sink.send(Message::Binary(bytes.into())).await {
                Ok(()) => {}
                Err(err) => {
                    self.chat_loading_tick = None;
                    self.state.loading_line = None;
                    self.streaming_response = None;
                    self.state.streaming_started = None;
                    self.state
                        .messages
                        .push(DisplayMessage::error(format!("Send failed: {}", err)));
                    self.state.gateway_status = crate::panes::GatewayStatus::Error;
                    self.ws_sink = None;
                }
            }
        } else {
            self.state
                .messages
                .push(DisplayMessage::warning("Cannot send: gateway not connected."));
        }
    }

    pub async fn stop_gateway(&mut self) {
        let had_connection = self.ws_sink.is_some();

        if let Some(handle) = self.reader_task.take() {
            handle.abort();
        }

        if let Some(mut sink) = self.ws_sink.take() {
            let _ = sink.send(Message::Close(None)).await;
            let _ = sink.close().await;
        }

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
            self.state.gateway_status = crate::panes::GatewayStatus::Disconnected;
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

    pub async fn restart_gateway(&mut self) {
        self.stop_gateway().await;

        let tx = self.action_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = tx.send(Action::ReconnectGateway);
        });
    }

    pub fn extract_model_api_key(&mut self) -> Option<String> {
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

    pub fn extract_vault_password(&self) -> Option<String> {
        if !self.state.config.secrets_password_protected {
            return None;
        }
        self.state
            .secrets_manager
            .password()
            .map(|s| s.to_string())
    }

    pub fn gateway_defaults(config: &Config) -> (u16, &'static str) {
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

    pub fn spawn_device_flow(&mut self, provider: String) {
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

            let _ = tx.send(Action::DeviceFlowCodeReady {
                url: auth.verification_uri.clone(),
                code: auth.user_code.clone(),
            });

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
                    Ok(None) => {}
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

    pub async fn request_secrets_list(&mut self) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::SecretsList,
            payload: ClientPayload::SecretsList,
        };
        self.send_frame(frame).await;
    }

    pub async fn send_chat(&mut self, messages: Vec<ChatMessage>) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat { messages },
        };
        self.send_frame(frame).await;
    }

    pub async fn send_unlock_vault(&mut self, password: String) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::UnlockVault,
            payload: ClientPayload::UnlockVault { password },
        };
        self.send_frame(frame).await;
    }

    pub async fn send_reload(&mut self) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::Reload,
            payload: ClientPayload::Reload,
        };
        self.send_frame(frame).await;
    }

    pub async fn send_secrets_get(&mut self, key: String) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::SecretsGet,
            payload: ClientPayload::SecretsGet { key },
        };
        self.send_frame(frame).await;
    }

    pub async fn send_secrets_store(&mut self, key: String, value: String) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::SecretsStore,
            payload: ClientPayload::SecretsStore { key, value },
        };
        self.send_frame(frame).await;
    }

    pub async fn send_secrets_delete(&mut self, key: String) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::SecretsDelete,
            payload: ClientPayload::SecretsDelete { key },
        };
        self.send_frame(frame).await;
    }

    pub async fn send_secrets_setup_totp(&mut self) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::SecretsSetupTotp,
            payload: ClientPayload::SecretsSetupTotp,
        };
        self.send_frame(frame).await;
    }

    pub async fn send_cancel(&mut self) {
        let frame = ClientFrame {
            frame_type: ClientFrameType::Cancel,
            payload: ClientPayload::Empty,
        };
        self.send_frame(frame).await;
    }

    pub fn handle_set_provider(&mut self, provider: String) -> Result<Option<Action>> {
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
        let def = providers::provider_by_id(&provider);
        let auth_method = def
            .map(|d| d.auth_method)
            .unwrap_or(providers::AuthMethod::ApiKey);

        match auth_method {
            providers::AuthMethod::DeviceFlow => {
                if let Some(secret_key) = providers::secret_key_for_provider(&provider) {
                    let has_key = self.cached_secrets.iter().any(|e| {
                        e.get("name").and_then(|n| n.as_str()) == Some(secret_key)
                    });
                    if has_key {
                        self.state.messages.push(DisplayMessage::success(format!(
                            "Access token for {} is already stored.",
                            providers::display_name_for_provider(&provider),
                        )));
                        return Ok(Some(Action::FetchModels(provider)));
                    } else {
                        return Ok(Some(Action::StartDeviceFlow(provider)));
                    }
                }
            }
            providers::AuthMethod::ApiKey => {
                if let Some(secret_key) = providers::secret_key_for_provider(&provider) {
                    let has_key = self.cached_secrets.iter().any(|e| {
                        e.get("name").and_then(|n| n.as_str()) == Some(secret_key)
                    });
                    if has_key {
                        self.state.messages.push(DisplayMessage::success(format!(
                            "API key for {} is already stored.",
                            providers::display_name_for_provider(&provider),
                        )));
                        return Ok(Some(Action::FetchModels(provider)));
                    } else {
                        return Ok(Some(Action::PromptApiKey(provider)));
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

}

/// Collapse a potentially long block of model "thinking" text into a
/// compact one-line summary for display in the message pane.
///
/// Extracts the first meaningful sentence (up to ~120 chars) and formats
/// it with an italic style hint so the TUI renders it as a muted summary.
fn collapse_thinking_text(text: &str) -> String {
    // Skip common preamble patterns
    let text = text.trim();
    if text.is_empty() {
        return "Reasoning…".to_string();
    }

    // Try to find the first sentence
    let first_line = text.lines().next().unwrap_or(text);
    let first_line = first_line.trim();

    // Find a sentence boundary within a reasonable length
    let max_len = 120;
    let preview = if first_line.len() <= max_len {
        first_line.to_string()
    } else {
        // Look for a natural break point (sentence end, comma, etc.)
        let chunk = &first_line[..max_len];
        if let Some(pos) = chunk.rfind(". ") {
            format!("{}.", &chunk[..pos])
        } else if let Some(pos) = chunk.rfind(", ") {
            format!("{}…", &chunk[..pos])
        } else if let Some(pos) = chunk.rfind(' ') {
            format!("{}…", &chunk[..pos])
        } else {
            format!("{}…", chunk)
        }
    };

    // Include a character count so the user knows how much was collapsed
    let char_count = text.len();
    if char_count > 200 {
        format!("{preview}  ({char_count} chars)")
    } else {
        preview
    }
}


impl App {

    pub async fn handle_action(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::Info(msg) => {
                self.state.messages.push(DisplayMessage::info(&msg));
            }
            Action::Success(msg) => {
                self.state.messages.push(DisplayMessage::success(&msg));
            }
            Action::Warning(msg) => {
                self.state.messages.push(DisplayMessage::warning(&msg));
            }
            Action::Error(msg) => {
                self.state.messages.push(DisplayMessage::error(&msg));
            }
            Action::GatewayStreamStart => {
                self.state.loading_line = Some("⏳ Waiting for response...".to_string());
                self.state.streaming_started = Some(std::time::Instant::now());
            }
            Action::GatewayThinkingStart => {
                self.state.loading_line = Some("🤔 Thinking...".to_string());
                self.state.streaming_started = Some(std::time::Instant::now());
            }
            Action::GatewayThinkingDelta => {
                if let Some(started) = self.state.streaming_started {
                    let elapsed = started.elapsed().as_secs();
                    self.state.loading_line = Some(format!("🤔 Thinking... ({}s)", elapsed));
                }
            }
            Action::GatewayThinkingEnd => {
                self.state.loading_line = None;
            }
            Action::GatewayChunk(delta) => {
                debug!(delta_len = delta.len(), delta_preview = &delta[..delta.len().min(50)], "Received chunk");

                if self.streaming_response.is_none() {
                    debug!("First chunk - initializing streaming response");
                    self.state.loading_line = None;
                    self.streaming_response = Some(String::new());
                    self.state.streaming_started = Some(std::time::Instant::now());
                    if !self.showing_hatching {
                        self.state.messages.push(DisplayMessage::assistant(""));
                    }
                }

                if let Some(ref mut buf) = self.streaming_response {
                    buf.push_str(&delta);
                    debug!(buf_len = buf.len(), "Buffer update");

                    if !self.showing_hatching {
                        if let Some(last) = self.state.messages.last_mut() {
                            last.update_content(buf.clone());
                            debug!(content_len = last.content.len(), "Updated last message");
                        } else {
                            tracing::warn!("No last message to update!");
                        }
                    }
                }
            }
            Action::GatewayResponseDone => {
                self.chat_loading_tick = None;
                self.state.loading_line = None;
                self.state.streaming_started = None;

                if let Some(buf) = self.streaming_response.take() {
                    debug!(buf_len = buf.len(), showing_hatching = self.showing_hatching, "Response done");

                    if self.showing_hatching {
                        if !buf.is_empty() {
                            if let Some(ref mut hatching) = self.hatching_page {
                                let mut ps = self.state.pane_state();
                                let _ = hatching.update(Action::HatchingResponse(buf), &mut ps);
                            }
                        }
                        return Ok(Some(Action::Update));
                    }

                    let trimmed = buf.trim_end().to_string();
                    debug!(trimmed_len = trimmed.len(), messages_count = self.state.messages.len(), "Response done");

                    if let Some(last) = self.state.messages.last_mut() {
                        debug!(role = ?last.role, "Response done - last message role");
                        if matches!(last.role, crate::panes::MessageRole::Assistant) {
                            last.content = trimmed.clone();
                            debug!(content_len = trimmed.len(), "Response done - set content");
                        }
                    }

                    if !trimmed.is_empty() {
                        self.state.conversation_history.push(ChatMessage::text("assistant", &trimmed));
                        self.save_history();
                    }
                } else {
                    // This is normal when the model's final round had no text output
                    // (e.g., the streaming buffer was collapsed into a thinking summary
                    // before the tool call, and the model finished without further text).
                    debug!("Response done: no streaming_response buffer (collapsed to thinking or empty)");
                }
            }
            Action::GatewayToolCall { name, arguments, .. } => {
                // If there's accumulated streaming text from the current round,
                // collapse it into a compact "thinking" summary so it doesn't
                // create an ever-growing wall of repeated text.
                if let Some(buf) = self.streaming_response.take() {
                    let trimmed = buf.trim();
                    if !trimmed.is_empty() {
                        if let Some(last) = self.state.messages.last_mut() {
                            if matches!(last.role, crate::panes::MessageRole::Assistant) {
                                // Build a short summary from the first meaningful sentence
                                let summary = collapse_thinking_text(trimmed);
                                last.role = crate::panes::MessageRole::Thinking;
                                last.update_content(summary);
                            }
                        }
                    }
                }
                // Pretty display for ask_user; compact JSON for everything else.
                if name == "ask_user" {
                    let display = format_ask_user_call(&arguments);
                    self.state.messages.push(DisplayMessage::tool_call(display));
                } else {
                    let args_str = arguments.to_string();
                    let display_args = if args_str.len() > 200 {
                        format!("{}…", &args_str[..200])
                    } else {
                        args_str
                    };
                    self.state.messages.push(DisplayMessage::tool_call(format!("{name}({display_args})")));
                }
            }
            Action::GatewayToolResult { name, result, is_error, .. } => {
                let prefix = if is_error { "⚠ " } else { "" };
                // Pretty display for ask_user results; compact for everything else.
                let display_result = if name == "ask_user" {
                    format_ask_user_result(&result, is_error)
                } else if result.len() > 500 {
                    let truncated = &result[..result.char_indices()
                        .take_while(|(i, _)| *i < 500)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(500)];
                    format!("{prefix}{truncated}… ({} bytes total)", result.len())
                } else {
                    format!("{prefix}{result}")
                };
                self.state.messages.push(DisplayMessage::tool_result(format!("{name}: {display_result}")));
            }
            Action::GatewayAuthenticated => {
                self.state.messages.push(DisplayMessage::success("Authenticated with gateway."));
                self.state.gateway_status = crate::panes::GatewayStatus::Connected;
                self.request_secrets_list().await;
            }
            Action::GatewayVaultUnlocked => {
                self.state.messages.push(DisplayMessage::success("Gateway vault unlocked."));
                self.state.gateway_status = crate::panes::GatewayStatus::Connected;
            }
            Action::GatewayAuthChallenge => {
                self.state.gateway_status = crate::panes::GatewayStatus::AuthRequired;
                self.state.messages.push(DisplayMessage::warning("Gateway requires 2FA authentication."));
                return Ok(Some(Action::GatewayAuthChallenge));
            }
            Action::GatewayVaultLocked => {
                if let Some(pw) = self.deferred_vault_password.take() {
                    return Ok(Some(Action::GatewayUnlockVault(pw)));
                }
                self.state.gateway_status = crate::panes::GatewayStatus::VaultLocked;
                self.state.messages.push(DisplayMessage::warning(
                    "Gateway vault is locked — enter password to unlock.",
                ));
                return Ok(Some(Action::GatewayVaultLocked));
            }
            Action::SecretsListResult { entries } => {
                self.cached_secrets = entries;
            }
            Action::SecretsGetResult { key, value } => {
                if self.pending_secret_key.as_deref() == Some(&key) {
                    self.pending_secret_key = None;
                    if value.is_some() {
                        return Ok(Some(Action::FetchModels(key)));
                    } else {
                        return Ok(Some(Action::PromptApiKey(key)));
                    }
                }
            }
            Action::SecretsStoreResult { ok, message } => {
                if ok {
                    self.state.messages.push(DisplayMessage::success(&message));
                } else {
                    self.state.messages.push(DisplayMessage::error(&message));
                }
                self.request_secrets_list().await;
            }
            Action::SecretsPeekResult { ok, fields, message, .. } => {
                if ok {
                    self.secret_viewer = Some(SecretViewerState {
                        name: String::new(),
                        fields,
                        revealed: false,
                        selected: 0,
                        scroll_offset: 0,
                        status: None,
                    });
                } else {
                    let msg = message.unwrap_or_else(|| "Failed to peek".into());
                    self.state.messages.push(DisplayMessage::error(&msg));
                }
            }
            Action::SecretsSetPolicyResult { ok, message } => {
                if ok {
                    self.state.messages.push(DisplayMessage::success("Policy updated."));
                } else {
                    let msg = message.unwrap_or_else(|| "Failed to set policy.".into());
                    self.state.messages.push(DisplayMessage::error(&msg));
                }
                self.request_secrets_list().await;
            }
            Action::SecretsSetDisabledResult { .. } => {
                self.state.messages.push(DisplayMessage::success("Credential updated."));
                self.request_secrets_list().await;
            }
            Action::SecretsDeleteCredentialResult { .. } => {
                self.state.messages.push(DisplayMessage::success("Credential deleted."));
                self.request_secrets_list().await;
            }
            Action::SecretsHasTotpResult { .. } => {}
            Action::SecretsSetupTotpResult { ok, uri, message } => {
                if ok {
                    self.totp_dialog = Some(crate::dialogs::TotpDialogState {
                        phase: crate::dialogs::TotpDialogPhase::ShowUri {
                            uri: uri.unwrap_or_default(),
                            input: String::new(),
                        },
                    });
                } else {
                    let msg = message.unwrap_or_else(|| "Failed to set up 2FA".into());
                    self.state.messages.push(DisplayMessage::error(&msg));
                }
            }
            Action::SecretsVerifyTotpResult { ok } => {
                if ok {
                    self.state.config.totp_enabled = true;
                    let _ = self.state.config.save(None);
                    self.state.messages.push(DisplayMessage::success("2FA configured successfully."));
                    self.totp_dialog = Some(crate::dialogs::TotpDialogState {
                        phase: crate::dialogs::TotpDialogPhase::Verified,
                    });
                } else {
                    if let Some(ref mut dlg) = self.totp_dialog {
                        if let crate::dialogs::TotpDialogPhase::ShowUri { ref uri, .. } = dlg.phase {
                            let saved_uri = uri.clone();
                            dlg.phase = crate::dialogs::TotpDialogPhase::Failed {
                                uri: saved_uri,
                                input: String::new(),
                            };
                        }
                    }
                }
            }
            Action::SecretsRemoveTotpResult { ok } => {
                if ok {
                    self.state.config.totp_enabled = false;
                    let _ = self.state.config.save(None);
                    self.state.messages.push(DisplayMessage::info("2FA has been removed."));
                } else {
                    self.state.messages.push(DisplayMessage::error("Failed to remove 2FA"));
                }
            }
            _ => {}
        }

        Ok(Some(Action::Update))
    }
}

/// Format an `ask_user` tool call for human-readable display in the messages pane.
fn format_ask_user_call(arguments: &serde_json::Value) -> String {
    let prompt_type = arguments
        .get("prompt_type")
        .and_then(|v| v.as_str())
        .unwrap_or("question");
    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(no title)");

    let mut out = format!("ask_user  💬 {}", title);

    if let Some(desc) = arguments.get("description").and_then(|v| v.as_str()) {
        out.push_str(&format!("\n  {}", desc));
    }

    match prompt_type {
        "select" | "multi_select" => {
            let kind = if prompt_type == "select" {
                "Pick one"
            } else {
                "Pick any"
            };
            out.push_str(&format!("\n  [{}]", kind));
            if let Some(opts) = arguments.get("options").and_then(|v| v.as_array()) {
                for (i, opt) in opts.iter().enumerate() {
                    let label = opt
                        .as_str()
                        .or_else(|| opt.get("label").and_then(|v| v.as_str()))
                        .unwrap_or("?");
                    let desc = opt
                        .get("description")
                        .and_then(|v| v.as_str());
                    if let Some(d) = desc {
                        out.push_str(&format!("\n    {}. {} — {}", i + 1, label, d));
                    } else {
                        out.push_str(&format!("\n    {}. {}", i + 1, label));
                    }
                }
            }
        }
        "confirm" => {
            out.push_str("\n  [Yes / No]");
        }
        "text" => {
            if let Some(ph) = arguments.get("placeholder").and_then(|v| v.as_str()) {
                out.push_str(&format!("\n  (placeholder: {})", ph));
            }
        }
        "form" => {
            out.push_str("\n  [Form]");
            if let Some(fields) = arguments.get("fields").and_then(|v| v.as_array()) {
                for f in fields {
                    let label = f
                        .get("label")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let req = f
                        .get("required")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if req {
                        out.push_str(&format!("\n    • {} *", label));
                    } else {
                        out.push_str(&format!("\n    • {}", label));
                    }
                }
            }
        }
        _ => {}
    }

    out
}

/// Format an `ask_user` tool result for human-readable display.
fn format_ask_user_result(result: &str, is_error: bool) -> String {
    if is_error {
        return format!("⚠ {}", result);
    }

    // The result is either a JSON value or a plain string like "User dismissed…"
    match serde_json::from_str::<serde_json::Value>(result) {
        Ok(serde_json::Value::String(s)) => format!("→ {}", s),
        Ok(serde_json::Value::Bool(b)) => {
            if b {
                "→ Yes".to_string()
            } else {
                "→ No".to_string()
            }
        }
        Ok(serde_json::Value::Array(arr)) => {
            let items: Vec<String> = arr
                .iter()
                .map(|v| v.as_str().unwrap_or("?").to_string())
                .collect();
            format!("→ {}", items.join(", "))
        }
        Ok(serde_json::Value::Object(map)) => {
            let fields: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let val = match v.as_str() {
                        Some(s) => s.to_string(),
                        None => v.to_string(),
                    };
                    format!("{}: {}", k, val)
                })
                .collect();
            format!("→ {}", fields.join(", "))
        }
        Ok(serde_json::Value::Null) => "→ (dismissed)".to_string(),
        Ok(other) => format!("→ {}", other),
        Err(_) => {
            // Plain text result (e.g. "User dismissed the prompt…")
            result.to_string()
        }
    }
}