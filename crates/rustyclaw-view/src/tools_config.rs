//! Component data for the tool configuration/enable-disable panel.

use crate::tone::Tone;

/// Display data for a single tool's config entry.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ToolConfigData {
    pub name: String,
    pub category: String,
    pub enabled: bool,
    pub policy: String,
    pub description: String,
}

impl From<&rustyclaw_core::gateway::protocol::frames::ToolConfigDto> for ToolConfigData {
    fn from(dto: &rustyclaw_core::gateway::protocol::frames::ToolConfigDto) -> Self {
        Self {
            name: dto.name.clone(),
            category: dto.category.clone(),
            enabled: dto.enabled,
            policy: dto.policy.clone(),
            description: dto.description.clone(),
        }
    }
}

impl ToolConfigData {
    /// Tone for the enabled/disabled badge.
    pub fn enabled_tone(&self) -> Tone {
        if self.enabled {
            Tone::Success
        } else {
            Tone::Neutral
        }
    }

    /// Policy tone (reuses secrets policy styling).
    pub fn policy_tone(&self) -> Tone {
        match self.policy.as_str() {
            "OPEN" => Tone::Success,
            "ASK" => Tone::Warning,
            "AUTH" => Tone::Danger,
            "SKILL" => Tone::Info,
            _ => Tone::Neutral,
        }
    }

    /// Category icon.
    pub fn category_icon(&self) -> &'static str {
        match self.category.as_str() {
            "filesystem" => "📁",
            "shell" => "💻",
            "network" | "web" => "🌐",
            "memory" => "🧠",
            "communication" => "💬",
            "media" => "🎨",
            "code" => "📝",
            _ => "🔧",
        }
    }
}

/// Full state for the tool configuration panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ToolConfigPanelData {
    pub tools: Vec<ToolConfigData>,
    pub selected: Option<usize>,
    pub filter_category: Option<String>,
    pub status: Option<String>,
}

impl ToolConfigPanelData {
    /// Tools matching the current filter.
    pub fn filtered_tools(&self) -> Vec<&ToolConfigData> {
        match &self.filter_category {
            Some(cat) => self.tools.iter().filter(|t| &t.category == cat).collect(),
            None => self.tools.iter().collect(),
        }
    }

    /// Unique categories present.
    pub fn categories(&self) -> Vec<&str> {
        let mut cats: Vec<&str> = self
            .tools
            .iter()
            .map(|t| t.category.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        cats.sort();
        cats
    }

    pub fn enabled_count(&self) -> usize {
        self.tools.iter().filter(|t| t.enabled).count()
    }

    pub fn total_count(&self) -> usize {
        self.tools.len()
    }
}
