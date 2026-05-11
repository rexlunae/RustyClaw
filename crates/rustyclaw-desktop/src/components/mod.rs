//! UI components for the desktop client.

mod chat;
mod credential_request;
mod device_flow;
mod hatching;
mod message;
mod pairing;
mod settings;
mod sidebar;
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
pub use settings::SettingsDialog;
pub use sidebar::Sidebar;
pub use swarm_panel::{SwarmAgentInfo, SwarmInfo, SwarmPanel};
pub use tool_approval::ToolApprovalDialog;
pub use user_prompt::UserPromptDialog;
pub use vault_unlock::VaultUnlockDialog;
