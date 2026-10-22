//! Weapon weight in the live duel (FEEL1): one weapon row per slot from
//! `content/design/weapons.rows`, armed before the first tick.
//!
//! An armed weapon gives its swinger the row's own startup, recovery and
//! damage and the stagger its clean hit holds on the victim
//! ([`engine_combat::heft`]); a pugio strikes first and frees its wielder
//! soon, a clava lands late and holds its victim long. Off by default, and
//! off leaves every tick and every hash untouched, so the golden and the
//! arena digest never move. Armed, the rows fold into the checkpoint after
//! the rules, so two bouts with other weapons never hash alike.

use crate::Sim;
use engine_combat::fight::{FightState, FIGHT_RULES_V2};
use engine_combat::heft::{Heft, HeftError};

/// The shipped weapons table: the designer rows the live duel arms.
/// Embedded at build time, so a tick never reads a file.
pub const WEAPONS_ROWS: &str = include_str!("../../../content/design/weapons.rows");

/// Checkpoint tag byte for the armed weapon rows.
const HEFT_TAG: u8 = 0xB5;

/// Why [`Sim::arm_weapons`] refused: nothing was armed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ArmError {
    /// Weapons arm only under the rules v2 rows, before the first tick.
    Closed,
    /// One weapon id per fighter, no more and no fewer.
    Count {
        /// Ids given.
        got: usize,
        /// Fighters in the match.
        fighters: usize,
    },
    /// A weapon row the rows refuse.
    Row(HeftError),
}

impl core::fmt::Display for ArmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Closed => f.write_str("weapons arm only under rules v2, before the first tick"),
            Self::Count { got, fighters } => {
                write!(
                    f,
                    "{got} weapon ids for {fighters} fighters: give one per fighter"
                )
            }
            Self::Row(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ArmError {}

impl Sim {
    /// Arm one weapon per fighter, in slot order, from the shipped rows
    /// ([`WEAPONS_ROWS`]): `["pugio", "clava"]` gives slot 0 the dagger and
    /// slot 1 the club.
    ///
    /// # Errors
    ///
    /// [`ArmError`] when the match is past tick 0 or runs the rules v1 rows,
    /// the id count is not the fighter count, or a row is refused; nothing
    /// is armed then.
    pub fn arm_weapons(&mut self, ids: &[&str]) -> Result<(), ArmError> {
        self.arm_weapon_rows(WEAPONS_ROWS, ids)
    }

    /// [`Sim::arm_weapons`] from explicit rows text (a mod's own table).
    ///
    /// # Errors
    ///
    /// As [`Sim::arm_weapons`].
    pub fn arm_weapon_rows(&mut self, rows: &str, ids: &[&str]) -> Result<(), ArmError> {
        if self.tick != 0 || self.rules.version != FIGHT_RULES_V2 {
            return Err(ArmError::Closed);
        }
        if ids.len() != self.fighters {
            return Err(ArmError::Count {
                got: ids.len(),
                fighters: self.fighters,
            });
        }
        let hefts = ids
            .iter()
            .map(|id| Heft::from_rows(rows, id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(ArmError::Row)?;
        for (fighter, heft) in self.fight.fighters.iter_mut().zip(hefts) {
            fighter.heft = Some(heft);
        }
        Ok(())
    }

    /// The weapon slot `slot` swings, or `None` when it swings the table row
    /// or the slot is out of the match. Presentation reads it to time the
    /// swing and size the impact; never hashed here.
    #[must_use]
    pub fn weapon(&self, slot: u8) -> Option<Heft> {
        self.fight.fighter(slot)?.heft
    }
}

/// Fold the armed weapon rows into a checkpoint hash: the tag byte, then per
/// active slot its mass, damage, startup, recovery and react. A match with
/// no weapon armed returns the hash untouched (golden-frozen bytes).
pub(crate) fn fold(hash: u64, fight: &FightState, fighters: usize) -> u64 {
    let active = fight.fighters.get(..fighters).unwrap_or(&[]);
    if active.iter().all(|fighter| fighter.heft.is_none()) {
        return hash;
    }
    let mut digest = engine_core::hash::Fnv64::with_state(hash);
    digest.write_u8(HEFT_TAG);
    for fighter in active {
        let heft = fighter.heft.unwrap_or(Heft {
            mass_g: 0,
            damage: 0,
            startup_ticks: 0,
            recovery_ticks: 0,
            react_ticks: 0,
        });
        digest.write_u32(heft.mass_g);
        digest.write_u32(u32::from(heft.damage));
        digest.write_u32(heft.startup_ticks);
        digest.write_u32(heft.recovery_ticks);
        digest.write_u32(heft.react_ticks);
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
    use engine_input::fighter::ATTACK;
    use engine_input::{Buttons, FighterId};

    /// What one swing did, measured off the live duel.
    #[derive(Debug, PartialEq, Eq)]
    struct Swing {
        /// Ticks from the press to the hit.
        startup: u32,
        /// Stepped fight ticks the swinger stayed committed after the hit.
        recovery: u32,
        /// Hit-react ticks the hit put on the victim.
        stagger: u32,
        /// Health the victim lost, permille.
        damage: u16,
    }

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

    /// A rules v2 duel at arm's length: slot 0 swings at slot 1, 1 m ahead.
    fn duel(tape: &FighterTape) -> Sim {
        let mut sim = Sim::build_world(
            &config(TABLES_VERSION_V2),
            tape,
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("the duel builds");
        sim.states[0].pos = [0.0, 0.0, 0.0];
        sim.states[1].pos = [0.0, 0.0, 1.0];
        sim
    }

    /// Slot 0 swings `weapon` at a bare slot 1 once and the duel measures it.
    fn swing(weapon: &str) -> Swing {
        let tape = press_tape(&config(TABLES_VERSION_V2));
        let mut sim = duel(&tape);
        sim.arm_weapon_rows(
            &format!("{WEAPONS_ROWS}\nbare {}", bare_row()),
            &[weapon, "bare"],
        )
        .expect("the rows arm");
        let mut hit = None;
        while hit.is_none() {
            sim.step(&tape).expect("the duel steps");
            let row = sim.events.last().expect("a row per tick");
            hit = row.events.iter().find_map(|event| match *event {
                FightEvent::Hit {
                    attacker: 0,
                    damage,
                    ..
                } => Some((row.tick, damage)),
                _ => None,
            });
            assert!(sim.tick < 60, "{weapon} never landed");
        }
        let (startup, damage) = hit.expect("landed");
        // The hit tick itself is the victim's first held tick (its react
        // clock already ran once); count every stepped fight tick after it
        // while the swinger is committed and while the victim is held.
        let (mut recovery, mut stagger) = (0, 1);
        while sim.fight.fighters[0].is_attacking || sim.fight.fighters[1].react > 0 {
            let frozen = sim.fight.hitstop > 0;
            let (committed, held) = (
                sim.fight.fighters[0].is_attacking,
                sim.fight.fighters[1].react > 0,
            );
            sim.step(&tape).expect("the duel steps");
            if !frozen {
                recovery += u32::from(committed);
                stagger += u32::from(held);
            }
        }
        Swing {
            startup,
            recovery,
            stagger,
            damage,
        }
    }

    /// The gladius numbers under another id: the victim swings the table.
    fn bare_row() -> &'static str {
        "mass_g=1000 damage_permille=120 speed_ticks=15 recovery_ticks=8 stagger_permille=400"
    }

    #[test]
    fn a_dagger_and_a_club_swing_with_their_own_weight() {
        let dagger = swing("pugio");
        let club = swing("clava");
        println!("pugio: {dagger:?}");
        println!("clava: {club:?}");
        let rows = |id| Heft::from_rows(WEAPONS_ROWS, id).expect("shipped row");
        let (pugio, clava) = (rows("pugio"), rows("clava"));
        assert_eq!(
            dagger.startup, pugio.startup_ticks,
            "the pugio lands on its row's tick"
        );
        assert_eq!(
            club.startup, clava.startup_ticks,
            "the clava lands on its row's tick"
        );
        assert_eq!(dagger.recovery, pugio.recovery_ticks);
        assert_eq!(club.recovery, clava.recovery_ticks);
        assert_eq!(dagger.stagger, pugio.react_ticks);
        assert_eq!(club.stagger, clava.react_ticks);
        assert_eq!(dagger.damage, pugio.damage);
        assert_eq!(club.damage, clava.damage);
        assert!(
            club.startup > dagger.startup
                && club.recovery > dagger.recovery
                && club.stagger > dagger.stagger,
            "the club starts later, recovers longer and staggers harder: {club:?} vs {dagger:?}"
        );
    }

    #[test]
    fn a_gladius_bout_steps_as_the_table_and_only_the_hash_tag_differs() {
        let tape = press_tape(&config(TABLES_VERSION_V2));
        let mut bare = duel(&tape);
        let mut armed = duel(&tape);
        armed
            .arm_weapons(&["gladius", "gladius"])
            .expect("the gladius arms");
        for _ in 0..40 {
            bare.step(&tape).expect("steps");
            armed.step(&tape).expect("steps");
            assert_eq!(
                bare.events.last(),
                armed.events.last(),
                "same outcomes, tick by tick"
            );
        }
        let (bare_hash, armed_hash) = (
            bare.checkpoints.last().expect("a checkpoint").hash,
            armed.checkpoints.last().expect("a checkpoint").hash,
        );
        assert_ne!(bare_hash, armed_hash, "an armed bout folds its weapons");
    }

    #[test]
    fn weapons_refuse_v1_a_started_match_a_short_list_and_an_unknown_row() {
        let v1 = config(TABLES_VERSION_V1);
        let tape = press_tape(&v1);
        let mut old =
            Sim::build_world(&v1, &tape, false, false, MatchWorld::empty()).expect("builds");
        assert_eq!(old.arm_weapons(&["pugio", "pugio"]), Err(ArmError::Closed));
        let v2 = press_tape(&config(TABLES_VERSION_V2));
        let mut sim = duel(&v2);
        assert_eq!(
            sim.arm_weapons(&["pugio"]),
            Err(ArmError::Count {
                got: 1,
                fighters: 2
            })
        );
        assert!(matches!(
            sim.arm_weapons(&["pugio", "falx"]),
            Err(ArmError::Row(_))
        ));
        assert_eq!(sim.weapon(0), None, "a refused list arms nothing");
        sim.step(&v2).expect("steps");
        assert_eq!(sim.arm_weapons(&["pugio", "pugio"]), Err(ArmError::Closed));
    }

    #[test]
    fn unarmed_fold_leaves_the_hash_untouched() {
        let fight = FightState::new(2).expect("two fighters");
        assert_eq!(fold(0x1234, &fight, 2), 0x1234);
    }
}
