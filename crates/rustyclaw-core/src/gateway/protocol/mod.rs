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

pub mod frames;
pub mod server;
pub mod types;

pub use frames::{
    ClientFrame, ClientFrameType, ClientPayload, SecretEntryDto, ServerFrame, ServerFrameType,
    ServerPayload, StatusType, TaskInfoDto, ThreadInfoDto, deserialize_frame, serialize_frame,
};
