//! Top-level application component.

use dioxus::prelude::*;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::components::{
    Chat, HatchingDialog, HatchingResult, PairingDialog, SettingsDialog, Sidebar, generate_qr_code,
};
use crate::gateway::{GatewayClient, GatewayCommand, GatewayEvent};
use crate::state::{AppState, ConnectionStatus, Theme, ThreadInfo};

/// Bundled stylesheet — embedded directly in the binary so the desktop crate
/// can be run with plain `cargo run`/`cargo build` without the `dx` CLI.
const STYLES: &str = include_str!("../assets/styles.css");

#[component]
pub fn App() -> Element {
    // Application state
    let mut state = use_signal(AppState::default);

    // Gateway client (set when connected)
    let gateway: Signal<Option<Arc<Mutex<GatewayClient>>>> = use_signal(|| None);

    // Dialog visibility
    let mut show_pairing = use_signal(|| false);
    let mut show_hatching = use_signal(|| state.read().needs_hatching);
    let mut show_settings = use_signal(|| false);

    // QR code for pairing
    let mut qr_code_url = use_signal(|| None::<String>);
    let mut public_key = use_signal(|| None::<String>);

    // Auto-connect on mount
    use_effect(move || {
        let url = state.read().gateway_url.clone();
        spawn(async move {
            connect_to_gateway(&url, state, gateway).await;
        });
    });

    // Handle gateway events
    use_effect(move || {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                loop {
                    let client_guard = client.lock().await;
                    if !client_guard.is_connected() {
                        break;
                    }
                    if let Some(event) = client_guard.recv().await {
                        drop(client_guard);
                        handle_gateway_event(event, state);
                    } else {
                        break;
                    }
                }
            });
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
                let client_guard = client.lock().await;
                if let Err(e) = client_guard.chat(message).await {
                    tracing::error!("Failed to send message: {}", e);
                }
            });
        }
    };

    let on_new_thread = move |_| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let client_guard = client.lock().await;
                let _ = client_guard
                    .send(GatewayCommand::ThreadCreate { label: None })
                    .await;
            });
        }
    };

    let on_switch_thread = move |thread_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let client_guard = client.lock().await;
                let _ = client_guard
                    .send(GatewayCommand::ThreadSwitch { thread_id })
                    .await;
            });
        }
        state.write().clear_messages();
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
                threads: state.read().threads.clone(),
                foreground_id: state.read().foreground_thread_id,
                collapsed: sidebar_collapsed,
                on_toggle_collapse: move |_| {
                    let v = state.read().sidebar_collapsed;
                    state.write().sidebar_collapsed = !v;
                },
                on_new_thread: on_new_thread,
                on_switch_thread: on_switch_thread,
                on_pair: move |_| show_pairing.set(true),
                on_settings: move |_| show_settings.set(true),
            }

            div { class: "main",
                // Top bar with current thread / model summary
                TopBar {
                    agent_name: state.read().agent_name.clone(),
                    model: state.read().model.clone(),
                    provider: state.read().provider.clone(),
                    foreground_id: state.read().foreground_thread_id,
                    threads: state.read().threads.clone(),
                    on_settings: move |_| show_settings.set(true),
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
                } else if matches!(state.read().connection.clone(), ConnectionStatus::Connecting | ConnectionStatus::Authenticating) {
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

                Chat {
                    messages: state.read().messages.iter().cloned().collect::<Vec<_>>(),
                    input: state.read().input.clone(),
                    is_processing: state.read().is_processing,
                    is_thinking: state.read().is_thinking,
                    agent_name: state.read().agent_name.clone(),
                    on_submit: on_submit,
                    on_input_change: move |value| state.write().input = value,
                }
            }

            // Modals
            HatchingDialog {
                visible: *show_hatching.read(),
                on_complete: move |result: HatchingResult| {
                    if let Some(personality) = result.personality.clone() {
                        state.write().status_message = Some(format!("Personality set: {}", personality));
                    }
                    state.write().agent_name = Some(result.name);
                    show_hatching.set(false);
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
                on_close: move |_| show_settings.set(false),
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct TopBarProps {
    agent_name: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    foreground_id: Option<u64>,
    threads: Vec<ThreadInfo>,
    on_settings: EventHandler<()>,
}

#[component]
fn TopBar(props: TopBarProps) -> Element {
    let title = props
        .foreground_id
        .and_then(|id| props.threads.iter().find(|t| t.id == id).cloned())
        .and_then(|t| t.label.clone().or(Some(format!("Session #{}", t.id))))
        .unwrap_or_else(|| "New conversation".to_string());

    let sub_parts: Vec<String> = [
        props.agent_name.clone(),
        match (props.provider.as_ref(), props.model.as_ref()) {
            (Some(p), Some(m)) => Some(format!("{p} · {m}")),
            (None, Some(m)) => Some(m.clone()),
            (Some(p), None) => Some(p.clone()),
            _ => None,
        },
    ]
    .into_iter()
    .flatten()
    .collect();

    let sub_text = sub_parts.join(" — ");

    rsx! {
        div { class: "topbar",
            div {
                style: "display: flex; flex-direction: column; min-width: 0;",
                span { class: "topbar-title", "{title}" }
                if !sub_text.is_empty() {
                    span { class: "topbar-sub", "{sub_text}" }
                }
            }
            div { class: "topbar-right",
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

/// Connect to the gateway.
async fn connect_to_gateway(
    url: &str,
    mut state: Signal<AppState>,
    mut gateway: Signal<Option<Arc<Mutex<GatewayClient>>>>,
) {
    state.write().connection = ConnectionStatus::Connecting;

    match GatewayClient::connect(url).await {
        Ok(client) => {
            gateway.set(Some(Arc::new(Mutex::new(client))));
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
            state.write().status_message = Some(format!(
                "Tool approval requested: {} ({}) — {} chars",
                name,
                id,
                arguments.len()
            ));
        }
        GatewayEvent::ThreadsUpdate {
            threads,
            foreground_id,
        } => {
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
        GatewayEvent::Error { message } => {
            state.write().status_message = Some(message);
            state.write().is_processing = false;
        }
        GatewayEvent::Info { message } => {
            state.write().status_message = Some(message);
        }
    }
}
