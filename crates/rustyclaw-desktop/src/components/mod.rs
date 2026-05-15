//! UI components for the desktop client.
//!
//! Module structure aligned with the TUI client after Phase D
//! structural refactoring:
//!
//!   - `chat.rs`           — composite of Messages + InputBar
//!   - `messages.rs`       — message list, empty state, indicators
//!   - `message.rs`        — individual message bubble
//!   - `input_bar.rs`      — text input + model bar
//!   - `sidebar.rs`        — thread sidebar
//!   - `tool_call.rs`      — tool call panel
//!   - dialog modules      — credential, device_flow, hatching,
//!                           pairing, settings, swarm, tool_approval,
//!                           user_prompt, vault_unlock

mod chat;
mod credential_request;
mod device_flow;
mod hatching;
mod input_bar;
mod message;
mod messages;
mod pairing;
mod secrets;
mod settings;
mod sidebar;
mod tabs;
mod swarm_panel;
mod tool_approval;
mod tool_call;
mod user_prompt;
mod vault_unlock;

pub use chat::Chat;
pub use credential_request::CredentialRequestDialog;
pub use device_flow::DeviceFlowDialog;
pub use hatching::{HatchingDialog, HatchingResult};
pub use pairing::{PairingDialog, generate_qr_code};
pub use secrets::{SecretsCommand, SecretsDialog};
pub use settings::SettingsDialog;
pub use sidebar::Sidebar;
pub use tabs::TabBar;
pub use swarm_panel::{SwarmAgentInfo, SwarmInfo, SwarmPanel};
pub use tool_approval::ToolApprovalDialog;
pub use user_prompt::UserPromptDialog;
pub use vault_unlock::VaultUnlockDialog;
