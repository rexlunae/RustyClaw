//! UI components for the desktop client.

mod chat;
mod hatching;
mod message;
mod pairing;
mod settings;
mod sidebar;
mod tool_call;

pub use chat::Chat;
pub use hatching::{HatchingDialog, HatchingResult};
pub use pairing::{PairingDialog, generate_qr_code};
pub use settings::SettingsDialog;
pub use sidebar::Sidebar;
