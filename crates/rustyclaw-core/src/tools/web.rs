//! Web tools: web_fetch and web_search.
//!
//! Provides async HTTP operations using reqwest.

use super::helpers::vault;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tracing::{debug, instrument, warn};

// ── Async implementations ───────────────────────────────────────────────────

/// Fetch a URL and extract readable content as markdown or plain text (async).
#[instrument(skip(args, _workspace_dir), fields(url))]
pub async fn exec_web_fetch_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
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

    debug!(extract_mode, max_chars, use_cookies, "Fetching URL");

    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // Parse URL for domain extraction
    let parsed_url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let domain = parsed_url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let path = parsed_url.path();
    let is_secure = parsed_url.scheme() == "https";

    // Build async HTTP client
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("RustyClaw/0.1 (web_fetch tool)")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Build request with optional cookies
    let mut request = client.get(url);

    if use_cookies {
        if let Some(cookie_header) = get_cookie_header_async(domain, path, is_secure).await {
            request = request.header("Cookie", cookie_header);
        }
    }

    if let Some(auth) = authorization {
        request = request.header("Authorization", auth);
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
        ));
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
        .map_err(|e| format!("Failed to read response body: {}", e))?;

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
            return Err("Page returned no extractable content".to_string());
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
pub async fn exec_web_search_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    tracing::Span::current().record("query", query);

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(10)
        .max(1) as usize;

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
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", &api_key)
        .send()
        .await
        .map_err(|e| format!("Brave Search request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        warn!(status = status.as_u16(), "Brave Search API error");
        return Err(format!(
            "Brave Search API error {}: {}",
            status.as_u16(),
            body
        ));
    }

    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Brave Search response: {}", e))?;

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
        let _ = vault_guard.store_cookies_from_response(domain, headers, true);
    }
}

// ── Sync wrappers (for ToolDef compatibility) ───────────────────────────────

/// Sync wrapper for web_fetch.
#[instrument(skip(args, _workspace_dir), fields(url))]
pub fn exec_web_fetch(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    exec_web_fetch_sync(args, _workspace_dir)
}

/// Sync wrapper for web_search.
#[instrument(skip(args, _workspace_dir), fields(query))]
pub fn exec_web_search(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    exec_web_search_sync(args, _workspace_dir)
}

// ── Sync implementations (fallback) ─────────────────────────────────────────

fn exec_web_fetch_sync(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
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
        return Err("URL must start with http:// or https://".to_string());
    }

    let parsed_url = url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let domain = parsed_url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let path = parsed_url.path();
    let is_secure = parsed_url.scheme() == "https";

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("RustyClaw/0.1 (web_fetch tool)")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let mut request = client.get(url);

    if use_cookies {
        if let Some(cookie_header) = get_cookie_header_sync(domain, path, is_secure) {
            request = request.header("Cookie", cookie_header);
        }
    }

    if let Some(auth) = authorization {
        request = request.header("Authorization", auth);
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
        .map_err(|e| format!("HTTP request failed: {}", e))?;

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
        ));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let body = response
        .text()
        .map_err(|e| format!("Failed to read response body: {}", e))?;

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
            return Err("Page returned no extractable content".to_string());
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

fn exec_web_search_sync(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .min(10)
        .max(1) as usize;

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
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", &api_key)
        .send()
        .map_err(|e| format!("Brave Search request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!(
            "Brave Search API error {}: {}",
            status.as_u16(),
            body
        ));
    }

    let data: Value = response
        .json()
        .map_err(|e| format!("Failed to parse Brave Search response: {}", e))?;

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

fn get_cookie_header_sync(domain: &str, path: &str, is_secure: bool) -> Option<String> {
    let vault_ref = vault()?;
    let mut vault_guard = vault_ref.blocking_lock();
    vault_guard
        .cookie_header_for_request(domain, path, is_secure, true)
        .ok()
        .flatten()
}

fn store_response_cookies_sync(domain: &str, headers: &[String]) {
    if let Some(vault_ref) = vault() {
        let mut vault_guard = vault_ref.blocking_lock();
        let _ = vault_guard.store_cookies_from_response(domain, headers, true);
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
