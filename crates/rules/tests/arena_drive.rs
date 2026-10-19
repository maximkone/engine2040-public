//! A rules script drives real arena fighters through the controller hook.
//! Needs the `arena` feature: `cargo test -p engine-rules --features arena --test arena_drive`.
#![cfg(feature = "arena")]

use std::cell::RefCell;
use std::rc::Rc;

use engine_arena::controller::Buttons;
use engine_arena::spine_step::{run_spine, run_spine_controlled};
use engine_arena::world::MatchWorld;
use engine_arena::{
    initial_state, run, run_controlled, ArenaConfig, Checkpoint, FighterId, FighterInput,
    FighterTape,
};
use engine_rules::arena::{DriveStats, RulesController};
use engine_rules::Rules;

const BASH: &str = include_str!("../scripts/shield_bash_arena.rhai");
/// The original fixture: effect actions only, no input actions.
const BASH_FX: &str = include_str!("../scripts/shield_bash.rhai");
/// Attack plus block bits, the hold the bash needs.
const ATTACK_BLOCK: u8 = 0x01 | 0x02;

fn config(seed: u64) -> ArenaConfig {
    ArenaConfig {
        seed,
        ticks: 600,
        fighters: 2,
        half_extent_m: 12.0,
        table_version: 1,
    }
}

fn installed() -> Rc<RefCell<Rules>> {
    let mut rules = Rules::default();
    rules.set(BASH, 0).expect("the arena shield bash installs");
    Rc::new(RefCell::new(rules))
}

fn driven(seed: u64, slots: &[u8]) -> (Vec<Checkpoint>, DriveStats) {
    let rules = installed();
    let mut controller = RulesController::new(Rc::clone(&rules), slots);
    let stats = controller.stats();
    let checkpoints = run_controlled(&config(seed), &FighterTape(Vec::new()), &mut controller);
    let stats = stats.borrow().clone();
    (checkpoints, stats)
}

#[test]
fn a_scripted_slot_changes_the_match_and_replays_exactly() {
    let plain = run(&config(2040), &FighterTape(Vec::new()));
    assert_eq!(plain.len(), 600);
    let (first, stats) = driven(2040, &[0]);
    assert_eq!(first.len(), 600);
    assert!(
        stats.inputs_applied > 0,
        "the script drove inputs: {stats:?}"
    );
    assert_ne!(
        first, plain,
        "the script changed what was played: {stats:?}"
    );
    assert_eq!(stats.ticks, 600);
    assert_eq!(stats.faults, 0);
    let (second, again) = driven(2040, &[0]);
    assert_eq!(second, first, "same seed twice, equal checkpoints");
    assert_eq!(again, stats);
    let (other, _) = driven(7, &[0]);
    assert_ne!(other, first, "a different seed is a different match");
}

#[test]
fn an_unscripted_controller_is_the_plain_run() {
    let (none, stats) = driven(2040, &[]);
    assert_eq!(none, run(&config(2040), &FighterTape(Vec::new())));
    assert_eq!(stats.inputs_applied, 0);
}

#[test]
fn memory_is_reset_between_matches() {
    // One shared engine, two matches: the second must not inherit the first's bash cooldown.
    let rules = installed();
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut controller = RulesController::new(Rc::clone(&rules), &[0, 1]);
        runs.push(run_controlled(
            &config(2040),
            &FighterTape(Vec::new()),
            &mut controller,
        ));
    }
    assert_eq!(runs[0], runs[1]);
}

#[test]
fn the_spine_step_runs_the_same_script() {
    let world = MatchWorld::default();
    let plain = run_spine(&config(2040), &FighterTape(Vec::new()), &world);
    let rules = installed();
    let controller = RulesController::new(Rc::clone(&rules), &[0]);
    let spine = run_spine_controlled(
        &config(2040),
        &FighterTape(Vec::new()),
        &world,
        Box::new(controller),
    );
    assert_ne!(spine, plain);
    // On an empty world the spine step and the legacy step agree, scripted or not.
    assert_eq!(spine, driven(2040, &[0]).0);
}

#[test]
fn a_failing_script_call_keeps_the_brain_input_and_never_stops_the_match() {
    // `tick()` divided by zero at tick 50: the call fails, is counted, the match goes on.
    let mut rules = Rules::default();
    rules
        .set("if tick() == 50 { let z = tick() - 50; press(1 / z); }", 0)
        .expect("compiles; the smoke worlds never reach tick 50");
    let mut controller = RulesController::new(Rc::new(RefCell::new(rules)), &[0]);
    let stats = controller.stats();
    let result = run_controlled(&config(2040), &FighterTape(Vec::new()), &mut controller);
    assert_eq!(result, run(&config(2040), &FighterTape(Vec::new())));
    assert_eq!(stats.borrow().faults, 1);
    assert!(stats.borrow().last_error.is_some());
}

#[test]
fn explain_at_a_tick_logs_the_host_calls_without_changing_the_match() {
    let rules = installed();
    let controller = RulesController::new(Rc::clone(&rules), &[0]).explain_at(30);
    let stats = controller.stats();
    let mut controller = controller;
    let with = run_controlled(&config(2040), &FighterTape(Vec::new()), &mut controller);
    assert_eq!(with, driven(2040, &[0]).0);
    let stats = stats.borrow();
    let explanation = stats.explanation.as_ref().expect("explained at tick 30");
    assert_eq!(explanation.tick, 30);
    assert_eq!(explanation.fighters.len(), 1, "only the scripted slot");
    assert!(!explanation.fighters[0].calls.is_empty());
}

/// The blessed arena hashes: the row's second-PC replay fixture, written once
/// by `bless_arena_hashes` and read here.
const HASHES: &str = include_str!("data/arena_drive_hashes.txt");
/// Seeds the fixture holds, in file order.
const HASH_SEEDS: [u64; 2] = [2040, 7];
/// Tick whose checkpoint hash the fixture carries (the row names 599).
const HASH_TICK: u32 = 599;

/// The hash the row fixes: the scripted drive's checkpoint at [`HASH_TICK`],
/// reproducible twice on this machine before it is reported.
fn hashed_drive(seed: u64) -> u64 {
    let first = last_hash(seed);
    let second = last_hash(seed);
    assert_eq!(
        first, second,
        "seed {seed}: the arena drive must be reproducible x2 on this machine"
    );
    first
}

/// One run's checkpoint hash at the row's tick.
fn last_hash(seed: u64) -> u64 {
    let (checkpoints, stats) = driven(seed, &[0]);
    assert_eq!(stats.faults, 0, "the script ran clean: {stats:?}");
    let last = checkpoints.last().expect("600 checkpoints");
    assert_eq!(last.tick, HASH_TICK, "the row names tick {HASH_TICK}");
    last.hash
}

/// One fixture line: `<seed> <tick> <hash>`, lower-case hex, fixed width.
fn hash_line(seed: u64) -> String {
    format!("{seed} {HASH_TICK} {:016x}", hashed_drive(seed))
}

/// Writes the hash fixture once on the blessing machine. Ignored by default:
/// `cargo test -p engine-rules --features arena --test arena_drive
/// bless_arena_hashes -- --ignored`.
#[test]
#[ignore = "writes the hash fixture; run once on the machine that blesses it"]
fn bless_arena_hashes() {
    let mut text = String::from(
        "# arena_drive_hashes: seed tick hash of the scripted arena drive (NS32d).\n\
         # Config: 600 ticks, 2 fighters, half 12 m, table v1, slot 0 runs\n\
         # scripts/shield_bash_arena.rhai; the hash is the tick 599 checkpoint.\n\
         # Written by: cargo test -p engine-rules --features arena --test arena_drive \\\n\
         #   bless_arena_hashes -- --ignored\n\
         # A hash that differs on another machine is a finding for docs/BLOCKERS.md, never a\n\
         # loosened test: floats enter the arena hash.\n",
    );
    for seed in HASH_SEEDS {
        text.push_str(&hash_line(seed));
        text.push('\n');
    }
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/arena_drive_hashes.txt");
    std::fs::write(&path, text).expect("the fixture path is in the tree");
    println!(
        "blessed {} line(s) into {}",
        HASH_SEEDS.len(),
        path.display()
    );
}

/// The second-PC replay check: the fixture's hashes must come back on this
/// machine, and every run must repeat itself. A mismatch is the row's finding
/// (floats enter the arena hash), written to `docs/BLOCKERS.md`, not a loosened
/// bar.
#[test]
fn arena_drive_hashes() {
    let wanted: Vec<&str> = HASHES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    assert_eq!(
        wanted.len(),
        HASH_SEEDS.len(),
        "the fixture carries one line per seed"
    );
    for (index, seed) in HASH_SEEDS.iter().enumerate() {
        let got = hash_line(*seed);
        assert_eq!(
            got, wanted[index],
            "seed {seed}: the arena hash differs from the blessed fixture; a mismatch across \
             machines is a finding for docs/BLOCKERS.md, never a loosened test"
        );
        println!("{got}");
    }
    println!("arena_drive_hashes: {} line(s) equal", HASH_SEEDS.len());
}

/// Tape that walks slot 0 from its spawn straight at slot 1's spawn, holding
/// attack plus block every tick, while slot 1 stands still. Movement is
/// world-planar in wishes, so the aim comes from the spawn positions. The
/// small arena keeps the walk inside 600 ticks (about 33 mm per tick).
fn approach_tape() -> (ArenaConfig, FighterTape) {
    let cfg = ArenaConfig {
        seed: 2040,
        ticks: 600,
        fighters: 2,
        half_extent_m: 5.0,
        table_version: 1,
    };
    let spawns = initial_state(2040, 2, 5.0).expect("valid spawn");
    let dx = spawns[1].pos[0] - spawns[0].pos[0];
    let dz = spawns[1].pos[2] - spawns[0].pos[2];
    let len = (dx * dx + dz * dz).sqrt();
    let (mx, mz) = if len > 0.0 {
        (
            (dx * 100.0 / len).round().clamp(-100.0, 100.0) as i8,
            (dz * 100.0 / len).round().clamp(-100.0, 100.0) as i8,
        )
    } else {
        (100, 0)
    };
    let mut inputs = Vec::with_capacity(2 * 600);
    for tick in 0..600 {
        inputs.push(FighterInput {
            tick,
            fighter: FighterId(0),
            mx,
            mz,
            facing: 0,
            buttons: Buttons(ATTACK_BLOCK),
        });
        inputs.push(FighterInput {
            tick,
            fighter: FighterId(1),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        });
    }
    (cfg, FighterTape(inputs))
}

#[test]
fn effects_apply_in_the_real_arena() {
    // Every applied effect lands: damage, heal, stun and drain change the match
    // with no input action and no failure, and the same seed replays exactly.
    let mut rules = Rules::default();
    rules
        .set(
            "if id == 0 { damage(1, 100); heal(0, 50); stun(1, 10); drain_stamina(1, 20); }",
            0,
        )
        .expect("effect script installs");
    let mut controller = RulesController::new(Rc::new(RefCell::new(rules)), &[0]);
    let stats = controller.stats();
    let plain = run(&config(2040), &FighterTape(Vec::new()));
    let first = run_controlled(&config(2040), &FighterTape(Vec::new()), &mut controller);
    assert_eq!(first.len(), 600);
    assert_ne!(first, plain, "script effects changed the match");
    {
        let stats = stats.borrow();
        assert_eq!(stats.inputs_applied, 0, "no input action emitted");
        assert!(stats.effects_applied > 0, "effects applied: {stats:?}");
        assert_eq!(stats.faults, 0, "{stats:?}");
    }
    let mut rules = Rules::default();
    rules
        .set(
            "if id == 0 { damage(1, 100); heal(0, 50); stun(1, 10); drain_stamina(1, 20); }",
            0,
        )
        .expect("effect script installs");
    let mut again = RulesController::new(Rc::new(RefCell::new(rules)), &[0]);
    assert_eq!(
        run_controlled(&config(2040), &FighterTape(Vec::new()), &mut again),
        first,
        "same seed twice, equal checkpoints"
    );

    // Push and shield stay ignored: a script that only shoves and shields plays
    // the exact plain match.
    let mut rules = Rules::default();
    rules
        .set(
            "if id == 0 { let foe = nearest_foe(id); if foe >= 0 { push(foe, 500, 0); shield(id, 30); } }",
            0,
        )
        .expect("push/shield script installs");
    let mut controller = RulesController::new(Rc::new(RefCell::new(rules)), &[0]);
    let stats = controller.stats();
    let ignored = run_controlled(&config(2040), &FighterTape(Vec::new()), &mut controller);
    assert_eq!(ignored, plain, "push and shield change nothing");
    assert!(stats.borrow().effects_ignored > 0, "{:?}", stats.borrow());
    assert_eq!(stats.borrow().effects_applied, 0);

    // The original fixture changes the real match: slot 0 walks into reach
    // holding block and attack, then the bash stuns and wounds slot 1.
    let mut rules = Rules::default();
    rules.set(BASH_FX, 0).expect("the fixture installs");
    let tape = approach_tape();
    let taped = run(&tape.0, &tape.1);
    assert_eq!(taped.len(), 600);
    let mut controller = RulesController::new(Rc::new(RefCell::new(rules)), &[0]);
    let stats = controller.stats();
    let bashed = run_controlled(&tape.0, &tape.1, &mut controller);
    assert_eq!(bashed.len(), 600);
    assert_ne!(bashed, taped, "the fixture changed the match");
    {
        let stats = stats.borrow();
        assert_eq!(stats.inputs_applied, 0, "the fixture emits no input action");
        assert!(stats.effects_applied > 0, "the bash applied: {stats:?}");
        assert!(
            stats.effects_ignored > 0,
            "push and shield ignored: {stats:?}"
        );
        assert_eq!(stats.faults, 0, "{stats:?}");
    }
    let mut rules = Rules::default();
    rules.set(BASH_FX, 0).expect("the fixture installs");
    let mut again = RulesController::new(Rc::new(RefCell::new(rules)), &[0]);
    assert_eq!(
        run_controlled(&tape.0, &tape.1, &mut again),
        bashed,
        "the bashed match replays exactly"
    );
}
