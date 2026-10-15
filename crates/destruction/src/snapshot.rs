//! snapshot: one canonical snapshot of destruction state, so a net rollback
//! across a collapse resims to byte-equal collections and debris.
//!
//! Consumer: the match rollback path and the replay proof: the arena saves
//! its bonded collections and the debris ledger with [`capture`] at each
//! checkpoint and rolls back with [`restore`]. The limb pool of
//! `engine-physics` keeps its own snapshot bytes; the caller concatenates
//! them (destruction never imports physics). Data owner:
//! `engine-destruction`. Units: opaque canonical bytes. Determinism scope:
//! the bytes are the concatenation of each part's canonical bytes in the
//! order given, length-prefixed, so equal states give equal bytes on every
//! PC; the digest is FNV-1a over them. Errors: [`SnapshotError`] for
//! truncated, versioned wrong, mismatched or trailing bytes; [`restore`] is
//! atomic: it parses and rebuilds everything first and replaces the live
//! state only when every part succeeded.

use crate::bonded::{BondedCollection, BreakError};
use crate::budget::{BudgetError, DebrisLedger};
use crate::cursor::Cursor;
use engine_core::Fnv64;

/// Wire version of the world snapshot bytes.
pub const WORLD_SNAPSHOT_V1: u32 = 1;
/// Most bonded collections one snapshot carries.
pub const MAX_COLLECTIONS: usize = 64;

/// Why a restore was refused; nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The bytes are truncated, versioned wrong or carry trailing data.
    Bad,
    /// The snapshot holds a different number of collections than supplied.
    CollectionCount {
        /// Collections in the snapshot.
        have: u32,
        /// Collections supplied to restore into.
        want: u32,
    },
    /// A collection part was refused.
    Bonded(BreakError),
    /// The debris part was refused.
    Debris(BudgetError),
}

impl core::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bad => write!(f, "world snapshot bytes are not valid"),
            Self::CollectionCount { have, want } => {
                write!(
                    f,
                    "world snapshot holds {have} collections, expected {want}"
                )
            }
            Self::Bonded(e) => write!(f, "world snapshot collection refused: {e}"),
            Self::Debris(e) => write!(f, "world snapshot debris refused: {e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

fn put_part(out: &mut Vec<u8>, part: &[u8]) {
    out.extend_from_slice(&u64::try_from(part.len()).unwrap_or(u64::MAX).to_le_bytes());
    out.extend_from_slice(part);
}

/// One length-prefixed part.
fn take_part<'a>(r: &mut Cursor<'a, SnapshotError>) -> Result<&'a [u8], SnapshotError> {
    let len = usize::try_from(r.read_u64()?).map_err(|_| SnapshotError::Bad)?;
    r.take(len)
}

/// Canonical bytes of the destruction state: version, collection count,
/// each collection's bytes, then the debris ledger's, each length-prefixed.
#[must_use]
pub fn capture(collections: &[&BondedCollection], ledger: &DebrisLedger) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&WORLD_SNAPSHOT_V1.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(collections.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for c in collections {
        put_part(&mut out, &c.snapshot_bytes());
    }
    put_part(&mut out, &ledger.snapshot_bytes());
    out
}

/// Digest of capture bytes; the checkpoint receipt a replay compares.
#[must_use]
pub fn digest(bytes: &[u8]) -> u64 {
    let mut h = Fnv64::new();
    h.write(bytes);
    h.finish()
}

/// Rolls the supplied collections and ledger back to `bytes`. Atomic: on
/// any error no live state changed.
pub fn restore(
    collections: &mut [&mut BondedCollection],
    ledger: &mut DebrisLedger,
    bytes: &[u8],
) -> Result<(), SnapshotError> {
    if collections.len() > MAX_COLLECTIONS {
        return Err(SnapshotError::Bad);
    }
    let mut r = Cursor::new(bytes, SnapshotError::Bad);
    if r.read_u32()? != WORLD_SNAPSHOT_V1 {
        return Err(SnapshotError::Bad);
    }
    let have = r.read_u32()?;
    let want = u32::try_from(collections.len()).unwrap_or(u32::MAX);
    if have != want {
        return Err(SnapshotError::CollectionCount { have, want });
    }
    let mut staged: Vec<BondedCollection> = Vec::with_capacity(collections.len());
    for live in collections.iter() {
        let part = take_part(&mut r)?;
        let mut copy = (**live).clone();
        copy.restore_bytes(part).map_err(SnapshotError::Bonded)?;
        staged.push(copy);
    }
    let debris = take_part(&mut r)?;
    let rebuilt = DebrisLedger::from_snapshot_bytes(debris).map_err(SnapshotError::Debris)?;
    if !r.is_done() {
        return Err(SnapshotError::Bad);
    }
    for (live, copy) in collections.iter_mut().zip(staged) {
        **live = copy;
    }
    *ledger = rebuilt;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bonded::BreakEvent;
    use crate::budget::{arena_default_budget, ShardSeed};
    use crate::structure::{analyze, building, BuildingSpec};

    const DT: f32 = 1.0 / 60.0;

    /// One tick of the match: input (a cut at tick 2), step, freed chunks
    /// become debris shards, ledger steps. Index-derived coordinates keep
    /// the bridge free of float casts.
    fn tick(field: &mut BondedCollection, ledger: &mut DebrisLedger, cut: Option<usize>, t: u32) {
        if t == 2 {
            if let Some(c) = cut {
                field.sever_chunk(c, t).expect("in range");
            }
        }
        field.step(DT);
        let mut events = Vec::new();
        field.drain_into(&mut events);
        let seeds: Vec<ShardSeed> = events
            .iter()
            .filter_map(|e| match e {
                BreakEvent::ChunkFreed { chunk, .. } => Some(ShardSeed {
                    id: *chunk,
                    island: chunk / 64,
                    pos_mm: [i32::try_from(*chunk).unwrap_or(0) * 7, 2000, 0],
                    vel_mm_per_tick: [3, 0, -2],
                }),
                BreakEvent::BondBroken { .. } => None,
            })
            .collect();
        // The cap may drop some; duplicates never occur (a chunk frees once).
        ledger.admit(t, &seeds).expect("admits");
        ledger.step([0, 0, 0]);
    }

    fn fresh(seed: u64) -> (BondedCollection, DebrisLedger, usize) {
        let built = building(&BuildingSpec::three_story(seed)).expect("valid building");
        let mut field = BondedCollection::new(&built.desc).expect("collection builds");
        let capacity = field.max_load_ratio_q16() * 5 / 4;
        field.set_capacity_q16(capacity);
        let weakest = usize::try_from(analyze(&field).weakest.expect("stands")).expect("small");
        let ledger = DebrisLedger::new(&arena_default_budget()).expect("ledger");
        (field, ledger, weakest)
    }

    #[test]
    fn rollback_across_a_collapse_is_byte_equal_to_the_straight_run() {
        let mut agree = 0;
        for seed in 0..5u64 {
            let (mut field, mut ledger, weakest) = fresh(seed);
            // Straight run for 90 ticks, a checkpoint at tick 6 (inside the collapse).
            let mut checkpoint = None;
            for t in 0..90 {
                tick(&mut field, &mut ledger, Some(weakest), t);
                if t == 5 {
                    checkpoint = Some(capture(&[&field], &ledger));
                }
            }
            let straight = capture(&[&field], &ledger);
            assert_eq!(straight, capture(&[&field], &ledger), "bytes x2");
            // Roll back and resim the same tape.
            let bytes = checkpoint.expect("checkpoint taken");
            restore(&mut [&mut field], &mut ledger, &bytes).expect("restores");
            assert_eq!(capture(&[&field], &ledger), bytes, "restore then capture");
            for t in 6..90 {
                tick(&mut field, &mut ledger, Some(weakest), t);
            }
            if capture(&[&field], &ledger) == straight {
                agree += 1;
            }
            assert!(!field.freed_chunks().is_empty(), "seed {seed} collapsed");
        }
        assert_eq!(agree, 5, "rollback resim equals the straight run");
    }

    #[test]
    fn an_eight_tick_rollback_resims_equal_at_every_tick_of_the_collapse() {
        let (mut field, mut ledger, weakest) = fresh(3);
        let mut checked = 0;
        for t in 0..60 {
            tick(&mut field, &mut ledger, Some(weakest), t);
            if t >= 2 {
                let at = capture(&[&field], &ledger);
                // Eight ticks ahead on a copy, then back and again.
                let mut field2 = field.clone();
                let mut ledger2 = ledger.clone();
                for k in 1..=8 {
                    tick(&mut field2, &mut ledger2, Some(weakest), t + k);
                }
                let ahead = capture(&[&field2], &ledger2);
                restore(&mut [&mut field2], &mut ledger2, &at).expect("restores");
                for k in 1..=8 {
                    tick(&mut field2, &mut ledger2, Some(weakest), t + k);
                }
                assert_eq!(capture(&[&field2], &ledger2), ahead, "tick {t}");
                checked += 1;
            }
        }
        assert_eq!(checked, 58);
    }

    #[test]
    fn restore_is_atomic_and_refuses_bad_bytes() {
        let (mut field, mut ledger, weakest) = fresh(1);
        for t in 0..10 {
            tick(&mut field, &mut ledger, Some(weakest), t);
        }
        let good = capture(&[&field], &ledger);
        let before = digest(&good);
        // A bad ledger part must leave the collection untouched too.
        let mut bad = good.clone();
        let last = bad.len() - 1;
        bad[last] = 9;
        assert!(restore(&mut [&mut field], &mut ledger, &bad).is_err());
        let mut other = field.clone();
        other.step(DT);
        assert!(restore(&mut [&mut other], &mut ledger, &bad).is_err());
        assert_eq!(digest(&capture(&[&field], &ledger)), before);
        let mut wrong_count = good.clone();
        wrong_count[4] = 2;
        assert_eq!(
            restore(&mut [&mut field], &mut ledger, &wrong_count),
            Err(SnapshotError::CollectionCount { have: 2, want: 1 })
        );
        for cut in [0, 4, 7, 8, 20, good.len() - 1] {
            assert!(restore(&mut [&mut field], &mut ledger, &good[..cut]).is_err());
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(restore(&mut [&mut field], &mut ledger, &trailing).is_err());
        assert_eq!(digest(&capture(&[&field], &ledger)), before);
        assert!(restore(&mut [&mut field], &mut ledger, &good).is_ok());
    }
}
