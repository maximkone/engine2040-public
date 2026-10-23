//! audit: the thinking-LOD and perception-clock proofs for [`Legion`].
//!
//! The tape is one seeded 500-soldier battle line: anchors drift, squads
//! change orders, the formation and spacing change, and every body walks
//! toward its decided slot. It proves four things:
//! 1. the same seed replays a byte-identical state and output trace twice,
//!    and a second seed changes it (the gate can say no);
//! 2. [`ThinkingLod::On`] and [`ThinkingLod::Off`] agree on every tick's
//!    digest and output, so the LOD never changes the outcome;
//! 3. every squad perceives within its documented interval (1, 2, 4 or 8
//!    ticks from its nearest soldier, capped by the perception clock);
//! 4. no squad under a moving order sits still off its slots for
//!    [`STUCK_WINDOW_TICKS`] ticks, over a named seed set.
//!
//! Everything is integer math over the same public [`Legion`] API the
//! arena uses. The audit reads the private clock fields through the crate,
//! so it never needs a second copy of the policy.

use super::*;
use engine_core::{CounterCursor, Fnv64};

/// Ticks one audit tape runs.
const TAPE_TICKS: u32 = 600;
/// Soldiers seated on the tape: the 500-soldier contract size.
const TAPE_SOLDIERS: usize = CONTRACT_SOLDIERS;
/// Grid edge the tape bodies stand on.
const TAPE_GRID: (usize, usize) = (1024, 1024);
/// Replay seed; the same seed must give the same trace twice.
const TAPE_SEED: u64 = 2040;
/// Second seed; its trace must differ from the replay seed's.
const TAPE_SEED_OTHER: u64 = 2041;
/// Seeds of the stuck drill.
const STUCK_SEEDS: [u64; 4] = [2040, 2041, 2042, 2043];
/// Ticks a moving-order squad may sit off its slots without any progress
/// before the drill calls it stuck.
const STUCK_WINDOW_TICKS: usize = 16;
/// Ticks between anchor drifts on the tape.
const ANCHOR_DRIFT_TICKS: u32 = 10;
/// Ticks between enemy-anchor drifts on the tape.
const ENEMY_DRIFT_TICKS: u32 = 14;
/// Ticks between squad order changes on the tape.
const ORDER_CHANGE_TICKS: u32 = 50;
/// Ticks between formation changes on the tape.
const FORMATION_CHANGE_TICKS: u32 = 150;
/// Ticks between spacing changes on the tape.
const SPACING_CHANGE_TICKS: u32 = 200;

/// One tape run: per-tick state digests plus output hashes, the final
/// digest and the total think count.
struct TapeRun {
    states: Vec<u64>,
    outputs: Vec<u64>,
    final_digest: u64,
    thinks: usize,
}

impl TapeRun {
    /// Fold the whole per-tick trace into one `u64`, so a replay compares
    /// every tick while printing one number.
    fn trace(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_len(self.states.len());
        for digest in &self.states {
            hash.write_u64(*digest);
        }
        for output in &self.outputs {
            hash.write_u64(*output);
        }
        hash.finish()
    }
}

/// Run the seeded tape with `lod`, observing the legion after every step.
fn run_tape_observed(
    seed: u64,
    lod: ThinkingLod,
    mut observe: impl FnMut(&Legion, u32, usize),
) -> TapeRun {
    let mut drift = CounterCursor::new(seed);
    let mut legion = Legion::new(
        TAPE_SOLDIERS,
        Formation::Column,
        (256, 512),
        (320, 512),
        TAPE_GRID,
    )
    .expect("the tape legion seats");
    legion.set_thinking_lod(lod);
    let mut bodies = legion.slots().to_vec();
    let mut slots = vec![(0usize, 0usize); TAPE_SOLDIERS];
    let mut brace = vec![false; TAPE_SOLDIERS];
    let mut states = Vec::with_capacity(TAPE_TICKS as usize);
    let mut outputs = Vec::with_capacity(TAPE_TICKS as usize);
    let mut thinks = 0usize;
    for tick in 0..TAPE_TICKS {
        if tick.is_multiple_of(ANCHOR_DRIFT_TICKS) {
            let anchor = drift_cell(legion.anchor(), &mut drift);
            legion
                .set_anchor(anchor)
                .expect("the tape anchor is on the grid");
        }
        if tick.is_multiple_of(ENEMY_DRIFT_TICKS) {
            let enemy = drift_cell(legion.enemy(), &mut drift);
            legion
                .set_enemy(enemy)
                .expect("the tape enemy is on the grid");
        }
        if tick.is_multiple_of(ORDER_CHANGE_TICKS) {
            let turn = (tick / ORDER_CHANGE_TICKS) as usize;
            for squad in 0..legion.squad_count() {
                let byte = ((squad + turn) % 6) as u8;
                let order = Order::from_u8(byte).expect("the tape cycles drill orders");
                legion
                    .set_order(squad, order)
                    .expect("the tape squad is on the roster");
            }
        }
        if tick.is_multiple_of(FORMATION_CHANGE_TICKS) {
            let byte = ((tick / FORMATION_CHANGE_TICKS) % 4) as u8;
            legion.set_formation(Formation::from_u8(byte).expect("the tape cycles formations"));
        }
        if tick.is_multiple_of(SPACING_CHANGE_TICKS) {
            let spacing = 1 + (tick / SPACING_CHANGE_TICKS) % 4;
            legion
                .set_spacing(spacing)
                .expect("the tape spacing is legal");
        }
        thinks += legion
            .step(tick, &bodies, &mut slots, &mut brace)
            .expect("the tape lane matches the legion");
        for (body, slot) in bodies.iter_mut().zip(&slots) {
            *body = step_cell(*body, *slot);
        }
        states.push(legion.digest());
        let mut output = Fnv64::new();
        for (slot, held) in slots.iter().zip(&brace) {
            output.write_u64(slot.0 as u64);
            output.write_u64(slot.1 as u64);
            output.write_bool(*held);
        }
        outputs.push(output.finish());
        observe(&legion, tick, thinks);
    }
    TapeRun {
        final_digest: legion.digest(),
        states,
        outputs,
        thinks,
    }
}

/// Run the tape with no observer.
fn run_tape(seed: u64, lod: ThinkingLod) -> TapeRun {
    run_tape_observed(seed, lod, |_, _, _| {})
}

/// Move one cell on each axis toward `to` (the arena's close-up shape).
fn step_cell(from: (usize, usize), to: (usize, usize)) -> (usize, usize) {
    let axis = |a: usize, b: usize| {
        if a < b {
            a + 1
        } else if a > b {
            a - 1
        } else {
            a
        }
    };
    (axis(from.0, to.0), axis(from.1, to.1))
}

/// Drift a cell by -1, 0 or +1 on each axis, clamped to a safe band so the
/// formation offsets around it stay on the grid.
fn drift_cell(cell: (usize, usize), cursor: &mut CounterCursor) -> (usize, usize) {
    let axis = |value: usize, cursor: &mut CounterCursor| {
        let delta = cursor.next_range(0, 3) as i32 - 1;
        (value as i32 + delta).clamp(128, 896) as usize
    };
    (axis(cell.0, cursor), axis(cell.1, cursor))
}

#[test]
fn tape_replays_byte_identical_and_answers_a_second_seed() {
    let first = run_tape(TAPE_SEED, ThinkingLod::On);
    let second = run_tape(TAPE_SEED, ThinkingLod::On);
    assert_eq!(
        first.trace(),
        second.trace(),
        "the same seed changed the state or output trace"
    );
    assert_eq!(first.final_digest, second.final_digest);
    assert_eq!(first.thinks, second.thinks);
    assert_eq!(first.states, second.states);
    let other = run_tape(TAPE_SEED_OTHER, ThinkingLod::On);
    assert_ne!(
        first.trace(),
        other.trace(),
        "the second seed replayed the first seed's trace"
    );
    assert_ne!(
        first.final_digest, other.final_digest,
        "the second seed ended in the first seed's state"
    );
    eprintln!(
        "tape seed {TAPE_SEED}: final {:016x}, trace {:016x}, {} thinks; \
         seed {TAPE_SEED_OTHER}: final {:016x}, trace {:016x} (differs)",
        first.final_digest,
        first.trace(),
        first.thinks,
        other.final_digest,
        other.trace()
    );
}

#[test]
fn thinking_lod_never_changes_the_outcome() {
    let on = run_tape(TAPE_SEED, ThinkingLod::On);
    let off = run_tape(TAPE_SEED, ThinkingLod::Off);
    assert_eq!(on.states.len(), off.states.len());
    for tick in 0..on.states.len() {
        assert_eq!(
            on.states[tick], off.states[tick],
            "state differs at tick {tick} with the perception clock on"
        );
        assert_eq!(
            on.outputs[tick], off.outputs[tick],
            "slots or braces differ at tick {tick} with the perception clock on"
        );
    }
    assert_eq!(on.final_digest, off.final_digest);
    assert!(
        on.thinks < off.thinks,
        "the LOD saved no decides: {} on vs {} off",
        on.thinks,
        off.thinks
    );
    eprintln!(
        "tape LOD: {} thinks with the clock, {} at full rate ({:.1}x fewer)",
        on.thinks,
        off.thinks,
        off.thinks as f64 / on.thinks as f64
    );
}

#[test]
fn perception_clock_bounds_every_squad_on_the_tape() {
    let squads = TAPE_SOLDIERS.div_ceil(SQUAD_SIZE);
    let mut decided = vec![0usize; squads];
    let mut ticks_of: Vec<Vec<u32>> = vec![Vec::new(); squads];
    let mut intervals = vec![0u32; squads];
    let run = run_tape_observed(TAPE_SEED, ThinkingLod::On, |legion, tick, _| {
        for (squad, count) in decided.iter_mut().enumerate() {
            let lag = legion
                .perception_lag(tick, squad)
                .expect("the tape squad is on the roster");
            let interval = legion
                .perception_interval(squad)
                .expect("the tape squad is on the roster");
            intervals[squad] = interval;
            assert!(
                (1..=PERCEPTION_INTERVAL_TICKS).contains(&interval),
                "squad {squad} at tick {tick} has interval {interval}"
            );
            assert!(
                lag <= interval,
                "squad {squad} at tick {tick}: lag {lag} past its {interval}-tick interval"
            );
            if lag == 0 {
                *count += 1;
                ticks_of[squad].push(tick);
            }
        }
        assert!(
            legion.max_perception_lag(tick) <= PERCEPTION_INTERVAL_TICKS,
            "a squad at tick {tick} went past the {} tick clock",
            PERCEPTION_INTERVAL_TICKS
        );
    });
    assert_eq!(run.states.len(), TAPE_TICKS as usize);
    for (squad, count) in decided.iter().enumerate() {
        assert!(
            *count >= (TAPE_TICKS / PERCEPTION_INTERVAL_TICKS - 1) as usize,
            "squad {squad} perceived {count} times in {TAPE_TICKS} ticks"
        );
        assert!(
            ticks_of[squad]
                .windows(2)
                .all(|pair| pair[1] - pair[0] <= PERCEPTION_INTERVAL_TICKS),
            "squad {squad} skipped its perception wave: {:?}",
            &ticks_of[squad][..ticks_of[squad].len().min(4)]
        );
    }
    // Name a near, a middle and a far squad: what it perceives and when.
    for squad in [0usize, squads / 2, squads - 1] {
        eprintln!(
            "squad {squad}: interval {} ticks, decided {} times, first {:?}",
            intervals[squad],
            decided[squad],
            &ticks_of[squad][..ticks_of[squad].len().min(5)]
        );
    }
}

#[test]
fn static_squads_perceive_on_their_documented_wave() {
    let grid = (400usize, 400usize);
    let mut legion = Legion::new(
        CONTRACT_SOLDIERS,
        Formation::Column,
        (20, 20),
        (380, 380),
        grid,
    )
    .expect("the static legion seats");
    let soldiers = [(20usize, 20usize); CONTRACT_SOLDIERS];
    let mut slots = [(0usize, 0usize); CONTRACT_SOLDIERS];
    let mut brace = [false; CONTRACT_SOLDIERS];
    let mut wave = Vec::new();
    for tick in 0..17u32 {
        wave.push(
            legion
                .step(tick, &soldiers, &mut slots, &mut brace)
                .expect("the static lane matches"),
        );
        for squad in [0usize, 31, 62] {
            assert_eq!(
                legion.perception_interval(squad).expect("squad on roster"),
                8
            );
            assert!(
                legion.perception_lag(tick, squad).expect("squad on roster") < 8,
                "a static far squad went past its wave at tick {tick}"
            );
        }
    }
    // What the clock does and when: no step decide until tick 8, then one
    // leader per squad on each 8-tick wave.
    assert_eq!(wave[..8].iter().sum::<usize>(), 0);
    assert_eq!(wave[8], 63);
    assert_eq!(wave[9..16].iter().sum::<usize>(), 0);
    assert_eq!(wave[16], 63);
    // A shared input change re-decides the whole squad at once, not just
    // its leader: the clock never holds a stale anchor.
    legion
        .set_anchor((24, 20))
        .expect("the moved anchor is on the grid");
    let thinks = legion
        .step(17, &soldiers, &mut slots, &mut brace)
        .expect("the static lane matches");
    assert_eq!(thinks, CONTRACT_SOLDIERS);
    // A near squad's documented interval is one tick: its first perception
    // after the opening decision is tick 1, and every soldier joins it.
    let mut near = Legion::new(8, Formation::Line, (20, 10), (22, 12), (40, 20))
        .expect("the near legion seats");
    assert_eq!(near.perception_interval(0).expect("squad on roster"), 1);
    let near_lane = [(20usize, 10usize); 8];
    let mut near_slots = [(0usize, 0usize); 8];
    let mut near_brace = [false; 8];
    assert_eq!(
        near.step(1, &near_lane, &mut near_slots, &mut near_brace)
            .expect("the near lane matches"),
        8
    );
}

/// A moving-order squad that sat off its decided slots for
/// [`STUCK_WINDOW_TICKS`] ticks while neither a soldier nor a slot moved.
/// `move_bodies` false is the detector's negative control: frozen bodies
/// must be flagged, so the drill can say no to a stalled harness.
fn stuck_drill(seed: u64, move_bodies: bool) -> Vec<(usize, u32)> {
    let mut legion = Legion::new(
        TAPE_SOLDIERS,
        Formation::Column,
        (512, 512),
        (512, 128),
        TAPE_GRID,
    )
    .expect("the drill legion seats");
    for squad in 0..legion.squad_count() {
        let byte = 1 + (squad % 5) as u8;
        legion
            .set_order(squad, Order::from_u8(byte).expect("a moving drill order"))
            .expect("the drill squad is on the roster");
    }
    let mut cursor = CounterCursor::new(seed);
    let mut bodies: Vec<(usize, usize)> = (0..TAPE_SOLDIERS)
        .map(|_| {
            let x = 384 + cursor.next_range(0, 257) as usize;
            let y = 384 + cursor.next_range(0, 257) as usize;
            (x, y)
        })
        .collect();
    let mut slots = vec![(0usize, 0usize); TAPE_SOLDIERS];
    let mut brace = vec![false; TAPE_SOLDIERS];
    let mut before_bodies = bodies.clone();
    let mut before_slots = legion.slots().to_vec();
    let mut idle = vec![0usize; legion.squad_count()];
    let mut stuck = Vec::new();
    for tick in 0..TAPE_TICKS {
        legion
            .step(tick, &bodies, &mut slots, &mut brace)
            .expect("the drill lane matches");
        if move_bodies {
            for (body, slot) in bodies.iter_mut().zip(&slots) {
                *body = step_cell(*body, *slot);
            }
        }
        for (squad, idle_ticks) in idle.iter_mut().enumerate() {
            let start = squad * SQUAD_SIZE;
            let end = (start + SQUAD_SIZE).min(TAPE_SOLDIERS);
            let seated = (start..end).all(|i| bodies[i] == slots[i]);
            let moved = (start..end).any(|i| bodies[i] != before_bodies[i]);
            let slots_moved = (start..end).any(|i| slots[i] != before_slots[i]);
            if !seated && !moved && !slots_moved {
                *idle_ticks += 1;
                if *idle_ticks == STUCK_WINDOW_TICKS {
                    stuck.push((squad, tick));
                }
            } else {
                *idle_ticks = 0;
            }
        }
        before_bodies.copy_from_slice(&bodies);
        before_slots.copy_from_slice(&slots);
    }
    stuck
}

#[test]
fn seeded_drill_leaves_no_squad_stuck() {
    let mut flagged = Vec::new();
    for seed in STUCK_SEEDS {
        let stuck = stuck_drill(seed, true);
        if !stuck.is_empty() {
            flagged.push((seed, stuck));
        }
    }
    assert!(
        flagged.is_empty(),
        "{} seeds left squads stuck: {:?}",
        flagged.len(),
        &flagged[..flagged.len().min(2)]
    );
    // The detector can fire: frozen bodies never leave their scatter, so
    // the first squad that misses its slots for 16 ticks is flagged.
    let frozen = stuck_drill(STUCK_SEEDS[0], false);
    assert!(
        !frozen.is_empty(),
        "the stuck detector never fired on frozen bodies"
    );
    eprintln!(
        "stuck drill: {} seeds x {TAPE_TICKS} ticks, 0 stuck; \
         frozen control flagged {} squads (first {:?})",
        STUCK_SEEDS.len(),
        frozen.len(),
        &frozen[..frozen.len().min(3)]
    );
}
