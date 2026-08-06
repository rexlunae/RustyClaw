// ── Device flow dialog — OAuth device authorization overlay ──────────────────

use crate::theme;
use iocraft::prelude::*;
use rustyclaw_view::DeviceFlowData;

pub const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Format a URL as an OSC 8 clickable terminal hyperlink.
///
/// Most modern terminals (iTerm2, GNOME Terminal, Windows Terminal, Kitty,
/// WezTerm, Alacritty 0.14+) support the OSC 8 escape sequence for inline
/// hyperlinks.  Terminals that don't recognise it simply display the text.
pub fn osc8_hyperlink(url: &str, label: &str) -> String {
    format!("\x1b]8;;{}\x1b\\{}\x1b]8;;\x1b\\", url, label)
}

/// Open a URL in the user's default browser (best-effort, non-blocking).
///
/// The verification URL arrives in the provider's device-flow response, so it
/// goes through `open_external` for the scheme allowlist rather than straight
/// to a platform opener. Failure stays non-fatal — the dialog renders the same
/// URL as an OSC 8 link and as plain text, so the user still has a way through
/// — but it is logged rather than dropped, since "the browser never opened"
/// and "the URL was refused" look identical on screen.
pub fn open_url_in_browser(url: &str) {
    if let Err(e) = rustyclaw_core::open_external::open_external(url) {
        rustyclaw_view::tracing::debug!(
            "device flow browser open failed, falling back to the on-screen link: {e}"
        );
    }
}

#[derive(Default, Props)]
pub struct DeviceFlowDialogProps {
    /// Shared dialog data from `rustyclaw-view`.
    pub data: DeviceFlowData,
}

#[component]
pub fn DeviceFlowDialog(props: &DeviceFlowDialogProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER[props.data.tick % SPINNER.len()];

    // Build the clickable hyperlink text using OSC 8 escape sequences.
    let hyperlink = osc8_hyperlink(&props.data.url, &props.data.url);

    let browser_hint = if props.data.browser_opened {
        "  ✓ Opened in your browser"
    } else {
        "  Enter to open in browser"
    };

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 60,
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
                        content: hyperlink,
                        color: theme::ACCENT_BRIGHT,
                        weight: Weight::Bold,
                    )
                }
                Text(
                    content: browser_hint.to_string(),
                    color: if props.data.browser_opened { theme::SUCCESS } else { theme::MUTED },
                )
                View(height: 1)

                Text(content: "2. Enter this code:", color: theme::TEXT)
                View(height: 1)
                View(
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                ) {
                    Text(
                        content: format!("  {}  ", props.data.code),
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
                    content: "Enter open browser · Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
