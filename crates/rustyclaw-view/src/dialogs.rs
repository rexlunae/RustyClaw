//! Component data for modal dialogs.
//!
//! Each dialog type (tool approval, auth, vault unlock, etc.) gets
//! its own data struct describing exactly what the dialog needs to
//! render.  Event handlers (submit, cancel, dismiss) are provided
//! by the framework-specific wrapper — these are pure data.

use rustyclaw_core::user_prompt_types::{PromptType, UserPrompt};

// ── Tool approval ───────────────────────────────────────────────────────────

/// Data for the tool approval dialog.
///
/// Shown when a tool call requires user approval before execution.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolApprovalData {
    /// Tool call ID (matches the approval response flow).
    pub id: String,

    /// Tool name (e.g. "write_file", "web_search").
    pub name: String,

    /// Pretty-printed JSON arguments.
    pub arguments: String,

    /// Whether "Allow" is currently selected (vs "Deny").
    pub selected_allow: bool,
}

// ── TOTP authentication ─────────────────────────────────────────────────────

/// Data for the TOTP authentication dialog.
///
/// Shown when the gateway requires a 6-digit TOTP code.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AuthDialogData {
    /// The digits entered so far (0–6 characters).
    pub code: String,

    /// Optional error/status message (e.g. "Invalid code, try again").
    pub error: String,
}

// ── Vault unlock ────────────────────────────────────────────────────────────

/// Data for the vault unlock dialog.
///
/// Shown when the gateway's encrypted vault is locked and needs a
/// password to decrypt.  The actual password is never stored in
/// props — only the length (for masked display) and any error message.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VaultUnlockData {
    /// Length of the current password input (for masked display).
    pub password_len: usize,

    /// Optional error/status message.
    pub error: String,
}

// ── User prompt ─────────────────────────────────────────────────────────────

/// Data for the user prompt dialog.
///
/// Shown when the agent asks the user a question via the `ask_user`
/// tool (confirm, text input, select, multi-select).
#[derive(Clone, Debug, PartialEq)]
pub struct UserPromptData {
    /// The question title from the agent.
    pub title: String,

    /// Optional longer description.
    pub description: String,

    /// Current user input text (for text input types).
    pub input: String,

    /// Selected option index (for Select/MultiSelect).
    pub selected: usize,

    /// The type of input requested.
    pub prompt_type: Option<PromptType>,
}

impl From<UserPrompt> for UserPromptData {
    fn from(prompt: UserPrompt) -> Self {
        Self {
            title: prompt.title,
            description: prompt.description.unwrap_or_default(),
            input: String::new(),
            selected: 0,
            prompt_type: Some(prompt.prompt_type),
        }
    }
}

// ── Credential request ──────────────────────────────────────────────────────

/// Data for the credential request dialog.
///
/// Shown when the gateway needs an API key or other credential.
/// The actual credential value is never stored in props — only
/// the input length (for masked display).
#[derive(Clone, Debug, PartialEq)]
pub struct CredentialRequestData {
    /// The provider that needs a credential (e.g. "openai", "anthropic").
    pub provider: String,

    /// The name of the secret being requested.
    pub secret_name: String,

    /// Human-readable message explaining what is needed.
    pub message: String,

    /// Length of the current input (masked as dots).
    pub input_len: usize,
}

// ── Device flow ─────────────────────────────────────────────────────────────

/// Data for the device-flow OAuth dialog.
///
/// Shown when a provider requires browser-based OAuth via a
/// device code flow (user visits a URL and enters a code).
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceFlowData {
    /// The verification URL the user should visit.
    pub url: String,

    /// The one-time user code to enter on that page.
    pub code: String,

    /// Optional message from the provider that triggered the flow.
    pub message: Option<String>,

    /// Whether the browser was already opened automatically.
    pub browser_opened: bool,

    /// Spinner tick for the waiting animation.
    pub tick: usize,
}

// ── Pairing ─────────────────────────────────────────────────────────────────

/// Steps in the SSH gateway pairing wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairingStep {
    #[default]
    ShowKey,
    EnterGateway,
    Connecting,
    Complete,
}

/// Input fields in the gateway entry step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairingField {
    #[default]
    Host,
    Port,
}

/// Data for the SSH pairing wizard dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct PairingDialogData {
    /// Current step in the pairing flow.
    pub step: PairingStep,

    /// Which field is active (Host or Port).
    pub field: PairingField,

    /// The client's public key in OpenSSH format.
    pub public_key: String,

    /// The key fingerprint (SHA256:...).
    pub fingerprint: String,

    /// ASCII art fingerprint visualization.
    pub fingerprint_art: String,

    /// QR code ASCII art (optional).
    pub qr_ascii: String,

    /// Gateway host address (for the EnterGateway step).
    pub host: String,

    /// Gateway port (for the EnterGateway step).
    pub port: String,

    /// Optional error message.
    pub error: String,
}

// ── Hatching ────────────────────────────────────────────────────────────────

/// Animation states for the first-run identity hatching sequence.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum HatchState {
    #[default]
    Egg,
    Crack1,
    Crack2,
    Breaking,
    Hatched,
    /// Waiting for model response.
    Connecting,
    /// Model generated its identity.
    Awakened {
        identity: String,
    },
}

/// Data for the hatching dialog (first-run identity generation).
#[derive(Clone, Debug, PartialEq)]
pub struct HatchingDialogData {
    /// Current animation state.
    pub state: HatchState,

    /// Animation tick counter.
    pub tick: usize,

    /// Whether a hatching API request is in flight.
    pub pending: bool,
}

// ── Secrets dialog ──────────────────────────────────────────────────────────

/// A single secret entry shown in the secrets management dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct SecretInfoData {
    /// The secret key/name.
    pub key: String,

    /// Whether the agent has access to this secret.
    pub agent_access: bool,

    /// Whether this is a TOTP token (vs raw API key).
    pub is_totp: bool,
}

// ── Skills dialog ───────────────────────────────────────────────────────────

/// A single skill entry shown in the skills management dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillInfoData {
    /// Skill name.
    pub name: String,

    /// Short description.
    pub description: String,

    /// Whether the skill is currently enabled.
    pub enabled: bool,
}

// ── Tool permissions dialog ─────────────────────────────────────────────────

/// A single tool permission entry shown in the tool perms dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPermInfoData {
    /// Tool name.
    pub name: String,

    /// Whether the tool is allowed to auto-run without approval.
    pub auto_approve: bool,

    /// How many times this tool has been called this session.
    pub call_count: usize,
}
