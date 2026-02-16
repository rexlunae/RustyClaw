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

/// Shared health statistics
pub struct HealthStats {
    pub start_time: u64,
    pub total_connections: AtomicU64,
    pub active_connections: AtomicU64,
    pub total_messages: AtomicU64,
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
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.start_time
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

    eprintln!("[health] Listening on http://{}", listen_addr);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                eprintln!("[health] Shutting down health check server");
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let stats_clone = stats.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_health_request(stream, stats_clone).await {
                        eprintln!("[health] Request error: {}", e);
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
    let (status, body) = match path {
        "/health" => {
            // Simple health check
            let response = json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_secs": stats.uptime_secs(),
            });
            ("200 OK", response.to_string())
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
                },
                "timestamp": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            });
            ("200 OK", response.to_string())
        }
        _ => {
            // 404 Not Found
            let response = json!({
                "error": "Not Found",
                "available_endpoints": ["/health", "/status"],
            });
            ("404 Not Found", response.to_string())
        }
    };

    // Send HTTP response
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        status,
        body.len(),
        body
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}
