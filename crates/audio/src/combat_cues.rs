//! combat_cues: fight events to named cue plays.
//!
//! The game maps each [`FightCue`] onto a named cue from the arena bank
//! (real clips in `content/audio`, synth takes when a file is missing),
//! with a gain and a source position. The mix layer plays the returned
//! [`CuePlay`] through a [`CueBank`]; variant choice stays seeded there,
//! so the same event script picks the same takes. Pure data mapping: no
//! mixer, no clock, no allocation.

use crate::crowd::FightCue;
use crate::cue::names;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
