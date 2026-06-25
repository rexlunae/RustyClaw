// ── Services dialog — managed backend services overlay ──────────────────────
//
// Opened with Ctrl-S (when available); Esc closes.  Shows all managed
// services with their name, type, status, uptime, and restart count.

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct ServicesDialogProps {
    pub services: Option<rustyclaw_view::ServiceListData>,
}

#[component]
pub fn ServicesDialog(props: &ServicesDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref data) = props.services {
        if data.services.is_empty() {
            rows.push(("".into(), "(no services configured)".into()));
        } else {
            rows.push((
                "Summary".into(),
                format!(
                    "{} running / {} total",
                    data.running_count(),
                    data.total_count(),
                ),
            ));
            rows.push(("".into(), "".into()));

            for svc in &data.services {
                let status_symbol = match svc.status.as_str() {
                    "Running" => "[*]",
                    "Starting" => "[~]",
                    "Unhealthy" => "[!]",
                    "Stopping" => "[~]",
                    "Failed" => "[X]",
                    _ => "[-]",
                };

                rows.push((
                    svc.name.clone(),
                    format!("{} {} ({})", status_symbol, svc.status, svc.service_type,),
                ));

                let mut details = Vec::new();
                if let Some(pid) = svc.pid {
                    details.push(format!("PID {}", pid));
                }
                details.push(format!("up {}", svc.uptime_display()));
                if svc.restart_count > 0 {
                    details.push(format!("{} restarts", svc.restart_count));
                }
                if svc.mcp_tools > 0 {
                    details.push(format!("{} MCP tools", svc.mcp_tools));
                }
                if let Some(ok) = svc.health_ok {
                    details.push(if ok {
                        "healthy".into()
                    } else {
                        "unhealthy".into()
                    });
                }
                rows.push(("".into(), format!("  {}", details.join(" | "))));
            }
        }
    } else {
        rows.push(("".into(), "(service manager not initialised)".into()));
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
                    content: "Managed Services",
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
                    Text(content: "close", color: theme::MUTED)
                }
            }
        }
    }
}
