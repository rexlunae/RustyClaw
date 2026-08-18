//! Support functions for the desktop `App` component: gateway connection,
//! gateway-event application, DOM queries, directory helpers, and swarm ops.

use std::sync::Arc;

use dioxus::prelude::*;
use rustyclaw_view::{serde_json, tracing};

use crate::state::AppState;
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::gateway::client::ThreadEvent;
use rustyclaw_core::gateway::client_types::{GatewayCommand, GatewayEvent};
use rustyclaw_core::types::MessageRole;
use rustyclaw_core::ui::{ConnectionStatus, ThreadInfo};
use rustyclaw_view::{SecretsDialogData, SwarmAgentData, SwarmData};

// ── Shared buffer for the worker → UI bridge ───────────────────────────────

/// An entry in the ordered event buffer.  Consecutive Chunk events
/// are coalesced into a single `Chunks` entry to reduce signal writes,
/// while preserving the ordering of non-chunk events relative to chunks.
//
// The size gap the lint sees is `GatewayEvent`'s largest variant, and its
// suggestion — boxing — is the wrong trade here: entries live in a Vec
// that is pushed once by the worker and drained once per UI tick, so the
// move cost the lint prices in is paid once per event either way, while a
// Box would add an allocation per event on the bridge's hot path.
#[allow(clippy::large_enum_variant)]
pub(crate) enum BufferEntry {
    Event {
        /// The thread whose turn produced this, when it is turn-scoped.
        thread_id: Option<u64>,
        event: GatewayEvent,
    },
    Chunks {
        /// Coalescing is per thread: with a turn running in each of two
        /// threads, merging their chunks by adjacency alone would splice two
        /// different answers into one string.
        thread_id: Option<u64>,
        text: String,
        count: u32,
        bytes: usize,
    },
}

/// Intermediate buffer between the tokio event-consumer worker and
/// the Dioxus UI task.  The worker writes at full speed; the UI task
/// drains on each `Notify` wake-up.
#[derive(Default)]
pub(crate) struct EventBuffer {
    pub(crate) entries: Vec<BufferEntry>,
}

impl EventBuffer {
    /// Append events, coalescing consecutive chunks of the same thread.
    ///
    /// Shared by the worker's steady-state path and its shutdown drain: a
    /// terminal `Disconnected` is the event the UI most needs, and it is the
    /// one most likely to be sitting in the channel when the worker notices
    /// the connection is gone.
    pub(crate) fn push_events(&mut self, events: impl IntoIterator<Item = ThreadEvent>) {
        for threaded in events {
            let thread = threaded.thread_id;
            match threaded.event {
                GatewayEvent::Chunk { delta } => match self.entries.last_mut() {
                    Some(BufferEntry::Chunks {
                        thread_id,
                        text,
                        count,
                        bytes,
                    }) if *thread_id == thread => {
                        *count += 1;
                        *bytes += delta.len();
                        text.push_str(&delta);
                    }
                    _ => self.entries.push(BufferEntry::Chunks {
                        thread_id: thread,
                        text: delta.clone(),
                        count: 1,
                        bytes: delta.len(),
                    }),
                },
                other => self.entries.push(BufferEntry::Event {
                    thread_id: thread,
                    event: other,
                }),
            }
        }
    }
}

/// Connect to the gateway.
pub(crate) async fn connect_to_gateway_candidates(
    urls: Vec<String>,
    mut state: Signal<AppState>,
    gateway: Signal<Option<Arc<GatewayClient>>>,
) -> bool {
    for url in urls {
        state.write().gateway_url = url.clone();
        connect_to_gateway(&url, state, gateway).await;
        if state.read().is_connected() {
            crate::save_gateway_url(&url);
            return true;
        }
    }
    false
}

/// Connect to the gateway.
pub(crate) async fn connect_to_gateway(
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
            // The whole chain, not just the outermost context. `connect`
            // wraps every failure in "Gateway at … is not responding", but
            // the cause underneath is what's actionable — "Permission denied
            // (publickey)", "Connection refused", a host-key mismatch — and
            // `to_string` on an anyhow error prints only the top layer.
            state.write().connection = ConnectionStatus::Error(format!("{:#}", e));
            tracing::error!("Failed to connect to gateway: {:#}", e);
        }
    }
}

/// Handle a gateway event.
pub(crate) fn handle_gateway_event(
    thread_id: Option<u64>,
    event: GatewayEvent,
    mut state: Signal<AppState>,
) {
    match event {
        GatewayEvent::Connected {
            agent,
            vault_locked,
            provider,
            model,
        } => {
            let mut s = state.write();
            s.connection = ConnectionStatus::Connected;
            s.agent_name = agent;
            s.vault_locked = vault_locked;
            s.provider = provider.map(|p| normalize_provider_id(&p).to_string());
            s.model = model;
            // A fresh session has nothing in flight; clear any indicator
            // state left over from a request the old connection dropped, so
            // it can't block history hydration or show a phantom spinner.
            s.is_processing = false;
            s.is_streaming = false;
            s.is_thinking = false;
            s.in_flight.clear();
            s.unowned_turn_in_flight = false;
            // Requests queued under the old connection can never be
            // answered on this one.
            s.clear_user_prompt();
            s.pending_tool_approvals.clear();
            s.pending_credential_requests.clear();
            s.pending_device_flows.clear();
        }
        GatewayEvent::Disconnected { reason } => {
            let mut s = state.write();
            s.connection = ConnectionStatus::Disconnected;
            if let Some(r) = reason {
                s.push_notice(MessageRole::Warning, format!("Disconnected: {}", r));
            }
            // The in-flight request (if any) died with the connection.
            s.is_processing = false;
            s.is_streaming = false;
            s.is_thinking = false;
            s.in_flight.clear();
            s.unowned_turn_in_flight = false;
            // Including anything it was waiting on: the turns that asked
            // are gone, so no tool result will ever retire these and an
            // answer would go into a closed connection. Leaving them up
            // would be an invitation to answer nothing.
            s.clear_user_prompt();
            s.pending_tool_approvals.clear();
            s.pending_credential_requests.clear();
            s.pending_device_flows.clear();
        }
        GatewayEvent::AuthRequired => {
            state.write().connection = ConnectionStatus::Authenticating;
        }
        GatewayEvent::AuthSuccess => {
            state.write().connection = ConnectionStatus::Authenticated;
        }
        GatewayEvent::AuthFailed { message, retry } => {
            let text = if retry {
                format!("Auth failed (retry allowed): {}", message)
            } else {
                format!("Auth failed: {}", message)
            };
            state.write().push_notice(MessageRole::Error, text);
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
            state
                .write()
                .push_notice(MessageRole::Error, format!("Model error: {}", message));
        }
        // A turn opens by naming its thread, and closes the same way. The
        // frames in between carry no id — they belong to the turn, and the
        // turn's thread is `streaming_thread_id`. When the user has switched
        // away from it, they must not touch the on-screen view or its
        // indicators; the backgrounded thread's transcript arrives via the
        // gateway's history snapshot on completion.
        GatewayEvent::StreamStart {
            thread_id: announced,
        } => {
            let mut s = state.write();
            // The gateway's answer to "which thread is this turn in" beats
            // the guess made when the message was sent.
            s.adopt_stream_thread(announced);
            if s.frame_targets_view(announced) {
                s.start_assistant_message();
            }
        }
        GatewayEvent::ThinkingStart => {
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.start_thinking_message();
            }
        }
        GatewayEvent::ThinkingEnd => {
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.end_thinking_message();
            }
        }
        GatewayEvent::Chunk { delta } => {
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.append_to_current_message(&delta);
            }
        }
        GatewayEvent::ResponseDone {
            thread_id: announced,
        } => {
            let mut s = state.write();
            // Only the turn being tracked can end it. A close-out naming a
            // different thread — a refused message, a turn started from
            // another client on the same agent — would otherwise retire the
            // live response, taking the Stop button and the working
            // indicator with it while the model is still going.
            if s.frame_is_for_current_turn(announced) {
                s.response_done(announced);
            }
            // The turn is over either way; credential requests and sign-in
            // flows it was waiting on can no longer be answered.
            s.retire_credentials_for_thread(announced);
            s.retire_device_flows_for_thread(announced);
            // Nor can approvals and questions it never resolved — a turn
            // displaced by a newer message dies mid-wait, and this
            // close-out is the only retirement its requests will get.
            s.retire_requests_for_thread(announced);
        }
        GatewayEvent::ToolCall {
            id,
            name,
            arguments,
        } => {
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.add_tool_call(id, name, arguments);
                // A tool call marks the end of this round's text stream; the
                // gateway is now executing the tool. Switch the indicator from
                // "Streaming…" (which would sit frozen) to the processing bar
                // while the tool panel shows the running call. `is_processing`
                // stays set until ResponseDone.
                s.is_streaming = false;
            }
        }
        GatewayEvent::ToolOutput { id, chunk, .. } => {
            // Live output from a running tool: update its panel in place.
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.append_tool_output(&id, &chunk);
            }
        }
        GatewayEvent::ToolResult {
            id,
            name: _,
            result,
            is_error,
        } => {
            let mut s = state.write();
            // The `ask_user` tool's result means the gateway has stopped
            // waiting — answered, cancelled, or timed out. Retire the card
            // even if the user never touched it, and regardless of which
            // thread is on screen. Scoped to the result's own thread: the
            // id is a colliding call id, and after the user answered this
            // entry an id-only match would discard another turn's
            // still-unanswered request instead.
            s.clear_user_prompt_if(thread_id, &id);
            // Same contract for approvals: this result arrives whether the
            // user answered or the gateway gave up, and an abandoned entry
            // at the head of the queue would hide every later request.
            s.retire_tool_approval(thread_id, &id);
            // A failed tool call already surfaces inline: the tool panel
            // shows Failed status with the full error result. No banner.
            if s.frame_targets_view(thread_id) {
                s.set_tool_result(&id, result, is_error);
            }
        }
        GatewayEvent::ToolStatus {
            id,
            name: _,
            elapsed_ms,
            pid,
            cpu_percent,
            memory_bytes,
            state: proc_state,
            message,
        } => {
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.set_tool_live_status(
                    &id,
                    rustyclaw_core::ui::ToolLiveStatus {
                        elapsed_ms,
                        pid,
                        cpu_percent,
                        memory_bytes,
                        state: proc_state,
                        message,
                    },
                );
            }
        }
        GatewayEvent::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => {
            // Queued, not overwritten: a second turn's request while one is on
            // screen waits its turn. Overwriting meant the first was never
            // shown again, and its two-minute timeout read as a denial of a
            // tool the user never saw.
            state
                .write()
                .pending_tool_approvals
                .push_back((thread_id, id, name, arguments));
        }
        GatewayEvent::ThreadsUpdate {
            threads,
            foreground_id,
        } => {
            tracing::debug!(
                count = threads.len(),
                foreground_id = ?foreground_id,
                "ThreadsUpdate received"
            );
            let mapped: Vec<ThreadInfo> = threads
                .into_iter()
                .map(|t| ThreadInfo {
                    id: t.id,
                    project_id: t.project_id,
                    label: t.label,
                    description: t.description,
                    status: t.status,
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                    working_dir: t.working_dir,
                    pinned: t.pinned,
                })
                .collect();
            let mut s = state.write();
            // Reconciled before the foreground derives the indicators, so
            // Stop and the composer gate come up right. Both directions:
            // see `apply_thread_statuses` for why "Ready" has to clear the
            // entry and not only "Streaming" set it.
            //
            // The race the old add-only comment named — a stale list
            // clearing a turn this client has just sent — is real but
            // bounded, and the gateway closes it: the Chat arm calls
            // `begin_turn` and sends a fresh thread list in the same breath,
            // so an authoritative "Streaming" lands within a round trip of a
            // submit. A stale "Ready" still in flight at submit time can
            // un-gate the composer until it does. That window is one round
            // trip and self-correcting, which a permanent lockout is not.
            s.apply_thread_statuses(&mapped);
            s.threads = mapped;
            s.set_foreground_thread(foreground_id);
        }
        GatewayEvent::DownloadsUpdate { downloads } => {
            // The whole list, every time. The panel is a view of the
            // gateway's registry rather than something this client
            // accumulates, so a missed update costs nothing beyond one stale
            // frame — and no merge can go wrong.
            state.write().downloads = rustyclaw_view::DownloadsData {
                downloads: downloads.into_iter().map(Into::into).collect(),
            };
        }
        GatewayEvent::PluginsUpdate { plugins } => {
            tracing::info!(count = plugins.len(), "PluginsUpdate received");
            let snapshots: Vec<crate::components::PluginSnapshot> = plugins
                .into_iter()
                .map(|p| {
                    // A plugin whose state does not parse still belongs in the
                    // list — it renders with empty state rather than vanishing
                    // from the panel with no explanation.
                    let state = serde_json::from_str(&p.state_json).unwrap_or_else(|e| {
                        tracing::warn!(
                            plugin = %p.name,
                            error = %e,
                            "Plugin state did not parse; rendering it empty"
                        );
                        serde_json::Value::Object(Default::default())
                    });
                    crate::components::PluginSnapshot {
                        name: p.name,
                        description: p.description,
                        emoji: p.emoji,
                        version: p.version,
                        enabled: p.enabled,
                        state,
                        actions: p
                            .actions
                            .into_iter()
                            .map(|a| crate::components::PluginActionInfo {
                                name: a.name,
                                description: a.description,
                            })
                            .collect(),
                        html_template: p.html_template,
                    }
                })
                .collect();
            let mut s = state.write();
            // The dock is hidden until a plugin is actually called for. A
            // plugin update is the agent calling for its plugins, so the dock
            // shows itself when there is something to render — and folds away
            // when the last plugin disappears instead of leaving an empty
            // column.
            if snapshots.is_empty() {
                s.plugin_dock_visible = false;
                s.active_plugin = None;
            } else {
                s.plugin_dock_visible = true;
                // Selecting the first plugin when nothing is selected means the
                // panel shows content immediately instead of a bare tab strip.
                if s.active_plugin.is_none() {
                    s.active_plugin = snapshots.first().map(|p| p.name.clone());
                }
            }
            s.plugins = snapshots;
        }
        GatewayEvent::ProjectsUpdate {
            projects,
            active_id,
        } => {
            state.write().projects = projects
                .into_iter()
                .map(|p| rustyclaw_core::ui::ProjectInfo {
                    id: p.id,
                    name: p.name,
                    path: p.path,
                    pinned: p.pinned,
                })
                .collect();
            state.write().active_project_id = active_id;
        }
        GatewayEvent::ThreadHistory {
            thread_id,
            ok,
            messages,
            error,
        } => {
            if !ok {
                // A failed history load used to be a `warn!` and nothing else,
                // so the user just got a blank transcript with no indication
                // anything had gone wrong — indistinguishable from an empty
                // thread. Say so on screen.
                let err = error.unwrap_or_else(|| "unknown error".to_string());
                tracing::warn!(thread_id, error = %err, "ThreadHistory request failed");
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Could not load history for thread {thread_id}: {err}"),
                );
            } else {
                // INFO, not DEBUG: the matching "requesting thread history"
                // line is INFO, and a request with no visible reply is
                // precisely the symptom worth diagnosing from a normal log.
                tracing::info!(
                    thread_id,
                    incoming_messages = messages.len(),
                    "Desktop received thread history reply"
                );
                let converted = crate::state::ui_history_from_gateway(messages);
                let shown = converted.len();
                state.write().apply_thread_history(thread_id, converted);
                // An empty transcript for a thread the sidebar says has
                // messages is a bug, not a normal state — make it visible
                // rather than rendering a blank pane.
                if shown == 0 {
                    tracing::warn!(
                        thread_id,
                        "thread history reply contained no displayable messages"
                    );
                }
            }
        }
        GatewayEvent::ThreadMessages {
            thread_id,
            messages,
        } => {
            state.write().hydrate_thread_messages(thread_id, messages);
        }
        GatewayEvent::ThreadExportResult {
            thread_id,
            ok,
            filename,
            content,
            error,
        } => {
            if ok {
                // The user picked where the copy goes; write it there from
                // the client. The dialog that asked stays open until they
                // save or cancel, and the notice confirms the result.
                spawn(async move {
                    let Some(handle) = rfd::AsyncFileDialog::new()
                        .set_title("Save thread copy")
                        .set_file_name(&filename)
                        .save_file()
                        .await
                    else {
                        // User cancelled the save dialog — nothing to report.
                        return;
                    };
                    match handle.write(content.as_bytes()).await {
                        Ok(_) => {
                            state.write().push_notice(
                                MessageRole::Success,
                                format!(
                                    "Saved thread copy ({} bytes) to {}",
                                    content.len(),
                                    handle.path().display()
                                ),
                            );
                        }
                        Err(e) => {
                            state.write().push_notice(
                                MessageRole::Error,
                                format!(
                                    "Could not save thread copy to {}: {}",
                                    handle.path().display(),
                                    e
                                ),
                            );
                        }
                    }
                });
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    error.unwrap_or_else(|| format!("Failed to export thread {thread_id}")),
                );
            }
        }
        GatewayEvent::UserPromptRequest { id: _, prompt } => {
            // Tagged with the turn that asked it, so a question from a
            // conversation the user is not looking at waits there rather
            // than appearing in the one they are.
            state.write().set_user_prompt(prompt, thread_id);
        }
        GatewayEvent::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => {
            // Tagged with the asking turn's thread: a credential wait
            // ending is what ends its turn, so the turn's close-out is the
            // signal that this request can no longer be answered.
            state.write().pending_credential_requests.push_back((
                thread_id,
                id,
                provider,
                secret_name,
                message,
            ));
        }
        GatewayEvent::DeviceFlowStart { url, code, message } => {
            // Tagged with the turn that started the flow, and queued: two
            // conversations can both need to sign in, and dismissing this
            // dialog aims a Cancel at the flow's own turn — not at whatever
            // happens to be on screen.
            state
                .write()
                .pending_device_flows
                .push_back((thread_id, url, code, message));
        }
        GatewayEvent::DeviceFlowComplete => {
            state.write().retire_completed_device_flow(thread_id);
        }
        GatewayEvent::SecretsListResult { ok, entries } => {
            if ok {
                // Keep the last-known TOTP state; it arrives separately
                // via SecretsHasTotpResult.
                let (agent_access, has_totp) = {
                    let s = state.read();
                    (s.agent_access, s.secrets_data.has_totp)
                };
                let data = SecretsDialogData::from_vault(
                    entries.iter().map(Into::into).collect(),
                    agent_access,
                    has_totp,
                );
                state.write().secrets_data = data;
            } else {
                state
                    .write()
                    .push_notice(MessageRole::Error, "Failed to list secrets.");
            }
        }
        GatewayEvent::MessengerConfigResult {
            accounts,
            routes,
            threads,
            available_kinds,
            vault_locked,
        } => {
            // `apply` keeps the current selection in range rather than
            // resetting it; refreshes land after every mutation.
            state.write().messengers_data.apply(
                accounts.iter().map(Into::into).collect(),
                routes.iter().map(Into::into).collect(),
                threads.iter().map(Into::into).collect(),
                available_kinds,
                vault_locked,
            );
        }
        GatewayEvent::MessengerAccountResult {
            ok,
            errors,
            message,
            ..
        } => {
            // Several validation problems arrive together; all of them belong
            // in the notice, not just the first.
            let text = match (errors.is_empty(), message) {
                (false, _) => errors.join("; "),
                (true, Some(message)) => message,
                (true, None) => match ok {
                    true => "Saved".to_string(),
                    false => "Failed".to_string(),
                },
            };
            if ok {
                // The desktop editor stays mounted until a save is confirmed;
                // this is the signal that it can be discarded.
                state.write().messengers_data.commits += 1;
            }
            state.write().messengers_data.set_status(text.clone(), !ok);
            state.write().push_notice(
                match ok {
                    true => MessageRole::Success,
                    false => MessageRole::Error,
                },
                text,
            );
        }
        GatewayEvent::MessengerRouteResult { ok, message } => {
            let text = message.unwrap_or_else(|| match ok {
                true => "Route saved".to_string(),
                false => "Route change failed".to_string(),
            });
            state.write().messengers_data.set_status(text, !ok);
        }
        GatewayEvent::SecretsStoreResult { ok, message } => {
            if ok {
                state
                    .write()
                    .push_notice(MessageRole::Success, "Secret stored successfully.");
                // Trigger refresh to show new secret
                // (parent doesn't have gateway handle here, so we just update status)
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Failed to store secret: {}", message),
                );
            }
        }
        GatewayEvent::SecretsDeleteResult { ok, message } => {
            if ok {
                state
                    .write()
                    .push_notice(MessageRole::Success, "Secret deleted.");
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Failed to delete secret: {}", message.unwrap_or_default()),
                );
            }
        }
        GatewayEvent::SecretsSetPolicyResult { ok, message } => {
            if ok {
                state
                    .write()
                    .push_notice(MessageRole::Success, "Policy updated.");
            } else {
                state.write().push_notice(
                    MessageRole::Error,
                    format!("Failed to set policy: {}", message.unwrap_or_default()),
                );
            }
        }
        GatewayEvent::ModelReloaded { provider, model } => {
            state.write().push_notice(
                MessageRole::Success,
                format!("Model reloaded: {provider}/{model}"),
            );
        }
        GatewayEvent::ThinkingDelta { delta } => {
            // Accumulate the reasoning text into the open thinking block so
            // the transcript can show *why* the agent did what it did.
            let mut s = state.write();
            if s.frame_targets_view(thread_id) {
                s.append_thinking(&delta);
            }
        }
        GatewayEvent::ThreadSwitched { .. } => {
            // Thread state syncs via ThreadsUpdate/ThreadHistory.
        }
        GatewayEvent::Warning { message } => {
            state.write().push_notice(MessageRole::Warning, message);
        }
        // Secrets-management results without a desktop UI surface. These are
        // driven from the TUI's secrets manager; the desktop ignores them.
        GatewayEvent::SecretsHasTotpResult { has_totp } => {
            state.write().secrets_data.has_totp = has_totp;
        }
        GatewayEvent::SecretsPeekResult {
            ok,
            fields,
            message,
            totp_required,
        } => {
            let mut s = state.write();
            if ok {
                s.secrets_data.finish_reveal(fields);
            } else if totp_required {
                // Covers both the first attempt (sent without a code) and a
                // rejected one, so the prompt stays up carrying the reason.
                s.secrets_data.require_reveal_code(message);
            } else {
                s.secrets_data.clear_reveal();
                s.push_notice(
                    MessageRole::Error,
                    message.unwrap_or_else(|| "Failed to reveal credential".to_string()),
                );
            }
        }
        GatewayEvent::SecretsGetResult { .. }
        | GatewayEvent::SecretsSetDisabledResult { .. }
        | GatewayEvent::SecretsDeleteCredentialResult { .. }
        | GatewayEvent::SecretsSetupTotpResult { .. }
        | GatewayEvent::SecretsVerifyTotpResult { .. } => {}
        // Tool groups have no desktop surface yet — the plugin manager UI
        // (plan §10) will render them; until then the reply frame is
        // ignored, explicitly, like the TUI does with PluginsUpdate.
        GatewayEvent::ToolGroupsUpdate { .. } => {}
        GatewayEvent::SecretsRemoveTotpResult { .. } => {}
        GatewayEvent::Error { message } => {
            let mut s = state.write();
            s.push_notice(MessageRole::Error, message);
            // Fallback for gateways that never name their turns: with no
            // tracked turns there is no close-out coming, and this is the
            // only thing standing between an error and a stuck composer.
            // When turns are tracked, retirement belongs to the error's own
            // `ResponseDone` — clearing here would take the working state
            // off a conversation that is still answering whenever some
            // *other* turn errors.
            if s.in_flight.is_empty() {
                s.is_processing = false;
            }
        }
        GatewayEvent::Info { message } => {
            state.write().push_notice(MessageRole::Info, message);
        }
        GatewayEvent::DomQuery { .. } => {
            // Handled directly in the UI updater task via handle_dom_query.
        }
        GatewayEvent::HostInfo {
            hostname,
            os,
            arch,
            cpu_brand,
            cpu_cores_physical,
            cpu_cores_logical,
            cpu_frequency_mhz,
            total_memory_bytes,
            total_swap_bytes,
            disk_total_bytes,
            disk_available_bytes,
            gpus,
            summary,
        } => {
            let gib = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
            state.write().host_info = Some(rustyclaw_view::HostInfoData {
                hostname,
                os,
                arch,
                cpu_brand,
                cpu_cores_physical,
                cpu_cores_logical,
                cpu_frequency_mhz,
                total_memory_gib: gib(total_memory_bytes),
                total_swap_gib: gib(total_swap_bytes),
                disk_total_gib: gib(disk_total_bytes),
                disk_available_gib: gib(disk_available_bytes),
                gpus: gpus
                    .into_iter()
                    .map(|g| rustyclaw_view::GpuDisplayInfo {
                        name: g.name,
                        vendor: g.vendor,
                        vram_gib: gib(g.vram_bytes),
                    })
                    .collect(),
                summary,
            });
        }
        GatewayEvent::LoadStatus {
            load_score,
            avg_load_score,
            cpu_percent,
            memory_percent,
            summary,
        } => {
            state.write().load_status = Some(rustyclaw_view::LoadStatusData {
                load_score,
                avg_load_score,
                cpu_percent,
                memory_percent,
                summary,
            });
        }
        GatewayEvent::ServiceList { services } => {
            state.write().services_data = Some(rustyclaw_view::ServiceListData {
                services: services.into_iter().map(Into::into).collect(),
            });
        }
        GatewayEvent::ServiceActionResult { service, .. } => {
            if let Some(svc) = service {
                let info = rustyclaw_view::ServiceInfoData::from(svc);
                let mut st = state.write();
                if let Some(ref mut data) = st.services_data {
                    if let Some(existing) = data.services.iter_mut().find(|s| s.name == info.name) {
                        *existing = info;
                    } else {
                        data.services.push(info);
                    }
                }
            }
        }
        GatewayEvent::ServiceLogs { .. } => {
            // Logs are displayed in a separate dialog; no state update needed.
        }
        // ── Engines ──────────────────────────────────────────────────────
        GatewayEvent::EngineListResult { engines } => {
            let mut s = state.write();
            let (host_ram, host_vram, host_gpu) = host_resources(&s);
            let panel = s
                .engines_data
                .get_or_insert_with(rustyclaw_view::EnginesPanelData::default);
            panel.engines = engines.into_iter().map(dto_to_engine_data).collect();
            panel.host_ram_bytes = host_ram;
            panel.host_vram_bytes = host_vram;
            panel.host_gpu_name = host_gpu;
            // Default the active tab to the first engine so the dialog opens
            // on a populated tab instead of a blank one.
            if panel.selected_engine.is_none() {
                panel.selected_engine = panel.engines.first().map(|e| e.id.clone());
            }
        }
        GatewayEvent::EngineModelListResult { engine, models } => {
            let mut s = state.write();
            let panel = s
                .engines_data
                .get_or_insert_with(rustyclaw_view::EnginesPanelData::default);
            panel.models = models
                .into_iter()
                .map(|m| dto_to_model_data(&engine, m))
                .collect();
            panel.selected_engine = Some(engine);
        }
        GatewayEvent::ProviderModelListResult {
            provider,
            models,
            error,
            ..
        } => {
            if let Some(err) = error {
                // Keep the static fallback in the picker.  The provider
                // deliberately stays in `provider_models_requested`:
                // removing it here would re-trigger the request effect
                // (which observes this same state signal) and spin an
                // unthrottled retry loop.  The guard is instead cleared
                // when the user explicitly switches to this provider
                // (`on_model_change` in app/mod.rs), so retries are
                // user-driven and bounded.
                tracing::warn!(provider = %provider, error = %err, "Live provider model fetch failed");
            } else {
                state.write().provider_models.insert(provider, models);
            }
        }
        GatewayEvent::EnginePullProgress {
            engine,
            model,
            percent,
            downloaded_bytes,
            total_bytes,
            status,
        } => {
            let mut s = state.write();
            let panel = s
                .engines_data
                .get_or_insert_with(rustyclaw_view::EnginesPanelData::default);
            panel.pull_progress = Some(rustyclaw_view::PullProgressData {
                engine,
                model,
                percent,
                downloaded_bytes,
                total_bytes,
                status,
            });
        }
        GatewayEvent::EngineActionProgress {
            engine,
            line,
            percent: _,
        } => {
            // Fold the install line into the engine's tab so it renders live.
            let mut s = state.write();
            let panel = s
                .engines_data
                .get_or_insert_with(rustyclaw_view::EnginesPanelData::default);
            panel.push_install_line(&engine, line);
        }
        GatewayEvent::EngineActionResult {
            engine,
            model,
            ok,
            message,
        } => {
            let mut s = state.write();
            if let Some(ref mut panel) = s.engines_data {
                // A pull just finished (successfully or not) — clear the bar.
                if model.is_some() {
                    panel.pull_progress = None;
                }
                // Record the terminal outcome on the engine's install panel,
                // but only while an install is actually in progress —
                // EngineActionResult also fires for start/stop, which must
                // not overwrite a completed install's status.
                if panel.install_output.get(&engine).is_some_and(|o| !o.done) {
                    panel.finish_install(&engine, ok, message.clone());
                }
            }
            // Refresh engine/model lists so the dialog reflects the change.
            if ok {
                s.engines_stale = true;
                s.push_notice(MessageRole::Success, format!("Engine: {}", message));
            } else {
                s.push_notice(MessageRole::Error, format!("Engine error: {}", message));
            }
        }
        // ── Panels (cron / memory / MCP / channels / tool config) ─────────
        GatewayEvent::CronListResult { jobs } => {
            let mut s = state.write();
            let panel = s
                .cron_data
                .get_or_insert_with(rustyclaw_view::CronPanelData::default);
            panel.jobs = jobs.iter().map(Into::into).collect();
            panel.status = None;
        }
        GatewayEvent::CronUpsertResult { ok, message, .. }
        | GatewayEvent::CronActionResult { ok, message } => {
            let mut s = state.write();
            s.cron_stale = true;
            if !ok {
                let msg = message.unwrap_or_else(|| "cron operation failed".into());
                s.push_notice(MessageRole::Error, format!("Cron: {}", msg));
            }
        }
        GatewayEvent::MemoryListResult { entries } => {
            let mut s = state.write();
            let panel = s
                .memory_data
                .get_or_insert_with(rustyclaw_view::MemoryPanelData::default);
            panel.entries = entries.iter().map(Into::into).collect();
            panel.status = None;
        }
        GatewayEvent::MemoryUpsertResult { ok, message, .. }
        | GatewayEvent::MemoryDeleteResult { ok, message } => {
            let mut s = state.write();
            s.memory_stale = true;
            if !ok {
                let msg = message.unwrap_or_else(|| "memory operation failed".into());
                s.push_notice(MessageRole::Error, format!("Memory: {}", msg));
            }
        }
        GatewayEvent::HistorySearchResult { entries } => {
            let mut s = state.write();
            let panel = s
                .memory_data
                .get_or_insert_with(rustyclaw_view::MemoryPanelData::default);
            panel.history = entries.iter().map(Into::into).collect();
        }
        GatewayEvent::McpListResult { servers } => {
            let mut s = state.write();
            let panel = s
                .mcp_data
                .get_or_insert_with(rustyclaw_view::McpPanelData::default);
            panel.servers = servers.iter().map(Into::into).collect();
            panel.status = None;
        }
        GatewayEvent::McpConnectResult { ok, message, .. }
        | GatewayEvent::McpDisconnectResult { ok, message } => {
            let mut s = state.write();
            s.mcp_stale = true;
            if !ok {
                let msg = message.unwrap_or_else(|| "MCP operation failed".into());
                s.push_notice(MessageRole::Error, format!("MCP: {}", msg));
            }
        }
        GatewayEvent::ChannelStatusResult { channels } => {
            let mut s = state.write();
            let panel = s
                .channels_data
                .get_or_insert_with(rustyclaw_view::ChannelsPanelData::default);
            panel.channels = channels.iter().map(Into::into).collect();
            panel.status = None;
        }
        GatewayEvent::ChannelPairResult { ok, message, .. } => {
            let mut s = state.write();
            s.channels_stale = true;
            if !ok {
                let msg = message.unwrap_or_else(|| "channel operation failed".into());
                s.push_notice(MessageRole::Error, format!("Channels: {}", msg));
            }
        }
        GatewayEvent::ToolConfigResult { tools } => {
            let mut s = state.write();
            let panel = s
                .tools_data
                .get_or_insert_with(rustyclaw_view::ToolConfigPanelData::default);
            panel.tools = tools.iter().map(Into::into).collect();
        }
        GatewayEvent::ToolToggleResult { ok, message } => {
            let mut s = state.write();
            s.tools_stale = true;
            if !ok {
                let msg = message.unwrap_or_else(|| "tool toggle failed".into());
                s.push_notice(MessageRole::Error, format!("Tools: {}", msg));
            }
        }
        GatewayEvent::UsageStatsResult {
            totals,
            per_model,
            per_session,
        } => {
            let mut s = state.write();
            let panel = s
                .analytics_data
                .get_or_insert_with(rustyclaw_view::AnalyticsPanelData::default);
            panel.period = totals.period.clone();
            panel.totals = (&totals).into();
            panel.per_model = per_model.iter().map(Into::into).collect();
            panel.per_session = per_session.iter().map(Into::into).collect();
            panel.status = None;
        }
        GatewayEvent::LogsResult {
            ok,
            source,
            lines,
            message,
        } => {
            let mut s = state.write();
            let panel = s
                .logs_data
                .get_or_insert_with(rustyclaw_view::LogsPanelData::default);
            panel.source = rustyclaw_view::LogSource::from_wire(&source);
            panel.lines = lines;
            panel.status = match (ok, message) {
                (false, Some(msg)) => Some(msg),
                _ => None,
            };
        }
        GatewayEvent::AgentsUpdate { .. } => {
            // The desktop has no agent-switcher surface yet; the agent list
            // is currently only consumed by the TUI selector dialog.
        }
        GatewayEvent::AgentSwitched { agent_id, name } => {
            state.write().push_notice(
                MessageRole::Success,
                format!("Switched to agent '{}' ({})", name, agent_id),
            );
        }
    }
}

/// Extract host RAM/VRAM/GPU-name for the engines panel header from the
/// gateway host info (if it has been fetched).
fn host_resources(s: &AppState) -> (u64, u64, Option<String>) {
    let Some(ref host) = s.host_info else {
        return (0, 0, None);
    };
    let gib = 1024.0 * 1024.0 * 1024.0;
    let ram = (host.total_memory_gib * gib) as u64;
    let vram = (host.gpus.iter().map(|g| g.vram_gib).sum::<f64>() * gib) as u64;
    let gpu = host.gpus.first().map(|g| g.name.clone());
    (ram, vram, gpu)
}

fn dto_to_engine_data(
    dto: rustyclaw_core::gateway::protocol::frames::EngineInfoDto,
) -> rustyclaw_view::LocalEngineData {
    rustyclaw_view::LocalEngineData {
        id: dto.id,
        display_name: dto.display_name,
        installed: dto.installed,
        running: dto.running,
        version: dto.version,
        endpoint: dto.endpoint,
        available_models: dto.available_models,
        loaded_models: dto.loaded_models,
        caps: rustyclaw_view::EngineCapsData {
            can_install: dto.capabilities.can_install,
            can_start: dto.capabilities.can_start,
            can_stop: dto.capabilities.can_stop,
            can_pull: dto.capabilities.can_pull,
            can_remove: dto.capabilities.can_remove,
            can_load: dto.capabilities.can_load,
            can_unload: dto.capabilities.can_unload,
        },
    }
}

fn dto_to_model_data(
    engine: &str,
    dto: rustyclaw_core::gateway::protocol::frames::EngineModelDto,
) -> rustyclaw_view::LocalModelData {
    rustyclaw_view::LocalModelData {
        engine: engine.to_string(),
        name: dto.name,
        size_bytes: dto.size_bytes,
        quantization: dto.quantization,
        context_length: dto.context_length,
        loaded: dto.loaded,
        vram_bytes: dto.vram_bytes,
        family: dto.family,
        format: dto.format,
        fits_host: dto.fits_host,
        fit_warning_msg: dto.fit_warning,
    }
}

pub(crate) fn normalize_provider_id(id: &str) -> &str {
    match id {
        "copilot" | "github_copilot" | "githubcopilot" => "github-copilot",
        other => other,
    }
}

// ── DOM query handler ───────────────────────────────────────────────────────

/// Execute a JavaScript expression in the webview and send the result
/// back to the gateway as a `DomQueryResponse`.
pub(crate) async fn handle_dom_query(
    client: &Arc<GatewayClient>,
    id: String,
    js: String,
) -> anyhow::Result<()> {
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
                break (
                    format!("eval error after {} attempts: {}", attempts, e),
                    true,
                );
            }
        }
    };

    client
        .send(GatewayCommand::DomQueryResponse {
            id,
            result,
            is_error,
        })
        .await
        .context("sending DomQueryResponse")?;
    Ok(())
}

pub(crate) fn display_path(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME")
        && path.starts_with(&home)
    {
        return path.replacen(&home, "~", 1);
    }
    path.to_string()
}

pub(crate) fn build_directory_options(base_path: &str) -> Vec<rustyclaw_view::DirectoryOption> {
    use std::path::Path;

    let mut options: Vec<rustyclaw_view::DirectoryOption> = Vec::new();
    let base = Path::new(base_path);

    options.push(rustyclaw_view::DirectoryOption {
        path: base_path.to_string(),
        display_name: display_path(base_path),
        is_selected: true,
    });

    if let Some(parent) = base.parent() {
        let parent_str = parent.display().to_string();
        options.push(rustyclaw_view::DirectoryOption {
            path: parent_str.clone(),
            display_name: format!("../ ({})", display_path(&parent_str)),
            is_selected: false,
        });
    }

    if let Ok(home) = std::env::var("HOME")
        && home != base_path
    {
        options.push(rustyclaw_view::DirectoryOption {
            path: home.clone(),
            display_name: "Home (~)".to_string(),
            is_selected: false,
        });
    }

    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.filter_map(Result::ok).take(24) {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let p = entry.path().display().to_string();
            if p == base_path {
                continue;
            }
            let label = entry.file_name().to_string_lossy().to_string();
            options.push(rustyclaw_view::DirectoryOption {
                path: p,
                display_name: label,
                is_selected: false,
            });
        }
    }

    options
}

// ── Swarm helpers ───────────────────────────────────────────────────────────

/// The swarm store and agent registry rooted at this installation's
/// settings dir. Swarms are persistent groups of registered agents, so the
/// desktop shares state with the gateway, CLI, and model tools.
fn swarm_context() -> anyhow::Result<(
    rustyclaw_core::swarm::SwarmStore,
    rustyclaw_core::agents::AgentRegistry,
)> {
    let config = crate::resolved_config();
    Ok((
        rustyclaw_core::swarm::SwarmStore::new(&config.settings_dir),
        rustyclaw_core::agents::AgentRegistry::new(&config.settings_dir, &config.agent_name),
    ))
}

/// Build the current list of swarm infos from the on-disk store.
pub(crate) fn get_swarm_infos() -> Vec<SwarmData> {
    use rustyclaw_core::swarm::member_statuses;

    let Ok((store, registry)) = swarm_context() else {
        return Vec::new();
    };

    store
        .list()
        .into_iter()
        .map(|record| {
            let statuses = member_statuses(&record, &registry);
            let registered = statuses.iter().filter(|s| s.registered).count();
            SwarmData {
                name: record.config.name.clone(),
                status: if registered == statuses.len() {
                    "ready".to_string()
                } else {
                    "degraded".to_string()
                },
                description: record.config.description.clone(),
                active_sessions: statuses.iter().filter(|s| s.session_active).count() as u64,
                age_secs: record.age_secs(),
                agents: statuses
                    .iter()
                    .map(|s| SwarmAgentData {
                        id: s.agent_id.clone(),
                        name: s.name.clone(),
                        role: s.role.to_string(),
                        description: s.description.clone(),
                        has_session: s.session_active,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Create a swarm from a built-in template, materializing its members as
/// registered agents.
pub(crate) fn create_swarm_from_template(template: &str) -> anyhow::Result<()> {
    use rustyclaw_core::swarm::{builtin_templates, create_swarm};

    let cfg = builtin_templates()
        .into_iter()
        .find(|t| t.name == template)
        .ok_or_else(|| anyhow::anyhow!("Unknown template: {}", template))?;

    let (store, registry) = swarm_context()?;
    create_swarm(&store, &registry, cfg)?;
    Ok(())
}

/// Complete a swarm's active sessions (the swarm and its agents remain).
pub(crate) fn stop_swarm(name: &str) -> anyhow::Result<()> {
    use rustyclaw_core::swarm::stop_swarm_sessions;

    let (store, _registry) = swarm_context()?;
    stop_swarm_sessions(&store, name)?;
    Ok(())
}

/// Delete a swarm and the registered agents it created.
pub(crate) fn delete_swarm(name: &str) -> anyhow::Result<()> {
    let (store, registry) = swarm_context()?;
    rustyclaw_core::swarm::delete_swarm(&store, &registry, name, false)?;
    Ok(())
}

/// Load the local skills list for the skills manager dialog.
///
/// Skills live on the local filesystem, so this mirrors the TUI's local
/// `SkillManager` rather than going through the gateway.
pub(crate) fn load_skills_list() -> Vec<rustyclaw_view::SkillInfoData> {
    // The resolved config, not a fresh `Config::load(None)`: `skills_dirs()`
    // derives from `settings_dir`, which is exactly what `--profile` and
    // `--settings-dir` rewrite. Reading the default location listed — and
    // toggling wrote into — another profile's skills.
    let config = crate::resolved_config();
    let mut mgr = rustyclaw_core::skills::SkillManager::with_dirs(config.skills_dirs());
    // An unreadable skills dir would otherwise render as an empty dialog that
    // looks like "you have no skills" rather than "we could not read them".
    if let Err(e) = mgr.load_skills() {
        tracing::warn!("skills dialog listing nothing — load failed: {}", e);
    }
    mgr.get_skills()
        .iter()
        .map(|s| rustyclaw_view::SkillInfoData {
            name: s.name.clone(),
            description: s.description.clone().unwrap_or_default(),
            enabled: s.enabled,
        })
        .collect()
}

/// Toggle a skill's enabled state and return the refreshed list.
pub(crate) fn toggle_skill(name: &str) -> Vec<rustyclaw_view::SkillInfoData> {
    {
        // Same directory the listing above reads; see the note there.
        let config = crate::resolved_config();
        let mut mgr = rustyclaw_core::skills::SkillManager::with_dirs(config.skills_dirs());
        if let Err(e) = mgr.load_skills() {
            tracing::warn!("cannot toggle '{}' — loading skills failed: {}", name, e);
        }
        let enabled = mgr
            .get_skills()
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.enabled);
        if let Some(enabled) = enabled {
            // The dialog re-reads from disk on the way out, so a dropped error
            // here shows the switch snapping back with no explanation.
            if let Err(e) = mgr.set_skill_enabled(name, !enabled) {
                tracing::warn!("toggling skill '{}' failed: {}", name, e);
            }
        }
    }
    load_skills_list()
}

// ── Local gateway daemon ────────────────────────────────────────────────────

/// The local-daemon half of the gateway panel, read from disk and the process
/// table.
///
/// Every field here comes from somewhere that blocks — `daemon::status` walks
/// the process table via `sysinfo`, `Config::load` reads a file — so this is
/// built on a blocking thread and handed to the UI as a value. The connection
/// half of [`rustyclaw_view::GatewayControlData`] is filled in by the caller
/// from live signals; nothing about it needs a disk read.
pub struct LocalGatewaySnapshot {
    /// Whether the daemon named by the PID file is running, stale or absent.
    pub local: rustyclaw_view::LocalDaemonState,
    /// Address the daemon would be started on, from the resolved config.
    pub ssh_listen: String,
    /// Where the daemon's output goes — the place a failed start explains
    /// itself, since the panel only gets the spawn error.
    pub log_path: String,
    /// Whether the local secrets vault needs a password, and so whether a
    /// daemon started from the panel comes up with the vault locked.
    pub vault_password_protected: bool,
}

impl LocalGatewaySnapshot {
    /// Copy the snapshot over the daemon half of the panel's data, leaving
    /// `url`, `connected`, `pending` and `last_action` alone — those belong to
    /// the caller, not to the disk.
    ///
    /// One method rather than two open-coded blocks: the panel is refreshed
    /// from two places, and a field added to the snapshot but assigned in only
    /// one of them shows the right value on open and a stale one after an
    /// action, which is the harder of the two to notice.
    fn apply(self, data: &mut rustyclaw_view::GatewayControlData) {
        data.local = self.local;
        data.ssh_listen = self.ssh_listen;
        data.log_path = self.log_path;
        data.vault_password_protected = self.vault_password_protected;
    }
}

/// The address the local daemon listens on, matching what `gateway start`
/// would use.
///
/// Duplicated from the CLI rather than shared because the CLI's copy is
/// inline at two call sites in `main.rs`; if it moves somewhere callable this
/// should call it instead.
fn ssh_listen_addr(config: &rustyclaw_core::config::Config) -> String {
    config
        .ssh
        .as_ref()
        .map(|s| s.bind.clone())
        .unwrap_or_else(|| "0.0.0.0:2222".to_string())
}

/// Read the current state of the gateway daemon on this machine. Blocking.
pub fn probe_local_gateway() -> LocalGatewaySnapshot {
    let config = crate::resolved_config();
    LocalGatewaySnapshot {
        local: rustyclaw_view::LocalDaemonState::probe(&config.settings_dir),
        ssh_listen: ssh_listen_addr(&config),
        log_path: rustyclaw_core::daemon::log_path(&config.settings_dir)
            .display()
            .to_string(),
        vault_password_protected: config.secrets_password_protected,
    }
}

/// Which lifecycle action to run against the local daemon.
///
/// Alias of the view crate's [`rustyclaw_view::PendingAction`] rather than a
/// parallel enum, so "the action running" and "the action the panel is
/// spinning a button for" cannot drift apart.
pub type LocalGatewayAction = rustyclaw_view::PendingAction;

/// Run a lifecycle action against the gateway daemon on this machine.
/// Blocking — `daemon::stop` waits up to two seconds for the process to go.
///
/// The vault password is deliberately never passed. The CLI prompts for it on
/// a terminal; the desktop has no equivalent that does not mean building a
/// password prompt whose only job is to hand a secret to a subprocess. A
/// gateway started from here therefore comes up with the vault locked, which
/// the panel says out loud, and the existing unlock dialog handles it over the
/// session once connected.
pub fn run_local_gateway_action(action: LocalGatewayAction) -> rustyclaw_view::ActionOutcome {
    use rustyclaw_core::daemon;
    use rustyclaw_view::ActionOutcome;

    let config = crate::resolved_config();
    let dir = config.settings_dir.clone();
    let listen = ssh_listen_addr(&config);

    let start = |dir: &std::path::Path| {
        daemon::start(
            dir,
            &listen,
            &[],
            None,
            // See the doc comment: no password, vault comes up locked.
            None,
            config.tls_cert.as_deref(),
            config.tls_key.as_deref(),
            None,
        )
    };

    match action {
        LocalGatewayAction::Start => match start(&dir) {
            Ok(pid) => ActionOutcome::ok(format!("Gateway started (PID {pid}, SSH {listen}).")),
            Err(e) => ActionOutcome::failed(format!("Could not start the gateway: {e}")),
        },
        LocalGatewayAction::Stop => match daemon::stop(&dir) {
            Ok(daemon::StopResult::Stopped { pid }) => {
                ActionOutcome::ok(format!("Gateway stopped (was PID {pid})."))
            }
            Ok(daemon::StopResult::WasStale { pid }) => ActionOutcome::ok(format!(
                "Cleaned up a stale PID file — PID {pid} was already gone."
            )),
            Ok(daemon::StopResult::WasNotRunning) => {
                ActionOutcome::ok("Gateway was not running.".to_string())
            }
            Err(e) => ActionOutcome::failed(format!("Could not stop the gateway: {e}")),
        },
        LocalGatewayAction::Restart => {
            // A failed stop is fatal to the restart: starting on top of a
            // gateway that is still holding the listen port produces a second
            // process that dies on bind, overwrites the PID file on its way
            // in, and leaves the panel pointing at a corpse.
            let stopped = match daemon::stop(&dir) {
                Ok(result) => result,
                Err(e) => {
                    return ActionOutcome::failed(format!(
                        "Could not stop the gateway, so it was not restarted: {e}"
                    ));
                }
            };
            // Same brief pause the CLI takes, for the same reason: the port
            // is not free the instant the process is gone.
            if matches!(stopped, daemon::StopResult::Stopped { .. }) {
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            match start(&dir) {
                Ok(pid) => {
                    ActionOutcome::ok(format!("Gateway restarted (PID {pid}, SSH {listen})."))
                }
                Err(e) => ActionOutcome::failed(format!(
                    "Stopped the gateway but could not start it again: {e}"
                )),
            }
        }
    }
}

/// Re-probe the local daemon and write the result into the panel's state.
///
/// The probe runs on a blocking thread: `sysinfo` walking the process table
/// on the UI task is a visible stutter on a busy machine, and the same code
/// path is reached right after `daemon::stop`, which has just spent up to two
/// seconds waiting.
pub fn refresh_gateway_control(mut state: Signal<AppState>) {
    spawn(async move {
        match rustyclaw_view::tokio::task::spawn_blocking(probe_local_gateway).await {
            Ok(snapshot) => snapshot.apply(&mut state.write().gateway_control),
            Err(e) => {
                // A panic in the probe would otherwise leave the panel showing
                // whatever it last read, with no hint that it is stale.
                tracing::error!(error = ?e, "probing the local gateway failed");
                state.write().gateway_control.last_action =
                    Some(rustyclaw_view::ActionOutcome::failed(
                        "Could not read the local gateway's status.",
                    ));
            }
        }
    });
}

/// Run a local daemon action, then re-probe so the panel shows the result.
///
/// `pending` is set before the blocking call and cleared after the re-probe,
/// so the buttons stay disabled across the whole sequence rather than coming
/// back to life in the window between the action landing and the status
/// catching up — which is exactly the window in which the panel is wrong.
pub fn run_gateway_action(mut state: Signal<AppState>, action: LocalGatewayAction) {
    // The buttons are disabled while an action is pending, but a second click
    // can land before the re-render that disables them. Two overlapping
    // restarts race on one PID file: the second `stop` reads the PID the first
    // `start` has not written yet, and the panel ends up naming a process that
    // no longer exists.
    if state.read().gateway_control.pending.is_some() {
        return;
    }
    state.write().gateway_control.pending = Some(action);
    spawn(async move {
        let outcome = match rustyclaw_view::tokio::task::spawn_blocking(move || {
            run_local_gateway_action(action)
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::error!(error = ?e, ?action, "gateway action panicked");
                rustyclaw_view::ActionOutcome::failed(
                    "The gateway action failed unexpectedly; see the desktop log.",
                )
            }
        };
        // Re-probe on the same task rather than delegating to
        // `refresh_gateway_control`: that spawns, so `pending` would clear
        // before the fresh status arrived.
        let snapshot = rustyclaw_view::tokio::task::spawn_blocking(probe_local_gateway).await;
        let mut s = state.write();
        s.gateway_control.last_action = Some(outcome);
        match snapshot {
            Ok(snapshot) => snapshot.apply(&mut s.gateway_control),
            Err(e) => tracing::error!(error = ?e, "re-probing the local gateway failed"),
        }
        s.gateway_control.pending = None;
    });
}

// ── Task boundary ───────────────────────────────────────────────────────────

/// Spawn a task whose failure is reported where the whole path is visible.
///
/// These tasks are detached: `spawn` hands back nothing to await, so there is
/// no caller for `?` to reach and every one of them used to end in
/// `let _ = client.send(..)`. Logging at the send site is not much better —
/// "command was not sent" names the leaf and not the thing the user did, and
/// the leaf is the least interesting frame in the stack.
///
/// This gives the task the boundary it was missing. The body is ordinary
/// fallible Rust — `?` after each step, `.context(..)` naming what that step
/// was for — and the error arrives here carrying the chain that produced it,
/// which is reported in one place. `{:?}` on an `anyhow::Error` prints the
/// whole chain, so the log line reads as the path rather than the symptom.
///
/// `what` names the user action, since it is the outermost frame and the one
/// the chain hangs from.
pub fn spawn_reporting<F>(what: &'static str, fut: F)
where
    F: std::future::Future<Output = anyhow::Result<()>> + 'static,
{
    spawn(async move {
        if let Err(e) = fut.await {
            tracing::error!(action = what, error = ?e, "Action failed");
        }
    });
}

#[cfg(test)]
mod event_buffer_tests {
    use super::*;

    fn chunk(thread_id: Option<u64>, delta: &str) -> ThreadEvent {
        ThreadEvent {
            thread_id,
            event: GatewayEvent::Chunk {
                delta: delta.to_string(),
            },
        }
    }

    fn disconnected() -> ThreadEvent {
        ThreadEvent {
            thread_id: None,
            event: GatewayEvent::Disconnected {
                reason: Some("write failed".to_string()),
            },
        }
    }

    #[test]
    fn chunks_of_one_thread_coalesce() {
        let mut buf = EventBuffer::default();
        buf.push_events([chunk(Some(1), "he"), chunk(Some(1), "llo")]);

        assert_eq!(buf.entries.len(), 1, "one coalesced entry");
        match &buf.entries[0] {
            BufferEntry::Chunks {
                thread_id,
                text,
                count,
                bytes,
            } => {
                assert_eq!(*thread_id, Some(1));
                assert_eq!(text, "hello");
                assert_eq!(*count, 2);
                assert_eq!(*bytes, 5);
            }
            _ => panic!("expected a Chunks entry"),
        }
    }

    /// Two turns stream interleaved; merging by adjacency alone would
    /// splice two different answers into one string.
    #[test]
    fn chunks_of_different_threads_stay_apart() {
        let mut buf = EventBuffer::default();
        buf.push_events([
            chunk(Some(1), "a"),
            chunk(Some(2), "b"),
            chunk(Some(1), "c"),
        ]);

        assert_eq!(buf.entries.len(), 3, "no cross-thread coalescing");
    }

    /// Coalescing must not reorder anything around a non-chunk event —
    /// StreamStart → Chunks → ResponseDone sequencing is load-bearing.
    #[test]
    fn a_non_chunk_event_breaks_the_run_and_keeps_order() {
        let mut buf = EventBuffer::default();
        buf.push_events([chunk(Some(1), "a"), disconnected(), chunk(Some(1), "b")]);

        assert_eq!(buf.entries.len(), 3);
        assert!(matches!(buf.entries[0], BufferEntry::Chunks { .. }));
        assert!(matches!(
            buf.entries[1],
            BufferEntry::Event {
                event: GatewayEvent::Disconnected { .. },
                ..
            }
        ));
        assert!(matches!(buf.entries[2], BufferEntry::Chunks { .. }));
    }

    /// The event the UI most needs off a dying connection: it is what clears
    /// the spinner and marks the connection dropped. The worker drains it
    /// into this buffer on its way out, so it must survive intact.
    #[test]
    fn a_terminal_disconnect_survives_buffering() {
        let mut buf = EventBuffer::default();
        buf.push_events([chunk(Some(1), "partial"), disconnected()]);

        assert!(
            buf.entries.iter().any(|e| matches!(
                e,
                BufferEntry::Event {
                    event: GatewayEvent::Disconnected { .. },
                    ..
                }
            )),
            "the disconnect notice must reach the UI"
        );
    }
}
