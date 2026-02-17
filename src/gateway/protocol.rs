//! Protocol types for gateway WebSocket communication.
//!
//! This module provides typed frame definitions for binary serialization
//! using bincode. The client and server are compiled together, so they
//! share the exact same types.
//!
//! ## Binary Protocol
//!
//! Frames are serialized using bincode and sent as WebSocket Binary messages.
//! Each frame has a type enum as the first field to allow dispatch.
//!
//! ## Backwards Compatibility
//!
//! The protocol supports receiving JSON text frames for backwards compatibility
//! with older versions. The receiver detects the format and handles accordingly.

use serde::{Deserialize, Serialize};

/// Incoming frame types from client to gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ClientFrameType {
    /// Authentication response with TOTP code.
    AuthResponse = 0,
    /// Unlock the vault with password.
    UnlockVault = 1,
    /// List all secrets.
    SecretsList = 2,
    /// Get a specific secret.
    SecretsGet = 3,
    /// Store a secret.
    SecretsStore = 4,
    /// Delete a secret.
    SecretsDelete = 5,
    /// Peek at a credential (display without exposing value).
    SecretsPeek = 6,
    /// Set access policy for a credential.
    SecretsSetPolicy = 7,
    /// Enable/disable a credential.
    SecretsSetDisabled = 8,
    /// Delete a credential entirely.
    SecretsDeleteCredential = 9,
    /// Check if TOTP is configured.
    SecretsHasTotp = 10,
    /// Set up TOTP for the vault.
    SecretsSetupTotp = 11,
    /// Verify a TOTP code.
    SecretsVerifyTotp = 12,
    /// Remove TOTP from the vault.
    SecretsRemoveTotp = 13,
    /// Reload configuration.
    Reload = 14,
    /// Cancel the current tool loop.
    Cancel = 15,
    /// Chat message (default).
    Chat = 16,
}

/// Outgoing frame types from gateway to client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ServerFrameType {
    /// Authentication challenge request.
    AuthChallenge = 0,
    /// Authentication result.
    AuthResult = 1,
    /// Too many auth attempts, locked out.
    AuthLocked = 2,
    /// Hello message on connect.
    Hello = 3,
    /// Status update frame.
    Status = 4,
    /// Vault unlocked result.
    VaultUnlocked = 5,
    /// Secrets list result.
    SecretsListResult = 6,
    /// Secrets store result.
    SecretsStoreResult = 7,
    /// Secrets get result.
    SecretsGetResult = 8,
    /// Secrets delete result.
    SecretsDeleteResult = 9,
    /// Secrets peek result.
    SecretsPeekResult = 10,
    /// Secrets set policy result.
    SecretsSetPolicyResult = 11,
    /// Secrets set disabled result.
    SecretsSetDisabledResult = 12,
    /// Secrets delete credential result.
    SecretsDeleteCredentialResult = 13,
    /// Secrets has TOTP result.
    SecretsHasTotpResult = 14,
    /// Secrets setup TOTP result.
    SecretsSetupTotpResult = 15,
    /// Secrets verify TOTP result.
    SecretsVerifyTotpResult = 16,
    /// Secrets remove TOTP result.
    SecretsRemoveTotpResult = 17,
    /// Reload result.
    ReloadResult = 18,
    /// Error frame.
    Error = 19,
    /// Info frame.
    Info = 20,
    /// Stream start.
    StreamStart = 21,
    /// Chunk of response text.
    Chunk = 22,
    /// Thinking start (for extended thinking).
    ThinkingStart = 23,
    /// Thinking delta (streaming thinking content).
    ThinkingDelta = 24,
    /// Thinking end.
    ThinkingEnd = 25,
    /// Tool call from model.
    ToolCall = 26,
    /// Tool result from execution.
    ToolResult = 27,
    /// Response complete.
    ResponseDone = 28,
}

/// Status frame sub-types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum StatusType {
    /// Model is configured.
    ModelConfigured = 0,
    /// Credentials loaded.
    CredentialsLoaded = 1,
    /// Credentials missing.
    CredentialsMissing = 2,
    /// Model connecting.
    ModelConnecting = 3,
    /// Model ready.
    ModelReady = 4,
    /// Model error.
    ModelError = 5,
    /// No model configured.
    NoModel = 6,
    /// Vault is locked.
    VaultLocked = 7,
}

// ============================================================================
// Binary Frame Types - these are the actual wire format
// ============================================================================

/// Generic client frame envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFrame {
    pub frame_type: ClientFrameType,
    pub payload: ClientPayload,
}

/// Payload variants for client frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientPayload {
    Empty,
    AuthResponse {
        code: String,
    },
    UnlockVault {
        password: String,
    },
    SecretsGet {
        key: String,
    },
    SecretsStore {
        key: String,
        value: String,
    },
    SecretsDelete {
        key: String,
    },
    SecretsPeek {
        name: String,
    },
    SecretsSetPolicy {
        name: String,
        policy: String,
        skills: Vec<String>,
    },
    SecretsSetDisabled {
        name: String,
        disabled: bool,
    },
    SecretsDeleteCredential {
        name: String,
    },
    SecretsVerifyTotp {
        code: String,
    },
}

/// Generic server frame envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerFrame {
    pub frame_type: ServerFrameType,
    pub payload: ServerPayload,
}

/// Payload variants for server frames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerPayload {
    Empty,
    Hello {
        agent: String,
        settings_dir: String,
        vault_locked: bool,
        provider: Option<String>,
        model: Option<String>,
    },
    AuthChallenge {
        method: String,
    },
    AuthResult {
        ok: bool,
        message: Option<String>,
        retry: Option<bool>,
    },
    AuthLocked {
        message: String,
        retry_after: Option<u64>,
    },
    Status {
        status: StatusType,
        detail: String,
    },
    VaultUnlocked {
        ok: bool,
        message: Option<String>,
    },
    SecretsListResult {
        ok: bool,
        entries: Vec<SecretEntryDto>,
    },
    SecretsStoreResult {
        ok: bool,
        message: String,
    },
    SecretsGetResult {
        ok: bool,
        key: String,
        value: Option<String>,
        message: Option<String>,
    },
    SecretsDeleteResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsPeekResult {
        ok: bool,
        fields: Vec<(String, String)>,
        message: Option<String>,
    },
    SecretsSetPolicyResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsSetDisabledResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsDeleteCredentialResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsHasTotpResult {
        has_totp: bool,
    },
    SecretsSetupTotpResult {
        ok: bool,
        uri: Option<String>,
        message: Option<String>,
    },
    SecretsVerifyTotpResult {
        ok: bool,
        message: Option<String>,
    },
    SecretsRemoveTotpResult {
        ok: bool,
        message: Option<String>,
    },
    ReloadResult {
        ok: bool,
        provider: String,
        model: String,
        message: Option<String>,
    },
    Error {
        ok: bool,
        message: String,
    },
    Info {
        message: String,
    },
    StreamStart,
    Chunk {
        delta: String,
    },
    ThinkingStart,
    ThinkingDelta {
        delta: String,
    },
    ThinkingEnd,
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolResult {
        id: String,
        name: String,
        result: String,
        is_error: bool,
    },
    ResponseDone {
        ok: bool,
    },
}

/// DTO for secret entries in list results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntryDto {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub policy: String,
    pub disabled: bool,
}

// ============================================================================
// Binary Serialization Helpers
// ============================================================================

/// Serialize a frame to binary using bincode with serde.
pub fn serialize_frame<T: serde::Serialize>(frame: &T) -> Result<Vec<u8>, String> {
    bincode::serde::encode_to_vec(frame, bincode::config::standard()).map_err(|e| e.to_string())
}

/// Deserialize a frame from binary using bincode with serde.
pub fn deserialize_frame<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let (result, _) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())
        .map_err(|e| e.to_string())?;
    Ok(result)
}

/// Helper to send a ServerFrame as a binary WebSocket message.
#[macro_export]
macro_rules! send_binary_frame {
    ($writer:expr, $frame:expr) => {{
        let bytes = $crate::gateway::serialize_frame(&$frame)
            .map_err(|e| anyhow::anyhow!("Failed to serialize frame: {}", e))?;
        $writer
            .send(tokio_tungstenite::tungstenite::Message::Binary(bytes))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send frame: {}", e))
    }};
}

/// Helper to parse a client frame from binary WebSocket message bytes.
#[macro_export]
macro_rules! parse_binary_client_frame {
    ($bytes:expr) => {{
        $crate::gateway::deserialize_frame::<$crate::gateway::ClientFrame>($bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse client frame: {}", e))
    }};
}
