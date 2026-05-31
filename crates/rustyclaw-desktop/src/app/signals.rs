//! Bundled `Signal` handles for the desktop `App` component, so the dialog
//! render code can move into its own module. `Signal<T>` is `Copy`.

use std::sync::Arc;

use dioxus::prelude::*;

use crate::app_support::connect_to_gateway;
use crate::state::AppState;
use rustyclaw_core::gateway::GatewayClient;
use rustyclaw_core::user_prompt_types::UserPrompt;
use rustyclaw_view::HatchingDialogData;

#[derive(Clone, Copy)]
pub(super) struct AppSignals {
    pub state: Signal<AppState>,
    pub gateway: Signal<Option<Arc<GatewayClient>>>,
    pub did_auto_connect: Signal<bool>,
    pub active_event_client: Signal<Option<Arc<GatewayClient>>>,
    pub auth_code: Signal<String>,
    pub show_pairing: Signal<bool>,
    pub hatching_dialog: Signal<HatchingDialogData>,
    pub show_settings: Signal<bool>,
    pub show_swarm: Signal<bool>,
    pub swarm_creating: Signal<bool>,
    pub tool_approval_id: Signal<String>,
    pub tool_approval_name: Signal<String>,
    pub tool_approval_args: Signal<String>,
    pub show_tool_approval: Signal<bool>,
    pub show_vault_unlock: Signal<bool>,
    pub vault_unlock_error: Signal<Option<String>>,
    pub show_user_prompt: Signal<bool>,
    pub user_prompt_data: Signal<Option<UserPrompt>>,
    pub show_cred_request: Signal<bool>,
    pub cred_request_id: Signal<String>,
    pub cred_request_provider: Signal<String>,
    pub cred_request_secret: Signal<String>,
    pub cred_request_message: Signal<String>,
    pub qr_code_url: Signal<Option<String>>,
    pub public_key: Signal<Option<String>>,
    pub show_secrets: Signal<bool>,
    pub pending_thread_delete: Signal<Option<(u64, String)>>,
    pub did_init_directories: Signal<bool>,
    pub show_connection: Signal<bool>,
}

/// Reconnect to the gateway using the current `state.gateway_url`.
pub(super) fn do_reconnect(sig: AppSignals) {
    let state = sig.state;
    let gateway = sig.gateway;
    let url = state.read().gateway_url.clone();
    spawn(async move {
        connect_to_gateway(&url, state, gateway).await;
    });
}
