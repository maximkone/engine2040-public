//! controller: the one hook where something outside the arena writes fighter inputs.
//!
//! Owner: engine-arena. Producer: any crate that implements [`SlotController`] (the rules crate
//! does, behind its `arena` feature, so the default build carries no scripting engine).
//! Consumers: the legacy step ([`run_controlled`](crate::run_controlled)) and the spine step
//! ([`spine_step::run_spine_controlled`](crate::spine_step::run_spine_controlled)).
//!
//! Once per tick, after the tape and the brains have filled the merged input row and before
//! combat resolves, the arena builds a [`ControlView`] (integers only: millimetres,
//! centidegrees, permille, ticks) and calls [`SlotController::control`] with the row. Only the
//! slots the controller claims through [`SlotController::controls`], and that the tape did not
//! cover this tick, may change; every other slot is put back, `tick` and `fighter` are forced,
//! and the wishes, facing and buttons are clamped to their legal ranges, so a controller can
//! never make the strict combat step refuse a run. A controller reads the world and writes
//! inputs; it never touches sim state. No controller means the arena runs the exact legacy
//! path (the golden is untouched).
//!
//! A controller may also return script effects ([`take_effects`](SlotController::take_effects)):
//! `damage`, `heal`, `stun` and `drain_stamina`, in `(actor id, emission)` order. The arena
//! applies them to fight state ([`apply_script_effects`]) after the inputs are decided and
//! before combat resolves; `push` and `shield` have no [`ScriptEffect`] and stay ignored.
//! The default is no effects, so controllers that only steer keep the legacy run exactly.
//!
//! Determinism: the view is built from the sim state by integer conversion (`round`, no libm
//! trig); the controller is called in one fixed place, once per tick.

/// Neutral names for the arena's input and tape types, for crates above the arena.
pub use crate::{FighterInput as SlotInput, FighterTape as SlotTape};
use engine_combat::{FightState, FighterState as CombatState};
pub use engine_input::fighter::Buttons;

use crate::{FighterId, FighterInput, MAX_SLOTS};

/// Largest forward or right wish, percent of top speed.
const WISH_MAX: i8 = 100;
/// Largest absolute facing, centidegrees.
const FACING_MAX: i16 = 18_000;
/// Every button bit the combat step knows (attack, block, dodge).
const BUTTONS_KNOWN: u8 = 0x07;
/// Full health in permille: heals cap here, and the dead (zero) never revive.
const FULL_HP_PERMILLE: u16 = 1_000;

/// One script effect the arena applies to fight state before combat resolves.
///
/// Integers only: slot ids into `FightState.fighters`, health in permille, time in ticks,
/// stamina in points. `push` and `shield` have no variant and stay ignored by construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptEffect {
    /// Remove health, permille (`0..=1000`).
    Damage {
        /// Fighter hit.
        target: u8,
        /// Permille removed.
        permille: u16,
    },
    /// Restore health, permille (`0..=1000`), capped at full, never revives the dead.
    Heal {
        /// Fighter healed.
        target: u8,
        /// Permille restored.
        permille: u16,
    },
    /// Stagger a fighter: `react` keeps the longest, never shortens.
    Stun {
        /// Fighter staggered.
        target: u8,
        /// Ticks of stagger.
        ticks: u16,
    },
    /// Remove stamina points (`0..=1000`), saturating at zero.
    DrainStamina {
        /// Fighter drained.
        target: u8,
        /// Points removed.
        amount: u16,
    },
}

/// Apply script effects to fight state in order, the same fold the reference rules arena
/// uses: damage saturates, heals cap at full and skip the dead, stuns keep the longest
/// stagger, drains saturate. Targets outside the live fighter range are skipped, so a
/// stale id writes nothing. An empty slice leaves the fight untouched.
pub fn apply_script_effects(fight: &mut FightState, effects: &[ScriptEffect]) {
    for effect in effects {
        let slot = match *effect {
            ScriptEffect::Damage { target, .. }
            | ScriptEffect::Heal { target, .. }
            | ScriptEffect::Stun { target, .. }
            | ScriptEffect::DrainStamina { target, .. } => usize::from(target),
        };
        if slot >= usize::from(fight.fighter_count) {
            continue;
        }
        let fighter = &mut fight.fighters[slot];
        match *effect {
            ScriptEffect::Damage { permille, .. } => {
                fighter.hp_permille = fighter.hp_permille.saturating_sub(permille);
            }
            ScriptEffect::Heal { permille, .. } => {
                if fighter.hp_permille > 0 {
                    fighter.hp_permille = fighter
                        .hp_permille
                        .saturating_add(permille)
                        .min(FULL_HP_PERMILLE);
                }
            }
            ScriptEffect::Stun { ticks, .. } => {
                fighter.react = fighter.react.max(u32::from(ticks));
            }
            ScriptEffect::DrainStamina { amount, .. } => {
                fighter.stamina = fighter.stamina.saturating_sub(amount);
            }
        }
    }
}

/// One fighter as a controller sees it: integers only, copied in for the call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotView {
    /// False once out of the fight (hit points at zero).
    pub alive: bool,
    /// Position east, millimetres.
    pub x_mm: i32,
    /// Position north, millimetres.
    pub z_mm: i32,
    /// Facing of the last applied input, centidegrees (0 before the first tick).
    pub facing_cdeg: i16,
    /// Health, permille.
    pub hp_permille: u16,
    /// Stamina points.
    pub stamina: u16,
    /// Ticks since the attack started.
    pub attack_age: u32,
    /// Attack in flight.
    pub attacking: bool,
    /// Block held.
    pub blocking: bool,
    /// Dodge in flight.
    pub dodging: bool,
    /// Stagger ticks left.
    pub react: u32,
    /// Knockdown ticks left.
    pub knockdown: u32,
}

/// What a controller sees at the start of a tick.
#[derive(Clone, Copy, Debug)]
pub struct ControlView<'a> {
    /// Zero-based fixed tick about to be simulated.
    pub tick: u32,
    /// Match seed.
    pub seed: u64,
    /// Every fighter in slot order (`slots.len()` is the fighter count).
    pub slots: &'a [SlotView],
}

/// Writes the inputs of the slots it claims. Not `Send`-bound: a controller may own a
/// single-threaded engine and live on one worker thread.
pub trait SlotController {
    /// True when this controller decides `slot` (read once per tick per slot).
    fn controls(&self, slot: u8) -> bool;

    /// Decide this tick. `row[slot]` holds the input the brain proposed; overwrite the
    /// claimed slots' entries (wishes, facing, buttons). Entries of other slots, and the
    /// `tick` and `fighter` fields, are restored by the arena after the call.
    fn control(&mut self, view: &ControlView<'_>, row: &mut [FighterInput]);

    /// Script effects from the last [`control`](Self::control) call, in apply order. The
    /// arena drains them once per tick and folds them into fight state before combat
    /// resolves. Default: no effects, so steering-only controllers keep the legacy run.
    /// Integer-only, no allocation when empty; `push` and `shield` never reach here.
    fn take_effects(&mut self) -> Vec<ScriptEffect> {
        Vec::new()
    }
}

/// Integer view of one fighter from arena meters and combat state.
pub(crate) fn slot_view(pos: [f32; 3], combat: &CombatState, facing_cdeg: i16) -> SlotView {
    SlotView {
        alive: combat.hp_permille > 0,
        x_mm: mm(pos[0]),
        z_mm: mm(pos[2]),
        facing_cdeg,
        hp_permille: combat.hp_permille,
        stamina: combat.stamina,
        attack_age: combat.attack_age,
        attacking: combat.is_attacking,
        blocking: combat.is_blocking,
        dodging: combat.is_dodging,
        react: combat.react,
        knockdown: combat.knockdown,
    }
}

/// Meters to millimetres (truncated, clamped; non-finite maps to zero).
fn mm(meters: f32) -> i32 {
    if !meters.is_finite() {
        return 0;
    }
    crate::match_wire::pos_mm([meters, 0.0, 0.0])[0]
}

/// Call the controller once for this tick and keep only what it may change.
///
/// `from_tape[slot]` marks slots the tape covered this tick: a human input always wins.
/// `row` holds `fighters` inputs for `tick`.
pub(crate) fn drive(
    controller: &mut dyn SlotController,
    view: &ControlView<'_>,
    row: &mut [FighterInput],
    from_tape: &[bool; MAX_SLOTS],
) {
    let fighters = row.len();
    let mut proposed = [row[0]; MAX_SLOTS];
    proposed[..fighters].copy_from_slice(row);
    controller.control(view, row);
    for (slot, input) in row.iter_mut().enumerate() {
        let id = u8::try_from(slot).unwrap_or(u8::MAX);
        let claimed = !from_tape[slot] && controller.controls(id);
        if claimed {
            *input = FighterInput {
                tick: view.tick,
                fighter: FighterId(id),
                mx: input.mx.clamp(-WISH_MAX, WISH_MAX),
                mz: input.mz.clamp(-WISH_MAX, WISH_MAX),
                facing: input.facing.clamp(-FACING_MAX, FACING_MAX),
                buttons: Buttons(input.buttons.0 & BUTTONS_KNOWN),
            };
        } else {
            *input = proposed[slot];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claims slot 1 and shoves everything it can reach out of range.
    struct Rowdy;

    impl SlotController for Rowdy {
        fn controls(&self, slot: u8) -> bool {
            slot == 1
        }

        fn control(&mut self, _: &ControlView<'_>, row: &mut [FighterInput]) {
            for input in row.iter_mut() {
                input.mx = 127;
                input.mz = -128;
                input.facing = i16::MAX;
                input.buttons = Buttons(0xFF);
                input.tick = 999;
                input.fighter = FighterId(7);
            }
        }
    }

    fn idle(tick: u32, slot: u8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId(slot),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    #[test]
    fn only_claimed_slots_change_and_values_are_clamped() {
        let slots = [SlotView::default(); 3];
        let view = ControlView {
            tick: 5,
            seed: 1,
            slots: &slots,
        };
        let mut row = [idle(5, 0), idle(5, 1), idle(5, 2)];
        drive(&mut Rowdy, &view, &mut row, &[false; MAX_SLOTS]);
        assert_eq!(row[0], idle(5, 0));
        assert_eq!(row[2], idle(5, 2));
        assert_eq!(
            row[1],
            FighterInput {
                tick: 5,
                fighter: FighterId(1),
                mx: 100,
                mz: -100,
                facing: 18_000,
                buttons: Buttons(0x07),
            }
        );
    }

    #[test]
    fn a_tape_input_wins_over_the_controller() {
        let slots = [SlotView::default(); 2];
        let view = ControlView {
            tick: 0,
            seed: 0,
            slots: &slots,
        };
        let mut row = [idle(0, 0), idle(0, 1)];
        let mut from_tape = [false; MAX_SLOTS];
        from_tape[1] = true;
        drive(&mut Rowdy, &view, &mut row, &from_tape);
        assert_eq!(row[1], idle(0, 1));
    }

    #[test]
    fn view_converts_meters_to_millimetres() {
        let combat = CombatState::default();
        let view = slot_view([1.2344, 0.0, -3.5], &combat, 450);
        assert_eq!((view.x_mm, view.z_mm, view.facing_cdeg), (1234, -3500, 450));
        assert!(view.alive);
        assert_eq!(
            slot_view([f32::NAN, 0.0, f32::INFINITY], &combat, 0).x_mm,
            0
        );
    }

    fn two_fight() -> FightState {
        FightState::new(2).expect("2 fighters are legal")
    }

    #[test]
    fn effects_apply_in_order_with_sim_fold() {
        let mut fight = two_fight();
        apply_script_effects(
            &mut fight,
            &[
                ScriptEffect::Damage {
                    target: 1,
                    permille: 300,
                },
                ScriptEffect::Stun {
                    target: 1,
                    ticks: 45,
                },
                ScriptEffect::DrainStamina {
                    target: 1,
                    amount: 40,
                },
            ],
        );
        assert_eq!(fight.fighters[1].hp_permille, 700);
        assert_eq!(fight.fighters[1].react, 45);
        assert_eq!(fight.fighters[1].stamina, 60);
        // A shorter stun never shortens, damage saturates at zero.
        apply_script_effects(
            &mut fight,
            &[
                ScriptEffect::Stun {
                    target: 1,
                    ticks: 5,
                },
                ScriptEffect::Damage {
                    target: 1,
                    permille: 900,
                },
            ],
        );
        assert_eq!(fight.fighters[1].react, 45);
        assert_eq!(fight.fighters[1].hp_permille, 0);
    }

    #[test]
    fn heal_caps_at_full_and_never_raises_the_dead() {
        let mut fight = two_fight();
        apply_script_effects(
            &mut fight,
            &[ScriptEffect::Damage {
                target: 0,
                permille: 200,
            }],
        );
        apply_script_effects(
            &mut fight,
            &[ScriptEffect::Heal {
                target: 0,
                permille: 500,
            }],
        );
        assert_eq!(fight.fighters[0].hp_permille, 1_000);
        apply_script_effects(
            &mut fight,
            &[ScriptEffect::Damage {
                target: 1,
                permille: 1_000,
            }],
        );
        apply_script_effects(
            &mut fight,
            &[ScriptEffect::Heal {
                target: 1,
                permille: 500,
            }],
        );
        assert_eq!(fight.fighters[1].hp_permille, 0, "the dead stay dead");
    }

    #[test]
    fn stale_targets_write_nothing_and_empty_is_a_no_op() {
        let mut fight = two_fight();
        let before = fight;
        apply_script_effects(
            &mut fight,
            &[
                ScriptEffect::Damage {
                    target: 2,
                    permille: 500,
                },
                ScriptEffect::Damage {
                    target: 9,
                    permille: 500,
                },
            ],
        );
        assert_eq!(fight, before);
        apply_script_effects(&mut fight, &[]);
        assert_eq!(fight, before);
    }

    /// Emits one damage per tick through the new hook; inputs stay the brain's.
    struct Blaster {
        hits: Vec<ScriptEffect>,
    }

    impl SlotController for Blaster {
        fn controls(&self, slot: u8) -> bool {
            slot == 0
        }

        fn control(&mut self, _: &ControlView<'_>, _: &mut [FighterInput]) {}

        fn take_effects(&mut self) -> Vec<ScriptEffect> {
            std::mem::take(&mut self.hits)
        }
    }

    #[test]
    fn the_default_hook_emits_nothing() {
        struct Calm;
        impl SlotController for Calm {
            fn controls(&self, _: u8) -> bool {
                true
            }
            fn control(&mut self, _: &ControlView<'_>, _: &mut [FighterInput]) {}
        }
        assert!(Calm.take_effects().is_empty());
    }

    #[test]
    fn control_then_drain_is_the_tick_shape_the_arena_runs() {
        let slots = [SlotView::default(); 2];
        let view = ControlView {
            tick: 0,
            seed: 0,
            slots: &slots,
        };
        let mut row = [idle(0, 0), idle(0, 1)];
        let mut blaster = Blaster {
            hits: vec![ScriptEffect::Damage {
                target: 1,
                permille: 120,
            }],
        };
        drive(&mut blaster, &view, &mut row, &[false; MAX_SLOTS]);
        // No input action was decided, so the brain's row is untouched ...
        assert_eq!(row, [idle(0, 0), idle(0, 1)]);
        // ... while the drained effect still lands on fight state.
        let mut fight = two_fight();
        apply_script_effects(&mut fight, &blaster.take_effects());
        assert_eq!(fight.fighters[1].hp_permille, 880);
        assert!(blaster.take_effects().is_empty(), "drained once");
    }
    /// Claims the slots in `mask` and holds attack, walking toward +x.
    struct Charger {
        mask: u8,
        calls: u32,
    }

    impl SlotController for Charger {
        fn controls(&self, slot: u8) -> bool {
            self.mask & (1 << slot) != 0
        }

        fn control(&mut self, view: &ControlView<'_>, row: &mut [FighterInput]) {
            self.calls += 1;
            for (slot, input) in row.iter_mut().enumerate() {
                if self.controls(u8::try_from(slot).unwrap_or(u8::MAX)) {
                    input.mx = 100;
                    input.buttons = Buttons(0x01);
                    input.facing = i16::try_from(view.tick % 100).unwrap_or(0);
                }
            }
        }
    }

    fn config() -> crate::ArenaConfig {
        crate::ArenaConfig {
            seed: 2040,
            ticks: 180,
            fighters: 3,
            half_extent_m: 10.0,
            table_version: engine_combat::tables::TABLES_VERSION_V1,
        }
    }

    #[test]
    fn no_claimed_slot_is_the_legacy_run_and_a_claim_changes_it() {
        let tape = crate::FighterTape(Vec::new());
        let plain = crate::run(&config(), &tape);
        assert_eq!(plain.len(), 180);
        let mut idle = Charger { mask: 0, calls: 0 };
        assert_eq!(crate::run_controlled(&config(), &tape, &mut idle), plain);
        assert_eq!(idle.calls, 180, "asked once per tick");
        let mut one = Charger { mask: 1, calls: 0 };
        let driven = crate::run_controlled(&config(), &tape, &mut one);
        assert_ne!(driven, plain);
        let mut again = Charger { mask: 1, calls: 0 };
        assert_eq!(crate::run_controlled(&config(), &tape, &mut again), driven);
    }

    #[cfg(feature = "spine")]
    #[test]
    fn the_spine_step_honors_the_same_hook() {
        use crate::spine_step::{run_spine, run_spine_controlled};
        use crate::world::MatchWorld;
        let tape = crate::FighterTape(Vec::new());
        let world = MatchWorld::default();
        let plain = run_spine(&config(), &tape, &world);
        let none = run_spine_controlled(
            &config(),
            &tape,
            &world,
            Box::new(Charger { mask: 0, calls: 0 }),
        );
        assert_eq!(none, plain);
        let driven = run_spine_controlled(
            &config(),
            &tape,
            &world,
            Box::new(Charger { mask: 1, calls: 0 }),
        );
        assert_ne!(driven, plain);
        // Empty-world spine equals the legacy step, controller or not.
        let legacy = crate::run_controlled(&config(), &tape, &mut Charger { mask: 1, calls: 0 });
        assert_eq!(driven, legacy);
    }
}
