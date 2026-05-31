// ── Root ────────────────────────────────────────────────────────────────────
//
// Top-level layout. Receives terminal size explicitly (as iocraft fullscreen
// examples do) and composes Messages+Sidebar, InputBar, StatusBar.

use iocraft::prelude::*;

use crate::components::api_key_dialog::ApiKeyDialog;
use crate::components::auth_dialog::AuthDialog;
use crate::components::command_menu::CommandMenu;
use crate::components::credential_request_dialog::CredentialRequestDialog;
use crate::components::details_dialog::DetailsDialog;
use crate::components::device_flow_dialog::DeviceFlowDialog;
use crate::components::hatching_dialog::HatchingDialog;
use crate::components::input_bar::InputBar;
use crate::components::messages::Messages;
use crate::components::model_selector_dialog::ModelSelectorDialog;
use crate::components::pairing_dialog::PairingDialog;
use crate::components::provider_selector_dialog::ProviderSelectorDialog;
use crate::components::secrets_dialog::SecretsDialog;
use crate::components::sidebar::Sidebar;
use crate::components::skills_dialog::SkillsDialog;
use crate::components::status_bar::StatusBar;
use crate::components::thread_tabs::ThreadTabs;
use crate::components::tool_approval_dialog::ToolApprovalDialog;
use crate::components::tool_perms_dialog::ToolPermsDialog;
use crate::components::user_prompt_dialog::UserPromptDialog;
use crate::components::vault_unlock_dialog::VaultUnlockDialog;
use crate::theme;
use crate::types::DisplayMessage;
use rustyclaw_view::{
    ApiKeyDialogData, AuthDialogData, CredentialRequestData, DeviceFlowData, HatchingDialogData,
    ModelSelectorData, PairingDialogData, ProviderSelectorData, SecretInfoData, SecretsDialogData,
    SkillInfoData, ToolApprovalData, ToolPermInfoData, VaultUnlockData,
};

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
    pub selected_message_idx: Option<usize>,

    // command menu (slash completions)
    pub command_completions: Vec<String>,
    pub command_selected: Option<usize>,

    // input
    pub composer: rustyclaw_view::ComposerData,
    pub input_value: String,
    pub input_cursor_offset: usize,
    pub on_change: HandlerMut<'static, String>,
    pub on_submit: HandlerMut<'static, String>,
    pub input_has_focus: bool,

    // sidebar (two-level: projects → threads)
    pub surface: rustyclaw_view::ChatSurfaceData,
    pub threads: Vec<rustyclaw_view::SidebarItemData>,
    pub projects: Vec<rustyclaw_core::ui::ProjectInfo>,
    pub active_project_id: u64,
    pub tab_focused: bool,
    pub tab_selected: usize,

    // status bar
    pub hint: String,

    // auth dialog overlay
    pub show_auth_dialog: bool,
    pub auth_dialog: AuthDialogData,

    // tool approval dialog overlay
    pub show_tool_approval: bool,
    pub tool_approval: ToolApprovalData,

    // vault unlock dialog overlay
    pub show_vault_unlock: bool,
    pub vault_unlock: VaultUnlockData,

    // user prompt dialog overlay
    pub show_user_prompt: bool,
    pub user_prompt_title: String,
    pub user_prompt_desc: String,
    pub user_prompt_input: String,
    pub user_prompt_type: Option<rustyclaw_core::user_prompt_types::PromptType>,
    pub user_prompt_selected: usize,

    // credential request dialog overlay
    pub show_credential_request: bool,
    pub credential_request: CredentialRequestData,

    // secrets dialog overlay
    pub show_secrets_dialog: bool,
    pub secrets_data: Vec<SecretInfoData>,
    pub secrets_agent_access: bool,
    pub secrets_has_totp: bool,
    pub secrets_selected: Option<usize>,
    pub secrets_scroll_offset: usize,
    pub secrets_add_step: u8,
    pub secrets_add_name: String,
    pub secrets_add_value: String,

    // skills dialog overlay
    pub show_skills_dialog: bool,
    pub skills_data: Vec<SkillInfoData>,
    pub skills_selected: Option<usize>,
    pub skills_scroll_offset: usize,

    // details dialog overlay (extended structured details for the
    // most recent warning/error toast)
    pub show_details_dialog: bool,
    pub details_dialog_text: String,
    pub details_dialog_is_error: bool,
    pub details_dialog_scroll: usize,

    // tool permissions dialog overlay
    pub show_tool_perms_dialog: bool,
    pub tool_perms_data: Vec<ToolPermInfoData>,
    pub tool_perms_selected: Option<usize>,
    pub tool_perms_scroll_offset: usize,

    // hatching dialog overlay (first run)
    pub hatching_dialog: HatchingDialogData,

    // provider selector dialog overlay
    pub show_provider_selector: bool,
    pub provider_selector: ProviderSelectorData,

    // API key dialog overlay
    pub show_api_key_dialog: bool,
    pub api_key_dialog: ApiKeyDialogData,

    // device flow dialog overlay
    pub show_device_flow: bool,
    pub device_flow: DeviceFlowData,

    // model selector dialog overlay
    pub show_model_selector: bool,
    pub model_selector: ModelSelectorData,

    // pairing dialog overlay (SSH pairing)
    pub show_pairing: bool,
    pub pairing: PairingDialogData,
}

#[component]
pub fn Root(props: &mut RootProps) -> impl Into<AnyElement<'static>> {
    let show_auth = props.show_auth_dialog;
    let show_approval = props.show_tool_approval;
    let show_vault = props.show_vault_unlock;
    let show_prompt = props.show_user_prompt;
    let show_credential = props.show_credential_request;
    let auth_dialog = props.auth_dialog.clone();
    let tool_approval = props.tool_approval.clone();
    let vault_unlock = props.vault_unlock.clone();
    let credential_request = props.credential_request.clone();

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
    let show_details = props.show_details_dialog;
    let details_text = std::mem::take(&mut props.details_dialog_text);
    let details_is_error = props.details_dialog_is_error;
    let details_scroll = props.details_dialog_scroll;
    let tool_perms_data = std::mem::take(&mut props.tool_perms_data);
    let tool_perms_selected = props.tool_perms_selected;
    let tool_perms_scroll = props.tool_perms_scroll_offset;
    #[allow(unused_variables)]
    let show_tool_perms = props.show_tool_perms_dialog;

    let hatching_dialog = props.hatching_dialog.clone();
    let show_hatching = hatching_dialog.should_render(show_auth);

    // Provider / model selection dialog state
    let show_provider_sel = props.show_provider_selector;
    let provider_selector = props.provider_selector.clone();

    let show_apikey = props.show_api_key_dialog;
    let api_key_dialog = props.api_key_dialog.clone();

    let show_devflow = props.show_device_flow;
    let device_flow = props.device_flow.clone();

    let show_model_sel = props.show_model_selector;
    let model_selector = props.model_selector.clone();

    // Pairing dialog state
    let show_pairing = props.show_pairing;
    let pairing = props.pairing.clone();

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
                // Left sidebar: two-level project → thread navigation.
                ThreadTabs(
                    threads: props.threads.clone(),
                    projects: props.projects.clone(),
                    active_project_id: props.active_project_id,
                    focused: props.tab_focused,
                    selected: props.tab_selected,
                )
                // Chat area: messages + input
                View(
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                ) {
                    Messages(
                        messages: props.messages.clone(),
                        scroll_offset: props.scroll_offset,
                        surface: props.surface.clone(),
                        assistant_name: if props.soul_name.is_empty() {
                            None
                        } else {
                            Some(props.soul_name.clone())
                        },
                        selected_idx: props.selected_message_idx,
                    )
                    CommandMenu(
                        completions: props.command_completions.clone(),
                        selected: props.command_selected,
                    )
                    InputBar(
                        composer: props.composer.clone(),
                        value: props.input_value.clone(),
                        cursor_offset: props.input_cursor_offset,
                        on_change: props.on_change.take(),
                        on_submit: props.on_submit.take(),
                        gateway_icon: props.gateway_icon.clone(),
                        gateway_label: props.gateway_label.clone(),
                        gateway_color: props.gateway_color,
                        has_focus: props.input_has_focus,
                    )
                }
                // Sidebar (simplified: no thread list)
                Sidebar(
                    gateway_label: props.gateway_label.clone(),
                    surface: props.surface.clone(),
                )
            }

            // ── Status bar (1 row) ──────────────────────────────────────
            StatusBar(
                hint: props.hint.clone(),
                surface: props.surface.clone(),
                soul_name: props.soul_name.clone(),
                model_label: props.model_label.clone(),
            )

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
                            data: hatching_dialog,
                        )
                    }
                }.into_any()
            } else {
                element! { View() }.into_any()
            })

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
                            data: auth_dialog,
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
                            data: tool_approval,
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
                            data: vault_unlock,
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

            // ── Credential request dialog overlay ────────────────────────
            #(if show_credential {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        CredentialRequestDialog(
                            data: credential_request,
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
                            data: SecretsDialogData {
                                secrets: secrets_data,
                                agent_access: secrets_agent,
                                has_totp: secrets_totp,
                                selected: secrets_selected,
                                scroll_offset: secrets_scroll,
                                add_step: secrets_add_step,
                                add_name: secrets_add_name,
                                add_value: secrets_add_value,
                                status: None,
                            },
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

            // ── Details dialog overlay (extended error/warning) ─────────
            #(if show_details {
                element! {
                    View(
                        width: props.width,
                        height: props.height,
                        position: Position::Absolute,
                        top: 0,
                        left: 0,
                    ) {
                        DetailsDialog(
                            details: details_text,
                            is_error: details_is_error,
                            scroll_offset: details_scroll,
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
                            data: provider_selector,
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
                            data: api_key_dialog,
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
                            data: device_flow,
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
                            data: model_selector,
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
                            data: pairing,
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
