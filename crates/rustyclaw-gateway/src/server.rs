//! Gateway server engine.
//!
//! The core session loop: accepts transports, authenticates connections,
//! dispatches chat/messenger requests to model providers, and streams results
//! back. Driven by [`run_gateway`], which accepts both networked and
//! SSH-subsystem stdio transports. Invoked from the binary entry point in
//! `main.rs`.

use anyhow::{Context, Result};
use rustyclaw_core::ignore::Ignore;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

use rustyclaw_core::gateway::{
    ClientFrame, ClientFrameType, ClientPayload, ProbeResult, ServerFrame, ServerFrameType,
    ServerPayload, SessionOrigin, StatusType, WireFrame, deserialize_frame, protocol, transport,
};
use rustyclaw_core::providers as crate_providers;

use protocol::server::send_frame;

use crate::thread_updates::{
    send_projects_update, send_thread_messages_update_shared, send_threads_update_shared,
};
use crate::{
    SharedConfig, SharedCopilotSession, SharedModelCtx, SharedModelRegistry, SharedObserver,
    SharedSkillManager, SharedTaskManager, SharedVault, TOTP_LOCKOUT_SECS, ToolCancelFlag, admin,
    auth, concurrent, download_handler, plugin_handler, project_handler, providers, thread_handler,
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

/// Transfers already announced by this process, keyed by download id.
///
/// The download-wake counterpart to [`RESUMED_TURNS`], and there for the same
/// reason: a transfer belongs to an agent, an agent can be open in several
/// windows at once, and each window's connection watches the same broadcast
/// independently.
static ANNOUNCED_DOWNLOADS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = std::sync::LazyLock::new(Default::default);

/// Claim the right to announce a finished transfer. True for the first caller
/// and false for every one after it.
fn claim_download_announcement(id: &str) -> bool {
    ANNOUNCED_DOWNLOADS
        .lock()
        .expect("download announcement registry poisoned")
        .insert(id.to_string())
}

/// Drop claims for transfers the registry has forgotten.
///
/// Without this the set is the one thing here that grows for the life of the
/// process: `RESUMED_TURNS` is bounded by how many threads exist, but a claim
/// is minted per transfer and every download ever started would keep its
/// entry. Ids are unique for the life of the process, so a dropped claim can
/// never be re-minted by a later transfer — forgetting is safe exactly
/// because the id will not come back.
pub(crate) fn forget_download_announcements(ids: &[rustyclaw_core::downloads::DownloadId]) {
    if ids.is_empty() {
        return;
    }
    let mut claimed = ANNOUNCED_DOWNLOADS
        .lock()
        .expect("download announcement registry poisoned");
    for id in ids {
        claimed.remove(id);
    }
}

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
    /// What this turn's client is looking at, read live.
    ///
    /// The turn sends sidebar updates back, and their `foreground_id` tells
    /// the client which conversation to show. Cloning the *value* at spawn
    /// would send a stale one after a mid-turn switch and pull the user back
    /// to the thread they just left; the cell is shared with the connection
    /// loop, so what goes out is where they are now.
    foreground: crate::ForegroundCell,
    /// The connection this turn belongs to, so a download the turn starts can
    /// be routed back to the right client.
    connection_id: u64,
    /// The agent this turn belongs to. What a download the turn starts is
    /// owned by — the connection is not, because it can switch agents.
    agent_id: String,
    /// Where this turn's messages come from. Injected into the system prompt
    /// so the agent knows.
    session_origin: SessionOrigin,
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
    // Which conversation, on which connection, anything this turn starts in
    // the background belongs to. Scoped around the whole turn rather than
    // passed to `execute_tool`: the one tool that cares is `web_fetch`, and
    // the alternative is a parameter on every tool signature between here and
    // it.
    let origin = rustyclaw_core::downloads::DownloadOrigin {
        agent: deps.agent_id.clone(),
        connection: deps.connection_id,
        thread: turn_thread.map(|t| t.0),
    };
    let handle = tokio::spawn(rustyclaw_core::downloads::with_origin(origin, async move {
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
            deps.session_origin,
            &deps.foreground,
            is_resume,
        )
        .await;
        match result {
            // The turn already recorded its own assistant message; `None`
            // keeps the loop from adding a second copy.
            Ok(()) => sink.done(None).await,
            Err(e) => sink.error(format!("{e:#}")).await,
        }
    }));
    (handle, tool_cancel)
}

/// Re-offer the download completions that arrived while `thread` was busy.
///
/// They go back through the same channel the watcher uses rather than being
/// acted on here, so the decision to wake — and everything that hangs off it,
/// the thread's history, the turn ids, the stream ids — lives in one arm of
/// the loop instead of three.
///
/// Re-offered, not replayed blindly: the arm re-checks whether the thread is
/// busy, because the client's next message may already have started another
/// turn between the completion and this call.
fn requeue_deferred_wakes(
    deferred: &mut std::collections::HashMap<
        rustyclaw_core::threads::ThreadId,
        Vec<rustyclaw_core::downloads::Download>,
    >,
    thread: rustyclaw_core::threads::ThreadId,
    wake_tx: &tokio::sync::mpsc::UnboundedSender<rustyclaw_core::downloads::Download>,
) -> Result<()> {
    for download in deferred.remove(&thread).unwrap_or_default() {
        // The receiver is the loop making this call, so a failure here means
        // the loop is gone — in which case propagating is the right end.
        wake_tx
            .send(download)
            .context("re-offering a finished download to the connection loop")?;
    }
    Ok(())
}

/// Writes the stop indicators for a connection's turns, however it ends.
///
/// A turn's start marker is written to the thread's log the moment the turn
/// begins, and its stop indicator when the turn ends. A connection that goes
/// away with turns still running has to write those stop indicators itself:
/// the turns are aborted with it and no completion will ever drain for them.
/// Left open, the thread reports as "Streaming" with nothing remaining that
/// could ever close it — the composer gated on a reply that is not coming.
/// The start-up sweep is no help either, because it only touches markers
/// older than the running process, so a live daemon carries the stuck thread
/// for as long as it runs.
///
/// The handler writes to its client on nearly every line and any of those
/// writes can fail, so "however it ends" is the hard part. Cleanup at the
/// bottom of the function runs only when control reaches the bottom, and a
/// `?` anywhere above it — or a panic — skipped it. Holding the close-out in
/// a guard hands that to the scope instead of to whoever edits the function
/// next: it cannot be skipped by an early return, and it cannot be forgotten
/// by an edit that adds one.
///
/// The work is async and `Drop` is not, which is why the guard has two
/// halves. [`close_out`](Self::close_out) is the ordinary path — awaited
/// where the connection ends, it finishes before the handler returns, so
/// what is on disk afterwards is settled. `Drop` is the backstop for the
/// paths that never reach it and can only spawn the work, which is the whole
/// reason the ordinary path is still called explicitly.
struct TurnCloseout {
    /// The turns to close. Holding this also keeps the registry alive to be
    /// read here, since dropping it is what aborts the turns.
    active: Arc<Mutex<concurrent::ActiveTasks>>,
    /// Read at close-out rather than captured, because a connection can
    /// switch agents and the turns to close belong to the one it ended on.
    store: crate::agent_handler::StoreCell,
    done: bool,
}

impl TurnCloseout {
    fn new(
        active: Arc<Mutex<concurrent::ActiveTasks>>,
        store: crate::agent_handler::StoreCell,
    ) -> Self {
        Self {
            active,
            store,
            done: false,
        }
    }

    /// Close out the connection's turns and persist. Idempotent.
    async fn close_out(&mut self) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        Self::run(self.active.clone(), self.store.clone()).await;
    }

    /// The close-out proper, owning its handles so `Drop` can spawn it.
    async fn run(
        active: Arc<Mutex<concurrent::ActiveTasks>>,
        store: crate::agent_handler::StoreCell,
    ) {
        let store = store
            .read()
            .expect("connection store cell poisoned")
            .clone();
        let running = active.lock().await.running_threads();
        if !running.is_empty() {
            let mut tm = store.thread_mgr.lock().await;
            for thread in running {
                // Cancelled, which is what happened: the connection went
                // away mid-turn. The log keeps that visible rather than
                // claiming an answer that was never given.
                tm.end_turn(thread, false);
            }
        }
        // The last write of the session, carrying everything said during it.
        // Through the focused variant: the thread this client was looking at
        // lives in a per-connection cell, and the store's own pointer is how
        // the next window finds its way back to it.
        crate::helpers::persist_threads_focused(
            &store.thread_mgr,
            &store.threads_path,
            crate::foreground_of(&store.foreground),
        )
        .await;
    }
}

impl Drop for TurnCloseout {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // Spawned, because `Drop` cannot await. This is the path an early
        // return or a panic takes, so it is worth saying out loud: reaching
        // it means the connection ended somewhere that did not expect to be
        // the end.
        warn!("Connection ended without closing out its turns; doing it now");
        // Asked for rather than assumed: `tokio::spawn` panics with no
        // runtime, and a panic in `Drop` during a panicking unwind aborts the
        // process. Losing the close-out costs a thread stuck at "Streaming"
        // until the next start sweeps it; taking the daemon down costs every
        // other connection.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            error!("No runtime left to close out this connection's turns");
            return;
        };
        let (active, store) = (self.active.clone(), self.store.clone());
        handle.spawn(async move { Self::run(active, store).await });
    }
}

/// How long the connection loop waits for room in the outbound queue before
/// treating the client as gone. Generous — a client that is reading at all
/// drains 256 queued frames in milliseconds — so only a transport that has
/// genuinely stopped reaches it.
const OUTBOUND_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

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
    // Whether this connection is local (loopback or unknown peer) or remote.
    // Combined with the client's declared kind on each Chat frame, this is
    // the "origin" the agent sees in its system prompt.
    let remote_peer = match peer_info.addr {
        Some(addr) => !addr.ip().is_loopback(),
        None => false,
    };

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
    // Open where this agent was last left. From here the pointer is this
    // connection's alone — see `AgentSession::foreground_id`.
    agent_session.restore_foreground().await;
    // Where the connection's thread state currently lives, for the tasks that
    // outlive any one agent — see `ConnectionStore`.
    let store_cell: crate::agent_handler::StoreCell =
        Arc::new(std::sync::RwLock::new(agent_session.store()));

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
        agent_session.foreground_id(),
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
    let http = rustyclaw_core::providers::http_client();

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
    // Built here, before the resume below can open the connection's first
    // turn marker: from this line on, every way out of this function closes
    // the connection's turns. See `TurnCloseout`.
    let mut closeout = TurnCloseout::new(active_tasks.clone(), store_cell.clone());
    // The reader delivers answers; the turns claim the ids. Both sides hold
    // the same registries.
    let reader_approvals = approvals.clone();
    let reader_user_prompts = user_prompts.clone();
    let reader_credentials = credentials.clone();
    let reader_dom_queries = dom_queries.clone();
    // ── Download completions ───────────────────────────────────────
    //
    // The download registry is process-global and its broadcast carries every
    // transfer's every change. This connection wants two different slices of
    // that, and the watcher below splits them: the panel redraws on every
    // change to a transfer this connection started, while the agent is woken
    // only where one ended. Waking on progress would start a turn every
    // quarter-megabyte; redrawing only on completion would be a progress bar
    // that never moves.
    let connection_id = rustyclaw_core::downloads::next_connection_id();
    // The panel side. A tick rather than the changed record: the update sends
    // the whole list, so what arrived matters less than that something did,
    // and a burst of progress on several transfers collapses into one redraw.
    let (panel_tx, mut panel_rx) = tokio::sync::mpsc::channel::<()>(1);
    // Unbounded on purpose. The loop is the only receiver, and the loop sends
    // into it too — deferring a completion that arrived while the thread was
    // busy, then re-offering it once the turn ends. A bounded channel would
    // let that self-send block on a receiver that is the sender's own caller.
    let (wake_tx, mut wake_rx) =
        tokio::sync::mpsc::unbounded_channel::<rustyclaw_core::downloads::Download>();
    // Which agent this connection is currently showing. A connection can
    // switch agents wholesale, and the watcher below outlives any one of them,
    // so it cannot capture an agent id — it has to read the current one per
    // event. Mirrors `store_cell`, which exists for the same reason.
    let current_agent = Arc::new(std::sync::RwLock::new(agent_session.agent_id.clone()));
    let watcher_agent = current_agent.clone();
    let watcher_wake_tx = wake_tx.clone();
    let watcher_panel_tx = panel_tx.clone();
    // Its own token, not the connection's. The connection's is a child of the
    // gateway-wide shutdown token and is never cancelled when a single client
    // leaves, and the watcher's other two exits — a closed panel or wake
    // channel — are both behind `belongs_to(connection_id)`, which nothing can
    // satisfy once this connection is gone. So without a token of its own the
    // task parks on `recv()` for the life of the process, one per client that
    // has ever connected, waking for every other connection's progress.
    let watcher_cancel = cancel.child_token();
    // A drop guard rather than a cancel at teardown: the loop below returns
    // early on any write failure via `?`, and those paths would skip it.
    let _watcher_guard = watcher_cancel.clone().drop_guard();
    tokio::spawn(async move {
        let mut events = rustyclaw_core::downloads::subscribe();
        loop {
            let event = tokio::select! {
                _ = watcher_cancel.cancelled() => break,
                event = events.recv() => event,
            };
            match event {
                Ok(rustyclaw_core::downloads::DownloadEvent::Changed(download)) => {
                    // Read per event, never captured: between two events this
                    // connection may have switched to a different agent, and
                    // the previous agent's transfers are no longer its to see.
                    let mine = {
                        let agent = watcher_agent.read().expect("current agent cell poisoned");
                        download.belongs_to(&agent)
                    };
                    if !mine {
                        continue;
                    }
                    let terminal = download.status.is_terminal();
                    // A full channel already holds an unread tick, and the
                    // update it triggers will read the list *after* this
                    // change — so dropping this one loses nothing. That is
                    // what keeps a fast transfer from queueing a redraw per
                    // chunk behind a slow writer.
                    match watcher_panel_tx.try_send(()) {
                        Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(())) => {}
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => break,
                    }
                    if !terminal {
                        continue;
                    }
                    // The receiver is the connection loop; once it is gone
                    // there is no agent left to wake.
                    if watcher_wake_tx.send(download).is_err() {
                        break;
                    }
                }
                Ok(rustyclaw_core::downloads::DownloadEvent::Removed { agent, .. }) => {
                    // A panel tick and nothing else. The agent is never woken
                    // for a removal: forgetting a finished transfer is the
                    // user tidying their list, not news worth a turn.
                    let mine = {
                        let current = watcher_agent.read().expect("current agent cell poisoned");
                        *current == agent
                    };
                    if !mine {
                        continue;
                    }
                    match watcher_panel_tx.try_send(()) {
                        Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Full(())) => {}
                        Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => break,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    // Said out loud rather than swallowed: each missed event
                    // may have been a completion, and a completion dropped
                    // here is a file the agent is never told about. Nothing
                    // can reconstruct it — the registry still holds the
                    // record, but the edge is gone.
                    warn!(
                        missed,
                        "Download events were dropped; some completions will not wake the agent"
                    );
                }
                // The sender is a process-lifetime static, so this is
                // unreachable in practice — but a watcher that spun on a
                // closed channel would busy-loop a core.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    // Completions that landed while their thread had a turn running. Waking
    // then would displace that turn — aborting the user's own request to
    // announce a file — so they wait here and are re-offered when the turn
    // ends. Keyed by thread because that is what has to go idle.
    let mut deferred_wakes: std::collections::HashMap<
        rustyclaw_core::threads::ThreadId,
        Vec<rustyclaw_core::downloads::Download>,
    > = std::collections::HashMap::new();
    // Cleared only if the corresponding channel is somehow closed, which this
    // scope's own senders make impossible; see the arms that read them.
    let mut wakes_open = true;
    let mut panel_open = true;

    // Counter for turn ids, so a turn's completion cannot retire the turn
    // that replaced it.
    let mut next_turn_id: u64 = 0;
    // Server-initiated turns (resumed ones) run on even stream ids;
    // clients allocate odd ones, so the two can never collide.
    let mut next_server_stream_id: u64 = 0;

    // ── Send initial thread list ───────────────────────────────────
    // Freshly-connected clients need to know the current thread state.
    if let Err(e) = send_threads_update_shared(
        &mut *writer,
        &agent_session.thread_mgr,
        &task_mgr,
        None,
        agent_session.foreground_id(),
    )
    .await
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
            send_threads_update_shared(
                &mut *writer,
                &agent_session.thread_mgr,
                &task_mgr,
                None,
                agent_session.foreground_id(),
            )
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
            send_threads_update_shared(
                &mut *writer,
                &agent_session.thread_mgr,
                &task_mgr,
                None,
                agent_session.foreground_id(),
            )
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
                    foreground: agent_session.foreground.clone(),
                    connection_id,
                    agent_id: agent_session.agent_id.clone(),
                    session_origin: if remote_peer {
                        SessionOrigin::Remote
                    } else {
                        SessionOrigin::Local
                    },
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
    // Bounded wait, because this handle belongs to the loop below — and that
    // loop is the only reader of the inbound frame channel. Waiting here
    // indefinitely stops it reading, which stops the reader task forwarding,
    // which is what would keep this queue full: the ring described on
    // `with_patience`. A queue that has not moved in this long is a client
    // that has stopped reading, and the connection is better ended loudly
    // than left open and mute.
    let mut writer: Box<dyn transport::TransportWriter> = Box::new(
        rustyclaw_core::gateway::QueuedWriter::with_patience(out_tx.clone(), OUTBOUND_PATIENCE),
    );

    let reader_cancel = cancel.clone();
    // Deliberately a cell rather than a handle: switching agents replaces the
    // session wholesale, thread store included, and the reader answers history
    // for the life of the connection. Keeping the store it saw at connect
    // would answer from the previous agent after a switch — and ids restart
    // low in each store, so the likely result is not an empty transcript but
    // another agent's conversation under a plausible-looking id.
    let reader_store = store_cell.clone();
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
                                    let thread_mgr_now = reader_store
                                        .read()
                                        .expect("connection store cell poisoned")
                                        .thread_mgr
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

    // Main message handling loop — receives from channel.
    //
    // Captured rather than propagated. Every `?` below is a write to the
    // client, and a client that has stopped reading makes those fail — the
    // ordinary way this connection ends, not an exceptional one. Closing the
    // turn markers no longer depends on reaching the bottom of this function
    // (`TurnCloseout` is what guarantees that), but two things still want the
    // tidy path: the frames already queued get drained on the way out rather
    // than dropped, and the close-out is *awaited* here instead of being left
    // to the guard's spawn, so a caller that looks at the store afterwards
    // sees a settled one. The connection still ends on the error.
    let loop_result: Result<()> = async {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    // Dropping a JoinHandle detaches the task; a turn left
                    // running would keep model calls going and then block
                    // forever on a frame channel nobody drains.
                    active_tasks.lock().await.abort_all();
                    writer.close().await.ignore();
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
                                ClientPayload::Chat {
                                    messages,
                                    thread_id,
                                    client_kind,
                                } => {
                                    // The origin the agent will see: remote
                                    // connections report as Remote (the UI
                                    // kind is not visible from here), local
                                    // ones as the client's declared kind when
                                    // it sent one, else Local.
                                    let session_origin = if remote_peer {
                                        SessionOrigin::Remote
                                    } else {
                                        match client_kind {
                                            Some(SessionOrigin::Unknown) | None => {
                                                SessionOrigin::Local
                                            }
                                            Some(kind) => kind,
                                        }
                                    };
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
                                        let adopted = agent_session.switch_foreground(want).await;
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
                                                agent_session.foreground_id(),
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
                                        let tm = agent_session.thread_mgr.lock().await;
                                        messages
                                            .iter()
                                            .rev()
                                            .find(|m| m.role == "user")
                                            .and_then(|last| {
                                                // Exempt where the message was
                                                // typed, which is this client's
                                                // thread — not whatever the
                                                // shared manager last pointed at.
                                                tm.find_best_match(
                                                    &last.content,
                                                    agent_session.foreground_id(),
                                                )
                                            })
                                            .and_then(|better| {
                                                tm.get(better).map(|t| {
                                                    (better, t.compact_summary.clone())
                                                })
                                            })
                                    };
                                    if let Some((better, context_summary)) = auto_switch {
                                        // This connection follows its own guess;
                                        // the other windows on this agent keep
                                        // looking at whatever they were.
                                        agent_session.switch_foreground(better).await;
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
                                            agent_session.foreground_id(),
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
                                        None => agent_session.ensure_foreground().await,
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
                                    //
                                    // Read from `turn_thread`, the thread just
                                    // settled on, rather than from a foreground
                                    // looked up again: they are the same here,
                                    // and asking twice is how they come to
                                    // disagree.
                                    if let Some(pid) = {
                                        let tm = agent_session.thread_mgr.lock().await;
                                        turn_thread
                                            .and_then(|id| tm.get(id))
                                            .map(|t| t.project_id)
                                    } {
                                        if pid != agent_session.project_mgr.active_id() {
                                            project_handler::activate_project(
                                                &mut *writer,
                                                &mut config,
                                                &mut agent_session.project_mgr,
                                                &agent_session.thread_mgr,
                                                &agent_session.projects_path,
                                                pid,
                                                turn_thread,
                                            )
                                            .await?;
                                        } else {
                                            // Synchronous, so the guard in the
                                            // argument list never spans an await.
                                            project_handler::repoint_workspace(
                                                &mut config,
                                                &agent_session.project_mgr,
                                                &*agent_session.thread_mgr.lock().await,
                                                turn_thread,
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
                                        agent_session.foreground_id(),
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
                                                foreground: agent_session.foreground.clone(),
                                                connection_id,
                                                agent_id: agent_session.agent_id.clone(),
                                                session_origin,
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
                                ClientPayload::DownloadsRequest => {
                                    download_handler::send_downloads_update(&mut *writer, &agent_session.agent_id).await?;
                                }
                                ClientPayload::DownloadCancel { id } => {
                                    download_handler::handle_download_cancel(&mut *writer, &agent_session.agent_id, &id).await?;
                                }
                                ClientPayload::DownloadsClearFinished => {
                                    download_handler::handle_downloads_clear_finished(&mut *writer, &agent_session.agent_id).await?;
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
                                    let foreground = agent_session.foreground_id();
                                    if pid != agent_session.project_mgr.active_id() {
                                        project_handler::activate_project(
                                            &mut *writer,
                                            &mut config,
                                            &mut agent_session.project_mgr,
                                            &agent_session.thread_mgr,
                                            &agent_session.projects_path,
                                            pid,
                                            foreground,
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
                                        &agent_session.foreground,
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
                                        &agent_session.foreground,
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
                                    let foreground = agent_session.foreground_id();
                                    let foreground_project = {
                                        let tm = agent_session.thread_mgr.lock().await;
                                        foreground.and_then(|id| tm.get(id)).map(|t| t.project_id)
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
                                                foreground,
                                            )
                                            .await?;
                                        } else {
                                            project_handler::repoint_workspace(
                                                &mut config,
                                                &agent_session.project_mgr,
                                                &*agent_session.thread_mgr.lock().await,
                                                foreground,
                                            );
                                        }
                                    }
                                }
                                ClientPayload::ThreadList => {
                                    thread_handler::handle_thread_list(&mut *writer, &agent_session.thread_mgr, &task_mgr, agent_session.foreground_id()).await?;
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
                                        &agent_session.foreground,
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
                                        agent_session.foreground_id(),
                                    )
                                    .await?;
                                }
                                ClientPayload::ThreadPin { thread_id, pinned } => {
                                    thread_handler::handle_thread_pin(
                                        &mut *writer,
                                        &agent_session.thread_mgr,
                                        &task_mgr,
                                        &agent_session.threads_path,
                                        thread_id,
                                        pinned,
                                        agent_session.foreground_id(),
                                    )
                                    .await?;
                                }
                                ClientPayload::ThreadMove { thread_id, project_id } => {
                                    thread_handler::handle_thread_move(
                                        &mut *writer,
                                        &mut config,
                                        &agent_session.thread_mgr,
                                        &agent_session.project_mgr,
                                        &task_mgr,
                                        &agent_session.threads_path,
                                        thread_id,
                                        project_id,
                                        agent_session.foreground_id(),
                                    )
                                    .await?;
                                }
                                ClientPayload::ThreadExport { thread_id } => {
                                    thread_handler::handle_thread_export(
                                        &mut *writer,
                                        &agent_session.thread_mgr,
                                        &agent_session.project_mgr,
                                        thread_id,
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
                                        agent_session.foreground_id(),
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
                                ClientPayload::ProjectList => {
                                    project_handler::handle_project_list(&mut *writer, &agent_session.project_mgr).await?;
                                }
                                ClientPayload::ProjectCreate { name, path } => {
                                    let foreground = agent_session.foreground_id();
                                    project_handler::handle_project_create(
                                        &mut *writer,
                                        &mut config,
                                        &mut agent_session.project_mgr,
                                        &agent_session.thread_mgr,
                                        &agent_session.projects_path,
                                        name,
                                        path,
                                        foreground,
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
                                ClientPayload::ProjectPin { project_id, pinned } => {
                                    project_handler::handle_project_pin(
                                        &mut *writer,
                                        &mut agent_session.project_mgr,
                                        &agent_session.projects_path,
                                        project_id,
                                        pinned,
                                    )
                                    .await?;
                                }
                                ClientPayload::ProjectUpdate { project_id, name, path } => {
                                    let foreground = agent_session.foreground_id();
                                    project_handler::handle_project_update(
                                        &mut *writer,
                                        &mut config,
                                        &mut agent_session.project_mgr,
                                        &agent_session.thread_mgr,
                                        &agent_session.projects_path,
                                        project_id,
                                        name,
                                        path,
                                        foreground,
                                    )
                                    .await?;
                                }
                                ClientPayload::ProjectDelete { project_id } => {
                                    let foreground = agent_session.foreground_id();
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
                                        foreground,
                                    )
                                    .await?;
                                    crate::helpers::persist_threads(&mut *agent_session.thread_mgr.lock().await, &agent_session.threads_path);
                                    send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None, agent_session.foreground_id()).await?;
                                }
                                ClientPayload::ProjectSwitch { project_id } => {
                                    let foreground = agent_session.foreground_id();
                                    project_handler::handle_project_switch(
                                        &mut *writer,
                                        &mut config,
                                        &mut agent_session.project_mgr,
                                        &agent_session.thread_mgr,
                                        &agent_session.projects_path,
                                        project_id,
                                        foreground,
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
                                        &store_cell,
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
                                        // The downloads watcher reads this per
                                        // event; until it moves, the panel would go
                                        // on showing the previous agent's URLs and
                                        // destination paths.
                                        *current_agent
                                            .write()
                                            .expect("current agent cell poisoned") =
                                            agent_session.agent_id.clone();
                                        // The panel is stale the moment the agent
                                        // changes, so correct it rather than
                                        // waiting for the next transfer event.
                                        download_handler::send_downloads_update(
                                            &mut *writer,
                                            &agent_session.agent_id,
                                        )
                                        .await?;
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
                // Both of these carry the same `drain_deadline` guard as the
                // frame arm above, and for a sharper reason than "the client has
                // left". A wake taken while draining consumes the process-wide
                // announcement claim, which is never released, and then has its
                // freshly spawned turn aborted by the drain arm moments later —
                // so every other window on this agent is told the transfer is
                // already someone else's to announce, and it is announced
                // nowhere. Leaving it unclaimed lets a connection that can still
                // serve it do so. The panel arm is the milder case: frames
                // written to a client that has gone away.
                tick = panel_rx.recv(), if panel_open && drain_deadline.is_none() => {
                    match tick {
                        Some(()) => {
                            download_handler::send_downloads_update(&mut *writer, &agent_session.agent_id).await?;
                        }
                        None => {
                            // Unreachable for the same reason as the wake channel
                            // below, and disabled for the same reason: `recv` on a
                            // closed channel returns instantly, so an arm that
                            // ignored this would spin a core.
                            error!("Download event channel closed; the downloads panel will not update");
                            panel_open = false;
                        }
                    }
                }
                finished = wake_rx.recv(), if wakes_open && drain_deadline.is_none() => {
                    let Some(download) = finished else {
                        // Unreachable: this scope holds a sender for the life of
                        // the loop. Disabling the arm rather than looping is the
                        // difference between a lost feature and a spun core,
                        // because `recv` on a closed channel returns instantly.
                        error!("Download completion channel closed; completions will no longer wake the agent");
                        wakes_open = false;
                        continue;
                    };
                    // The thread id is only meaningful inside the store it was
                    // minted in, and this connection may have switched agents
                    // while the bytes were arriving. Ids restart low in every
                    // agent's store, so resolving it against the wrong one does
                    // not fail — it lands on an unrelated conversation, files a
                    // notice there and spawns a turn on it. Checked before the
                    // thread id is read, not after.
                    if !download.belongs_to(&agent_session.agent_id) {
                        debug!(
                            download = %download.id,
                            "Download finished for an agent this connection is no longer showing; not announcing it"
                        );
                        continue;
                    }
                    // A transfer started outside any conversation — the CLI's
                    // one-shot paths — has no transcript to be announced in.
                    let Some(thread) = download
                        .origin
                        .as_ref()
                        .and_then(|o| o.thread)
                        .map(rustyclaw_core::threads::ThreadId)
                    else {
                        debug!(download = %download.id, "Download finished outside a conversation; nothing to notify");
                        continue;
                    };
                    // Waking a thread that is mid-turn would *displace* that turn
                    // — the Chat arm's rule is one turn per thread, and the loser
                    // is aborted at its next await. Announcing a file by killing
                    // the request the user is waiting on is not a trade worth
                    // making, so it waits: the Done and Error arms re-offer
                    // whatever is parked here once the thread goes idle.
                    if active_tasks.lock().await.running_threads().contains(&thread) {
                        debug!(
                            download = %download.id,
                            thread = thread.0,
                            "Download finished while the thread was busy; deferring the wake"
                        );
                        deferred_wakes.entry(thread).or_default().push(download);
                        continue;
                    }
                    // Exactly one connection may announce a given transfer.
                    //
                    // Ownership is by agent, and an agent can be open in more than
                    // one window — a second desktop window, or a TUI alongside the
                    // app, both defaulting to `main`. Every such connection has its
                    // own watcher over the same process-global broadcast and its
                    // own `active_tasks`, so all of them pass the filter above and
                    // none of them can see that another has already started a turn.
                    // Without this the notice is appended once per window and the
                    // second turn displaces the first — aborting the reply the
                    // first had just begun.
                    //
                    // Claimed here rather than at deferral: a connection that parks
                    // a wake may go away while holding it, and the transfer should
                    // still be announced by whoever is idle. Keyed by transfer id
                    // alone, which is already unique for the life of the process.
                    if !claim_download_announcement(&download.id) {
                        debug!(
                            download = %download.id,
                            "Another connection is announcing this transfer; skipping"
                        );
                        continue;
                    }
                    let notice = download.summary();
                    // The conversation can have been deleted while the bytes were
                    // arriving. Filing the notice anywhere else would put it in a
                    // transcript that never asked for the file.
                    let history = {
                        let mut tm = agent_session.thread_mgr.lock().await;
                        match tm.get_mut(thread) {
                            Some(t) => {
                                // Recorded as the user's turn rather than a
                                // system message: a system message part-way
                                // through a conversation is rejected outright by
                                // some providers, and this has to reach every one
                                // of them. The wording is what marks it as the
                                // environment speaking, not the person.
                                t.add_message(rustyclaw_core::threads::MessageRole::User, &notice);
                                tm.begin_turn(thread);
                                crate::helpers::persist_threads(&mut tm, &agent_session.threads_path);
                                tm.get(thread).map(crate::thread_updates::thread_history_messages)
                            }
                            None => None,
                        }
                    };
                    let Some(messages) = history else {
                        info!(
                            download = %download.id,
                            thread = thread.0,
                            "Download finished but its conversation is gone; not announcing it"
                        );
                        continue;
                    };
                    send_thread_messages_update_shared(&mut *writer, thread, &agent_session.thread_mgr).await?;
                    send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None, agent_session.foreground_id()).await?;

                    next_turn_id += 1;
                    let turn_id = next_turn_id;
                    // Even ids: server-initiated, like a resumed turn. A client
                    // allocates odd ones, so the two can never collide.
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
                            foreground: agent_session.foreground.clone(),
                            connection_id,
                            agent_id: agent_session.agent_id.clone(),
                            session_origin: if remote_peer {
                                SessionOrigin::Remote
                            } else {
                                SessionOrigin::Local
                            },
                        },
                        messages,
                        stream_id,
                        turn_id,
                        Some(thread),
                        model_task_tx.clone(),
                        // The notice is already in the thread's log — recorded a
                        // few lines up, before the history was read back. This is
                        // the resume shape, not the chat one: the turn replays a
                        // transcript that already ends with its own last user
                        // message, so it must not append it a second time.
                        true,
                    );
                    active_tasks
                        .lock()
                        .await
                        .register(thread, turn_id, stream_id, handle, tool_cancel);
                }
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
                                requeue_deferred_wakes(&mut deferred_wakes, thread_id, &wake_tx)?;

                                // Send updated thread list (status may have changed)
                                send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None, agent_session.foreground_id()).await?;

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
                                // A turn that failed still frees the thread, and
                                // a download that finished behind it is still
                                // worth saying. Nothing about the failure makes
                                // the file less arrived.
                                requeue_deferred_wakes(&mut deferred_wakes, thread_id, &wake_tx)?;
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
                                send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None, agent_session.foreground_id()).await?;

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
                            // Healed first: one of these events is another
                            // window closing the thread this one is in, and
                            // reporting the pointer as it stands would tell the
                            // client it has nothing open.
                            let foreground = agent_session.heal_foreground().await;
                            send_threads_update_shared(&mut *writer, &agent_session.thread_mgr, &task_mgr, None, foreground).await?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .await;

    // Clean up reader task
    reader_handle.abort();
    // The writer task ends on its own once every queue handle is dropped, but
    // the reader task holds one and has just been aborted — so drop this
    // function's handles and let the drain finish before the connection goes.
    // Aborting it instead would discard frames already queued, including the
    // close-out written just above.
    drop(writer);
    drop(out_tx);
    tokio::time::timeout(std::time::Duration::from_secs(5), writer_handle)
        .await
        .ignore();

    // The ordinary end of the connection: awaited here so what is on disk is
    // settled before this returns, rather than left to the guard's spawn. A
    // process that crashes reaches neither — that is the one case that leaves
    // markers open, and exactly the one the resume path above exists for.
    closeout.close_out().await;

    // Said out loud, and only now that the cleanup above has run. A
    // connection that ends this way ended because it could no longer reach
    // its client, and the log is the only place that fact exists — the
    // client, by definition, did not receive it.
    if let Err(e) = &loop_result {
        warn!(error = %e, "Connection ended on an error; its turns were closed out");
    }
    loop_result
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

    /// An agent can be open in several windows, and each window's connection
    /// watches the same process-global broadcast with its own `active_tasks`.
    /// Without a shared claim every one of them announces the same transfer:
    /// the notice is appended once per window, and the second turn displaces
    /// the first — killing the reply it had just started.
    #[test]
    fn only_one_connection_announces_a_given_transfer() {
        // Ids are unique for the life of the process, and this set is global,
        // so the test names its own rather than reusing another test's.
        let id = "dl-claim-test-only-one";
        assert!(
            claim_download_announcement(id),
            "the first connection to reach an idle thread announces it"
        );
        assert!(
            !claim_download_announcement(id),
            "a second window on the same agent must not announce it again"
        );
        // A different transfer is unaffected — the claim is per transfer, not
        // a latch that silences everything after the first.
        assert!(claim_download_announcement("dl-claim-test-another"));
    }

    /// The claim set is the one structure here that would otherwise grow for
    /// the life of the process — `RESUMED_TURNS` is bounded by how many
    /// threads exist, but a claim is minted per transfer. Clearing a finished
    /// transfer is the moment its id stops meaning anything, so it is the
    /// moment to let go of the claim.
    #[test]
    fn forgetting_a_transfer_releases_its_claim() {
        let id = "dl-forget-test".to_string();
        assert!(claim_download_announcement(&id));
        assert!(!claim_download_announcement(&id), "claimed while it exists");

        forget_download_announcements(std::slice::from_ref(&id));

        // Re-claimable only because ids are never reused: this proves the
        // entry is gone rather than that a later transfer could steal it.
        assert!(
            claim_download_announcement(&id),
            "a forgotten transfer should leave nothing behind"
        );
        forget_download_announcements(std::slice::from_ref(&id));

        // An empty list is a no-op rather than a lock round-trip, since a
        // clear that removed nothing is the common case.
        forget_download_announcements(&[]);
    }

    /// A finished transfer parked under `thread`, for the deferral tests.
    fn parked(
        id: &str,
        thread: u64,
    ) -> (
        rustyclaw_core::threads::ThreadId,
        rustyclaw_core::downloads::Download,
    ) {
        let mut mgr = rustyclaw_core::downloads::DownloadManager::new();
        let registered = mgr.register(
            "https://e/a".into(),
            std::path::PathBuf::from("/tmp/a"),
            None,
            Some(rustyclaw_core::downloads::DownloadOrigin {
                agent: "researcher".into(),
                connection: 1,
                thread: Some(thread),
            }),
        );
        let mut download = mgr
            .finish(
                &registered.id,
                rustyclaw_core::downloads::DownloadStatus::Complete,
            )
            .expect("first ending");
        download.id = id.to_string();
        (rustyclaw_core::threads::ThreadId(thread), download)
    }

    /// A completion parked behind a running turn is offered again when that
    /// turn ends, rather than being dropped to avoid displacing it.
    #[test]
    fn a_deferred_completion_is_re_offered_when_its_thread_goes_idle() {
        let (thread, download) = parked("dl_1", 7);
        let mut deferred = std::collections::HashMap::new();
        deferred.insert(thread, vec![download]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        requeue_deferred_wakes(&mut deferred, thread, &tx)
            .expect("the loop still owns the receiver");

        assert_eq!(
            rx.try_recv().map(|d| d.id).ok(),
            Some("dl_1".to_string()),
            "the wake has to come back, or the agent is never told about the file"
        );
        assert!(
            !deferred.contains_key(&thread),
            "a re-offered wake must not stay parked, or it is delivered again \
             at the end of every later turn"
        );
    }

    /// One thread going idle does not release another thread's parked wake.
    #[test]
    fn a_turn_ending_releases_only_its_own_threads_completions() {
        let (mine, my_download) = parked("dl_1", 7);
        let (other, other_download) = parked("dl_2", 9);
        let mut deferred = std::collections::HashMap::new();
        deferred.insert(mine, vec![my_download]);
        deferred.insert(other, vec![other_download]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        requeue_deferred_wakes(&mut deferred, mine, &tx).expect("the loop still owns the receiver");

        assert_eq!(rx.try_recv().map(|d| d.id).ok(), Some("dl_1".to_string()));
        assert!(
            rx.try_recv().is_err(),
            "the other thread is still busy; waking it now would displace its turn"
        );
        assert!(deferred.contains_key(&other));
    }

    struct MockTransport {
        peer: PeerInfo,
        incoming: Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
        outgoing: Arc<Mutex<Vec<ServerFrame>>>,
        /// A patient client: once its frames run out, hang up only after
        /// this many close-outs have been sent — the way a real client
        /// stays connected while its turn streams. `None` hangs up the
        /// moment the frames run out (which asks running turns to stop).
        hang_up_after_done: Option<usize>,
        /// See [`MockWriter::fail_from_streaming`].
        fail_writes_from_streaming: bool,
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
        /// Fail every write from the first thread list that reports a thread
        /// as streaming — the broadcast the connection loop sends right after
        /// opening a turn marker. A transport that dies exactly there is what
        /// catches a teardown that does not run on the error path.
        fail_from_streaming: bool,
        /// Set once the transport has failed; it never recovers.
        dead: bool,
    }

    /// Does this frame announce a thread as mid-turn?
    fn announces_streaming(frame: &ServerFrame) -> bool {
        matches!(
            &frame.payload,
            ServerPayload::ThreadsUpdate { threads, .. }
                if threads.iter().any(|t| t.status.as_deref() == Some("Streaming"))
        )
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
                    fail_writes_from_streaming: false,
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
                    fail_writes_from_streaming: false,
                },
                outgoing,
            )
        }

        /// A client whose later frames are decided while the connection is
        /// already running.
        ///
        /// `with_frames*` fixes the whole script up front, which cannot
        /// express "send this *while* that turn is still streaming" — the
        /// only shape in which a serialising gateway differs from an
        /// interleaving one. The returned queue is the client's keyboard:
        /// pushing to it mid-connection is a user typing during someone
        /// else's answer.
        fn injectable(
            peer: PeerInfo,
            initial: Vec<Option<ClientFrame>>,
        ) -> (
            Self,
            Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
            Arc<Mutex<Vec<ServerFrame>>>,
        ) {
            let outgoing = Arc::new(Mutex::new(Vec::new()));
            let incoming = Arc::new(Mutex::new(VecDeque::from(initial)));
            (
                Self {
                    peer,
                    incoming: incoming.clone(),
                    outgoing: outgoing.clone(),
                    // Never hangs up by itself. A count of close-outs cannot
                    // express "stay connected while I decide what to send
                    // next", and a client that disconnects mid-test asks the
                    // gateway to cancel turns — which would be measuring the
                    // disconnect path instead. The test ends the connection
                    // by pushing an explicit end-of-stream.
                    hang_up_after_done: Some(usize::MAX),
                    fail_writes_from_streaming: false,
                },
                incoming,
                outgoing,
            )
        }

        /// A client whose transport dies the instant the gateway announces a
        /// turn as streaming — the write immediately after the turn's start
        /// marker is written to the thread's log.
        fn dying_on_streaming(
            peer: PeerInfo,
            frames: Vec<Option<ClientFrame>>,
        ) -> (Self, Arc<Mutex<Vec<ServerFrame>>>) {
            let outgoing = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    peer,
                    incoming: Arc::new(Mutex::new(VecDeque::from(frames))),
                    outgoing: outgoing.clone(),
                    hang_up_after_done: Some(usize::MAX),
                    fail_writes_from_streaming: true,
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
                    fail_from_streaming: self.fail_writes_from_streaming,
                    dead: false,
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
            if self.fail_from_streaming && announces_streaming(frame) {
                // Latched: a transport does not come back, so every write
                // from here on fails too.
                self.dead = true;
            }
            if self.dead {
                anyhow::bail!("mock transport is gone");
            }
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
                client_kind: None,
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
                client_kind: None,
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
                client_kind: None,
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

    /// A connection that ends on a failed write still closes its turn marker.
    ///
    /// Every write in the connection loop can fail, and the loop propagated
    /// those failures straight out of the connection handler — past the
    /// teardown that writes the stop indicators for turns this connection
    /// opened. The marker stayed open, so the thread reported as "Streaming"
    /// with nothing left that could ever close it. Not until the *next*
    /// process start, either: the sweep only touches markers older than the
    /// running process, so a live daemon carried the stuck thread for as long
    /// as it ran.
    ///
    /// The transport here dies on the broadcast the loop sends immediately
    /// after opening the marker, which is the narrowest window there is.
    #[tokio::test]
    async fn a_connection_that_dies_mid_turn_still_closes_its_marker() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;

        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
        let (mock_transport, outgoing) = MockTransport::dying_on_streaming(
            peer,
            vec![Some(ClientFrame {
                frame_type: ClientFrameType::Chat,
                payload: ClientPayload::Chat {
                    messages: vec![rustyclaw_core::gateway::ChatMessage::text("user", "hello")],
                    thread_id: None,
                    client_kind: None,
                },
            })],
        );

        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();

        let outcome = handle_transport_connection(
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
        .await;
        assert!(
            outcome.is_err(),
            "a transport that stopped accepting writes must end the connection"
        );

        // The premise: the gateway did open a turn before the transport went.
        assert!(
            outgoing.lock().await.iter().any(announces_streaming),
            "the test only means anything if a turn was announced as streaming"
        );

        let restored = rustyclaw_core::threads::ThreadStore::at_legacy_path(&threads_path)
            .load()
            .expect("the store should load");
        let open: Vec<_> = restored.open_threads().into_iter().map(|id| id.0).collect();
        assert!(
            open.is_empty(),
            "a connection that ended on a write failure must leave no turn \
             marker open; these were left: {open:?}"
        );

        Ok(())
    }

    /// Dropping the guard without closing out still closes the markers.
    ///
    /// This is the path an early return or a panic takes — the one no
    /// explicit call reaches, which is the reason the close-out is a guard
    /// rather than a few lines at the bottom of the connection handler.
    /// `Drop` cannot await, so the work is spawned; the test waits for it
    /// the way the next process start would find it.
    #[tokio::test]
    async fn a_dropped_closeout_still_closes_the_markers() -> Result<()> {
        let (_tmp, cfg) = test_config_with_temp_state()?;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        std::fs::create_dir_all(threads_path.parent().unwrap())?;

        let thread_mgr = rustyclaw_core::threads::manager_for(&threads_path);
        let stuck = {
            let mut tm = thread_mgr.lock().await;
            let id = tm.create_chat("mid-turn");
            tm.begin_turn(id);
            id
        };

        let active = Arc::new(Mutex::new(concurrent::ActiveTasks::new()));
        active.lock().await.register(
            stuck,
            1,
            1,
            tokio::spawn(std::future::pending::<()>()),
            crate::ToolCancelFlag::default(),
        );
        let store: crate::agent_handler::StoreCell = Arc::new(std::sync::RwLock::new(
            crate::agent_handler::ConnectionStore {
                thread_mgr: thread_mgr.clone(),
                threads_path: threads_path.clone(),
                foreground: Default::default(),
            },
        ));

        // Never closed out — the shape of a handler that returned early.
        drop(TurnCloseout::new(active, store));

        // The spawned close-out has to land; poll rather than sleep a fixed
        // span so the test is not a race dressed up as a wait.
        let closed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if !thread_mgr
                    .lock()
                    .await
                    .get(stuck)
                    .expect("thread exists")
                    .is_open()
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(
            closed.is_ok(),
            "dropping the guard must close the turn marker, not leave the \
             thread reporting as Streaming forever"
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
                client_kind: None,
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
                client_kind: None,
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
                client_kind: None,
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
                client_kind: None,
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

    /// Holds a scripted model's replies open so a test can keep a turn in
    /// flight while it does something else on the same connection.
    ///
    /// Every existing turn test answers instantly, which means no turn is
    /// ever really *running* when the next frame arrives — so none of them
    /// can tell a gateway that interleaves work from one that serialises it.
    /// This is the difference between "the reply was correct" and "the reply
    /// did not have to wait for another thread".
    #[derive(Clone)]
    struct ModelGate {
        /// Requests that have reached the model and are being held. A test
        /// waits on this rather than sleeping, so it knows the turn is
        /// genuinely inside the provider call and not merely spawned.
        arrivals: Arc<std::sync::atomic::AtomicUsize>,
        release: tokio::sync::watch::Sender<bool>,
    }

    impl ModelGate {
        fn new() -> Self {
            Self {
                arrivals: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                release: tokio::sync::watch::channel(false).0,
            }
        }

        fn arrived(&self) -> usize {
            self.arrivals.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// Wait until `n` requests are being held, or fail the test.
        ///
        /// Polling rather than a signal because the interesting failure is
        /// "the turn never reached the model at all", and that should report
        /// as a clear timeout here rather than as a hang somewhere later.
        async fn await_arrivals(&self, n: usize, what: &str) {
            let deadline = std::time::Duration::from_secs(10);
            tokio::time::timeout(deadline, async {
                while self.arrived() < n {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for {n} model request(s) while {what}; saw {}",
                    self.arrived()
                )
            });
        }

        /// Let every held request answer, and every later one pass straight
        /// through.
        fn release(&self) {
            self.release.send(true).ignore();
        }
    }

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
        spawn_mock_model_gated(None).await
    }

    /// The same endpoint, optionally holding every reply until released.
    async fn spawn_mock_model_gated(gate: Option<ModelGate>) -> std::net::SocketAddr {
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
                let gate = gate.clone();
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
                    // Counted before waiting: the test's signal that a turn
                    // has reached the provider is this request arriving, not
                    // it being answered.
                    // Only completions are held. A connection also probes the
                    // provider while starting up, and gating that would stall
                    // the handshake before a single frame was served — the
                    // test would then be measuring its own harness rather
                    // than whether a turn blocks the loop.
                    let is_completion = headers
                        .lines()
                        .next()
                        .is_some_and(|line| line.contains("/chat/completions"));
                    if let Some(gate) = gate.as_ref().filter(|_| is_completion) {
                        gate.arrivals
                            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let mut rx = gate.release.subscribe();
                        loop {
                            // Copied out so the borrow guard is dropped
                            // before the await below.
                            let open = *rx.borrow();
                            if open || rx.changed().await.is_err() {
                                break;
                            }
                        }
                    }
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
                    sock.write_all(payload.as_bytes()).await.ignore();
                    sock.shutdown().await.ignore();
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
                client_kind: None,
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

    /// Spawn a connection whose frames a test can add to while it runs.
    ///
    /// Returns the client's inbound queue, the frames it has been sent, and
    /// the join handle — the connection is left running on purpose, because
    /// everything interesting happens while it is.
    fn spawn_live_connection(
        cfg: &Config,
        model_ctx: &SharedModelCtx,
        initial: Vec<Option<ClientFrame>>,
    ) -> (
        Arc<Mutex<VecDeque<Option<ClientFrame>>>>,
        Arc<Mutex<Vec<ServerFrame>>>,
        tokio::task::JoinHandle<Result<()>>,
    ) {
        let peer = PeerInfo {
            addr: Some("127.0.0.1:2222".parse().unwrap()),
            username: Some("tester".to_string()),
            key_fingerprint: Some("SHA256:test".to_string()),
            transport_type: TransportType::Ssh,
        };
        let (transport, incoming, outgoing) = MockTransport::injectable(peer, initial);
        let vault: SharedVault = Arc::new(Mutex::new(SecretsManager::new(cfg.credentials_dir())));
        let skill_mgr: SharedSkillManager =
            Arc::new(Mutex::new(SkillManager::new(cfg.skills_dir())));
        rustyclaw_core::tools::init_plugin_manager(&cfg.workspace_dir());
        let task_mgr: SharedTaskManager = Arc::new(rustyclaw_core::tasks::TaskManager::new());
        let model_registry = rustyclaw_core::models::create_model_registry();
        let cfg = Arc::new(RwLock::new(cfg.clone()));
        let model_ctx = model_ctx.clone();
        let handle = tokio::spawn(async move {
            handle_transport_connection(
                Box::new(transport),
                cfg,
                model_ctx,
                Arc::new(RwLock::new(None)),
                vault,
                skill_mgr,
                task_mgr,
                model_registry,
                None,
                auth::new_rate_limiter(),
                CancellationToken::new(),
            )
            .await
        });
        (incoming, outgoing, handle)
    }

    /// Wait for a frame the client should have been sent, or say what did
    /// arrive instead.
    ///
    /// The failure this exists to report is a gateway that is *not
    /// answering*, so the timeout has to be the assertion rather than an
    /// outer harness timeout that cannot say which frame was missing.
    async fn await_frame(
        outgoing: &Arc<Mutex<Vec<ServerFrame>>>,
        what: &str,
        matches: impl Fn(&ServerFrame) -> bool,
    ) -> ServerFrame {
        let deadline = std::time::Duration::from_secs(10);
        let found = tokio::time::timeout(deadline, async {
            loop {
                if let Some(f) = outgoing.lock().await.iter().find(|f| matches(f)) {
                    return f.clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        match found {
            Ok(f) => f,
            Err(_) => {
                let seen: Vec<String> = outgoing
                    .lock()
                    .await
                    .iter()
                    .map(|f| format!("{:?}", f.frame_type))
                    .collect();
                panic!("timed out waiting for {what}; frames seen: {seen:?}");
            }
        }
    }

    fn gated_model_ctx(addr: std::net::SocketAddr) -> SharedModelCtx {
        Arc::new(RwLock::new(Some(Arc::new(
            rustyclaw_core::gateway::ModelContext {
                provider: "openai".to_string(),
                model: "mock-model".to_string(),
                base_url: format!("http://{addr}/v1"),
                api_key: Some("test-key".to_string()),
            },
        ))))
    }

    /// Opening a thread must not wait for another thread's turn to finish.
    ///
    /// The reported symptom: creating a thread while one was answering
    /// blocked immediately. A turn is spawned, so the connection loop is
    /// free in principle — but only if nothing on the way to `ThreadCreate`
    /// waits on something the running turn holds. The model is held open
    /// here so the turn is genuinely mid-provider-call, which is the state
    /// every other turn test skips past by answering instantly.
    #[tokio::test]
    async fn a_thread_can_be_created_while_another_thread_is_answering() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, _beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let gate = ModelGate::new();
        let addr = spawn_mock_model_gated(Some(gate.clone())).await;
        let model_ctx = gated_model_ctx(addr);

        let (incoming, outgoing, handle) = spawn_live_connection(
            &cfg,
            &model_ctx,
            vec![Some(chat_frame(alpha.0, "hold the line"))],
        );

        // The turn is now inside the provider call and will stay there.
        gate.await_arrivals(1, "starting the first turn").await;

        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadCreate,
            payload: ClientPayload::ThreadCreate {
                label: "opened mid-answer".to_string(),
                project_id: 0,
            },
        }));

        // The whole test: this must come back with the turn still running.
        await_frame(&outgoing, "ThreadCreated while a turn was in flight", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadCreated)
        })
        .await;
        assert_eq!(
            gate.arrived(),
            1,
            "the first turn should still be waiting on the model"
        );

        gate.release();
        await_frame(&outgoing, "the held turn to finish", |f| {
            matches!(f.payload, ServerPayload::ResponseDone { .. })
        })
        .await;
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;
        Ok(())
    }

    /// Two threads must be able to be answering at the same time.
    ///
    /// This is the multiplexing claim itself: a second thread's turn should
    /// reach the model while the first is still waiting on it. If the
    /// gateway serialises turns, the second request never arrives until the
    /// first is released, and this times out with one arrival instead of
    /// two.
    #[tokio::test]
    async fn two_threads_can_be_answering_at_once() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (alpha, beta) = seed_two_threads(&cfg, "alpha", "beta")?;

        let gate = ModelGate::new();
        let addr = spawn_mock_model_gated(Some(gate.clone())).await;
        let model_ctx = gated_model_ctx(addr);

        let (incoming, outgoing, handle) =
            spawn_live_connection(&cfg, &model_ctx, vec![Some(chat_frame(alpha.0, "first"))]);

        gate.await_arrivals(1, "starting the first turn").await;

        // Typed into the other thread while the first is still answering.
        incoming
            .lock()
            .await
            .push_back(Some(chat_frame(beta.0, "second")));

        gate.await_arrivals(2, "a second turn should not wait for the first")
            .await;

        // Both are held at the model at the same instant — that is the
        // property, not merely that both eventually completed.
        gate.release();
        let done = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let n = outgoing
                    .lock()
                    .await
                    .iter()
                    .filter(|f| matches!(f.payload, ServerPayload::ResponseDone { .. }))
                    .count();
                if n >= 2 {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(done.is_ok(), "both turns should finish once released");
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;
        Ok(())
    }

    /// Seed a foreground thread with enough history to be worth compacting,
    /// plus a second thread to run a turn in and a third to switch to.
    fn seed_for_compaction(
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

        // The outgoing foreground: long enough to trip the compaction rule
        // (more than three messages, no summary yet).
        let stale = manager.create_chat("stale");
        for i in 0..6 {
            manager.add_message(stale, MessageRole::User, format!("q{i}"));
            manager.add_message(stale, MessageRole::Assistant, format!("a{i}"));
        }
        let busy = manager.create_chat("busy");
        let target = manager.create_chat("target");
        manager.switch_foreground(stale);
        manager.save_to_file(&threads_path)?;
        Ok((stale, busy, target))
    }

    /// Switching threads must not stop the connection serving everything
    /// else while it summarises the thread being left behind.
    ///
    /// Compaction is a provider round trip and it is awaited *inside* the
    /// connection loop, so for as long as it runs the connection answers
    /// nothing: not another thread's stream, not a Stop, not a thread being
    /// opened. That is the "it blocked immediately" report — the click that
    /// opens a thread is also the click that freezes the connection.
    ///
    /// No turn is needed to show this. The switch alone is enough, which is
    /// the point: the freeze is not a consequence of concurrency, it is
    /// what makes concurrency impossible.
    #[tokio::test]
    async fn a_switch_that_compacts_does_not_freeze_the_connection() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (_stale, _busy, target) = seed_for_compaction(&cfg)?;

        let gate = ModelGate::new();
        let addr = spawn_mock_model_gated(Some(gate.clone())).await;
        let model_ctx = gated_model_ctx(addr);

        let (incoming, outgoing, handle) = spawn_live_connection(
            &cfg,
            &model_ctx,
            vec![Some(ClientFrame {
                frame_type: ClientFrameType::ThreadSwitch,
                payload: ClientPayload::ThreadSwitch {
                    thread_id: target.0,
                },
            })],
        );

        // The switch is now inside the summarisation call.
        gate.await_arrivals(1, "the switch should be compacting")
            .await;

        // Anything at all, asked while that call is outstanding.
        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadCreate,
            payload: ClientPayload::ThreadCreate {
                label: "proof of life".to_string(),
                project_id: 0,
            },
        }));

        await_frame(
            &outgoing,
            "ThreadCreated while a compaction call was outstanding",
            |f| matches!(f.frame_type, ServerFrameType::ThreadCreated),
        )
        .await;

        gate.release();
        await_frame(&outgoing, "the switch to be acknowledged", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;
        Ok(())
    }

    /// Flipping between threads must not pay for the same summary twice.
    ///
    /// Compaction is eligible while a thread is long and unsummarised, and
    /// both stay true for the whole provider round trip. Now that a switch
    /// returns immediately, a user can switch away, back, and away again
    /// before the first summary lands — and every one of those would start
    /// another paid request, with all but one result thrown away and a
    /// "Compacting..." notice each time.
    #[tokio::test]
    async fn switching_back_and_forth_summarises_a_thread_once() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (stale, _busy, target) = seed_for_compaction(&cfg)?;

        let gate = ModelGate::new();
        let addr = spawn_mock_model_gated(Some(gate.clone())).await;
        let model_ctx = gated_model_ctx(addr);

        let switch_to = |id: u64| {
            Some(ClientFrame {
                frame_type: ClientFrameType::ThreadSwitch,
                payload: ClientPayload::ThreadSwitch { thread_id: id },
            })
        };

        let (incoming, outgoing, handle) =
            spawn_live_connection(&cfg, &model_ctx, vec![switch_to(target.0)]);

        // The first summary of `stale` is now in flight and stays there.
        gate.await_arrivals(1, "the first switch should be compacting")
            .await;

        // Back and away again, while it is still being written.
        incoming.lock().await.push_back(switch_to(stale.0));
        incoming.lock().await.push_back(switch_to(target.0));

        // Counted, not matched by id: the first switch already produced a
        // `ThreadSwitched` naming `target`, so waiting for one of those
        // again matches the frame that has already arrived and proves
        // nothing. All three must have been served — which is also the
        // freeze fix still holding — before the arrival count means
        // anything.
        let served = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let n = outgoing
                    .lock()
                    .await
                    .iter()
                    .filter(|f| matches!(f.frame_type, ServerFrameType::ThreadSwitched))
                    .count();
                if n >= 3 {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(served.is_ok(), "all three switches should have been served");

        // Counted from the notice, not the arrival count: the notice is
        // written synchronously in the connection loop just before the task
        // is spawned, so by the time all three switches are acknowledged it
        // is already there. Waiting on the model instead would race the
        // duplicate's HTTP request and pass whether or not the guard works —
        // it did, before this was corrected.
        let compacting_notices = outgoing
            .lock()
            .await
            .iter()
            .filter(|f| {
                matches!(&f.payload, ServerPayload::Info { message }
                    if message.contains("Compacting"))
            })
            .count();
        assert_eq!(
            compacting_notices, 1,
            "the same thread was summarised again while one call was already \
             in flight"
        );

        gate.release();
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;
        Ok(())
    }

    /// A switch with no model configured must leave the thread compactable.
    ///
    /// The in-flight marker is only cleared by the task that sets it, so
    /// marking a thread without starting one strands it: `!compacting` is
    /// false for the rest of the process and the thread is never summarised
    /// again, its context growing without bound. Nothing surfaces that — it
    /// looks exactly like a thread that has not needed compacting yet.
    ///
    /// One connection throughout, with the model appearing partway. The
    /// marker is not persisted, so a second connection would load a clean
    /// manager and could not see the leak at all — which is how the first
    /// version of this test passed while the bug was present.
    #[tokio::test]
    async fn a_switch_without_a_model_leaves_the_thread_compactable() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (stale, _busy, target) = seed_for_compaction(&cfg)?;

        let switch_to = |id: u64| {
            Some(ClientFrame {
                frame_type: ClientFrameType::ThreadSwitch,
                payload: ClientPayload::ThreadSwitch { thread_id: id },
            })
        };

        // Switch away from `stale` with nothing to summarise with.
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));
        let (incoming, outgoing, handle) =
            spawn_live_connection(&cfg, &model_ctx, vec![switch_to(target.0)]);
        await_frame(&outgoing, "the first switch to be acknowledged", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;
        assert!(
            !outgoing.lock().await.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::Info { message } if message.contains("Compacting")
            )),
            "nothing should be announced as compacting when there is no model"
        );

        // A model is configured on the same connection.
        let gate = ModelGate::new();
        let addr = spawn_mock_model_gated(Some(gate.clone())).await;
        *model_ctx.write().await = Some(Arc::new(rustyclaw_core::gateway::ModelContext {
            provider: "openai".to_string(),
            model: "mock-model".to_string(),
            base_url: format!("http://{addr}/v1"),
            api_key: Some("test-key".to_string()),
        }));

        // Back to `stale`, then away from it again — the switch that should
        // now summarise it.
        incoming.lock().await.push_back(switch_to(stale.0));
        incoming.lock().await.push_back(switch_to(target.0));
        let served = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let n = outgoing
                    .lock()
                    .await
                    .iter()
                    .filter(|f| matches!(f.frame_type, ServerFrameType::ThreadSwitched))
                    .count();
                if n >= 3 {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await;
        assert!(served.is_ok(), "all three switches should have been served");

        assert!(
            outgoing.lock().await.iter().any(|f| matches!(
                &f.payload,
                ServerPayload::Info { message } if message.contains("Compacting")
            )),
            "the thread was left permanently ineligible by a switch that \
             never started a summary"
        );

        gate.release();
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;
        Ok(())
    }

    /// Two windows on one agent must not delete each other's threads.
    ///
    /// `AgentSession::load` builds a *fresh* `ThreadManager` per connection,
    /// read from disk at connect time, and `ThreadStore::persist` is a
    /// reconciliation: it deletes the files of every thread the manager it
    /// is handed does not contain. So a second window's manager is a
    /// snapshot from before anything the first window has done since, and
    /// the moment it writes, that work is removed from disk.
    ///
    /// This is the same hazard the store's own test characterises, arriving
    /// through the ordinary path rather than a background task: two clients
    /// open on the same agent, which the download work already established
    /// is a routine thing for a user to do.
    #[tokio::test]
    async fn two_connections_on_one_agent_do_not_delete_each_others_threads() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (seeded, _other) = seed_two_threads(&cfg, "seeded", "other")?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let create = |label: &str| {
            Some(ClientFrame {
                frame_type: ClientFrameType::ThreadCreate,
                payload: ClientPayload::ThreadCreate {
                    label: label.to_string(),
                    project_id: 0,
                },
            })
        };

        // Both windows connect, so both load the store as it is now.
        let (in_a, out_a, handle_a) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        let (in_b, out_b, handle_b) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        await_frame(&out_a, "window A to finish connecting", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;
        await_frame(&out_b, "window B to finish connecting", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;

        // A creates a thread and persists it.
        in_a.lock().await.push_back(create("from A"));
        await_frame(&out_a, "A's thread to be created", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadCreated)
        })
        .await;

        // B creates one too. B's manager has never seen A's.
        in_b.lock().await.push_back(create("from B"));
        await_frame(&out_b, "B's thread to be created", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadCreated)
        })
        .await;

        in_a.lock().await.push_back(None);
        in_b.lock().await.push_back(None);
        handle_a.await.expect("A panicked")?;
        handle_b.await.expect("B panicked")?;

        // What a third window would open onto.
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        let reloaded = rustyclaw_core::threads::ThreadStore::load_or_migrate(&threads_path);
        let labels: Vec<String> = reloaded.list().iter().map(|t| t.label.clone()).collect();

        assert!(
            labels.iter().any(|l| l == "from A"),
            "the first window's thread was deleted by the second window's \
             write: {labels:?}"
        );
        assert!(labels.iter().any(|l| l == "from B"), "{labels:?}");
        assert!(
            reloaded.get(seeded).is_some(),
            "the seeded thread should survive too: {labels:?}"
        );
        Ok(())
    }

    /// One window's thread switch must not move another window's view.
    ///
    /// Sharing the manager between the windows open on an agent shares its
    /// `foreground_id`, which used to be per-connection only by accident of
    /// each connection owning a manager. A foreground is a statement about
    /// one client — which transcript is on screen — and clients act on it:
    /// the TUI treats a changed `foreground_id` in a `ThreadsUpdate` as an
    /// instruction to switch conversation and fetch its history. So B
    /// opening a thread dragged A onto it, mid-sentence.
    ///
    /// Both halves are checked, because they fail through different paths:
    /// the update A gets *unasked* (the manager's `Foregrounded` event
    /// reaches every subscriber) and the one A gets when it asks.
    #[tokio::test]
    async fn a_switch_in_one_window_leaves_the_other_windows_foreground_alone() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (first, second) = seed_two_threads(&cfg, "first", "second")?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let switch_to = |id: rustyclaw_core::threads::ThreadId| {
            Some(ClientFrame {
                frame_type: ClientFrameType::ThreadSwitch,
                payload: ClientPayload::ThreadSwitch { thread_id: id.0 },
            })
        };

        let (in_a, out_a, handle_a) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        let (in_b, out_b, handle_b) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        for (out, who) in [(&out_a, "A"), (&out_b, "B")] {
            await_frame(out, &format!("window {who} to finish connecting"), |f| {
                matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
            })
            .await;
        }

        // A settles on `first` and stays there for the rest of the test.
        in_a.lock().await.push_back(switch_to(first));
        await_frame(&out_a, "A's switch to first", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;
        // Everything from here on is what A sees *after* it stopped moving.
        out_a.lock().await.clear();

        // B moves to the other thread.
        in_b.lock().await.push_back(switch_to(second));
        await_frame(&out_b, "B's switch to second", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;

        // Ask A for its thread list: a frame we can wait for deterministically,
        // by which point B's switch has already been broadcast.
        in_a.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadList,
            payload: ClientPayload::ThreadList,
        }));
        await_frame(&out_a, "A's thread list", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;

        let moved: Vec<Option<u64>> = out_a
            .lock()
            .await
            .iter()
            .filter_map(|f| match &f.payload {
                ServerPayload::ThreadsUpdate { foreground_id, .. } => Some(*foreground_id),
                _ => None,
            })
            .filter(|fg| *fg != Some(first.0))
            .collect();
        assert!(
            moved.is_empty(),
            "window B's switch moved window A's foreground to {moved:?}; \
             A never left thread {}",
            first.0
        );

        // And B really did move — otherwise the assertion above passes for
        // the wrong reason.
        let b_foreground = out_b
            .lock()
            .await
            .iter()
            .rev()
            .find_map(|f| match &f.payload {
                ServerPayload::ThreadsUpdate { foreground_id, .. } => Some(*foreground_id),
                _ => None,
            })
            .expect("B received a threads update");
        assert_eq!(
            b_foreground,
            Some(second.0),
            "B asked to switch and should be looking at its own choice"
        );

        in_a.lock().await.push_back(None);
        in_b.lock().await.push_back(None);
        handle_a.await.expect("A panicked")?;
        handle_b.await.expect("B panicked")?;
        Ok(())
    }

    /// Closing a thread from one window must not strand another window in it.
    ///
    /// The pointer is per-connection but the threads are not: whoever issues
    /// the close re-elects for itself, and before this every *other* window
    /// kept an id that no longer resolved — reported downstream as nothing
    /// selected, with no history and no row highlighted, recoverable only by
    /// clicking. The manager's own `remove` used to elect for everyone.
    #[tokio::test]
    async fn closing_a_thread_does_not_strand_the_window_watching_it() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (first, second) = seed_two_threads(&cfg, "first", "second")?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let (incoming_a, outgoing_a, _a) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        let (incoming_b, outgoing_b, _b) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        for out in [&outgoing_a, &outgoing_b] {
            await_frame(out, "the connection to settle", |f| {
                matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
            })
            .await;
        }

        // B parks itself in `second`; A stays in `first`.
        incoming_b.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch {
                thread_id: second.0,
            },
        }));
        await_frame(&outgoing_b, "B's switch", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;
        // `await_frame` searches everything received so far, and B's opening
        // update already names a thread that is not `second` — exactly what
        // the assertion below looks for. Clear, so the frame it finds can
        // only be one sent after the close.
        outgoing_b.lock().await.clear();

        // A closes the thread B is sitting in.
        incoming_a.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadClose,
            payload: ClientPayload::ThreadClose {
                thread_id: second.0,
            },
        }));

        // B's next sidebar refresh should put it somewhere real.
        let update = await_frame(&outgoing_b, "B's update after the close", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
                && !matches!(
                    &f.payload,
                    ServerPayload::ThreadsUpdate { foreground_id, .. }
                        if *foreground_id == Some(second.0)
                )
        })
        .await;
        let ServerPayload::ThreadsUpdate { foreground_id, .. } = update.payload else {
            panic!("expected a ThreadsUpdate payload");
        };
        assert_eq!(
            foreground_id,
            Some(first.0),
            "the surviving thread should be elected for the window left behind"
        );
        Ok(())
    }

    /// One window backgrounding its thread must not blank the next one.
    ///
    /// The sentinel means "*I* want no conversation open" — it is one
    /// client speaking for itself. But it empties the shared manager's
    /// pointer, and a window opening afterwards used to adopt that verbatim:
    /// no thread selected, empty transcript, and a sidebar that hides
    /// message-less threads because none is marked foreground. Reloading the
    /// store per connection hid this, since `ThreadStore::load` elects.
    #[tokio::test]
    async fn a_backgrounded_window_does_not_blank_the_next_one() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (first, second) = seed_two_threads(&cfg, "first", "second")?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let (incoming_a, outgoing_a, _a) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        await_frame(&outgoing_a, "the first window to settle", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;

        // `thread_id: 0` — background whatever this window is in.
        incoming_a.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch { thread_id: 0 },
        }));
        await_frame(&outgoing_a, "the background", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;

        // A second window opens while the first is still connected and
        // still holding nothing.
        let (_incoming_b, outgoing_b, _b) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        let update = await_frame(&outgoing_b, "the second window's thread list", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;
        let ServerPayload::ThreadsUpdate { foreground_id, .. } = update.payload else {
            panic!("expected a ThreadsUpdate payload");
        };
        // Which of the two the rule picks is the election's business, and
        // the seeded pair can tie on `last_activity`; what this is about is
        // that *something* is open.
        assert!(
            foreground_id.is_some_and(|id| id == first.0 || id == second.0),
            "a new window should open on a conversation, not a blank screen; \
             got {foreground_id:?}"
        );
        Ok(())
    }

    /// A switch survives a kill, not just a clean close.
    ///
    /// The teardown write is the polite path and a killed process never
    /// reaches it. Recording the choice only there would mean a crash
    /// reopens on whatever the manager last happened to hold — so this
    /// asserts against the store *without* ending the connection, which is
    /// what a `SIGKILL` between the switch and the close looks like on disk.
    #[tokio::test]
    async fn a_thread_switch_is_on_disk_before_the_window_closes() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (_first, second) = seed_two_threads(&cfg, "first", "second")?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let (incoming, outgoing, _handle) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        await_frame(&outgoing, "the connection to settle", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;

        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch {
                thread_id: second.0,
            },
        }));
        await_frame(&outgoing, "the switch", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;

        // Deliberately no disconnect: the switch alone must have reached
        // the store.
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        let reloaded = rustyclaw_core::threads::ThreadStore::load_or_migrate(&threads_path);
        assert_eq!(
            reloaded.foreground_id(),
            Some(second),
            "the switch should be durable without waiting for a clean shutdown"
        );
        Ok(())
    }

    /// Typing into a thread picks it just as much as switching does, and
    /// must reach the store just as durably.
    ///
    /// While the pointer lived on the shared manager, the persist that
    /// follows a user message carried this for free. With the pointer on a
    /// per-connection cell that persist writes whatever the manager holds,
    /// so a client that chose its thread by typing into it had that choice
    /// live only in memory until a clean shutdown — and a killed gateway
    /// reopened somewhere else, or on another window's thread.
    #[tokio::test]
    async fn typing_into_a_thread_is_on_disk_before_the_window_closes() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        // Seeded foreground is `first`, so naming `second` is a real move.
        let (_first, second) = seed_two_threads(&cfg, "first", "second")?;
        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let (incoming, outgoing, _handle) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        await_frame(&outgoing, "the connection to settle", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;

        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![ChatMessage::text("user", "hello")],
                thread_id: Some(second.0),
                client_kind: None,
            },
        }));

        // Deliberately no disconnect — and polled rather than keyed off a
        // frame, because the write and the reply are not ordered with
        // respect to each other.
        let landed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let reloaded = rustyclaw_core::threads::ThreadStore::load_or_migrate(&threads_path);
                if reloaded.foreground_id() == Some(second) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            landed.is_ok(),
            "the thread a message was typed into should be durable without a clean shutdown"
        );
        Ok(())
    }

    /// Closing a window records where it was, so reopening lands there.
    ///
    /// The foreground is a per-connection cell, so the store only learns it
    /// when the connection says so on the way out. Miss that and the
    /// persisted pointer is whatever the manager happened to hold — the
    /// most recently created thread, or whatever was elected at load — and
    /// the user reopens in a conversation they never chose.
    ///
    /// The switch here is to the thread that is *not* the seeded foreground,
    /// so a pointer that simply never moved fails this.
    #[tokio::test]
    async fn a_window_reopens_on_the_thread_it_was_closed_in() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let (_first, second) = seed_two_threads(&cfg, "first", "second")?;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let (incoming, outgoing, handle) = spawn_live_connection(&cfg, &model_ctx, vec![]);
        await_frame(&outgoing, "the connection to settle", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadsUpdate)
        })
        .await;

        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadSwitch,
            payload: ClientPayload::ThreadSwitch {
                thread_id: second.0,
            },
        }));
        await_frame(&outgoing, "the switch", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadSwitched)
        })
        .await;

        // Close the window — the teardown is the only thing that can record
        // this, and it is the path that used to bypass it.
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;

        let threads_path = cfg
            .sessions_dir_for(rustyclaw_core::agents::MAIN_AGENT_ID)
            .join("threads.json");
        let reloaded = rustyclaw_core::threads::ThreadStore::load_or_migrate(&threads_path);
        assert_eq!(
            reloaded.foreground_id(),
            Some(second),
            "the store should name the thread the window was closed in"
        );
        Ok(())
    }

    /// Deleting an agent must take its conversations with it, even if
    /// another agent is created under the same id afterwards.
    ///
    /// The shared manager is cached for the life of the process, so the
    /// cache has to be told when the store it stands for is deleted.
    /// Otherwise the recreated agent is handed the dead one's manager and
    /// the first write puts those conversations back on disk — threads the
    /// user deleted, reappearing. Reloading per connection made this
    /// impossible; caching is what introduces it, so it is this change's to
    /// close.
    #[tokio::test]
    async fn a_recreated_agent_does_not_inherit_the_deleted_ones_threads() -> Result<()> {
        let (_tmp, mut cfg) = test_config_with_temp_state()?;
        cfg.totp_enabled = false;
        let model_ctx: SharedModelCtx = Arc::new(RwLock::new(None));

        let create_agent = || {
            Some(ClientFrame {
                frame_type: ClientFrameType::AgentCreate,
                payload: ClientPayload::AgentCreate {
                    name: "Researcher".into(),
                    agent_id: None,
                    description: None,
                },
            })
        };
        let switch_agent = |id: &str| {
            Some(ClientFrame {
                frame_type: ClientFrameType::AgentSwitch,
                payload: ClientPayload::AgentSwitch {
                    agent_id: id.to_string(),
                },
            })
        };

        // Create the agent, switch to it, and give it a conversation.
        let (incoming, outgoing, handle) = spawn_live_connection(
            &cfg,
            &model_ctx,
            vec![create_agent(), switch_agent("researcher")],
        );
        await_frame(&outgoing, "the switch to researcher", |f| {
            matches!(f.frame_type, ServerFrameType::AgentsUpdate)
        })
        .await;
        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::ThreadCreate,
            payload: ClientPayload::ThreadCreate {
                label: "secret plans".to_string(),
                project_id: 0,
            },
        }));
        await_frame(&outgoing, "the thread to be created", |f| {
            matches!(f.frame_type, ServerFrameType::ThreadCreated)
        })
        .await;

        // Switch away so the agent can be deleted, then delete it.
        incoming.lock().await.push_back(switch_agent("main"));
        incoming.lock().await.push_back(Some(ClientFrame {
            frame_type: ClientFrameType::AgentDelete,
            payload: ClientPayload::AgentDelete {
                agent_id: "researcher".into(),
            },
        }));
        // Recreate under the same id and switch back.
        incoming.lock().await.push_back(create_agent());
        incoming.lock().await.push_back(switch_agent("researcher"));
        incoming.lock().await.push_back(None);
        handle.await.expect("connection task panicked")?;

        let threads_path = cfg.sessions_dir_for("researcher").join("threads.json");
        let reloaded = rustyclaw_core::threads::ThreadStore::load_or_migrate(&threads_path);
        let labels: Vec<String> = reloaded.list().iter().map(|t| t.label.clone()).collect();
        assert!(
            !labels.iter().any(|l| l == "secret plans"),
            "a deleted agent's conversation came back with an agent of the \
             same name: {labels:?}"
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
