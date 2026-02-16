//! Voice features - Speech-to-Text (STT) and Text-to-Speech (TTS)
//!
//! This module provides voice interaction capabilities including:
//! - Speech-to-Text transcription via multiple providers
//! - Text-to-Speech synthesis with natural voices
//! - Wake word detection for hands-free activation
//! - Audio device management
//!
//! Supported providers:
//! - OpenAI (Whisper STT + TTS)
//! - ElevenLabs (Premium TTS)
//! - Google Cloud Speech (STT + TTS)
//! - Azure Cognitive Services (STT + TTS)
//! - Vosk (Local STT)
//! - Coqui TTS (Local TTS)
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use rustyclaw::voice::{VoiceManager, SttRequest, TtsRequest};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create voice manager from config
//! let mut voice_mgr = VoiceManager::from_env().await?;
//!
//! // Speech-to-Text
//! let audio_bytes = std::fs::read("input.wav")?;
//! let stt_result = voice_mgr.transcribe(SttRequest {
//!     audio: audio_bytes,
//!     language: Some("en-US".to_string()),
//!     model: None,
//! }).await?;
//! println!("Transcription: {}", stt_result.text);
//!
//! // Text-to-Speech
//! let tts_result = voice_mgr.synthesize(TtsRequest {
//!     text: "Hello from RustyClaw!".to_string(),
//!     voice: None,
//!     language: None,
//!     speed: Some(1.0),
//! }).await?;
//! std::fs::write("output.wav", tts_result.audio)?;
//! # Ok(())
//! # }
//! ```
//!
//! See [VOICE.md](../../docs/VOICE.md) for complete setup and configuration guide.

use anyhow::Result;
use serde::{Deserialize, Serialize};

mod types;
pub use types::*;

// Provider modules (feature-gated for optional dependencies)
#[cfg(feature = "voice-openai")]
pub mod openai;

#[cfg(feature = "voice-elevenlabs")]
pub mod elevenlabs;

#[cfg(feature = "voice-google")]
pub mod google;

#[cfg(feature = "voice-azure")]
pub mod azure;

#[cfg(feature = "voice-local")]
pub mod vosk;

#[cfg(feature = "voice-local")]
pub mod coqui;

/// Voice configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    /// Whether voice features are enabled
    pub enabled: bool,
    /// STT provider to use
    pub stt_provider: String,
    /// TTS provider to use
    pub tts_provider: String,
    /// Whether wake word detection is enabled
    pub wake_word_enabled: bool,
    /// Wake word phrase
    pub wake_word: String,
    /// Audio input device
    pub input_device: String,
    /// Audio output device
    pub output_device: String,
    /// Sample rate for audio capture
    pub sample_rate: u32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            stt_provider: "openai".to_string(),
            tts_provider: "openai".to_string(),
            wake_word_enabled: false,
            wake_word: "hey rustyclaw".to_string(),
            input_device: "default".to_string(),
            output_device: "default".to_string(),
            sample_rate: 16000,
        }
    }
}

/// Voice manager - coordinates STT, TTS, and wake word detection
pub struct VoiceManager {
    config: VoiceConfig,
    stt_provider: Box<dyn SttProvider>,
    tts_provider: Box<dyn TtsProvider>,
}

impl VoiceManager {
    /// Create voice manager from configuration
    pub async fn from_config(config: VoiceConfig) -> Result<Self> {
        let stt_provider = Self::create_stt_provider(&config).await?;
        let tts_provider = Self::create_tts_provider(&config).await?;

        Ok(Self {
            config,
            stt_provider,
            tts_provider,
        })
    }

    /// Create voice manager from environment variables
    pub async fn from_env() -> Result<Self> {
        let config = VoiceConfig::default();
        Self::from_config(config).await
    }

    /// Transcribe audio to text
    pub async fn transcribe(&self, request: SttRequest) -> Result<SttResponse> {
        self.stt_provider.transcribe(request).await
    }

    /// Synthesize text to audio
    pub async fn synthesize(&self, request: TtsRequest) -> Result<TtsResponse> {
        self.tts_provider.synthesize(request).await
    }

    async fn create_stt_provider(_config: &VoiceConfig) -> Result<Box<dyn SttProvider>> {
        // TODO: Create provider based on config.stt_provider
        anyhow::bail!("Voice features not yet implemented - see docs/VOICE.md for roadmap")
    }

    async fn create_tts_provider(_config: &VoiceConfig) -> Result<Box<dyn TtsProvider>> {
        // TODO: Create provider based on config.tts_provider
        anyhow::bail!("Voice features not yet implemented - see docs/VOICE.md for roadmap")
    }
}

// Stub provider implementations will go in separate files
// - src/voice/openai.rs
// - src/voice/elevenlabs.rs
// - src/voice/google.rs
// - src/voice/azure.rs
// - src/voice/vosk.rs
// - src/voice/coqui.rs
