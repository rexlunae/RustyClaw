//! Gateway-side handlers for kernel awareness requests (host info, load status).

use anyhow::Result;
use rustyclaw_core::gateway::TransportWriter;
use rustyclaw_core::gateway::protocol::frames::{
    GpuInfoDto, ServerFrame, ServerFrameType, ServerPayload,
};
use tracing::debug;

/// Handle a `HostInfoRequest` frame: respond with hardware capabilities.
pub async fn handle_host_info_request(writer: &mut dyn TransportWriter) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_host() {
        Some(host) => ServerPayload::HostInfoResult {
            hostname: host.hostname.clone(),
            os: format!("{} {}", host.os_name, host.os_version),
            arch: host.arch.clone(),
            cpu_brand: host.cpu_brand.clone(),
            cpu_cores_physical: host.cpu_cores_physical,
            cpu_cores_logical: host.cpu_cores_logical,
            cpu_frequency_mhz: host.cpu_frequency_mhz,
            total_memory_bytes: host.total_memory_bytes,
            total_swap_bytes: host.total_swap_bytes,
            disk_total_bytes: host.disk_total_bytes,
            disk_available_bytes: host.disk_available_bytes,
            gpus: host
                .gpus
                .iter()
                .map(|g| GpuInfoDto {
                    name: g.name.clone(),
                    vendor: g.vendor.clone(),
                    vram_bytes: g.vram_bytes.unwrap_or(0),
                })
                .collect(),
            summary: host.summary(),
        },
        None => ServerPayload::Error {
            ok: false,
            message: "Host capabilities not yet detected".into(),
        },
    };

    debug!("Sending host info result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::HostInfoResult,
            payload,
        })
        .await?;
    Ok(())
}

/// Handle a `LoadStatusRequest` frame: respond with current load metrics.
pub async fn handle_load_status_request(writer: &mut dyn TransportWriter) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_load_tracker() {
        Some(tracker) => {
            let tracker = tracker.read().await;
            let score = tracker.load_score() as f64;
            let avg = tracker.avg_load_score() as f64;
            let (cpu_pct, mem_pct) = tracker
                .latest()
                .map(|s| (s.cpu_usage_percent, s.memory_utilization() * 100.0))
                .unwrap_or((0.0, 0.0));
            ServerPayload::LoadStatusResult {
                load_score: score,
                avg_load_score: avg,
                cpu_percent: cpu_pct,
                memory_percent: mem_pct,
                summary: tracker.summary(),
            }
        }
        None => ServerPayload::Error {
            ok: false,
            message: "Load tracker not initialised".into(),
        },
    };

    debug!("Sending load status result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::LoadStatusResult,
            payload,
        })
        .await?;
    Ok(())
}
