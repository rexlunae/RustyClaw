//! Browser automation tool using chromiumoxide (CDP).
//!
//! This module provides real browser automation when the `browser` feature is enabled.
//! Falls back to stub implementation when disabled.

use serde_json::{json, Value};
use std::path::Path;
use tracing::{debug, warn, instrument};

#[cfg(feature = "browser")]
mod real {
    use super::*;
    use chromiumoxide::{Browser, BrowserConfig, Page};
    use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
    use futures_util::StreamExt;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;
    use std::collections::HashMap;

    /// Global browser instance, lazily initialized.
    static BROWSER: OnceLock<Mutex<Option<BrowserState>>> = OnceLock::new();

    struct BrowserState {
        browser: Browser,
        pages: HashMap<String, Page>,
        #[allow(dead_code)]
        handler_handle: tokio::task::JoinHandle<()>,
    }

    fn browser_state() -> &'static Mutex<Option<BrowserState>> {
        BROWSER.get_or_init(|| Mutex::new(None))
    }

    /// Start the browser if not already running.
    pub async fn ensure_browser() -> Result<(), String> {
        let mut state = browser_state().lock().await;
        if state.is_some() {
            debug!("Browser already running");
            return Ok(());
        }

        debug!("Launching browser");

        // Configure browser
        let config = BrowserConfig::builder()
            .with_head() // Show browser window (use .headless() for headless)
            .viewport(None) // Use default viewport
            .build()
            .map_err(|e| format!("Failed to build browser config: {}", e))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| format!("Failed to launch browser: {}", e))?;

        // Spawn handler task
        let handler_handle = tokio::spawn(async move {
            while let Some(_event) = handler.next().await {
                // Process events (required for browser to function)
            }
        });

        *state = Some(BrowserState {
            browser,
            pages: HashMap::new(),
            handler_handle,
        });

        debug!("Browser launched successfully");
        Ok(())
    }

    /// Get browser status.
    pub async fn status() -> Result<String, String> {
        let state = browser_state().lock().await;
        if let Some(ref s) = *state {
            let tab_count = s.pages.len();
            debug!(tabs = tab_count, "Browser status: running");
            Ok(json!({
                "running": true,
                "tabs": tab_count,
                "profile": "rustyclaw"
            }).to_string())
        } else {
            debug!("Browser status: not running");
            Ok(json!({
                "running": false,
                "tabs": 0,
                "profile": "rustyclaw"
            }).to_string())
        }
    }

    /// Start the browser.
    pub async fn start() -> Result<String, String> {
        ensure_browser().await?;
        Ok("Browser started successfully.".to_string())
    }

    /// Stop the browser.
    pub async fn stop() -> Result<String, String> {
        let mut state = browser_state().lock().await;
        if let Some(s) = state.take() {
            // Close all pages
            for (_id, page) in s.pages {
                let _ = page.close().await;
            }
            // Browser will be dropped, closing the connection
            Ok("Browser stopped.".to_string())
        } else {
            Ok("Browser was not running.".to_string())
        }
    }

    /// List open tabs.
    pub async fn list_tabs() -> Result<String, String> {
        let state = browser_state().lock().await;
        if let Some(ref s) = *state {
            let tabs: Vec<Value> = s.pages.keys().map(|id| {
                json!({
                    "id": id,
                    // Note: chromiumoxide doesn't expose URL easily without async call
                })
            }).collect();
            Ok(json!({ "tabs": tabs }).to_string())
        } else {
            Err("Browser not running. Use action='start' first.".to_string())
        }
    }

    /// Open a new tab with URL.
    pub async fn open_tab(url: &str) -> Result<String, String> {
        ensure_browser().await?;

        let mut state = browser_state().lock().await;
        let s = state.as_mut().ok_or("Browser not initialized")?;

        let page = s.browser.new_page(url)
            .await
            .map_err(|e| format!("Failed to open page: {}", e))?;

        // Generate a tab ID
        let tab_id = format!("tab_{}", s.pages.len());
        s.pages.insert(tab_id.clone(), page);

        Ok(json!({
            "success": true,
            "tabId": tab_id,
            "url": url
        }).to_string())
    }

    /// Navigate current page to URL.
    pub async fn navigate(tab_id: Option<&str>, url: &str) -> Result<String, String> {
        let mut state = browser_state().lock().await;
        let s = state.as_mut().ok_or("Browser not running")?;

        // Get the page (use provided tab_id or first available)
        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        page.goto(url)
            .await
            .map_err(|e| format!("Navigation failed: {}", e))?;

        Ok(json!({
            "success": true,
            "url": url
        }).to_string())
    }

    /// Take a screenshot.
    pub async fn screenshot(tab_id: Option<&str>, full_page: bool) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        let screenshot = if full_page {
            page.screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .full_page(true)
                    .build()
            )
            .await
            .map_err(|e| format!("Screenshot failed: {}", e))?
        } else {
            page.screenshot(
                chromiumoxide::page::ScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Png)
                    .build()
            )
            .await
            .map_err(|e| format!("Screenshot failed: {}", e))?
        };

        // Encode as base64
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let base64_data = STANDARD.encode(&screenshot);

        Ok(json!({
            "success": true,
            "format": "png",
            "data": format!("data:image/png;base64,{}", base64_data)
        }).to_string())
    }

    /// Get page content.
    pub async fn get_content(tab_id: Option<&str>) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        let content = page.content()
            .await
            .map_err(|e| format!("Failed to get content: {}", e))?;

        Ok(content)
    }

    /// Click an element by selector.
    pub async fn click(tab_id: Option<&str>, selector: &str) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        let element = page.find_element(selector)
            .await
            .map_err(|e| format!("Element not found: {}", e))?;

        element.click()
            .await
            .map_err(|e| format!("Click failed: {}", e))?;

        Ok(json!({
            "success": true,
            "action": "click",
            "selector": selector
        }).to_string())
    }

    /// Type text into an element.
    pub async fn type_text(tab_id: Option<&str>, selector: &str, text: &str) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        let element = page.find_element(selector)
            .await
            .map_err(|e| format!("Element not found: {}", e))?;

        element.click()
            .await
            .map_err(|e| format!("Click failed: {}", e))?;

        element.type_str(text)
            .await
            .map_err(|e| format!("Type failed: {}", e))?;

        Ok(json!({
            "success": true,
            "action": "type",
            "selector": selector,
            "text_length": text.len()
        }).to_string())
    }

    /// Press a key.
    pub async fn press_key(tab_id: Option<&str>, key: &str) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        // Use CDP DispatchKeyEventParams for key press
        use chromiumoxide::cdp::browser_protocol::input::{DispatchKeyEventParams, DispatchKeyEventType};

        // Press key down
        let key_down = DispatchKeyEventParams::builder()
            .key(key.to_string())
            .text(key.to_string())
            .r#type(DispatchKeyEventType::KeyDown)
            .build()
            .map_err(|e| format!("Failed to build key down params: {}", e))?;
        page.execute(key_down)
            .await
            .map_err(|e| format!("Key down failed: {}", e))?;

        // Release key
        let key_up = DispatchKeyEventParams::builder()
            .key(key.to_string())
            .text(key.to_string())
            .r#type(DispatchKeyEventType::KeyUp)
            .build()
            .map_err(|e| format!("Failed to build key up params: {}", e))?;
        page.execute(key_up)
            .await
            .map_err(|e| format!("Key up failed: {}", e))?;

        Ok(json!({
            "success": true,
            "action": "press",
            "key": key
        }).to_string())
    }

    /// Evaluate JavaScript.
    pub async fn evaluate(tab_id: Option<&str>, script: &str) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        let result: Value = page.evaluate(script)
            .await
            .map_err(|e| format!("Evaluate failed: {}", e))?
            .into_value()
            .map_err(|e| format!("Failed to convert result: {}", e))?;

        Ok(result.to_string())
    }

    /// Close a tab.
    pub async fn close_tab(tab_id: &str) -> Result<String, String> {
        let mut state = browser_state().lock().await;
        let s = state.as_mut().ok_or("Browser not running")?;

        if let Some(page) = s.pages.remove(tab_id) {
            page.close()
                .await
                .map_err(|e| format!("Failed to close tab: {}", e))?;
            Ok(json!({
                "success": true,
                "closed": tab_id
            }).to_string())
        } else {
            Err(format!("Tab not found: {}", tab_id))
        }
    }

    /// Get accessibility snapshot (simplified).
    pub async fn snapshot(tab_id: Option<&str>) -> Result<String, String> {
        let state = browser_state().lock().await;
        let s = state.as_ref().ok_or("Browser not running")?;

        let page = if let Some(id) = tab_id {
            s.pages.get(id).ok_or_else(|| format!("Tab not found: {}", id))?
        } else {
            s.pages.values().next().ok_or("No tabs open")?
        };

        // Get basic page info since full a11y tree is complex
        let title: String = page.evaluate("document.title")
            .await
            .map_err(|e| format!("Failed to get title: {}", e))?
            .into_value()
            .unwrap_or_default();

        let url: String = page.evaluate("window.location.href")
            .await
            .map_err(|e| format!("Failed to get URL: {}", e))?
            .into_value()
            .unwrap_or_default();

        // Get all interactive elements
        let elements: Value = page.evaluate(r#"
            Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"], [role="link"]'))
                .slice(0, 50)
                .map((el, i) => ({
                    ref: 'e' + i,
                    tag: el.tagName.toLowerCase(),
                    role: el.getAttribute('role') || el.tagName.toLowerCase(),
                    name: el.textContent?.trim().slice(0, 50) || el.getAttribute('aria-label') || el.getAttribute('placeholder') || '',
                    type: el.type || null,
                    href: el.href || null
                }))
        "#)
            .await
            .map_err(|e| format!("Failed to get elements: {}", e))?
            .into_value()
            .unwrap_or(json!([]));

        Ok(json!({
            "title": title,
            "url": url,
            "elements": elements
        }).to_string())
    }
}

/// Execute browser tool action.
///
/// When compiled with `browser` feature, uses real chromiumoxide CDP.
/// Otherwise, returns helpful stub responses.
#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_browser(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing browser tool");

    #[cfg(feature = "browser")]
    {
        // Run async operations in a blocking context
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|_| "Browser tool requires tokio runtime")?;

        let args = args.clone();
        rt.block_on(async move {
            exec_browser_async(&args, action).await
        })
    }

    #[cfg(not(feature = "browser"))]
    {
        exec_browser_stub(args, action)
    }
}

#[cfg(feature = "browser")]
async fn exec_browser_async(args: &Value, action: &str) -> Result<String, String> {
    let tab_id = args.get("targetId").and_then(|v| v.as_str());

    match action {
        "status" => real::status().await,
        "start" => real::start().await,
        "stop" => real::stop().await,
        "tabs" => real::list_tabs().await,

        "open" => {
            let url = args.get("targetUrl")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'targetUrl' for open action")?;
            real::open_tab(url).await
        }

        "navigate" => {
            let url = args.get("targetUrl")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'targetUrl' for navigate action")?;
            real::navigate(tab_id, url).await
        }

        "screenshot" => {
            let full_page = args.get("fullPage")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            real::screenshot(tab_id, full_page).await
        }

        "snapshot" => real::snapshot(tab_id).await,

        "close" => {
            let id = tab_id.ok_or("Missing 'targetId' for close action")?;
            real::close_tab(id).await
        }

        "act" => {
            let request = args.get("request")
                .ok_or("Missing 'request' for act action")?;

            let kind = request.get("kind")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'kind' in request")?;

            match kind {
                "click" => {
                    let selector = request.get("ref")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'ref' for click")?;
                    real::click(tab_id, selector).await
                }
                "type" => {
                    let selector = request.get("ref")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'ref' for type")?;
                    let text = request.get("text")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'text' for type")?;
                    real::type_text(tab_id, selector, text).await
                }
                "press" => {
                    let key = request.get("key")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'key' for press")?;
                    real::press_key(tab_id, key).await
                }
                "evaluate" => {
                    let script = request.get("fn")
                        .and_then(|v| v.as_str())
                        .ok_or("Missing 'fn' for evaluate")?;
                    real::evaluate(tab_id, script).await
                }
                _ => Err(format!("Unknown act kind: {}", kind))
            }
        }

        "console" => {
            // Console logs would need event subscription
            Ok("Console log capture requires persistent event handling. Use 'evaluate' with console inspection instead.".to_string())
        }

        "pdf" => {
            // PDF generation
            Ok("PDF generation not yet implemented. Use screenshot for now.".to_string())
        }

        "profiles" => {
            Ok(json!({
                "profiles": ["rustyclaw"],
                "current": "rustyclaw",
                "note": "RustyClaw uses a single managed browser profile"
            }).to_string())
        }

        "focus" => {
            // Tab focusing would need window management
            Ok("Tab focus not implemented - tabs are managed internally.".to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: status, start, stop, tabs, open, navigate, screenshot, snapshot, close, act, profiles",
            action
        ))
    }
}

#[cfg(not(feature = "browser"))]
mod lite {
    //! Lightweight browser automation without CDP.
    //!
    //! Uses reqwest to fetch pages, parses HTML to extract interactive elements,
    //! links, and text content.  Tracks "tabs" (URL + cached content) in memory.

    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    /// A lightweight "tab" backed by reqwest.
    struct LiteTab {
        url: String,
        title: String,
        body: String,           // raw HTML
        text_content: String,   // stripped text
        links: Vec<Value>,
        forms: Vec<Value>,
        interactive: Vec<Value>,
    }

    /// Global tab store.
    struct LiteBrowser {
        tabs: HashMap<String, LiteTab>,
        next_id: usize,
    }

    static LITE: OnceLock<Mutex<LiteBrowser>> = OnceLock::new();

    fn browser() -> &'static Mutex<LiteBrowser> {
        LITE.get_or_init(|| {
            Mutex::new(LiteBrowser {
                tabs: HashMap::new(),
                next_id: 0,
            })
        })
    }

    fn http_client() -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(
                "Mozilla/5.0 (compatible; RustyClaw/0.1; +https://github.com/RustyClaw)",
            )
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))
    }

    /// Fetch a URL and build a LiteTab from the response.
    fn fetch_tab(url: &str) -> Result<LiteTab, String> {
        let client = http_client()?;
        let resp = client
            .get(url)
            .send()
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        let body = resp
            .text()
            .map_err(|e| format!("Failed to read body: {}", e))?;

        let title = extract_tag(&body, "title").unwrap_or_default();
        let links = extract_links(&body);
        let forms = extract_forms(&body);
        let interactive = extract_interactive(&body);
        let text_content = strip_html(&body);

        Ok(LiteTab {
            url: url.to_string(),
            title,
            body,
            text_content,
            links,
            forms,
            interactive,
        })
    }

    // ── HTML helpers ────────────────────────────────────────────────────────

    fn extract_tag(html: &str, tag: &str) -> Option<String> {
        let lower = html.to_lowercase();
        let open = format!("<{}", tag);
        let close = format!("</{}>", tag);
        let start = lower.find(&open)?;
        let after = html[start..].find('>')? + start + 1;
        let end = lower[after..].find(&close)? + after;
        Some(html[after..end].trim().to_string())
    }

    fn strip_html(html: &str) -> String {
        let mut out = String::with_capacity(html.len() / 2);
        let mut in_tag = false;
        let mut last_space = false;
        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => {
                    in_tag = false;
                    if !last_space {
                        out.push(' ');
                        last_space = true;
                    }
                }
                _ if !in_tag => {
                    if ch.is_whitespace() {
                        if !last_space {
                            out.push(' ');
                            last_space = true;
                        }
                    } else {
                        out.push(ch);
                        last_space = false;
                    }
                }
                _ => {}
            }
        }
        out.trim().to_string()
    }

    /// Extract `<a href="…">text</a>` into JSON values.
    fn extract_links(html: &str) -> Vec<Value> {
        let mut links = Vec::new();
        let lower = html.to_lowercase();
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find("<a ") {
            let abs = search_from + pos;
            let tag_end = match lower[abs..].find('>') {
                Some(e) => abs + e,
                None => break,
            };
            let close = match lower[tag_end..].find("</a>") {
                Some(c) => tag_end + c,
                None => {
                    search_from = tag_end + 1;
                    continue;
                }
            };
            let tag_str = &html[abs..tag_end + 1];
            let href = attr_value(tag_str, "href").unwrap_or_default();
            let text = strip_html(&html[tag_end + 1..close]);
            if !href.is_empty() {
                links.push(json!({
                    "tag": "a",
                    "href": href,
                    "text": if text.len() > 80 { format!("{}…", &text[..80]) } else { text },
                }));
            }
            if links.len() >= 50 {
                break;
            }
            search_from = close + 4;
        }
        links
    }

    /// Extract `<form …>` tags.
    fn extract_forms(html: &str) -> Vec<Value> {
        let mut forms = Vec::new();
        let lower = html.to_lowercase();
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find("<form") {
            let abs = search_from + pos;
            let tag_end = match lower[abs..].find('>') {
                Some(e) => abs + e,
                None => break,
            };
            let tag_str = &html[abs..tag_end + 1];
            let action = attr_value(tag_str, "action").unwrap_or_default();
            let method = attr_value(tag_str, "method").unwrap_or_else(|| "GET".into());
            forms.push(json!({
                "tag": "form",
                "action": action,
                "method": method.to_uppercase(),
            }));
            if forms.len() >= 20 {
                break;
            }
            search_from = tag_end + 1;
        }
        forms
    }

    /// Extract interactive elements (buttons, inputs, selects, textareas).
    fn extract_interactive(html: &str) -> Vec<Value> {
        let mut elements = Vec::new();
        let tags = ["button", "input", "select", "textarea"];
        let lower = html.to_lowercase();

        for tag_name in &tags {
            let open = format!("<{}", tag_name);
            let mut search_from = 0;
            while let Some(pos) = lower[search_from..].find(&open) {
                let abs = search_from + pos;
                let tag_end = match lower[abs..].find('>') {
                    Some(e) => abs + e,
                    None => break,
                };
                let tag_str = &html[abs..tag_end + 1];
                let mut elem = json!({
                    "tag": tag_name,
                    "ref": format!("e{}", elements.len()),
                });
                if let Some(t) = attr_value(tag_str, "type") {
                    elem["type"] = json!(t);
                }
                if let Some(n) = attr_value(tag_str, "name") {
                    elem["name"] = json!(n);
                }
                if let Some(p) = attr_value(tag_str, "placeholder") {
                    elem["placeholder"] = json!(p);
                }
                if let Some(v) = attr_value(tag_str, "value") {
                    elem["value"] = json!(v);
                }
                if let Some(l) = attr_value(tag_str, "aria-label") {
                    elem["aria-label"] = json!(l);
                }
                elements.push(elem);
                if elements.len() >= 50 {
                    break;
                }
                search_from = tag_end + 1;
            }
            if elements.len() >= 50 {
                break;
            }
        }
        elements
    }

    /// Extract a single HTML attribute value from a tag string.
    fn attr_value(tag: &str, attr: &str) -> Option<String> {
        let lower = tag.to_lowercase();
        let needle = format!("{}=\"", attr);
        let pos = lower.find(&needle)?;
        let start = pos + needle.len();
        let end = lower[start..].find('"')? + start;
        Some(tag[start..end].to_string())
    }

    // ── Public API ──────────────────────────────────────────────────────────

    pub fn status() -> Result<String, String> {
        let br = browser().lock().map_err(|_| "lock poisoned")?;
        Ok(json!({
            "running": true,
            "mode": "lite",
            "tabs": br.tabs.len(),
            "profile": "rustyclaw",
            "note": "Lite mode (reqwest). Build with --features browser for full CDP.",
        })
        .to_string())
    }

    pub fn start() -> Result<String, String> {
        // No-op in lite mode; browser() initialises lazily
        Ok("Browser lite mode ready (reqwest-based). No external browser needed.".to_string())
    }

    pub fn stop() -> Result<String, String> {
        let mut br = browser().lock().map_err(|_| "lock poisoned")?;
        br.tabs.clear();
        br.next_id = 0;
        Ok("Lite browser state cleared.".to_string())
    }

    pub fn list_tabs() -> Result<String, String> {
        let br = browser().lock().map_err(|_| "lock poisoned")?;
        let tabs: Vec<Value> = br
            .tabs
            .iter()
            .map(|(id, t)| {
                json!({
                    "id": id,
                    "url": t.url,
                    "title": t.title,
                })
            })
            .collect();
        Ok(json!({ "tabs": tabs }).to_string())
    }

    pub fn open_tab(url: &str) -> Result<String, String> {
        let tab = fetch_tab(url)?;
        let mut br = browser().lock().map_err(|_| "lock poisoned")?;
        let id = format!("tab_{}", br.next_id);
        br.next_id += 1;
        let title = tab.title.clone();
        br.tabs.insert(id.clone(), tab);
        Ok(json!({
            "success": true,
            "tabId": id,
            "url": url,
            "title": title,
        })
        .to_string())
    }

    pub fn navigate(tab_id: Option<&str>, url: &str) -> Result<String, String> {
        let tab = fetch_tab(url)?;
        let mut br = browser().lock().map_err(|_| "lock poisoned")?;
        let id = match tab_id {
            Some(id) => {
                if !br.tabs.contains_key(id) {
                    return Err(format!("Tab not found: {}", id));
                }
                id.to_string()
            }
            None => br
                .tabs
                .keys()
                .next()
                .cloned()
                .ok_or("No tabs open")?,
        };
        let title = tab.title.clone();
        br.tabs.insert(id, tab);
        Ok(json!({
            "success": true,
            "url": url,
            "title": title,
        })
        .to_string())
    }

    pub fn snapshot(tab_id: Option<&str>) -> Result<String, String> {
        let br = browser().lock().map_err(|_| "lock poisoned")?;
        let tab = match tab_id {
            Some(id) => br.tabs.get(id).ok_or_else(|| format!("Tab not found: {}", id))?,
            None => br.tabs.values().next().ok_or("No tabs open")?,
        };
        Ok(json!({
            "title": tab.title,
            "url": tab.url,
            "elements": tab.interactive,
            "links": tab.links.len(),
            "forms": tab.forms.len(),
        })
        .to_string())
    }

    pub fn get_content(tab_id: Option<&str>) -> Result<String, String> {
        let br = browser().lock().map_err(|_| "lock poisoned")?;
        let tab = match tab_id {
            Some(id) => br.tabs.get(id).ok_or_else(|| format!("Tab not found: {}", id))?,
            None => br.tabs.values().next().ok_or("No tabs open")?,
        };
        let text = &tab.text_content;
        if text.len() > 50_000 {
            Ok(format!("{}…\n\n[truncated at 50KB]", &text[..50_000]))
        } else {
            Ok(text.clone())
        }
    }

    pub fn screenshot() -> Result<String, String> {
        Ok(json!({
            "note": "Screenshots require the 'browser' feature (CDP). Use 'snapshot' for an accessibility-style view of the page.",
        })
        .to_string())
    }

    pub fn close_tab(tab_id: &str) -> Result<String, String> {
        let mut br = browser().lock().map_err(|_| "lock poisoned")?;
        if br.tabs.remove(tab_id).is_some() {
            Ok(json!({ "success": true, "closed": tab_id }).to_string())
        } else {
            Err(format!("Tab not found: {}", tab_id))
        }
    }
}

#[cfg(not(feature = "browser"))]
fn exec_browser_stub(args: &Value, action: &str) -> Result<String, String> {
    debug!(action, "Browser stub mode (feature not enabled)");
    let tab_id = args.get("targetId").and_then(|v| v.as_str());

    match action {
        "status" => lite::status(),
        "start" => lite::start(),
        "stop" => lite::stop(),
        "tabs" => lite::list_tabs(),

        "open" => {
            let url = args
                .get("targetUrl")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'targetUrl' for open action")?;
            lite::open_tab(url)
        }

        "navigate" => {
            let url = args
                .get("targetUrl")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'targetUrl' for navigate action")?;
            lite::navigate(tab_id, url)
        }

        "screenshot" => lite::screenshot(),
        "snapshot" => lite::snapshot(tab_id),
        "close" => {
            let id = tab_id.ok_or("Missing 'targetId' for close action")?;
            lite::close_tab(id)
        }

        "act" => {
            let request = args
                .get("request")
                .ok_or("Missing 'request' for act action")?;
            let kind = request
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'kind' in request")?;

            match kind {
                "click" | "type" | "press" => Ok(json!({
                    "note": format!("Action '{}' requires the 'browser' feature (CDP). Use 'snapshot' to see interactive elements.", kind),
                })
                .to_string()),
                "evaluate" => Ok(json!({
                    "note": "JavaScript evaluation requires the 'browser' feature (CDP). Use 'get_content' for page text.",
                })
                .to_string()),
                _ => Err(format!("Unknown act kind: {}", kind)),
            }
        }

        "content" | "get_content" => lite::get_content(tab_id),

        "profiles" => Ok(json!({
            "profiles": ["rustyclaw"],
            "current": "rustyclaw",
            "mode": "lite",
        })
        .to_string()),

        "console" | "pdf" | "focus" => Ok(json!({
            "note": format!("Action '{}' requires the 'browser' feature (CDP).", action),
        })
        .to_string()),

        _ => Err(format!(
            "Unknown action: {}. Valid: status, start, stop, tabs, open, navigate, snapshot, screenshot, close, act, content, profiles",
            action
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_browser_stub_status() {
        let args = json!({ "action": "status" });
        let result = exec_browser(&args, &PathBuf::from("/tmp"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_browser_missing_action() {
        let args = json!({});
        let result = exec_browser(&args, &PathBuf::from("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("action"));
    }
}
