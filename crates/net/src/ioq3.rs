//! ioq3 netchan header and fragment lane: the pattern-only header beside the
//! `KIND_*` wire path, split out of `session` when that file reached the
//! ratchet's production-line cap. No version bump, no behavior change.

use crate::session::NetError;

/// ioq3 netchan header beside the KIND path (pattern only, no version bump).
pub const IOQ3_FRAGMENT_BIT: u32 = 1 << 31;
/// ioq3 header bytes: sequence 4 + qport 2 + start 2 + length 2.
pub const IOQ3_HEADER_LEN: usize = 10;
/// ioq3 fragment payload cap (1400 minus 100, pattern only).
pub const IOQ3_FRAGMENT_SIZE: usize = 1300;
/// ioq3 OOB sequence (-1 as u32), never a data sequence.
pub const IOQ3_OOB_SEQUENCE: u32 = u32::MAX;
/// ioq3 fragment header: one sequence shared by every fragment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ioq3Header {
    /// Sequence with FRAGMENT_BIT set when fragmented.
    pub sequence: u32,
    /// Port slot for remapped ports.
    pub qport: u16,
    /// Byte offset of this fragment.
    pub start: u16,
    /// Payload bytes in this fragment.
    pub length: u16,
}
/// Encode one ioq3 header as 10 little-endian bytes.
pub fn ioq3_encode_header(h: &Ioq3Header) -> [u8; IOQ3_HEADER_LEN] {
    let mut out = [0u8; IOQ3_HEADER_LEN];
    out[0..4].copy_from_slice(&h.sequence.to_le_bytes());
    out[4..6].copy_from_slice(&h.qport.to_le_bytes());
    out[6..8].copy_from_slice(&h.start.to_le_bytes());
    out[8..10].copy_from_slice(&h.length.to_le_bytes());
    out
}
/// Decode one ioq3 header; loud refuse on short, OOB, or length mismatch.
pub fn ioq3_decode_header(bytes: &[u8]) -> Result<Ioq3Header, NetError> {
    if bytes.len() < IOQ3_HEADER_LEN {
        return Err(NetError::Truncated);
    }
    let sequence = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let qport = u16::from_le_bytes([bytes[4], bytes[5]]);
    let start = u16::from_le_bytes([bytes[6], bytes[7]]);
    let length = u16::from_le_bytes([bytes[8], bytes[9]]);
    if sequence == IOQ3_OOB_SEQUENCE {
        return Err(NetError::BadState);
    }
    let start_us = usize::from(start);
    let len_us = usize::from(length);
    if len_us > bytes.len() - IOQ3_HEADER_LEN || start_us.checked_add(len_us).is_none() {
        return Err(NetError::Truncated);
    }
    Ok(Ioq3Header {
        sequence,
        qport,
        start,
        length,
    })
}
/// Split payload into same-sequence fragments (single keeps raw sequence).
pub fn ioq3_fragment_message(sequence: u32, qport: u16, payload: &[u8]) -> Vec<Vec<u8>> {
    if payload.is_empty() {
        return Vec::new();
    }
    let count = payload.chunks(IOQ3_FRAGMENT_SIZE).len();
    let sequence = if count > 1 {
        sequence | IOQ3_FRAGMENT_BIT
    } else {
        sequence & !IOQ3_FRAGMENT_BIT
    };
    let mut out = Vec::with_capacity(count);
    let mut offset = 0usize;
    for chunk in payload.chunks(IOQ3_FRAGMENT_SIZE) {
        let start = u16::try_from(offset).unwrap_or(u16::MAX);
        let length = u16::try_from(chunk.len()).unwrap_or(u16::MAX);
        let header = Ioq3Header {
            sequence,
            qport,
            start,
            length,
        };
        let mut datagram = Vec::with_capacity(IOQ3_HEADER_LEN + chunk.len());
        datagram.extend_from_slice(&ioq3_encode_header(&header));
        datagram.extend_from_slice(chunk);
        out.push(datagram);
        offset += chunk.len();
    }
    out
}
/// Reassemble same-sequence fragments byte-exact; dupes ignored, gaps loud.
pub fn ioq3_reassemble(fragments: &[Vec<u8>]) -> Result<Vec<u8>, NetError> {
    if fragments.is_empty() {
        return Err(NetError::Truncated);
    }
    let first = ioq3_decode_header(&fragments[0])?;
    let mut parts: Vec<(usize, Vec<u8>)> = Vec::with_capacity(fragments.len());
    for datagram in fragments {
        let header = ioq3_decode_header(datagram)?;
        if header.sequence != first.sequence || header.qport != first.qport {
            return Err(NetError::BadState);
        }
        let body = datagram[IOQ3_HEADER_LEN..].to_vec();
        if body.len() != usize::from(header.length) {
            return Err(NetError::Truncated);
        }
        parts.push((usize::from(header.start), body));
    }
    parts.sort_by_key(|(start, _)| *start);
    let mut out = Vec::new();
    let mut next = 0usize;
    for (start, body) in &parts {
        if *start == next {
            out.extend_from_slice(body);
            next += body.len();
        } else if *start < next {
            let end = start.saturating_add(body.len());
            if end > next || out[*start..end] != body[..] {
                return Err(NetError::BadState);
            }
        } else {
            return Err(NetError::Truncated);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioq3_fragments_reassemble() {
        let sent: Vec<u8> = (0..7000)
            .map(|i| u8::try_from((i * 31 + 7) % 251).expect("pattern fits"))
            .collect();
        let fragments = ioq3_fragment_message(42, 7, &sent);
        assert_eq!(fragments.len(), 6, "7000B needs 6 fragments at 1300B");
        let headers: Vec<Ioq3Header> = fragments
            .iter()
            .map(|d| ioq3_decode_header(d).expect("header"))
            .collect();
        assert!(headers.iter().all(|h| h.sequence == headers[0].sequence));
        assert_ne!(headers[0].sequence & IOQ3_FRAGMENT_BIT, 0);
        // Seeded drop every 5th datagram leaves a gap the assembler names.
        let mut delivered: Vec<Vec<u8>> = fragments
            .iter()
            .enumerate()
            .filter(|(i, _)| i % 5 != 4)
            .map(|(_, d)| d.clone())
            .collect();
        assert_eq!(ioq3_reassemble(&delivered), Err(NetError::Truncated));
        // Retransmit the dropped fragment, then tolerate one dupe plus reorder.
        delivered.push(fragments[4].clone());
        delivered.push(fragments[0].clone());
        delivered.swap(0, 1);
        let rebuilt = ioq3_reassemble(&delivered).expect("reassemble");
        assert_eq!(rebuilt.len(), sent.len(), "reassembly diff must be 0 bytes");
        assert_eq!(rebuilt, sent, "reassembly must be byte-exact");
        // Bad headers refuse loud: short bytes and the OOB sequence.
        assert!(ioq3_decode_header(&fragments[0][..3]).is_err());
        let oob = ioq3_encode_header(&Ioq3Header {
            sequence: IOQ3_OOB_SEQUENCE,
            qport: 7,
            start: 0,
            length: 0,
        });
        assert_eq!(ioq3_decode_header(&oob), Err(NetError::BadState));
    }
}
