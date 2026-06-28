//! Web extract tool: dedicated readability-style content extraction.
//!
//! Provides a focused `web_extract` tool for extracting clean, readable content
//! from web pages. While `web_fetch` already supports extraction via its
//! `extract_mode` parameter, this tool is optimized specifically for content
//! extraction with additional options like selector targeting and metadata.

use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;
use tracing::{debug, instrument};

use super::ToolParam;

// ── Tool executor (async) ───────────────────────────────────────────────────

/// Extract clean, readable content from a URL (async).
#[instrument(skip(args, _workspace_dir), fields(url))]
pub async fn exec_web_extract_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: url".to_string())?;

    tracing::Span::current().record("url", url);

    let selector = args.get("selector").and_then(|v| v.as_str());

    let include_metadata = args
        .get("include_metadata")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_u64())
        .unwrap_or(50_000) as usize;

    let output_format = args
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("markdown");

    debug!(
        ?selector,
        include_metadata, max_chars, output_format, "Extracting content"
    );

    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    // Fetch the page
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("RustyClaw/0.1 (web_extract tool)")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {} for URL: {}", status, url));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let html = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    // Extract metadata if requested
    let metadata = if include_metadata {
        Some(extract_metadata(&html, url))
    } else {
        None
    };

    // Extract content based on selector or full-page readability
    let extracted = if let Some(sel) = selector {
        extract_with_selector(&html, sel, output_format)?
    } else {
        extract_readable(&html, output_format)?
    };

    // Truncate if needed
    let content = if extracted.len() > max_chars {
        let mut truncated = extracted[..max_chars].to_string();
        truncated.push_str("\n\n[... content truncated ...]");
        truncated
    } else {
        extracted
    };

    // Build result
    if let Some(meta) = metadata {
        Ok(json!({
            "url": url,
            "content_type": content_type,
            "metadata": meta,
            "content": content,
            "chars": content.len(),
        })
        .to_string())
    } else {
        Ok(content)
    }
}

/// Sync stub for the static ToolDef.
pub fn exec_web_extract_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Err("web_extract requires async execution".into())
}

// ── Content extraction helpers ──────────────────────────────────────────────

/// Extract readable content from HTML using readability heuristics.
#[cfg(feature = "web-tools")]
fn extract_readable(html: &str, format: &str) -> Result<String, String> {
    // Use scraper to parse and clean the HTML
    let document = scraper::Html::parse_document(html);

    // Remove script, style, nav, footer, header elements
    let body_selector = scraper::Selector::parse("body").unwrap();
    let body = document.select(&body_selector).next();

    let clean_html = if let Some(body_el) = body {
        let mut text_parts: Vec<String> = Vec::new();
        collect_text_content(&body_el, &mut text_parts);
        text_parts.join("\n")
    } else {
        // Fallback: just strip all tags
        strip_html_tags(html)
    };

    match format {
        "markdown" => Ok(html2md::parse_html(html)),
        "text" => Ok(clean_html),
        _ => Ok(clean_html),
    }
}

/// Extract readable content without web-tools feature (plain text fallback).
#[cfg(not(feature = "web-tools"))]
fn extract_readable(html: &str, _format: &str) -> Result<String, String> {
    Ok(strip_html_tags(html))
}

/// Extract content matching a CSS selector.
#[cfg(feature = "web-tools")]
fn extract_with_selector(html: &str, selector: &str, format: &str) -> Result<String, String> {
    let document = scraper::Html::parse_document(html);
    let sel = scraper::Selector::parse(selector)
        .map_err(|e| format!("Invalid CSS selector '{}': {:?}", selector, e))?;

    let elements: Vec<String> = document
        .select(&sel)
        .map(|el| match format {
            "markdown" => html2md::parse_html(&el.html()),
            _ => el.text().collect::<Vec<_>>().join(" "),
        })
        .collect();

    if elements.is_empty() {
        Err(format!(
            "No elements found matching selector: '{}'",
            selector
        ))
    } else {
        Ok(elements.join("\n\n---\n\n"))
    }
}

#[cfg(not(feature = "web-tools"))]
fn extract_with_selector(_html: &str, _selector: &str, _format: &str) -> Result<String, String> {
    Err("CSS selector extraction requires the 'web-tools' feature".to_string())
}

/// Extract page metadata (title, description, author, etc.).
fn extract_metadata(html: &str, url: &str) -> Value {
    let title = extract_meta_content(html, "<title>", "</title>");
    let description = extract_meta_property(html, "description")
        .or_else(|| extract_meta_property(html, "og:description"));
    let author = extract_meta_property(html, "author");

    json!({
        "url": url,
        "title": title,
        "description": description,
        "author": author,
    })
}

/// Simple extraction of content between tags.
fn extract_meta_content(html: &str, open: &str, close: &str) -> Option<String> {
    let start = html.find(open)?;
    let content_start = start + open.len();
    let end = html[content_start..].find(close)?;
    let content = html[content_start..content_start + end].trim();
    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

/// Extract meta tag content by name or property attribute.
fn extract_meta_property(html: &str, name: &str) -> Option<String> {
    // Look for <meta name="..." content="..."> or <meta property="..." content="...">
    let patterns = [
        format!(r#"name="{}" content=""#, name),
        format!(r#"name='{}' content='"#, name),
        format!(r#"property="{}" content=""#, name),
        format!(r#"property='{}' content='"#, name),
        format!(r#"content="" name="{}""#, name),
    ];

    let lower_html = html.to_lowercase();
    for pattern in &patterns {
        if let Some(pos) = lower_html.find(&pattern.to_lowercase()) {
            let after_pattern = pos + pattern.len();
            let quote_char = if pattern.ends_with('"') { '"' } else { '\'' };
            if let Some(end) = html[after_pattern..].find(quote_char) {
                let content = &html[after_pattern..after_pattern + end];
                if !content.is_empty() {
                    return Some(content.to_string());
                }
            }
        }
    }
    None
}

/// Strip HTML tags and return plain text.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            }
            _ if !in_tag => {
                if c.is_whitespace() {
                    if !last_was_space {
                        result.push(' ');
                        last_was_space = true;
                    }
                } else {
                    result.push(c);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }

    result.trim().to_string()
}

/// Recursively collect text content from an HTML element, skipping
/// script/style/nav/footer elements.
#[cfg(feature = "web-tools")]
fn collect_text_content(element: &scraper::ElementRef, parts: &mut Vec<String>) {
    use scraper::Node;

    let skip_tags = [
        "script", "style", "nav", "footer", "header", "aside", "noscript",
    ];

    for child in element.children() {
        match child.value() {
            Node::Text(text) => {
                let t = text.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
            Node::Element(el) => {
                let tag = el.name();
                if !skip_tags.contains(&tag) {
                    if let Some(child_ref) = scraper::ElementRef::wrap(child) {
                        collect_text_content(&child_ref, parts);
                    }
                }
            }
            _ => {}
        }
    }
}

// ── Parameter definitions ───────────────────────────────────────────────────

pub fn web_extract_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "url".into(),
            description: "URL to extract content from.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "selector".into(),
            description: "Optional CSS selector to target specific elements. \
                          If omitted, extracts the full page using readability heuristics."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "format".into(),
            description: "Output format: 'markdown' (default) or 'text'.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "include_metadata".into(),
            description: "Include page metadata (title, description, author). Default: false."
                .into(),
            param_type: "boolean".into(),
            required: false,
        },
        ToolParam {
            name: "max_chars".into(),
            description: "Maximum characters to return. Default: 50000.".into(),
            param_type: "integer".into(),
            required: false,
        },
    ]
}
