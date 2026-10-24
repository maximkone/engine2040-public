//! blast: an authority explosion turned into impacts, so the existing bond rule decides what
//! breaks.
//!
//! Consumer: the arena match, when an explosion goes off; the break events it queues drain like
//! any other and the picture (`engine-physfx`, never imported here) shoves its own debris from
//! the same integers. Data owner: `engine-destruction`. Units: millimeters, shard-unit impulses.
//! Determinism scope: integer distance (`isqrt` on `u64`, no float root), integer falloff, then
//! the same index-order `apply_impact` the sword uses, so a recorded blast replays to the same
//! events and the same `state_hash` on every PC.

use super::{BondedCollection, BreakError};
use engine_core::checked::f32_from_u64;

/// Largest accepted blast reach, millimeters (100 m).
pub const MAX_BLAST_RADIUS_MM: u32 = 100_000;
/// Largest accepted blast center coordinate, millimeters.
const MAX_CENTER_MM: u32 = 1_000_000;

/// A chunk coordinate in meters as whole millimeters, truncating toward zero.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "bonded chunk coordinates are validated finite and well inside +-1e6 m, so the millimetre product is exact in f32 and fits i64 with room to spare"
)]
fn mm(meters: f32) -> i64 {
    (meters * 1000.0) as i64
}

impl BondedCollection {
    /// Applies an explosion at `center_mm` with `radius_mm` reach and `impulse` shard units at the
    /// center, falling off linearly to nothing at the radius, as one impact per chunk inside it
    /// in index order (so the bond rule, the threshold and the support refresh are exactly the
    /// sword's). A chunk at the radius or beyond is not touched. Returns how many chunks were
    /// struck, whether or not the impact was strong enough to break a bond. A bad blast is
    /// refused with [`BreakError::BadBlast`] and changes nothing.
    pub fn apply_blast(
        &mut self,
        center_mm: [i32; 3],
        radius_mm: u32,
        impulse: u32,
        tick: u32,
    ) -> Result<usize, BreakError> {
        if radius_mm == 0
            || radius_mm > MAX_BLAST_RADIUS_MM
            || center_mm.iter().any(|c| c.unsigned_abs() > MAX_CENTER_MM)
        {
            return Err(BreakError::BadBlast);
        }
        let reach = u64::from(radius_mm);
        let mut struck = 0;
        for chunk in 0..self.chunks.len() {
            let c = self.chunks[chunk];
            let d2: u64 = [c.x, c.y, c.z]
                .into_iter()
                .zip(center_mm)
                .map(|(at, center)| {
                    let d = (mm(at) - i64::from(center)).unsigned_abs();
                    d.saturating_mul(d)
                })
                .fold(0, u64::saturating_add);
            let distance = d2.isqrt();
            if distance >= reach {
                continue;
            }
            let share = u64::from(impulse) * (reach - distance) / reach;
            self.apply_impact(chunk, f32_from_u64(share), tick)?;
            struck += 1;
        }
        Ok(struck)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bonded::{Bond, BondedDesc, BreakEvent, Chunk3};

    /// Three chunks in a row, `x` = 0, 1 and 2 m, 2 m up; chunk 0 is anchored, masses 1, 1, 2.
    fn chain() -> BondedCollection {
        let cube = |x, mass| Chunk3 {
            x,
            y: 2.0,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass,
        };
        BondedCollection::new(&BondedDesc {
            chunks: vec![cube(0.0, 1.0), cube(1.0, 1.0), cube(2.0, 2.0)],
            bonds: vec![
                Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 1,
                    b: 2,
                    breaking_strain: 1.0,
                },
            ],
            anchors: vec![0],
            break_threshold: 3.0,
        })
        .expect("chain")
    }

    /// 200 mm from the tip chunk (x = 2000 mm), 1 m reach: only the tip is inside.
    const AT_TIP: [i32; 3] = [2_200, 2_000, 0];

    #[test]
    fn blast_breaks_the_bond_of_a_chunk_inside_and_no_other() {
        let mut field = chain();
        // 20 units at the center, 80% at 200 mm: 16 against the tip's 2 * 3 = 6 threshold.
        assert_eq!(field.apply_blast(AT_TIP, 1_000, 20, 4), Ok(1));
        let events = field.drain_events();
        assert!(
            matches!(
                events[0],
                BreakEvent::BondBroken {
                    tick: 4,
                    bond: 1,
                    ..
                }
            ),
            "{events:?}"
        );
        assert!(
            events.contains(&BreakEvent::ChunkFreed { tick: 4, chunk: 2 }),
            "{events:?}"
        );
        assert!(field.is_bond_broken(1));
        assert!(!field.is_bond_broken(0), "the far bond held");
        assert!(
            field.is_supported(1),
            "the middle chunk still hangs on the anchor"
        );
    }

    #[test]
    fn blast_falloff_decides_the_break_and_a_weak_blast_only_chips() {
        let mut weak = chain();
        let before = weak.state_hash();
        // 7 units at the center is 5.6 at the tip: under the 6 fatal gate, so
        // no bond snaps, but past the elastic half (3) it chips the bond.
        assert_eq!(weak.apply_blast(AT_TIP, 1_000, 7, 4), Ok(1));
        assert!(weak.drain_events().is_empty(), "no bond may snap");
        assert!(!weak.is_bond_broken(1), "the weak blast only chips");
        let strain = weak.strain_of(1).expect("bond 1 exists");
        assert!(
            strain > 0.0 && strain < 1.0,
            "chipped below the limit: {strain}"
        );
        assert_ne!(weak.state_hash(), before, "the chip is hashed state");
        // At or inside the elastic band the blast changes nothing at all:
        // 3 units is 2.4 at the tip, under the elastic 3.
        let mut elastic = chain();
        let intact = elastic.state_hash();
        assert_eq!(elastic.apply_blast(AT_TIP, 1_000, 3, 4), Ok(1));
        assert!(elastic.drain_events().is_empty());
        assert_eq!(elastic.state_hash(), intact);
        // A wide reach brings the middle chunk in too, at a smaller share.
        let mut wide = chain();
        assert_eq!(wide.apply_blast(AT_TIP, 2_000, 40, 4), Ok(2));
        // Tip: 36 > 6 breaks bond 1. Middle (1200 mm): 16 > 3 breaks bond 0 as well.
        assert!(wide.is_bond_broken(1) && wide.is_bond_broken(0));
    }

    #[test]
    fn blast_outside_the_radius_touches_nothing() {
        let mut field = chain();
        let before = field.state_hash();
        assert_eq!(
            field.apply_blast([30_000, 2_000, 0], 5_000, u32::MAX, 1),
            Ok(0)
        );
        // The edge itself is outside: the tip is exactly 1000 mm away.
        assert_eq!(
            field.apply_blast([3_000, 2_000, 0], 1_000, u32::MAX, 1),
            Ok(0)
        );
        assert_eq!(field.state_hash(), before);
        assert!(field.drain_events().is_empty());
    }

    #[test]
    fn blast_a_recorded_blast_replays_to_the_same_events_and_hash() {
        type Tape = [([i32; 3], u32, u32, u32)];
        let run = |blasts: &Tape| {
            let mut field = chain();
            for (center, radius, impulse, tick) in blasts {
                field
                    .apply_blast(*center, *radius, *impulse, *tick)
                    .expect("blast");
                field.step(1.0 / 60.0);
            }
            for _ in 0..30 {
                field.step(1.0 / 60.0);
            }
            (field.state_hash(), field.drain_events())
        };
        let tape: [([i32; 3], u32, u32, u32); 2] =
            [(AT_TIP, 2_000, 40, 2), ([0, 2_000, 0], 800, 30, 5)];
        let (a, b) = (run(&tape), run(&tape));
        assert_eq!(a, b);
        assert!(!a.1.is_empty());
        assert_ne!(a.0, run(&[]).0, "the blasts changed the field");
    }

    #[test]
    fn blast_refusals_leave_the_field_unchanged() {
        let mut field = chain();
        let before = field.state_hash();
        for (center, radius) in [
            (AT_TIP, 0),
            (AT_TIP, MAX_BLAST_RADIUS_MM + 1),
            ([1_000_001, 0, 0], 1_000),
            ([0, i32::MIN, 0], 1_000),
        ] {
            assert_eq!(
                field.apply_blast(center, radius, 100, 1),
                Err(BreakError::BadBlast),
                "{center:?} {radius}"
            );
        }
        assert_eq!(field.state_hash(), before);
        assert!(field.drain_events().is_empty());
    }
}
