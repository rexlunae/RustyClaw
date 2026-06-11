//! Top-level application component.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons, Notification};
use std::sync::{Arc, Mutex as StdMutex};

use crate::components::{Chat, NewProjectDialog, Sidebar};

use crate::app_support::*;
use crate::state::AppState;
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::ui::ConnectionStatus;
use rustyclaw_core::user_prompt_types::UserPrompt;

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

    // User prompt state
    let mut show_user_prompt = use_signal(|| false);
    let mut user_prompt_data: Signal<Option<UserPrompt>> = use_signal(|| None);

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
    // request bypass and no explicit CLI override is provided.
    let mut show_connection =
        use_signal(move || configured_gateway_url.is_none() && !skip_dialog && !bypass_dialog);

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
        show_user_prompt,
        user_prompt_data,
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
                                s.append_to_current_message(&text);
                                s.streaming_chunks += count;
                                s.streaming_bytes += bytes;
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

        if let Some(prompt) = &s.pending_user_prompt {
            user_prompt_data.set(Some(prompt.clone()));
            show_user_prompt.set(true);
        } else {
            show_user_prompt.set(false);
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
    let on_submit = move |message: String| {
        let attachments = state.read().prompt_attachments.clone();
        let prompt = build_prompt_with_attachments(&message, &attachments);
        {
            let mut s = state.write();
            s.add_user_message(prompt.clone());
            s.prompt_attachments.clear();
        }
        state.write().is_processing = true;

        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client.chat(prompt).await {
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
                s.status_message = Some(format!("Attached file {}", path));
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
                s.status_message = Some(format!("Attached directory {}", path));
            }
        });
    };

    let on_clear_attachments = move |_| {
        let mut s = state.write();
        s.prompt_attachments.clear();
        s.status_message = Some("Cleared prompt attachments".to_string());
    };

    let on_remove_attachment = move |path: String| {
        let mut s = state.write();
        let before = s.prompt_attachments.len();
        s.prompt_attachments.retain(|item| item.path != path);
        if s.prompt_attachments.len() != before {
            s.status_message = Some(format!("Removed attachment {}", path));
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

    let on_switch_project = move |project_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ProjectSwitch { project_id })
                    .await;
            });
        }
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
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ThreadSwitch { thread_id })
                    .await;
                // Pull authoritative history from the gateway so this
                // client reflects work done from any other session.
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

    let on_delete_thread = move |thread_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client.send(GatewayCommand::ThreadClose { thread_id }).await;
            });
        }
    };

    let on_cancel = move |_| {
        state.write().status_message = Some("Cancellation requested…".to_string());
        state.write().finish_current_message();
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client.send(GatewayCommand::Cancel).await;
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
            } else if event.id == ids.toggle_left_sidebar {
                let v = state.read().left_sidebar_visible;
                state.write().left_sidebar_visible = !v;
            } else if event.id == ids.toggle_right_sidebar {
                let v = state.read().right_sidebar_visible;
                state.write().right_sidebar_visible = !v;
            } else if event.id == ids.settings {
                show_settings.set(true);
            } else if event.id == ids.secrets {
                show_secrets.set(true);
                let gw = gateway.read().clone();
                if let Some(client) = gw {
                    spawn(async move {
                        let _ = client.send(GatewayCommand::SecretsList).await;
                    });
                }
            } else if event.id == ids.pair {
                show_pairing.set(true);
            } else if event.id == ids.swarm {
                show_swarm.set(true);
            } else if event.id == ids.skills {
                state.write().status_message =
                    Some("Skills manager coming soon on desktop".to_string());
            } else if event.id == ids.quit {
                dioxus::desktop::window().close();
            }
        }
    });

    let on_file_browser_toggle = move |path: std::path::PathBuf| {
        state.write().file_browser.toggle_expand(&path);
    };

    let on_file_browser_select = move |path: std::path::PathBuf| {
        let path_str = path.to_string_lossy().into_owned();
        let attachment = rustyclaw_view::PromptAttachment::from_file_path(path_str.clone());
        let mut s = state.write();
        if !s
            .prompt_attachments
            .iter()
            .any(|item| item.path == attachment.path)
        {
            s.prompt_attachments.push(attachment);
        }
        s.status_message = Some(format!("Attached {}", path.display()));
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
                        let v = state.read().right_sidebar_visible;
                        state.write().right_sidebar_visible = !v;
                    },
                    "◫"
                }
            }

            // ── Workspace: left sidebar + main content + right sidebar ──────
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
                        on_delete_thread: on_delete_thread,
                        on_new_thread_in: on_new_thread_in,
                        on_new_project: on_new_project,
                        on_switch_project: on_switch_project,
                        on_rename_project: on_rename_project,
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
                                });
                            }
                        },
                        on_settings: move |_| show_settings.set(true),
                    }
                }

            div { class: "main",
                // Connection / status banners — which banners appear, and
                // which actions each offers, is decided by the view layer.
                for banner in rustyclaw_view::build_banners(
                    &state.read().connection,
                    state.read().status_message.as_deref(),
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
                                for action in banner.actions.iter().cloned() {
                                    Button {
                                        color: BulmaColor::Ghost,
                                        size: BulmaSize::Small,
                                        onclick: move |_| match action.kind {
                                            BannerActionKind::Reconnect => do_reconnect(sig),
                                            BannerActionKind::PairGateway => show_pairing.set(true),
                                            BannerActionKind::DismissStatus => {
                                                state.write().status_message = None;
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
                    input: state.read().input.clone(),
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
                    on_submit: on_submit,
                    on_cancel: on_cancel,
                    on_input_change: move |value| state.write().input = value,
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
                        state.write().provider = Some(provider);
                        state.write().model = Some(model);
                    },
                    on_add_provider: move |_| show_settings.set(true),
                    on_add_file_attachment: on_add_file_attachment,
                    on_add_directory_attachment: on_add_directory_attachment,
                    on_clear_attachments: on_clear_attachments,
                    on_remove_attachment: on_remove_attachment,
                    on_toggle_directory_selector: move |_| {
                        let is_expanded = state.read().directory_selector_expanded;
                        if is_expanded {
                            state.write().directory_selector_expanded = false;
                        } else {
                            let base = state
                                .read()
                                .working_directory
                                .clone()
                                .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()));
                            if let Some(path) = base {
                                let options = build_directory_options(&path);
                                let mut s = state.write();
                                s.available_directories = options;
                                s.directory_selector_expanded = true;
                                s.directory_selector_error = None;
                            }
                        }
                    },
                    on_select_directory: move |path: String| {
                        if path == DIRECTORY_OTHER_SENTINEL {
                            let start_dir = state
                                .read()
                                .working_directory
                                .clone()
                                .unwrap_or_else(|| ".".to_string());
                            spawn(async move {
                                let picked = rfd::AsyncFileDialog::new()
                                    .set_directory(start_dir)
                                    .pick_folder()
                                    .await;
                                if let Some(folder) = picked {
                                    let selected = folder.path().display().to_string();
                                    match std::env::set_current_dir(&selected) {
                                        Ok(()) => {
                                            let options = build_directory_options(&selected);
                                            let mut s = state.write();
                                            s.working_directory = Some(selected.clone());
                                            s.available_directories = options;
                                            s.file_browser = rustyclaw_view::FileBrowserData::load(
                                                &selected,
                                            );
                                            s.directory_selector_expanded = false;
                                            s.directory_selector_error = None;
                                            s.status_message = Some(format!(
                                                "Working directory set to {}",
                                                display_path(&selected)
                                            ));
                                            // Tell the gateway so agent tools use the new dir.
                                            let gw = gateway.read().clone();
                                            if let Some(client) = gw {
                                                let path = selected.clone();
                                                let _ = client
                                                    .send(GatewayCommand::SetWorkingDirectory { path })
                                                    .await;
                                            }
                                        }
                                        Err(e) => {
                                            let mut s = state.write();
                                            s.directory_selector_error = Some(format!(
                                                "Failed to change directory: {}",
                                                e
                                            ));
                                        }
                                    }
                                } else {
                                    state.write().directory_selector_expanded = false;
                                }
                            });
                            return;
                        }

                        match std::env::set_current_dir(&path) {
                            Ok(()) => {
                                let options = build_directory_options(&path);
                                let mut s = state.write();
                                s.working_directory = Some(path.clone());
                                s.available_directories = options;
                                s.file_browser = rustyclaw_view::FileBrowserData::load(&path);
                                s.directory_selector_expanded = false;
                                s.directory_selector_error = None;
                                s.status_message = Some(format!(
                                    "Working directory set to {}",
                                    display_path(&path)
                                ));
                                // Tell the gateway so agent tools use the new dir.
                                let gw = gateway.read().clone();
                                if let Some(client) = gw {
                                    let p = path.clone();
                                    spawn(async move {
                                        let _ = client
                                            .send(GatewayCommand::SetWorkingDirectory { path: p })
                                            .await;
                                    });
                                }
                            }
                            Err(e) => {
                                let mut s = state.write();
                                s.directory_selector_error = Some(format!(
                                    "Failed to change directory: {}",
                                    e
                                ));
                            }
                        }
                    },
                }
            }

                if state.read().right_sidebar_visible {
                    aside { class: "sidebar sidebar-right",
                        crate::components::FileBrowser {
                            data: state.read().file_browser.clone(),
                            on_toggle: on_file_browser_toggle,
                            on_select: on_file_browser_select,
                        }
                    }
                }
            }

            // New-project dialog
            NewProjectDialog {
                visible: show_new_project(),
                on_cancel: move |_| show_new_project.set(false),
                on_create: move |(name, path): (String, String)| {
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

            // Modals
            {render_dialogs(sig)}
        }
    }
}
