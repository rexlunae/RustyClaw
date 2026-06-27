//! Modal/overlay dialog rendering for the desktop `App` component.

#![allow(unused_imports)]
use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Control, Field, FieldLabel, Notification,
};
use rustyclaw_view::tracing;

use crate::app_support::{
    connect_to_gateway, create_swarm_from_template, get_swarm_infos, stop_swarm,
};
use crate::components::*;
use crate::state::{AppState, Theme};
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::ui::{ConnectionStatus, ThreadInfo};
use rustyclaw_core::user_prompt_types::{PromptResponseValue, UserPrompt};
use rustyclaw_view::*;

use super::signals::{AppSignals, do_reconnect};

pub(super) fn render_dialogs(sig: AppSignals) -> Element {
    #[allow(unused_mut, unused_variables)]
    let AppSignals {
        mut state,
        mut gateway,
        mut did_auto_connect,
        mut active_event_client,
        mut auth_code,
        mut show_pairing,
        mut hatching_dialog,
        mut show_settings,
        mut show_swarm,
        mut swarm_creating,
        mut tool_approval_id,
        mut tool_approval_name,
        mut tool_approval_args,
        mut show_tool_approval,
        mut show_vault_unlock,
        mut vault_unlock_error,
        mut show_user_prompt,
        mut user_prompt_data,
        mut show_cred_request,
        mut cred_request_id,
        mut cred_request_provider,
        mut cred_request_secret,
        mut cred_request_message,
        mut qr_code_url,
        mut public_key,
        mut show_secrets,
        mut pending_thread_delete,
        mut did_init_directories,
        mut show_connection,
        mut connection_prefs,
    } = sig;

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
                        let _ = client.send(GatewayCommand::SecretsDelete { key }).await;
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

    rsx! {
            ConnectionDialog {
                visible: *show_connection.read(),
                gateway_url: state.read().gateway_url.clone(),
                status: state.read().connection.clone(),
                data: connection_prefs.read().clone(),
                on_connect: move |url: String| {
                    // Record in the history (most recent first); the default
                    // marker is only changed explicitly via the star toggle.
                    rustyclaw_core::client_prefs::record_recent_connection(&url);
                    connection_prefs.set(ConnectionDialogData::load());
                    state.write().gateway_url = url.clone();
                    // Mark auto-connect as done so it does not also fire when
                    // the dialog auto-closes after the connection succeeds.
                    did_auto_connect.set(true);
                    spawn(async move {
                        connect_to_gateway(&url, state, gateway).await;
                    });
                },
                on_set_default: move |(url, is_default): (String, bool)| {
                    rustyclaw_core::client_prefs::set_default_connection(&url, is_default);
                    connection_prefs.set(ConnectionDialogData::load());
                },
                on_remove: move |url: String| {
                    rustyclaw_core::client_prefs::remove_connection(&url);
                    connection_prefs.set(ConnectionDialogData::load());
                },
                on_toggle_autoconnect: move |enabled: bool| {
                    rustyclaw_core::client_prefs::set_autoconnect_on_startup(enabled);
                    connection_prefs.set(ConnectionDialogData::load());
                },
                on_cancel: move |_| show_connection.set(false),
            }

            HatchingDialog {
                data: {
                    let mut data = hatching_dialog.read().clone();
                    if !data.should_render(matches!(
                        state.read().connection,
                        ConnectionStatus::Authenticating
                    )) {
                        data.hide_temporarily();
                    }
                    data
                },
                on_update: move |data| hatching_dialog.set(data),
                on_complete: move |result: rustyclaw_view::HatchingResult| {
                    if let Some(personality) = result.personality.clone() {
                        state.write().status_message = Some(format!("Personality set: {}", personality));
                    }
                    let name = result.name.clone();
                    state.write().agent_name = Some(result.name);
                    // Persist the name to the gateway config.
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            let _ = client.send(GatewayCommand::SetAgentName { name }).await;
                        });
                    }
                },
                on_cancel: move |_| {},
            }

            PairingDialog {
                visible: *show_pairing.read(),
                data: PairingDialogData {
                    step: rustyclaw_view::PairingStep::EnterGateway,
                    field: rustyclaw_view::PairingField::Host,
                    public_key: public_key.read().clone().unwrap_or_default(),
                    fingerprint: String::new(),
                    fingerprint_art: String::new(),
                    qr_ascii: String::new(),
                    host: "127.0.0.1".to_string(),
                    port: "2222".to_string(),
                    error: String::new(),
                },
                qr_code_data_url: qr_code_url.read().clone(),
                on_host_change: move |_| {},
                on_port_change: move |_| {},
                on_connect: move |_| {
                    show_pairing.set(false);
                    do_reconnect(sig);
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
                on_reconnect: move |_| {
                    let url = state.read().gateway_url.clone();
                    crate::save_gateway_url(&url);
                    do_reconnect(sig);
                },
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
                data: ToolApprovalData {
                    id: tool_approval_id.read().clone(),
                    name: tool_approval_name.read().clone(),
                    arguments: tool_approval_args.read().clone(),
                    selected_allow: true,
                },
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
                data: VaultUnlockData {
                    password_len: 0,
                    error: vault_unlock_error.read().clone().unwrap_or_default(),
                },
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
                prompt_id: user_prompt_data
                    .read()
                    .as_ref()
                    .map(|p| p.id.clone())
                    .unwrap_or_default(),
                data: user_prompt_data.read().clone().map(UserPromptData::from),
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
                data: CredentialRequestData {
                    provider: cred_request_provider.read().clone(),
                    secret_name: cred_request_secret.read().clone(),
                    message: cred_request_message.read().clone(),
                    input_len: 0,
                },
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
                data: DeviceFlowData {
                    url: state
                        .read()
                        .pending_device_flow
                        .as_ref()
                        .map(|(u, _, _)| u.clone())
                        .unwrap_or_default(),
                    code: state
                        .read()
                        .pending_device_flow
                        .as_ref()
                        .map(|(_, c, _)| c.clone())
                        .unwrap_or_default(),
                    message: state
                        .read()
                        .pending_device_flow
                        .as_ref()
                        .and_then(|(_, _, m)| m.clone()),
                    browser_opened: false,
                    tick: 0,
                },
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
                RcModal {
                    active: true,
                    title: "Delete thread?",
                    width: 420,
                    class: "modal-confirm",
                    onclose: move |_| pending_thread_delete.set(None),
                    footer: rsx! {
                        Buttons {
                            Button {
                                color: BulmaColor::Light,
                                onclick: move |_| pending_thread_delete.set(None),
                                "Cancel"
                            }
                            Button {
                                color: BulmaColor::Danger,
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
                    },
                    p { "This will permanently delete \"{thread_label}\" and its messages." }
                    p { class: "modal-muted", "This action cannot be undone." }
                }
            }

            SystemInfoDialog {
                visible: state.read().show_system_info,
                host: state.read().host_info.clone(),
                load: state.read().load_status.clone(),
                on_close: move |_| state.write().show_system_info = false,
            }

            ServicesDialog {
                visible: state.read().show_services_dialog,
                services: state.read().services_data.clone(),
                on_close: move |_| state.write().show_services_dialog = false,
            }

            // TOTP authentication modal
            if matches!(state.read().connection.clone(), ConnectionStatus::Authenticating) {
                RcModal {
                    active: true,
                    title: "Gateway Authentication",
                    width: 420,
                    closable: false,
                    onclose: move |_| {},
                    footer: rsx! {
                        Buttons {
                            Button {
                                color: BulmaColor::Primary,
                                disabled: auth_code.read().len() != 6,
                                onclick: move |_| {
                                    let code: String = auth_code
                                        .read()
                                        .chars()
                                        .filter(|c| c.is_ascii_digit())
                                        .take(6)
                                        .collect();
                                    if code.len() != 6 {
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
                    },
                    p { class: "rc-dialog-lead",
                        "Enter the TOTP code from your authenticator app to connect to the gateway."
                    }
                    Field {
                        FieldLabel { "TOTP Code" }
                        Control {
                            input {
                                class: "input totp-input",
                                r#type: "text",
                                placeholder: "000000",
                                value: "{auth_code}",
                                autofocus: true,
                                maxlength: "6",
                                oninput: move |evt| {
                                    let sanitized: String = evt
                                        .value()
                                        .chars()
                                        .filter(|c| c.is_ascii_digit())
                                        .take(6)
                                        .collect();
                                    auth_code.set(sanitized);
                                },
                                onkeydown: move |evt: KeyboardEvent| {
                                    if evt.key() == Key::Enter {
                                        evt.prevent_default();
                                        let code: String = auth_code
                                            .read()
                                            .chars()
                                            .filter(|c| c.is_ascii_digit())
                                            .take(6)
                                            .collect();
                                        if code.len() != 6 {
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
                }
            }
    }
}
