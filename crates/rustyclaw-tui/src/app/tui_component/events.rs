//! Gateway-event handling for the TUI root: applies each `GwEvent` to UI state.

use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use rustyclaw_view::tracing;

use super::display_message_from_gateway;
use super::state;
use crate::app::{GwEvent, PanelKind, UserInput};
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

/// Apply a single gateway event to the UI state bundle.
pub(super) fn apply_gw_event(
    ev: GwEvent,
    ui: state::Ui,
    needs_hatching: bool,
    tx_for_history: &UserTx,
) {
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
        mut threads,
        mut projects,
        mut active_project_id,
        mut tab_focused,
        mut tab_selected,
        mut thread_messages_cache,
        mut foreground_thread_id,
        mut command_completions,
        mut command_selected,
        mut model_completion_provider,
        mut model_completion_models,
        mut model_completion_loading,
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
        mut services_data,
        mut show_engines_dialog,
        mut engines_data,
        mut engines_cursor,
        mut show_cron_dialog,
        mut cron_data,
        mut show_memory_dialog,
        mut memory_data,
        mut show_mcp_dialog,
        mut mcp_data,
        mut show_channels_dialog,
        mut channels_data,
        mut show_analytics_dialog,
        mut analytics_data,
        mut show_logs_dialog,
        mut logs_data,
    } = ui;
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
            let mut m = messages.read().clone();
            m.push(DisplayMessage::warning(format!("Disconnected: {}", reason)));
            messages.set(m);
        }
        GwEvent::Connected => {
            gw_status.set(rustyclaw_core::types::GatewayStatus::Connected);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::info("Gateway connected."));
            messages.set(m);
            // Reset foreground tracking so the next ThreadsUpdate
            // always triggers a fresh history fetch, even when the
            // same thread stays foreground across a reconnect.
            foreground_thread_id.set(None);
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.send(UserInput::RefreshThreads);
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
                    let _ = tx.send(UserInput::RefreshThreads);
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
            // Always stop the spinner / streaming state so
            // the TUI doesn't get stuck in "Thinking…" after
            // a provider error (e.g. 400 Bad Request).
            streaming.set(false);
            stream_start.set(None);
            elapsed.set(String::new());
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
        GwEvent::StreamStart => {
            streaming.set(true);
            // Keep the earlier start time if we already
            // began timing on user submit.
            if stream_start.get().is_none() {
                stream_start.set(Some(Instant::now()));
            }
            streaming_buf.set(String::new());
        }
        GwEvent::Chunk(text) => {
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
        GwEvent::ResponseDone => {
            // Capture the accumulated assistant text and
            // send it back to the tokio loop so it gets
            // appended to the conversation history.
            let completed_text = streaming_buf.read().clone();

            if !completed_text.is_empty() {
                if let Ok(guard) = tx_for_history.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send(UserInput::AssistantResponse(completed_text));
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
                    let _ = tx.send(UserInput::RefreshTasks);
                }
            }
        }
        GwEvent::ThinkingStart => {
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
        GwEvent::ThinkingDelta(delta) => {
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
        GwEvent::ThinkingEnd => {
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
            id,
            name,
            arguments,
        } => {
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
        GwEvent::ToolStatus { id, status } => {
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
        GwEvent::ToolOutput { id, chunk } => {
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
            id,
            name,
            result,
            is_error,
        } => {
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
            id,
            name,
            arguments,
        } => {
            // Show tool approval dialog
            tool_approval_id.set(id);
            tool_approval_name.set(name.clone());
            tool_approval_args.set(arguments.clone());
            tool_approval_selected.set(true);
            show_tool_approval.set(true);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::system(format!(
                "🔐 Tool approval required: {} — press Enter to allow, Esc to deny",
                name,
            )));
            messages.set(m);
        }
        GwEvent::UserPromptRequest(prompt) => {
            // Show user prompt dialog
            user_prompt_id.set(prompt.id.clone());
            user_prompt_title.set(prompt.title.clone());
            user_prompt_desc.set(prompt.description.clone().unwrap_or_default());
            user_prompt_input.set(String::new());
            user_prompt_type.set(Some(prompt.prompt_type.clone()));
            // Set default selection based on prompt type
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
            // Seed MultiSelect checkboxes from the prompt's defaults.
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
        }
        GwEvent::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => {
            credential_request_id.set(id);
            credential_request_provider.set(provider.clone());
            credential_request_secret_name.set(secret_name.clone());
            credential_request_message.set(message.clone());
            credential_request_input.set(String::new());
            show_credential_request.set(true);
            let mut m = messages.read().clone();
            m.push(DisplayMessage::warning(format!(
                "🔑 Credential required for {} ({}) — enter API key",
                provider, secret_name,
            )));
            messages.set(m);
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
                    let _ = tx.send(UserInput::RefreshSecrets);
                }
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
                if let Some(thread_id) = foreground_id {
                    tracing::debug!(
                        thread_id,
                        previous_foreground = ?previous_foreground,
                        "TUI requesting thread history after ThreadsUpdate"
                    );
                    if let Ok(guard) = tx_for_history.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(UserInput::RequestThreadHistory(thread_id));
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
        GwEvent::ThreadMessages {
            thread_id: _,
            messages: thread_messages,
        } => {
            messages.set(
                thread_messages
                    .into_iter()
                    .map(display_message_from_gateway)
                    .collect(),
            );
            scroll_offset.set(0);
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
            // Ask the gateway for the authoritative,
            // cross-session history for this thread so
            // the local cache stays consistent with
            // what the gateway has persisted.
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.send(UserInput::RequestThreadHistory(thread_id));
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
            provider,
            url,
            code,
        } => {
            device_flow_provider.set(provider);
            device_flow_url.set(url.clone());
            device_flow_code.set(code);
            device_flow_tick.set(0);
            // Auto-open the verification URL in the browser
            crate::components::device_flow_dialog::open_url_in_browser(&url);
            device_flow_browser_opened.set(true);
            show_device_flow.set(true);
        }
        GwEvent::DeviceFlowDone => {
            show_device_flow.set(false);
            device_flow_browser_opened.set(false);
        }
        GwEvent::DeviceFlowToken { provider, token } => {
            // Forward the obtained token to the tokio loop
            // for storage + model fetching, reusing SubmitApiKey.
            if let Ok(guard) = tx_for_history.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.send(UserInput::SubmitApiKey {
                        provider,
                        key: token,
                    });
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
            show_engines_dialog.set(true);
        }
        GwEvent::EngineListResult { engines } => {
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
            ok,
            message,
            ..
        } => {
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
                    let _ = tx.send(UserInput::RefreshPanel(panel));
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
    rustyclaw_view::SidebarItemData {
        id: t.id,
        project_id: t.project_id,
        label: if t.label.is_empty() {
            None
        } else {
            Some(t.label.clone())
        },
        description: t.description.clone(),
        status: t.status.clone().unwrap_or_default(),
        is_foreground: t.is_foreground,
        message_count: t.message_count,
    }
}
