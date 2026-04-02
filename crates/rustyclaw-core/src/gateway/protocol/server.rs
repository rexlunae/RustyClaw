//! Server-side helpers for the gateway protocol.
//!
//! This module provides helpers for the gateway server to send frames to clients.

use super::frames::{
    ClientFrame, SecretEntryDto, ServerFrame, ServerFrameType, ServerPayload, TaskInfoDto,
    deserialize_frame,
};
use crate::gateway::transport::TransportWriter;
use anyhow::Result;

/// Send a ServerFrame via any transport writer.
pub async fn send_frame(writer: &mut dyn TransportWriter, frame: &ServerFrame) -> Result<()> {
    writer.send(frame).await
}

/// Parse a ClientFrame from binary WebSocket message bytes.
pub fn parse_client_frame(bytes: &[u8]) -> Result<ClientFrame> {
    deserialize_frame(bytes).map_err(|e| anyhow::anyhow!("parse failed: {}", e))
}

/// Build and send a hello frame.
pub async fn send_hello(
    writer: &mut dyn TransportWriter,
    agent: &str,
    settings_dir: &str,
    vault_locked: bool,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::Hello,
        payload: ServerPayload::Hello {
            agent: agent.into(),
            settings_dir: settings_dir.into(),
            vault_locked,
            provider: provider.map(|s| s.into()),
            model: model.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send an auth challenge frame.
pub async fn send_auth_challenge(writer: &mut dyn TransportWriter, method: &str) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::AuthChallenge,
        payload: ServerPayload::AuthChallenge {
            method: method.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send an auth result frame.
pub async fn send_auth_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
    retry: Option<bool>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::AuthResult,
        payload: ServerPayload::AuthResult {
            ok,
            message: message.map(|s| s.into()),
            retry,
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send an error frame.
pub async fn send_error(writer: &mut dyn TransportWriter, message: &str) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::Error,
        payload: ServerPayload::Error {
            ok: false,
            message: message.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send an info frame.
pub async fn send_info(writer: &mut dyn TransportWriter, message: &str) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::Info,
        payload: ServerPayload::Info {
            message: message.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a status frame.
pub async fn send_status(
    writer: &mut dyn TransportWriter,
    status: super::frames::StatusType,
    detail: &str,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::Status,
        payload: ServerPayload::Status {
            status,
            detail: detail.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a vault_unlocked frame.
pub async fn send_vault_unlocked(writer: &mut dyn TransportWriter, ok: bool, message: Option<&str>) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::VaultUnlocked,
        payload: ServerPayload::VaultUnlocked {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_list_result frame.
pub async fn send_secrets_list_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    entries: Vec<SecretEntryDto>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsListResult,
        payload: ServerPayload::SecretsListResult { ok, entries },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_store_result frame.
pub async fn send_secrets_store_result(writer: &mut dyn TransportWriter, ok: bool, message: &str) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsStoreResult,
        payload: ServerPayload::SecretsStoreResult {
            ok,
            message: message.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_get_result frame.
pub async fn send_secrets_get_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    key: &str,
    value: Option<&str>,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsGetResult,
        payload: ServerPayload::SecretsGetResult {
            ok,
            key: key.into(),
            value: value.map(|s| s.into()),
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_delete_result frame.
pub async fn send_secrets_delete_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsDeleteResult,
        payload: ServerPayload::SecretsDeleteResult {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_peek_result frame.
pub async fn send_secrets_peek_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    fields: Vec<(String, String)>,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsPeekResult,
        payload: ServerPayload::SecretsPeekResult {
            ok,
            fields,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_set_policy_result frame.
pub async fn send_secrets_set_policy_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsSetPolicyResult,
        payload: ServerPayload::SecretsSetPolicyResult {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_set_disabled_result frame.
pub async fn send_secrets_set_disabled_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsSetDisabledResult,
        payload: ServerPayload::SecretsSetDisabledResult {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_delete_credential_result frame.
pub async fn send_secrets_delete_credential_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsDeleteCredentialResult,
        payload: ServerPayload::SecretsDeleteCredentialResult {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_has_totp_result frame.
pub async fn send_secrets_has_totp_result(writer: &mut dyn TransportWriter, has_totp: bool) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsHasTotpResult,
        payload: ServerPayload::SecretsHasTotpResult { has_totp },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_setup_totp_result frame.
pub async fn send_secrets_setup_totp_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    uri: Option<&str>,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsSetupTotpResult,
        payload: ServerPayload::SecretsSetupTotpResult {
            ok,
            uri: uri.map(|s| s.into()),
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_verify_totp_result frame.
pub async fn send_secrets_verify_totp_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsVerifyTotpResult,
        payload: ServerPayload::SecretsVerifyTotpResult {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a secrets_remove_totp_result frame.
pub async fn send_secrets_remove_totp_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::SecretsRemoveTotpResult,
        payload: ServerPayload::SecretsRemoveTotpResult {
            ok,
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a reload_result frame.
pub async fn send_reload_result(
    writer: &mut dyn TransportWriter,
    ok: bool,
    provider: &str,
    model: &str,
    message: Option<&str>,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::ReloadResult,
        payload: ServerPayload::ReloadResult {
            ok,
            provider: provider.into(),
            model: model.into(),
            message: message.map(|s| s.into()),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a chunk frame.
pub async fn send_chunk(writer: &mut dyn TransportWriter, delta: &str) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::Chunk,
        payload: ServerPayload::Chunk {
            delta: delta.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a response done frame.
pub async fn send_response_done(writer: &mut dyn TransportWriter, ok: bool) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::ResponseDone,
        payload: ServerPayload::ResponseDone { ok },
    };
    send_frame(writer, &frame).await
}

/// Build and send a stream start frame.
pub async fn send_stream_start(writer: &mut dyn TransportWriter) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::StreamStart,
        payload: ServerPayload::StreamStart,
    };
    send_frame(writer, &frame).await
}

/// Build and send a tool call frame.
pub async fn send_tool_call(writer: &mut dyn TransportWriter, id: &str, name: &str, arguments: &str) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::ToolCall,
        payload: ServerPayload::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a tool result frame.
pub async fn send_tool_result(
    writer: &mut dyn TransportWriter,
    id: &str,
    name: &str,
    result: &str,
    is_error: bool,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::ToolResult,
        payload: ServerPayload::ToolResult {
            id: id.into(),
            name: name.into(),
            result: result.into(),
            is_error,
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a tool approval request frame.
pub async fn send_tool_approval_request(
    writer: &mut dyn TransportWriter,
    id: &str,
    name: &str,
    arguments: &str,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::ToolApprovalRequest,
        payload: ServerPayload::ToolApprovalRequest {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a user-prompt request frame (for the `ask_user` tool).
pub async fn send_user_prompt_request(
    writer: &mut dyn TransportWriter,
    id: &str,
    prompt: &crate::user_prompt_types::UserPrompt,
) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::UserPromptRequest,
        payload: ServerPayload::UserPromptRequest {
            id: id.into(),
            prompt: prompt.clone(),
        },
    };
    send_frame(writer, &frame).await
}

/// Build and send a tasks update frame.
pub async fn send_tasks_update(writer: &mut dyn TransportWriter, tasks: Vec<TaskInfoDto>) -> Result<()> {
    let frame = ServerFrame {
        frame_type: ServerFrameType::TasksUpdate,
        payload: ServerPayload::TasksUpdate { tasks },
    };
    send_frame(writer, &frame).await
}
