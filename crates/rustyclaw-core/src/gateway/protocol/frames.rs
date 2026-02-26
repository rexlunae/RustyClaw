//! Frame types and serialization for gateway protocol.
//!
//! This module contains the shared types used by both client and server.

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
    /// User response to a tool approval request.
    ToolApprovalResponse = 17,
    /// User response to a structured prompt (ask_user tool).
    UserPromptResponse = 18,
    /// Request current task list.
    TasksRequest = 19,
    /// Create a new thread.
    ThreadCreate = 20,
    /// Switch to a different thread.
    ThreadSwitch = 21,
    /// Request thread list.
    ThreadList = 22,
    /// Close/delete a thread.
    ThreadClose = 23,
    /// Rename a thread.
    ThreadRename = 24,
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
    /// Tool approval request — ask user to approve a tool call.
    ToolApprovalRequest = 29,
    /// Structured user prompt request (ask_user tool).
    UserPromptRequest = 30,
    /// Task list update.
    TasksUpdate = 31,
    /// Thread list update.
    ThreadsUpdate = 32,
    /// Thread created result.
    ThreadCreated = 33,
    /// Thread switched result.
    ThreadSwitched = 34,
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
// Binary Frame Types
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
    AuthChallenge {
        method: String,
    },
    AuthResponse {
        code: String,
    },
    UnlockVault {
        password: String,
    },
    Reload,
    Chat {
        messages: Vec<super::types::ChatMessage>,
    },
    SecretsList,
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
    SecretsHasTotp,
    SecretsSetupTotp,
    SecretsVerifyTotp {
        code: String,
    },
    SecretsRemoveTotp,
    ToolApprovalResponse {
        id: String,
        approved: bool,
    },
    UserPromptResponse {
        id: String,
        dismissed: bool,
        value: crate::user_prompt_types::PromptResponseValue,
    },
    /// Request current task list (optionally filtered by session).
    TasksRequest {
        session: Option<String>,
    },
    /// Create a new thread.
    ThreadCreate {
        label: String,
    },
    /// Switch to a different thread.
    ThreadSwitch {
        thread_id: u64,
    },
    /// Request list of threads.
    ThreadList,
    /// Close/delete a thread.
    ThreadClose {
        thread_id: u64,
    },
    /// Rename a thread.
    ThreadRename {
        thread_id: u64,
        new_label: String,
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
        arguments: String,
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
    ToolApprovalRequest {
        id: String,
        name: String,
        arguments: String,
    },
    UserPromptRequest {
        id: String,
        prompt: crate::user_prompt_types::UserPrompt,
    },
    TasksUpdate {
        tasks: Vec<TaskInfoDto>,
    },
    ThreadsUpdate {
        threads: Vec<ThreadInfoDto>,
        foreground_id: Option<u64>,
    },
    ThreadCreated {
        thread_id: u64,
        label: String,
    },
    ThreadSwitched {
        thread_id: u64,
        /// Optional summary of the thread being switched to
        context_summary: Option<String>,
    },
}

/// DTO for task info in updates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskInfoDto {
    pub id: u64,
    pub label: String,
    pub description: Option<String>,
    pub status: String,
    pub is_foreground: bool,
}

/// DTO for thread info in updates (unified tasks + threads).
/// NOTE: Do NOT use skip_serializing_if with bincode - it breaks deserialization
/// since bincode is not self-describing (positional format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadInfoDto {
    pub id: u64,
    pub label: String,
    /// Description (for spawned tasks)
    pub description: Option<String>,
    /// Task status (None = simple thread, Some = spawned task)
    pub status: Option<String>,
    pub is_foreground: bool,
    pub message_count: usize,
    pub has_summary: bool,
}

/// DTO for secret entries in list results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecretEntryDto {
    pub name: String,
    pub label: String,
    pub kind: String,
    pub policy: String,
    pub disabled: bool,
}

// ============================================================================
// Serialization
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

#[cfg(test)]
mod tests {
    use super::*;

    mod serialization {
        use super::*;

        #[test]
        fn test_server_frame_type_values() {
            assert_eq!(ServerFrameType::AuthChallenge as u8, 0);
            assert_eq!(ServerFrameType::AuthResult as u8, 1);
            assert_eq!(ServerFrameType::AuthLocked as u8, 2);
            assert_eq!(ServerFrameType::Hello as u8, 3);
            assert_eq!(ServerFrameType::Status as u8, 4);
            assert_eq!(ServerFrameType::VaultUnlocked as u8, 5);
            assert_eq!(ServerFrameType::SecretsListResult as u8, 6);
            assert_eq!(ServerFrameType::SecretsStoreResult as u8, 7);
            assert_eq!(ServerFrameType::SecretsGetResult as u8, 8);
            assert_eq!(ServerFrameType::SecretsDeleteResult as u8, 9);
            assert_eq!(ServerFrameType::SecretsPeekResult as u8, 10);
            assert_eq!(ServerFrameType::SecretsSetPolicyResult as u8, 11);
            assert_eq!(ServerFrameType::SecretsSetDisabledResult as u8, 12);
            assert_eq!(ServerFrameType::SecretsDeleteCredentialResult as u8, 13);
            assert_eq!(ServerFrameType::SecretsHasTotpResult as u8, 14);
            assert_eq!(ServerFrameType::SecretsSetupTotpResult as u8, 15);
            assert_eq!(ServerFrameType::SecretsVerifyTotpResult as u8, 16);
            assert_eq!(ServerFrameType::SecretsRemoveTotpResult as u8, 17);
            assert_eq!(ServerFrameType::ReloadResult as u8, 18);
            assert_eq!(ServerFrameType::Error as u8, 19);
            assert_eq!(ServerFrameType::Info as u8, 20);
            assert_eq!(ServerFrameType::StreamStart as u8, 21);
            assert_eq!(ServerFrameType::Chunk as u8, 22);
            assert_eq!(ServerFrameType::ThinkingStart as u8, 23);
            assert_eq!(ServerFrameType::ThinkingDelta as u8, 24);
            assert_eq!(ServerFrameType::ThinkingEnd as u8, 25);
            assert_eq!(ServerFrameType::ToolCall as u8, 26);
            assert_eq!(ServerFrameType::ToolResult as u8, 27);
            assert_eq!(ServerFrameType::ResponseDone as u8, 28);
            assert_eq!(ServerFrameType::ToolApprovalRequest as u8, 29);
            assert_eq!(ServerFrameType::UserPromptRequest as u8, 30);
        }

        #[test]
        fn test_client_frame_type_values() {
            assert_eq!(ClientFrameType::AuthResponse as u8, 0);
            assert_eq!(ClientFrameType::UnlockVault as u8, 1);
            assert_eq!(ClientFrameType::SecretsList as u8, 2);
            assert_eq!(ClientFrameType::SecretsGet as u8, 3);
            assert_eq!(ClientFrameType::SecretsStore as u8, 4);
            assert_eq!(ClientFrameType::SecretsDelete as u8, 5);
            assert_eq!(ClientFrameType::SecretsPeek as u8, 6);
            assert_eq!(ClientFrameType::SecretsSetPolicy as u8, 7);
            assert_eq!(ClientFrameType::SecretsSetDisabled as u8, 8);
            assert_eq!(ClientFrameType::SecretsDeleteCredential as u8, 9);
            assert_eq!(ClientFrameType::SecretsHasTotp as u8, 10);
            assert_eq!(ClientFrameType::SecretsSetupTotp as u8, 11);
            assert_eq!(ClientFrameType::SecretsVerifyTotp as u8, 12);
            assert_eq!(ClientFrameType::SecretsRemoveTotp as u8, 13);
            assert_eq!(ClientFrameType::Reload as u8, 14);
            assert_eq!(ClientFrameType::Cancel as u8, 15);
            assert_eq!(ClientFrameType::Chat as u8, 16);
            assert_eq!(ClientFrameType::ToolApprovalResponse as u8, 17);
            assert_eq!(ClientFrameType::UserPromptResponse as u8, 18);
        }

        #[test]
        fn test_status_type_values() {
            assert_eq!(StatusType::ModelConfigured as u8, 0);
            assert_eq!(StatusType::CredentialsLoaded as u8, 1);
            assert_eq!(StatusType::CredentialsMissing as u8, 2);
            assert_eq!(StatusType::ModelConnecting as u8, 3);
            assert_eq!(StatusType::ModelReady as u8, 4);
            assert_eq!(StatusType::ModelError as u8, 5);
            assert_eq!(StatusType::NoModel as u8, 6);
            assert_eq!(StatusType::VaultLocked as u8, 7);
        }

        #[test]
        fn test_server_frame_roundtrip_hello() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Hello,
                payload: ServerPayload::Hello {
                    agent: "test-agent".into(),
                    settings_dir: "/tmp/settings".into(),
                    vault_locked: false,
                    provider: Some("anthropic".into()),
                    model: Some("claude-3".into()),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            match decoded.payload {
                ServerPayload::Hello {
                    agent,
                    settings_dir,
                    vault_locked,
                    provider,
                    model,
                } => {
                    assert_eq!(agent, "test-agent");
                    assert_eq!(settings_dir, "/tmp/settings");
                    assert!(!vault_locked);
                    assert_eq!(provider, Some("anthropic".into()));
                    assert_eq!(model, Some("claude-3".into()));
                }
                _ => panic!("Expected Hello payload"),
            }
        }

        #[test]
        fn test_server_frame_roundtrip_chunk() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Chunk,
                payload: ServerPayload::Chunk {
                    delta: "Hello, world!".into(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            match decoded.payload {
                ServerPayload::Chunk { delta } => {
                    assert_eq!(delta, "Hello, world!");
                }
                _ => panic!("Expected Chunk payload"),
            }
        }

        #[test]
        fn test_server_frame_roundtrip_status() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Status,
                payload: ServerPayload::Status {
                    status: StatusType::ModelReady,
                    detail: "Connected to Claude 3.5 Sonnet".into(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            match decoded.payload {
                ServerPayload::Status { status, detail } => {
                    assert_eq!(status, StatusType::ModelReady);
                    assert_eq!(detail, "Connected to Claude 3.5 Sonnet");
                }
                _ => panic!("Expected Status payload"),
            }
        }

        #[test]
        fn test_server_frame_roundtrip_auth_result() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::AuthResult,
                payload: ServerPayload::AuthResult {
                    ok: true,
                    message: Some("Authenticated successfully".into()),
                    retry: None,
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            match decoded.payload {
                ServerPayload::AuthResult { ok, message, retry } => {
                    assert!(ok);
                    assert_eq!(message, Some("Authenticated successfully".into()));
                    assert!(retry.is_none());
                }
                _ => panic!("Expected AuthResult payload"),
            }
        }

        #[test]
        fn test_client_frame_roundtrip_chat() {
            let frame = ClientFrame {
                frame_type: ClientFrameType::Chat,
                payload: ClientPayload::Empty,
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ClientFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            assert_eq!(decoded.frame_type, ClientFrameType::Chat);
            matches!(decoded.payload, ClientPayload::Empty);
        }

        #[test]
        fn test_client_frame_roundtrip_secrets_store() {
            let frame = ClientFrame {
                frame_type: ClientFrameType::SecretsStore,
                payload: ClientPayload::SecretsStore {
                    key: "OPENAI_API_KEY".into(),
                    value: "sk-test123".into(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ClientFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            match decoded.payload {
                ClientPayload::SecretsStore { key, value } => {
                    assert_eq!(key, "OPENAI_API_KEY");
                    assert_eq!(value, "sk-test123");
                }
                _ => panic!("Expected SecretsStore payload"),
            }
        }

        #[test]
        fn test_secret_entry_dto_roundtrip() {
            let entry = SecretEntryDto {
                name: "api_key".into(),
                label: "OpenAI API Key".into(),
                kind: "ApiKey".into(),
                policy: "always".into(),
                disabled: false,
            };

            let json = serde_json::to_string(&entry).expect("JSON serialize should succeed");
            let decoded: SecretEntryDto =
                serde_json::from_str(&json).expect("JSON deserialize should succeed");

            assert_eq!(decoded.name, "api_key");
            assert_eq!(decoded.label, "OpenAI API Key");
            assert_eq!(decoded.kind, "ApiKey");
            assert_eq!(decoded.policy, "always");
            assert!(!decoded.disabled);
        }

        #[test]
        fn test_user_prompt_response_bincode_roundtrip() {
            use crate::user_prompt_types::PromptResponseValue;

            let frame = ClientFrame {
                frame_type: ClientFrameType::UserPromptResponse,
                payload: ClientPayload::UserPromptResponse {
                    id: "call_456".into(),
                    dismissed: false,
                    value: PromptResponseValue::Text("hello world".into()),
                },
            };
            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ClientFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");
            match decoded.payload {
                ClientPayload::UserPromptResponse {
                    id,
                    dismissed,
                    value,
                } => {
                    assert_eq!(id, "call_456");
                    assert!(!dismissed);
                    assert_eq!(value, PromptResponseValue::Text("hello world".into()));
                }
                _ => panic!("Expected UserPromptResponse payload"),
            }
        }

        #[test]
        fn test_server_user_prompt_request_bincode_roundtrip() {
            use crate::user_prompt_types::{PromptType, UserPrompt};

            let prompt = UserPrompt {
                id: "call_789".into(),
                title: "What is your name?".into(),
                description: Some("Please enter your full name".into()),
                prompt_type: PromptType::TextInput {
                    placeholder: Some("John Doe".into()),
                    default: None,
                },
            };

            let frame = ServerFrame {
                frame_type: ServerFrameType::UserPromptRequest,
                payload: ServerPayload::UserPromptRequest {
                    id: "call_789".into(),
                    prompt: prompt.clone(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            assert_eq!(decoded.frame_type, ServerFrameType::UserPromptRequest);
            match decoded.payload {
                ServerPayload::UserPromptRequest { id, prompt: p } => {
                    assert_eq!(id, "call_789");
                    assert_eq!(p.title, "What is your name?");
                    assert_eq!(p.description, Some("Please enter your full name".into()));
                    assert!(matches!(p.prompt_type, PromptType::TextInput { .. }));
                }
                _ => panic!("Expected UserPromptRequest payload"),
            }
        }

        #[test]
        fn test_client_frame_roundtrip_auth_response() {
            let frame = ClientFrame {
                frame_type: ClientFrameType::AuthResponse,
                payload: ClientPayload::AuthResponse {
                    code: "123456".into(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ClientFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            assert_eq!(decoded.frame_type, ClientFrameType::AuthResponse);
            match decoded.payload {
                ClientPayload::AuthResponse { code } => {
                    assert_eq!(code, "123456");
                }
                _ => panic!("Expected AuthResponse payload"),
            }
        }

        #[test]
        fn test_server_frame_roundtrip_auth_challenge() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::AuthChallenge,
                payload: ServerPayload::AuthChallenge {
                    method: "totp".into(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            assert_eq!(decoded.frame_type, ServerFrameType::AuthChallenge);
            match decoded.payload {
                ServerPayload::AuthChallenge { method } => {
                    assert_eq!(method, "totp");
                }
                _ => panic!("Expected AuthChallenge payload"),
            }
        }

        #[test]
        fn test_server_tool_call_bincode_roundtrip() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::ToolCall,
                payload: ServerPayload::ToolCall {
                    id: "call_001".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"/tmp/test"}"#.into(),
                },
            };

            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ServerFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");

            match decoded.payload {
                ServerPayload::ToolCall {
                    id,
                    name,
                    arguments,
                } => {
                    assert_eq!(id, "call_001");
                    assert_eq!(name, "read_file");
                    assert_eq!(arguments, r#"{"path":"/tmp/test"}"#);
                }
                _ => panic!("Expected ToolCall payload"),
            }
        }
    }
}

#[cfg(test)]
mod frame_size_tests {
    use super::*;

    #[test]
    fn test_threads_update_size() {
        let thread = ThreadInfoDto {
            id: 1,
            label: "Main".to_string(),
            description: None,
            status: None,
            is_foreground: true,
            message_count: 0,
            has_summary: false,
        };
        
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadsUpdate,
            payload: ServerPayload::ThreadsUpdate {
                threads: vec![thread],
                foreground_id: Some(1),
            },
        };
        
        let bytes = serialize_frame(&frame).unwrap();
        println!("ThreadsUpdate with 1 thread: {} bytes", bytes.len());
        println!("Bytes: {:?}", bytes);
        
        // With bincode standard config (varint encoding), small values are compact.
        // 16 bytes is correct for this minimal frame.
        // Key test: can we deserialize it without error?
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("Round-trip deserialization failed");
        
        // Verify we got the right frame type
        assert!(matches!(decoded.frame_type, ServerFrameType::ThreadsUpdate));
        if let ServerPayload::ThreadsUpdate { threads, foreground_id } = decoded.payload {
            assert_eq!(threads.len(), 1);
            assert_eq!(threads[0].id, 1);
            assert_eq!(threads[0].label, "Main");
            assert_eq!(threads[0].description, None);
            assert_eq!(threads[0].status, None);
            assert_eq!(threads[0].is_foreground, true);
            assert_eq!(threads[0].message_count, 0);
            assert_eq!(threads[0].has_summary, false);
            assert_eq!(foreground_id, Some(1));
        } else {
            panic!("Wrong payload type");
        }
    }
}
