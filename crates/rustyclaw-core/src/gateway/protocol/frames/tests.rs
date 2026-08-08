//! Tests for protocol frame types.

use super::*;

mod serialization {
    use super::*;
    use std::path::{Path, PathBuf};

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
        assert_eq!(ServerFrameType::DownloadsUpdate as u8, 93);
        assert_eq!(ServerFrameType::ThreadMessages as u8, 40);
        assert_eq!(ServerFrameType::ProjectsUpdate as u8, 41);
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
        assert_eq!(ClientFrameType::ProjectList as u8, 31);
        assert_eq!(ClientFrameType::ProjectCreate as u8, 32);
        assert_eq!(ClientFrameType::ProjectRename as u8, 33);
        assert_eq!(ClientFrameType::ProjectDelete as u8, 34);
        assert_eq!(ClientFrameType::ProjectSwitch as u8, 35);
        assert_eq!(ClientFrameType::ProjectUpdate as u8, 78);
        assert_eq!(ClientFrameType::ThreadUpdate as u8, 79);
        assert_eq!(ClientFrameType::DownloadsRequest as u8, 91);
        assert_eq!(ClientFrameType::DownloadCancel as u8, 92);
        assert_eq!(ClientFrameType::DownloadsClearFinished as u8, 93);
    }

    /// Typing the path fields as `PathBuf` did not change the wire format.
    ///
    /// serde encodes a path as its UTF-8 `str`, so bincode emits exactly the
    /// length-prefixed bytes it emitted when these were `String`s — an older
    /// peer decodes a newer peer's frames unchanged, and vice versa. This is
    /// the whole basis for calling the change wire-compatible, so it is
    /// asserted rather than assumed.
    #[test]
    fn path_fields_encode_byte_for_byte_like_strings() {
        #[derive(serde::Serialize)]
        struct ProjectAsStrings {
            id: u64,
            name: String,
            path: String,
        }

        let typed = ProjectInfoDto {
            id: 3,
            name: "Api".into(),
            path: PathBuf::from("/srv/api"),
        };
        let stringly = ProjectAsStrings {
            id: 3,
            name: "Api".into(),
            path: "/srv/api".into(),
        };
        assert_eq!(
            serialize_frame(&typed).unwrap(),
            serialize_frame(&stringly).unwrap(),
        );

        // Including the `Option` case, where the discriminant precedes it.
        #[derive(serde::Serialize)]
        struct OverrideAsString(Option<String>);

        for dir in ["/tmp/worktree", ""] {
            assert_eq!(
                serialize_frame(&Some(PathBuf::from(dir))).unwrap(),
                serialize_frame(&OverrideAsString(Some(dir.to_string()))).unwrap(),
            );
        }
        assert_eq!(
            serialize_frame(&Option::<PathBuf>::None).unwrap(),
            serialize_frame(&OverrideAsString(None)).unwrap(),
        );
    }

    /// Plugins had no wire representation at all before this: the manager was
    /// gateway-side only, so a client's plugin panel had nothing to render.
    #[test]
    fn test_plugins_update_roundtrip() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::PluginsUpdate,
            payload: ServerPayload::PluginsUpdate {
                plugins: vec![PluginInfoDto {
                    name: "chart".into(),
                    description: "Render live charts".into(),
                    emoji: Some("📊".into()),
                    version: "1.0.0".into(),
                    enabled: true,
                    state_json: r#"{"title":"Sales","data":[1,2,3]}"#.into(),
                    actions: vec![PluginActionDto {
                        name: "refresh".into(),
                        description: "Re-fetch data".into(),
                    }],
                    html_template: Some("index.html".into()),
                }],
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::PluginsUpdate { plugins } => {
                assert_eq!(plugins.len(), 1);
                assert_eq!(plugins[0].name, "chart");
                assert_eq!(plugins[0].emoji.as_deref(), Some("📊"));
                assert_eq!(plugins[0].actions[0].name, "refresh");
                assert_eq!(plugins[0].html_template.as_deref(), Some("index.html"));
                // The state survives as JSON text and parses back to the value
                // the gateway held.
                let state: serde_json::Value =
                    serde_json::from_str(&plugins[0].state_json).expect("state parses");
                assert_eq!(state["title"], "Sales");
                assert_eq!(state["data"][2], 3);
            }
            _ => panic!("Expected PluginsUpdate payload"),
        }
    }

    #[test]
    fn test_plugin_client_frames_roundtrip() {
        for (frame_type, payload) in [
            (ClientFrameType::PluginList, ClientPayload::PluginList),
            (
                ClientFrameType::PluginRefresh,
                ClientPayload::PluginRefresh {
                    plugin_name: "chart".into(),
                },
            ),
        ] {
            let frame = ClientFrame {
                frame_type,
                payload,
            };
            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ClientFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");
            assert_eq!(decoded.frame_type as u8, frame_type as u8);
        }
        assert_eq!(ClientFrameType::PluginList as u8, 80);
        assert_eq!(ClientFrameType::PluginRefresh as u8, 81);
        assert_eq!(ServerFrameType::PluginsUpdate as u8, 86);
    }

    #[test]
    fn test_workspace_file_frames_roundtrip() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::WorkspaceDirListing,
            payload: ServerPayload::WorkspaceDirListing {
                path: PathBuf::from("src"),
                entries: vec![WorkspaceEntryDto {
                    path: PathBuf::from("src/main.rs"),
                    name: "main.rs".into(),
                    is_dir: false,
                    size: 12,
                }],
                error: None,
                root: PathBuf::from("/srv/api"),
            },
        };
        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ServerPayload::WorkspaceDirListing { entries, error, .. } => {
                assert!(error.is_none());
                assert_eq!(entries[0].name, "main.rs");
                assert_eq!(entries[0].path, Path::new("src/main.rs"));
                assert_eq!(entries[0].size, 12);
            }
            _ => panic!("Expected WorkspaceDirListing payload"),
        }

        // A refusal must survive the wire too — it is the only way the user
        // learns why an open did nothing.
        let frame = ServerFrame {
            frame_type: ServerFrameType::WorkspaceFileContent,
            payload: ServerPayload::WorkspaceFileContent {
                path: PathBuf::from("../secret"),
                content: String::new(),
                error: Some("outside this thread's working directory".into()),
                root: PathBuf::from("/srv/api"),
            },
        };
        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ServerPayload::WorkspaceFileContent { content, error, .. } => {
                assert!(content.is_empty());
                assert!(error.unwrap().contains("outside"));
            }
            _ => panic!("Expected WorkspaceFileContent payload"),
        }

        assert_eq!(ClientFrameType::WorkspaceListDir as u8, 82);
        assert_eq!(ClientFrameType::WorkspaceReadFile as u8, 83);
        assert_eq!(ClientFrameType::WorkspaceWriteFile as u8, 84);
        assert_eq!(ServerFrameType::WorkspaceDirListing as u8, 87);
        assert_eq!(ServerFrameType::WorkspaceFileContent as u8, 88);
        assert_eq!(ServerFrameType::WorkspaceWriteResult as u8, 89);
    }

    #[test]
    fn test_project_update_client_roundtrip() {
        let frame = ClientFrame {
            frame_type: ClientFrameType::ProjectUpdate,
            payload: ClientPayload::ProjectUpdate {
                project_id: 7,
                name: "Renamed".into(),
                path: "/home/me/moved".into(),
            },
        };
        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ClientPayload::ProjectUpdate {
                project_id,
                name,
                path,
            } => {
                assert_eq!(project_id, 7);
                assert_eq!(name, "Renamed");
                assert_eq!(path, Path::new("/home/me/moved"));
            }
            _ => panic!("Expected ProjectUpdate payload"),
        }
    }

    #[test]
    fn test_thread_update_client_roundtrip() {
        // Both states of the override have to survive the wire: `None` is how
        // the client says "go back to inheriting the project's directory".
        for working_dir in [Some(PathBuf::from("/tmp/worktree")), None] {
            let frame = ClientFrame {
                frame_type: ClientFrameType::ThreadUpdate,
                payload: ClientPayload::ThreadUpdate {
                    thread_id: 5,
                    label: "Refactor".into(),
                    working_dir: working_dir.clone(),
                },
            };
            let bytes = serialize_frame(&frame).expect("serialize should succeed");
            let decoded: ClientFrame =
                deserialize_frame(&bytes).expect("deserialize should succeed");
            match decoded.payload {
                ClientPayload::ThreadUpdate {
                    thread_id,
                    label,
                    working_dir: got,
                } => {
                    assert_eq!(thread_id, 5);
                    assert_eq!(label, "Refactor");
                    assert_eq!(got, working_dir);
                }
                _ => panic!("Expected ThreadUpdate payload"),
            }
        }
    }

    #[test]
    fn test_projects_update_roundtrip() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ProjectsUpdate,
            payload: ServerPayload::ProjectsUpdate {
                projects: vec![
                    ProjectInfoDto {
                        id: 1,
                        name: "Default".into(),
                        path: "/home/me/ws".into(),
                    },
                    ProjectInfoDto {
                        id: 2,
                        name: "Side".into(),
                        path: "/home/me/side".into(),
                    },
                ],
                active_id: 2,
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::ProjectsUpdate {
                projects,
                active_id,
            } => {
                assert_eq!(projects.len(), 2);
                assert_eq!(projects[1].name, "Side");
                assert_eq!(projects[1].path, Path::new("/home/me/side"));
                assert_eq!(active_id, 2);
            }
            _ => panic!("Expected ProjectsUpdate payload"),
        }
    }

    #[test]
    fn test_project_create_client_roundtrip() {
        let frame = ClientFrame {
            frame_type: ClientFrameType::ProjectCreate,
            payload: ClientPayload::ProjectCreate {
                name: "Side".into(),
                path: "/home/me/side".into(),
            },
        };
        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ClientPayload::ProjectCreate { name, path } => {
                assert_eq!(name, "Side");
                assert_eq!(path, Path::new("/home/me/side"));
            }
            _ => panic!("Expected ProjectCreate payload"),
        }
    }

    #[test]
    fn test_agents_update_roundtrip() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::AgentsUpdate,
            payload: ServerPayload::AgentsUpdate {
                agents: vec![
                    AgentInfoDto {
                        id: "main".into(),
                        name: "RustyClaw".into(),
                        description: None,
                    },
                    AgentInfoDto {
                        id: "researcher".into(),
                        name: "Researcher".into(),
                        description: Some("digs through papers".into()),
                    },
                ],
                active_id: "researcher".into(),
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::AgentsUpdate { agents, active_id } => {
                assert_eq!(agents.len(), 2);
                assert_eq!(agents[0].id, "main");
                assert_eq!(
                    agents[1].description.as_deref(),
                    Some("digs through papers")
                );
                assert_eq!(active_id, "researcher");
            }
            _ => panic!("Expected AgentsUpdate payload"),
        }
    }

    #[test]
    fn test_agent_switch_client_roundtrip() {
        let frame = ClientFrame {
            frame_type: ClientFrameType::AgentSwitch,
            payload: ClientPayload::AgentSwitch {
                agent_id: "researcher".into(),
            },
        };
        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ClientPayload::AgentSwitch { agent_id } => assert_eq!(agent_id, "researcher"),
            _ => panic!("Expected AgentSwitch payload"),
        }
    }

    #[test]
    fn test_agent_frame_type_values() {
        assert_eq!(ClientFrameType::AgentListRequest as u8, 73);
        assert_eq!(ClientFrameType::AgentSwitch as u8, 74);
        assert_eq!(ClientFrameType::AgentCreate as u8, 75);
        assert_eq!(ClientFrameType::AgentDelete as u8, 76);
        assert_eq!(ServerFrameType::AgentsUpdate as u8, 83);
        assert_eq!(ServerFrameType::AgentSwitched as u8, 84);
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
    fn test_tool_status_and_process_control_frame_values() {
        assert_eq!(ServerFrameType::ToolStatus as u8, 81);
        assert_eq!(ClientFrameType::ProcessControl as u8, 72);
    }

    #[test]
    fn test_server_tool_status_bincode_roundtrip() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ToolStatus,
            payload: ServerPayload::ToolStatus {
                tool_id: "call_001".into(),
                name: "execute_command".into(),
                elapsed_ms: 12_500,
                pid: Some(4242),
                cpu_percent: Some(87.5),
                memory_bytes: Some(145 * 1024 * 1024),
                state: Some("running".into()),
                message: None,
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::ToolStatus {
                tool_id,
                name,
                elapsed_ms,
                pid,
                cpu_percent,
                memory_bytes,
                state,
                message,
            } => {
                assert_eq!(tool_id, "call_001");
                assert_eq!(name, "execute_command");
                assert_eq!(elapsed_ms, 12_500);
                assert_eq!(pid, Some(4242));
                assert_eq!(cpu_percent, Some(87.5));
                assert_eq!(memory_bytes, Some(145 * 1024 * 1024));
                assert_eq!(state.as_deref(), Some("running"));
                assert_eq!(message, None);
            }
            _ => panic!("Expected ToolStatus payload"),
        }
    }

    #[test]
    fn test_client_process_control_bincode_roundtrip() {
        use crate::exec_status::ProcessControlAction;

        let frame = ClientFrame {
            frame_type: ClientFrameType::ProcessControl,
            payload: ClientPayload::ProcessControl {
                pid: 4242,
                action: ProcessControlAction::Pause,
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ClientPayload::ProcessControl { pid, action } => {
                assert_eq!(pid, 4242);
                assert_eq!(action, ProcessControlAction::Pause);
            }
            _ => panic!("Expected ProcessControl payload"),
        }
    }

    #[test]
    fn test_engine_action_progress_frame_value() {
        assert_eq!(ServerFrameType::EngineActionProgress as u8, 82);
    }

    #[test]
    fn test_server_engine_action_progress_bincode_roundtrip() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::EngineActionProgress,
            payload: ServerPayload::EngineActionProgress {
                engine: "ollama".into(),
                line: ">>> downloading manifest".into(),
                percent: 0.0,
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

        match decoded.payload {
            ServerPayload::EngineActionProgress {
                engine,
                line,
                percent,
            } => {
                assert_eq!(engine, "ollama");
                assert_eq!(line, ">>> downloading manifest");
                assert_eq!(percent, 0.0);
            }
            _ => panic!("Expected EngineActionProgress payload"),
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
                thread_id: Some(12),
                client_kind: Some(crate::gateway::SessionOrigin::Tui),
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
        // The thread the message was typed into has to survive the trip, or
        // the gateway is back to guessing from its own foreground. Same for
        // the client's declared kind — that is what becomes the session
        // origin in the system prompt.
        match decoded.frame.payload {
            ClientPayload::Chat {
                thread_id,
                client_kind,
                ..
            } => {
                assert_eq!(thread_id, Some(12));
                assert_eq!(client_kind, Some(crate::gateway::SessionOrigin::Tui));
            }
            other => panic!("Expected Chat payload, got {other:?}"),
        }
    }

    /// The wire strings for `SessionOrigin` are part of the protocol: they
    /// must not change under refactors, or clients on the other end of an
    /// old frame stop matching.
    #[test]
    fn session_origin_serializes_to_its_protocol_strings() {
        use serde_json::json;
        for (origin, expected) in [
            (crate::gateway::SessionOrigin::Desktop, "desktop"),
            (crate::gateway::SessionOrigin::Tui, "tui"),
            (crate::gateway::SessionOrigin::Remote, "remote"),
            (crate::gateway::SessionOrigin::Local, "local"),
            (crate::gateway::SessionOrigin::Messenger, "messenger"),
            (crate::gateway::SessionOrigin::Trigger, "trigger"),
        ] {
            let wire = serde_json::to_value(origin).expect("serialize should succeed");
            assert_eq!(wire, json!(expected), "{origin:?} must stay {expected:?}");
        }
    }

    /// A turn announces its thread when it opens and when it closes.
    ///
    /// Everything between those two frames is unlabelled and belongs to the
    /// turn, so these are the only two chances a client gets to learn where
    /// the response is going — and the only way it can tell a close-out for
    /// its own turn from one for somebody else's.
    #[test]
    fn turn_boundary_frames_carry_their_thread() {
        let start = ServerFrame {
            frame_type: ServerFrameType::StreamStart,
            payload: ServerPayload::StreamStart {
                thread_id: Some(31),
            },
        };
        let bytes = serialize_frame(&start).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ServerPayload::StreamStart { thread_id } => assert_eq!(thread_id, Some(31)),
            other => panic!("Expected StreamStart payload, got {other:?}"),
        }

        let done = ServerFrame {
            frame_type: ServerFrameType::ResponseDone,
            payload: ServerPayload::ResponseDone {
                ok: true,
                thread_id: Some(31),
            },
        };
        let bytes = serialize_frame(&done).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ServerPayload::ResponseDone { ok, thread_id } => {
                assert!(ok);
                assert_eq!(thread_id, Some(31));
            }
            other => panic!("Expected ResponseDone payload, got {other:?}"),
        }
    }
}

// ── Diagnostic: transcripts carrying tool calls ─────────────────────────
mod thread_history_wire {
    use super::*;
    use crate::gateway::{ChatMessage, ToolCallRecord};

    /// The JSON a thread persists for an assistant turn that called a tool.
    fn stored_tool_calls() -> serde_json::Value {
        serde_json::json!([{
            "id": "call_1",
            "name": "read_file",
            "arguments": {"path": "src/main.rs"}
        }])
    }

    /// A transcript whose assistant turn made a tool call must survive the wire.
    ///
    /// This is the bug that made threads with real work in them open empty
    /// while a short chat opened fine. `tool_calls` was
    /// `Option<serde_json::Value>`, and frames are bincode — not
    /// self-describing — so `Value` encoded on the gateway and then failed to
    /// decode on the client with `AnyNotSupported`. The reply was built and
    /// sent; it simply could not be read. Every thread that had ever run a
    /// tool was undeliverable, which is every thread worth opening.
    #[test]
    fn a_history_reply_with_tool_calls_survives_the_wire() {
        let calls = ToolCallRecord::from_stored_json(&stored_tool_calls());
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadHistoryReply,
            payload: ServerPayload::ThreadHistoryReply {
                thread_id: 2,
                ok: true,
                messages: vec![
                    ChatMessage::text("user", "read the file"),
                    ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: Some(calls),
                        tool_call_id: None,
                        media: None,
                    },
                    ChatMessage {
                        role: "tool".into(),
                        content: "fn main() {}".into(),
                        tool_calls: None,
                        tool_call_id: Some("call_1".into()),
                        media: None,
                    },
                ],
                error: None,
            },
        };

        let bytes = serialize_frame(&frame).expect("a transcript with tool calls must serialize");
        let decoded: ServerFrame =
            deserialize_frame(&bytes).expect("a transcript with tool calls must deserialize");

        match decoded.payload {
            ServerPayload::ThreadHistoryReply { messages, .. } => {
                assert_eq!(messages.len(), 3);
                let calls = messages[1]
                    .tool_calls
                    .as_ref()
                    .expect("the assistant turn kept its tool call");
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "read_file");
                assert!(
                    calls[0].arguments.contains("src/main.rs"),
                    "arguments survived as text: {:?}",
                    calls[0].arguments
                );
            }
            other => panic!("wrong payload: {other:?}"),
        }
    }

    /// The same for `ThreadMessages`, the other frame carrying a transcript.
    ///
    /// It is served by a different call site, so it broke the same way and had
    /// to be fixed separately — worth pinning separately too.
    #[test]
    fn a_thread_messages_frame_with_tool_calls_survives_the_wire() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadMessages,
            payload: ServerPayload::ThreadMessages {
                thread_id: 3,
                messages: vec![ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: Some(ToolCallRecord::from_stored_json(&stored_tool_calls())),
                    tool_call_id: None,
                    media: None,
                }],
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ServerPayload::ThreadMessages { messages, .. } => {
                assert_eq!(
                    messages[0].tool_calls.as_ref().map(|c| c.len()),
                    Some(1),
                    "the tool call must survive"
                );
            }
            other => panic!("wrong payload: {other:?}"),
        }
    }

    /// The tool-free case, as a control.
    ///
    /// It passed throughout the outage, which is precisely why the bug looked
    /// like "some threads" rather than "the transcript path is broken".
    #[test]
    fn a_history_reply_without_tool_calls_survives_the_wire() {
        let frame = ServerFrame {
            frame_type: ServerFrameType::ThreadHistoryReply,
            payload: ServerPayload::ThreadHistoryReply {
                thread_id: 1,
                ok: true,
                messages: vec![
                    ChatMessage::text("user", "hello"),
                    ChatMessage::text("assistant", "hi there"),
                    ChatMessage::text("user", "thanks"),
                ],
                error: None,
            },
        };

        let bytes = serialize_frame(&frame).expect("serialize should succeed");
        let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");
        match decoded.payload {
            ServerPayload::ThreadHistoryReply { messages, .. } => assert_eq!(messages.len(), 3),
            other => panic!("wrong payload: {other:?}"),
        }
    }

    /// Arguments already stored as text are not re-quoted on each hop.
    ///
    /// Providers differ: some record arguments as an object, some as a JSON
    /// string. Blind `to_string` would wrap the latter in another set of
    /// quotes every time a transcript was served.
    #[test]
    fn string_arguments_are_not_double_encoded() {
        let stored = serde_json::json!([{
            "id": "c", "name": "bash", "arguments": "{\"cmd\":\"ls\"}"
        }]);
        let calls = ToolCallRecord::from_stored_json(&stored);
        assert_eq!(calls[0].arguments, r#"{"cmd":"ls"}"#);
    }

    /// A malformed stored call degrades rather than disappearing.
    #[test]
    fn unrecognised_tool_call_shapes_still_render_something() {
        let stored = serde_json::json!([{"unexpected": true}]);
        let calls = ToolCallRecord::from_stored_json(&stored);
        assert_eq!(calls.len(), 1, "the call is kept, not dropped");
        assert!(calls[0].id.is_empty());
        assert!(calls[0].name.is_empty());
    }
}
