//! rollback: bounded spine-byte checkpoints for net resimulation.
//!
//! Consumer: net resim after a loopback drop or rejoin. Data owner: the
//! caller owns the [`RollbackRing`]; spine bytes stay opaque here so `save`
//! never imports arena or ecs code. Units: ticks (`u64`), bytes.
//! Determinism: same tick plus same spine bytes give the same frame bytes;
//! convergence is byte equality on the spine payload. Errors: [`RollbackErr`].

use std::collections::VecDeque;

/// Frame schema version. Bump only with a hash-law receipt.
pub const ROLLBACK_SCHEMA_VERSION: u8 = 1;
/// Frame versions this slice opens.
pub const SUPPORTED_ROLLBACK_VERSIONS: &[u8] = &[1];
/// Most checkpoints one ring holds (mirrors net `MAX_ROLLBACK`).
pub const MAX_ROLLBACK_FRAMES: usize = 128;
/// Most spine bytes one frame carries (32 slots encode ~2.8 KiB).
pub const MAX_SPINE_BYTES: usize = 65_536;
/// Spine snapshot version word `save` accepts (`engine-ecs` `SPINE_VERSION`).
pub const SPINE_VERSION_EXPECTED: u16 = 1;

/// Frame header length: version(1) + tick(8) + length(4).
pub const FRAME_HEADER_LEN: usize = 13;

/// Why a rollback frame was refused. Nothing is forged on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackErr {
    /// Frame version outside [`SUPPORTED_ROLLBACK_VERSIONS`].
    BadVersion,
    /// Truncated header, short payload, or trailing bytes.
    Truncated,
    /// Spine payload over [`MAX_SPINE_BYTES`].
    TooLarge,
    /// No checkpoint at that tick.
    UnknownTick,
    /// Spine bytes fail the snapshot gate (short or wrong version word).
    BadSpine,
}

impl std::fmt::Display for RollbackErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RollbackErr::BadVersion => f.write_str("rollback bad version"),
            RollbackErr::Truncated => f.write_str("rollback truncated"),
            RollbackErr::TooLarge => f.write_str("rollback spine too large"),
            RollbackErr::UnknownTick => f.write_str("rollback unknown tick"),
            RollbackErr::BadSpine => f.write_str("rollback bad spine bytes"),
        }
    }
}

impl std::error::Error for RollbackErr {}

/// One checkpoint: the tick plus opaque `spine::snapshot` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackFrame {
    /// Fixed tick the spine bytes were captured at.
    pub tick: u64,
    /// Opaque `Spine::snapshot` bytes (versioned by the spine itself).
    pub spine: Vec<u8>,
}

/// True when the bytes look like a spine snapshot: at least a version word
/// plus a slot count, with the expected version word first.
///
/// Single owner of the snapshot gate: `persist` routes its seal and open
/// paths through this body, so the `dup_fns` clone group cannot reappear.
pub(crate) fn spine_gate(spine: &[u8]) -> bool {
    if spine.len() < 6 || spine.len() > MAX_SPINE_BYTES {
        return false;
    }
    u16::from_le_bytes([spine[0], spine[1]]) == SPINE_VERSION_EXPECTED
}

/// Encode one frame: version(1) + tick LE(8) + length LE(4) + spine bytes.
///
/// Rejects oversize payloads and non-snapshot bytes before allocating.
pub fn encode_frame(tick: u64, spine: &[u8]) -> Result<Vec<u8>, RollbackErr> {
    if spine.len() > MAX_SPINE_BYTES {
        return Err(RollbackErr::TooLarge);
    }
    if !spine_gate(spine) {
        return Err(RollbackErr::BadSpine);
    }
    let len = u32::try_from(spine.len()).map_err(|_| RollbackErr::TooLarge)?;
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + spine.len());
    out.push(ROLLBACK_SCHEMA_VERSION);
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(spine);
    Ok(out)
}

/// Decode one frame. Exact length only: short payloads and trailing bytes
/// both fail, so a dropped datagram never decodes into a guess.
pub fn decode_frame(bytes: &[u8]) -> Result<RollbackFrame, RollbackErr> {
    if bytes.len() < FRAME_HEADER_LEN {
        return Err(RollbackErr::Truncated);
    }
    if bytes[0] > ROLLBACK_SCHEMA_VERSION || !SUPPORTED_ROLLBACK_VERSIONS.contains(&bytes[0]) {
        return Err(RollbackErr::BadVersion);
    }
    let mut tick_b = [0u8; 8];
    let mut len_b = [0u8; 4];
    tick_b.copy_from_slice(&bytes[1..9]);
    len_b.copy_from_slice(&bytes[9..13]);
    let len = super::len_usize(len_b);
    if len > MAX_SPINE_BYTES {
        return Err(RollbackErr::TooLarge);
    }
    if bytes.len() != FRAME_HEADER_LEN + len {
        return Err(RollbackErr::Truncated);
    }
    let spine = bytes[FRAME_HEADER_LEN..].to_vec();
    if !spine_gate(&spine) {
        return Err(RollbackErr::BadSpine);
    }
    Ok(RollbackFrame {
        tick: u64::from_le_bytes(tick_b),
        spine,
    })
}

/// True when two spine payloads converge: byte equality, the resim target
/// after a drop/rejoin over the same channel.
#[must_use]
pub fn converges(a: &[u8], b: &[u8]) -> bool {
    a == b
}

/// Bounded checkpoint ring for resimulation. Ticks must arrive in order;
/// the oldest frame evicts past capacity.
#[derive(Clone, Debug)]
pub struct RollbackRing {
    frames: VecDeque<RollbackFrame>,
    cap: usize,
}

impl RollbackRing {
    /// Empty ring holding at most `cap` frames (clamped to `1..=128`).
    pub fn new(cap: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            cap: cap.clamp(1, MAX_ROLLBACK_FRAMES),
        }
    }

    /// Capacity after clamping.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Stored frame count.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when no checkpoint is stored.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Latest tick stored, if any.
    pub fn latest_tick(&self) -> Option<u64> {
        self.frames.back().map(|frame| frame.tick)
    }

    /// Push one checkpoint. The spine bytes pass the snapshot gate first,
    /// so a corrupt capture never enters the ring.
    pub fn push(&mut self, tick: u64, spine: &[u8]) -> Result<(), RollbackErr> {
        if spine.len() > MAX_SPINE_BYTES {
            return Err(RollbackErr::TooLarge);
        }
        if !spine_gate(spine) {
            return Err(RollbackErr::BadSpine);
        }
        self.frames.push_back(RollbackFrame {
            tick,
            spine: spine.to_vec(),
        });
        while self.frames.len() > self.cap {
            self.frames.pop_front();
        }
        Ok(())
    }

    /// Borrow the spine bytes stored at `tick`.
    pub fn get(&self, tick: u64) -> Result<&[u8], RollbackErr> {
        self.frames
            .iter()
            .find(|frame| frame.tick == tick)
            .map(|frame| frame.spine.as_slice())
            .ok_or(RollbackErr::UnknownTick)
    }

    /// Drop every frame after `tick` (the resim window replays from `tick`).
    /// Returns the dropped count; unknown ticks change nothing.
    pub fn rollback_to(&mut self, tick: u64) -> Result<usize, RollbackErr> {
        let pos = self
            .frames
            .iter()
            .position(|frame| frame.tick == tick)
            .ok_or(RollbackErr::UnknownTick)?;
        let dropped = self.frames.len() - pos - 1;
        self.frames.truncate(pos + 1);
        Ok(dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smallest bytes that pass the spine gate: version word plus count.
    fn spine_bytes(fill: u8) -> Vec<u8> {
        let mut out = vec![0u8; 16];
        out[0..2].copy_from_slice(&SPINE_VERSION_EXPECTED.to_le_bytes());
        for b in out.iter_mut().skip(6) {
            *b = fill;
        }
        out
    }

    #[test]
    fn frame_roundtrip_x2_is_byte_equal() {
        let spine = spine_bytes(7);
        let (a, b) = (
            encode_frame(600, &spine).expect("valid frame encodes"),
            encode_frame(600, &spine).expect("valid frame encodes"),
        );
        assert_eq!(a, b, "same tick and spine give same bytes");
        let frame = decode_frame(&a).expect("roundtrip decodes");
        assert_eq!(frame.tick, 600);
        assert_eq!(frame.spine, spine);
        assert!(converges(&frame.spine, &spine));
    }

    #[test]
    fn loopback_drop_and_rejoin_converge_on_spine_bytes() {
        // Two rings over the same channel: server and rejoining client.
        let (mut server, mut client) = (RollbackRing::new(128), RollbackRing::new(128));
        for tick in 0..10u64 {
            let spine = spine_bytes(tick as u8);
            server.push(tick, &spine).expect("server stores");
            // The loopback drops tick 5 on the client; the rejoin replays it.
            if tick != 5 {
                client.push(tick, &spine).expect("client stores");
            }
        }
        assert_eq!(client.get(5), Err(RollbackErr::UnknownTick));
        // Resync: the missing checkpoint arrives and both sides agree.
        let missing = server.get(5).expect("server kept the drop").to_vec();
        client.push(5, &missing).expect("rejoin replays the drop");
        assert!(converges(server.get(5).unwrap(), client.get(5).unwrap()));
        // Resim from the drop point replays the suffix identically.
        client.rollback_to(5).expect("rewind to the drop");
        for tick in 6..10u64 {
            client
                .push(tick, server.get(tick).unwrap())
                .expect("suffix replays");
        }
        for tick in 0..10u64 {
            if tick == 5 {
                continue;
            }
            assert!(
                converges(server.get(tick).unwrap(), client.get(tick).unwrap()),
                "tick {tick} converges"
            );
        }
    }

    #[test]
    fn version_length_and_spine_gates_are_loud() {
        let spine = spine_bytes(1);
        let mut bad = encode_frame(3, &spine).expect("valid frame encodes");
        bad[0] = ROLLBACK_SCHEMA_VERSION + 1;
        assert_eq!(decode_frame(&bad), Err(RollbackErr::BadVersion));
        assert_eq!(decode_frame(&[]), Err(RollbackErr::Truncated));
        assert_eq!(
            decode_frame(&encode_frame(3, &spine).unwrap()[..10]),
            Err(RollbackErr::Truncated)
        );
        let mut trailing = encode_frame(3, &spine).expect("valid frame encodes");
        trailing.push(0);
        assert_eq!(decode_frame(&trailing), Err(RollbackErr::Truncated));
        let big = vec![0u8; MAX_SPINE_BYTES + 1];
        assert_eq!(encode_frame(0, &big), Err(RollbackErr::TooLarge));
        assert_eq!(encode_frame(0, &[0u8; 3]), Err(RollbackErr::BadSpine));
        let mut wrong = spine.clone();
        wrong[0..2].copy_from_slice(&9u16.to_le_bytes());
        assert_eq!(encode_frame(0, &wrong), Err(RollbackErr::BadSpine));
    }

    #[test]
    fn ring_evicts_oldest_and_rewinds_the_suffix() {
        let mut ring = RollbackRing::new(4);
        assert_eq!(ring.cap(), 4);
        for tick in 0..6u64 {
            ring.push(tick, &spine_bytes(tick as u8))
                .expect("push works");
        }
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.get(0), Err(RollbackErr::UnknownTick));
        assert_eq!(ring.latest_tick(), Some(5));
        assert_eq!(ring.rollback_to(3).expect("rewind works"), 2);
        assert_eq!(ring.latest_tick(), Some(3));
        assert_eq!(ring.rollback_to(9), Err(RollbackErr::UnknownTick));
        assert!(RollbackRing::new(0).cap() == 1);
        assert!(RollbackRing::new(9999).cap() == MAX_ROLLBACK_FRAMES);
    }
}
