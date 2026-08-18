//! Shared state bundle for the TUI root component.
//!
//! [`Ui`] groups every `State<…>` handle declared in `TuiRoot` so the gateway-
//! event and keyboard handlers can be split into their own modules. iocraft
//! `State<T>` is `Copy`, so `Ui` is `Copy` and cheap to pass around. Handlers
//! destructure it back into the original names, keeping their bodies verbatim.

use std::collections::HashMap;
use std::time::Instant;

use iocraft::prelude::*;

use crate::types::DisplayMessage;

/// The controllable child process behind the currently-running tool call,
/// tracked so the inline chat controls (pause/stop/kill) know which PID
/// to target and whether it is paused.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ActiveProcess {
    pub tool_id: String,
    pub pid: u32,
    pub paused: bool,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(super) struct Ui {
    pub messages: State<Vec<DisplayMessage>>,
    pub input_value: State<String>,
    pub input_cursor_offset: State<usize>,
    /// Submitted prompts, oldest first, for bash-style up/down recall.
    pub input_history: State<Vec<String>>,
    /// Position while browsing history (`None` = not browsing).
    pub history_index: State<Option<usize>>,
    /// The in-progress draft stashed when history browsing begins.
    pub history_draft: State<String>,
    pub gw_status: State<rustyclaw_core::types::GatewayStatus>,
    pub streaming: State<bool>,
    pub stream_start: State<Option<Instant>>,
    /// When the current thinking block began (for "Thought for Xs").
    pub thinking_start: State<Option<Instant>>,
    /// Start times of in-flight tool calls, by tool-call id, so results
    /// can be stamped with a wall-clock duration.
    pub tool_started: State<HashMap<String, Instant>>,
    /// The controllable process behind the currently-running tool call
    /// (None when no tool is waiting on a child process).
    pub active_process: State<Option<ActiveProcess>>,
    pub elapsed: State<String>,
    pub scroll_offset: State<i32>,
    pub spinner_tick: State<usize>,
    pub should_quit: State<bool>,
    pub streaming_buf: State<String>,
    pub dynamic_model_label: State<Option<String>>,
    pub dynamic_provider_id: State<Option<String>>,
    pub selected_message_idx: State<Option<usize>>,
    pub show_auth_dialog: State<bool>,
    pub auth_code: State<String>,
    pub auth_error: State<String>,
    pub show_tool_approval: State<bool>,
    pub tool_approval_id: State<String>,
    pub tool_approval_name: State<String>,
    pub tool_approval_args: State<String>,
    pub tool_approval_selected: State<bool>,
    pub show_vault_unlock: State<bool>,
    pub vault_password: State<String>,
    pub vault_error: State<String>,
    pub hatching_dialog: State<rustyclaw_view::HatchingDialogData>,
    pub show_pairing: State<bool>,
    pub pairing_step: State<rustyclaw_view::PairingStep>,
    pub pairing_field: State<rustyclaw_view::PairingField>,
    pub pairing_public_key: State<String>,
    pub pairing_fingerprint: State<String>,
    pub pairing_fingerprint_art: State<String>,
    pub pairing_qr_ascii: State<String>,
    pub pairing_host: State<String>,
    pub pairing_port: State<String>,
    pub pairing_error: State<String>,
    pub show_user_prompt: State<bool>,
    pub user_prompt_id: State<String>,
    pub user_prompt_title: State<String>,
    pub user_prompt_desc: State<String>,
    pub user_prompt_input: State<String>,
    pub user_prompt_type: State<Option<rustyclaw_core::user_prompt_types::PromptType>>,
    pub user_prompt_selected: State<usize>,
    /// Per-option checked flags for MultiSelect prompts.
    pub user_prompt_checked: State<Vec<bool>>,
    pub show_credential_request: State<bool>,
    pub credential_request_id: State<String>,
    pub credential_request_provider: State<String>,
    pub credential_request_secret_name: State<String>,
    pub credential_request_message: State<String>,
    pub credential_request_input: State<String>,
    /// The turn whose request the visible credential dialog belongs to, so
    /// a close-out for another turn cannot tear it down — and the one for
    /// its own turn can.
    pub credential_request_thread: State<Option<u64>>,
    pub show_provider_selector: State<bool>,
    pub provider_selector_items: State<Vec<String>>,
    pub provider_selector_ids: State<Vec<String>>,
    pub provider_selector_hints: State<Vec<String>>,
    pub provider_selector_cursor: State<usize>,
    pub show_api_key_dialog: State<bool>,
    pub api_key_provider: State<String>,
    pub api_key_provider_display: State<String>,
    pub api_key_input: State<String>,
    pub api_key_help_url: State<String>,
    pub api_key_help_text: State<String>,
    pub show_device_flow: State<bool>,
    pub device_flow_provider: State<String>,
    pub device_flow_url: State<String>,
    pub device_flow_code: State<String>,
    pub device_flow_tick: State<usize>,
    pub device_flow_browser_opened: State<bool>,
    pub show_model_selector: State<bool>,
    pub model_selector_provider: State<String>,
    pub model_selector_provider_display: State<String>,
    pub model_selector_models: State<Vec<String>>,
    pub model_selector_cursor: State<usize>,
    pub model_selector_loading: State<bool>,
    pub show_agent_selector: State<bool>,
    pub agent_selector_agents: State<Vec<rustyclaw_view::AgentItemData>>,
    pub agent_selector_active_id: State<String>,
    pub agent_selector_cursor: State<usize>,
    /// Display name of the active agent (overrides the static soul name
    /// in the header after an agent switch).
    pub dynamic_agent_name: State<Option<String>>,
    pub threads: State<Vec<rustyclaw_view::SidebarItemData>>,
    pub projects: State<Vec<rustyclaw_core::ui::ProjectInfo>>,
    pub active_project_id: State<u64>,
    pub tab_focused: State<bool>,
    pub tab_selected: State<usize>,
    pub thread_messages_cache: State<HashMap<u64, Vec<DisplayMessage>>>,
    pub foreground_thread_id: State<Option<u64>>,
    /// Threads with a turn still running.
    ///
    /// The spinner and Esc are gated on `streaming`, which describes the
    /// view and is cleared whenever the view moves. Without a record of what
    /// is actually running, coming back to a conversation that is still
    /// answering showed no indicator and Esc did nothing.
    pub in_flight: State<std::collections::HashSet<u64>>,
    /// Requests waiting for a dialog that is already occupied, oldest first.
    ///
    /// Turns run per thread, so two of them can ask at once. A second
    /// request used to overwrite the dialog's signals — the first was never
    /// shown again, and its timeout was read as a denial of a tool the user
    /// never saw. Each queue drains into its dialog as the dialog frees up.
    pub queued_tool_approvals: State<Vec<(Option<u64>, String, String, String)>>,
    /// The turn whose request the visible approval dialog shows, so a
    /// tool result attributed to another turn cannot tear it down.
    pub tool_approval_thread: State<Option<u64>>,
    pub queued_user_prompts:
        State<Vec<(Option<u64>, rustyclaw_core::user_prompt_types::UserPrompt)>>,
    /// As `tool_approval_thread`, for the visible `ask_user` card.
    pub user_prompt_thread: State<Option<u64>>,
    #[allow(clippy::type_complexity)]
    pub queued_credentials: State<Vec<(Option<u64>, String, String, String, String)>>,
    /// Device-flow prompts waiting for the dialog, oldest first, tagged with
    /// each flow's owner (a turn, an old gateway, or this client itself).
    pub queued_device_flows: State<Vec<(crate::app::DeviceFlowOwner, String, String, String)>>,
    /// The owner of the flow the visible device dialog shows, so a
    /// completion or close-out for any other owner cannot tear it down.
    pub device_flow_owner: State<Option<crate::app::DeviceFlowOwner>>,
    pub command_completions: State<Vec<String>>,
    pub command_selected: State<Option<usize>>,
    pub model_completion_provider: State<Option<String>>,
    pub model_completion_models: State<Vec<String>>,
    pub model_completion_loading: State<Option<String>>,
    /// Hugging Face Hub autocomplete: the query the loaded repo ids
    /// answer, the ids themselves, and the query currently being fetched.
    pub hub_completion_query: State<Option<String>>,
    pub hub_completion_models: State<Vec<String>>,
    pub hub_completion_loading: State<Option<String>>,
    pub prompt_attachments: State<Vec<rustyclaw_view::PromptAttachment>>,
    pub show_secrets_dialog: State<bool>,
    pub secrets_dialog_data: State<Vec<rustyclaw_view::SecretInfoData>>,
    pub secrets_agent_access: State<bool>,
    pub secrets_has_totp: State<bool>,
    pub secrets_selected: State<Option<usize>>,
    pub secrets_scroll_offset: State<usize>,
    pub secrets_add_step: State<u8>,
    pub secrets_add_name: State<String>,
    pub secrets_add_value: State<String>,
    /// Revealed credential: name plus its `(label, value)` pairs. Cleared on
    /// dismiss so plaintext does not outlive the viewer.
    pub secrets_revealed: State<Option<(String, Vec<(String, String)>)>>,
    /// Secret a reveal is in flight for.
    pub secrets_reveal_pending: State<Option<String>>,
    /// TOTP code being typed for the reveal step-up check.
    pub secrets_reveal_code: State<String>,
    /// Whether the gateway asked for a code before revealing.
    pub secrets_reveal_totp_prompt: State<bool>,
    /// Error from the last rejected reveal.
    pub secrets_reveal_error: State<String>,
    pub show_skills_dialog: State<bool>,
    pub skills_dialog_data: State<Vec<rustyclaw_view::SkillInfoData>>,
    pub skills_selected: State<Option<usize>>,
    pub show_details_dialog: State<bool>,
    pub details_dialog_text: State<String>,
    pub details_dialog_is_error: State<bool>,
    pub details_dialog_scroll: State<usize>,
    pub show_tool_perms_dialog: State<bool>,
    pub tool_perms_dialog_data: State<Vec<rustyclaw_view::ToolPermInfoData>>,
    pub tool_perms_selected: State<Option<usize>>,
    pub skills_scroll_offset: State<usize>,
    pub tool_perms_scroll_offset: State<usize>,
    pub host_info: State<Option<rustyclaw_view::HostInfoData>>,
    pub load_status: State<Option<rustyclaw_view::LoadStatusData>>,
    pub show_system_info: State<bool>,
    pub show_downloads_dialog: State<bool>,
    pub downloads_data: State<Option<rustyclaw_view::DownloadsData>>,
    pub downloads_cursor: State<usize>,
    pub show_services_dialog: State<bool>,
    pub services_data: State<Option<rustyclaw_view::ServiceListData>>,
    pub show_engines_dialog: State<bool>,
    pub engines_data: State<Option<rustyclaw_view::EnginesPanelData>>,
    pub engines_cursor: State<usize>,
    /// Parameter-edit mode for the engines dialog (p toggles).
    pub engines_params_edit: State<bool>,
    /// Focused field index within the active engine's parameter list.
    pub engines_params_cursor: State<usize>,
    /// In-progress parameter edits per engine id, seeded from the config the
    /// gateway last reported; Enter saves the active engine's draft.
    pub engines_params_drafts:
        State<std::collections::HashMap<String, rustyclaw_core::engines::EngineConfig>>,
    /// Outcome of the last engine model action (engine, ok, message), shown
    /// in the engines dialog so Load/Unload always answer visibly.
    pub engines_action_result: State<Option<(String, bool, String)>>,
    /// Whether the gateway's `EngineConfigList` snapshot has arrived for the
    /// current connection.  Until it does, the panel's engine configs are
    /// placeholders, so saving parameters would overwrite the real
    /// endpoint/port/models_dir/extra_args with blanks.
    pub engines_configs_received: State<bool>,
    pub show_cron_dialog: State<bool>,
    pub cron_data: State<Option<rustyclaw_view::CronPanelData>>,
    pub show_memory_dialog: State<bool>,
    pub memory_data: State<Option<rustyclaw_view::MemoryPanelData>>,
    pub show_mcp_dialog: State<bool>,
    pub mcp_data: State<Option<rustyclaw_view::McpPanelData>>,
    pub show_channels_dialog: State<bool>,
    pub channels_data: State<Option<rustyclaw_view::ChannelsPanelData>>,
    pub show_messengers_dialog: State<bool>,
    pub messengers_data: State<Option<rustyclaw_view::MessengersPanelData>>,
    pub show_analytics_dialog: State<bool>,
    pub analytics_data: State<Option<rustyclaw_view::AnalyticsPanelData>>,
    pub show_logs_dialog: State<bool>,
    pub logs_data: State<Option<rustyclaw_view::LogsPanelData>>,
}
