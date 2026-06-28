//! Component data for the channel/messenger status panel.

use crate::tone::Tone;

/// Display data for a single messenger channel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChannelStatusData {
    pub name: String,
    pub channel_type: String,
    pub paired: bool,
    pub online: bool,
    pub last_message: Option<String>,
}

impl ChannelStatusData {
    /// Convert from the protocol DTO.
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::ChannelStatusDto) -> Self {
        Self {
            name: dto.name.clone(),
            channel_type: dto.channel_type.clone(),
            paired: dto.paired,
            online: dto.online,
            last_message: dto.last_message.clone(),
        }
    }

    /// Status tone for the channel.
    pub fn status_tone(&self) -> Tone {
        if !self.paired {
            Tone::Neutral
        } else if self.online {
            Tone::Success
        } else {
            Tone::Warning
        }
    }

    /// Status label.
    pub fn status_label(&self) -> &'static str {
        if !self.paired {
            "Not Paired"
        } else if self.online {
            "Online"
        } else {
            "Offline"
        }
    }

    /// Icon for the channel type.
    pub fn channel_icon(&self) -> &'static str {
        match self.channel_type.to_lowercase().as_str() {
            "signal" => "📱",
            "telegram" => "✈️",
            "discord" => "🎮",
            "slack" => "💼",
            "whatsapp" => "📞",
            "matrix" => "🔗",
            "beeper" => "🐝",
            _ => "💬",
        }
    }
}

/// Full state for the channels panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ChannelsPanelData {
    pub channels: Vec<ChannelStatusData>,
    pub selected: Option<usize>,
    pub status: Option<String>,
}

impl ChannelsPanelData {
    pub fn paired_count(&self) -> usize {
        self.channels.iter().filter(|c| c.paired).count()
    }

    pub fn online_count(&self) -> usize {
        self.channels.iter().filter(|c| c.online).count()
    }

    pub fn total_count(&self) -> usize {
        self.channels.len()
    }

    pub fn selected_channel(&self) -> Option<&ChannelStatusData> {
        self.selected.and_then(|i| self.channels.get(i))
    }
}
