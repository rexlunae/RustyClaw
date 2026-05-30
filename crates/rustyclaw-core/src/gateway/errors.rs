//! Unified gateway error types and centralised handling.
//!
//! All errors that the gateway can encounter during a dispatch cycle are
//! represented by [`GatewayError`], which implements [`std::error::Error`]
//! so it can be wrapped inside an [`anyhow_tracing::Error`] for structured
//! field propagation.
//!
//! Flow:
//!
//! 1. **Classify** — [`GatewayError::classify_model_error`] inspects a raw
//!    `anyhow::Error` (plus provider context) and returns an
//!    `anyhow_tracing::Error` wrapping the appropriate `GatewayError`
//!    variant, with structured fields (`provider`, `gateway_error_kind`,
//!    etc.) attached.
//!
//! 2. **Handle** — [`handle`] downcasts the `anyhow_tracing::Error` to
//!    recover the `GatewayError` variant and executes the correct recovery
//!    or reporting logic (device flow, API key prompt, info/error frame to
//!    the client, etc.).  Returns a [`ControlFlow`] telling the caller
//!    whether to retry or abort.
//!
//! 3. **Log** — because the error carries `anyhow_tracing` fields, any
//!    `tracing::warn!(error = %err, …)` automatically includes the
//!    structured context in JSON log output.

use std::fmt;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex;
use tracing::{info, warn};

use super::SharedVault;
use super::protocol;
use super::providers;
use super::transport::TransportWriter;
use super::types::ProviderRequest;
use crate::providers as crate_providers;

// ── Error enum ──────────────────────────────────────────────────────────────

/// Stable string tag for each error kind, used as the `gateway_error_kind`
/// field in structured logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Auth,
    Provider,
    TokenLimit,
    ToolLoopExhausted,
    ContextCompaction,
    Cancelled,
    Vault,
    DeviceFlow,
    Config,
    TokenRefresh,
    UnexpectedFinish,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Provider => "provider",
            Self::TokenLimit => "token_limit",
            Self::ToolLoopExhausted => "tool_loop_exhausted",
            Self::ContextCompaction => "context_compaction",
            Self::Cancelled => "cancelled",
            Self::Vault => "vault",
            Self::DeviceFlow => "device_flow",
            Self::Config => "config",
            Self::TokenRefresh => "token_refresh",
            Self::UnexpectedFinish => "unexpected_finish",
        }
    }
}

/// Categories of errors the gateway can encounter and report to the client.
///
/// Implements [`std::error::Error`] so it can be wrapped in an
/// [`anyhow_tracing::Error`] with `.with_field()` for structured logging.
#[derive(Debug, Clone)]
pub enum GatewayError {
    /// Authentication failure — 401/403, token refresh, expired key, etc.
    Auth { provider: String, message: String },

    /// Model API returned an error that is not auth-related.
    Provider { message: String },

    /// The response was truncated because the model hit its token limit.
    TokenLimit,

    /// The agentic tool loop hit the safety ceiling.
    ToolLoopExhausted { rounds: usize },

    /// Context compaction failed (non-fatal — the call can proceed).
    ContextCompaction { message: String },

    /// The user cancelled the current run.
    Cancelled,

    /// Vault operation failed (unlock, store, get, delete, etc.).
    Vault { message: String },

    /// Device flow initiation or polling failed.
    DeviceFlow { provider: String, message: String },

    /// Provider/model not found or misconfigured.
    Config { message: String },

    /// Token refresh (bearer / Copilot session) failed.
    TokenRefresh { message: String },

    /// The model finished with an unexpected reason but no tool calls.
    /// This is informational (not an error) — logged via `send_info`.
    UnexpectedFinish { reason: String },
}

impl fmt::Display for GatewayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth { provider, message } => {
                write!(f, "Authentication failed for {}: {}", provider, message)
            }
            Self::Provider { message } => write!(f, "{}", message),
            Self::TokenLimit => write!(f, "Response truncated due to token limit."),
            Self::ToolLoopExhausted { rounds } => write!(
                f,
                "Safety limit reached ({} tool rounds) — stopping to prevent infinite loop.",
                rounds
            ),
            Self::ContextCompaction { message } => {
                write!(f, "Context compaction failed: {}", message)
            }
            Self::Cancelled => write!(f, "Run cancelled by user."),
            Self::Vault { message } => write!(f, "{}", message),
            Self::DeviceFlow { provider, message } => {
                write!(f, "Device flow for {} failed: {}", provider, message)
            }
            Self::Config { message } => write!(f, "{}", message),
            Self::TokenRefresh { message } => {
                write!(f, "Token refresh failed: {}", message)
            }
            Self::UnexpectedFinish { reason } => {
                write!(
                    f,
                    "Model finished with reason '{}' but no tool calls.",
                    reason
                )
            }
        }
    }
}

impl std::error::Error for GatewayError {}

impl GatewayError {
    /// The stable error-kind tag for this variant.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Auth { .. } => ErrorKind::Auth,
            Self::Provider { .. } => ErrorKind::Provider,
            Self::TokenLimit => ErrorKind::TokenLimit,
            Self::ToolLoopExhausted { .. } => ErrorKind::ToolLoopExhausted,
            Self::ContextCompaction { .. } => ErrorKind::ContextCompaction,
            Self::Cancelled => ErrorKind::Cancelled,
            Self::Vault { .. } => ErrorKind::Vault,
            Self::DeviceFlow { .. } => ErrorKind::DeviceFlow,
            Self::Config { .. } => ErrorKind::Config,
            Self::TokenRefresh { .. } => ErrorKind::TokenRefresh,
            Self::UnexpectedFinish { .. } => ErrorKind::UnexpectedFinish,
        }
    }

    /// Whether this error is non-fatal (the dispatch loop should continue).
    pub fn is_non_fatal(&self) -> bool {
        matches!(self, Self::ContextCompaction { .. })
    }

    /// Wrap this error in an [`anyhow_tracing::Error`] with structured
    /// fields attached (`gateway_error_kind`, and `provider` when
    /// available).
    pub fn into_traced(self) -> anyhow_tracing::Error {
        let kind = self.kind();
        let provider = match &self {
            Self::Auth { provider, .. } | Self::DeviceFlow { provider, .. } => {
                Some(provider.clone())
            }
            _ => None,
        };
        let mut err = anyhow_tracing::Error::from(anyhow::Error::from(self))
            .with_field("gateway_error_kind", kind.as_str());
        if let Some(p) = provider {
            err = err.with_field("provider", p);
        }
        err
    }
}

// ── Classification ──────────────────────────────────────────────────────────

/// Check whether an error message matches common HTTP auth-error patterns.
fn is_auth_error(error_msg: &str) -> bool {
    let patterns = [
        "returned 401",
        "returned 403",
        "HTTP 401",
        "HTTP 403",
        "401 Unauthorized",
        "403 Forbidden",
        "authentication_error",
        "invalid_api_key",
        "invalid x-api-key",
    ];
    let lower = error_msg.to_lowercase();
    patterns.iter().any(|p| lower.contains(&p.to_lowercase()))
}

/// Inspect a raw model-call error and return a traced error wrapping the
/// appropriate [`GatewayError`] variant.
///
/// The returned [`anyhow_tracing::Error`] carries structured fields
/// (`gateway_error_kind`, `provider`) suitable for JSON log output.
pub fn classify_model_error(err: anyhow::Error, provider: &str) -> anyhow_tracing::Error {
    let msg = err.to_string();
    let gw = if is_auth_error(&msg) {
        let display = crate_providers::display_name_for_provider(provider);
        GatewayError::Auth {
            provider: provider.to_string(),
            message: format!("Authentication failed for {}", display),
        }
    } else {
        GatewayError::Provider { message: msg }
    };
    gw.into_traced()
}

/// Recover the [`GatewayError`] from an `anyhow_tracing::Error` produced
/// by [`classify_model_error`] or [`GatewayError::into_traced`].
fn downcast_gateway_error(err: &anyhow_tracing::Error) -> Option<&GatewayError> {
    // anyhow_tracing wraps an anyhow::Error; downcast through it.
    err.downcast_ref::<GatewayError>()
}

// ── Centralised handling ────────────────────────────────────────────────────

/// Handle a gateway error by sending the appropriate frames to the client
/// and performing any recovery (device flow, credential prompt, etc.).
///
/// Returns `ControlFlow::Continue(())` when the caller should retry the
/// model call (e.g. after obtaining fresh credentials) and
/// `ControlFlow::Break(())` when the conversation should stop.
///
/// The incoming `anyhow_tracing::Error` carries structured fields that are
/// logged automatically via `tracing::warn!`.
pub async fn handle(
    err: anyhow_tracing::Error,
    writer: &mut dyn TransportWriter,
    resolved: &mut ProviderRequest,
    original_api_key: &mut Option<String>,
    vault: &SharedVault,
    credential_rx: &std::sync::Arc<
        Mutex<tokio::sync::mpsc::Receiver<(String, bool, Option<String>)>>,
    >,
    tool_cancel: &Arc<AtomicBool>,
) -> anyhow::Result<ControlFlow<(), ()>> {
    // Try to recover the typed GatewayError.
    let gw_err = match downcast_gateway_error(&err) {
        Some(e) => e.clone(),
        None => {
            // Fallback: unknown error — report as generic provider error.
            warn!(error = %err, "Unclassified gateway error");
            protocol::server::send_error(writer, &err.to_string()).await?;
            providers::send_response_done(writer).await?;
            return Ok(ControlFlow::Break(()));
        }
    };

    // Log the classified error with structured fields.
    warn!(
        gateway_error_kind = %gw_err.kind(),
        error = %err,
        "Gateway error"
    );

    match gw_err {
        // ── Auth errors ─────────────────────────────────────────────
        GatewayError::Auth {
            ref provider,
            ref message,
        } => {
            let provider_def = crate_providers::provider_by_id(provider);
            let secret_name =
                crate_providers::secret_key_for_provider(provider).unwrap_or("API_KEY");
            let display = crate_providers::display_name_for_provider(provider);
            let auth_method = provider_def
                .map(|p| p.auth_method)
                .unwrap_or(crate_providers::AuthMethod::ApiKey);

            match auth_method {
                crate_providers::AuthMethod::DeviceFlow => {
                    handle_device_flow(
                        writer,
                        resolved,
                        original_api_key,
                        vault,
                        provider_def,
                        secret_name,
                        display,
                        tool_cancel,
                        Some(message.as_str()),
                    )
                    .await
                }
                crate_providers::AuthMethod::ApiKey
                | crate_providers::AuthMethod::None
                | crate_providers::AuthMethod::OptionalApiKey => {
                    handle_credential_prompt(
                        writer,
                        resolved,
                        original_api_key,
                        vault,
                        credential_rx,
                        provider,
                        secret_name,
                        display,
                        provider_def,
                    )
                    .await
                }
            }
        }

        // ── Non-fatal: context compaction ────────────────────────────
        GatewayError::ContextCompaction { ref message } => {
            let _ = protocol::server::send_info(
                writer,
                &format!("Context compaction failed: {}", message),
            )
            .await;
            Ok(ControlFlow::Continue(()))
        }

        // ── Cancelled ───────────────────────────────────────────────
        GatewayError::Cancelled => {
            protocol::server::send_info(writer, "Run cancelled by user.").await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Token refresh ───────────────────────────────────────────
        GatewayError::TokenRefresh { ref message } => {
            // For DeviceFlow providers, a failed token refresh means the
            // stored OAuth token is stale — re-run the device flow instead
            // of just showing an error.
            let provider_def = crate_providers::provider_by_id(&resolved.provider);
            let auth_method = provider_def
                .map(|p| p.auth_method)
                .unwrap_or(crate_providers::AuthMethod::ApiKey);

            if auth_method == crate_providers::AuthMethod::DeviceFlow {
                let provider_id = resolved.provider.clone();
                let secret_name =
                    crate_providers::secret_key_for_provider(&provider_id).unwrap_or("API_KEY");
                let display = crate_providers::display_name_for_provider(&provider_id);
                handle_device_flow(
                    writer,
                    resolved,
                    original_api_key,
                    vault,
                    provider_def,
                    secret_name,
                    display,
                    tool_cancel,
                    Some(message.as_str()),
                )
                .await
            } else {
                protocol::server::send_error(writer, &format!("Token refresh failed: {}", message))
                    .await?;
                providers::send_response_done(writer).await?;
                Ok(ControlFlow::Break(()))
            }
        }

        // ── Token limit ─────────────────────────────────────────────
        GatewayError::TokenLimit => {
            protocol::server::send_info(writer, "Response truncated due to token limit.").await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Tool loop exhausted ─────────────────────────────────────
        GatewayError::ToolLoopExhausted { rounds } => {
            protocol::server::send_error(
                writer,
                &format!(
                    "Safety limit reached ({} tool rounds) — stopping to prevent infinite loop.",
                    rounds
                ),
            )
            .await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Device flow failure (standalone, not during auth retry) ──
        GatewayError::DeviceFlow {
            ref provider,
            ref message,
        } => {
            protocol::server::send_error(
                writer,
                &format!("Device flow for {} failed: {}", provider, message),
            )
            .await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Config error ────────────────────────────────────────────
        GatewayError::Config { ref message } => {
            protocol::server::send_error(writer, message).await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Vault error ─────────────────────────────────────────────
        GatewayError::Vault { ref message } => {
            protocol::server::send_error(writer, message).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Unexpected finish reason (info, not error) ──────────────
        GatewayError::UnexpectedFinish { ref reason } => {
            protocol::server::send_info(
                writer,
                &format!("Model finished with reason '{}' but no tool calls.", reason),
            )
            .await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }

        // ── Generic provider error ──────────────────────────────────
        GatewayError::Provider { ref message } => {
            protocol::server::send_error(writer, message).await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }
    }
}

// ── Device flow sub-handler ─────────────────────────────────────────────────

async fn handle_device_flow(
    writer: &mut dyn TransportWriter,
    resolved: &mut ProviderRequest,
    original_api_key: &mut Option<String>,
    vault: &SharedVault,
    provider_def: Option<&'static crate_providers::ProviderDef>,
    secret_name: &str,
    display: &str,
    tool_cancel: &Arc<AtomicBool>,
    trigger_message: Option<&str>,
) -> anyhow::Result<ControlFlow<(), ()>> {
    let df_config = match provider_def.and_then(|p| p.device_flow) {
        Some(cfg) => cfg,
        None => {
            protocol::server::send_error(
                writer,
                &format!(
                    "Authentication failed for {} but no device flow config found.",
                    display
                ),
            )
            .await?;
            providers::send_response_done(writer).await?;
            return Ok(ControlFlow::Break(()));
        }
    };

    // Send the provider's error message so the client can display it.
    let info_msg = match trigger_message {
        Some(msg) => format!("{}: {} \u{2014} starting device flow\u{2026}", display, msg),
        None => format!(
            "Authentication failed for {}. Starting device flow\u{2026}",
            display
        ),
    };
    protocol::server::send_info(writer, &info_msg).await?;

    let auth_resp = match crate_providers::start_device_flow(df_config).await {
        Ok(r) => r,
        Err(e) => {
            protocol::server::send_error(
                writer,
                &format!("Failed to start device flow for {}: {}", display, e),
            )
            .await?;
            providers::send_response_done(writer).await?;
            return Ok(ControlFlow::Break(()));
        }
    };

    protocol::server::send_device_flow_start(
        writer,
        &auth_resp.verification_uri,
        &auth_resp.user_code,
        trigger_message,
    )
    .await?;

    // Poll for the token.
    let interval = std::time::Duration::from_secs(auth_resp.interval.max(5));
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(auth_resp.expires_in);
    let mut token_result = None;

    let mut poll_count: u32 = 0;
    loop {
        tokio::time::sleep(interval).await;
        if tool_cancel.load(Ordering::Relaxed) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = protocol::server::send_info(
                writer,
                &format!(
                    "{}: device flow timed out after {} polls",
                    display, poll_count
                ),
            )
            .await;
            break;
        }
        poll_count += 1;
        match crate_providers::poll_device_token(df_config, &auth_resp.device_code).await {
            Ok(Some(token)) => {
                info!(polls = poll_count, "Device flow poll succeeded");
                token_result = Some(token);
                break;
            }
            Ok(None) => {
                if poll_count == 1 {
                    info!("Device flow polling active — waiting for user to authorize");
                }
            }
            Err(e) => {
                let _ = protocol::server::send_info(
                    writer,
                    &format!("{}: poll error — {}", display, e),
                )
                .await;
                warn!(error = %e, "Device flow poll failed");
                break;
            }
        }
    }

    protocol::server::send_device_flow_complete(writer).await?;

    if let Some(token) = token_result {
        {
            let mut v = vault.lock().await;
            if let Err(e) = v.store_secret(secret_name, &token) {
                warn!(error = %e, "Failed to store device flow token in vault");
            }
        }
        resolved.api_key = Some(token.clone());
        *original_api_key = Some(token);
        protocol::server::send_info(
            writer,
            &format!("{} authenticated via device flow — retrying…", display),
        )
        .await?;
        Ok(ControlFlow::Continue(()))
    } else {
        protocol::server::send_error(
            writer,
            &format!("Device flow for {} timed out or failed.", display),
        )
        .await?;
        providers::send_response_done(writer).await?;
        Ok(ControlFlow::Break(()))
    }
}

// ── Credential prompt sub-handler ───────────────────────────────────────────

async fn handle_credential_prompt(
    writer: &mut dyn TransportWriter,
    resolved: &mut ProviderRequest,
    original_api_key: &mut Option<String>,
    vault: &SharedVault,
    credential_rx: &std::sync::Arc<
        Mutex<tokio::sync::mpsc::Receiver<(String, bool, Option<String>)>>,
    >,
    provider: &str,
    secret_name: &str,
    display: &str,
    provider_def: Option<&'static crate_providers::ProviderDef>,
) -> anyhow::Result<ControlFlow<(), ()>> {
    let help_text = provider_def
        .and_then(|p| p.help_text)
        .unwrap_or("Enter the API key for this provider");

    let request_id = format!("cred_{}", uuid::Uuid::new_v4());
    let message = format!("Authentication failed for {}. {}.", display, help_text);

    protocol::server::send_credential_request(writer, &request_id, provider, secret_name, &message)
        .await?;

    // Wait for the user's response (with 5 minute timeout).
    let cred_result = {
        let mut rx = credential_rx.lock().await;
        tokio::time::timeout(std::time::Duration::from_secs(300), rx.recv()).await
    };

    match cred_result {
        Ok(Some((id, dismissed, value))) if id == request_id && !dismissed => {
            if let Some(key) = value {
                {
                    let mut v = vault.lock().await;
                    if let Err(e) = v.store_secret(secret_name, &key) {
                        warn!(error = %e, "Failed to store credential in vault");
                    }
                }
                resolved.api_key = Some(key.clone());
                *original_api_key = Some(key);
                protocol::server::send_info(
                    writer,
                    &format!("Credential received for {} — retrying…", display),
                )
                .await?;
                Ok(ControlFlow::Continue(()))
            } else {
                protocol::server::send_error(writer, "No credential value provided.").await?;
                providers::send_response_done(writer).await?;
                Ok(ControlFlow::Break(()))
            }
        }
        _ => {
            protocol::server::send_error(
                writer,
                &format!(
                    "Authentication failed for {} and no credential was provided.",
                    display
                ),
            )
            .await?;
            providers::send_response_done(writer).await?;
            Ok(ControlFlow::Break(()))
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_auth_error_positive_cases() {
        assert!(is_auth_error("Provider returned 401 Unauthorized"));
        assert!(is_auth_error("Anthropic returned 403 Forbidden"));
        assert!(is_auth_error("HTTP 401 from api.openai.com"));
        assert!(is_auth_error("HTTP 403 access denied"));
        assert!(is_auth_error("401 Unauthorized"));
        assert!(is_auth_error("403 Forbidden"));
        assert!(is_auth_error("authentication_error: invalid credentials"));
        assert!(is_auth_error("invalid_api_key"));
        assert!(is_auth_error("invalid x-api-key header"));
        assert!(is_auth_error("AUTHENTICATION_ERROR"));
    }

    #[test]
    fn test_is_auth_error_negative_cases() {
        assert!(!is_auth_error("Connection timeout after 30s"));
        assert!(!is_auth_error("returned 500 Internal Server Error"));
        assert!(!is_auth_error("Rate limited — try again later"));
        assert!(!is_auth_error("Model not found"));
        assert!(!is_auth_error(""));
        assert!(!is_auth_error("Some random error message"));
    }

    #[test]
    fn test_classify_model_error_auth() {
        let err = anyhow::anyhow!("Provider returned 401 Unauthorized");
        let traced = classify_model_error(err, "anthropic");
        let gw = downcast_gateway_error(&traced);
        assert!(gw.is_some());
        assert!(matches!(gw.unwrap(), GatewayError::Auth { .. }));
    }

    #[test]
    fn test_classify_model_error_provider() {
        let err = anyhow::anyhow!("Connection timeout after 30s");
        let traced = classify_model_error(err, "openai");
        let gw = downcast_gateway_error(&traced);
        assert!(gw.is_some());
        assert!(matches!(gw.unwrap(), GatewayError::Provider { .. }));
    }

    #[test]
    fn test_error_kind_as_str() {
        assert_eq!(ErrorKind::Auth.as_str(), "auth");
        assert_eq!(ErrorKind::Provider.as_str(), "provider");
        assert_eq!(ErrorKind::TokenLimit.as_str(), "token_limit");
        assert_eq!(ErrorKind::ToolLoopExhausted.as_str(), "tool_loop_exhausted");
        assert_eq!(ErrorKind::ContextCompaction.as_str(), "context_compaction");
        assert_eq!(ErrorKind::Cancelled.as_str(), "cancelled");
        assert_eq!(ErrorKind::Vault.as_str(), "vault");
        assert_eq!(ErrorKind::DeviceFlow.as_str(), "device_flow");
        assert_eq!(ErrorKind::Config.as_str(), "config");
        assert_eq!(ErrorKind::TokenRefresh.as_str(), "token_refresh");
        assert_eq!(ErrorKind::UnexpectedFinish.as_str(), "unexpected_finish");
    }

    #[test]
    fn test_into_traced_carries_fields() {
        let gw = GatewayError::Auth {
            provider: "anthropic".into(),
            message: "invalid key".into(),
        };
        let traced = gw.into_traced();
        // The error should carry the gateway_error_kind field.
        let fields: Vec<_> = traced.fields().iter().map(|(k, _)| *k).collect();
        assert!(
            fields.contains(&"gateway_error_kind"),
            "missing gateway_error_kind in {:?}",
            fields
        );
        assert!(
            fields.contains(&"provider"),
            "missing provider in {:?}",
            fields
        );
    }

    #[test]
    fn test_display_variants() {
        let auth = GatewayError::Auth {
            provider: "anthropic".into(),
            message: "invalid key".into(),
        };
        assert!(auth.to_string().contains("anthropic"));

        let token_limit = GatewayError::TokenLimit;
        assert!(token_limit.to_string().contains("truncated"));

        let tool_loop = GatewayError::ToolLoopExhausted { rounds: 500 };
        assert!(tool_loop.to_string().contains("500"));

        let cancelled = GatewayError::Cancelled;
        assert!(cancelled.to_string().contains("cancelled"));

        let ctx = GatewayError::ContextCompaction {
            message: "OOM".into(),
        };
        assert!(ctx.to_string().contains("OOM"));
        assert!(ctx.is_non_fatal());

        let provider = GatewayError::Provider {
            message: "timeout".into(),
        };
        assert!(!provider.is_non_fatal());

        let unexpected = GatewayError::UnexpectedFinish {
            reason: "content_filter".into(),
        };
        assert!(unexpected.to_string().contains("content_filter"));
        assert!(!unexpected.is_non_fatal());
    }

    #[test]
    fn test_downcast_roundtrip() {
        let gw = GatewayError::TokenLimit;
        let traced = gw.into_traced();
        let recovered = downcast_gateway_error(&traced).unwrap();
        assert!(matches!(recovered, GatewayError::TokenLimit));
    }
}
