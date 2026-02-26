//! Client-side protocol helpers.
//!
//! This module provides helpers for the TUI client to convert server frames
//! into application actions.

use crate::action::Action;
use rustyclaw_core::gateway::{ServerFrame, ServerPayload, StatusType};

/// Result of processing a server frame - includes optional action and whether to update UI.
pub struct FrameAction {
    pub action: Option<Action>,
    pub update_ui: bool,
}

impl FrameAction {
    pub fn none() -> Self {
        Self {
            action: None,
            update_ui: false,
        }
    }
    pub fn update(action: Action) -> Self {
        Self {
            action: Some(action),
            update_ui: true,
        }
    }
    pub fn just_action(action: Action) -> Self {
        Self {
            action: Some(action),
            update_ui: false,
        }
    }
}

/// Convert a server frame into TUI actions.
/// This encapsulates all the protocol parsing logic in one place.
pub fn server_frame_to_action(frame: &ServerFrame) -> FrameAction {
    use ServerPayload;

    match &frame.payload {
        ServerPayload::Hello { .. } => {
            FrameAction::just_action(Action::Info("Gateway connected.".into()))
        }
        ServerPayload::Status { status, detail } => {
            use StatusType::*;
            match status {
                ModelConfigured => {
                    FrameAction::just_action(Action::Info(format!("Model: {detail}")))
                }
                CredentialsLoaded => FrameAction::just_action(Action::Info(detail.clone())),
                CredentialsMissing => FrameAction::just_action(Action::Warning(detail.clone())),
                ModelConnecting => FrameAction::just_action(Action::Info(detail.clone())),
                ModelReady => FrameAction::just_action(Action::Success(detail.clone())),
                ModelError => FrameAction::just_action(Action::Error(detail.clone())),
                NoModel => FrameAction::just_action(Action::Warning(detail.clone())),
                VaultLocked => FrameAction::just_action(Action::GatewayVaultLocked),
            }
        }
        ServerPayload::AuthChallenge { .. } => {
            FrameAction::just_action(Action::GatewayAuthChallenge)
        }
        ServerPayload::AuthResult { ok, message, retry } => {
            if *ok {
                FrameAction::update(Action::GatewayAuthenticated)
            } else if retry.unwrap_or(false) {
                FrameAction::just_action(Action::Warning(
                    message
                        .clone()
                        .unwrap_or_else(|| "Invalid code. Try again.".into()),
                ))
            } else {
                FrameAction::just_action(Action::Error(
                    message
                        .clone()
                        .unwrap_or_else(|| "Authentication failed.".into()),
                ))
            }
        }
        ServerPayload::AuthLocked { message, .. } => {
            FrameAction::just_action(Action::Error(message.clone()))
        }
        ServerPayload::VaultUnlocked { ok, message } => {
            if *ok {
                FrameAction::update(Action::GatewayVaultUnlocked)
            } else {
                FrameAction::just_action(Action::Error(
                    message
                        .clone()
                        .unwrap_or_else(|| "Failed to unlock vault.".into()),
                ))
            }
        }
        ServerPayload::ReloadResult {
            ok,
            provider,
            model,
            message,
        } => {
            if *ok {
                FrameAction::just_action(Action::Success(format!(
                    "Gateway config reloaded: {} / {}",
                    provider, model
                )))
            } else {
                FrameAction::just_action(Action::Error(format!(
                    "Reload failed: {}",
                    message.as_deref().unwrap_or("Unknown error")
                )))
            }
        }
        ServerPayload::SecretsListResult { ok: _, entries } => {
            FrameAction::just_action(Action::SecretsListResult {
                entries: entries.clone(),
            })
        }
        ServerPayload::SecretsStoreResult { ok, message } => {
            FrameAction::just_action(Action::SecretsStoreResult {
                ok: *ok,
                message: message.clone(),
            })
        }
        ServerPayload::SecretsGetResult {
            ok: _, key, value, ..
        } => FrameAction::just_action(Action::SecretsGetResult {
            key: key.clone(),
            value: value.clone(),
        }),
        ServerPayload::SecretsPeekResult {
            ok,
            fields,
            message,
        } => FrameAction::just_action(Action::SecretsPeekResult {
            name: String::new(),
            ok: *ok,
            fields: fields.clone(),
            message: message.clone(),
        }),
        ServerPayload::SecretsSetPolicyResult { ok, message } => {
            FrameAction::just_action(Action::SecretsSetPolicyResult {
                ok: *ok,
                message: message.clone(),
            })
        }
        ServerPayload::SecretsSetDisabledResult { ok, message: _, .. } => {
            FrameAction::just_action(Action::SecretsSetDisabledResult {
                ok: *ok,
                cred_name: String::new(),
                disabled: false,
            })
        }
        ServerPayload::SecretsDeleteResult { ok, .. } => {
            FrameAction::just_action(Action::SecretsDeleteCredentialResult {
                ok: *ok,
                cred_name: String::new(),
            })
        }
        ServerPayload::SecretsDeleteCredentialResult { ok, .. } => {
            FrameAction::just_action(Action::SecretsDeleteCredentialResult {
                ok: *ok,
                cred_name: String::new(),
            })
        }
        ServerPayload::SecretsHasTotpResult { has_totp } => {
            FrameAction::just_action(Action::SecretsHasTotpResult {
                has_totp: *has_totp,
            })
        }
        ServerPayload::SecretsSetupTotpResult { ok, uri, message } => {
            FrameAction::just_action(Action::SecretsSetupTotpResult {
                ok: *ok,
                uri: uri.clone(),
                message: message.clone(),
            })
        }
        ServerPayload::SecretsVerifyTotpResult { ok, .. } => {
            FrameAction::just_action(Action::SecretsVerifyTotpResult { ok: *ok })
        }
        ServerPayload::SecretsRemoveTotpResult { ok, .. } => {
            FrameAction::just_action(Action::SecretsRemoveTotpResult { ok: *ok })
        }
        ServerPayload::StreamStart => FrameAction::just_action(Action::GatewayStreamStart),
        ServerPayload::ThinkingStart => FrameAction::just_action(Action::GatewayThinkingStart),
        ServerPayload::ThinkingDelta { .. } => {
            FrameAction::just_action(Action::GatewayThinkingDelta)
        }
        ServerPayload::ThinkingEnd => FrameAction::just_action(Action::GatewayThinkingEnd),
        ServerPayload::Chunk { delta } => {
            FrameAction::just_action(Action::GatewayChunk(delta.clone()))
        }
        ServerPayload::ResponseDone { .. } => FrameAction::just_action(Action::GatewayResponseDone),
        ServerPayload::ToolCall {
            id,
            name,
            arguments,
        } => FrameAction::just_action(Action::GatewayToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        }),
        ServerPayload::ToolResult {
            id,
            name,
            result,
            is_error,
        } => FrameAction::just_action(Action::GatewayToolResult {
            id: id.clone(),
            name: name.clone(),
            result: result.clone(),
            is_error: *is_error,
        }),
        ServerPayload::Error { message, .. } => {
            FrameAction::just_action(Action::Error(message.clone()))
        }
        ServerPayload::Info { message } => FrameAction::just_action(Action::Info(message.clone())),
        ServerPayload::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => FrameAction::just_action(Action::ToolApprovalRequest {
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        }),
        ServerPayload::UserPromptRequest { id, prompt } => {
            let mut prompt = prompt.clone();
            prompt.id = id.clone();
            FrameAction::just_action(Action::UserPromptRequest(prompt))
        }
        ServerPayload::TasksUpdate { tasks } => FrameAction::just_action(Action::TasksUpdate(
            tasks
                .iter()
                .map(|t| crate::action::TaskInfo {
                    id: t.id,
                    label: t.label.clone(),
                    description: t.description.clone(),
                    status: t.status.clone(),
                    is_foreground: t.is_foreground,
                })
                .collect(),
        )),
        ServerPayload::ThreadsUpdate {
            threads,
            foreground_id,
        } => FrameAction::just_action(Action::ThreadsUpdate {
            threads: threads
                .iter()
                .map(|t| crate::action::ThreadInfo {
                    id: t.id,
                    label: t.label.clone(),
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                    has_summary: t.has_summary,
                })
                .collect(),
            foreground_id: *foreground_id,
        }),
        ServerPayload::ThreadCreated { thread_id, label } => {
            // Thread was created — we'll get a ThreadsUpdate too
            FrameAction::none()
        }
        ServerPayload::ThreadSwitched {
            thread_id,
            context_summary,
        } => {
            // Thread was switched — we'll get a ThreadsUpdate too
            // Could show the context_summary in messages if desired
            FrameAction::none()
        }
        ServerPayload::Empty => FrameAction::none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;

    #[test]
    fn test_frame_action_none() {
        let action = FrameAction::none();
        assert!(action.action.is_none());
        assert!(!action.update_ui);
    }

    #[test]
    fn test_frame_action_update() {
        let action = FrameAction::update(Action::Update);
        assert!(matches!(action.action, Some(Action::Update)));
        assert!(action.update_ui);
    }

    #[test]
    fn test_frame_action_just_action() {
        let action = FrameAction::just_action(Action::Tick);
        assert!(matches!(action.action, Some(Action::Tick)));
        assert!(!action.update_ui);
    }

    mod action_conversion {
        use super::*;
        use crate::action::Action;
        use rustyclaw_core::gateway::{SecretEntryDto, ServerFrameType};

        #[test]
        fn test_hello_frame_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Hello,
                payload: ServerPayload::Hello {
                    agent: "test".into(),
                    settings_dir: "/tmp".into(),
                    vault_locked: false,
                    provider: None,
                    model: None,
                },
            };

            let result = server_frame_to_action(&frame);
            assert!(matches!(result.action, Some(Action::Info(_))));
        }

        #[test]
        fn test_status_model_ready_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Status,
                payload: ServerPayload::Status {
                    status: StatusType::ModelReady,
                    detail: "Claude 3.5 Sonnet".into(),
                },
            };

            let result = server_frame_to_action(&frame);
            assert!(matches!(result.action, Some(Action::Success(_))));
        }

        #[test]
        fn test_status_vault_locked_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Status,
                payload: ServerPayload::Status {
                    status: StatusType::VaultLocked,
                    detail: "Vault is locked".into(),
                },
            };

            let result = server_frame_to_action(&frame);
            assert!(matches!(result.action, Some(Action::GatewayVaultLocked)));
        }

        #[test]
        fn test_chunk_frame_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Chunk,
                payload: ServerPayload::Chunk {
                    delta: "Hello".into(),
                },
            };

            let result = server_frame_to_action(&frame);
            match result.action {
                Some(Action::GatewayChunk(text)) => assert_eq!(text, "Hello"),
                _ => panic!("Expected GatewayChunk action"),
            }
        }

        #[test]
        fn test_tool_call_frame_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::ToolCall,
                payload: ServerPayload::ToolCall {
                    id: "call_001".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"/tmp/test"}"#.into(),
                },
            };

            let result = server_frame_to_action(&frame);
            match result.action {
                Some(Action::GatewayToolCall {
                    id,
                    name,
                    arguments: _,
                }) => {
                    assert_eq!(id, "call_001");
                    assert_eq!(name, "read_file");
                }
                _ => panic!("Expected GatewayToolCall action"),
            }
        }

        #[test]
        fn test_error_frame_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::Error,
                payload: ServerPayload::Error {
                    ok: false,
                    message: "Connection failed".into(),
                },
            };

            let result = server_frame_to_action(&frame);
            match result.action {
                Some(Action::Error(msg)) => assert_eq!(msg, "Connection failed"),
                _ => panic!("Expected Error action"),
            }
        }

        #[test]
        fn test_secrets_list_result_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::SecretsListResult,
                payload: ServerPayload::SecretsListResult {
                    ok: true,
                    entries: vec![SecretEntryDto {
                        name: "api_key".into(),
                        label: "API Key".into(),
                        kind: "ApiKey".into(),
                        policy: "always".into(),
                        disabled: false,
                    }],
                },
            };

            let result = server_frame_to_action(&frame);
            match result.action {
                Some(Action::SecretsListResult { entries }) => {
                    assert_eq!(entries.len(), 1);
                }
                _ => panic!("Expected SecretsListResult action"),
            }
        }

        #[test]
        fn test_auth_challenge_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::AuthChallenge,
                payload: ServerPayload::AuthChallenge {
                    method: "totp".into(),
                },
            };

            let result = server_frame_to_action(&frame);
            assert!(matches!(result.action, Some(Action::GatewayAuthChallenge)));
        }

        #[test]
        fn test_response_done_to_action() {
            let frame = ServerFrame {
                frame_type: ServerFrameType::ResponseDone,
                payload: ServerPayload::ResponseDone { ok: true },
            };

            let result = server_frame_to_action(&frame);
            assert!(matches!(result.action, Some(Action::GatewayResponseDone)));
        }

        #[test]
        fn test_streaming_frames_to_actions() {
            let start_frame = ServerFrame {
                frame_type: ServerFrameType::StreamStart,
                payload: ServerPayload::StreamStart,
            };
            assert!(matches!(
                server_frame_to_action(&start_frame).action,
                Some(Action::GatewayStreamStart)
            ));

            let thinking_frame = ServerFrame {
                frame_type: ServerFrameType::ThinkingStart,
                payload: ServerPayload::ThinkingStart,
            };
            assert!(matches!(
                server_frame_to_action(&thinking_frame).action,
                Some(Action::GatewayThinkingStart)
            ));

            let end_frame = ServerFrame {
                frame_type: ServerFrameType::ThinkingEnd,
                payload: ServerPayload::ThinkingEnd,
            };
            assert!(matches!(
                server_frame_to_action(&end_frame).action,
                Some(Action::GatewayThinkingEnd)
            ));
        }
    }
}
