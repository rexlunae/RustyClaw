//! Binary serialization for gateway protocol frames.

use super::WireFrame;

/// Errors produced when encoding or decoding protocol frames.
#[derive(Debug, thiserror::Error)]
pub enum FrameCodecError {
    /// Incoming frame exceeded [`MAX_FRAME_SIZE`].
    #[error("Frame too large: {len} bytes (max {max})")]
    TooLarge { len: usize, max: usize },
    #[error(transparent)]
    Encode(#[from] bincode::error::EncodeError),
    #[error(transparent)]
    Decode(#[from] bincode::error::DecodeError),
}

/// Serialize a frame to binary using bincode with serde.
pub fn serialize_frame<T: serde::Serialize>(frame: &T) -> Result<Vec<u8>, FrameCodecError> {
    Ok(bincode::serde::encode_to_vec(
        frame,
        bincode::config::standard(),
    )?)
}

/// Maximum frame payload size accepted by deserialization (defense-in-depth).
///
/// The SSH/stdin transport already enforces this limit, but we check here too
/// so that any future transport that forgets the check cannot OOM the process.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024; // 16 MB

/// Deserialize a frame from binary using bincode with serde.
pub fn deserialize_frame<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, FrameCodecError> {
    if bytes.len() > MAX_FRAME_SIZE {
        return Err(FrameCodecError::TooLarge {
            len: bytes.len(),
            max: MAX_FRAME_SIZE,
        });
    }
    let (result, _) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())?;
    Ok(result)
}

/// Serialize a multiplexed wire frame.
pub fn serialize_wire_frame<T: serde::Serialize>(
    frame: &WireFrame<T>,
) -> Result<Vec<u8>, FrameCodecError> {
    serialize_frame(frame)
}

/// Deserialize a multiplexed wire frame.
pub fn deserialize_wire_frame<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<WireFrame<T>, FrameCodecError> {
    deserialize_frame(bytes)
}
