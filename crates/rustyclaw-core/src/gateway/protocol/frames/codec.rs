//! Binary serialization for gateway protocol frames.

use super::WireFrame;

/// Errors produced when encoding or decoding protocol frames.
#[derive(Debug, thiserror::Error)]
pub enum FrameCodecError {
    /// Incoming frame exceeded [`MAX_FRAME_SIZE`].
    #[error("Frame too large: {len} bytes (max {max})")]
    TooLarge { len: usize, max: usize },
    /// A frame decoded, but did not account for the whole buffer.
    ///
    /// Bincode stops as soon as it has filled the type it was asked for, so a
    /// buffer holding one thing can decode as a shorter, entirely different
    /// thing and report success. Left unchecked that turns a corrupt or
    /// mis-framed read into a plausible-looking frame rather than an error.
    #[error("Frame decoded {consumed} of {len} bytes; the rest is unaccounted for")]
    TrailingBytes { consumed: usize, len: usize },
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
    let (result, consumed) = bincode::serde::decode_from_slice(bytes, bincode::config::standard())?;
    // A partial decode is not a success. Bincode stops the moment it has
    // filled the requested type, so the leading bytes of one frame can satisfy
    // a shorter, unrelated one — a control-stream wire frame begins
    // `[version=3, stream=0, …]`, which reads perfectly well as a bare
    // `ServerFrame` of two bytes and leaves the real payload on the floor.
    // Accepting that silently is what turned an undecodable transcript into a
    // thread that simply never opened, with no error anywhere.
    if consumed != bytes.len() {
        return Err(FrameCodecError::TrailingBytes {
            consumed,
            len: bytes.len(),
        });
    }
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
