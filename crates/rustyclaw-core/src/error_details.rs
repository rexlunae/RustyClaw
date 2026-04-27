//! Structured, user-facing error details for HTTP-bound calls.
//!
//! The provider model-fetch / Copilot session / OAuth device-flow paths
//! talk to several different APIs, and when they fail the user
//! historically only saw a single one-line `format!` blob.  That makes
//! it hard to figure out *which* request failed — was it the token
//! exchange?  the `/models` call?  did the server return 401, or 200
//! with an empty body?
//!
//! This module standardises that error reporting:
//!
//! * [`RequestDetails`] carries a structured snapshot of an outbound
//!   HTTP request and (optionally) the response it received: method,
//!   URL, request headers, status, response headers, body excerpt, the
//!   logical "step" name the call belongs to, and the provider id.
//! * [`RequestDetails::attach_to`] decorates an
//!   [`anyhow_tracing::Error`] with the same information as named
//!   fields, so that `tracing::warn!(error = %err, …)` carries the
//!   structured fields through to JSON log output.
//! * [`RequestDetails::emit_warning`] / [`emit_error`] both attach the
//!   fields and emit a `tracing::warn!` / `tracing::error!` event with
//!   `target = "rustyclaw::providers"` so the same information lands in
//!   structured logs.
//! * [`render_extended`] formats the resulting error chain plus all
//!   attached fields into a multi-line string that the TUI can show in
//!   its "details" dialog when a warning or error is selected.
//!
//! Sensitive header values (`Authorization` and any header whose value
//! contains the active bearer token) are redacted via the same helpers
//! that drive [`crate::providers::format_request_context`], so the same
//! representation is safe to log and to display in the UI.

use std::fmt;

use crate::providers::{redact_header, redact_secret, truncate_for_error};

/// Structured snapshot of an outbound HTTP call (and its response, when
/// available) for inclusion in error messages and structured logs.
///
/// Construct with [`RequestDetails::new`] and incrementally fill in the
/// optional fields with the builder-style setters as the call
/// progresses.
#[derive(Debug, Clone, Default)]
pub struct RequestDetails {
    /// HTTP method (e.g. `"GET"`, `"POST"`).
    pub method: String,
    /// Full request URL, including query string.
    pub url: String,
    /// Outgoing headers.  `Authorization` and any value containing the
    /// active bearer token will be redacted at format time.
    pub request_headers: Vec<(String, String)>,
    /// Response status code, if a response was received.
    pub status: Option<u16>,
    /// Response headers, if a response was received.
    pub response_headers: Vec<(String, String)>,
    /// Truncated response body excerpt, if available.
    pub body_excerpt: Option<String>,
    /// Provider id (e.g. `"github-copilot"`, `"openai"`, `"google"`).
    pub provider: Option<String>,
    /// Logical step name, e.g. `"copilot.session_exchange"` or
    /// `"openai.models"`.  Stable string so log queries can target it.
    pub step: &'static str,
    /// For Copilot, the plan-specific API host (e.g.
    /// `https://api.individual.githubcopilot.com`) discovered from the
    /// session-token exchange.  Helps confirm which endpoint was used.
    pub endpoints_api: Option<String>,
    /// Optional bearer token used for the request.  Only used to drive
    /// the redaction logic; never serialised or rendered.
    bearer_token: Option<String>,
}

impl RequestDetails {
    /// Start building a snapshot for a single HTTP call.
    pub fn new(step: &'static str, method: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            step,
            ..Self::default()
        }
    }

    /// Attach the provider id (e.g. `"github-copilot"`).
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Attach the bearer token used for this request, so that header
    /// values containing it can be redacted at render time.  The token
    /// itself is never included in the rendered output.
    pub fn with_bearer(mut self, token: Option<&str>) -> Self {
        self.bearer_token = token.map(str::to_owned);
        self
    }

    /// Replace the request-headers list.  Each `(name, value)` pair
    /// will be redacted by the same logic as
    /// [`crate::providers::format_request_context`] when rendered.
    pub fn with_request_headers<I, S1, S2>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (S1, S2)>,
        S1: Into<String>,
        S2: Into<String>,
    {
        self.request_headers = headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self
    }

    /// Record a response that was received (status + headers).
    pub fn with_response(mut self, status: u16, headers: &reqwest::header::HeaderMap) -> Self {
        self.status = Some(status);
        self.response_headers = headers
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        self
    }

    /// Record a (possibly large) response body, truncated for display.
    pub fn with_body(mut self, body: impl AsRef<str>) -> Self {
        self.body_excerpt = Some(truncate_for_error(body.as_ref()));
        self
    }

    /// Record the plan-specific endpoint URL discovered from the
    /// Copilot session-token exchange, when available.
    pub fn with_endpoints_api(mut self, api: Option<&str>) -> Self {
        self.endpoints_api = api.map(str::to_owned);
        self
    }

    fn redacted_headers(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        let bearer = self.bearer_token.as_deref();
        headers
            .iter()
            .map(|(name, value)| (name.clone(), redact_header(name, value, bearer)))
            .collect()
    }

    /// Render the request headers as a comma-separated `Header: value`
    /// list with sensitive values redacted.
    pub fn rendered_request_headers(&self) -> Vec<(String, String)> {
        let mut out = self.redacted_headers(&self.request_headers);
        // If the call carried a bearer token but it wasn't included in
        // the explicit `request_headers` list (we use
        // `bearer_auth(token)` directly on the reqwest builder), surface
        // a redacted Authorization line so the user sees one was sent.
        let already_has_auth = out
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"));
        if !already_has_auth && let Some(tok) = self.bearer_token.as_deref() {
            out.push((
                "Authorization".to_string(),
                format!("Bearer {}", redact_secret(tok)),
            ));
        }
        out
    }

    /// Render the response headers as a comma-separated list with
    /// sensitive values redacted.
    pub fn rendered_response_headers(&self) -> Vec<(String, String)> {
        self.redacted_headers(&self.response_headers)
    }

    /// Attach the structured fields on this snapshot to an
    /// [`anyhow_tracing::Error`].  The fields appended are: `step`,
    /// `provider`, `method`, `url`, `status`, `request_headers`,
    /// `response_headers`, `body`, and `endpoints_api` (only those that
    /// have values).  All header values are redacted as described
    /// above.
    pub fn attach_to(&self, mut err: anyhow_tracing::Error) -> anyhow_tracing::Error {
        err = err.with_field("step", self.step);
        if let Some(provider) = &self.provider {
            err = err.with_field("provider", provider);
        }
        err = err.with_field("method", &self.method);
        err = err.with_field("url", &self.url);
        if let Some(status) = self.status {
            err = err.with_field("status", status);
        }
        let req = render_headers(&self.rendered_request_headers());
        err = err.with_field("request_headers", req);
        let resp_headers = self.rendered_response_headers();
        if !resp_headers.is_empty() {
            err = err.with_field("response_headers", render_headers(&resp_headers));
        }
        if let Some(body) = &self.body_excerpt {
            err = err.with_field("body", body);
        }
        if let Some(api) = &self.endpoints_api {
            err = err.with_field("endpoints_api", api);
        }
        err
    }

    /// Emit a `tracing::warn!` with the structured fields, then attach
    /// them to the error.  Convenience for "I have an error and want
    /// both a structured log line and a TUI-displayable error".
    pub fn emit_warning(self, err: anyhow_tracing::Error) -> anyhow_tracing::Error {
        self.emit(tracing::Level::WARN, &err);
        self.attach_to(err)
    }

    /// Emit a `tracing::error!` with the structured fields, then attach
    /// them to the error.
    pub fn emit_error(self, err: anyhow_tracing::Error) -> anyhow_tracing::Error {
        self.emit(tracing::Level::ERROR, &err);
        self.attach_to(err)
    }

    fn emit(&self, level: tracing::Level, err: &anyhow_tracing::Error) {
        // We can't pass arbitrary key/value pairs to a single
        // `tracing::event!` call from a function (the macro requires
        // literal keys), so we render them into one structured line.
        let req_headers = render_headers(&self.rendered_request_headers());
        let resp_headers = render_headers(&self.rendered_response_headers());
        let status = self.status.map(|s| s.to_string()).unwrap_or_default();
        let provider = self.provider.as_deref().unwrap_or("");
        let endpoints_api = self.endpoints_api.as_deref().unwrap_or("");
        let body = self.body_excerpt.as_deref().unwrap_or("");
        match level {
            tracing::Level::ERROR => tracing::error!(
                target: "rustyclaw::providers",
                step = self.step,
                provider = %provider,
                method = %self.method,
                url = %self.url,
                status = %status,
                request_headers = %req_headers,
                response_headers = %resp_headers,
                endpoints_api = %endpoints_api,
                body = %body,
                error = %err,
                "provider request failed",
            ),
            _ => tracing::warn!(
                target: "rustyclaw::providers",
                step = self.step,
                provider = %provider,
                method = %self.method,
                url = %self.url,
                status = %status,
                request_headers = %req_headers,
                response_headers = %resp_headers,
                endpoints_api = %endpoints_api,
                body = %body,
                error = %err,
                "provider request failed",
            ),
        }
    }
}

/// Render a list of `(name, value)` header pairs as a single line
/// suitable for inclusion in a structured `tracing` field or an error
/// message body.
pub(crate) fn render_headers(headers: &[(String, String)]) -> String {
    let mut out = String::new();
    for (i, (name, value)) in headers.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
    }
    out
}

/// Render an [`anyhow_tracing::Error`] in long form, suitable for a
/// scrollable "details" dialog in the TUI:
///
/// ```text
/// Failed to fetch models from GitHub Copilot
///
/// Caused by:
///   1. GET https://… returned HTTP 401
///   2. unauthorized
///
/// Fields:
///   step             = copilot.models_request
///   provider         = github-copilot
///   method           = GET
///   url              = https://api.individual.githubcopilot.com/models
///   status           = 401
///   request_headers  = Accept: application/vnd.github+json, …
///   response_headers = content-type: application/json, …
///   body             = {"message": "Bad credentials", …}
/// ```
pub fn render_extended<E: ErrorLike + ?Sized>(err: &E) -> String {
    let mut s = String::new();
    s.push_str(&err.display_message());
    s.push('\n');

    // Causal chain
    let chain: Vec<String> = err.cause_chain();
    if chain.len() > 1 {
        s.push_str("\nCaused by:\n");
        for (i, cause) in chain.iter().enumerate().skip(1) {
            s.push_str(&format!("  {}. {}\n", i, cause));
        }
    }

    // Structured fields (anyhow-tracing only)
    let fields = err.fields();
    if !fields.is_empty() {
        s.push_str("\nFields:\n");
        let key_width = fields
            .iter()
            .map(|(k, _)| k.len())
            .max()
            .unwrap_or(0)
            .min(24);
        for (key, value) in &fields {
            s.push_str(&format!(
                "  {:<width$} = {}\n",
                key,
                value,
                width = key_width
            ));
        }
    }

    s.trim_end().to_string()
}

/// Anything that can produce a display message, a chain of causes, and
/// optionally a list of named fields.  Implemented for both
/// `anyhow_tracing::Error` and `anyhow::Error` so the TUI can render
/// either through [`render_extended`].
pub trait ErrorLike {
    /// Top-level error message.
    fn display_message(&self) -> String;
    /// Full causal chain, with the top-level message first.
    fn cause_chain(&self) -> Vec<String>;
    /// Named structured fields, if any.
    fn fields(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }
}

impl ErrorLike for anyhow_tracing::Error {
    fn display_message(&self) -> String {
        // The `Display` impl on `anyhow_tracing::Error` appends fields
        // in `[k=v, …]` form which is not what we want for the
        // human-readable header — strip that off and emit fields
        // separately below.
        format!("{}", anyhow_chain_top(self))
    }

    fn cause_chain(&self) -> Vec<String> {
        self.chain().map(|c| c.to_string()).collect()
    }

    fn fields(&self) -> Vec<(&'static str, String)> {
        self.fields()
            .iter()
            .map(|(k, v)| (*k, v.to_string()))
            .collect()
    }
}

impl ErrorLike for anyhow::Error {
    fn display_message(&self) -> String {
        self.to_string()
    }

    fn cause_chain(&self) -> Vec<String> {
        self.chain().map(|c| c.to_string()).collect()
    }
}

/// Helper to display an `anyhow_tracing::Error` without the trailing
/// `[k=v, …]` field list (those are surfaced separately by
/// [`render_extended`]).
fn anyhow_chain_top(err: &anyhow_tracing::Error) -> impl fmt::Display + '_ {
    struct Top<'a>(&'a anyhow_tracing::Error);
    impl fmt::Display for Top<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // chain().next() is the top-level error; fall back to the
            // full Display if for some reason the chain is empty.
            if let Some(top) = self.0.chain().next() {
                write!(f, "{}", top)
            } else {
                write!(f, "{}", self.0)
            }
        }
    }
    Top(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_to_redacts_authorization_header() {
        let err = anyhow_tracing::Error::msg("boom");
        let details = RequestDetails::new("test.step", "GET", "https://example.com/models")
            .with_provider("openai")
            .with_request_headers([
                ("Accept", "application/json"),
                ("Authorization", "Bearer s3cr3t-t0k3n"),
            ])
            .with_bearer(Some("s3cr3t-t0k3n"));
        let err = details.attach_to(err);
        let rendered = render_extended(&err);
        assert!(!rendered.contains("s3cr3t-t0k3n"), "leaked: {}", rendered);
        assert!(rendered.contains("Authorization"));
        assert!(rendered.contains("<redacted"));
    }

    #[test]
    fn attach_to_redacts_value_containing_bearer() {
        let err = anyhow_tracing::Error::msg("boom");
        let details = RequestDetails::new("test.step", "GET", "https://example.com/models")
            .with_request_headers([("X-Custom", "prefix:super-secret-token:suffix")])
            .with_bearer(Some("super-secret-token"));
        let err = details.attach_to(err);
        let rendered = render_extended(&err);
        assert!(
            !rendered.contains("super-secret-token"),
            "leaked: {}",
            rendered
        );
    }

    #[test]
    fn attach_to_includes_status_and_body_when_available() {
        let err = anyhow_tracing::Error::msg("boom");
        let details = RequestDetails::new("test.step", "GET", "https://example.com/models")
            .with_provider("openai")
            .with_body("{\"error\": \"bad credentials\"}");
        let mut details = details;
        details.status = Some(401);
        let err = details.attach_to(err);
        let rendered = render_extended(&err);
        assert!(rendered.contains("status"));
        assert!(rendered.contains("401"));
        assert!(rendered.contains("bad credentials"));
        assert!(rendered.contains("provider"));
        assert!(rendered.contains("openai"));
        assert!(rendered.contains("step"));
    }

    #[test]
    fn render_extended_includes_cause_chain() {
        let err: anyhow_tracing::Error = anyhow_tracing::Error::msg("inner failure")
            .context("middle layer")
            .context("outer summary");
        let rendered = render_extended(&err);
        assert!(rendered.contains("outer summary"));
        assert!(rendered.contains("Caused by:"));
        assert!(rendered.contains("middle layer"));
        assert!(rendered.contains("inner failure"));
    }
}
