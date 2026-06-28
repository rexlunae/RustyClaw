// ── Channels dialog — messenger channel status overlay ───────────────────────
//
// Opened with /channels; Esc closes. Shows messenger channels with
// their pairing/online status and pair/unpair actions.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct ChannelsDialogProps {
    pub data: Option<rustyclaw_view::ChannelsPanelData>,
}

#[component]
pub fn ChannelsDialog(props: &ChannelsDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.channels.is_empty() {
            rows.push(("".into(), "(no channels configured)".into()));
        } else {
            rows.push((
                "Summary".into(),
                format!(
                    "{} online / {} paired / {} total",
                    data.online_count(),
                    data.paired_count(),
                    data.total_count(),
                ),
            ));
            rows.push(("".into(), "".into()));

            for (i, ch) in data.channels.iter().enumerate() {
                let marker = if Some(i) == data.selected { ">" } else { " " };
                let status_symbol = if ch.online {
                    "[*]"
                } else if ch.paired {
                    "[~]"
                } else {
                    "[-]"
                };

                rows.push((
                    format!("{} {} {}", marker, ch.channel_icon(), ch.name),
                    format!(
                        "{} {} ({})",
                        status_symbol,
                        ch.status_label(),
                        ch.channel_type
                    ),
                ));
            }
        }
    } else {
        rows.push(("".into(), "(channel system not initialised)".into()));
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 65pct,
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
                    content: "Channels",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<20} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "p ", color: theme::ACCENT_BRIGHT)
                    Text(content: "pair/unpair", color: theme::MUTED)
                }
            }
        }
    }
}
