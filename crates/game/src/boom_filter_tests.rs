//! Tests of `boom_filter`: the pull-in is faster than the release, a two-tick graze
//! moves no eye, the held point never moves back out while occluded, and the same
//! frames twice give the same length.

use super::*;

/// One 60 Hz frame.
const TICK: f32 = 1.0 / 60.0;
/// The boom the shots ask for in these tests, meters.
const ASK: f32 = 6.0;
/// The length a pillar leaves of it, meters.
const PILLAR: f32 = 2.0;

/// `n` frames of a steady run at `raw`, the length after the last one.
fn run(filter: &mut BoomFilter, n: u32, raw: f32) -> f32 {
    let mut len = filter.len_m;
    for _ in 0..n {
        len = filter.step(TICK, ASK, raw);
    }
    len
}

#[test]
fn boom_pulls_in_faster_than_it_releases() {
    let mut filter = BoomFilter::default();
    assert_eq!(run(&mut filter, 20, ASK), ASK, "a clear boom is the ask");
    // A pillar takes 4 m of it: no eye moves until the occlusion has lasted the hold.
    assert_eq!(
        run(&mut filter, OCCLUSION_HOLD_TICKS - 1, PILLAR),
        ASK,
        "the hold lets no eye move yet"
    );
    let pulled = run(&mut filter, 13, PILLAR);
    assert!(
        pulled <= PILLAR + 0.1 * (ASK - PILLAR) + 1.0e-4,
        "90 percent of the pull-in inside 12 ticks past the hold: {pulled} m"
    );
    assert!(filter.occluded_s > 0.0, "the filter knows it is occluded");
    // The pillar clears: the release is slower, 90 percent inside 24 ticks.
    let released = run(&mut filter, 24, ASK);
    assert!(
        released >= ASK - 0.1 * (ASK - pulled) - 1.0e-4,
        "90 percent of the release inside 24 ticks: {released} m from {pulled} m"
    );
    assert!(released < ASK, "the release eases, it does not snap");
    assert_eq!(filter.occluded_s, 0.0, "a clear boom ends the occlusion");
}

#[test]
fn short_graze_moves_no_eye() {
    let mut filter = BoomFilter::default();
    let held = run(&mut filter, 30, ASK);
    assert_eq!(held, ASK, "the clear boom settled first");
    // Two occluded ticks, then clear again: a graze is not an occlusion.
    assert_eq!(run(&mut filter, 2, PILLAR), held, "the graze moves no eye");
    assert_eq!(
        run(&mut filter, 30, ASK),
        held,
        "and nothing moves after it"
    );
}

#[test]
fn the_held_point_never_moves_back_out_while_occluded() {
    let mut filter = BoomFilter::default();
    run(&mut filter, 10, ASK);
    // The pillar stays and a body walks behind it: the raw length swings 2.0..2.6 m.
    let mut last = run(&mut filter, OCCLUSION_HOLD_TICKS + 1, PILLAR);
    for raw in [2.6f32, 2.0, 2.4, 2.1, 2.6, 2.3] {
        let len = run(&mut filter, 20, raw);
        assert!(
            len <= last + 1.0e-6,
            "the eye never moves back out through the wall: {len} m from {last} m"
        );
        last = len;
    }
    assert!(
        last < PILLAR + 0.5,
        "and it holds the nearest point: {last} m"
    );
}

#[test]
fn the_same_frames_twice_give_the_same_length() {
    let frames = [
        (ASK, ASK),
        (ASK, PILLAR),
        (ASK, PILLAR),
        (ASK, ASK),
        (ASK, 4.0),
        (ASK, ASK),
    ];
    let run_script = || {
        let mut filter = BoomFilter::default();
        let mut lens = Vec::new();
        for _ in 0..8 {
            for (ask, raw) in frames {
                lens.push(filter.step(TICK, ask, raw));
            }
        }
        lens
    };
    assert_eq!(run_script(), run_script(), "same frames, same length x2");
}

#[test]
fn a_bad_frame_time_or_length_is_floored_not_propagated() {
    let mut filter = BoomFilter::default();
    let first = filter.step(f32::NAN, ASK, ASK);
    assert_eq!(first, ASK, "a bad dt takes one tick");
    assert!(first.is_finite());
    let bad = filter.step(TICK, f32::NAN, PILLAR);
    assert_eq!(bad, first, "a bad ask keeps the length the eye had");
    let over = filter.step(TICK, ASK, ASK + 9.0);
    assert_eq!(over, ASK, "a sweep longer than the boom floors at the boom");
}
