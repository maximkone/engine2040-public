//! Worn armour in the live duel (FEEL4): each slot wears the kit's pieces,
//! and a blow on plate deflects instead of cutting.
//!
//! [`Sim::wear_armour`] sets [`FighterState::worn`] per slot before the first
//! tick, under rules v2 only. Resolving a landed blow (`crate::authority`)
//! hands the victim's worn bits to the cut resolver: a blade whose edge
//! reaches a plated joint (the lorica segmentata) is stopped there, and the
//! match records one [`DeflectCue`] for it (presentation only, never hashed);
//! every other blow resolves as before, with the worn bits as the joint's
//! guard. Unworn matches never read a worn bit, and `fold` writes no bytes
//! for them, so rules v1 and unarmoured v2 bouts keep every byte.

use crate::{authority, geometry_rules, FighterState, Sim};
use engine_combat::{
    Cut, FightEvent, ARMOR_GALEA, ARMOR_GREAVE, ARMOR_LORICA, ARMOR_MANICA, ARMOR_SCUTUM,
};

/// Checkpoint tag byte for the worn armour bits.
const ARMOUR_TAG: u8 = 0xB7;

/// Why [`Sim::wear_armour`] refused: nothing was worn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArmourError {
    /// Armour wears only under the rules v2 rows, before the first tick.
    Closed,
    /// One kit list per fighter, no more and no fewer.
    Count {
        /// Kit lists given.
        got: usize,
        /// Fighters in the match.
        fighters: usize,
    },
    /// A piece the kit does not name (see [`piece_bit`]).
    Piece {
        /// Slot whose kit names the unknown piece.
        slot: usize,
    },
}

impl core::fmt::Display for ArmourError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Closed => f.write_str("armour wears only under rules v2, before the first tick"),
            Self::Count { got, fighters } => {
                write!(
                    f,
                    "{got} kit lists for {fighters} fighters: give one per fighter"
                )
            }
            Self::Piece { slot } => write!(f, "slot {slot} wears a piece the kit does not name"),
        }
    }
}

impl std::error::Error for ArmourError {}

/// One blow a plate stopped (FEEL4): the spark cue the game shows at the
/// contact point. Presentation only, never hashed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeflectCue {
    /// Attacking slot.
    pub attacker: u8,
    /// Victim slot wearing the plate.
    pub victim: u8,
    /// The plated joint the edge reached first (`JointId::as_u8`).
    pub joint: u8,
}

/// The `ARMOR_*` bit a kit piece wears, or `None` for a name the kit does
/// not carry. `helmet` is the galea (neck), `lorica` the plate.
#[must_use]
pub fn piece_bit(piece: &str) -> Option<u8> {
    match piece {
        "lorica" => Some(ARMOR_LORICA),
        "helmet" => Some(ARMOR_GALEA),
        "manica" => Some(ARMOR_MANICA),
        "scutum" => Some(ARMOR_SCUTUM),
        "greave" => Some(ARMOR_GREAVE),
        _ => None,
    }
}

impl Sim {
    /// Wear one kit per fighter, in slot order, by piece name:
    /// `[&["lorica", "helmet"], &[]]` puts a lorica and a helmet on slot 0
    /// and leaves slot 1 bare.
    ///
    /// # Errors
    ///
    /// [`ArmourError`] when the match runs the rules v1 rows or is past tick
    /// 0, the kit count is not the fighter count, or a piece is unknown;
    /// nothing is worn then.
    pub fn wear_armour(&mut self, kit: &[&[&str]]) -> Result<(), ArmourError> {
        if self.tick != 0 || !geometry_rules(self.rules.version) {
            return Err(ArmourError::Closed);
        }
        if kit.len() != self.fighters {
            return Err(ArmourError::Count {
                got: kit.len(),
                fighters: self.fighters,
            });
        }
        let mut bits = Vec::with_capacity(kit.len());
        for (slot, pieces) in kit.iter().enumerate() {
            let mut bit = 0u8;
            for piece in pieces.iter() {
                bit |= piece_bit(piece).ok_or(ArmourError::Piece { slot })?;
            }
            bits.push(bit);
        }
        for (state, bit) in self.states.iter_mut().zip(bits) {
            state.worn = bit;
        }
        Ok(())
    }

    /// The `ARMOR_*` bits a slot wears; `0` when unarmoured or out of the match.
    #[must_use]
    pub fn worn(&self, slot: u8) -> u8 {
        self.states
            .get(usize::from(slot))
            .map_or(0, |state| state.worn)
    }

    /// The deflect cues of the last stepped tick, one per blow a plate
    /// stopped, in the order the blows resolved. Empty under v1 and on
    /// quiet ticks.
    #[must_use]
    pub fn deflects(&self) -> &[DeflectCue] {
        &self.last_deflects
    }
}

impl Sim {
    /// Resolve one tick's landed hits into severed joints, and record the
    /// tick's deflect cues in place of the last tick's (FEEL4). V2 only; the
    /// caller gates on the rules.
    pub(crate) fn resolve_cuts(&mut self, events: &[FightEvent]) -> Vec<Cut> {
        self.last_deflects.clear();
        authority::resolve_cuts(
            &self.states,
            &self.fight,
            self.tick,
            &mut self.limbs,
            events,
            &mut self.last_deflects,
        )
    }
}

/// Fold the worn bits into a checkpoint hash: the tag byte, then every
/// active slot's bits. A match where nobody wears armour returns the hash
/// untouched (golden-frozen bytes).
pub(crate) fn fold(hash: u64, states: &[FighterState]) -> u64 {
    if states.iter().all(|state| state.worn == 0) {
        return hash;
    }
    let mut digest = engine_core::hash::Fnv64::with_state(hash);
    digest.write_u8(ARMOUR_TAG);
    for state in states {
        digest.write_u8(state.worn);
    }
    digest.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::MatchWorld;
    use crate::{ArenaConfig, FighterInput, FighterTape};
    use engine_combat::fight::FightEvent;
    use engine_combat::tables::{TABLES_VERSION_V1, TABLES_VERSION_V2};
    use engine_combat::JointId;
    use engine_input::fighter::ATTACK;
    use engine_input::{Buttons, FighterId};

    fn config(table_version: u32) -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 80,
            fighters: 2,
            half_extent_m: 20.0,
            table_version,
        }
    }

    /// Slot 0 presses attack on tick 0; everyone faces +z.
    fn press_tape(config: &ArenaConfig) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("slots 0..2 are legal"),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: if tick == 0 && fighter == 0 {
                        Buttons(ATTACK)
                    } else {
                        Buttons::NONE
                    },
                });
            }
        }
        FighterTape(inputs)
    }

    /// Slot 0 swings at slot 1, one metre ahead, while slot 1 wears `kit`;
    /// returns the cuts and deflect cues of the tick the blow lands.
    fn first_blow(kit: &[&[&str]]) -> (Vec<engine_combat::Cut>, Vec<DeflectCue>) {
        let config = config(TABLES_VERSION_V2);
        let tape = press_tape(&config);
        let mut sim = Sim::build_world(&config, &tape, false, false, MatchWorld::empty())
            .expect("the duel builds");
        sim.states[0].pos = [0.0, 0.0, 0.0];
        sim.states[1].pos = [0.0, 0.0, 1.0];
        sim.wear_armour(kit).expect("the kit wears under v2");
        let mut landed = false;
        while !landed {
            sim.step(&tape).expect("the duel steps");
            landed = sim.events.last().is_some_and(|row| {
                row.events
                    .iter()
                    .any(|event| matches!(event, FightEvent::Hit { attacker: 0, .. }))
            });
            assert!(sim.tick < 60, "the blow never landed");
        }
        let cuts = sim
            .cuts
            .last()
            .map(|row| row.cuts.clone())
            .unwrap_or_default();
        (cuts, sim.deflects().to_vec())
    }

    #[test]
    fn a_plated_torso_deflects_the_blow_a_bare_torso_cuts() {
        let (bare_cuts, bare_cues) = first_blow(&[&[], &[]]);
        assert!(
            bare_cuts.iter().any(|cut| cut.victim == 1),
            "the same blow cuts a bare torso"
        );
        assert!(bare_cues.is_empty(), "no plate, no deflect");

        let (plate_cuts, plate_cues) = first_blow(&[&[], &["lorica"]]);
        assert!(plate_cuts.is_empty(), "the plate stops the blow: no cut");
        assert_eq!(
            plate_cues,
            [DeflectCue {
                attacker: 0,
                victim: 1,
                joint: JointId::Waist.as_u8(),
            }],
            "the plate emits one deflect cue at the waist it struck"
        );
    }

    #[test]
    fn a_helmet_guards_but_never_deflects() {
        let (_, cues) = first_blow(&[&[], &["helmet"]]);
        assert!(cues.is_empty(), "only plate deflects");
    }

    #[test]
    fn armour_refuses_v1_late_and_bad_kits_wearing_nothing() {
        let mut v1 = Sim::build_world(
            &config(TABLES_VERSION_V1),
            &press_tape(&config(TABLES_VERSION_V1)),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("the v1 duel builds");
        assert_eq!(
            v1.wear_armour(&[&[], &["lorica"]]),
            Err(ArmourError::Closed)
        );
        assert_eq!(v1.worn(1), 0);

        let tape = press_tape(&config(TABLES_VERSION_V2));
        let mut sim = Sim::build_world(
            &config(TABLES_VERSION_V2),
            &tape,
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("the duel builds");
        assert_eq!(
            sim.wear_armour(&[&[]]),
            Err(ArmourError::Count {
                got: 1,
                fighters: 2
            })
        );
        assert_eq!(
            sim.wear_armour(&[&[], &["shield"]]),
            Err(ArmourError::Piece { slot: 1 })
        );
        assert_eq!(sim.worn(1), 0, "a refused kit wears nothing");
        sim.wear_armour(&[&["scutum"], &["lorica", "helmet"]])
            .expect("a legal kit wears");
        assert_eq!(sim.worn(0), ARMOR_SCUTUM);
        assert_eq!(sim.worn(1), ARMOR_LORICA | ARMOR_GALEA);
        sim.step(&tape).expect("the duel steps");
        assert_eq!(
            sim.wear_armour(&[&[], &[]]),
            Err(ArmourError::Closed),
            "armour is fixed once the match starts"
        );
    }

    #[test]
    fn unworn_states_leave_the_hash_alone() {
        let bare = FighterState {
            id: FighterId::new(0).expect("slot 0 legal"),
            pos: [0.0, 0.0, 0.0],
            yaw: 0.0,
            worn: 0,
        };
        assert_eq!(fold(7, &[bare]), 7, "no armour, no bytes");
        let plated = FighterState {
            worn: ARMOR_LORICA,
            ..bare
        };
        assert_ne!(fold(7, &[plated]), 7, "worn armour moves the checkpoint");
    }
}
