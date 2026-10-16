//! parity: legacy step against spine step, tick by tick (flag `spine`).
//!
//! Owner: engine-arena. The harness steps the frozen legacy [`Sim`] and the
//! [`SpineMatch`] in lockstep on one config and tape, compares the spine
//! columns against the legacy state after every tick, then the checkpoint
//! hashes, and reports either an exact match or the first divergent tick
//! with the component that differs. The legacy path stays the golden
//! authority; nothing here writes to it.
//!
//! Scope: the kinematic default track (the `sim` feature adds physics
//! controllers the spine step does not own). Units: ticks, meters.

use engine_ecs::spine::Column;

use crate::spine_step::{raw_fixed, SpineMatch};
use crate::world::MatchWorld;
use crate::{canon_zero, ArenaConfig, FighterTape, Sim};

/// What differed first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Divergence {
    /// A spine column differs from the legacy state it mirrors.
    Column(Column),
    /// Every column matches but the checkpoint hash does not (events,
    /// destruction or the world digest).
    Checkpoint,
}

/// Verdict of one parity run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParityReport {
    /// Ticks compared before the run ended or diverged.
    pub ticks: u32,
    /// First divergent tick and what differed, `None` for an exact match.
    pub first_divergence: Option<(u32, Divergence)>,
}

impl ParityReport {
    /// True when every tick matched.
    pub fn exact(&self) -> bool {
        self.first_divergence.is_none()
    }
}

/// Step both paths over `config` and `tape` (spine path in `world`) and
/// report. `None` when either path refuses to build (invalid input).
pub fn parity(
    config: &ArenaConfig,
    tape: &FighterTape,
    world: &MatchWorld,
) -> Option<ParityReport> {
    let mut legacy = Sim::build(config, tape, false, false)?;
    let mut spine = SpineMatch::build(config, tape, world.clone())?;
    let n = usize::from(config.fighters);
    let mut before = legacy.states.clone();
    for tick in 0..config.ticks {
        legacy.step(tape)?;
        let spun = spine.step()?;
        if let Some(column) = first_column_difference(&legacy, &spine, &before, n) {
            return Some(ParityReport {
                ticks: tick + 1,
                first_divergence: Some((tick, Divergence::Column(column))),
            });
        }
        let mine = legacy.checkpoints.last().copied()?;
        if mine.hash != spine.base_hash() || mine.tick != spun.tick {
            return Some(ParityReport {
                ticks: tick + 1,
                first_divergence: Some((tick, Divergence::Checkpoint)),
            });
        }
        before.clone_from(&legacy.states);
    }
    Some(ParityReport {
        ticks: config.ticks,
        first_divergence: None,
    })
}

/// First spine column (canonical order) that differs from the legacy state.
#[expect(
    clippy::needless_range_loop,
    reason = "slot indexes the legacy arrays and five spine columns in step"
)]
fn first_column_difference(
    legacy: &Sim,
    spine: &SpineMatch,
    before: &[crate::FighterState],
    n: usize,
) -> Option<Column> {
    let s = spine.spine();
    for column in Column::ORDER {
        for slot in 0..n {
            let differs = match column {
                Column::Transform => {
                    let (a, b) = (&legacy.states[slot], &s.transforms()[slot]);
                    a.pos
                        .iter()
                        .zip(b.pos.iter())
                        .any(|(x, y)| bits(*x) != bits(*y))
                        || bits(a.yaw) != bits(b.yaw)
                }
                Column::Body => {
                    let body = &s.bodies()[slot];
                    let (now, was) = (&legacy.states[slot], &before[slot]);
                    // A held (knocked-out) fighter has zero delta, which the
                    // same expression yields.
                    body.vel
                        != [
                            raw_fixed(now.pos[0] - was.pos[0]),
                            raw_fixed(now.pos[2] - was.pos[2]),
                        ]
                }
                Column::Vitals => {
                    let (f, v) = (&legacy.fight.fighters[slot], &s.vitals()[slot]);
                    f.hp_permille != v.hp_permille || f.stamina != v.stamina
                }
                Column::Ai => {
                    let Some(input) = legacy.last_inputs()[slot] else {
                        return Some(column);
                    };
                    let ai = &s.ai()[slot];
                    input.mx != ai.intent.mx
                        || input.mz != ai.intent.mz
                        || input.facing != ai.intent.facing
                        || input.buttons.0 != ai.intent.buttons
                }
                Column::Anim => {
                    let (l, a) = (&legacy.locos[slot], &s.anims()[slot]);
                    let live = legacy.fight.fighters[slot].hp_permille != 0;
                    // Knocked-out fighters hold their last gait; the column
                    // keeps what it held, which the legacy machine also keeps.
                    live && (bits(l.phase()) != bits(a.phase)
                        || bits(l.facing()[0]) != bits(a.facing[0])
                        || bits(l.facing()[1]) != bits(a.facing[1])
                        || l.is_airborne() != a.airborne
                        || l.interrupt_active() != a.interrupt)
                }
            };
            if differs {
                return Some(column);
            }
        }
    }
    None
}

/// Canonical `f32` bits (signed zero folded), the rule every hash uses.
fn bits(value: f32) -> u32 {
    canon_zero(value).to_bits()
}

#[cfg(all(test, not(feature = "sim")))]
mod tests {
    use super::*;
    use crate::seed;
    use crate::world::Obstacle;
    use crate::{FighterId, FighterInput};
    use engine_input::fighter::Buttons;

    /// Seeded tape: slot 0 plays a scripted human, every other slot is a
    /// brain. A new input every 7 ticks with wishes and buttons drawn from
    /// the seed stream.
    fn seeded_tape(seed_value: u64, ticks: u32) -> FighterTape {
        let inputs_n = usize::try_from(ticks / 7 + 1).unwrap_or(0);
        let words = seed::expand(seed_value ^ 0x5eed, 2 + inputs_n * 3);
        let mut inputs = Vec::new();
        let mut at = 0usize;
        let mut tick = 0u32;
        while tick < ticks {
            let w = words[2 + at];
            let mx = i16::try_from(w % 201).unwrap_or(0) - 100;
            let mz = i16::try_from((w >> 8) % 201).unwrap_or(0) - 100;
            let facing = i32::try_from((w >> 16) % 36_001).unwrap_or(0) - 18_000;
            let buttons = u8::try_from((w >> 40) % 8).unwrap_or(0);
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(0),
                mx: i8::try_from(mx).unwrap_or(0),
                mz: i8::try_from(mz).unwrap_or(0),
                facing: i16::try_from(facing).unwrap_or(0),
                buttons: Buttons(buttons),
            });
            at += 3;
            tick += 7;
        }
        FighterTape(inputs)
    }

    fn config(seed_value: u64, fighters: u8) -> ArenaConfig {
        ArenaConfig::try_new(seed_value, 600, fighters, 12.0).expect("valid config")
    }

    #[test]
    fn twenty_seeded_tapes_match_exactly_over_600_ticks() {
        for s in 0..20u64 {
            let cfg = config(1000 + s, 2 + u8::try_from(s % 3).unwrap_or(0));
            let tape = seeded_tape(s, 600);
            let report = parity(&cfg, &tape, &MatchWorld::empty()).expect("both paths build");
            assert_eq!(
                report,
                ParityReport {
                    ticks: 600,
                    first_divergence: None
                },
                "seed {s}"
            );
        }
    }

    /// Balance-run-shaped workload: 2 fighters, 600 ticks, best of 15 batches of 40 whole runs each
    /// (wall clock lives in this test only, never in the sim). Run with
    /// `--ignored --nocapture`: loaded machines make timing flaky.
    #[test]
    #[ignore = "timing: run alone on a quiet machine"]
    fn spine_cost_within_ten_percent_of_legacy() {
        use std::time::Instant;
        let cfg = config(2040, 2);
        let tape = seeded_tape(11, 600);
        let world = MatchWorld::empty();
        let best = |f: &dyn Fn()| {
            (0..15)
                .map(|_| {
                    let t = Instant::now();
                    for _ in 0..40 {
                        f();
                    }
                    t.elapsed()
                })
                .min()
                .expect("fifteen batches")
        };
        let legacy = best(&|| {
            std::hint::black_box(crate::run(&cfg, &tape));
        });
        let spine = best(&|| {
            std::hint::black_box(crate::spine_step::run_spine(&cfg, &tape, &world));
        });
        let ratio = spine.as_secs_f64() / legacy.as_secs_f64();
        eprintln!("legacy {legacy:?} spine {spine:?} ratio {ratio:.3}");
        assert!(ratio <= 1.10, "spine step costs {ratio:.3}x legacy");
    }

    #[test]
    fn a_wall_in_the_way_reports_the_exact_first_divergence() {
        // A big block at the center: fighters spawn across the arena and
        // walk at each other, so the spine path diverges from the empty
        // arena the legacy step plays.
        let block = Obstacle::aligned([0.0, 0.0], [3.0, 3.0]);
        let world = MatchWorld::try_new(vec![block]).expect("valid");
        let mut diverged = 0;
        for s in 0..20u64 {
            let cfg = config(1000 + s, 2);
            let tape = seeded_tape(s, 600);
            let report = parity(&cfg, &tape, &world).expect("both paths build");
            // Same call twice names the same tick and component.
            assert_eq!(parity(&cfg, &tape, &world), Some(report));
            if let Some((tick, why)) = report.first_divergence {
                assert!(tick < 600);
                assert_eq!(report.ticks, tick + 1);
                assert_eq!(why, Divergence::Column(Column::Transform));
                diverged += 1;
            } else {
                assert_eq!(report.ticks, 600);
            }
        }
        // The digest is folded out of the compare, so a run diverges only
        // when a fighter actually met the block.
        assert!(diverged >= 1, "no seed met the block");
    }
}
