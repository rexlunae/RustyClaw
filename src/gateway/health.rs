//! Health check and status endpoint for remote monitoring.
//!
//! Provides HTTP endpoints for:
//! - /health - Simple health check (returns 200 OK if running)
//! - /status - Detailed status with metrics

use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use anyhow::{Context, Result};
use tracing::{debug, info};

/// Shared health statistics
pub struct HealthStats {
    pub start_time: u64,
    pub total_connections: AtomicU64,
    pub active_connections: AtomicU64,
    pub total_messages: AtomicU64,
    pub model_requests: AtomicU64,
    pub model_errors: AtomicU64,
    pub tool_calls: AtomicU64,
    pub tool_errors: AtomicU64,
}

impl HealthStats {
    pub fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        Self {
            start_time: now,
            total_connections: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            total_messages: AtomicU64::new(0),
            model_requests: AtomicU64::new(0),
            model_errors: AtomicU64::new(0),
            tool_calls: AtomicU64::new(0),
            tool_errors: AtomicU64::new(0),
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.start_time
    }
    
    /// Record a model request
    pub fn record_model_request(&self) {
        self.model_requests.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a model error
    pub fn record_model_error(&self) {
        self.model_errors.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a tool call
    pub fn record_tool_call(&self) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Record a tool error
    pub fn record_tool_error(&self) {
        self.tool_errors.fetch_add(1, Ordering::Relaxed);
    }
}

impl Default for HealthStats {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedHealthStats = Arc<HealthStats>;

/// Start HTTP health check server
pub async fn start_health_server(
    listen_addr: &str,
    stats: SharedHealthStats,
    cancel: CancellationToken,
) -> Result<()> {
    let listener = TcpListener::bind(listen_addr)
        .await
        .context("Failed to bind health check server")?;

    info!(address = %listen_addr, "Health check server listening");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                info!("Shutting down health check server");
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let stats_clone = stats.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_health_request(stream, stats_clone).await {
                        debug!(error = %e, "Health check request error");
                    }
                });
            }
        }
    }

    Ok(())
}

async fn handle_health_request(
    mut stream: tokio::net::TcpStream,
    stats: SharedHealthStats,
) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Read HTTP request (simple, just get the path)
    let mut buffer = [0u8; 1024];
    let n = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..n]);

    // Parse path
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    // Generate response
    let (status, content_type, body) = match path {
        "/health" => {
            // Simple health check
            let response = json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_secs": stats.uptime_secs(),
            });
            ("200 OK", "application/json", response.to_string())
        }
        "/status" => {
            // Detailed status
            let response = json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_secs": stats.uptime_secs(),
                "metrics": {
                    "total_connections": stats.total_connections.load(Ordering::Relaxed),
                    "active_connections": stats.active_connections.load(Ordering::Relaxed),
                    "total_messages": stats.total_messages.load(Ordering::Relaxed),
                    "model_requests": stats.model_requests.load(Ordering::Relaxed),
                    "model_errors": stats.model_errors.load(Ordering::Relaxed),
                    "tool_calls": stats.tool_calls.load(Ordering::Relaxed),
                    "tool_errors": stats.tool_errors.load(Ordering::Relaxed),
                },
                "timestamp": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            });
            ("200 OK", "application/json", response.to_string())
        }
        "/metrics" => {
            // Prometheus-compatible metrics endpoint
            let version = env!("CARGO_PKG_VERSION");
            let uptime = stats.uptime_secs();
            let total_conn = stats.total_connections.load(Ordering::Relaxed);
            let active_conn = stats.active_connections.load(Ordering::Relaxed);
            let total_msgs = stats.total_messages.load(Ordering::Relaxed);
            let model_reqs = stats.model_requests.load(Ordering::Relaxed);
            let model_errs = stats.model_errors.load(Ordering::Relaxed);
            let tool_calls = stats.tool_calls.load(Ordering::Relaxed);
            let tool_errs = stats.tool_errors.load(Ordering::Relaxed);
            
            let body = format!(
                "# HELP rustyclaw_up Whether RustyClaw is running (1 = up)\n\
                 # TYPE rustyclaw_up gauge\n\
                 rustyclaw_up 1\n\
                 \n\
                 # HELP rustyclaw_info Version and build info\n\
                 # TYPE rustyclaw_info gauge\n\
                 rustyclaw_info{{version=\"{}\"}} 1\n\
                 \n\
                 # HELP rustyclaw_uptime_seconds How long the gateway has been running\n\
                 # TYPE rustyclaw_uptime_seconds counter\n\
                 rustyclaw_uptime_seconds {}\n\
                 \n\
                 # HELP rustyclaw_connections_total Total WebSocket connections since start\n\
                 # TYPE rustyclaw_connections_total counter\n\
                 rustyclaw_connections_total {}\n\
                 \n\
                 # HELP rustyclaw_connections_active Current active WebSocket connections\n\
                 # TYPE rustyclaw_connections_active gauge\n\
                 rustyclaw_connections_active {}\n\
                 \n\
                 # HELP rustyclaw_messages_total Total messages processed\n\
                 # TYPE rustyclaw_messages_total counter\n\
                 rustyclaw_messages_total {}\n\
                 \n\
                 # HELP rustyclaw_model_requests_total Total model API requests\n\
                 # TYPE rustyclaw_model_requests_total counter\n\
                 rustyclaw_model_requests_total {}\n\
                 \n\
                 # HELP rustyclaw_model_errors_total Total model API errors\n\
                 # TYPE rustyclaw_model_errors_total counter\n\
                 rustyclaw_model_errors_total {}\n\
                 \n\
                 # HELP rustyclaw_tool_calls_total Total tool invocations\n\
                 # TYPE rustyclaw_tool_calls_total counter\n\
                 rustyclaw_tool_calls_total {}\n\
                 \n\
                 # HELP rustyclaw_tool_errors_total Total tool execution errors\n\
                 # TYPE rustyclaw_tool_errors_total counter\n\
                 rustyclaw_tool_errors_total {}\n",
                version, uptime, total_conn, active_conn, total_msgs,
                model_reqs, model_errs, tool_calls, tool_errs
            );
            ("200 OK", "text/plain; version=0.0.4; charset=utf-8", body)
        }
        _ => {
            // 404 Not Found
            let response = json!({
                "error": "Not Found",
                "available_endpoints": ["/health", "/status", "/metrics"],
            });
            ("404 Not Found", "application/json", response.to_string())
        }
    };

    // Send HTTP response
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}
