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
//!   - `user_prompt.rs`    — inline agent-question card (`ask_user` tool)
//!   - dialog modules      — credential, device_flow, hatching,
//!     pairing, settings, swarm, tool_approval, vault_unlock
//!
//! This module also provides the shared Bulma plumbing: [`tone_color`]
//! maps the view layer's semantic [`Tone`] to a Bulma colour, and
//! [`RcModal`] is the one modal shell every dialog renders into.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{
    BulmaColor, Modal, ModalCard, ModalCardBody, ModalCardFoot, ModalCardHead,
};
use rustyclaw_core::ignore::Ignore;
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
mod dir_picker;
mod downloads;
mod edit_dialogs;
mod editor;
mod engines;
mod file_browser;
mod hatching;
mod logs;
mod mcp;
mod memory;
mod messengers;
mod new_project;
mod pairing;
mod plugin_panel;
mod secrets;
mod services;
mod settings;
mod sidebar;
mod skills;
mod swarm_panel;
mod system_info;
mod tool_approval;
mod tools_config;
mod unsaved_changes;
mod user_prompt;
mod vault_unlock;

pub use analytics::AnalyticsDialog;
#[allow(unused_imports)]
pub use approvals::ApprovalsDialog;
pub use channels::ChannelsDialog;
pub use chat::Chat;
pub use connection::ConnectionDialog;
pub use credential_request::CredentialRequestDialog;
pub use cron::{CronCommand, CronDialog};
pub use device_flow::DeviceFlowDialog;
pub use downloads::DownloadsDialog;
pub use edit_dialogs::{EditProjectDialog, EditThreadDialog};
pub use editor::{EDITOR_PLUGIN, Editor, EditorAction};
#[allow(unused_imports)]
pub use engines::EnginesDialog;
#[allow(unused_imports)]
pub use file_browser::FileBrowser;
pub use hatching::HatchingDialog;
pub use logs::LogsDialog;
pub use mcp::McpDialog;
pub use memory::MemoryDialog;
pub use messengers::{MessengerCommand, MessengersDialog};
pub use new_project::NewProjectDialog;
pub use pairing::{PairingDialog, generate_qr_code};
pub use plugin_panel::{
    NativePluginTab, PluginActionEvent, PluginActionInfo, PluginPanel, PluginSnapshot,
};
pub use secrets::{SecretsCommand, SecretsDialog};
pub use services::ServicesDialog;
pub use settings::SettingsDialog;
pub use sidebar::Sidebar;
pub use skills::SkillsDialog;
pub use swarm_panel::SwarmPanel;
pub use system_info::SystemInfoDialog;
pub use tool_approval::ToolApprovalDialog;
pub use tools_config::ToolsConfigDialog;
pub use unsaved_changes::{UnsavedChangesDialog, UnsavedChoice};
pub use vault_unlock::VaultUnlockDialog;

/// Copy text to the system clipboard via the webview's Clipboard API.
pub(crate) fn copy_to_clipboard(text: String) {
    spawn(async move {
        let js = format!("navigator.clipboard.writeText({:?})", text);
        document::eval(&js).await.ignore();
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

/// Map a view-layer semantic [`Tone`] to a CSS modifier class.
///
/// Used where a custom element (rather than a Bulma widget) needs to carry
/// the tone, e.g. the sidebar's connection dot.
pub(crate) fn tone_modifier(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral => "is-neutral",
        Tone::Primary => "is-primary",
        Tone::Info => "is-info",
        Tone::Success => "is-success",
        Tone::Warning => "is-warning",
        Tone::Danger => "is-danger",
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
