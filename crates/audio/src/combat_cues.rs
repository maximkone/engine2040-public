//! combat_cues: fight events to named cue plays.
//!
//! The game maps each [`FightCue`] onto a named cue from the arena bank
//! (real clips in `content/audio`, synth takes when a file is missing),
//! with a gain and a source position. The mix layer plays the returned
//! [`CuePlay`] through a [`CueBank`]; variant choice stays seeded there,
//! so the same event script picks the same takes. The mapping is pure data;
//! the `queue_` helpers put it on a [`FeedTx`] (no lock, no allocation).

use crate::crowd::FightCue;
use crate::cue::names;
use crate::feed::{FeedError, FeedTx};

/// One cue play request: which cue, how loud, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CuePlay {
    /// Cue name in the arena bank (one of [`names`]).
    pub name: &'static str,
    /// Play level, `0.0..=1.0` (times the cue's own trim; sanitized by the mixer).
    pub gain: f32,
    /// Source `[east, up, south]` in centimeters; `None` plays flat.
    pub pos_cm: Option<[i32; 3]>,
}

/// Arena floor under the footstep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Arena sand (gravel recordings).
    Sand,
    /// Stone tunnel.
    Stone,
    /// Wood platform.
    Wood,
    /// Grass.
    Grass,
    /// Metal grate.
    Metal,
    /// Shallow water.
    Water,
}

impl Surface {
    /// Footstep cue for this floor (one of [`names`]).
    pub fn footstep_cue(self) -> &'static str {
        match self {
            Self::Sand => names::FOOTSTEP,
            Self::Stone => names::FOOTSTEP_STONE,
            Self::Wood => names::FOOTSTEP_WOOD,
            Self::Grass => names::FOOTSTEP_GRASS,
            Self::Metal => names::FOOTSTEP_METAL,
            Self::Water => names::FOOTSTEP_WATER,
        }
    }
}

/// Map one fight event onto its cue play. Impacts sound at the victim
/// (the blow lands there); a dodged swing whooshes at the attacker (the
/// weapon missed from there). `Blocked` and `Parried` are distinct cues,
/// not a quieter hit: shield wood and blade metal read differently.
/// `StaminaOut` is silent (no cue in the arena bank). Never panics.
pub fn cue_for_event(
    event: FightCue,
    attacker_pos: [i32; 3],
    victim_pos: [i32; 3],
) -> Option<CuePlay> {
    match event {
        FightCue::Hit { .. } => Some(CuePlay {
            name: names::HIT,
            gain: 1.0,
            pos_cm: Some(victim_pos),
        }),
        FightCue::Blocked { .. } => Some(CuePlay {
            name: names::BLOCK,
            gain: 0.9,
            pos_cm: Some(victim_pos),
        }),
        FightCue::Parried { .. } => Some(CuePlay {
            name: names::PARRY,
            gain: 0.9,
            pos_cm: Some(victim_pos),
        }),
        FightCue::Dodged { .. } => Some(CuePlay {
            name: names::SWING,
            gain: 0.5,
            pos_cm: Some(attacker_pos),
        }),
        FightCue::Knockdown { .. } => Some(CuePlay {
            name: names::KNOCKDOWN,
            gain: 1.0,
            pos_cm: Some(victim_pos),
        }),
        FightCue::StaminaOut { .. } => None,
    }
}

/// One footstep on `surface` at `pos_cm`, at walking level.
pub fn footstep(surface: Surface, pos_cm: [i32; 3]) -> CuePlay {
    CuePlay {
        name: surface.footstep_cue(),
        gain: 0.8,
        pos_cm: Some(pos_cm),
    }
}

/// Queue one fight event on the feed: the cue [`cue_for_event`] names, played
/// at its source. A silent event queues nothing and is not an error. Never
/// blocks and never allocates.
///
/// # Errors
///
/// As [`FeedTx::play`]: [`FeedError::Full`] (counted) or
/// [`FeedError::Closed`].
pub fn queue_fight_event(
    tx: &mut FeedTx,
    tick: u32,
    event: FightCue,
    attacker_pos: [i32; 3],
    victim_pos: [i32; 3],
) -> Result<(), FeedError> {
    match cue_for_event(event, attacker_pos, victim_pos) {
        Some(play) => tx.play(tick, play.name, play.gain, play.pos_cm),
        None => Ok(()),
    }
}

/// Queue one footstep on `surface` at `pos_cm` on the feed.
///
/// # Errors
///
/// As [`queue_fight_event`].
pub fn queue_footstep(
    tx: &mut FeedTx,
    tick: u32,
    surface: Surface,
    pos_cm: [i32; 3],
) -> Result<(), FeedError> {
    let play = footstep(surface, pos_cm);
    tx.play(tick, play.name, play.gain, play.pos_cm)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::feed::{feed, AudioCommand, Command};
    use crate::{AudioEngine, CueBank, FeedStats, FrameSource, MixerStats};

    const ATTACKER: [i32; 3] = [100, 0, 0];
    const VICTIM: [i32; 3] = [300, 0, 100];

    #[test]
    fn every_event_maps_to_its_own_cue() {
        let cases = [
            (
                FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 120,
                },
                names::HIT,
                1.0,
                VICTIM,
            ),
            (
                FightCue::Blocked {
                    attacker: 0,
                    victim: 1,
                    chip: 24,
                },
                names::BLOCK,
                0.9,
                VICTIM,
            ),
            (
                FightCue::Parried {
                    attacker: 0,
                    victim: 1,
                },
                names::PARRY,
                0.9,
                VICTIM,
            ),
            (
                FightCue::Dodged {
                    attacker: 0,
                    victim: 1,
                },
                names::SWING,
                0.5,
                ATTACKER,
            ),
            (
                FightCue::Knockdown {
                    attacker: 0,
                    victim: 1,
                },
                names::KNOCKDOWN,
                1.0,
                VICTIM,
            ),
        ];
        for (event, name, gain, pos) in cases {
            assert_eq!(
                cue_for_event(event, ATTACKER, VICTIM),
                Some(CuePlay {
                    name,
                    gain,
                    pos_cm: Some(pos),
                }),
                "{event:?}"
            );
        }
        assert_eq!(
            cue_for_event(FightCue::StaminaOut { fighter: 1 }, ATTACKER, VICTIM),
            None,
            "no stamina cue in the bank"
        );
    }

    #[test]
    fn mapping_is_pure_data() {
        let event = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        };
        assert_eq!(
            cue_for_event(event, ATTACKER, VICTIM),
            cue_for_event(event, ATTACKER, VICTIM),
            "same inputs, same play"
        );
        let at_victim = cue_for_event(event, VICTIM, VICTIM)
            .expect("hit plays")
            .pos_cm
            .expect("impacts are positional");
        assert_eq!(at_victim, VICTIM);
    }

    #[test]
    fn footsteps_cover_every_surface() {
        let surfaces = [
            (Surface::Sand, names::FOOTSTEP),
            (Surface::Stone, names::FOOTSTEP_STONE),
            (Surface::Wood, names::FOOTSTEP_WOOD),
            (Surface::Grass, names::FOOTSTEP_GRASS),
            (Surface::Metal, names::FOOTSTEP_METAL),
            (Surface::Water, names::FOOTSTEP_WATER),
        ];
        for (surface, name) in surfaces {
            assert_eq!(surface.footstep_cue(), name);
            let play = footstep(surface, VICTIM);
            assert_eq!(
                (play.name, play.gain, play.pos_cm),
                (name, 0.8, Some(VICTIM))
            );
        }
    }

    fn bank() -> Arc<CueBank> {
        Arc::new(CueBank::placeholder(48_000).expect("shipped table is valid"))
    }

    /// The five fight events the arena sends, each with the cue, gain and
    /// source it must map to. The expectations are written out here, not read
    /// back from `cue_for_event`.
    fn five_events() -> [(FightCue, &'static str, f32, [i32; 3]); 5] {
        [
            (
                FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 120,
                },
                names::HIT,
                1.0,
                VICTIM,
            ),
            (
                FightCue::Blocked {
                    attacker: 0,
                    victim: 1,
                    chip: 24,
                },
                names::BLOCK,
                0.9,
                VICTIM,
            ),
            (
                FightCue::Parried {
                    attacker: 0,
                    victim: 1,
                },
                names::PARRY,
                0.9,
                VICTIM,
            ),
            (
                FightCue::Dodged {
                    attacker: 0,
                    victim: 1,
                },
                names::SWING,
                0.5,
                ATTACKER,
            ),
            (
                FightCue::Knockdown {
                    attacker: 0,
                    victim: 1,
                },
                names::KNOCKDOWN,
                1.0,
                VICTIM,
            ),
        ]
    }

    #[test]
    fn fight_events_reach_the_feed_as_their_cue_at_their_source() {
        let bank = bank();
        let (mut tx, mut rx) = feed(16, Arc::clone(&bank));
        for (tick, (event, _, _, _)) in (0u32..).zip(five_events()) {
            queue_fight_event(&mut tx, tick, event, ATTACKER, VICTIM).expect("room");
        }
        for (tick, (_, name, gain, pos)) in (0u32..).zip(five_events()) {
            assert_eq!(
                rx.pop(),
                Some(Command::Play(AudioCommand {
                    tick,
                    cue: bank.id(name).expect("shipped cue"),
                    gain,
                    pos_cm: Some(pos),
                })),
                "{name} at tick {tick}"
            );
        }
        assert!(rx.pop().is_none());
        assert_eq!(tx.stats().sent, 5);
    }

    #[test]
    fn a_silent_event_is_not_an_error_and_queues_nothing() {
        let (mut tx, mut rx) = feed(4, bank());
        queue_fight_event(
            &mut tx,
            3,
            FightCue::StaminaOut { fighter: 1 },
            ATTACKER,
            VICTIM,
        )
        .expect("a silent event is not an error");
        assert!(rx.pop().is_none());
        assert_eq!(tx.stats().sent, 0);
        assert_eq!(tx.stats().unknown_cue, 0);
    }

    #[test]
    fn a_full_feed_refuses_a_fight_event_and_counts_it() {
        let (mut tx, _rx) = feed(2, bank());
        tx.play(0, names::HIT, 1.0, None).expect("room");
        tx.play(0, names::HIT, 1.0, None).expect("room");
        let parry = FightCue::Parried {
            attacker: 0,
            victim: 1,
        };
        assert_eq!(
            queue_fight_event(&mut tx, 1, parry, ATTACKER, VICTIM),
            Err(FeedError::Full)
        );
        assert_eq!(tx.stats().dropped_full, 1, "the refusal is counted");
    }

    #[test]
    fn footsteps_queue_their_surface_cue_at_the_feet() {
        let bank = bank();
        let (mut tx, mut rx) = feed(16, Arc::clone(&bank));
        let surfaces = [
            (Surface::Sand, names::FOOTSTEP),
            (Surface::Stone, names::FOOTSTEP_STONE),
            (Surface::Wood, names::FOOTSTEP_WOOD),
            (Surface::Grass, names::FOOTSTEP_GRASS),
            (Surface::Metal, names::FOOTSTEP_METAL),
            (Surface::Water, names::FOOTSTEP_WATER),
        ];
        for (tick, (surface, _)) in (0u32..).zip(surfaces) {
            queue_footstep(&mut tx, tick, surface, VICTIM).expect("room");
        }
        for (tick, (_, name)) in (0u32..).zip(surfaces) {
            assert_eq!(
                rx.pop(),
                Some(Command::Play(AudioCommand {
                    tick,
                    cue: bank.id(name).expect("shipped cue"),
                    gain: 0.8,
                    pos_cm: Some(VICTIM),
                })),
                "{name}"
            );
        }
        assert!(rx.pop().is_none());
    }

    /// One arena-like script through a fresh engine on `seed`: a listener
    /// move, the five fight events, a silent stamina event and one footstep.
    /// Returns the feed counters, the mixer counters, the culled count and
    /// the rendered stereo buffer.
    fn run_script(seed: u64) -> (FeedStats, MixerStats, u64, Vec<f32>) {
        let (mut tx, mut engine) = AudioEngine::with_feed(bank(), 64, 48_000, seed);
        tx.listener(0, ATTACKER, 0).expect("room");
        for (tick, (event, _, _, _)) in (1u32..).zip(five_events()) {
            queue_fight_event(&mut tx, tick, event, ATTACKER, VICTIM).expect("room");
        }
        queue_fight_event(
            &mut tx,
            6,
            FightCue::StaminaOut { fighter: 1 },
            ATTACKER,
            VICTIM,
        )
        .expect("silent");
        queue_footstep(&mut tx, 7, Surface::Sand, VICTIM).expect("room");
        let mut out = vec![0.0f32; 2 * 4_800];
        engine.render_stereo_into(&mut out);
        (
            tx.stats(),
            engine.mixer().mixer_stats(),
            engine.mixer().culled(),
            out,
        )
    }

    #[test]
    fn fight_script_through_the_engine_applies_everything_and_replays_per_seed() {
        let (feed_stats, mix, culled, first) = run_script(2040);
        assert_eq!(
            (feed_stats.sent, feed_stats.applied),
            (7, 7),
            "listener, five events, one footstep"
        );
        assert_eq!(feed_stats.last_applied_tick, 7);
        assert_eq!((feed_stats.dropped_full, feed_stats.unknown_cue), (0, 0));
        assert_eq!(
            (mix.started, mix.rejected, culled),
            (6, 0, 0),
            "six plays sound: none refused, none out of range"
        );
        assert!(first.iter().any(|&s| s != 0.0), "the fight is audible");
        let (_, _, _, again) = run_script(2040);
        assert_eq!(first, again, "same seed and script replay the same bytes");
    }
}
