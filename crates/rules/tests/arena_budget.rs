//! Quiet-box p99 for the arena adapter: time `RulesController::control` per tick.
//!
//! Needs the `arena` feature and a quiet box (loops off or heavy queue idle):
//! `cargo test --release -p engine-rules --features arena --test arena_budget -- --ignored`
//! Wall-clock measurement is test-only (the tick never reads a clock), so this file is
//! allowed the types the simulation clippy config bans.
#![cfg(feature = "arena")]
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "timing gates measure wall time; nothing here runs inside a tick"
)]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use engine_arena::controller::{Buttons, ControlView, SlotController, SlotView};
use engine_arena::{FighterId, FighterInput};
use engine_rules::arena::RulesController;
use engine_rules::Rules;

const BASH: &str = include_str!("../scripts/shield_bash_arena.rhai");

/// Fighters on the adapter's busiest path: every slot scripted, every fighter its own
/// team, so each script call scans the full foe list (the worst `nearest_foe` case).
const FIGHTERS: usize = 16;
/// Timed ticks, after a short untimed warm-up.
const TICKS: u32 = 600;

/// Sixteen living fighters with integer positions that walk a little each tick, so no two
/// ticks cost exactly the same scan. Deterministic: the same tick always builds the same row.
fn slots(tick: u32) -> Vec<SlotView> {
    (0..FIGHTERS)
        .map(|index| {
            let drift = (tick.wrapping_mul(13).wrapping_add(index as u32 * 101) % 2_000) as i32;
            SlotView {
                alive: true,
                x_mm: (index as i32 - 8) * 1_400 + drift - 1_000,
                z_mm: (index as i32 % 4) * 2_000 - drift,
                facing_cdeg: 0,
                hp_permille: 1_000,
                stamina: 1_000,
                attack_age: 0,
                attacking: false,
                blocking: false,
                dodging: false,
                react: 0,
                knockdown: 0,
                ..SlotView::default()
            }
        })
        .collect()
}

fn row(tick: u32) -> Vec<FighterInput> {
    (0..FIGHTERS)
        .map(|index| FighterInput {
            tick,
            fighter: FighterId::new(u8::try_from(index).unwrap_or(u8::MAX))
                .expect("fewer than the participant cap"),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        })
        .collect()
}

fn drive(controller: &mut RulesController, tick: u32) {
    let view_slots = slots(tick);
    let view = ControlView {
        tick,
        seed: 2040,
        slots: &view_slots,
    };
    let mut input_row = row(tick);
    controller.control(&view, &mut input_row);
}

fn p99_ms(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    samples[samples.len() * 99 / 100]
}

#[test]
#[ignore = "strict timing gate for a quiet PC: cargo test --release -p engine-rules --features arena --test arena_budget -- --ignored"]
fn arena_drive_p99() {
    let mut rules = Rules::default();
    rules.set(BASH, 0).expect("the arena shield bash installs");
    let shared = Rc::new(RefCell::new(rules));
    let all: Vec<u8> = (0..FIGHTERS as u8).collect();
    let mut controller = RulesController::new(Rc::clone(&shared), &all);
    let stats = controller.stats();
    // Warm up (first compile caches, allocator steady state); not timed.
    for tick in 0..60 {
        drive(&mut controller, tick);
    }
    let mut samples = Vec::with_capacity(TICKS as usize);
    for tick in 0..TICKS {
        let view_slots = slots(tick);
        let view = ControlView {
            tick,
            seed: 2040,
            slots: &view_slots,
        };
        let mut input_row = row(tick);
        let started = Instant::now();
        controller.control(&view, &mut input_row);
        samples.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let p99 = p99_ms(&mut samples);
    let stats = stats.borrow();
    eprintln!("arena drive p99 per tick ({FIGHTERS} scripted fighters, {TICKS} ticks): {p99:.4} ms; {stats:?}");
    assert_eq!(stats.ticks, 60 + TICKS);
    assert!(
        stats.inputs_applied > 0,
        "the script drove inputs: {stats:?}"
    );
    assert_eq!(stats.faults, 0, "no script call may fail: {stats:?}");
    assert!(p99 <= 0.2, "p99 {p99:.4} ms per tick, over the 0.2 ms bar");
}
