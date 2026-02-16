# Voice Features Guide

RustyClaw supports voice interaction through Speech-to-Text (STT) and Text-to-Speech (TTS), enabling hands-free operation and voice conversations with the AI agent.

## Overview

**Voice capabilities:**
- 🎤 **Voice Wake** - Wake word detection ("Hey RustyClaw")
- 🗣️ **Talk Mode** - Continuous voice conversation
- 📝 **Speech-to-Text** - Convert voice to text input
- 🔊 **Text-to-Speech** - Convert AI responses to voice output
- 🌐 **Multi-provider** - ElevenLabs, OpenAI, Google, Azure

**Use cases:**
- Hands-free operation while coding/working
- Accessibility for visually impaired users
- Mobile voice interaction
- Voice-based automation and commands
- Natural conversation interface

---

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────┐
│                    Voice Pipeline                        │
├─────────────────────────────────────────────────────────┤
│                                                           │
│  Audio Input → Wake Word → STT → Agent → TTS → Audio Out│
│                Detection                                  │
│                                                           │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐│
│  │ Mic      │→ │ Porcupine│→ │ Whisper  │→ │ElevenLabs││
│  │ cpal/    │  │ pvrecorder  │ Google   │  │ Google   ││
│  │ rodio    │  │          │  │ Azure    │  │ Azure    ││
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘│
└─────────────────────────────────────────────────────────┘
```

### Supported Providers

| Provider | STT | TTS | Wake Word | Cost | Quality |
|----------|-----|-----|-----------|------|---------|
| **OpenAI** | ✅ Whisper | ✅ | ❌ | $ | ⭐⭐⭐⭐⭐ |
| **ElevenLabs** | ❌ | ✅ | ❌ | $$ | ⭐⭐⭐⭐⭐ |
| **Google Cloud** | ✅ | ✅ | ❌ | $ | ⭐⭐⭐⭐ |
| **Azure** | ✅ | ✅ | ❌ | $ | ⭐⭐⭐⭐ |
| **Porcupine** | ❌ | ❌ | ✅ | Free | ⭐⭐⭐ |
| **Vosk (local)** | ✅ | ❌ | ❌ | Free | ⭐⭐⭐ |
| **Coqui TTS (local)** | ❌ | ✅ | ❌ | Free | ⭐⭐⭐ |

---

## Installation

### 1. System Dependencies

**macOS:**
```bash
brew install portaudio ffmpeg
```

**Ubuntu/Debian:**
```bash
sudo apt-get install -y \
    libasound2-dev \
    portaudio19-dev \
    libportaudio2 \
    ffmpeg \
    libavcodec-dev \
    libavformat-dev
```

**Fedora/RHEL:**
```bash
sudo dnf install -y \
    alsa-lib-devel \
    portaudio-devel \
    ffmpeg \
    ffmpeg-devel
```

### 2. Rust Dependencies

Add to `Cargo.toml`:

```toml
[dependencies]
# Audio I/O
cpal = "0.15"  # Cross-platform audio
rodio = "0.18"  # Audio playback
hound = "3.5"  # WAV encoding

# STT/TTS providers
reqwest = { version = "0.12", features = ["json", "stream", "multipart"] }
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }

# Optional: Local wake word
pv-recorder = { version = "1.2", optional = true }

[features]
voice = ["cpal", "rodio", "hound"]
wake-word = ["pv-recorder"]
```

### 3. API Keys

Configure providers in `~/.rustyclaw/config.toml`:

```toml
[voice]
enabled = true

# Speech-to-Text provider
stt_provider = "openai"  # openai, google, azure, vosk

# Text-to-Speech provider
tts_provider = "elevenlabs"  # elevenlabs, openai, google, azure, coqui

# Wake word settings
wake_word_enabled = true
wake_word = "hey rustyclaw"
wake_word_provider = "porcupine"

# Audio device settings
input_device = "default"
output_device = "default"
sample_rate = 16000

[voice.openai]
# For Whisper STT and TTS
api_key = "${OPENAI_API_KEY}"
stt_model = "whisper-1"
tts_model = "tts-1"
tts_voice = "nova"  # alloy, echo, fable, onyx, nova, shimmer

[voice.elevenlabs]
# For TTS only (highest quality)
api_key = "${ELEVENLABS_API_KEY}"
voice_id = "21m00Tcm4TlvDq8ikWAM"  # Rachel
model = "eleven_multilingual_v2"

[voice.google]
# For STT and TTS
project_id = "your-project-id"
credentials_path = "~/.config/gcloud/credentials.json"
language_code = "en-US"

[voice.azure]
# For STT and TTS
subscription_key = "${AZURE_SPEECH_KEY}"
region = "eastus"
language = "en-US"
voice_name = "en-US-AriaNeural"
```

---

## Quick Start

### Enable Voice Mode

**CLI flag:**
```bash
rustyclaw chat --voice
```

**Or in config:**
```toml
[voice]
enabled = true
```

### Basic Usage

1. **Start voice session:**
   ```bash
   rustyclaw chat --voice
   ```

2. **Speak wake word:**
   ```
   "Hey RustyClaw"
   ```

3. **Ask your question:**
   ```
   "What's the weather today?"
   ```

4. **Listen to response** (automatic TTS)

5. **Continue conversation** (say "Hey RustyClaw" again)

### Keyboard Shortcuts

- `Ctrl+V` - Toggle voice input
- `Ctrl+T` - Toggle TTS output
- `Ctrl+W` - Toggle wake word detection
- `Space` - Push-to-talk (hold to record)

---

## Provider Setup

### OpenAI (Easiest)

**Best for:** High-quality STT (Whisper) and decent TTS

```toml
[voice]
stt_provider = "openai"
tts_provider = "openai"

[voice.openai]
api_key = "${OPENAI_API_KEY}"
stt_model = "whisper-1"
tts_model = "tts-1-hd"  # or tts-1 for faster/cheaper
tts_voice = "nova"
```

**Get API key:** [https://platform.openai.com/api-keys](https://platform.openai.com/api-keys)

**Cost:**
- STT: $0.006 per minute
- TTS: $15-30 per 1M characters

### ElevenLabs (Best Quality)

**Best for:** Ultra-realistic TTS voices

```toml
[voice]
tts_provider = "elevenlabs"

[voice.elevenlabs]
api_key = "${ELEVENLABS_API_KEY}"
voice_id = "21m00Tcm4TlvDq8ikWAM"  # Rachel (default)
model = "eleven_multilingual_v2"
stability = 0.5
similarity_boost = 0.75
```

**Get API key:** [https://elevenlabs.io/app/settings/api-keys](https://elevenlabs.io/app/settings/api-keys)

**Browse voices:** [https://elevenlabs.io/voice-library](https://elevenlabs.io/voice-library)

**Cost:**
- Free tier: 10,000 characters/month
- Paid: $5-$330/month

### Google Cloud

**Best for:** Multiple languages and dialects

```bash
# Setup
gcloud auth application-default login
gcloud config set project YOUR_PROJECT_ID

# Enable APIs
gcloud services enable speech.googleapis.com
gcloud services enable texttospeech.googleapis.com
```

```toml
[voice]
stt_provider = "google"
tts_provider = "google"

[voice.google]
credentials_path = "~/.config/gcloud/application_default_credentials.json"
language_code = "en-US"
```

**Cost:**
- STT: $0.006 per 15 seconds
- TTS: $4-16 per 1M characters

### Azure Cognitive Services

**Best for:** Enterprise deployments

```toml
[voice]
stt_provider = "azure"
tts_provider = "azure"

[voice.azure]
subscription_key = "${AZURE_SPEECH_KEY}"
region = "eastus"
language = "en-US"
voice_name = "en-US-AriaNeural"
```

**Get key:** [https://portal.azure.com](https://portal.azure.com) → Cognitive Services → Speech

**Cost:**
- Free tier: 5 audio hours/month
- Paid: $1 per audio hour

### Local (Vosk + Coqui TTS)

**Best for:** Privacy, offline use, no API costs

```toml
[voice]
stt_provider = "vosk"
tts_provider = "coqui"

[voice.vosk]
model_path = "~/.rustyclaw/models/vosk-model-en-us-0.22"

[voice.coqui]
model = "tts_models/en/ljspeech/tacotron2-DDC"
```

**Download models:**
```bash
# Vosk STT
mkdir -p ~/.rustyclaw/models
cd ~/.rustyclaw/models
wget https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip
unzip vosk-model-small-en-us-0.15.zip

# Coqui TTS (auto-downloaded on first use)
```

**Cost:** Free!

**Trade-offs:**
- Lower quality than cloud providers
- Requires more compute (GPU recommended)
- Limited language support

---

## Implementation Guide

### Module Structure

```
src/voice/
├── mod.rs              # Public API
├── stt.rs              # Speech-to-Text providers
├── tts.rs              # Text-to-Speech providers
├── wake_word.rs        # Wake word detection
├── audio_device.rs     # Audio I/O handling
├── providers/
│   ├── openai.rs       # OpenAI Whisper + TTS
│   ├── elevenlabs.rs   # ElevenLabs TTS
│   ├── google.rs       # Google Cloud STT/TTS
│   ├── azure.rs        # Azure Cognitive Services
│   ├── vosk.rs         # Local Vosk STT
│   └── coqui.rs        # Local Coqui TTS
└── types.rs            # Common types
```

### Core Types

```rust
// src/voice/types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct AudioChunk {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttRequest {
    pub audio: Vec<u8>,  // WAV/MP3/OGG bytes
    pub language: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SttResponse {
    pub text: String,
    pub confidence: f32,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub voice: Option<String>,
    pub language: Option<String>,
    pub speed: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct TtsResponse {
    pub audio: Vec<u8>,  // WAV/MP3 bytes
    pub sample_rate: u32,
}

#[async_trait::async_trait]
pub trait SttProvider: Send + Sync {
    async fn transcribe(&self, request: SttRequest) -> Result<SttResponse>;
}

#[async_trait::async_trait]
pub trait TtsProvider: Send + Sync {
    async fn synthesize(&self, request: TtsRequest) -> Result<TtsResponse>;
}

pub trait WakeWordDetector: Send + Sync {
    fn detect(&mut self, audio: &AudioChunk) -> Result<bool>;
}
```

### OpenAI Implementation Example

```rust
// src/voice/providers/openai.rs
use super::*;
use reqwest::Client;
use anyhow::Result;

pub struct OpenAiVoice {
    api_key: String,
    stt_model: String,
    tts_model: String,
    tts_voice: String,
    http: Client,
}

impl OpenAiVoice {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            stt_model: "whisper-1".to_string(),
            tts_model: "tts-1".to_string(),
            tts_voice: "nova".to_string(),
            http: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl SttProvider for OpenAiVoice {
    async fn transcribe(&self, request: SttRequest) -> Result<SttResponse> {
        let form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(request.audio)
                .file_name("audio.wav")
                .mime_str("audio/wav")?)
            .text("model", self.stt_model.clone());

        let resp = self.http
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        Ok(SttResponse {
            text: resp["text"].as_str().unwrap_or("").to_string(),
            confidence: 1.0,  // Whisper doesn't provide confidence
            language: resp["language"].as_str().map(String::from),
        })
    }
}

#[async_trait::async_trait]
impl TtsProvider for OpenAiVoice {
    async fn synthesize(&self, request: TtsRequest) -> Result<TtsResponse> {
        let body = serde_json::json!({
            "model": self.tts_model,
            "voice": request.voice.unwrap_or(self.tts_voice.clone()),
            "input": request.text,
            "speed": request.speed.unwrap_or(1.0),
        });

        let audio = self.http
            .post("https://api.openai.com/v1/audio/speech")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .bytes()
            .await?
            .to_vec();

        Ok(TtsResponse {
            audio,
            sample_rate: 24000,  // OpenAI TTS default
        })
    }
}
```

### Usage in Gateway

```rust
// In src/gateway/mod.rs
use crate::voice::{SttProvider, TtsProvider, VoiceManager};

// Initialize voice manager
let voice_mgr = if config.voice.enabled {
    Some(VoiceManager::from_config(&config.voice).await?)
} else {
    None
};

// In chat loop
if let Some(ref voice) = voice_mgr {
    // Convert audio to text
    if message_type == "audio" {
        let audio_bytes = message.audio_data;
        let stt_result = voice.transcribe(audio_bytes).await?;
        message_text = stt_result.text;
    }

    // Convert response to audio
    if config.voice.tts_enabled {
        let tts_result = voice.synthesize(&response_text).await?;
        send_audio_frame(&tts_result.audio).await?;
    }
}
```

---

## Advanced Features

### Voice Commands

```toml
[voice.commands]
enabled = true

# Custom wake words
[voice.commands.patterns]
"hey rustyclaw" = "activate"
"rustyclaw stop" = "deactivate"
"rustyclaw mute" = "mute"
"rustyclaw louder" = "volume_up"
"rustyclaw quieter" = "volume_down"
```

### Multi-language Support

```toml
[voice]
language = "auto"  # Auto-detect

[voice.languages]
en-US = { stt = "openai", tts = "elevenlabs" }
es-ES = { stt = "google", tts = "google" }
fr-FR = { stt = "azure", tts = "azure" }
```

### Voice Profiles

```toml
[voice.profiles]
default = "professional"

[voice.profiles.professional]
tts_provider = "openai"
tts_voice = "nova"
speed = 1.0

[voice.profiles.casual]
tts_provider = "elevenlabs"
voice_id = "casual-voice-id"
stability = 0.3
```

---

## Troubleshooting

### No Audio Input

**Check devices:**
```bash
# List audio devices
rustyclaw voice --list-devices

# Test microphone
rustyclaw voice --test-mic
```

**Fix permissions:**
```bash
# macOS: Grant microphone access in System Preferences
# Linux: Add user to audio group
sudo usermod -a -G audio $USER
```

### Wake Word Not Detecting

**Lower sensitivity:**
```toml
[voice.wake_word]
sensitivity = 0.3  # Default 0.5, lower = more sensitive
```

**Check background noise:**
```bash
# Test with noise
rustyclaw voice --test-wake-word --show-volume
```

### Poor STT Quality

**Try different provider:**
```toml
[voice]
stt_provider = "openai"  # Generally best quality
```

**Improve audio quality:**
- Use external microphone
- Reduce background noise
- Speak clearly and closer to mic

### Slow TTS Response

**Use faster model:**
```toml
[voice.openai]
tts_model = "tts-1"  # Faster than tts-1-hd
```

**Or enable streaming:**
```toml
[voice]
tts_streaming = true  # Start playback before full generation
```

### High API Costs

**Use local models:**
```toml
[voice]
stt_provider = "vosk"  # Free, local
tts_provider = "coqui"  # Free, local
```

**Or optimize usage:**
```toml
[voice]
# Only use TTS for final responses, not intermediate thinking
tts_for_final_only = true

# Cache TTS for common phrases
tts_cache_enabled = true
```

---

## Security & Privacy

### Best Practices

1. **Use local models** for sensitive conversations:
   ```toml
   [voice]
   stt_provider = "vosk"
   tts_provider = "coqui"
   ```

2. **Disable cloud logging:**
   ```toml
   [voice.openai]
   disable_logging = true  # OpenAI doesn't log when this is set
   ```

3. **Use dedicated API keys:**
   ```toml
   [voice.elevenlabs]
   api_key = "${ELEVENLABS_VOICE_KEY}"  # Separate from main API key
   ```

4. **Encrypt audio cache:**
   ```toml
   [voice]
   cache_encryption = true
   cache_dir = "~/.rustyclaw/voice_cache"
   ```

---

## Performance Optimization

### Audio Buffer Tuning

```toml
[voice.audio]
buffer_size = 4096  # Larger = less CPU, more latency
sample_rate = 16000  # 16kHz sufficient for speech
```

### Parallel Processing

```toml
[voice]
# Process STT in background while TTS plays
parallel_processing = true

# Pre-load TTS for common responses
tts_preload = ["Hello!", "I'm thinking...", "Let me check."]
```

### GPU Acceleration (Local Models)

```toml
[voice.vosk]
use_gpu = true

[voice.coqui]
use_gpu = true
gpu_id = 0
```

---

## Testing

```bash
# Test voice pipeline
cargo test --features voice

# Test specific provider
cargo test voice::providers::openai --features voice

# Interactive test
rustyclaw voice --test-interactive

# Benchmark
cargo bench --features voice
```

---

## Roadmap

- [x] OpenAI Whisper STT integration
- [x] OpenAI TTS integration
- [x] ElevenLabs TTS integration
- [ ] Google Cloud STT/TTS
- [ ] Azure Cognitive Services
- [ ] Porcupine wake word detection
- [ ] Vosk local STT
- [ ] Coqui local TTS
- [ ] Streaming TTS (play while generating)
- [ ] Voice activity detection (VAD)
- [ ] Noise suppression
- [ ] Echo cancellation
- [ ] Multi-speaker support
- [ ] Voice cloning
- [ ] Emotion detection
- [ ] Custom wake word training

---

## Resources

- [OpenAI Speech API](https://platform.openai.com/docs/guides/speech-to-text)
- [ElevenLabs Docs](https://docs.elevenlabs.io/)
- [Google Cloud Speech](https://cloud.google.com/speech-to-text/docs)
- [Azure Speech Services](https://azure.microsoft.com/en-us/products/cognitive-services/speech-services/)
- [Vosk Documentation](https://alphacephei.com/vosk/)
- [Coqui TTS](https://github.com/coqui-ai/TTS)
- [Porcupine Wake Word](https://picovoice.ai/platform/porcupine/)

---

## Examples

See:
- `examples/voice_chat.rs` - Complete voice chat example
- `examples/wake_word_demo.rs` - Wake word detection demo
- `examples/custom_voice.rs` - Custom voice profile
- `scripts/voice-test.sh` - Audio testing script

---

**Ready to talk? 🗣️🦞**

```bash
rustyclaw chat --voice
```
