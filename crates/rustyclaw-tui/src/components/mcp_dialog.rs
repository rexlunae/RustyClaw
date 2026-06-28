// ── MCP dialog — MCP server management overlay ──────────────────────────────
//
// Opened with /mcp; Esc closes. Shows connected MCP servers with their
// tools, health status, and connection details.

use crate::theme;
use iocraft::prelude::*;

#[allow(dead_code)]
#[derive(Default, Props)]
pub struct McpDialogProps {
    pub data: Option<rustyclaw_view::McpPanelData>,
}

#[component]
pub fn McpDialog(props: &McpDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.data {
        if data.servers.is_empty() {
            rows.push(("".into(), "(no MCP servers configured)".into()));
        } else {
            rows.push((
                "Summary".into(),
                format!(
                    "{} connected / {} total",
                    data.connected_count(),
                    data.total_count(),
                ),
            ));
            rows.push(("".into(), "".into()));

            for (i, server) in data.servers.iter().enumerate() {
                let marker = if Some(i) == data.selected { ">" } else { " " };
                let status_symbol = match server.status.as_str() {
                    "connected" => "[*]",
                    "connecting" => "[~]",
                    "error" => "[!]",
                    _ => "[-]",
                };

                rows.push((
                    format!("{} {}", marker, server.name),
                    format!(
                        "{} {} ({} tools)",
                        status_symbol,
                        server.status,
                        server.tool_count()
                    ),
                ));
                rows.push(("".into(), format!("  Target: {}", server.target())));
            }
        }
    } else {
        rows.push(("".into(), "(MCP system not initialised)".into()));
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
                    content: "MCP Servers",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<16} ", label) },
                            color: theme::ACCENT_BRIGHT,
                        )
                        Text(content: value, color: theme::TEXT, wrap: TextWrap::Wrap)
                    }
                }))

                View(height: 1)

                View(flex_direction: FlexDirection::Row) {
                    Text(content: "Esc ", color: theme::ACCENT_BRIGHT)
                    Text(content: "close  ", color: theme::MUTED)
                    Text(content: "c ", color: theme::ACCENT_BRIGHT)
                    Text(content: "connect  ", color: theme::MUTED)
                    Text(content: "d ", color: theme::ACCENT_BRIGHT)
                    Text(content: "disconnect", color: theme::MUTED)
                }
            }
        }
    }
}
