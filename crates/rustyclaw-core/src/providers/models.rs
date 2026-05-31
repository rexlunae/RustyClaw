//! Dynamic model-list fetching from provider APIs.

#![allow(unused_imports)]
use super::*;

pub async fn fetch_models(
    provider_id: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
) -> Result<Vec<String>> {
    // Delegate to the detailed version and strip down to IDs.
    fetch_models_detailed(provider_id, api_key, base_url_override)
        .await
        .map(|v| v.into_iter().map(|m| m.id).collect())
}

/// Fetch models with full metadata (pricing, context length, name).
///
/// Providers that don't expose rich metadata will still return [`ModelInfo`]
/// entries — just with `None` for the optional fields.
pub async fn fetch_models_detailed(
    provider_id: &str,
    api_key: Option<&str>,
    base_url_override: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    let def =
        provider_by_id(provider_id).ok_or_else(|| anyhow!("Unknown provider: {}", provider_id))?;

    let base = base_url_override.or(def.base_url).unwrap_or("");

    if base.is_empty() {
        bail!(
            "No base URL configured for {}. Set one in config.toml or use /provider.",
            def.display
        );
    }

    // Anthropic has no public models endpoint — return the static list.
    if provider_id == "anthropic" {
        let static_models: Vec<ModelInfo> = def
            .models
            .iter()
            .map(|id| ModelInfo {
                id: id.to_string(),
                name: None,
                context_length: None,
                pricing_prompt: None,
                pricing_completion: None,
            })
            .collect();
        return Ok(static_models);
    }

    let result: Result<Vec<ModelInfo>> = match provider_id {
        // Google Gemini uses a different response shape
        "google" => fetch_google_models_detailed(base, api_key).await,
        // GitHub Copilot exposes a Copilot-specific model list API.
        "github-copilot" => fetch_github_copilot_models_detailed(base, api_key).await,
        // Local providers — auth optional, OpenAI-compatible /v1/models
        "ollama" | "lmstudio" | "exo" => {
            fetch_openai_compatible_models_detailed(base, api_key).await
        }
        // Everything else is OpenAI-compatible
        _ => fetch_openai_compatible_models_detailed(base, api_key).await,
    };

    match result {
        Ok(models) if models.is_empty() => Err(anyhow!(
            "The {} API returned an empty model list.",
            def.display
        )),
        Ok(models) => Ok(models),
        Err(e) => Err(e.context(format!("Failed to fetch models from {}", def.display))),
    }
}

/// Non-chat model ID patterns.  Any model whose ID contains one of these
/// substrings (case-insensitive) is filtered out of the selector.
const NON_CHAT_PATTERNS: &[&str] = &[
    "embed",
    "tts",
    "whisper",
    "dall-e",
    "davinci",
    "babbage",
    "moderation",
    "search",
    "similarity",
    "code-search",
    "text-search",
    "audio",
    "realtime",
    "transcri",
    "computer-use",
    "canary", // internal/experimental
];

pub const COPILOT_API_ACCEPT: &str = "application/vnd.github+json";
pub const COPILOT_API_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
pub const COPILOT_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
pub(crate) const GITHUB_USER_AGENT: &str = "RustyClaw";

/// Curated catalog of GitHub Copilot chat models.
///
/// The Copilot `/models` endpoint only returns models the user has
/// explicitly enabled in their GitHub Copilot model-picker settings.
/// To surface every known model the way other clients (e.g. OpenClaw
/// via `@mariozechner/pi-ai`) do, we merge this static catalog with
/// the live API response: live entries take precedence (they carry
/// authoritative metadata), and any catalog entry whose id is missing
/// from the live list is appended so the user can at least *see* it
/// in the picker.  Selecting a model the account doesn't actually
/// have access to will fail at chat-request time with the upstream
/// error message.
///
/// Entries: `(id, display_name, context_window_tokens)`.  Pricing is
/// omitted because Copilot access is subscription-based — showing
/// "$0/$0 per 1M tok" would be misleading.
const COPILOT_STATIC_CATALOG: &[(&str, &str, u64)] = &[
    ("claude-haiku-4.5", "Claude Haiku 4.5", 128_000),
    ("claude-opus-4.5", "Claude Opus 4.5", 128_000),
    ("claude-opus-4.6", "Claude Opus 4.6", 128_000),
    ("claude-sonnet-4", "Claude Sonnet 4", 128_000),
    ("claude-sonnet-4.5", "Claude Sonnet 4.5", 128_000),
    ("claude-sonnet-4.6", "Claude Sonnet 4.6", 128_000),
    ("gemini-2.5-pro", "Gemini 2.5 Pro", 128_000),
    ("gemini-3-flash-preview", "Gemini 3 Flash", 128_000),
    ("gemini-3-pro-preview", "Gemini 3 Pro Preview", 128_000),
    ("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview", 128_000),
    ("gpt-4.1", "GPT-4.1", 64_000),
    ("gpt-4o", "GPT-4o", 64_000),
    ("gpt-5", "GPT-5", 128_000),
    ("gpt-5-mini", "GPT-5-mini", 128_000),
    ("gpt-5.1", "GPT-5.1", 128_000),
    ("gpt-5.1-codex", "GPT-5.1-Codex", 128_000),
    ("gpt-5.1-codex-max", "GPT-5.1-Codex-max", 128_000),
    ("gpt-5.1-codex-mini", "GPT-5.1-Codex-mini", 128_000),
    ("gpt-5.2", "GPT-5.2", 128_000),
    ("gpt-5.2-codex", "GPT-5.2-Codex", 272_000),
    ("grok-code-fast-1", "Grok Code Fast 1", 128_000),
];

/// Build [`ModelInfo`] entries for the static Copilot catalog.
fn copilot_static_models() -> Vec<ModelInfo> {
    COPILOT_STATIC_CATALOG
        .iter()
        .map(|(id, name, ctx)| ModelInfo {
            id: (*id).to_string(),
            name: Some((*name).to_string()),
            context_length: Some(*ctx),
            pricing_prompt: None,
            pricing_completion: None,
        })
        .collect()
}

/// Merge a live model list with the static Copilot catalog.
///
/// Live entries are kept as-is (the API knows the user's current
/// access state best).  Any catalog id not present in `live` is
/// appended so newly-released models still appear in the picker.
fn merge_copilot_models(live: Vec<ModelInfo>) -> Vec<ModelInfo> {
    use std::collections::HashSet;
    let live_ids: HashSet<String> = live.iter().map(|m| m.id.clone()).collect();
    let mut merged = live;
    for entry in copilot_static_models() {
        if !live_ids.contains(&entry.id) {
            merged.push(entry);
        }
    }
    merged.sort_by(|a, b| a.id.cmp(&b.id));
    merged
}

/// Check whether a model entry looks like it supports chat completions.
///
/// 1. If the entry has `capabilities.chat` (GitHub Copilot style),
///    use that.
/// 2. Otherwise fall back to filtering out known non-chat ID patterns.
pub(crate) fn is_chat_model(entry: &serde_json::Value) -> bool {
    // GitHub Copilot and some providers expose capabilities metadata.
    if let Some(caps) = entry.get("capabilities") {
        if caps.get("chat").and_then(|v| v.as_bool()).unwrap_or(false) {
            return true;
        }
        if caps.get("type").and_then(|v| v.as_str()) == Some("chat") {
            return true;
        }
        return false;
    }

    // Some endpoints use object type "model" vs "embedding" etc.
    if let Some(obj) = entry.get("object").and_then(|v| v.as_str()) {
        if obj != "model" {
            return false;
        }
    }

    // Fall back to ID pattern matching.
    let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let lower = id.to_lowercase();
    !NON_CHAT_PATTERNS.iter().any(|pat| lower.contains(pat))
}

pub(crate) fn parse_models_response(body: &serde_json::Value) -> Vec<ModelInfo> {
    let mut models: Vec<ModelInfo> = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| is_chat_model(m))
                .filter_map(|m| {
                    let id = m.get("id").and_then(|v| v.as_str())?.to_string();
                    let name = m.get("name").and_then(|v| v.as_str()).map(String::from);
                    let context_length = m.get("context_length").and_then(|v| v.as_u64());
                    // OpenRouter-style pricing: { "prompt": "0.000015", "completion": "0.000075" }
                    let pricing_prompt =
                        m.get("pricing")
                            .and_then(|p| p.get("prompt"))
                            .and_then(|v| {
                                v.as_str()
                                    .and_then(|s| s.parse::<f64>().ok())
                                    .or_else(|| v.as_f64())
                            });
                    let pricing_completion = m
                        .get("pricing")
                        .and_then(|p| p.get("completion"))
                        .and_then(|v| {
                            v.as_str()
                                .and_then(|s| s.parse::<f64>().ok())
                                .or_else(|| v.as_f64())
                        });
                    Some(ModelInfo {
                        id,
                        name,
                        context_length,
                        pricing_prompt,
                        pricing_completion,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    models
}

/// Fetch from an OpenAI-compatible `/models` endpoint with full metadata.
///
/// Works for OpenAI, xAI, OpenRouter, Ollama, and custom providers.  Only
/// models that appear to support chat completions are returned (see
/// [`is_chat_model`]).
async fn fetch_openai_compatible_models_detailed(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let mut details = RequestDetails::new("openai.models", "GET", url.clone())
        .with_request_headers([("Accept", "application/json")])
        .with_bearer(api_key);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .with_context(|| format!("failed to build HTTP client for GET {}", url))
        .map_err(|e| details.clone().emit_warning(e))?;

    let mut req = client.get(&url);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let err = wrap_err(e).context(format!("GET {} failed to send", url));
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
            "GET {} returned HTTP {} — body: {}",
            url,
            status,
            truncate_for_error(&body)
        );
        return Err(details.emit_warning(err));
    }

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            let err = wrap_err(e).context(format!("GET {}: failed to read response body", url));
            return Err(details.emit_warning(err));
        }
    };
    details = details.with_body(&body_text);

    let body: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            let err = wrap_err(e).context(format!("GET {}: failed to parse JSON response", url));
            return Err(details.emit_warning(err));
        }
    };

    Ok(parse_models_response(&body))
}

/// Fetch from GitHub Copilot's model list API.
///
/// `api.githubcopilot.com/models` requires a short-lived **session
/// token** obtained from `api.github.com/copilot_internal/v2/token`.
/// Calling it with a long-lived OAuth token does not consistently
/// fail with `401`: it can return `200` with an empty `data` array,
/// which would otherwise surface as a misleading "empty model list"
/// warning.  We therefore always exchange the OAuth token for a
/// session token first (matching the behaviour of
/// [`CopilotSession::get_token`]), and only fall back to using the
/// supplied token directly if the exchange fails — that way callers
/// that pass a pre-exchanged session token still work.
///
/// The exchange response also carries a **plan-specific** API base
/// URL (e.g. `https://api.individual.githubcopilot.com`) in
/// `endpoints.api`.  The generic `api.githubcopilot.com/models` host
/// returns an empty `data` array for users routed to a dedicated
/// plan host, so we prefer the discovered base when available and
/// only fall back to the configured `base_url` when no endpoint is
/// provided.
async fn fetch_github_copilot_models_detailed(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("failed to build HTTP client")?;
    let fallback_url = format!("{}/models", base_url.trim_end_matches('/'));

    // No credentials yet — still return the curated catalog so the
    // model picker isn't empty before the user finishes signing in.
    let Some(key) = api_key else {
        return Ok(copilot_static_models());
    };

    let live_result: Result<Vec<ModelInfo>> = match exchange_copilot_session(&client, key).await {
        Ok(session) => {
            let endpoints_api = session
                .endpoints
                .as_ref()
                .and_then(|e| e.api.as_deref())
                .map(|s| s.trim_end_matches('/').to_string());
            let url = endpoints_api
                .as_deref()
                .map(|api| format!("{}/models", api))
                .unwrap_or_else(|| fallback_url.clone());
            send_copilot_models_request(&client, &url, &session.token, endpoints_api.as_deref())
                .await
        }
        // Exchange failed.  The stored secret may already be a session
        // token (e.g. imported manually), so try hitting `/models` with
        // the supplied token directly as a fallback.
        Err(exchange_err) => match send_copilot_models_request(&client, &fallback_url, key, None)
            .await
        {
            Ok(models) if !models.is_empty() => Ok(models),
            Ok(_) => Err(exchange_err.context(
                "Copilot session-token exchange failed and the fallback /models request \
                 returned no models",
            )),
            Err(fallback_err) => Err(exchange_err.context(format!(
                "Copilot session-token exchange failed; fallback /models request also failed: {:#}",
                fallback_err,
            ))),
        },
    };

    // Always merge with the curated catalog so newer/preview models the
    // user hasn't opted into yet still appear in the picker.  If the
    // live fetch failed outright, fall back to the catalog alone — the
    // exchange/request error was already surfaced as a warning via
    // `RequestDetails::emit_warning`.
    match live_result {
        Ok(live) => Ok(merge_copilot_models(live)),
        Err(e) => {
            tracing::warn!(
                target: "rustyclaw::providers",
                error = %format!("{:#}", e),
                "Falling back to static Copilot model catalog after live /models fetch failed"
            );
            Ok(copilot_static_models())
        }
    }
}

async fn send_copilot_models_request(
    client: &reqwest::Client,
    url: &str,
    bearer_token: &str,
    endpoints_api: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    let headers: [(&str, &str); 5] = [
        ("Accept", COPILOT_API_ACCEPT),
        ("User-Agent", COPILOT_API_USER_AGENT),
        ("Editor-Version", COPILOT_EDITOR_VERSION),
        ("Editor-Plugin-Version", COPILOT_EDITOR_PLUGIN_VERSION),
        ("Copilot-Integration-Id", COPILOT_INTEGRATION_ID),
    ];

    let mut details = RequestDetails::new("copilot.models_request", "GET", url.to_string())
        .with_provider("github-copilot")
        .with_request_headers(headers.iter().map(|(k, v)| (*k, *v)))
        .with_bearer(Some(bearer_token))
        .with_endpoints_api(endpoints_api);

    let mut req = client.get(url).bearer_auth(bearer_token);
    for (name, value) in headers {
        req = req.header(name, value);
    }

    let context = || format_request_context("GET", url, &headers, Some(bearer_token));

    let resp = match req.send().await {
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
    details = details.with_body(&body_text);

    let body: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            let err = wrap_err(e).context(format!("{}: failed to parse JSON response", context()));
            return Err(details.emit_warning(err));
        }
    };
    Ok(parse_models_response(&body))
}

/// Render an HTTP request's calling-side context (method, URL, headers)
/// for inclusion in user-facing error messages.  Any `Authorization`
/// header — and any header value that matches the supplied bearer
/// token — is redacted so secrets never leak into logs.
pub(crate) fn format_request_context(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    bearer_token: Option<&str>,
) -> String {
    let mut rendered: Vec<String> = headers
        .iter()
        .map(|(name, value)| format!("{}: {}", name, redact_header(name, value, bearer_token)))
        .collect();
    if let Some(tok) = bearer_token {
        rendered.push(format!("Authorization: Bearer {}", redact_secret(tok)));
    }
    format!("{} {} (headers: [{}])", method, url, rendered.join(", "),)
}

pub(crate) fn redact_header(name: &str, value: &str, bearer_token: Option<&str>) -> String {
    if name.eq_ignore_ascii_case("authorization") {
        return redact_secret(value);
    }
    if let Some(tok) = bearer_token
        && !tok.is_empty()
        && value.contains(tok)
    {
        return redact_secret(value);
    }
    value.to_string()
}

pub(crate) fn redact_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }
    format!("<redacted len={}>", trimmed.len())
}

pub fn truncate_for_error(body: &str) -> String {
    const MAX: usize = 512;
    if body.len() <= MAX {
        body.to_string()
    } else {
        let mut end = MAX;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}… (truncated, {} bytes total)", &body[..end], body.len())
    }
}

/// Fetch from the Google Gemini `/models` endpoint with metadata.
async fn fetch_google_models_detailed(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<ModelInfo>> {
    let key = match api_key {
        Some(k) => k,
        // No key — return empty so the outer match produces a clear error
        None => return Ok(Vec::new()),
    };

    // The Gemini API uses `?key=…` rather than a bearer header, but we
    // still want the structured details to capture the URL with the
    // key redacted, plus response status/headers and any error body.
    let public_url = format!("{}/models?key=<redacted>", base_url.trim_end_matches('/'));
    let real_url = format!("{}/models?key={}", base_url.trim_end_matches('/'), key);

    let mut details = RequestDetails::new("google.models", "GET", public_url.clone())
        .with_provider("google")
        .with_request_headers([("Accept", "application/json")])
        .with_bearer(Some(key));

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .with_context(|| format!("failed to build HTTP client for GET {}", public_url))
        .map_err(|e| details.clone().emit_warning(e))?;

    let resp = match client.get(&real_url).send().await {
        Ok(r) => r,
        Err(e) => {
            // Strip the URL from the reqwest error before it enters the
            // cause chain — otherwise its Display would leak the API key
            // (which lives in the query string for Gemini) into both
            // `tracing::warn!(error = %err)` output and the TUI details
            // dialog. We re-attach the redacted `public_url` via context.
            let err =
                wrap_err(e.without_url()).context(format!("GET {} failed to send", public_url));
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
            "GET {} returned HTTP {} — body: {}",
            public_url,
            status,
            truncate_for_error(&body)
        );
        return Err(details.emit_warning(err));
    }

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            // See note above re: stripping URL from reqwest errors on the
            // Gemini path — the request URL contains the API key.
            let err = wrap_err(e.without_url())
                .context(format!("GET {}: failed to read response body", public_url));
            return Err(details.emit_warning(err));
        }
    };
    details = details.with_body(&body_text);

    let body: serde_json::Value = match serde_json::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            let err =
                wrap_err(e).context(format!("GET {}: failed to parse JSON response", public_url));
            return Err(details.emit_warning(err));
        }
    };

    let models = body
        .get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let raw_name = m.get("name").and_then(|v| v.as_str())?;
                    let id = raw_name
                        .strip_prefix("models/")
                        .unwrap_or(raw_name)
                        .to_string();
                    let display_name = m
                        .get("displayName")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    // Google returns inputTokenLimit / outputTokenLimit
                    let context_length = m.get("inputTokenLimit").and_then(|v| v.as_u64());
                    Some(ModelInfo {
                        id,
                        name: display_name,
                        context_length,
                        pricing_prompt: None,
                        pricing_completion: None,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}
