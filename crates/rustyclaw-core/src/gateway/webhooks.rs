//! Webhook endpoints for external triggers.
//!
//! Provides HTTP endpoints for external systems to interact with RustyClaw:
//! - POST /hooks/wake   - Wake an idle agent (bring it out of sleep/standby)
//! - POST /hooks/agent  - Send a message directly to a specific agent session
//!
//! All webhook requests require a bearer token for authentication.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Webhook configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Whether webhooks are enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Bearer token required for webhook authentication.
    /// If not set, webhooks will be disabled for security.
    #[serde(default)]
    pub token: Option<String>,

    /// Maximum payload size in bytes (default: 64 KB).
    #[serde(default = "default_max_payload")]
    pub max_payload_bytes: usize,
}

fn default_max_payload() -> usize {
    65_536
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: None,
            max_payload_bytes: default_max_payload(),
        }
    }
}

/// Incoming webhook request body for /hooks/wake.
#[derive(Debug, Deserialize)]
pub struct WakeRequest {
    /// Optional reason for waking the agent.
    pub reason: Option<String>,
    /// Optional session/thread to target.
    pub session_id: Option<String>,
}

/// Incoming webhook request body for /hooks/agent.
#[derive(Debug, Deserialize)]
pub struct AgentRequest {
    /// The message to send to the agent.
    pub message: String,
    /// Target agent/session ID (uses default if not specified).
    pub agent_id: Option<String>,
    /// Optional metadata passed through to the agent context.
    pub metadata: Option<serde_json::Value>,
}

/// Webhook response.
#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Pending webhook messages that the gateway loop should pick up.
pub struct WebhookQueue {
    pending: Vec<PendingWebhook>,
}

/// A webhook event waiting to be processed.
#[derive(Debug)]
pub enum PendingWebhook {
    Wake {
        reason: Option<String>,
        session_id: Option<String>,
    },
    AgentMessage {
        message: String,
        agent_id: Option<String>,
        metadata: Option<serde_json::Value>,
    },
}

impl WebhookQueue {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Enqueue a wake event.
    pub fn enqueue_wake(&mut self, reason: Option<String>, session_id: Option<String>) {
        self.pending.push(PendingWebhook::Wake {
            reason,
            session_id,
        });
    }

    /// Enqueue an agent message.
    pub fn enqueue_agent_message(
        &mut self,
        message: String,
        agent_id: Option<String>,
        metadata: Option<serde_json::Value>,
    ) {
        self.pending.push(PendingWebhook::AgentMessage {
            message,
            agent_id,
            metadata,
        });
    }

    /// Drain all pending webhooks.
    pub fn drain(&mut self) -> Vec<PendingWebhook> {
        std::mem::take(&mut self.pending)
    }

    /// Check if there are pending webhooks.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

impl Default for WebhookQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared webhook queue.
pub type SharedWebhookQueue = Arc<Mutex<WebhookQueue>>;

/// Handle an incoming webhook HTTP request.
///
/// This is designed to be called from the health server's request handler
/// (or a dedicated webhook listener) when a POST to /hooks/* is received.
pub async fn handle_webhook_request(
    path: &str,
    body: &str,
    auth_header: Option<&str>,
    config: &WebhookConfig,
    queue: SharedWebhookQueue,
) -> (String, String, String) {
    // Check if webhooks are enabled
    if !config.enabled {
        return (
            "403 Forbidden".to_string(),
            "application/json".to_string(),
            json!({"error": "Webhooks are not enabled"}).to_string(),
        );
    }

    // Validate auth token
    if let Some(expected_token) = &config.token {
        let provided = auth_header
            .and_then(|h| h.strip_prefix("Bearer "))
            .unwrap_or("");

        if provided != expected_token {
            warn!("Webhook auth failed for {}", path);
            return (
                "401 Unauthorized".to_string(),
                "application/json".to_string(),
                json!({"error": "Invalid or missing authorization token"}).to_string(),
            );
        }
    } else {
        // No token configured — reject for security
        return (
            "403 Forbidden".to_string(),
            "application/json".to_string(),
            json!({"error": "Webhook token not configured"}).to_string(),
        );
    }

    // Check payload size
    if body.len() > config.max_payload_bytes {
        return (
            "413 Payload Too Large".to_string(),
            "application/json".to_string(),
            json!({"error": "Payload exceeds max size", "max_bytes": config.max_payload_bytes})
                .to_string(),
        );
    }

    match path {
        "/hooks/wake" => handle_wake(body, queue).await,
        "/hooks/agent" => handle_agent(body, queue).await,
        _ => (
            "404 Not Found".to_string(),
            "application/json".to_string(),
            json!({"error": "Unknown webhook endpoint", "available": ["/hooks/wake", "/hooks/agent"]})
                .to_string(),
        ),
    }
}

async fn handle_wake(body: &str, queue: SharedWebhookQueue) -> (String, String, String) {
    let req: WakeRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                "400 Bad Request".to_string(),
                "application/json".to_string(),
                json!({"error": format!("Invalid JSON: {}", e)}).to_string(),
            );
        }
    };

    info!(
        reason = ?req.reason,
        session = ?req.session_id,
        "Webhook: wake request received"
    );

    let mut q = queue.lock().await;
    q.enqueue_wake(req.reason.clone(), req.session_id.clone());

    let resp = WebhookResponse {
        status: "accepted".to_string(),
        message: Some("Wake signal queued".to_string()),
        session_id: req.session_id,
    };

    (
        "202 Accepted".to_string(),
        "application/json".to_string(),
        serde_json::to_string(&resp).unwrap_or_default(),
    )
}

async fn handle_agent(body: &str, queue: SharedWebhookQueue) -> (String, String, String) {
    let req: AgentRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                "400 Bad Request".to_string(),
                "application/json".to_string(),
                json!({"error": format!("Invalid JSON: {}", e)}).to_string(),
            );
        }
    };

    if req.message.is_empty() {
        return (
            "400 Bad Request".to_string(),
            "application/json".to_string(),
            json!({"error": "Message cannot be empty"}).to_string(),
        );
    }

    info!(
        agent = ?req.agent_id,
        message_len = req.message.len(),
        "Webhook: agent message received"
    );

    debug!(message = %req.message, "Webhook agent message content");

    let mut q = queue.lock().await;
    q.enqueue_agent_message(req.message, req.agent_id.clone(), req.metadata);

    let resp = WebhookResponse {
        status: "accepted".to_string(),
        message: Some("Message queued for agent".to_string()),
        session_id: req.agent_id,
    };

    (
        "202 Accepted".to_string(),
        "application/json".to_string(),
        serde_json::to_string(&resp).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_config_defaults() {
        let config = WebhookConfig::default();
        assert!(!config.enabled);
        assert!(config.token.is_none());
        assert_eq!(config.max_payload_bytes, 65_536);
    }

    #[test]
    fn test_webhook_queue() {
        let mut queue = WebhookQueue::new();
        assert!(queue.is_empty());

        queue.enqueue_wake(Some("test".to_string()), None);
        assert!(!queue.is_empty());

        queue.enqueue_agent_message("hello".to_string(), None, None);

        let drained = queue.drain();
        assert_eq!(drained.len(), 2);
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn test_webhook_disabled() {
        let config = WebhookConfig::default(); // disabled
        let queue = Arc::new(Mutex::new(WebhookQueue::new()));

        let (status, _, body) =
            handle_webhook_request("/hooks/wake", "{}", None, &config, queue).await;
        assert!(status.contains("403"));
        assert!(body.contains("not enabled"));
    }

    #[tokio::test]
    async fn test_webhook_auth_required() {
        let config = WebhookConfig {
            enabled: true,
            token: Some("secret123".to_string()),
            ..Default::default()
        };
        let queue = Arc::new(Mutex::new(WebhookQueue::new()));

        // No auth header
        let (status, _, _) =
            handle_webhook_request("/hooks/wake", "{}", None, &config, queue.clone()).await;
        assert!(status.contains("401"));

        // Wrong token
        let (status, _, _) = handle_webhook_request(
            "/hooks/wake",
            "{}",
            Some("Bearer wrong"),
            &config,
            queue.clone(),
        )
        .await;
        assert!(status.contains("401"));

        // Correct token
        let (status, _, _) = handle_webhook_request(
            "/hooks/wake",
            "{}",
            Some("Bearer secret123"),
            &config,
            queue,
        )
        .await;
        assert!(status.contains("202"));
    }

    #[tokio::test]
    async fn test_webhook_wake() {
        let config = WebhookConfig {
            enabled: true,
            token: Some("tok".to_string()),
            ..Default::default()
        };
        let queue = Arc::new(Mutex::new(WebhookQueue::new()));

        let body = r#"{"reason": "cron trigger"}"#;
        let (status, _, resp) = handle_webhook_request(
            "/hooks/wake",
            body,
            Some("Bearer tok"),
            &config,
            queue.clone(),
        )
        .await;

        assert!(status.contains("202"));
        assert!(resp.contains("accepted"));

        let q = queue.lock().await;
        assert!(!q.is_empty());
    }

    #[tokio::test]
    async fn test_webhook_agent_empty_message() {
        let config = WebhookConfig {
            enabled: true,
            token: Some("tok".to_string()),
            ..Default::default()
        };
        let queue = Arc::new(Mutex::new(WebhookQueue::new()));

        let body = r#"{"message": ""}"#;
        let (status, _, _) = handle_webhook_request(
            "/hooks/agent",
            body,
            Some("Bearer tok"),
            &config,
            queue,
        )
        .await;

        assert!(status.contains("400"));
    }
}
