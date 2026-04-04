//! UI Components for the desktop client.

mod chat;
mod hatching;
mod message;
mod pairing;
mod sidebar;
mod tool_call;

pub use chat::Chat;
pub use hatching::{HatchingDialog, HatchingResult};
pub use message::MessageBubble;
pub use pairing::{generate_qr_code, PairingDialog};
pub use sidebar::Sidebar;
pub use tool_call::ToolCallPanel;
