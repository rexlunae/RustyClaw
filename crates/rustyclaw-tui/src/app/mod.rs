mod tui_component;
// ── App module ──────────────────────────────────────────────────────────────
//
// Re-exports from app.rs for the public path `rustyclaw_tui::app::App`.

mod app;

pub use app::App;
pub(crate) use app::{GwEvent, UserInput};
