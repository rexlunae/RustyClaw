// ── Device flow dialog — OAuth device authorization overlay ──────────────────

use crate::theme;
use iocraft::prelude::*;

pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Default, Props)]
pub struct DeviceFlowDialogProps {
    /// The verification URL the user should visit.
    pub url: String,
    /// The one-time user code to enter on that page.
    pub code: String,
    /// Spinner tick for the waiting animation.
    pub tick: usize,
}

#[component]
pub fn DeviceFlowDialog(props: &DeviceFlowDialogProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER[props.tick % SPINNER.len()];

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 56,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::INFO,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
            ) {
                Text(
                    content: "🔗 Device Authorization",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )
                View(height: 1)

                Text(content: "1. Open this URL in your browser:", color: theme::TEXT)
                View(height: 1)
                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: props.url.clone(),
                        color: theme::ACCENT_BRIGHT,
                        weight: Weight::Bold,
                    )
                }
                View(height: 1)

                Text(content: "2. Enter this code:", color: theme::TEXT)
                View(height: 1)
                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: format!("  {}  ", props.code),
                        color: theme::WARN,
                        weight: Weight::Bold,
                    )
                }
                View(height: 1)

                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: format!("{} Waiting for authorization…", spinner),
                        color: theme::MUTED,
                    )
                }
                View(height: 1)
                Text(
                    content: "Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
