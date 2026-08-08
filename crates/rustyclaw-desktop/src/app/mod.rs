//! Top-level application component.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons, Notification};
use rustyclaw_view::{tokio, tracing};
use std::sync::{Arc, Mutex as StdMutex};

use crate::components::{
    Chat, EditProjectDialog, EditThreadDialog, NewProjectDialog, PluginActionEvent, Sidebar,
};

use crate::app_support::*;
use crate::state::AppState;
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::gateway::SessionOrigin;
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_core::user_prompt_types::PromptResponseValue;

use rustyclaw_view::{
    BannerActionKind, HatchingDialogData, PromptAttachment, build_prompt_with_attachments,
};

mod dialogs;
mod signals;

use anyhow::Context;
use dialogs::render_dialogs;
use signals::do_reconnect;

const DIRECTORY_OTHER_SENTINEL: &str = "__directory_other__";

/// Bundled stylesheets — embedded directly in the binary so the desktop crate
/// can be run with plain `cargo run`/`cargo build` without the `dx` CLI.
/// Bulma provides the component framework; `styles.css` layers the RustyClaw
/// brand theme and app-shell layout on top.
const BULMA: &str = include_str!("../../assets/bulma.min.css");
const STYLES: &str = include_str!("../../assets/styles.css");

/// Delegated click handler that routes external links out to the OS browser.
///
/// Registered once (guarded by a window flag, since the effect can re-run) and
/// installed in the capture phase so it wins regardless of anything the
/// rendered markup does. Only `http(s)` and `mailto:` are forwarded; anything
/// else is left for the webview to ignore, and the Rust side re-validates the
/// scheme before it reaches a shell.
const LINK_INTERCEPT_JS: &str = r#"
(function () {
  if (window.__rcLinkIntercept) return;
  window.__rcLinkIntercept = true;
  document.addEventListener(
    "click",
    function (ev) {
      if (ev.defaultPrevented || ev.button !== 0) return;
      var el = ev.target;
      while (el && el.nodeType === 1 && el.tagName !== "A") el = el.parentElement;
      if (!el || el.tagName !== "A") return;
      var href = el.getAttribute("href") || "";
      if (!/^\s*(https?:|mailto:)/i.test(href)) return;
      ev.preventDefault();
      ev.stopPropagation();
      dioxus.send(href.trim());
    },
    true
  );
})();
"#;

#[component]
pub fn App() -> Element {
    // Application state
    let mut state = use_signal(AppState::default);

    // Gateway client (set when connected)
    let gateway: Signal<Option<Arc<GatewayClient>>> = use_signal(|| None);
    let mut did_auto_connect = use_signal(|| false);
    let mut active_event_client: Signal<Option<Arc<GatewayClient>>> = use_signal(|| None);
    let auth_code = use_signal(String::new);

    // Dialog visibility
    let mut show_pairing = use_signal(|| false);
    let hatching_dialog = use_signal(|| HatchingDialogData::new(state.read().needs_hatching));
    let mut show_settings = use_signal(|| false);
    let mut show_swarm = use_signal(|| false);
    let swarm_creating = use_signal(|| false);

    // Tool approval state
    let mut tool_approval_id = use_signal(String::new);
    let mut tool_approval_name = use_signal(String::new);
    let mut tool_approval_args = use_signal(String::new);
    let mut show_tool_approval = use_signal(|| false);

    // Vault unlock state
    let mut show_vault_unlock = use_signal(|| false);
    let vault_unlock_error = use_signal(|| None::<String>);

    // Credential request state
    let mut show_cred_request = use_signal(|| false);
    let mut cred_request_id = use_signal(String::new);
    let mut cred_request_provider = use_signal(String::new);
    let mut cred_request_secret = use_signal(String::new);
    let mut cred_request_message = use_signal(String::new);

    // QR code for pairing
    let qr_code_url = use_signal(|| None::<String>);
    let public_key = use_signal(|| None::<String>);

    // Secrets management state
    let mut show_secrets = use_signal(|| false);
    let show_messengers = use_signal(|| false);

    // New-project dialog state
    let mut show_new_project = use_signal(|| false);
    // Edit dialogs mount only while open so their fields always initialise
    // from the row's current values.
    let mut edit_project = use_signal(|| None::<u64>);
    let mut edit_thread = use_signal(|| None::<u64>);

    // Thread deletion confirmation state
    let pending_thread_delete = use_signal(|| None::<(u64, String)>);

    // Initialize directory chooser state once.
    let mut did_init_directories = use_signal(|| false);

    let configured_gateway_url = crate::configured_gateway_url();
    let skip_dialog = crate::skip_connection_dialog();
    let bypass_dialog = crate::should_bypass_connection_dialog();
    let startup_auto_connect_urls = if let Some(url) = configured_gateway_url.clone() {
        vec![url]
    } else if skip_dialog {
        let mut urls = crate::load_auto_connect_gateway_urls();
        if urls.is_empty() {
            urls.push(state.read().gateway_url.clone());
        }
        urls
    } else if bypass_dialog {
        crate::load_auto_connect_gateway_urls()
    } else {
        Vec::new()
    };

    // Connection dialog is shown only when startup configuration does not
    // request bypass and no explicit CLI override is provided. The
    // --pick-connection flag (used by "New Connection Window") forces it.
    let force_dialog = crate::force_connection_dialog();
    let mut show_connection = use_signal(move || {
        force_dialog || (configured_gateway_url.is_none() && !skip_dialog && !bypass_dialog)
    });

    // Connection history / default / autoconnect from the client prefs.
    let connection_prefs = use_signal(rustyclaw_view::ConnectionDialogData::load);

    let sig = signals::AppSignals {
        state,
        gateway,
        did_auto_connect,
        active_event_client,
        auth_code,
        show_pairing,
        hatching_dialog,
        show_settings,
        show_swarm,
        swarm_creating,
        tool_approval_id,
        tool_approval_name,
        tool_approval_args,
        show_tool_approval,
        show_vault_unlock,
        vault_unlock_error,
        show_cred_request,
        cred_request_id,
        cred_request_provider,
        cred_request_secret,
        cred_request_message,
        qr_code_url,
        public_key,
        show_secrets,
        show_messengers,
        pending_thread_delete,
        did_init_directories,
        show_connection,
        connection_prefs,
    };

    // Auto-connect on mount
    use_effect(move || {
        if *did_auto_connect.read() {
            return;
        }
        // When the connection dialog is showing we wait for the user
        // to confirm/edit the URL before attempting any connection.
        if *show_connection.read() {
            return;
        }
        did_auto_connect.set(true);

        let startup_urls = startup_auto_connect_urls.clone();
        spawn(async move {
            if startup_urls.is_empty() {
                return;
            }
            let _ = connect_to_gateway_candidates(startup_urls, state, gateway).await;
        });
    });

    // Set the macOS Dock icon once tao/NSApplication is running.
    // The main.rs call is a best-effort early attempt; this one runs
    // inside the Dioxus event loop so NSApplication is fully initialized.
    #[cfg(target_os = "macos")]
    use_effect(move || {
        crate::set_dock_icon();
    });

    // Links in agent output open in the user's browser.
    //
    // A plain anchor click inside a webview navigates the webview itself,
    // which would replace the whole app UI with the target page, and
    // `target="_blank"` is simply swallowed because no new-window handler is
    // installed. Both are intercepted here and handed to the OS instead.
    use_future(move || async move {
        let mut eval = document::eval(LINK_INTERCEPT_JS);
        loop {
            match eval.recv::<String>().await {
                Ok(url) => match rustyclaw_core::open_external::open_external(&url) {
                    Ok(()) => tracing::debug!(%url, "opened link externally"),
                    Err(e) => tracing::warn!(%url, error = %e, "refused to open link"),
                },
                Err(e) => {
                    tracing::warn!(error = %e, "link interceptor stopped");
                    break;
                }
            }
        }
    });

    // Close the connection dialog automatically once we've successfully
    // connected (or authenticated, for gateways that require auth).
    use_effect(move || {
        let status = state.read().connection.clone();
        if matches!(
            status,
            ConnectionStatus::Connected | ConnectionStatus::Authenticated
        ) && *show_connection.read()
        {
            show_connection.set(false);
        }
    });

    // Ask the gateway for the active provider's live model list so the
    // model picker reflects the provider API instead of the static
    // catalogue.  Re-runs on provider switches; the `requested` set keeps
    // it to one request per provider.  Failed fetches leave the guard in
    // place (clearing it here would loop, since this effect observes the
    // same state); `on_model_change` clears it on an explicit provider
    // switch so the user can retry.
    use_effect(move || {
        let (provider, connected, already_requested) = {
            let s = state.read();
            let provider = s.provider.clone();
            let connected = matches!(
                s.connection,
                ConnectionStatus::Connected | ConnectionStatus::Authenticated
            );
            let requested = provider
                .as_deref()
                .map(|p| s.provider_models_requested.contains(p))
                .unwrap_or(true);
            (provider, connected, requested)
        };
        let Some(provider) = provider else { return };
        if !connected || already_requested {
            return;
        }
        let Some(client) = gateway.read().clone() else {
            return;
        };
        state
            .write()
            .provider_models_requested
            .insert(provider.clone());
        spawn(async move {
            if let Err(e) = client
                .send(GatewayCommand::ProviderModelList { provider })
                .await
            {
                tracing::warn!("Failed to request provider model list: {}", e);
            }
        });
    });

    use_effect(move || {
        if *did_init_directories.read() {
            return;
        }
        did_init_directories.set(true);

        let current_dir = state.read().working_directory.clone().or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });
        if let Some(path) = current_dir {
            let options = build_directory_options(&path);
            let mut s = state.write();
            s.working_directory = Some(path);
            s.available_directories = options;
        }
    });

    // Re-fetch engine/model lists after an engine action completes, so the
    // Local Models dialog reflects installs/starts/pulls/removals.
    use_effect(move || {
        if !state.read().engines_stale {
            return;
        }
        state.write().engines_stale = false;
        if !state.read().show_engines_dialog {
            return;
        }
        let selected = state
            .read()
            .engines_data
            .as_ref()
            .and_then(|d| d.selected_engine.clone());
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("load engines", async move {
                client
                    .send(GatewayCommand::EngineList)
                    .await
                    .context("sending EngineList")?;
                if let Some(engine) = selected {
                    client
                        .send(GatewayCommand::EngineModelList { engine })
                        .await
                        .context("sending EngineModelList")?;
                }
                Ok(())
            });
        }
    });

    // Re-fetch panel lists after a mutation result marks them stale, so the
    // cron/memory/MCP/channels/tool dialogs reflect the change.
    use_effect(move || {
        let (cron, memory, mcp, channels, tools) = {
            let s = state.read();
            (
                s.cron_stale && s.show_cron_dialog,
                s.memory_stale && s.show_memory_dialog,
                s.mcp_stale && s.show_mcp_dialog,
                s.channels_stale && s.show_channels_dialog,
                s.tools_stale && s.show_tools_dialog,
            )
        };
        let any_stale = {
            let s = state.read();
            s.cron_stale || s.memory_stale || s.mcp_stale || s.channels_stale || s.tools_stale
        };
        if !any_stale {
            return;
        }
        {
            let mut s = state.write();
            s.cron_stale = false;
            s.memory_stale = false;
            s.mcp_stale = false;
            s.channels_stale = false;
            s.tools_stale = false;
        }
        if !(cron || memory || mcp || channels || tools) {
            return;
        }
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("refresh panels", async move {
                if cron {
                    client
                        .send(GatewayCommand::CronList)
                        .await
                        .context("sending CronList")?;
                }
                if memory {
                    client
                        .send(GatewayCommand::MemoryList {
                            query: None,
                            limit: None,
                        })
                        .await
                        .context("sending MemoryList")?;
                }
                if mcp {
                    client
                        .send(GatewayCommand::McpList)
                        .await
                        .context("sending McpList")?;
                }
                if channels {
                    client
                        .send(GatewayCommand::ChannelStatus)
                        .await
                        .context("sending ChannelStatus")?;
                }
                if tools {
                    client
                        .send(GatewayCommand::ToolConfigList)
                        .await
                        .context("sending ToolConfigList")?;
                }
                Ok(())
            });
        }
    });

    // Handle gateway events
    use_effect(move || {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            if active_event_client
                .read()
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &client))
            {
                return;
            }
            active_event_client.set(Some(client.clone()));

            // Shared buffer between the tokio worker and the
            // Dioxus UI task.  The worker pushes events at full
            // speed; the UI task drains the buffer when notified.
            let buffer: Arc<StdMutex<EventBuffer>> =
                Arc::new(StdMutex::new(EventBuffer::default()));
            let notify = Arc::new(tokio::sync::Notify::new());
            // End-of-stream, published by the worker once it has made its
            // final hand-off from the gateway channel into `buffer`.
            //
            // The connection flag cannot serve this purpose. The writer
            // clears it right after queueing `Disconnected` on the channel,
            // while moving that event into the buffer is the worker's job —
            // a separate task that need not have been polled yet. A consumer
            // watching the flag can therefore see "disconnected" with an
            // empty buffer while the terminal event is still in the channel,
            // and leave before it is ever handed over. Only the worker knows
            // when there is genuinely nothing left to deliver.
            let worker_done = Arc::new(std::sync::atomic::AtomicBool::new(false));

            // ── Worker (tokio thread) ──────────────────────────
            // Runs on the tokio runtime, completely independent
            // of the Dioxus virtualdom.  Never blocked by
            // rendering — the SSH reader will never stall.
            let client_w = client.clone();
            let buf_w = buffer.clone();
            let notify_w = notify.clone();
            let done_w = worker_done.clone();
            tokio::spawn(async move {
                loop {
                    // End of stream is the event channel closing, never the
                    // connection flag. A write failure kills only the writer;
                    // the reader stays live and keeps delivering. Quitting on
                    // the flag threw those remaining events away — fatally
                    // during login, where a refused pre-auth command marks the
                    // connection dead and the `AuthSuccess` that would close
                    // the TOTP prompt then never gets processed, locking the
                    // user out of an app that is still receiving frames.
                    //
                    // The client holds only the receiver, so both tasks
                    // dropping their senders is exactly "nothing more will
                    // ever arrive" — the honest signal to stop on.
                    let first = match client_w.recv().await {
                        Some(e) => e,
                        None => break,
                    };
                    let extra = client_w.drain_available().await;

                    buf_w
                        .lock()
                        .expect("stream buffer poisoned")
                        .push_events(std::iter::once(first).chain(extra));
                    notify_w.notify_one();
                }
                // Publish end-of-stream only after the final drain above, so
                // a consumer that observes it has necessarily been handed
                // everything. Then wake it one last time to come collect.
                done_w.store(true, std::sync::atomic::Ordering::SeqCst);
                notify_w.notify_one();
            });

            // ── UI updater (Dioxus task) ───────────────────────
            // Suspends on `notified().await`, which is a *true*
            // suspend — the virtualdom stops polling us and can
            // render.  When the worker signals new data, the
            // waker fires and we drain the buffer in one shot.
            let client_ui = client.clone();
            spawn(async move {
                let mut last_foreground_history_request: Option<u64> = None;
                let mut refreshed_threads_this_connection = false;
                loop {
                    notify.notified().await;

                    let entries = {
                        let mut b = buffer.lock().expect("stream buffer poisoned");
                        std::mem::take(&mut b.entries)
                    };

                    // Process entries in original order so that
                    // StreamStart → Chunks → ResponseDone sequencing
                    // is preserved.
                    for entry in entries {
                        match entry {
                            BufferEntry::Event {
                                event: GatewayEvent::DomQuery { id, js },
                                ..
                            } => {
                                // Off this task, not awaited in place. It
                                // evaluates script in the webview and then
                                // waits for room on the command queue, and
                                // this task is the only drain for gateway
                                // events — so either wait, held here, is a
                                // stalled connection. In its own task both
                                // are free to block: nothing else is waiting
                                // on them, so the reply may take as long as
                                // it takes.
                                //
                                // The page already ran the script; this is
                                // the answer going back. Losing it leaves the
                                // caller waiting on a reply that will never
                                // arrive, so it is worth naming.
                                let dom_client = client_ui.clone();
                                spawn(async move {
                                    if let Err(e) = handle_dom_query(&dom_client, id, js).await {
                                        tracing::error!(error = ?e, "Action failed");
                                    }
                                });
                            }
                            BufferEntry::Event { thread_id, event } => {
                                let triggers_refresh = matches!(
                                    event,
                                    GatewayEvent::Connected { .. }
                                        | GatewayEvent::AuthSuccess
                                        | GatewayEvent::VaultUnlocked
                                );
                                // On a fresh connection the gateway is a new session;
                                // reset the guard so the foreground thread's history
                                // is always fetched, even if the thread ID is unchanged.
                                let should_reset_history_guard = matches!(
                                    event,
                                    GatewayEvent::Connected { .. } | GatewayEvent::AuthSuccess
                                );
                                let history_target = match &event {
                                    GatewayEvent::ThreadsUpdate {
                                        foreground_id: Some(thread_id),
                                        ..
                                    } => Some(*thread_id),
                                    _ => None,
                                };
                                handle_gateway_event(thread_id, event, state);
                                if triggers_refresh && !refreshed_threads_this_connection {
                                    // Latch only on a send that actually left
                                    // the process. `Connected` can arrive while
                                    // a TOTP prompt is still up, and a gateway
                                    // that refuses pre-auth commands drops that
                                    // refresh — latching anyway left the
                                    // sidebar empty for the rest of the session
                                    // even after the user logged in. Same shape
                                    // as the history guard below. `AuthSuccess`
                                    // fires this again, so the retry is free.
                                    // `try_send`, never `send`: this task is
                                    // the only drain for gateway events, and
                                    // awaiting the command queue from here
                                    // deadlocks the connection outright (see
                                    // `GatewayClient::try_send`).
                                    match client_ui.try_send(GatewayCommand::ThreadList) {
                                        Ok(()) => refreshed_threads_this_connection = true,
                                        Err(e) => tracing::error!(
                                            error = %e,
                                            "Thread list refresh failed to send; \
                                             will retry on the next trigger"
                                        ),
                                    }
                                }
                                if should_reset_history_guard {
                                    last_foreground_history_request = None;
                                }
                                if let Some(thread_id) = history_target
                                    && last_foreground_history_request != Some(thread_id)
                                {
                                    tracing::info!(
                                        thread_id,
                                        previous = ?last_foreground_history_request,
                                        "Desktop requesting thread history after ThreadsUpdate"
                                    );
                                    // Only a request that actually went out
                                    // counts as made. The guard exists to stop
                                    // re-asking for the same thread, but a send
                                    // can fail — a write-dead connection fails
                                    // every one — and recording those left the
                                    // thread marked "already fetched" forever:
                                    // the sidebar kept its server-pushed
                                    // message count while the transcript that
                                    // was never requested stayed empty for the
                                    // rest of the session.
                                    // `try_send` for the same reason as the
                                    // refresh above. This is the send that
                                    // closed the loop in practice: it follows
                                    // the `ThreadsUpdate` at the end of a
                                    // turn, which is exactly when a long
                                    // turn's frames have filled the queues.
                                    match client_ui.try_send(GatewayCommand::ThreadHistoryRequest {
                                        thread_id,
                                    }) {
                                        Ok(()) => last_foreground_history_request = Some(thread_id),
                                        Err(e) => tracing::error!(
                                            thread_id,
                                            error = %e,
                                            "Thread history request failed to send; \
                                             leaving it to be retried"
                                        ),
                                    }
                                }
                            }
                            BufferEntry::Chunks {
                                thread_id,
                                text,
                                count,
                                bytes,
                            } => {
                                let mut s = state.write();
                                // Chunks belong to the turn that produced
                                // them, and the turn to its thread. A turn
                                // running in a thread the user is not looking
                                // at streams nowhere; its transcript arrives
                                // whole when it finishes.
                                if s.frame_targets_view(thread_id) {
                                    s.append_to_current_message(&text);
                                    s.streaming_chunks += count;
                                    s.streaming_bytes += bytes;
                                }
                            }
                        }
                    }

                    // Leave only once the worker has finished *and* its output
                    // is drained. `handle_gateway_event` is the only thing
                    // that clears the spinner and the requests that died with
                    // the connection, so an exit that skips buffered entries
                    // strands exactly the notice this loop exists to deliver.
                    //
                    // Deliberately not the connection flag: that goes false
                    // while the terminal event may still be in the channel,
                    // un-handed-over (see `worker_done`). Waiting on the
                    // worker instead makes the hand-off, not the connection's
                    // state, the thing that decides there is nothing left.
                    //
                    // Neither test alone suffices — the worker can still be
                    // pushing when the buffer momentarily empties, and it can
                    // finish while a batch sits unread — and this cannot park
                    // forever: every push is followed by a `notify_one`, as is
                    // the store above, and each drain takes *all* entries.
                    if worker_done.load(std::sync::atomic::Ordering::SeqCst) {
                        let drained = buffer
                            .lock()
                            .expect("stream buffer poisoned")
                            .entries
                            .is_empty();
                        if drained {
                            break;
                        }
                    }
                }
            });
        }
    });

    // Sync pending events from state into dialog signals
    use_effect(move || {
        let s = state.read();
        if let Some((_, id, name, args)) = s.pending_tool_approvals.front() {
            tool_approval_id.set(id.clone());
            tool_approval_name.set(name.clone());
            tool_approval_args.set(args.clone());
            show_tool_approval.set(true);
        } else {
            show_tool_approval.set(false);
        }

        if s.vault_locked && matches!(s.connection, ConnectionStatus::Connected) {
            show_vault_unlock.set(true);
        } else {
            show_vault_unlock.set(false);
        }

        if let Some((_, id, provider, secret, msg)) = s.pending_credential_requests.front() {
            cred_request_id.set(id.clone());
            cred_request_provider.set(provider.clone());
            cred_request_secret.set(secret.clone());
            cred_request_message.set(msg.clone());
            show_cred_request.set(true);
        } else {
            show_cred_request.set(false);
        }
    });

    // Reflect theme on the root element so CSS variables update.
    let theme_attr = state.read().theme.as_attr();
    let sidebar_collapsed = state.read().sidebar_collapsed;

    // Handlers
    let mut on_submit = move |message: String| {
        let attachments = state.read().prompt_attachments.clone();
        let prompt = build_prompt_with_attachments(&message, &attachments);
        {
            let mut s = state.write();
            s.add_user_message(prompt.clone());
            s.prompt_attachments.clear();
            // Records which thread owns the response, so its stream events
            // don't follow the user if they switch threads mid-response.
            s.mark_request_started();
        }

        // Name the thread on the wire. The message was typed into *this*
        // thread and belongs to it however long the frame takes to arrive,
        // and whatever the gateway's foreground is doing meanwhile.
        let turn_thread = state.read().foreground_thread_id;
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client
                    .chat_in_thread(prompt, turn_thread, Some(SessionOrigin::Desktop))
                    .await
                {
                    tracing::error!("Failed to send message: {}", e);
                }
            });
        }
    };

    let on_add_file_attachment = move |_| {
        let start_dir = state.read().working_directory.clone().or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });
        spawn(async move {
            let mut dialog = rfd::AsyncFileDialog::new();
            if let Some(dir) = start_dir {
                dialog = dialog.set_directory(dir);
            }
            if let Some(file) = dialog.pick_file().await {
                let path = file.path().display().to_string();
                let attachment = PromptAttachment::from_file_path(path.clone());
                let mut s = state.write();
                if !s
                    .prompt_attachments
                    .iter()
                    .any(|item| item.path == attachment.path)
                {
                    s.prompt_attachments.push(attachment);
                }
                s.push_notice(MessageRole::Info, format!("Attached file {}", path));
            }
        });
    };

    let on_add_directory_attachment = move |_| {
        let start_dir = state.read().working_directory.clone().or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });
        spawn(async move {
            let mut dialog = rfd::AsyncFileDialog::new();
            if let Some(dir) = start_dir {
                dialog = dialog.set_directory(dir);
            }
            if let Some(folder) = dialog.pick_folder().await {
                let path = folder.path().display().to_string();
                let attachment = PromptAttachment::from_directory_path(path.clone());
                let mut s = state.write();
                if !s
                    .prompt_attachments
                    .iter()
                    .any(|item| item.path == attachment.path)
                {
                    s.prompt_attachments.push(attachment);
                }
                s.push_notice(MessageRole::Info, format!("Attached directory {}", path));
            }
        });
    };

    let on_remove_attachment = move |path: String| {
        let mut s = state.write();
        let before = s.prompt_attachments.len();
        s.prompt_attachments.retain(|item| item.path != path);
        if s.prompt_attachments.len() != before {
            s.push_notice(MessageRole::Info, format!("Removed attachment {}", path));
        }
    };

    // Create a new thread in a specific project (the sidebar's per-project +).
    let on_new_thread_in = move |project_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("create thread", async move {
                client
                    .send(GatewayCommand::ThreadCreate {
                        label: None,
                        project_id: Some(project_id),
                    })
                    .await
                    .context("sending ThreadCreate")?;
                Ok(())
            });
        }
        // Save current thread's messages and start with empty chat.
        // The gateway will assign a new foreground via ThreadsUpdate.
        let mut s = state.write();
        if let Some(current_id) = s.foreground_thread_id
            && !s.messages.is_empty()
        {
            let msgs = s.messages.clone();
            s.save_thread_messages(current_id, msgs);
        }
        s.messages.clear();
    };

    let on_new_project = move |_| show_new_project.set(true);

    let on_switch_project = move |project_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client
                    .send(GatewayCommand::ProjectSwitch { project_id })
                    .await
                {
                    tracing::error!(error = %e, "ProjectSwitch send failed");
                }
            });
        }
    };

    let on_rename_project = move |(project_id, new_name): (u64, String)| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("rename project", async move {
                client
                    .send(GatewayCommand::ProjectRename {
                        project_id,
                        new_name,
                    })
                    .await
                    .context("sending ProjectRename")?;
                Ok(())
            });
        }
    };

    let on_edit_project = move |project_id: u64| edit_project.set(Some(project_id));

    let on_delete_project = move |project_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("delete project", async move {
                client
                    .send(GatewayCommand::ProjectDelete { project_id })
                    .await
                    .context("sending ProjectDelete")?;
                Ok(())
            });
        }
    };

    let on_switch_thread = move |thread_id: u64| {
        // `switch_thread` resets the view for the incoming thread.
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("open thread", async move {
                client
                    .send(GatewayCommand::ThreadSwitch { thread_id })
                    .await
                    .with_context(|| format!("switching to thread {thread_id}"))?;
                tracing::info!(
                    thread_id,
                    "Desktop requesting thread history after ThreadSwitch"
                );
                // Stopping here matters: the view has already been reset for
                // the incoming thread, so a history request that never leaves
                // the process is the empty transcript the user ends up
                // looking at.
                client
                    .send(GatewayCommand::ThreadHistoryRequest { thread_id })
                    .await
                    .with_context(|| format!("requesting history for thread {thread_id}"))?;
                Ok(())
            });
        }
        state.write().switch_thread(thread_id);
    };

    let on_rename_thread = move |(thread_id, new_label): (u64, String)| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("rename thread", async move {
                client
                    .send(GatewayCommand::ThreadRename {
                        thread_id,
                        new_label,
                    })
                    .await
                    .context("sending ThreadRename")?;
                Ok(())
            });
        }
    };

    let on_edit_thread = move |thread_id: u64| edit_thread.set(Some(thread_id));

    let on_delete_thread = move |thread_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("close thread", async move {
                client
                    .send(GatewayCommand::ThreadClose { thread_id })
                    .await
                    .context("sending ThreadClose")?;
                Ok(())
            });
        }
    };

    let on_cancel = move |_| {
        let mut s = state.write();
        s.push_notice(MessageRole::Info, "Cancellation requested…");
        // Names the turn it means: the gateway cannot resolve "the current
        // one" for us once turns run per thread. Read and retire together —
        // see `stop_current_turn` for why the order is not a call-site
        // detail.
        let thread_id = s.stop_current_turn();
        drop(s);
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("cancel turn", async move {
                client
                    .send(GatewayCommand::Cancel { thread_id })
                    .await
                    .context("sending Cancel")?;
                Ok(())
            });
        }
    };

    // Structured answers for the inline agent-question card (`ask_user` tool).
    let on_prompt_respond = move |(id, value): (String, PromptResponseValue)| {
        // Retire only the question being answered — the one the visible
        // card showed. Questions queue per thread now, and clearing the
        // lot (or the oldest holder of a colliding id, which can be a
        // hidden thread's question) would discard another turn's question
        // unanswered — it would wait out its five-minute window on a card
        // the user was never shown again.
        state.write().answer_user_prompt(&id);
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("answer prompt", async move {
                client
                    .send(GatewayCommand::UserPromptResponse {
                        id,
                        dismissed: false,
                        value,
                    })
                    .await
                    .context("sending UserPromptResponse")?;
                Ok(())
            });
        }
    };

    let on_prompt_dismiss = move |id: String| {
        // Dismissal is an answer too: it retires its own card only.
        state.write().answer_user_prompt(&id);
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("answer prompt", async move {
                client
                    .send(GatewayCommand::UserPromptResponse {
                        id,
                        dismissed: true,
                        value: PromptResponseValue::Text(String::new()),
                    })
                    .await
                    .context("sending UserPromptResponse")?;
                Ok(())
            });
        }
    };

    // Secrets dialog event handler

    // ── Native OS menu event handler ──────────────────────────────────────
    use dioxus::desktop::use_muda_event_handler;
    use_muda_event_handler(move |event| {
        if let Some(ids) = crate::menu::app_menu_ids() {
            if event.id == ids.new_thread {
                let gw = gateway.read().clone();
                if let Some(client) = gw {
                    spawn_reporting("create thread", async move {
                        client
                            .send(GatewayCommand::ThreadCreate {
                                label: None,
                                project_id: None,
                            })
                            .await
                            .context("sending ThreadCreate")?;
                        Ok(())
                    });
                }
                let mut s = state.write();
                if let Some(current_id) = s.foreground_thread_id
                    && !s.messages.is_empty()
                {
                    let msgs = s.messages.clone();
                    s.save_thread_messages(current_id, msgs);
                }
                s.messages.clear();
            } else if event.id == ids.new_connection_window {
                crate::spawn_connection_window();
            } else if event.id == ids.toggle_left_sidebar {
                let v = state.read().left_sidebar_visible;
                state.write().left_sidebar_visible = !v;
            } else if event.id == ids.toggle_right_sidebar {
                let v = state.read().plugin_dock_visible;
                state.write().plugin_dock_visible = !v;
            } else if event.id == ids.settings {
                show_settings.set(true);
            } else if event.id == ids.messengers {
                let mut show_messengers = show_messengers;
                show_messengers.set(true);
                let gw = gateway.read().clone();
                if let Some(client) = gw {
                    spawn_reporting("load messenger config", async move {
                        client
                            .send(GatewayCommand::MessengerConfig)
                            .await
                            .context("sending MessengerConfig")?;
                        Ok(())
                    });
                }
            } else if event.id == ids.secrets {
                show_secrets.set(true);
                let gw = gateway.read().clone();
                if let Some(client) = gw {
                    spawn_reporting("load secrets", async move {
                        client
                            .send(GatewayCommand::SecretsList)
                            .await
                            .context("sending SecretsList")?;
                        client
                            .send(GatewayCommand::SecretsHasTotp)
                            .await
                            .context("sending SecretsHasTotp")?;
                        Ok(())
                    });
                }
            } else if event.id == ids.pair {
                show_pairing.set(true);
            } else if event.id == ids.swarm {
                show_swarm.set(true);
            } else if event.id == ids.skills {
                let skills = crate::app_support::load_skills_list();
                let mut s = state.write();
                s.skills_data = skills;
                s.show_skills_dialog = !s.show_skills_dialog;
            } else if event.id == ids.system_info {
                let v = state.read().show_system_info;
                state.write().show_system_info = !v;
                if !v {
                    // Opening: fetch host capabilities (once) and a fresh
                    // load sample so the panel has data to show.
                    let need_host = state.read().host_info.is_none();
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load host status", async move {
                            if need_host {
                                client
                                    .send(GatewayCommand::HostInfoRequest)
                                    .await
                                    .context("sending HostInfoRequest")?;
                            }
                            client
                                .send(GatewayCommand::LoadStatusRequest)
                                .await
                                .context("sending LoadStatusRequest")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.downloads {
                let v = state.read().show_downloads_dialog;
                state.write().show_downloads_dialog = !v;
                if !v {
                    // Opening: ask for the current list. Later changes arrive
                    // unprompted, so this is the only request the panel makes.
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load downloads", async move {
                            client
                                .send(GatewayCommand::DownloadsRequest)
                                .await
                                .context("sending DownloadsRequest")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.services {
                let v = state.read().show_services_dialog;
                state.write().show_services_dialog = !v;
                if !v {
                    // Opening: fetch the service list.
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load services", async move {
                            client
                                .send(GatewayCommand::ServiceList)
                                .await
                                .context("sending ServiceList")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.local_models {
                let v = state.read().show_engines_dialog;
                state.write().show_engines_dialog = !v;
                if !v {
                    // Opening: fetch the engine list (and host info for the
                    // resource header, if we don't have it yet).
                    let need_host = state.read().host_info.is_none();
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load engines", async move {
                            client
                                .send(GatewayCommand::EngineList)
                                .await
                                .context("sending EngineList")?;
                            if need_host {
                                client
                                    .send(GatewayCommand::HostInfoRequest)
                                    .await
                                    .context("sending HostInfoRequest")?;
                            }
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.cron {
                let v = state.read().show_cron_dialog;
                state.write().show_cron_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load cron jobs", async move {
                            client
                                .send(GatewayCommand::CronList)
                                .await
                                .context("sending CronList")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.memory {
                let v = state.read().show_memory_dialog;
                state.write().show_memory_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load memory", async move {
                            client
                                .send(GatewayCommand::MemoryList {
                                    query: None,
                                    limit: None,
                                })
                                .await
                                .context("sending MemoryList")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.mcp {
                let v = state.read().show_mcp_dialog;
                state.write().show_mcp_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load MCP servers", async move {
                            client
                                .send(GatewayCommand::McpList)
                                .await
                                .context("sending McpList")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.channels {
                let v = state.read().show_channels_dialog;
                state.write().show_channels_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load channels", async move {
                            client
                                .send(GatewayCommand::ChannelStatus)
                                .await
                                .context("sending ChannelStatus")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.tool_perms {
                let v = state.read().show_tools_dialog;
                state.write().show_tools_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load tool config", async move {
                            client
                                .send(GatewayCommand::ToolConfigList)
                                .await
                                .context("sending ToolConfigList")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.analytics {
                let v = state.read().show_analytics_dialog;
                state.write().show_analytics_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load usage stats", async move {
                            client
                                .send(GatewayCommand::UsageStats { period: None })
                                .await
                                .context("sending UsageStats")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.logs {
                let v = state.read().show_logs_dialog;
                state.write().show_logs_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("load logs", async move {
                            client
                                .send(GatewayCommand::Logs {
                                    source: "gateway".into(),
                                    tail: None,
                                })
                                .await
                                .context("sending Logs")?;
                            Ok(())
                        });
                    }
                }
            } else if event.id == ids.quit {
                dioxus::desktop::window().close();
            }
        }
    });

    // Top-bar title: "Project — Thread" for the active project / foreground thread.
    let topbar_title = {
        let s = state.read();
        let proj = s
            .projects
            .iter()
            .find(|p| p.id == s.active_project_id)
            .map(|p| p.name.clone());
        let thread = s
            .foreground_thread_id
            .and_then(|id| s.threads.iter().find(|t| t.id == id))
            .and_then(|t| t.label.clone());
        match (proj, thread) {
            (Some(p), Some(t)) => format!("{p} — {t}"),
            (Some(p), None) => p,
            (None, Some(t)) => t,
            (None, None) => "RustyClaw".to_string(),
        }
    };

    // Apply a picked directory both locally and on the gateway, so the
    // thread's effective working directory follows the picker. Shared by the
    // recent-favourites list and the "other…" file dialog.
    let mut apply_directory = move |path: String| match std::env::set_current_dir(&path) {
        Ok(()) => {
            let options = build_directory_options(&path);
            {
                let mut s = state.write();
                s.working_directory = Some(path.clone());
                s.available_directories = options;
                s.directory_selector_expanded = false;
                s.directory_selector_error = None;
                s.push_notice(
                    MessageRole::Info,
                    format!("Working directory set to {}", display_path(&path)),
                );
            }
            let gw = gateway.read().clone();
            if let Some(client) = gw {
                let p = std::path::PathBuf::from(path);
                spawn(async move {
                    if let Err(e) = client
                        .send(GatewayCommand::SetWorkingDirectory { path: p })
                        .await
                    {
                        tracing::error!(error = %e, "SetWorkingDirectory send failed");
                    }
                });
            }
        }
        Err(e) => {
            state.write().directory_selector_error =
                Some(format!("Failed to change directory: {}", e));
        }
    };

    // The sidebar tree, with each row's state icon resolved from the
    // client-side knowledge the gateway's status string does not carry
    // (pending prompts, locally-known in-flight turns).
    let sidebar_tree = {
        let s = state.read();
        let mut tree =
            rustyclaw_view::SidebarTree::build(&s.projects, &s.threads, s.active_project_id);
        for group in &mut tree.groups {
            for item in &mut group.threads {
                item.state = s.thread_state(item.id);
            }
        }
        tree
    };

    rsx! {
        style { dangerous_inner_html: BULMA }
        style { dangerous_inner_html: STYLES }

        div {
            id: "rc-root",
            class: "app",
            "data-theme": "{theme_attr}",

            // ── Top bar: sidebar toggles + global actions ──────────────────
            div { class: "rc-tab-row",
                Button {
                    color: BulmaColor::Ghost,
                    size: BulmaSize::Small,
                    class: "sidebar-toggle-btn",
                    onclick: move |_| {
                        let v = state.read().left_sidebar_visible;
                        state.write().left_sidebar_visible = !v;
                    },
                    "☰"
                }
                // The sidebar is now the sole thread/project navigation; the
                // active thread/project title fills the top bar.
                div { class: "rc-topbar-title", "{topbar_title}" }
                Buttons { class: "rc-tab-actions", addons: true,
                    Button {
                        color: BulmaColor::Ghost,
                        size: BulmaSize::Small,
                        class: "icon-btn",
                        onclick: move |_| {
                            show_secrets.set(true);
                            let gw = gateway.read().clone();
                            if let Some(client) = gw {
                                spawn_reporting("load secrets", async move {
                                    client.send(GatewayCommand::SecretsList).await.context("sending SecretsList")?;
                        client.send(GatewayCommand::SecretsHasTotp).await.context("sending SecretsHasTotp")?;
                                    Ok(())
                                });
                            }
                        },
                        "🔑"
                    }
                    Button {
                        color: BulmaColor::Ghost,
                        size: BulmaSize::Small,
                        class: "icon-btn",
                        onclick: move |_| show_swarm.set(true),
                        "🐝"
                    }
                    Button {
                        color: BulmaColor::Ghost,
                        size: BulmaSize::Small,
                        class: "icon-btn",
                        onclick: move |_| show_settings.set(true),
                        "⚙"
                    }
                }
                Button {
                    color: BulmaColor::Ghost,
                    size: BulmaSize::Small,
                    class: "sidebar-toggle-btn",
                    onclick: move |_| {
                        let v = state.read().plugin_dock_visible;
                        state.write().plugin_dock_visible = !v;
                    },
                    "◫"
                }
            }

            // ── Workspace: sidebar + main content + plugin dock ─────────────
            div { class: "rc-workspace",
                if state.read().left_sidebar_visible {
                    Sidebar {
                        connection: state.read().connection.clone(),
                        agent_name: state.read().agent_name.clone(),
                        model: state.read().model.clone(),
                        provider: state.read().provider.clone(),
                        collapsed: sidebar_collapsed,
                        on_toggle_collapse: move |_| {
                            let v = state.read().sidebar_collapsed;
                            state.write().sidebar_collapsed = !v;
                        },
                        on_switch_thread: on_switch_thread,
                        on_rename_thread: on_rename_thread,
                        on_edit_thread: on_edit_thread,
                        on_delete_thread: on_delete_thread,
                        on_new_thread_in: on_new_thread_in,
                        on_new_project: on_new_project,
                        on_switch_project: on_switch_project,
                        on_rename_project: on_rename_project,
                        on_edit_project: on_edit_project,
                        on_delete_project: on_delete_project,
                        tree: sidebar_tree,
                        foreground_id: state.read().foreground_thread_id,
                        on_pair: move |_| show_pairing.set(true),
                        on_secrets: move |_| {
                            show_secrets.set(true);
                            let gw = gateway.read().clone();
                            if let Some(client) = gw {
                                spawn_reporting("load secrets", async move {
                                    client.send(GatewayCommand::SecretsList).await.context("sending SecretsList")?;
                        client.send(GatewayCommand::SecretsHasTotp).await.context("sending SecretsHasTotp")?;
                                    Ok(())
                                });
                            }
                        },
                        on_settings: move |_| show_settings.set(true),
                        on_local_models: move |_| {
                            state.write().show_engines_dialog = true;
                            let need_host = state.read().host_info.is_none();
                            let gw = gateway.read().clone();
                            if let Some(client) = gw {
                                spawn_reporting("load engines", async move {
                                    client.send(GatewayCommand::EngineList).await.context("sending EngineList")?;
                                    if need_host {
                                        client.send(GatewayCommand::HostInfoRequest).await.context("sending HostInfoRequest")?;
                                    }
                                    Ok(())
                                });
                            }
                        },
                    }
                }

            div { class: "main",
                // Connection banners (connecting / lost link). Transient
                // status and error text now lands inline in the transcript
                // as notice messages, so only live connection state renders
                // as a top banner.
                for banner in rustyclaw_view::build_banners(
                    &state.read().connection,
                    None,
                ) {
                    Notification {
                        color: crate::components::tone_color(banner.tone),
                        light: true,
                        class: "banner",
                        span { class: "banner-text",
                            if !banner.icon.is_empty() {
                                "{banner.icon} "
                            }
                            "{banner.text}"
                        }
                        if !banner.actions.is_empty() {
                            Buttons { class: "banner-actions",
                                for (action, banner_text) in banner
                                    .actions
                                    .iter()
                                    .cloned()
                                    .map(|a| (a, banner.text.clone()))
                                {
                                    Button {
                                        color: BulmaColor::Ghost,
                                        size: BulmaSize::Small,
                                        onclick: move |_| match action.kind {
                                            BannerActionKind::Reconnect => do_reconnect(sig),
                                            BannerActionKind::PairGateway => show_pairing.set(true),
                                            BannerActionKind::DismissStatus => {
                                                // Status text now lives inline in the
                                                // transcript; connection banners have
                                                // no dismissable status to clear.
                                            }
                                            BannerActionKind::CopyText => {
                                                crate::components::copy_to_clipboard(
                                                    banner_text.clone(),
                                                );
                                            }
                                        },
                                        "{action.label}"
                                    }
                                }
                            }
                        }
                    }
                }

                // Apply a picked directory both locally and on the gateway,
                // so the thread's effective working directory follows the
                // picker. Shared by the recent-favourites list and the
                // "other…" file dialog.
                Chat {
                    messages: state.read().messages.iter().cloned().collect::<Vec<_>>(),
                    surface: rustyclaw_view::ChatSurfaceData {
                        is_processing: state.read().is_processing,
                        is_thinking: state.read().is_thinking,
                        is_streaming: state.read().is_streaming,
                        streaming_chunks: state.read().streaming_chunks,
                        streaming_bytes: state.read().streaming_bytes,
                        elapsed: None,
                        spinner_tick: 0,
                    },
                    bottom_bar: rustyclaw_view::BottomBarData {
                        composer: rustyclaw_view::ComposerData {
                            is_processing: state.read().is_processing,
                            current_provider: state.read().provider.clone(),
                            current_model: state.read().model.clone(),
                            attachments: state.read().prompt_attachments.clone(),
                        },
                        directory_selector: rustyclaw_view::DirectorySelectorState {
                            current_path: state.read().working_directory.clone(),
                            current_display: state
                                .read()
                                .working_directory
                                .clone()
                                .as_deref()
                                .map(display_path),
                            available_directories: state.read().available_directories.clone(),
                            is_expanded: state.read().directory_selector_expanded,
                            error: state.read().directory_selector_error.clone(),
                        },
                    },
                    agent_name: state.read().agent_name.clone(),
                    pending_prompt: state.read().visible_user_prompt(),
                    provider_models: state.read().provider_models.clone(),
                    on_submit: on_submit,
                    on_cancel: on_cancel,
                    on_prompt_respond: on_prompt_respond,
                    on_prompt_dismiss: on_prompt_dismiss,
                    on_model_change: move |(provider, model): (String, String)| {
                        let prov_clone = provider.clone();
                        let model_clone = model.clone();
                        let gw = gateway.read().clone();
                        if let Some(client) = gw {
                            spawn(async move {
                                if let Err(e) = client.send(GatewayCommand::ModelSwitch {
                                    provider: prov_clone,
                                    model: model_clone,
                                }).await {
                                    tracing::error!("Failed to send model switch: {}", e);
                                }
                            });
                        }
                        let mut s = state.write();
                        // Re-arm the live model fetch for this provider if
                        // no list is cached yet (e.g. an earlier fetch
                        // failed) — an explicit switch is the retry point.
                        if !s.provider_models.contains_key(&provider) {
                            s.provider_models_requested.remove(&provider);
                        }
                        s.provider = Some(provider);
                        s.model = Some(model);
                    },
                    on_add_provider: move |_| show_settings.set(true),
                    on_add_file_attachment: on_add_file_attachment,
                    on_add_directory_attachment: on_add_directory_attachment,
                    on_remove_attachment: on_remove_attachment,

                    on_select_directory: move |path: String| {
                        if path == DIRECTORY_OTHER_SENTINEL {
                            let start_dir = state
                                .read()
                                .working_directory
                                .clone()
                                .unwrap_or_else(|| ".".to_string());
                            spawn(async move {
                                match rfd::AsyncFileDialog::new()
                                    .set_directory(start_dir)
                                    .pick_folder()
                                    .await
                                {
                                    Some(folder) => apply_directory(
                                        folder.path().display().to_string(),
                                    ),
                                    None => state.write().directory_selector_expanded = false,
                                }
                            });
                            return;
                        }
                        apply_directory(path);
                    },
                }
            }

                // Plugin dock. It only renders when the user has opened it or a
                // plugin is actually loaded — an install with no plugins keeps
                // the full width for chat instead of an empty panel.
                if state.read().plugin_dock_visible {
                    aside { class: "sidebar plugin-dock",
                        crate::components::PluginPanel {
                            plugins: state.read().plugins.clone(),
                            native: Vec::new(),
                            active_plugin: state.read().active_plugin.clone(),
                            on_select_plugin: move |name: String| {
                                state.write().active_plugin = Some(name);
                            },
                            on_action: move |event: PluginActionEvent| {
                                // Plugin actions are declared for the *agent* to
                                // carry out — the manager has no executor of its
                                // own — so clicking one asks the agent to run it
                                // rather than pretending the UI can.
                                on_submit(format!(
                                    "Run the `{}` action on the `{}` plugin.",
                                    event.action_name, event.plugin_name
                                ));
                            },
                            on_refresh: move |name: String| {
                                let gw = gateway.read().clone();
                                if let Some(client) = gw {
                                    spawn(async move {
                                        if let Err(e) = client
                                            .send(GatewayCommand::PluginRefresh { plugin_name: name })
                                            .await
                                        {
                                            tracing::error!(error = %e, "PluginRefresh send failed");
                                        }
                                    });
                                }
                            },
                        }
                    }
                }
            }

            // New-project dialog
            NewProjectDialog {
                visible: show_new_project(),
                on_cancel: move |_| show_new_project.set(false),
                on_create: move |(name, path): (String, std::path::PathBuf)| {
                    show_new_project.set(false);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("create project", async move {
                            client.send(GatewayCommand::ProjectCreate { name, path }).await.context("sending ProjectCreate")?;
                            Ok(())
                        });
                    }
                },
            }

            // Edit dialogs. Rendered from an Option so each open remounts with
            // the row's current values; a row that vanished (deleted from
            // another client) simply renders nothing.
            if let Some(project) = edit_project()
                .and_then(|id| state.read().projects.iter().find(|p| p.id == id).cloned())
            {
                EditProjectDialog {
                    project_id: project.id,
                    name: project.name.clone(),
                    path: project.path.clone(),
                    on_cancel: move |_| edit_project.set(None),
                    on_save: move |(project_id, name, path): (u64, String, std::path::PathBuf)| {
                        edit_project.set(None);
                        let gw = gateway.read().clone();
                        if let Some(client) = gw {
                            spawn(async move {
                                if let Err(e) = client
                                    .send(GatewayCommand::ProjectUpdate { project_id, name, path })
                                    .await
                                {
                                    tracing::error!(project_id, error = %e, "ProjectUpdate send failed");
                                }
                            });
                        }
                    },
                }
            }

            {
                edit_thread()
                    .and_then(|id| state.read().threads.iter().find(|t| t.id == id).cloned())
                    .map(|thread| {
                        // A thread with project_id 0 belongs to the active project.
                        let project_id = if thread.project_id == 0 {
                            state.read().active_project_id
                        } else {
                            thread.project_id
                        };
                        let project_path = state
                            .read()
                            .projects
                            .iter()
                            .find(|p| p.id == project_id)
                            .map(|p| p.path.clone())
                            .unwrap_or_default();
                        rsx! {
                            EditThreadDialog {
                    thread_id: thread.id,
                    label: thread.label.clone().unwrap_or_default(),
                    working_dir: thread.working_dir.clone(),
                    project_path,
                    on_cancel: move |_| edit_thread.set(None),
                    on_save: move |(thread_id, label, working_dir): (
                        u64,
                        String,
                        Option<std::path::PathBuf>,
                    )| {
                        edit_thread.set(None);
                        let gw = gateway.read().clone();
                        if let Some(client) = gw {
                            spawn(async move {
                                if let Err(e) = client
                                    .send(GatewayCommand::ThreadUpdate { thread_id, label, working_dir })
                                    .await
                                {
                                    tracing::error!(thread_id, error = %e, "ThreadUpdate send failed");
                                }
                            });
                        }
                    },
                            }
                        }
                    })
            }

            // Modals
            {render_dialogs(sig)}
        }
    }
}
