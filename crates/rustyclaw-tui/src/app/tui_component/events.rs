//! Gateway-event handling for the TUI root: applies each `GwEvent` to UI state.

use std::collections::HashMap;
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use iocraft::prelude::State;
use rustyclaw_view::tracing;

use super::display_message_from_gateway;
use super::state;
use crate::app::{DeviceFlowOwner, GwEvent, PanelKind, UserInput};
use crate::types::DisplayMessage;

type UserTx = Arc<StdMutex<Option<sync_mpsc::Sender<UserInput>>>>;

/// Close the newest open thinking block, if any: stamp its duration and
/// fold it to its one-line gist, or drop it when no reasoning text ever
/// arrived. "Open" means not yet closed out — closing always collapses
/// the block and records the duration when known. Returns whether a
/// block was closed.
fn close_open_thinking(m: &mut Vec<DisplayMessage>, duration_ms: Option<u64>) -> bool {
    let Some(idx) = m.iter().rposition(|x| {
        x.role == rustyclaw_core::types::MessageRole::Thinking
            && x.duration_ms.is_none()
            && !x.collapsed
    }) else {
        return false;
    };
    if m[idx].content.trim().is_empty() {
        m.remove(idx);
    } else {
        m[idx].duration_ms = duration_ms;
        m[idx].collapsed = true;
    }
    true
}

/// Point the view's streaming indicators at `incoming`.
///
/// `streaming`, the elapsed timer and `streaming_buf` are one set shared by
/// the whole view, so they only ever describe the thread being shown. When
/// the view moves they are stale — nothing else clears them, since a
/// close-out for the thread that left is ignored precisely because it is no
/// longer on screen, and the spinner would run forever.
///
/// Clearing alone is not enough either: `streaming` is what Esc is gated on,
/// so a conversation still answering when the user returns to it would show
/// no progress and could not be stopped at all. The flag is set from what is
/// actually running.
///
/// The partial text is dropped in both directions — the answer so far lives
/// in the transcript, and the gateway's history snapshot is what makes a
/// returned-to conversation whole.
fn rebase_view_streaming(
    incoming: Option<u64>,
    in_flight: &std::collections::HashSet<u64>,
    streaming: &mut State<bool>,
    stream_start: &mut State<Option<Instant>>,
    elapsed: &mut State<String>,
    streaming_buf: &mut State<String>,
) {
    let still_answering = incoming.is_some_and(|thread| in_flight.contains(&thread));
    streaming.set(still_answering);
    stream_start.set(still_answering.then(Instant::now));
    elapsed.set(String::new());
    streaming_buf.set(String::new());
}

/// Whether a turn-scoped frame from `thread_id` belongs to the thread on
/// screen.
///
/// The TUI shows one thread at a time and keeps a single streaming buffer, so
/// "which turn should render" has exactly one right answer: the one running
/// in the foreground thread. Tracking it in a separate slot could only ever
/// disagree — the slot followed whichever turn started last, so opening a
/// second turn elsewhere silently stole the screen from the first.
///
/// `None` is a gateway too old to attribute its frames, and is trusted.
fn renders_here(thread_id: Option<u64>, foreground: Option<u64>) -> bool {
    match (thread_id, foreground) {
        (Some(announced), Some(on_screen)) => announced == on_screen,
        _ => true,
    }
}

/// Surface the next queued request whenever its dialog is free.
///
/// Requests only ever enqueue; this is the one place a dialog is populated,
/// so a second request arriving while one is on screen waits instead of
/// overwriting it. Called before every event — including ones whose arms
/// return early — and from the poll loop's timer tick, because the trigger
/// that frees a dialog is a keypress, and the connection can stay quiet for
/// as long as a model call takes: waiting for inbound traffic could sit a
/// blocked request past its own deadline while the user stares at an idle
/// screen.
pub(super) fn drain_queued_dialogs(ui: &state::Ui) {
    let mut show_tool_approval = ui.show_tool_approval;
    let mut queued_tool_approvals = ui.queued_tool_approvals;
    let mut tool_approval_thread = ui.tool_approval_thread;
    let mut tool_approval_id = ui.tool_approval_id;
    let mut tool_approval_name = ui.tool_approval_name;
    let mut tool_approval_args = ui.tool_approval_args;
    let mut tool_approval_selected = ui.tool_approval_selected;
    let mut show_user_prompt = ui.show_user_prompt;
    let mut queued_user_prompts = ui.queued_user_prompts;
    let mut user_prompt_thread = ui.user_prompt_thread;
    let mut user_prompt_id = ui.user_prompt_id;
    let mut user_prompt_title = ui.user_prompt_title;
    let mut user_prompt_desc = ui.user_prompt_desc;
    let mut user_prompt_input = ui.user_prompt_input;
    let mut user_prompt_type = ui.user_prompt_type;
    let mut user_prompt_selected = ui.user_prompt_selected;
    let mut user_prompt_checked = ui.user_prompt_checked;
    let mut show_credential_request = ui.show_credential_request;
    let mut queued_credentials = ui.queued_credentials;
    let mut credential_request_id = ui.credential_request_id;
    let mut credential_request_provider = ui.credential_request_provider;
    let mut credential_request_secret_name = ui.credential_request_secret_name;
    let mut credential_request_message = ui.credential_request_message;
    let mut credential_request_input = ui.credential_request_input;
    let mut credential_request_thread = ui.credential_request_thread;
    let mut show_device_flow = ui.show_device_flow;
    let mut queued_device_flows = ui.queued_device_flows;
    let mut device_flow_owner = ui.device_flow_owner;
    let mut device_flow_provider = ui.device_flow_provider;
    let mut device_flow_url = ui.device_flow_url;
    let mut device_flow_code = ui.device_flow_code;
    let mut device_flow_tick = ui.device_flow_tick;
    let mut device_flow_browser_opened = ui.device_flow_browser_opened;

    if !show_tool_approval.get() {
        let mut queue = queued_tool_approvals.read().clone();
        if !queue.is_empty() {
            let (owner, id, name, arguments) = queue.remove(0);
            queued_tool_approvals.set(queue);
            tool_approval_thread.set(owner);
            tool_approval_id.set(id);
            tool_approval_name.set(name);
            tool_approval_args.set(arguments);
            tool_approval_selected.set(true);
            show_tool_approval.set(true);
        }
    }
    if !show_user_prompt.get() {
        let mut queue = queued_user_prompts.read().clone();
        if !queue.is_empty() {
            let (owner, prompt) = queue.remove(0);
            queued_user_prompts.set(queue);
            user_prompt_thread.set(owner);
            user_prompt_id.set(prompt.id.clone());
            user_prompt_title.set(prompt.title.clone());
            user_prompt_desc.set(prompt.description.clone().unwrap_or_default());
            user_prompt_input.set(String::new());
            user_prompt_type.set(Some(prompt.prompt_type.clone()));
            let default_sel = match &prompt.prompt_type {
                rustyclaw_core::user_prompt_types::PromptType::Select { default, .. } => {
                    default.unwrap_or(0)
                }
                rustyclaw_core::user_prompt_types::PromptType::Confirm { default } => {
                    if *default {
                        0
                    } else {
                        1
                    }
                }
                _ => 0,
            };
            user_prompt_selected.set(default_sel);
            let checked = match &prompt.prompt_type {
                rustyclaw_core::user_prompt_types::PromptType::MultiSelect {
                    options,
                    defaults,
                } => {
                    let mut checked = vec![false; options.len()];
                    for &i in defaults {
                        if let Some(slot) = checked.get_mut(i) {
                            *slot = true;
                        }
                    }
                    checked
                }
                _ => Vec::new(),
            };
            user_prompt_checked.set(checked);
            show_user_prompt.set(true);
        }
    }
    if !show_credential_request.get() {
        let mut queue = queued_credentials.read().clone();
        if !queue.is_empty() {
            let (thread, id, provider, secret_name, message) = queue.remove(0);
            queued_credentials.set(queue);
            credential_request_thread.set(thread);
            credential_request_id.set(id);
            credential_request_provider.set(provider);
            credential_request_secret_name.set(secret_name);
            credential_request_message.set(message);
            credential_request_input.set(String::new());
            show_credential_request.set(true);
        }
    }
    if !show_device_flow.get() {
        let mut queue = queued_device_flows.read().clone();
        if !queue.is_empty() {
            let (owner, provider, url, code) = queue.remove(0);
            queued_device_flows.set(queue);
            device_flow_owner.set(Some(owner));
            device_flow_provider.set(provider);
            device_flow_url.set(url.clone());
            device_flow_code.set(code);
            device_flow_tick.set(0);
            // Opened when the dialog surfaces, not when the request was
            // queued — the user should meet the browser tab and the code
            // together.
            crate::components::device_flow_dialog::open_url_in_browser(&url);
            device_flow_browser_opened.set(true);
            show_device_flow.set(true);
        }
    }
}

/// Apply a single gateway event to the UI state bundle.
pub(super) fn apply_gw_event(
    ev: GwEvent,
    ui: state::Ui,
    needs_hatching: bool,
    tx_for_history: &UserTx,
) {
    let ui_for_drain = ui;
    #[allow(unused_variables, unused_mut)]
    let state::Ui {
        mut messages,
        mut input_value,
        mut input_cursor_offset,
        mut gw_status,
        mut streaming,
        mut stream_start,
        mut thinking_start,
        mut tool_started,
        mut active_process,
        mut elapsed,
        mut scroll_offset,
        mut spinner_tick,
        mut should_quit,
        mut streaming_buf,
        mut dynamic_model_label,
        mut dynamic_provider_id,
        mut selected_message_idx,
        mut show_auth_dialog,
        mut auth_code,
        mut auth_error,
        mut show_tool_approval,
        mut tool_approval_thread,
        mut tool_approval_id,
        mut tool_approval_name,
        mut tool_approval_args,
        mut tool_approval_selected,
        mut show_vault_unlock,
        mut vault_password,
        mut vault_error,
        mut hatching_dialog,
        mut show_pairing,
        mut pairing_step,
        mut pairing_field,
        mut pairing_public_key,
        mut pairing_fingerprint,
        mut pairing_fingerprint_art,
        mut pairing_qr_ascii,
        mut pairing_host,
        mut pairing_port,
        mut pairing_error,
        mut show_user_prompt,
        mut user_prompt_thread,
        mut user_prompt_id,
        mut user_prompt_title,
        mut user_prompt_desc,
        mut user_prompt_input,
        mut user_prompt_type,
        mut user_prompt_selected,
        mut user_prompt_checked,
        mut show_credential_request,
        mut credential_request_id,
        mut credential_request_provider,
        mut credential_request_secret_name,
        mut credential_request_message,
        mut credential_request_input,
        mut credential_request_thread,
        mut show_provider_selector,
        mut provider_selector_items,
        mut provider_selector_ids,
        mut provider_selector_hints,
        mut provider_selector_cursor,
        mut show_api_key_dialog,
        mut api_key_provider,
        mut api_key_provider_display,
        mut api_key_input,
        mut api_key_help_url,
        mut api_key_help_text,
        mut show_device_flow,
        mut device_flow_provider,
        mut device_flow_url,
        mut device_flow_code,
        mut device_flow_tick,
        mut device_flow_browser_opened,
        mut show_model_selector,
        mut model_selector_provider,
        mut model_selector_provider_display,
        mut model_selector_models,
        mut model_selector_cursor,
        mut model_selector_loading,
        mut show_agent_selector,
        mut agent_selector_agents,
        mut agent_selector_active_id,
        mut agent_selector_cursor,
        mut dynamic_agent_name,
        mut threads,
        mut projects,
        mut active_project_id,
        mut tab_focused,
        mut tab_selected,
        mut thread_messages_cache,
        mut foreground_thread_id,
        mut in_flight,
        mut queued_tool_approvals,
        mut queued_user_prompts,
        mut queued_credentials,
        mut queued_device_flows,
        mut device_flow_owner,
        mut command_completions,
        mut command_selected,
        mut model_completion_provider,
        mut model_completion_models,
        mut model_completion_loading,
        mut hub_completion_query,
        mut hub_completion_models,
        mut hub_completion_loading,
        mut prompt_attachments,
        mut show_secrets_dialog,
        mut secrets_dialog_data,
        mut secrets_agent_access,
        mut secrets_has_totp,
        mut secrets_selected,
        mut secrets_scroll_offset,
        mut secrets_add_step,
        mut secrets_add_name,
        mut secrets_add_value,
        mut show_skills_dialog,
        mut skills_dialog_data,
        mut skills_selected,
        mut show_details_dialog,
        mut details_dialog_text,
        mut details_dialog_is_error,
        mut details_dialog_scroll,
        mut show_tool_perms_dialog,
        mut tool_perms_dialog_data,
        mut tool_perms_selected,
        mut skills_scroll_offset,
        mut tool_perms_scroll_offset,
        mut host_info,
        mut load_status,
        mut show_system_info,
        show_services_dialog: _,
        mut show_downloads_dialog,
        mut downloads_data,
        mut downloads_cursor,
        mut services_data,
        mut show_engines_dialog,
        mut engines_data,
        mut engines_cursor,
        mut engines_params_edit,
        mut engines_params_cursor,
        mut engines_params_drafts,
        mut engines_action_result,
        mut show_cron_dialog,
        mut cron_data,
        mut show_memory_dialog,
        mut memory_data,
        mut show_mcp_dialog,
        mut mcp_data,
        mut show_channels_dialog,
        mut channels_data,
        mut show_messengers_dialog,
        mut messengers_data,
        mut show_analytics_dialog,
        mut analytics_data,
        mut show_logs_dialog,
        mut logs_data,
        mut secrets_revealed,
        secrets_reveal_pending,
        mut secrets_reveal_code,
        mut secrets_reveal_totp_prompt,
        mut secrets_reveal_error,
        ..
    } = ui;
    drain_queued_dialogs(&ui_for_drain);
    match ev {
        GwEvent::AuthChallenge => {
            // Gateway wants TOTP — show the dialog
            gw_status.set(rustyclaw_core::types::GatewayStatus::AuthRequired);
            let mut hatching = hatching_dialog.read().clone();
            hatching.hide_temporarily();
            hatching_dialog.set(hatching);
            show_auth_dialog.set(true);
            auth_code.set(String::new());
            auth_error.set(String::new());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::info(
                "Authentication required — enter TOTP code",
            ));
            messages.set(m);
        }
        GwEvent::Disconnected(reason) => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::Disconnected);
            show_auth_dialog.set(false);
            active_process.set(None);
            // Every turn died with the connection. Leaving them recorded
            // re-arms the spinner — and the Esc gate — for replies that can
            // never arrive, on every later visit to those threads.
            in_flight.set(std::collections::HashSet::new());
            // The queued requests died with their turns: nothing will ever
            // retire them, and an answer would go into a closed connection.
            // Left in place, the first event after reconnect would drain a
            // stale request into a dialog.
            queued_tool_approvals.set(Vec::new());
            queued_user_prompts.set(Vec::new());
            queued_credentials.set(Vec::new());
            queued_device_flows.set(Vec::new());
            device_flow_owner.set(None);
            // The requests already drained into dialogs died with their
            // turns too; a box left on screen would swallow keyboard input
            // for an answer that has nowhere to go.
            show_tool_approval.set(false);
            show_user_prompt.set(false);
            show_credential_request.set(false);
            credential_request_thread.set(None);
            show_device_flow.set(false);
            device_flow_browser_opened.set(false);
            streaming.set(false);
            stream_start.set(None);
            elapsed.set(String::new());
            streaming_buf.set(String::new());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::warning(format!("Disconnected: {}", reason)));
            messages.set(m);
        }
        GwEvent::Connected => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::Connected);
            // A fresh session has nothing in flight, whatever the previous
            // one left behind.
            in_flight.set(std::collections::HashSet::new());
            queued_tool_approvals.set(Vec::new());
            queued_user_prompts.set(Vec::new());
            queued_credentials.set(Vec::new());
            queued_device_flows.set(Vec::new());
            device_flow_owner.set(None);
            show_tool_approval.set(false);
            show_user_prompt.set(false);
            show_credential_request.set(false);
            credential_request_thread.set(None);
            show_device_flow.set(false);
            device_flow_browser_opened.set(false);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::info("Gateway connected."));
            messages.set(m);
            // Reset foreground tracking so the next ThreadsUpdate
            // always triggers a fresh history fetch, even when the
            // same thread stays foreground across a reconnect.
            foreground_thread_id.set(None);
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(tx, UserInput::RefreshThreads);
                }
            }
        }
        GwEvent::Authenticated => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::Connected);
            show_auth_dialog.set(false);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success("Authenticated"));
            messages.set(m);
            // Also reset on auth success (SSH key auth skips Connected).
            foreground_thread_id.set(None);
            // Request initial thread list
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(tx, UserInput::RefreshThreads);
                }
            }
            // Show hatching now that auth is complete.
            let mut hatching = hatching_dialog.read().clone();
            hatching.show_if_needed(needs_hatching);
            hatching_dialog.set(hatching);
        }
        GwEvent::Info(s) => {
            // Check for "Model ready" or similar to upgrade status
            let mut m = messages.read().clone();
            m.push(DisplayMessage::info(s));
            messages.set(m);
        }
        GwEvent::Success(s) => {
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success(s));
            messages.set(m);
        }
        GwEvent::Warning { summary, details } => {
            // If auth dialog is open, treat warnings as auth retries
            if show_auth_dialog.get() {
                auth_error.set(summary.clone());
                auth_code.set(String::new());
            }
            let mut m = messages.read().clone();
            let msg = match details {
                Some(d) => DisplayMessage::with_details(
                    rustyclaw_core::types::MessageRole::Warning,
                    summary,
                    d,
                ),
                None => DisplayMessage::warning(summary),
            };
            m.push(msg);
            messages.set(m);
        }
        GwEvent::Error { summary, details } => {
            // Auth errors close the dialog
            if show_auth_dialog.get() {
                show_auth_dialog.set(false);
                auth_code.set(String::new());
                auth_error.set(String::new());
            }
            // Fallback for gateways that never name their turns: with no
            // tracked turns there is no close-out coming, and this is what
            // keeps a provider error (e.g. 400 Bad Request) from leaving
            // the spinner stuck in "Thinking…". When turns are tracked,
            // retirement belongs to the error's own `ResponseDone` —
            // stopping here would blank the on-screen turn's progress
            // whenever some *other* turn errors.
            if in_flight.read().is_empty() {
                streaming.set(false);
                stream_start.set(None);
                elapsed.set(String::new());
            }
            streaming_buf.set(String::new());

            let mut m = messages.read().clone();
            let msg = match details {
                Some(d) => DisplayMessage::with_details(
                    rustyclaw_core::types::MessageRole::Error,
                    summary,
                    d,
                ),
                None => DisplayMessage::error(summary),
            };
            m.push(msg);
            messages.set(m);
        }
        GwEvent::StreamStart(thread_id) => {
            // Nothing focused means the gateway elected a thread for this
            // turn; follow it onto the screen. Otherwise the thread on
            // screen is the answer to "which turn should render", and a
            // turn opening anywhere else must not move it — that is what a
            // second slot got wrong: adopting every announcement in turn,
            // it ended up naming whichever turn started last rather than
            // the one the user is reading.
            if foreground_thread_id.get().is_none() && thread_id.is_some() {
                foreground_thread_id.set(thread_id);
            }
            // Recorded whether or not it is on screen: coming back to a
            // conversation that is still answering has to restore its
            // spinner, and Esc is gated on that same flag.
            if let Some(thread) = thread_id {
                let mut running = in_flight.read().clone();
                running.insert(thread);
                in_flight.set(running);
            }
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }
            streaming.set(true);
            // Keep the earlier start time if we already
            // began timing on user submit.
            if stream_start.get().is_none() {
                stream_start.set(Some(Instant::now()));
            }
            streaming_buf.set(String::new());
        }
        GwEvent::Chunk(thread_id, text) => {
            // A chunk from a turn running in a thread that is not on screen
            // must not join this answer: one `streaming_buf` is shared, so
            // appending it would splice two replies into one message. That
            // thread's transcript arrives whole when the user switches to
            // it.
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }
            let mut buf = streaming_buf.read().clone();
            buf.push_str(&text);
            streaming_buf.set(buf);

            let mut m = messages.read().clone();
            if let Some(last) = m.last_mut() {
                if last.role == rustyclaw_core::types::MessageRole::Assistant {
                    last.append(&text);
                } else {
                    m.push(DisplayMessage::assistant(&text));
                }
            } else {
                m.push(DisplayMessage::assistant(&text));
            }
            messages.set(m);
        }
        GwEvent::ResponseDone(thread_id) => {
            // Retired before the render gate: a turn that finished off-screen
            // is no longer running, and returning to it must not show a
            // spinner for an answer that is already complete.
            if let Some(thread) = thread_id {
                let mut running = in_flight.read().clone();
                running.remove(&thread);
                in_flight.set(running);
                // Credential requests and sign-in flows the ended turn was
                // waiting on can no longer be answered — their waits ending
                // is what ends the turn, so this close-out retires them.
                let mut queue = queued_credentials.read().clone();
                let before = queue.len();
                queue.retain(|(owner, ..)| *owner != Some(thread));
                if queue.len() != before {
                    queued_credentials.set(queue);
                }
                let mut flows = queued_device_flows.read().clone();
                let before = flows.len();
                flows.retain(|(owner, ..)| *owner != DeviceFlowOwner::Turn(thread));
                if flows.len() != before {
                    queued_device_flows.set(flows);
                }
                if show_device_flow.get()
                    && device_flow_owner.get() == Some(DeviceFlowOwner::Turn(thread))
                {
                    show_device_flow.set(false);
                    device_flow_browser_opened.set(false);
                    device_flow_owner.set(None);
                }
                // The displayed credential request left the queue when the
                // dialog drained it, so the retain above cannot reach it —
                // without this the password box outlives the request and
                // swallows an answer nobody is waiting for.
                if show_credential_request.get() && credential_request_thread.get() == Some(thread)
                {
                    show_credential_request.set(false);
                    credential_request_thread.set(None);
                }
                // Approvals and questions the ended turn never resolved die
                // with it too. Normally their own ToolResult retires them; a
                // turn displaced by a newer message in its thread is aborted
                // mid-wait and never sends one, so the close-out the gateway
                // emits on its behalf is the only retirement they will get —
                // queued or already on screen.
                let mut approvals = queued_tool_approvals.read().clone();
                let before = approvals.len();
                approvals.retain(|(owner, ..)| *owner != Some(thread));
                if approvals.len() != before {
                    queued_tool_approvals.set(approvals);
                }
                if show_tool_approval.get() && tool_approval_thread.get() == Some(thread) {
                    show_tool_approval.set(false);
                }
                let mut prompts = queued_user_prompts.read().clone();
                let before = prompts.len();
                prompts.retain(|(owner, _)| *owner != Some(thread));
                if prompts.len() != before {
                    queued_user_prompts.set(prompts);
                }
                if show_user_prompt.get() && user_prompt_thread.get() == Some(thread) {
                    show_user_prompt.set(false);
                }
            }
            // Only the turn on screen can end what is on screen. A close-out
            // from a turn running elsewhere would stop the spinner and file
            // this half-streamed answer as finished while it is still being
            // written.
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }
            // Capture the accumulated assistant text and
            // send it back to the tokio loop so it gets
            // appended to the conversation history.
            let completed_text = streaming_buf.read().clone();

            if !completed_text.is_empty() {
                if let Ok(guard) = tx_for_history.lock() {
                    if let Some(ref tx) = *guard {
                        crate::app::events::submit(
                            tx,
                            UserInput::AssistantResponse(completed_text),
                        );
                    }
                }
            }
            streaming.set(false);
            stream_start.set(None);
            active_process.set(None);
            elapsed.set(String::new());
            streaming_buf.set(String::new());
            // Auto-collapse the just-completed assistant message
            // if it is long enough to warrant folding.
            let mut m = messages.read().clone();
            if let Some(last) = m.last_mut() {
                last.auto_collapse_if_needed();
            }
            messages.set(m);
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(tx, UserInput::RefreshTasks);
                }
            }
        }
        GwEvent::ThinkingStart(thread_id) => {
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }

            // Thinking is a form of streaming — show spinner
            streaming.set(true);
            if stream_start.get().is_none() {
                stream_start.set(Some(Instant::now()));
            }
            thinking_start.set(Some(Instant::now()));
            let mut m = messages.read().clone();
            // A dropped stream can leave a block open with no ThinkingEnd;
            // fold it (without a duration) so only one block is ever open.
            close_open_thinking(&mut m, None);
            m.push(DisplayMessage::thinking(""));
            messages.set(m);
        }
        GwEvent::ThinkingDelta(thread_id, delta) => {
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }

            // Accumulate the reasoning text into the open thinking block
            // so the user can expand it later and see *why* the agent did
            // what it did.
            let mut m = messages.read().clone();
            match m.last_mut() {
                Some(last) if last.role == rustyclaw_core::types::MessageRole::Thinking => {
                    last.append(&delta);
                }
                _ => {
                    let mut msg = DisplayMessage::thinking("");
                    msg.append(&delta);
                    m.push(msg);
                }
            }
            messages.set(m);
        }
        GwEvent::ThinkingEnd(thread_id) => {
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }

            // Thinking done, but streaming may continue with chunks.
            // Don't clear streaming here — just close out the thinking
            // block: stamp its duration and fold it to a one-line gist
            // (drop it entirely if the provider sent no reasoning text).
            let duration_ms = thinking_start.get().map(|t| t.elapsed().as_millis() as u64);
            thinking_start.set(None);
            let mut m = messages.read().clone();
            // The open block is usually last, but text chunks may already
            // have started a new assistant bubble after it — search from
            // the rear for the newest thinking block not yet closed out.
            if close_open_thinking(&mut m, duration_ms) {
                messages.set(m);
            }
        }
        GwEvent::ModelReady(detail) => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::ModelReady);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success(detail));
            messages.set(m);
        }
        GwEvent::ModelReloaded { provider, model } => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::ModelReady);
            let label = if provider.is_empty() {
                String::new()
            } else if model.is_empty() {
                provider.clone()
            } else {
                format!("{} / {}", provider, model)
            };
            let msg_text = if label.is_empty() {
                "Model switched to (none)".to_string()
            } else {
                format!("Model switched to {}", label)
            };
            dynamic_provider_id.set(Some(provider));
            dynamic_model_label.set(Some(label));
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success(msg_text));
            messages.set(m);
        }
        GwEvent::ToolCall {
            thread_id,
            id,
            name,
            arguments,
        } => {
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }
            let mut started = tool_started.read().clone();
            started.insert(id.clone(), Instant::now());
            tool_started.set(started);
            let mut m = messages.read().clone();
            if m.last()
                .map(|x| x.role == rustyclaw_core::types::MessageRole::Assistant)
                .unwrap_or(false)
            {
                if let Some(last) = m.last_mut() {
                    last.add_tool_call(id, name, arguments);
                }
            } else {
                let mut assistant = DisplayMessage::assistant("");
                assistant.add_tool_call(id, name, arguments);
                m.push(assistant);
            }
            messages.set(m);
        }
        GwEvent::ToolStatus {
            thread_id,
            id,
            status,
        } => {
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }

            // Track the controllable process (if any) behind the running
            // call so the inline pause/stop/kill keys know their target.
            active_process.set(status.pid.map(|pid| super::state::ActiveProcess {
                tool_id: id.clone(),
                pid,
                paused: status.is_paused(),
            }));
            let mut m = messages.read().clone();
            for msg in m.iter_mut().rev() {
                if msg.set_tool_live_status(&id, status.clone()) {
                    break;
                }
            }
            messages.set(m);
        }
        GwEvent::ToolOutput {
            thread_id,
            id,
            chunk,
        } => {
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }

            // Live output from a running tool: fold it into that tool's
            // panel so the row updates in place while the process runs.
            let mut m = messages.read().clone();
            for msg in m.iter_mut().rev() {
                if msg.append_tool_output(&id, &chunk) {
                    messages.set(m);
                    return;
                }
            }
        }
        GwEvent::ToolResult {
            thread_id,
            id,
            name,
            result,
            is_error,
        } => {
            // This result arrives whether the user answered or the gateway
            // gave up waiting; either way nobody is listening for an
            // approval of this call any more. An abandoned entry left at
            // the head of the queue would hide every later request, each of
            // which then times out unseen as a denial. Retired before the
            // render gate: a background turn's approvals die too.
            // Oldest holder of the id first, and only one: call ids like
            // `call_0` collide across concurrent turns, and the gateway
            // resolves its waiters oldest-first. The displayed request was
            // drained from the head of the queue, so it is older than any
            // queued entry sharing its id — a blanket removal would take a
            // second turn's request down with the one this result ends.
            // …and only within the result's own turn: after the user
            // answered this turn's request, an id-only match would reach
            // across and remove another turn's still-unanswered entry.
            // `None` is an old gateway, which runs one turn and can speak
            // for anything.
            let speaks_for = |owner: Option<u64>| thread_id.is_none() || owner == thread_id;
            if show_tool_approval.get()
                && *tool_approval_id.read() == id
                && speaks_for(tool_approval_thread.get())
            {
                show_tool_approval.set(false);
            } else {
                let mut queue = queued_tool_approvals.read().clone();
                if let Some(pos) = queue
                    .iter()
                    .position(|(owner, queued_id, ..)| queued_id == &id && speaks_for(*owner))
                {
                    queue.remove(pos);
                    queued_tool_approvals.set(queue);
                }
            }
            // Same contract for `ask_user`: the prompt id is its tool-call
            // id, and this result arrives whether the user answered, Stop
            // was pressed, or the five-minute wait expired. A dead card
            // left up would block every question queued behind it — and
            // swallow the keyboard, since normal input is suppressed while
            // a prompt is shown.
            // Dialog first, then oldest queued — same FIFO-per-id contract
            // as the approvals above.
            if show_user_prompt.get()
                && *user_prompt_id.read() == id
                && speaks_for(user_prompt_thread.get())
            {
                show_user_prompt.set(false);
            } else {
                let mut queue = queued_user_prompts.read().clone();
                if let Some(pos) = queue
                    .iter()
                    .position(|(owner, prompt)| prompt.id == id && speaks_for(*owner))
                {
                    queue.remove(pos);
                    queued_user_prompts.set(queue);
                }
            }
            if !renders_here(thread_id, foreground_thread_id.get()) {
                return;
            }
            let mut started = tool_started.read().clone();
            let duration_ms = started.remove(&id).map(|t| t.elapsed().as_millis() as u64);
            tool_started.set(started);
            // The call is finished — its process is no longer controllable.
            if active_process
                .read()
                .as_ref()
                .is_some_and(|ap| ap.tool_id == id)
            {
                active_process.set(None);
            }
            let mut m = messages.read().clone();
            let mut matched = false;
            for msg in m.iter_mut().rev() {
                let before = msg.tool_calls.len();
                msg.set_tool_result(&id, result.clone(), is_error, duration_ms);
                let after_match = msg
                    .tool_calls
                    .iter()
                    .any(|tc| tc.id == id && tc.result.is_some());
                if before > 0 && after_match {
                    matched = true;
                    break;
                }
            }
            if !matched {
                let mut fallback = DisplayMessage::assistant("");
                fallback.add_tool_call(id, name, "{}".to_string());
                fallback.set_tool_result(
                    &fallback.tool_calls[0].id.clone(),
                    result,
                    is_error,
                    duration_ms,
                );
                m.push(fallback);
            }
            messages.set(m);
        }
        GwEvent::ToolApprovalRequest {
            thread_id,
            id,
            name,
            arguments,
        } => {
            let mut queue = queued_tool_approvals.read().clone();
            queue.push((thread_id, id, name.clone(), arguments));
            queued_tool_approvals.set(queue);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::system(format!(
                "🔐 Tool approval required: {} — press Enter to allow, Esc to deny",
                name,
            )));
            messages.set(m);
            drain_queued_dialogs(&ui_for_drain);
        }
        GwEvent::UserPromptRequest { thread_id, prompt } => {
            let mut queue = queued_user_prompts.read().clone();
            queue.push((thread_id, prompt.clone()));
            queued_user_prompts.set(queue);

            // Build informative message based on prompt type
            let hint = match &prompt.prompt_type {
                rustyclaw_core::user_prompt_types::PromptType::Select { options, .. } => {
                    let opt_list: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
                    format!("Options: {}", opt_list.join(", "))
                }
                rustyclaw_core::user_prompt_types::PromptType::Confirm { .. } => {
                    "Yes/No".to_string()
                }
                rustyclaw_core::user_prompt_types::PromptType::MultiSelect { options, .. } => {
                    let opt_list: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
                    format!("Select any of: {} (Space toggles)", opt_list.join(", "))
                }
                _ => "Type your answer".to_string(),
            };
            let mut m = messages.read().clone();
            m.push(DisplayMessage::system(format!(
                "❓ Agent asks: {} — {}",
                prompt.title, hint,
            )));
            if let Some(desc) = &prompt.description {
                if !desc.is_empty() {
                    m.push(DisplayMessage::info(desc.clone()));
                }
            }
            messages.set(m);
            drain_queued_dialogs(&ui_for_drain);
        }
        GwEvent::CredentialRequest {
            thread_id,
            id,
            provider,
            secret_name,
            message,
        } => {
            let mut queue = queued_credentials.read().clone();
            queue.push((
                thread_id,
                id,
                provider.clone(),
                secret_name.clone(),
                message,
            ));
            queued_credentials.set(queue);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::warning(format!(
                "🔑 Credential required for {} ({}) — enter API key",
                provider, secret_name,
            )));
            messages.set(m);
            drain_queued_dialogs(&ui_for_drain);
        }
        GwEvent::VaultLocked => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::VaultLocked);
            show_vault_unlock.set(true);
            vault_password.set(String::new());
            vault_error.set(String::new());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::warning(
                "🔒 Vault is locked — enter password to unlock".to_string(),
            ));
            messages.set(m);
        }
        GwEvent::VaultUnlocked => {
            show_vault_unlock.set(false);
            vault_password.set(String::new());
            vault_error.set(String::new());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success("🔓 Vault unlocked".to_string()));
            messages.set(m);
        }
        GwEvent::ShowSecrets {
            secrets,
            agent_access,
            has_totp,
        } => {
            secrets_dialog_data.set(secrets);
            secrets_agent_access.set(agent_access);
            secrets_has_totp.set(has_totp);
            if !show_secrets_dialog.get() {
                // First open — reset selection and scroll
                secrets_selected.set(Some(0));
                secrets_scroll_offset.set(0);
                secrets_add_step.set(0);
            }
            show_secrets_dialog.set(true);
        }
        GwEvent::ShowSkills { skills } => {
            skills_dialog_data.set(skills);
            if !show_skills_dialog.get() {
                // First open — reset selection and scroll
                skills_selected.set(Some(0));
                skills_scroll_offset.set(0);
            }
            show_skills_dialog.set(true);
        }
        GwEvent::ShowToolPerms { tools } => {
            tool_perms_dialog_data.set(tools);
            if !show_tool_perms_dialog.get() {
                // First open — reset selection and scroll
                tool_perms_selected.set(Some(0));
                tool_perms_scroll_offset.set(0);
            }
            show_tool_perms_dialog.set(true);
        }
        GwEvent::RefreshSecrets => {
            // Gateway mutation succeeded — re-fetch list
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(tx, UserInput::RefreshSecrets);
                }
            }
        }
        GwEvent::SecretRevealed {
            ok,
            fields,
            message,
            totp_required,
        } => {
            if ok {
                let name = secrets_reveal_pending.read().clone().unwrap_or_default();
                secrets_revealed.set(Some((name, fields)));
                secrets_reveal_totp_prompt.set(false);
                secrets_reveal_code.set(String::new());
                secrets_reveal_error.set(String::new());
            } else if totp_required {
                // Either the first attempt (no code sent yet) or a rejected
                // one — both land here, so the prompt stays up and shows why.
                secrets_reveal_totp_prompt.set(true);
                secrets_reveal_code.set(String::new());
                secrets_reveal_error.set(message.unwrap_or_default());
            }
        }
        GwEvent::ThreadsUpdate {
            threads: mut thread_list,
            foreground_id,
        } => {
            let previous_foreground = foreground_thread_id.get();
            tracing::debug!(
                total_threads = thread_list.len(),
                foreground_id = ?foreground_id,
                captions = ?thread_list
                    .iter()
                    .map(|t| format!("{}:{}", t.id, t.label))
                    .collect::<Vec<_>>(),
                "TUI ThreadsUpdate received"
            );
            if let Some(active_id) = foreground_id {
                for thread in &mut thread_list {
                    thread.is_foreground = thread.id == active_id;
                }
            }
            // "Streaming" in the status column is derived from the
            // gateway's turn markers, so it is authoritative: a turn is
            // running in that thread whether or not this client saw it
            // start — a reconnect, another client, or a turn the gateway
            // resumed after a restart. Seed the in-flight set from it
            // (add-only; removal belongs to each turn's close-out).
            {
                let mut running = in_flight.read().clone();
                let before = running.len();
                for t in thread_list
                    .iter()
                    .filter(|t| t.status.as_deref() == Some("Streaming"))
                {
                    running.insert(t.id);
                }
                if running.len() != before {
                    in_flight.set(running);
                }
            }
            // Adapt transport threads to view items, group them through the
            // shared SidebarTree, then flatten back to a project-ordered list.
            // The flat order matches the rendered tree, so the keyboard's flat
            // selection index lines up with what the user sees. Grouping +
            // orphan placement live entirely in rustyclaw-view (one definition
            // shared with the desktop).
            let items: Vec<rustyclaw_view::SidebarItemData> =
                thread_list.iter().map(item_from_thread).collect();
            let tree = rustyclaw_view::SidebarTree::from_items(
                &projects.read(),
                items,
                active_project_id.get(),
            );
            threads.set(tree.into_flat_items());
            // Keep local foreground in sync and request
            // authoritative history when gateway picks
            // a new foreground (including initial load).
            if foreground_id != previous_foreground {
                foreground_thread_id.set(foreground_id);
                // The spinner, the timer and the streaming buffer describe
                // the view, not a turn — there is one set of them and one
                // thread on screen. When the view moves, they belong to a
                // conversation that is no longer being shown, and nothing
                // will ever clear them: a close-out for that thread is
                // correctly ignored now, so the spinner would run forever.
                rebase_view_streaming(
                    foreground_id,
                    &in_flight.read().clone(),
                    &mut streaming,
                    &mut stream_start,
                    &mut elapsed,
                    &mut streaming_buf,
                );
                if let Some(thread_id) = foreground_id {
                    tracing::debug!(
                        thread_id,
                        previous_foreground = ?previous_foreground,
                        "TUI requesting thread history after ThreadsUpdate"
                    );
                    if let Ok(guard) = tx_for_history.lock() {
                        if let Some(ref tx) = *guard {
                            crate::app::events::submit(
                                tx,
                                UserInput::RequestThreadHistory(thread_id),
                            );
                        }
                    }
                }
            }
            // Update tab_selected to stay in bounds
            let count = threads.read().len();
            if count > 0 && tab_selected.get() >= count {
                tab_selected.set(count - 1);
            }
            // Show first-run hatching only after the gateway
            // is usable enough to provide thread state. This
            // avoids racing with a later TOTP AuthChallenge.
            if needs_hatching && !show_auth_dialog.get() {
                let mut hatching = hatching_dialog.read().clone();
                hatching.show_if_needed(needs_hatching);
                hatching_dialog.set(hatching);
            }
        }
        GwEvent::ProjectsUpdate {
            projects: project_list,
            active_id,
        } => {
            projects.set(project_list);
            active_project_id.set(active_id);
            // Re-group existing items now that the project set/active changed.
            let items = threads.read().clone();
            let tree = rustyclaw_view::SidebarTree::from_items(&projects.read(), items, active_id);
            threads.set(tree.into_flat_items());
        }
        GwEvent::AgentsUpdate { agents, active_id } => {
            // Keep the cursor on the active agent when (re)opening the list.
            let active_idx = agents.iter().position(|a| a.id == active_id).unwrap_or(0);
            if agent_selector_cursor.get() >= agents.len() {
                agent_selector_cursor.set(active_idx);
            }
            agent_selector_agents.set(agents);
            agent_selector_active_id.set(active_id);
        }
        GwEvent::AgentSwitched { agent_id, name } => {
            agent_selector_active_id.set(agent_id.clone());
            dynamic_agent_name.set(Some(name.clone()));
            show_agent_selector.set(false);
            // Threads/projects belong to the new agent now — drop the old
            // agent's scrollback cache; fresh ThreadsUpdate/ProjectsUpdate
            // frames follow immediately.
            thread_messages_cache.set(HashMap::new());
            foreground_thread_id.set(None);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success(format!(
                "Switched to agent '{}' ({})",
                name, agent_id
            )));
            messages.set(m);
        }
        GwEvent::ShowAgentSelector => {
            let agents = agent_selector_agents.read().clone();
            let active_id = agent_selector_active_id.read().clone();
            let active_idx = agents.iter().position(|a| a.id == active_id).unwrap_or(0);
            agent_selector_cursor.set(active_idx);
            show_agent_selector.set(true);
        }
        GwEvent::ThreadMessages {
            thread_id,
            messages: thread_messages,
        } => {
            // `thread_id == 0` is the gateway's "nothing is focused"
            // sentinel: it carries an empty list to blank the view after
            // backgrounding, and no real thread ever has id 0. It cannot
            // be cached — the key names nothing — and it cannot be matched
            // against the foreground: the `ThreadsUpdate` preceding it
            // already set the foreground to `None`, so the equality below
            // would drop it and leave the stale transcript on screen. A
            // reply still streaming into the view (a turn sent before any
            // thread existed) keeps its words; the sentinel is not a
            // close-out.
            if thread_id == 0 {
                if !streaming.get() {
                    messages.set(Vec::new());
                    scroll_offset.set(0);
                }
                return;
            }
            let converted: Vec<DisplayMessage> = thread_messages
                .into_iter()
                .map(display_message_from_gateway)
                .collect();
            // The snapshot is authoritative for its own thread whichever
            // that is — it feeds the cache a later switch restores from.
            // This is how a background turn's transcript arrives whole.
            let mut cache = thread_messages_cache.read().clone();
            cache.insert(thread_id, converted.clone());
            thread_messages_cache.set(cache);
            // But only the foreground thread's snapshot may take the
            // screen. A turn completing in a background thread sends one
            // too, and applying it would swap the conversation being read
            // for another — mid-word, when an answer is still streaming
            // here.
            if foreground_thread_id.get() == Some(thread_id) {
                messages.set(converted);
                scroll_offset.set(0);
            }
        }
        GwEvent::ThreadSwitched {
            thread_id,
            context_summary,
        } => {
            // Save the outgoing thread's scrollback
            // before swapping so we can restore it on
            // a future switch back.
            let previous_id = foreground_thread_id.get();
            let current_messages = messages.read().clone();
            if let Some(prev) = previous_id {
                if prev != thread_id {
                    let mut cache = thread_messages_cache.read().clone();
                    if current_messages.is_empty() {
                        cache.remove(&prev);
                    } else {
                        cache.insert(prev, current_messages);
                    }
                    thread_messages_cache.set(cache);
                }
            }

            // Restore cached scrollback for the new
            // thread, or fall back to the gateway's
            // context summary if no cache exists.
            let cached = thread_messages_cache.read().get(&thread_id).cloned();
            let mut m = match cached {
                Some(prior) if !prior.is_empty() => prior,
                _ => {
                    let mut seed = Vec::new();
                    seed.push(DisplayMessage::info(format!(
                        "Switched to thread (id: {})",
                        thread_id
                    )));
                    if let Some(summary) = context_summary {
                        seed.push(DisplayMessage::assistant(format!(
                            "[Previous context]\n\n{}",
                            summary
                        )));
                    }
                    seed
                }
            };
            messages.set(std::mem::take(&mut m));
            foreground_thread_id.set(Some(thread_id));
            // See the `ThreadsUpdate` arm: the streaming indicators belong
            // to whatever is on screen, and the screen just changed.
            rebase_view_streaming(
                Some(thread_id),
                &in_flight.read().clone(),
                &mut streaming,
                &mut stream_start,
                &mut elapsed,
                &mut streaming_buf,
            );
            // Ask the gateway for the authoritative,
            // cross-session history for this thread so
            // the local cache stays consistent with
            // what the gateway has persisted.
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(tx, UserInput::RequestThreadHistory(thread_id));
                }
            }
            // Unfocus tab after switch
            tab_focused.set(false);
        }
        GwEvent::ThreadHistory {
            thread_id,
            ok,
            messages: history,
            error,
        } => {
            if !ok {
                if let Some(err) = error {
                    tracing::warn!(
                        thread_id,
                        error = %err,
                        "TUI thread history request failed"
                    );
                    let mut m = messages.read().clone();
                    m.push(DisplayMessage::warning(format!(
                        "Could not load history for thread {}: {}",
                        thread_id, err
                    )));
                    messages.set(m);
                }
            } else {
                tracing::debug!(
                    thread_id,
                    incoming_messages = history.len(),
                    foreground = ?foreground_thread_id.get(),
                    "TUI thread history reply received"
                );
                let converted: Vec<DisplayMessage> = rustyclaw_view::convert_history(&history);
                tracing::debug!(
                    thread_id,
                    converted_messages = converted.len(),
                    "TUI thread history converted"
                );
                // Update the cache so a future
                // switch-back is also authoritative.
                let mut cache = thread_messages_cache.read().clone();
                if converted.is_empty() {
                    cache.remove(&thread_id);
                } else {
                    cache.insert(thread_id, converted.clone());
                }
                thread_messages_cache.set(cache);
                // Only replace the live view if this
                // reply is for the thread the user is
                // currently looking at.
                if foreground_thread_id.get() == Some(thread_id) {
                    messages.set(converted);
                }
            }
        }
        GwEvent::ShowProviderSelector {
            providers,
            provider_ids,
            auth_hints,
        } => {
            provider_selector_items.set(providers);
            provider_selector_ids.set(provider_ids);
            provider_selector_hints.set(auth_hints);
            provider_selector_cursor.set(0);
            show_provider_selector.set(true);
        }
        GwEvent::PromptApiKey {
            provider,
            provider_display,
            help_url,
            help_text,
        } => {
            api_key_provider.set(provider);
            api_key_provider_display.set(provider_display);
            api_key_input.set(String::new());
            api_key_help_url.set(help_url);
            api_key_help_text.set(help_text);
            show_api_key_dialog.set(true);
        }
        GwEvent::DeviceFlowCode {
            owner,
            provider,
            url,
            code,
        } => {
            // Queued like every other request two turns can raise at once:
            // a second sign-in must not overwrite the first, whose code
            // would never be shown while its flow waited out its window.
            let mut queue = queued_device_flows.read().clone();
            queue.push((owner, provider, url, code));
            queued_device_flows.set(queue);
            drain_queued_dialogs(&ui_for_drain);
        }
        GwEvent::DeviceFlowDone(owner) => {
            // Only this flow's completion takes the dialog down; another
            // sign-in finishing — another turn's, or one this client
            // started itself — must not tear away a code the user is
            // still typing. Owners compare exactly: an old gateway's
            // Unattributed flow was drained with that same owner, and a
            // local flow with Local, so every completion finds precisely
            // the flow it refers to.
            if show_device_flow.get() && device_flow_owner.get() == Some(owner) {
                show_device_flow.set(false);
                device_flow_browser_opened.set(false);
                device_flow_owner.set(None);
            }
            // Whether displayed or still queued, the flow is over — a
            // queued entry left behind would later drain into the dialog
            // with a code that already expired.
            let mut queue = queued_device_flows.read().clone();
            let before = queue.len();
            queue.retain(|(o, ..)| *o != owner);
            if queue.len() != before {
                queued_device_flows.set(queue);
            }
        }
        GwEvent::DeviceFlowToken { provider, token } => {
            // Forward the obtained token to the tokio loop
            // for storage + model fetching, reusing SubmitApiKey.
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(
                        tx,
                        UserInput::SubmitApiKey {
                            provider,
                            key: token,
                        },
                    );
                }
            }
        }
        GwEvent::FetchModelsLoading {
            provider,
            provider_display,
        } => {
            model_selector_provider.set(provider);
            model_selector_provider_display.set(provider_display);
            model_selector_models.set(Vec::new());
            model_selector_cursor.set(0);
            model_selector_loading.set(true);
            show_model_selector.set(true);
        }
        GwEvent::ShowModelSelector {
            provider,
            provider_display,
            models,
        } => {
            model_completion_provider.set(Some(provider.clone()));
            model_completion_models.set(models.clone());
            model_completion_loading.set(None);
            model_selector_provider.set(provider);
            model_selector_provider_display.set(provider_display);
            model_selector_models.set(models);
            model_selector_cursor.set(0);
            model_selector_loading.set(false);
            show_model_selector.set(true);
        }
        GwEvent::PromptAttachmentsChanged { attachments } => {
            prompt_attachments.set(attachments);
        }
        GwEvent::ModelCompletionsLoaded { provider, models } => {
            model_completion_provider.set(Some(provider.clone()));
            model_completion_models.set(models.clone());
            model_completion_loading.set(None);

            // If the user is currently typing /model… for this
            // provider, rebuild the autocomplete dropdown so the
            // freshly-fetched models appear without waiting for
            // another keystroke.  The on_change handler that
            // normally populates `command_completions` only fires
            // when the input value changes, so without this the
            // dropdown is stuck on the static list that was in
            // effect when the fetch was first triggered.
            let current_input = input_value.read().clone();
            if let Some(partial) = current_input.strip_prefix('/') {
                if partial.starts_with("model") {
                    let filtered =
                        rustyclaw_view::build_slash_completions(&provider, Some(&models), partial);
                    if filtered.is_empty() {
                        command_completions.set(Vec::new());
                        command_selected.set(None);
                    } else {
                        command_completions.set(filtered);
                        command_selected.set(None);
                    }
                }
            }
        }
        GwEvent::HubModelCompletionsLoaded { query, models } => {
            hub_completion_query.set(Some(query.clone()));
            hub_completion_models.set(models.clone());
            if hub_completion_loading.read().as_deref() == Some(query.as_str()) {
                hub_completion_loading.set(None);
            }

            // If the user is still typing an /engines model argument,
            // rebuild the dropdown so the search results appear without
            // waiting for another keystroke (same reasoning as the
            // ModelCompletionsLoaded arm above). `entries_from_cache`
            // narrows by substring when the input has moved past the
            // query these results answer.
            let current_input = input_value.read().clone();
            if let Some(partial) = current_input.strip_prefix('/') {
                if let Some(ctx) = rustyclaw_view::hub_completion_context(partial) {
                    let provider = dynamic_provider_id.read().clone().unwrap_or_default();
                    let entries = ctx.entries_from_cache(Some(query.as_str()), &models);
                    let filtered = rustyclaw_view::build_slash_completions_with_hub(
                        &provider, None, &entries, partial,
                    );
                    if filtered.is_empty() {
                        command_completions.set(Vec::new());
                        command_selected.set(None);
                    } else {
                        command_completions.set(filtered);
                        command_selected.set(None);
                    }
                }
            }
        }
        GwEvent::PairingSuccess { gateway_name } => {
            // Pairing succeeded — update dialog state
            pairing_step.set(rustyclaw_view::PairingStep::Complete);
            pairing_error.set(String::new());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::success(format!(
                "Successfully paired with gateway: {}",
                gateway_name
            )));
            messages.set(m);
        }
        GwEvent::PairingError(err) => {
            // Pairing failed — show error
            pairing_step.set(rustyclaw_view::PairingStep::EnterGateway);
            pairing_error.set(err.clone());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::error(format!("Pairing failed: {}", err)));
            messages.set(m);
        }
        GwEvent::HostInfo(data) => {
            host_info.set(Some(data));
        }
        GwEvent::LoadStatus(data) => {
            load_status.set(Some(data));
        }
        GwEvent::ServiceList(data) => {
            services_data.set(Some(data));
        }
        GwEvent::ServiceActionResult { service } => {
            if let Some(info) = service {
                let mut current = services_data.read().clone().unwrap_or_default();
                if let Some(existing) = current.services.iter_mut().find(|s| s.name == info.name) {
                    *existing = info;
                } else {
                    current.services.push(info);
                }
                services_data.set(Some(current));
            }
        }
        // ── Engines ──────────────────────────────────────────────────────
        GwEvent::ShowEngines => {
            // A fresh open starts in normal mode with no stale drafts.
            if !show_engines_dialog.get() {
                engines_params_edit.set(false);
                engines_params_drafts.write().clear();
            }
            show_engines_dialog.set(true);
        }
        GwEvent::EngineListResult { engines } => {
            // The configs came from the gateway; drafts seeded from an older
            // snapshot are stale, so drop them (p re-seeds from the fresh
            // config when the user next enters edit mode).
            engines_params_drafts.write().clear();
            let mut data = engines_data.read().clone().unwrap_or_default();
            // Fill in host resources from the last HostInfo snapshot.
            if let Some(host) = host_info.read().as_ref() {
                data.host_ram_bytes = (host.total_memory_gib * 1e9) as u64;
                data.host_vram_bytes = host.gpus.iter().map(|g| (g.vram_gib * 1e9) as u64).sum();
                data.host_gpu_name = host.gpus.first().map(|g| g.name.clone());
            }
            data.engines = engines;
            // Keep the cursor in range and the selection marker in sync.
            let cursor = engines_cursor
                .get()
                .min(data.engines.len().saturating_sub(1));
            engines_cursor.set(cursor);
            data.selected_engine = data.engines.get(cursor).map(|e| e.id.clone());
            engines_data.set(Some(data));
        }
        GwEvent::EngineConfigList { configs } => {
            let mut data = engines_data.read().clone().unwrap_or_default();
            for engine in &mut data.engines {
                if let Some(cfg) = configs.get(&engine.id) {
                    engine.config = cfg.clone();
                }
            }
            engines_data.set(Some(data));
        }
        GwEvent::EngineModelListResult { engine, models } => {
            let mut data = engines_data.read().clone().unwrap_or_default();
            data.selected_engine = Some(engine.clone());
            data.models = models;
            if let Some(idx) = data.engines.iter().position(|e| e.id == engine) {
                engines_cursor.set(idx);
            }
            engines_data.set(Some(data));
        }
        GwEvent::EnginePullProgress {
            engine,
            model,
            percent,
            downloaded_bytes,
            total_bytes,
            status,
        } => {
            let mut data = engines_data.read().clone().unwrap_or_default();
            let finished = status == "complete" || status == "failed";
            if finished {
                data.pull_progress = None;
                let mut m = messages.read().clone();
                if status == "complete" {
                    m.push(DisplayMessage::success(format!(
                        "Pull complete: {} ({})",
                        model, engine
                    )));
                } else {
                    m.push(DisplayMessage::warning(format!(
                        "Pull failed: {} ({})",
                        model, engine
                    )));
                }
                messages.set(m);
            } else {
                data.pull_progress = Some(rustyclaw_view::PullProgressData {
                    engine,
                    model,
                    percent,
                    downloaded_bytes,
                    total_bytes,
                    status,
                });
            }
            engines_data.set(Some(data));
        }
        GwEvent::EngineActionProgress { engine, line } => {
            // Fold the install line into that engine's tab so its output
            // renders live in the dialog rather than scrolling the chat.
            let mut data = engines_data.read().clone().unwrap_or_default();
            data.push_install_line(&engine, line);
            engines_data.set(Some(data));
        }
        GwEvent::EngineActionResult {
            engine,
            model,
            ok,
            message,
        } => {
            // Keep the outcome of model actions (Load/Unload) for the
            // dialog's inline feedback; lifecycle actions surface as
            // notices instead.
            if model.is_some() {
                engines_action_result.set(Some((engine.clone(), ok, message.clone())));
            }
            // Record the terminal outcome on the engine's install panel (so
            // the dialog shows "install complete/failed"), and also surface a
            // one-line notice in the chat. Only finish an install that's
            // actually in progress — EngineActionResult also fires for
            // start/stop, which must not overwrite a completed install.
            let mut data = engines_data.read().clone().unwrap_or_default();
            if data.install_output.get(&engine).is_some_and(|o| !o.done) {
                data.finish_install(&engine, ok, message.clone());
            }
            engines_data.set(Some(data));
            let mut m = messages.read().clone();
            if ok {
                m.push(DisplayMessage::info(format!("Engine: {}", message)));
            } else {
                m.push(DisplayMessage::warning(format!(
                    "Engine error: {}",
                    message
                )));
            }
            messages.set(m);
        }
        GwEvent::ClearMessages => {
            streaming_buf.set(String::new());
            selected_message_idx.set(None);
            scroll_offset.set(0);
            messages.set(vec![DisplayMessage::info(
                "Messages cleared. (Thread history on the gateway is unaffected — switch threads to reload it.)",
            )]);
        }
        GwEvent::ShowGatewayStatus => {
            let status = gw_status.get();
            let model_label = dynamic_model_label
                .read()
                .clone()
                .unwrap_or_else(|| "(no model)".to_string());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::info(format!(
                "Gateway: {} · {}",
                status.label(),
                model_label
            )));
            messages.set(m);
        }
        // ── Downloads panel ──────────────────────────────────────────────
        GwEvent::ShowDownloads => {
            show_downloads_dialog.set(true);
        }
        GwEvent::DownloadsUpdate { downloads } => {
            // Clamped rather than reset: a transfer finishing while the panel
            // is open shortens the list, and a cursor left past the end would
            // highlight nothing until the user moved it.
            downloads_cursor.set(
                downloads_cursor
                    .get()
                    .min(downloads.len().saturating_sub(1)),
            );
            downloads_data.set(Some(rustyclaw_view::DownloadsData { downloads }));
        }
        // ── Gateway panels (cron / memory / MCP / channels) ──────────────
        GwEvent::ShowCron => {
            let mut data = cron_data.read().clone().unwrap_or_default();
            data.status = Some("Loading…".into());
            cron_data.set(Some(data));
            show_cron_dialog.set(true);
        }
        GwEvent::CronListResult { jobs } => {
            let mut data = cron_data.read().clone().unwrap_or_default();
            data.selected = match jobs.is_empty() {
                true => None,
                false => Some(data.selected.unwrap_or(0).min(jobs.len() - 1)),
            };
            data.jobs = jobs;
            data.status = None;
            cron_data.set(Some(data));
        }
        GwEvent::ShowMemory { query } => {
            let mut data = memory_data.read().clone().unwrap_or_default();
            data.search_query = query.unwrap_or_default();
            data.status = Some("Loading…".into());
            memory_data.set(Some(data));
            show_memory_dialog.set(true);
        }
        GwEvent::MemoryListResult { entries } => {
            let mut data = memory_data.read().clone().unwrap_or_default();
            data.selected = match entries.is_empty() {
                true => None,
                false => Some(data.selected.unwrap_or(0).min(entries.len() - 1)),
            };
            data.entries = entries;
            data.status = None;
            memory_data.set(Some(data));
        }
        GwEvent::HistorySearchResult { entries } => {
            let mut data = memory_data.read().clone().unwrap_or_default();
            data.history = entries;
            data.status = None;
            memory_data.set(Some(data));
            show_memory_dialog.set(true);
        }
        GwEvent::ShowMcp => {
            let mut data = mcp_data.read().clone().unwrap_or_default();
            data.status = Some("Loading…".into());
            mcp_data.set(Some(data));
            show_mcp_dialog.set(true);
        }
        GwEvent::McpListResult { servers } => {
            let mut data = mcp_data.read().clone().unwrap_or_default();
            data.selected = match servers.is_empty() {
                true => None,
                false => Some(data.selected.unwrap_or(0).min(servers.len() - 1)),
            };
            data.servers = servers;
            data.status = None;
            mcp_data.set(Some(data));
        }
        GwEvent::ShowChannels => {
            let mut data = channels_data.read().clone().unwrap_or_default();
            data.status = Some("Loading…".into());
            channels_data.set(Some(data));
            show_channels_dialog.set(true);
        }
        GwEvent::ChannelStatusResult { channels } => {
            let mut data = channels_data.read().clone().unwrap_or_default();
            data.selected = match channels.is_empty() {
                true => None,
                false => Some(data.selected.unwrap_or(0).min(channels.len() - 1)),
            };
            data.channels = channels;
            data.status = None;
            channels_data.set(Some(data));
        }
        GwEvent::ShowMessengers => {
            let mut data = messengers_data.read().clone().unwrap_or_default();
            data.set_status("Loading…", false);
            messengers_data.set(Some(data));
            show_messengers_dialog.set(true);
        }
        GwEvent::MessengerConfigResult {
            accounts,
            routes,
            threads,
            available_kinds,
            vault_locked,
        } => {
            let mut data = messengers_data.read().clone().unwrap_or_default();
            // A refresh arrives after every mutation, so `apply` keeps the
            // cursor where the user left it rather than resetting to the top.
            data.apply(accounts, routes, threads, available_kinds, vault_locked);
            // The load is over, so the "Loading…" placeholder must go — but
            // only that. The gateway answers every mutation with an action
            // result *followed by* this refresh, so clearing the status
            // unconditionally would wipe each "Saved" or failure message the
            // moment it was set.
            if data.status.as_deref() == Some("Loading…") {
                data.status = None;
                data.status_is_error = false;
            }
            // The editor is closed by whatever opened it; a refresh landing
            // mid-edit must not discard what the user has typed.
            messengers_data.set(Some(data));
        }
        GwEvent::MessengerActionResult { ok, message } => {
            let mut data = messengers_data.read().clone().unwrap_or_default();
            match (&message, ok) {
                (Some(message), _) => data.set_status(message.clone(), !ok),
                (None, true) => data.set_status("Saved", false),
                (None, false) => data.set_status("Failed", true),
            }
            // A rejected save leaves the editor open with its contents intact
            // so the user can fix the problem the message just named.
            if ok {
                data.commits += 1;
                data.editor = None;
                data.route_editor = None;
                data.kind_picker = None;
            }
            messengers_data.set(Some(data));
        }
        GwEvent::ShowAnalytics => {
            let mut data = analytics_data.read().clone().unwrap_or_default();
            data.status = Some("Loading…".into());
            analytics_data.set(Some(data));
            show_analytics_dialog.set(true);
        }
        GwEvent::UsageStatsResult {
            totals,
            per_model,
            per_session,
        } => {
            let mut data = analytics_data.read().clone().unwrap_or_default();
            data.period = totals.period.clone();
            data.totals = totals;
            data.per_model = per_model;
            data.per_session = per_session;
            data.status = None;
            analytics_data.set(Some(data));
        }
        GwEvent::ShowLogs { source } => {
            let mut data = logs_data.read().clone().unwrap_or_default();
            data.source = rustyclaw_view::LogSource::from_wire(&source);
            data.status = Some("Loading…".into());
            logs_data.set(Some(data));
            show_logs_dialog.set(true);
        }
        GwEvent::LogsResult {
            ok,
            source,
            lines,
            message,
        } => {
            let mut data = logs_data.read().clone().unwrap_or_default();
            data.source = rustyclaw_view::LogSource::from_wire(&source);
            data.lines = lines;
            data.status = match (ok, message) {
                (false, Some(msg)) => Some(msg),
                _ => None,
            };
            data.scroll_offset = data.lines.len().saturating_sub(1);
            logs_data.set(Some(data));
            show_logs_dialog.set(true);
        }
        GwEvent::PanelActionResult { panel, ok, message } => {
            // Surface the outcome in the panel's status line (and the
            // message log on failure), then re-fetch the list.
            let status = match (&message, ok) {
                (Some(msg), _) => Some(msg.clone()),
                (None, true) => Some("Done".into()),
                (None, false) => Some("Failed".into()),
            };
            match panel {
                PanelKind::Cron => {
                    let mut data = cron_data.read().clone().unwrap_or_default();
                    data.status = status;
                    cron_data.set(Some(data));
                }
                PanelKind::Memory => {
                    let mut data = memory_data.read().clone().unwrap_or_default();
                    data.status = status;
                    memory_data.set(Some(data));
                }
                PanelKind::Mcp => {
                    let mut data = mcp_data.read().clone().unwrap_or_default();
                    data.status = status;
                    mcp_data.set(Some(data));
                }
                PanelKind::Channels => {
                    let mut data = channels_data.read().clone().unwrap_or_default();
                    data.status = status;
                    channels_data.set(Some(data));
                }
            }
            if !ok {
                let mut m = messages.read().clone();
                m.push(DisplayMessage::warning(format!(
                    "{}: {}",
                    panel.label(),
                    message.unwrap_or_else(|| "operation failed".into())
                )));
                messages.set(m);
            }
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    crate::app::events::submit(tx, UserInput::RefreshPanel(panel));
                }
            }
        }
    }
}

/// Adapt a transport `ThreadInfoDto` into the shared view-layer
/// [`SidebarItemData`](rustyclaw_view::SidebarItemData).
///
/// This is the client's transport→view boundary; grouping and display logic
/// then live entirely in rustyclaw-view.
fn item_from_thread(t: &crate::action::ThreadInfo) -> rustyclaw_view::SidebarItemData {
    let status = t.status.clone().unwrap_or_default();
    rustyclaw_view::SidebarItemData {
        id: t.id,
        project_id: t.project_id,
        label: if t.label.is_empty() {
            None
        } else {
            Some(t.label.clone())
        },
        description: t.description.clone(),
        status: status.clone(),
        state: rustyclaw_view::ThreadState::from_status(&status),
        is_foreground: t.is_foreground,
        message_count: t.message_count,
        working_dir: t.working_dir.clone(),
        pinned: t.pinned,
    }
}
