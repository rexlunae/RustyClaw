// ── Root ────────────────────────────────────────────────────────────────────
//
// Top-level layout. Receives terminal size explicitly (as iocraft fullscreen
// examples do) and composes Messages+Sidebar, InputBar, StatusBar.

use iocraft::prelude::*;

use crate::components::api_key_dialog::ApiKeyDialog;
use crate::components::auth_dialog::AuthDialog;
use crate::components::command_menu::CommandMenu;
use crate::components::device_flow_dialog::DeviceFlowDialog;
use crate::components::hatching_dialog::HatchingDialog;
use crate::components::input_bar::InputBar;
use crate::components::messages::Messages;
use crate::components::model_selector_dialog::ModelSelectorDialog;
use crate::components::pairing_dialog::PairingDialog;
use crate::components::provider_selector_dialog::ProviderSelectorDialog;
use crate::components::secrets_dialog::{SecretInfo, SecretsDialog};
use crate::components::sidebar::Sidebar;
use crate::components::skills_dialog::{SkillInfo, SkillsDialog};
use crate::components::status_bar::StatusBar;
use crate::components::tool_approval_dialog::ToolApprovalDialog;
use crate::components::tool_perms_dialog::{ToolPermInfo, ToolPermsDialog};
use crate::components::user_prompt_dialog::UserPromptDialog;
use crate::components::vault_unlock_dialog::VaultUnlockDialog;
use crate::theme;
use crate::types::DisplayMessage;

#[derive(Default, Props)]
pub struct RootProps {
    // terminal
    pub width: u16,
    pub height: u16,

    // identity / model (shown in status bar)
    pub soul_name: String,
    pub model_label: String,

    // gateway (used by input bar & sidebar)
    pub gateway_icon: String,
    pub gateway_label: String,
    pub gateway_color: Option<Color>,

    // messages
    pub messages: Vec<DisplayMessage>,
    pub scroll_offset: i32,

    // command menu (slash completions)
    pub command_completions: Vec<String>,
    pub command_selected: Option<usize>,

    // input
    pub input_value: String,
    pub on_change: HandlerMut<'static, String>,
    pub on_submit: HandlerMut<'static, String>,
    pub input_has_focus: bool,

    // sidebar
    pub task_text: String,
    pub streaming: bool,
    pub elapsed: String,
    pub threads: Vec<crate::action::ThreadInfo>,
    pub sidebar_focused: bool,
    pub sidebar_selected: usize,

    // status bar
    pub hint: String,
    pub spinner_tick: usize,

    // auth dialog overlay
    pub show_auth_dialog: bool,
    pub auth_code: String,
    pub auth_error: String,

    // tool approval dialog overlay
    pub show_tool_approval: bool,
    pub tool_approval_name: String,
    pub tool_approval_args: String,
    pub tool_approval_selected: bool,

    // vault unlock dialog overlay
    pub show_vault_unlock: bool,
    pub vault_password_len: usize,
    pub vault_error: String,

    // user prompt dialog overlay
    pub show_user_prompt: bool,
    pub user_prompt_title: String,
    pub user_prompt_desc: String,
    pub user_prompt_input: String,
    pub user_prompt_type: Option<rustyclaw_core::user_prompt_types::PromptType>,
    pub user_prompt_selected: usize,

    // secrets dialog overlay
    pub show_secrets_dialog: bool,
    pub secrets_data: Vec<SecretInfo>,
    pub secrets_agent_access: bool,
    pub secrets_has_totp: bool,
    pub secrets_selected: Option<usize>,
    pub secrets_scroll_offset: usize,
    pub secrets_add_step: u8,
    pub secrets_add_name: String,
    pub secrets_add_value: String,

    // skills dialog overlay
    pub show_skills_dialog: bool,
    pub skills_data: Vec<SkillInfo>,
    pub skills_selected: Option<usize>,
    pub skills_scroll_offset: usize,

    // tool permissions dialog overlay
    pub show_tool_perms_dialog: bool,
    pub tool_perms_data: Vec<ToolPermInfo>,
    pub tool_perms_selected: Option<usize>,
    pub tool_perms_scroll_offset: usize,

    // hatching dialog overlay (first run)
    pub show_hatching: bool,
    pub hatching_state: crate::components::hatching_dialog::HatchState,
    pub hatching_agent_name: String,

    // provider selector dialog overlay
    pub show_provider_selector: bool,
    pub provider_selector_items: Vec<String>,
    pub provider_selector_ids: Vec<String>,
    pub provider_selector_hints: Vec<String>,
    pub provider_selector_cursor: usize,

    // API key dialog overlay
    pub show_api_key_dialog: bool,
    pub api_key_provider_display: String,
    pub api_key_input_len: usize,
    pub api_key_help_url: String,
    pub api_key_help_text: String,

    // device flow dialog overlay
    pub show_device_flow: bool,
    pub device_flow_url: String,
    pub device_flow_code: String,
    pub device_flow_tick: usize,

    // model selector dialog overlay
    pub show_model_selector: bool,
    pub model_selector_provider_display: String,
    pub model_selector_models: Vec<String>,
    pub model_selector_cursor: usize,
    pub model_selector_loading: bool,

    // pairing dialog overlay (SSH pairing)
    pub show_pairing: bool,
    pub pairing_step: crate::components::pairing_dialog::PairingStep,
    pub pairing_field: crate::components::pairing_dialog::PairingField,
    pub pairing_public_key: String,
    pub pairing_fingerprint: String,
    pub pairing_fingerprint_art: String,
    pub pairing_qr_ascii: String,
    pub pairing_host: String,
    pub pairing_port: String,
    pub pairing_error: String,
}

#[component]
pub fn Root(props: &mut RootProps) -> impl Into<AnyElement<'static>> {
    let show_auth = props.show_auth_dialog;
    let show_approval = props.show_tool_approval;
    let show_vault = props.show_vault_unlock;
    let show_prompt = props.show_user_prompt;

    let secrets_data = std::mem::take(&mut props.secrets_data);
    let secrets_agent = props.secrets_agent_access;
    let secrets_totp = props.secrets_has_totp;
    let secrets_selected = props.secrets_selected;
    let secrets_scroll = props.secrets_scroll_offset;
    let secrets_add_step = props.secrets_add_step;
    let secrets_add_name = std::mem::take(&mut props.secrets_add_name);
    let secrets_add_value = std::mem::take(&mut props.secrets_add_value);
    #[allow(unused_variables)]
    let show_secrets = props.show_secrets_dialog;
    let skills_data = std::mem::take(&mut props.skills_data);
    let skills_selected = props.skills_selected;
    let skills_scroll = props.skills_scroll_offset;
    #[allow(unused_variables)]
    let show_skills = props.show_skills_dialog;
    let tool_perms_data = std::mem::take(&mut props.tool_perms_data);
    let tool_perms_selected = props.tool_perms_selected;
    let tool_perms_scroll = props.tool_perms_scroll_offset;
    #[allow(unused_variables)]
    let show_tool_perms = props.show_tool_perms_dialog;
    
    let show_hatching = props.show_hatching;
    let hatching_state = props.hatching_state.clone();
    let hatching_agent_name = props.hatching_agent_name.clone();

    // Provider / model selection dialog state
    let show_provider_sel = props.show_provider_selector;
    let provider_sel_items = std::mem::take(&mut props.provider_selector_items);
    let provider_sel_ids = std::mem::take(&mut props.provider_selector_ids);
    let provider_sel_hints = std::mem::take(&mut props.provider_selector_hints);
    let provider_sel_cursor = props.provider_selector_cursor;

    let show_apikey = props.show_api_key_dialog;
    let apikey_display = props.api_key_provider_display.clone();
    let apikey_input_len = props.api_key_input_len;
    let apikey_help_url = props.api_key_help_url.clone();
    let apikey_help_text = props.api_key_help_text.clone();

    let show_devflow = props.show_device_flow;
    let devflow_url = props.device_flow_url.clone();
    let devflow_code = props.device_flow_code.clone();
    let devflow_tick = props.device_flow_tick;

    let show_model_sel = props.show_model_selector;
    let model_sel_display = props.model_selector_provider_display.clone();
    let model_sel_models = std::mem::take(&mut props.model_selector_models);
    let model_sel_cursor = props.model_selector_cursor;
    let model_sel_loading = props.model_selector_loading;

    // Pairing dialog state
    let show_pairing = props.show_pairing;
    let pairing_step = props.pairing_step;
    let pairing_field = props.pairing_field;
    let pairing_public_key = std::mem::take(&mut props.pairing_public_key);
    let pairing_fingerprint = std::mem::take(&mut props.pairing_fingerprint);
    let pairing_fingerprint_art = std::mem::take(&mut props.pairing_fingerprint_art);
    let pairing_qr_ascii = std::mem::take(&mut props.pairing_qr_ascii);
    let pairing_host = std::mem::take(&mut props.pairing_host);
    let pairing_port = std::mem::take(&mut props.pairing_port);
    let pairing_error = std::mem::take(&mut props.pairing_error);

    element! {
        View(
            width: props.width,
            height: props.height,
            flex_direction: FlexDirection::Column,
            background_color: theme::BG_MAIN,
        ) {
            // ── Main area (flex grow) ───────────────────────────────────
            View(
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                width: 100pct,
            ) {
                // Chat area: messages + input
                View(
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                ) {
                    Messages(
                        messages: props.messages.clone(),
                        scroll_offset: props.scroll_offset,
                        streaming: props.streaming,
                        spinner_tick: props.spinner_tick,
                        elapsed: props.elapsed.clone(),
                        assistant_name: if props.soul_name.is_empty() {
                            None
                        } else {
                            Some(props.soul_name.clone())
                        },
                    )
                    CommandMenu(
                        completions: props.command_completions.clone(),
                        selected: props.command_selected,
                    )
                    InputBar(
                        value: props.input_value.clone(),
                        on_change: props.on_change.take(),
                        on_submit: props.on_submit.take(),
                        gateway_icon: props.gateway_icon.clone(),
                        gateway_label: props.gateway_label.clone(),
                        gateway_color: props.gateway_color,
                        has_focus: props.input_has_focus,
                    )
                }
                // Sidebar
                Sidebar(
                    gateway_label: props.gateway_label.clone(),
                    task_text: props.task_text.clone(),
                    streaming: props.streaming,
                    elapsed: props.elapsed.clone(),
                    spinner_tick: props.spinner_tick,
                    threads: props.threads.clone(),
                    focused: props.sidebar_focused,
                    selected: props.sidebar_selected,
                )
            }

            // ── Status bar (1 row) ──────────────────────────────────────
            StatusBar(
                hint: props.hint.clone(),
                streaming: props.streaming,
                elapsed: props.elapsed.clone(),
                spinner_tick: props.spinner_tick,
                soul_name: props.soul_name.clone(),
                model_label: props.model_label.clone(),
            )

            // ── Auth dialog overlay ─────────────────────────────────────
            #(if show_auth {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        AuthDialog(
                            code: props.auth_code.clone(),
                            error: props.auth_error.clone(),
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Tool approval dialog overlay ────────────────────────────
            #(if show_approval {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        ToolApprovalDialog(
                            tool_name: props.tool_approval_name.clone(),
                            arguments: props.tool_approval_args.clone(),
                            selected_allow: props.tool_approval_selected,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Vault unlock dialog overlay ─────────────────────────────
            #(if show_vault {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        VaultUnlockDialog(
                            password_len: props.vault_password_len,
                            error: props.vault_error.clone(),
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── User prompt dialog overlay ──────────────────────────────
            #(if show_prompt {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        UserPromptDialog(
                            title: props.user_prompt_title.clone(),
                            description: props.user_prompt_desc.clone(),
                            input: props.user_prompt_input.clone(),
                            prompt_type: props.user_prompt_type.clone(),
                            selected: props.user_prompt_selected,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Secrets dialog overlay ──────────────────────────────────
            #(if show_secrets {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        SecretsDialog(
                            secrets: secrets_data,
                            agent_access: secrets_agent,
                            has_totp: secrets_totp,
                            selected: secrets_selected,
                            scroll_offset: secrets_scroll,
                            add_step: secrets_add_step,
                            add_name: secrets_add_name,
                            add_value: secrets_add_value,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Skills dialog overlay ───────────────────────────────────
            #(if show_skills {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        SkillsDialog(
                            skills: skills_data,
                            selected: skills_selected,
                            scroll_offset: skills_scroll,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Tool permissions dialog overlay ─────────────────────────
            #(if show_tool_perms {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        ToolPermsDialog(
                            tools: tool_perms_data,
                            selected: tool_perms_selected,
                            scroll_offset: tool_perms_scroll,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Hatching dialog overlay (first run) ─────────────────────
            #(if show_hatching {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        HatchingDialog(
                            state: hatching_state,
                            agent_name: hatching_agent_name,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Provider selector dialog overlay ────────────────────────
            #(if show_provider_sel {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        ProviderSelectorDialog(
                            providers: provider_sel_items,
                            provider_ids: provider_sel_ids,
                            auth_hints: provider_sel_hints,
                            cursor: provider_sel_cursor,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── API key dialog overlay ──────────────────────────────────
            #(if show_apikey {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        ApiKeyDialog(
                            provider_display: apikey_display,
                            input_len: apikey_input_len,
                            help_url: apikey_help_url,
                            help_text: apikey_help_text,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Device flow dialog overlay ──────────────────────────────
            #(if show_devflow {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        DeviceFlowDialog(
                            url: devflow_url,
                            code: devflow_code,
                            tick: devflow_tick,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Model selector dialog overlay ───────────────────────────
            #(if show_model_sel {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        ModelSelectorDialog(
                            provider_display: model_sel_display,
                            models: model_sel_models,
                            cursor: model_sel_cursor,
                            loading: model_sel_loading,
                            spinner_tick: devflow_tick,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

            // ── Pairing dialog overlay ──────────────────────────────────
            #(if show_pairing {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        PairingDialog(
                            step: pairing_step,
                            public_key: pairing_public_key,
                            fingerprint: pairing_fingerprint,
                            fingerprint_art: pairing_fingerprint_art,
                            qr_ascii: pairing_qr_ascii,
                            gateway_host: pairing_host,
                            gateway_port: pairing_port,
                            active_field: pairing_field,
                            error: pairing_error,
                            success: String::new(),
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })
        }
    }
}
