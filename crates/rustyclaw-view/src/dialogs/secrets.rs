//! Secrets-management dialog data.

use crate::tone::Tone;

/// The policy cycle order used by the "cycle policy" action:
/// OPEN → ASK → AUTH → SKILL → OPEN.
pub fn next_policy(current: &str) -> &'static str {
    use rustyclaw_core::secrets::AccessPolicy;
    AccessPolicy::from_badge(current)
        .map(|p| p.cycled().badge())
        .unwrap_or("OPEN")
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

impl From<&rustyclaw_core::gateway::client_types::SecretEntryInfo> for SecretInfoData {
    fn from(e: &rustyclaw_core::gateway::client_types::SecretEntryInfo) -> Self {
        Self {
            key: e.name.clone(),
            label: e.label.clone(),
            kind: e.kind.clone(),
            policy: e.policy.clone(),
            disabled: e.disabled,
        }
    }
}

impl From<rustyclaw_core::gateway::client_types::SecretEntryDto> for SecretInfoData {
    fn from(dto: rustyclaw_core::gateway::client_types::SecretEntryDto) -> Self {
        Self::from(&rustyclaw_core::gateway::client_types::SecretEntryInfo::from(dto))
    }
}

impl SecretInfoData {
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
            "TRIGGER" => Tone::Info,
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

    /// The revealed credential: its name and `(label, value)` pairs.
    ///
    /// Held only while the viewer is open — dismissing clears it so a
    /// plaintext secret does not linger in the dialog's state.
    pub revealed: Option<(String, Vec<(String, String)>)>,

    /// Name of the secret a reveal is currently pending for, if any.
    pub reveal_pending: Option<String>,

    /// TOTP code being typed for the step-up check (empty when not prompting).
    pub reveal_code: String,

    /// Whether the gateway asked for a TOTP code before revealing.
    pub reveal_totp_prompt: bool,

    /// Error from the last rejected reveal attempt.
    pub reveal_error: String,
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
            revealed: None,
            reveal_pending: None,
            reveal_code: String::new(),
            reveal_totp_prompt: false,
            reveal_error: String::new(),
        }
    }

    /// Begin revealing `name`; the first request carries no code so the
    /// gateway can say whether one is needed.
    pub fn start_reveal(&mut self, name: &str) {
        self.revealed = None;
        self.reveal_pending = Some(name.to_string());
        self.reveal_code = String::new();
        self.reveal_totp_prompt = false;
        self.reveal_error = String::new();
    }

    /// The gateway wants a TOTP code before it will reveal.
    pub fn require_reveal_code(&mut self, message: Option<String>) {
        self.reveal_totp_prompt = true;
        self.reveal_code = String::new();
        self.reveal_error = message.unwrap_or_default();
    }

    /// Store revealed fields and close the code prompt.
    pub fn finish_reveal(&mut self, fields: Vec<(String, String)>) {
        let name = self.reveal_pending.take().unwrap_or_default();
        self.revealed = Some((name, fields));
        self.reveal_totp_prompt = false;
        self.reveal_code = String::new();
        self.reveal_error = String::new();
    }

    /// Drop any revealed plaintext and cancel a pending reveal.
    pub fn clear_reveal(&mut self) {
        self.revealed = None;
        self.reveal_pending = None;
        self.reveal_code = String::new();
        self.reveal_totp_prompt = false;
        self.reveal_error = String::new();
    }

    /// Whether any part of the reveal flow is on screen.
    pub fn reveal_active(&self) -> bool {
        self.revealed.is_some() || self.reveal_totp_prompt
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
        let new_policy = next_policy(&secret.policy);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> SecretsDialogData {
        SecretsDialogData::from_vault(Vec::new(), false, true)
    }

    #[test]
    fn reveal_flow_carries_name_from_request_to_result() {
        let mut d = dialog();
        d.start_reveal("github-token");
        assert_eq!(d.reveal_pending.as_deref(), Some("github-token"));
        assert!(!d.reveal_active(), "no prompt or value on screen yet");

        d.require_reveal_code(Some("Enter your TOTP code.".into()));
        assert!(d.reveal_totp_prompt);
        assert!(d.reveal_active());

        d.finish_reveal(vec![("Token".into(), "ghp_secret".into())]);
        let (name, fields) = d.revealed.clone().expect("revealed");
        assert_eq!(name, "github-token");
        assert_eq!(fields[0].1, "ghp_secret");
        // The prompt closes and the typed code is not retained.
        assert!(!d.reveal_totp_prompt);
        assert!(d.reveal_code.is_empty());
    }

    #[test]
    fn clear_reveal_drops_plaintext_and_prompt_state() {
        let mut d = dialog();
        d.start_reveal("db-password");
        d.finish_reveal(vec![("Password".into(), "hunter2".into())]);
        d.clear_reveal();
        assert!(d.revealed.is_none(), "plaintext must not outlive dismissal");
        assert!(d.reveal_pending.is_none());
        assert!(!d.reveal_active());
    }

    #[test]
    fn starting_a_new_reveal_clears_the_previous_value() {
        let mut d = dialog();
        d.start_reveal("first");
        d.finish_reveal(vec![("Token".into(), "one".into())]);
        // Selecting another secret must not leave the old plaintext showing.
        d.start_reveal("second");
        assert!(d.revealed.is_none());
        assert_eq!(d.reveal_pending.as_deref(), Some("second"));
    }
}
