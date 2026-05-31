//! Gateway server engine.
//!
//! The core session loop: accepts transports, authenticates connections,
//! dispatches chat/messenger requests to model providers, and streams results
//! back. Driven by [`run_gateway`], which accepts both networked and
//! SSH-subsystem stdio transports. Invoked from the binary entry point in
//! `main.rs`.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{trace, warn};

use rustyclaw_core::gateway::{
    ClientFrame, ClientFrameType, ClientPayload, ProbeResult, ServerFrame, ServerFrameType,
    ServerPayload, StatusType, WireFrame, deserialize_frame, protocol, transport,
};
use rustyclaw_core::providers as crate_providers;

use protocol::server::send_frame;

use crate::thread_updates::{send_thread_messages_update, send_threads_update};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedObserver,
    SharedSkillManager, SharedTaskManager, SharedVault, TOTP_LOCKOUT_SECS, ToolCancelFlag, admin,
    auth, concurrent, providers, thread_handler,
};

pub(crate) async fn handle_connection(
    conn: Box<dyn transport::Transport>,
    shared_config: SharedConfig,
    shared_model_ctx: SharedModelCtx,
    shared_copilot_session: SharedCopilotSession,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: SharedTaskManager,
    model_registry: SharedModelRegistry,
    observer: Option<SharedObserver>,
    rate_limiter: auth::RateLimiter,
    cancel: CancellationToken,
) -> Result<()> {
    let peer_info = conn.peer_info().clone();
    let (mut reader, mut writer) = conn.into_split();
    let peer_ip = peer_info.addr.map(|a| a.ip());

    // Snapshot config and model context for this connection.
    // Reload updates the shared state; new connections pick up changes.
    let mut config = shared_config.read().await.clone();
    let model_ctx = shared_model_ctx.read().await.clone();

    // Thread manager for multi-task conversations.
    // Load from persistent storage or create new with default "Main" thread.
    let threads_path = config.sessions_dir().join("threads.json");
    let mut thread_mgr = rustyclaw_core::threads::ThreadManager::load_or_default(&threads_path);

    // Subscribe to thread events for push-based sidebar updates
    let mut thread_events_rx = thread_mgr.subscribe();

    // ── TOTP authentication challenge ───────────────────────────────
    //
    // If TOTP 2FA is enabled, require it for every transport.
    // SSH public-key auth is necessary but not sufficient.
    if config.totp_enabled {
        // Rate limiting requires a peer IP.
        let rate_ip = match peer_ip {
            Some(ip) => ip,
            None => {
                warn!("TOTP required but no peer IP available");
                writer.close().await?;
                return Ok(());
            }
        };

        // Check rate limit first.
        if let Some(remaining) = auth::check_rate_limit(&rate_limiter, rate_ip).await {
            send_frame(
                &mut *writer,
                &ServerFrame {
                    frame_type: ServerFrameType::AuthLocked,
                    payload: ServerPayload::AuthLocked {
                        message: format!("Too many failed attempts. Try again in {}s.", remaining),
                        retry_after: Some(remaining),
                    },
                },
            )
            .await?;
            writer.close().await?;
            return Ok(());
        }

        // Send challenge.
        protocol::server::send_auth_challenge(&mut *writer, "totp")
            .await
            .context("Failed to send auth_challenge")?;

        // Allow up to 3 attempts before closing the connection.
        const MAX_TOTP_ATTEMPTS: u8 = 3;
        let mut attempts = 0u8;

        loop {
            // Wait for auth_response (with a timeout).
            let auth_result = tokio::time::timeout(
                std::time::Duration::from_secs(120),
                auth::wait_for_auth_response(&mut *reader),
            )
            .await;

            match auth_result {
                Ok(Ok(code)) => {
                    let valid = {
                        let mut v = vault.lock().await;
                        match v.verify_totp(code.trim()) {
                            Ok(result) => result,
                            Err(e) => {
                                warn!(error = %e, "TOTP verification error (vault issue?)");
                                false
                            }
                        }
                    };
                    if valid {
                        auth::clear_rate_limit(&rate_limiter, rate_ip).await;
                        protocol::server::send_auth_result(&mut *writer, true, None, None).await?;
                        break; // Authentication successful, continue to main loop
                    } else {
                        attempts += 1;
                        let locked_out = auth::record_totp_failure(&rate_limiter, rate_ip).await;

                        if locked_out {
                            let msg = format!(
                                "Invalid code. Too many failures — locked out for {}s.",
                                TOTP_LOCKOUT_SECS,
                            );
                            protocol::server::send_auth_result(
                                &mut *writer,
                                false,
                                Some(&msg),
                                None,
                            )
                            .await?;
                            writer.close().await?;
                            return Ok(());
                        } else if attempts >= MAX_TOTP_ATTEMPTS {
                            let msg = "Invalid code. Maximum attempts exceeded.";
                            protocol::server::send_auth_result(
                                &mut *writer,
                                false,
                                Some(msg),
                                None,
                            )
                            .await?;
                            writer.close().await?;
                            return Ok(());
                        } else {
                            let remaining = MAX_TOTP_ATTEMPTS - attempts;
                            let msg = format!(
                                "Invalid 2FA code. {} attempt{} remaining.",
                                remaining,
                                if remaining == 1 { "" } else { "s" }
                            );
                            protocol::server::send_auth_result(
                                &mut *writer,
                                false,
                                Some(&msg),
                                Some(true),
                            )
                            .await?;
                            // Continue loop to allow retry
                        }
                    }
                }
                Ok(Err(e)) => {
                    warn!(peer = ?peer_info.addr, error = %e, "Authentication error");
                    return Ok(());
                }
                Err(_) => {
                    protocol::server::send_auth_result(
                        &mut *writer,
                        false,
                        Some("Authentication timed out."),
                        None,
                    )
                    .await?;
                    writer.close().await?;
                    return Ok(());
                }
            }
        }
    }

    // ── Check vault status ──────────────────────────────────────────
    let vault_is_locked = {
        let v = vault.lock().await;
        v.is_locked()
    };

    // ── Send hello ──────────────────────────────────────────────────
    protocol::server::send_hello(
        &mut *writer,
        &config.agent_name,
        &config.settings_dir.to_string_lossy(),
        vault_is_locked,
        model_ctx.as_ref().map(|c| c.provider.as_str()),
        model_ctx.as_ref().map(|c| c.model.as_str()),
    )
    .await
    .context("Failed to send hello message")?;

    if vault_is_locked {
        protocol::server::send_status(
            &mut *writer,
            StatusType::VaultLocked,
            "Secrets vault is locked — provide password to unlock",
        )
        .await
        .context("Failed to send vault_locked status")?;
    }

    // ── Report model status to the freshly-connected client ────────
    let http = reqwest::Client::new();

    match model_ctx {
        Some(ref ctx) => {
            let display = crate_providers::display_name_for_provider(&ctx.provider);

            // 1. Model configured
            let detail = format!("{} / {}", display, ctx.model);
            protocol::server::send_status(&mut *writer, StatusType::ModelConfigured, &detail)
                .await
                .context("Failed to send model_configured status")?;

            // 2. Credentials
            if ctx.api_key.is_some() {
                protocol::server::send_status(
                    &mut *writer,
                    StatusType::CredentialsLoaded,
                    &format!("{} API key loaded", display),
                )
                .await
                .context("Failed to send credentials_loaded status")?;
            } else if crate_providers::secret_key_for_provider(&ctx.provider).is_some()
                && crate_providers::provider_by_id(&ctx.provider).map(|p| p.auth_method)
                    != Some(crate_providers::AuthMethod::OptionalApiKey)
            {
                protocol::server::send_status(
                    &mut *writer,
                    StatusType::CredentialsMissing,
                    &format!("No API key for {} — model calls will fail", display),
                )
                .await
                .context("Failed to send credentials_missing status")?;
            }

            // 3. Validate the connection with a lightweight probe
            //
            // For Copilot providers, exchange the OAuth token for a session
            // token first — the probe must use the session token too.
            //
            // If the cached model context has no API key, try fetching it
            // from the vault (it may have been stored since startup).
            let probe_ctx = if ctx.api_key.is_none() {
                if let Some(key_name) = crate_providers::secret_key_for_provider(&ctx.provider) {
                    let mut v = vault.lock().await;
                    if let Ok(Some(key)) = v.get_secret(key_name, true) {
                        let mut updated = (**ctx).clone();
                        updated.api_key = Some(key);
                        std::sync::Arc::new(updated)
                    } else {
                        ctx.clone()
                    }
                } else {
                    ctx.clone()
                }
            } else {
                ctx.clone()
            };

            protocol::server::send_status(
                &mut *writer,
                StatusType::ModelConnecting,
                &format!("Probing {} …", ctx.base_url),
            )
            .await
            .context("Failed to send model_connecting status")?;

            // Read current copilot session from shared state
            let copilot_session = shared_copilot_session.read().await.clone();

            match providers::validate_model_connection(
                &http,
                &probe_ctx,
                copilot_session.as_deref(),
            )
            .await
            {
                ProbeResult::Ready => {
                    protocol::server::send_status(
                        &mut *writer,
                        StatusType::ModelReady,
                        &format!("{} / {} ready", display, ctx.model),
                    )
                    .await
                    .context("Failed to send model_ready status")?;
                }
                ProbeResult::Connected { warning } => {
                    // Auth is fine, provider is reachable — the specific
                    // probe request wasn't accepted, but chat will likely
                    // work with the real request format.
                    protocol::server::send_status(
                        &mut *writer,
                        StatusType::ModelReady,
                        &format!("{} / {} connected (probe: {})", display, ctx.model, warning),
                    )
                    .await
                    .context("Failed to send model_ready status")?;
                }
                ProbeResult::AuthError { detail } => {
                    protocol::server::send_status(
                        &mut *writer,
                        StatusType::ModelError,
                        &format!("{} auth failed: {}", display, detail),
                    )
                    .await
                    .context("Failed to send model_error status")?;
                }
                ProbeResult::Unreachable { detail } => {
                    protocol::server::send_status(
                        &mut *writer,
                        StatusType::ModelError,
                        &format!("{} probe failed: {}", display, detail),
                    )
                    .await
                    .context("Failed to send model_error status")?;
                }
            }
        }
        None => {
            protocol::server::send_status(
                &mut *writer,
                StatusType::NoModel,
                "No model configured — clients must send full credentials",
            )
            .await
            .context("Failed to send no_model status")?;
        }
    }

    // ── Spawn reader task with cancel flag ─────────────────────────
    //
    // The reader runs in a separate task so it can receive cancel messages
    // even while dispatch_text_message is running. Messages are forwarded
    // through a channel; cancel requests set a shared flag.
    let tool_cancel: ToolCancelFlag = Arc::new(AtomicBool::new(false));
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<WireFrame<ClientFrame>>(32);

    // Channel for tool-approval responses (used by the Ask permission flow).
    let (approval_tx, approval_rx) = tokio::sync::mpsc::channel::<(String, bool)>(4);
    let approval_rx = Arc::new(Mutex::new(approval_rx));

    // Channel for user-prompt responses (used by the ask_user tool).
    let (user_prompt_tx, user_prompt_rx) = tokio::sync::mpsc::channel::<(
        String,
        bool,
        rustyclaw_core::user_prompt_types::PromptResponseValue,
    )>(4);
    let user_prompt_rx = Arc::new(Mutex::new(user_prompt_rx));

    // Channel for credential responses (used when auth fails mid-conversation).
    let (credential_tx, credential_rx) =
        tokio::sync::mpsc::channel::<(String, bool, Option<String>)>(4);
    let credential_rx = Arc::new(Mutex::new(credential_rx));

    // Channel for DOM query responses (used by the client_dom_query tool).
    let (dom_query_tx, dom_query_rx) = tokio::sync::mpsc::channel::<(String, String, bool)>(4);
    let dom_query_rx = Arc::new(Mutex::new(dom_query_rx));

    // Channel for model task responses (concurrent execution).
    let (_model_task_tx, mut model_task_rx) = concurrent::channel();

    // Track active model tasks per thread.
    let mut active_tasks = concurrent::ActiveTasks::new();

    // ── Send initial thread list ───────────────────────────────────
    // Freshly-connected clients need to know the current thread state.
    if let Err(e) = send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await {
        warn!(error = %e, "Failed to send initial thread list");
    }

    let reader_cancel = cancel.clone();
    let reader_tool_cancel = tool_cancel.clone();
    let reader_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = reader_cancel.cancelled() => break,
                result = reader.recv() => {
                    match result {
                        Ok(Some(envelope)) => {
                            let stream_id = envelope.stream_id;
                            let frame = envelope.frame.clone();
                            trace!(stream_id, frame_type = ?frame.frame_type, "Received client frame");
                            // Intercept cancel, approval, and prompt responses
                            if frame.frame_type == ClientFrameType::Cancel {
                                reader_tool_cancel.store(true, Ordering::Relaxed);
                                continue;
                            }
                            if frame.frame_type == ClientFrameType::ToolApprovalResponse {
                                if let ClientPayload::ToolApprovalResponse { id, approved } = frame.payload {
                                    let _ = approval_tx.send((id, approved)).await;
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::UserPromptResponse {
                                if let ClientPayload::UserPromptResponse { id, dismissed, value } = frame.payload {
                                    let _ = user_prompt_tx.send((id, dismissed, value)).await;
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::CredentialResponse {
                                if let ClientPayload::CredentialResponse { id, dismissed, value } = frame.payload {
                                    let _ = credential_tx.send((id, dismissed, value)).await;
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::DomQueryResponse {
                                if let ClientPayload::DomQueryResponse { id, result, is_error } = frame.payload {
                                    let _ = dom_query_tx.send((id, result, is_error)).await;
                                    continue;
                                }
                            }
                            // Forward all other frames to the main loop
                            if frame_tx.send(envelope).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break, // Clean disconnect
                        Err(e) => {
                            trace!(error = %e, "Error reading from transport");
                            break;
                        }
                    }
                }
            }
        }
    });

    // Main message handling loop — receives from channel
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = writer.close().await;
                break;
            }
            msg = frame_rx.recv() => {
                let envelope = match msg {
                    Some(f) => f,
                    None => break, // Channel closed (reader exited)
                };
                let stream_id = envelope.stream_id;
                let frame = envelope.frame;

                trace!(stream_id, frame_type = ?frame.frame_type, "Handling client frame");
                // Reset cancel flag for new request
                tool_cancel.store(false, Ordering::Relaxed);

                        // Handle the frame based on type
                        match frame.payload {
                            payload @ (ClientPayload::UnlockVault { .. }
                            | ClientPayload::SecretsList
                            | ClientPayload::SecretsStore { .. }
                            | ClientPayload::SecretsGet { .. }
                            | ClientPayload::SecretsDelete { .. }
                            | ClientPayload::SecretsPeek { .. }
                            | ClientPayload::SecretsSetPolicy { .. }
                            | ClientPayload::SecretsSetDisabled { .. }
                            | ClientPayload::SecretsDeleteCredential { .. }
                            | ClientPayload::SecretsHasTotp
                            | ClientPayload::SecretsSetupTotp
                            | ClientPayload::SecretsVerifyTotp { .. }
                            | ClientPayload::SecretsRemoveTotp) => {
                                crate::secrets_handler::handle_secrets_frame(
                                    &mut *writer,
                                    &vault,
                                    payload,
                                )
                                .await?;
                            }
                            ClientPayload::Reload => {
                                admin::handle_reload(
                                    &mut *writer,
                                    &config,
                                    &vault,
                                    &shared_config,
                                    &shared_model_ctx,
                                    &shared_copilot_session,
                                    &model_registry,
                                )
                                .await?;
                            }
                            ClientPayload::Chat { messages } => {
                                crate::chat::handle_chat_frame(
                                    &http,
                                    messages,
                                    stream_id,
                                    &mut *writer,
                                    &config,
                                    &vault,
                                    &skill_mgr,
                                    &task_mgr,
                                    observer.as_ref(),
                                    &tool_cancel,
                                    &shared_config,
                                    &shared_model_ctx,
                                    &shared_copilot_session,
                                    &approval_rx,
                                    &user_prompt_rx,
                                    &credential_rx,
                                    &dom_query_rx,
                                    &mut thread_mgr,
                                    &threads_path,
                                )
                                .await?;
                            }
                            ClientPayload::TasksRequest { session } => {
                                thread_handler::handle_tasks_request(&mut *writer, &task_mgr, session).await?;
                            }
                            ClientPayload::ThreadCreate { label } => {
                                thread_handler::handle_thread_create(
                                    &mut *writer,
                                    &mut thread_mgr,
                                    &task_mgr,
                                    &threads_path,
                                    label,
                                )
                                .await?;
                            }
                            ClientPayload::ThreadSwitch { thread_id } => {
                                thread_handler::handle_thread_switch(
                                    &mut *writer,
                                    &mut thread_mgr,
                                    &task_mgr,
                                    &threads_path,
                                    &shared_model_ctx,
                                    &http,
                                    thread_id,
                                )
                                .await?;
                            }
                            ClientPayload::ThreadList => {
                                thread_handler::handle_thread_list(&mut *writer, &mut thread_mgr, &task_mgr).await?;
                            }
                            ClientPayload::ThreadHistoryRequest { thread_id } => {
                                thread_handler::handle_thread_history(&mut *writer, &thread_mgr, thread_id).await?;
                            }
                            ClientPayload::ThreadClose { thread_id } => {
                                thread_handler::handle_thread_close(
                                    &mut *writer,
                                    &mut thread_mgr,
                                    &task_mgr,
                                    &threads_path,
                                    thread_id,
                                )
                                .await?;
                            }
                            ClientPayload::ThreadRename { thread_id, new_label } => {
                                thread_handler::handle_thread_rename(
                                    &mut *writer,
                                    &mut thread_mgr,
                                    &task_mgr,
                                    &threads_path,
                                    thread_id,
                                    new_label,
                                )
                                .await?;
                            }
                            ClientPayload::ModelSwitch { provider, model } => {
                                admin::handle_model_switch(
                                    &mut *writer,
                                    &vault,
                                    &shared_config,
                                    &shared_model_ctx,
                                    &shared_copilot_session,
                                    provider,
                                    model,
                                )
                                .await?;
                            }
                            ClientPayload::SetAgentName { name } => {
                                admin::handle_set_agent_name(&mut config, &shared_config, name).await;
                            }
                            ClientPayload::SetWorkingDirectory { path } => {
                                admin::handle_set_working_directory(&mut config, path);
                            }
                            ClientPayload::Empty | ClientPayload::AuthChallenge { .. } | ClientPayload::AuthResponse { .. } | ClientPayload::ToolApprovalResponse { .. } | ClientPayload::UserPromptResponse { .. } | ClientPayload::CredentialResponse { .. } | ClientPayload::DomQueryResponse { .. } => {
                                // AuthChallenge/AuthResponse handled in auth phase.
                                // ToolApprovalResponse handled by the reader task.
                                // UserPromptResponse handled by the reader task.
                                // CredentialResponse handled by the reader task.
                                // DomQueryResponse handled by the reader task.
                            }
                        }
            }
            // Handle messages from spawned model tasks
            model_msg = model_task_rx.recv() => {
                if let Some(task_msg) = model_msg {
                    match task_msg {
                        concurrent::ModelTaskMessage::Frame(data) => {
                            // Deserialize and forward frame to client
                            if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                                send_frame(&mut *writer, &frame).await?;
                            }
                        }
                        concurrent::ModelTaskMessage::Done { thread_id, response } => {
                            // Task completed - remove from active tasks
                            active_tasks.remove(&thread_id);

                            // Record assistant response in thread history if provided
                            if let Some(text) = response {
                                if let Some(thread) = thread_mgr.get_mut(thread_id) {
                                    thread.add_message(rustyclaw_core::threads::MessageRole::Assistant, &text);
                                }
                                send_thread_messages_update(&mut *writer, thread_id, &thread_mgr).await?;
                            }

                            // Send updated thread list (status may have changed)
                            send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;

                            // Persist thread state
                            let _ = thread_mgr.save_to_file(&threads_path);
                        }
                        concurrent::ModelTaskMessage::Error { thread_id, message } => {
                            // Task failed - remove from active tasks
                            active_tasks.remove(&thread_id);

                            // Send error frame
                            let error_frame = ServerFrame {
                                frame_type: ServerFrameType::Error,
                                payload: ServerPayload::Error {
                                    ok: false,
                                    message,
                                },
                            };
                            send_frame(&mut *writer, &error_frame).await?;

                            // Send updated thread list
                            send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                        }
                    }
                }
            }
            // Handle thread events for push-based sidebar updates
            thread_event = thread_events_rx.recv() => {
                if let Ok(event) = thread_event {
                    // Only send updates for events that affect sidebar display
                    if event.triggers_sidebar_update() {
                        send_threads_update(&mut *writer, &thread_mgr, &task_mgr, None).await?;
                    }
                }
            }
        }
    }

    // Clean up reader task
    reader_handle.abort();

    // Persist thread state on disconnect
    let _ = thread_mgr.save_to_file(&threads_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::handle_transport_connection;
    use async_trait::async_trait;
    use rustyclaw_core::config::Config;
    use rustyclaw_core::gateway::{
        ChatMessage, PeerInfo, Transport, TransportReader, TransportType, TransportWriter,
    };
    use rustyclaw_core::secrets::SecretsManager;
    use rustyclaw_core::skills::SkillManager;
    use std::collections::VecDeque;
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    struct MockTransport {
        peer: PeerInfo,
        incoming: Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
        outgoing: Arc<Mutex<Vec<ServerFrame>>>,
    }

    struct MockReader {
        peer: PeerInfo,
        incoming: Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
    }

    struct MockWriter {
        outgoing: Arc<Mutex<Vec<ServerFrame>>>,
    }

    impl MockTransport {
        fn with_frames(
            peer: PeerInfo,
            frames: Vec<Option<ClientFrame>>,
        ) -> (Self, Arc<Mutex<Vec<ServerFrame>>>) {
            let outgoing = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    peer,
                    incoming: Arc::new(Mutex::new(VecDeque::from(frames))),
                    outgoing: outgoing.clone(),
                },
                outgoing,
            )
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        fn peer_info(&self) -> &PeerInfo {
            &self.peer
        }

        async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
            Ok(self
                .incoming
                .lock()
                .await
                .pop_front()
                .unwrap_or(None)
                .map(WireFrame::control))
        }

        async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
            self.outgoing.lock().await.push(frame.clone());
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn into_split(self: Box<Self>) -> (Box<dyn TransportReader>, Box<dyn TransportWriter>) {
            (
                Box::new(MockReader {
                    peer: self.peer.clone(),
                    incoming: self.incoming.clone(),
                }),
                Box::new(MockWriter {
                    outgoing: self.outgoing.clone(),
                }),
            )
        }
    }

    #[async_trait]
    impl TransportReader for MockReader {
        async fn recv(&mut self) -> Result<Option<WireFrame<ClientFrame>>> {
            Ok(self
                .incoming
                .lock()
                .await
                .pop_front()
                .unwrap_or(None)
                .map(WireFrame::control))
        }

        fn peer_info(&self) -> &PeerInfo {
            &self.peer
        }
    }

    #[async_trait]
    impl TransportWriter for MockWriter {
        async fn send_on_stream(&mut self, _stream_id: u64, frame: &ServerFrame) -> Result<()> {
            self.outgoing.lock().await.push(frame.clone());
            Ok(())
        }

        async fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn test_config_with_temp_state() -> Result<(tempfile::TempDir, Config)> {
        let tmp = tempdir()?;
        let cfg = Config {
            settings_dir: tmp.path().join("state"),
            ..Config::default()
        };

        std::fs::create_dir_all(cfg.settings_dir.clone())?;
        std::fs::create_dir_all(cfg.workspace_dir())?;
        std::fs::create_dir_all(cfg.credentials_dir())?;
        std::fs::create_dir_all(cfg.sessions_dir())?;
        std::fs::create_dir_all(cfg.skills_dir())?;

        Ok((tmp, cfg))
    }

    #[tokio::test]
    async fn ssh_connection_requires_totp_when_enabled() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = true;

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };

        // Disconnect immediately after first server write.
        let (mock_transport, outgoing) = MockTransport::with_frames(peer, vec![None]);

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        handle_transport_connection(
            Box::new(mock_transport),
            Arc::new(RwLock::new(cfg)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            vault,
            skill_mgr,
            task_mgr,
            model_registry,
            None,
            auth::new_rate_limiter(),
            CancellationToken::new(),
        )
        .await?;

        let frames = outgoing.lock().await;
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::AuthChallenge)),
            "Expected TOTP auth challenge for SSH connection when totp_enabled=true"
        );

        Ok(())
    }

    #[tokio::test]
    async fn transport_connection_processes_chat_frames() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "Hello?")],
            },
        };

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };

        let (mock_transport, outgoing) = MockTransport::with_frames(peer, vec![Some(chat), None]);

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        handle_transport_connection(
            Box::new(mock_transport),
            Arc::new(RwLock::new(cfg)),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            vault,
            skill_mgr,
            task_mgr,
            model_registry,
            None,
            auth::new_rate_limiter(),
            CancellationToken::new(),
        )
        .await?;

        let frames = outgoing.lock().await;
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::Hello)),
            "Expected hello frame"
        );
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::Error)),
            "Expected chat request to be processed and produce an error frame when model context is missing"
        );

        Ok(())
    }
}
