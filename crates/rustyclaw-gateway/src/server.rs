//! Gateway server engine.
//!
//! The core session loop: accepts transports, authenticates connections,
//! dispatches chat/messenger requests to model providers, and streams results
//! back. Driven by [`run_gateway`], which accepts both networked and
//! SSH-subsystem stdio transports. Invoked from the binary entry point in
//! `main.rs`.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{trace, warn};

use rustyclaw_core::gateway::{
    ClientFrame, ClientFrameType, ClientPayload, ProbeResult, ServerFrame, ServerFrameType,
    ServerPayload, StatusType, WireFrame, deserialize_frame, protocol, transport,
};
use rustyclaw_core::providers as crate_providers;

use protocol::server::send_frame;

use crate::thread_updates::{
    send_projects_update, send_thread_messages_update_shared, send_threads_update_shared,
};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedObserver,
    SharedSkillManager, SharedTaskManager, SharedVault, TOTP_LOCKOUT_SECS, ToolCancelFlag, admin,
    auth, concurrent, plugin_handler, project_handler, providers, thread_handler,
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

    // Publish the settings dir so tools (agents_list, agents_create, …) can
    // reach the installation-wide agent registry.
    rustyclaw_core::runtime_ctx::set_agent_registry_info(&config.settings_dir, &config.agent_name);

    // Per-agent conversation state (threads + projects). Each connection
    // starts on the `main` agent; `AgentSwitch` frames swap this wholesale.
    // The connection's original base system prompt is kept so switching back
    // from an agent with a prompt override restores it.
    let base_system_prompt = config.system_prompt.clone();
    let mut agent_session =
        crate::agent_handler::AgentSession::load(&config, rustyclaw_core::agents::MAIN_AGENT_ID);
    rustyclaw_core::runtime_ctx::set_active_agent(&agent_session.agent_id);

    // Point the workspace at the restored foreground thread's effective
    // directory — its own override, else its project's — so tools run in the
    // right place from the first turn. Reading the active project's path
    // directly here would drop a restored thread's pin until something else
    // happened to repoint, which is exactly what `repoint_workspace` exists
    // to prevent.
    project_handler::repoint_workspace(
        &mut config,
        &agent_session.project_mgr,
        &*agent_session.thread_mgr.lock().await,
    );

    // Local engine registry for model management.
    let engine_registry = rustyclaw_core::engines::EngineRegistry::new();

    // Subscribe to thread events for push-based sidebar updates
    let mut thread_events_rx = agent_session.thread_mgr.lock().await.subscribe();

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
    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;

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

    // ── Spawn reader task ──────────────────────────────────────────
    //
    // The reader runs in a separate task so responses the running turn is
    // waiting on — tool approvals, `ask_user` answers, credentials — reach
    // it while it is mid-flight. Everything else is forwarded to the loop
    // below through a channel.
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<WireFrame<ClientFrame>>(32);

    // Everything the client answers by call id, routed to the call that
    // asked. Not one shared channel each: a waiter that received an id it
    // did not recognise had already consumed it, destroying another call's
    // answer — and the approval site read the unrecognised id as a *denial*,
    // refusing a tool in the user's name. See `pending`.
    let approvals: Arc<crate::pending::PendingResponses<bool>> = Arc::default();
    let user_prompts: Arc<
        crate::pending::PendingResponses<(
            bool,
            rustyclaw_core::user_prompt_types::PromptResponseValue,
        )>,
    > = Arc::default();
    let credentials: Arc<crate::pending::PendingResponses<(bool, Option<String>)>> = Arc::default();
    let dom_queries: Arc<crate::pending::PendingResponses<(String, bool)>> = Arc::default();

    // Channel for model task responses (concurrent execution).
    let (model_task_tx, mut model_task_rx) = concurrent::channel();

    // Track active model tasks per thread.
    //
    // Shared with the reader task so a Stop is acted on the moment it is
    // decoded. The loop is free while a turn runs, but it still awaits
    // handlers that take their time — thread switching summarises through
    // the model, engine and provider calls go to the network — and a Stop
    // queued behind one of those is the "Stop does nothing" symptom this
    // branch set out to remove. The reader holds a `Weak`: the loop's `Arc`
    // is the only owner, so dropping it still runs the abort in `Drop` even
    // if the reader outlives the connection handler.
    let active_tasks = Arc::new(Mutex::new(concurrent::ActiveTasks::new()));
    let reader_tasks = Arc::downgrade(&active_tasks);
    // The reader delivers answers; the turns claim the ids. Both sides hold
    // the same registries.
    let reader_approvals = approvals.clone();
    let reader_user_prompts = user_prompts.clone();
    let reader_credentials = credentials.clone();
    let reader_dom_queries = dom_queries.clone();
    // Counter for turn ids, so a turn's completion cannot retire the turn
    // that replaced it.
    let mut next_turn_id: u64 = 0;

    // ── Send initial thread list ───────────────────────────────────
    // Freshly-connected clients need to know the current thread state.
    if let Err(e) =
        send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None).await
    {
        warn!(error = %e, "Failed to send initial thread list");
    }
    if let Err(e) = send_projects_update(&mut *writer, &agent_session.project_mgr).await {
        warn!(error = %e, "Failed to send initial project list");
    }
    if let Err(e) =
        crate::agent_handler::send_agents_update(&mut *writer, &config, &agent_session.agent_id)
            .await
    {
        warn!(error = %e, "Failed to send initial agent list");
    }
    if let Err(e) = plugin_handler::send_plugins_update(&mut *writer).await {
        warn!(error = %e, "Failed to send initial plugin list");
    }

    let reader_cancel = cancel.clone();
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
                            // Stop, handled here rather than queued behind
                            // whatever the loop is currently awaiting.
                            //
                            // The client names the turn it means. It has to:
                            // with turns running per thread, "the running
                            // turn" is not something the gateway can resolve
                            // on the user's behalf, and stopping the wrong
                            // conversation is worse than stopping none.
                            // A client that names nothing gets the old
                            // behaviour, which is correct exactly while one
                            // turn is running.
                            if frame.frame_type == ClientFrameType::Cancel {
                                let named = match &frame.payload {
                                    ClientPayload::Cancel { thread_id } => *thread_id,
                                    // Pre-3 clients send `Empty` here.
                                    _ => None,
                                };
                                match reader_tasks.upgrade() {
                                    Some(tasks) => {
                                        // Bound, not held across the body:
                                        // the connection loop wants this lock
                                        // too, and this task must go straight
                                        // back to decoding frames.
                                        let stopped = {
                                            let tasks = tasks.lock().await;
                                            match named {
                                                Some(id) => tasks.request_cancel(
                                                    &rustyclaw_core::threads::ThreadId(id),
                                                ),
                                                None => tasks.request_cancel_sole(),
                                            }
                                        };
                                        if !stopped {
                                            trace!("Cancel with no turn running");
                                        }
                                    }
                                    None => trace!("Cancel after the connection ended"),
                                }
                                continue;
                            }
                            // Process control must be handled here, in the
                            // reader task, so it works while the main loop is
                            // blocked awaiting the very tool being controlled.
                            if frame.frame_type == ClientFrameType::ProcessControl {
                                if let ClientPayload::ProcessControl { pid, action } = frame.payload {
                                    match rustyclaw_core::exec_status::control(pid, action) {
                                        Ok(msg) => tracing::info!(pid, %action, "Process control: {msg}"),
                                        Err(e) => tracing::warn!(pid, %action, "Process control failed: {e}"),
                                    }
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::ToolApprovalResponse {
                                if let ClientPayload::ToolApprovalResponse { id, approved } = frame.payload {
                                    if !reader_approvals.deliver(&id, approved) {
                                        trace!(%id, "Approval for a call nobody is waiting on");
                                    }
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::UserPromptResponse {
                                if let ClientPayload::UserPromptResponse { id, dismissed, value } = frame.payload {
                                    if !reader_user_prompts.deliver(&id, (dismissed, value)) {
                                        trace!(%id, "Answer to a question nobody is waiting on");
                                    }
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::CredentialResponse {
                                if let ClientPayload::CredentialResponse { id, dismissed, value } = frame.payload {
                                    if !reader_credentials.deliver(&id, (dismissed, value)) {
                                        trace!(%id, "Credential for a call nobody is waiting on");
                                    }
                                    continue;
                                }
                            }
                            if frame.frame_type == ClientFrameType::DomQueryResponse {
                                if let ClientPayload::DomQueryResponse { id, result, is_error } = frame.payload {
                                    if !reader_dom_queries.deliver(&id, (result, is_error)) {
                                        trace!(%id, "DOM result for a call nobody is waiting on");
                                    }
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

    // How long a turn gets to wind down after the reader ends before the
    // connection stops waiting for it.
    const TURN_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

    // Set when the reader ends (client disconnected, or the transport ran
    // out of frames). The loop keeps running until in-flight turns finish so
    // their last frames still reach the writer — a turn no longer completes
    // before this point, now that it runs in its own task.
    let mut drain_deadline: Option<tokio::time::Instant> = None;

    // Main message handling loop — receives from channel
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                // Dropping a JoinHandle detaches the task; a turn left
                // running would keep model calls going and then block
                // forever on a frame channel nobody drains.
                active_tasks.lock().await.abort_all();
                let _ = writer.close().await;
                break;
            }
            _ = async {
                match drain_deadline {
                    Some(at) => tokio::time::sleep_until(at).await,
                    // Unreachable: the arm is disabled without a deadline.
                    None => std::future::pending().await,
                }
            }, if drain_deadline.is_some() => {
                warn!("A turn was still running when the client went away; stopping it");
                active_tasks.lock().await.abort_all();
                break;
            }
            msg = frame_rx.recv(), if drain_deadline.is_none() => {
                let envelope = match msg {
                    Some(f) => f,
                    None => {
                        // Reader exited. Ask the running turn to stop and
                        // give it a moment to flush; nothing here waits on a
                        // model call for a client that has left.
                        // Bound first: a guard taken in an `if` scrutinee
                        // outlives the body, and this mutex is not reentrant.
                        let idle = active_tasks.lock().await.running_threads().is_empty();
                        if idle {
                            break;
                        }
                        active_tasks.lock().await.request_cancel_all();
                        drain_deadline = Some(tokio::time::Instant::now() + TURN_DRAIN_GRACE);
                        continue;
                    }
                };
                let stream_id = envelope.stream_id;
                let frame = envelope.frame;

                trace!(stream_id, frame_type = ?frame.frame_type, "Handling client frame");

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
                            // Answered in the reader task, which is the whole
                            // point of it: a Stop queued behind whatever the
                            // loop is awaiting is a Stop that does nothing.
                            // It never reaches here.
                            ClientPayload::Cancel { .. } => {}
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
                            ClientPayload::Chat { messages, thread_id } => {
                                // A turn runs in its own task. The loop goes
                                // straight back to serving frames, so thread
                                // switches, history requests and project
                                // changes all still answer while the model
                                // works — or while it sits on an `ask_user`
                                // question, which used to hold the whole
                                // connection until the user answered it.
                                //
                                // Which thread the message belongs to is
                                // settled *here*, before the turn is handed
                                // off, including the auto-switch to a better
                                // matching thread. Deciding it inside the
                                // task would race the ThreadSwitch frames
                                // this loop is now free to serve: a switch
                                // landing between the spawn and the task's
                                // first poll would file the message, and the
                                // reply, in whichever thread the user had
                                // just opened.
                                active_tasks.lock().await.reap_finished();
                                // The client names the thread it typed into,
                                // and that name wins. The gateway's own
                                // foreground is only a cache of what a client
                                // last asked for, and it moves on its own
                                // now: a `ThreadSwitch` served while the user
                                // was still typing, an auto-switch from the
                                // previous turn, another connection on the
                                // same agent. Reading it at processing time
                                // is exactly how a message lands in a
                                // transcript the user was not looking at.
                                //
                                // `None` means the client has no opinion —
                                // older clients, and the CLI — so the old
                                // behaviour (elect, and auto-switch on a
                                // label match) still applies there.
                                let requested =
                                    thread_id.map(rustyclaw_core::threads::ThreadId);
                                if let Some(want) = requested {
                                    // Adopt it as the foreground too: the
                                    // rest of the turn's setup — workspace
                                    // dir, model context, history — reads
                                    // from there, and leaving the two
                                    // disagreeing is the confusion this is
                                    // meant to end.
                                    let adopted = {
                                        let mut tm = agent_session.thread_mgr.lock().await;
                                        tm.switch_foreground(want)
                                    };
                                    if !adopted {
                                        // The thread went away between the
                                        // client composing and this frame
                                        // arriving. Say so and drop the
                                        // message: filing it under some other
                                        // thread would put the user's words
                                        // somewhere they never chose.
                                        let mut scoped =
                                            rustyclaw_core::gateway::ScopedTransportWriter::new(
                                                &mut *writer,
                                                stream_id,
                                            );
                                        protocol::server::send_error(
                                            &mut scoped,
                                            &format!(
                                                "Thread {} no longer exists — \
                                                 the message was not sent.",
                                                want.0
                                            ),
                                        )
                                        .await?;
                                        // Correlated by thread id, so a
                                        // client tracking several threads
                                        // retires the right one.
                                        protocol::server::send_response_done(
                                            &mut scoped,
                                            false,
                                            Some(want.0),
                                        )
                                        .await?;
                                        send_threads_update_shared(
                                            &mut *writer,
                                            &agent_session.thread_mgr,
                                            &task_mgr,
                                            None,
                                        )
                                        .await?;
                                        continue;
                                    }
                                }
                                let auto_switch = if requested.is_some() {
                                    // Never second-guess an explicit thread.
                                    // `find_best_match` moves the foreground
                                    // whenever another thread's *label* shows
                                    // up anywhere in the message text, which
                                    // is a guess, and a guess must not
                                    // override a statement.
                                    None
                                } else {
                                    let mut tm = agent_session.thread_mgr.lock().await;
                                    messages
                                        .iter()
                                        .rev()
                                        .find(|m| m.role == "user")
                                        .and_then(|last| tm.find_best_match(&last.content))
                                        .filter(|better| tm.switch_foreground(*better))
                                        .map(|better| {
                                            (
                                                better,
                                                tm.foreground()
                                                    .and_then(|t| t.compact_summary.clone()),
                                            )
                                        })
                                };
                                if let Some((better, context_summary)) = auto_switch {
                                    send_frame(
                                        &mut *writer,
                                        &ServerFrame {
                                            frame_type: ServerFrameType::ThreadSwitched,
                                            payload: ServerPayload::ThreadSwitched {
                                                thread_id: better.0,
                                                context_summary,
                                            },
                                        },
                                    )
                                    .await?;
                                    send_threads_update_shared(
                                        &mut *writer,
                                        &agent_session.thread_mgr,
                                        &task_mgr,
                                        None,
                                    )
                                    .await?;
                                    send_thread_messages_update_shared(&mut *writer, better, &agent_session.thread_mgr)
                                    .await?;
                                }
                                // A message needs a thread to live in. If
                                // none is focused — the client backgrounded
                                // it — elect one rather than filing the turn
                                // under `ThreadId(0)`: zero is the wire
                                // sentinel for "nothing is focused", and a
                                // frame carrying it tells the desktop to
                                // clear the transcript.
                                let turn_thread = match requested {
                                    Some(want) => Some(want),
                                    None => agent_session
                                        .thread_mgr
                                        .lock()
                                        .await
                                        .ensure_foreground(),
                                };
                                // Settling the thread is only half of it. The
                                // turn's file and command tools run in the
                                // workspace directory, and that is global
                                // config state — `switch_foreground` flips a
                                // flag and nothing else. `ThreadSwitch`
                                // repoints explicitly right after switching;
                                // a turn that settles its own thread has to
                                // do the same, whether the client named it or
                                // the label match picked it.
                                //
                                // Otherwise this is the same race one layer
                                // down, and worse: a `ThreadSwitch` to B
                                // served while the user was typing into A
                                // leaves the turn correctly filed under A but
                                // writing files into B's directory. Right
                                // conversation, wrong project on disk.
                                if let Some(pid) = {
                                    let tm = agent_session.thread_mgr.lock().await;
                                    tm.foreground().map(|t| t.project_id)
                                } {
                                    if pid != agent_session.project_mgr.active_id() {
                                        project_handler::activate_project(
                                            &mut *writer,
                                            &mut config,
                                            &mut agent_session.project_mgr,
                                            &agent_session.thread_mgr,
                                            &agent_session.projects_path,
                                            pid,
                                        )
                                        .await?;
                                    } else {
                                        // Synchronous, so the guard in the
                                        // argument list never spans an await.
                                        project_handler::repoint_workspace(
                                            &mut config,
                                            &agent_session.project_mgr,
                                            &*agent_session.thread_mgr.lock().await,
                                        );
                                    }
                                }
                                // A key for tracking the turn even when
                                // there is no thread at all to elect. It
                                // never reaches the client.
                                let turn_key = turn_thread
                                    .unwrap_or(rustyclaw_core::threads::ThreadId(0));
                                {
                                    // A fresh flag per turn: see ActiveTasks.
                                    let tool_cancel: ToolCancelFlag =
                                        Arc::new(AtomicBool::new(false));
                                    next_turn_id += 1;
                                    let turn_id = next_turn_id;
                                    let mut sink = concurrent::ChannelSink::new(
                                        model_task_tx.clone(),
                                        turn_key,
                                        turn_id,
                                        stream_id,
                                    );
                                    let http = http.clone();
                                    let config = config.clone();
                                    let vault = vault.clone();
                                    let skill_mgr = skill_mgr.clone();
                                    let task_mgr = task_mgr.clone();
                                    let observer = observer.clone();
                                    let turn_cancel = tool_cancel.clone();
                                    let shared_config = shared_config.clone();
                                    let shared_model_ctx = shared_model_ctx.clone();
                                    let shared_copilot_session = shared_copilot_session.clone();
                                    let approvals = approvals.clone();
                                    let user_prompts = user_prompts.clone();
                                    let credentials = credentials.clone();
                                    let dom_queries = dom_queries.clone();
                                    let thread_mgr = agent_session.thread_mgr.clone();
                                    let threads_path = agent_session.threads_path.clone();
                                    let handle = tokio::spawn(async move {
                                        let result = crate::chat::handle_chat_frame(
                                            &http,
                                            messages,
                                            stream_id,
                                            &mut sink,
                                            &config,
                                            &vault,
                                            &skill_mgr,
                                            &task_mgr,
                                            observer.as_ref(),
                                            &turn_cancel,
                                            &shared_config,
                                            &shared_model_ctx,
                                            &shared_copilot_session,
                                            &approvals,
                                            &user_prompts,
                                            &credentials,
                                            &dom_queries,
                                            &thread_mgr,
                                            turn_thread,
                                            &threads_path,
                                        )
                                        .await;
                                        match result {
                                            // The turn already recorded its
                                            // own assistant message; `None`
                                            // keeps the loop from adding a
                                            // second copy.
                                            Ok(()) => sink.done(None).await,
                                            Err(e) => sink.error(format!("{e:#}")).await,
                                        }
                                    });
                                    // One turn per thread, any number of
                                    // threads. `register` displaces the entry
                                    // for this thread only, so a second
                                    // message in the same conversation still
                                    // replaces its predecessor while turns
                                    // elsewhere keep running.
                                    //
                                    // This used to `abort_all()` first: with
                                    // one shared response channel per kind,
                                    // two turns would take each other's tool
                                    // approvals and `ask_user` answers, and a
                                    // stolen approval read as a denial. Those
                                    // are routed by call id now, so the
                                    // reason is gone.
                                    active_tasks.lock().await.register(
                                        turn_key,
                                        turn_id,
                                        handle,
                                        tool_cancel,
                                    );
                                }
                            }
                            ClientPayload::TasksRequest { session } => {
                                thread_handler::handle_tasks_request(&mut *writer, &task_mgr, session).await?;
                            }
                            ClientPayload::ThreadCreate { label, project_id } => {
                                // 0 means "the active project".
                                let pid = if project_id == 0 {
                                    agent_session.project_mgr.active_id()
                                } else {
                                    rustyclaw_core::projects::ProjectId(project_id)
                                };
                                // Creating into a different project also makes it
                                // active and repoints the workspace dir.
                                if pid != agent_session.project_mgr.active_id() {
                                    project_handler::activate_project(
                                        &mut *writer,
                                        &mut config,
                                        &mut agent_session.project_mgr,
                                        &agent_session.thread_mgr,
                                        &agent_session.projects_path,
                                        pid,
                                    )
                                    .await?;
                                }
                                thread_handler::handle_thread_create(
                                    &mut *writer,
                                    &agent_session.thread_mgr,
                                    &task_mgr,
                                    &agent_session.threads_path,
                                    pid,
                                    label,
                                )
                                .await?;
                            }
                            ClientPayload::ThreadSwitch { thread_id } => {
                                // Read before the call, never inside its
                                // argument list: a temporary guard lives to
                                // the end of the statement, which here spans
                                // an awaited handler that talks to the model.
                                // The reader task takes this same lock to act
                                // on Stop, so holding it across that call
                                // would stall every inbound frame — Stop,
                                // tool approvals, `ask_user` answers — for as
                                // long as the provider took.
                                let busy_threads =
                                    active_tasks.lock().await.running_threads();
                                thread_handler::handle_thread_switch(
                                    &mut *writer,
                                    &agent_session.thread_mgr,
                                    &task_mgr,
                                    &agent_session.threads_path,
                                    &shared_model_ctx,
                                    &http,
                                    thread_id,
                                    &busy_threads,
                                )
                                .await?;
                                // Repoint the workspace at the new foreground
                                // thread's effective directory: its own override
                                // when it has one, else its project's directory.
                                // Threads in the active project still need this —
                                // an override differs from the project dir.
                                // Bound to a local first: a guard produced in
                                // the scrutinee of an `if let` lives for the
                                // whole success block, and both arms below
                                // take this lock again. `tokio::sync::Mutex`
                                // is not reentrant, so that is a hang, not a
                                // slow path.
                                let foreground_project = {
                                    let tm = agent_session.thread_mgr.lock().await;
                                    tm.foreground().map(|t| t.project_id)
                                };
                                if let Some(pid) = foreground_project {
                                    if pid != agent_session.project_mgr.active_id() {
                                        project_handler::activate_project(
                                            &mut *writer,
                                            &mut config,
                                            &mut agent_session.project_mgr,
                                            &agent_session.thread_mgr,
                                            &agent_session.projects_path,
                                            pid,
                                        )
                                        .await?;
                                    } else {
                                        project_handler::repoint_workspace(
                                            &mut config,
                                            &agent_session.project_mgr,
                                            &*agent_session.thread_mgr.lock().await,
                                        );
                                    }
                                }
                            }
                            ClientPayload::ThreadList => {
                                thread_handler::handle_thread_list(&mut *writer, &agent_session.thread_mgr, &task_mgr).await?;
                            }
                            ClientPayload::ThreadHistoryRequest { thread_id } => {
                                thread_handler::handle_thread_history(&mut *writer, &agent_session.thread_mgr, thread_id).await?;
                            }
                            ClientPayload::ThreadClose { thread_id } => {
                                // The turn writing to this thread has nowhere
                                // to put its answer once the thread is gone:
                                // every persistence point resolves the thread
                                // by id, so it would keep calling the model
                                // and dropping the results, while staying
                                // registered and holding the connection busy.
                                let stopped = active_tasks
                                    .lock()
                                    .await
                                    .request_cancel(&rustyclaw_core::threads::ThreadId(thread_id));
                                if stopped {
                                    trace!(thread_id, "Stopping the turn for a closed thread");
                                }
                                thread_handler::handle_thread_close(
                                    &mut *writer,
                                    &agent_session.thread_mgr,
                                    &task_mgr,
                                    &agent_session.threads_path,
                                    thread_id,
                                )
                                .await?;
                            }
                            ClientPayload::ThreadRename { thread_id, new_label } => {
                                thread_handler::handle_thread_rename(
                                    &mut *writer,
                                    &agent_session.thread_mgr,
                                    &task_mgr,
                                    &agent_session.threads_path,
                                    thread_id,
                                    new_label,
                                )
                                .await?;
                            }
                            ClientPayload::ThreadUpdate { thread_id, label, working_dir } => {
                                thread_handler::handle_thread_update(
                                    &mut *writer,
                                    &mut config,
                                    &agent_session.thread_mgr,
                                    &agent_session.project_mgr,
                                    &task_mgr,
                                    &agent_session.threads_path,
                                    thread_id,
                                    label,
                                    working_dir,
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
                            ClientPayload::PluginList => {
                                plugin_handler::handle_plugin_list(&mut *writer).await?;
                            }
                            ClientPayload::PluginRefresh { plugin_name } => {
                                plugin_handler::handle_plugin_refresh(&mut *writer, plugin_name).await?;
                            }
                            ClientPayload::WorkspaceListDir { path } => {
                                let root = config.workspace_dir();
                                crate::workspace_files::handle_list_dir(&mut *writer, &root, path).await?;
                            }
                            ClientPayload::WorkspaceReadFile { path } => {
                                let root = config.workspace_dir();
                                crate::workspace_files::handle_read_file(&mut *writer, &root, path).await?;
                            }
                            ClientPayload::WorkspaceWriteFile { path, content, expected_root } => {
                                let root = config.workspace_dir();
                                crate::workspace_files::handle_write_file(&mut *writer, &root, path, content, expected_root).await?;
                            }
                            ClientPayload::ProjectList => {
                                project_handler::handle_project_list(&mut *writer, &agent_session.project_mgr).await?;
                            }
                            ClientPayload::ProjectCreate { name, path } => {
                                project_handler::handle_project_create(
                                    &mut *writer,
                                    &mut config,
                                    &mut agent_session.project_mgr,
                                    &agent_session.thread_mgr,
                                    &agent_session.projects_path,
                                    name,
                                    path,
                                )
                                .await?;
                            }
                            ClientPayload::ProjectRename { project_id, new_name } => {
                                project_handler::handle_project_rename(
                                    &mut *writer,
                                    &mut agent_session.project_mgr,
                                    &agent_session.projects_path,
                                    project_id,
                                    new_name,
                                )
                                .await?;
                            }
                            ClientPayload::ProjectUpdate { project_id, name, path } => {
                                project_handler::handle_project_update(
                                    &mut *writer,
                                    &mut config,
                                    &mut agent_session.project_mgr,
                                    &agent_session.thread_mgr,
                                    &agent_session.projects_path,
                                    project_id,
                                    name,
                                    path,
                                )
                                .await?;
                            }
                            ClientPayload::ProjectDelete { project_id } => {
                                // Reassign the doomed project's threads to Default
                                // so they aren't orphaned, then delete + repoint.
                                let pid = rustyclaw_core::projects::ProjectId(project_id);
                                let orphans: Vec<_> =
                                    agent_session.thread_mgr.lock().await.threads_for(pid).iter().map(|t| t.id).collect();
                                for tid in orphans {
                                    agent_session.thread_mgr.lock().await.set_project(
                                        tid,
                                        rustyclaw_core::projects::DEFAULT_PROJECT_ID,
                                    );
                                }
                                project_handler::handle_project_delete(
                                    &mut *writer,
                                    &mut config,
                                    &mut agent_session.project_mgr,
                                    &agent_session.thread_mgr,
                                    &agent_session.projects_path,
                                    project_id,
                                )
                                .await?;
                                crate::helpers::persist_threads(&*agent_session.thread_mgr.lock().await, &agent_session.threads_path);
                                send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None).await?;
                            }
                            ClientPayload::ProjectSwitch { project_id } => {
                                project_handler::handle_project_switch(
                                    &mut *writer,
                                    &mut config,
                                    &mut agent_session.project_mgr,
                                    &agent_session.thread_mgr,
                                    &agent_session.projects_path,
                                    project_id,
                                )
                                .await?;
                            }
                            ClientPayload::AgentListRequest => {
                                crate::agent_handler::handle_agent_list(
                                    &mut *writer,
                                    &config,
                                    &agent_session.agent_id,
                                )
                                .await?;
                            }
                            ClientPayload::AgentSwitch { agent_id } => {
                                let switched = crate::agent_handler::handle_agent_switch(
                                    &mut *writer,
                                    &mut config,
                                    &base_system_prompt,
                                    &mut agent_session,
                                    &task_mgr,
                                    agent_id,
                                )
                                .await?;
                                if switched {
                                    // The thread manager was replaced — follow
                                    // the new one's sidebar events.
                                    thread_events_rx = agent_session.thread_mgr.lock().await.subscribe();
                                }
                            }
                            ClientPayload::AgentCreate { name, agent_id, description } => {
                                crate::agent_handler::handle_agent_create(
                                    &mut *writer,
                                    &config,
                                    &agent_session.agent_id,
                                    name,
                                    agent_id,
                                    description,
                                )
                                .await?;
                            }
                            ClientPayload::AgentDelete { agent_id } => {
                                crate::agent_handler::handle_agent_delete(
                                    &mut *writer,
                                    &config,
                                    &agent_session.agent_id,
                                    agent_id,
                                )
                                .await?;
                            }
                            ClientPayload::HostInfoRequest => {
                                crate::kernel_handler::handle_host_info_request(&mut *writer).await?;
                            }
                            ClientPayload::LoadStatusRequest => {
                                crate::kernel_handler::handle_load_status_request(&mut *writer).await?;
                            }
                            ClientPayload::ServiceListRequest => {
                                crate::service_handler::handle_service_list(&mut *writer).await?;
                            }
                            ClientPayload::ServiceStartRequest { name } => {
                                crate::service_handler::handle_service_start(&mut *writer, &name).await?;
                            }
                            ClientPayload::ServiceStopRequest { name } => {
                                crate::service_handler::handle_service_stop(&mut *writer, &name).await?;
                            }
                            ClientPayload::ServiceRestartRequest { name } => {
                                crate::service_handler::handle_service_restart(&mut *writer, &name).await?;
                            }
                            ClientPayload::ServiceLogsRequest { name, tail } => {
                                crate::service_handler::handle_service_logs(&mut *writer, &name, tail).await?;
                            }
                            // ── New UI panel requests (stub handlers) ──
                            payload @ (ClientPayload::CronListRequest
                            | ClientPayload::CronUpsertRequest { .. }
                            | ClientPayload::CronActionRequest { .. }
                            | ClientPayload::MemoryListRequest { .. }
                            | ClientPayload::MemoryUpsertRequest { .. }
                            | ClientPayload::MemoryDeleteRequest { .. }
                            | ClientPayload::HistorySearchRequest { .. }
                            | ClientPayload::UsageStatsRequest { .. }
                            | ClientPayload::LogsRequest { .. }
                            | ClientPayload::McpListRequest
                            | ClientPayload::McpConnectRequest { .. }
                            | ClientPayload::McpDisconnectRequest { .. }
                            | ClientPayload::ToolConfigRequest
                            | ClientPayload::ToolToggleRequest { .. }
                            | ClientPayload::ChannelStatusRequest
                            | ClientPayload::ChannelPairRequest { .. }
                            | ClientPayload::PendingApprovalsRequest
                            | ClientPayload::ApprovalsBatchAction { .. }
                            | ClientPayload::VoiceStart { .. }
                            | ClientPayload::VoiceStop
                            | ClientPayload::VoiceAudioChunk { .. }
                            | ClientPayload::PreviewRequest { .. }
                            | ClientPayload::PreviewFollowToggle { .. }) => {
                                crate::panel_handler::handle_panel_request(&mut *writer, payload, &mut config).await?;
                            }
                            payload @ (ClientPayload::EngineList
                            | ClientPayload::EngineAction { .. }
                            | ClientPayload::EngineModelList { .. }
                            | ClientPayload::EngineModelPull { .. }
                            | ClientPayload::EngineModelAction { .. }) => {
                                crate::engine_handler::handle_engine_request(
                                    &mut *writer,
                                    payload,
                                    &engine_registry,
                                    &config.engines,
                                ).await?;
                            }
                            ClientPayload::EngineConfigSet { engine, config: new_cfg } => {
                                // Persist engine config change, then ack.
                                config.engines.insert(engine.clone(), new_cfg.clone());
                                crate::helpers::persist_config(&config);
                                crate::engine_handler::handle_engine_request(
                                    &mut *writer,
                                    ClientPayload::EngineConfigSet { engine, config: new_cfg },
                                    &engine_registry,
                                    &config.engines,
                                ).await?;
                            }
                            ClientPayload::ProviderModelList { provider } => {
                                handle_provider_model_list(&mut *writer, &provider, &config, &vault).await?;
                            }
                            ClientPayload::Empty | ClientPayload::AuthChallenge { .. } | ClientPayload::AuthResponse { .. } | ClientPayload::ToolApprovalResponse { .. } | ClientPayload::UserPromptResponse { .. } | ClientPayload::CredentialResponse { .. } | ClientPayload::DomQueryResponse { .. } | ClientPayload::ProcessControl { .. } => {
                                // AuthChallenge/AuthResponse handled in auth phase.
                                // ToolApprovalResponse handled by the reader task.
                                // UserPromptResponse handled by the reader task.
                                // CredentialResponse handled by the reader task.
                                // DomQueryResponse handled by the reader task.
                                // ProcessControl handled by the reader task.
                            }
                        }
            }
            // Handle messages from spawned model tasks
            model_msg = model_task_rx.recv() => {
                if let Some(task_msg) = model_msg {
                    match task_msg {
                        concurrent::ModelTaskMessage::Frame {
                            stream_id,
                            turn_id: _,
                            data,
                        } => {
                            // Deserialize and forward frame to client, on the
                            // stream its request came in on.
                            if let Ok(frame) = deserialize_frame::<ServerFrame>(&data) {
                                writer.send_on_stream(stream_id, &frame).await?;
                            }
                        }
                        concurrent::ModelTaskMessage::Done { thread_id, turn_id, response } => {
                            // Retire this turn — unless the client's next
                            // message already started another one on this
                            // thread, which `reap_finished` allows.
                            active_tasks.lock().await.remove_if(&thread_id, turn_id);
                            let last_turn_drained =
                                drain_deadline.is_some() && active_tasks.lock().await.running_threads().is_empty();

                            // Record assistant response in thread history if provided
                            if let Some(text) = response {
                                {
                                    let mut tm = agent_session.thread_mgr.lock().await;
                                    if let Some(thread) = tm.get_mut(thread_id) {
                                        thread.add_message(rustyclaw_core::threads::MessageRole::Assistant, &text);
                                    }
                                }
                                send_thread_messages_update_shared(&mut *writer, thread_id, &agent_session.thread_mgr).await?;
                            }

                            // Send updated thread list (status may have changed)
                            send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None).await?;

                            // Persist thread state
                            crate::helpers::persist_threads(&*agent_session.thread_mgr.lock().await, &agent_session.threads_path);

                            if last_turn_drained {
                                break;
                            }
                        }
                        concurrent::ModelTaskMessage::Error { thread_id, turn_id, message } => {
                            // Same identity check as Done above.
                            active_tasks.lock().await.remove_if(&thread_id, turn_id);
                            let last_turn_drained =
                                drain_deadline.is_some() && active_tasks.lock().await.running_threads().is_empty();

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
                            send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None).await?;

                            if last_turn_drained {
                                break;
                            }
                        }
                    }
                }
            }
            // Handle thread events for push-based sidebar updates
            thread_event = thread_events_rx.recv() => {
                if let Ok(event) = thread_event {
                    // Only send updates for events that affect sidebar display
                    if event.triggers_sidebar_update() {
                        send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None).await?;
                    }
                }
            }
        }
    }

    // Clean up reader task
    reader_handle.abort();

    // Persist thread state on disconnect. This is the last write of the
    // session and carries everything said during it, so a failure here is
    // the most expensive one to lose silently.
    crate::helpers::persist_threads(
        &*agent_session.thread_mgr.lock().await,
        &agent_session.threads_path,
    );

    Ok(())
}

/// Handle a `ProviderModelList` request: fetch the live model list for a
/// cloud provider using the gateway's vault-held API key (falling back to
/// the provider's env var), replying with a `ProviderModelListResult`
/// frame that carries either the model ids or an error string.
async fn handle_provider_model_list(
    writer: &mut dyn rustyclaw_core::gateway::TransportWriter,
    provider: &str,
    config: &rustyclaw_core::config::Config,
    vault: &SharedVault,
) -> Result<()> {
    // API key: vault first (where onboarding stores it), then env var.
    let api_key = match crate_providers::secret_key_for_provider(provider) {
        Some(key_name) => {
            let from_vault = vault.lock().await.get_secret(key_name, true).ok().flatten();
            from_vault.or_else(|| std::env::var(key_name).ok())
        }
        None => None,
    };

    // Respect a base-URL override when the request targets the currently
    // configured provider.
    let base_url = config
        .model
        .as_ref()
        .filter(|m| m.provider == provider)
        .and_then(|m| m.base_url.clone());

    let (models, error) = match crate_providers::fetch_models(
        provider,
        api_key.as_deref(),
        base_url.as_deref(),
    )
    .await
    {
        Ok(models) => (models, None),
        Err(e) => (Vec::new(), Some(format!("{:#}", e))),
    };

    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::ProviderModelListResult,
            payload: ServerPayload::ProviderModelListResult {
                provider: provider.to_string(),
                models,
                error,
            },
        },
    )
    .await
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
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
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
    async fn agent_create_switch_and_delete_roundtrip() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;

        let create = ClientFrame {
            frame_type: ClientFrameType::AgentCreate,
            payload: ClientPayload::AgentCreate {
                name: "Researcher".into(),
                agent_id: None,
                description: Some("digs through papers".into()),
            },
        };
        let switch = ClientFrame {
            frame_type: ClientFrameType::AgentSwitch,
            payload: ClientPayload::AgentSwitch {
                agent_id: "researcher".into(),
            },
        };
        let delete_active = ClientFrame {
            frame_type: ClientFrameType::AgentDelete,
            payload: ClientPayload::AgentDelete {
                agent_id: "researcher".into(),
            },
        };

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };

        let (mock_transport, outgoing) = MockTransport::with_frames(
            peer,
            vec![Some(create), Some(switch), Some(delete_active), None],
        );

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
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

        // Create must broadcast an AgentsUpdate that includes the new agent.
        let has_researcher_in_list = frames.iter().any(|f| {
            matches!(
                &f.payload,
                ServerPayload::AgentsUpdate { agents, .. }
                    if agents.iter().any(|a| a.id == "researcher")
            )
        });
        assert!(
            has_researcher_in_list,
            "Expected researcher in AgentsUpdate"
        );

        // Switch must confirm with AgentSwitched and mark it active.
        let switched = frames.iter().any(|f| {
            matches!(
                &f.payload,
                ServerPayload::AgentSwitched { agent_id, name }
                    if agent_id == "researcher" && name == "Researcher"
            )
        });
        assert!(switched, "Expected AgentSwitched frame for researcher");
        let active_after_switch = frames.iter().any(|f| {
            matches!(
                &f.payload,
                ServerPayload::AgentsUpdate { active_id, .. } if active_id == "researcher"
            )
        });
        assert!(
            active_after_switch,
            "Expected AgentsUpdate with researcher active"
        );

        // Deleting the active agent must be refused.
        let delete_refused = frames.iter().any(|f| {
            matches!(
                &f.payload,
                ServerPayload::Error { message, .. }
                    if message.contains("active agent")
            )
        });
        assert!(delete_refused, "Expected error deleting the active agent");

        Ok(())
    }

    /// Switching threads must not wedge the connection.
    ///
    /// The follow-up that repoints the workspace reads the new foreground
    /// thread and then, in both branches, touches the thread manager again.
    /// Taking the lock in the `if let` scrutinee kept its guard alive across
    /// the whole block, and `tokio::sync::Mutex` is not reentrant — so the
    /// first thread switch a user made hung the connection loop for good,
    /// taking any running turn with it. This drives a real `ThreadSwitch`
    /// through `handle_transport_connection` and fails on its timeout if
    /// that ever comes back.
    #[tokio::test]
    async fn switching_threads_does_not_wedge_the_connection() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;

        // Seed a thread so the switch has somewhere to land — the deadlock
        // needed a foreground thread to exist afterwards.
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;
        let target = {
            let mut manager = rustyclaw_core::threads::ThreadManager::new();
            let id = manager.create_chat("seeded");
            manager.save_to_file(&threads_path)?;
            id
        };

        let switch = ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch {
                thread_id: target.0,
            },
        };
        // A follow-up frame proves the loop kept serving after the switch.
        let list = ClientFrame {
            frame_type: ClientFrameType::ThreadList,
            payload: ClientPayload::ThreadList,
        };

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
        let (mock_transport, outgoing) =
            MockTransport::with_frames(peer, vec![Some(switch), Some(list), None]);

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        tokio::time::timeout(
            std::time::Duration::from_secs(20),
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
            ),
        )
        .await
        .expect("a thread switch must not deadlock the connection loop")?;

        let frames = outgoing.lock().await;
        assert!(
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::ThreadSwitched)),
            "Expected the switch to be acknowledged"
        );
        assert!(
            frames
                .iter()
                .filter(|f| matches!(f.frame_type, ServerFrameType::ThreadsUpdate))
                .count()
                >= 2,
            "Expected the loop to still answer the frame after the switch"
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
                thread_id: None,
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
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
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

    /// Seed two chat threads and leave `first` in the foreground.
    fn seed_two_threads(
        cfg: &Config,
        first: &str,
        second: &str,
    ) -> Result<(
        rustyclaw_core::threads::ThreadId,
        rustyclaw_core::threads::ThreadId,
    )> {
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;
        let mut manager = rustyclaw_core::threads::ThreadManager::new();
        let a = manager.create_chat(first);
        let b = manager.create_chat(second);
        manager.switch_foreground(a);
        manager.save_to_file(&threads_path)?;
        Ok((a, b))
    }

    /// A message that names its thread stays in it.
    ///
    /// `find_best_match` moves the foreground whenever another thread's label
    /// turns up anywhere in the message text — "how does the parser work?"
    /// with a thread called "parser" is enough. That guess used to run on
    /// every message, so a user talking *about* another thread had their
    /// words filed in it. A client that names the thread it typed into has
    /// stated the answer, and a guess must not overrule a statement.
    #[tokio::test]
    async fn a_named_thread_beats_the_auto_switch_guess() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                // Mentions the other thread by name: bait for the guess.
                messages: vec![ChatMessage::text("user", "remind me what beta was for")],
                thread_id: Some(alpha.0),
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
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
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
            !frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::ThreadSwitched { thread_id, .. } if *thread_id == beta.0
            )),
            "The named thread must not be second-guessed by a label match"
        );
        let last_foreground = frames
            .iter()
            .rev()
            .find_map(|f| match &f.payload {
                ServerPayload::ThreadsUpdate { foreground_id, .. } => Some(*foreground_id),
                _ => None,
            })
            .expect("Expected at least one ThreadsUpdate");
        assert_eq!(
            last_foreground,
            Some(alpha.0),
            "The named thread should be the one in use"
        );

        Ok(())
    }

    /// Two messages in different threads both start a turn.
    ///
    /// The second used to be refused with "still working on the previous
    /// message" — not because two turns could not run, but because the four
    /// client-response channels were one per connection and whichever turn
    /// held the lock consumed the other's mail. Now that answers are routed
    /// by call id, both run.
    ///
    /// No model is configured here, so each turn fails fast; what is being
    /// asserted is that the *second message was accepted* — the refusal
    /// notice is gone, and both threads got a turn of their own.
    #[tokio::test]
    async fn two_threads_can_both_be_working() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let chat = |thread: rustyclaw_core::threads::ThreadId, text: &str| ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", text)],
                thread_id: Some(thread.0),
            },
        };

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
        let (mock_transport, outgoing) = MockTransport::with_frames(
            peer,
            vec![Some(chat(alpha, "first")), Some(chat(beta, "second")), None],
        );

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        tokio::time::timeout(
            std::time::Duration::from_secs(20),
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
            ),
        )
        .await
        .expect("two concurrent turns must not wedge the connection")?;

        let frames = outgoing.lock().await;
        assert!(
            !frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::Info { message, .. } if message.contains("Still working")
            )),
            "the second message must not be turned away"
        );

        Ok(())
    }

    /// With no thread named, the old behaviour is untouched.
    ///
    /// Clients that predate this — and the headless one-shot, which has no
    /// thread of its own — send `None`, and the gateway still elects and
    /// auto-switches for them.
    #[tokio::test]
    async fn an_unnamed_thread_still_auto_switches() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (_alpha, beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "remind me what beta was for")],
                thread_id: None,
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
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
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
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::ThreadSwitched { thread_id, .. } if *thread_id == beta.0
            )),
            "Without a named thread the gateway should still pick by label match"
        );

        Ok(())
    }

    /// A message for a thread that no longer exists is refused, not re-homed.
    ///
    /// The thread can go away between the user typing and the frame landing —
    /// closed in another window, deleted with its project. Falling back to
    /// "whatever is in the foreground" would put the user's words in a
    /// conversation they never chose; saying so and dropping the message is
    /// the only honest option. The refusal names the thread so a client
    /// tracking several can retire the right one.
    #[tokio::test]
    async fn a_message_for_a_vanished_thread_is_refused() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, _beta) = seed_two_threads(&cfg, "alpha", "beta")?;
        // An id that was never minted: nothing to file the message under.
        let missing = alpha.0 + 4_000;

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "still there?")],
                thread_id: Some(missing),
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
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
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
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::Error { message, .. } if message.contains("no longer exists")
            )),
            "Expected the vanished thread to be reported"
        );
        assert!(
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::ResponseDone { ok, thread_id }
                    if !*ok && *thread_id == Some(missing)
            )),
            "Expected a close-out naming the thread the client was waiting on"
        );

        Ok(())
    }
}
