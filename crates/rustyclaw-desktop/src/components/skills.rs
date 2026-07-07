//! Skills manager dialog — list loaded skills and toggle them.
//!
//! Skills live on the local filesystem (workspace/local/bundled dirs), so
//! the dialog operates on the local `SkillManager` — same as the TUI.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, BulmaSize, Button, Buttons, Table, Tag};
use rustyclaw_view::SkillInfoData;

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct SkillsDialogProps {
    pub visible: bool,
    pub skills: Vec<SkillInfoData>,
    /// Toggle a skill's enabled state by name.
    pub on_toggle: EventHandler<String>,
    pub on_close: EventHandler<()>,
}

#[component]
pub fn SkillsDialog(props: SkillsDialogProps) -> Element {
    if !props.visible {
        return rsx! {};
    }

    let enabled_count = props.skills.iter().filter(|s| s.enabled).count();
    let total = props.skills.len();

    rsx! {
        RcModal {
            active: true,
            title: "🧩 Skills",
            width: 640,
            onclose: move |_| props.on_close.call(()),
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Primary,
                        onclick: move |_| props.on_close.call(()),
                        "Done"
                    }
                }
            },

            if props.skills.is_empty() {
                p { class: "has-text-grey",
                    "No skills found. Install skills with /skill install or from ClawHub."
                }
            } else {
                div { class: "mb-3",
                    Tag { color: BulmaColor::Info, light: true, rounded: true,
                        "{enabled_count} enabled / {total} total"
                    }
                }

                Table { fullwidth: true, hoverable: true, narrow: true,
                    thead {
                        tr {
                            th { "Skill" }
                            th { "Description" }
                            th { "Status" }
                            th { "" }
                        }
                    }
                    tbody {
                        for skill in props.skills.iter() {
                            {
                                let name = skill.name.clone();
                                let description = skill.description.clone();
                                let enabled = skill.enabled;
                                let on_toggle = props.on_toggle;
                                let toggle_name = name.clone();
                                rsx! {
                                    tr { key: "{name}",
                                        td { "{name}" }
                                        td { class: "has-text-grey", "{description}" }
                                        td {
                                            Tag {
                                                color: if enabled { Some(BulmaColor::Success) } else { None },
                                                light: true,
                                                rounded: true,
                                                size: BulmaSize::Small,
                                                if enabled { "Enabled" } else { "Disabled" }
                                            }
                                        }
                                        td {
                                            Button {
                                                color: if enabled { BulmaColor::Light } else { BulmaColor::Primary },
                                                outlined: !enabled,
                                                size: BulmaSize::Small,
                                                onclick: move |_| on_toggle.call(toggle_name.clone()),
                                                if enabled { "Disable" } else { "Enable" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
