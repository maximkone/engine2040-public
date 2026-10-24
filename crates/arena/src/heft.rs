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
//!
//! BG15: a row also brings its steel's condition, and the cut resolver scales
//! a landed blow by it ([`Sim::edge_wear_q16`]), so the kit's dulled edge
//! reaches the cut instead of dying at `wear_q16: 0`.

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

    /// The dulled edge slot `slot`'s kit carries, in Q16 of one (BG15): the
    /// armed row's own wear, `0` for a bare hand, a v1 match or a slot out of
    /// the match. `authority::resolve_cuts` scales the
    /// blow by it, so a dulled kit cuts less.
    #[must_use]
    pub fn edge_wear_q16(&self, slot: u8) -> u32 {
        self.weapon(slot).map_or(0, |heft| heft.edge_wear_q16())
    }

    /// The mean edge wear the kits of this match carry, in Q16, over the
    /// active slots (BG15): what a duel receipt prints beside a cut count so
    /// a reader sees the wear reach the steel, not only the row.
    #[must_use]
    pub fn mean_edge_wear_q16(&self) -> u32 {
        if self.fighters == 0 {
            return 0;
        }
        let total: u64 = (0..self.fighters)
            .map(|slot| u64::from(self.edge_wear_q16(u8::try_from(slot).unwrap_or(u8::MAX))))
            .sum();
        u32::try_from(total / u64::try_from(self.fighters).unwrap_or(1)).unwrap_or(u32::MAX)
    }
}

/// Fold the armed weapon rows into a checkpoint hash: the tag byte, then per
/// active slot its mass, damage, startup, recovery, react and edge wear. A
/// match with no weapon armed returns the hash untouched (golden-frozen
/// bytes).
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
            edge_wear_permille: 0,
        });
        digest.write_u32(heft.mass_g);
        digest.write_u32(u32::from(heft.damage));
        digest.write_u32(heft.startup_ticks);
        digest.write_u32(heft.recovery_ticks);
        digest.write_u32(heft.react_ticks);
        digest.write_u32(u32::from(heft.edge_wear_permille));
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

    /// Both slots press attack every `every` ticks for a whole round: enough
    /// steel in the air for the cut path to run, no wall clock in it.
    fn assault_tape(config: &ArenaConfig, every: u32) -> FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for fighter in 0..config.fighters {
                let press = tick % every == 0;
                inputs.push(FighterInput {
                    tick,
                    fighter: FighterId::new(fighter).expect("slots 0..2 are legal"),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: if press {
                        Buttons(ATTACK)
                    } else {
                        Buttons::NONE
                    },
                });
            }
        }
        FighterTape(inputs)
    }

    /// BG15: the kit's steel comes to the duel with it. Five headless rounds
    /// of the kit gladius print the mean edge wear the slots carry, beside
    /// what the rounds landed, and the wear must not read zero — nor stop at
    /// the readout: every cut the rounds land carries the kit's dulled
    /// energy, never the fresh-edge number.
    #[test]
    fn bg15_a_five_round_headless_duel_prints_a_non_zero_mean_edge_wear() {
        use engine_combat::{effective_q16, BladeSwing, MAX_ENERGY_Q16, MAX_SHARPNESS_Q16};
        let round = config(TABLES_VERSION_V2);
        let tape = assault_tape(&round, 20);
        let gladius = Heft::from_rows(WEAPONS_ROWS, "gladius").expect("the shipped row");
        // The energy the arena hands the resolver: its full-force blade, the
        // kit row's wear. Geometry is irrelevant to `effective_q16`.
        let blade = |wear_q16: u32| BladeSwing {
            attacker: 0,
            victim: 1,
            swing: 0,
            p0_mm: [0, 0, 0],
            p1_mm: [0, 0, 0],
            blade_half_mm: 0,
            energy_q16: MAX_ENERGY_Q16,
            sharpness_q16: MAX_SHARPNESS_Q16,
            wear_q16,
        };
        let kit_cut_energy =
            u32::try_from(effective_q16(&blade(gladius.edge_wear_q16()))).expect("fits a cut row");
        let fresh_cut_energy = u32::try_from(effective_q16(&blade(0))).expect("fits a cut row");
        assert!(
            kit_cut_energy < fresh_cut_energy,
            "the kit's dulled row must carry less than fresh steel: {kit_cut_energy}"
        );
        let mut hits = 0usize;
        let mut cuts = 0usize;
        let mut mean_wear_q16 = 0u32;
        for round_index in 1..=5 {
            let mut sim = duel(&tape);
            sim.arm_weapons(&["gladius", "gladius"])
                .expect("the kit gladius arms");
            mean_wear_q16 = sim.mean_edge_wear_q16();
            assert!(
                mean_wear_q16 > 0,
                "round {round_index}: the armed kit must carry its edge wear into the match"
            );
            while sim.tick < round.ticks {
                sim.step(&tape).expect("the round steps");
            }
            hits += sim
                .events
                .iter()
                .flat_map(|row| row.events.iter())
                .filter(|event| matches!(event, FightEvent::Hit { .. }))
                .count();
            cuts += sim.cuts.iter().map(|row| row.cuts.len()).sum::<usize>();
            for row in &sim.cuts {
                for hit in &row.cuts {
                    assert_eq!(
                        hit.energy_q16, kit_cut_energy,
                        "round {round_index}: tick {} cut the kit's own dulled energy, not a fresh edge",
                        row.tick
                    );
                }
            }
            println!(
                "arena kit duel: round={round_index} wear_q16={} slots=[{},{}] hits_total={hits} cuts_total={cuts}",
                mean_wear_q16,
                sim.edge_wear_q16(0),
                sim.edge_wear_q16(1),
            );
        }
        println!(
            "arena kit duel: rounds=5 kit=gladius mean_edge_wear_q16={mean_wear_q16} hits={hits} cuts={cuts}"
        );
        assert!(hits > 0, "the rounds must land blows, got {hits}");
        assert!(cuts > 0, "the rounds must sever joints, got {cuts}");
        assert!(
            mean_wear_q16 > 0,
            "the duel's mean edge wear reads zero: the kit wear never reached the blow"
        );
        // The duel reads the row's own number, not a fresh edge.
        assert_eq!(mean_wear_q16, gladius.edge_wear_q16());
    }

    /// BG15: the wear reads through the armed slot and stays `0` for a bare
    /// hand, so a match that arms nothing cuts exactly as it did.
    #[test]
    fn bg15_a_bare_slot_carries_no_edge_wear() {
        let tape = press_tape(&config(TABLES_VERSION_V2));
        let mut sim = duel(&tape);
        assert_eq!(sim.edge_wear_q16(0), 0, "a bare hand has no edge to dull");
        assert_eq!(sim.mean_edge_wear_q16(), 0);
        sim.arm_weapons(&["pilum", "gladius"])
            .expect("the rows arm");
        let (pilum, gladius) = (
            Heft::from_rows(WEAPONS_ROWS, "pilum").expect("the shipped row"),
            Heft::from_rows(WEAPONS_ROWS, "gladius").expect("the shipped row"),
        );
        assert_eq!(sim.edge_wear_q16(0), pilum.edge_wear_q16());
        assert_eq!(
            sim.mean_edge_wear_q16(),
            (pilum.edge_wear_q16() + gladius.edge_wear_q16()) / 2,
            "the mean sits over the active slots"
        );
        assert_eq!(
            sim.edge_wear_q16(9),
            0,
            "a slot out of the match carries none"
        );
    }
}
