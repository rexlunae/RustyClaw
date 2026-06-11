//! First-run "hatching" wizard for naming and personality setup.

use dioxus::prelude::*;
use dioxus_bulma::prelude::{BulmaColor, Button, Buttons, Control, Field, FieldLabel, Help};
use rustyclaw_view::{HatchingDialogData, HatchingResult};

use super::RcModal;

#[derive(Props, Clone, PartialEq)]
pub struct HatchingDialogProps {
    pub data: HatchingDialogData,
    pub on_update: EventHandler<HatchingDialogData>,
    pub on_complete: EventHandler<HatchingResult>,
    pub on_cancel: EventHandler<()>,
}

#[component]
pub fn HatchingDialog(props: HatchingDialogProps) -> Element {
    let data = props.data.clone();
    if !data.visible {
        return rsx! {};
    }

    let is_complete_disabled = data.name_input.trim().is_empty();
    let name_input = data.name_input.clone();
    let personality_input = data.personality_input.clone();
    let on_update = props.on_update;
    let on_complete = props.on_complete;
    let on_cancel = props.on_cancel;
    let data_close = data.clone();
    let data_name_input = data.clone();
    let data_name_enter = data.clone();
    let data_personality_input = data.clone();
    let data_cancel = data.clone();
    let data_complete = data.clone();

    rsx! {
        RcModal {
            active: true,
            title: "🥚 Set up your agent",
            width: 480,
            onclose: move |_| {
                let mut data = data_close.clone();
                data.dismiss();
                on_update.call(data);
                on_cancel.call(());
            },
            footer: rsx! {
                Buttons {
                    Button {
                        color: BulmaColor::Light,
                        onclick: move |_| {
                            let mut data = data_cancel.clone();
                            data.dismiss();
                            on_update.call(data);
                            on_cancel.call(());
                        },
                        "Cancel"
                    }
                    Button {
                        color: BulmaColor::Primary,
                        disabled: is_complete_disabled,
                        onclick: move |_| {
                            if let Some(result) = data_complete.completion() {
                                let mut next_data = data_complete.clone();
                                next_data.dismiss();
                                on_update.call(next_data);
                                on_complete.call(result);
                            }
                        },
                        "✓ Complete"
                    }
                }
            },

            Field {
                FieldLabel { "Agent name" }
                Control {
                    input {
                        class: "input",
                        r#type: "text",
                        placeholder: "Give your agent a name",
                        value: name_input,
                        autofocus: true,
                        oninput: move |evt| {
                            let mut data = data_name_input.clone();
                            data.name_input = evt.value();
                            on_update.call(data);
                        },
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() == Key::Enter && !data_name_enter.name_input.trim().is_empty() {
                                evt.prevent_default();
                                if let Some(result) = data_name_enter.completion() {
                                    let mut next_data = data_name_enter.clone();
                                    next_data.dismiss();
                                    on_update.call(next_data);
                                    on_complete.call(result);
                                }
                            }
                        }
                    }
                }
                Help { "This is how RustyClaw will refer to your agent in the UI." }
            }

            Field {
                FieldLabel { "Personality (optional)" }
                Control {
                    textarea {
                        class: "textarea",
                        placeholder: "e.g. Friendly and curious, with a dry sense of humour",
                        rows: "4",
                        value: personality_input,
                        oninput: move |evt| {
                            let mut data = data_personality_input.clone();
                            data.personality_input = evt.value();
                            on_update.call(data);
                        },
                    }
                }
                Help { "Leave blank to use the default. You can change this later." }
            }
        }
    }
}
