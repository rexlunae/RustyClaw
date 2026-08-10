//! `http_request`: an HTTP client with the method, headers and body exposed.
//!
//! [`web_fetch`](super::web) reads pages. It sends a GET, extracts the
//! readable part of the HTML, and treats a non-2xx reply as a failure. That is
//! the right shape for reading documentation and the wrong shape for talking
//! to an API, where the method matters, the body matters, and a 422 with a
//! JSON explanation is the answer rather than an error (issue #145).
//!
//! Both tools share the SSRF checks in [`super::web`] — the initial URL is
//! validated before the request, and every redirect hop is re-validated by the
//! client's policy. That sharing is deliberate: a second HTTP tool with its
//! own copy of the rules is a second place for them to drift.

use crate::tools::error::{ToolError, ToolResult};
use crate::tools::web::{ssrf_check_blocking, ssrf_redirect_policy};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, instrument, warn};

/// Methods the tool will send.
///
/// An allowlist rather than "pass the string through": `reqwest` accepts any
/// token as a method, so a typo like `"GTE"` would be sent verbatim and come
/// back as a puzzling 405 rather than as "you spelled GET wrong".
const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// Hard ceiling on the response body held in memory, regardless of `max_chars`.
///
/// `max_chars` bounds what is *shown*. Without a separate byte cap, pointing
/// the tool at a large file would still read the whole thing into memory
/// before truncating the display copy.
const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

/// Default and ceiling for the per-request timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;

/// How the caller asked for the request body to be built.
///
/// Modelled as one value rather than three independent options so that
/// "`json` and `form` together" is a state the code cannot hold, instead of
/// one it has to remember to check for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestBody {
    /// No body.
    None,
    /// Sent verbatim. `Content-Type` comes from `headers`, if the caller set one.
    Raw(String),
    /// Serialised as JSON, with `Content-Type: application/json`.
    Json(String),
    /// URL-encoded, with `Content-Type: application/x-www-form-urlencoded`.
    Form(Vec<(String, String)>),
}

impl RequestBody {
    /// Read the three body parameters, refusing any combination of them.
    pub(crate) fn from_args(args: &Value) -> Result<Self, String> {
        let raw = args.get("body").and_then(|v| v.as_str());
        let json = args.get("json").filter(|v| !v.is_null());
        let form = args.get("form").and_then(|v| v.as_object());

        let given: Vec<&str> = [
            raw.map(|_| "body"),
            json.map(|_| "json"),
            form.map(|_| "form"),
        ]
        .into_iter()
        .flatten()
        .collect();

        if given.len() > 1 {
            // Picking one silently would send a body the caller did not ask
            // for, and the request would look like it worked.
            return Err(format!(
                "Give only one of body, json or form — got {}",
                given.join(" and ")
            ));
        }

        if let Some(raw) = raw {
            return Ok(RequestBody::Raw(raw.to_string()));
        }
        if let Some(json) = json {
            return Ok(RequestBody::Json(json.to_string()));
        }
        if let Some(form) = form {
            let mut pairs = Vec::with_capacity(form.len());
            for (key, value) in form {
                // A form field is a string on the wire. Rendering a number or
                // a bool the way JSON writes it is what a caller means by
                // `{"page": 2}`; rendering a nested object as `{"a":1}` is
                // not, so those are refused rather than guessed at.
                let rendered = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    other => {
                        return Err(format!(
                            "form field `{key}` is {}, which has no single form encoding — \
                             use `body` or `json` for structured values",
                            kind_of(other)
                        ));
                    }
                };
                pairs.push((key.clone(), rendered));
            }
            return Ok(RequestBody::Form(pairs));
        }
        Ok(RequestBody::None)
    }
}

/// The JSON type name, for error messages.
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Normalise and check the method.
pub(crate) fn method_of(args: &Value) -> Result<reqwest::Method, String> {
    let raw = args
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase();

    if !METHODS.contains(&raw.as_str()) {
        return Err(format!(
            "Unsupported method `{raw}`. Use one of: {}",
            METHODS.join(", ")
        ));
    }
    reqwest::Method::from_bytes(raw.as_bytes()).map_err(|e| format!("Invalid method `{raw}`: {e}"))
}

/// Clamp the requested timeout into something a stuck request cannot outlive.
pub(crate) fn timeout_of(args: &Value) -> Duration {
    let secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Render the response for the model.
///
/// A free function over the parts rather than over a `reqwest::Response`, so
/// the formatting — which is what the model actually reads — can be tested
/// without a live server.
pub(crate) fn render_response(
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &str,
    body_bytes: usize,
    truncated_at_cap: bool,
    max_chars: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("HTTP {status} {reason}\n"));

    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\n"));
    }

    if body.is_empty() {
        out.push_str("\n(empty body)\n");
        return out;
    }

    out.push('\n');
    let shown: String = body.chars().take(max_chars).collect();
    out.push_str(&shown);
    if !shown.ends_with('\n') {
        out.push('\n');
    }

    // Two different truncations, and conflating them would mislead: one says
    // "the response was bigger than the display limit", the other says "the
    // tool stopped reading and the rest never arrived".
    if shown.chars().count() < body.chars().count() {
        out.push_str(&format!(
            "\n[shown: {} of {} characters — raise max_chars for more]\n",
            shown.chars().count(),
            body.chars().count()
        ));
    }
    if truncated_at_cap {
        out.push_str(&format!(
            "\n[body exceeded the {MAX_BODY_BYTES}-byte read cap; stopped after \
             {body_bytes} bytes — use web_fetch with to_file to save it instead]\n"
        ));
    }
    out
}

/// Response header names worth showing.
///
/// The full set is mostly transport noise (`Date`, `Server`, CDN headers) that
/// costs context and tells the model nothing. These are the ones that change
/// what a caller does next.
const INTERESTING_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "location",
    "retry-after",
    "www-authenticate",
    "link",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "x-request-id",
];

/// Pick the response headers to show, in a stable order.
pub(crate) fn interesting_headers(all: &[(String, String)]) -> Vec<(String, String)> {
    let mut picked: Vec<(String, String)> = Vec::new();
    for want in INTERESTING_HEADERS {
        for (name, value) in all {
            if name.eq_ignore_ascii_case(want) {
                picked.push((name.clone(), value.clone()));
            }
        }
    }
    picked
}

/// Perform an HTTP request with the method, headers and body under the
/// caller's control.
#[instrument(skip(args, _workspace_dir), fields(url, method))]
pub async fn exec_http_request_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: url".to_string())?;

    let method = method_of(args)?;
    let body = RequestBody::from_args(args)?;
    let timeout = timeout_of(args);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(50_000) as usize;

    tracing::Span::current().record("url", url);
    tracing::Span::current().record("method", method.as_str());

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".into());
    }

    // Same SSRF story as web_fetch: the validator resolves, so it runs off the
    // async executor, and every redirect hop is re-checked by the policy.
    {
        let url_owned = url.to_string();
        tokio::task::spawn_blocking(move || ssrf_check_blocking(&url_owned))
            .await
            .map_err(|e| format!("SSRF validation task failed: {e}"))??;
    }

    let client = reqwest::Client::builder()
        .user_agent("RustyClaw/0.1 (http_request tool)")
        .redirect(ssrf_redirect_policy(10))
        .timeout(timeout)
        .build()
        .map_err(|e| ToolError::context("Failed to create HTTP client", e))?;

    let mut request = client.request(method.clone(), url);

    if let Some(auth) = args.get("authorization").and_then(|v| v.as_str()) {
        request = request.header("Authorization", auth);
        warn!(
            url = %url,
            "http_request: Authorization header provided — value NOT logged (security sensitive)"
        );
    }

    if let Some(headers) = args.get("headers").and_then(|v| v.as_object()) {
        for (key, value) in headers {
            if let Some(val_str) = value.as_str() {
                request = request.header(key.as_str(), val_str);
            }
        }
    }

    // Applied after the caller's headers so an explicit Content-Type in
    // `headers` is not silently overwritten by the body's default.
    request = match &body {
        RequestBody::None => request,
        RequestBody::Raw(raw) => request.body(raw.clone()),
        RequestBody::Json(json) => {
            let has_content_type = args
                .get("headers")
                .and_then(|v| v.as_object())
                .is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("content-type")));
            let request = if has_content_type {
                request
            } else {
                request.header("Content-Type", "application/json")
            };
            request.body(json.clone())
        }
        RequestBody::Form(pairs) => request.form(pairs),
    };

    debug!(method = %method, timeout_secs = timeout.as_secs(), "Sending HTTP request");

    let response = request.send().await.map_err(|e| {
        warn!(error = %e, "HTTP request failed");
        format!("HTTP request failed: {e}")
    })?;

    let status = response.status();
    let reason = status.canonical_reason().unwrap_or("").to_string();
    let all_headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect();

    // Read with a byte cap rather than `.text()`, which would pull an
    // arbitrarily large body into memory before anything got truncated.
    let (bytes, truncated_at_cap) = read_capped(response).await?;
    let body_bytes = bytes.len();
    let text = String::from_utf8_lossy(&bytes).to_string();

    debug!(
        status = status.as_u16(),
        body_bytes, "Received HTTP response"
    );

    // A non-2xx is returned, not raised. For an API, the failing status *and*
    // its body are the answer — web_fetch turns them into an error, which
    // throws away the part that explains what went wrong.
    Ok(render_response(
        status.as_u16(),
        &reason,
        &interesting_headers(&all_headers),
        &text,
        body_bytes,
        truncated_at_cap,
        max_chars,
    ))
}

/// Read the body, stopping at [`MAX_BODY_BYTES`].
///
/// Returns the bytes read and whether the cap is why reading stopped.
async fn read_capped(mut response: reqwest::Response) -> ToolResult<(Vec<u8>, bool)> {
    let mut out: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(ToolError::Http)? {
        let room = MAX_BODY_BYTES.saturating_sub(out.len());
        if chunk.len() >= room {
            out.extend_from_slice(&chunk[..room]);
            return Ok((out, true));
        }
        out.extend_from_slice(&chunk);
    }
    Ok((out, false))
}

/// Sync entry point, for the registry's `execute` slot.
///
/// The async implementation above is what actually runs — `http_request` is
/// listed in `ASYNC_NATIVE_TOOLS` — but the registry requires a sync function,
/// and a tool reached through the sync path should still work rather than
/// return "unimplemented".
pub fn exec_http_request(args: &Value, workspace_dir: &Path) -> ToolResult {
    let args = args.clone();
    let workspace_dir = workspace_dir.to_path_buf();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ToolError::context("Failed to build a runtime for http_request", e))?
            .block_on(exec_http_request_async(&args, &workspace_dir))
    })
    .join()
    .map_err(|_| "http_request worker thread panicked".to_string())?
}

/// Parameter schema. Kept next to the implementation so the two are edited
/// together.
pub fn http_request_params() -> Vec<crate::tools::ToolParam> {
    use crate::tools::ToolParam;
    vec![
        ToolParam {
            name: "url".into(),
            description: "HTTP or HTTPS URL to request.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "method".into(),
            description: "GET (default), POST, PUT, PATCH, DELETE, HEAD or OPTIONS.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "headers".into(),
            description: "Request headers as a JSON object, e.g. {\"X-Api-Key\": \"...\"}. \
                          A Content-Type set here wins over the one implied by 'json'."
                .into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "authorization".into(),
            description: "Authorization header value, e.g. 'Bearer <token>' or 'token <pat>'."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "json".into(),
            description: "Request body as JSON. Sends Content-Type: application/json. \
                          Mutually exclusive with 'body' and 'form'."
                .into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "form".into(),
            description: "Request body as form fields. Sends Content-Type: \
                          application/x-www-form-urlencoded. Values must be strings, \
                          numbers or booleans. Mutually exclusive with 'body' and 'json'."
                .into(),
            param_type: "object".into(),
            required: false,
        },
        ToolParam {
            name: "body".into(),
            description: "Request body sent verbatim. Set Content-Type yourself via \
                          'headers'. Mutually exclusive with 'json' and 'form'."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "timeout_secs".into(),
            description: "Request timeout in seconds. Default 30, maximum 300.".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "max_chars".into(),
            description: "Maximum characters of the response body to return. Default 50000.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_missing_method_means_get() {
        assert_eq!(
            method_of(&json!({})).expect("default"),
            reqwest::Method::GET
        );
    }

    /// Lowercase is what a model writes about half the time.
    #[test]
    fn the_method_is_case_and_space_insensitive() {
        assert_eq!(
            method_of(&json!({"method": " post "})).expect("normalised"),
            reqwest::Method::POST
        );
    }

    /// A typo passed straight to reqwest becomes a 405 from the server, which
    /// reads as "the API refused" rather than "you misspelled GET".
    #[test]
    fn a_misspelled_method_is_refused_by_name() {
        let err = method_of(&json!({"method": "GTE"})).expect_err("must be refused");
        assert!(err.contains("GTE"), "got: {err}");
        assert!(
            err.contains("GET"),
            "the message should list what is allowed: {err}"
        );
    }

    #[test]
    fn no_body_parameters_means_no_body() {
        assert_eq!(
            RequestBody::from_args(&json!({})).expect("no body"),
            RequestBody::None
        );
    }

    /// Sending one of them silently would produce a request the caller never
    /// described, and it would look like it worked.
    #[test]
    fn two_body_parameters_are_refused_rather_than_ranked() {
        let err = RequestBody::from_args(&json!({"body": "x", "json": {"a": 1}}))
            .expect_err("must be refused");
        assert!(err.contains("body"), "got: {err}");
        assert!(err.contains("json"), "got: {err}");
    }

    #[test]
    fn json_bodies_are_serialised_whole() {
        let body = RequestBody::from_args(&json!({"json": {"a": 1}})).expect("json body");
        assert_eq!(body, RequestBody::Json("{\"a\":1}".to_string()));
    }

    /// `{"page": 2}` plainly means the string "2" on the wire.
    #[test]
    fn form_scalars_are_rendered_the_way_a_caller_means_them() {
        let body = RequestBody::from_args(&json!({"form": {"page": 2, "draft": true}}))
            .expect("form body");
        assert_eq!(
            body,
            RequestBody::Form(vec![
                ("draft".to_string(), "true".to_string()),
                ("page".to_string(), "2".to_string()),
            ])
        );
    }

    /// A nested object has no single form encoding, and guessing one would
    /// send `{"a":1}` as a field value without saying so.
    #[test]
    fn a_nested_form_field_is_refused_rather_than_guessed() {
        let err = RequestBody::from_args(&json!({"form": {"filter": {"a": 1}}}))
            .expect_err("must be refused");
        assert!(err.contains("filter"), "got: {err}");
    }

    #[test]
    fn the_timeout_is_clamped_at_both_ends() {
        assert_eq!(timeout_of(&json!({})), Duration::from_secs(30));
        assert_eq!(
            timeout_of(&json!({"timeout_secs": 0})),
            Duration::from_secs(1)
        );
        assert_eq!(
            timeout_of(&json!({"timeout_secs": 100_000})),
            Duration::from_secs(300)
        );
    }

    /// Transport noise costs context and changes nothing the caller does.
    #[test]
    fn only_the_headers_that_change_what_happens_next_are_shown() {
        let all = vec![
            (
                "date".to_string(),
                "Sun, 10 Aug 2026 00:00:00 GMT".to_string(),
            ),
            ("server".to_string(), "nginx".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
            ("retry-after".to_string(), "30".to_string()),
        ];
        let picked = interesting_headers(&all);
        let names: Vec<&str> = picked.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["content-type", "retry-after"]);
    }

    /// A 404 body is the answer, not an error, and the rendering has to carry
    /// the status so the model can tell.
    #[test]
    fn a_failing_status_is_rendered_with_its_body() {
        let out = render_response(
            404,
            "Not Found",
            &[("content-type".to_string(), "application/json".to_string())],
            "{\"message\":\"No such repository\"}",
            33,
            false,
            50_000,
        );
        assert!(out.starts_with("HTTP 404 Not Found\n"), "got: {out}");
        assert!(out.contains("No such repository"), "got: {out}");
    }

    /// The two truncations mean different things: one says "ask for more",
    /// the other says "the rest never arrived".
    #[test]
    fn the_two_truncation_notes_say_different_things() {
        let shown_short = render_response(200, "OK", &[], "abcdefghij", 10, false, 4);
        assert!(
            shown_short.contains("shown: 4 of 10 characters"),
            "{shown_short}"
        );
        assert!(!shown_short.contains("read cap"), "{shown_short}");

        let hit_cap = render_response(200, "OK", &[], "abcd", 4, true, 50_000);
        assert!(hit_cap.contains("read cap"), "{hit_cap}");
        assert!(!hit_cap.contains("raise max_chars"), "{hit_cap}");
    }

    #[test]
    fn an_empty_body_says_so_rather_than_showing_nothing() {
        let out = render_response(204, "No Content", &[], "", 0, false, 50_000);
        assert!(out.contains("(empty body)"), "got: {out}");
    }

    /// Through the real entry point, not the helpers: a tool that can choose
    /// its own method and body is a better SSRF primitive than web_fetch, so
    /// the gate has to be on the path that actually runs.
    #[tokio::test]
    async fn a_loopback_target_is_refused_before_the_request() {
        let args = json!({"url": "http://127.0.0.1:9/admin", "method": "DELETE"});
        let err = exec_http_request_async(&args, Path::new("."))
            .await
            .expect_err("loopback must be refused");
        // Port 9 is discard: if the SSRF gate were missing, this would fail
        // with a connection error instead, so the assertion distinguishes
        // "refused" from "tried and could not connect".
        assert!(
            err.to_string().to_lowercase().contains("block")
                || err.to_string().to_lowercase().contains("private")
                || err.to_string().to_lowercase().contains("loopback"),
            "expected an SSRF refusal, got: {err}"
        );
    }

    /// `file://` and friends are not this tool's business, and reqwest would
    /// give a less clear error than saying so directly.
    #[tokio::test]
    async fn a_non_http_scheme_is_refused_by_name() {
        let args = json!({"url": "file:///etc/passwd"});
        let err = exec_http_request_async(&args, Path::new("."))
            .await
            .expect_err("file:// must be refused");
        assert!(err.to_string().contains("http://"), "got: {err}");
    }

    /// The url parameter is the one thing with no sensible default.
    #[tokio::test]
    async fn a_missing_url_says_which_parameter_is_missing() {
        let err = exec_http_request_async(&json!({"method": "POST"}), Path::new("."))
            .await
            .expect_err("must be refused");
        assert!(err.to_string().contains("url"), "got: {err}");
    }
}
