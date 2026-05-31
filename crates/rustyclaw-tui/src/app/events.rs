//! TUI gateway event types.
//!
//! These are the events the gateway-reader task sends into the iocraft
//! render loop. They are TUI-local because they describe UI state and
//! dialog prompts, not the wire protocol.

use rustyclaw_view::PromptAttachment;

/// Events pushed from the gateway reader into the iocraft render component.
#[derive(Debug, Clone)]
pub(crate) enum GwEvent {
    Disconnected(String),
    Connected,
    AuthChallenge,
    Authenticated,
    ModelReady(String),
    /// Gateway reloaded config — update model label in status bar.
    ModelReloaded {
        provider: String,
        model: String,
    },
    Info(String),
    Success(String),
    /// A non-fatal warning.
    Warning {
        summary: String,
        details: Option<String>,
    },
    /// An error.
    Error {
        summary: String,
        details: Option<String>,
    },
    StreamStart,
    Chunk(String),
    ResponseDone,
    ThinkingStart,
    ThinkingDelta,
    ThinkingEnd,
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },
    /// Gateway requests user approval for a tool call (Ask mode).
    ToolApprovalRequest {
        id: String,
        name: String,
        arguments: String,
    },
    /// Gateway requests structured user input (ask_user tool).
    UserPromptRequest(rustyclaw_core::user_prompt_types::UserPrompt),
    /// Gateway requests an API key or credential from the user.
    CredentialRequest {
        id: String,
        provider: String,
        secret_name: String,
        message: String,
    },
    /// Vault is locked — user needs to provide password.
    VaultLocked,
    /// Vault was successfully unlocked.
    VaultUnlocked,
    /// Show secrets info dialog.
    ShowSecrets {
        secrets: Vec<rustyclaw_view::SecretInfoData>,
        agent_access: bool,
        has_totp: bool,
    },
    /// Show skills info dialog.
    ShowSkills {
        skills: Vec<rustyclaw_view::SkillInfoData>,
    },
    /// Show tool permissions info dialog.
    ShowToolPerms {
        tools: Vec<rustyclaw_view::ToolPermInfoData>,
    },
    /// A secrets mutation succeeded — re-fetch the list from the gateway.
    RefreshSecrets,
    /// Thread list update from gateway (unified tasks + threads).
    ThreadsUpdate {
        threads: Vec<crate::action::ThreadInfo>,
        foreground_id: Option<u64>,
    },

    /// Project list update (the sidebar's top level).
    ProjectsUpdate {
        projects: Vec<rustyclaw_core::ui::ProjectInfo>,
        active_id: u64,
    },

    /// Live message/history update for a thread.
    ThreadMessages {
        #[allow(dead_code)]
        thread_id: u64,
        messages: Vec<rustyclaw_core::gateway::protocol::types::ChatMessage>,
    },

    /// Thread switch confirmed — clear messages and show context.
    ThreadSwitched {
        thread_id: u64,
        context_summary: Option<String>,
    },
    /// Authoritative thread history from the gateway, in chronological order.
    ThreadHistory {
        thread_id: u64,
        ok: bool,
        messages: Vec<rustyclaw_core::gateway::protocol::types::ChatMessage>,
        error: Option<String>,
    },
    /// Open the provider selector dialog.
    ShowProviderSelector {
        providers: Vec<String>,
        provider_ids: Vec<String>,
        auth_hints: Vec<String>,
    },
    /// Open the API key input dialog.
    PromptApiKey {
        provider: String,
        provider_display: String,
        help_url: String,
        help_text: String,
    },
    /// Show the device flow verification dialog.
    DeviceFlowCode {
        provider: String,
        url: String,
        code: String,
    },
    /// Device flow completed — dismiss dialog and store token.
    DeviceFlowDone,
    /// Device flow succeeded — store token and proceed to model selection.
    DeviceFlowToken {
        provider: String,
        token: String,
    },
    /// Open the model selector dialog.
    ShowModelSelector {
        provider: String,
        provider_display: String,
        models: Vec<String>,
    },
    /// Prompt attachment list changed.
    PromptAttachmentsChanged {
        attachments: Vec<PromptAttachment>,
    },
    /// Live model IDs loaded for slash-command autocomplete.
    ModelCompletionsLoaded {
        provider: String,
        models: Vec<String>,
    },
    /// Model fetch is in progress (show loading spinner).
    FetchModelsLoading {
        provider: String,
        provider_display: String,
    },
    /// SSH pairing connection succeeded.
    PairingSuccess {
        gateway_name: String,
    },
    /// SSH pairing connection failed.
    PairingError(String),
}

impl GwEvent {
    /// Warning event with no extended details.
    pub fn warning(summary: impl Into<String>) -> Self {
        GwEvent::Warning {
            summary: summary.into(),
            details: None,
        }
    }

    /// Error event with no extended details.
    pub fn error(summary: impl Into<String>) -> Self {
        GwEvent::Error {
            summary: summary.into(),
            details: None,
        }
    }

    /// Error event from an `anyhow_tracing::Error`.
    pub fn error_from_err(err: &anyhow_tracing::Error) -> Self {
        GwEvent::Error {
            summary: format!("{:#}", err),
            details: Some(rustyclaw_core::error_details::render_extended(err)),
        }
    }
}
