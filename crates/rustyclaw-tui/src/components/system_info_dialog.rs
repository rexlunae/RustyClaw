// ── System info dialog — host capabilities + load status overlay ─────────────
//
// Opened with Ctrl-H; Esc closes.  Displays host hardware info (CPU,
// GPU, RAM, disk) and current load status (load score, CPU%, memory%).

use crate::theme;
use iocraft::prelude::*;

#[derive(Default, Props)]
pub struct SystemInfoDialogProps {
    pub host: Option<rustyclaw_view::HostInfoData>,
    pub load: Option<rustyclaw_view::LoadStatusData>,
}

#[component]
pub fn SystemInfoDialog(props: &SystemInfoDialogProps) -> impl Into<AnyElement<'static>> {
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ref h) = props.host {
        rows.push(("Hostname".into(), h.hostname.clone()));
        rows.push(("OS / Arch".into(), format!("{} ({})", h.os, h.arch)));
        rows.push((
            "CPU".into(),
            format!(
                "{} ({}/{}c @ {}MHz)",
                h.cpu_brand, h.cpu_cores_physical, h.cpu_cores_logical, h.cpu_frequency_mhz,
            ),
        ));
        rows.push(("RAM".into(), format!("{:.1} GiB", h.total_memory_gib)));
        if h.total_swap_gib > 0.0 {
            rows.push(("Swap".into(), format!("{:.1} GiB", h.total_swap_gib)));
        }
        rows.push((
            "Disk".into(),
            format!(
                "{:.1} / {:.1} GiB free ({})",
                h.disk_available_gib,
                h.disk_total_gib,
                h.disk_used_percent(),
            ),
        ));
        for (i, gpu) in h.gpus.iter().enumerate() {
            rows.push((
                format!("GPU {}", i),
                format!(
                    "{} ({}, {:.1} GiB VRAM)",
                    gpu.name, gpu.vendor, gpu.vram_gib,
                ),
            ));
        }
    } else {
        rows.push(("Host".into(), "(not yet detected)".into()));
    }

    rows.push(("".into(), "".into())); // spacer

    if let Some(ref l) = props.load {
        let score_color_label = l.load_label();
        rows.push((
            "Load Score".into(),
            format!("{:.2} [{}]", l.load_score, score_color_label,),
        ));
        rows.push(("Avg Load".into(), format!("{:.2}", l.avg_load_score)));
        rows.push(("CPU Usage".into(), format!("{:.1}%", l.cpu_percent)));
        rows.push(("Memory".into(), format!("{:.1}%", l.memory_percent)));
    } else {
        rows.push(("Load".into(), "(not yet sampled)".into()));
    }

    element! {
        View(
            width: 100pct,
            height: 100pct,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        ) {
            View(
                width: 60pct,
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
                    content: "System Information",
                    color: theme::INFO,
                    weight: Weight::Bold,
                )

                View(height: 1)

                #(rows.into_iter().enumerate().map(|(i, (label, value))| element! {
                    View(key: i as u32, flex_direction: FlexDirection::Row) {
                        Text(
                            content: if label.is_empty() { String::new() } else { format!("{:<12} ", label) },
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
