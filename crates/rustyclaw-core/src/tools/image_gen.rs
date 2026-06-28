//! Image generation tool: text → image via configured provider.
//!
//! Provider-agnostic image generation routed through the provider abstraction.
//! Gated behind the `image-gen` Cargo feature flag.

use serde_json::{Value, json};
use std::path::Path;
use tracing::{debug, instrument};

use super::ToolParam;

// ── Tool executor ───────────────────────────────────────────────────────────

/// Execute the `image_generate` tool (async).
#[instrument(skip(args, workspace_dir), fields(prompt))]
pub async fn exec_image_generate_async(
    args: &Value,
    workspace_dir: &Path,
) -> Result<String, String> {
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: prompt".to_string())?;

    tracing::Span::current().record("prompt", prompt);

    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("dall-e-3");

    let size = args
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");

    let quality = args
        .get("quality")
        .and_then(|v| v.as_str())
        .unwrap_or("standard");

    let output_path = args.get("output_path").and_then(|v| v.as_str());

    let provider = args
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openai");

    debug!(model, size, quality, provider, "Generating image");

    // Resolve API key from vault
    let api_key = resolve_api_key(provider)?;

    // Dispatch to the appropriate provider
    match provider {
        "openai" => {
            generate_openai(
                prompt,
                model,
                size,
                quality,
                &api_key,
                output_path,
                workspace_dir,
            )
            .await
        }
        "gemini" => {
            generate_gemini(prompt, model, size, &api_key, output_path, workspace_dir).await
        }
        _ => Err(format!(
            "Unsupported image generation provider: '{}'. Supported: openai, gemini",
            provider
        )),
    }
}

/// Sync stub for the static ToolDef (actual execution is async).
pub fn exec_image_generate_stub(_args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    Err("image_generate requires async execution via the gateway".into())
}

// ── Provider implementations ────────────────────────────────────────────────

/// Generate image via OpenAI DALL-E API.
async fn generate_openai(
    prompt: &str,
    model: &str,
    size: &str,
    quality: &str,
    api_key: &str,
    output_path: Option<&str>,
    workspace_dir: &Path,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let body = json!({
        "model": model,
        "prompt": prompt,
        "n": 1,
        "size": size,
        "quality": quality,
        "response_format": "url",
    });

    let response = client
        .post("https://api.openai.com/v1/images/generations")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("OpenAI API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("OpenAI API error ({}): {}", status, error_body));
    }

    let result: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse OpenAI response: {}", e))?;

    let image_url = result
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|img| img.get("url"))
        .and_then(|u| u.as_str())
        .ok_or("No image URL in OpenAI response")?;

    // Download the image to a local file
    let file_path = download_image(image_url, output_path, workspace_dir, "openai").await?;

    Ok(json!({
        "provider": "openai",
        "model": model,
        "prompt": prompt,
        "size": size,
        "quality": quality,
        "path": file_path,
        "url": image_url,
    })
    .to_string())
}

/// Generate image via Google Gemini / Imagen API.
async fn generate_gemini(
    prompt: &str,
    model: &str,
    size: &str,
    api_key: &str,
    output_path: Option<&str>,
    workspace_dir: &Path,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let body = json!({
        "contents": [{
            "parts": [{
                "text": format!("Generate an image: {}", prompt)
            }]
        }],
        "generationConfig": {
            "responseModalities": ["image"],
            "imageDimensions": size,
        }
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Gemini API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Gemini API error ({}): {}", status, error_body));
    }

    let result: Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Gemini response: {}", e))?;

    // Extract base64 image data from Gemini response
    let image_data = result
        .get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|p| p.as_array())
        .and_then(|parts| parts.iter().find(|p| p.get("inlineData").is_some()))
        .and_then(|p| p.get("inlineData"))
        .and_then(|d| d.get("data"))
        .and_then(|d| d.as_str())
        .ok_or("No image data in Gemini response")?;

    // Save base64 image to file
    let file_path = save_base64_image(image_data, output_path, workspace_dir, "gemini")?;

    Ok(json!({
        "provider": "gemini",
        "model": model,
        "prompt": prompt,
        "size": size,
        "path": file_path,
    })
    .to_string())
}

// ── Helper functions ────────────────────────────────────────────────────────

/// Resolve the API key for a given provider from environment variables.
fn resolve_api_key(provider: &str) -> Result<String, String> {
    let key_names = match provider {
        "openai" => &["OPENAI_API_KEY", "OPENAI_KEY"][..],
        "gemini" => &["GEMINI_API_KEY", "GOOGLE_API_KEY"][..],
        _ => return Err(format!("No API key mapping for provider: {}", provider)),
    };

    // Try environment variables
    for name in key_names {
        if let Ok(val) = std::env::var(name) {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }

    Err(format!(
        "No API key found for provider '{}'. Set one of: {} \
         (via environment variable or secrets vault)",
        provider,
        key_names.join(", ")
    ))
}

/// Download an image from a URL and save it locally.
async fn download_image(
    url: &str,
    output_path: Option<&str>,
    workspace_dir: &Path,
    provider: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to download image: {}", e))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read image bytes: {}", e))?;

    let file_path = if let Some(path) = output_path {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            workspace_dir.join(path)
        }
    } else {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let media_dir = workspace_dir.join("media").join("generated");
        let _ = std::fs::create_dir_all(&media_dir);
        media_dir.join(format!("{}_{}.png", provider, timestamp))
    };

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::fs::write(&file_path, &bytes).map_err(|e| format!("Failed to write image file: {}", e))?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Save base64-encoded image data to a file.
fn save_base64_image(
    data: &str,
    output_path: Option<&str>,
    workspace_dir: &Path,
    provider: &str,
) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| format!("Failed to decode base64 image: {}", e))?;

    let file_path = if let Some(path) = output_path {
        let p = Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            workspace_dir.join(path)
        }
    } else {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let media_dir = workspace_dir.join("media").join("generated");
        let _ = std::fs::create_dir_all(&media_dir);
        media_dir.join(format!("{}_{}.png", provider, timestamp))
    };

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::fs::write(&file_path, &bytes).map_err(|e| format!("Failed to write image file: {}", e))?;

    Ok(file_path.to_string_lossy().to_string())
}

// ── Parameter definitions ───────────────────────────────────────────────────

pub fn image_generate_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "prompt".into(),
            description: "Text prompt describing the image to generate.".into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "model".into(),
            description: "Model to use for generation. Default: 'dall-e-3' (OpenAI) or \
                          'imagen-3.0-generate-001' (Gemini)."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "size".into(),
            description: "Image dimensions: '1024x1024' (default), '1792x1024', '1024x1792'."
                .into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "quality".into(),
            description: "Quality: 'standard' (default) or 'hd'. Only for OpenAI.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "provider".into(),
            description: "Provider: 'openai' (default) or 'gemini'.".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "output_path".into(),
            description: "File path for the generated image. Auto-generated if omitted.".into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}
