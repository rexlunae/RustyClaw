//! Normal-mode (no dialog open) keyboard handling for the TUI root.

use iocraft::prelude::*;
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};

use std::time::Instant;

use super::state;
use crate::app::UserInput;
use crate::types::DisplayMessage;

type UserTx = Arc<StdMutex<Option<sync_mpsc::Sender<UserInput>>>>;

/// Handle a key press when no modal dialog is focused.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_normal_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    ui: state::Ui,
    tx_for_keys: &UserTx,
    tx_for_model_completions: &UserTx,
    prop_provider_id: &str,
    prop_gateway_host: &str,
    prop_gateway_port: &str,
) {
    #[allow(unused_variables, unused_mut)]
    let state::Ui {
        mut messages,
        mut input_value,
        mut input_cursor_offset,
        mut gw_status,
        mut streaming,
        mut stream_start,
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
    } = ui;
    // ── Normal mode keyboard ────────────────────────
    // Details dialog: Esc to close, PgUp/PgDn to scroll
    if show_details_dialog.get() {
        match code {
            KeyCode::Esc => {
                show_details_dialog.set(false);
                details_dialog_scroll.set(0);
            }
            KeyCode::PageDown | KeyCode::Down => {
                let total = details_dialog_text.read().lines().count();
                let next = (details_dialog_scroll.get() + 5).min(total.saturating_sub(1));
                details_dialog_scroll.set(next);
            }
            KeyCode::PageUp | KeyCode::Up => {
                let cur = details_dialog_scroll.get();
                details_dialog_scroll.set(cur.saturating_sub(5));
            }
            _ => {}
        }
        return;
    }

    // Info dialogs: Esc to close, Up/Down to navigate, Enter to act
    if show_skills_dialog.get() {
        const VISIBLE_ROWS: usize = 20;
        match code {
            KeyCode::Esc => {
                show_skills_dialog.set(false);
            }
            KeyCode::Up => {
                let cur = skills_selected.get().unwrap_or(0);
                let len = skills_dialog_data.read().len();
                if len > 0 {
                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                    skills_selected.set(Some(next));
                    // Adjust scroll offset
                    let so = skills_scroll_offset.get();
                    if next < so {
                        skills_scroll_offset.set(next);
                    } else if next >= so + VISIBLE_ROWS {
                        skills_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                    }
                }
            }
            KeyCode::Down => {
                let cur = skills_selected.get().unwrap_or(0);
                let len = skills_dialog_data.read().len();
                if len > 0 {
                    let next = (cur + 1) % len;
                    skills_selected.set(Some(next));
                    // Adjust scroll offset
                    let so = skills_scroll_offset.get();
                    if next < so {
                        skills_scroll_offset.set(next);
                    } else if next >= so + VISIBLE_ROWS {
                        skills_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                    }
                }
            }
            KeyCode::Enter => {
                let idx = skills_selected.get().unwrap_or(0);
                let data = skills_dialog_data.read();
                if let Some(skill) = data.get(idx) {
                    let name = skill.name.clone();
                    drop(data);
                    if let Ok(guard) = tx_for_keys.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(UserInput::ToggleSkill { name });
                        }
                    }
                }
            }
            _ => {}
        }
        return;
    }
    if show_tool_perms_dialog.get() {
        const VISIBLE_ROWS: usize = 20;
        match code {
            KeyCode::Esc => {
                show_tool_perms_dialog.set(false);
            }
            KeyCode::Up => {
                let cur = tool_perms_selected.get().unwrap_or(0);
                let len = tool_perms_dialog_data.read().len();
                if len > 0 {
                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                    tool_perms_selected.set(Some(next));
                    let so = tool_perms_scroll_offset.get();
                    if next < so {
                        tool_perms_scroll_offset.set(next);
                    } else if next >= so + VISIBLE_ROWS {
                        tool_perms_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                    }
                }
            }
            KeyCode::Down => {
                let cur = tool_perms_selected.get().unwrap_or(0);
                let len = tool_perms_dialog_data.read().len();
                if len > 0 {
                    let next = (cur + 1) % len;
                    tool_perms_selected.set(Some(next));
                    let so = tool_perms_scroll_offset.get();
                    if next < so {
                        tool_perms_scroll_offset.set(next);
                    } else if next >= so + VISIBLE_ROWS {
                        tool_perms_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                    }
                }
            }
            KeyCode::Enter => {
                let idx = tool_perms_selected.get().unwrap_or(0);
                let data = tool_perms_dialog_data.read();
                if let Some(tool) = data.get(idx) {
                    let name = tool.name.clone();
                    drop(data);
                    if let Ok(guard) = tx_for_keys.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(UserInput::CycleToolPermission { name });
                        }
                    }
                }
            }
            _ => {}
        }
        return;
    }
    if show_secrets_dialog.get() {
        const VISIBLE_ROWS: usize = 20;
        // Add-secret inline input mode
        let add_step = secrets_add_step.get();
        if add_step > 0 {
            match code {
                KeyCode::Esc => {
                    secrets_add_step.set(0);
                    secrets_add_name.set(String::new());
                    secrets_add_value.set(String::new());
                }
                KeyCode::Enter => {
                    if add_step == 1 {
                        // Name entered, move to value
                        if !secrets_add_name.read().trim().is_empty() {
                            secrets_add_step.set(2);
                        }
                    } else {
                        // Value entered, submit
                        let name = secrets_add_name.read().trim().to_string();
                        let value = secrets_add_value.read().clone();
                        if !name.is_empty() && !value.is_empty() {
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::AddSecret { name, value });
                                }
                            }
                        }
                        secrets_add_step.set(0);
                        secrets_add_name.set(String::new());
                        secrets_add_value.set(String::new());
                    }
                }
                KeyCode::Backspace => {
                    if add_step == 1 {
                        let mut s = secrets_add_name.read().clone();
                        s.pop();
                        secrets_add_name.set(s);
                    } else {
                        let mut s = secrets_add_value.read().clone();
                        s.pop();
                        secrets_add_value.set(s);
                    }
                }
                KeyCode::Char(c) => {
                    if add_step == 1 {
                        let mut s = secrets_add_name.read().clone();
                        s.push(c);
                        secrets_add_name.set(s);
                    } else {
                        let mut s = secrets_add_value.read().clone();
                        s.push(c);
                        secrets_add_value.set(s);
                    }
                }
                _ => {}
            }
            return;
        }
        // Normal secrets dialog navigation
        match code {
            KeyCode::Esc => {
                show_secrets_dialog.set(false);
            }
            KeyCode::Up => {
                let cur = secrets_selected.get().unwrap_or(0);
                let len = secrets_dialog_data.read().len();
                if len > 0 {
                    let next = if cur == 0 { len - 1 } else { cur - 1 };
                    secrets_selected.set(Some(next));
                    let so = secrets_scroll_offset.get();
                    if next < so {
                        secrets_scroll_offset.set(next);
                    } else if next >= so + VISIBLE_ROWS {
                        secrets_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                    }
                }
            }
            KeyCode::Down => {
                let cur = secrets_selected.get().unwrap_or(0);
                let len = secrets_dialog_data.read().len();
                if len > 0 {
                    let next = (cur + 1) % len;
                    secrets_selected.set(Some(next));
                    let so = secrets_scroll_offset.get();
                    if next < so {
                        secrets_scroll_offset.set(next);
                    } else if next >= so + VISIBLE_ROWS {
                        secrets_scroll_offset.set(next.saturating_sub(VISIBLE_ROWS - 1));
                    }
                }
            }
            KeyCode::Enter => {
                // Cycle permission policy
                let idx = secrets_selected.get().unwrap_or(0);
                let data = secrets_dialog_data.read();
                if let Some(secret) = data.get(idx) {
                    let name = secret.key.clone();
                    let policy = secret.policy.clone();
                    drop(data);
                    if let Ok(guard) = tx_for_keys.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(UserInput::CycleSecretPolicy {
                                name,
                                current_policy: policy,
                            });
                        }
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                // Delete selected secret
                let idx = secrets_selected.get().unwrap_or(0);
                let data = secrets_dialog_data.read();
                if let Some(secret) = data.get(idx) {
                    let name = secret.key.clone();
                    drop(data);
                    if let Ok(guard) = tx_for_keys.lock() {
                        if let Some(ref tx) = *guard {
                            let _ = tx.send(UserInput::DeleteSecret { name });
                        }
                    }
                }
            }
            KeyCode::Char('a') => {
                // Start add-secret inline input
                secrets_add_step.set(1);
                secrets_add_name.set(String::new());
                secrets_add_value.set(String::new());
            }
            _ => {}
        }
        return;
    }

    // Command menu intercepts when visible
    let menu_open = rustyclaw_view::CommandMenuData {
        completions: command_completions.read().clone(),
        selected: command_selected.get(),
    }
    .is_open();

    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            should_quit.set(true);
            if let Ok(guard) = tx_for_keys.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.send(UserInput::Quit);
                }
            }
        }
        KeyCode::Esc if streaming.get() => {
            // Cancel current run while preserving app session.
            streaming.set(false);
            stream_start.set(None);
            elapsed.set(String::new());
            let mut m = messages.read().clone();
            m.push(DisplayMessage::info("Cancellation requested…"));
            messages.set(m);
            if let Ok(guard) = tx_for_keys.lock() {
                if let Some(ref tx) = *guard {
                    let _ = tx.send(UserInput::CancelCurrentRequest);
                }
            }
        }
        KeyCode::Tab if menu_open => {
            let mut menu = rustyclaw_view::CommandMenuData {
                completions: command_completions.read().clone(),
                selected: command_selected.get(),
            };
            if let Some(input) = menu.select_next_input_value() {
                input_value.set(input);
            }
            command_completions.set(menu.completions);
            command_selected.set(menu.selected);
        }
        KeyCode::BackTab if menu_open => {
            let mut menu = rustyclaw_view::CommandMenuData {
                completions: command_completions.read().clone(),
                selected: command_selected.get(),
            };
            if let Some(input) = menu.select_prev_input_value() {
                input_value.set(input);
            }
            command_completions.set(menu.completions);
            command_selected.set(menu.selected);
        }
        KeyCode::Up if menu_open => {
            let mut menu = rustyclaw_view::CommandMenuData {
                completions: command_completions.read().clone(),
                selected: command_selected.get(),
            };
            if let Some(input) = menu.select_prev_input_value() {
                input_value.set(input);
            }
            command_completions.set(menu.completions);
            command_selected.set(menu.selected);
        }
        KeyCode::Down if menu_open => {
            let mut menu = rustyclaw_view::CommandMenuData {
                completions: command_completions.read().clone(),
                selected: command_selected.get(),
            };
            if let Some(input) = menu.select_next_input_value() {
                input_value.set(input);
            }
            command_completions.set(menu.completions);
            command_selected.set(menu.selected);
        }
        KeyCode::Esc if menu_open => {
            let mut menu = rustyclaw_view::CommandMenuData {
                completions: command_completions.read().clone(),
                selected: command_selected.get(),
            };
            menu.clear();
            command_completions.set(menu.completions);
            command_selected.set(menu.selected);
        }
        KeyCode::Home if !tab_focused.get() => {
            input_cursor_offset.set(0);
        }
        KeyCode::End if !tab_focused.get() => {
            input_cursor_offset.set(input_value.read().len());
        }
        KeyCode::Left if !tab_focused.get() => {
            let cursor = input_cursor_offset.get();
            if cursor > 0 {
                let text = input_value.read();
                let mut next = cursor.saturating_sub(1);
                while next > 0 && !text.is_char_boundary(next) {
                    next -= 1;
                }
                input_cursor_offset.set(next);
            }
        }
        KeyCode::Right if !tab_focused.get() => {
            let cursor = input_cursor_offset.get();
            let text = input_value.read();
            if cursor < text.len() {
                let mut next = cursor + 1;
                while next < text.len() && !text.is_char_boundary(next) {
                    next += 1;
                }
                input_cursor_offset.set(next);
            }
        }
        KeyCode::Backspace if !tab_focused.get() => {
            let cursor = input_cursor_offset.get();
            if cursor > 0 {
                let mut text = input_value.read().clone();
                let mut remove_at = cursor.saturating_sub(1);
                while remove_at > 0 && !text.is_char_boundary(remove_at) {
                    remove_at -= 1;
                }
                if remove_at < text.len() {
                    text.remove(remove_at);
                    input_value.set(text.clone());
                    input_cursor_offset.set(remove_at);
                    let current_text = text;
                    if let Some(partial) = current_text.strip_prefix('/') {
                        let current_pid = dynamic_provider_id
                            .read()
                            .clone()
                            .unwrap_or_else(|| prop_provider_id.to_string());
                        if partial.starts_with("model") {
                            let loaded = model_completion_provider.read().as_deref()
                                == Some(current_pid.as_str());
                            let loading = model_completion_loading.read().as_deref()
                                == Some(current_pid.as_str());
                            if !loaded && !loading {
                                model_completion_loading.set(Some(current_pid.clone()));
                                if let Ok(guard) = tx_for_model_completions.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::FetchModelCompletions {
                                            provider: current_pid.clone(),
                                        });
                                    }
                                }
                            }
                        }

                        let live_models = model_completion_models.read().clone();
                        let live_provider_matches = model_completion_provider.read().as_deref()
                            == Some(current_pid.as_str());
                        let filtered = rustyclaw_view::build_slash_completions(
                            &current_pid,
                            if live_provider_matches {
                                Some(live_models.as_slice())
                            } else {
                                None
                            },
                            partial,
                        );
                        if filtered.is_empty() {
                            command_completions.set(Vec::new());
                            command_selected.set(None);
                        } else {
                            command_completions.set(filtered);
                            command_selected.set(None);
                        }
                    } else {
                        command_completions.set(Vec::new());
                        command_selected.set(None);
                    }
                }
            }
        }
        KeyCode::Delete if !tab_focused.get() => {
            let cursor = input_cursor_offset.get();
            let mut text = input_value.read().clone();
            if cursor < text.len() && text.is_char_boundary(cursor) {
                text.remove(cursor);
                input_value.set(text.clone());
                let current_text = text;
                if let Some(partial) = current_text.strip_prefix('/') {
                    let current_pid = dynamic_provider_id
                        .read()
                        .clone()
                        .unwrap_or_else(|| prop_provider_id.to_string());
                    if partial.starts_with("model") {
                        let loaded = model_completion_provider.read().as_deref()
                            == Some(current_pid.as_str());
                        let loading = model_completion_loading.read().as_deref()
                            == Some(current_pid.as_str());
                        if !loaded && !loading {
                            model_completion_loading.set(Some(current_pid.clone()));
                            if let Ok(guard) = tx_for_model_completions.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::FetchModelCompletions {
                                        provider: current_pid.clone(),
                                    });
                                }
                            }
                        }
                    }

                    let live_models = model_completion_models.read().clone();
                    let live_provider_matches =
                        model_completion_provider.read().as_deref() == Some(current_pid.as_str());
                    let filtered = rustyclaw_view::build_slash_completions(
                        &current_pid,
                        if live_provider_matches {
                            Some(live_models.as_slice())
                        } else {
                            None
                        },
                        partial,
                    );
                    if filtered.is_empty() {
                        command_completions.set(Vec::new());
                        command_selected.set(None);
                    } else {
                        command_completions.set(filtered);
                        command_selected.set(None);
                    }
                } else {
                    command_completions.set(Vec::new());
                    command_selected.set(None);
                }
            }
        }
        KeyCode::Char(c)
            if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !tab_focused.get() =>
        {
            // Some terminals deliver Shift+<letter> as the
            // lowercase char plus a SHIFT modifier instead of
            // pre-shifting the codepoint. Normalize that here so
            // typed uppercase letters are preserved.
            let c = if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            };
            let mut text = input_value.read().clone();
            let cursor = input_cursor_offset.get().min(text.len());
            text.insert(cursor, c);
            let next_cursor = cursor + c.len_utf8();
            input_value.set(text.clone());
            input_cursor_offset.set(next_cursor);

            if let Some(partial) = text.strip_prefix('/') {
                let current_pid = dynamic_provider_id
                    .read()
                    .clone()
                    .unwrap_or_else(|| prop_provider_id.to_string());
                if partial.starts_with("model") {
                    let loaded =
                        model_completion_provider.read().as_deref() == Some(current_pid.as_str());
                    let loading =
                        model_completion_loading.read().as_deref() == Some(current_pid.as_str());
                    if !loaded && !loading {
                        model_completion_loading.set(Some(current_pid.clone()));
                        if let Ok(guard) = tx_for_model_completions.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::FetchModelCompletions {
                                    provider: current_pid.clone(),
                                });
                            }
                        }
                    }
                }

                let live_models = model_completion_models.read().clone();
                let live_provider_matches =
                    model_completion_provider.read().as_deref() == Some(current_pid.as_str());
                let filtered = rustyclaw_view::build_slash_completions(
                    &current_pid,
                    if live_provider_matches {
                        Some(live_models.as_slice())
                    } else {
                        None
                    },
                    partial,
                );
                if filtered.is_empty() {
                    command_completions.set(Vec::new());
                    command_selected.set(None);
                } else {
                    command_completions.set(filtered);
                    command_selected.set(None);
                }
            } else {
                command_completions.set(Vec::new());
                command_selected.set(None);
            }
        }
        KeyCode::Enter if tab_focused.get() => {
            let thread_list = threads.read().clone();
            if let Some(thread) = thread_list.get(tab_selected.get()) {
                // Send thread switch request
                if let Ok(guard) = tx_for_keys.lock() {
                    if let Some(ref tx) = *guard {
                        let _ = tx.send(UserInput::ThreadSwitch(thread.id));
                    }
                }
            }
            // Return focus to input after tab selection
            tab_focused.set(false);
        }
        KeyCode::Enter => {
            let val = input_value.to_string();
            if !val.is_empty() {
                input_value.set(String::new());
                input_cursor_offset.set(0);
                let mut menu = rustyclaw_view::CommandMenuData {
                    completions: command_completions.read().clone(),
                    selected: command_selected.get(),
                };
                menu.clear();
                command_completions.set(menu.completions);
                command_selected.set(menu.selected);
                // Snap to bottom so user sees their message + response
                scroll_offset.set(0);
                if let Ok(guard) = tx_for_keys.lock() {
                    if let Some(ref tx) = *guard {
                        if val.starts_with('/') {
                            let _ = tx
                                .send(UserInput::Command(val.trim_start_matches('/').to_string()));
                        } else {
                            let mut m = messages.read().clone();
                            m.push(DisplayMessage::user(&val));
                            m.push(DisplayMessage::info("Running… Press Esc to cancel."));
                            messages.set(m);
                            // Start the spinner immediately so the user
                            // sees feedback while waiting for the model.
                            streaming.set(true);
                            stream_start.set(Some(Instant::now()));
                            let _ = tx.send(UserInput::Chat(val));
                        }
                    }
                }
            }
        }
        // Tab toggles tab bar focus when command menu is not open
        KeyCode::Tab if !menu_open => {
            tab_focused.set(!tab_focused.get());
        }
        // Tab navigation when focused
        KeyCode::Left if tab_focused.get() => {
            let thread_count = threads.read().len();
            if thread_count > 0 {
                let current = tab_selected.get();
                tab_selected.set(current.saturating_sub(1));
            }
        }
        KeyCode::Right if tab_focused.get() => {
            let thread_count = threads.read().len();
            if thread_count > 0 {
                let current = tab_selected.get();
                tab_selected.set((current + 1).min(thread_count - 1));
            }
        }
        KeyCode::Esc if tab_focused.get() => {
            // Escape returns focus to input
            tab_focused.set(false);
        }
        KeyCode::Up => {
            scroll_offset.set(scroll_offset.get() + 1);
        }
        KeyCode::Down => {
            scroll_offset.set((scroll_offset.get() - 1).max(0));
        }
        // Ctrl+D opens the details dialog for the most recent
        // warning/error message that carries extended
        // structured details (URL, status, redacted headers,
        // body excerpt, full cause chain).  Only fires when
        // there's actually something to show.
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            let msgs = messages.read();
            // Walk backwards to find the most recent
            // warning/error with details attached.
            let entry = msgs.iter().rev().find(|m| {
                matches!(
                    m.role,
                    rustyclaw_core::types::MessageRole::Warning
                        | rustyclaw_core::types::MessageRole::Error
                ) && m.details.is_some()
            });
            if let Some(msg) = entry {
                details_dialog_text.set(msg.details.clone().unwrap_or_default());
                details_dialog_is_error.set(matches!(
                    msg.role,
                    rustyclaw_core::types::MessageRole::Error
                ));
                details_dialog_scroll.set(0);
                show_details_dialog.set(true);
            }
        }
        // Ctrl+P / Ctrl+Shift+P opens pairing dialog.
        // Many terminals normalize Ctrl+Shift+P to Ctrl+P.
        KeyCode::Char(c)
            if modifiers.contains(KeyModifiers::CONTROL)
                && c.eq_ignore_ascii_case(&'p')
                && !show_pairing.get() =>
        {
            {
                // Generate keypair and populate dialog
                use rustyclaw_core::pairing::{
                    ClientKeyPair, PairingData, format_fingerprint_art, generate_pairing_qr_ascii,
                    key_fingerprint,
                };
                match ClientKeyPair::load_or_generate(None) {
                    Ok(kp) => {
                        let pk = kp.public_key_openssh();
                        pairing_public_key.set(pk.clone());
                        let fp = key_fingerprint(&kp);
                        pairing_fingerprint_art.set(format_fingerprint_art(&fp));
                        pairing_fingerprint.set(fp);

                        // Generate QR code for pairing
                        let pairing_data = PairingData::client(&pk, None);
                        match generate_pairing_qr_ascii(&pairing_data) {
                            Ok(qr) => pairing_qr_ascii.set(qr),
                            Err(_) => pairing_qr_ascii.set(String::new()),
                        }
                    }
                    Err(e) => {
                        pairing_error.set(format!("Key generation failed: {}", e));
                    }
                }
                pairing_step.set(rustyclaw_view::PairingStep::ShowKey);
                pairing_field.set(rustyclaw_view::PairingField::Host);
                pairing_host.set(prop_gateway_host.to_string());
                pairing_port.set(prop_gateway_port.to_string());
                show_pairing.set(true);
            }
        }
        KeyCode::Char('e') if modifiers.contains(KeyModifiers::CONTROL) => {
            let mut m = messages.read().clone();
            let idx = selected_message_idx
                .get()
                .unwrap_or_else(|| m.len().saturating_sub(1));
            if let Some(msg) = m.get_mut(idx) {
                msg.toggle_collapse();
            }
            messages.set(m);
        }
        KeyCode::Char('y') if modifiers.contains(KeyModifiers::CONTROL) => {
            let m = messages.read();
            let idx = selected_message_idx
                .get()
                .unwrap_or_else(|| m.len().saturating_sub(1));
            if let Some(msg) = m.get(idx) {
                let content = msg.content.clone();
                drop(m);
                let copied = std::process::Command::new("wl-copy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        child
                            .stdin
                            .as_mut()
                            .unwrap()
                            .write_all(content.as_bytes())?;
                        child.wait().map(|_| ())
                    })
                    .or_else(|_| {
                        std::process::Command::new("xclip")
                            .args(["-selection", "clipboard"])
                            .stdin(std::process::Stdio::piped())
                            .spawn()
                            .and_then(|mut child| {
                                use std::io::Write;
                                child
                                    .stdin
                                    .as_mut()
                                    .unwrap()
                                    .write_all(content.as_bytes())?;
                                child.wait().map(|_| ())
                            })
                    });
                let mut m2 = messages.read().clone();
                if copied.is_ok() {
                    m2.push(DisplayMessage::success("✓ Copied to clipboard"));
                } else {
                    m2.push(DisplayMessage::error(
                        "Could not copy: install wl-copy or xclip",
                    ));
                }
                messages.set(m2);
            }
        }
        KeyCode::Char('s') if modifiers.contains(KeyModifiers::CONTROL) => {
            let m = messages.read();
            let idx = selected_message_idx
                .get()
                .unwrap_or_else(|| m.len().saturating_sub(1));
            if let Some(msg) = m.get(idx) {
                let content = msg.content.clone();
                drop(m);
                let dir = dirs::home_dir()
                    .map(|h| h.join(".rustyclaw").join("messages"))
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let _ = std::fs::create_dir_all(&dir);
                let filename = format!("{}.md", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
                let path = dir.join(&filename);
                let saved = std::fs::write(&path, &content);
                let mut m2 = messages.read().clone();
                if saved.is_ok() {
                    m2.push(DisplayMessage::success(format!(
                        "✓ Saved to ~/.rustyclaw/messages/{filename}"
                    )));
                } else {
                    m2.push(DisplayMessage::error("Could not save file"));
                }
                messages.set(m2);
            }
        }
        _ => {}
    }
}
