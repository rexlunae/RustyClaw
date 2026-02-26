use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strum::Display;

/// Actions that drive the application, inspired by openapi-tui.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Display)]
#[strum(serialize_all = "snake_case")]
pub enum Action {
    Tick,
    Render,
    Resize(u16, u16),
    Quit,
    Suspend,
    Resume,
    Error(String),
    Help,
    FocusNext,
    FocusPrev,
    Focus,
    UnFocus,
    Up,
    Down,
    Submit,
    Update,
    Tab(u32),
    ToggleFullScreen,
    StatusLine(String),
    TimedStatusLine(String, u64),
    /// The user submitted text from the input bar (prompt or /command)
    InputSubmit(String),
    /// Request to connect (or reconnect) to the gateway
    ReconnectGateway,
    /// Request to disconnect from the gateway
    DisconnectGateway,
    /// Request to restart the gateway (stop + start)
    RestartGateway,
    /// Send a text message to the gateway (prompt from the input bar)
    SendToGateway(String),
    /// The gateway reader detected a connection drop
    GatewayDisconnected(String),
    /// Toggle the skills dialog overlay
    ShowSkills,
    /// Toggle the secrets dialog overlay
    ShowSecrets,
    /// Copy the currently selected message to clipboard
    CopyMessage,
    /// Show the provider-selection dialog
    ShowProviderSelector,
    /// Set the active provider (triggers auth check + model fetch)
    SetProvider(String),
    /// Open the API-key input dialog for the given provider
    PromptApiKey(String),
    /// The user entered an API key in the dialog — proceed to store confirmation
    ConfirmStoreSecret {
        provider: String,
        key: String,
    },
    /// Fetch models from the provider API, then open the model selector
    FetchModels(String),
    /// The model fetch failed
    FetchModelsFailed(String),
    /// Open the model-selection dialog with a fetched list
    ShowModelSelector {
        provider: String,
        models: Vec<String>,
    },
    /// Begin OAuth device flow authentication for the given provider
    StartDeviceFlow(String),
    /// Device flow: verification URL and user code are ready for display
    DeviceFlowCodeReady {
        url: String,
        code: String,
    },
    /// Device flow authentication succeeded — store the token and proceed
    DeviceFlowAuthenticated {
        provider: String,
        token: String,
    },
    /// Device flow authentication failed
    DeviceFlowFailed(String),
    /// Open the credential-management dialog for a secret
    ShowCredentialDialog {
        name: String,
        disabled: bool,
        policy: String,
    },
    /// Open the 2FA (TOTP) setup / management dialog
    ShowTotpSetup,
    /// Gateway sent an auth_challenge — prompt user for TOTP code
    GatewayAuthChallenge,
    /// User submitted a TOTP code for gateway authentication
    GatewayAuthResponse(String),
    /// Gateway vault is locked — prompt user for password
    GatewayVaultLocked,
    /// User submitted a password to unlock the gateway vault
    GatewayUnlockVault(String),
    /// Close the hatching animation and transition to normal TUI
    CloseHatching,
    /// Begin the hatching exchange — send the identity prompt to the gateway
    BeginHatchingExchange,
    /// A gateway response routed to the hatching exchange
    HatchingResponse(String),
    /// The hatching exchange is complete — save SOUL.md and close
    FinishHatching(String),
    /// Gateway returned the secrets list
    SecretsListResult {
        entries: Vec<rustyclaw_core::gateway::SecretEntryDto>,
    },
    /// Gateway returned a secret value (for provider probing, device flow, etc.)
    SecretsGetResult {
        key: String,
        value: Option<String>,
    },
    /// Gateway stored a secret successfully
    SecretsStoreResult {
        ok: bool,
        message: String,
    },
    /// Gateway returned peek result (for secret viewer)
    SecretsPeekResult {
        name: String,
        ok: bool,
        fields: Vec<(String, String)>,
        message: Option<String>,
    },
    /// Gateway set policy result
    SecretsSetPolicyResult {
        ok: bool,
        message: Option<String>,
    },
    /// Gateway set disabled result
    SecretsSetDisabledResult {
        ok: bool,
        cred_name: String,
        disabled: bool,
    },
    /// Gateway deleted credential result
    SecretsDeleteCredentialResult {
        ok: bool,
        cred_name: String,
    },
    /// Gateway returned TOTP status
    SecretsHasTotpResult {
        has_totp: bool,
    },
    /// Gateway returned TOTP setup URI
    SecretsSetupTotpResult {
        ok: bool,
        uri: Option<String>,
        message: Option<String>,
    },
    /// Gateway returned TOTP verification result
    SecretsVerifyTotpResult {
        ok: bool,
    },
    /// Gateway returned TOTP removal result
    SecretsRemoveTotpResult {
        ok: bool,
    },
    /// Gateway stream started (API connected, waiting for response)
    GatewayStreamStart,
    /// Gateway extended thinking started
    GatewayThinkingStart,
    /// Gateway extended thinking delta (update loading indicator)
    GatewayThinkingDelta,
    /// Gateway extended thinking ended
    GatewayThinkingEnd,
    /// Gateway sent a text chunk
    GatewayChunk(String),
    /// Gateway response is complete
    GatewayResponseDone,
    /// Gateway sent a tool call from the model
    GatewayToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Gateway sent a tool result from execution
    GatewayToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },
    /// Gateway authenticated successfully
    GatewayAuthenticated,
    /// Gateway vault unlocked successfully
    GatewayVaultUnlocked,
    /// Info message
    Info(String),
    /// Success message
    Success(String),
    /// Warning message
    Warning(String),
    /// Show the tool permissions editor dialog
    ShowToolPermissions,
    /// Save updated tool permissions to config
    SaveToolPermissions(HashMap<String, rustyclaw_core::tools::ToolPermission>),
    /// Gateway is requesting user approval to run a tool (Ask mode)
    ToolApprovalRequest {
        id: String,
        name: String,
        arguments: String,
    },
    /// User responded to a tool approval request
    ToolApprovalResponse {
        id: String,
        approved: bool,
    },
    /// Gateway is requesting structured user input (ask_user tool)
    UserPromptRequest(rustyclaw_core::user_prompt_types::UserPrompt),
    /// User responded to a structured prompt
    UserPromptResponse(rustyclaw_core::user_prompt_types::UserPromptResponse),
    /// Gateway sent a tasks update
    TasksUpdate(Vec<TaskInfo>),
    /// A long-running slash-command tool finished (msg, is_error)
    ToolCommandDone {
        message: String,
        is_error: bool,
    },
    Noop,
}

/// Task info for TUI display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: u64,
    pub label: String,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
}
