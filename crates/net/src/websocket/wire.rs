//! wire: the layout of one WebSocket message, shared by both ends.
//!
//! One message is `class u8, payload`. The payload is the exact bytes the
//! codec produced (`encode_state_delta`, `encode_snapshot`, a lobby message)
//! and obeys the same 1200-byte budget as every other transport. The message
//! boundary is the WebSocket's own, so no length field is carried.

use crate::quic::{check_budget, ControlKind};
use crate::session::{NetError, MAX_PACKET_BYTES};

/// Header length: the class byte.
pub const HEADER_LEN: usize = 1;
/// The longest message either end accepts: header plus a full budget payload.
pub const MAX_MESSAGE_BYTES: usize = HEADER_LEN + MAX_PACKET_BYTES;

/// Class byte of an unreliable-class frame (input, state, delta).
const TAG_DATAGRAM: u8 = 0;
/// Class byte of a full snapshot message.
const TAG_SNAPSHOT: u8 = 1;
/// Class byte of a lobby message.
const TAG_LOBBY: u8 = 2;

/// What a message carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// An unreliable-class frame (inputs, states, deltas).
    Datagram,
    /// A reliable message with its stream class.
    Reliable(ControlKind),
}

/// Build one message. An over-budget payload fails before it is queued,
/// exactly like the datagram path.
pub fn encode(class: Class, payload: &[u8]) -> Result<Vec<u8>, NetError> {
    check_budget(payload)?;
    let tag = match class {
        Class::Datagram => TAG_DATAGRAM,
        Class::Reliable(ControlKind::Snapshot) => TAG_SNAPSHOT,
        Class::Reliable(ControlKind::Lobby) => TAG_LOBBY,
    };
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(tag);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Split one message into its class and payload. An empty message, an unknown
/// class or an over-budget payload is a protocol violation, never a reason to
/// allocate.
pub fn decode(message: &[u8]) -> Result<(Class, &[u8]), NetError> {
    let (&tag, payload) = message.split_first().ok_or(NetError::Truncated)?;
    let class = match tag {
        TAG_DATAGRAM => Class::Datagram,
        TAG_SNAPSHOT => Class::Reliable(ControlKind::Snapshot),
        TAG_LOBBY => Class::Reliable(ControlKind::Lobby),
        other => return Err(NetError::BadKind(other)),
    };
    check_budget(payload)?;
    Ok((class, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSES: [Class; 3] = [
        Class::Datagram,
        Class::Reliable(ControlKind::Snapshot),
        Class::Reliable(ControlKind::Lobby),
    ];

    #[test]
    fn ws_transport_wire_roundtrips_every_class_byte_exact() {
        for class in CLASSES {
            let payload: Vec<u8> = (0..=255u8).cycle().take(MAX_PACKET_BYTES).collect();
            let message = encode(class, &payload).expect("a full budget payload fits");
            assert_eq!(message.len(), MAX_MESSAGE_BYTES);
            assert_eq!(decode(&message), Ok((class, payload.as_slice())));
        }
        let empty = encode(Class::Datagram, &[]).expect("an empty payload is legal");
        assert_eq!(decode(&empty), Ok((Class::Datagram, &[][..])));
    }

    #[test]
    fn ws_transport_wire_gives_each_class_its_own_tag() {
        let tags: Vec<u8> = CLASSES
            .iter()
            .map(|&class| encode(class, b"x").expect("encodes")[0])
            .collect();
        assert_eq!(tags, vec![0, 1, 2], "the tags are the wire contract");
    }

    #[test]
    fn ws_transport_wire_refuses_oversize_before_queueing() {
        let big = vec![0u8; MAX_PACKET_BYTES + 1];
        assert_eq!(encode(Class::Datagram, &big), Err(NetError::TooLarge));
    }

    #[test]
    fn ws_transport_wire_refuses_hostile_messages() {
        assert_eq!(decode(&[]), Err(NetError::Truncated));
        assert_eq!(decode(&[3, 1, 2]), Err(NetError::BadKind(3)));
        assert_eq!(decode(&[255]), Err(NetError::BadKind(255)));
        let mut over = vec![TAG_LOBBY];
        over.extend(std::iter::repeat_n(7u8, MAX_PACKET_BYTES + 1));
        assert_eq!(decode(&over), Err(NetError::TooLarge));
    }
}
