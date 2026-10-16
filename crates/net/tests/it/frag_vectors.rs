// Attribution: fragment shape inspired by naia (MIT) `message_fragmenter`
// with `FRAGMENTATION_LIMIT` 400B; reimplemented fresh here, no donor bytes.
// Link: https://github.com/naia-rs/naia (MIT). Pattern only.
//! Reliable-only fragmentation vectors: 1500B+ event reassembles byte-exact,
//! oversize unordered-unreliable refuses loud, malformed drops are counted.
//! Owner: engine-net tests. Units: bytes. Deterministic: fixed fixtures only.
//! Errors: typed `FragError` with `Display`, no panics on wire bytes.

use std::collections::BTreeMap;
use std::fmt;

/// Max payload bytes per fragment, mirroring the naia 400B shape.
const FRAGMENTATION_LIMIT: usize = 400;
/// Wire header: magic(1) + msg id(2) + index(2) + count(2) + total len(4).
const HEADER_LEN: usize = 11;
/// Magic byte guarding against garbage datagrams.
const FRAG_MAGIC: u8 = 0x46;
/// Largest reassembled message accepted (10x the fragment limit).
const MAX_MESSAGE_LEN: usize = 4_000;

/// Channel under test: only reliable fragments; unreliable never fragments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Reliable,
    UnorderedUnreliable,
}

/// Typed refusal for oversize and malformed wire bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FragError {
    /// Unreliable payload over the single-datagram limit.
    Oversize { len: usize, limit: usize },
    /// Wire bytes fail the header or consistency checks.
    Malformed(&'static str),
    /// Fragments seen but the message is not whole yet.
    Incomplete,
}

impl fmt::Display for FragError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversize { len, limit } => {
                write!(f, "oversize: {len}B over unreliable limit {limit}B")
            }
            Self::Malformed(why) => write!(f, "malformed fragment: {why}"),
            Self::Incomplete => write!(f, "incomplete message"),
        }
    }
}

/// Deterministic 1537B payload: non-multiple of 400B, fixed pattern.
fn reliable_fixture(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from((i * 31 + 7) % 251).expect("pattern fits in u8"))
        .collect()
}

/// Split a payload into wire fragments; unreliable never fragments.
fn fragment(channel: Channel, msg_id: u16, payload: &[u8]) -> Result<Vec<Vec<u8>>, FragError> {
    if channel == Channel::UnorderedUnreliable && payload.len() > FRAGMENTATION_LIMIT {
        return Err(FragError::Oversize {
            len: payload.len(),
            limit: FRAGMENTATION_LIMIT,
        });
    }
    if payload.len() > MAX_MESSAGE_LEN {
        return Err(FragError::Oversize {
            len: payload.len(),
            limit: MAX_MESSAGE_LEN,
        });
    }
    let total = u32::try_from(payload.len()).expect("fixture under 4GB");
    let count = payload.len().div_ceil(FRAGMENTATION_LIMIT);
    let count_u16 = u16::try_from(count).expect("fixture needs few fragments");
    let mut out = Vec::with_capacity(count.max(1));
    for (index, chunk) in payload.chunks(FRAGMENTATION_LIMIT).enumerate() {
        let index_u16 = u16::try_from(index).expect("fixture needs few fragments");
        let mut wire = Vec::with_capacity(HEADER_LEN + chunk.len());
        wire.push(FRAG_MAGIC);
        wire.extend_from_slice(&msg_id.to_le_bytes());
        wire.extend_from_slice(&index_u16.to_le_bytes());
        wire.extend_from_slice(&count_u16.to_le_bytes());
        wire.extend_from_slice(&total.to_le_bytes());
        wire.extend_from_slice(chunk);
        out.push(wire);
    }
    Ok(out)
}

/// Decoded header plus payload slice bounds.
struct Decoded {
    msg_id: u16,
    index: usize,
    count: usize,
    total: usize,
}

/// Validate one wire datagram without panicking on short/garbage input.
fn decode_fragment(wire: &[u8]) -> Result<(Decoded, &[u8]), FragError> {
    if wire.len() < HEADER_LEN {
        return Err(FragError::Malformed("short header"));
    }
    if wire[0] != FRAG_MAGIC {
        return Err(FragError::Malformed("bad magic"));
    }
    let msg_id = u16::from_le_bytes([wire[1], wire[2]]);
    let index = usize::from(u16::from_le_bytes([wire[3], wire[4]]));
    let count = usize::from(u16::from_le_bytes([wire[5], wire[6]]));
    let total = usize::try_from(u32::from_le_bytes([wire[7], wire[8], wire[9], wire[10]]))
        .expect("u32 fits in usize on supported targets");
    if count == 0 || count > 16 {
        return Err(FragError::Malformed("bad count"));
    }
    if index >= count {
        return Err(FragError::Malformed("index out of range"));
    }
    if total == 0 || total > MAX_MESSAGE_LEN {
        return Err(FragError::Malformed("bad total"));
    }
    let body = &wire[HEADER_LEN..];
    if body.len() > FRAGMENTATION_LIMIT {
        return Err(FragError::Malformed("fragment over limit"));
    }
    if body.is_empty() {
        return Err(FragError::Malformed("empty fragment"));
    }
    Ok((
        Decoded {
            msg_id,
            index,
            count,
            total,
        },
        body,
    ))
}

/// Reliable reassembler: accepts out-of-order fragments, counts malformed.
#[derive(Debug, Default)]
struct ReliableReassembler {
    msg_id: Option<u16>,
    count: usize,
    total: usize,
    parts: BTreeMap<usize, Vec<u8>>,
    dropped_malformed: u64,
}

impl ReliableReassembler {
    /// Feed one datagram; `Ok(None)` means still incomplete, `Ok(Some(_))`
    /// is the byte-exact message. Malformed input bumps the drop counter.
    fn push(&mut self, wire: &[u8]) -> Result<Option<Vec<u8>>, FragError> {
        let (head, body) = decode_fragment(wire).map_err(|e| {
            self.dropped_malformed += 1;
            e
        })?;
        if head.total.div_ceil(FRAGMENTATION_LIMIT) != head.count {
            self.dropped_malformed += 1;
            return Err(FragError::Malformed("count disagrees with total"));
        }
        match self.msg_id {
            None => {
                self.msg_id = Some(head.msg_id);
                self.count = head.count;
                self.total = head.total;
            }
            Some(id) if id == head.msg_id => {
                if head.count != self.count || head.total != self.total {
                    self.dropped_malformed += 1;
                    return Err(FragError::Malformed("inconsistent message"));
                }
            }
            Some(_) => {
                self.dropped_malformed += 1;
                return Err(FragError::Malformed("mixed message id"));
            }
        }
        self.parts
            .entry(head.index)
            .or_insert_with(|| body.to_vec());
        if self.parts.len() < self.count {
            return Err(FragError::Incomplete);
        }
        let mut msg = Vec::with_capacity(self.total);
        for index in 0..self.count {
            match self.parts.get(&index) {
                Some(part) => msg.extend_from_slice(part),
                None => return Err(FragError::Incomplete),
            }
        }
        msg.truncate(self.total);
        if msg.len() != self.total {
            self.dropped_malformed += 1;
            return Err(FragError::Malformed("length mismatch"));
        }
        Ok(Some(msg))
    }
}

#[test]
fn reliable_event_reassembles_byte_exact_within_overhead() {
    let payload = reliable_fixture(1_537);
    assert!(payload.len() > 1_500);
    let mut frags = fragment(Channel::Reliable, 7, &payload).expect("reliable fragments");
    assert_eq!(frags.len(), 4);
    // Reverse delivery proves order independence.
    frags.reverse();
    let wire_total: usize = frags.iter().map(Vec::len).sum();
    let overhead = wire_total - payload.len();
    assert!(
        overhead * 10 <= payload.len(),
        "frag overhead {overhead}B over 10% of {}B",
        payload.len()
    );
    let mut rx = ReliableReassembler::default();
    let mut done = None;
    for wire in &frags {
        match rx.push(wire) {
            Ok(Some(msg)) => done = Some(msg),
            Ok(None) => {}
            Err(FragError::Incomplete) => {}
            Err(other) => panic!("valid fragment refused: {other}"),
        }
    }
    let msg = done.expect("four fragments complete the message");
    assert_eq!(msg, payload, "reassembly must be byte-exact");
    assert_eq!(rx.dropped_malformed, 0);
}

#[test]
fn unordered_unreliable_oversize_refuses_loud() {
    let just_over = reliable_fixture(FRAGMENTATION_LIMIT + 1);
    let big = reliable_fixture(1_537);
    for payload in [&just_over, &big] {
        let err = fragment(Channel::UnorderedUnreliable, 3, payload)
            .expect_err("unreliable oversize must refuse");
        assert_eq!(
            err,
            FragError::Oversize {
                len: payload.len(),
                limit: FRAGMENTATION_LIMIT
            }
        );
        assert!(!err.to_string().is_empty(), "refusal must name the cause");
    }
    // The limit itself still sends as one datagram.
    let edge = reliable_fixture(FRAGMENTATION_LIMIT);
    let frags = fragment(Channel::UnorderedUnreliable, 3, &edge).expect("limit fits");
    assert_eq!(frags.len(), 1);
    assert_eq!(frags[0].len(), HEADER_LEN + FRAGMENTATION_LIMIT);
}

#[test]
fn malformed_drops_are_counted_and_link_stays_usable() {
    let payload = reliable_fixture(1_537);
    let frags = fragment(Channel::Reliable, 9, &payload).expect("reliable fragments");
    let mut rx = ReliableReassembler::default();
    let bad_magic = [
        0x00, 0x46, 0x09, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01, 0x06, 0x00, 0xAA,
    ];
    let short: &[u8] = &[FRAG_MAGIC, 0x01];
    let empty: &[u8] = &[];
    let mut index_oob = frags[0].clone();
    index_oob[3] = 0xFF;
    index_oob[4] = 0xFF;
    let mut truncated = frags[1].clone();
    truncated.truncate(4);
    for bad in [&bad_magic[..], short, empty, &index_oob[..], &truncated[..]] {
        assert!(
            rx.push(bad).is_err(),
            "malformed wire must not decode silently"
        );
    }
    assert_eq!(rx.dropped_malformed, 5);
    // The same reassembler still completes the real message afterwards.
    let mut done = None;
    for wire in &frags {
        match rx.push(wire) {
            Ok(Some(msg)) => done = Some(msg),
            Ok(None) => {}
            Err(FragError::Incomplete) => {}
            Err(other) => panic!("valid fragment refused after drops: {other}"),
        }
    }
    assert_eq!(done.expect("link stays usable"), payload);
    assert_eq!(
        rx.dropped_malformed, 5,
        "valid fragments never count as drops"
    );
}
