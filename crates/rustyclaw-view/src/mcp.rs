//! Component data for the MCP server management panel.

use crate::tone::Tone;

/// Display data for a single MCP server.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct McpServerData {
    pub name: String,
    pub status: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub tools: Vec<String>,
    pub health_ok: Option<bool>,
}

impl McpServerData {
    /// Convert from the protocol DTO.
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::McpServerDto) -> Self {
        Self {
            name: dto.name.clone(),
            status: dto.status.clone(),
            command: dto.command.clone(),
            url: dto.url.clone(),
            tools: dto.tools.clone(),
            health_ok: dto.health_ok,
        }
    }

    /// Status tone for badges.
    pub fn status_tone(&self) -> Tone {
        match self.status.as_str() {
            "connected" => Tone::Success,
            "connecting" => Tone::Info,
            "disconnected" => Tone::Neutral,
            "error" => Tone::Danger,
            _ => Tone::Warning,
        }
    }

    /// Health display.
    pub fn health_label(&self) -> &'static str {
        match self.health_ok {
            Some(true) => "Healthy",
            Some(false) => "Unhealthy",
            None => "Unknown",
        }
    }

    /// Connection target (command or URL).
    pub fn target(&self) -> &str {
        if let Some(ref cmd) = self.command {
            cmd
        } else if let Some(ref url) = self.url {
            url
        } else {
            "(none)"
        }
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

/// Full state for the MCP management panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct McpPanelData {
    pub servers: Vec<McpServerData>,
    pub selected: Option<usize>,
    pub status: Option<String>,
}

impl McpPanelData {
    pub fn connected_count(&self) -> usize {
        self.servers
            .iter()
            .filter(|s| s.status == "connected")
            .count()
    }

    pub fn total_count(&self) -> usize {
        self.servers.len()
    }

    pub fn selected_server(&self) -> Option<&McpServerData> {
        self.selected.and_then(|i| self.servers.get(i))
    }

    pub fn select_next(&mut self) {
        let max = self.servers.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur >= max { 0 } else { cur + 1 });
    }

    pub fn select_prev(&mut self) {
        let max = self.servers.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur == 0 { max } else { cur - 1 });
    }
}
