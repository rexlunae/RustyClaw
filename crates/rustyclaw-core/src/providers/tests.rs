//! Provider tests.

#![allow(unused_imports)]
use super::device_flow::parse_form_encoded_token_response;
use super::models::{is_chat_model, parse_models_response};
use super::*;

use super::*;

#[test]
fn test_provider_by_id() {
    let provider = provider_by_id("anthropic");
    assert!(provider.is_some());
    assert_eq!(provider.unwrap().display, "Anthropic (Claude)");

    let provider = provider_by_id("github-copilot");
    assert!(provider.is_some());
    assert_eq!(provider.unwrap().display, "GitHub Copilot");
    assert_eq!(provider.unwrap().auth_method, AuthMethod::DeviceFlow);

    let provider = provider_by_id("nonexistent");
    assert!(provider.is_none());
}

#[test]
fn test_deepseek_provider_config() {
    let provider = provider_by_id("deepseek").unwrap();
    assert_eq!(provider.display, "DeepSeek");
    assert_eq!(provider.auth_method, AuthMethod::ApiKey);
    assert_eq!(provider.secret_key, Some("DEEPSEEK_API_KEY"));
    assert_eq!(provider.base_url, Some("https://api.deepseek.com/v1"));
    assert!(provider.models.contains(&"deepseek-v4-pro"));
    assert!(provider.models.contains(&"deepseek-v4-flash"));
}

#[test]
fn test_provider_auth_methods() {
    // API key providers
    let anthropic = provider_by_id("anthropic").unwrap();
    assert_eq!(anthropic.auth_method, AuthMethod::ApiKey);
    assert!(anthropic.device_flow.is_none());

    // Device flow providers
    let copilot = provider_by_id("github-copilot").unwrap();
    assert_eq!(copilot.auth_method, AuthMethod::DeviceFlow);
    assert!(copilot.device_flow.is_some());

    let copilot_proxy = provider_by_id("copilot-proxy").unwrap();
    assert_eq!(copilot_proxy.auth_method, AuthMethod::DeviceFlow);
    assert!(copilot_proxy.device_flow.is_some());

    // Optional auth providers
    let ollama = provider_by_id("ollama").unwrap();
    assert_eq!(ollama.auth_method, AuthMethod::OptionalApiKey);
    assert_eq!(ollama.secret_key, Some("OLLAMA_API_KEY"));
}

#[test]
fn test_github_copilot_provider_config() {
    let provider = provider_by_id("github-copilot").unwrap();
    assert_eq!(provider.id, "github-copilot");
    assert_eq!(provider.secret_key, Some("GITHUB_COPILOT_TOKEN"));

    let device_config = provider.device_flow.unwrap();
    assert_eq!(
        device_config.device_auth_url,
        "https://github.com/login/device/code"
    );
    assert_eq!(
        device_config.token_url,
        "https://github.com/login/oauth/access_token"
    );
    assert!(!device_config.client_id.is_empty());
}

#[test]
fn test_copilot_proxy_provider_config() {
    let provider = provider_by_id("copilot-proxy").unwrap();
    assert_eq!(provider.id, "copilot-proxy");
    assert_eq!(provider.secret_key, Some("COPILOT_PROXY_TOKEN"));
    assert_eq!(provider.base_url, None); // Should prompt for URL

    let device_config = provider.device_flow.unwrap();
    // Should use same device flow as github-copilot
    assert_eq!(
        device_config.device_auth_url,
        "https://github.com/login/device/code"
    );
}

#[test]
fn test_token_response_parsing() {
    // Test successful token response
    let json = r#"{"access_token":"test_token","token_type":"bearer"}"#;
    let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Success { access_token, .. } => {
            assert_eq!(access_token, "test_token");
        }
        _ => panic!("Expected Success variant"),
    }

    // Test pending response
    let json = r#"{"error":"authorization_pending"}"#;
    let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Pending { error, .. } => {
            assert_eq!(error, "authorization_pending");
        }
        _ => panic!("Expected Pending variant"),
    }

    // Test success response with extra fields (e.g. scope)
    let json = r#"{"access_token":"gho_xxx","token_type":"bearer","scope":"read:user"}"#;
    let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Success { access_token, .. } => {
            assert_eq!(access_token, "gho_xxx");
        }
        _ => panic!("Expected Success variant"),
    }

    // Test success response even if token_type is missing
    let json = r#"{"access_token":"gho_xxx"}"#;
    let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Success {
            access_token,
            token_type,
            ..
        } => {
            assert_eq!(access_token, "gho_xxx");
            assert_eq!(token_type, "bearer"); // defaults to "bearer"
        }
        _ => panic!("Expected Success variant"),
    }

    // Test error response with description
    let json = r#"{"error":"access_denied","error_description":"user denied"}"#;
    let raw: RawTokenResponse = serde_json::from_str(json).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Pending {
            error,
            error_description,
        } => {
            assert_eq!(error, "access_denied");
            assert_eq!(error_description, Some("user denied".to_string()));
        }
        _ => panic!("Expected Pending variant"),
    }
}

#[test]
fn test_form_encoded_token_response_parsing() {
    // Success response in URL-encoded format
    let body = "access_token=gho_xxx123&token_type=bearer&scope=read%3Auser";
    let raw = parse_form_encoded_token_response(body).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Success { access_token, .. } => {
            assert_eq!(access_token, "gho_xxx123");
        }
        _ => panic!("Expected Success variant"),
    }

    // Pending response in URL-encoded format ('+' must decode to space)
    let body = "error=authorization_pending&error_description=waiting+for+user";
    let raw = parse_form_encoded_token_response(body).unwrap();
    let response: TokenResponse = raw.into();
    match response {
        TokenResponse::Pending {
            error,
            error_description,
        } => {
            assert_eq!(error, "authorization_pending");
            assert_eq!(error_description, Some("waiting for user".to_string()));
        }
        _ => panic!("Expected Pending variant"),
    }

    // Should return None for JSON
    assert!(parse_form_encoded_token_response(r#"{"access_token":"x"}"#).is_none());

    // Should return None for empty / no '='
    assert!(parse_form_encoded_token_response("hello world").is_none());
}

#[test]
fn test_all_providers_have_valid_config() {
    for provider in PROVIDERS {
        // Verify basic fields are set
        assert!(!provider.id.is_empty());
        assert!(!provider.display.is_empty());

        // Verify auth consistency
        match provider.auth_method {
            AuthMethod::ApiKey => {
                assert!(
                    provider.secret_key.is_some(),
                    "Provider {} with ApiKey auth must have secret_key",
                    provider.id
                );
                assert!(
                    provider.device_flow.is_none(),
                    "Provider {} with ApiKey auth should not have device_flow",
                    provider.id
                );
            }
            AuthMethod::DeviceFlow => {
                assert!(
                    provider.secret_key.is_some(),
                    "Provider {} with DeviceFlow auth must have secret_key",
                    provider.id
                );
                assert!(
                    provider.device_flow.is_some(),
                    "Provider {} with DeviceFlow auth must have device_flow config",
                    provider.id
                );
            }
            AuthMethod::None => {
                assert!(
                    provider.secret_key.is_none(),
                    "Provider {} with None auth should not have secret_key",
                    provider.id
                );
                assert!(
                    provider.device_flow.is_none(),
                    "Provider {} with None auth should not have device_flow",
                    provider.id
                );
            }
            AuthMethod::OptionalApiKey => {
                assert!(
                    provider.device_flow.is_none(),
                    "Provider {} with OptionalApiKey auth should not have device_flow",
                    provider.id
                );
            }
        }
    }
}

#[test]
fn test_needs_copilot_session() {
    assert!(needs_copilot_session("github-copilot"));
    assert!(needs_copilot_session("copilot-proxy"));
    assert!(!needs_copilot_session("openai"));
    assert!(!needs_copilot_session("anthropic"));
    assert!(!needs_copilot_session("google"));
    assert!(!needs_copilot_session("ollama"));
    assert!(!needs_copilot_session("custom"));
}

#[test]
fn test_copilot_capabilities_type_chat_is_chat_model() {
    let entry = serde_json::json!({
        "id": "claude-sonnet-4.5",
        "object": "model",
        "capabilities": {
            "type": "chat"
        }
    });

    assert!(is_chat_model(&entry));
}

#[test]
fn test_parse_copilot_models_response_filters_non_chat_models() {
    let body = serde_json::json!({
        "data": [
            {
                "id": "gpt-5.2",
                "object": "model",
                "name": "GPT 5.2",
                "capabilities": { "type": "chat" }
            },
            {
                "id": "text-embedding-3-large",
                "object": "model",
                "capabilities": { "chat": false }
            }
        ]
    });

    let models = parse_models_response(&body);

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "gpt-5.2");
    assert_eq!(models[0].name.as_deref(), Some("GPT 5.2"));
}

#[test]
fn test_parse_anthropic_models_page() {
    let body = serde_json::json!({
        "data": [
            {
                "type": "model",
                "id": "claude-opus-4-20250514",
                "display_name": "Claude Opus 4",
                "created_at": "2025-05-14T00:00:00Z"
            },
            {
                "type": "model",
                "id": "claude-sonnet-4-20250514",
                "display_name": "Claude Sonnet 4",
                "created_at": "2025-05-14T00:00:00Z"
            }
        ],
        "has_more": false,
        "first_id": "claude-opus-4-20250514",
        "last_id": "claude-sonnet-4-20250514"
    });

    let models = super::models::parse_anthropic_models_page(&body);

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "claude-opus-4-20250514");
    assert_eq!(models[0].name.as_deref(), Some("Claude Opus 4"));
    assert_eq!(models[1].id, "claude-sonnet-4-20250514");
}

/// Sanity check: a realistic Copilot `/models` response (mixed
/// chat + embedding entries, both with and without
/// `capabilities.type == "chat"`) should not get filtered down
/// to zero by [`parse_models_response`].
///
/// This guards against regressions where we tighten the filter
/// such that real Copilot Pro / Business model shapes get
/// dropped, leaving the user staring at "empty model list".
#[test]
fn test_parse_copilot_models_response_realistic_pro_and_business_shapes() {
    let body = serde_json::json!({
        "data": [
            // Pro plan: chat model, capabilities.type = "chat"
            {
                "id": "gpt-4.1",
                "object": "model",
                "model_picker_enabled": true,
                "name": "GPT-4.1",
                "vendor": "Azure OpenAI",
                "capabilities": {
                    "family": "gpt-4.1",
                    "type": "chat",
                    "supports": { "tool_calls": true, "streaming": true }
                }
            },
            // Business plan: chat model, identical structurally
            {
                "id": "claude-sonnet-4",
                "object": "model",
                "model_picker_enabled": true,
                "name": "Claude Sonnet 4",
                "vendor": "Anthropic",
                "capabilities": {
                    "family": "claude-sonnet-4",
                    "type": "chat",
                    "supports": { "tool_calls": true, "streaming": true }
                }
            },
            // Embedding model — should be filtered out
            {
                "id": "text-embedding-3-small",
                "object": "model",
                "name": "Embedding v3 small",
                "capabilities": { "type": "embeddings" }
            },
            // Chat-capable model with no `capabilities.type` but
            // present in id (defensive): should still appear,
            // since the filter is a heuristic.
            {
                "id": "gpt-5",
                "object": "model",
                "name": "GPT 5"
            }
        ]
    });
    let models = parse_models_response(&body);
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4.1"), "ids = {:?}", ids);
    assert!(ids.contains(&"claude-sonnet-4"), "ids = {:?}", ids);
    assert!(
        !ids.contains(&"text-embedding-3-small"),
        "embedding leaked: ids = {:?}",
        ids
    );
    assert!(
        !models.is_empty(),
        "Copilot response with chat-capable models should not be filtered to empty"
    );
}

#[test]
fn test_copilot_session_response_parsing() {
    let json = r#"{"token":"tid=abc123;exp=9999999999","expires_at":1750000000}"#;
    let resp: CopilotSessionResponse = serde_json::from_str(json).unwrap();
    assert!(resp.token.starts_with("tid="));
    assert_eq!(resp.expires_at, 1750000000);
    assert!(resp.endpoints.is_none());
}

#[test]
fn test_copilot_session_response_parses_plan_endpoints() {
    let json = r#"{
        "token": "tid=abc123;exp=9999999999",
        "expires_at": 1750000000,
        "endpoints": {
            "api": "https://api.individual.githubcopilot.com",
            "telemetry": "https://copilot-telemetry.githubusercontent.com/telemetry"
        }
    }"#;
    let resp: CopilotSessionResponse = serde_json::from_str(json).unwrap();
    let endpoints = resp.endpoints.expect("endpoints should be parsed");
    assert_eq!(
        endpoints.api.as_deref(),
        Some("https://api.individual.githubcopilot.com"),
    );
}

#[test]
fn test_redact_secret_hides_token_value() {
    let r = redact_secret("tid=abc123;exp=9999999999");
    assert!(r.starts_with("<redacted"), "got {}", r);
    assert!(!r.contains("abc123"), "got {}", r);
    assert!(r.contains("len="));
    assert_eq!(redact_secret(""), "<empty>");
}

#[test]
fn test_redact_header_redacts_authorization_case_insensitive() {
    let bearer = Some("super-secret-token");
    assert!(!redact_header("Authorization", "Bearer foo", bearer).contains("foo"));
    assert!(!redact_header("authorization", "token foo", bearer).contains("foo"));
}

#[test]
fn test_redact_header_redacts_value_containing_bearer() {
    let bearer = Some("super-secret-token");
    let redacted = redact_header("X-Custom", "prefix:super-secret-token:suffix", bearer);
    assert!(!redacted.contains("super-secret-token"), "got {}", redacted);
}

#[test]
fn test_redact_header_passes_through_non_secret() {
    assert_eq!(
        redact_header("User-Agent", "RustyClaw", Some("tok")),
        "RustyClaw",
    );
}

#[test]
fn test_format_request_context_includes_method_url_and_redacts_auth() {
    let headers = [
        ("Accept", "application/vnd.github+json"),
        ("User-Agent", "RustyClaw"),
    ];
    let ctx = format_request_context(
        "GET",
        "https://api.example.com/models",
        &headers,
        Some("super-secret-token"),
    );
    assert!(ctx.starts_with("GET https://api.example.com/models"));
    assert!(ctx.contains("Accept: application/vnd.github+json"));
    assert!(ctx.contains("User-Agent: RustyClaw"));
    assert!(ctx.contains("Authorization: Bearer <redacted"));
    assert!(!ctx.contains("super-secret-token"));
}

#[test]
fn test_truncate_for_error_truncates_long_bodies() {
    let body = "x".repeat(2000);
    let truncated = truncate_for_error(&body);
    assert!(truncated.len() < body.len());
    assert!(truncated.contains("truncated"));
    assert!(truncated.contains("2000 bytes"));
}

#[test]
fn test_truncate_for_error_passes_through_short_bodies() {
    assert_eq!(truncate_for_error("hello"), "hello");
}

// ── Provider HTTP client deadlines ──────────────────────────────────────────

/// Guards the property the default exists for: a provider request must not be
/// able to wait forever. The 1h56m stall that motivated this was a request
/// with no read deadline at all, ended only by TCP giving up.
#[test]
fn a_provider_request_is_bounded_by_default() {
    // SAFETY: single-threaded test with no other reader of this variable.
    unsafe { std::env::remove_var("RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS") };

    assert_eq!(
        super::provider_read_timeout(),
        Some(super::PROVIDER_READ_TIMEOUT),
        "the default must bound how long a provider may go silent"
    );
    assert!(
        super::PROVIDER_READ_TIMEOUT >= std::time::Duration::from_secs(60),
        "generous enough that a slow time-to-first-token is not cut off"
    );
}

/// The override exists for local models with long cold-start latency, and `0`
/// is the documented way back to the old unbounded behaviour.
#[test]
fn the_override_is_honoured_including_the_opt_out() {
    // SAFETY: as above.
    unsafe { std::env::set_var("RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS", "600") };
    assert_eq!(
        super::provider_read_timeout(),
        Some(std::time::Duration::from_secs(600))
    );

    unsafe { std::env::set_var("RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS", "0") };
    assert_eq!(
        super::provider_read_timeout(),
        None,
        "0 disables the read timeout"
    );

    // A typo must not silently disable the deadline — that would turn a
    // mistake into the exact bug this bounds.
    unsafe { std::env::set_var("RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS", "banana") };
    assert_eq!(
        super::provider_read_timeout(),
        Some(super::PROVIDER_READ_TIMEOUT),
        "an unparseable value falls back to the default, not to unbounded"
    );

    unsafe { std::env::remove_var("RUSTYCLAW_PROVIDER_READ_TIMEOUT_SECS") };
}

/// A missing pin file must fail loudly, not silently disable the pin: an
/// operator who configured a pin believes provider traffic is protected, and
/// a quietly-ignored pin would betray that.
#[test]
fn a_missing_pin_file_is_a_hard_error() {
    let err = super::set_provider_tls_pin(std::path::Path::new(
        "/nonexistent/rustyclaw-provider-ca.pem",
    ));
    assert!(err.is_err(), "a missing pin file must fail");
}

/// Pinning a valid certificate installs the pin; pinning a *second* one is
/// refused — the pin is a boot-time decision, and a later reload silently
/// swapping it would defeat the purpose. Uses a real generated cert so the
/// parse path is exercised, not just the lock.
#[test]
fn a_second_pin_set_is_refused() {
    // Self-signed cert, so the test never trips on a foreign CA.
    let signing_key = rcgen::KeyPair::generate().expect("key pair");
    let params =
        rcgen::CertificateParams::new(vec!["rustyclaw-pin-test".to_string()]).expect("cert params");
    let cert = params.self_signed(&signing_key).expect("generate cert");
    let pem = cert.pem();

    let dir = std::env::temp_dir().join(format!("rustyclaw-tls-pin-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("ca.pem");
    std::fs::write(&path, pem.as_bytes()).expect("write pem");

    let first = super::set_provider_tls_pin(&path);
    assert!(
        first.is_ok(),
        "a valid cert must install: {}",
        first.err().map(|e| e.to_string()).unwrap_or_default()
    );
    let second = super::set_provider_tls_pin(&path);
    assert!(
        second.is_err(),
        "a second set must be refused once a pin is installed"
    );
    std::fs::remove_dir_all(&dir).ok();
}
