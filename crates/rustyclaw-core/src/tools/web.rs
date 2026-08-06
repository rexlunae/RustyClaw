//! Web tools: web_fetch and web_search.
//!
//! Provides async HTTP operations using reqwest.

use super::helpers::vault;
use crate::ignore::Ignore;
use crate::security::SsrfValidator;
use crate::tools::error::{ToolError, ToolResult};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, instrument, warn};

/// Build a reqwest redirect policy that re-validates **every** hop against the
/// SSRF validator and caps the chain length.
///
/// `reqwest` resolves and follows redirects internally, so validating only the
/// initial URL is insufficient — a public URL can 30x-redirect to
/// `http://169.254.169.254/` or an internal address. This policy blocks
/// private, loopback, link-local, and cloud-metadata targets on each hop.
///
/// Note: the validator performs its own DNS resolution; the kernel resolves
/// again when connecting, so a determined DNS-rebinding attacker could still
/// race the two lookups. The validator's double-resolution check narrows that
/// window but does not fully close it (documented limitation).
fn ssrf_redirect_policy(max: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= max {
            return attempt.error(format!("too many redirects (>{max})"));
        }
        let url = attempt.url().clone();
        match SsrfValidator::default().validate_url(url.as_str()) {
            Ok(()) => attempt.follow(),
            Err(e) => attempt.error(format!("SSRF: blocked redirect to {url}: {e}")),
        }
    })
}

/// Validate a URL against the SSRF validator (blocking DNS resolution).
///
/// Propagates the typed [`SsrfError`] so callers can distinguish a security
/// `Blocked` verdict from a transient resolution failure; the rendered
/// message is unchanged.
fn ssrf_check_blocking(url: &str) -> ToolResult<()> {
    Ok(SsrfValidator::default().validate_url(url)?)
}

/// How long a `web_fetch` request is allowed to take.
///
/// The two modes need genuinely different answers, and the difference is easy
/// to collapse back into one `.timeout(..)` by accident — which is what makes
/// this an enum with a test rather than an `if` inside the builder chain.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Deadline {
    /// A whole request, body included, must finish inside this.
    Total(Duration),
    /// No cap on the total. Connecting is bounded, and each read is bounded
    /// separately — the clock resets on every successful read, so a stalled
    /// connection is still caught while a long transfer is not punished for
    /// being long.
    Streaming { connect: Duration, read: Duration },
}

impl Deadline {
    /// Reading a page is a single short burst; a download is not.
    pub(crate) fn for_fetch(is_download: bool) -> Self {
        if is_download {
            Deadline::Streaming {
                connect: Duration::from_secs(30),
                read: Duration::from_secs(60),
            }
        } else {
            Deadline::Total(Duration::from_secs(30))
        }
    }

    fn apply(&self, builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        match *self {
            Deadline::Total(d) => builder.timeout(d),
            Deadline::Streaming { connect, read } => {
                builder.connect_timeout(connect).read_timeout(read)
            }
        }
    }
}

// ── Async implementations ───────────────────────────────────────────────────

/// Fetch a URL and extract readable content as markdown or plain text (async).
#[instrument(skip(args, workspace_dir), fields(url))]
pub async fn exec_web_fetch_async(args: &Value, workspace_dir: &Path) -> ToolResult {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: url".to_string())?;

    tracing::Span::current().record("url", url);

    let extract_mode = args
        .get("extract_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("markdown");

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(50_000) as usize;

    let use_cookies = args
        .get("use_cookies")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let authorization = args.get("authorization").and_then(|v| v.as_str());
    let custom_headers = args.get("headers").and_then(|v| v.as_object());
    // Read here rather than at the download branch below: a client's deadline
    // is fixed when it is built, and a transfer needs a different one.
    let to_file = args.get("to_file").and_then(|v| v.as_str());

    debug!(extract_mode, max_chars, use_cookies, "Fetching URL");

    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".into());
    }

    // SSRF protection: block private/loopback/link-local/cloud-metadata targets.
    // validate_url performs blocking DNS resolution, so run it off the async
    // executor. Redirects are re-validated per-hop by the client's redirect
    // policy below.
    {
        let url_owned = url.to_string();
        tokio::task::spawn_blocking(move || ssrf_check_blocking(&url_owned))
            .await
            .map_err(|e| format!("SSRF validation task failed: {}", e))??;
    }

    // Parse URL for domain extraction
    let parsed_url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let domain = parsed_url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let path = parsed_url.path();
    let is_secure = parsed_url.scheme() == "https";

    // Build async HTTP client
    let builder = reqwest::Client::builder()
        .user_agent("RustyClaw/0.1 (web_fetch tool)")
        .redirect(ssrf_redirect_policy(10));
    let client = Deadline::for_fetch(to_file.is_some())
        .apply(builder)
        .build()
        .map_err(|e| ToolError::context("Failed to create HTTP client", e))?;

    // Build request with optional cookies
    let mut request = client.get(url);

    if use_cookies {
        if let Some(cookie_header) = get_cookie_header_async(domain, path, is_secure).await {
            request = request.header("Cookie", cookie_header);
        }
    }

    if let Some(auth) = authorization {
        request = request.header("Authorization", auth);
        warn!(
            url = %url,
            "web_fetch: Authorization header provided — value NOT logged (security sensitive)"
        );
    }

    if let Some(headers) = custom_headers {
        for (key, value) in headers {
            if let Some(val_str) = value.as_str() {
                request = request.header(key.as_str(), val_str);
            }
        }
    }

    let response = request.send().await.map_err(|e| {
        warn!(error = %e, "HTTP request failed");
        format!("HTTP request failed: {}", e)
    })?;

    let status = response.status();
    debug!(status = status.as_u16(), "Received HTTP response");

    // Store Set-Cookie headers before consuming the response
    if use_cookies {
        let set_cookie_headers: Vec<String> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .collect();

        if !set_cookie_headers.is_empty() {
            store_response_cookies_async(domain, &set_cookie_headers).await;
        }
    }

    if !status.is_success() {
        return Err(format!(
            "HTTP {} — {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        )
        .into());
    }

    // ── Download mode ───────────────────────────────────────────────────
    //
    // Placed after the request is built and the status checked, so a
    // download gets the same SSRF validation, redirect policy, cookies and
    // auth headers as a read — a second code path would be a second place
    // for those to be forgotten.
    if let Some(to_file) = to_file {
        return start_download(response, url, to_file, workspace_dir).await;
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = response
        .text()
        .await
        .map_err(|e| ToolError::context("Failed to read response body", e))?;

    // If it's not HTML, return as-is
    if !content_type.contains("html") {
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        return Ok(result);
    }

    // Parse HTML and extract content
    #[cfg(feature = "web-tools")]
    {
        let document = scraper::Html::parse_document(&body);
        let content = extract_readable_content(&document);

        let result = match extract_mode {
            "text" => html_to_text(&content),
            _ => html2md::parse_html(&content),
        };

        let mut result = result
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        while result.contains("\n\n\n") {
            result = result.replace("\n\n\n", "\n\n");
        }

        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }

        if result.trim().is_empty() {
            return Err("Page returned no extractable content".into());
        }

        Ok(result)
    }

    #[cfg(not(feature = "web-tools"))]
    {
        let _ = extract_mode;
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        Ok(result)
    }
}

/// Search the web using Brave Search API (async).
#[instrument(skip(args, _workspace_dir), fields(query))]
pub async fn exec_web_search_async(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    tracing::Span::current().record("query", query);

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let country = args.get("country").and_then(|v| v.as_str()).unwrap_or("US");

    let search_lang = args.get("search_lang").and_then(|v| v.as_str());
    let freshness = args.get("freshness").and_then(|v| v.as_str());

    debug!(count, country, "Searching with Brave API");

    let api_key = std::env::var("BRAVE_API_KEY").map_err(|_| {
        warn!("BRAVE_API_KEY environment variable not set");
        "BRAVE_API_KEY environment variable not set. \
         Get a free API key at https://brave.com/search/api/"
            .to_string()
    })?;

    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count,
    );

    if country != "ALL" {
        url.push_str(&format!("&country={}", country));
    }
    if let Some(lang) = search_lang {
        url.push_str(&format!("&search_lang={}", lang));
    }
    if let Some(fresh) = freshness {
        url.push_str(&format!("&freshness={}", fresh));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ToolError::context("Failed to create HTTP client", e))?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", &api_key)
        .send()
        .await
        .map_err(|e| ToolError::context("Brave Search request failed", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        warn!(status = status.as_u16(), "Brave Search API error");
        return Err(format!("Brave Search API error {}: {}", status.as_u16(), body).into());
    }

    let data: Value = response
        .json()
        .await
        .map_err(|e| ToolError::context("Failed to parse Brave Search response", e))?;

    let web_results = data
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array());

    let Some(results) = web_results else {
        debug!("No results found");
        return Ok("No results found.".to_string());
    };

    if results.is_empty() {
        debug!("Empty results array");
        return Ok("No results found.".to_string());
    }

    debug!(result_count = results.len(), "Search complete");

    let mut output = String::new();
    output.push_str(&format!("Search results for: {}\n\n", query));

    for (i, result) in results.iter().take(count).enumerate() {
        let title = result
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("(no title)");
        let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let description = result
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");

        output.push_str(&format!("{}. {}\n", i + 1, title));
        output.push_str(&format!("   {}\n", url));
        if !description.is_empty() {
            output.push_str(&format!("   {}\n", description));
        }
        output.push('\n');
    }

    Ok(output)
}

// ── Cookie helpers (async) ──────────────────────────────────────────────────

async fn get_cookie_header_async(domain: &str, path: &str, is_secure: bool) -> Option<String> {
    let vault_ref = vault()?;
    let mut vault_guard = vault_ref.lock().await;
    vault_guard
        .cookie_header_for_request(domain, path, is_secure, true)
        .ok()
        .flatten()
}

async fn store_response_cookies_async(domain: &str, headers: &[String]) {
    if let Some(vault_ref) = vault() {
        let mut vault_guard = vault_ref.lock().await;
        vault_guard
            .store_cookies_from_response(domain, headers, true)
            .ignore();
    }
}

// ── Sync wrappers (for ToolDef compatibility) ───────────────────────────────

/// Sync wrapper for web_fetch.
#[instrument(skip(args, _workspace_dir), fields(url))]
pub fn exec_web_fetch(args: &Value, _workspace_dir: &Path) -> ToolResult {
    exec_web_fetch_sync(args, _workspace_dir)
}

/// Sync wrapper for web_search.
#[instrument(skip(args, _workspace_dir), fields(query))]
pub fn exec_web_search(args: &Value, _workspace_dir: &Path) -> ToolResult {
    exec_web_search_sync(args, _workspace_dir)
}

// ── Sync implementations (fallback) ─────────────────────────────────────────

fn exec_web_fetch_sync(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: url".to_string())?;

    let extract_mode = args
        .get("extract_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("markdown");

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(50_000) as usize;

    let use_cookies = args
        .get("use_cookies")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let authorization = args.get("authorization").and_then(|v| v.as_str());
    let custom_headers = args.get("headers").and_then(|v| v.as_object());

    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".into());
    }

    // SSRF protection: block private/loopback/link-local/cloud-metadata targets.
    // Redirects are re-validated per-hop by the client's redirect policy below.
    ssrf_check_blocking(url)?;

    let parsed_url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let domain = parsed_url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let path = parsed_url.path();
    let is_secure = parsed_url.scheme() == "https";

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("RustyClaw/0.1 (web_fetch tool)")
        .redirect(ssrf_redirect_policy(10))
        .build()
        .map_err(|e| ToolError::context("Failed to create HTTP client", e))?;

    let mut request = client.get(url);

    if use_cookies {
        if let Some(cookie_header) = get_cookie_header_sync(domain, path, is_secure) {
            request = request.header("Cookie", cookie_header);
        }
    }

    if let Some(auth) = authorization {
        request = request.header("Authorization", auth);
        warn!(
            url = %url,
            "web_fetch: Authorization header provided — value NOT logged (security sensitive)"
        );
    }

    if let Some(headers) = custom_headers {
        for (key, value) in headers {
            if let Some(val_str) = value.as_str() {
                request = request.header(key.as_str(), val_str);
            }
        }
    }

    let response = request
        .send()
        .map_err(|e| ToolError::context("HTTP request failed", e))?;

    let status = response.status();

    if use_cookies {
        let set_cookie_headers: Vec<String> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .collect();

        if !set_cookie_headers.is_empty() {
            store_response_cookies_sync(domain, &set_cookie_headers);
        }
    }

    if !status.is_success() {
        return Err(format!(
            "HTTP {} — {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Unknown")
        )
        .into());
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = response
        .text()
        .map_err(|e| ToolError::context("Failed to read response body", e))?;

    if !content_type.contains("html") {
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        return Ok(result);
    }

    #[cfg(feature = "web-tools")]
    {
        let document = scraper::Html::parse_document(&body);
        let content = extract_readable_content(&document);

        let result = match extract_mode {
            "text" => html_to_text(&content),
            _ => html2md::parse_html(&content),
        };

        let mut result = result
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        while result.contains("\n\n\n") {
            result = result.replace("\n\n\n", "\n\n");
        }

        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }

        if result.trim().is_empty() {
            return Err("Page returned no extractable content".into());
        }

        Ok(result)
    }

    #[cfg(not(feature = "web-tools"))]
    {
        let _ = extract_mode;
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        Ok(result)
    }
}

fn exec_web_search_sync(args: &Value, _workspace_dir: &Path) -> ToolResult {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let country = args.get("country").and_then(|v| v.as_str()).unwrap_or("US");

    let search_lang = args.get("search_lang").and_then(|v| v.as_str());
    let freshness = args.get("freshness").and_then(|v| v.as_str());

    let api_key = std::env::var("BRAVE_API_KEY").map_err(|_| {
        "BRAVE_API_KEY environment variable not set. \
         Get a free API key at https://brave.com/search/api/"
            .to_string()
    })?;

    let mut url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count,
    );

    if country != "ALL" {
        url.push_str(&format!("&country={}", country));
    }
    if let Some(lang) = search_lang {
        url.push_str(&format!("&search_lang={}", lang));
    }
    if let Some(fresh) = freshness {
        url.push_str(&format!("&freshness={}", fresh));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ToolError::context("Failed to create HTTP client", e))?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", &api_key)
        .send()
        .map_err(|e| ToolError::context("Brave Search request failed", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("Brave Search API error {}: {}", status.as_u16(), body).into());
    }

    let data: Value = response
        .json()
        .map_err(|e| ToolError::context("Failed to parse Brave Search response", e))?;

    let web_results = data
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array());

    let Some(results) = web_results else {
        return Ok("No results found.".to_string());
    };

    if results.is_empty() {
        return Ok("No results found.".to_string());
    }

    let mut output = String::new();
    output.push_str(&format!("Search results for: {}\n\n", query));

    for (i, result) in results.iter().take(count).enumerate() {
        let title = result
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("(no title)");
        let url = result.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let description = result
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");

        output.push_str(&format!("{}. {}\n", i + 1, title));
        output.push_str(&format!("   {}\n", url));
        if !description.is_empty() {
            output.push_str(&format!("   {}\n", description));
        }
        output.push('\n');
    }

    Ok(output)
}

// ── Cookie helpers (sync) ───────────────────────────────────────────────────
//
// These are called from blocking HTTP code but may run inside a tokio runtime.
// We use `block_in_place` to safely allow blocking in the current thread.

fn get_cookie_header_sync(domain: &str, path: &str, is_secure: bool) -> Option<String> {
    let vault_ref = vault()?;
    tokio::task::block_in_place(|| {
        let mut vault_guard = vault_ref.blocking_lock();
        vault_guard
            .cookie_header_for_request(domain, path, is_secure, true)
            .ok()
            .flatten()
    })
}

fn store_response_cookies_sync(domain: &str, headers: &[String]) {
    if let Some(vault_ref) = vault() {
        tokio::task::block_in_place(|| {
            let mut vault_guard = vault_ref.blocking_lock();
            vault_guard
                .store_cookies_from_response(domain, headers, true)
                .ignore();
        });
    }
}

// ── HTML extraction helpers ─────────────────────────────────────────────────

#[cfg(feature = "web-tools")]
fn extract_readable_content(document: &scraper::Html) -> String {
    use scraper::Selector;

    let content_selectors = [
        "article",
        "main",
        "[role=\"main\"]",
        ".post-content",
        ".article-content",
        ".entry-content",
        ".content",
        "#content",
        ".post",
        ".article",
    ];

    for selector_str in content_selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = document.select(&selector).next() {
                return element.html();
            }
        }
    }

    if let Ok(body_selector) = Selector::parse("body") {
        if let Some(body) = document.select(&body_selector).next() {
            return body.html();
        }
    }

    document.html()
}

#[cfg(feature = "web-tools")]
fn html_to_text(html: &str) -> String {
    use scraper::{Html, Selector};

    let document = Html::parse_fragment(html);
    let mut text = String::new();

    fn extract_text(node: scraper::ElementRef, text: &mut String) {
        for child in node.children() {
            if let Some(element) = scraper::ElementRef::wrap(child) {
                let tag = element.value().name();
                if matches!(
                    tag,
                    "script" | "style" | "nav" | "header" | "footer" | "aside" | "noscript"
                ) {
                    continue;
                }
                if matches!(
                    tag,
                    "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr"
                ) {
                    text.push('\n');
                }
                extract_text(element, text);
                if matches!(tag, "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                    text.push('\n');
                }
            } else if let Some(text_node) = child.value().as_text() {
                text.push_str(text_node.trim());
                text.push(' ');
            }
        }
    }

    if let Ok(selector) = Selector::parse("body") {
        if let Some(body) = document.select(&selector).next() {
            extract_text(body, &mut text);
        }
    }

    if text.is_empty() {
        for element in document.root_element().children() {
            if let Some(el) = scraper::ElementRef::wrap(element) {
                extract_text(el, &mut text);
            }
        }
    }

    text
}

// ── Downloads ───────────────────────────────────────────────────────────────

/// How much must arrive before progress is announced again.
///
/// Every chunk would be a broadcast per few kilobytes, which is a lot of
/// traffic to move a progress bar the user cannot see move that finely. The
/// terminal event is always announced regardless of this.
const PROGRESS_STEP_BYTES: u64 = 256 * 1024;

/// The most a single transfer may write.
///
/// Every other write the model can perform is bounded by something: a
/// `write_file` is capped by what the model can emit, and a non-download
/// `web_fetch` by `max_chars`. A download is bounded by nothing — it is
/// detached from the turn on purpose, so no turn ends it, and
/// `Deadline::Streaming` only bounds a stall rather than a body that keeps
/// arriving. A URL that streams indefinitely would write until the filesystem
/// filled, taking the gateway host down with it.
///
/// Deliberately generous: this is a backstop against an unbounded stream, not
/// a policy about how large a file the user may fetch. Exceeding it fails the
/// transfer like any other error — the partial file stays on disk and the
/// reason reaches both the panel and the agent.
pub(crate) const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Whether `bytes` is past the cap.
///
/// A function rather than a bare `>` at each site so the up-front refusal and
/// the streaming check cannot disagree about which side of the boundary is
/// allowed — exactly `MAX_DOWNLOAD_BYTES` is fine, one more is not.
pub(crate) fn exceeds_download_cap(bytes: u64) -> bool {
    bytes > MAX_DOWNLOAD_BYTES
}

/// The message used when a transfer is refused or stopped for its size, so
/// the up-front check and the streaming check cannot drift apart.
pub(crate) fn too_large(bytes: u64) -> String {
    format!(
        "exceeds the {} maximum download size (needed {})",
        crate::downloads::human_bytes(MAX_DOWNLOAD_BYTES),
        crate::downloads::human_bytes(bytes),
    )
}

/// Stream a response to a file, returning as soon as the transfer is
/// registered rather than when it finishes.
///
/// A large file can take minutes. Holding the tool call open for it would
/// block the turn, keep the model waiting on bytes it has no use for, and
/// make the whole thing invisible until it ended. Instead the transfer
/// becomes a registry entry the panel can watch, and the agent is told about
/// it again when it completes.
/// Resolve and open a download's destination under the same guards
/// `write_file` applies, returning the open handle and its canonical path.
///
/// Separated from [`start_download`] so the guards can be tested without a
/// live `reqwest::Response` — the protected-path rejection in particular has
/// no other way to be exercised, and it is the one that matters most.
pub(crate) async fn open_download_dest(
    workspace_dir: &Path,
    to_file: &str,
) -> Result<(std::fs::File, std::path::PathBuf), ToolError> {
    // Same resolution and the same guards as `write_file`: the destination
    // comes from the model, so it gets the workspace boundary, the protected
    // path list and the symlink-race protection rather than a bare open.
    let dest = crate::tools::helpers::resolve_path(workspace_dir, to_file);

    // Before anything is created or opened. `open_file_write_safe` does the
    // symlink-race half only — it has no idea what the credentials directory
    // is, so without this a download could truncate a vault file that
    // `write_file` refuses to touch.
    if crate::tools::helpers::is_protected_path(&dest) {
        warn!(path = %dest.display(), "Attempted download to protected path");
        return Err(crate::tools::helpers::VAULT_ACCESS_DENIED.into());
    }

    // `write_file` promises the destination's folders are created for it and
    // does so here; a download aimed at a new subfolder would otherwise fail
    // at the open with a bare ENOENT.
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            format!(
                "Failed to create directories for '{}': {}",
                dest.display(),
                e
            )
        })?;
    }

    tokio::task::spawn_blocking(move || crate::tools::helpers::open_file_write_safe(&dest))
        .await
        .map_err(|e| format!("Failed to open destination: {}", e))?
        .map_err(|e| format!("Failed to open destination: {}", e).into())
}

async fn start_download(
    response: reqwest::Response,
    url: &str,
    to_file: &str,
    workspace_dir: &Path,
) -> ToolResult {
    use crate::downloads::{DownloadStatus, announce, lock_registry};

    let total_bytes = response.content_length();

    // Refused before the destination is created, when the server declares a
    // size over the cap. Truncating an existing file and *then* failing would
    // destroy it for nothing.
    if let Some(declared) = total_bytes
        && exceeds_download_cap(declared)
    {
        return Err(format!("Refusing to download {url}: it {}", too_large(declared)).into());
    }

    let (file, canonical) = open_download_dest(workspace_dir, to_file).await?;
    // Taken before the streaming task below takes ownership of the path.
    let dest_display = canonical.display().to_string();

    let download = lock_registry().register(
        url.to_string(),
        canonical.clone(),
        total_bytes,
        // Captured here, in the turn's task. The streaming task below is
        // spawned and would not inherit it — which is the whole reason
        // the tag is taken at registration rather than at completion.
        crate::downloads::current_origin(),
    );
    announce(download.clone());

    let id = download.id.clone();
    let id_for_task = id.clone();
    let url_for_task = url.to_string();
    let mut response = response;
    let mut file = tokio::fs::File::from_std(file);

    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;

        let mut received: u64 = 0;
        let mut announced_at: u64 = 0;
        // `Ok(())` means the body ended cleanly; anything else names what
        // stopped it, and that reason reaches both the panel and the agent.
        let outcome: Result<(), String> = loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = file.write_all(&chunk).await {
                        break Err(format!("writing to {}: {e}", canonical.display()));
                    }
                    received += chunk.len() as u64;
                    // The declared length is a claim, and a chunked response
                    // makes no claim at all, so the cap is enforced against
                    // what has actually arrived.
                    if exceeds_download_cap(received) {
                        break Err(format!("{url_for_task} {}", too_large(received)));
                    }
                    // Recorded on every chunk, announced only every
                    // `PROGRESS_STEP_BYTES`. The two are separate because
                    // they answer different questions: the record is how a
                    // cancel from the panel reaches this loop — `advance`
                    // refuses once the transfer is terminal — and checking
                    // that only per announcement would leave a slow download
                    // writing for minutes after the user stopped it. The
                    // announcement is what redraws a progress bar, and one
                    // per 8 KB chunk is a redraw storm.
                    let updated = lock_registry().advance(&id_for_task, received);
                    // `None` means it ended underneath us — cancelled from
                    // the panel — so stop writing rather than finish a file
                    // nobody wants. Its ending has already been announced.
                    // It cannot also mean "the registry was unreadable":
                    // `lock_registry` recovers a poisoned lock rather than
                    // failing, precisely so this test keeps one meaning.
                    //
                    // Flushed on the way out even though nothing is waiting
                    // for the file: a `tokio::fs::File` dispatches writes to a
                    // blocking pool, and one dropped without a flush may
                    // discard what is still buffered and swallow the error.
                    // The partial file is deliberately left on disk for the
                    // user, so it should hold the bytes the panel last said
                    // had arrived rather than silently fewer.
                    let Some(updated) = updated else {
                        file.flush().await.ignore();
                        return;
                    };
                    if received - announced_at >= PROGRESS_STEP_BYTES {
                        announced_at = received;
                        announce(updated);
                    }
                }
                Ok(None) => break Ok(()),
                Err(e) => break Err(format!("reading from {url_for_task}: {e}")),
            }
        };

        let status = match (outcome, file.flush().await) {
            (Ok(()), Ok(())) => DownloadStatus::Complete,
            (Ok(()), Err(e)) => DownloadStatus::Failed {
                error: format!("flushing {}: {e}", canonical.display()),
            },
            (Err(e), _) => DownloadStatus::Failed { error: e },
        };

        let finished = {
            let mut m = lock_registry();
            m.advance(&id_for_task, received);
            m.finish(&id_for_task, status)
        };
        // Absent means something else ended it first, and that ending has
        // already been announced. Announcing again would wake the agent
        // twice for one file.
        if let Some(d) = finished {
            announce(d);
        }
    });

    Ok(format!(
        "Download started: {id}\n\
         url: {url}\n\
         destination: {}\n\
         size: {}\n\n\
         It is running in the background — you will be told when it finishes, \
         and the user can watch it in the downloads panel. Do not poll for it.",
        dest_display,
        match total_bytes {
            Some(n) => format!("{n} bytes"),
            None => "unknown (server sent no length)".to_string(),
        }
    ))
}
