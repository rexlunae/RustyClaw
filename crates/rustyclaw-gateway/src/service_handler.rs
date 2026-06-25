//! Gateway-side handlers for managed service requests.

use anyhow::Result;
use rustyclaw_core::gateway::TransportWriter;
use rustyclaw_core::gateway::protocol::frames::{
    ServerFrame, ServerFrameType, ServerPayload, ServiceInfoDto,
};
use tracing::debug;

/// Handle a `ServiceListRequest` frame: respond with all services.
pub async fn handle_service_list(writer: &mut dyn TransportWriter) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_service_manager() {
        Some(mgr) => {
            let mgr = mgr.read().await;
            let services: Vec<ServiceInfoDto> = mgr.list().into_iter().map(Into::into).collect();
            ServerPayload::ServiceListResult { services }
        }
        None => ServerPayload::ServiceListResult {
            services: Vec::new(),
        },
    };

    debug!("Sending service list result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::ServiceListResult,
            payload,
        })
        .await?;
    Ok(())
}

/// Handle a `ServiceStartRequest` frame: start a named service.
pub async fn handle_service_start(writer: &mut dyn TransportWriter, name: &str) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_service_manager() {
        Some(mgr) => {
            let mut mgr = mgr.write().await;
            match mgr.start(name).await {
                Ok(info) => ServerPayload::ServiceActionResult {
                    ok: true,
                    service: Some(info.into()),
                    message: None,
                },
                Err(e) => ServerPayload::ServiceActionResult {
                    ok: false,
                    service: None,
                    message: Some(e),
                },
            }
        }
        None => ServerPayload::ServiceActionResult {
            ok: false,
            service: None,
            message: Some("Service manager not initialised".into()),
        },
    };

    debug!(service = %name, "Sending service start result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::ServiceStartResult,
            payload,
        })
        .await?;
    Ok(())
}

/// Handle a `ServiceStopRequest` frame: stop a named service.
pub async fn handle_service_stop(writer: &mut dyn TransportWriter, name: &str) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_service_manager() {
        Some(mgr) => {
            let mut mgr = mgr.write().await;
            match mgr.stop(name).await {
                Ok(info) => ServerPayload::ServiceActionResult {
                    ok: true,
                    service: Some(info.into()),
                    message: None,
                },
                Err(e) => ServerPayload::ServiceActionResult {
                    ok: false,
                    service: None,
                    message: Some(e),
                },
            }
        }
        None => ServerPayload::ServiceActionResult {
            ok: false,
            service: None,
            message: Some("Service manager not initialised".into()),
        },
    };

    debug!(service = %name, "Sending service stop result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::ServiceStopResult,
            payload,
        })
        .await?;
    Ok(())
}

/// Handle a `ServiceRestartRequest` frame: restart a named service.
pub async fn handle_service_restart(writer: &mut dyn TransportWriter, name: &str) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_service_manager() {
        Some(mgr) => {
            let mut mgr = mgr.write().await;
            match mgr.restart(name).await {
                Ok(info) => ServerPayload::ServiceActionResult {
                    ok: true,
                    service: Some(info.into()),
                    message: None,
                },
                Err(e) => ServerPayload::ServiceActionResult {
                    ok: false,
                    service: None,
                    message: Some(e),
                },
            }
        }
        None => ServerPayload::ServiceActionResult {
            ok: false,
            service: None,
            message: Some("Service manager not initialised".into()),
        },
    };

    debug!(service = %name, "Sending service restart result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::ServiceRestartResult,
            payload,
        })
        .await?;
    Ok(())
}

/// Handle a `ServiceLogsRequest` frame: return recent log lines.
pub async fn handle_service_logs(
    writer: &mut dyn TransportWriter,
    name: &str,
    tail: Option<usize>,
) -> Result<()> {
    let payload = match rustyclaw_core::runtime_ctx::get_service_manager() {
        Some(mgr) => {
            let mgr = mgr.read().await;
            match mgr.logs(name, tail) {
                Ok(lines) => ServerPayload::ServiceLogsResult {
                    ok: true,
                    name: name.to_string(),
                    lines,
                    message: None,
                },
                Err(e) => ServerPayload::ServiceLogsResult {
                    ok: false,
                    name: name.to_string(),
                    lines: Vec::new(),
                    message: Some(e),
                },
            }
        }
        None => ServerPayload::ServiceLogsResult {
            ok: false,
            name: name.to_string(),
            lines: Vec::new(),
            message: Some("Service manager not initialised".into()),
        },
    };

    debug!(service = %name, "Sending service logs result");
    writer
        .send(&ServerFrame {
            frame_type: ServerFrameType::ServiceLogsResult,
            payload,
        })
        .await?;
    Ok(())
}
