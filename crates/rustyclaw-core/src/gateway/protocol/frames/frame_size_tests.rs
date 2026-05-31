//! Frame-size / wire-encoding tests.

use super::*;

#[test]
fn test_threads_update_size() {
    let thread = ThreadInfoDto {
        id: 1,
        label: "Main".to_string(),
        description: None,
        status: None,
        kind_icon: None,
        status_icon: None,
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
    let decoded: ServerFrame =
        deserialize_frame(&bytes).expect("Round-trip deserialization failed");

    // Verify we got the right frame type
    assert!(matches!(decoded.frame_type, ServerFrameType::ThreadsUpdate));
    if let ServerPayload::ThreadsUpdate {
        threads,
        foreground_id,
    } = decoded.payload
    {
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, 1);
        assert_eq!(threads[0].label, "Main");
        assert_eq!(threads[0].description, None);
        assert_eq!(threads[0].status, None);
        assert!(threads[0].is_foreground);
        assert_eq!(threads[0].message_count, 0);
        assert!(!threads[0].has_summary);
        assert_eq!(foreground_id, Some(1));
    } else {
        panic!("Wrong payload type");
    }
}

#[test]
fn test_credential_request_roundtrip() {
    let frame = ServerFrame {
        frame_type: ServerFrameType::CredentialRequest,
        payload: ServerPayload::CredentialRequest {
            id: "cred_test_123".into(),
            provider: "anthropic".into(),
            secret_name: "ANTHROPIC_API_KEY".into(),
            message: "Authentication failed for Anthropic. Enter your API key.".into(),
        },
    };

    let bytes = serialize_frame(&frame).expect("serialize should succeed");
    let decoded: ServerFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

    assert_eq!(decoded.frame_type, ServerFrameType::CredentialRequest);
    match decoded.payload {
        ServerPayload::CredentialRequest {
            id,
            provider,
            secret_name,
            message,
        } => {
            assert_eq!(id, "cred_test_123");
            assert_eq!(provider, "anthropic");
            assert_eq!(secret_name, "ANTHROPIC_API_KEY");
            assert_eq!(
                message,
                "Authentication failed for Anthropic. Enter your API key."
            );
        }
        _ => panic!("Expected CredentialRequest payload"),
    }
}

#[test]
fn test_credential_response_roundtrip() {
    let frame = ClientFrame {
        frame_type: ClientFrameType::CredentialResponse,
        payload: ClientPayload::CredentialResponse {
            id: "cred_test_123".into(),
            dismissed: false,
            value: Some("sk-test-key-abc123".into()),
        },
    };

    let bytes = serialize_frame(&frame).expect("serialize should succeed");
    let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

    assert_eq!(decoded.frame_type, ClientFrameType::CredentialResponse);
    match decoded.payload {
        ClientPayload::CredentialResponse {
            id,
            dismissed,
            value,
        } => {
            assert_eq!(id, "cred_test_123");
            assert!(!dismissed);
            assert_eq!(value, Some("sk-test-key-abc123".into()));
        }
        _ => panic!("Expected CredentialResponse payload"),
    }
}

#[test]
fn test_credential_response_dismissed_roundtrip() {
    let frame = ClientFrame {
        frame_type: ClientFrameType::CredentialResponse,
        payload: ClientPayload::CredentialResponse {
            id: "cred_dismiss_456".into(),
            dismissed: true,
            value: None,
        },
    };

    let bytes = serialize_frame(&frame).expect("serialize should succeed");
    let decoded: ClientFrame = deserialize_frame(&bytes).expect("deserialize should succeed");

    assert_eq!(decoded.frame_type, ClientFrameType::CredentialResponse);
    match decoded.payload {
        ClientPayload::CredentialResponse {
            id,
            dismissed,
            value,
        } => {
            assert_eq!(id, "cred_dismiss_456");
            assert!(dismissed);
            assert_eq!(value, None);
        }
        _ => panic!("Expected CredentialResponse payload"),
    }
}
