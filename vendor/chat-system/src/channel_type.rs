//! Channel/platform type definitions and capabilities.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Supported inbound message delivery modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InboundMode {
    None,
    Polling,
    GatewayLoop,
    SocketMode,
    Webhook,
}

/// Capabilities of a particular channel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelCapabilities {
    pub inbound_mode: InboundMode,
    pub supports_outbound: bool,
    pub supports_streaming: bool,
    pub supports_interactive: bool,
    pub supports_threads: bool,
    pub supports_voice_ingest: bool,
    pub supports_pairing: bool,
    pub supports_otp: bool,
    pub supports_reactions: bool,
    pub supports_location: bool,
}

/// Descriptor combining a channel type with its display name and capabilities.
#[derive(Debug, Clone, Copy)]
pub struct ChannelDescriptor {
    pub channel_type: ChannelType,
    pub display_name: &'static str,
    pub capabilities: ChannelCapabilities,
}

/// All supported chat platform / channel types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    Whatsapp,
    #[serde(rename = "msteams")]
    MsTeams,
    Discord,
    Slack,
    Irc,
    Matrix,
    Signal,
    GoogleChat,
    IMessage,
    Console,
    Webhook,
}

impl ChannelType {
    pub const ALL: &'static [ChannelType] = &[
        ChannelType::Telegram,
        ChannelType::Whatsapp,
        ChannelType::MsTeams,
        ChannelType::Discord,
        ChannelType::Slack,
        ChannelType::Irc,
        ChannelType::Matrix,
        ChannelType::Signal,
        ChannelType::GoogleChat,
        ChannelType::IMessage,
        ChannelType::Console,
        ChannelType::Webhook,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ChannelType::Telegram => "telegram",
            ChannelType::Whatsapp => "whatsapp",
            ChannelType::MsTeams => "msteams",
            ChannelType::Discord => "discord",
            ChannelType::Slack => "slack",
            ChannelType::Irc => "irc",
            ChannelType::Matrix => "matrix",
            ChannelType::Signal => "signal",
            ChannelType::GoogleChat => "googlechat",
            ChannelType::IMessage => "imessage",
            ChannelType::Console => "console",
            ChannelType::Webhook => "webhook",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ChannelType::Telegram => "Telegram",
            ChannelType::Whatsapp => "WhatsApp",
            ChannelType::MsTeams => "Microsoft Teams",
            ChannelType::Discord => "Discord",
            ChannelType::Slack => "Slack",
            ChannelType::Irc => "IRC",
            ChannelType::Matrix => "Matrix",
            ChannelType::Signal => "Signal",
            ChannelType::GoogleChat => "Google Chat",
            ChannelType::IMessage => "iMessage",
            ChannelType::Console => "Console",
            ChannelType::Webhook => "Webhook",
        }
    }

    pub fn descriptor(&self) -> ChannelDescriptor {
        let caps = match self {
            ChannelType::Telegram => ChannelCapabilities {
                inbound_mode: InboundMode::Polling,
                supports_outbound: true,
                supports_streaming: true,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: true,
                supports_pairing: false,
                supports_otp: true,
                supports_reactions: false,
                supports_location: true,
            },
            ChannelType::Whatsapp => ChannelCapabilities {
                inbound_mode: InboundMode::GatewayLoop,
                supports_outbound: true,
                supports_streaming: true,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: true,
                supports_pairing: true,
                supports_otp: true,
                supports_reactions: false,
                supports_location: false,
            },
            ChannelType::MsTeams => ChannelCapabilities {
                inbound_mode: InboundMode::Webhook,
                supports_outbound: true,
                supports_streaming: true,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: true,
            },
            ChannelType::Discord => ChannelCapabilities {
                inbound_mode: InboundMode::GatewayLoop,
                supports_outbound: true,
                supports_streaming: true,
                supports_interactive: true,
                supports_threads: true,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: true,
            },
            ChannelType::Slack => ChannelCapabilities {
                inbound_mode: InboundMode::SocketMode,
                supports_outbound: true,
                supports_streaming: true,
                supports_interactive: true,
                supports_threads: true,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: true,
                supports_location: false,
            },
            ChannelType::Irc => ChannelCapabilities {
                inbound_mode: InboundMode::GatewayLoop,
                supports_outbound: true,
                supports_streaming: false,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: false,
            },
            ChannelType::Matrix => ChannelCapabilities {
                inbound_mode: InboundMode::GatewayLoop,
                supports_outbound: true,
                supports_streaming: true,
                supports_interactive: false,
                supports_threads: true,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: true,
                supports_location: false,
            },
            ChannelType::Signal => ChannelCapabilities {
                inbound_mode: InboundMode::GatewayLoop,
                supports_outbound: true,
                supports_streaming: false,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: true,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: false,
            },
            ChannelType::GoogleChat => ChannelCapabilities {
                inbound_mode: InboundMode::Webhook,
                supports_outbound: true,
                supports_streaming: false,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: false,
            },
            ChannelType::IMessage => ChannelCapabilities {
                inbound_mode: InboundMode::Polling,
                supports_outbound: true,
                supports_streaming: false,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: false,
            },
            ChannelType::Console => ChannelCapabilities {
                inbound_mode: InboundMode::None,
                supports_outbound: true,
                supports_streaming: false,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: false,
            },
            ChannelType::Webhook => ChannelCapabilities {
                inbound_mode: InboundMode::None,
                supports_outbound: true,
                supports_streaming: false,
                supports_interactive: false,
                supports_threads: false,
                supports_voice_ingest: false,
                supports_pairing: false,
                supports_otp: false,
                supports_reactions: false,
                supports_location: false,
            },
        };
        ChannelDescriptor {
            channel_type: *self,
            display_name: self.display_name(),
            capabilities: caps,
        }
    }
}

impl fmt::Display for ChannelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ChannelType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "telegram" => Ok(ChannelType::Telegram),
            "whatsapp" => Ok(ChannelType::Whatsapp),
            "msteams" | "teams" | "microsoft teams" => Ok(ChannelType::MsTeams),
            "discord" => Ok(ChannelType::Discord),
            "slack" => Ok(ChannelType::Slack),
            "irc" => Ok(ChannelType::Irc),
            "matrix" => Ok(ChannelType::Matrix),
            "signal" => Ok(ChannelType::Signal),
            "googlechat" | "google chat" => Ok(ChannelType::GoogleChat),
            "imessage" => Ok(ChannelType::IMessage),
            "console" => Ok(ChannelType::Console),
            "webhook" => Ok(ChannelType::Webhook),
            other => Err(anyhow::anyhow!("unknown channel type: {}", other)),
        }
    }
}
