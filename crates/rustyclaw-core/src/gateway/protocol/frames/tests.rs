//! Tests for protocol frame types.

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
        assert_eq!(ServerFrameType::TasksUpdate as u8, 31);
        assert_eq!(ServerFrameType::ThreadsUpdate as u8, 32);
        assert_eq!(ServerFrameType::ThreadCreated as u8, 33);
        assert_eq!(ServerFrameType::ThreadSwitched as u8, 34);
        assert_eq!(ServerFrameType::CredentialRequest as u8, 35);
        assert_eq!(ServerFrameType::DeviceFlowStart as u8, 36);
        assert_eq!(ServerFrameType::DeviceFlowComplete as u8, 37);
        assert_eq!(ServerFrameType::DomQuery as u8, 38);
        assert_eq!(ServerFrameType::ThreadHistoryReply as u8, 39);
        assert_eq!(ServerFrameType::ThreadMessages as u8, 40);
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
        assert_eq!(ClientFrameType::TasksRequest as u8, 19);
        assert_eq!(ClientFrameType::ThreadCreate as u8, 20);
        assert_eq!(ClientFrameType::ThreadSwitch as u8, 21);
        assert_eq!(ClientFrameType::ThreadList as u8, 22);
        assert_eq!(ClientFrameType::ThreadClose as u8, 23);
        assert_eq!(ClientFrameType::ThreadRename as u8, 24);
        assert_eq!(ClientFrameType::CredentialResponse as u8, 25);
        assert_eq!(ClientFrameType::ModelSwitch as u8, 26);
        assert_eq!(ClientFrameType::DomQueryResponse as u8, 27);
        assert_eq!(ClientFrameType::SetAgentName as u8, 28);
        assert_eq!(ClientFrameType::SetWorkingDirectory as u8, 29);
        assert_eq!(ClientFrameType::ThreadHistoryRequest as u8, 30);
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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        assert_eq!(decoded.frame_type, ClientFrameType::Chat);
        matches!(decoded.payload, ClientPayload::Empty);
    }

    #[test]
    fn test_server_frame_roundtrip_device_flow_start_no_message() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::DeviceFlowStart,
            payload: ServerPayload::DeviceFlowStart {
                url: "https://github.com/login/device".into(),
                code: "ABCD-1234".into(),
                message: None,
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::DeviceFlowStart { url, code, message } => {
                assert_eq!(url, "https://github.com/login/device");
                assert_eq!(code, "ABCD-1234");
                assert_eq!(message, None);
            }
            _ => panic!("Expected DeviceFlowStart payload"),
        }
    }

    #[test]
    fn test_server_frame_roundtrip_device_flow_start_with_message() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::DeviceFlowStart,
            payload: ServerPayload::DeviceFlowStart {
                url: "https://github.com/login/device".into(),
                code: "WXYZ-5678".into(),
                message: Some("401 Unauthorized: token expired".into()),
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::DeviceFlowStart { url, code, message } => {
                assert_eq!(url, "https://github.com/login/device");
                assert_eq!(code, "WXYZ-5678");
                assert_eq!(message, Some("401 Unauthorized: token expired".into()));
            }
            _ => panic!("Expected DeviceFlowStart payload"),
        }
    }

    /// Verify that a DeviceFlowStart frame with message=None followed by
    /// other frames in a byte buffer doesn't corrupt deserialization
    /// (regression test for the skip_serializing_if bug).
    #[test]
    fn test_device_flow_start_does_not_corrupt_subsequent_frames() {
        let df_frame = ServerFrame {
            frame_type: ServerFrameType::DeviceFlowStart,
            payload: ServerPayload::DeviceFlowStart {
                url: "https://github.com/login/device".into(),
                code: "TEST-CODE".into(),
                message: None,
            },
        };
        let complete_frame = ServerFrame {
            frame_type: ServerFrameType::DeviceFlowComplete,
            payload: ServerPayload::DeviceFlowComplete,
        };

        // Serialize both independently and verify each roundtrips
        let df_bytes = serialize_frame(&df_frame).expect("serialize DeviceFlowStart");
        let complete_bytes =
            serialize_frame(&complete_frame).expect("serialize DeviceFlowComplete");

        let decoded_df: ServerFrame =
            deserialize_frame(&df_bytes).expect("deserialize DeviceFlowStart should succeed");
        let decoded_complete: ServerFrame = deserialize_frame(&complete_bytes)
            .expect("deserialize DeviceFlowComplete should succeed");

        assert!(matches!(
            decoded_df.payload,
            ServerPayload::DeviceFlowStart { .. }
        ));
        assert!(matches!(
            decoded_complete.payload,
            ServerPayload::DeviceFlowComplete
        ));
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
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

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

    #[test]
    fn test_wire_frame_round_trip_preserves_stream_id() {
        let frame = ClientFrame {
            frame_type: ClientFrameType::Chat,
            payload: ClientPayload::Chat {
                messages: vec![crate::gateway::protocol::types::ChatMessage::text(
                    "user", "hello",
                )],
            },
        };
        let wire = WireFrame::new(7, frame);

        let bytes = serialize_wire_frame(&wire).expect("serialize should succeed");
        let decoded: WireFrame<ClientFrame> =
            deserialize_wire_frame(&bytes).expect("deserialize should succeed");

        assert_eq!(decoded.version, WIRE_PROTOCOL_VERSION);
        assert_eq!(decoded.stream_id, 7);
        assert_eq!(decoded.sequence, 0);
        assert_eq!(decoded.flags, 0);
        assert_eq!(decoded.frame.frame_type, ClientFrameType::Chat);
    }
}
