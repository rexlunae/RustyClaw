//! Top-level application component.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons, Notification};
use rustyclaw_view::{tokio, tracing};
use std::sync::{Arc, Mutex as StdMutex};

use crate::components::{
    Chat, EditProjectDialog, EditThreadDialog, NewProjectDialog, PluginActionEvent, Sidebar,
};

use crate::app_support::*;
use crate::state::{AppState, PendingWorkspaceChange};
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_core::user_prompt_types::PromptResponseValue;

use rustyclaw_view::{
    BannerActionKind, HatchingDialogData, PromptAttachment, build_prompt_with_attachments,
};

mod dialogs;
mod signals;

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

    // Keep the editor's root file listing in step with the workspace.
    //
    // The gateway only sends a listing in reply to a request, and the editor
    // component stays mounted across thread switches, project switches, and
    // reconnects — so a mount-time effect inside it would fire once and leave
    // the tree blank forever after the first reset. Watching the generation
    // counter here covers every reset path, including reconnect, which has no
    // gateway handle of its own. Tracking the generation rather than "is the
    // cache empty" means a directory that really is empty, or one whose
    // listing failed, does not re-request in a loop.
    let mut listed_generation = use_signal(|| None::<u64>);
    use_effect(move || {
        let (generation, connected) = {
            let s = state.read();
            (
                s.workspace.generation(),
                matches!(
                    s.connection,
                    ConnectionStatus::Connected | ConnectionStatus::Authenticated
                ),
            )
        };
        if !connected || *listed_generation.read() == Some(generation) {
            return;
        }
        listed_generation.set(Some(generation));
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client
                    .send(GatewayCommand::WorkspaceListDir {
                        path: std::path::PathBuf::new(),
                    })
                    .await
                {
                    tracing::error!(error = %e, "Root workspace listing failed to send");
                }
            });
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
            if let Some(root) = s.working_directory.as_deref() {
                s.file_browser = rustyclaw_view::FileBrowserData::load(root);
            }
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
            spawn(async move {
                let _ = client.send(GatewayCommand::EngineList).await;
                if let Some(engine) = selected {
                    let _ = client
                        .send(GatewayCommand::EngineModelList { engine })
                        .await;
                }
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
            spawn(async move {
                if cron {
                    let _ = client.send(GatewayCommand::CronList).await;
                }
                if memory {
                    let _ = client
                        .send(GatewayCommand::MemoryList {
                            query: None,
                            limit: None,
                        })
                        .await;
                }
                if mcp {
                    let _ = client.send(GatewayCommand::McpList).await;
                }
                if channels {
                    let _ = client.send(GatewayCommand::ChannelStatus).await;
                }
                if tools {
                    let _ = client.send(GatewayCommand::ToolConfigList).await;
                }
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

            // ── Worker (tokio thread) ──────────────────────────
            // Runs on the tokio runtime, completely independent
            // of the Dioxus virtualdom.  Never blocked by
            // rendering — the SSH reader will never stall.
            let client_w = client.clone();
            let buf_w = buffer.clone();
            let notify_w = notify.clone();
            tokio::spawn(async move {
                loop {
                    if !client_w.is_connected() {
                        break;
                    }
                    let first = match client_w.recv().await {
                        Some(e) => e,
                        None => break,
                    };
                    let extra = client_w.drain_available().await;

                    {
                        let mut b = buf_w.lock().expect("stream buffer poisoned");
                        for event in std::iter::once(first).chain(extra) {
                            match event {
                                GatewayEvent::Chunk { delta } => {
                                    // Coalesce consecutive chunks into one entry.
                                    if let Some(BufferEntry::Chunks {
                                        text, count, bytes, ..
                                    }) = b.entries.last_mut()
                                    {
                                        *count += 1;
                                        *bytes += delta.len();
                                        text.push_str(&delta);
                                    } else {
                                        b.entries.push(BufferEntry::Chunks {
                                            text: delta.clone(),
                                            count: 1,
                                            bytes: delta.len(),
                                        });
                                    }
                                }
                                other => b.entries.push(BufferEntry::Event(other)),
                            }
                        }
                    }
                    notify_w.notify_one();
                }
                // Final wake so the UI task can observe disconnect.
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

                    if !client.is_connected() {
                        break;
                    }

                    let entries = {
                        let mut b = buffer.lock().expect("stream buffer poisoned");
                        std::mem::take(&mut b.entries)
                    };

                    // Process entries in original order so that
                    // StreamStart → Chunks → ResponseDone sequencing
                    // is preserved.
                    for entry in entries {
                        match entry {
                            BufferEntry::Event(GatewayEvent::DomQuery { id, js }) => {
                                handle_dom_query(&client_ui, id, js).await;
                            }
                            BufferEntry::Event(event) => {
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
                                handle_gateway_event(event, state);
                                if triggers_refresh && !refreshed_threads_this_connection {
                                    refreshed_threads_this_connection = true;
                                    let _ = client_ui.send(GatewayCommand::ThreadList).await;
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
                                    let _ = client_ui
                                        .send(GatewayCommand::ThreadHistoryRequest { thread_id })
                                        .await;
                                    last_foreground_history_request = Some(thread_id);
                                }
                            }
                            BufferEntry::Chunks { text, count, bytes } => {
                                let mut s = state.write();
                                // Chunks belong to the thread that submitted
                                // the request; don't stream into a thread the
                                // user has switched to in the meantime.
                                if s.stream_targets_foreground() {
                                    s.append_to_current_message(&text);
                                    s.streaming_chunks += count;
                                    s.streaming_bytes += bytes;
                                }
                            }
                        }
                    }
                }
            });
        }
    });

    // Sync pending events from state into dialog signals
    use_effect(move || {
        let s = state.read();
        if let Some((id, name, args)) = &s.pending_tool_approval {
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

        if let Some((id, provider, secret, msg)) = &s.pending_credential_request {
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
        let turn_thread = state.read().streaming_thread_id;
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client.chat_in_thread(prompt, turn_thread).await {
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
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ThreadCreate {
                        label: None,
                        project_id: Some(project_id),
                    })
                    .await;
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

    /// Say what unsaved work a workspace move discarded, if any.
    ///
    /// Take the list as an argument rather than doing the rebase here: a
    /// caller must bind the rebase result to a variable first, so the write
    /// guard is released before this takes its own.
    fn warn_dropped(mut state: Signal<AppState>, dropped: Vec<std::path::PathBuf>) {
        if dropped.is_empty() {
            return;
        }
        let names: Vec<String> = dropped.iter().map(|p| p.display().to_string()).collect();
        state.write().push_notice(
            MessageRole::Warning,
            format!(
                "Working directory changed with unsaved editor changes; discarded: {}",
                names.join(", ")
            ),
        );
    }

    // ── Workspace changes and unsaved editor work ───────────────────────
    //
    // Anything that repoints the working directory invalidates every path the
    // editor holds, so unsaved changes cannot come along. `guard` defers the
    // change behind a prompt when there is work at stake; `apply` performs it
    // once there is nothing to lose (or the user has said to go ahead).
    let mut apply_workspace_change = move |change: PendingWorkspaceChange| {
        let gw = gateway.read().clone();
        match change {
            // The reset happens inside the success arm only: a directory that
            // fails to open has not moved anything, and discarding the
            // editor's work for a change that did not happen would be a
            // gratuitous loss.
            PendingWorkspaceChange::Directory(path) => match std::env::set_current_dir(&path) {
                Ok(()) => {
                    let dropped = state.write().workspace.rebase(path.clone().into());
                    warn_dropped(state, dropped);
                    let options = build_directory_options(&path);
                    {
                        let mut s = state.write();
                        s.working_directory = Some(path.clone());
                        s.available_directories = options;
                        s.file_browser = rustyclaw_view::FileBrowserData::load(&path);
                        s.directory_selector_expanded = false;
                        s.directory_selector_error = None;
                        s.push_notice(
                            MessageRole::Info,
                            format!("Working directory set to {}", display_path(&path)),
                        );
                    }
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
            },
            PendingWorkspaceChange::Project(project_id) => {
                let dropped = state.write().workspace.rebase_to_current_thread();
                warn_dropped(state, dropped);
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
            }
            PendingWorkspaceChange::Thread(thread_id) => {
                // `switch_thread` resets the view itself.
                if let Some(client) = gw {
                    spawn(async move {
                        let _ = client
                            .send(GatewayCommand::ThreadSwitch { thread_id })
                            .await;
                        tracing::info!(
                            thread_id,
                            "Desktop requesting thread history after ThreadSwitch"
                        );
                        let _ = client
                            .send(GatewayCommand::ThreadHistoryRequest { thread_id })
                            .await;
                    });
                }
                state.write().switch_thread(thread_id);
            }
        }
    };

    // Apply now when nothing is at stake; otherwise park it behind the prompt.
    let mut guard_workspace_change = move |change: PendingWorkspaceChange| {
        if state.read().workspace.unsaved().is_empty() {
            apply_workspace_change(change);
        } else {
            state.write().pending_workspace_change = Some(change);
        }
    };

    let on_switch_project = move |project_id: u64| {
        // A different project means a different directory.
        guard_workspace_change(PendingWorkspaceChange::Project(project_id));
    };

    let on_rename_project = move |(project_id, new_name): (u64, String)| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ProjectRename {
                        project_id,
                        new_name,
                    })
                    .await;
            });
        }
    };

    let on_edit_project = move |project_id: u64| edit_project.set(Some(project_id));

    let on_delete_project = move |project_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ProjectDelete { project_id })
                    .await;
            });
        }
    };

    let on_switch_thread = move |thread_id: u64| {
        // A thread may pin its own working directory, so this can move the
        // workspace out from under the editor.
        guard_workspace_change(PendingWorkspaceChange::Thread(thread_id));
    };

    let on_rename_thread = move |(thread_id, new_label): (u64, String)| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ThreadRename {
                        thread_id,
                        new_label,
                    })
                    .await;
            });
        }
    };

    let on_edit_thread = move |thread_id: u64| edit_thread.set(Some(thread_id));

    let on_delete_thread = move |thread_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client.send(GatewayCommand::ThreadClose { thread_id }).await;
            });
        }
    };

    let on_cancel = move |_| {
        let mut s = state.write();
        s.push_notice(MessageRole::Info, "Cancellation requested…");
        s.finish_current_message();
        // Stop applies to a turn parked on a question too: the gateway drops
        // the wait, so the card must go with it.
        s.clear_user_prompt();
        // Stop names the turn it means. With turns running per thread, the
        // gateway cannot resolve "the current one" for us without guessing.
        let thread_id = s.streaming_thread_id;
        drop(s);
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client.send(GatewayCommand::Cancel { thread_id }).await;
            });
        }
    };

    // Structured answers for the inline agent-question card (`ask_user` tool).
    let on_prompt_respond = move |(id, value): (String, PromptResponseValue)| {
        state.write().clear_user_prompt();
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::UserPromptResponse {
                        id,
                        dismissed: false,
                        value,
                    })
                    .await;
            });
        }
    };

    let on_prompt_dismiss = move |id: String| {
        state.write().clear_user_prompt();
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::UserPromptResponse {
                        id,
                        dismissed: true,
                        value: PromptResponseValue::Text(String::new()),
                    })
                    .await;
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
                    spawn(async move {
                        let _ = client
                            .send(GatewayCommand::ThreadCreate {
                                label: None,
                                project_id: None,
                            })
                            .await;
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
            } else if event.id == ids.secrets {
                show_secrets.set(true);
                let gw = gateway.read().clone();
                if let Some(client) = gw {
                    spawn(async move {
                        let _ = client.send(GatewayCommand::SecretsList).await;
                        let _ = client.send(GatewayCommand::SecretsHasTotp).await;
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
                        spawn(async move {
                            if need_host {
                                let _ = client.send(GatewayCommand::HostInfoRequest).await;
                            }
                            let _ = client.send(GatewayCommand::LoadStatusRequest).await;
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
                        spawn(async move {
                            let _ = client.send(GatewayCommand::ServiceList).await;
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
                        spawn(async move {
                            let _ = client.send(GatewayCommand::EngineList).await;
                            if need_host {
                                let _ = client.send(GatewayCommand::HostInfoRequest).await;
                            }
                        });
                    }
                }
            } else if event.id == ids.cron {
                let v = state.read().show_cron_dialog;
                state.write().show_cron_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::CronList).await;
                        });
                    }
                }
            } else if event.id == ids.memory {
                let v = state.read().show_memory_dialog;
                state.write().show_memory_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client
                                .send(GatewayCommand::MemoryList {
                                    query: None,
                                    limit: None,
                                })
                                .await;
                        });
                    }
                }
            } else if event.id == ids.mcp {
                let v = state.read().show_mcp_dialog;
                state.write().show_mcp_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::McpList).await;
                        });
                    }
                }
            } else if event.id == ids.channels {
                let v = state.read().show_channels_dialog;
                state.write().show_channels_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::ChannelStatus).await;
                        });
                    }
                }
            } else if event.id == ids.tool_perms {
                let v = state.read().show_tools_dialog;
                state.write().show_tools_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::ToolConfigList).await;
                        });
                    }
                }
            } else if event.id == ids.analytics {
                let v = state.read().show_analytics_dialog;
                state.write().show_analytics_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client
                                .send(GatewayCommand::UsageStats { period: None })
                                .await;
                        });
                    }
                }
            } else if event.id == ids.logs {
                let v = state.read().show_logs_dialog;
                state.write().show_logs_dialog = !v;
                if !v {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client
                                .send(GatewayCommand::Logs {
                                    source: "gateway".into(),
                                    tail: None,
                                })
                                .await;
                        });
                    }
                }
            } else if event.id == ids.quit {
                dioxus::desktop::window().close();
            }
        }
    });

    // ── Editor plugin ───────────────────────────────────────────────────
    //
    // The editor is a native plugin; these translate its requests into the
    // workspace file frames, which the gateway confines to the thread's
    // effective working directory.
    let on_editor_action = move |action: crate::components::EditorAction| {
        use crate::components::EditorAction as A;
        // Opening a file adds its tab and focuses it before the contents
        // arrive, so the pane can show "Loading…" rather than nothing.
        if let A::OpenFile(ref path) = action {
            state.write().workspace.open_file(path.clone());
        }
        let command = match action {
            A::OpenFile(path) => GatewayCommand::WorkspaceReadFile { path },
            A::Save { path, content } => {
                // Remember what is being written: the result frame carries
                // only path/ok/error, and the buffer cannot be reconciled
                // without knowing which text actually reached disk. The root
                // travels too, so the gateway refuses a buffer captured
                // before the workspace moved.
                let expected_root = {
                    let mut s = state.write();
                    s.workspace.begin_save(path.clone(), content.clone());
                    s.workspace
                        .root()
                        .map(|r| r.to_path_buf())
                        .unwrap_or_default()
                };
                GatewayCommand::WorkspaceWriteFile {
                    path,
                    content,
                    expected_root,
                }
            }
        };
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client.send(command).await {
                    tracing::error!(error = %e, "Editor request failed to send");
                }
            });
        }
    };

    let on_editor_toggle_dir = move |path: std::path::PathBuf| {
        // Fetch on first expand only; a collapsed-then-reopened directory
        // keeps what it already has.
        let needs_listing = state.write().workspace.toggle_dir(path.clone());
        if needs_listing {
            let gw = gateway.read().clone();
            if let Some(client) = gw {
                spawn(async move {
                    if let Err(e) = client.send(GatewayCommand::WorkspaceListDir { path }).await {
                        tracing::error!(error = %e, "Editor directory listing failed to send");
                    }
                });
            }
        }
    };

    let on_editor_close_tab = move |path: std::path::PathBuf| {
        // Unsaved text is kept: closing a tab should not be a silent way to
        // throw work away. Reopening the file shows it again.
        state.write().workspace.close_file(&path);
    };

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
                                spawn(async move {
                                    let _ = client.send(GatewayCommand::SecretsList).await;
                        let _ = client.send(GatewayCommand::SecretsHasTotp).await;
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
                        tree: rustyclaw_view::SidebarTree::build(
                            &state.read().projects,
                            &state.read().threads,
                            state.read().active_project_id,
                        ),
                        foreground_id: state.read().foreground_thread_id,
                        on_pair: move |_| show_pairing.set(true),
                        on_secrets: move |_| {
                            show_secrets.set(true);
                            let gw = gateway.read().clone();
                            if let Some(client) = gw {
                                spawn(async move {
                                    let _ = client.send(GatewayCommand::SecretsList).await;
                        let _ = client.send(GatewayCommand::SecretsHasTotp).await;
                                });
                            }
                        },
                        on_settings: move |_| show_settings.set(true),
                        on_local_models: move |_| {
                            state.write().show_engines_dialog = true;
                            let need_host = state.read().host_info.is_none();
                            let gw = gateway.read().clone();
                            if let Some(client) = gw {
                                spawn(async move {
                                    let _ = client.send(GatewayCommand::EngineList).await;
                                    if need_host {
                                        let _ = client.send(GatewayCommand::HostInfoRequest).await;
                                    }
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
                                    Some(folder) => guard_workspace_change(
                                        PendingWorkspaceChange::Directory(
                                            folder.path().display().to_string(),
                                        ),
                                    ),
                                    None => state.write().directory_selector_expanded = false,
                                }
                            });
                            return;
                        }
                        guard_workspace_change(PendingWorkspaceChange::Directory(path));
                    },
                }
            }

                // Plugin dock. Replaces the old right sidebar, which hard-coded a
                // Files/Plugins tab pair. It only renders when at least one
                // plugin is loaded, so an install with no plugins gets the full
                // width for chat instead of an empty panel.
                if state.read().plugin_dock_visible {
                    aside { class: "sidebar plugin-dock",
                        crate::components::PluginPanel {
                            plugins: state.read().plugins.clone(),
                            native: vec![crate::components::NativePluginTab {
                                name: crate::components::EDITOR_PLUGIN.to_string(),
                                emoji: "📝".to_string(),
                                body: rsx! {
                                    crate::components::Editor {
                                        listings: state.read().workspace.listings().clone(),
                                        files: state.read().workspace.files().clone(),
                                        edits: state.read().workspace.edits().clone(),
                                        expanded: state.read().workspace.expanded().clone(),
                                        open: state.read().workspace.open().to_vec(),
                                        active: state.read().workspace.active().map(|p| p.to_path_buf()),
                                        root_label: state
                                            .read()
                                            .working_directory
                                            .clone()
                                            .unwrap_or_else(|| "workspace".to_string()),
                                        on_action: on_editor_action,
                                        on_toggle_dir: on_editor_toggle_dir,
                                        on_select_tab: move |path: std::path::PathBuf| {
                                            state.write().workspace.focus(path);
                                        },
                                        on_close_tab: on_editor_close_tab,
                                        on_edit: move |(path, text): (std::path::PathBuf, String)| {
                                            state.write().workspace.set_edit(path, text);
                                        },
                                    }
                                },
                            }],
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
                        spawn(async move {
                            let _ = client
                                .send(GatewayCommand::ProjectCreate { name, path })
                                .await;
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

            // Unsaved-changes prompt, gating any workspace change.
            {
                state.read().pending_workspace_change.clone().map(|change| {
                    let files = state.read().workspace.unsaved();
                    let destination = match &change {
                        PendingWorkspaceChange::Directory(path) => display_path(path),
                        PendingWorkspaceChange::Project(_) => "another project".to_string(),
                        PendingWorkspaceChange::Thread(_) => "another thread".to_string(),
                    };
                    rsx! {
                        crate::components::UnsavedChangesDialog {
                            files,
                            destination,
                            on_choose: move |choice: crate::components::UnsavedChoice| {
                                use crate::components::UnsavedChoice as C;
                                let change = change.clone();
                                state.write().pending_workspace_change = None;
                                match choice {
                                    // Nothing moves, so nothing is lost.
                                    C::Cancel => {}
                                    C::Discard => apply_workspace_change(change),
                                    C::Save => {
                                        // The gateway handles a connection's frames in
                                        // order, so writes queued before the repoint
                                        // resolve against the directory they were
                                        // written in. Failures still surface as error
                                        // notices from WorkspaceWriteResult.
                                        let (pending, expected_root) = {
                                            let s = state.read();
                                            let root = s
                                                .workspace
                                                .root()
                                                .map(|r| r.to_path_buf())
                                                .unwrap_or_default();
                                            let files: Vec<(std::path::PathBuf, String)> = s
                                                .workspace
                                                .unsaved()
                                                .into_iter()
                                                .filter_map(|p| {
                                                    s.workspace.edits().get(&p).map(|c| (p, c.clone()))
                                                })
                                                .collect();
                                            (files, root)
                                        };
                                        let gw = gateway.read().clone();
                                        if let Some(client) = gw {
                                            spawn(async move {
                                                for (path, content) in pending {
                                                    if let Err(e) = client
                                                        .send(GatewayCommand::WorkspaceWriteFile {
                                                            path,
                                                            content,
                                                            expected_root: expected_root.clone(),
                                                        })
                                                        .await
                                                    {
                                                        tracing::error!(
                                                            error = %e,
                                                            "Save-before-switch write failed to send"
                                                        );
                                                    }
                                                }
                                            });
                                        }
                                        // The writes are queued; treat them as
                                        // landed so the rebase does not warn
                                        // about work the user just chose to
                                        // save. Failures still report.
                                        state.write().workspace.assume_saved();
                                        apply_workspace_change(change);
                                    }
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
