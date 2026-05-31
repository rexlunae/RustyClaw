//! OAuth device flow + GitHub Copilot session-token exchange.

#![allow(unused_imports)]
use super::models::GITHUB_USER_AGENT;
use super::*;

// ── OAuth Device Flow ───────────────────────────────────────────────────────

use serde::Deserialize;

/// Response from the device authorization endpoint.
#[derive(Debug, Deserialize)]
pub struct DeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Response from the token endpoint.
///
/// Uses a flat struct with all-optional fields for robust deserialization.
/// GitHub's token endpoint returns either a success object (with
/// `access_token`) or an error object (with `error`), but
/// `#[serde(untagged)]` enums are fragile and silently fail when the
/// response shape differs even slightly from what's expected.
#[derive(Debug, Deserialize)]
pub struct RawTokenResponse {
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    pub token_type: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Interpreted token response for pattern-matching callers.
#[derive(Debug)]
pub enum TokenResponse {
    Success {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        token_type: String,
    },
    Pending {
        error: String,
        error_description: Option<String>,
    },
}

impl From<RawTokenResponse> for TokenResponse {
    fn from(raw: RawTokenResponse) -> Self {
        if let Some(access_token) = raw.access_token {
            TokenResponse::Success {
                access_token,
                token_type: raw.token_type.unwrap_or_else(|| "bearer".to_string()),
                refresh_token: raw.refresh_token,
                expires_in: raw.expires_in,
            }
        } else {
            TokenResponse::Pending {
                error: raw.error.unwrap_or_else(|| "unknown".to_string()),
                error_description: raw.error_description,
            }
        }
    }
}

/// Initiate OAuth device flow and return device code and verification URL.
pub async fn start_device_flow(config: &DeviceFlowConfig) -> Result<DeviceAuthResponse> {
    let url = config.device_auth_url.to_string();
    let mut details = RequestDetails::new("device_flow.start", "POST", url.clone())
        .with_request_headers([
            ("Accept", "application/json"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ]);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .with_context(|| format!("failed to build HTTP client for POST {}", url))
        .map_err(|e| details.clone().emit_warning(e))?;

    let mut params: Vec<(&str, &str)> = vec![("client_id", config.client_id)];
    if let Some(scope) = config.scope {
        params.push(("scope", scope));
    }

    let resp = match client
        .post(config.device_auth_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let err =
                wrap_err(e).context(format!("POST {} failed to send device-code request", url));
            return Err(details.emit_warning(err));
        }
    };

    let status = resp.status();
    let response_headers = resp.headers().clone();
    details = details.with_response(status.as_u16(), &response_headers);

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        details = details.with_body(&body);
        let err = anyhow!(
            "POST {} returned HTTP {} — body: {}",
            url,
            status,
            truncate_for_error(&body)
        );
        return Err(details.emit_warning(err));
    }

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            let err = wrap_err(e).context(format!("POST {}: failed to read response body", url));
            return Err(details.emit_warning(err));
        }
    };
    details = details.with_body(&body_text);

    match serde_json::from_str::<DeviceAuthResponse>(&body_text) {
        Ok(auth_response) => Ok(auth_response),
        Err(e) => {
            let err = wrap_err(e).context(format!(
                "POST {}: failed to parse device-authorization response",
                url
            ));
            Err(details.emit_warning(err))
        }
    }
}

/// Poll the token endpoint to complete device flow authentication.
///
/// Returns Ok(Some(token)) when authentication succeeds,
/// Ok(None) when still pending, and Err when authentication fails.
pub async fn poll_device_token(
    config: &DeviceFlowConfig,
    device_code: &str,
) -> Result<Option<String>> {
    let url = config.token_url.to_string();
    let mut details = RequestDetails::new("device_flow.poll", "POST", url.clone())
        .with_request_headers([
            ("Accept", "application/json"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ]);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .with_context(|| format!("failed to build HTTP client for POST {}", url))
        .map_err(|e| details.clone().emit_warning(e))?;

    let params = [
        ("client_id", config.client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];

    let resp = match client
        .post(config.token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let err =
                wrap_err(e).context(format!("POST {} failed to send token-poll request", url));
            return Err(details.emit_warning(err));
        }
    };

    let status = resp.status();
    let response_headers = resp.headers().clone();
    details = details.with_response(status.as_u16(), &response_headers);

    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            let err = wrap_err(e).context(format!("POST {}: failed to read response body", url));
            return Err(details.emit_warning(err));
        }
    };
    details = details.clone().with_body(&body);

    // Log poll response at info level for debugging, but redact if it
    // contains an access token (secret).
    let safe_preview = if body.contains("access_token") {
        "<redacted: contains access_token>".to_string()
    } else {
        let mut end = body.len().min(120);
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        body[..end].to_string()
    };
    tracing::info!(
        status = %status,
        body_len = body.len(),
        body_preview = %safe_preview,
        "Device flow token poll response"
    );

    // Parse as a flat struct first, then interpret.  This avoids the
    // fragility of serde(untagged) which silently fails when the
    // response shape is slightly unexpected.
    let raw: RawTokenResponse = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(_json_err) => {
            // Fallback: try URL-encoded form parsing (GitHub may return
            // "access_token=xxx&token_type=bearer" instead of JSON).
            match parse_form_encoded_token_response(&body) {
                Some(r) => r,
                None => {
                    let err = anyhow!(
                        "POST {}: failed to parse token response (tried JSON and form-encoded): {}",
                        url,
                        &body[..body.len().min(200)]
                    );
                    return Err(details.emit_warning(err));
                }
            }
        }
    };
    let token_response: TokenResponse = raw.into();

    match token_response {
        TokenResponse::Success { access_token, .. } => {
            tracing::info!("Device flow authentication succeeded");
            Ok(Some(access_token))
        }
        TokenResponse::Pending {
            error,
            error_description,
        } => {
            if error == "authorization_pending" || error == "slow_down" {
                tracing::trace!("Device flow still pending: {}", error);
                Ok(None) // Still waiting for user authorization
            } else {
                let err = match error_description {
                    Some(desc) => anyhow!("Authentication failed: {} ({})", error, desc),
                    None => anyhow!("Authentication failed: {}", error),
                };
                Err(details.emit_warning(err))
            }
        }
    }
}

/// Parse a URL-encoded token response (fallback when JSON isn't returned).
///
/// GitHub's token endpoint historically defaults to `application/x-www-form-urlencoded`.
/// Format: `access_token=xxx&token_type=bearer&scope=read:user`
/// Or: `error=authorization_pending&error_description=...`
pub(crate) fn parse_form_encoded_token_response(body: &str) -> Option<RawTokenResponse> {
    let trimmed = body.trim();
    if trimmed.starts_with('{') || !trimmed.contains('=') {
        return None;
    }
    let params: std::collections::HashMap<String, String> = trimmed
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| {
            // In application/x-www-form-urlencoded, '+' means space.
            // Replace before percent-decoding since urlencoding::decode
            // only handles %XX sequences (RFC 3986), not form '+' → space.
            let plus_decoded = v.replace('+', " ");
            let decoded = urlencoding::decode(&plus_decoded)
                .map(|d| d.into_owned())
                .unwrap_or_else(|_| plus_decoded);
            (k.to_string(), decoded)
        })
        .collect();

    Some(RawTokenResponse {
        access_token: params.get("access_token").cloned(),
        refresh_token: params.get("refresh_token").cloned(),
        expires_in: params.get("expires_in").and_then(|s| s.parse().ok()),
        token_type: params.get("token_type").cloned(),
        error: params.get("error").cloned(),
        error_description: params.get("error_description").cloned(),
    })
}

// ── Copilot session token exchange ──────────────────────────────────────────

/// Response from the Copilot internal token endpoint.
///
/// The `token` field is a short-lived session token (valid ~30 min).
/// `expires_at` is a Unix timestamp indicating when it expires.
///
/// `endpoints.api` carries the **plan-specific** API base URL
/// (e.g. `https://api.individual.githubcopilot.com`,
/// `https://api.business.githubcopilot.com`).  When present it must
/// be used in preference to the generic `https://api.githubcopilot.com`
/// host: the generic host's `/models` listing returns an empty `data`
/// array for users whose Copilot plan is served from a dedicated host,
/// which would otherwise surface as a misleading "empty model list".
#[derive(Debug, Deserialize)]
pub struct CopilotSessionResponse {
    pub token: String,
    pub expires_at: i64,
    #[serde(default)]
    pub endpoints: Option<CopilotSessionEndpoints>,
}

/// Plan-specific API endpoints returned by the Copilot token exchange.
#[derive(Debug, Deserialize)]
pub struct CopilotSessionEndpoints {
    /// Base URL for the Copilot API (e.g.
    /// `https://api.individual.githubcopilot.com`).  Trailing slashes
    /// should be stripped before composing request URLs.
    #[serde(default)]
    pub api: Option<String>,
}

/// Exchange a GitHub OAuth token for a short-lived Copilot API session token.
///
/// The Copilot chat API (`api.githubcopilot.com`) requires a session token
/// obtained by presenting the long-lived OAuth device-flow token to
/// GitHub's internal token endpoint.  Session tokens expire after ~30
/// minutes; the caller should cache and refresh before `expires_at`.
pub async fn exchange_copilot_session(
    http: &reqwest::Client,
    oauth_token: &str,
) -> Result<CopilotSessionResponse> {
    let url = "https://api.github.com/copilot_internal/v2/token";
    let auth_value = format!("token {}", oauth_token);
    let headers: [(&str, &str); 2] = [
        ("Authorization", auth_value.as_str()),
        ("User-Agent", GITHUB_USER_AGENT),
    ];
    let context = || format_request_context("GET", url, &headers, Some(oauth_token));

    let mut details = RequestDetails::new("copilot.session_exchange", "GET", url.to_string())
        .with_provider("github-copilot")
        .with_request_headers(headers.iter().map(|(k, v)| (*k, *v)))
        .with_bearer(Some(oauth_token));

    let resp = match http
        .get(url)
        .header("Authorization", &auth_value)
        .header("User-Agent", GITHUB_USER_AGENT)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let err = wrap_err(e).context(format!("{}: failed to send request", context()));
            return Err(details.emit_warning(err));
        }
    };

    let status = resp.status();
    let response_headers = resp.headers().clone();
    details = details.with_response(status.as_u16(), &response_headers);

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        details = details.with_body(&body);
        let err = anyhow!(
            "{} returned HTTP {} — body: {}",
            context(),
            status,
            truncate_for_error(&body)
        );
        return Err(details.emit_warning(err));
    }

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            let err = wrap_err(e).context(format!("{}: failed to read response body", context()));
            return Err(details.emit_warning(err));
        }
    };
    details = details.clone().with_body(&body_text);

    match serde_json::from_str::<CopilotSessionResponse>(&body_text) {
        Ok(session) => {
            // Surface the discovered plan-specific endpoint so the caller's
            // logs and future error messages can show which host was used.
            if let Some(api) = session.endpoints.as_ref().and_then(|e| e.api.as_deref()) {
                tracing::debug!(
                    target: "rustyclaw::providers",
                    step = "copilot.session_exchange",
                    endpoints_api = api,
                    "Copilot session token issued"
                );
            }
            Ok(session)
        }
        Err(e) => {
            let err =
                wrap_err(e).context(format!("{}: failed to parse session response", context()));
            Err(details.emit_warning(err))
        }
    }
}

/// Whether the given provider requires Copilot session-token exchange.
pub fn needs_copilot_session(provider_id: &str) -> bool {
    matches!(provider_id, "github-copilot" | "copilot-proxy")
}
