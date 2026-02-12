use serde::{Deserialize, Serialize};
use strum::Display;

/// Actions that drive the application, inspired by openapi-tui.
#[derive(Debug, Clone, PartialEq, Serialize, Display, Deserialize)]
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
    /// A message received from the gateway
    GatewayMessage(String),
    /// The gateway reader detected a connection drop
    GatewayDisconnected(String),
    /// Toggle the skills dialog overlay
    ShowSkills,
    /// Show the provider-selection dialog
    ShowProviderSelector,
    /// Set the active provider (triggers auth check + model fetch)
    SetProvider(String),
    /// Open the API-key input dialog for the given provider
    PromptApiKey(String),
    /// The user entered an API key in the dialog — proceed to store confirmation
    ConfirmStoreSecret { provider: String, key: String },
    /// Fetch models from the provider API, then open the model selector
    FetchModels(String),
    /// The model fetch failed
    FetchModelsFailed(String),
    /// Open the model-selection dialog with a fetched list
    ShowModelSelector { provider: String, models: Vec<String> },
    /// Begin OAuth device flow authentication for the given provider
    StartDeviceFlow(String),
    /// Device flow: verification URL and user code are ready for display
    DeviceFlowCodeReady { url: String, code: String },
    /// Device flow authentication succeeded — store the token and proceed
    DeviceFlowAuthenticated { provider: String, token: String },
    /// Device flow authentication failed
    DeviceFlowFailed(String),
    /// Open the credential-management dialog for a secret
    ShowCredentialDialog { name: String, disabled: bool, policy: String },
    /// Open the 2FA (TOTP) setup / management dialog
    ShowTotpSetup,
    /// Close the hatching animation and transition to normal TUI
    CloseHatching,
    Noop,
}
