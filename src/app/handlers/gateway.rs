use crate::action::Action;
use crate::app::App;
use crate::config::Config;
use crate::daemon;
use crate::dialogs::{FetchModelsLoading, SecretViewerState, SPINNER_FRAMES};
use crate::gateway::ChatMessage;
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
use tracing::{debug, warn};

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
                Ok(Message::Text(text)) => {
                    let _ = tx.send(Action::GatewayMessage(text.to_string()));
                }
                Ok(Message::Binary(data)) => {
                    let text = String::from_utf8_lossy(&data).to_string();
                    let _ = tx.send(Action::GatewayMessage(text));
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
        if let Some(ref mut sink) = self.ws_sink {
            let json_value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|_| {
                serde_json::json!({ "type": "chat", "messages": [], "content": text })
            });
            let bytes = match serde_json::to_vec(&json_value) {
                Ok(b) => b,
                Err(err) => {
                    self.chat_loading_tick = None;
                    self.state.loading_line = None;
                    self.state.messages.push(DisplayMessage::error(format!("Serialization failed: {}", err)));
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
        let frame = serde_json::json!({"type": "secrets_list"});
        self.send_to_gateway(frame.to_string()).await;
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

    pub async fn handle_gateway_message(&mut self, text: &str) -> Result<Option<Action>> {
        let parsed = serde_json::from_str::<serde_json::Value>(text).ok();
        let frame_type = parsed
            .as_ref()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()));

        debug!(frame_type = ?frame_type, len = text.len(), "Received frame");

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
                    self.state.gateway_status = crate::panes::GatewayStatus::ModelError;
                    self.state.messages.push(DisplayMessage::warning(detail));
                }
                "model_connecting" => {
                    self.state.messages.push(DisplayMessage::info(detail));
                }
                "model_ready" => {
                    self.state.gateway_status = crate::panes::GatewayStatus::ModelReady;
                    self.state.messages.push(DisplayMessage::success(detail));
                }
                "model_error" => {
                    self.state.gateway_status = crate::panes::GatewayStatus::ModelError;
                    self.state.messages.push(DisplayMessage::error(detail));
                }
                "no_model" => {
                    self.state.messages.push(DisplayMessage::warning(detail));
                }
                "vault_locked" => {
                    if let Some(pw) = self.deferred_vault_password.take() {
                        return Ok(Some(Action::GatewayUnlockVault(pw)));
                    }
                    self.state.gateway_status = crate::panes::GatewayStatus::VaultLocked;
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

        if frame_type == Some("auth_challenge") {
            self.state.gateway_status = crate::panes::GatewayStatus::AuthRequired;
            self.state
                .messages
                .push(DisplayMessage::warning("Gateway requires 2FA authentication."));
            return Ok(Some(Action::GatewayAuthChallenge));
        }

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
                self.state.gateway_status = crate::panes::GatewayStatus::Connected;
                self.request_secrets_list().await;
            } else if retry {
                let msg = parsed
                    .as_ref()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("Invalid code. Try again.");
                self.state.messages.push(DisplayMessage::warning(msg));
                return Ok(Some(Action::GatewayAuthChallenge));
            } else {
                let msg = parsed
                    .as_ref()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("Authentication failed.");
                self.state.messages.push(DisplayMessage::error(msg));
                self.state.gateway_status = crate::panes::GatewayStatus::Error;
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("auth_locked") {
            let msg = parsed
                .as_ref()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                .unwrap_or("Too many failed attempts.");
            self.state.messages.push(DisplayMessage::error(msg));
            self.state.gateway_status = crate::panes::GatewayStatus::Error;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("vault_unlocked") {
            let ok = parsed
                .as_ref()
                .and_then(|v| v.get("ok").and_then(|o| o.as_bool()))
                .unwrap_or(false);
            if ok {
                self.state
                    .messages
                    .push(DisplayMessage::success("Gateway vault unlocked."));
                self.state.gateway_status = crate::panes::GatewayStatus::Connected;
            } else {
                let msg = parsed
                    .as_ref()
                    .and_then(|v| v.get("message").and_then(|m| m.as_str()))
                    .unwrap_or("Failed to unlock vault.");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("reload_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok")).and_then(|o| o.as_bool()).unwrap_or(false);
            if ok {
                let provider = parsed.as_ref().and_then(|v| v.get("provider")).and_then(|p| p.as_str()).unwrap_or("?");
                let model = parsed.as_ref().and_then(|v| v.get("model")).and_then(|m| m.as_str()).unwrap_or("?");
                self.state.messages.push(DisplayMessage::success(format!(
                    "Gateway config reloaded: {} / {}", provider, model
                )));
            } else {
                let msg = parsed.as_ref().and_then(|v| v.get("message")).and_then(|m| m.as_str()).unwrap_or("Unknown error");
                self.state.messages.push(DisplayMessage::error(format!(
                    "Reload failed: {}", msg
                )));
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_list_result") {
            let entries = parsed
                .as_ref()
                .and_then(|v| v.get("entries"))
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            self.cached_secrets = entries;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_store_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str())).unwrap_or("");
            if ok {
                self.state.messages.push(DisplayMessage::success(msg));
            } else {
                self.state.messages.push(DisplayMessage::error(msg));
            }
            self.request_secrets_list().await;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_get_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            let key = parsed.as_ref().and_then(|v| v.get("key").and_then(|k| k.as_str())).unwrap_or("").to_string();
            let value = parsed.as_ref().and_then(|v| v.get("value").and_then(|val| val.as_str())).map(|s| s.to_string());
            if self.pending_secret_key.as_deref() == Some(&key) {
                self.pending_secret_key = None;
                if ok && value.is_some() {
                    return Ok(Some(Action::FetchModels(key)));
                } else {
                    return Ok(Some(Action::PromptApiKey(key)));
                }
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_peek_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            let name = parsed.as_ref().and_then(|v| v.get("name").and_then(|n| n.as_str())).unwrap_or("").to_string();
            if ok {
                let fields: Vec<(String, String)> = parsed.as_ref()
                    .and_then(|v| v.get("fields"))
                    .and_then(|f| f.as_array())
                    .map(|arr| arr.iter().filter_map(|item| {
                        let pair = item.as_array()?;
                        Some((pair.get(0)?.as_str()?.to_string(), pair.get(1)?.as_str()?.to_string()))
                    }).collect())
                    .unwrap_or_default();
                self.secret_viewer = Some(SecretViewerState {
                    name,
                    fields,
                    revealed: false,
                    selected: 0,
                    scroll_offset: 0,
                    status: None,
                });
            } else {
                let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str())).unwrap_or("Failed to peek");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_set_policy_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str()));
            if ok {
                self.state.messages.push(DisplayMessage::success("Policy updated."));
            } else {
                self.state.messages.push(DisplayMessage::error(msg.unwrap_or("Failed to set policy.")));
            }
            self.request_secrets_list().await;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_set_disabled_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            if ok {
                self.state.messages.push(DisplayMessage::success("Credential updated."));
            } else {
                let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str())).unwrap_or("Failed");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            self.request_secrets_list().await;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_delete_result") || frame_type == Some("secrets_delete_credential_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            if ok {
                self.state.messages.push(DisplayMessage::success("Credential deleted."));
            } else {
                let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str())).unwrap_or("Failed");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            self.request_secrets_list().await;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_has_totp_result") {
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_setup_totp_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            if ok {
                let uri = parsed.as_ref().and_then(|v| v.get("uri").and_then(|u| u.as_str())).unwrap_or("").to_string();
                self.totp_dialog = Some(crate::dialogs::TotpDialogState {
                    phase: crate::dialogs::TotpDialogPhase::ShowUri {
                        uri,
                        input: String::new(),
                    },
                });
            } else {
                let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str())).unwrap_or("Failed to set up 2FA");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_verify_totp_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
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
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("secrets_remove_totp_result") {
            let ok = parsed.as_ref().and_then(|v| v.get("ok").and_then(|o| o.as_bool())).unwrap_or(false);
            if ok {
                self.state.config.totp_enabled = false;
                let _ = self.state.config.save(None);
                self.state.messages.push(DisplayMessage::info("2FA has been removed."));
            } else {
                let msg = parsed.as_ref().and_then(|v| v.get("message").and_then(|m| m.as_str())).unwrap_or("Failed to remove 2FA");
                self.state.messages.push(DisplayMessage::error(msg));
            }
            return Ok(Some(Action::Update));
        }

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

        if frame_type == Some("debug") {
            return Ok(Some(Action::Update));
        }

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

        if frame_type == Some("stream_start") {
            self.state.loading_line = Some("⏳ Waiting for response...".to_string());
            self.state.streaming_started = Some(std::time::Instant::now());
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("thinking_start") {
            self.state.loading_line = Some("🤔 Thinking...".to_string());
            self.state.streaming_started = Some(std::time::Instant::now());
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("thinking_delta") {
            if let Some(started) = self.state.streaming_started {
                let elapsed = started.elapsed().as_secs();
                self.state.loading_line = Some(format!("🤔 Thinking... ({}s)", elapsed));
            }
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("thinking_end") {
            self.state.loading_line = None;
            return Ok(Some(Action::Update));
        }

        if frame_type == Some("chunk") {
            let delta = parsed
                .as_ref()
                .and_then(|v| v.get("delta").and_then(|d| d.as_str()))
                .unwrap_or("");

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
                buf.push_str(delta);
                debug!(buf_len = buf.len(), "Buffer update");

                if !self.showing_hatching {
                    if let Some(last) = self.state.messages.last_mut() {
                        last.update_content(buf.clone());
                        debug!(content_len = last.content.len(), "Updated last message");
                    } else {
                        warn!("No last message to update!");
                    }
                } else {
                    debug!("Hatching mode - not updating messages");
                }
            }

            return Ok(Some(Action::Update));
        }

        if frame_type == Some("response_done") {
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
                warn!("Response done: no streaming_response buffer!");
            }
            return Ok(Some(Action::Update));
        }

        let payload = parsed.as_ref().and_then(|v| {
            if v.get("type").and_then(|t| t.as_str()) == Some("response") {
                v.get("received").and_then(|r| r.as_str()).map(String::from)
            } else {
                None
            }
        });

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

        if self.showing_hatching {
            if let Some(content) = payload {
                if let Some(ref mut hatching) = self.hatching_page {
                    let mut ps = self.state.pane_state();
                    let _ = hatching.update(Action::HatchingResponse(content), &mut ps);
                }
            }
            return Ok(Some(Action::Update));
        }

        let display = payload.as_deref().unwrap_or(text);

        self.chat_loading_tick = None;
        self.state.loading_line = None;

        self.state.messages.push(DisplayMessage::assistant(display));
        Ok(Some(Action::Update))
    }
}
