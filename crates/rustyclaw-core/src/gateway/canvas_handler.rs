//! Canvas tool execution handler for the gateway.

use tracing::{debug, warn, instrument};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::canvas::CanvasHost;

pub type SharedCanvasHost = Arc<Mutex<CanvasHost>>;

/// Check if a tool name is a canvas tool.
pub fn is_canvas_tool(name: &str) -> bool {
    name.starts_with("canvas_")
}

/// Execute a canvas tool call.
#[instrument(skip(args, canvas, session), fields(%name))]
pub async fn execute_canvas_tool(
    name: &str,
    args: &serde_json::Value,
    canvas: &SharedCanvasHost,
    session: &str,
) -> Result<String, String> {
    debug!("Executing canvas tool");

    let host = canvas.lock().await;

    match name {
        "canvas_present" => {
            // Show the canvas (for node-based canvases, this would signal the node)
            let url = host.canvas_url(session);
            Ok(format!("Canvas available at: {}", url))
        }

        "canvas_hide" => {
            // Hide the canvas (node signal)
            Ok("Canvas hidden".to_string())
        }

        "canvas_navigate" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required parameter: url".to_string())?;

            // For local paths, write a redirect or serve directly
            if url.starts_with("http://") || url.starts_with("https://") {
                Ok(format!("Navigated to external URL: {}", url))
            } else {
                let canvas_url = format!("{}{}", host.canvas_url(session), url.trim_start_matches('/'));
                Ok(format!("Navigated to: {}", canvas_url))
            }
        }

        "canvas_write" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required parameter: path".to_string())?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required parameter: content".to_string())?;

            drop(host); // Release lock before async operation
            let host = canvas.lock().await;
            
            match host.write_file(session, path, content.as_bytes()).await {
                Ok(file_path) => Ok(format!("Wrote canvas file: {}", file_path.display())),
                Err(e) => Err(format!("Failed to write canvas file: {}", e)),
            }
        }

        "canvas_a2ui_push" => {
            let text = args
                .get("text")
                .and_then(|v| v.as_str());
            let jsonl = args
                .get("jsonl")
                .and_then(|v| v.as_str());

            drop(host); // Release lock
            let host = canvas.lock().await;

            if let Some(text) = text {
                // Simple text push
                match host.push_text(session, text).await {
                    Ok(()) => Ok("A2UI text pushed".to_string()),
                    Err(e) => Err(format!("Failed to push A2UI: {}", e)),
                }
            } else if let Some(jsonl) = jsonl {
                // Parse JSONL and push
                let messages: Result<Vec<crate::canvas::A2UIMessage>, _> = jsonl
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| serde_json::from_str(l))
                    .collect();

                match messages {
                    Ok(msgs) => {
                        match host.push_a2ui(session, msgs).await {
                            Ok(()) => Ok("A2UI messages pushed".to_string()),
                            Err(e) => Err(format!("Failed to push A2UI: {}", e)),
                        }
                    }
                    Err(e) => Err(format!("Invalid A2UI JSONL: {}", e)),
                }
            } else {
                Err("Either 'text' or 'jsonl' parameter required".to_string())
            }
        }

        "canvas_a2ui_reset" => {
            drop(host);
            let host = canvas.lock().await;
            
            match host.reset_a2ui(session).await {
                Ok(()) => Ok("A2UI state reset".to_string()),
                Err(e) => Err(format!("Failed to reset A2UI: {}", e)),
            }
        }

        "canvas_snapshot" => {
            drop(host);
            let host = canvas.lock().await;
            
            match host.snapshot(session).await {
                Ok(data) => {
                    // Return base64-encoded image
                    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data);
                    Ok(format!("data:image/png;base64,{}", b64))
                }
                Err(e) => Err(format!("Snapshot failed: {}", e)),
            }
        }

        _ => {
            warn!(tool = name, "Unknown canvas tool");
            Err(format!("Unknown canvas tool: {}", name))
        }
    }
}

/// Generate canvas tools section for the system prompt.
pub fn generate_canvas_prompt_section() -> String {
    r#"
## Canvas Tools

Canvas provides an agent-controlled visual workspace for HTML, CSS, JS, and A2UI content.

- **canvas_present**: Show the canvas panel
- **canvas_hide**: Hide the canvas panel  
- **canvas_navigate**: Navigate to a URL or local path
  - Parameters: `url` (string) — path or URL to navigate to
- **canvas_write**: Write a file to the canvas directory
  - Parameters: `path` (string), `content` (string)
- **canvas_a2ui_push**: Push A2UI component updates
  - Parameters: `text` (string) OR `jsonl` (string — A2UI messages as JSONL)
- **canvas_a2ui_reset**: Reset A2UI state for the session
- **canvas_snapshot**: Capture canvas as image (returns base64 PNG)

"#.to_string()
}
