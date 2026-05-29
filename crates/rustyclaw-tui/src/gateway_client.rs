//! Client-side protocol helpers.
//!
//! This module provides helpers for the TUI client to convert server frames
//! into application actions.

use crate::action::Action;
use rustyclaw_core::gateway::{GatewayEvent, SecretEntryDto, ServerFrame, ServerPayload};

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
///
/// Wire-frame parsing is delegated to the shared
/// [`GatewayEvent::from_server_frame`] so the TUI and desktop clients agree on
/// exactly one mapping from the binary protocol to client-facing events. This
/// function only adapts that shared [`GatewayEvent`] into the TUI's [`Action`]
/// enum (see [`event_to_action`]).
///
/// `ThreadsUpdate` is the one exception: its frame carries the rich wire
/// [`rustyclaw_core::gateway::protocol::ThreadInfoDto`] (kind/status icons,
/// summary flag) that the client-facing event DTO drops, and the TUI sidebar
/// renders those fields. So it is handled directly from the frame to keep them.
pub fn server_frame_to_action(frame: &ServerFrame) -> FrameAction {
    if let ServerPayload::ThreadsUpdate {
        threads,
        foreground_id,
    } = &frame.payload
    {
        return FrameAction::just_action(Action::ThreadsUpdate {
            threads: threads.clone(),
            foreground_id: *foreground_id,
        });
    }

    match GatewayEvent::from_server_frame(frame.clone()) {
        Some(event) => event_to_action(event),
        None => FrameAction::none(),
    }
}

/// Adapt a shared [`GatewayEvent`] into the TUI's [`Action`] enum.
///
/// This is the TUI-specific presentation mapping; it deliberately has no
/// knowledge of the wire protocol. The `update_ui` flag is set only for the
/// connection-lifecycle events that previously forced an immediate redraw.
fn event_to_action(event: GatewayEvent) -> FrameAction {
    use GatewayEvent as E;

    match event {
        E::Connected { .. } => FrameAction::update(Action::GatewayConnected),
        E::Disconnected { reason } => {
            FrameAction::just_action(Action::GatewayDisconnected(reason.unwrap_or_default()))
        }
        E::AuthRequired => FrameAction::just_action(Action::GatewayAuthChallenge),
        E::AuthSuccess => FrameAction::update(Action::GatewayAuthenticated),
        E::AuthFailed { message, retry } => {
            if retry {
                FrameAction::just_action(Action::Warning(if message.is_empty() {
                    "Invalid code. Try again.".into()
                } else {
                    message
                }))
            } else {
                FrameAction::just_action(Action::Error(if message.is_empty() {
                    "Authentication failed.".into()
                } else {
                    message
                }))
            }
        }
        E::VaultLocked => FrameAction::just_action(Action::GatewayVaultLocked),
        E::VaultUnlocked => FrameAction::update(Action::GatewayVaultUnlocked),
        E::ModelReady { model } => FrameAction::just_action(Action::Success(model)),
        E::ModelError { message } => FrameAction::just_action(Action::Error(message)),
        E::ModelReloaded { provider, model } => {
            FrameAction::just_action(Action::GatewayReloaded { provider, model })
        }
        E::StreamStart => FrameAction::just_action(Action::GatewayStreamStart),
        E::ThinkingStart => FrameAction::just_action(Action::GatewayThinkingStart),
        E::ThinkingDelta => FrameAction::just_action(Action::GatewayThinkingDelta),
        E::ThinkingEnd => FrameAction::just_action(Action::GatewayThinkingEnd),
        E::Chunk { delta } => FrameAction::just_action(Action::GatewayChunk(delta)),
        E::ResponseDone => FrameAction::just_action(Action::GatewayResponseDone),
        E::ToolCall {
            id,
            name,
            arguments,
        } => FrameAction::just_action(Action::GatewayToolCall {
            id,
            name,
            arguments,
        }),
        E::ToolResult {
            id,
            name,
            result,
            is_error,
        } => FrameAction::just_action(Action::GatewayToolResult {
            id,
            name,
            result,
            is_error,
        }),
        E::ToolApprovalRequest {
            id,
            name,
            arguments,
        } => FrameAction::just_action(Action::ToolApprovalRequest {
            id,
            name,
            arguments,
        }),
        E::UserPromptRequest { prompt, .. } => {
            FrameAction::just_action(Action::UserPromptRequest(prompt))
        }
        E::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => FrameAction::just_action(Action::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        }),
        E::DeviceFlowStart { url, code, .. } => {
            FrameAction::just_action(Action::DeviceFlowCodeReady { url, code })
        }
        E::DeviceFlowComplete => FrameAction::just_action(Action::DeviceFlowComplete),
        E::ThreadMessages {
            thread_id,
            messages,
        } => FrameAction::just_action(Action::ThreadMessages {
            thread_id,
            messages,
        }),
        E::ThreadSwitched {
            thread_id,
            context_summary,
        } => FrameAction::just_action(Action::ThreadSwitched {
            thread_id,
            context_summary,
        }),
        E::ThreadHistory {
            thread_id,
            ok,
            messages,
            error,
        } => FrameAction::just_action(Action::ThreadHistory {
            thread_id,
            ok,
            messages,
            error,
        }),
        E::Error { message } => FrameAction::just_action(Action::Error(message)),
        E::Info { message } => FrameAction::just_action(Action::Info(message)),
        E::Warning { message } => FrameAction::just_action(Action::Warning(message)),
        // DOM queries require a webview; the TUI cannot evaluate JS, so ignore.
        E::DomQuery { .. } => FrameAction::none(),
        E::SecretsListResult { entries, .. } => FrameAction::just_action(Action::SecretsListResult {
            // `SecretEntryInfo` and the wire `SecretEntryDto` are field-for-field
            // identical, so this round-trips losslessly.
            entries: entries
                .into_iter()
                .map(|e| SecretEntryDto {
                    name: e.name,
                    label: e.label,
                    kind: e.kind,
                    policy: e.policy,
                    disabled: e.disabled,
                })
                .collect(),
        }),
        E::SecretsStoreResult { ok, message } => {
            FrameAction::just_action(Action::SecretsStoreResult { ok, message })
        }
        E::SecretsGetResult { key, value } => {
            FrameAction::just_action(Action::SecretsGetResult { key, value })
        }
        E::SecretsDeleteResult { ok, .. } | E::SecretsDeleteCredentialResult { ok } => {
            FrameAction::just_action(Action::SecretsDeleteCredentialResult {
                ok,
                cred_name: String::new(),
            })
        }
        E::SecretsPeekResult {
            ok,
            fields,
            message,
        } => FrameAction::just_action(Action::SecretsPeekResult {
            name: String::new(),
            ok,
            fields,
            message,
        }),
        E::SecretsSetPolicyResult { ok, message } => {
            FrameAction::just_action(Action::SecretsSetPolicyResult { ok, message })
        }
        E::SecretsSetDisabledResult { ok } => {
            FrameAction::just_action(Action::SecretsSetDisabledResult {
                ok,
                cred_name: String::new(),
                disabled: false,
            })
        }
        E::SecretsHasTotpResult { has_totp } => {
            FrameAction::just_action(Action::SecretsHasTotpResult { has_totp })
        }
        E::SecretsSetupTotpResult { ok, uri, message } => {
            FrameAction::just_action(Action::SecretsSetupTotpResult { ok, uri, message })
        }
        E::SecretsVerifyTotpResult { ok } => {
            FrameAction::just_action(Action::SecretsVerifyTotpResult { ok })
        }
        E::SecretsRemoveTotpResult { ok } => {
            FrameAction::just_action(Action::SecretsRemoveTotpResult { ok })
        }
        // `ThreadsUpdate` is intercepted at the frame level in
        // `server_frame_to_action` to preserve the rich wire DTO, so this arm
        // is unreachable in practice. Kept (with a best-effort, icon-less
        // conversion) only to keep the match exhaustive.
        E::ThreadsUpdate {
            threads,
            foreground_id,
        } => FrameAction::just_action(Action::ThreadsUpdate {
            threads: threads
                .into_iter()
                .map(|t| crate::action::ThreadInfo {
                    id: t.id,
                    label: t.label.unwrap_or_default(),
                    description: t.description,
                    status: if t.status.is_empty() {
                        None
                    } else {
                        Some(t.status)
                    },
                    kind_icon: None,
                    status_icon: None,
                    is_foreground: t.is_foreground,
                    message_count: t.message_count,
                    has_summary: false,
                })
                .collect(),
            foreground_id,
        }),
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
        use rustyclaw_core::gateway::{SecretEntryDto, ServerFrameType, StatusType};

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
            assert!(matches!(result.action, Some(Action::GatewayConnected)));
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
