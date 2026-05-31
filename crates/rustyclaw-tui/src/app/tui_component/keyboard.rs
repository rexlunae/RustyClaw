//! Keyboard event handling for the TUI root: dialog focus + dispatch.

use iocraft::prelude::*;
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};

use super::keyboard_normal;
use super::state;
use crate::app::UserInput;
use crate::types::DisplayMessage;

type UserTx = Arc<StdMutex<Option<sync_mpsc::Sender<UserInput>>>>;

/// Handle one terminal event (dialogs first, then normal-mode keys).
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_key_event(
    event: TerminalEvent,
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
    match event {
        TerminalEvent::Key(KeyEvent {
            code,
            kind,
            modifiers,
            ..
        }) if kind != KeyEventKind::Release => {
            // ── Auth dialog has focus when visible ───────────
            if show_auth_dialog.get() {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        should_quit.set(true);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::Quit);
                            }
                        }
                    }
                    KeyCode::Esc => {
                        // Cancel auth dialog
                        show_auth_dialog.set(false);
                        auth_code.set(String::new());
                        auth_error.set(String::new());
                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::info("Authentication cancelled."));
                        messages.set(m);
                        gw_status.set(rustyclaw_core::types::GatewayStatus::Disconnected);
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        let mut code_val = auth_code.read().clone();
                        if code_val.len() < 6 {
                            code_val.push(c);
                            auth_code.set(code_val);
                        }
                    }
                    KeyCode::Backspace => {
                        let mut code_val = auth_code.read().clone();
                        code_val.pop();
                        auth_code.set(code_val);
                    }
                    KeyCode::Enter => {
                        let code_val = auth_code.read().clone();
                        if code_val.len() == 6 {
                            // Submit the TOTP code — keep dialog open
                            // until Authenticated/Error arrives
                            auth_code.set(String::new());
                            auth_error.set("Verifying…".to_string());
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::AuthResponse(code_val));
                                }
                            }
                        }
                        // If < 6 digits, ignore Enter
                    }
                    _ => {}
                }
                return;
            }

            // ── Tool approval dialog ────────────────────────
            if show_tool_approval.get() {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        should_quit.set(true);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::Quit);
                            }
                        }
                    }
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        // Toggle between Allow / Deny
                        tool_approval_selected.set(!tool_approval_selected.get());
                    }
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        // Quick-approve
                        let id = tool_approval_id.read().clone();
                        show_tool_approval.set(false);
                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::success(format!(
                            "✓ Approved: {}",
                            &*tool_approval_name.read()
                        )));
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ =
                                    tx.send(UserInput::ToolApprovalResponse { id, approved: true });
                            }
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        // Deny
                        let id = tool_approval_id.read().clone();
                        show_tool_approval.set(false);
                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::warning(format!(
                            "✗ Denied: {}",
                            &*tool_approval_name.read()
                        )));
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::ToolApprovalResponse {
                                    id,
                                    approved: false,
                                });
                            }
                        }
                    }
                    KeyCode::Enter => {
                        let id = tool_approval_id.read().clone();
                        let approved = tool_approval_selected.get();
                        show_tool_approval.set(false);
                        let mut m = messages.read().clone();
                        if approved {
                            m.push(DisplayMessage::success(format!(
                                "✓ Approved: {}",
                                &*tool_approval_name.read()
                            )));
                        } else {
                            m.push(DisplayMessage::warning(format!(
                                "✗ Denied: {}",
                                &*tool_approval_name.read()
                            )));
                        }
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::ToolApprovalResponse { id, approved });
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Provider selector dialog ────────────────────
            if show_provider_selector.get() {
                match code {
                    KeyCode::Esc => {
                        show_provider_selector.set(false);
                    }
                    KeyCode::Up => {
                        let cur = provider_selector_cursor.get();
                        if cur > 0 {
                            provider_selector_cursor.set(cur - 1);
                        }
                    }
                    KeyCode::Down => {
                        let cur = provider_selector_cursor.get();
                        let len = provider_selector_ids.read().len();
                        if cur + 1 < len {
                            provider_selector_cursor.set(cur + 1);
                        }
                    }
                    KeyCode::Enter => {
                        let cur = provider_selector_cursor.get();
                        let ids = provider_selector_ids.read().clone();
                        if let Some(id) = ids.get(cur) {
                            show_provider_selector.set(false);
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::SelectProvider(id.clone()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── API key dialog ──────────────────────────────
            if show_api_key_dialog.get() {
                match code {
                    KeyCode::Esc => {
                        show_api_key_dialog.set(false);
                        api_key_input.set(String::new());
                    }
                    KeyCode::Char(c) => {
                        let mut val = api_key_input.read().clone();
                        val.push(c);
                        api_key_input.set(val);
                    }
                    KeyCode::Backspace => {
                        let mut val = api_key_input.read().clone();
                        val.pop();
                        api_key_input.set(val);
                    }
                    KeyCode::Enter => {
                        let key_val = api_key_input.read().clone();
                        if !key_val.is_empty() {
                            let provider = api_key_provider.read().clone();
                            show_api_key_dialog.set(false);
                            api_key_input.set(String::new());
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::SubmitApiKey {
                                        provider,
                                        key: key_val,
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Device flow dialog ──────────────────────────
            if show_device_flow.get() {
                match code {
                    KeyCode::Esc => {
                        show_device_flow.set(false);
                        device_flow_browser_opened.set(false);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::CancelProviderFlow);
                            }
                        }
                    }
                    KeyCode::Enter => {
                        // Open (or re-open) the URL in the browser
                        let url = device_flow_url.read().clone();
                        if !url.is_empty() {
                            crate::components::device_flow_dialog::open_url_in_browser(&url);
                            device_flow_browser_opened.set(true);
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Model selector dialog ───────────────────────
            if show_model_selector.get() {
                match code {
                    KeyCode::Esc => {
                        show_model_selector.set(false);
                    }
                    KeyCode::Up if !model_selector_loading.get() => {
                        let cur = model_selector_cursor.get();
                        if cur > 0 {
                            model_selector_cursor.set(cur - 1);
                        }
                    }
                    KeyCode::Down if !model_selector_loading.get() => {
                        let cur = model_selector_cursor.get();
                        let len = model_selector_models.read().len();
                        if cur + 1 < len {
                            model_selector_cursor.set(cur + 1);
                        }
                    }
                    KeyCode::Enter if !model_selector_loading.get() => {
                        let cur = model_selector_cursor.get();
                        let models = model_selector_models.read().clone();
                        if let Some(model) = models.get(cur) {
                            let provider = model_selector_provider.read().clone();
                            show_model_selector.set(false);
                            if let Ok(guard) = tx_for_keys.lock()
                                && let Some(ref tx) = *guard
                            {
                                let _ = tx.send(UserInput::SelectModel {
                                    provider,
                                    model: model.clone(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Vault unlock dialog ─────────────────────────
            if show_vault_unlock.get() {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        should_quit.set(true);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::Quit);
                            }
                        }
                    }
                    KeyCode::Esc => {
                        show_vault_unlock.set(false);
                        vault_password.set(String::new());
                        vault_error.set(String::new());
                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::info("Vault unlock cancelled."));
                        messages.set(m);
                    }
                    KeyCode::Char(c) => {
                        let mut pw = vault_password.read().clone();
                        pw.push(c);
                        vault_password.set(pw);
                    }
                    KeyCode::Backspace => {
                        let mut pw = vault_password.read().clone();
                        pw.pop();
                        vault_password.set(pw);
                    }
                    KeyCode::Enter => {
                        let pw = vault_password.read().clone();
                        if !pw.is_empty() {
                            vault_password.set(String::new());
                            vault_error.set("Unlocking…".to_string());
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::VaultUnlock(pw));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Pairing dialog ──────────────────────────────
            if show_pairing.get() {
                use rustyclaw_view::{PairingField, PairingStep};
                let step = *pairing_step.read();
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        should_quit.set(true);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::Quit);
                            }
                        }
                    }
                    KeyCode::Esc => {
                        match step {
                            PairingStep::ShowKey => {
                                // Cancel — close dialog
                                show_pairing.set(false);
                            }
                            PairingStep::EnterGateway => {
                                // Go back to ShowKey
                                pairing_step.set(PairingStep::ShowKey);
                                pairing_error.set(String::new());
                            }
                            PairingStep::Connecting => {
                                // Cancel connection
                                pairing_step.set(PairingStep::EnterGateway);
                            }
                            PairingStep::Complete => {
                                show_pairing.set(false);
                            }
                        }
                    }
                    KeyCode::Enter => {
                        match step {
                            PairingStep::ShowKey => {
                                // Proceed to EnterGateway
                                pairing_step.set(PairingStep::EnterGateway);
                            }
                            PairingStep::EnterGateway => {
                                let host = pairing_host.read().clone();
                                let port_str = pairing_port.read().clone();
                                let public_key = pairing_public_key.read().clone();

                                if host.is_empty() {
                                    pairing_error.set("Host is required".to_string());
                                } else {
                                    let port: u16 = port_str.parse().unwrap_or(2222);
                                    pairing_step.set(PairingStep::Connecting);

                                    // Send connection request to async handler
                                    if let Ok(guard) = tx_for_keys.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::PairingConnect {
                                                host,
                                                port,
                                                public_key,
                                            });
                                        }
                                    }
                                }
                            }
                            PairingStep::Connecting => {
                                // Wait for connection
                            }
                            PairingStep::Complete => {
                                show_pairing.set(false);
                            }
                        }
                    }
                    KeyCode::Tab if step == PairingStep::EnterGateway => {
                        // Toggle between host and port fields
                        let field = *pairing_field.read();
                        pairing_field.set(match field {
                            PairingField::Host => PairingField::Port,
                            PairingField::Port => PairingField::Host,
                        });
                    }
                    KeyCode::Char(c) if step == PairingStep::EnterGateway => {
                        let field = *pairing_field.read();
                        match field {
                            PairingField::Host => {
                                let mut h = pairing_host.read().clone();
                                h.push(c);
                                pairing_host.set(h);
                            }
                            PairingField::Port => {
                                if c.is_ascii_digit() {
                                    let mut p = pairing_port.read().clone();
                                    p.push(c);
                                    pairing_port.set(p);
                                }
                            }
                        }
                        pairing_error.set(String::new());
                    }
                    KeyCode::Backspace if step == PairingStep::EnterGateway => {
                        let field = *pairing_field.read();
                        match field {
                            PairingField::Host => {
                                let mut h = pairing_host.read().clone();
                                h.pop();
                                pairing_host.set(h);
                            }
                            PairingField::Port => {
                                let mut p = pairing_port.read().clone();
                                p.pop();
                                pairing_port.set(p);
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Hatching dialog ─────────────────────────────
            if hatching_dialog.read().visible {
                let key = match code {
                    KeyCode::Enter => Some(rustyclaw_view::HatchingKey::Enter),
                    KeyCode::Tab => Some(rustyclaw_view::HatchingKey::Tab),
                    KeyCode::Esc => Some(rustyclaw_view::HatchingKey::Escape),
                    KeyCode::Backspace => Some(rustyclaw_view::HatchingKey::Backspace),
                    KeyCode::Char(c) => {
                        // Some terminals deliver Shift+<letter> as the lowercase char
                        // plus SHIFT modifier instead of pre-shifting the codepoint.
                        let c = if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase()
                        {
                            c.to_ascii_uppercase()
                        } else {
                            c
                        };
                        Some(rustyclaw_view::HatchingKey::Char(c))
                    }
                    _ => None,
                };

                if let Some(key) = key {
                    let mut hatching = hatching_dialog.read().clone();
                    match hatching.handle_key(key) {
                        rustyclaw_view::HatchingEvent::Completed(result) => {
                            let payload = result.as_payload();
                            let name = result.name.clone();
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::HatchingComplete(payload));
                                }
                            }
                            let mut m = messages.read().clone();
                            m.push(DisplayMessage::success(format!(
                                "Welcome, {}! SOUL.md saved.",
                                name
                            )));
                            messages.set(m);
                        }
                        rustyclaw_view::HatchingEvent::Skipped => {
                            let mut m = messages.read().clone();
                            m.push(DisplayMessage::info(
                                "Hatching skipped. You can customize SOUL.md manually.",
                            ));
                            messages.set(m);
                        }
                        rustyclaw_view::HatchingEvent::Updated
                        | rustyclaw_view::HatchingEvent::Ignored => {}
                    }
                    hatching_dialog.set(hatching);
                }
                return;
            }

            // ── User prompt dialog ──────────────────────────
            if show_user_prompt.get() {
                let prompt_type = user_prompt_type.read().clone();
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        should_quit.set(true);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::Quit);
                            }
                        }
                    }
                    KeyCode::Esc => {
                        let id = user_prompt_id.read().clone();
                        show_user_prompt.set(false);
                        user_prompt_input.set(String::new());
                        user_prompt_type.set(None);
                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::info("Prompt dismissed."));
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::UserPromptResponse {
                                        id,
                                        dismissed: true,
                                        value: rustyclaw_core::user_prompt_types::PromptResponseValue::Text(String::new()),
                                    });
                            }
                        }
                    }
                    // Navigation for Select/MultiSelect
                    KeyCode::Up | KeyCode::Char('k') => {
                        if let Some(
                            rustyclaw_core::user_prompt_types::PromptType::Select { .. }
                            | rustyclaw_core::user_prompt_types::PromptType::MultiSelect { .. },
                        ) = &prompt_type
                        {
                            let current = user_prompt_selected.get();
                            if current > 0 {
                                user_prompt_selected.set(current - 1);
                            }
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if let Some(
                            rustyclaw_core::user_prompt_types::PromptType::Select {
                                options, ..
                            }
                            | rustyclaw_core::user_prompt_types::PromptType::MultiSelect {
                                options,
                                ..
                            },
                        ) = &prompt_type
                        {
                            let current = user_prompt_selected.get();
                            if current + 1 < options.len() {
                                user_prompt_selected.set(current + 1);
                            }
                        }
                    }
                    // Left/Right for Confirm
                    KeyCode::Left | KeyCode::Right => {
                        if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm {
                            ..
                        }) = prompt_type
                        {
                            let current = user_prompt_selected.get();
                            user_prompt_selected.set(if current == 0 { 1 } else { 0 });
                        }
                    }
                    // Y/N shortcuts for Confirm
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm {
                            ..
                        }) = prompt_type
                        {
                            user_prompt_selected.set(0); // Yes
                        } else {
                            // Normal text input
                            let mut input = user_prompt_input.read().clone();
                            input.push(if code == KeyCode::Char('Y') { 'Y' } else { 'y' });
                            user_prompt_input.set(input);
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm {
                            ..
                        }) = prompt_type
                        {
                            user_prompt_selected.set(1); // No
                        } else {
                            // Normal text input
                            let mut input = user_prompt_input.read().clone();
                            input.push(if code == KeyCode::Char('N') { 'N' } else { 'n' });
                            user_prompt_input.set(input);
                        }
                    }
                    KeyCode::Char(c) => {
                        // Only for TextInput types
                        if matches!(
                            prompt_type,
                            None | Some(
                                rustyclaw_core::user_prompt_types::PromptType::TextInput { .. }
                            ) | Some(rustyclaw_core::user_prompt_types::PromptType::Form { .. })
                        ) {
                            let mut input = user_prompt_input.read().clone();
                            input.push(c);
                            user_prompt_input.set(input);
                        }
                    }
                    KeyCode::Backspace => {
                        let mut input = user_prompt_input.read().clone();
                        input.pop();
                        user_prompt_input.set(input);
                    }
                    KeyCode::Enter => {
                        let id = user_prompt_id.read().clone();
                        let input = user_prompt_input.read().clone();
                        let selected = user_prompt_selected.get();
                        show_user_prompt.set(false);
                        user_prompt_input.set(String::new());
                        user_prompt_type.set(None);

                        // Build response based on prompt type
                        let (value, display) = match &prompt_type {
                            Some(rustyclaw_core::user_prompt_types::PromptType::Select {
                                options,
                                ..
                            }) => {
                                let label = options
                                    .get(selected)
                                    .map(|o| o.label.clone())
                                    .unwrap_or_default();
                                (rustyclaw_core::user_prompt_types::PromptResponseValue::Selected(vec![label.clone()]), format!("→ {}", label))
                            }
                            Some(rustyclaw_core::user_prompt_types::PromptType::Confirm {
                                ..
                            }) => {
                                let yes = selected == 0;
                                (
                                    rustyclaw_core::user_prompt_types::PromptResponseValue::Confirm(
                                        yes,
                                    ),
                                    format!("→ {}", if yes { "Yes" } else { "No" }),
                                )
                            }
                            Some(rustyclaw_core::user_prompt_types::PromptType::MultiSelect {
                                options,
                                ..
                            }) => {
                                // TODO: track multiple selections properly
                                let label = options
                                    .get(selected)
                                    .map(|o| o.label.clone())
                                    .unwrap_or_default();
                                (rustyclaw_core::user_prompt_types::PromptResponseValue::Selected(vec![label.clone()]), format!("→ {}", label))
                            }
                            _ => (
                                rustyclaw_core::user_prompt_types::PromptResponseValue::Text(
                                    input.clone(),
                                ),
                                format!("→ {}", input),
                            ),
                        };

                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::user(display));
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::UserPromptResponse {
                                    id,
                                    dismissed: false,
                                    value,
                                });
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            // ── Credential request dialog ────────────────────
            if show_credential_request.get() {
                match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        should_quit.set(true);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::Quit);
                            }
                        }
                    }
                    KeyCode::Esc => {
                        let id = credential_request_id.read().clone();
                        show_credential_request.set(false);
                        credential_request_input.set(String::new());
                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::info("Credential request dismissed."));
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::CredentialResponse {
                                    id,
                                    dismissed: true,
                                    value: None,
                                });
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        let mut input = credential_request_input.read().clone();
                        input.push(c);
                        credential_request_input.set(input);
                    }
                    KeyCode::Backspace => {
                        let mut input = credential_request_input.read().clone();
                        input.pop();
                        credential_request_input.set(input);
                    }
                    KeyCode::Enter => {
                        let id = credential_request_id.read().clone();
                        let input = credential_request_input.read().clone();
                        show_credential_request.set(false);
                        credential_request_input.set(String::new());

                        let mut m = messages.read().clone();
                        m.push(DisplayMessage::user("→ [credential provided]".to_string()));
                        messages.set(m);
                        if let Ok(guard) = tx_for_keys.lock() {
                            if let Some(ref tx) = *guard {
                                let _ = tx.send(UserInput::CredentialResponse {
                                    id,
                                    dismissed: false,
                                    value: Some(input),
                                });
                            }
                        }
                    }
                    _ => {}
                }
                return;
            }

            keyboard_normal::handle_normal_key(
                code,
                modifiers,
                ui,
                tx_for_keys,
                tx_for_model_completions,
                prop_provider_id,
                prop_gateway_host,
                prop_gateway_port,
            );
        }
        _ => {}
    }
}
