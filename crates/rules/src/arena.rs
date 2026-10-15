//! The arena adapter: a rules script drives chosen fighter slots of the real arena match.
//!
//! [`RulesController`] implements `engine_arena::SlotController`. Once per tick the arena hands it
//! every fighter as integers; it builds the rules [`World`] (the merged input row gives each
//! fighter's wish and buttons: the brain's proposal for a scripted slot, the tape's for a
//! human one), runs the installed script for the scripted slots only ([`Rules::tick_slots`])
//! and folds the script's *input* actions (`walk`, `face`, `press`) into those slots' inputs.
//! Effect actions (`damage`, `heal`, `stun`, `drain_stamina`) are buffered and handed back
//! through [`SlotController::take_effects`](engine_arena::SlotController::take_effects),
//! which the arena applies to fight state before combat resolves; `push` and `shield` are
//! only counted here and stay ignored in the real arena.
//!
//! A failing script call is a [`Fault`](crate::Fault): that slot keeps the brain's proposal
//! for the tick, the run goes on, and the fault is counted in [`DriveStats`]. Memory is reset
//! when a controller is built, so the same config twice gives the same checkpoints.
//!
//! Feature `arena` only: the default build of every tick crate carries no scripting engine.

use std::cell::RefCell;
use std::rc::Rc;

use engine_arena::controller::SlotInput;
use engine_arena::controller::{Buttons, ScriptEffect};
use engine_arena::{ControlView, SlotController};

use crate::error::RulesError;
use crate::host::{Action, ActorView, World};
use crate::limits::MAX_FIGHTERS;
use crate::rules::{Explanation, Rules};

/// What one driven run did, readable after the arena finished.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriveStats {
    /// Ticks the script was asked to decide.
    pub ticks: u32,
    /// Input actions folded into scripted slots.
    pub inputs_applied: u32,
    /// Effect actions buffered for the arena to apply (`damage`, `heal`, `stun`,
    /// `drain_stamina`).
    pub effects_applied: u32,
    /// Effect actions the script emitted that the real arena does not apply
    /// (`push`, `shield`).
    pub effects_ignored: u32,
    /// Script calls that failed at run time (the slot kept the brain's input).
    pub faults: u32,
    /// The most recent failure, if any.
    pub last_error: Option<RulesError>,
    /// Hash of the script that ran, 0 when none was installed.
    pub script_hash: u64,
    /// The dry-run explanation of the scripted slots at the tick asked for with
    /// [`RulesController::explain_at`].
    pub explanation: Option<Explanation>,
}

/// Drives the slots in a bit mask with the script installed in a shared [`Rules`].
pub struct RulesController {
    rules: Rc<RefCell<Rules>>,
    slots: u64,
    explain_at: Option<u32>,
    stats: Rc<RefCell<DriveStats>>,
    /// Effect actions from the last [`control`](SlotController::control) call, in
    /// `(actor id, emission)` order, waiting for the arena to drain them.
    pending: Vec<ScriptEffect>,
}

impl RulesController {
    /// A controller for `slots` (each below 64; the arena has at most 16); rule memory is
    /// reset so the match starts clean.
    pub fn new(rules: Rc<RefCell<Rules>>, slots: &[u8]) -> Self {
        let mask = slots
            .iter()
            .filter(|slot| usize::from(**slot) < MAX_FIGHTERS)
            .fold(0_u64, |mask, slot| mask | (1_u64 << slot));
        {
            let mut engine = rules.borrow_mut();
            engine.reset_memory();
        }
        let hash = rules.borrow().script_hash().unwrap_or(0);
        Self {
            rules,
            slots: mask,
            explain_at: None,
            stats: Rc::new(RefCell::new(DriveStats {
                script_hash: hash,
                ..DriveStats::default()
            })),
            pending: Vec::new(),
        }
    }

    /// Also dry-run an explanation of the scripted slots when tick `tick` is decided (every
    /// host call logged); it lands in [`DriveStats::explanation`]. Changes nothing in the match.
    #[must_use]
    pub fn explain_at(mut self, tick: u32) -> Self {
        self.explain_at = Some(tick);
        self
    }

    /// The stats cell: clone it before boxing the controller into a run.
    pub fn stats(&self) -> Rc<RefCell<DriveStats>> {
        Rc::clone(&self.stats)
    }

    /// The rules world for this tick: every fighter in slot order, the row's inputs read in.
    fn world(view: &ControlView<'_>, row: &[SlotInput]) -> World {
        let fighters = view
            .slots
            .iter()
            .zip(row)
            .enumerate()
            .map(|(index, (slot, input))| {
                let id = u8::try_from(index).unwrap_or(u8::MAX);
                ActorView {
                    id,
                    // Free-for-all: every fighter is its own team, like the brains' foe search.
                    team: id,
                    alive: slot.alive,
                    x_mm: slot.x_mm,
                    z_mm: slot.z_mm,
                    facing_cdeg: input.facing,
                    hp_permille: slot.hp_permille,
                    stamina: slot.stamina,
                    attack_age: slot.attack_age,
                    attacking: slot.attacking,
                    blocking: slot.blocking,
                    dodging: slot.dodging,
                    react: slot.react,
                    knockdown: slot.knockdown,
                    shield: 0,
                    mx: input.mx,
                    mz: input.mz,
                    buttons: input.buttons.0,
                }
            })
            .collect();
        World {
            tick: view.tick,
            seed: view.seed,
            fighters,
        }
    }
}

impl SlotController for RulesController {
    fn controls(&self, slot: u8) -> bool {
        usize::from(slot) < MAX_FIGHTERS && (self.slots >> slot) & 1 == 1
    }

    fn control(&mut self, view: &ControlView<'_>, row: &mut [SlotInput]) {
        let world = Self::world(view, row);
        let mut stats = self.stats.borrow_mut();
        if self.explain_at == Some(view.tick) {
            stats.explanation = self.rules.borrow().explain_slots(&world, self.slots).ok();
        }
        let report = self.rules.borrow_mut().tick_slots(&world, self.slots);
        stats.ticks += 1;
        let report = match report {
            Ok(report) => report,
            Err(error) => {
                stats.faults += 1;
                stats.last_error = Some(error);
                return;
            }
        };
        for fault in &report.faults {
            stats.faults += 1;
            stats.last_error = Some(fault.error.clone());
        }
        for emitted in &report.actions {
            let Some(input) = row.get_mut(usize::from(emitted.actor)) else {
                continue;
            };
            match emitted.action {
                Action::Wish { mx, mz } => {
                    input.mx = mx;
                    input.mz = mz;
                }
                Action::Face { cdeg } => input.facing = cdeg,
                Action::Press { buttons } => input.buttons = Buttons(buttons),
                Action::Damage { target, permille } => {
                    self.pending.push(ScriptEffect::Damage { target, permille });
                    stats.effects_applied += 1;
                    continue;
                }
                Action::Heal { target, permille } => {
                    self.pending.push(ScriptEffect::Heal { target, permille });
                    stats.effects_applied += 1;
                    continue;
                }
                Action::Stun { target, ticks } => {
                    self.pending.push(ScriptEffect::Stun { target, ticks });
                    stats.effects_applied += 1;
                    continue;
                }
                Action::DrainStamina { target, amount } => {
                    self.pending
                        .push(ScriptEffect::DrainStamina { target, amount });
                    stats.effects_applied += 1;
                    continue;
                }
                Action::Push { .. } | Action::Shield { .. } => {
                    stats.effects_ignored += 1;
                    continue;
                }
            }
            stats.inputs_applied += 1;
        }
    }

    /// Hand the buffered effects to the arena, in `(actor id, emission)` order; the
    /// buffer is empty again afterwards. A failed call already dropped its actions in
    /// [`Rules::tick_slots`], so only clean effects travel.
    fn take_effects(&mut self) -> Vec<ScriptEffect> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod arena_controls_tests {
    //! Caller-visible contract for the arena adapter: which slots the script
    //! drives, and the fresh-controller shape the arena drains.
    //!
    //! Consumer: the real arena calls `controls(slot)` per fighter, drains
    //! `take_effects` after every tick and reads `stats` for the report.
    //! Units: slot ids `0..64` (`MAX_FIGHTERS`). Determinism: pure mask plus
    //! counters, no clock, no RNG, no IO. Each test below names the root
    //! path, so a removed mask check or a non-empty fresh buffer fails here
    //! first.

    use std::cell::RefCell;
    use std::rc::Rc;

    use engine_arena::SlotController;

    use super::{DriveStats, RulesController};
    use crate::Rules;

    /// The arena only drives masked slots below the 64-fighter cap: an
    /// out-of-range config arms nothing, so it cannot shift-overflow.
    #[test]
    fn controls_pins_only_masked_slots_below_cap() {
        let rules = Rc::new(RefCell::new(Rules::default()));
        let controller = RulesController::new(Rc::clone(&rules), &[0, 1, 63]);
        assert!(controller.controls(0), "slot 0 masked drives");
        assert!(controller.controls(1), "slot 1 masked drives");
        assert!(controller.controls(63), "slot 63 is the top valid id");
        assert!(
            !controller.controls(2),
            "slot 2 unmasked stays brain-driven"
        );
        assert!(
            !controller.controls(62),
            "slot 62 unmasked stays brain-driven"
        );
        assert!(!controller.controls(64), "slot 64 is past the cap");
        assert!(!controller.controls(70), "slot 70 filtered at build");
        assert!(!controller.controls(255), "slot 255 never driven");

        let wide = RulesController::new(Rc::clone(&rules), &[70, 200, 255]);
        for slot in [0u8, 1, 63, 64, 70, 200, 255] {
            assert!(
                !wide.controls(slot),
                "out-of-range config drives nothing at {slot}"
            );
        }

        let none = RulesController::new(Rc::clone(&rules), &[]);
        assert!(!none.controls(0), "empty mask drives nothing");
        assert!(!none.controls(63), "empty mask drives nothing at the top");
    }

    /// A fresh controller drains empty and reports zero counters with hash 0
    /// and no error or explanation; `explain_at` keeps the same mask.
    #[test]
    fn fresh_controller_starts_empty_with_zero_hash() {
        let rules = Rc::new(RefCell::new(Rules::default()));
        let mut controller = RulesController::new(Rc::clone(&rules), &[0]);
        assert!(
            controller.take_effects().is_empty(),
            "fresh buffer drains empty"
        );
        assert!(
            controller.take_effects().is_empty(),
            "drain stays empty without a tick"
        );
        let stats = controller.stats();
        let seen = stats.borrow();
        assert_eq!(seen.ticks, 0, "no tick decided yet");
        assert_eq!(seen.inputs_applied, 0, "no input folded yet");
        assert_eq!(seen.effects_applied, 0, "no effect buffered yet");
        assert_eq!(seen.effects_ignored, 0, "no effect ignored yet");
        assert_eq!(seen.faults, 0, "no fault counted yet");
        assert_eq!(seen.script_hash, 0, "no script installed hashes 0");
        assert_eq!(seen.last_error, None, "no failure recorded yet");
        assert!(seen.explanation.is_none(), "no explanation yet");
        drop(seen);

        assert_eq!(
            DriveStats::default().script_hash,
            0,
            "default stats hash 0 via root"
        );
        assert_eq!(DriveStats::default().ticks, 0, "default ticks 0");

        let chained = RulesController::new(Rc::clone(&rules), &[0]).explain_at(30);
        assert!(chained.controls(0), "explain_at keeps slot 0");
        assert!(!chained.controls(1), "explain_at keeps slot 1 undriven");
    }
}
