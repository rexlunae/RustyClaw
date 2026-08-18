//! Modal/overlay dialog rendering for the desktop `App` component.
//!
//! This is a *component*, not a function `App` calls inline, and that is
//! load-bearing for typing latency. Several dialogs mirror what the user types
//! into a signal that `App` owns (the TOTP code, the agent name in the
//! hatching dialog, …). Called inline, every read of those signals subscribed
//! `App`'s scope, so one keystroke in a dialog re-rendered the entire window —
//! cloning the whole message list into `Chat`'s props and rebuilding the
//! sidebar tree per character. As a component this has its own reactive scope:
//! the same keystroke re-renders the dialogs and nothing else.
//!
//! Its props compare equal on every render (`Signal` is `PartialEq` by
//! identity), so `App` re-rendering does not re-render the dialogs either —
//! they update from the signals they read. See `docs/input-latency.md`.

#![allow(unused_imports)]
use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, BulmaSize, Button, Buttons, Control, Field, FieldLabel, Notification,
};
use rustyclaw_view::tracing;

use crate::app_support::{
    connect_to_gateway, create_swarm_from_template, delete_swarm, get_swarm_infos, spawn_reporting,
    stop_swarm,
};
use crate::components::*;
use crate::state::{AppState, Theme};
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::gateway::{EngineActionKind, GatewayClient, ModelActionKind};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ConnectionStatus, ThreadInfo};
use rustyclaw_view::*;

use super::signals::{AppSignals, do_reconnect};
use anyhow::Context;

#[component]
pub(super) fn Dialogs(sig: AppSignals) -> Element {
    #[allow(unused_mut, unused_variables)]
    let AppSignals {
        mut state,
        mut gateway,
        mut did_auto_connect,
        mut active_event_client,
        mut show_pairing,
        mut show_settings,
        mut show_swarm,
        mut swarm_creating,
        mut tool_approval_id,
        mut tool_approval_name,
        mut tool_approval_args,
        mut show_tool_approval,
        mut show_vault_unlock,
        mut vault_unlock_error,
        mut show_cred_request,
        mut cred_request_id,
        mut cred_request_provider,
        mut cred_request_secret,
        mut cred_request_message,
        mut qr_code_url,
        mut public_key,
        mut show_secrets,
        mut show_messengers,
        mut pending_thread_delete,
        mut did_init_directories,
        mut show_connection,
        mut connection_prefs,
    } = sig;

    // Draft text typed into a dialog. Owned here rather than in `App` so a
    // keystroke re-renders this scope only — see the module docs.
    let mut auth_code = use_signal(String::new);
    let mut hatching_dialog = use_signal(|| HatchingDialogData::new(state.read().needs_hatching));

    let on_secrets_command = move |cmd: SecretsCommand| {
        // Dropping revealed plaintext is local state, not a gateway round
        // trip — and it must still work when the connection has gone away.
        if matches!(cmd, SecretsCommand::ClearReveal) {
            state.write().secrets_data.clear_reveal();
            return;
        }
        // Record which secret a fresh reveal is for, so the result knows what
        // to label and a TOTP retry knows what to re-request. Only on the
        // first press — a retry carrying a code must keep the existing state.
        if let SecretsCommand::Peek { name, code: None } = &cmd {
            state.write().secrets_data.start_reveal(name);
        }
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("secrets command", async move {
                match cmd {
                    SecretsCommand::Refresh => {
                        client
                            .send(GatewayCommand::SecretsList)
                            .await
                            .context("sending SecretsList")?;
                        client
                            .send(GatewayCommand::SecretsHasTotp)
                            .await
                            .context("sending SecretsHasTotp")?;
                    }
                    SecretsCommand::Store { key, value } => {
                        client
                            .send(GatewayCommand::SecretsStore { key, value })
                            .await
                            .context("sending SecretsStore")?;
                        // Re-fetch so the new entry shows up immediately
                        // (the gateway handles frames in order).
                        client
                            .send(GatewayCommand::SecretsList)
                            .await
                            .context("sending SecretsList")?;
                    }
                    SecretsCommand::Delete { key } => {
                        client
                            .send(GatewayCommand::SecretsDelete { key })
                            .await
                            .context("sending SecretsDelete")?;
                        client
                            .send(GatewayCommand::SecretsList)
                            .await
                            .context("sending SecretsList")?;
                    }
                    SecretsCommand::Peek { name, code } => {
                        client
                            .send(GatewayCommand::SecretsPeek { name, code })
                            .await
                            .context("sending SecretsPeek")?;
                    }
                    // Handled before the gateway lookup above.
                    SecretsCommand::ClearReveal => {}
                    SecretsCommand::SetPolicy { name, policy } => {
                        client
                            .send(GatewayCommand::SecretsSetPolicy {
                                name,
                                policy,
                                skills: Vec::new(),
                            })
                            .await
                            .context("sending SecretsSetPolicy")?;
                        client
                            .send(GatewayCommand::SecretsList)
                            .await
                            .context("sending SecretsList")?;
                    }
                }
                Ok(())
            });
        }
    };

    // Every messenger mutation is followed by a refresh request. The gateway
    // pushes a fresh view after a successful change, but not after a rejected
    // one — and a form left showing what the gateway refused is a form that
    // lies about what is saved.
    let on_messenger_command = move |cmd: MessengerCommand| {
        let gw = gateway.read().clone();
        if let Some(client) = gw {
            spawn_reporting("send command", async move {
                let command = match cmd {
                    MessengerCommand::Refresh => GatewayCommand::MessengerConfig,
                    MessengerCommand::SaveAccount {
                        original_name,
                        name,
                        messenger_type,
                        enabled,
                        fields,
                        secrets,
                        display_name,
                        bio,
                        avatar_path,
                    } => GatewayCommand::MessengerAccountSave {
                        original_name,
                        name,
                        messenger_type,
                        enabled,
                        fields,
                        secrets,
                        display_name,
                        bio,
                        avatar_path,
                        agent_id: None,
                    },
                    MessengerCommand::DeleteAccount { name } => {
                        GatewayCommand::MessengerAccountDelete { name }
                    }
                    MessengerCommand::MigrateSecrets { name } => {
                        GatewayCommand::MessengerSecretsMigrate { name }
                    }
                    MessengerCommand::SaveRoute {
                        messenger,
                        channel,
                        thread_id,
                        agent_id,
                        enabled,
                    } => GatewayCommand::MessengerRouteSave {
                        messenger,
                        channel,
                        thread_id,
                        agent_id,
                        enabled,
                    },
                    MessengerCommand::DeleteRoute { messenger, channel } => {
                        GatewayCommand::MessengerRouteDelete { messenger, channel }
                    }
                };
                client.send(command).await.context("sending command")?;
                Ok(())
            });
        }
    };

    // The sign-in prompt this render displays, snapshotted so closing it
    // retires exactly this one. Reading the queue head again at click time
    // races the gateway's own retirements — a completion or close-out can
    // replace the head between render and click, and acting on the new
    // head would cancel a turn the user was not looking at.
    let displayed_device_flow = state.read().pending_device_flows.front().cloned();
    let displayed_device_flow_code = displayed_device_flow
        .as_ref()
        .map(|(_, _, code, _)| code.clone());

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
                        state.write().push_notice(
                            MessageRole::Success,
                            format!("Personality set: {}", personality),
                        );
                    }
                    let name = result.name.clone();
                    state.write().agent_name = Some(result.name);
                    // Persist the name to the gateway config.
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("rename agent", async move {
                            client.send(GatewayCommand::SetAgentName { name }).await.context("sending SetAgentName")?;
                            Ok(())
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
                custom_providers: state.read().custom_providers.clone(),
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
                    state.write().push_notice(
                        MessageRole::Success,
                        format!(
                            "API key saved for {}",
                            rustyclaw_core::providers::display_name_for_provider(&provider_id)
                        ),
                    );
                },
                on_custom_provider_add: move |cfg: rustyclaw_core::providers::CustomProviderConfig| {
                    // Persist to the local config; `Config::save` re-registers
                    // the runtime provider catalogue, so the model bar picks
                    // the new provider up immediately.
                    //
                    // `reload_config`, not `resolved_config`: this is a
                    // read-modify-write, so it needs the file's current
                    // contents rather than the startup snapshot. It is also
                    // not `Config::load(None)`, which read the *default*
                    // location and then saved to that config's settings dir —
                    // under `--profile` the provider landed in the wrong
                    // installation.
                    match crate::reload_config() {
                        Ok(mut config) => {
                            let id = cfg.id.clone();
                            let replaced = config
                                .custom_providers
                                .iter()
                                .position(|p| p.id == id);
                            match replaced {
                                Some(idx) => config.custom_providers[idx] = cfg,
                                None => config.custom_providers.push(cfg),
                            }
                            match config.save(None) {
                                Ok(()) => {
                                    let mut s = state.write();
                                    s.custom_providers = config.custom_providers.clone();
                                    s.push_notice(
                                        MessageRole::Success,
                                        if replaced.is_some() {
                                            format!("Updated custom provider '{}'.", id)
                                        } else {
                                            format!("Added custom provider '{}'.", id)
                                        },
                                    );
                                }
                                Err(e) => state.write().push_notice(
                                    MessageRole::Error,
                                    format!("Failed to save config: {}", e),
                                ),
                            }
                        }
                        Err(e) => state.write().push_notice(
                            MessageRole::Error,
                            format!("Failed to load config: {}", e),
                        ),
                    }
                },
                on_custom_provider_remove: move |id: String| {
                    // Same read-modify-write as the add handler above.
                    match crate::reload_config() {
                        Ok(mut config) => {
                            let before = config.custom_providers.len();
                            config.custom_providers.retain(|p| p.id != id);
                            if config.custom_providers.len() == before {
                                state.write().push_notice(
                                    MessageRole::Error,
                                    format!("No custom provider named '{}'.", id),
                                );
                            } else {
                                match config.save(None) {
                                    Ok(()) => {
                                        let mut s = state.write();
                                        s.custom_providers = config.custom_providers.clone();
                                        s.push_notice(
                                            MessageRole::Success,
                                            format!("Removed custom provider '{}'.", id),
                                        );
                                    }
                                    Err(e) => state.write().push_notice(
                                        MessageRole::Error,
                                        format!("Failed to save config: {}", e),
                                    ),
                                }
                            }
                        }
                        Err(e) => state.write().push_notice(
                            MessageRole::Error,
                            format!("Failed to load config: {}", e),
                        ),
                    }
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
                            state.write().push_notice(
                                MessageRole::Error,
                                format!("Failed to create swarm: {}", e),
                            );
                        }
                    });
                },
                on_stop: move |name: String| {
                    if let Err(e) = stop_swarm(&name) {
                        state.write().push_notice(
                            MessageRole::Error,
                            format!("Failed to stop swarm: {}", e),
                        );
                    }
                },
                on_delete: move |name: String| {
                    if let Err(e) = delete_swarm(&name) {
                        state.write().push_notice(
                            MessageRole::Error,
                            format!("Failed to delete swarm: {}", e),
                        );
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
                    // By id, not pop_front: the gateway can retire the displayed
                    // entry between render and click, and popping the head
                    // would discard a request that was never shown.
                    state.write().retire_tool_approval(None, &id);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("answer tool approval", async move {
                            client.send(GatewayCommand::ToolApprove { id, approved: true }).await.context("sending ToolApprove")?;
                            Ok(())
                        });
                    }
                },
                on_deny: move |id: String| {
                    // By id, not pop_front: the gateway can retire the displayed
                    // entry between render and click, and popping the head
                    // would discard a request that was never shown.
                    state.write().retire_tool_approval(None, &id);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("answer tool approval", async move {
                            client.send(GatewayCommand::ToolApprove { id, approved: false }).await.context("sending ToolApprove")?;
                            Ok(())
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
                        spawn_reporting("unlock vault", async move {
                            client.send(GatewayCommand::VaultUnlock { password }).await.context("sending VaultUnlock")?;
                            Ok(())
                        });
                    }
                },
                on_cancel: move |_| show_vault_unlock.set(false),
            }

            // The agent's structured questions (`ask_user` tool) render
            // inline in the chat stream (see `components::UserPromptCard`),
            // not as a modal.

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
                    state.write().retire_credential_request(&id);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("submit credential", async move {
                            client.send(GatewayCommand::CredentialResponse {
                                id,
                                dismissed: false,
                                value: Some(value),
                            }).await.context("sending CredentialResponse")?;
                            Ok(())
                        });
                    }
                },
                on_dismiss: move |id: String| {
                    state.write().retire_credential_request(&id);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("submit credential", async move {
                            client.send(GatewayCommand::CredentialResponse {
                                id,
                                dismissed: true,
                                value: None,
                            }).await.context("sending CredentialResponse")?;
                            Ok(())
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

            MessengersDialog {
                visible: *show_messengers.read(),
                data: state.read().messengers_data.clone(),
                on_command: on_messenger_command,
                on_close: move |_| show_messengers.set(false),
            }

            DeviceFlowDialog {
                visible: displayed_device_flow.is_some(),
                data: DeviceFlowData {
                    url: displayed_device_flow
                        .as_ref()
                        .map(|(_, u, _, _)| u.clone())
                        .unwrap_or_default(),
                    code: displayed_device_flow
                        .as_ref()
                        .map(|(_, _, c, _)| c.clone())
                        .unwrap_or_default(),
                    message: displayed_device_flow
                        .as_ref()
                        .and_then(|(_, _, _, m)| m.clone()),
                    browser_opened: false,
                    tick: 0,
                },
                on_close: move |_| {
                    // Cancel the turn the flow belongs to — which need not
                    // be the conversation on screen, now that any turn can
                    // start a sign-in. Aiming at the foreground here could
                    // kill an unrelated reply while the poll ran on.
                    // By the displayed prompt's code, not pop_front: the
                    // gateway can retire the head between render and click,
                    // and popping would discard a prompt that was never
                    // shown and cancel that other flow's turn instead. If
                    // the displayed flow is already gone, so is its turn —
                    // there is nothing left to cancel.
                    let Some(code) = displayed_device_flow_code.clone() else {
                        return;
                    };
                    let Some(thread_id) = state.write().retire_device_flow(&code) else {
                        return;
                    };
                    state
                        .write()
                        .push_notice(MessageRole::Info, "Device flow cancelled.");
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("cancel turn", async move {
                            client.send(GatewayCommand::Cancel { thread_id }).await.context("sending Cancel")?;
                            Ok(())
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
                                        spawn_reporting("delete thread", async move {
                                            if let Some(fallback_id) = fallback_id {
                                                client.send(GatewayCommand::ThreadSwitch { thread_id: fallback_id }).await.context("sending ThreadSwitch")?;
                                            }
                                            client.send(GatewayCommand::ThreadClose { thread_id }).await.context("sending ThreadClose")?;
                                            Ok(())
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

            GatewayControlDialog {
                visible: state.read().show_gateway_dialog,
                data: {
                    // The daemon half is a stored snapshot; the connection
                    // half is live, so it is read here rather than mirrored
                    // into the snapshot and left to go stale the moment the
                    // session drops while the dialog is open.
                    let s = state.read();
                    rustyclaw_view::GatewayControlData {
                        url: s.gateway_url.clone(),
                        connected: s.is_connected(),
                        ..s.gateway_control.clone()
                    }
                },
                on_command: move |cmd| {
                    use rustyclaw_view::PendingAction;
                    let action = match cmd {
                        GatewayControlCommand::Start => Some(PendingAction::Start),
                        GatewayControlCommand::Stop => Some(PendingAction::Stop),
                        GatewayControlCommand::Restart => Some(PendingAction::Restart),
                        GatewayControlCommand::Reload => None,
                    };
                    match action {
                        Some(action) => crate::app_support::run_gateway_action(state, action),
                        None => {
                            // Reload travels over the session. Its outcome
                            // arrives as ModelReloaded or Error and lands in
                            // the transcript, so there is nothing to write
                            // into the panel here.
                            let gw = gateway.read().clone();
                            if let Some(client) = gw {
                                spawn_reporting("reload gateway config", async move {
                                    client
                                        .send(GatewayCommand::Reload)
                                        .await
                                        .context("sending Reload")?;
                                    Ok(())
                                });
                            }
                        }
                    }
                },
                on_close: move |_| state.write().show_gateway_dialog = false,
            }

            SystemInfoDialog {
                visible: state.read().show_system_info,
                host: state.read().host_info.clone(),
                load: state.read().load_status.clone(),
                on_close: move |_| state.write().show_system_info = false,
            }

            DownloadsDialog {
                visible: state.read().show_downloads_dialog,
                downloads: state.read().downloads.clone(),
                on_close: move |_| state.write().show_downloads_dialog = false,
                on_cancel: move |id: String| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("cancel download", async move {
                            client
                                .send(GatewayCommand::DownloadCancel { id })
                                .await
                                .context("sending DownloadCancel")?;
                            Ok(())
                        });
                    }
                },
                on_clear_finished: move |_| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("clear finished downloads", async move {
                            client
                                .send(GatewayCommand::DownloadsClearFinished)
                                .await
                                .context("sending DownloadsClearFinished")?;
                            Ok(())
                        });
                    }
                },
            }

            ServicesDialog {
                visible: state.read().show_services_dialog,
                services: state.read().services_data.clone(),
                on_close: move |_| state.write().show_services_dialog = false,
            }

            EnginesDialog {
                visible: state.read().show_engines_dialog,
                data: state.read().engines_data.clone(),
                on_close: move |_| {
                    let mut s = state.write();
                    s.show_engines_dialog = false;
                    // The inline action result belongs to the dialog; don't
                    // let it linger into the next open.
                    s.engine_action_result = None;
                },
                action_pending: state.read().engine_model_action_pending.clone(),
                action_result: state.read().engine_action_result.clone(),
                on_clear_action_result: move |_| {
                    state.write().engine_action_result = None;
                },
                on_engine_action: move |(engine, action): (String, EngineActionKind)| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client
                                .send(GatewayCommand::EngineAction { engine, action })
                                .await
                            {
                                tracing::error!("Failed to send engine action: {}", e);
                            }
                        });
                    }
                },
                on_model_action: move |(engine, model, action): (String, String, ModelActionKind)| {
                    state
                        .write()
                        .engine_model_action_pending = Some((engine.clone(), model.clone()));
                    // A Load should honour the context window saved in the
                    // engine's parameters (the gateway maps it per engine:
                    // --n-ctx / --ctx-size / --num-ctx).
                    let saved_ctx = state
                        .read()
                        .engines_data
                        .as_ref()
                        .and_then(|d| d.engine(&engine))
                        .and_then(|e| e.config.context_length);
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client
                                .send(GatewayCommand::EngineModelAction {
                                    engine,
                                    model,
                                    action,
                                    context_length: saved_ctx,
                                    extra_args: Vec::new(),
                                })
                                .await
                            {
                                tracing::error!("Failed to send model action: {}", e);
                            }
                        });
                    }
                },
                on_pull: move |(engine, model): (String, String)| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client
                                .send(GatewayCommand::EngineModelPull {
                                    engine,
                                    model,
                                    expected_size_bytes: None,
                                })
                                .await
                            {
                                tracing::error!("Failed to send model pull: {}", e);
                            }
                        });
                    }
                },
                on_select_engine: move |engine: String| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client
                                .send(GatewayCommand::EngineModelList { engine })
                                .await
                            {
                                tracing::error!("Failed to request engine models: {}", e);
                            }
                        });
                    }
                },
                on_use_model: move |(engine, model): (String, String)| {
                    // Local engine ids double as provider ids (ollama,
                    // lmstudio, llamacpp, exo), so switching the chat to a
                    // local model is a regular provider/model switch.
                    let gw = gateway.read().clone();
                    let provider = engine.clone();
                    let model_for_state = model.clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client
                                .send(GatewayCommand::ModelSwitch { provider, model })
                                .await
                            {
                                tracing::error!("Failed to switch to local model: {}", e);
                            }
                        });
                    }
                    let mut s = state.write();
                    s.provider = Some(engine.clone());
                    s.model = Some(model_for_state.clone());
                    s.show_engines_dialog = false;
                    s.push_notice(
                        MessageRole::Success,
                        format!("Switched to {} / {}", engine, model_for_state),
                    );
                },
                on_config_save: move |(engine, config): (String, rustyclaw_core::engines::EngineConfig)| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client
                                .send(GatewayCommand::EngineConfigSet { engine, config })
                                .await
                            {
                                tracing::error!("Failed to save engine config: {}", e);
                            }
                        });
                    }
                },
                configs_received: state.read().engine_configs_received,
                on_refresh: move |_| {
                    let gw = gateway.read().clone();
                    let selected = state
                        .read()
                        .engines_data
                        .as_ref()
                        .and_then(|d| d.selected_engine.clone());
                    if let Some(client) = gw {
                        spawn(async move {
                            if let Err(e) = client.send(GatewayCommand::EngineList).await {
                                tracing::error!("Failed to request engine list: {}", e);
                            }
                            if let Some(engine) = selected
                                && let Err(e) = client
                                    .send(GatewayCommand::EngineModelList { engine })
                                    .await
                            {
                                tracing::error!("Failed to request engine models: {}", e);
                            }
                        });
                    }
                },
            }

            CronDialog {
                visible: state.read().show_cron_dialog,
                data: state.read().cron_data.clone(),
                threads: state
                    .read()
                    .threads
                    .iter()
                    .map(|t| (t.id, t.label.clone().unwrap_or_else(|| format!("Thread {}", t.id))))
                    .collect::<Vec<_>>(),
                // Same live lists the composer's model bar draws on, so the
                // wake editor offers exactly the models the session can use.
                provider_models: state.read().provider_models.clone(),
                on_provider_pick: move |provider: String| {
                    // The standing fetch in `App` only follows the session's
                    // own provider, so a wake pinned to a different one would
                    // see the static catalogue — empty, for every local
                    // server. Ask for the real list on demand, sharing the
                    // app's per-provider guard so re-opening the dialog does
                    // not re-ask.
                    if state.read().provider_models_requested.contains(&provider) {
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
                },
                on_close: move |_| state.write().show_cron_dialog = false,
                on_command: move |cmd: crate::components::CronCommand| {
                    let gw = gateway.read().clone();
                    if let Some(client) = gw {
                        spawn_reporting("cron command", async move {
                            match cmd {
                                crate::components::CronCommand::Action { id, action } => {
                                    client
                                        .send(GatewayCommand::CronAction { id, action })
                                        .await
                                        .context("sending CronAction")?;
                                }
                                crate::components::CronCommand::Save {
                                    id,
                                    name,
                                    expr,
                                    prompt,
                                    provider,
                                    model,
                                    paused,
                                    thread_id,
                                    clear_thread,
                                } => {
                                    client
                                        .send(GatewayCommand::CronUpsert {
                                            id,
                                            name,
                                            expr,
                                            payload: prompt,
                                            // Carried from the row, not
                                            // assumed: the editor has no
                                            // pause control, and sending
                                            // `false` here resumed a paused
                                            // wake on any unrelated edit.
                                            paused,
                                            // The editor authors wakes: the
                                            // prompt runs as an agent turn.
                                            agent_turn: true,
                                            provider,
                                            model,
                                            thread_id,
                                            clear_thread,
                                        })
                                        .await
                                        .context("sending CronUpsert")?;
                                }
                            }
                            // Re-fetch so the panel reflects the change
                            // (the gateway handles frames in order).
                            client
                                .send(GatewayCommand::CronList)
                                .await
                                .context("sending CronList")?;
                            Ok(())
                        });
                    }
                },
            }

            MemoryDialog {
                visible: state.read().show_memory_dialog,
                data: state.read().memory_data.clone(),
                on_close: move |_| state.write().show_memory_dialog = false,
            }

            McpDialog {
                visible: state.read().show_mcp_dialog,
                data: state.read().mcp_data.clone(),
                on_close: move |_| state.write().show_mcp_dialog = false,
            }

            ChannelsDialog {
                visible: state.read().show_channels_dialog,
                data: state.read().channels_data.clone(),
                on_close: move |_| state.write().show_channels_dialog = false,
            }

            ToolsConfigDialog {
                visible: state.read().show_tools_dialog,
                data: state.read().tools_data.clone(),
                on_close: move |_| state.write().show_tools_dialog = false,
            }

            SkillsDialog {
                visible: state.read().show_skills_dialog,
                skills: state.read().skills_data.clone(),
                on_toggle: move |name: String| {
                    let skills = crate::app_support::toggle_skill(&name);
                    state.write().skills_data = skills;
                },
                on_close: move |_| state.write().show_skills_dialog = false,
            }

            AnalyticsDialog {
                visible: state.read().show_analytics_dialog,
                data: state.read().analytics_data.clone(),
                on_close: move |_| state.write().show_analytics_dialog = false,
            }

            LogsDialog {
                visible: state.read().show_logs_dialog,
                data: state.read().logs_data.clone(),
                on_close: move |_| state.write().show_logs_dialog = false,
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
                            Button {
                                color: BulmaColor::Danger,
                                onclick: move |_| {
                                    // Give up on this gateway: close the
                                    // connection and return to the connection
                                    // dialog so a different one can be picked.
                                    let gw = gateway.read().clone();
                                    if let Some(client) = gw {
                                        spawn(async move {
                                            client.close().await;
                                        });
                                    }
                                    gateway.set(None);
                                    active_event_client.set(None);
                                    auth_code.set(String::new());
                                    state.write().connection = ConnectionStatus::Disconnected;
                                    show_connection.set(true);
                                },
                                "Cancel"
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
