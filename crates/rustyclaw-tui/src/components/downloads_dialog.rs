// ── Downloads dialog — file transfers the agent started ─────────────────────
//
// Opened with /downloads; Esc closes, ↑/↓ move the selection.
//
// Transfers are deliberately not chat events: one outlives the turn that
// started it, changes many times a second, and the user wants to see where a
// file landed without scrolling back through the conversation. Mutations go
// through the slash subcommands — `/downloads cancel <id>`, `/downloads
// clear` — as every other gateway panel in the TUI does.

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct DownloadsDialogProps {
    pub downloads: Option<rustyclaw_view::DownloadsData>,
    /// Which row is highlighted, so the id to type into `/downloads cancel`
    /// can be read off the panel.
    pub selected: usize,
}

/// A text progress bar, since the terminal has no widget for one.
///
/// `None` for a chunked response — which declares no length — draws a bar of
/// dots rather than a filled one at some invented fraction.
fn bar(fraction: Option<f32>, width: usize) -> String {
    match fraction {
        Some(f) => {
            let filled = ((f * width as f32).round() as usize).min(width);
            format!("[{}{}]", "█".repeat(filled), "░".repeat(width - filled))
        }
        None => format!("[{}]", "·".repeat(width)),
    }
}

#[component]
pub fn DownloadsDialog(props: &DownloadsDialogProps) -> impl Into<AnyElement<'static>> {
    // Each row is (label, value, colour) so a failed transfer's reason can be
    // red without a second pass over the list.
    let mut rows: Vec<(String, String, Color)> = Vec::new();

    match props.downloads.as_ref() {
        None => rows.push((String::new(), "(no downloads yet)".into(), theme::MUTED)),
        Some(data) if data.is_empty() => rows.push((
            String::new(),
            "(no downloads yet — the agent starts one by fetching a URL to a file)".into(),
            theme::MUTED,
        )),
        Some(data) => {
            rows.push((
                "Summary".into(),
                format!(
                    "{} running / {} total",
                    data.active_count(),
                    data.downloads.len()
                ),
                theme::TEXT,
            ));
            rows.push((String::new(), String::new(), theme::TEXT));

            for (i, d) in data.downloads.iter().enumerate() {
                let marker = if i == props.selected { "›" } else { " " };
                let status_colour = match d.status.as_str() {
                    "running" => theme::INFO,
                    "complete" => theme::SUCCESS,
                    "failed" => theme::ERROR,
                    _ => theme::MUTED,
                };
                rows.push((
                    format!("{marker} {}", d.file_name()),
                    format!("{} · {}", d.status_label(), d.id),
                    status_colour,
                ));
                if d.is_running() {
                    let pct = match d.percent() {
                        Some(p) => format!(" {p}%"),
                        None => String::new(),
                    };
                    rows.push((
                        String::new(),
                        format!("  {}{pct}", bar(d.fraction(), 24)),
                        theme::INFO,
                    ));
                }
                let mut details = vec![d.size_display()];
                if let Some(took) = d.duration_display() {
                    details.push(took);
                }
                rows.push((
                    String::new(),
                    format!("  {}", details.join(" | ")),
                    theme::MUTED,
                ));
                // The destination is the reason for opening this panel at all:
                // the agent chose where the file went.
                rows.push((String::new(), format!("  {}", d.dest), theme::MUTED));
                // In full rather than as "failed" — whether to retry, use
                // another URL, or give up depends on what went wrong.
                if let Some(error) = d.error.as_ref() {
                    rows.push((String::new(), format!("  {error}"), theme::ERROR));
                }
            }
        }
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 70pct,
                max_height: 80pct,
                flex_direction: FlexDirection::Column,
                border_style: BorderStyle::Round,
                border_color: theme::INFO,
                background_color: theme::BG_SURFACE,
                padding_left: 2,
                padding_right: 2,
                padding_top: 1,
                padding_bottom: 1,
                overflow: Overflow::Hidden,
            ) {
                Text(
                    content: "Downloads",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value, colour))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<22} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: colour, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "↑/↓ ", color: theme::ACCENT_BRIGHT)
                    Text(content: "select  ", color: theme::MUTED)
                    Text(content: "/downloads cancel <id> ", color: theme::ACCENT_BRIGHT)
                    Text(content: "stop  ", color: theme::MUTED)
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close", color: theme::MUTED)
                }
            }
        }
    }
}
