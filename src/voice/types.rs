//! Core types for voice features

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Raw audio chunk with metadata
#[derive(Debug, Clone)]
pub struct AudioChunk {
    /// Audio samples (f32 format, normalized to [-1.0, 1.0])
    pub samples: Vec<f32>,
    /// Sample rate in Hz (e.g., 16000, 44100)
    pub sample_rate: u32,
    /// Number of audio channels (1 = mono, 2 = stereo)
    pub channels: u16,
}

/// Request for Speech-to-Text transcription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttRequest {
    /// Audio data in WAV/MP3/OGG format
    pub audio: Vec<u8>,
    /// Optional language hint (e.g., "en-US", "es-ES")
    pub language: Option<String>,
    /// Optional model name (provider-specific)
    pub model: Option<String>,
}

/// Response from Speech-to-Text transcription
#[derive(Debug, Clone)]
pub struct SttResponse {
    /// Transcribed text
    pub text: String,
    /// Confidence score (0.0 to 1.0), if available
    pub confidence: f32,
    /// Detected language, if different from request
    pub language: Option<String>,
}

/// Request for Text-to-Speech synthesis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsRequest {
    /// Text to synthesize
    pub text: String,
    /// Optional voice ID or name (provider-specific)
    pub voice: Option<String>,
    /// Optional language code (e.g., "en-US")
    pub language: Option<String>,
    /// Optional speed multiplier (1.0 = normal, 0.5 = half speed, 2.0 = double speed)
    pub speed: Option<f32>,
}

/// Response from Text-to-Speech synthesis
#[derive(Debug, Clone)]
pub struct TtsResponse {
    /// Audio data in WAV/MP3 format
    pub audio: Vec<u8>,
    /// Sample rate of the audio data
    pub sample_rate: u32,
}

/// Trait for Speech-to-Text providers
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Transcribe audio to text
    async fn transcribe(&self, request: SttRequest) -> Result<SttResponse>;

    /// Get the provider name
    fn name(&self) -> &str {
        "unknown"
    }
}

/// Trait for Text-to-Speech providers
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Synthesize text to audio
    async fn synthesize(&self, request: TtsRequest) -> Result<TtsResponse>;

    /// Get the provider name
    fn name(&self) -> &str {
        "unknown"
    }

    /// List available voices (optional)
    async fn list_voices(&self) -> Result<Vec<VoiceInfo>> {
        Ok(Vec::new())
    }
}

/// Information about a TTS voice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceInfo {
    /// Voice ID
    pub id: String,
    /// Display name
    pub name: String,
    /// Language code
    pub language: String,
    /// Gender (male, female, neutral)
    pub gender: String,
    /// Description
    pub description: Option<String>,
}

/// Trait for wake word detection
pub trait WakeWordDetector: Send + Sync {
    /// Detect wake word in audio chunk
    /// Returns true if wake word was detected
    fn detect(&mut self, audio: &AudioChunk) -> Result<bool>;

    /// Get the wake word phrase
    fn wake_word(&self) -> &str;

    /// Set detection sensitivity (0.0 to 1.0, higher = more sensitive)
    fn set_sensitivity(&mut self, sensitivity: f32);
}
