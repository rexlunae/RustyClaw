//! Media pipeline for image, audio, and video processing.
//!
//! Provides utilities for handling media attachments in messenger conversations:
//! - Image processing (resize, format conversion, size caps)
//! - Audio transcription (via external tools like whisper)
//! - Video frame extraction
//! - MIME type detection
//! - Size limit enforcement

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::{debug, warn};

/// Supported media types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Audio,
    Video,
    Document,
    Unknown,
}

impl MediaType {
    /// Detect media type from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "tiff" | "ico" => {
                Self::Image
            }
            "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "wma" | "opus" => Self::Audio,
            "mp4" | "webm" | "avi" | "mov" | "mkv" | "flv" | "wmv" => Self::Video,
            "pdf" | "doc" | "docx" | "txt" | "rtf" | "odt" => Self::Document,
            _ => Self::Unknown,
        }
    }

    /// Detect media type from MIME type string.
    pub fn from_mime(mime: &str) -> Self {
        if mime.starts_with("image/") {
            Self::Image
        } else if mime.starts_with("audio/") {
            Self::Audio
        } else if mime.starts_with("video/") {
            Self::Video
        } else if mime.starts_with("application/pdf")
            || mime.starts_with("application/msword")
            || mime.starts_with("text/")
        {
            Self::Document
        } else {
            Self::Unknown
        }
    }
}

/// Media pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Maximum file size in bytes for image uploads (default: 10 MB).
    #[serde(default = "default_image_max")]
    pub image_max_bytes: usize,

    /// Maximum file size in bytes for audio uploads (default: 25 MB).
    #[serde(default = "default_audio_max")]
    pub audio_max_bytes: usize,

    /// Maximum file size in bytes for video uploads (default: 50 MB).
    #[serde(default = "default_video_max")]
    pub video_max_bytes: usize,

    /// Maximum image dimension (width or height) for resizing.
    #[serde(default = "default_max_dimension")]
    pub max_image_dimension: u32,

    /// Whether to auto-transcribe audio attachments.
    #[serde(default)]
    pub auto_transcribe: bool,

    /// Whisper model size for transcription ("tiny", "base", "small", "medium", "large").
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,

    /// Temporary directory for processed media.
    #[serde(default = "default_temp_dir")]
    pub temp_dir: PathBuf,
}

fn default_image_max() -> usize {
    10 * 1024 * 1024
}
fn default_audio_max() -> usize {
    25 * 1024 * 1024
}
fn default_video_max() -> usize {
    50 * 1024 * 1024
}
fn default_max_dimension() -> u32 {
    2048
}
fn default_whisper_model() -> String {
    "base".to_string()
}
fn default_temp_dir() -> PathBuf {
    std::env::temp_dir().join("rustyclaw-media")
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            image_max_bytes: default_image_max(),
            audio_max_bytes: default_audio_max(),
            video_max_bytes: default_video_max(),
            max_image_dimension: default_max_dimension(),
            auto_transcribe: false,
            whisper_model: default_whisper_model(),
            temp_dir: default_temp_dir(),
        }
    }
}

/// Result of processing a media file.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessedMedia {
    /// Original file path.
    pub original_path: PathBuf,
    /// Processed file path (may be same as original if no processing needed).
    pub processed_path: PathBuf,
    /// Detected media type.
    pub media_type: MediaType,
    /// File size in bytes after processing.
    pub size_bytes: u64,
    /// Optional transcription text (for audio/video).
    pub transcription: Option<String>,
    /// Optional description (for images via vision model).
    pub description: Option<String>,
    /// MIME type.
    pub mime_type: String,
}

/// Check if a file exceeds size limits.
pub fn check_size_limit(path: &Path, config: &MediaConfig) -> Result<(), String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("Cannot read file metadata: {}", e))?;
    let size = metadata.len() as usize;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let media_type = MediaType::from_extension(ext);

    let limit = match media_type {
        MediaType::Image => config.image_max_bytes,
        MediaType::Audio => config.audio_max_bytes,
        MediaType::Video => config.video_max_bytes,
        _ => config.video_max_bytes, // use largest limit as fallback
    };

    if size > limit {
        Err(format!(
            "File size ({} bytes) exceeds limit ({} bytes) for {:?}",
            size, limit, media_type
        ))
    } else {
        Ok(())
    }
}

/// Resize an image using ImageMagick convert or ffmpeg.
pub fn resize_image(
    input: &Path,
    max_dimension: u32,
    output_dir: &Path,
) -> Result<PathBuf, String> {
    let filename = input
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("output.jpg");
    let output = output_dir.join(format!("resized_{}", filename));

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output dir: {}", e))?;

    // Try ImageMagick first
    let result = Command::new("convert")
        .args([
            input.to_string_lossy().as_ref(),
            "-resize",
            &format!("{}x{}>", max_dimension, max_dimension),
            output.to_string_lossy().as_ref(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(out) = result {
        if out.status.success() {
            debug!(input = %input.display(), output = %output.display(), "Image resized with ImageMagick");
            return Ok(output);
        }
    }

    // Fallback to ffmpeg
    let result = Command::new("ffmpeg")
        .args([
            "-i",
            input.to_string_lossy().as_ref(),
            "-vf",
            &format!(
                "scale='min({0},iw)':min'({0},ih)':force_original_aspect_ratio=decrease",
                max_dimension
            ),
            "-y",
            output.to_string_lossy().as_ref(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(out) = result {
        if out.status.success() {
            debug!(input = %input.display(), output = %output.display(), "Image resized with ffmpeg");
            return Ok(output);
        }
    }

    // No resize tools available — return original
    warn!("No image resize tools available (install ImageMagick or ffmpeg)");
    Ok(input.to_path_buf())
}

/// Transcribe an audio file using whisper.
pub fn transcribe_audio(input: &Path, model: &str) -> Result<String, String> {
    // Try whisper CLI (OpenAI's whisper or whisper.cpp)
    let result = Command::new("whisper")
        .args([
            input.to_string_lossy().as_ref(),
            "--model",
            model,
            "--output_format",
            "txt",
            "--output_dir",
            "/tmp",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(out) = result {
        if out.status.success() {
            // whisper writes to <input_name>.txt
            let txt_path = PathBuf::from("/tmp").join(
                input
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("audio"),
            ).with_extension("txt");

            if let Ok(text) = std::fs::read_to_string(&txt_path) {
                debug!(input = %input.display(), "Audio transcribed with whisper");
                return Ok(text.trim().to_string());
            }
        }
    }

    // Fallback: try whisper.cpp main binary
    let result = Command::new("main")
        .args([
            "-m",
            &format!("models/ggml-{}.bin", model),
            "-f",
            input.to_string_lossy().as_ref(),
            "--output-txt",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(out) = result {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !text.is_empty() {
                return Ok(text);
            }
        }
    }

    Err("Transcription failed. Install whisper (pip install openai-whisper) \
         or whisper.cpp for audio transcription support."
        .to_string())
}

/// Extract a frame from a video at a given timestamp.
pub fn extract_video_frame(
    input: &Path,
    timestamp_secs: f64,
    output_dir: &Path,
) -> Result<PathBuf, String> {
    let filename = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("frame");
    let output = output_dir.join(format!("{}_frame.jpg", filename));

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output dir: {}", e))?;

    let result = Command::new("ffmpeg")
        .args([
            "-i",
            input.to_string_lossy().as_ref(),
            "-ss",
            &format!("{:.2}", timestamp_secs),
            "-frames:v",
            "1",
            "-y",
            output.to_string_lossy().as_ref(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match result {
        Ok(out) if out.status.success() => {
            debug!(
                input = %input.display(),
                timestamp = timestamp_secs,
                "Video frame extracted"
            );
            Ok(output)
        }
        _ => Err("Failed to extract video frame. Install ffmpeg for video support.".to_string()),
    }
}

/// Detect MIME type of a file using the `file` command.
pub fn detect_mime_type(path: &Path) -> String {
    let result = Command::new("file")
        .args(["--mime-type", "-b"])
        .arg(path.to_string_lossy().as_ref())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    if let Ok(out) = result {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }

    // Fallback based on extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_from_extension() {
        assert_eq!(MediaType::from_extension("jpg"), MediaType::Image);
        assert_eq!(MediaType::from_extension("PNG"), MediaType::Image);
        assert_eq!(MediaType::from_extension("mp3"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("mp4"), MediaType::Video);
        assert_eq!(MediaType::from_extension("pdf"), MediaType::Document);
        assert_eq!(MediaType::from_extension("xyz"), MediaType::Unknown);
    }

    #[test]
    fn test_media_type_from_mime() {
        assert_eq!(MediaType::from_mime("image/jpeg"), MediaType::Image);
        assert_eq!(MediaType::from_mime("audio/mpeg"), MediaType::Audio);
        assert_eq!(MediaType::from_mime("video/mp4"), MediaType::Video);
        assert_eq!(MediaType::from_mime("application/pdf"), MediaType::Document);
        assert_eq!(
            MediaType::from_mime("application/octet-stream"),
            MediaType::Unknown
        );
    }

    #[test]
    fn test_media_config_defaults() {
        let config = MediaConfig::default();
        assert_eq!(config.image_max_bytes, 10 * 1024 * 1024);
        assert_eq!(config.audio_max_bytes, 25 * 1024 * 1024);
        assert_eq!(config.video_max_bytes, 50 * 1024 * 1024);
        assert_eq!(config.max_image_dimension, 2048);
        assert!(!config.auto_transcribe);
        assert_eq!(config.whisper_model, "base");
    }

    #[test]
    fn test_check_size_limit_nonexistent() {
        let config = MediaConfig::default();
        let result = check_size_limit(Path::new("/tmp/nonexistent.jpg"), &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_detect_mime_fallback() {
        // Test extension-based fallback for nonexistent file
        let mime = detect_mime_type(Path::new("/tmp/nonexistent.jpg"));
        // Will use either `file` command or extension fallback
        assert!(mime.contains("image") || mime.contains("octet-stream"));
    }
}
