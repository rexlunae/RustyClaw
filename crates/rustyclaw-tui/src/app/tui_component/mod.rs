use iocraft::prelude::*;
use std::collections::HashMap;
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use crate::components::root::Root;
use crate::theme;
use crate::types::DisplayMessage;

use crate::app::{GwEvent, UserInput};

mod events;
mod keyboard;
mod keyboard_normal;
mod state;

pub(super) fn display_message_from_gateway(
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
#[allow(unused_mut)]
pub fn TuiRoot(props: &TuiRootProps, mut hooks: Hooks) -> impl Into<AnyElement<'static>> {
    let (width, height) = hooks.use_terminal_size();
    let mut system = hooks.use_context_mut::<SystemContext>();

    // ── Local UI state ──────────────────────────────────────────────
    let messages: State<Vec<DisplayMessage>> = hooks.use_state(Vec::new);
    let input_value = hooks.use_state(String::new);
    let input_cursor_offset = hooks.use_state(|| 0usize);
    let gw_status = hooks.use_state(|| rustyclaw_core::types::GatewayStatus::Connecting);
    let streaming = hooks.use_state(|| false);
    let stream_start: State<Option<Instant>> = hooks.use_state(|| None);
    let mut elapsed = hooks.use_state(String::new);
    let mut scroll_offset = hooks.use_state(|| 0i32);
    let mut spinner_tick = hooks.use_state(|| 0usize);
    let should_quit = hooks.use_state(|| false);
    let streaming_buf = hooks.use_state(String::new);
    let dynamic_model_label: State<Option<String>> = hooks.use_state(|| None);
    let dynamic_provider_id: State<Option<String>> = hooks.use_state(|| None);
    let selected_message_idx: State<Option<usize>> = hooks.use_state(|| None);

    // ── Auth dialog state ───────────────────────────────────────────
    let show_auth_dialog = hooks.use_state(|| false);
    let auth_code = hooks.use_state(String::new);
    let auth_error = hooks.use_state(String::new);

    // ── Tool approval dialog state ──────────────────────────────────
    let show_tool_approval = hooks.use_state(|| false);
    let tool_approval_id = hooks.use_state(String::new);
    let tool_approval_name = hooks.use_state(String::new);
    let tool_approval_args = hooks.use_state(String::new);
    let tool_approval_selected = hooks.use_state(|| true); // true = Allow

    // ── Vault unlock dialog state ───────────────────────────────────
    let show_vault_unlock = hooks.use_state(|| false);
    let vault_password = hooks.use_state(String::new);
    let vault_error = hooks.use_state(String::new);

    // ── Hatching dialog state ───────────────────────────────────────
    // Start hidden; the shared view model reveals it after auth succeeds so it
    // never competes with the TOTP dialog for screen space.
    let needs_hatching = props.needs_hatching;
    let hatching_dialog: State<rustyclaw_view::HatchingDialogData> =
        hooks.use_state(rustyclaw_view::HatchingDialogData::default);

    // ── Pairing dialog state ────────────────────────────────────────
    let show_pairing = hooks.use_state(|| false);
    let pairing_step: State<rustyclaw_view::PairingStep> =
        hooks.use_state(|| rustyclaw_view::PairingStep::ShowKey);
    let pairing_field: State<rustyclaw_view::PairingField> =
        hooks.use_state(|| rustyclaw_view::PairingField::Host);
    let pairing_public_key = hooks.use_state(String::new);
    let pairing_fingerprint = hooks.use_state(String::new);
    let pairing_fingerprint_art = hooks.use_state(String::new);
    let pairing_qr_ascii = hooks.use_state(String::new);
    let pairing_host = hooks.use_state(String::new);
    let pairing_port = hooks.use_state(|| "2222".to_string());
    let pairing_error = hooks.use_state(String::new);

    // ── User prompt dialog state ────────────────────────────────────
    let show_user_prompt = hooks.use_state(|| false);
    let user_prompt_id = hooks.use_state(String::new);
    let user_prompt_title = hooks.use_state(String::new);
    let user_prompt_desc = hooks.use_state(String::new);
    let user_prompt_input = hooks.use_state(String::new);
    let user_prompt_type: State<Option<rustyclaw_core::user_prompt_types::PromptType>> =
        hooks.use_state(|| None);
    let user_prompt_selected = hooks.use_state(|| 0usize);

    // ── Credential request dialog state ───────────────────────────────
    let show_credential_request = hooks.use_state(|| false);
    let credential_request_id = hooks.use_state(String::new);
    let credential_request_provider = hooks.use_state(String::new);
    let credential_request_secret_name = hooks.use_state(String::new);
    let credential_request_message = hooks.use_state(String::new);
    let credential_request_input = hooks.use_state(String::new);

    // ── Provider / model selection dialog state ─────────────────────
    let show_provider_selector = hooks.use_state(|| false);
    let provider_selector_items: State<Vec<String>> = hooks.use_state(Vec::new);
    let provider_selector_ids: State<Vec<String>> = hooks.use_state(Vec::new);
    let provider_selector_hints: State<Vec<String>> = hooks.use_state(Vec::new);
    let provider_selector_cursor = hooks.use_state(|| 0usize);

    let show_api_key_dialog = hooks.use_state(|| false);
    let api_key_provider = hooks.use_state(String::new);
    let api_key_provider_display = hooks.use_state(String::new);
    let api_key_input = hooks.use_state(String::new);
    let api_key_help_url = hooks.use_state(String::new);
    let api_key_help_text = hooks.use_state(String::new);

    let show_device_flow = hooks.use_state(|| false);
    let device_flow_provider = hooks.use_state(String::new);
    let device_flow_url = hooks.use_state(String::new);
    let device_flow_code = hooks.use_state(String::new);
    let mut device_flow_tick = hooks.use_state(|| 0usize);
    let device_flow_browser_opened = hooks.use_state(|| false);

    let show_model_selector = hooks.use_state(|| false);
    let model_selector_provider = hooks.use_state(String::new);
    let model_selector_provider_display = hooks.use_state(String::new);
    let model_selector_models: State<Vec<String>> = hooks.use_state(Vec::new);
    let model_selector_cursor = hooks.use_state(|| 0usize);
    let model_selector_loading = hooks.use_state(|| false);

    // ── Thread state (unified tasks + threads) ───────────────────────
    let threads: State<Vec<rustyclaw_view::SidebarItemData>> = hooks.use_state(Vec::new);
    let projects: State<Vec<rustyclaw_core::ui::ProjectInfo>> = hooks.use_state(Vec::new);
    let active_project_id = hooks.use_state(|| 0u64);
    let tab_focused = hooks.use_state(|| false);
    let tab_selected = hooks.use_state(|| 0usize);
    // Per-thread message cache so switching tabs restores prior
    // scrollback instead of clearing the chat (matches desktop client).
    let thread_messages_cache: State<HashMap<u64, Vec<DisplayMessage>>> =
        hooks.use_state(HashMap::new);
    let foreground_thread_id: State<Option<u64>> = hooks.use_state(|| None);

    // ── Command menu (slash-command completions) ────────────────────
    let command_completions: State<Vec<String>> = hooks.use_state(Vec::new);
    let command_selected: State<Option<usize>> = hooks.use_state(|| None);
    let model_completion_provider: State<Option<String>> = hooks.use_state(|| None);
    let model_completion_models: State<Vec<String>> = hooks.use_state(Vec::new);
    let model_completion_loading: State<Option<String>> = hooks.use_state(|| None);
    let prompt_attachments: State<Vec<rustyclaw_view::PromptAttachment>> =
        hooks.use_state(Vec::new);

    // ── Info dialog state (secrets / skills / tool permissions) ──────
    let show_secrets_dialog = hooks.use_state(|| false);
    let secrets_dialog_data: State<Vec<rustyclaw_view::SecretInfoData>> = hooks.use_state(Vec::new);
    let secrets_agent_access = hooks.use_state(|| false);
    let secrets_has_totp = hooks.use_state(|| false);
    let secrets_selected: State<Option<usize>> = hooks.use_state(|| Some(0));
    let secrets_scroll_offset = hooks.use_state(|| 0usize);
    // Add-secret inline input: 0 = off, 1 = entering name, 2 = entering value
    let secrets_add_step = hooks.use_state(|| 0u8);
    let secrets_add_name = hooks.use_state(String::new);
    let secrets_add_value = hooks.use_state(String::new);

    let show_skills_dialog = hooks.use_state(|| false);
    let skills_dialog_data: State<Vec<rustyclaw_view::SkillInfoData>> = hooks.use_state(Vec::new);
    let skills_selected: State<Option<usize>> = hooks.use_state(|| Some(0));

    // Details dialog overlay — shows extended `RequestDetails`
    // (URL, status, redacted headers, body excerpt, full cause
    // chain) attached to the most recent warning/error toast.
    // Opened with Ctrl-D when the latest message has details.
    let show_details_dialog = hooks.use_state(|| false);
    let details_dialog_text = hooks.use_state(String::new);
    let details_dialog_is_error = hooks.use_state(|| false);
    let details_dialog_scroll = hooks.use_state(|| 0usize);

    let show_tool_perms_dialog = hooks.use_state(|| false);
    let tool_perms_dialog_data: State<Vec<rustyclaw_view::ToolPermInfoData>> =
        hooks.use_state(Vec::new);
    let tool_perms_selected: State<Option<usize>> = hooks.use_state(|| Some(0));

    // Scroll offsets for interactive dialogs
    let skills_scroll_offset = hooks.use_state(|| 0usize);
    let tool_perms_scroll_offset = hooks.use_state(|| 0usize);

    // ── Channel access ──────────────────────────────────────────────
    let gw_rx: Arc<StdMutex<Option<sync_mpsc::Receiver<GwEvent>>>> =
        hooks.use_const(|| Arc::new(StdMutex::new(CHANNEL_RX.lock().unwrap().take())));
    let user_tx: Arc<StdMutex<Option<sync_mpsc::Sender<UserInput>>>> =
        hooks.use_const(|| Arc::new(StdMutex::new(CHANNEL_TX.lock().unwrap().take())));
    let prop_provider_id = props.provider_id.clone();
    let tx_for_model_completions = Arc::clone(&user_tx);

    let ui = state::Ui {
        messages,
        input_value,
        input_cursor_offset,
        gw_status,
        streaming,
        stream_start,
        elapsed,
        scroll_offset,
        spinner_tick,
        should_quit,
        streaming_buf,
        dynamic_model_label,
        dynamic_provider_id,
        selected_message_idx,
        show_auth_dialog,
        auth_code,
        auth_error,
        show_tool_approval,
        tool_approval_id,
        tool_approval_name,
        tool_approval_args,
        tool_approval_selected,
        show_vault_unlock,
        vault_password,
        vault_error,
        hatching_dialog,
        show_pairing,
        pairing_step,
        pairing_field,
        pairing_public_key,
        pairing_fingerprint,
        pairing_fingerprint_art,
        pairing_qr_ascii,
        pairing_host,
        pairing_port,
        pairing_error,
        show_user_prompt,
        user_prompt_id,
        user_prompt_title,
        user_prompt_desc,
        user_prompt_input,
        user_prompt_type,
        user_prompt_selected,
        show_credential_request,
        credential_request_id,
        credential_request_provider,
        credential_request_secret_name,
        credential_request_message,
        credential_request_input,
        show_provider_selector,
        provider_selector_items,
        provider_selector_ids,
        provider_selector_hints,
        provider_selector_cursor,
        show_api_key_dialog,
        api_key_provider,
        api_key_provider_display,
        api_key_input,
        api_key_help_url,
        api_key_help_text,
        show_device_flow,
        device_flow_provider,
        device_flow_url,
        device_flow_code,
        device_flow_tick,
        device_flow_browser_opened,
        show_model_selector,
        model_selector_provider,
        model_selector_provider_display,
        model_selector_models,
        model_selector_cursor,
        model_selector_loading,
        threads,
        projects,
        active_project_id,
        tab_focused,
        tab_selected,
        thread_messages_cache,
        foreground_thread_id,
        command_completions,
        command_selected,
        model_completion_provider,
        model_completion_models,
        model_completion_loading,
        prompt_attachments,
        show_secrets_dialog,
        secrets_dialog_data,
        secrets_agent_access,
        secrets_has_totp,
        secrets_selected,
        secrets_scroll_offset,
        secrets_add_step,
        secrets_add_name,
        secrets_add_value,
        show_skills_dialog,
        skills_dialog_data,
        skills_selected,
        show_details_dialog,
        details_dialog_text,
        details_dialog_is_error,
        details_dialog_scroll,
        show_tool_perms_dialog,
        tool_perms_dialog_data,
        tool_perms_selected,
        skills_scroll_offset,
        tool_perms_scroll_offset,
    };

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
                            events::apply_gw_event(ev, ui, needs_hatching, &tx_for_history);
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
        move |event| {
            keyboard::apply_key_event(
                event,
                ui,
                &tx_for_keys,
                &tx_for_model_completions,
                &prop_provider_id,
                &prop_gateway_host,
                &prop_gateway_port,
            )
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
            selected_message_idx: selected_message_idx.get(),
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
                && !hatching_dialog.read().visible
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
            threads: threads.read().clone(),
            projects: projects.read().clone(),
            active_project_id: active_project_id.get(),
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
            hatching_dialog: hatching_dialog.read().clone(),
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
