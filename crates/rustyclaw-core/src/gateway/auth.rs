use anyhow::Result;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn, instrument};

use super::protocol::server;
use super::{CopilotSession, ClientFrameType, ClientPayload};
use crate::providers;

/// Maximum consecutive TOTP failures before lockout.
const MAX_TOTP_FAILURES: u32 = 3;
/// Duration of the lockout after exceeding the failure limit.
const TOTP_LOCKOUT_SECS: u64 = 30;
/// Window within which failures are counted (resets after this).
const TOTP_FAILURE_WINDOW_SECS: u64 = 60;

/// Per-IP TOTP failure tracking.
#[derive(Debug, Clone)]
pub struct TotpAttempt {
    pub failures: u32,
    pub first_failure: Instant,
    pub lockout_until: Option<Instant>,
}

/// Thread-safe rate limiter shared across all connections.
pub type RateLimiter = Arc<Mutex<HashMap<IpAddr, TotpAttempt>>>;

pub fn new_rate_limiter() -> RateLimiter {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Check whether an IP is currently locked out. Returns the number of
/// seconds remaining if locked, or `None` if the IP may attempt auth.
#[instrument(skip(limiter), fields(%ip))]
pub async fn check_rate_limit(limiter: &RateLimiter, ip: IpAddr) -> Option<u64> {
    let mut map = limiter.lock().await;
    if let Some(attempt) = map.get_mut(&ip) {
        // Expire old failure windows.
        if attempt.first_failure.elapsed().as_secs() > TOTP_FAILURE_WINDOW_SECS {
            debug!("Failure window expired, resetting");
            *attempt = TotpAttempt {
                failures: 0,
                first_failure: Instant::now(),
                lockout_until: None,
            };
            return None;
        }
        // Check active lockout.
        if let Some(until) = attempt.lockout_until {
            if Instant::now() < until {
                let remaining = (until - Instant::now()).as_secs() + 1;
                debug!(remaining_secs = remaining, "IP is locked out");
                return Some(remaining);
            }
            // Lockout expired — reset.
            debug!("Lockout expired, resetting");
            *attempt = TotpAttempt {
                failures: 0,
                first_failure: Instant::now(),
                lockout_until: None,
            };
        }
    }
    None
}

/// Record a failed TOTP attempt. Returns `true` if the IP is now locked out.
#[instrument(skip(limiter), fields(%ip))]
pub async fn record_totp_failure(limiter: &RateLimiter, ip: IpAddr) -> bool {
    let mut map = limiter.lock().await;
    let attempt = map.entry(ip).or_insert_with(|| TotpAttempt {
        failures: 0,
        first_failure: Instant::now(),
        lockout_until: None,
    });

    // Reset if the window has expired.
    if attempt.first_failure.elapsed().as_secs() > TOTP_FAILURE_WINDOW_SECS {
        attempt.failures = 0;
        attempt.first_failure = Instant::now();
        attempt.lockout_until = None;
    }

    attempt.failures += 1;
    if attempt.failures >= MAX_TOTP_FAILURES {
        attempt.lockout_until = Some(Instant::now() + std::time::Duration::from_secs(TOTP_LOCKOUT_SECS));
        warn!(failures = attempt.failures, lockout_secs = TOTP_LOCKOUT_SECS, "TOTP lockout triggered");
        true
    } else {
        debug!(failures = attempt.failures, max = MAX_TOTP_FAILURES, "TOTP failure recorded");
        false
    }
}

/// Clear failure tracking for an IP after a successful auth.
#[instrument(skip(limiter), fields(%ip))]
pub async fn clear_rate_limit(limiter: &RateLimiter, ip: IpAddr) {
    debug!("Clearing rate limit after successful auth");
    let mut map = limiter.lock().await;
    map.remove(&ip);
}

/// Resolve the effective bearer token for an API call.
///
/// For Copilot providers the raw API key is an OAuth token that must be
/// exchanged for a short-lived session token.  For all other providers
/// the raw key is returned as-is.
#[instrument(skip(http, raw_key, session), fields(%provider))]
pub async fn resolve_bearer_token(
    http: &reqwest::Client,
    provider: &str,
    raw_key: Option<&str>,
    session: Option<&CopilotSession>,
) -> Result<Option<String>> {
    if providers::needs_copilot_session(provider) {
        if let Some(session) = session {
            debug!("Exchanging Copilot OAuth token for session token");
            return Ok(Some(session.get_token(http).await?));
        }
    }
    Ok(raw_key.map(String::from))
}

/// Wait for an `auth_response` frame from the client.
///
/// Reads WebSocket binary messages until we get a frame with
/// `ClientFrameType::AuthResponse`, or the connection drops.
#[instrument(skip(reader))]
pub async fn wait_for_auth_response<S>(
    reader: &mut futures_util::stream::SplitStream<WebSocketStream<S>>,
) -> Result<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    debug!("Waiting for auth_response frame");
    while let Some(msg) = reader.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                // Deserialize bincode frame
                match server::parse_client_frame(&data) {
                    Ok(frame) => {
                        if frame.frame_type == ClientFrameType::AuthResponse {
                            if let ClientPayload::AuthResponse { code } = frame.payload {
                                debug!("Received valid auth_response");
                                return Ok(code);
                            }
                            anyhow::bail!("AuthResponse frame has wrong payload type");
                        }
                        // Ignore non-auth frames during the handshake.
                    }
                    Err(e) => {
                        warn!(bytes = data.len(), error = %e, "Failed to parse client frame during auth handshake");
                        // Continue waiting for valid frame
                    }
                }
            }
            Ok(Message::Close(_)) => {
                warn!("Client disconnected during authentication");
                anyhow::bail!("Client disconnected during authentication");
            }
            Err(e) => {
                warn!(error = %e, "WebSocket error during authentication");
                anyhow::bail!("WebSocket error during authentication: {}", e);
            }
            _ => {
                // Ignore ping/pong/text
            }
        }
    }
    warn!("Connection closed before authentication completed");
    anyhow::bail!("Connection closed before authentication completed")
}
