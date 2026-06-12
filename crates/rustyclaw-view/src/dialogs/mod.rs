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

mod connection;
pub use connection::*;
mod secrets;
pub use secrets::*;

// ── Tool approval ───────────────────────────────────────────────────────────

/// Data for the tool approval dialog.
///
/// Shown when a tool call requires user approval before execution.
#[derive(Clone, Debug, PartialEq, Default)]
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
#[derive(Clone, Debug, PartialEq, Default)]
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
        matches!(
            self.prompt_type,
            Some(PromptType::Select { .. }) | Some(PromptType::MultiSelect { .. })
        )
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
#[derive(Clone, Debug, PartialEq, Default)]
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
#[derive(Clone, Debug, PartialEq, Default)]
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

// ── Provider selection ───────────────────────────────────────────────────────

/// A single provider choice shown in the provider selector dialog.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProviderOptionData {
    /// Provider identifier used in commands/events.
    pub id: String,

    /// Human-readable provider name.
    pub display_name: String,

    /// Auth method hint ("apikey", "deviceflow", "none", ...).
    pub auth_hint: String,
}

impl ProviderOptionData {
    /// Small status badge shown next to the provider name.
    pub fn auth_badge(&self) -> &'static str {
        match self.auth_hint.as_str() {
            "apikey" => " 🔑",
            "deviceflow" => " 🔗",
            "none" => " ✓",
            _ => "",
        }
    }
}

/// Data for the provider selector dialog.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProviderSelectorData {
    /// Available providers to choose from.
    pub providers: Vec<ProviderOptionData>,

    /// Currently highlighted index.
    pub cursor: usize,
}

impl ProviderSelectorData {
    /// The currently highlighted provider, if any.
    pub fn selected(&self) -> Option<&ProviderOptionData> {
        self.providers.get(self.cursor)
    }
}

// ── API key input ────────────────────────────────────────────────────────────

/// Data for the API-key input dialog.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ApiKeyDialogData {
    /// Provider identifier used when submitting the key.
    pub provider: String,

    /// Human-readable provider label.
    pub provider_display: String,

    /// Current input length (the actual key is not stored here).
    pub input_len: usize,

    /// Optional help URL for getting a key.
    pub help_url: String,

    /// Optional help text for getting a key.
    pub help_text: String,
}

impl ApiKeyDialogData {
    /// Masked display of the current key input.
    pub fn masked_input(&self, width: usize) -> String {
        if self.input_len == 0 {
            "·".repeat(width)
        } else {
            format!(
                "{}{}",
                "•".repeat(self.input_len),
                "·".repeat(width.saturating_sub(self.input_len))
            )
        }
    }

    pub fn has_help(&self) -> bool {
        !self.help_text.is_empty()
    }

    pub fn has_url(&self) -> bool {
        !self.help_url.is_empty()
    }
}

// ── Model selection ──────────────────────────────────────────────────────────

/// Data for the model selector dialog.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModelSelectorData {
    /// Provider identifier used when submitting the model.
    pub provider: String,

    /// Human-readable provider label.
    pub provider_display: String,

    /// Available model names.
    pub models: Vec<String>,

    /// Currently highlighted index.
    pub cursor: usize,

    /// Whether models are still being loaded.
    pub loading: bool,

    /// Spinner tick for loading animation.
    pub spinner_tick: usize,
}

impl ModelSelectorData {
    /// The currently highlighted model, if any.
    pub fn selected_model(&self) -> Option<&str> {
        self.models.get(self.cursor).map(String::as_str)
    }

    /// Visible slice bounds for a compact scrolling list.
    pub fn visible_window(&self, max_visible: usize) -> (usize, usize) {
        let total = self.models.len();
        if total <= max_visible {
            return (0, total);
        }

        let half = max_visible / 2;
        let start = self.cursor.saturating_sub(half);
        let end = (start + max_visible).min(total);
        let start = if end == total {
            total.saturating_sub(max_visible)
        } else {
            start
        };
        (start, end)
    }

    /// Compact "(index/total)" scroll hint for long lists.
    pub fn scroll_hint(&self, max_visible: usize) -> String {
        if self.models.len() > max_visible {
            format!("  ({}/{})", self.cursor + 1, self.models.len())
        } else {
            String::new()
        }
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
#[derive(Clone, Debug, PartialEq, Default)]
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

/// Which field currently has focus in the first-run hatching dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HatchFocus {
    #[default]
    Name,
    Personality,
}

/// Framework-neutral key input understood by the hatching view model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HatchingKey {
    Enter,
    Escape,
    Tab,
    Backspace,
    Char(char),
}

/// Outcome of applying an input event to the hatching view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HatchingEvent {
    Updated,
    Completed(HatchingResult),
    Skipped,
    Ignored,
}

/// Result of the first-run hatching process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HatchingResult {
    pub name: String,
    pub personality: Option<String>,
}

impl HatchingResult {
    /// Encode the result for the existing TUI persistence path.
    pub fn as_payload(&self) -> String {
        match &self.personality {
            Some(personality) => format!("{}\t{}", self.name, personality),
            None => self.name.clone(),
        }
    }
}

/// Data and shared behaviour for the first-run hatching prompt.
///
/// This is intentionally a short input form, not an animation or model-driven
/// conversation. Clients render it differently, but they share the same fields,
/// focus handling, completion semantics, and visibility rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HatchingDialogData {
    /// Whether the dialog is currently eligible to render.
    pub visible: bool,
    /// Whether the user already completed or skipped this prompt in this client
    /// session. Temporary hides for authentication do not set this.
    pub dismissed: bool,
    pub name_input: String,
    pub personality_input: String,
    pub focus: HatchFocus,
}

impl Default for HatchingDialogData {
    fn default() -> Self {
        Self {
            visible: false,
            dismissed: false,
            name_input: String::new(),
            personality_input: String::new(),
            focus: HatchFocus::Name,
        }
    }
}

impl HatchingDialogData {
    pub fn new(visible: bool) -> Self {
        Self {
            visible,
            ..Self::default()
        }
    }

    /// Show the prompt if first-run setup is still needed and it has not been
    /// completed or skipped in this UI session.
    pub fn show_if_needed(&mut self, needs_hatching: bool) {
        if needs_hatching && !self.dismissed {
            self.visible = true;
        }
    }

    /// Temporarily hide the prompt behind a higher-priority modal such as TOTP.
    pub fn hide_temporarily(&mut self) {
        self.visible = false;
    }

    /// Permanently dismiss the prompt for this UI session.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.dismissed = true;
    }

    /// Whether the prompt should render after applying modal priority.
    pub fn should_render(&self, blocked_by_auth: bool) -> bool {
        self.visible && !blocked_by_auth
    }

    pub fn name_focused(&self) -> bool {
        self.focus == HatchFocus::Name
    }

    pub fn completion(&self) -> Option<HatchingResult> {
        let name = self.name_input.trim().to_string();
        if name.is_empty() {
            return None;
        }
        let personality = self.personality_input.trim().to_string();
        Some(HatchingResult {
            name,
            personality: if personality.is_empty() {
                None
            } else {
                Some(personality)
            },
        })
    }

    pub fn handle_key(&mut self, key: HatchingKey) -> HatchingEvent {
        if !self.visible {
            return HatchingEvent::Ignored;
        }

        match key {
            HatchingKey::Enter => {
                self.dismiss();
                self.completion()
                    .map(HatchingEvent::Completed)
                    .unwrap_or(HatchingEvent::Skipped)
            }
            HatchingKey::Escape => {
                self.dismiss();
                HatchingEvent::Skipped
            }
            HatchingKey::Tab => {
                self.focus = match self.focus {
                    HatchFocus::Name => HatchFocus::Personality,
                    HatchFocus::Personality => HatchFocus::Name,
                };
                HatchingEvent::Updated
            }
            HatchingKey::Backspace => {
                if self.name_focused() {
                    self.name_input.pop();
                } else {
                    self.personality_input.pop();
                }
                HatchingEvent::Updated
            }
            HatchingKey::Char(c) => {
                if self.name_focused() {
                    self.name_input.push(c);
                } else {
                    self.personality_input.push(c);
                }
                HatchingEvent::Updated
            }
        }
    }
}

#[cfg(test)]
mod hatching_tests {
    use super::{HatchFocus, HatchingDialogData, HatchingEvent, HatchingKey};

    #[test]
    fn hatching_does_not_render_while_auth_is_blocking() {
        let data = HatchingDialogData::new(true);

        assert!(!data.should_render(true));
        assert!(data.should_render(false));
    }

    #[test]
    fn skipped_hatching_does_not_reopen_in_same_session() {
        let mut data = HatchingDialogData::new(true);

        assert_eq!(data.handle_key(HatchingKey::Escape), HatchingEvent::Skipped);
        data.show_if_needed(true);

        assert!(!data.visible);
        assert!(data.dismissed);
    }

    #[test]
    fn hatching_collects_name_and_personality_in_one_prompt() {
        let mut data = HatchingDialogData::new(true);

        for c in "Ferris".chars() {
            data.handle_key(HatchingKey::Char(c));
        }
        data.handle_key(HatchingKey::Tab);
        assert_eq!(data.focus, HatchFocus::Personality);
        for c in "curious".chars() {
            data.handle_key(HatchingKey::Char(c));
        }

        match data.handle_key(HatchingKey::Enter) {
            HatchingEvent::Completed(result) => {
                assert_eq!(result.name, "Ferris");
                assert_eq!(result.personality.as_deref(), Some("curious"));
            }
            other => panic!("expected completed hatching, got {other:?}"),
        }
        assert!(!data.visible);
        assert!(data.dismissed);
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

    /// Permission label (typically `ALLOW`, `ASK`, or `DENY`).
    pub permission: String,

    /// Short summary of what the tool does.
    pub summary: String,
}

impl ToolPermInfoData {
    /// Whether this tool is currently auto-approved.
    pub fn is_allow(&self) -> bool {
        self.permission.eq_ignore_ascii_case("ALLOW")
    }

    /// Whether this tool is currently denied.
    pub fn is_deny(&self) -> bool {
        self.permission.eq_ignore_ascii_case("DENY")
    }

    /// Whether this tool currently requires per-call approval.
    pub fn is_ask(&self) -> bool {
        self.permission.eq_ignore_ascii_case("ASK")
    }
}
