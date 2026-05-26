use iocraft::prelude::*;
use std::collections::HashMap;
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use crate::components::root::Root;
use crate::theme;
use crate::types::DisplayMessage;
use rustyclaw_view;

use crate::app::{GwEvent, UserInput};

fn display_message_from_gateway(
    message: rustyclaw_core::gateway::protocol::types::ChatMessage,
) -> DisplayMessage {
    match message.role.as_str() {
        "user" => DisplayMessage::user(message.display_content()),
        "assistant" => DisplayMessage::assistant(message.display_content()),
        "system" => DisplayMessage::system(message.display_content()),
        "tool" => DisplayMessage::tool_result(message.display_content()),
        _ => DisplayMessage::info(message.display_content()),
    }
}

#[derive(Default, Props)]
pub struct TuiRootProps {
    pub soul_name: String,
    pub model_label: String,
    /// Active provider ID (e.g. "openrouter") for provider-scoped completions.
    pub provider_id: String,
    pub hint: String,
    /// Whether the soul needs hatching (first run).
    pub needs_hatching: bool,
    /// Gateway host extracted from config gateway_url (pre-fills pairing dialog).
    pub gateway_host: String,
    /// Gateway port extracted from config gateway_url (pre-fills pairing dialog).
    pub gateway_port: String,
}

// ── Static channels ─────────────────────────────────────────────────
pub(super) static CHANNEL_RX: StdMutex<Option<sync_mpsc::Receiver<GwEvent>>> = StdMutex::new(None);
pub(super) static CHANNEL_TX: StdMutex<Option<sync_mpsc::Sender<UserInput>>> = StdMutex::new(None);

#[component]
pub fn TuiRoot(props: &TuiRootProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (width, height) = hooks.use_terminal_size();
    let mut system = hooks.use_context_mut::<SystemContext>();

    // ── Local UI state ──────────────────────────────────────────────
    let mut messages: State<Vec<DisplayMessage>> = hooks.use_state(Vec::new);
    let mut input_value = hooks.use_state(String::new);
    let mut input_cursor_offset = hooks.use_state(|| 0usize);
    let mut gw_status = hooks.use_state(|| rustyclaw_core::types::GatewayStatus::Connecting);
    let mut streaming = hooks.use_state(|| false);
    let mut stream_start: State<Option<Instant>> = hooks.use_state(|| None);
    let mut elapsed = hooks.use_state(String::new);
    let mut scroll_offset = hooks.use_state(|| 0i32);
    let mut spinner_tick = hooks.use_state(|| 0usize);
    let mut should_quit = hooks.use_state(|| false);
    let mut streaming_buf = hooks.use_state(String::new);
    let mut dynamic_model_label: State<Option<String>> = hooks.use_state(|| None);
    let mut dynamic_provider_id: State<Option<String>> = hooks.use_state(|| None);

    // ── Auth dialog state ───────────────────────────────────────────
    let mut show_auth_dialog = hooks.use_state(|| false);
    let mut auth_code = hooks.use_state(String::new);
    let mut auth_error = hooks.use_state(String::new);

    // ── Tool approval dialog state ──────────────────────────────────
    let mut show_tool_approval = hooks.use_state(|| false);
    let mut tool_approval_id = hooks.use_state(String::new);
    let mut tool_approval_name = hooks.use_state(String::new);
    let mut tool_approval_args = hooks.use_state(String::new);
    let mut tool_approval_selected = hooks.use_state(|| true); // true = Allow

    // ── Vault unlock dialog state ───────────────────────────────────
    let mut show_vault_unlock = hooks.use_state(|| false);
    let mut vault_password = hooks.use_state(String::new);
    let mut vault_error = hooks.use_state(String::new);

    // ── Hatching dialog state ───────────────────────────────────────
    // Start hidden; revealed after connection/auth succeeds so it
    // never competes with the TOTP dialog for screen space.
    let needs_hatching = props.needs_hatching;
    let mut show_hatching = hooks.use_state(|| false);
    let mut hatching_name_input: State<String> = hooks.use_state(String::new);
    let mut hatching_personality_input: State<String> = hooks.use_state(String::new);
    let mut hatching_focus_name: State<bool> = hooks.use_state(|| true);

    // ── Pairing dialog state ────────────────────────────────────────
    let mut show_pairing = hooks.use_state(|| false);
    let mut pairing_step: State<rustyclaw_view::PairingStep> =
        hooks.use_state(|| rustyclaw_view::PairingStep::ShowKey);
    let mut pairing_field: State<rustyclaw_view::PairingField> =
        hooks.use_state(|| rustyclaw_view::PairingField::Host);
    let mut pairing_public_key = hooks.use_state(String::new);
    let mut pairing_fingerprint = hooks.use_state(String::new);
    let mut pairing_fingerprint_art = hooks.use_state(String::new);
    let mut pairing_qr_ascii = hooks.use_state(String::new);
    let mut pairing_host = hooks.use_state(String::new);
    let mut pairing_port = hooks.use_state(|| "2222".to_string());
    let mut pairing_error = hooks.use_state(String::new);

    // ── User prompt dialog state ────────────────────────────────────
    let mut show_user_prompt = hooks.use_state(|| false);
    let mut user_prompt_id = hooks.use_state(String::new);
    let mut user_prompt_title = hooks.use_state(String::new);
    let mut user_prompt_desc = hooks.use_state(String::new);
    let mut user_prompt_input = hooks.use_state(String::new);
    let mut user_prompt_type: State<Option<rustyclaw_core::user_prompt_types::PromptType>> =
        hooks.use_state(|| None);
    let mut user_prompt_selected = hooks.use_state(|| 0usize);

    // ── Credential request dialog state ───────────────────────────────
    let mut show_credential_request = hooks.use_state(|| false);
    let mut credential_request_id = hooks.use_state(String::new);
    let mut credential_request_provider = hooks.use_state(String::new);
    let mut credential_request_secret_name = hooks.use_state(String::new);
    let mut credential_request_message = hooks.use_state(String::new);
    let mut credential_request_input = hooks.use_state(String::new);

    // ── Provider / model selection dialog state ─────────────────────
    let mut show_provider_selector = hooks.use_state(|| false);
    let mut provider_selector_items: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut provider_selector_ids: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut provider_selector_hints: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut provider_selector_cursor = hooks.use_state(|| 0usize);

    let mut show_api_key_dialog = hooks.use_state(|| false);
    let mut api_key_provider = hooks.use_state(String::new);
    let mut api_key_provider_display = hooks.use_state(String::new);
    let mut api_key_input = hooks.use_state(String::new);
    let mut api_key_help_url = hooks.use_state(String::new);
    let mut api_key_help_text = hooks.use_state(String::new);

    let mut show_device_flow = hooks.use_state(|| false);
    let mut device_flow_provider = hooks.use_state(String::new);
    let mut device_flow_url = hooks.use_state(String::new);
    let mut device_flow_code = hooks.use_state(String::new);
    let mut device_flow_tick = hooks.use_state(|| 0usize);
    let mut device_flow_browser_opened = hooks.use_state(|| false);

    let mut show_model_selector = hooks.use_state(|| false);
    let mut model_selector_provider = hooks.use_state(String::new);
    let mut model_selector_provider_display = hooks.use_state(String::new);
    let mut model_selector_models: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut model_selector_cursor = hooks.use_state(|| 0usize);
    let mut model_selector_loading = hooks.use_state(|| false);

    // ── Thread state (unified tasks + threads) ───────────────────────
    let mut threads: State<Vec<crate::action::ThreadInfo>> = hooks.use_state(Vec::new);
    let mut tab_focused = hooks.use_state(|| false);
    let mut tab_selected = hooks.use_state(|| 0usize);
    // Per-thread message cache so switching tabs restores prior
    // scrollback instead of clearing the chat (matches desktop client).
    let mut thread_messages_cache: State<HashMap<u64, Vec<DisplayMessage>>> =
        hooks.use_state(HashMap::new);
    let mut foreground_thread_id: State<Option<u64>> = hooks.use_state(|| None);

    // ── Command menu (slash-command completions) ────────────────────
    let mut command_completions: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut command_selected: State<Option<usize>> = hooks.use_state(|| None);
    let mut model_completion_provider: State<Option<String>> = hooks.use_state(|| None);
    let mut model_completion_models: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut model_completion_loading: State<Option<String>> = hooks.use_state(|| None);
    let mut prompt_attachments: State<Vec<rustyclaw_view::PromptAttachment>> =
        hooks.use_state(Vec::new);

    // ── Info dialog state (secrets / skills / tool permissions) ──────
    let mut show_secrets_dialog = hooks.use_state(|| false);
    let mut secrets_dialog_data: State<Vec<rustyclaw_view::SecretInfoData>> =
        hooks.use_state(Vec::new);
    let mut secrets_agent_access = hooks.use_state(|| false);
    let mut secrets_has_totp = hooks.use_state(|| false);
    let mut secrets_selected: State<Option<usize>> = hooks.use_state(|| Some(0));
    let mut secrets_scroll_offset = hooks.use_state(|| 0usize);
    // Add-secret inline input: 0 = off, 1 = entering name, 2 = entering value
    let mut secrets_add_step = hooks.use_state(|| 0u8);
    let mut secrets_add_name = hooks.use_state(String::new);
    let mut secrets_add_value = hooks.use_state(String::new);

    let mut show_skills_dialog = hooks.use_state(|| false);
    let mut skills_dialog_data: State<Vec<rustyclaw_view::SkillInfoData>> =
        hooks.use_state(Vec::new);
    let mut skills_selected: State<Option<usize>> = hooks.use_state(|| Some(0));

    // Details dialog overlay — shows extended `RequestDetails`
    // (URL, status, redacted headers, body excerpt, full cause
    // chain) attached to the most recent warning/error toast.
    // Opened with Ctrl-D when the latest message has details.
    let mut show_details_dialog = hooks.use_state(|| false);
    let mut details_dialog_text = hooks.use_state(String::new);
    let mut details_dialog_is_error = hooks.use_state(|| false);
    let mut details_dialog_scroll = hooks.use_state(|| 0usize);

    let mut show_tool_perms_dialog = hooks.use_state(|| false);
    let mut tool_perms_dialog_data: State<Vec<rustyclaw_view::ToolPermInfoData>> =
        hooks.use_state(Vec::new);
    let mut tool_perms_selected: State<Option<usize>> = hooks.use_state(|| Some(0));

    // Scroll offsets for interactive dialogs
    let mut skills_scroll_offset = hooks.use_state(|| 0usize);
    let mut tool_perms_scroll_offset = hooks.use_state(|| 0usize);

    // ── Channel access ──────────────────────────────────────────────
    let gw_rx: Arc<StdMutex<Option<sync_mpsc::Receiver<GwEvent>>>> =
        hooks.use_const(|| Arc::new(StdMutex::new(CHANNEL_RX.lock().unwrap().take())));
    let user_tx: Arc<StdMutex<Option<sync_mpsc::Sender<UserInput>>>> =
        hooks.use_const(|| Arc::new(StdMutex::new(CHANNEL_TX.lock().unwrap().take())));
    let prop_provider_id = props.provider_id.clone();
    let tx_for_model_completions = Arc::clone(&user_tx);

    // ── Poll gateway channel on a timer ─────────────────────────────
    hooks.use_future({
        let rx_handle = Arc::clone(&gw_rx);
        let tx_for_history = Arc::clone(&user_tx);
        async move {
            loop {
                smol::Timer::after(Duration::from_millis(30)).await;

                if let Ok(guard) = rx_handle.lock() {
                    if let Some(ref rx) = *guard {
                        while let Ok(ev) = rx.try_recv() {
                            match ev {
                                GwEvent::AuthChallenge => {
                                    // Gateway wants TOTP — show the dialog
                                    gw_status.set(rustyclaw_core::types::GatewayStatus::AuthRequired);
                                    show_auth_dialog.set(true);
                                    auth_code.set(String::new());
                                    auth_error.set(String::new());
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::info("Authentication required — enter TOTP code"));
                                    messages.set(m);
                                }
                                GwEvent::Disconnected(reason) => {
                                    gw_status.set(rustyclaw_core::types::GatewayStatus::Disconnected);
                                    show_auth_dialog.set(false);
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
                                    // Show hatching if needed (no TOTP required path).
                                    if needs_hatching && !show_hatching.get() {
                                        show_hatching.set(true);
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
                                    if needs_hatching && !show_hatching.get() {
                                        show_hatching.set(true);
                                    }
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
                                    elapsed.set(String::new());
                                    streaming_buf.set(String::new());
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
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::thinking("Thinking…"));
                                    messages.set(m);
                                }
                                GwEvent::ThinkingDelta => {
                                    // Thinking is ongoing — keep spinner alive
                                }
                                GwEvent::ThinkingEnd => {
                                    // Thinking done, but streaming may continue
                                    // with chunks. Don't clear streaming here.
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
                                GwEvent::ToolCall { id, name, arguments } => {
                                    let mut m = messages.read().clone();
                                    if m.last().map(|x| x.role == rustyclaw_core::types::MessageRole::Assistant).unwrap_or(false) {
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
                                GwEvent::ToolResult { id, name, result, is_error } => {
                                    let mut m = messages.read().clone();
                                    let mut matched = false;
                                    for msg in m.iter_mut().rev() {
                                        let before = msg.tool_calls.len();
                                        msg.set_tool_result(&id, result.clone(), is_error);
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
                                        );
                                        m.push(fallback);
                                    }
                                    messages.set(m);
                                }
                                GwEvent::ToolApprovalRequest { id, name, arguments } => {
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
                                    user_prompt_desc.set(
                                        prompt.description.clone().unwrap_or_default(),
                                    );
                                    user_prompt_input.set(String::new());
                                    user_prompt_type.set(Some(prompt.prompt_type.clone()));
                                    // Set default selection based on prompt type
                                    let default_sel = match &prompt.prompt_type {
                                        rustyclaw_core::user_prompt_types::PromptType::Select { default, .. } => {
                                            default.unwrap_or(0)
                                        }
                                        rustyclaw_core::user_prompt_types::PromptType::Confirm { default } => {
                                            if *default { 0 } else { 1 }
                                        }
                                        _ => 0,
                                    };
                                    user_prompt_selected.set(default_sel);
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
                                GwEvent::CredentialRequest { id, provider, secret_name, message } => {
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
                                GwEvent::ShowSecrets { secrets, agent_access, has_totp } => {
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
                                    tracing::info!(
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
                                    threads.set(thread_list);
                                    // Keep local foreground in sync and request
                                    // authoritative history when gateway picks
                                    // a new foreground (including initial load).
                                    if foreground_id != previous_foreground {
                                        foreground_thread_id.set(foreground_id);
                                        if let Some(thread_id) = foreground_id {
                                            tracing::info!(
                                                thread_id,
                                                previous_foreground = ?previous_foreground,
                                                "TUI requesting thread history after ThreadsUpdate"
                                            );
                                            if let Ok(guard) = tx_for_history.lock() {
                                                if let Some(ref tx) = *guard {
                                                    let _ = tx.send(
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
                                            let mut cache =
                                                thread_messages_cache.read().clone();
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
                                    let cached = thread_messages_cache
                                        .read()
                                        .get(&thread_id)
                                        .cloned();
                                    let mut m = match cached {
                                        Some(prior) if !prior.is_empty() => prior,
                                        _ => {
                                            let mut seed = Vec::new();
                                            seed.push(DisplayMessage::info(format!(
                                                "Switched to thread (id: {})",
                                                thread_id
                                            )));
                                            if let Some(summary) = context_summary {
                                                seed.push(DisplayMessage::assistant(
                                                    format!(
                                                        "[Previous context]\n\n{}",
                                                        summary
                                                    ),
                                                ));
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
                                            let _ = tx.send(
                                                UserInput::RequestThreadHistory(thread_id),
                                            );
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
                                        tracing::info!(
                                            thread_id,
                                            incoming_messages = history.len(),
                                            foreground = ?foreground_thread_id.get(),
                                            "TUI thread history reply received"
                                        );
                                        let converted: Vec<DisplayMessage> =
                                            rustyclaw_view::convert_history(&history);
                                        tracing::info!(
                                            thread_id,
                                            converted_messages = converted.len(),
                                            "TUI thread history converted"
                                        );
                                        // Update the cache so a future
                                        // switch-back is also authoritative.
                                        let mut cache =
                                            thread_messages_cache.read().clone();
                                        if converted.is_empty() {
                                            cache.remove(&thread_id);
                                        } else {
                                            cache.insert(thread_id, converted.clone());
                                        }
                                        thread_messages_cache.set(cache);
                                        // Only replace the live view if this
                                        // reply is for the thread the user is
                                        // currently looking at.
                                        if foreground_thread_id.get()
                                            == Some(thread_id)
                                        {
                                            messages.set(converted);
                                        }
                                    }
                                }
                                GwEvent::ShowProviderSelector { providers, provider_ids, auth_hints } => {
                                    provider_selector_items.set(providers);
                                    provider_selector_ids.set(provider_ids);
                                    provider_selector_hints.set(auth_hints);
                                    provider_selector_cursor.set(0);
                                    show_provider_selector.set(true);
                                }
                                GwEvent::PromptApiKey { provider, provider_display, help_url, help_text } => {
                                    api_key_provider.set(provider);
                                    api_key_provider_display.set(provider_display);
                                    api_key_input.set(String::new());
                                    api_key_help_url.set(help_url);
                                    api_key_help_text.set(help_text);
                                    show_api_key_dialog.set(true);
                                }
                                GwEvent::DeviceFlowCode { provider, url, code } => {
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
                                GwEvent::FetchModelsLoading { provider, provider_display } => {
                                    model_selector_provider.set(provider);
                                    model_selector_provider_display.set(provider_display);
                                    model_selector_models.set(Vec::new());
                                    model_selector_cursor.set(0);
                                    model_selector_loading.set(true);
                                    show_model_selector.set(true);
                                }
                                GwEvent::ShowModelSelector { provider, provider_display, models } => {
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
                                            let filtered = rustyclaw_view::build_slash_completions(
                                                &provider,
                                                Some(&models),
                                                partial,
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
                                        "Successfully paired with gateway: {}", gateway_name
                                    )));
                                    messages.set(m);
                                }
                                GwEvent::PairingError(err) => {
                                    // Pairing failed — show error
                                    pairing_step.set(rustyclaw_view::PairingStep::EnterGateway);
                                    pairing_error.set(err.clone());
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::error(format!(
                                        "Pairing failed: {}", err
                                    )));
                                    messages.set(m);
                                }
                            }
                        }
                    }
                }

                // Update spinner and elapsed timer
                spinner_tick.set(spinner_tick.get().wrapping_add(1));

                // Animate device flow spinner
                if show_device_flow.get() {
                    device_flow_tick.set(device_flow_tick.get().wrapping_add(1));
                }

                // (hatching is handled synchronously by keyboard input)

                if let Some(start) = stream_start.get() {
                    let d = start.elapsed();
                    let secs = d.as_secs();
                    elapsed.set(if secs >= 60 {
                        format!("{}m {:02}s", secs / 60, secs % 60)
                    } else {
                        format!("{}.{}s", secs, d.subsec_millis() / 100)
                    });
                }
            }
        }
    });

    // ── Keyboard handling ───────────────────────────────────────────
    let tx_for_keys = Arc::clone(&user_tx);
    let prop_gateway_host = props.gateway_host.clone();
    let prop_gateway_port = if props.gateway_port.is_empty() {
        "2222".to_string()
    } else {
        props.gateway_port.clone()
    };
    hooks.use_terminal_events({
        move |event| match event {
            TerminalEvent::Key(KeyEvent { code, kind, modifiers, .. })
                if kind != KeyEventKind::Release =>
            {
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
                                "✓ Approved: {}", &*tool_approval_name.read()
                            )));
                            messages.set(m);
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::ToolApprovalResponse {
                                        id,
                                        approved: true,
                                    });
                                }
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            // Deny
                            let id = tool_approval_id.read().clone();
                            show_tool_approval.set(false);
                            let mut m = messages.read().clone();
                            m.push(DisplayMessage::warning(format!(
                                "✗ Denied: {}", &*tool_approval_name.read()
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
                                    "✓ Approved: {}", &*tool_approval_name.read()
                                )));
                            } else {
                                m.push(DisplayMessage::warning(format!(
                                    "✗ Denied: {}", &*tool_approval_name.read()
                                )));
                            }
                            messages.set(m);
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::ToolApprovalResponse {
                                        id,
                                        approved,
                                    });
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
                if show_hatching.get() {
                    match code {
                        KeyCode::Enter => {
                            let name = hatching_name_input.read().trim().to_string();
                            show_hatching.set(false);
                            if name.is_empty() {
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::info("Hatching skipped. You can customize SOUL.md manually."));
                                messages.set(m);
                            } else {
                                let personality = hatching_personality_input.read().trim().to_string();
                                let payload = if personality.is_empty() {
                                    name.clone()
                                } else {
                                    format!("{}\t{}", name, personality)
                                };
                                if let Ok(guard) = tx_for_keys.lock() {
                                    if let Some(ref tx) = *guard {
                                        let _ = tx.send(UserInput::HatchingComplete(payload));
                                    }
                                }
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::success(format!("Welcome, {}! SOUL.md saved.", name)));
                                messages.set(m);
                            }
                        }
                        KeyCode::Tab => {
                            hatching_focus_name.set(!hatching_focus_name.get());
                        }
                        KeyCode::Esc => {
                            show_hatching.set(false);
                            let mut m = messages.read().clone();
                            m.push(DisplayMessage::info("Hatching skipped. You can customize SOUL.md manually."));
                            messages.set(m);
                        }
                        KeyCode::Char(c) => {
                            if hatching_focus_name.get() {
                                let mut name = hatching_name_input.read().clone();
                                name.push(c);
                                hatching_name_input.set(name);
                            } else {
                                let mut p = hatching_personality_input.read().clone();
                                p.push(c);
                                hatching_personality_input.set(p);
                            }
                        }
                        KeyCode::Backspace => {
                            if hatching_focus_name.get() {
                                let mut name = hatching_name_input.read().clone();
                                name.pop();
                                hatching_name_input.set(name);
                            } else {
                                let mut p = hatching_personality_input.read().clone();
                                p.pop();
                                hatching_personality_input.set(p);
                            }
                        }
                        _ => {}
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
                            if let Some(rustyclaw_core::user_prompt_types::PromptType::Select { .. } | rustyclaw_core::user_prompt_types::PromptType::MultiSelect { .. }) = &prompt_type {
                                let current = user_prompt_selected.get();
                                if current > 0 {
                                    user_prompt_selected.set(current - 1);
                                }
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let Some(rustyclaw_core::user_prompt_types::PromptType::Select { options, .. } | rustyclaw_core::user_prompt_types::PromptType::MultiSelect { options, .. }) = &prompt_type {
                                let current = user_prompt_selected.get();
                                if current + 1 < options.len() {
                                    user_prompt_selected.set(current + 1);
                                }
                            }
                        }
                        // Left/Right for Confirm
                        KeyCode::Left | KeyCode::Right => {
                            if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) = prompt_type {
                                let current = user_prompt_selected.get();
                                user_prompt_selected.set(if current == 0 { 1 } else { 0 });
                            }
                        }
                        // Y/N shortcuts for Confirm
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) = prompt_type {
                                user_prompt_selected.set(0); // Yes
                            } else {
                                // Normal text input
                                let mut input = user_prompt_input.read().clone();
                                input.push(if code == KeyCode::Char('Y') { 'Y' } else { 'y' });
                                user_prompt_input.set(input);
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') => {
                            if let Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) = prompt_type {
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
                            if matches!(prompt_type, None | Some(rustyclaw_core::user_prompt_types::PromptType::TextInput { .. }) | Some(rustyclaw_core::user_prompt_types::PromptType::Form { .. })) {
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
                                Some(rustyclaw_core::user_prompt_types::PromptType::Select { options, .. }) => {
                                    let label = options.get(selected).map(|o| o.label.clone()).unwrap_or_default();
                                    (rustyclaw_core::user_prompt_types::PromptResponseValue::Selected(vec![label.clone()]), format!("→ {}", label))
                                }
                                Some(rustyclaw_core::user_prompt_types::PromptType::Confirm { .. }) => {
                                    let yes = selected == 0;
                                    (rustyclaw_core::user_prompt_types::PromptResponseValue::Confirm(yes), format!("→ {}", if yes { "Yes" } else { "No" }))
                                }
                                Some(rustyclaw_core::user_prompt_types::PromptType::MultiSelect { options, .. }) => {
                                    // TODO: track multiple selections properly
                                    let label = options.get(selected).map(|o| o.label.clone()).unwrap_or_default();
                                    (rustyclaw_core::user_prompt_types::PromptResponseValue::Selected(vec![label.clone()]), format!("→ {}", label))
                                }
                                _ => {
                                    (rustyclaw_core::user_prompt_types::PromptResponseValue::Text(input.clone()), format!("→ {}", input))
                                }
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
                                        let _ = tx.send(UserInput::CycleSecretPolicy { name, current_policy: policy });
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
                                    let current_pid = dynamic_provider_id.read().clone()
                                        .unwrap_or_else(|| prop_provider_id.clone());
                                    if partial.starts_with("model") {
                                        let loaded = model_completion_provider
                                            .read()
                                            .as_deref()
                                            == Some(current_pid.as_str());
                                        let loading = model_completion_loading
                                            .read()
                                            .as_deref()
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
                    }
                    KeyCode::Delete if !tab_focused.get() => {
                        let cursor = input_cursor_offset.get();
                        let mut text = input_value.read().clone();
                        if cursor < text.len() && text.is_char_boundary(cursor) {
                            text.remove(cursor);
                            input_value.set(text.clone());
                            let current_text = text;
                            if let Some(partial) = current_text.strip_prefix('/') {
                                let current_pid = dynamic_provider_id.read().clone()
                                    .unwrap_or_else(|| prop_provider_id.clone());
                                if partial.starts_with("model") {
                                    let loaded = model_completion_provider
                                        .read()
                                        .as_deref()
                                        == Some(current_pid.as_str());
                                    let loading = model_completion_loading
                                        .read()
                                        .as_deref()
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
                        let c = if modifiers.contains(KeyModifiers::SHIFT)
                            && c.is_ascii_lowercase()
                        {
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
                            let current_pid = dynamic_provider_id.read().clone()
                                .unwrap_or_else(|| prop_provider_id.clone());
                            if partial.starts_with("model") {
                                let loaded = model_completion_provider
                                    .read()
                                    .as_deref()
                                    == Some(current_pid.as_str());
                                let loading = model_completion_loading
                                    .read()
                                    .as_deref()
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
                                if live_provider_matches { Some(live_models.as_slice()) } else { None },
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
                                        let _ = tx.send(UserInput::Command(
                                            val.trim_start_matches('/').to_string(),
                                        ));
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
                                ClientKeyPair,
                                key_fingerprint,
                                format_fingerprint_art,
                                generate_pairing_qr_ascii,
                                PairingData,
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
                            pairing_host.set(prop_gateway_host.clone());
                            pairing_port.set(prop_gateway_port.clone());
                            show_pairing.set(true);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    });

    if should_quit.get() {
        system.exit();
    }

    // Auto-scroll to bottom when streaming
    if streaming.get() {
        scroll_offset.set(0);
    }

    // Gateway display
    let status = gw_status.get();
    let gw_icon = theme::gateway_icon(&status).to_string();
    let gw_label = status.label().to_string();
    let gw_color = Some(theme::gateway_color(&status));

    // Clone props into owned values so closures below don't borrow `props`.
    let prop_soul_name = props.soul_name.clone();
    let prop_model_label = props.model_label.clone();
    let prop_provider_id = props.provider_id.clone();
    let prop_hint = props.hint.clone();

    element! {
        Root(
            width: width,
            height: height,
            soul_name: prop_soul_name,
            model_label: dynamic_model_label.read().clone().unwrap_or_else(|| prop_model_label.clone()),
            gateway_icon: gw_icon,
            gateway_label: gw_label,
            gateway_color: gw_color,
            messages: messages.read().clone(),
            scroll_offset: scroll_offset.get(),
            command_completions: command_completions.read().clone(),
            command_selected: command_selected.get(),
            composer: rustyclaw_view::ComposerData {
                is_processing: streaming.get(),
                current_provider: dynamic_provider_id
                    .read()
                    .clone()
                    .or_else(|| Some(prop_provider_id.clone())),
                current_model: dynamic_model_label.read().clone(),
                attachments: prompt_attachments.read().clone(),
            },
            input_value: input_value.to_string(),
            input_cursor_offset: input_cursor_offset.get(),
            input_has_focus: !show_auth_dialog.get()
                && !show_tool_approval.get()
                && !show_vault_unlock.get()
                && !show_user_prompt.get()
                && !show_credential_request.get()
                && !show_secrets_dialog.get()
                && !show_skills_dialog.get()
                && !show_tool_perms_dialog.get()
                && !show_hatching.get()
                && !show_provider_selector.get()
                && !show_api_key_dialog.get()
                && !show_device_flow.get()
                && !show_model_selector.get()
                && !show_pairing.get()
                && !tab_focused.get(),
            on_change: move |_new_val: String| {},
            on_submit: move |_val: String| {
                // Submit handled by Enter key above
            },
            surface: rustyclaw_view::ChatSurfaceData {
                is_processing: false,
                is_thinking: false,
                is_streaming: streaming.get(),
                streaming_chunks: 0,
                streaming_bytes: 0,
                elapsed: Some(elapsed.to_string()),
                spinner_tick: spinner_tick.get(),
            },
            tab_data: {
                            let thread_refs = threads.read();
                            rustyclaw_view::TabBarData::from_gateway_threads(&thread_refs)
                        },
            tab_focused: tab_focused.get(),
            tab_selected: tab_selected.get(),
            hint: prop_hint.clone(),
            show_auth_dialog: show_auth_dialog.get(),
            auth_dialog: rustyclaw_view::AuthDialogData {
                code: auth_code.read().clone(),
                error: auth_error.read().clone(),
            },
            show_tool_approval: show_tool_approval.get(),
            tool_approval: rustyclaw_view::ToolApprovalData {
                id: tool_approval_id.read().clone(),
                name: tool_approval_name.read().clone(),
                arguments: tool_approval_args.read().clone(),
                selected_allow: tool_approval_selected.get(),
            },
            show_vault_unlock: show_vault_unlock.get(),
            vault_unlock: rustyclaw_view::VaultUnlockData {
                password_len: vault_password.read().len(),
                error: vault_error.read().clone(),
            },
            show_user_prompt: show_user_prompt.get(),
            user_prompt_title: user_prompt_title.read().clone(),
            user_prompt_desc: user_prompt_desc.read().clone(),
            user_prompt_input: user_prompt_input.read().clone(),
            user_prompt_type: user_prompt_type.read().clone(),
            user_prompt_selected: user_prompt_selected.get(),
            show_credential_request: show_credential_request.get(),
            credential_request: rustyclaw_view::CredentialRequestData {
                provider: credential_request_provider.read().clone(),
                secret_name: credential_request_secret_name.read().clone(),
                message: credential_request_message.read().clone(),
                input_len: credential_request_input.read().len(),
            },
            show_secrets_dialog: show_secrets_dialog.get(),
            secrets_data: secrets_dialog_data.read().clone(),
            secrets_agent_access: secrets_agent_access.get(),
            secrets_has_totp: secrets_has_totp.get(),
            secrets_selected: secrets_selected.get(),
            secrets_scroll_offset: secrets_scroll_offset.get(),
            secrets_add_step: secrets_add_step.get(),
            secrets_add_name: secrets_add_name.read().clone(),
            secrets_add_value: secrets_add_value.read().clone(),
            show_skills_dialog: show_skills_dialog.get(),
            skills_data: skills_dialog_data.read().clone(),
            skills_selected: skills_selected.get(),
            skills_scroll_offset: skills_scroll_offset.get(),
            show_details_dialog: show_details_dialog.get(),
            details_dialog_text: details_dialog_text.read().clone(),
            details_dialog_is_error: details_dialog_is_error.get(),
            details_dialog_scroll: details_dialog_scroll.get(),
            show_tool_perms_dialog: show_tool_perms_dialog.get(),
            tool_perms_data: tool_perms_dialog_data.read().clone(),
            tool_perms_selected: tool_perms_selected.get(),
            tool_perms_scroll_offset: tool_perms_scroll_offset.get(),
            show_hatching: show_hatching.get(),
            hatching_name_input: hatching_name_input.read().clone(),
            hatching_personality_input: hatching_personality_input.read().clone(),
            hatching_focus_name: hatching_focus_name.get(),
            show_provider_selector: show_provider_selector.get(),
            provider_selector: rustyclaw_view::ProviderSelectorData {
                providers: provider_selector_items
                    .read()
                    .iter()
                    .cloned()
                    .zip(provider_selector_ids.read().iter().cloned())
                    .zip(provider_selector_hints.read().iter().cloned())
                    .map(|((display_name, id), auth_hint)| rustyclaw_view::ProviderOptionData {
                        id,
                        display_name,
                        auth_hint,
                    })
                    .collect(),
                cursor: provider_selector_cursor.get(),
            },
            show_api_key_dialog: show_api_key_dialog.get(),
            api_key_dialog: rustyclaw_view::ApiKeyDialogData {
                provider: api_key_provider.read().clone(),
                provider_display: api_key_provider_display.read().clone(),
                input_len: api_key_input.read().len(),
                help_url: api_key_help_url.read().clone(),
                help_text: api_key_help_text.read().clone(),
            },
            show_device_flow: show_device_flow.get(),
            device_flow: rustyclaw_view::DeviceFlowData {
                url: device_flow_url.read().clone(),
                code: device_flow_code.read().clone(),
                // The TUI event flow currently only provides URL + code.
                // Provider-specific explanatory text is not sent on this path.
                message: None,
                browser_opened: device_flow_browser_opened.get(),
                tick: device_flow_tick.get(),
            },
            show_model_selector: show_model_selector.get(),
            model_selector: rustyclaw_view::ModelSelectorData {
                provider: model_selector_provider.read().clone(),
                provider_display: model_selector_provider_display.read().clone(),
                models: model_selector_models.read().clone(),
                cursor: model_selector_cursor.get(),
                loading: model_selector_loading.get(),
                spinner_tick: device_flow_tick.get(),
            },
            show_pairing: show_pairing.get(),
            pairing: rustyclaw_view::PairingDialogData {
                step: *pairing_step.read(),
                field: *pairing_field.read(),
                public_key: pairing_public_key.read().clone(),
                fingerprint: pairing_fingerprint.read().clone(),
                fingerprint_art: pairing_fingerprint_art.read().clone(),
                qr_ascii: pairing_qr_ascii.read().clone(),
                host: pairing_host.read().clone(),
                port: pairing_port.read().clone(),
                error: pairing_error.read().clone(),
            },
        )
    }
}
