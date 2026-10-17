//! Spine parity column of the proof scorecard (ARC10): 20 seeded 600-tick
//! tapes, each stepped by the legacy arena step and by the spine step in
//! lockstep, reported as one line per build.
//!
//! Consumer: the `spine_parity_line` bin prints [`Column::line`] and
//! `tools/proof.mjs` puts it in the scorecard; a mismatch fails the gate and
//! names the first divergent seed, tick and component. Data owner: this module
//! owns the seeds and tape shape; `engine-arena` owns both steps and the
//! comparison ([`engine_arena::parity`]). Units: fixed ticks. Nothing here
//! touches the simulation hash: it only calls the public arena API.

use engine_arena::parity::parity;
use engine_arena::world::{MatchWorld, Obstacle};
use engine_arena::{seed, ArenaConfig, FighterId, FighterInput, FighterTape};

/// Seeded tapes compared per build.
pub const SEEDS: u32 = 20;
/// Fixed ticks each tape runs.
pub const TICKS: u32 = 600;
/// First tape seed; tape `n` uses `FIRST_SEED + n`.
const FIRST_SEED: u64 = 1000;
/// Arena half-extent in meters.
const HALF_EXTENT_M: f32 = 12.0;
/// Ticks between two scripted inputs of the human slot.
const INPUT_EVERY: u32 = 7;

/// The first tape that parted ways, and how.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Mismatch {
    /// Tape seed.
    pub seed: u64,
    /// First divergent tick.
    pub tick: u32,
    /// What differed: a spine column, the checkpoint hash, or a refused build.
    pub what: String,
}

/// One build's parity column.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Column {
    /// Tapes compared.
    pub seeds: u32,
    /// Tapes that matched on every tick.
    pub matched: u32,
    /// First mismatch in seed order, `None` when all matched.
    pub first: Option<Mismatch>,
}

impl Column {
    /// True when every tape matched.
    pub fn pass(&self) -> bool {
        self.matched == self.seeds
    }

    /// The scorecard line: `spine parity: seeds=20 ticks=600 match=20
    /// mismatch=0 first=none`.
    pub fn line(&self) -> String {
        let first = self.first.as_ref().map_or_else(
            || "none".to_owned(),
            |m| format!("seed {} tick {} {}", m.seed, m.tick, m.what),
        );
        format!(
            "spine parity: seeds={} ticks={TICKS} match={} mismatch={} first={first}",
            self.seeds,
            self.matched,
            self.seeds - self.matched,
        )
    }
}

/// Slot 0 plays a scripted human (a new input every [`INPUT_EVERY`] ticks,
/// wishes and buttons drawn from the seed), every other slot is a brain.
fn tape(seed_value: u64) -> FighterTape {
    let steps = usize::try_from(TICKS / INPUT_EVERY + 1).unwrap_or(0);
    let words = seed::expand(seed_value ^ 0x5eed, steps);
    let inputs = (0..TICKS)
        .step_by(INPUT_EVERY as usize)
        .zip(words)
        .map(|(tick, word)| {
            let wish = |bits: u64| i8::try_from(bits % 201).unwrap_or(0).wrapping_sub(100);
            let facing = i16::try_from((word >> 16) % 36_001).unwrap_or(0) - 18_000;
            let mut input = FighterInput {
                tick,
                fighter: FighterId(0),
                mx: wish(word),
                mz: wish(word >> 8),
                facing,
                buttons: Default::default(),
            };
            input.buttons.0 = u8::try_from((word >> 40) % 8).unwrap_or(0);
            input
        })
        .collect();
    FighterTape(inputs)
}

/// A block across the arena center that only the spine side knows about: the
/// seeded divergence the gate must catch.
pub fn seeded_wall() -> MatchWorld {
    let block = Obstacle::aligned([0.0, 0.0], [3.0, 3.0]);
    MatchWorld::try_new(vec![block]).unwrap_or_else(|_| MatchWorld::empty())
}

/// Compare the legacy and spine steps over the seeded tapes, the spine side
/// playing in `world` (empty for the build's real column).
pub fn column(world: &MatchWorld) -> Column {
    let mut matched = 0;
    let mut first = None;
    for n in 0..SEEDS {
        let seed_value = FIRST_SEED + u64::from(n);
        let fighters = 2 + u8::try_from(n % 3).unwrap_or(0);
        let verdict = ArenaConfig::try_new(seed_value, TICKS, fighters, HALF_EXTENT_M)
            .ok()
            .and_then(|config| parity(&config, &tape(seed_value), world));
        match verdict {
            Some(report) => match report.first_divergence {
                None => matched += 1,
                Some((tick, why)) => {
                    first.get_or_insert(Mismatch {
                        seed: seed_value,
                        tick,
                        what: format!("{why:?}"),
                    });
                }
            },
            None => {
                first.get_or_insert(Mismatch {
                    seed: seed_value,
                    tick: 0,
                    what: "build refused".to_owned(),
                });
            }
        }
    }
    Column {
        seeds: SEEDS,
        matched,
        first,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_matches_on_every_tape() {
        let clean = column(&MatchWorld::empty());
        assert_eq!(
            clean.line(),
            "spine parity: seeds=20 ticks=600 match=20 mismatch=0 first=none"
        );
        assert!(clean.pass());
    }

    #[test]
    fn a_seeded_divergence_fails_the_gate_and_names_where() {
        let seeded = column(&seeded_wall());
        println!("SEEDED {}", seeded.line());
        assert!(!seeded.pass());
        let first = seeded.first.clone().expect("a wall in the way diverges");
        assert!(first.tick < TICKS);
        assert_eq!(first.what, "Column(Transform)");
        assert!(seeded
            .line()
            .contains(&format!("first=seed {} tick {}", first.seed, first.tick)));
        assert_eq!(column(&seeded_wall()), seeded, "same tapes, same column");
    }
}
