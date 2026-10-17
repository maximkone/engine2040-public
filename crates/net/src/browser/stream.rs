//! stream: the WebTransport reliable stream's framing, browser side.
//!
//! One frame is `kind u8, length u16 little-endian, payload`: the layout the
//! native WebTransport server reads and writes (`webtransport::encode_message`
//! pins the two together in a test). A stream delivers bytes in chunks that
//! split frames anywhere, so [`StreamDecoder`] buffers until a frame is whole.

use crate::quic::{check_budget, ControlKind};
use crate::session::{NetError, MAX_PACKET_BYTES};

/// Frame header length: kind 1 + payload length 2.
pub const FRAME_HEADER_LEN: usize = 3;

const KIND_SNAPSHOT: u8 = 0;
const KIND_LOBBY: u8 = 1;

/// Frame one reliable message. An over-budget payload fails before it is
/// queued, exactly like the datagram path.
pub fn pack(kind: ControlKind, payload: &[u8]) -> Result<Vec<u8>, NetError> {
    check_budget(payload)?;
    let length = u16::try_from(payload.len()).map_err(|_| NetError::TooLarge)?;
    let kind_byte = match kind {
        ControlKind::Snapshot => KIND_SNAPSHOT,
        ControlKind::Lobby => KIND_LOBBY,
    };
    let [low, high] = length.to_le_bytes();
    let mut message = vec![kind_byte, low, high];
    message.extend_from_slice(payload);
    Ok(message)
}

/// Reassembles frames from stream chunks.
#[derive(Default, Debug)]
pub struct StreamDecoder {
    buffer: Vec<u8>,
}

impl StreamDecoder {
    /// An empty decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one chunk read from the stream.
    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// The next whole frame, `Ok(None)` while one is still incomplete. An
    /// unknown kind or an over-budget length is a protocol violation: the
    /// caller ends the session, and nothing is allocated for the claimed
    /// length.
    pub fn next_message(&mut self) -> Result<Option<(ControlKind, Vec<u8>)>, NetError> {
        let Some(header) = self.buffer.first_chunk::<FRAME_HEADER_LEN>() else {
            return Ok(None);
        };
        let kind = match header[0] {
            KIND_SNAPSHOT => ControlKind::Snapshot,
            KIND_LOBBY => ControlKind::Lobby,
            other => return Err(NetError::BadKind(other)),
        };
        let length = usize::from(u16::from_le_bytes([header[1], header[2]]));
        if length > MAX_PACKET_BYTES {
            return Err(NetError::TooLarge);
        }
        let end = FRAME_HEADER_LEN + length;
        if self.buffer.len() < end {
            return Ok(None);
        }
        let payload = self.buffer[FRAME_HEADER_LEN..end].to_vec();
        self.buffer.drain(..end);
        Ok(Some((kind, payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> Vec<(ControlKind, Vec<u8>)> {
        vec![
            (ControlKind::Lobby, b"hello".to_vec()),
            (ControlKind::Snapshot, vec![9; MAX_PACKET_BYTES]),
            (ControlKind::Lobby, Vec::new()),
        ]
    }

    #[test]
    fn browser_stream_reassembles_frames_from_any_chunking() {
        let wire: Vec<u8> = frames()
            .iter()
            .flat_map(|(kind, payload)| pack(*kind, payload).expect("encodes"))
            .collect();
        for chunk_len in [1, 2, 3, 7, 500, wire.len()] {
            let mut decoder = StreamDecoder::new();
            let mut got = Vec::new();
            for chunk in wire.chunks(chunk_len) {
                decoder.push(chunk);
                while let Some(frame) = decoder.next_message().expect("a clean stream") {
                    got.push(frame);
                }
            }
            assert_eq!(got, frames(), "chunks of {chunk_len}");
        }
    }

    #[test]
    fn browser_stream_waits_for_a_split_header_and_a_split_payload() {
        let mut decoder = StreamDecoder::new();
        let frame = pack(ControlKind::Snapshot, b"abcdef").expect("encodes");
        decoder.push(&frame[..2]);
        assert_eq!(decoder.next_message(), Ok(None), "header incomplete");
        decoder.push(&frame[2..6]);
        assert_eq!(decoder.next_message(), Ok(None), "payload incomplete");
        decoder.push(&frame[6..]);
        assert_eq!(
            decoder.next_message(),
            Ok(Some((ControlKind::Snapshot, b"abcdef".to_vec())))
        );
    }

    #[test]
    fn browser_stream_refuses_hostile_headers_without_waiting_for_payload() {
        let mut decoder = StreamDecoder::new();
        decoder.push(&[2, 0, 0]);
        assert_eq!(decoder.next_message(), Err(NetError::BadKind(2)));
        let mut decoder = StreamDecoder::new();
        decoder.push(&[KIND_LOBBY, 0xFF, 0xFF]);
        assert_eq!(decoder.next_message(), Err(NetError::TooLarge));
    }

    #[test]
    fn browser_stream_refuses_oversize_before_queueing() {
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert_eq!(pack(ControlKind::Snapshot, &big), Err(NetError::TooLarge));
    }

    #[cfg(feature = "webtransport")]
    #[test]
    fn browser_stream_framing_equals_the_native_webtransport_framing() {
        use crate::webtransport::{decode_header, encode_message};
        for (kind, payload) in frames() {
            let native = encode_message(kind, &payload).expect("native encodes");
            assert_eq!(pack(kind, &payload).expect("encodes"), native);
            let header = [native[0], native[1], native[2]];
            assert_eq!(decode_header(header), Ok((kind, payload.len())));
        }
    }
}
