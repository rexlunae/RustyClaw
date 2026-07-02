//! Gateway client interface and wire protocol.
//!
//! This module holds the **client-facing** half of the gateway: the wire
//! protocol ([`protocol`]), the transport abstraction ([`transport`]), the
//! client connection ([`client`]) and its command/event types
//! ([`client_types`]), plus the shared request/response types ([`types`]).
//!
//! The **server** implementation (session handling, model dispatch, messenger
//! and tool orchestration, the SSH server) lives in the separate
//! `rustyclaw-gateway` crate, which depends on this interface.

pub mod client;
pub mod client_types;
pub mod protocol;
pub mod ssh_connection;
pub mod transport;
mod types;

// Re-export client-facing types
pub use client::GatewayClient;
pub use client_types::{GatewayCommand, GatewayEvent, ThreadInfoDto};

// Re-export SSH connection transport (client-side)
pub use ssh_connection::{SshConnection, SshReader, SshWriter};

// Re-export protocol types
pub use protocol::{
    ChannelPairActionKind, ClientFrame, ClientFrameType, ClientPayload, CronActionKind,
    EngineActionKind, FrameCodecError, ModelActionKind, SecretEntryDto, ServerFrame,
    ServerFrameType, ServerPayload, ServiceInfoDto, StatusType, WireFrame, deserialize_frame,
    deserialize_wire_frame, serialize_frame, serialize_wire_frame,
};

// Re-export public types (includes protocol types via types module)
pub use types::{
    ChatMessage, ChatRequest, CopilotSession, GatewayOptions, MediaRef, ModelContext,
    ModelResponse, ParsedToolCall, ProbeResult, ProviderRequest, ToolCallResult,
};

// Re-export transport types
pub use transport::{
    PeerInfo, ScopedTransportWriter, Transport, TransportAcceptor, TransportReader, TransportType,
    TransportWriter,
};

// Re-export protocol server helpers used by the gateway server crate and by
// any client that needs to construct server frames.
pub use protocol::server::{
    parse_client_frame, send_credential_request, send_frame, send_reload_result,
    send_secrets_delete_credential_result, send_secrets_delete_result, send_secrets_get_result,
    send_secrets_has_totp_result, send_secrets_list_result, send_secrets_peek_result,
    send_secrets_remove_totp_result, send_secrets_set_disabled_result,
    send_secrets_set_policy_result, send_secrets_setup_totp_result, send_secrets_store_result,
    send_secrets_verify_totp_result, send_vault_unlocked,
};
