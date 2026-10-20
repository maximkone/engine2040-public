//! persist: versioned save payload for spine bytes.
//!
//! Consumer: the save envelope (`save_generation` seals what this module
//! encodes). Data owner: the caller owns seed, tick and the opaque
//! `spine::snapshot` bytes; `save` never imports arena or ecs code. Units:
//! seeds and ticks (`u64`), bytes. Determinism: same inputs give same
//! payload bytes; the save round-trip seals twice and compares bytes.
//! Errors: [`PersistErr`].

use super::rollback::{spine_gate, MAX_SPINE_BYTES};

/// Payload schema version. Bump only with a hash-law receipt.
pub const PERSIST_SCHEMA_VERSION: u8 = 1;
/// Payload versions this slice opens and migrates.
pub const SUPPORTED_PERSIST_VERSIONS: &[u8] = &[1];
/// Payload header length: version(1) + seed(8) + tick(8) + length(4).
pub const PERSIST_HEADER_LEN: usize = 21;

/// Why a persisted spine payload was refused. Nothing is forged on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistErr {
    /// Payload version outside [`SUPPORTED_PERSIST_VERSIONS`].
    BadVersion,
    /// Truncated header, short spine, or trailing bytes.
    Truncated,
    /// Spine payload over [`MAX_SPINE_BYTES`].
    TooLarge,
    /// Spine bytes fail the snapshot gate (short or wrong version word).
    BadSpine,
}

impl std::fmt::Display for PersistErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistErr::BadVersion => f.write_str("persist bad version"),
            PersistErr::Truncated => f.write_str("persist truncated"),
            PersistErr::TooLarge => f.write_str("persist spine too large"),
            PersistErr::BadSpine => f.write_str("persist bad spine bytes"),
        }
    }
}

impl std::error::Error for PersistErr {}

/// One decoded persistence payload: the envelope coordinates plus the
/// opaque spine snapshot the game restores through `Spine::restore`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedSpine {
    /// Payload schema version (always [`PERSIST_SCHEMA_VERSION`]).
    pub version: u8,
    /// Seed the spine was baked under.
    pub seed: u64,
    /// Tick the spine was captured at.
    pub tick: u64,
    /// Opaque `Spine::snapshot` bytes.
    pub spine: Vec<u8>,
}

/// Encode the envelope payload: version(1) + seed LE(8) + tick LE(8) +
/// length LE(4) + spine bytes.
///
/// Rejects oversize payloads and non-snapshot bytes before allocating, so
/// a bad capture never reaches the atomic write.
pub fn seal_spine(seed: u64, tick: u64, spine: &[u8]) -> Result<Vec<u8>, PersistErr> {
    if spine.len() > MAX_SPINE_BYTES {
        return Err(PersistErr::TooLarge);
    }
    if !spine_gate(spine) {
        return Err(PersistErr::BadSpine);
    }
    let len = u32::try_from(spine.len()).map_err(|_| PersistErr::TooLarge)?;
    let mut out = Vec::with_capacity(PERSIST_HEADER_LEN + spine.len());
    out.push(PERSIST_SCHEMA_VERSION);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(spine);
    Ok(out)
}

/// Decode and verify one payload. Exact length only: short spines and
/// trailing bytes both fail, so a torn write never loads into a guess.
pub fn open_spine(bytes: &[u8]) -> Result<PersistedSpine, PersistErr> {
    if bytes.len() < PERSIST_HEADER_LEN {
        return Err(PersistErr::Truncated);
    }
    if bytes[0] > PERSIST_SCHEMA_VERSION || !SUPPORTED_PERSIST_VERSIONS.contains(&bytes[0]) {
        return Err(PersistErr::BadVersion);
    }
    let mut seed_b = [0u8; 8];
    let mut tick_b = [0u8; 8];
    let mut len_b = [0u8; 4];
    seed_b.copy_from_slice(&bytes[1..9]);
    tick_b.copy_from_slice(&bytes[9..17]);
    len_b.copy_from_slice(&bytes[17..21]);
    let len = super::len_usize(len_b);
    if len > MAX_SPINE_BYTES {
        return Err(PersistErr::TooLarge);
    }
    if bytes.len() != PERSIST_HEADER_LEN + len {
        return Err(PersistErr::Truncated);
    }
    let spine = bytes[PERSIST_HEADER_LEN..].to_vec();
    if !spine_gate(&spine) {
        return Err(PersistErr::BadSpine);
    }
    Ok(PersistedSpine {
        version: bytes[0],
        seed: u64::from_le_bytes(seed_b),
        tick: u64::from_le_bytes(tick_b),
        spine,
    })
}

/// Migrate a decoded payload to the current schema (v1: passthrough).
/// Rejects versions outside [`SUPPORTED_PERSIST_VERSIONS`].
pub fn migrate_spine(saved: &PersistedSpine) -> Result<PersistedSpine, PersistErr> {
    if !SUPPORTED_PERSIST_VERSIONS.contains(&saved.version) {
        return Err(PersistErr::BadVersion);
    }
    Ok(saved.clone())
}

#[cfg(test)]
mod tests {
    use super::super::rollback::SPINE_VERSION_EXPECTED;
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
    fn save_roundtrip_x2_is_byte_equal() {
        let spine = spine_bytes(11);
        let (a, b) = (
            seal_spine(2040, 600, &spine).expect("valid spine seals"),
            seal_spine(2040, 600, &spine).expect("valid spine seals"),
        );
        assert_eq!(a, b, "same inputs give same payload bytes");
        let opened = open_spine(&a).expect("roundtrip opens");
        assert_eq!(
            (opened.version, opened.seed, opened.tick),
            (PERSIST_SCHEMA_VERSION, 2040, 600)
        );
        assert_eq!(opened.spine, spine);
        assert_eq!(migrate_spine(&opened).unwrap(), opened);
        // The payload rides the envelope round-trip unchanged.
        let sealed = super::super::seal(PERSIST_SCHEMA_VERSION, 2040, 600, &a)
            .expect("payload fits the envelope");
        let env = super::super::open(&sealed).expect("envelope opens");
        assert_eq!(open_spine(&env.payload).unwrap(), opened);
    }

    #[test]
    fn real_spine_save_roundtrip_x2_is_byte_equal() {
        use engine_ecs::spine::{Body, Spine, Transform, Vitals};

        let mut spine = Spine::with_capacity(2);
        for slot in 0..2 {
            let at = f32::from(i16::try_from(slot).expect("slot fits i16"));
            spine
                .spawn(
                    Transform {
                        pos: [at + 1.5, 0.0, at - 2.5],
                        yaw: at * 0.5,
                    },
                    Body::default(),
                    Vitals {
                        hp_permille: 1000 - u16::try_from(slot).expect("slot fits u16"),
                        stamina: 80,
                        poise: 4,
                        state: Vitals::ATTACKING,
                    },
                )
                .expect("fixture has room");
        }
        let snapshot = spine.snapshot();

        // Two independent seals of the real spine are byte-identical.
        let (first, second) = (
            seal_spine(2040, 600, &snapshot.bytes).expect("real spine seals"),
            seal_spine(2040, 600, &snapshot.bytes).expect("real spine seals"),
        );
        assert_eq!(first, second, "same spine seals the same payload x2");

        // The payload restores through the spine codec, twice.
        for payload in [&first, &second] {
            let opened = open_spine(payload).expect("payload opens");
            assert_eq!((opened.seed, opened.tick), (2040, 600));
            let restored = Spine::restore(&opened.spine).expect("spine restores");
            assert_eq!(
                restored.snapshot().bytes,
                snapshot.bytes,
                "restore then snapshot is byte-equal"
            );
        }

        // The envelope round-trip keeps the payload and the spine identity.
        let sealed = super::super::seal(PERSIST_SCHEMA_VERSION, 2040, 600, &first)
            .expect("payload fits the envelope");
        let env = super::super::open(&sealed).expect("envelope opens");
        let again = open_spine(&env.payload).expect("payload opens again");
        assert_eq!(again.spine, snapshot.bytes);
        assert_eq!(
            Spine::restore(&again.spine)
                .expect("spine restores from the envelope")
                .snapshot()
                .bytes,
            snapshot.bytes
        );
    }

    #[test]
    fn version_length_and_spine_gates_are_loud() {
        let spine = spine_bytes(2);
        let mut bad = seal_spine(1, 2, &spine).expect("valid spine seals");
        bad[0] = PERSIST_SCHEMA_VERSION + 1;
        assert_eq!(open_spine(&bad), Err(PersistErr::BadVersion));
        assert_eq!(open_spine(&[]), Err(PersistErr::Truncated));
        assert_eq!(
            open_spine(&seal_spine(1, 2, &spine).unwrap()[..10]),
            Err(PersistErr::Truncated)
        );
        let mut trailing = seal_spine(1, 2, &spine).expect("valid spine seals");
        trailing.push(0);
        assert_eq!(open_spine(&trailing), Err(PersistErr::Truncated));
        let big = vec![0u8; MAX_SPINE_BYTES + 1];
        assert_eq!(seal_spine(1, 2, &big), Err(PersistErr::TooLarge));
        assert_eq!(seal_spine(1, 2, &[0u8; 3]), Err(PersistErr::BadSpine));
        let mut wrong = spine.clone();
        wrong[0..2].copy_from_slice(&9u16.to_le_bytes());
        assert_eq!(seal_spine(1, 2, &wrong), Err(PersistErr::BadSpine));
        assert_eq!(
            migrate_spine(&PersistedSpine {
                version: 0,
                seed: 0,
                tick: 0,
                spine,
            }),
            Err(PersistErr::BadVersion)
        );
    }

    /// Single-owner pin: the spine snapshot gate lives in `rollback` and
    /// `persist` routes through it, so the `dup_fns` clone group of the two
    /// `spine_gate` copies cannot reappear. Fails to compile while persist
    /// keeps its own private copy.
    #[test]
    fn spine_gate_is_owned_once_by_rollback() {
        let good = spine_bytes(9);
        assert!(super::super::rollback::spine_gate(&good));
        assert!(!super::super::rollback::spine_gate(&[0u8; 3]));
        // Behavior agreement: the shared gate decides what both seal paths
        // accept, so folding the copy moves no boundary.
        assert!(seal_spine(1, 2, &good).is_ok());
        assert_eq!(seal_spine(1, 2, &[0u8; 3]), Err(PersistErr::BadSpine));
    }
}
