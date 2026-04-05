// ── Device flow dialog — OAuth device authorization overlay ──────────────────

use crate::theme;
use iocraft::prelude::*;

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
pub fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", url])
        .spawn();
}

#[derive(Default, Props)]
pub struct DeviceFlowDialogProps {
    /// The verification URL the user should visit.
    pub url: String,
    /// The one-time user code to enter on that page.
    pub code: String,
    /// Spinner tick for the waiting animation.
    pub tick: usize,
    /// Whether the browser was already opened automatically.
    pub browser_opened: bool,
}

#[component]
pub fn DeviceFlowDialog(props: &DeviceFlowDialogProps) -> impl Into<AnyElement<'static>> {
    let spinner = SPINNER[props.tick % SPINNER.len()];

    // Build the clickable hyperlink text using OSC 8 escape sequences.
    let hyperlink = osc8_hyperlink(&props.url, &props.url);

    let browser_hint = if props.browser_opened {
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
                    color: if props.browser_opened { theme::SUCCESS } else { theme::MUTED },
                )
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
                    content: "Enter open browser · Esc cancel",
                    color: theme::MUTED,
                )
            }
        }
    }
}
