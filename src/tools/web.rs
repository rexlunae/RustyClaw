//! Web tools: web_fetch and web_search.

use super::helpers::{config, vault};
use crate::safety::SafetyLayer;
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

    // Validate URL scheme
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // Safety layer: URL validation before making request
    if let Some(config_ref) = config() {
        let config_guard = config_ref.blocking_lock();
        if config_guard.ssrf.enabled {
            let safety = SafetyLayer::new(
                config_guard.ssrf.allow_private_ips,
                &config_guard.ssrf.blocked_cidrs,
                "warn",
                0.7,
            );
            if let Err(e) = safety.validate_url(url) {
                return Err(format!("Safety URL validation failed: {}", e));
            }
        }
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
    #[cfg(feature = "web-tools")]
    {
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

    // Without web-tools, return raw HTML body (no extraction)
    #[cfg(not(feature = "web-tools"))]
    {
        let _ = extract_mode; // suppress unused warning
        let mut result = body;
        if result.len() > max_chars {
            result.truncate(max_chars);
            result.push_str("\n\n[truncated]");
        }
        Ok(result)
    }
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
#[cfg(feature = "web-tools")]
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
#[cfg(feature = "web-tools")]
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

#[derive(Debug, Clone)]
struct SearchResult {
    title: String,
    url: String,
    description: String,
}

trait SearchProvider {
    fn name(&self) -> &'static str;
    fn search(
        &self,
        query: &str,
        count: usize,
        country: &str,
        search_lang: Option<&str>,
        freshness: Option<&str>,
    ) -> Result<Vec<SearchResult>, String>;
}

struct BraveSearchProvider {
    api_key: String,
}

impl SearchProvider for BraveSearchProvider {
    fn name(&self) -> &'static str {
        "brave"
    }

    fn search(
        &self,
        query: &str,
        count: usize,
        country: &str,
        search_lang: Option<&str>,
        freshness: Option<&str>,
    ) -> Result<Vec<SearchResult>, String> {
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
            .header("X-Subscription-Token", &self.api_key)
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
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        let mut out = Vec::new();
        for result in web_results.into_iter().take(count) {
            out.push(SearchResult {
                title: result
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(no title)")
                    .to_string(),
                url: result
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                description: result
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }

        Ok(out)
    }
}

struct DuckDuckGoSearchProvider;

impl DuckDuckGoSearchProvider {
    fn extract_topic_results(value: &Value, out: &mut Vec<SearchResult>) {
        let Some(items) = value.as_array() else {
            return;
        };

        for item in items {
            if let Some(nested) = item.get("Topics") {
                Self::extract_topic_results(nested, out);
                continue;
            }

            let text = item
                .get("Text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let url = item
                .get("FirstURL")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if text.is_empty() || url.is_empty() {
                continue;
            }

            let title = text
                .split(" - ")
                .next()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("(no title)")
                .to_string();

            out.push(SearchResult {
                title,
                url: url.to_string(),
                description: text.to_string(),
            });
        }
    }
}

impl SearchProvider for DuckDuckGoSearchProvider {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    fn search(
        &self,
        query: &str,
        count: usize,
        country: &str,
        search_lang: Option<&str>,
        freshness: Option<&str>,
    ) -> Result<Vec<SearchResult>, String> {
        // DuckDuckGo Instant Answer API has limited filtering controls.
        let _ = freshness;
        let kl = if let Some(lang) = search_lang {
            format!("{}-{}", country.to_lowercase(), lang.to_lowercase())
        } else {
            format!("{}-en", country.to_lowercase())
        };

        let url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&no_redirect=1&skip_disambig=0&kl={}",
            urlencoding::encode(query),
            urlencoding::encode(&kl)
        );

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("RustyClaw/0.1 (web_search tool)")
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        let response = client
            .get(&url)
            .send()
            .map_err(|e| format!("DuckDuckGo search request failed: {}", e))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().unwrap_or_default();
            return Err(format!("DuckDuckGo API error {}: {}", status, body));
        }

        let data: Value = response
            .json()
            .map_err(|e| format!("Failed to parse DuckDuckGo response: {}", e))?;

        let mut out = Vec::new();

        let heading = data
            .get("Heading")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let abstract_text = data
            .get("AbstractText")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let abstract_url = data
            .get("AbstractURL")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if !heading.is_empty() && !abstract_url.is_empty() {
            out.push(SearchResult {
                title: heading.to_string(),
                url: abstract_url.to_string(),
                description: abstract_text.to_string(),
            });
        }

        if let Some(results) = data.get("Results").and_then(|v| v.as_array()) {
            for item in results {
                let text = item
                    .get("Text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let url = item
                    .get("FirstURL")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if text.is_empty() || url.is_empty() {
                    continue;
                }
                let title = text
                    .split(" - ")
                    .next()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(no title)")
                    .to_string();
                out.push(SearchResult {
                    title,
                    url: url.to_string(),
                    description: text.to_string(),
                });
            }
        }

        if let Some(topics) = data.get("RelatedTopics") {
            Self::extract_topic_results(topics, &mut out);
        }

        out.truncate(count);
        Ok(out)
    }
}

fn format_search_results(
    query: &str,
    provider_name: &str,
    results: &[SearchResult],
    note: Option<&str>,
) -> String {
    if results.is_empty() {
        if let Some(n) = note {
            return format!("{}\n\nNo results found.", n);
        }
        return "No results found.".to_string();
    }

    let mut output = String::new();
    output.push_str(&format!("Search results for: {}\n", query));
    output.push_str(&format!("Provider: {}\n\n", provider_name));
    if let Some(n) = note {
        output.push_str(n);
        output.push_str("\n\n");
    }

    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!("{}. {}\n", i + 1, result.title));
        output.push_str(&format!("   {}\n", result.url));
        if !result.description.is_empty() {
            output.push_str(&format!("   {}\n", result.description));
        }
        output.push('\n');
    }

    output
}

/// Search the web using Brave API with DuckDuckGo fallback.
pub fn exec_web_search(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: query".to_string())?;

    let count = args
        .get("count")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 10) as usize;

    let country = args
        .get("country")
        .and_then(|v| v.as_str())
        .unwrap_or("US");

    let search_lang = args.get("search_lang").and_then(|v| v.as_str());
    let freshness = args.get("freshness").and_then(|v| v.as_str());
    let provider_pref = args
        .get("provider")
        .and_then(|v| v.as_str())
        .map(|v| v.to_lowercase())
        .or_else(|| {
            std::env::var("RUSTYCLAW_WEB_SEARCH_PROVIDER")
                .ok()
                .map(|v| v.to_lowercase())
        })
        .unwrap_or_else(|| "auto".to_string());

    let brave_api_key = std::env::var("BRAVE_API_KEY").ok();

    match provider_pref.as_str() {
        "brave" => {
            let key = brave_api_key.ok_or_else(|| {
                "BRAVE_API_KEY environment variable not set. \
                 Get a free API key at https://brave.com/search/api/"
                    .to_string()
            })?;
            let brave = BraveSearchProvider { api_key: key };
            let results = brave.search(query, count, country, search_lang, freshness)?;
            Ok(format_search_results(query, brave.name(), &results, None))
        }
        "duckduckgo" | "ddg" => {
            let ddg = DuckDuckGoSearchProvider;
            let results = ddg.search(query, count, country, search_lang, freshness)?;
            Ok(format_search_results(query, ddg.name(), &results, None))
        }
        "auto" => {
            if let Some(key) = brave_api_key {
                let brave = BraveSearchProvider { api_key: key };
                match brave.search(query, count, country, search_lang, freshness) {
                    Ok(results) => Ok(format_search_results(query, brave.name(), &results, None)),
                    Err(err) => {
                        let ddg = DuckDuckGoSearchProvider;
                        let results = ddg.search(query, count, country, search_lang, freshness)?;
                        let note = format!(
                            "Brave search failed ({}). Automatically fell back to DuckDuckGo.",
                            err
                        );
                        Ok(format_search_results(
                            query,
                            ddg.name(),
                            &results,
                            Some(&note),
                        ))
                    }
                }
            } else {
                let ddg = DuckDuckGoSearchProvider;
                let results = ddg.search(query, count, country, search_lang, freshness)?;
                Ok(format_search_results(query, ddg.name(), &results, None))
            }
        }
        other => Err(format!(
            "Invalid provider '{}'. Valid values: auto, brave, duckduckgo",
            other
        )),
    }
}
