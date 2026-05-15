//! Component data for modal dialogs.
//!
//! Each dialog type (tool approval, auth, vault unlock, etc.) gets
//! its own data struct describing exactly what the dialog needs to
//! render.  Event handlers (submit, cancel, dismiss) are provided
//! by the framework-specific wrapper — these are pure data.
//!
//! Methods on these structs centralise display formatting so that
//! both the desktop and TUI derive the same labels, summaries, and
//! preview text without duplicating logic.

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

impl ToolApprovalData {
    /// A one-line summary header, e.g. `"🔧 write_file"`.
    pub fn summary(&self) -> String {
        format!("🔧 {}", self.name)
    }

    /// Arguments truncated at `max_chars` characters for compact display.
    ///
    /// Also limits to `max_lines` lines.  Useful for the tool-approval
    /// preview area.
    pub fn arguments_preview(&self, max_chars: usize, max_lines: usize) -> String {
        rustyclaw_core::ui::truncate_content(&self.arguments, max_chars, max_lines)
    }
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

impl AuthDialogData {
    /// Whether the current code is a complete 6-digit TOTP.
    pub fn is_complete(&self) -> bool {
        self.code.len() == 6
    }

    /// Masked display of the code, e.g. `"• • • • • •"`.
    ///
    /// Shows filled dots for entered digits and hollow dots for
    /// remaining positions.
    pub fn masked_code(&self) -> String {
        let entered: String = self.code.chars().map(|_| '●').collect();
        let remaining_count = 6usize.saturating_sub(self.code.len());
        let remaining = "○".repeat(remaining_count);
        let combined = format!("{}{}", entered, remaining);
        // Space-separated for readability
        combined
            .chars()
            .map(|c| format!("{} ", c))
            .collect::<String>()
            .trim()
            .to_string()
    }
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

impl VaultUnlockData {
    /// Masked representation of the current password, e.g. `"••••••"`.
    pub fn masked_password(&self) -> String {
        if self.password_len == 0 {
            String::new()
        } else {
            "•".repeat(self.password_len)
        }
    }

    /// Whether a password has been entered (any non-zero length).
    pub fn has_input(&self) -> bool {
        self.password_len > 0
    }
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

impl UserPromptData {
    /// Human-readable label for the prompt type.
    ///
    /// Maps `PromptType::Confirm` → `"confirm"`,
    /// `PromptType::Text` → `"text input"`, etc.
    pub fn prompt_type_label(&self) -> &'static str {
        match self.prompt_type {
            Some(PromptType::Confirm { .. }) => "confirm",
            Some(PromptType::TextInput { .. }) => "text input",
            Some(PromptType::Select { .. }) => "select",
            Some(PromptType::MultiSelect { .. }) => "multi-select",
            Some(PromptType::Form { .. }) => "form",
            None => "—",
        }
    }

    /// Whether this prompt type accepts free-form text input.
    pub fn is_text_input(&self) -> bool {
        matches!(self.prompt_type, Some(PromptType::TextInput { .. }))
    }

    /// Whether this prompt expects a simple confirm/deny response.
    pub fn is_confirm(&self) -> bool {
        matches!(self.prompt_type, Some(PromptType::Confirm { .. }))
    }

    /// Whether this prompt expects selection from options.
    pub fn is_selection(&self) -> bool {
        matches!(self.prompt_type, Some(PromptType::Select { .. }) | Some(PromptType::MultiSelect { .. }))
    }

    /// Whether this prompt is a multi-field form.
    pub fn is_form(&self) -> bool {
        matches!(self.prompt_type, Some(PromptType::Form { .. }))
    }
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

impl CredentialRequestData {
    /// A one-line summary, e.g. `"Provide API key for anthropic"`.
    pub fn summary(&self) -> String {
        format!("🔑 {} — {}", self.secret_name, self.provider)
    }

    /// Masked representation of the current input.
    pub fn masked_input(&self) -> String {
        if self.input_len == 0 {
            String::new()
        } else {
            "•".repeat(self.input_len)
        }
    }

    /// Whether the user has entered any characters.
    pub fn has_input(&self) -> bool {
        self.input_len > 0
    }
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

impl DeviceFlowData {
    /// Display string with the code prominently, suitable for terminal output.
    ///
    /// Formats as `"Code: XXXXXX  |  URL: {url}"`.
    pub fn display_with_code(&self) -> String {
        format!("Code: {}  |  URL: {}", self.code, self.url)
    }
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

impl PairingStep {
    /// Human-readable label for the current step.
    pub fn label(&self) -> &'static str {
        match self {
            PairingStep::ShowKey => "Show public key",
            PairingStep::EnterGateway => "Enter gateway address",
            PairingStep::Connecting => "Connecting…",
            PairingStep::Complete => "Pairing complete",
        }
    }

    /// Whether the step represents an in-progress state (not idle or done).
    pub fn is_progress(&self) -> bool {
        matches!(self, PairingStep::Connecting)
    }

    /// Whether the step is the final/completed state.
    pub fn is_complete(&self) -> bool {
        matches!(self, PairingStep::Complete)
    }
}

/// Input fields in the gateway entry step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PairingField {
    #[default]
    Host,
    Port,
}

impl PairingField {
    /// Human-readable label for this field.
    pub fn label(&self) -> &'static str {
        match self {
            PairingField::Host => "Host",
            PairingField::Port => "Port",
        }
    }
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

impl PairingDialogData {
    /// The gateway address in `"host:port"` format, defaulting port to `"2222"`.
    pub fn gateway_address(&self) -> String {
        if self.port.is_empty() || self.port == "0" {
            format!("{}:2222", self.host)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
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

impl HatchState {
    /// Human-readable description of the current state.
    pub fn label(&self) -> &'static str {
        match self {
            HatchState::Egg => "Egg",
            HatchState::Crack1 => "Cracking…",
            HatchState::Crack2 => "Almost there…",
            HatchState::Breaking => "Breaking out!",
            HatchState::Hatched => "Hatched!",
            HatchState::Connecting => "Connecting…",
            HatchState::Awakened { .. } => "Awakened!",
        }
    }

    /// Whether the hatching sequence is active (before Awakened).
    pub fn is_hatching(&self) -> bool {
        matches!(
            self,
            HatchState::Egg
                | HatchState::Crack1
                | HatchState::Crack2
                | HatchState::Breaking
                | HatchState::Hatched
        )
    }
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

impl HatchingDialogData {
    /// Whether the dialog should show the loading/spinner state.
    pub fn is_busy(&self) -> bool {
        self.pending || matches!(self.state, HatchState::Connecting)
    }
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

impl SecretInfoData {
    /// Icon/indicator for the secret type.
    pub fn type_icon(&self) -> &'static str {
        if self.is_totp { "🔐" } else { "🔑" }
    }
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

impl SkillInfoData {
    /// Toggle label, e.g. `"Enable"` or `"Disable"`.
    pub fn toggle_label(&self) -> &'static str {
        if self.enabled { "Disable" } else { "Enable" }
    }
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

impl ToolPermInfoData {
    /// Access label, e.g. `"auto-approved"` or `"requires approval"`.
    pub fn access_label(&self) -> &'static str {
        if self.auto_approve { "auto-approved" } else { "requires approval" }
    }
}
