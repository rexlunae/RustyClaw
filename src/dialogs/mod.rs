//! Dialog modules for the TUI.
//!
//! Each dialog handles its own state, key handling, and rendering.

mod api_key;
mod auth_prompt;
mod credential;
mod model_selector;
mod policy_picker;
mod provider_selector;
mod secret_viewer;
mod totp;
mod vault_unlock;

pub use api_key::{
    ApiKeyDialogPhase, ApiKeyDialogState, draw_api_key_dialog, handle_api_key_dialog_key,
    handle_confirm_store_secret, open_api_key_dialog,
};
pub use auth_prompt::{AuthPromptState, draw_auth_prompt, handle_auth_prompt_key};
pub use credential::{
    CredDialogOption, CredentialDialogState, draw_credential_dialog, handle_credential_dialog_key,
    handle_credential_dialog_key_gateway,
};
pub use model_selector::{
    FetchModelsLoading, ModelSelectorState, draw_model_selector_dialog, handle_model_selector_key,
    open_model_selector, spawn_fetch_models,
};
pub use policy_picker::{
    PolicyPickerOption, PolicyPickerPhase, PolicyPickerState, draw_policy_picker,
    handle_policy_picker_key, handle_policy_picker_key_gateway,
};
pub use provider_selector::{
    ProviderSelectorState, draw_provider_selector_dialog, handle_provider_selector_key,
    open_provider_selector,
};
pub use secret_viewer::{
    SecretViewerState, copy_to_clipboard, draw_secret_viewer, handle_secret_viewer_key,
};
pub use totp::{TotpDialogPhase, TotpDialogState, draw_totp_dialog, handle_totp_dialog_key, handle_totp_dialog_key_gateway};
pub use vault_unlock::{
    VaultUnlockPromptState, draw_vault_unlock_prompt, handle_vault_unlock_prompt_key,
};

/// Spinner frames for loading animations.
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
