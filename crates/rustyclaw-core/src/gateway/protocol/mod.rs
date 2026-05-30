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
//! Text frames are not supported and will be rejected.

pub mod event_log;
pub mod frames;
pub mod server;
pub mod types;

pub use frames::{
    CONTROL_STREAM_ID, ClientFrame, ClientFrameType, ClientPayload, SecretEntryDto, ServerFrame,
    ServerFrameType, ServerPayload, StatusType, TaskInfoDto, ThreadInfoDto, WIRE_PROTOCOL_VERSION,
    WireFrame, deserialize_frame, deserialize_wire_frame, serialize_frame, serialize_wire_frame,
};
