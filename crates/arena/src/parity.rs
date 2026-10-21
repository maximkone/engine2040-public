//! parity: legacy step against spine step, tick by tick (flag `spine`).
//!
//! Owner: engine-arena. The harness steps the frozen legacy [`Sim`] and the
//! [`SpineMatch`] in lockstep on one config and tape, compares the spine
//! columns against the legacy state after every tick, then the v2 tracks
//! (cuts, balance, spear, height, the match observer) and finally the
//! checkpoint hashes, and reports either an exact match or the first
//! divergent tick with the component that differs. The legacy path stays
//! the golden authority; nothing here writes to it.
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
    /// The combat events of the tick differ.
    Event,
    /// The severed joints of the tick differ.
    Cut,
    /// A slot's integer balance or stagger memory differs.
    Balance,
    /// The spear grapple state or its events differ.
    Spear,
    /// The height channel differs.
    Height,
    /// The match observer row of the tick differs.
    Match,
    /// Every column and track matches but the checkpoint hash does not
    /// (destruction or the world digest).
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

/// Extra tracks both paths arm for one run. The default arms nothing, so
/// [`parity`] keeps its frozen meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    /// Arm the spear grapple on both paths.
    pub spear: bool,
    /// Arm the height channel on both paths.
    pub height: bool,
    /// Arm the match observer track at this wire version on both paths.
    pub match_version: Option<u32>,
}

/// Step both paths over `config` and `tape` (spine path in `world`) and
/// report. `None` when either path refuses to build (invalid input).
pub fn parity(
    config: &ArenaConfig,
    tape: &FighterTape,
    world: &MatchWorld,
) -> Option<ParityReport> {
    parity_with(config, tape, world, Extras::default())
}

/// [`parity`] with the extra tracks both paths arm: the spear grapple, the
/// height channel and the match observer track. `None` when either path
/// refuses to build (invalid input, an unknown wire version or a track
/// the path does not own).
pub fn parity_with(
    config: &ArenaConfig,
    tape: &FighterTape,
    world: &MatchWorld,
    extras: Extras,
) -> Option<ParityReport> {
    let mut legacy = Sim::build(config, tape, false, extras.match_version.is_some())?;
    if extras.spear && !legacy.set_spear() {
        return None;
    }
    if extras.height && !legacy.set_height() {
        return None;
    }
    if let Some(version) = extras.match_version {
        if !legacy.set_match_version(version) {
            return None;
        }
    }
    let mut spine = SpineMatch::build(config, tape, world.clone())?;
    if extras.spear {
        spine = spine.with_spear()?;
    }
    if extras.height {
        spine = spine.with_height()?;
    }
    if let Some(version) = extras.match_version {
        spine = spine.with_match(version)?;
    }
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
        if let Some(why) = first_track_difference(&legacy, &spine, n) {
            return Some(ParityReport {
                ticks: tick + 1,
                first_divergence: Some((tick, why)),
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

/// First v2 track (events, cuts, balance, spear, height, the match
/// observer) that differs between the two paths. Checked before the
/// checkpoint, so the named divergence is the track that moved.
fn first_track_difference(legacy: &Sim, spine: &SpineMatch, n: usize) -> Option<Divergence> {
    let legacy_events = legacy
        .events
        .last()
        .map_or(&[][..], |row| row.events.as_slice());
    if legacy_events != spine.events() {
        return Some(Divergence::Event);
    }
    let legacy_cuts = legacy
        .cuts
        .last()
        .map_or(&[][..], |row| row.cuts.as_slice());
    if legacy_cuts != spine.cuts() {
        return Some(Divergence::Cut);
    }
    for slot in 0..n {
        let legacy_balance = legacy
            .balance
            .get(slot)
            .map_or(0, |state| state.state_hash());
        let spine_balance = spine
            .balance()
            .get(slot)
            .map_or(0, |state| state.state_hash());
        if legacy_balance != spine_balance {
            return Some(Divergence::Balance);
        }
        let legacy_stagger = legacy
            .stagger
            .get(slot)
            .map_or(0, |state| state.state_hash());
        let spine_stagger = spine
            .stagger()
            .get(slot)
            .map_or(0, |state| state.state_hash());
        if legacy_stagger != spine_stagger {
            return Some(Divergence::Balance);
        }
    }
    let legacy_spear = legacy.spear.as_ref().map(|state| state.fold(0));
    let spine_spear = spine.spear_state().map(|state| state.fold(0));
    if legacy_spear != spine_spear {
        return Some(Divergence::Spear);
    }
    let legacy_spear_events = legacy
        .spear
        .as_ref()
        .map_or(&[][..], |state| state.events());
    if legacy_spear_events != spine.spear_events() {
        return Some(Divergence::Spear);
    }
    if legacy.rise.as_slice() != spine.rise() {
        return Some(Divergence::Height);
    }
    if legacy.match_log.last() != spine.match_row() {
        return Some(Divergence::Match);
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
    use engine_combat::FightEvent;
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

    /// The same config under the v2 move tables, so the rules-v2 track
    /// (cuts, limbs, balance, stagger) runs.
    fn config_v2(seed_value: u64, fighters: u8) -> ArenaConfig {
        ArenaConfig {
            table_version: 2,
            ..config(seed_value, fighters)
        }
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
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "wall clock lives in this test only, never in the sim"
    )]
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

    /// The v2 track: rules v2, the cut resolver, the balance and the limb
    /// pool run on both paths and must agree tick for tick.
    #[test]
    fn spine_v2_matches_the_legacy_fight_over_twenty_seeds() {
        for s in 0..20u64 {
            let cfg = config_v2(1000 + s, 2 + u8::try_from(s % 3).unwrap_or(0));
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

    /// The v2 track must reach both paths: blows land, joints are severed
    /// and the balance moves. A track that never fires would pin nothing.
    #[test]
    fn spine_v2_cuts_and_balance_reach_both_paths() {
        let mut hits = 0usize;
        let mut cuts = 0usize;
        let mut moved = 0usize;
        for s in 0..8u64 {
            let cfg = config_v2(2000 + s, 2);
            let tape = seeded_tape(s, 600);
            let mut legacy = Sim::build(&cfg, &tape, false, false).expect("legacy builds");
            for _ in 0..600 {
                legacy.step(&tape).expect("legacy steps");
            }
            hits += legacy
                .events
                .iter()
                .map(|row| {
                    row.events
                        .iter()
                        .filter(|event| matches!(event, FightEvent::Hit { .. }))
                        .count()
                })
                .sum::<usize>();
            cuts += legacy.cuts.iter().map(|row| row.cuts.len()).sum::<usize>();
            moved += legacy
                .balance
                .iter()
                .filter(|state| state.state_hash() != 0)
                .count();
        }
        assert!(hits > 0, "the v2 track must land blows");
        assert!(cuts > 0, "the v2 track must sever joints");
        assert!(moved > 0, "the v2 track must move the balance");
    }

    /// The spear grapple and the height channel run on both paths and must
    /// agree tick for tick.
    #[test]
    fn spine_spear_and_height_match_the_legacy_step() {
        let extras = Extras {
            spear: true,
            height: true,
            match_version: None,
        };
        for s in 0..6u64 {
            let cfg = config(3000 + s, 2);
            let tape = spear_tape(s, 600);
            let report =
                parity_with(&cfg, &tape, &MatchWorld::empty(), extras).expect("both paths build");
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

    /// The spear and the height channel must really run: the grapple
    /// throws and the jump leaves the floor.
    #[test]
    fn spine_spear_and_height_reach_both_paths() {
        let cfg = config(3000, 2);
        let tape = spear_tape(0, 600);
        let mut legacy = Sim::build(&cfg, &tape, false, false).expect("legacy builds");
        legacy.set_spear();
        legacy.set_height();
        let mut throws = 0usize;
        let mut airborne = 0usize;
        for _ in 0..600 {
            legacy.step(&tape).expect("legacy steps");
            if let Some(spear) = legacy.spear.as_ref() {
                throws += spear.events().len();
            }
            airborne += usize::from(legacy.rise.iter().any(|lane| *lane != 0));
        }
        assert!(throws > 0, "the spear must throw");
        assert!(airborne > 0, "the height channel must leave the floor");
    }

    /// The match observer track runs on both paths and must agree tick for
    /// tick.
    #[test]
    fn spine_match_track_matches_the_legacy_observer() {
        let extras = Extras {
            spear: false,
            height: false,
            match_version: Some(crate::MATCH_WIRE_VERSION_V1),
        };
        for s in 0..6u64 {
            let cfg = config(4000 + s, 2);
            let tape = seeded_tape(s, 600);
            let report =
                parity_with(&cfg, &tape, &MatchWorld::empty(), extras).expect("both paths build");
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

    /// The observer track must really run: the rows carry the exchanges of
    /// the match.
    #[test]
    fn spine_match_rows_reach_both_paths() {
        let cfg = config(4000, 2);
        let tape = seeded_tape(0, 600);
        let mut legacy = Sim::build(&cfg, &tape, false, true).expect("legacy builds");
        legacy.set_match_version(crate::MATCH_WIRE_VERSION_V1);
        for _ in 0..600 {
            legacy.step(&tape).expect("legacy steps");
        }
        let rows = legacy.match_log.len();
        let events: usize = legacy
            .match_log
            .iter()
            .map(|row| row.exchange.len() + row.sweep.len())
            .sum();
        assert_eq!(rows, 600, "one row per tick");
        assert!(events > 0, "the observer track must see the fight");
    }

    /// A tape that presses the spear and the jump buttons, so the grapple
    /// and the height channel have something to do.
    fn spear_tape(seed_value: u64, ticks: u32) -> FighterTape {
        const THROW_SPEAR: u8 = 0x10;
        const JUMP: u8 = 0x20;
        let words = seed::expand(seed_value ^ 0xbeef, 64);
        let mut inputs = Vec::new();
        let mut tick = 0u32;
        let mut at = 0usize;
        while tick < ticks {
            let w = words[at % 64];
            at += 1;
            let mut buttons = 0u8;
            if tick.is_multiple_of(30) {
                buttons |= THROW_SPEAR;
            }
            if tick.is_multiple_of(45) {
                buttons |= JUMP;
            }
            if tick % 45 == 4 {
                buttons |= JUMP;
            }
            let mx = i16::try_from(w % 101).unwrap_or(0) - 50;
            let facing = i32::try_from((w >> 16) % 36_001).unwrap_or(0) - 18_000;
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(0),
                mx: i8::try_from(mx).unwrap_or(0),
                mz: 0,
                facing: i16::try_from(facing).unwrap_or(0),
                buttons: Buttons(buttons),
            });
            tick += 1;
        }
        FighterTape(inputs)
    }
}
