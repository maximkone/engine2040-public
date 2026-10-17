//! Cannae team: headless 8-fighter double envelopment.
//!
//! Consumer: packaging and quality review over the headless match path.
//! Data owner: this test owns the team script; `engine-arena` owns match
//! authority, `engine-game` owns the session config. Units: wishes are
//! percent of top speed, facing is centidegrees, ticks are fixed steps.
//! Determinism scope: same seed plus same script twice gives identical
//! checkpoints. Errors: config validates before the run; bad rows fail.

use engine_arena::{ArenaConfig, ControlView, FighterId, FighterInput, SlotController, SlotView};
use engine_game::GameConfig;
use engine_input::fighter::{Buttons, ATTACK};
use engine_input::FighterTape;

/// Fixed match seed both runs share.
const SEED: u64 = 2040;
/// Fixed match length in ticks.
const TICKS: u32 = 600;
/// Fighters in the Cannae team.
const FIGHTERS: u8 = 8;
/// Arena half-extent in meters.
const HALF_EXTENT_M: f32 = 20.0;
/// Ticks per compass step for the circling wings.
const STEP_TICKS: u32 = 15;
/// Holder back-step: the center yields ground slowly.
const HOLDER_MX: i8 = -25;
/// Eight-point compass cycle the wings walk, percent of top speed.
const CIRCLE: [(i8, i8); 8] = [
    (100, 0),
    (70, 70),
    (0, 100),
    (-70, 70),
    (-100, 0),
    (-70, -70),
    (0, -100),
    (70, -70),
];

/// Cannae team script: slots 0-1 hold the center and yield, slots 2-4
/// circle port-side, slots 5-7 circle starboard-side. Pure function of
/// tick plus slot, so reruns are bit-identical.
struct CannaeTeam;

impl CannaeTeam {
    /// True for the two center holders.
    fn is_holder(slot: u8) -> bool {
        slot < 2
    }

    /// Compass index for a flanker at this tick; the right wing mirrors
    /// the left so both wings wheel inward. Pure.
    fn circle_index(slot: u8, tick: u32) -> usize {
        let phase = tick / STEP_TICKS + u32::from(slot) * 2;
        let forward = (phase % 8) as usize;
        if slot < 5 {
            forward
        } else {
            (8 - forward) % 8
        }
    }

    /// Facing for a compass index, centidegrees inside the legal range.
    /// Pure.
    fn facing_for(index: usize) -> i16 {
        i16::try_from(index).unwrap_or(0) * 4_500 - 13_500
    }

    /// Wish for one slot at this tick. Pure.
    fn wish_for(slot: u8, tick: u32) -> (i8, i8, i16, Buttons) {
        if Self::is_holder(slot) {
            (HOLDER_MX, 0, 0, Buttons::NONE)
        } else {
            let index = Self::circle_index(slot, tick);
            let (mx, mz) = CIRCLE[index];
            let buttons = if tick.is_multiple_of(30) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            };
            (mx, mz, Self::facing_for(index), buttons)
        }
    }
}

impl SlotController for CannaeTeam {
    fn controls(&self, slot: u8) -> bool {
        slot < FIGHTERS
    }

    fn control(&mut self, view: &ControlView<'_>, row: &mut [FighterInput]) {
        for (slot, input) in row.iter_mut().enumerate() {
            let id = u8::try_from(slot).unwrap_or(u8::MAX);
            if id >= FIGHTERS {
                continue;
            }
            let (mx, mz, facing, buttons) = Self::wish_for(id, view.tick);
            input.mx = mx;
            input.mz = mz;
            input.facing = facing;
            input.buttons = buttons;
        }
    }
}

/// Idle input for one slot at this tick.
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

/// Arena config for the Cannae match, copied from the game config.
fn arena_config() -> ArenaConfig {
    let game = GameConfig {
        seed: SEED,
        ticks: TICKS,
        fighters: FIGHTERS,
        half_extent_m: HALF_EXTENT_M,
    };
    game.validate().expect("cannae game config is valid");
    ArenaConfig {
        seed: game.seed,
        ticks: game.ticks,
        fighters: game.fighters,
        half_extent_m: game.half_extent_m,
        table_version: 1,
    }
}

#[test]
fn holders_yield_while_flankers_circle() {
    let slots = [SlotView::default(); FIGHTERS as usize];
    let view = ControlView {
        tick: 0,
        seed: SEED,
        slots: &slots,
    };
    let mut row: Vec<FighterInput> = (0..FIGHTERS).map(|slot| idle(0, slot)).collect();
    CannaeTeam.control(&view, &mut row);
    for slot in 0..FIGHTERS {
        let input = row[usize::from(slot)];
        if CannaeTeam::is_holder(slot) {
            assert_eq!(
                (input.mx, input.mz, input.buttons),
                (HOLDER_MX, 0, Buttons::NONE),
                "holder {slot} yields without striking"
            );
        } else {
            let index = CannaeTeam::circle_index(slot, 0);
            let (mx, mz) = CIRCLE[index];
            assert_eq!((input.mx, input.mz), (mx, mz), "flanker {slot} circles");
        }
    }
    // The wings move over time: tick 0 and tick STEP_TICKS differ.
    let later = ControlView {
        tick: STEP_TICKS,
        seed: SEED,
        slots: &slots,
    };
    let mut next: Vec<FighterInput> = (0..FIGHTERS).map(|slot| idle(STEP_TICKS, slot)).collect();
    CannaeTeam.control(&later, &mut next);
    assert_ne!(row[2].mx, next[2].mx, "left wing steps round the compass");
    assert_ne!(row[5].mx, next[5].mx, "right wing steps round the compass");
    // Holders keep yielding while the wings wheel.
    assert_eq!(next[0].mx, HOLDER_MX, "center still yields");
}

#[test]
fn seed_2040_replays_same_hash_twice() {
    let config = arena_config();
    let tape = FighterTape(Vec::new());
    let first = engine_arena::run_controlled(&config, &tape, &mut CannaeTeam);
    let second = engine_arena::run_controlled(&config, &tape, &mut CannaeTeam);
    assert_eq!(first.len() as u32, TICKS, "one checkpoint per tick");
    assert_eq!(second.len() as u32, TICKS, "rerun covers the same ticks");
    assert_eq!(first, second, "fixed seed 2040 replays the same hash");
    let plain = engine_arena::run(&config, &tape);
    assert_eq!(plain.len() as u32, TICKS, "plain run covers the same ticks");
    assert_ne!(first, plain, "team script steers off the all-brain path");
}
