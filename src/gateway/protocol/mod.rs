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

pub mod client;
pub mod frames;
pub mod server;
pub mod types;

pub use client::{server_frame_to_action, FrameAction};
pub use frames::{
    ClientFrame, ClientFrameType, ClientPayload, SecretEntryDto, ServerFrame, ServerFrameType,
    ServerPayload, StatusType, deserialize_frame, serialize_frame,
};
pub use types::{
    ChatMessage, MediaRef, ModelResponse, ParsedToolCall, ToolCallResult,
};
