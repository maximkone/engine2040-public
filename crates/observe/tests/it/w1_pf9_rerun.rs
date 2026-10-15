// Seeded paired-rerun vectors over the owned engine-observe rerun sink.
//
// Consumer: the headless paired-rerun harness reads two 120-frame sinks and
// names the first divergent tick. This file proves the accept with
// workspace-shared vectors (no donor bytes, no new external dependency):
// the sim stream is the shared engine-core SplitMix64 chain, the desync is
// one flipped bit at a fixed tick.
use engine_core::hash::{splitmix64, GOLDEN_GAMMA};
use engine_observe::rerun_sink::{RerunSink, MAX_RERUN_FRAMES};

/// Record `frames` ticks from `seed` into a sink, flipping one hash bit at
/// `diverge_at` when set to simulate a seeded desync.
fn record(seed: u64, frames: u64, diverge_at: Option<u64>) -> RerunSink {
    let mut sink = RerunSink::new();
    let mut state = seed;
    for tick in 0..frames {
        let mut hash = splitmix64(state);
        state = state.wrapping_add(GOLDEN_GAMMA);
        if diverge_at == Some(tick) {
            hash ^= 0x1;
        }
        sink.push(tick, hash);
    }
    sink
}

#[test]
fn w1_pf9_seeded_desync_names_first_divergent_tick() {
    let baseline = record(2040, MAX_RERUN_FRAMES as u64, None);
    let desynced = record(2040, MAX_RERUN_FRAMES as u64, Some(57));
    assert_eq!(baseline.len(), MAX_RERUN_FRAMES);
    assert_eq!(desynced.len(), MAX_RERUN_FRAMES);
    // The accept: the recording names the first divergent tick.
    assert_eq!(baseline.first_divergent(&desynced), Some(57));
    assert_eq!(desynced.first_divergent(&baseline), Some(57));
    // The prefix before the desync is identical.
    let prefix: Vec<u64> = baseline
        .iter()
        .zip(desynced.iter())
        .take(57)
        .map(|(a, b)| a.state_hash ^ b.state_hash)
        .collect();
    assert!(prefix.iter().all(|&xor| xor == 0));
}

#[test]
fn w1_pf9_identical_seeds_show_no_divergence() {
    let first = record(2040, MAX_RERUN_FRAMES as u64, None);
    let second = record(2040, MAX_RERUN_FRAMES as u64, None);
    assert_eq!(first.first_divergent(&second), None);
    assert_eq!(second.first_divergent(&first), None);
    // A different seed diverges at the first tick instead.
    let other_seed = record(99, MAX_RERUN_FRAMES as u64, None);
    assert_eq!(first.first_divergent(&other_seed), Some(0));
}

#[test]
fn w1_pf9_ring_holds_120_frames() {
    assert_eq!(MAX_RERUN_FRAMES, 120);
    let mut sink = record(2040, MAX_RERUN_FRAMES as u64, None);
    assert_eq!(sink.len(), 120);
    assert_eq!(sink.overflow_count(), 0);
    sink.push(120, 120);
    sink.push(121, 121);
    assert_eq!(sink.len(), 120);
    assert_eq!(sink.overflow_count(), 2);
    let ticks: Vec<u64> = sink.iter().map(|frame| frame.tick).collect();
    assert_eq!(ticks[0], 2);
    assert_eq!(ticks[119], 121);
}

#[test]
fn w1_pf9_empty_sinks_have_no_divergence() {
    let left = RerunSink::new();
    let right = RerunSink::new();
    assert!(left.is_empty());
    assert_eq!(left.first_divergent(&right), None);
    let partial = record(2040, 10, None);
    assert_eq!(left.first_divergent(&partial), Some(0));
}
