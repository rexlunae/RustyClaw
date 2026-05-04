use iocraft::prelude::*;
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use crate::components::root::Root;
use crate::theme;
use crate::types::DisplayMessage;

use crate::app::{GwEvent, UserInput};

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

/// Build the slash-command autocomplete list for a `/{partial}` input.
///
/// Merges the static command names for `provider` with any live-fetched
/// model IDs (deduplicating in case the live list overlaps the static
/// fallback), then filters by the user's partial.  When `live_models` is
/// `None`, only the static list is used.
fn build_slash_completions(
    provider: &str,
    live_models: Option<&[String]>,
    partial: &str,
) -> Vec<String> {
    let mut names = rustyclaw_core::commands::command_names_for_provider(provider);
    if let Some(live) = live_models {
        let mut seen: std::collections::HashSet<String> = names.iter().cloned().collect();
        for model in live {
            let entry = format!("model {}", model);
            if seen.insert(entry.clone()) {
                names.push(entry);
            }
        }
    }
    names
        .into_iter()
        .filter(|c| c.starts_with(partial))
        .collect()
}

#[component]
pub fn TuiRoot(props: &TuiRootProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (width, height) = hooks.use_terminal_size();
    let mut system = hooks.use_context_mut::<SystemContext>();

    // ── Local UI state ──────────────────────────────────────────────
    let mut messages: State<Vec<DisplayMessage>> = hooks.use_state(Vec::new);
    let mut input_value = hooks.use_state(String::new);
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
    let mut show_hatching = hooks.use_state(|| props.needs_hatching);
    let mut hatching_state: State<crate::components::hatching_dialog::HatchState> =
        hooks.use_state(|| crate::components::hatching_dialog::HatchState::Egg);
    let mut hatching_tick = hooks.use_state(|| 0usize);
    let mut hatching_pending = hooks.use_state(|| false); // True when waiting for hatching response

    // ── Pairing dialog state ────────────────────────────────────────
    let mut show_pairing = hooks.use_state(|| false);
    let mut pairing_step: State<crate::components::pairing_dialog::PairingStep> =
        hooks.use_state(|| crate::components::pairing_dialog::PairingStep::ShowKey);
    let mut pairing_field: State<crate::components::pairing_dialog::PairingField> =
        hooks.use_state(|| crate::components::pairing_dialog::PairingField::Host);
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
    let mut sidebar_focused = hooks.use_state(|| false);
    let mut sidebar_selected = hooks.use_state(|| 0usize);

    // ── Command menu (slash-command completions) ────────────────────
    let mut command_completions: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut command_selected: State<Option<usize>> = hooks.use_state(|| None);
    let mut model_completion_provider: State<Option<String>> = hooks.use_state(|| None);
    let mut model_completion_models: State<Vec<String>> = hooks.use_state(Vec::new);
    let mut model_completion_loading: State<Option<String>> = hooks.use_state(|| None);

    // ── Info dialog state (secrets / skills / tool permissions) ──────
    let mut show_secrets_dialog = hooks.use_state(|| false);
    let mut secrets_dialog_data: State<Vec<crate::components::secrets_dialog::SecretInfo>> =
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
    let mut skills_dialog_data: State<Vec<crate::components::skills_dialog::SkillInfo>> =
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
    let mut tool_perms_dialog_data: State<Vec<crate::components::tool_perms_dialog::ToolPermInfo>> =
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

    // ── Poll gateway channel on a timer ─────────────────────────────
    hooks.use_future({
        let rx_handle = Arc::clone(&gw_rx);
        let tx_for_history = Arc::clone(&user_tx);
        let tx_for_ticker = Arc::clone(&user_tx);
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
                                GwEvent::Authenticated => {
                                    gw_status.set(rustyclaw_core::types::GatewayStatus::Connected);
                                    show_auth_dialog.set(false);
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::success("Authenticated"));
                                    messages.set(m);
                                    // Request initial thread list
                                    if let Ok(guard) = tx_for_history.lock() {
                                        if let Some(ref tx) = *guard {
                                            let _ = tx.send(UserInput::RefreshThreads);
                                        }
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

                                    // Check if this was a hatching response
                                    if hatching_pending.get() {
                                        hatching_pending.set(false);
                                        // Set hatching state to Awakened with the identity
                                        hatching_state.set(
                                            crate::components::hatching_dialog::HatchState::Awakened {
                                                identity: completed_text.clone(),
                                            }
                                        );
                                        // Save to SOUL.md
                                        if let Ok(guard) = tx_for_history.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send(UserInput::HatchingComplete(completed_text));
                                            }
                                        }
                                    } else if !completed_text.is_empty() {
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
                                    // Refresh task list after response (not for hatching)
                                    if !hatching_pending.get() {
                                        if let Ok(guard) = tx_for_history.lock() {
                                            if let Some(ref tx) = *guard {
                                                let _ = tx.send(UserInput::RefreshTasks);
                                            }
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
                                GwEvent::ToolCall { name, arguments } => {
                                    let msg = if name == "ask_user" {
                                        // Don't show raw JSON args for ask_user — the dialog handles it
                                        format!("🔧 {} — preparing question…", name)
                                    } else {
                                        // Pretty-print JSON arguments if possible
                                        let pretty = serde_json::from_str::<serde_json::Value>(&arguments)
                                            .ok()
                                            .and_then(|v| serde_json::to_string_pretty(&v).ok())
                                            .unwrap_or(arguments);
                                        format!("🔧 {}\n{}", name, pretty)
                                    };
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::tool_call(msg));
                                    messages.set(m);
                                }
                                GwEvent::ToolResult { result } => {
                                    let preview = if result.len() > 200 {
                                        format!("{}…", &result[..200])
                                    } else {
                                        result
                                    };
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::tool_result(preview));
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
                                    threads: thread_list,
                                    foreground_id: _,
                                } => {
                                    threads.set(thread_list);
                                    // Update sidebar_selected to stay in bounds
                                    let count = threads.read().len();
                                    if count > 0 && sidebar_selected.get() >= count {
                                        sidebar_selected.set(count - 1);
                                    }
                                }
                                GwEvent::ThreadSwitched {
                                    thread_id,
                                    context_summary,
                                } => {
                                    // Clear messages for the new thread
                                    let mut m = Vec::new();
                                    m.push(DisplayMessage::info(format!(
                                        "Switched to thread (id: {})",
                                        thread_id
                                    )));
                                    // Show context summary if available
                                    if let Some(summary) = context_summary {
                                        m.push(DisplayMessage::assistant(format!(
                                            "[Previous context]\n\n{}",
                                            summary
                                        )));
                                    }
                                    messages.set(m);
                                    // Unfocus sidebar after switch
                                    sidebar_focused.set(false);
                                }
                                GwEvent::HatchingResponse(_identity) => {
                                    // Hatching response is handled via ResponseDone
                                    // since it comes through as streaming chunks.
                                    // This event is currently unused but defined for
                                    // potential future direct gateway hatching support.
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
                                            let filtered = build_slash_completions(
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
                                    pairing_step.set(crate::components::pairing_dialog::PairingStep::Complete);
                                    pairing_error.set(String::new());
                                    let mut m = messages.read().clone();
                                    m.push(DisplayMessage::success(format!(
                                        "Successfully paired with gateway: {}", gateway_name
                                    )));
                                    messages.set(m);
                                }
                                GwEvent::PairingError(err) => {
                                    // Pairing failed — show error
                                    pairing_step.set(crate::components::pairing_dialog::PairingStep::EnterGateway);
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

                // Animate hatching sequence (advance every 8 ticks ≈ 2 seconds)
                if show_hatching.get() && !hatching_pending.get() {
                    let tick = hatching_tick.get().wrapping_add(1);
                    hatching_tick.set(tick);
                    if tick % 8 == 0 {
                        let mut state = hatching_state.read().clone();
                        let should_connect = state.advance();
                        hatching_state.set(state);
                        if should_connect {
                            // Send hatching request to gateway
                            hatching_pending.set(true);
                            if let Ok(guard) = tx_for_ticker.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::HatchingRequest);
                                }
                            }
                        }
                    }
                }

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
                    use crate::components::pairing_dialog::{PairingStep, PairingField};
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
                            // If awakened, close the dialog
                            let state = hatching_state.read().clone();
                            if matches!(
                                state,
                                crate::components::hatching_dialog::HatchState::Awakened { .. }
                            ) {
                                show_hatching.set(false);
                                let mut m = messages.read().clone();
                                m.push(DisplayMessage::success("Identity established! Welcome to RustyClaw."));
                                messages.set(m);
                            }
                        }
                        KeyCode::Esc => {
                            // Allow skipping hatching
                            show_hatching.set(false);
                            let mut m = messages.read().clone();
                            m.push(DisplayMessage::info("Hatching skipped. You can customize SOUL.md manually."));
                            messages.set(m);
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
                                let name = secret.name.clone();
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
                                let name = secret.name.clone();
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
                let menu_open = !command_completions.read().is_empty();

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
                        // Cycle forward through completions
                        let completions = command_completions.read().clone();
                        let new_idx = match command_selected.get() {
                            Some(i) => (i + 1) % completions.len(),
                            None => 0,
                        };
                        command_selected.set(Some(new_idx));
                        // Apply the selected completion into the input
                        if let Some(cmd) = completions.get(new_idx) {
                            input_value.set(format!("/{}", cmd));
                        }
                    }
                    KeyCode::BackTab if menu_open => {
                        // Cycle backward through completions
                        let completions = command_completions.read().clone();
                        let new_idx = match command_selected.get() {
                            Some(0) | None => completions.len().saturating_sub(1),
                            Some(i) => i - 1,
                        };
                        command_selected.set(Some(new_idx));
                        if let Some(cmd) = completions.get(new_idx) {
                            input_value.set(format!("/{}", cmd));
                        }
                    }
                    KeyCode::Up if menu_open => {
                        // Navigate up through completions
                        let completions = command_completions.read().clone();
                        let new_idx = match command_selected.get() {
                            Some(0) | None => completions.len().saturating_sub(1),
                            Some(i) => i - 1,
                        };
                        command_selected.set(Some(new_idx));
                        if let Some(cmd) = completions.get(new_idx) {
                            input_value.set(format!("/{}", cmd));
                        }
                    }
                    KeyCode::Down if menu_open => {
                        // Navigate down through completions
                        let completions = command_completions.read().clone();
                        let new_idx = match command_selected.get() {
                            Some(i) => (i + 1) % completions.len(),
                            None => 0,
                        };
                        command_selected.set(Some(new_idx));
                        if let Some(cmd) = completions.get(new_idx) {
                            input_value.set(format!("/{}", cmd));
                        }
                    }
                    KeyCode::Esc if menu_open => {
                        // Close the command menu
                        command_completions.set(Vec::new());
                        command_selected.set(None);
                    }
                    KeyCode::Enter if sidebar_focused.get() => {
                        let thread_list = threads.read().clone();
                        if let Some(thread) = thread_list.get(sidebar_selected.get()) {
                            // Send thread switch request
                            if let Ok(guard) = tx_for_keys.lock() {
                                if let Some(ref tx) = *guard {
                                    let _ = tx.send(UserInput::ThreadSwitch(thread.id));
                                }
                            }
                        }
                        // Return focus to input after selection
                        sidebar_focused.set(false);
                    }
                    KeyCode::Enter => {
                        let val = input_value.to_string();
                        if !val.is_empty() {
                            input_value.set(String::new());
                            // Close command menu
                            command_completions.set(Vec::new());
                            command_selected.set(None);
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
                    // Tab toggles sidebar focus when command menu is not open
                    KeyCode::Tab if !menu_open => {
                        sidebar_focused.set(!sidebar_focused.get());
                    }
                    // Sidebar navigation when focused
                    KeyCode::Up if sidebar_focused.get() => {
                        let thread_count = threads.read().len();
                        if thread_count > 0 {
                            let current = sidebar_selected.get();
                            sidebar_selected.set(current.saturating_sub(1));
                        }
                    }
                    KeyCode::Down if sidebar_focused.get() => {
                        let thread_count = threads.read().len();
                        if thread_count > 0 {
                            let current = sidebar_selected.get();
                            sidebar_selected.set((current + 1).min(thread_count - 1));
                        }
                    }
                    KeyCode::Esc if sidebar_focused.get() => {
                        // Escape returns focus to input
                        sidebar_focused.set(false);
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
                            pairing_step.set(crate::components::pairing_dialog::PairingStep::ShowKey);
                            pairing_field.set(crate::components::pairing_dialog::PairingField::Host);
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
    let prop_soul_name_for_hatching = props.soul_name.clone();
    let prop_model_label = props.model_label.clone();
    let prop_provider_id = props.provider_id.clone();
    let prop_hint = props.hint.clone();
    let tx_for_model_completions = Arc::clone(&user_tx);

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
            input_value: input_value.to_string(),
            input_has_focus: !show_auth_dialog.get()
                && !show_tool_approval.get()
                && !show_vault_unlock.get()
                && !show_user_prompt.get()
                && !show_secrets_dialog.get()
                && !show_skills_dialog.get()
                && !show_tool_perms_dialog.get()
                && !show_hatching.get()
                && !show_provider_selector.get()
                && !show_api_key_dialog.get()
                && !show_device_flow.get()
                && !show_model_selector.get()
                && !show_pairing.get()
                && !sidebar_focused.get(),
            on_change: move |new_val: String| {
                input_value.set(new_val.clone());
                // Update slash-command completions
                if let Some(partial) = new_val.strip_prefix('/') {
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
                    let filtered = build_slash_completions(
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
            },
            on_submit: move |_val: String| {
                // Submit handled by Enter key above
            },
            task_text: if streaming.get() { "Streaming…".to_string() } else { "Idle".to_string() },
            streaming: streaming.get(),
            elapsed: elapsed.to_string(),
            threads: threads.read().clone(),
            sidebar_focused: sidebar_focused.get(),
            sidebar_selected: sidebar_selected.get(),
            hint: prop_hint.clone(),
            spinner_tick: spinner_tick.get(),
            show_auth_dialog: show_auth_dialog.get(),
            auth_code: auth_code.read().clone(),
            auth_error: auth_error.read().clone(),
            show_tool_approval: show_tool_approval.get(),
            tool_approval_name: tool_approval_name.read().clone(),
            tool_approval_args: tool_approval_args.read().clone(),
            tool_approval_selected: tool_approval_selected.get(),
            show_vault_unlock: show_vault_unlock.get(),
            vault_password_len: vault_password.read().len(),
            vault_error: vault_error.read().clone(),
            show_user_prompt: show_user_prompt.get(),
            user_prompt_title: user_prompt_title.read().clone(),
            user_prompt_desc: user_prompt_desc.read().clone(),
            user_prompt_input: user_prompt_input.read().clone(),
            user_prompt_type: user_prompt_type.read().clone(),
            user_prompt_selected: user_prompt_selected.get(),
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
            hatching_state: hatching_state.read().clone(),
            hatching_agent_name: prop_soul_name_for_hatching,
            show_provider_selector: show_provider_selector.get(),
            provider_selector_items: provider_selector_items.read().clone(),
            provider_selector_ids: provider_selector_ids.read().clone(),
            provider_selector_hints: provider_selector_hints.read().clone(),
            provider_selector_cursor: provider_selector_cursor.get(),
            show_api_key_dialog: show_api_key_dialog.get(),
            api_key_provider_display: api_key_provider_display.read().clone(),
            api_key_input_len: api_key_input.read().len(),
            api_key_help_url: api_key_help_url.read().clone(),
            api_key_help_text: api_key_help_text.read().clone(),
            show_device_flow: show_device_flow.get(),
            device_flow_url: device_flow_url.read().clone(),
            device_flow_code: device_flow_code.read().clone(),
            device_flow_tick: device_flow_tick.get(),
            device_flow_browser_opened: device_flow_browser_opened.get(),
            show_model_selector: show_model_selector.get(),
            model_selector_provider_display: model_selector_provider_display.read().clone(),
            model_selector_models: model_selector_models.read().clone(),
            model_selector_cursor: model_selector_cursor.get(),
            model_selector_loading: model_selector_loading.get(),
            show_pairing: show_pairing.get(),
            pairing_step: *pairing_step.read(),
            pairing_field: *pairing_field.read(),
            pairing_public_key: pairing_public_key.read().clone(),
            pairing_fingerprint: pairing_fingerprint.read().clone(),
            pairing_fingerprint_art: pairing_fingerprint_art.read().clone(),
            pairing_qr_ascii: pairing_qr_ascii.read().clone(),
            pairing_host: pairing_host.read().clone(),
            pairing_port: pairing_port.read().clone(),
            pairing_error: pairing_error.read().clone(),
        )
    }
}
