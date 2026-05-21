//! `rustyclaw-view` — shared component-oriented view data types.
//!
//! This crate defines the exact slice of data each UI component needs
//! to render.  It sits between the canonical domain models in
//! [`rustyclaw_core::ui`] and the framework-specific rendering code
//! in the TUI (iocraft) and desktop (Dioxus) crates.
//!
//! ## Design principles
//!
//! - **Data only.** No event handlers, no framework imports.  Each type
//!   is a plain struct with `Clone + Debug + PartialEq`.
//!
//! - **Component-oriented.** Each struct corresponds to one UI component
//!   (`MessageBubbleData`, `ToolCallData`, `SidebarItemData`, …).
//!   These are the exact shapes needed by the renderer — not the
//!   canonical domain model.
//!
//! - **Framework-agnostic wrappers.** Each client crate wraps these
//!   types in their own Props struct, adding framework-specific fields
//!   like `EventHandler` (Dioxus) or `State` / `Hooks` (iocraft).
//!
//! ## Why separate from `rustyclaw_core::ui`
//!
//! [`rustyclaw_core::ui`] owns the *canonical* models (`ChatMessage`,
//! `ToolCallInfo`, `ThreadInfo`, `DialogState`, `StreamingState`).
//! These carry enough state to translate from `GatewayEvent` and
//! manage intermediate state.
//!
//! This crate owns the *component* models — the specific data slices
//! that renderers consume.  A `ChatMessage` owns tool calls and
//! streaming state; a `MessageBubbleData` is just the bubble part.
//! Tool calls are a separate component (`ToolCallData`), not nested.
//!
//! Separating the two means a change to how the gateway processes
//! events (`ChatMessage`) doesn't affect renderer props, and a
//! change to how the bubble looks (`MessageBubbleData`) doesn't
//! require touching event-processing code.

pub mod composer;
pub mod command_menu;
pub mod conversation;
pub mod dialogs;
pub mod message;
pub mod sidebar;
pub mod status;
pub mod tabs;

// Re-export at crate root for convenience.
pub use composer::ComposerData;
pub use command_menu::{CommandMenuData, build_slash_completions};
pub use conversation::{
    ChatSurfaceData, DisplayMessageData, EmptyStateData, StarterPromptData, TopBarData,
    latest_details_index, starter_prompts,
};
pub use dialogs::{
    AuthDialogData, CredentialRequestData, DeviceFlowData, HatchState, HatchingDialogData,
    PairingDialogData, PairingField, PairingStep, SecretInfoData, SecretsDialogData,
    SkillInfoData, ToolApprovalData, ToolPermInfoData, UserPromptData, VaultUnlockData,
};
pub use message::{MessageBubbleData, StreamingIndicatorData, ToolCallData};
pub use sidebar::SidebarItemData;
pub use status::StatusBarData;
pub use tabs::{TabBarData, TabItemData};
