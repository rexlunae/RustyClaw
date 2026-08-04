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

/// When this gateway process started. A turn marker in the log older than
/// this was written by a previous process — the turn it opened died with
/// that process, and the resume path picks it up. A younger marker belongs
/// to a turn running right now on some connection of this process, and
/// must be left alone.
static PROCESS_START: std::sync::LazyLock<std::time::SystemTime> =
    std::sync::LazyLock::new(std::time::SystemTime::now);

/// Turns already resumed by this process, keyed by (agent, thread). Two
/// clients connecting together would otherwise each load the same open
/// marker from disk and both restart the same interrupted turn.
static RESUMED_TURNS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<(String, u64)>>,
> = std::sync::LazyLock::new(Default::default);

/// Everything a spawned turn takes from its connection — owned clones, so
/// the turn outlives the borrow of the loop that started it. Built per
/// spawn by the Chat arm and by the resume path, which is the point:
/// resuming an interrupted turn is starting a turn, not a special case.
struct TurnDeps {
    http: reqwest::Client,
    config: rustyclaw_core::config::Config,
    vault: SharedVault,
    skill_mgr: SharedSkillManager,
    task_mgr: SharedTaskManager,
    observer: Option<SharedObserver>,
    shared_config: SharedConfig,
    shared_model_ctx: SharedModelCtx,
    shared_copilot_session: SharedCopilotSession,
    approvals: Arc<crate::pending::PendingResponses<bool>>,
    user_prompts: Arc<
        crate::pending::PendingResponses<(
            bool,
            rustyclaw_core::user_prompt_types::PromptResponseValue,
        )>,
    >,
    credentials: Arc<crate::pending::PendingResponses<(bool, Option<String>)>>,
    dom_queries: Arc<crate::pending::PendingResponses<(String, bool)>>,
    thread_mgr: crate::SharedThreadMgr,
    threads_path: std::path::PathBuf,
}

/// Spawn one turn: run the conversation through `handle_chat_frame` in its
/// own task, reporting completion through the model-task channel. Returns
/// the join handle and the turn's cancel flag for `ActiveTasks::register`.
fn spawn_turn(
    deps: TurnDeps,
    messages: Vec<rustyclaw_core::gateway::ChatMessage>,
    stream_id: u64,
    turn_id: u64,
    turn_thread: Option<rustyclaw_core::threads::ThreadId>,
    model_task_tx: concurrent::ModelTaskTx,
    is_resume: bool,
) -> (tokio::task::JoinHandle<()>, ToolCancelFlag) {
    let turn_key = turn_thread.unwrap_or(rustyclaw_core::threads::ThreadId(0));
    let tool_cancel: ToolCancelFlag = Arc::new(AtomicBool::new(false));
    let turn_cancel = tool_cancel.clone();
    let mut sink = concurrent::ChannelSink::new(model_task_tx, turn_key, turn_id, stream_id);
    let handle = tokio::spawn(async move {
        let result = crate::chat::handle_chat_frame(
            &deps.http,
            messages,
            stream_id,
            &mut sink,
            &deps.config,
            &deps.vault,
            &deps.skill_mgr,
            &deps.task_mgr,
            deps.observer.as_ref(),
            &turn_cancel,
            &deps.shared_config,
            &deps.shared_model_ctx,
            &deps.shared_copilot_session,
            &deps.approvals,
            &deps.user_prompts,
            &deps.credentials,
            &deps.dom_queries,
            &deps.thread_mgr,
            turn_thread,
            &deps.threads_path,
            is_resume,
        )
        .await;
        match result {
            // The turn already recorded its own assistant message; `None`
            // keeps the loop from adding a second copy.
            Ok(()) => sink.done(None).await,
            Err(e) => sink.error(format!("{e:#}")).await,
        }
    });
    (handle, tool_cancel)
}

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
    // Server-initiated turns (resumed ones) run on even stream ids;
    // clients allocate odd ones, so the two can never collide.
    let mut next_server_stream_id: u64 = 0;

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

    // ── Resume turns a previous gateway process left open ──────────
    // A thread whose log ends inside a turn — a start marker with no stop
    // indicator — was still answering when its gateway died. It loads back
    // in as open, and open means running: restart the turn from the
    // thread's recorded conversation, so the answer the user was waiting
    // for arrives instead of silently never coming. Only markers older
    // than this process qualify; a younger one is a turn running right now
    // on another connection. The process-wide claim keeps two clients
    // connecting together from both resuming the same turn.
    {
        let (resumable, abandoned): (
            Vec<rustyclaw_core::threads::ThreadId>,
            Vec<rustyclaw_core::threads::ThreadId>,
        ) = {
            let tm = agent_session.thread_mgr.lock().await;
            let crashed: Vec<rustyclaw_core::threads::ThreadId> = tm
                .open_threads()
                .into_iter()
                .filter(|id| {
                    tm.get(*id)
                        .is_some_and(|t| t.open_turn.is_some_and(|at| at < *PROCESS_START))
                })
                .filter(|id| {
                    RESUMED_TURNS
                        .lock()
                        .expect("resume registry poisoned")
                        .insert((agent_session.agent_id.clone(), id.0))
                })
                .collect();
            crashed
                .into_iter()
                .partition(|id| tm.get(*id).is_some_and(|t| !t.messages.is_empty()))
        };
        // A crashed turn with no recorded conversation has nothing to
        // resume — but leaving its marker open would report the thread as
        // Streaming forever, with nothing left that could ever close it.
        // It gets its overdue stop indicator instead.
        if !abandoned.is_empty() {
            {
                let mut tm = agent_session.thread_mgr.lock().await;
                for thread in abandoned {
                    tm.end_turn(thread, false);
                }
                crate::helpers::persist_threads(&mut tm, &agent_session.threads_path);
            }
            // The initial thread list already went out with the marker
            // still open; without a refreshed one, this client keeps
            // showing the thread as Streaming — composer gated on a reply
            // that will never come — until some unrelated broadcast.
            send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None)
                .await?;
        }
        for thread in resumable {
            let (label, messages) = {
                let tm = agent_session.thread_mgr.lock().await;
                let Some(t) = tm.get(thread) else { continue };
                (
                    t.label.clone(),
                    crate::thread_updates::thread_history_messages(t),
                )
            };
            protocol::server::send_info(
                &mut *writer,
                &format!("Resuming interrupted turn in '{label}'…"),
            )
            .await?;
            {
                let mut tm = agent_session.thread_mgr.lock().await;
                // The orphaned marker gets its overdue stop indicator, and
                // the resumed turn opens its own — the log keeps the crash
                // visible instead of papering over it.
                tm.end_turn(thread, false);
                tm.begin_turn(thread);
                crate::helpers::persist_threads(&mut tm, &agent_session.threads_path);
            }
            send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None)
                .await?;
            next_turn_id += 1;
            let turn_id = next_turn_id;
            next_server_stream_id += 2;
            let stream_id = next_server_stream_id;
            let (handle, tool_cancel) = spawn_turn(
                TurnDeps {
                    http: http.clone(),
                    config: config.clone(),
                    vault: vault.clone(),
                    skill_mgr: skill_mgr.clone(),
                    task_mgr: task_mgr.clone(),
                    observer: observer.clone(),
                    shared_config: shared_config.clone(),
                    shared_model_ctx: shared_model_ctx.clone(),
                    shared_copilot_session: shared_copilot_session.clone(),
                    approvals: approvals.clone(),
                    user_prompts: user_prompts.clone(),
                    credentials: credentials.clone(),
                    dom_queries: dom_queries.clone(),
                    thread_mgr: agent_session.thread_mgr.clone(),
                    threads_path: agent_session.threads_path.clone(),
                },
                messages,
                stream_id,
                turn_id,
                Some(thread),
                model_task_tx.clone(),
                true,
            );
            active_tasks
                .lock()
                .await
                .register(thread, turn_id, stream_id, handle, tool_cancel);
        }
    }

    // ── Outbound queue ─────────────────────────────────────────────
    //
    // The transport takes one writer, and the loop below owned it — so
    // answering the client at all meant reaching that loop first, and a
    // read-only query waited on whatever it was doing. A client asking for a
    // thread's history could go unanswered indefinitely and be shown an empty
    // transcript for a thread with hundreds of messages.
    //
    // With the transport moved into a task of its own, a handle on this queue
    // is enough to answer, and the reader below can serve such queries
    // directly. One task still writes, so frames cannot interleave; the queue
    // fixes their order.
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<rustyclaw_core::gateway::Outbound>(256);
    let writer_handle = tokio::spawn(rustyclaw_core::gateway::drive_writer(writer, out_rx));
    let mut writer: Box<dyn transport::TransportWriter> =
        Box::new(rustyclaw_core::gateway::QueuedWriter::new(out_tx.clone()));

    let reader_cancel = cancel.clone();
    // Deliberately a cell rather than a handle: switching agents replaces the
    // session wholesale, thread store included, and the reader answers history
    // for the life of the connection. Keeping the store it saw at connect
    // would answer from the previous agent after a switch — and ids restart
    // low in each store, so the likely result is not an empty transcript but
    // another agent's conversation under a plausible-looking id.
    let thread_mgr_cell = Arc::new(std::sync::RwLock::new(agent_session.thread_mgr.clone()));
    let reader_thread_mgr = thread_mgr_cell.clone();
    let reader_out_tx = out_tx.clone();
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
                            // Thread history, answered here rather than queued
                            // behind whatever the main loop is awaiting. It is
                            // a read-only query, and going unanswered is not a
                            // delay the user sees as a delay: the client shows
                            // an empty transcript for a thread whose sidebar
                            // row says it has hundreds of messages.
                            if frame.frame_type == ClientFrameType::ThreadHistoryRequest {
                                if let ClientPayload::ThreadHistoryRequest { thread_id } = frame.payload {
                                    let mut history_writer = rustyclaw_core::gateway::QueuedWriter::new(reader_out_tx.clone());
                                    // Read per request, so a switch that
                                    // happened since the last one is honoured.
                                    let thread_mgr_now = reader_thread_mgr
                                        .read()
                                        .expect("thread manager cell poisoned")
                                        .clone();
                                    // Bounded, because the thread manager lock
                                    // is held across turn work and this task
                                    // also serves Stop — blocking here to wait
                                    // out a turn would cost the user the one
                                    // control that stops it. Answering "could
                                    // not read it" beats going quiet, which is
                                    // the silence being fixed.
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(5),
                                        thread_handler::handle_thread_history(
                                            &mut history_writer,
                                            &thread_mgr_now,
                                            thread_id,
                                        ),
                                    )
                                    .await
                                    {
                                        Ok(Ok(())) => {}
                                        Ok(Err(e)) => warn!(thread_id, error = %e, "Thread history reply failed to send"),
                                        Err(_) => {
                                            warn!(thread_id, "Thread history timed out waiting for the thread log");
                                            let reply = ServerFrame {
                                                frame_type: ServerFrameType::ThreadHistoryReply,
                                                payload: ServerPayload::ThreadHistoryReply {
                                                    thread_id,
                                                    ok: false,
                                                    messages: Vec::new(),
                                                    error: Some(
                                                        "Timed out reading the thread log; it is busy with a running turn"
                                                            .to_string(),
                                                    ),
                                                },
                                            };
                                            // Bounded too. Enqueuing waits when
                                            // the outbound queue is full, which
                                            // is exactly the state a wedged
                                            // transport produces — so the
                                            // apology for one stall could park
                                            // this task forever and cost the
                                            // user Stop. Shorter than the
                                            // lookup above: this is a courtesy
                                            // notice, and a queue with no room
                                            // means the client is not receiving
                                            // anything anyway.
                                            if tokio::time::timeout(
                                                std::time::Duration::from_secs(2),
                                                send_frame(&mut history_writer, &reply),
                                            )
                                            .await
                                            .is_err()
                                            {
                                                warn!(
                                                    thread_id,
                                                    "Gave up enqueuing the thread history timeout notice; \
                                                     the outbound queue is not draining"
                                                );
                                            }
                                        }
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
                                // Retire old ephemeral threads — after the
                                // turn's thread is settled, never before:
                                // the sweep could remove the very
                                // conversation this message was typed into
                                // (a completed task thread the user was
                                // still looking at), and resolution would
                                // then refuse the message as addressed to
                                // a thread that "no longer exists",
                                // dropping the user's words. The settled
                                // thread is exempt; its own activity
                                // refreshes its retention window.
                                agent_session
                                    .thread_mgr
                                    .lock()
                                    .await
                                    .cleanup_ephemeral_except(turn_thread);
                                // A second message in this conversation
                                // displaces the turn still running there —
                                // and a displaced turn is aborted at its
                                // next await, often the very wait for a
                                // tool approval or `ask_user` answer. It
                                // will never send the `ToolResult` that
                                // retires the box it left on the user's
                                // screen, nor its own close-out; unsent,
                                // the dead box hides every later request
                                // behind it. Close the old turn out here,
                                // before the new turn exists, so the
                                // retirement can never race the new
                                // turn's own requests — and on the old
                                // turn's stream, where clients track it.
                                // The registry lock is taken and released
                                // in this statement: an `if let` scrutinee's
                                // guard would live across the close-out
                                // write below, and the reader task takes
                                // the same lock to serve Stop — holding it
                                // across a network write would stall every
                                // inbound frame behind that write.
                                let displaced_stream =
                                    active_tasks.lock().await.displace(&turn_key);
                                if let Some(old_stream) = displaced_stream {
                                    let mut scoped =
                                        rustyclaw_core::gateway::ScopedTransportWriter::new(
                                            &mut *writer,
                                            old_stream,
                                        );
                                    protocol::server::send_response_done(
                                        &mut scoped,
                                        false,
                                        Some(turn_key.0).filter(|id| *id != 0),
                                    )
                                    .await?;
                                }
                                // Mark the turn open in the thread's log —
                                // the start half of the stop-indicator pair.
                                // A displaced predecessor gets its stop
                                // marker first: aborted, it will never
                                // write its own. The broadcast right after
                                // is what flips the thread to "Streaming"
                                // in every client's sidebar.
                                if let Some(thread) = turn_thread {
                                    let mut tm = agent_session.thread_mgr.lock().await;
                                    if displaced_stream.is_some() {
                                        tm.end_turn(thread, false);
                                    }
                                    tm.begin_turn(thread);
                                    crate::helpers::persist_threads(
                                        &mut tm,
                                        &agent_session.threads_path,
                                    );
                                }
                                send_threads_update_shared(
                                    &mut *writer,
                                    &agent_session.thread_mgr,
                                    &task_mgr,
                                    None,
                                )
                                .await?;
                                {
                                    next_turn_id += 1;
                                    let turn_id = next_turn_id;
                                    let (handle, tool_cancel) = spawn_turn(
                                        TurnDeps {
                                            http: http.clone(),
                                            config: config.clone(),
                                            vault: vault.clone(),
                                            skill_mgr: skill_mgr.clone(),
                                            task_mgr: task_mgr.clone(),
                                            observer: observer.clone(),
                                            shared_config: shared_config.clone(),
                                            shared_model_ctx: shared_model_ctx.clone(),
                                            shared_copilot_session: shared_copilot_session.clone(),
                                            approvals: approvals.clone(),
                                            user_prompts: user_prompts.clone(),
                                            credentials: credentials.clone(),
                                            dom_queries: dom_queries.clone(),
                                            thread_mgr: agent_session.thread_mgr.clone(),
                                            threads_path: agent_session.threads_path.clone(),
                                        },
                                        messages,
                                        stream_id,
                                        turn_id,
                                        turn_thread,
                                        model_task_tx.clone(),
                                        false,
                                    );
                                    // One turn per thread, any number of
                                    // threads. The predecessor for this
                                    // thread was displaced and closed out
                                    // above, before this turn existed;
                                    // turns elsewhere keep running.
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
                                        stream_id,
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
                                crate::helpers::persist_threads(&mut *agent_session.thread_mgr.lock().await, &agent_session.threads_path);
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
                                    &thread_mgr_cell,
                                    &task_mgr,
                                    agent_id,
                                )
                                .await?;
                                if switched {
                                    // The thread manager was replaced — follow
                                    // the new one's sidebar events. The reader's
                                    // cell was repointed inside the switch, before
                                    // the client heard about it; doing it here
                                    // would be after the new agent's thread list
                                    // had already gone out, and the client asks
                                    // for a transcript as soon as it sees one.
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
                                crate::panel_handler::handle_panel_request(&mut *writer, payload, &mut config, &shared_config).await?;
                            }
                            // ── Messenger setup ──
                            payload @ (ClientPayload::MessengerConfigRequest
                            | ClientPayload::MessengerAccountSave { .. }
                            | ClientPayload::MessengerAccountDelete { .. }
                            | ClientPayload::MessengerSecretsMigrate { .. }
                            | ClientPayload::MessengerRouteSave { .. }
                            | ClientPayload::MessengerRouteDelete { .. }) => {
                                crate::messenger_config_handler::handle_messenger_config(
                                    &mut *writer,
                                    payload,
                                    &mut config,
                                    &shared_config,
                                    &vault,
                                ).await?;
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
                                // Persist through the shared config — writing
                                // this connection's snapshot would erase
                                // settings other connections saved since it
                                // was taken (messenger accounts included).
                                config.engines.insert(engine.clone(), new_cfg.clone());
                                {
                                    let mut shared = shared_config.write().await;
                                    shared.engines.insert(engine.clone(), new_cfg.clone());
                                    crate::helpers::persist_config(&shared);
                                }
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
                        concurrent::ModelTaskMessage::Done { thread_id, stream_id, turn_id, response, closed_out } => {
                            // Backstop: every turn ends with exactly one
                            // close-out, whatever path ended it. A turn that
                            // reported an error frame and returned Ok — or a
                            // future early return nobody audited — must not
                            // leave the thread marked in-flight in every
                            // client forever.
                            if !closed_out {
                                // On the turn's own stream, like every frame
                                // that preceded it — a close-out on the
                                // control stream never releases the clients'
                                // per-stream bookkeeping for the turn.
                                let mut scoped =
                                    rustyclaw_core::gateway::ScopedTransportWriter::new(
                                        &mut *writer,
                                        stream_id,
                                    );
                                protocol::server::send_response_done(
                                    &mut scoped,
                                    false,
                                    Some(thread_id.0).filter(|id| *id != 0),
                                )
                                .await?;
                            }
                            // Retire this turn — unless the client's next
                            // message already started another one on this
                            // thread, which `reap_finished` allows.
                            let still_this_turn =
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

                            // The turn's stop indicator: recorded before
                            // the broadcast below, so the thread list the
                            // clients get says "Ready" — and before the
                            // persist, so a crash after this point still
                            // leaves a closed turn on disk. Only with the
                            // licence: a Done drained after the thread's
                            // next turn began belongs to a displaced
                            // predecessor, whose marker the displacement
                            // already closed — writing one here would
                            // close the *new* turn's marker while it
                            // streams. A completion whose entry was merely
                            // reaped keeps the licence: it is still the
                            // thread's last word, and nothing else will
                            // ever close the marker.
                            if still_this_turn {
                                agent_session.thread_mgr.lock().await.end_turn(thread_id, true);
                            }

                            // Send updated thread list (status may have changed)
                            send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None).await?;

                            // Persist thread state
                            crate::helpers::persist_threads(&mut *agent_session.thread_mgr.lock().await, &agent_session.threads_path);

                            if last_turn_drained {
                                break;
                            }
                        }
                        concurrent::ModelTaskMessage::Error { thread_id, stream_id, turn_id, message, closed_out } => {
                            // Same identity check as Done above — and the
                            // same licence: only the turn still registered
                            // may write its stop indicator.
                            let still_this_turn =
                                active_tasks.lock().await.remove_if(&thread_id, turn_id);
                            if still_this_turn {
                                // A failed turn still ends — with a stop
                                // indicator that says so.
                                agent_session.thread_mgr.lock().await.end_turn(thread_id, false);
                                crate::helpers::persist_threads(&mut *agent_session.thread_mgr.lock().await, &agent_session.threads_path);
                            }
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

                            // A turn that fails still ends, and its ending
                            // must say which turn. The success path closes
                            // out through the turn's own sink, which stamps
                            // the thread; this path bypasses the sink, so it
                            // stamps by hand — unless the turn already sent
                            // its own close-out before the error surfaced.
                            // Without one the clients keep the errored
                            // thread marked in-flight forever — a stuck
                            // spinner if it is on screen, a phantom one when
                            // the user comes back to it. Zero is the
                            // "no thread" registry key and never goes to the
                            // client.
                            if !closed_out {
                                // On the turn's own stream, like every frame
                                // that preceded it — a close-out on the
                                // control stream never releases the clients'
                                // per-stream bookkeeping for the turn.
                                let mut scoped =
                                    rustyclaw_core::gateway::ScopedTransportWriter::new(
                                        &mut *writer,
                                        stream_id,
                                    );
                                protocol::server::send_response_done(
                                    &mut scoped,
                                    false,
                                    Some(thread_id.0).filter(|id| *id != 0),
                                )
                                .await?;
                            }

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
    // The writer task ends on its own once every queue handle is dropped, but
    // the reader task holds one and has just been aborted — so drop this
    // function's handles and let the drain finish before the connection goes.
    // Aborting it instead would discard frames already queued, including the
    // close-out written just above.
    drop(writer);
    drop(out_tx);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), writer_handle).await;

    // Persist thread state on disconnect. This is the last write of the
    // session and carries everything said during it, so a failure here is
    // the most expensive one to lose silently.
    // The connection is over and its turns are aborted with it — no
    // completion message will ever drain for them. Their stop indicators
    // say cancelled; left open, the threads would load as "open" on the
    // next process start and resume turns whose client asked this process,
    // which answered (or died trying) already. A process that crashes
    // never gets here — that is the one case that leaves markers open,
    // and exactly the one the resume path exists for.
    {
        let running = active_tasks.lock().await.running_threads();
        if !running.is_empty() {
            let mut tm = agent_session.thread_mgr.lock().await;
            for thread in running {
                tm.end_turn(thread, false);
            }
        }
    }
    crate::helpers::persist_threads(
        &mut *agent_session.thread_mgr.lock().await,
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
        /// A patient client: once its frames run out, hang up only after
        /// this many close-outs have been sent — the way a real client
        /// stays connected while its turn streams. `None` hangs up the
        /// moment the frames run out (which asks running turns to stop).
        hang_up_after_done: Option<usize>,
    }

    struct MockReader {
        peer: PeerInfo,
        incoming: Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
        outgoing: Arc<Mutex<Vec<ServerFrame>>>,
        hang_up_after_done: Option<usize>,
    }

    /// Shared recv for both mock halves: deliver queued frames, then
    /// either hang up at once or wait for the agreed number of
    /// `ResponseDone` frames first.
    async fn mock_recv(
        incoming: &Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
        outgoing: &Arc<Mutex<Vec<ServerFrame>>>,
        hang_up_after_done: Option<usize>,
    ) -> Result<Option<WireFrame<ClientFrame>>> {
        loop {
            if let Some(frame) = incoming.lock().await.pop_front() {
                return Ok(frame.map(WireFrame::control));
            }
            let Some(needed) = hang_up_after_done else {
                return Ok(None);
            };
            let done = outgoing
                .lock()
                .await
                .iter()
                .filter(|f| matches!(f.payload, ServerPayload::ResponseDone { .. }))
                .count();
            if done >= needed {
                return Ok(None);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
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
                    hang_up_after_done: None,
                },
                outgoing,
            )
        }

        /// A client that sends its frames and then stays connected until
        /// `turns` close-outs have come back — hanging up mid-turn asks
        /// the gateway to cancel the turn, which is the disconnect
        /// behaviour, not the conversation behaviour.
        fn with_frames_until_done(
            peer: PeerInfo,
            frames: Vec<Option<ClientFrame>>,
            turns: usize,
        ) -> (Self, Arc<Mutex<Vec<ServerFrame>>>) {
            let outgoing = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    peer,
                    incoming: Arc::new(Mutex::new(VecDeque::from(frames))),
                    outgoing: outgoing.clone(),
                    hang_up_after_done: Some(turns),
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
            mock_recv(&self.incoming, &self.outgoing, self.hang_up_after_done).await
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
                    outgoing: self.outgoing.clone(),
                    hang_up_after_done: self.hang_up_after_done,
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
            mock_recv(&self.incoming, &self.outgoing, self.hang_up_after_done).await
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

    /// A turn writes its start and stop markers into the thread's log, and
    /// the client sees the transition.
    ///
    /// The turn markers are the thread's status: an open marker is what
    /// clients render as "Streaming", the stop indicator is what returns
    /// the thread to "Ready" — and a turn that ends without one would load
    /// as still-open on the next start and be resumed. No model is
    /// configured, so the turn fails fast; the marker contract holds on
    /// every path, this one included.
    #[tokio::test]
    async fn a_turn_opens_and_closes_its_markers() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, _beta) = seed_two_threads(&cfg, "alpha", "beta")?;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "hello")],
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
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::ThreadsUpdate { threads, .. }
                    if threads.iter().any(|t| t.id == alpha.0
                        && t.status.as_deref() == Some("Streaming"))
            )),
            "the turn's start must broadcast the thread as Streaming"
        );

        // The stop indicator reached the log: the thread loads closed, so
        // the next gateway start has nothing to resume.
        let restored = rustyclaw_core::threads::ThreadStore::at_legacy_path(&threads_path)
            .load()
            .expect("the store should load");
        assert!(
            !restored.get(alpha).expect("thread exists").is_open(),
            "an ended turn must leave no open marker behind"
        );

        Ok(())
    }

    /// A turn interrupted by a dead gateway is resumed on the next start.
    ///
    /// A start marker with no stop indicator is exactly what a process
    /// leaves when it dies mid-answer. The thread loads back in as open,
    /// and the first connection restarts its turn from the recorded
    /// conversation — the user gets their answer (here: the no-model
    /// error) instead of a conversation that silently went quiet.
    #[tokio::test]
    async fn an_interrupted_turn_is_resumed_on_start() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;

        // A thread whose log ends inside a turn, stamped long before this
        // process started — the shape a crash leaves behind.
        let store = rustyclaw_core::threads::ThreadStore::at_legacy_path(&threads_path);
        let mut manager = rustyclaw_core::threads::ThreadManager::new();
        let interrupted = manager.create_chat("interrupted");
        manager.add_message(
            interrupted,
            rustyclaw_core::threads::MessageRole::User,
            "finish this thought",
        );
        store.persist(&mut manager).map_err(anyhow::Error::from)?;
        {
            use std::io::Write;
            let log = threads_path
                .parent()
                .unwrap()
                .join("threads")
                .join(format!("{}.log.jsonl", interrupted.0));
            let record =
                serde_json::to_string(&rustyclaw_core::threads::ThreadLogRecord::TurnStarted {
                    at: std::time::SystemTime::UNIX_EPOCH,
                })?;
            let mut f = std::fs::OpenOptions::new().append(true).open(&log)?;
            writeln!(f, "{record}")?;
        }

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
        // No client frames at all: the resume is the gateway's own doing.
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
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::Info { message, .. } if message.contains("Resuming")
            )),
            "the client is told the turn is being picked back up"
        );
        assert!(
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::ResponseDone { thread_id, .. }
                    if *thread_id == Some(interrupted.0)
            )),
            "the resumed turn runs to a close-out naming its thread"
        );
        let restored = rustyclaw_core::threads::ThreadStore::at_legacy_path(&threads_path)
            .load()
            .expect("the store should load");
        assert!(
            !restored.get(interrupted).expect("thread exists").is_open(),
            "the resumed turn's stop indicator closes the thread"
        );

        Ok(())
    }

    /// A crashed turn with nothing recorded is closed, not resumed — and
    /// not left open.
    ///
    /// A process can die between writing a turn's start marker and its
    /// first message. There is no conversation to resume, but an open
    /// marker with nothing left to close it would report the thread as
    /// "Streaming" forever, gating the composer on a reply that will
    /// never come. The next start writes the overdue stop indicator.
    #[tokio::test]
    async fn an_abandoned_open_marker_is_closed_on_start() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;

        // An empty thread whose log holds only an ancient start marker.
        let store = rustyclaw_core::threads::ThreadStore::at_legacy_path(&threads_path);
        let mut manager = rustyclaw_core::threads::ThreadManager::new();
        let stuck = manager.create_chat("stuck");
        store.persist(&mut manager).map_err(anyhow::Error::from)?;
        {
            use std::io::Write;
            let log = threads_path
                .parent()
                .unwrap()
                .join("threads")
                .join(format!("{}.log.jsonl", stuck.0));
            let record =
                serde_json::to_string(&rustyclaw_core::threads::ThreadLogRecord::TurnStarted {
                    at: std::time::SystemTime::UNIX_EPOCH,
                })?;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)?;
            writeln!(f, "{record}")?;
        }

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
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
            !frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::Info { message, .. } if message.contains("Resuming")
            )),
            "there is nothing to resume"
        );
        // The client is told, not just the disk: the initial list went out
        // before the sweep, so a refreshed one must follow with the thread
        // back at Ready — otherwise this client gates its composer on a
        // reply that will never come.
        let last_status = frames
            .iter()
            .rev()
            .find_map(|f| match &f.payload {
                ServerPayload::ThreadsUpdate { threads, .. } => threads
                    .iter()
                    .find(|t| t.id == stuck.0)
                    .map(|t| t.status.clone()),
                _ => None,
            })
            .expect("a ThreadsUpdate mentioning the thread");
        assert_eq!(
            last_status.as_deref(),
            Some("Ready"),
            "the client's final thread list must show the swept thread as Ready"
        );
        let restored = rustyclaw_core::threads::ThreadStore::at_legacy_path(&threads_path)
            .load()
            .expect("the store should load");
        assert!(
            !restored.get(stuck).expect("thread exists").is_open(),
            "the abandoned marker must be closed, not left Streaming forever"
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

    /// A turn that fails still ends, and its ending names its thread.
    ///
    /// The success path closes out through the turn's sink, which stamps the
    /// thread. The error path bypasses the sink — and used to send an Error
    /// frame and nothing else, leaving the thread marked in-flight in every
    /// client forever. "No model configured" is enough to reach it.
    #[tokio::test]
    async fn an_errored_turn_closes_out_naming_its_thread() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, _beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let chat = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "hello?")],
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
            frames
                .iter()
                .any(|f| matches!(f.frame_type, ServerFrameType::Error)),
            "No model is configured, so the turn must fail"
        );
        assert!(
            frames.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::ResponseDone { ok, thread_id }
                    if !*ok && *thread_id == Some(alpha.0)
            )),
            "The failure must close out the turn, naming its thread"
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

    // ── Content-routing tests: a scripted model, real turns ─────────────

    /// A scripted OpenAI-compatible model endpoint.
    ///
    /// Every reply is derived from the request that produced it: the last
    /// user message it was shown, and how many user messages the request
    /// carried. A reply filed in the wrong thread, or built from another
    /// conversation's history, is therefore visible in the transcript
    /// itself — no inspection of internals required. Speaks both the
    /// streaming (SSE) protocol the chat path uses and the plain-JSON
    /// protocol internal calls (compaction summaries) use.
    async fn spawn_mock_model() -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        fn content_text(value: &serde_json::Value) -> String {
            match value {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(parts) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(""),
                _ => String::new(),
            }
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock model");
        let addr = listener.local_addr().expect("mock model addr");
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 8192];
                    let header_end = loop {
                        let n = match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break pos + 4;
                        }
                        if buf.len() > 1_000_000 {
                            return;
                        }
                    };
                    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let content_length: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    while buf.len() < header_end + content_length {
                        let n = match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let body = &buf[header_end..(header_end + content_length).min(buf.len())];
                    let request: serde_json::Value =
                        serde_json::from_slice(body).unwrap_or_default();
                    let users: Vec<String> = request
                        .get("messages")
                        .and_then(|m| m.as_array())
                        .map(|msgs| {
                            msgs.iter()
                                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                                .filter_map(|m| m.get("content").map(content_text))
                                .collect()
                        })
                        .unwrap_or_default();
                    let text = format!(
                        "reply({}|u{})",
                        users.last().cloned().unwrap_or_default(),
                        users.len()
                    );
                    let streaming = request
                        .get("stream")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let payload = if streaming {
                        let chunk =
                            |delta: serde_json::Value, finish: serde_json::Value| -> String {
                                format!(
                                    "data: {}\n\n",
                                    serde_json::json!({
                                        "id": "mock", "object": "chat.completion.chunk",
                                        "created": 0, "model": "mock",
                                        "choices": [{"index": 0, "delta": delta,
                                                     "finish_reason": finish}]
                                    })
                                )
                            };
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{}{}{}data: [DONE]\n\n",
                            chunk(
                                serde_json::json!({"role": "assistant", "content": ""}),
                                serde_json::Value::Null
                            ),
                            chunk(
                                serde_json::json!({"content": text}),
                                serde_json::Value::Null
                            ),
                            chunk(
                                serde_json::json!({}),
                                serde_json::Value::String("stop".into())
                            ),
                        )
                    } else {
                        let body = serde_json::json!({
                            "id": "mock", "object": "chat.completion", "created": 0,
                            "model": "mock",
                            "choices": [{"index": 0,
                                         "message": {"role": "assistant", "content": text},
                                         "finish_reason": "stop"}],
                            "usage": {"prompt_tokens": 1, "completion_tokens": 1,
                                      "total_tokens": 2}
                        })
                        .to_string();
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                    };
                    let _ = sock.write_all(payload.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        addr
    }

    /// Run one connection against shared on-disk state, returning every
    /// frame the client was sent. `turns` is how many close-outs the
    /// client waits for before hanging up — pass the number of Chat
    /// frames, or 0 for connections that run no turns.
    async fn run_connection(
        cfg: &Config,
        model_ctx: &SharedModelCtx,
        frames: Vec<Option<ClientFrame>>,
        turns: usize,
    ) -> Result<Vec<ServerFrame>> {
        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
        let (mock_transport, outgoing) = if turns == 0 {
            MockTransport::with_frames(peer, frames)
        } else {
            MockTransport::with_frames_until_done(peer, frames, turns)
        };
        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();
        handle_transport_connection(
            Box::new(mock_transport),
            Arc::new(RwLock::new(cfg.clone())),
            model_ctx.clone(),
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
        let frames = outgoing.lock().await.clone();
        Ok(frames)
    }

    fn chat_frame(thread: u64, text: &str) -> ClientFrame {
        ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", text)],
                thread_id: Some(thread),
            },
        }
    }

    fn history_request(thread: u64) -> ClientFrame {
        ClientFrame {
            frame_type: ClientFrameType::ThreadHistoryRequest,
            payload: ClientPayload::ThreadHistoryRequest { thread_id: thread },
        }
    }

    /// Every `ThreadHistoryReply` in a frame stream, in arrival order, as
    /// `(thread, [(role, content)…])`.
    fn history_replies(frames: &[ServerFrame]) -> Vec<(u64, Vec<(String, String)>)> {
        frames
            .iter()
            .filter_map(|f| match &f.payload {
                ServerPayload::ThreadHistoryReply {
                    thread_id,
                    ok,
                    messages,
                    ..
                } => {
                    assert!(ok, "history reply for thread {thread_id} failed");
                    Some((
                        *thread_id,
                        messages
                            .iter()
                            .map(|m| (m.role.clone(), m.content.clone()))
                            .collect(),
                    ))
                }
                _ => None,
            })
            .collect()
    }

    /// Seed three threads: a plain chat, and two whose assistant turns ran
    /// tools — the shape of a thread anyone actually works in.
    fn seed_threads_with_tool_calls(
        cfg: &Config,
    ) -> Result<(
        rustyclaw_core::threads::ThreadId,
        rustyclaw_core::threads::ThreadId,
        rustyclaw_core::threads::ThreadId,
    )> {
        use rustyclaw_core::threads::MessageRole;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;
        let mut manager = rustyclaw_core::threads::ThreadManager::new();

        // Plain: no tool ever ran here.
        let plain = manager.create_chat("plain");
        manager.add_message(plain, MessageRole::User, "hello");
        manager.add_message(plain, MessageRole::Assistant, "hi there");
        manager.add_message(plain, MessageRole::User, "thanks");

        // Two threads that ran tools, as any real session does.
        let worked_a = manager.create_chat("worked-a");
        let worked_b = manager.create_chat("worked-b");
        for (id, tool) in [(worked_a, "read_file"), (worked_b, "bash")] {
            manager.add_message(id, MessageRole::User, "do the thing");
            if let Some(thread) = manager.get_mut(id) {
                thread.add_assistant_with_tool_calls(
                    String::new(),
                    serde_json::json!([{
                        "id": "call_1",
                        "name": tool,
                        "arguments": {"path": "src/main.rs"}
                    }]),
                );
                thread.add_tool_result("call_1", "ok");
            }
            manager.add_message(id, MessageRole::Assistant, "done");
        }

        manager.switch_foreground(plain);
        manager.save_to_file(&threads_path)?;
        Ok((plain, worked_a, worked_b))
    }

    /// Every transcript frame the gateway sends must survive the real codec.
    ///
    /// `MockTransport` records `ServerFrame`s directly and never encodes them,
    /// so a frame that builds correctly and cannot be *transmitted* passes
    /// every other test in this module. That gap is exactly how transcripts
    /// carrying tool calls came to be undeliverable: `tool_calls` was a
    /// `serde_json::Value`, frames are bincode, and `Value` decodes through
    /// `deserialize_any`, which bincode refuses. The gateway encoded and sent;
    /// the client could not read it and the thread opened empty.
    fn assert_frames_survive_the_wire(frames: &[ServerFrame]) {
        for frame in frames {
            let bytes = match rustyclaw_core::gateway::serialize_frame(frame) {
                Ok(bytes) => bytes,
                Err(e) => panic!("frame {:?} could not be encoded: {e}", frame.frame_type),
            };
            if let Err(e) = deserialize_frame::<ServerFrame>(&bytes) {
                panic!(
                    "frame {:?} encoded but could not be decoded by a client: {e}",
                    frame.frame_type
                );
            }
        }
    }

    /// Interleaved history fetches across several threads are all answered,
    /// and every answer is transmissible.
    ///
    /// Mirrors a real report: clicking between threads, one answered every
    /// time and two never were. Not timing — the failing pair failed between
    /// two successful fetches of the working one. The discriminator was
    /// content: the thread that worked had three plain messages, and the ones
    /// that never arrived had run tools.
    #[tokio::test]
    async fn interleaved_history_fetches_are_all_answered_and_transmissible() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (plain, worked_a, worked_b) = seed_threads_with_tool_calls(&cfg)?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        // The user's click order, both threads that ran tools asked for on
        // either side of the one that always worked.
        let frames = run_connection(
            &cfg,
            &model_ctx,
            vec![
                Some(history_request(worked_a.0)),
                Some(history_request(worked_b.0)),
                Some(history_request(plain.0)),
                Some(history_request(worked_b.0)),
                Some(history_request(worked_a.0)),
                None,
            ],
            0,
        )
        .await?;

        let replies = history_replies(&frames);
        let answered: Vec<u64> = replies.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            answered,
            vec![worked_a.0, worked_b.0, plain.0, worked_b.0, worked_a.0],
            "every request must be answered, in order asked"
        );

        // The reply must also be *deliverable*. Before the fix the tool-call
        // threads were answered here and still never reached the client.
        assert_frames_survive_the_wire(&frames);

        // And the tool calls must actually arrive, not merely survive as an
        // empty list — a transcript that drops them renders a turn that
        // silently did nothing.
        for (thread_id, messages) in &replies {
            if *thread_id == plain.0 {
                continue;
            }
            assert!(
                messages.iter().any(|(role, _)| role == "assistant"),
                "thread {thread_id} kept its assistant turn"
            );
        }
        Ok(())
    }

    /// A single fetch of a tool-running thread, reduced to the essentials.
    ///
    /// Kept separate from the interleaved case so a regression names itself:
    /// this one failing means transcripts with tool calls are undeliverable,
    /// independent of ordering or how many threads are in play.
    #[tokio::test]
    async fn a_thread_that_ran_tools_can_be_fetched_and_transmitted() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (_plain, worked_a, _worked_b) = seed_threads_with_tool_calls(&cfg)?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let frames = run_connection(
            &cfg,
            &model_ctx,
            vec![Some(history_request(worked_a.0)), None],
            0,
        )
        .await?;

        let replies = history_replies(&frames);
        assert_eq!(replies.len(), 1, "the fetch was answered");
        assert_frames_survive_the_wire(&frames);

        let (_, messages) = &replies[0];
        assert!(
            messages.iter().any(|(role, _)| role == "tool"),
            "the tool result is part of the transcript: {messages:?}"
        );
        Ok(())
    }

    /// The gateway returns the right messages, in the right threads, in
    /// the right order — whatever order the client asks in.
    ///
    /// Three real turns run against the scripted model: two in one thread,
    /// one in another. Each scripted reply names the user message it
    /// answers and how many user messages the model was shown, so a turn
    /// filed under the wrong thread, or a prompt assembled from another
    /// thread's conversation, changes the recorded text and fails the
    /// comparison. The histories are then fetched thread-B-first and
    /// thread-A-first, and must be identical either way.
    #[tokio::test]
    async fn histories_are_right_in_every_request_order() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let addr = spawn_mock_model().await;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(Some(Arc::new(
            rustyclaw_core::gateway::ModelContext {
                provider: "openai".to_string(),
                model: "mock-model".to_string(),
                base_url: format!("http://{addr}/v1"),
                api_key: Some("test-key".to_string()),
            },
        ))));

        // One connection per turn, so each turn completes (and persists)
        // before the next begins.
        let first = run_connection(
            &cfg,
            &model_ctx,
            vec![Some(chat_frame(alpha.0, "alpha one"))],
            1,
        )
        .await?;
        run_connection(
            &cfg,
            &model_ctx,
            vec![Some(chat_frame(beta.0, "beta one"))],
            1,
        )
        .await?;
        run_connection(
            &cfg,
            &model_ctx,
            vec![Some(chat_frame(alpha.0, "alpha two"))],
            1,
        )
        .await?;

        // The completion snapshot of the first turn already names its
        // thread and carries exactly that turn's exchange.
        let snapshot = first
            .iter()
            .rev()
            .find_map(|f| match &f.payload {
                ServerPayload::ThreadMessages {
                    thread_id,
                    messages,
                } if *thread_id == alpha.0 => Some(messages.clone()),
                _ => None,
            })
            .expect("the finished turn sends its thread's snapshot");
        assert_eq!(
            snapshot
                .iter()
                .map(|m| (m.role.as_str(), m.content.as_str()))
                .collect::<Vec<_>>(),
            vec![("user", "alpha one"), ("assistant", "reply(alpha one|u1)")],
        );

        let expected_alpha: Vec<(String, String)> = vec![
            ("user".into(), "alpha one".into()),
            ("assistant".into(), "reply(alpha one|u1)".into()),
            ("user".into(), "alpha two".into()),
            ("assistant".into(), "reply(alpha two|u2)".into()),
        ];
        let expected_beta: Vec<(String, String)> = vec![
            ("user".into(), "beta one".into()),
            ("assistant".into(), "reply(beta one|u1)".into()),
        ];

        for order in [[beta.0, alpha.0], [alpha.0, beta.0]] {
            let frames = run_connection(
                &cfg,
                &model_ctx,
                vec![
                    Some(history_request(order[0])),
                    Some(history_request(order[1])),
                    None,
                ],
                0,
            )
            .await?;
            let replies = history_replies(&frames);
            assert_eq!(replies.len(), 2, "one reply per request");
            assert_eq!(
                [replies[0].0, replies[1].0],
                order,
                "replies arrive in the order they were asked for"
            );
            for (thread, messages) in &replies {
                let want = if *thread == alpha.0 {
                    &expected_alpha
                } else {
                    &expected_beta
                };
                assert_eq!(
                    messages, want,
                    "thread {thread} must return its own messages, whole and in order \
                     (request order {order:?})"
                );
            }
        }

        Ok(())
    }

    /// Switching between threads shows each thread its own transcript,
    /// in either order.
    ///
    /// `ThreadSwitch` answers with the switched thread's `ThreadMessages`;
    /// switching B→A must show the same transcripts as A→B, and neither
    /// may leak the other conversation's scripted replies.
    #[tokio::test]
    async fn switch_order_does_not_change_what_each_thread_shows() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let addr = spawn_mock_model().await;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(Some(Arc::new(
            rustyclaw_core::gateway::ModelContext {
                provider: "openai".to_string(),
                model: "mock-model".to_string(),
                base_url: format!("http://{addr}/v1"),
                api_key: Some("test-key".to_string()),
            },
        ))));

        run_connection(
            &cfg,
            &model_ctx,
            vec![Some(chat_frame(alpha.0, "alpha one"))],
            1,
        )
        .await?;
        run_connection(
            &cfg,
            &model_ctx,
            vec![Some(chat_frame(beta.0, "beta one"))],
            1,
        )
        .await?;

        let switch = |thread: u64| ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch { thread_id: thread },
        };
        for order in [[beta.0, alpha.0], [alpha.0, beta.0]] {
            let frames = run_connection(
                &cfg,
                &model_ctx,
                vec![Some(switch(order[0])), Some(switch(order[1])), None],
                0,
            )
            .await?;
            // The snapshot that follows each switch belongs to the thread
            // that was switched to, and carries exactly its conversation.
            for thread in order {
                let messages = frames
                    .iter()
                    .filter_map(|f| match &f.payload {
                        ServerPayload::ThreadMessages {
                            thread_id,
                            messages,
                        } if *thread_id == thread => Some(messages.clone()),
                        _ => None,
                    })
                    .next_back()
                    .expect("each switch answers with that thread's snapshot");
                let label = if thread == alpha.0 { "alpha" } else { "beta" };
                assert_eq!(
                    messages
                        .iter()
                        .map(|m| (m.role.as_str(), m.content.to_string()))
                        .collect::<Vec<_>>(),
                    vec![
                        ("user", format!("{label} one")),
                        ("assistant", format!("reply({label} one|u1)")),
                    ],
                    "switch order {order:?}"
                );
            }
        }

        Ok(())
    }
}
