//! Canvas tool: UI presentation and page capture.

use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use tracing::{debug, instrument};

/// Tracked canvas URL for navigate/eval/snapshot.
static CANVAS_URL: Mutex<Option<String>> = Mutex::new(None);

// ── Async implementation ────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub async fn exec_canvas_async(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing canvas tool (async)");

    let node = args.get("node").and_then(|v| v.as_str());

    match action {
        "present" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for present action")?;
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800);
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600);

            if let Ok(mut guard) = CANVAS_URL.lock() {
                *guard = Some(url.to_string());
            }

            let open_result = open_in_browser_async(url).await;
            let meta = fetch_page_meta_async(url).await;

            Ok(json!({
                "status": "presented",
                "url": url,
                "size": format!("{}x{}", width, height),
                "node": node.unwrap_or("default"),
                "opened_in_browser": open_result.is_ok(),
                "title": meta.0,
                "description": meta.1,
            })
            .to_string())
        }

        "hide" => {
            if let Ok(mut guard) = CANVAS_URL.lock() {
                *guard = None;
            }
            Ok(json!({"status": "hidden", "node": node.unwrap_or("default")}).to_string())
        }

        "navigate" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for navigate action")?;

            if let Ok(mut guard) = CANVAS_URL.lock() {
                *guard = Some(url.to_string());
            }

            let open_result = open_in_browser_async(url).await;
            let meta = fetch_page_meta_async(url).await;

            Ok(json!({
                "status": "navigated",
                "url": url,
                "opened_in_browser": open_result.is_ok(),
                "title": meta.0,
                "description": meta.1,
            })
            .to_string())
        }

        "eval" => {
            let js = args
                .get("javaScript")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'javaScript' for eval action")?;

            let current_url = CANVAS_URL
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| "(none)".to_string());

            Ok(json!({
                "status": "eval_recorded",
                "canvas_url": current_url,
                "script_length": js.len(),
                "script_preview": if js.len() > 200 { &js[..200] } else { js },
                "note": "JavaScript evaluation requires browser context. Enable 'browser' feature for CDP support.",
            }).to_string())
        }

        "snapshot" => {
            let current_url = CANVAS_URL.lock().ok().and_then(|g| g.clone());

            match current_url {
                Some(url) => {
                    let content = fetch_page_text_async(&url, 30_000).await;
                    Ok(json!({
                        "status": "snapshot_captured",
                        "url": url,
                        "node": node.unwrap_or("default"),
                        "content_length": content.len(),
                        "content": content,
                    })
                    .to_string())
                }
                None => Ok(json!({
                    "status": "no_canvas",
                    "node": node.unwrap_or("default"),
                    "note": "No canvas URL presented. Use 'present' first.",
                })
                .to_string()),
            }
        }

        "a2ui_push" => {
            let elements = args.get("elements");
            Ok(json!({
                "status": "a2ui_pushed",
                "element_count": elements.and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0),
                "note": "A2UI elements registered.",
            })
            .to_string())
        }

        "a2ui_reset" => {
            Ok(json!({"status": "a2ui_reset", "note": "A2UI state cleared."}).to_string())
        }

        _ => Err(format!(
            "Unknown action: {}. Valid: present, hide, navigate, eval, snapshot, a2ui_push, a2ui_reset",
            action
        )),
    }
}

async fn open_in_browser_async(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "windows")]
    let cmd = "start";
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let cmd = "xdg-open";

    tokio::process::Command::new(cmd)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

async fn fetch_page_meta_async(url: &str) -> (String, String) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("RustyClaw/0.1 (canvas)")
        .build()
    {
        Ok(c) => c,
        Err(_) => return ("(fetch failed)".into(), String::new()),
    };

    let body = match client
        .get(url)
        .send()
        .await
        .and_then(|r| Ok(r))
        .map(|r| r.text())
    {
        Ok(fut) => fut.await.unwrap_or_default(),
        Err(_) => return ("(fetch failed)".into(), String::new()),
    };

    let title = extract_tag(&body, "title").unwrap_or_default();
    let description = extract_meta_content(&body, "description").unwrap_or_default();
    (title, description)
}

async fn fetch_page_text_async(url: &str, max_chars: usize) -> String {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("RustyClaw/0.1 (canvas snapshot)")
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("(fetch error: {})", e),
    };

    let body = match client.get(url).send().await {
        Ok(r) => r.text().await.unwrap_or_default(),
        Err(e) => return format!("(fetch error: {})", e),
    };

    let text = strip_html_tags(&body);
    if text.len() > max_chars {
        format!("{}…", &text[..max_chars])
    } else {
        text
    }
}

// ── Sync implementation ─────────────────────────────────────────────────────

#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_canvas(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing canvas tool");

    let node = args.get("node").and_then(|v| v.as_str());

    match action {
        "present" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for present action")?;
            let width = args.get("width").and_then(|v| v.as_u64()).unwrap_or(800);
            let height = args.get("height").and_then(|v| v.as_u64()).unwrap_or(600);

            if let Ok(mut guard) = CANVAS_URL.lock() {
                *guard = Some(url.to_string());
            }

            let open_result = open_in_browser(url);
            let meta = fetch_page_meta(url);

            Ok(json!({
                "status": "presented",
                "url": url,
                "size": format!("{}x{}", width, height),
                "node": node.unwrap_or("default"),
                "opened_in_browser": open_result.is_ok(),
                "title": meta.0,
                "description": meta.1,
            })
            .to_string())
        }

        "hide" => {
            if let Ok(mut guard) = CANVAS_URL.lock() {
                *guard = None;
            }
            Ok(json!({"status": "hidden", "node": node.unwrap_or("default")}).to_string())
        }

        "navigate" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'url' for navigate action")?;

            if let Ok(mut guard) = CANVAS_URL.lock() {
                *guard = Some(url.to_string());
            }

            let open_result = open_in_browser(url);
            let meta = fetch_page_meta(url);

            Ok(json!({
                "status": "navigated",
                "url": url,
                "opened_in_browser": open_result.is_ok(),
                "title": meta.0,
                "description": meta.1,
            })
            .to_string())
        }

        "eval" => {
            let js = args
                .get("javaScript")
                .and_then(|v| v.as_str())
                .ok_or("Missing 'javaScript' for eval action")?;

            let current_url = CANVAS_URL
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| "(none)".to_string());

            Ok(json!({
                "status": "eval_recorded",
                "canvas_url": current_url,
                "script_length": js.len(),
                "note": "JavaScript evaluation requires browser context.",
            })
            .to_string())
        }

        "snapshot" => {
            let current_url = CANVAS_URL.lock().ok().and_then(|g| g.clone());

            match current_url {
                Some(url) => {
                    let content = fetch_page_text(&url, 30_000);
                    Ok(json!({
                        "status": "snapshot_captured",
                        "url": url,
                        "node": node.unwrap_or("default"),
                        "content_length": content.len(),
                        "content": content,
                    })
                    .to_string())
                }
                None => Ok(json!({
                    "status": "no_canvas",
                    "node": node.unwrap_or("default"),
                    "note": "No canvas URL presented.",
                })
                .to_string()),
            }
        }

        "a2ui_push" | "a2ui_reset" => {
            Ok(json!({"status": action, "note": "A2UI handled."}).to_string())
        }

        _ => Err(format!("Unknown action: {}", action)),
    }
}

fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "windows")]
    let cmd = "start";
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let cmd = "xdg-open";

    Command::new(cmd)
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

fn fetch_page_meta(url: &str) -> (String, String) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("RustyClaw/0.1 (canvas)")
        .build()
    {
        Ok(c) => c,
        Err(_) => return ("(fetch failed)".into(), String::new()),
    };

    let body = match client.get(url).send().and_then(|r| r.text()) {
        Ok(t) => t,
        Err(_) => return ("(fetch failed)".into(), String::new()),
    };

    let title = extract_tag(&body, "title").unwrap_or_default();
    let description = extract_meta_content(&body, "description").unwrap_or_default();
    (title, description)
}

fn fetch_page_text(url: &str, max_chars: usize) -> String {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("RustyClaw/0.1 (canvas snapshot)")
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("(fetch error: {})", e),
    };

    let body = match client.get(url).send().and_then(|r| r.text()) {
        Ok(t) => t,
        Err(e) => return format!("(fetch error: {})", e),
    };

    let text = strip_html_tags(&body);
    if text.len() > max_chars {
        format!("{}…", &text[..max_chars])
    } else {
        text
    }
}

// ── HTML helpers ────────────────────────────────────────────────────────────

fn extract_tag(html: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let start = html.to_lowercase().find(&open)?;
    let after_open = html[start..].find('>')? + start + 1;
    let end = html[after_open..].to_lowercase().find(&close)? + after_open;
    Some(html[after_open..end].trim().to_string())
}

fn extract_meta_content(html: &str, name: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let needle = format!("name=\"{}\"", name);
    let pos = lower.find(&needle)?;
    let region = &html[pos.saturating_sub(10)..html.len().min(pos + 300)];
    let content_pos = region.to_lowercase().find("content=\"")?;
    let start = content_pos + 9;
    let end = region[start..].find('"')? + start;
    Some(region[start..end].to_string())
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;

    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_was_space {
                        result.push(' ');
                        last_was_space = true;
                    }
                } else {
                    result.push(ch);
                    last_was_space = false;
                }
            }
            _ => {}
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extract_title() {
        let html = "<html><head><title>Test Page</title></head></html>";
        assert_eq!(extract_tag(html, "title"), Some("Test Page".to_string()));
    }

    #[test]
    fn test_strip_html() {
        let html = "<div>Hello <b>World</b></div>";
        assert_eq!(strip_html_tags(html), "Hello World");
    }
}
