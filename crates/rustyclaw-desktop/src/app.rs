//! Main application component.

use std::sync::Arc;
use dioxus::prelude::*;
use dioxus_bulma::prelude::*;
use tokio::sync::Mutex;

use crate::components::{Chat, HatchingDialog, PairingDialog, Sidebar};
use crate::gateway::{GatewayClient, GatewayEvent};
use crate::state::{AppState, ChatMessage, ConnectionStatus, MessageRole, ThreadInfo};

/// Main application component.
#[component]
pub fn App() -> Element {
    // Application state
    let mut state = use_signal(AppState::default);
    
    // Gateway client (optional, set when connected)
    let gateway: Signal<Option<Arc<Mutex<GatewayClient>>>> = use_signal(|| None);
    
    // Dialog visibility
    let mut show_pairing = use_signal(|| false);
    let mut show_hatching = use_signal(|| false);
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
    
    // Handlers
    let on_submit = move |message: String| {
        // Add user message
        state.write().add_user_message(message.clone());
        state.write().is_processing = true;
        
        // Send to gateway
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
                let _ = client_guard.send(crate::gateway::protocol::GatewayCommand::ThreadCreate { label: None }).await;
            });
        }
    };
    
    let on_switch_thread = move |thread_id: u64| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn(async move {
                let client_guard = client.lock().await;
                let _ = client_guard.send(crate::gateway::protocol::GatewayCommand::ThreadSwitch { thread_id }).await;
            });
        }
        // Clear messages when switching
        state.write().clear_messages();
    };
    
    rsx! {
        // Include Bulma CSS
        link { rel: "stylesheet", href: "https://cdn.jsdelivr.net/npm/bulma@0.9.4/css/bulma.min.css" }
        link { rel: "stylesheet", href: "https://cdnjs.cloudflare.com/ajax/libs/font-awesome/6.4.0/css/all.min.css" }
        
        // Custom styles
        style { r#"
            .app-container {{
                display: flex;
                height: 100vh;
                overflow: hidden;
            }}
            .main-content {{
                flex: 1;
                display: flex;
                flex-direction: column;
                overflow: hidden;
            }}
            .streaming-cursor {{
                animation: blink 1s infinite;
            }}
            @keyframes blink {{
                0%, 50% {{ opacity: 1; }}
                51%, 100% {{ opacity: 0; }}
            }}
        "# }
        
        div { class: "app-container",
            // Sidebar
            Sidebar {
                connection: state.read().connection.clone(),
                agent_name: state.read().agent_name.clone(),
                model: state.read().model.clone(),
                provider: state.read().provider.clone(),
                threads: state.read().threads.clone(),
                foreground_id: state.read().foreground_thread_id,
                on_new_thread: on_new_thread,
                on_switch_thread: on_switch_thread,
                on_settings: move |_| show_settings.set(true),
            }
            
            // Main content
            div { class: "main-content",
                // Status bar
                if let Some(msg) = &state.read().status_message {
                    Notification {
                        color: Color::Info,
                        style: "margin: 0; border-radius: 0;",
                        
                        button { 
                            class: "delete",
                            onclick: move |_| state.write().status_message = None,
                        }
                        "{msg}"
                    }
                }
                
                // Chat area
                Chat {
                    messages: state.read().messages.iter().cloned().collect(),
                    input: state.read().input.clone(),
                    is_processing: state.read().is_processing,
                    is_thinking: state.read().is_thinking,
                    on_submit: on_submit,
                    on_input_change: move |value| state.write().input = value,
                }
            }
        }
        
        // Dialogs
        HatchingDialog {
            visible: *show_hatching.read(),
            on_complete: move |result| {
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
            gateway_port: 9001,
            on_host_change: move |_| {},
            on_port_change: move |_| {},
            on_connect: move |_| {
                show_pairing.set(false);
                let url = state.read().gateway_url.clone();
                spawn(async move {
                    connect_to_gateway(&url, state, gateway).await;
                });
            },
            on_generate_key: move |_| {
                // Generate keypair (placeholder)
                public_key.set(Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA... desktop-client".to_string()));
                if let Some(key) = &*public_key.read() {
                    qr_code_url.set(crate::components::pairing::generate_qr_code(key));
                }
            },
            on_cancel: move |_| show_pairing.set(false),
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
        GatewayEvent::Connected { agent, vault_locked, provider, model } => {
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
        GatewayEvent::AuthFailed { message, .. } => {
            state.write().status_message = Some(format!("Auth failed: {}", message));
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
        GatewayEvent::ToolCall { id, name, arguments } => {
            state.write().add_tool_call(id, name, arguments);
        }
        GatewayEvent::ToolResult { id, result, is_error, .. } => {
            state.write().set_tool_result(&id, result, is_error);
        }
        GatewayEvent::ToolApprovalRequest { .. } => {
            // TODO: Show approval dialog
        }
        GatewayEvent::ThreadsUpdate { threads, foreground_id } => {
            state.write().threads = threads.into_iter().map(|t| ThreadInfo {
                id: t.id,
                label: t.label,
                description: t.description,
                status: t.status,
                is_foreground: t.is_foreground,
                message_count: t.message_count,
            }).collect();
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
