// ── Dialog state types ──────────────────────────────────────────────────────
//
// State structs for modal overlays. These are kept minimal — the actual
// dialog iocraft components will be added as needed.

pub use rustyclaw_core::user_prompt_types::{
    FormField, PromptOption, PromptResponseValue, PromptType, UserPrompt, UserPromptResponse,
};

// ── Tool approval ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolApprovalState {
    pub id: String,
    pub name: String,
    pub arguments: String,
    pub selected_allow: bool,
}

impl ToolApprovalState {
    pub fn new(id: String, name: String, arguments: String) -> Self {
        Self {
            id,
            name,
            arguments,
            selected_allow: true,
        }
    }
}

// ── Provider / model selection ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ProviderSelectorState {
    pub providers: Vec<String>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct ApiKeyDialogState {
    pub provider: String,
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct ModelSelectorState {
    pub provider: String,
    pub models: Vec<String>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct FetchModelsLoading {
    pub provider: String,
    pub tick: usize,
}

// ── Credentials ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CredentialDialogState {
    pub credentials: Vec<CredDialogOption>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct CredDialogOption {
    pub name: String,
    pub provider: String,
    pub has_totp: bool,
}

#[derive(Debug, Clone)]
pub struct PolicyPickerState {
    pub credential: String,
    pub cursor: usize,
}

// ── TOTP ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TotpDialogState {
    pub credential: String,
    pub phase: TotpDialogPhase,
}

#[derive(Debug, Clone)]
pub enum TotpDialogPhase {
    Setup {
        qr_lines: Vec<String>,
        secret: String,
    },
    Verify {
        input: String,
        error: Option<String>,
    },
    Remove {
        confirmed: bool,
    },
}

// ── Auth / vault ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AuthPromptState {
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct VaultUnlockPromptState {
    pub input: String,
    pub error: Option<String>,
}

// ── Secret viewer ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SecretViewerState {
    pub key: String,
    pub value: String,
    pub revealed: bool,
}

// ── Tool permissions ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolPermissionsState {
    pub tools: Vec<ToolPermissionEntry>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct ToolPermissionEntry {
    pub name: String,
    pub permission: String,
}

// ── User prompt ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct UserPromptState {
    pub prompt: UserPrompt,
    pub phase: UserPromptPhase,
}

#[derive(Debug, Clone)]
pub enum UserPromptPhase {
    Select { cursor: usize },
    MultiSelect { cursor: usize, selected: Vec<bool> },
    Confirm { yes: bool },
    TextInput { input: String },
    Form { cursor: usize, inputs: Vec<String> },
}

impl UserPromptState {
    pub fn new(prompt: UserPrompt) -> Self {
        let phase = match &prompt.prompt_type {
            PromptType::Select { default, options } => UserPromptPhase::Select {
                cursor: default.unwrap_or(0).min(options.len().saturating_sub(1)),
            },
            PromptType::MultiSelect { defaults, options } => {
                let mut selected = vec![false; options.len()];
                for &i in defaults {
                    if i < selected.len() {
                        selected[i] = true;
                    }
                }
                UserPromptPhase::MultiSelect {
                    cursor: 0,
                    selected,
                }
            }
            PromptType::Confirm { default } => UserPromptPhase::Confirm { yes: *default },
            PromptType::TextInput { default, .. } => UserPromptPhase::TextInput {
                input: default.clone().unwrap_or_default(),
            },
            PromptType::Form { fields } => UserPromptPhase::Form {
                cursor: 0,
                inputs: fields
                    .iter()
                    .map(|f| f.default.clone().unwrap_or_default())
                    .collect(),
            },
        };
        Self { prompt, phase }
    }
}

pub const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
