//! UI components for the desktop client.
//!
//! Components render shared view-models from `rustyclaw-view` with
//! `dioxus-bulma` widgets.  Module structure aligned with the TUI client:
//!
//!   - `chat.rs`           — composite of Messages + InputBar
//!   - `messages.rs`       — message list, empty state, indicators
//!   - `message.rs`        — individual message bubble
//!   - `input_bar.rs`      — text input + model bar
//!   - `sidebar.rs`        — project/thread sidebar
//!   - `tool_call.rs`      — tool call panel
//!   - dialog modules      — credential, device_flow, hatching,
//!     pairing, settings, swarm, tool_approval,
//!     user_prompt, vault_unlock
//!
//! This module also provides the shared Bulma plumbing: [`tone_color`]
//! maps the view layer's semantic [`Tone`] to a Bulma colour, and
//! [`RcModal`] is the one modal shell every dialog renders into.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, Modal, ModalCard, ModalCardBody, ModalCardFoot, ModalCardHead,
};
use rustyclaw_view::Tone;

mod analytics;
mod approvals;
mod channels;
mod chat;
mod composer_accessory;
mod connection;
mod credential_request;
mod cron;
mod device_flow;
mod engines;
mod file_browser;
mod hatching;
mod logs;
mod mcp;
mod memory;
mod new_project;
mod pairing;
mod secrets;
mod services;
mod settings;
mod sidebar;
mod swarm_panel;
mod system_info;
mod tool_approval;
mod tools_config;
mod user_prompt;
mod vault_unlock;

#[allow(unused_imports)]
pub use analytics::AnalyticsDialog;
#[allow(unused_imports)]
pub use approvals::ApprovalsDialog;
#[allow(unused_imports)]
pub use channels::ChannelsDialog;
pub use chat::Chat;
pub use connection::ConnectionDialog;
pub use credential_request::CredentialRequestDialog;
#[allow(unused_imports)]
pub use cron::CronDialog;
pub use device_flow::DeviceFlowDialog;
#[allow(unused_imports)]
pub use engines::EnginesDialog;
pub use file_browser::FileBrowser;
pub use hatching::HatchingDialog;
#[allow(unused_imports)]
pub use logs::LogsDialog;
#[allow(unused_imports)]
pub use mcp::McpDialog;
#[allow(unused_imports)]
pub use memory::MemoryDialog;
pub use new_project::NewProjectDialog;
pub use pairing::{PairingDialog, generate_qr_code};
pub use secrets::{SecretsCommand, SecretsDialog};
pub use services::ServicesDialog;
pub use settings::SettingsDialog;
pub use sidebar::Sidebar;
pub use swarm_panel::SwarmPanel;
pub use system_info::SystemInfoDialog;
pub use tool_approval::ToolApprovalDialog;
#[allow(unused_imports)]
pub use tools_config::ToolsConfigDialog;
pub use user_prompt::UserPromptDialog;
pub use vault_unlock::VaultUnlockDialog;

/// Copy text to the system clipboard via the webview's Clipboard API.
pub(crate) fn copy_to_clipboard(text: String) {
    spawn(async move {
        let js = format!("navigator.clipboard.writeText({:?})", text);
        let _ = document::eval(&js).await;
    });
}

/// Map a view-layer semantic [`Tone`] to a Bulma colour.
///
/// `Tone::Neutral` maps to `None` so the widget keeps its scheme colour.
pub(crate) fn tone_color(tone: Tone) -> Option<BulmaColor> {
    match tone {
        Tone::Neutral => None,
        Tone::Primary => Some(BulmaColor::Primary),
        Tone::Info => Some(BulmaColor::Info),
        Tone::Success => Some(BulmaColor::Success),
        Tone::Warning => Some(BulmaColor::Warning),
        Tone::Danger => Some(BulmaColor::Danger),
    }
}

/// Props for [`RcModal`].
#[derive(Props, Clone, PartialEq)]
pub struct RcModalProps {
    /// Whether the modal is shown. When `false` nothing renders.
    pub active: bool,
    /// Header title text.
    pub title: String,
    /// Preferred card width in pixels (clamped to the viewport).
    #[props(default)]
    pub width: Option<u32>,
    /// Extra class for the modal card.
    #[props(default)]
    pub class: Option<String>,
    /// Whether the backdrop click / header ✕ dismisses the dialog.
    #[props(default = true)]
    pub closable: bool,
    /// Dismiss handler (backdrop click or header ✕).
    pub onclose: EventHandler<()>,
    /// Footer content, typically a `Buttons` row. Omitted → no footer.
    #[props(default)]
    pub footer: Option<Element>,
    pub children: Element,
}

/// Shared modal shell: Bulma `Modal` + `ModalCard` with a title header,
/// scrollable body, and optional footer.
#[component]
pub fn RcModal(props: RcModalProps) -> Element {
    if !props.active {
        return rsx! {};
    }

    let card_style = props
        .width
        .map(|w| format!("width: min({w}px, calc(100vw - 40px));"))
        .unwrap_or_default();
    let head_class = if props.closable {
        None
    } else {
        Some("rc-no-close".to_string())
    };
    let closable = props.closable;
    let onclose = props.onclose;

    rsx! {
        Modal {
            active: true,
            onclose: move |_| {
                if closable {
                    onclose.call(());
                }
            },
            ModalCard {
                class: props.class.clone(),
                style: card_style,
                ModalCardHead {
                    class: head_class,
                    onclose: move |_| {
                        if closable {
                            onclose.call(());
                        }
                    },
                    p { class: "modal-card-title", "{props.title}" }
                }
                ModalCardBody { {props.children} }
                if let Some(footer) = props.footer {
                    ModalCardFoot { class: "rc-modal-foot", {footer} }
                }
            }
        }
    }
}
