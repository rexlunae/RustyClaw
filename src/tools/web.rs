//! Web tools: web_fetch and web_search.

use super::helpers::vault;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// Fetch a URL and extract readable content as markdown or plain text.
///
/// When `use_cookies` is true, automatically:
/// - Attaches stored cookies matching the request domain
/// - Stores any Set-Cookie headers from the response
pub fn exec_web_fetch(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
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

    // Cookie jar support
    let use_cookies = args
        .get("use_cookies")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // Parse URL for domain extraction
    let parsed_url =
        url::Url::parse(url).map_err(|e| format!("Invalid URL: {}", e))?;
    let domain = parsed_url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let path = parsed_url.path();
    let is_secure = parsed_url.scheme() == "https";

    // Build HTTP client
    let client_builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("RustyClaw/0.1 (web_fetch tool)")
        // Don't follow redirects automatically so we can handle Set-Cookie
        .redirect(reqwest::redirect::Policy::limited(10));

    let client = client_builder
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Build request with optional cookies
    let mut request = client.get(url);

    if use_cookies {
        if let Some(cookie_header) = get_cookie_header(domain, path, is_secure) {
            request = request.header("Cookie", cookie_header);
        }
    }

    let response = request
        .send()
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();

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
            store_response_cookies(domain, &set_cookie_headers);
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

    // If it's not HTML, return as-is (might be JSON, plain text, etc.)
    if !content_type.contains("html") {
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        return Ok(result);
    }

    // Parse HTML and extract content
    let document = scraper::Html::parse_document(&body);

    // Try to find the main content area
    let content = extract_readable_content(&document);

    let result = match extract_mode {
        "text" => {
            // Plain text extraction
            html_to_text(&content)
        }
        _ => {
            // Markdown conversion (default)
            html2md::parse_html(&content)
        }
    };

    // Clean up the result
    let mut result = result
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");

    // Collapse multiple blank lines
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    // Truncate if needed
    if result.len() > max_chars {
        result.truncate(max_chars);
        result.push_str("\n\n[truncated]");
    }

    if result.trim().is_empty() {
        return Err("Page returned no extractable content".to_string());
    }

    Ok(result)
}

/// Get the Cookie header for a request, if cookies are available.
fn get_cookie_header(domain: &str, path: &str, is_secure: bool) -> Option<String> {
    let vault_ref = vault()?;
    let mut vault_guard = vault_ref.blocking_lock();

    // Use agent_access setting — no explicit user approval for cookie reads
    // during web_fetch (the user approved by setting use_cookies=true)
    vault_guard
        .cookie_header_for_request(domain, path, is_secure, true)
        .ok()
        .flatten()
}

/// Store Set-Cookie headers from a response.
fn store_response_cookies(domain: &str, headers: &[String]) {
    if let Some(vault_ref) = vault() {
        let mut vault_guard = vault_ref.blocking_lock();
        // Best effort — don't fail the request if cookie storage fails
        let _ = vault_guard.store_cookies_from_response(domain, headers, true);
    }
}

/// Extract the main readable content from an HTML document.
fn extract_readable_content(document: &scraper::Html) -> String {
    use scraper::Selector;

    // Selectors for main content areas (in priority order)
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

    // Try each content selector
    for selector_str in content_selectors {
        if let Ok(selector) = Selector::parse(selector_str) {
            if let Some(element) = document.select(&selector).next() {
                return element.html();
            }
        }
    }

    // Fall back to body, stripping unwanted elements
    if let Ok(body_selector) = Selector::parse("body") {
        if let Some(body) = document.select(&body_selector).next() {
            return body.html();
        }
    }

    // Last resort: return the whole document
    document.html()
}

/// Convert HTML to plain text, stripping all tags.
fn html_to_text(html: &str) -> String {
    use scraper::{Html, Selector};

    let document = Html::parse_fragment(html);
    let mut text = String::new();

    // Walk the document and extract text nodes
    fn extract_text(node: scraper::ElementRef, text: &mut String) {
        for child in node.children() {
            if let Some(element) = scraper::ElementRef::wrap(child) {
                let tag = element.value().name();
                // Skip script, style, nav, header, footer
                if matches!(
                    tag,
                    "script" | "style" | "nav" | "header" | "footer" | "aside" | "noscript"
                ) {
                    continue;
                }
                // Add newlines for block elements
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

    // If no body found, try the root
    if text.is_empty() {
        for element in document.root_element().children() {
            if let Some(el) = scraper::ElementRef::wrap(element) {
                extract_text(el, &mut text);
            }
        }
    }

    text
}

/// Search the web using Brave Search API.
pub fn exec_web_search(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
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

    let country = args
        .get("country")
        .and_then(|v| v.as_str())
        .unwrap_or("US");

    let search_lang = args.get("search_lang").and_then(|v| v.as_str());
    let freshness = args.get("freshness").and_then(|v| v.as_str());

    // Get API key from environment
    let api_key = std::env::var("BRAVE_API_KEY").map_err(|_| {
        "BRAVE_API_KEY environment variable not set. \
         Get a free API key at https://brave.com/search/api/"
            .to_string()
    })?;

    // Build the request URL
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

    // Make the request
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

    // Extract web results
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

    // Format results
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
