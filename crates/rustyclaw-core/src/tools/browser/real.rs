//! Real browser automation via chromiumoxide (CDP). Compiled only with the
//! `browser` feature.

use super::*;
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::Mutex;

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
        })
        .to_string())
    } else {
        debug!("Browser status: not running");
        Ok(json!({
            "running": false,
            "tabs": 0,
            "profile": "rustyclaw"
        })
        .to_string())
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
        let tabs: Vec<Value> = s
            .pages
            .keys()
            .map(|id| {
                json!({
                    "id": id,
                    // Note: chromiumoxide doesn't expose URL easily without async call
                })
            })
            .collect();
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

    let page = s
        .browser
        .new_page(url)
        .await
        .map_err(|e| format!("Failed to open page: {}", e))?;

    // Generate a tab ID
    let tab_id = format!("tab_{}", s.pages.len());
    s.pages.insert(tab_id.clone(), page);

    Ok(json!({
        "success": true,
        "tabId": tab_id,
        "url": url
    })
    .to_string())
}

/// Navigate current page to URL.
pub async fn navigate(tab_id: Option<&str>, url: &str) -> Result<String, String> {
    let mut state = browser_state().lock().await;
    let s = state.as_mut().ok_or("Browser not running")?;

    // Get the page (use provided tab_id or first available)
    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    page.goto(url)
        .await
        .map_err(|e| format!("Navigation failed: {}", e))?;

    Ok(json!({
        "success": true,
        "url": url
    })
    .to_string())
}

/// Take a screenshot.
pub async fn screenshot(tab_id: Option<&str>, full_page: bool) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    let screenshot = if full_page {
        page.screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .full_page(true)
                .build(),
        )
        .await
        .map_err(|e| format!("Screenshot failed: {}", e))?
    } else {
        page.screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .build(),
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
    })
    .to_string())
}

/// Get page content.
#[allow(dead_code)]
pub async fn get_content(tab_id: Option<&str>) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    let content = page
        .content()
        .await
        .map_err(|e| format!("Failed to get content: {}", e))?;

    Ok(content)
}

/// Click an element by selector.
pub async fn click(tab_id: Option<&str>, selector: &str) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    let element = page
        .find_element(selector)
        .await
        .map_err(|e| format!("Element not found: {}", e))?;

    element
        .click()
        .await
        .map_err(|e| format!("Click failed: {}", e))?;

    Ok(json!({
        "success": true,
        "action": "click",
        "selector": selector
    })
    .to_string())
}

/// Type text into an element.
pub async fn type_text(tab_id: Option<&str>, selector: &str, text: &str) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    let element = page
        .find_element(selector)
        .await
        .map_err(|e| format!("Element not found: {}", e))?;

    element
        .click()
        .await
        .map_err(|e| format!("Click failed: {}", e))?;

    element
        .type_str(text)
        .await
        .map_err(|e| format!("Type failed: {}", e))?;

    Ok(json!({
        "success": true,
        "action": "type",
        "selector": selector,
        "text_length": text.len()
    })
    .to_string())
}

/// Press a key.
pub async fn press_key(tab_id: Option<&str>, key: &str) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    // Use CDP DispatchKeyEventParams for key press
    use chromiumoxide::cdp::browser_protocol::input::{
        DispatchKeyEventParams, DispatchKeyEventType,
    };

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
    })
    .to_string())
}

/// Evaluate JavaScript.
pub async fn evaluate(tab_id: Option<&str>, script: &str) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    let result: Value = page
        .evaluate(script)
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
        })
        .to_string())
    } else {
        Err(format!("Tab not found: {}", tab_id))
    }
}

/// Get accessibility snapshot (simplified).
pub async fn snapshot(tab_id: Option<&str>) -> Result<String, String> {
    let state = browser_state().lock().await;
    let s = state.as_ref().ok_or("Browser not running")?;

    let page = if let Some(id) = tab_id {
        s.pages
            .get(id)
            .ok_or_else(|| format!("Tab not found: {}", id))?
    } else {
        s.pages.values().next().ok_or("No tabs open")?
    };

    // Get basic page info since full a11y tree is complex
    let title: String = page
        .evaluate("document.title")
        .await
        .map_err(|e| format!("Failed to get title: {}", e))?
        .into_value()
        .unwrap_or_default();

    let url: String = page
        .evaluate("window.location.href")
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
    })
    .to_string())
}
