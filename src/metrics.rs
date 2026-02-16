//! Prometheus metrics for RustyClaw gateway
//!
//! Provides observability metrics for production monitoring:
//! - Gateway connection metrics
//! - Authentication metrics
//! - Request/tool execution metrics
//! - Provider API call metrics
//! - Token usage tracking

use lazy_static::lazy_static;
use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Encoder, Gauge,
    HistogramVec, TextEncoder,
};
use std::net::SocketAddr;
use std::time::Duration;
use warp::Filter;

lazy_static! {
    /// Active WebSocket connections
    pub static ref GATEWAY_CONNECTIONS: Gauge = register_gauge!(
        "rustyclaw_gateway_connections",
        "Number of active WebSocket connections"
    )
    .unwrap();

    /// Total authentication attempts
    pub static ref AUTH_ATTEMPTS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_auth_attempts_total",
        "Total number of authentication attempts",
        &["result"]  // "success" or "failure"
    )
    .unwrap();

    /// Request duration histogram
    pub static ref REQUEST_DURATION_SECONDS: HistogramVec = register_histogram_vec!(
        "rustyclaw_request_duration_seconds",
        "Request processing duration in seconds",
        &["request_type"],  // "chat", "control", etc.
        vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0]
    )
    .unwrap();

    /// Tool execution counts
    pub static ref TOOL_CALLS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_tool_calls_total",
        "Total number of tool executions",
        &["tool_name", "result"]  // result: "success" or "error"
    )
    .unwrap();

    /// Provider API call counts
    pub static ref PROVIDER_REQUESTS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_provider_requests_total",
        "Total number of provider API requests",
        &["provider", "result"]  // provider: "anthropic", "openai", etc.
    )
    .unwrap();

    /// Token usage tracking
    pub static ref TOKENS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_tokens_total",
        "Total number of tokens processed",
        &["provider", "type"]  // type: "input" or "output"
    )
    .unwrap();

    /// Prompt injection detection counts
    pub static ref PROMPT_INJECTION_DETECTED: CounterVec = register_counter_vec!(
        "rustyclaw_prompt_injection_detected_total",
        "Total number of prompt injection attempts detected",
        &["action"]  // "warn", "block", "sanitize"
    )
    .unwrap();

    /// SSRF validation failures
    pub static ref SSRF_BLOCKED_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_ssrf_blocked_total",
        "Total number of SSRF attempts blocked",
        &["reason"]  // "private_ip", "cloud_metadata", etc.
    )
    .unwrap();

    /// Retry attempts for transient outbound failures.
    pub static ref RETRY_ATTEMPTS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_retry_attempts_total",
        "Total number of outbound retry attempts",
        &["provider", "reason"]
    )
    .unwrap();

    /// Retry delay histogram (seconds).
    pub static ref RETRY_DELAY_SECONDS: HistogramVec = register_histogram_vec!(
        "rustyclaw_retry_delay_seconds",
        "Delay applied before retrying outbound requests",
        &["provider", "reason"],
        vec![0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0]
    )
    .unwrap();
}

/// Start the Prometheus metrics HTTP server
///
/// Serves metrics on the specified address (default: localhost:9090)
/// Returns a future that runs until cancelled
pub async fn start_metrics_server(addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[metrics] Starting Prometheus metrics server on {}", addr);

    // Create the /metrics route
    let metrics_route = warp::path("metrics")
        .and(warp::get())
        .map(|| {
            let encoder = TextEncoder::new();
            let metric_families = prometheus::gather();
            let mut buffer = Vec::new();

            if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
                eprintln!("[metrics] Error encoding metrics: {}", e);
                return warp::reply::with_status(
                    "Error encoding metrics".to_string(),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                );
            }

            warp::reply::with_status(
                String::from_utf8_lossy(&buffer).to_string(),
                warp::http::StatusCode::OK,
            )
        });

    // Health check endpoint
    let health_route = warp::path("health")
        .and(warp::get())
        .map(|| warp::reply::with_status("OK", warp::http::StatusCode::OK));

    let routes = metrics_route.or(health_route);

    eprintln!("[metrics] Metrics available at http://{}/metrics", addr);
    eprintln!("[metrics] Health check available at http://{}/health", addr);

    warp::serve(routes).run(addr).await;

    Ok(())
}

/// Helper to record connection opened
pub fn record_connection_opened() {
    GATEWAY_CONNECTIONS.inc();
}

/// Helper to record connection closed
pub fn record_connection_closed() {
    GATEWAY_CONNECTIONS.dec();
}

/// Helper to record authentication attempt
pub fn record_auth_attempt(success: bool) {
    let result = if success { "success" } else { "failure" };
    AUTH_ATTEMPTS_TOTAL.with_label_values(&[result]).inc();
}

/// Helper to record tool call
pub fn record_tool_call(tool_name: &str, success: bool) {
    let result = if success { "success" } else { "error" };
    TOOL_CALLS_TOTAL
        .with_label_values(&[tool_name, result])
        .inc();
}

/// Helper to record provider request
pub fn record_provider_request(provider: &str, success: bool) {
    let result = if success { "success" } else { "error" };
    PROVIDER_REQUESTS_TOTAL
        .with_label_values(&[provider, result])
        .inc();
}

/// Helper to record token usage
pub fn record_tokens(provider: &str, input_tokens: u64, output_tokens: u64) {
    if input_tokens > 0 {
        TOKENS_TOTAL
            .with_label_values(&[provider, "input"])
            .inc_by(input_tokens as f64);
    }
    if output_tokens > 0 {
        TOKENS_TOTAL
            .with_label_values(&[provider, "output"])
            .inc_by(output_tokens as f64);
    }
}

/// Helper to record prompt injection detection
pub fn record_prompt_injection(action: &str) {
    PROMPT_INJECTION_DETECTED.with_label_values(&[action]).inc();
}

/// Helper to record SSRF block
pub fn record_ssrf_blocked(reason: &str) {
    SSRF_BLOCKED_TOTAL.with_label_values(&[reason]).inc();
}

/// Helper to record outbound retry behavior.
pub fn record_retry(provider: &str, reason: &str, delay: Duration) {
    RETRY_ATTEMPTS_TOTAL
        .with_label_values(&[provider, reason])
        .inc();
    RETRY_DELAY_SECONDS
        .with_label_values(&[provider, reason])
        .observe(delay.as_secs_f64());
}

/// Timer for request duration tracking
pub struct RequestTimer {
    request_type: String,
    start: std::time::Instant,
}

impl RequestTimer {
    pub fn new(request_type: &str) -> Self {
        Self {
            request_type: request_type.to_string(),
            start: std::time::Instant::now(),
        }
    }
}

impl Drop for RequestTimer {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_secs_f64();
        REQUEST_DURATION_SECONDS
            .with_label_values(&[&self.request_type])
            .observe(duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registration() {
        // Just verify that all metrics are properly registered
        // by accessing them without panicking
        let _ = &*GATEWAY_CONNECTIONS;
        let _ = &*AUTH_ATTEMPTS_TOTAL;
        let _ = &*REQUEST_DURATION_SECONDS;
        let _ = &*TOOL_CALLS_TOTAL;
        let _ = &*PROVIDER_REQUESTS_TOTAL;
        let _ = &*TOKENS_TOTAL;
        let _ = &*PROMPT_INJECTION_DETECTED;
        let _ = &*SSRF_BLOCKED_TOTAL;
        let _ = &*RETRY_ATTEMPTS_TOTAL;
        let _ = &*RETRY_DELAY_SECONDS;
    }

    #[test]
    fn test_connection_metrics() {
        let initial = GATEWAY_CONNECTIONS.get();
        record_connection_opened();
        assert_eq!(GATEWAY_CONNECTIONS.get(), initial + 1.0);
        record_connection_closed();
        assert_eq!(GATEWAY_CONNECTIONS.get(), initial);
    }

    #[test]
    fn test_auth_metrics() {
        record_auth_attempt(true);
        record_auth_attempt(false);
        // Metrics are recorded, no panic
    }

    #[test]
    fn test_request_timer() {
        let _timer = RequestTimer::new("test");
        // Timer will record duration on drop
    }
}
