//! frame: the reliable-stream framing both ends of a session share.
//!
//! One frame is `kind u8, length u16 little-endian, payload`. The payload is
//! the exact bytes the codec produced (`encode_snapshot`, a lobby message),
//! so a stream message arrives byte-identical to the QUIC and UDP paths. The
//! payload obeys the same 1200-byte budget as every other transport.

use crate::quic::{check_budget, ControlKind};
use crate::session::{NetError, MAX_PACKET_BYTES};

/// Frame header length: kind 1 + payload length 2.
pub const FRAME_HEADER_LEN: usize = 3;

/// Kind byte of a full snapshot message.
const KIND_SNAPSHOT: u8 = 0;
/// Kind byte of a lobby message.
const KIND_LOBBY: u8 = 1;

/// Frame one reliable message. Over-budget payloads fail before they are
/// queued, exactly like the datagram path.
pub fn encode_message(kind: ControlKind, payload: &[u8]) -> Result<Vec<u8>, NetError> {
    check_budget(payload)?;
    let length = u16::try_from(payload.len()).map_err(|_| NetError::TooLarge)?;
    let kind_byte = match kind {
        ControlKind::Snapshot => KIND_SNAPSHOT,
        ControlKind::Lobby => KIND_LOBBY,
    };
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    out.push(kind_byte);
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// Read one frame header: the message class and the payload length that
/// follows. An unknown kind or a length over the budget is a protocol
/// violation, never a reason to allocate.
pub fn decode_header(header: [u8; FRAME_HEADER_LEN]) -> Result<(ControlKind, usize), NetError> {
    let kind = match header[0] {
        KIND_SNAPSHOT => ControlKind::Snapshot,
        KIND_LOBBY => ControlKind::Lobby,
        other => return Err(NetError::BadKind(other)),
    };
    let length = usize::from(u16::from_le_bytes([header[1], header[2]]));
    if length > MAX_PACKET_BYTES {
        return Err(NetError::TooLarge);
    }
    Ok((kind, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webtransport_frame_roundtrips_both_kinds_byte_exact() {
        for kind in [ControlKind::Snapshot, ControlKind::Lobby] {
            let payload: Vec<u8> = (0..=255u8).cycle().take(MAX_PACKET_BYTES).collect();
            let framed = encode_message(kind, &payload).expect("a full budget frame fits");
            assert_eq!(framed.len(), FRAME_HEADER_LEN + MAX_PACKET_BYTES);
            let header = [framed[0], framed[1], framed[2]];
            assert_eq!(decode_header(header), Ok((kind, MAX_PACKET_BYTES)));
            assert_eq!(
                &framed[FRAME_HEADER_LEN..],
                payload.as_slice(),
                "payload untouched"
            );
        }
    }

    #[test]
    fn webtransport_frame_refuses_oversize_before_queueing() {
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert_eq!(
            encode_message(ControlKind::Snapshot, &big),
            Err(NetError::TooLarge)
        );
    }

    #[test]
    fn webtransport_frame_header_refuses_unknown_kind_and_oversize_length() {
        assert_eq!(decode_header([2, 0, 0]), Err(NetError::BadKind(2)));
        assert_eq!(decode_header([255, 1, 0]), Err(NetError::BadKind(255)));
        let over = u16::try_from(MAX_PACKET_BYTES + 1)
            .expect("the budget fits u16")
            .to_le_bytes();
        assert_eq!(
            decode_header([KIND_LOBBY, over[0], over[1]]),
            Err(NetError::TooLarge)
        );
        // The largest length a hostile u16 can name is refused the same way.
        assert_eq!(
            decode_header([KIND_LOBBY, 0xFF, 0xFF]),
            Err(NetError::TooLarge)
        );
    }
}
