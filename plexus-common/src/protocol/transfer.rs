//! Binary frame header layout for file-transfer chunks.
//!
//! Per PROTOCOL.md §4.3: every binary WebSocket frame's payload starts with
//! a 16-byte UUID v7 (`transfer_id`), followed by the chunk's bytes.

use crate::errors::ProtocolError;
use uuid::Uuid;

/// Size of the binary frame header in bytes (the UUID).
pub const HEADER_SIZE: usize = 16;

/// Pack a transfer chunk for a binary WS frame.
///
/// Returns a buffer containing the 16-byte transfer_id followed by `chunk`.
/// Allocates one Vec; caller can use the result directly as the WS payload.
pub fn pack_chunk(transfer_id: Uuid, chunk: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_SIZE + chunk.len());
    out.extend_from_slice(transfer_id.as_bytes());
    out.extend_from_slice(chunk);
    out
}

/// Parse a binary WS frame payload into `(transfer_id, chunk_bytes)`.
///
/// `chunk_bytes` borrows from the input; no allocation. Returns
/// `ProtocolError::MalformedFrame` if the input is shorter than `HEADER_SIZE`.
pub fn parse_chunk(payload: &[u8]) -> Result<(Uuid, &[u8]), ProtocolError> {
    if payload.len() < HEADER_SIZE {
        return Err(ProtocolError::MalformedFrame(format!(
            "binary frame payload is {} bytes; expected at least {} (header)",
            payload.len(),
            HEADER_SIZE
        )));
    }
    let mut id_bytes = [0u8; HEADER_SIZE];
    id_bytes.copy_from_slice(&payload[..HEADER_SIZE]);
    let id = Uuid::from_bytes(id_bytes);
    Ok((id, &payload[HEADER_SIZE..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn header_size_is_16() {
        assert_eq!(HEADER_SIZE, 16);
    }

    #[test]
    fn pack_then_parse_roundtrip() {
        let id = Uuid::now_v7();
        let chunk = b"hello world";
        let packed = pack_chunk(id, chunk);
        let (parsed_id, parsed_chunk) = parse_chunk(&packed).expect("parses");
        assert_eq!(parsed_id, id);
        assert_eq!(parsed_chunk, chunk);
    }

    #[test]
    fn pack_empty_chunk() {
        let id = Uuid::now_v7();
        let packed = pack_chunk(id, &[]);
        assert_eq!(packed.len(), HEADER_SIZE);
        let (parsed_id, parsed_chunk) = parse_chunk(&packed).expect("parses");
        assert_eq!(parsed_id, id);
        assert_eq!(parsed_chunk, b"");
    }

    #[test]
    fn parse_too_short_returns_none() {
        let short = vec![0u8; 8];
        assert!(parse_chunk(&short).is_err());
    }

    #[test]
    fn parse_exactly_header_size_returns_empty_chunk() {
        let id = Uuid::now_v7();
        let header_only = id.as_bytes().to_vec();
        let (parsed_id, parsed_chunk) = parse_chunk(&header_only).expect("parses");
        assert_eq!(parsed_id, id);
        assert_eq!(parsed_chunk, b"");
    }
}
