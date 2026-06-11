//! Secrets-management dialog data.

use crate::tone::Tone;

/// The policy cycle order used by the "cycle policy" action:
/// OPEN → ASK → AUTH → SKILL → OPEN.
pub fn next_policy(current: &str) -> &'static str {
    match current {
        "OPEN" => "ASK",
        "ASK" => "AUTH",
        "AUTH" => "SKILL",
        "SKILL" => "OPEN",
        _ => "OPEN",
    }
}

/// Legend entries for the policy badges: `(policy, tone, meaning)`.
pub const POLICY_LEGEND: [(&str, Tone, &str); 5] = [
    ("OPEN", Tone::Success, "anytime"),
    ("ASK", Tone::Warning, "per-use"),
    ("AUTH", Tone::Danger, "re-auth"),
    ("SKILL", Tone::Info, "gated"),
    ("OFF", Tone::Neutral, "disabled"),
];

// ── Secrets dialog ──────────────────────────────────────────────────────────

/// A single secret entry shown in the secrets management dialog.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SecretInfoData {
    /// The secret key/name.
    pub key: String,

    /// Human-readable label.
    pub label: String,

    /// Kind/category (api_key, token, username_password, etc.).
    pub kind: String,

    /// Access policy (OPEN, ASK, AUTH, SKILL, DISABLED).
    pub policy: String,

    /// Whether the secret is disabled.
    pub disabled: bool,
}

impl SecretInfoData {
    /// Convert from the gateway protocol DTO.
    pub fn from_entry_info(e: &rustyclaw_core::gateway::client_types::SecretEntryInfo) -> Self {
        Self {
            key: e.name.clone(),
            label: e.label.clone(),
            kind: e.kind.clone(),
            policy: e.policy.clone(),
            disabled: e.disabled,
        }
    }

    /// Convert directly from a SecretEntryDto (gateway frame).
    pub fn from_dto(dto: rustyclaw_core::gateway::client_types::SecretEntryDto) -> Self {
        Self {
            key: dto.name,
            label: dto.label,
            kind: dto.kind,
            policy: dto.policy,
            disabled: dto.disabled,
        }
    }

    /// Icon/indicator for the secret type.
    pub fn type_icon(&self) -> &'static str {
        match self.kind.as_str() {
            "token" | "api_key" => "🔑",
            "ssh_key" => "🔐",
            "username_password" => "👤",
            "payment_method" => "💳",
            _ => "🔑",
        }
    }

    /// Policy display label with color hint.
    pub fn policy_label(&self) -> &str {
        if self.disabled { "OFF" } else { &self.policy }
    }

    /// Semantic tone for the policy badge.
    pub fn policy_tone(&self) -> Tone {
        if self.disabled {
            return Tone::Neutral;
        }
        match self.policy.as_str() {
            "OPEN" => Tone::Success,
            "ASK" => Tone::Warning,
            "AUTH" => Tone::Danger,
            "SKILL" => Tone::Info,
            _ => Tone::Neutral,
        }
    }
}

/// Full state for the secrets management dialog.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SecretsDialogData {
    /// All secrets currently in the vault.
    pub secrets: Vec<SecretInfoData>,

    /// Whether the agent has access to secrets.
    pub agent_access: bool,

    /// Whether any secret has TOTP 2FA set up.
    pub has_totp: bool,

    /// Currently selected index (for keyboard navigation).
    pub selected: Option<usize>,

    /// Scroll offset for the secrets list.
    pub scroll_offset: usize,

    /// Add-secret workflow step (0=idle, 1=entering name, 2=entering value).
    pub add_step: u8,

    /// Name being entered for a new secret.
    pub add_name: String,

    /// Value being entered for a new secret.
    pub add_value: String,

    /// Optional status/error message to display.
    pub status: Option<String>,
}

impl SecretsDialogData {
    /// Create a new secrets dialog from raw gateway data.
    pub fn from_vault(secrets: Vec<SecretInfoData>, agent_access: bool, has_totp: bool) -> Self {
        Self {
            secrets,
            agent_access,
            has_totp,
            selected: None,
            scroll_offset: 0,
            add_step: 0,
            add_name: String::new(),
            add_value: String::new(),
            status: None,
        }
    }

    /// Number of visible secrets (excluding disabled/ui helpers).
    pub fn count(&self) -> usize {
        self.secrets.len()
    }

    /// Get the currently selected secret, if any.
    pub fn selected_secret(&self) -> Option<&SecretInfoData> {
        self.selected.and_then(|i| self.secrets.get(i))
    }

    /// Move selection up.
    pub fn select_prev(&mut self) {
        let max = self.secrets.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur == 0 { max } else { cur - 1 });
        // Adjust scroll
        if let Some(sel) = self.selected {
            if sel < self.scroll_offset {
                self.scroll_offset = sel;
            }
        }
    }

    /// Move selection down.
    pub fn select_next(&mut self) {
        let max = self.secrets.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur >= max { 0 } else { cur + 1 });
        // Adjust scroll
        if let Some(sel) = self.selected {
            if sel >= self.scroll_offset + 20 {
                self.scroll_offset = sel.saturating_sub(19);
            }
        }
    }

    /// Start the add-secret flow (step 1 = name entry).
    pub fn start_add(&mut self) {
        self.add_step = 1;
        self.add_name = String::new();
        self.add_value = String::new();
    }

    /// Advance to the next add step or commit.
    pub fn advance_add(&mut self) -> Option<(String, String)> {
        match self.add_step {
            1 => {
                self.add_step = 2;
                None
            }
            2 => {
                let name = std::mem::take(&mut self.add_name);
                let value = std::mem::take(&mut self.add_value);
                self.add_step = 0;
                Some((name, value))
            }
            _ => None,
        }
    }

    /// Cancel the add-secret flow.
    pub fn cancel_add(&mut self) {
        self.add_step = 0;
        self.add_name = String::new();
        self.add_value = String::new();
    }

    /// Whether the add flow is active.
    pub fn is_adding(&self) -> bool {
        self.add_step > 0
    }

    /// Append a character to the current add input.
    pub fn add_char(&mut self, c: char) {
        if self.add_step == 1 {
            self.add_name.push(c);
        } else if self.add_step == 2 {
            self.add_value.push(c);
        }
    }

    /// Delete last character from the current add input.
    pub fn add_backspace(&mut self) {
        if self.add_step == 1 {
            self.add_name.pop();
        } else if self.add_step == 2 {
            self.add_value.pop();
        }
    }

    /// Cycle the policy of the currently selected secret.
    /// Returns the new policy if changed.
    pub fn cycle_policy(&mut self) -> Option<String> {
        let sel = self.selected?;
        let secret = self.secrets.get_mut(sel)?;
        if secret.disabled {
            return None;
        }
        let new_policy = match secret.policy.as_str() {
            "OPEN" => "ASK",
            "ASK" => "AUTH",
            "AUTH" => "SKILL",
            "SKILL" => "OPEN",
            _ => "OPEN",
        };
        secret.policy = new_policy.to_string();
        Some(new_policy.to_string())
    }

    /// Remove the currently selected secret.
    /// Returns the key to delete.
    pub fn delete_selected(&mut self) -> Option<String> {
        let sel = self.selected?;
        if sel < self.secrets.len() {
            let removed = self.secrets.remove(sel);
            // Adjust selection
            if self.secrets.is_empty() {
                self.selected = None;
            } else if sel >= self.secrets.len() {
                self.selected = Some(self.secrets.len() - 1);
            }
            Some(removed.key)
        } else {
            None
        }
    }
}
