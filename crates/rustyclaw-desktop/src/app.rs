//! Top-level application component.

use dioxus::prelude::*;
use std::sync::{Arc, Mutex as StdMutex};

use crate::components::{
    Chat, CredentialRequestDialog, DeviceFlowDialog, HatchingDialog, HatchingResult,
    PairingDialog, SecretsCommand, SecretsDialog, SettingsDialog, Sidebar, SwarmPanel, TabBar,
    ToolApprovalDialog, UserPromptDialog,
    VaultUnlockDialog, generate_qr_code,
};
use crate::gateway::{GatewayClient, GatewayCommand, GatewayEvent};
use crate::state::{AppState, Theme};
use rustyclaw_core::ui::{ConnectionStatus, ThreadInfo};
use rustyclaw_core::user_prompt_types::{PromptResponseValue, UserPrompt};
use rustyclaw_view::{SecretInfoData, SecretsDialogData, SwarmAgentData, SwarmData};

/// Bundled stylesheet — embedded directly in the binary so the desktop crate
/// can be run with plain `cargo run`/`cargo build` without the `dx` CLI.
const STYLES: &str = include_str!("../assets/styles.css");

#[component]
pub fn App() -> Element {
    // Application state
    let mut state = use_signal(AppState::default);

    // Gateway client (set when connected)
    let gateway: Signal<Option<Arc<GatewayClient>>> = use_signal(|| None);
    let mut did_auto_connect = use_signal(|| false);
    let mut active_event_client: Signal<Option<Arc<GatewayClient>>> = use_signal(|| None);
    let mut auth_code = use_signal(String::new);

    // Dialog visibility
    let mut show_pairing = use_signal(|| false);
    let mut show_hatching = use_signal(|| state.read().needs_hatching);
    let mut show_settings = use_signal(|| false);
    let mut show_swarm = use_signal(|| false);
    let mut swarm_creating = use_signal(|| false);

    // Tool approval state
    let mut tool_approval_id = use_signal(String::new);
    let mut tool_approval_name = use_signal(String::new);
    let mut tool_approval_args = use_signal(String::new);
    let mut show_tool_approval = use_signal(|| false);

    // Vault unlock state
    let mut show_vault_unlock = use_signal(|| false);
    let mut vault_unlock_error = use_signal(|| None::<String>);

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
    let mut qr_code_url = use_signal(|| None::<String>);
    let mut public_key = use_signal(|| None::<String>);

    // Secrets management state
    let mut show_secrets = use_signal(|| false);

    // Thread deletion confirmation state
    let mut pending_thread_delete = use_signal(|| None::<(u64, String)>);

    // Auto-connect on mount
    use_effect(move || {
        if *did_auto_connect.read() {
            return;
        }
        did_auto_connect.set(true);

        let url = state.read().gateway_url.clone();
        spawn(async move {
            connect_to_gateway(&url, state, gateway).await;
        });
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
                                let should_refresh_threads = matches!(
                                    event,
                                    GatewayEvent::Connected { .. }
                                        | GatewayEvent::AuthSuccess
                                        | GatewayEvent::VaultUnlocked
                                );
                                handle_gateway_event(event, state);
                                if should_refresh_threads {
                                    let _ = client_ui.send(GatewayCommand::ThreadList).await;
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
        state.write().add_user_message(message.clone());
        state.write().is_processing = true;

        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client.chat(message).await {
                    tracing::error!("Failed to send message: {}", e);
                }
            });
        }
    };

    let on_new_thread = move |_| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ThreadCreate { label: None })
                    .await;
            });
        }
        // Save current thread's messages and start with empty chat.
        // The gateway will assign a new foreground via ThreadsUpdate.
        let mut s = state.write();
        if let Some(current_id) = s.foreground_thread_id {
            if !s.messages.is_empty() {
                let msgs = s.messages.clone();
                s.save_thread_messages(current_id, msgs);
            }
        }
        s.messages.clear();
    };

    let on_switch_thread = move |thread_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let _ = client
                    .send(GatewayCommand::ThreadSwitch { thread_id })
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
                let _ = client
                    .send(GatewayCommand::ThreadClose { thread_id })
                    .await;
            });
        }
    };

    let on_cancel = move |_| {
        state.write().status_message = Some("Cancellation requested…".to_string());
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                if let Err(e) = client.send(GatewayCommand::Cancel).await {
                    tracing::error!("Failed to send cancel: {}", e);
                }
            });
        }
    };

    // Secrets dialog event handler
    let on_secrets_command = move |cmd: SecretsCommand| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                match cmd {
                    SecretsCommand::Refresh => {
                        let _ = client.send(GatewayCommand::SecretsList).await;
                    }
                    SecretsCommand::Store { key, value } => {
                        let _ = client
                            .send(GatewayCommand::SecretsStore { key, value })
                            .await;
                    }
                    SecretsCommand::Delete { key } => {
                        let _ = client
                            .send(GatewayCommand::SecretsDelete { key })
                            .await;
                    }
                    SecretsCommand::SetPolicy { name, policy } => {
                        let _ = client
                            .send(GatewayCommand::SecretsSetPolicy {
                                name,
                                policy,
                                skills: Vec::new(),
                            })
                            .await;
                    }
                }
            });
        }
    };

    // Closure used by every "reconnect" entry-point. It only captures `Copy`
    // signals, so it is itself `Copy`; rebinding is therefore cheap.
    let do_reconnect = move || {
        let url = state.read().gateway_url.clone();
        spawn(async move {
            connect_to_gateway(&url, state, gateway).await;
        });
    };

    rsx! {
        style { dangerous_inner_html: STYLES }

        div {
            id: "rc-root",
            class: "app",
            "data-theme": "{theme_attr}",

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
                on_new_thread: on_new_thread,
                on_switch_thread: on_switch_thread,
                on_rename_thread: on_rename_thread,
                on_delete_thread: on_delete_thread,
                threads: state
                    .read()
                    .threads
                    .iter()
                    .map(rustyclaw_view::SidebarItemData::from)
                    .collect(),
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

            div { class: "main",
                // Top bar with current thread / model summary
                TopBar {
                    data: rustyclaw_view::TopBarData::from_threads(
                        state.read().foreground_thread_id,
                        &state.read().threads,
                        state.read().agent_name.clone(),
                        state.read().provider.clone(),
                        state.read().model.clone(),
                    ),
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
                    on_swarm: move |_| show_swarm.set(true),
                }

                // Connection / status banners
                if let ConnectionStatus::Error(err) = state.read().connection.clone() {
                    div { class: "banner is-danger",
                        span { class: "banner-text",
                            "🚫 Connection error: {err}"
                        }
                        div { class: "banner-actions",
                            button {
                                class: "btn btn-ghost btn-sm",
                                onclick: move |_| do_reconnect(),
                                "↻ Retry"
                            }
                            button {
                                class: "btn btn-subtle btn-sm",
                                onclick: move |_| show_pairing.set(true),
                                "Pair gateway"
                            }
                        }
                    }
                } else if matches!(state.read().connection.clone(), ConnectionStatus::Connecting) {
                    div { class: "banner is-info",
                        span { class: "banner-text",
                            "🔄 Connecting to gateway…"
                        }
                    }
                }

                if let Some(msg) = state.read().status_message.clone() {
                    div { class: "banner is-warn",
                        span { class: "banner-text", "{msg}" }
                        div { class: "banner-actions",
                            button {
                                class: "btn btn-ghost btn-sm",
                                onclick: move |_| state.write().status_message = None,
                                "Dismiss"
                            }
                        }
                    }
                }

                // Thread tab bar
                TabBar {
                    data: rustyclaw_view::TabBarData::from_threads(
                        &state.read().threads,
                    ),
                    on_switch: on_switch_thread,
                    on_new: on_new_thread,
                    on_close: move |id| {
                        let label = state
                            .read()
                            .threads
                            .iter()
                            .find(|thread| thread.id == id)
                            .and_then(|thread| thread.label.clone())
                            .unwrap_or_else(|| format!("Session #{}", id));
                        pending_thread_delete.set(Some((id, label)));
                    },
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
                    agent_name: state.read().agent_name.clone(),
                    current_provider: state.read().provider.clone(),
                    current_model: state.read().model.clone(),
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
                }
            }

            // Modals
            HatchingDialog {
                visible: *show_hatching.read(),
                on_complete: move |result: HatchingResult| {
                    if let Some(personality) = result.personality.clone() {
                        state.write().status_message = Some(format!("Personality set: {}", personality));
                    }
                    let name = result.name.clone();
                    state.write().agent_name = Some(result.name);
                    show_hatching.set(false);
                    // Persist the name to the gateway config.
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::SetAgentName { name }).await;
                        });
                    }
                },
                on_cancel: move |_| show_hatching.set(false),
            }

            PairingDialog {
                visible: *show_pairing.read(),
                public_key: public_key.read().clone(),
                qr_code_data_url: qr_code_url.read().clone(),
                gateway_host: "127.0.0.1".to_string(),
                gateway_port: 2222,
                on_host_change: move |_| {},
                on_port_change: move |_| {},
                on_connect: move |_| {
                    show_pairing.set(false);
                    do_reconnect();
                },
                on_generate_key: move |_| {
                    public_key.set(Some(
                        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... desktop-client".to_string(),
                    ));
                    if let Some(key) = &*public_key.read() {
                        qr_code_url.set(generate_qr_code(key));
                    }
                },
                on_cancel: move |_| show_pairing.set(false),
            }

            SettingsDialog {
                visible: *show_settings.read(),
                theme: state.read().theme,
                gateway_url: state.read().gateway_url.clone(),
                on_theme_change: move |t: Theme| state.write().theme = t,
                on_gateway_url_change: move |v: String| state.write().gateway_url = v,
                on_reconnect: move |_| do_reconnect(),
                on_credential_save: move |(provider_id, api_key): (String, String)| {
                    let secret_key = rustyclaw_core::providers::secret_key_for_provider(&provider_id)
                        .unwrap_or(&provider_id)
                        .to_string();
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client.send(GatewayCommand::SecretsStore {
                                key: secret_key,
                                value: api_key,
                            }).await {
                                tracing::error!("Failed to store credential: {}", e);
                            }
                        });
                    }
                    state.write().status_message = Some(format!(
                        "API key saved for {}",
                        rustyclaw_core::providers::display_name_for_provider(&provider_id)
                    ));
                },
                on_close: move |_| show_settings.set(false),
            }

            SwarmPanel {
                swarms: get_swarm_infos(),
                creating: *swarm_creating.read(),
                visible: *show_swarm.read(),
                on_create: move |template: String| {
                    swarm_creating.set(true);
                    spawn(async move {
                        let result = create_swarm_from_template(&template);
                        swarm_creating.set(false);
                        if let Err(e) = result {
                            state.write().status_message =
                                Some(format!("Failed to create swarm: {}", e));
                        }
                    });
                },
                on_stop: move |name: String| {
                    if let Err(e) = stop_swarm(&name) {
                        state.write().status_message =
                            Some(format!("Failed to stop swarm: {}", e));
                    }
                },
                on_close: move |_| show_swarm.set(false),
            }

            ToolApprovalDialog {
                visible: *show_tool_approval.read(),
                id: tool_approval_id.read().clone(),
                tool_name: tool_approval_name.read().clone(),
                arguments: tool_approval_args.read().clone(),
                on_approve: move |id: String| {
                    state.write().pending_tool_approval = None;
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::ToolApprove { id, approved: true }).await;
                        });
                    }
                },
                on_deny: move |id: String| {
                    state.write().pending_tool_approval = None;
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::ToolApprove { id, approved: false }).await;
                        });
                    }
                },
            }

            VaultUnlockDialog {
                visible: *show_vault_unlock.read(),
                error: vault_unlock_error.read().clone(),
                on_submit: move |password: String| {
                    vault_unlock_error.set(None);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::VaultUnlock { password }).await;
                        });
                    }
                },
                on_cancel: move |_| show_vault_unlock.set(false),
            }

            UserPromptDialog {
                visible: *show_user_prompt.read(),
                prompt: user_prompt_data.read().clone(),
                on_respond: move |(id, value): (String, PromptResponseValue)| {
                    state.write().pending_user_prompt = None;
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::UserPromptResponse {
                                id,
                                dismissed: false,
                                value,
                            }).await;
                        });
                    }
                },
                on_dismiss: move |id: String| {
                    state.write().pending_user_prompt = None;
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::UserPromptResponse {
                                id,
                                dismissed: true,
                                value: PromptResponseValue::Text(String::new()),
                            }).await;
                        });
                    }
                },
            }

            CredentialRequestDialog {
                visible: *show_cred_request.read(),
                id: cred_request_id.read().clone(),
                provider: cred_request_provider.read().clone(),
                secret_name: cred_request_secret.read().clone(),
                message: cred_request_message.read().clone(),
                on_submit: move |(id, value): (String, String)| {
                    state.write().pending_credential_request = None;
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::CredentialResponse {
                                id,
                                dismissed: false,
                                value: Some(value),
                            }).await;
                        });
                    }
                },
                on_dismiss: move |id: String| {
                    state.write().pending_credential_request = None;
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::CredentialResponse {
                                id,
                                dismissed: true,
                                value: None,
                            }).await;
                        });
                    }
                },
            }

            SecretsDialog {
                visible: *show_secrets.read(),
                data: state.read().secrets_data.clone(),
                on_command: on_secrets_command,
                on_close: move |_| show_secrets.set(false),
            }

            DeviceFlowDialog {
                visible: state.read().pending_device_flow.is_some(),
                url: state.read().pending_device_flow.as_ref().map(|(u, _, _)| u.clone()).unwrap_or_default(),
                code: state.read().pending_device_flow.as_ref().map(|(_, c, _)| c.clone()).unwrap_or_default(),
                message: state.read().pending_device_flow.as_ref().and_then(|(_, _, m)| m.clone()),
                on_close: move |_| {
                    state.write().pending_device_flow = None;
                    state.write().status_message = Some("Device flow cancelled.".to_string());
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::Cancel).await;
                        });
                    }
                },
            }

            if let Some((thread_id, thread_label)) = pending_thread_delete.read().clone() {
                div { class: "modal-backdrop",
                    div { class: "modal modal-confirm",
                        div { class: "modal-head",
                            div { class: "modal-title", "Delete thread?" }
                            button {
                                class: "modal-close",
                                onclick: move |_| pending_thread_delete.set(None),
                                "✕"
                            }
                        }
                        div { class: "modal-body",
                            p { "This will permanently delete \"{thread_label}\" and its messages." }
                            p { class: "modal-muted", "This action cannot be undone." }
                        }
                        div { class: "modal-foot",
                            button {
                                class: "btn btn-ghost",
                                onclick: move |_| pending_thread_delete.set(None),
                                "Cancel"
                            }
                            button {
                                class: "btn btn-danger",
                                onclick: move |_| {
                                    pending_thread_delete.set(None);
                                    let fallback_id = {
                                        let s = state.read();
                                        if s.foreground_thread_id == Some(thread_id) {
                                            s.threads
                                                .iter()
                                                .filter(|thread| thread.id != thread_id)
                                                .map(|thread| thread.id)
                                                .next_back()
                                        } else {
                                            None
                                        }
                                    };
                                    if let Some(fallback_id) = fallback_id {
                                        state.write().switch_thread(fallback_id);
                                    }
                                    let gw = gateway.read().clone();
                                    if let Some(client) = gw {
                                        spawn(async move {
                                            if let Some(fallback_id) = fallback_id {
                                                let _ = client
                                                    .send(GatewayCommand::ThreadSwitch { thread_id: fallback_id })
                                                    .await;
                                            }
                                            let _ = client
                                                .send(GatewayCommand::ThreadClose { thread_id })
                                                .await;
                                        });
                                    }
                                },
                                "Delete Thread"
                            }
                        }
                    }
                }
            }

            // TOTP authentication modal
            if matches!(state.read().connection.clone(), ConnectionStatus::Authenticating) {
                div { class: "modal-backdrop",
                    onclick: |_| {},
                    div {
                        class: "modal",
                        onclick: move |evt| evt.stop_propagation(),
                        div { class: "modal-head",
                            span { class: "modal-title", "Gateway Authentication" }
                        }
                        div { class: "modal-body",
                            p {
                                style: "margin-bottom: 16px; color: var(--rc-text-muted); font-size: 13px;",
                                "Enter the TOTP code from your authenticator app to connect to the gateway."
                            }
                            div { class: "field",
                                label { class: "field-label", "TOTP Code" }
                                input {
                                    class: "input totp-input",
                                    r#type: "text",
                                    placeholder: "000000",
                                    value: "{auth_code}",
                                    autofocus: true,
                                    maxlength: "8",
                                    oninput: move |evt| auth_code.set(evt.value()),
                                    onkeydown: move |evt: KeyboardEvent| {
                                        if evt.key() == Key::Enter {
                                            evt.prevent_default();
                                            let code = auth_code.read().trim().to_string();
                                            if code.is_empty() {
                                                return;
                                            }
                                            let gw = gateway.read().clone();
                                            if let Some(client) = gw {
                                                auth_code.set(String::new());
                                                spawn(async move {
                                                    if let Err(e) = client.send(GatewayCommand::Auth { code }).await {
                                                        tracing::error!("Failed to send auth code: {}", e);
                                                    }
                                                });
                                            }
                                        }
                                    },
                                }
                            }
                        }
                        div { class: "modal-foot",
                            button {
                                class: "btn btn-primary",
                                disabled: auth_code.read().trim().is_empty(),
                                onclick: move |_| {
                                    let code = auth_code.read().trim().to_string();
                                    if code.is_empty() {
                                        return;
                                    }
                                    let gw = gateway.read().clone();
                                    if let Some(client) = gw {
                                        auth_code.set(String::new());
                                        spawn(async move {
                                            if let Err(e) = client.send(GatewayCommand::Auth { code }).await {
                                                tracing::error!("Failed to send auth code: {}", e);
                                            }
                                        });
                                    }
                                },
                                "Verify"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct TopBarProps {
    data: rustyclaw_view::TopBarData,
    on_secrets: EventHandler<()>,
    on_settings: EventHandler<()>,
    on_swarm: EventHandler<()>,
}

#[component]
fn TopBar(props: TopBarProps) -> Element {
    rsx! {
        div { class: "topbar",
            div {
                style: "display: flex; flex-direction: column; min-width: 0;",
                span { class: "topbar-title", "{props.data.title}" }
                if !props.data.subtitle.is_empty() {
                    span { class: "topbar-sub", "{props.data.subtitle}" }
                }
            }
            div { class: "topbar-right",
                button {
                    class: "icon-btn",
                    title: "Secrets Vault",
                    onclick: move |_| props.on_secrets.call(()),
                    "🔑"
                }
                button {
                    class: "icon-btn",
                    title: "Swarm Manager",
                    onclick: move |_| props.on_swarm.call(()),
                    "🐝"
                }
                button {
                    class: "icon-btn",
                    title: "Settings",
                    onclick: move |_| props.on_settings.call(()),
                    "⚙"
                }
            }
        }
    }
}

// ── Shared buffer for the worker → UI bridge ───────────────────────────────

/// An entry in the ordered event buffer.  Consecutive Chunk events
/// are coalesced into a single `Chunks` entry to reduce signal writes,
/// while preserving the ordering of non-chunk events relative to chunks.
enum BufferEntry {
    Event(GatewayEvent),
    Chunks { text: String, count: u32, bytes: usize },
}

/// Intermediate buffer between the tokio event-consumer worker and
/// the Dioxus UI task.  The worker writes at full speed; the UI task
/// drains on each `Notify` wake-up.
#[derive(Default)]
struct EventBuffer {
    entries: Vec<BufferEntry>,
}

/// Connect to the gateway.
async fn connect_to_gateway(
    url: &str,
    mut state: Signal<AppState>,
    mut gateway: Signal<Option<Arc<GatewayClient>>>,
) {
    state.write().connection = ConnectionStatus::Connecting;

    match GatewayClient::connect(url).await {
        Ok(client) => {
            gateway.set(Some(Arc::new(client)));
            state.write().connection = ConnectionStatus::Connected;
        }
        Err(e) => {
            state.write().connection = ConnectionStatus::Error(e.to_string());
            tracing::error!("Failed to connect to gateway: {}", e);
        }
    }
}

/// Handle a gateway event.
fn handle_gateway_event(event: GatewayEvent, mut state: Signal<AppState>) {
    match event {
        GatewayEvent::Connected {
            agent,
            vault_locked,
            provider,
            model,
        } => {
            state.write().connection = ConnectionStatus::Connected;
            state.write().agent_name = agent;
            state.write().vault_locked = vault_locked;
            state.write().provider = provider;
            state.write().model = model;
        }
        GatewayEvent::Disconnected { reason } => {
            state.write().connection = ConnectionStatus::Disconnected;
            if let Some(r) = reason {
                state.write().status_message = Some(format!("Disconnected: {}", r));
            }
        }
        GatewayEvent::AuthRequired => {
            state.write().connection = ConnectionStatus::Authenticating;
        }
        GatewayEvent::AuthSuccess => {
            state.write().connection = ConnectionStatus::Authenticated;
        }
        GatewayEvent::AuthFailed { message, retry } => {
            state.write().status_message = Some(if retry {
                format!("Auth failed (retry allowed): {}", message)
            } else {
                format!("Auth failed: {}", message)
            });
        }
        GatewayEvent::VaultLocked => {
            state.write().vault_locked = true;
        }
        GatewayEvent::VaultUnlocked => {
            state.write().vault_locked = false;
        }
        GatewayEvent::ModelReady { model } => {
            state.write().model = Some(model);
        }
        GatewayEvent::ModelError { message } => {
            state.write().status_message = Some(format!("Model error: {}", message));
        }
        GatewayEvent::StreamStart => {
            state.write().start_assistant_message();
        }
        GatewayEvent::ThinkingStart => {
            state.write().is_thinking = true;
        }
        GatewayEvent::ThinkingEnd => {
            state.write().is_thinking = false;
        }
        GatewayEvent::Chunk { delta } => {
            state.write().append_to_current_message(&delta);
        }
        GatewayEvent::ResponseDone => {
            state.write().finish_current_message();
        }
        GatewayEvent::ToolCall {
            id,
            name,
            arguments,
        } => {
            state.write().add_tool_call(id, name, arguments);
        }
        GatewayEvent::ToolResult {
            id,
            name,
            result,
            is_error,
        } => {
            if is_error {
                state.write().status_message = Some(format!("Tool '{}' failed", name));
            }
            state.write().set_tool_result(&id, result, is_error);
        }
        GatewayEvent::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => {
            state.write().pending_tool_approval = Some((id, name, arguments));
        }
        GatewayEvent::ThreadsUpdate {
            threads,
            foreground_id,
        } => {
            let count = threads.len();
            tracing::info!(count, foreground_id = ?foreground_id, "ThreadsUpdate received");
            state.write().threads = threads
                .into_iter()
                .map(|t| ThreadInfo {
                    id: t.id,
                    label: t.label,
                    description: t.description,
                    status: t.status,
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                })
                .collect();
            state.write().foreground_thread_id = foreground_id;
        }
        GatewayEvent::UserPromptRequest { id: _, prompt } => {
            state.write().pending_user_prompt = Some(prompt);
        }
        GatewayEvent::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => {
            state.write().pending_credential_request =
                Some((id, provider, secret_name, message));
        }
        GatewayEvent::DeviceFlowStart { url, code, message } => {
            state.write().pending_device_flow = Some((url, code, message));
        }
        GatewayEvent::DeviceFlowComplete => {
            state.write().pending_device_flow = None;
        }
        GatewayEvent::SecretsListResult { ok, entries } => {
            if ok {
                let data = SecretsDialogData::from_vault(
                    entries
                        .into_iter()
                        .map(|e| SecretInfoData {
                            key: e.name,
                            label: e.label,
                            kind: e.kind,
                            policy: e.policy,
                            disabled: e.disabled,
                        })
                        .collect(),
                    state.read().agent_access,
                    false, // has_totp — would need gateway peek
                );
                state.write().secrets_data = data;
            } else {
                state.write().status_message =
                    Some("Failed to list secrets.".to_string());
            }
        }
        GatewayEvent::SecretsStoreResult { ok, message } => {
            if ok {
                state.write().status_message =
                    Some("Secret stored successfully.".to_string());
                // Trigger refresh to show new secret
                // (parent doesn't have gateway handle here, so we just update status)
            } else {
                state.write().status_message =
                    Some(format!("Failed to store secret: {}", message));
            }
        }
        GatewayEvent::SecretsDeleteResult { ok, message } => {
            if ok {
                state.write().status_message =
                    Some("Secret deleted.".to_string());
            } else {
                state.write().status_message = Some(format!(
                    "Failed to delete secret: {}",
                    message.unwrap_or_default()
                ));
            }
        }
        GatewayEvent::SecretsSetPolicyResult { ok, message } => {
            if ok {
                state.write().status_message =
                    Some("Policy updated.".to_string());
            } else {
                state.write().status_message = Some(format!(
                    "Failed to set policy: {}",
                    message.unwrap_or_default()
                ));
            }
        }
        GatewayEvent::Error { message } => {
            state.write().status_message = Some(message);
            state.write().is_processing = false;
        }
        GatewayEvent::Info { message } => {
            state.write().status_message = Some(message);
        }
        GatewayEvent::DomQuery { .. } => {
            // Handled directly in the UI updater task via handle_dom_query.
        }
    }
}

// ── DOM query handler ───────────────────────────────────────────────────────

/// Execute a JavaScript expression in the webview and send the result
/// back to the gateway as a `DomQueryResponse`.
async fn handle_dom_query(client: &Arc<GatewayClient>, id: String, js: String) {
    let wrapped = format!(
        r#"(function() {{
            try {{
                var __result = (function() {{ return {js}; }})();
                if (typeof __result === 'undefined') return JSON.stringify({{__ok:true,__v:'undefined'}});
                if (typeof __result === 'string') return JSON.stringify({{__ok:true,__v:__result}});
                return JSON.stringify({{__ok:true,__v:JSON.stringify(__result)}});
            }} catch(e) {{
                return JSON.stringify({{__ok:false,__v:e.message}});
            }}
        }})()"#,
    );

    // Retry up to 3 times to work around Dioxus EvalError::Finished
    // bug (https://github.com/DioxusLabs/dioxus/issues/3084) where
    // eval sometimes reports "already ran" spuriously.
    let mut attempts = 0;
    let (result, is_error) = loop {
        attempts += 1;
        let eval = document::eval(&wrapped);
        match eval.await {
            Ok(val) => {
                let raw = match val {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                break match serde_json::from_str::<serde_json::Value>(&raw) {
                    Ok(obj) => {
                        let ok = obj.get("__ok").and_then(|v| v.as_bool()).unwrap_or(false);
                        let v = obj
                            .get("__v")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        (v, !ok)
                    }
                    Err(_) => (raw, false),
                };
            }
            Err(e) => {
                if attempts < 3 {
                    tracing::warn!(attempt = attempts, error = %e, "DOM eval failed, retrying");
                    continue;
                }
                break (format!("eval error after {} attempts: {}", attempts, e), true);
            }
        }
    };

    let _ = client
        .send(GatewayCommand::DomQueryResponse {
            id,
            result,
            is_error,
        })
        .await;
}

// ── Swarm helpers ───────────────────────────────────────────────────────────

/// Build the current list of swarm infos from the global swarm manager.
fn get_swarm_infos() -> Vec<SwarmData> {
    use rustyclaw_core::swarm::swarm_manager;

    let mgr = match swarm_manager().lock() {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    mgr.list()
        .into_iter()
        .map(|inst| SwarmData {
            name: inst.config.name.clone(),
            status: inst.status.to_string(),
            description: inst.config.description.clone(),
            tasks_routed: inst.tasks_routed,
            uptime_secs: inst.runtime_secs(),
            agents: inst
                .config
                .agents
                .iter()
                .map(|a| SwarmAgentData {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    role: a.role.to_string(),
                    description: a.description.clone(),
                    has_session: inst.agent_sessions.contains_key(&a.id),
                })
                .collect(),
        })
        .collect()
}

/// Create a swarm from a built-in template.
fn create_swarm_from_template(template: &str) -> Result<(), String> {
    use rustyclaw_core::swarm::{builtin_templates, swarm_manager};

    let templates = builtin_templates();
    let cfg = templates
        .into_iter()
        .find(|t| t.name == template)
        .ok_or_else(|| format!("Unknown template: {}", template))?;

    let name = cfg.name.clone();
    let mgr = swarm_manager();
    let mut m = mgr.lock().map_err(|_| "Lock error".to_string())?;
    m.create(cfg)?;
    m.start(&name)?;
    Ok(())
}

/// Stop a running swarm.
fn stop_swarm(name: &str) -> Result<(), String> {
    use rustyclaw_core::swarm::swarm_manager;

    let mgr = swarm_manager();
    let mut m = mgr.lock().map_err(|_| "Lock error".to_string())?;
    m.stop(name)
}
