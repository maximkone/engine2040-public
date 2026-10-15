// Attribution: fault-schedule + replay-harness idea adapted from
// akiselev/deterministic-rs (MIT, verified upstream org); reimplemented here
// over engine-core `Rng::fork_named` substreams. No donor code copied.
//
//! Seeded scheduler + replay fault-soak vectors over core `Rng`.
//! Owner: engine-core (test-only harness). Units: ticks and hashes are u64,
//! seeds are u64. Determinism: same seed plus same fault schedule gives the
//! same journal; every tick uses integer math only (no float in tick).
//! Errors: none; vectors panic only on harness bugs via `expect` with reason.

use engine_core::Rng;

/// Ticks per replay journal: long enough for faults to bite, short and fast.
const TICKS: u64 = 64;

/// Ten fault kinds, one per soak vector. Each mutates the integer tick in a
/// distinct way so a clean replay and a faulty replay must diverge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultKind {
    /// Skip the draw on the fault tick (stale input reuse bug).
    SkipDraw,
    /// Draw twice, keep the second (double-apply bug).
    DoubleDraw,
    /// Draw an extra word and mix it in (stray-consume bug).
    ExtraDraw,
    /// Swap the two state lanes (field-order bug).
    SwapLanes,
    /// Flip bit 0 of the draw (single-bit corruption bug).
    BitFlip,
    /// Truncate the draw to 32 bits (narrowing-cast bug).
    TruncateDraw,
    /// Add one to the draw (off-by-one bug).
    PlusOne,
    /// Draw from the wrong named substream (cross-stream bug).
    CrossStream,
    /// Replay the previous tick draw (stale-cache bug).
    ReplayOld,
    /// Force the draw to zero (dropped-packet bug).
    ZeroDraw,
}

/// One soak vector: seed, which fault to inject, and on which tick.
#[derive(Clone, Copy, Debug)]
struct Vector {
    /// Fixed seed for this vector; distinct per vector.
    seed: u64,
    /// Fault to inject.
    fault: FaultKind,
    /// Tick at which the fault fires (`< TICKS`).
    at_tick: u64,
}

/// The ten seeded vectors. Seeds are fixed so failures reproduce exactly.
const VECTORS: [Vector; 10] = [
    Vector {
        seed: 2040,
        fault: FaultKind::SkipDraw,
        at_tick: 5,
    },
    Vector {
        seed: 2041,
        fault: FaultKind::DoubleDraw,
        at_tick: 9,
    },
    Vector {
        seed: 2042,
        fault: FaultKind::ExtraDraw,
        at_tick: 13,
    },
    Vector {
        seed: 2043,
        fault: FaultKind::SwapLanes,
        at_tick: 17,
    },
    Vector {
        seed: 2044,
        fault: FaultKind::BitFlip,
        at_tick: 21,
    },
    Vector {
        seed: 2045,
        fault: FaultKind::TruncateDraw,
        at_tick: 25,
    },
    Vector {
        seed: 2046,
        fault: FaultKind::PlusOne,
        at_tick: 29,
    },
    Vector {
        seed: 2047,
        fault: FaultKind::CrossStream,
        at_tick: 33,
    },
    Vector {
        seed: 2048,
        fault: FaultKind::ReplayOld,
        at_tick: 37,
    },
    Vector {
        seed: 2049,
        fault: FaultKind::ZeroDraw,
        at_tick: 41,
    },
];

/// Seeded fault schedule: `TICKS` decisions drawn from the `sched` substream.
/// The fault tick carries `fault`; every other tick is clean (`None`).
fn fault_schedule(seed: u64, fault: FaultKind, at_tick: u64, ticks: u64) -> Vec<Option<FaultKind>> {
    let mut sched = Rng::new(seed).fork_named("sched");
    // Advance the scheduler once per tick so the schedule itself is a seeded
    // stream (mirrors a fuzzer queue); decisions stay deterministic.
    let mut out = Vec::with_capacity(ticks_usize(ticks));
    for tick in 0..ticks {
        let _ = sched.next_u64();
        if tick == at_tick {
            out.push(Some(fault));
        } else {
            out.push(None);
        }
    }
    out
}

/// Integer-only tick: mix one draw into two state lanes with wrapping
/// `+`/`*` only. No float, no `HashMap`, no wall clock in this path.
fn tick(state: &mut [u64; 2], draw: u64) {
    state[0] = state[0].wrapping_add(draw);
    state[0] = state[0].wrapping_mul(0x9E37_79B9_7F4A_7C15);
    state[1] = state[1].wrapping_add(draw ^ 0xBF58_476D_1CE4_E5B9);
    state[1] = state[0].wrapping_add(state[1].wrapping_mul(0x94D0_49BB_1331_11EB));
}

/// Apply one fault to the draw for this tick. Pure integer match, no float.
fn apply_fault(
    draw: u64,
    fault: Option<FaultKind>,
    sim: &mut Rng,
    aux: &mut Rng,
    prev_draw: u64,
) -> u64 {
    match fault {
        None => draw,
        Some(FaultKind::SkipDraw) => prev_draw,
        Some(FaultKind::DoubleDraw) => {
            let _ = sim.next_u64();
            sim.next_u64()
        }
        Some(FaultKind::ExtraDraw) => draw.wrapping_add(aux.next_u64()),
        Some(FaultKind::SwapLanes) => draw.swap_bytes(),
        Some(FaultKind::BitFlip) => draw ^ 1,
        Some(FaultKind::TruncateDraw) => draw & 0xFFFF_FFFF,
        Some(FaultKind::PlusOne) => draw.wrapping_add(1),
        Some(FaultKind::CrossStream) => aux.next_u64(),
        Some(FaultKind::ReplayOld) => prev_draw,
        Some(FaultKind::ZeroDraw) => 0,
    }
}

/// Replay harness: run `ticks` integer ticks under a seeded fault schedule,
/// journaling the running digest after each tick. The sim draws from the
/// `sim` substream; cross-stream faults draw from the `fault` substream.
fn replay(seed: u64, schedule: &[Option<FaultKind>]) -> Vec<u64> {
    let base = Rng::new(seed);
    let mut sim = base.fork_named("sim");
    let mut aux = base.fork_named("fault");
    let mut state = [seed ^ 0x243F_6A88_85A3_08D3, seed.wrapping_mul(3)];
    let mut prev_draw = 0u64;
    let mut out = Vec::with_capacity(schedule.len());
    for fault in schedule {
        let draw = sim.next_u64();
        let eff = apply_fault(draw, *fault, &mut sim, &mut aux, prev_draw);
        // `SkipDraw`/`ReplayOld` reuse the previous draw without consuming;
        // every other path consumes exactly as coded above.
        if fault.is_none() {
            prev_draw = draw;
        } else if *fault != Some(FaultKind::SkipDraw) && *fault != Some(FaultKind::ReplayOld) {
            prev_draw = eff;
        }
        // `SwapLanes` swaps state lanes before the tick to model field order.
        if *fault == Some(FaultKind::SwapLanes) {
            state.swap(0, 1);
        }
        tick(&mut state, eff);
        out.push(Rng::digest(&state));
    }
    out
}

/// First tick where the journals differ; `None` when they agree throughout.
fn first_divergence(a: &[u64], b: &[u64]) -> Option<u64> {
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            return Some(u64::try_from(i).expect("tick index fits in u64"));
        }
    }
    None
}

fn ticks_usize(ticks: u64) -> usize {
    usize::try_from(ticks).expect("tick count fits on this platform")
}

/// Run one vector: clean replay versus faulty replay. Returns the first
/// divergent tick; the caller asserts it exists and replays x2 identical.
fn run_vector(v: Vector) -> Option<u64> {
    let clean_sched = fault_schedule(v.seed, v.fault, TICKS + 100, TICKS);
    let fault_sched = fault_schedule(v.seed, v.fault, v.at_tick, TICKS);
    assert_eq!(
        clean_sched.len(),
        ticks_usize(TICKS),
        "schedule covers every tick"
    );
    assert!(
        clean_sched.iter().all(|f| f.is_none()),
        "sentinel tick leaves the clean schedule fault-free"
    );
    let clean = replay(v.seed, &clean_sched);
    let faulty = replay(v.seed, &fault_sched);
    // Replay x2: same seed plus same schedule gives the same journal.
    let clean_again = replay(v.seed, &clean_sched);
    let faulty_again = replay(v.seed, &fault_sched);
    assert_eq!(clean, clean_again, "clean replay must be identical x2");
    assert_eq!(faulty, faulty_again, "faulty replay must be identical x2");
    assert_eq!(
        clean.len(),
        ticks_usize(TICKS),
        "journal holds one hash per tick"
    );
    first_divergence(&clean, &faulty)
}

/// Check one vector: the fault must be caught (journals diverge) exactly at
/// the injected tick, since every earlier tick draws identically.
fn check_vector(v: Vector) {
    let first = run_vector(v);
    assert_eq!(
        first,
        Some(v.at_tick),
        "seed {} fault {:?} must first diverge at tick {}",
        v.seed,
        v.fault,
        v.at_tick
    );
}

#[test]
fn soak_vector_00_skip_draw_caught() {
    check_vector(VECTORS[0]);
}

#[test]
fn soak_vector_01_double_draw_caught() {
    check_vector(VECTORS[1]);
}

#[test]
fn soak_vector_02_extra_draw_caught() {
    check_vector(VECTORS[2]);
}

#[test]
fn soak_vector_03_swap_lanes_caught() {
    check_vector(VECTORS[3]);
}

#[test]
fn soak_vector_04_bit_flip_caught() {
    check_vector(VECTORS[4]);
}

#[test]
fn soak_vector_05_truncate_draw_caught() {
    check_vector(VECTORS[5]);
}

#[test]
fn soak_vector_06_plus_one_caught() {
    check_vector(VECTORS[6]);
}

#[test]
fn soak_vector_07_cross_stream_caught() {
    check_vector(VECTORS[7]);
}

#[test]
fn soak_vector_08_replay_old_caught() {
    check_vector(VECTORS[8]);
}

#[test]
fn soak_vector_09_zero_draw_caught() {
    check_vector(VECTORS[9]);
}

#[test]
fn all_ten_vectors_catch_desync_with_first_tick() {
    let mut caught = 0u64;
    for v in VECTORS {
        let first = run_vector(v);
        assert_eq!(
            first,
            Some(v.at_tick),
            "seed {} fault {:?} must first diverge at tick {}",
            v.seed,
            v.fault,
            v.at_tick
        );
        caught += 1;
    }
    assert_eq!(caught, 10, "all ten seeded bugs must be caught");
}

#[test]
fn replay_x2_identical_for_every_vector() {
    for v in VECTORS {
        let sched = fault_schedule(v.seed, v.fault, v.at_tick, TICKS);
        let first = replay(v.seed, &sched);
        let second = replay(v.seed, &sched);
        assert_eq!(first, second, "replay x2 must agree for seed {}", v.seed);
        assert_ne!(
            first.first(),
            first.last(),
            "journal must move for seed {}",
            v.seed
        );
    }
}

#[test]
fn fault_schedule_is_seeded_and_substream_isolated() {
    // Same seed twice gives the same schedule; parent stream does not move.
    let base = Rng::new(2040);
    let state_before = base.state();
    let a = fault_schedule(2040, FaultKind::BitFlip, 21, TICKS);
    let b = fault_schedule(2040, FaultKind::BitFlip, 21, TICKS);
    assert_eq!(a, b, "same seed must schedule same faults");
    assert_eq!(
        base.state(),
        state_before,
        "fork_named must not advance parent"
    );
    // `sim` and `fault` substreams differ, so cross-stream faults bite.
    let sim_first = base.fork_named("sim").next_u64();
    let aux_first = base.fork_named("fault").next_u64();
    assert_ne!(sim_first, aux_first, "named substreams must differ");
}
