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
pub use pairing::{PairingDialog, generate_qr_code};
pub use sidebar::Sidebar;
pub use tool_call::ToolCallPanel;
