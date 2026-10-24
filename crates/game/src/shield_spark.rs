//! shield_spark: a held block throws a short ring of bright sparks off the
//! shield rim that faces the attacker (SW172).
//!
//! Consumer: the arena's fight fx (`bin/arena_frame/fight_fx.rs`), which hands
//! the burst to the presenter's existing particle pool in the same tick the
//! block cue arrives. Presentation only: nothing here reads back into the sim
//! or the hash.
//!
//! A landed block is a metal ring, not a chest wound: the sparks leave the
//! shield's rim, the point on the victim's guard closest to the attacker, and
//! fly back along the blow. Units: meters, ticks at 60 Hz, counts.
//! Determinism: float geometry from the two feet positions only, no clock, no
//! RNG; the same cue tape throws the same sparks twice.

use engine_audio::FightCue;

use crate::FighterView;

/// Fewest sparks one block throws.
pub const SPARK_MIN: u32 = 6;
/// Most sparks one block throws, however hard the blow.
pub const SPARK_MAX: u32 = 10;
/// Chip damage that adds one spark on top of [`SPARK_MIN`].
pub const CHIP_PER_SPARK: u16 = 120;
/// Height of the shield rim above the victim's feet, meters.
pub const RIM_H_M: f32 = 1.15;
/// Distance from the victim's centre to the rim toward the attacker, meters.
pub const RIM_REACH_M: f32 = 0.44;
/// How far the spark cone tips up: the emitter normalizes the direction.
pub const RIM_LIFT: f32 = 0.85;

/// One block's spark ring: where it leaves the shield, which way it flies and
/// how many sparks it holds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RimSpark {
    /// The tick the block was resolved on: the burst lands in that same tick.
    pub tick: u32,
    /// Rim point of the shield facing the attacker, world meters.
    pub at_m: [f32; 3],
    /// Unit ground direction from the victim to the attacker, tipped up.
    pub dir: [f32; 3],
    /// Sparks: [`SPARK_MIN`]..=[`SPARK_MAX`].
    pub count: u32,
}

/// Sparks one block throws: a short bright ring, one more per
/// [`CHIP_PER_SPARK`] of chip damage, never over [`SPARK_MAX`].
pub fn spark_count(chip: u16) -> u32 {
    (SPARK_MIN + u32::from(chip) / u32::from(CHIP_PER_SPARK)).min(SPARK_MAX)
}

/// The rim spark of one block: the rim faces the attacker, the sparks fly back
/// along the blow from the same tick.
pub fn rim_spark_at(attacker_m: [f32; 3], victim_m: [f32; 3], tick: u32, chip: u16) -> RimSpark {
    let (dx, dz) = (attacker_m[0] - victim_m[0], attacker_m[2] - victim_m[2]);
    let len = (dx * dx + dz * dz).sqrt();
    let toward = if len.is_finite() && len > 1e-3 {
        [dx / len, 0.0, dz / len]
    } else {
        [0.0, 0.0, 1.0]
    };
    RimSpark {
        tick,
        at_m: [
            victim_m[0] + toward[0] * RIM_REACH_M,
            victim_m[1] + RIM_H_M,
            victim_m[2] + toward[2] * RIM_REACH_M,
        ],
        dir: [toward[0], RIM_LIFT, toward[2]],
        count: spark_count(chip),
    }
}

/// The rim spark one fight cue asks for: only a block throws one, so a clean
/// hit, a parry, a dodge or a stamina miss leaves the rim cold. A block whose
/// fighters are not in `views` (a slot that left) throws nothing.
pub fn rim_spark_for(cue: &FightCue, tick: u32, views: &[FighterView]) -> Option<RimSpark> {
    let FightCue::Blocked {
        attacker,
        victim,
        chip,
    } = *cue
    else {
        return None;
    };
    let from = views.iter().find(|view| view.slot == attacker)?.pos_m;
    let to = views.iter().find(|view| view.slot == victim)?.pos_m;
    Some(rim_spark_at(from, to, tick, chip))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn views() -> [FighterView; 2] {
        [
            FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [2.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ]
    }

    #[test]
    fn block_spark_a_blocked_strike_sparks_at_the_rim_within_one_tick() {
        let cue = FightCue::Blocked {
            attacker: 0,
            victim: 1,
            chip: 10,
        };
        let spark = rim_spark_for(&cue, 42, &views()).expect("a block throws sparks");
        assert_eq!(
            spark.tick, 42,
            "the burst lands on the block's own tick, not the next"
        );
        assert!(
            (SPARK_MIN..=SPARK_MAX).contains(&spark.count),
            "6..=10 sparks, got {}",
            spark.count
        );
        assert_eq!(
            spark.at_m,
            [2.0 - RIM_REACH_M, RIM_H_M, 0.0],
            "the rim point on the victim's side of the shield faces the attacker"
        );
        assert!(
            spark.dir[0] < 0.0,
            "the sparks fly back at the attacker: {:?}",
            spark.dir
        );
        assert!(spark.dir[1] > 0.0, "the ring lifts as it leaves the rim");
        // A blow on the other side mirrors the rim point.
        let mirrored = rim_spark_at([0.0, 0.0, 0.0], [0.0, 0.0, 2.0], 7, 0);
        assert_eq!(mirrored.at_m, [0.0, RIM_H_M, 2.0 - RIM_REACH_M]);
        assert!(mirrored.dir[2] < 0.0);
        // Much chip damage keeps the ring short and bright.
        assert_eq!(spark_count(0), SPARK_MIN);
        assert_eq!(spark_count(u16::MAX), SPARK_MAX);
        assert!(spark_count(240) > spark_count(0));
    }

    #[test]
    fn block_spark_a_clean_hit_sparks_none_from_the_rim() {
        for cue in [
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 300,
            },
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
            FightCue::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightCue::StaminaOut { fighter: 1 },
        ] {
            assert!(
                rim_spark_for(&cue, 9, &views()).is_none(),
                "{cue:?} must not throw rim sparks"
            );
        }
        // A block with a fighter that already left the ring throws nothing.
        let gone = [views()[0]];
        assert!(rim_spark_for(
            &FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 0,
            },
            9,
            &gone
        )
        .is_none());
    }

    #[test]
    fn block_spark_same_tape_twice_is_equal() {
        // A short tape: three blocks, a hit between them, one dodge.
        let tape = [
            (
                0u32,
                FightCue::Blocked {
                    attacker: 0,
                    victim: 1,
                    chip: 0,
                },
            ),
            (
                1,
                FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 90,
                },
            ),
            (
                2,
                FightCue::Blocked {
                    attacker: 0,
                    victim: 1,
                    chip: 500,
                },
            ),
            (
                3,
                FightCue::Dodged {
                    attacker: 1,
                    victim: 0,
                },
            ),
            (
                4,
                FightCue::Blocked {
                    attacker: 1,
                    victim: 0,
                    chip: 60,
                },
            ),
        ];
        let run = || {
            tape.iter()
                .filter_map(|(tick, cue)| rim_spark_for(cue, *tick, &views()))
                .map(|spark| (spark.tick, spark.count, spark.at_m, spark.dir))
                .collect::<Vec<_>>()
        };
        let first = run();
        assert_eq!(first.len(), 3, "three blocks, three rings");
        assert_eq!(first, run(), "the same tape throws the same sparks");
        assert_eq!(first[0].0, 0, "the first block is stamped with its tick");
        assert!(
            first[1].1 > first[0].1,
            "a chipped block throws more sparks"
        );
    }
}
