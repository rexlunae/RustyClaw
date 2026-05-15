//! Thread tab bar — replaces the old thread list in the sidebar.
//!
//! Renders horizontally above the message area.  Each tab shows
//! the session label, message count, and a close button.

use dioxus::prelude::*;
use rustyclaw_view::TabBarData;

#[derive(Props, Clone, PartialEq)]
pub struct TabBarProps {
    pub data: TabBarData,
    pub on_switch: EventHandler<u64>,
    pub on_new: EventHandler<()>,
    pub on_close: EventHandler<u64>,
}

#[component]
pub fn TabBar(props: TabBarProps) -> Element {
    // DEBUG: always-rendered stripe so we can see if the component mounts
    rsx! {
        div {
            class: "tab-bar-debug",
            style: "display: flex; align-items: center; padding: 6px 12px; background: #ff4444; color: white; font-weight: bold; min-height: 32px;",
            "■■ TAB BAR ■■"
        }
    }
}
