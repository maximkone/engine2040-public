//! warp_ground ground-feel accept sweep: foot slide under 1 cm from 0.5 to
//! 7 m/s through the full locomotion + warp + foot-lock chain, strafes to
//! 135 degrees without skating, typed failures, replay x2 and zero heap.
#![cfg(test)]

use crate::clip::TICK_DT;
use crate::contract::{LocalPose, RootDelta};
use crate::feet::{
    feet_hash, planted_from_contacts, slide_mm, FootLocker, MM_PER_M, SLIDE_BUDGET_MM_PIVOT,
};
use crate::locomotion::{
    LocoOutput, LocoState, Locomotion, LOCO_LEG_JOINTS as LEGS, MAX_WISH_SPEED,
};
use crate::warp_ground::{
    speed_rate, stride_for, GroundWarp, WarpError, WarpReceipt, MAX_WARP_SPEED_M_S,
    MIN_WARP_SPEED_M_S, ORIENT_STEP_CRUISE_RAD, ORIENT_STEP_PIVOT_RAD, RATE_MAX, SLIDE_BUDGET_MM,
    STRAFE_WARP_MAX_RAD, STRIDE_MAX,
};
use core::f32::consts::FRAC_1_SQRT_2 as SQ;
use engine_core::alloc_probe::measure;

/// Matched clip per capsule speed, the way motion matching picks a clip near
/// the speed and the warp covers the residual: walk 1.5, run 4.0, sprint
/// 6.5 m/s.
fn clip_for(speed: f32) -> f32 {
    let clips = [1.5f32, 4.0, 6.5];
    let mut best = clips[0];
    for candidate in clips.iter().skip(1) {
        if (speed - *candidate).abs() < (speed - best).abs() {
            best = *candidate;
        }
    }
    best
}

/// Capsule speed (m/s) of one output root.
fn capsule_speed(out: &LocoOutput) -> f32 {
    let dp = out.root.dp;
    (dp[0] * dp[0] + dp[1] * dp[1]).sqrt() / TICK_DT
}

/// Replay harness for the twin determinism checks: a fresh warp plus
/// locomotion over `script` (one `(speed, wish)` per tick, stamps `0..`),
/// every output post-processed through `warp_output`. Returns the warp,
/// per-tick receipts, per-tick outputs and the machine.
fn replay_chain(
    script: &[(f32, [f32; 2])],
    window_ticks: u32,
    clip_speed: f32,
    start_speed: f32,
) -> (GroundWarp, Vec<WarpReceipt>, Vec<LocoOutput>, Locomotion) {
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    warp.begin_transient(LocoState::Start, window_ticks, clip_speed, start_speed)
        .expect("window");
    let mut machine = Locomotion::new();
    let mut receipts = Vec::new();
    let mut outs = Vec::new();
    for (t, (speed, wish)) in script.iter().enumerate() {
        let tick = u32::try_from(t).expect("tick fits u32");
        let mut out = machine.update(*speed, *wish, tick, None).expect("loco");
        let capsule = capsule_speed(&out);
        receipts.push(
            warp.warp_output(&mut out, capsule, *wish, &LEGS)
                .expect("warp"),
        );
        outs.push(out);
    }
    (warp, receipts, outs, machine)
}

/// Full consumer chain for one (speed, wish) run: locomotion emits the
/// capsule intent, the warp post-processes it, and planted treadmill feet
/// ride the warped clip cursor under the capsule root while the foot-lock
/// locker meters slide. The belt advances by the warp's own clip steps, so
/// residual slide is exactly the match error. Returns `(warped worst mm,
/// unwarped baseline worst mm)` metered past the start transient.
fn chain_slide(speed: f32, wish: [f32; 2], ticks: u32) -> (u32, u32) {
    let clip = clip_for(speed);
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    warp.begin_transient(LocoState::Start, 30, clip, speed)
        .expect("window");
    let mut machine = Locomotion::new();
    let mut locker = FootLocker::with_defaults();
    let (mut root, mut base, mut belt) = ([0.0f32; 2], [0.0f32; 2], [0.0f32; 2]);
    let mut base_belt = 0.0f32;
    let (mut worst, mut base_worst) = (0u32, 0u32);
    let mut plant_base = [[0.0f32; 2]; 2];
    let mut planted_now = [false; 2];
    for t in 0..ticks {
        if t == 40 {
            // Meter past the start transient: fresh anchors in the steady
            // regime, so only steady match error counts.
            locker.reset();
            planted_now = [false; 2];
        }
        let mut out = machine.update(speed, wish, t, None).expect("loco tick");
        let dp = out.root.dp;
        let capsule = capsule_speed(&out);
        let receipt = warp
            .warp_output(&mut out, capsule, wish, &LEGS)
            .expect("warp tick");
        root = [root[0] + out.root.dp[0], root[1] + out.root.dp[1]];
        base = [base[0] + dp[0], base[1] + dp[1]];
        belt = [
            belt[0] + wish[0] * receipt.clip_step,
            belt[1] + wish[1] * receipt.clip_step,
        ];
        let meter = t >= 40;
        for foot in 0..2u8 {
            let slot = usize::from(foot);
            let down = planted_from_contacts(&out.contacts, t, foot);
            let side = if foot == 0 { 0.11 } else { -0.11 };
            let toe = if foot == 0 { 0.05 } else { -0.05 };
            let raw = [root[0] + side - belt[0], 0.03, root[1] + toe - belt[1]];
            let raw_base = [
                base[0] + side - wish[0] * base_belt,
                0.03,
                base[1] + toe - wish[1] * base_belt,
            ];
            if !down {
                planted_now[slot] = false;
                locker.update(slot, raw, false).expect("release");
                continue;
            }
            if !planted_now[slot] {
                planted_now[slot] = true;
                plant_base[slot] = [raw_base[0], raw_base[2]];
            }
            if meter {
                if let Some(anchor) = locker.anchor(slot).expect("anchor read") {
                    worst = worst.max(slide_mm(raw, anchor));
                }
                let held = [plant_base[slot][0], 0.03, plant_base[slot][1]];
                base_worst = base_worst.max(slide_mm(raw_base, held));
            }
            locker.update(slot, raw, true).expect("lock");
        }
        base_belt += clip * TICK_DT;
    }
    (worst, base_worst)
}

#[test]
fn stride_sweep_slide_under_1cm() {
    let mut sweep_base = 0u32;
    for speed in [
        MIN_WARP_SPEED_M_S,
        1.0,
        1.5,
        2.0,
        4.0,
        6.0,
        MAX_WARP_SPEED_M_S,
    ] {
        let (worst, baseline) = chain_slide(speed, [0.0, 1.0], 120);
        println!("warp straight v={speed}: warped worst={worst} mm baseline={baseline} mm");
        assert!(
            worst <= SLIDE_BUDGET_MM,
            "v={speed}: warped slide {worst} mm past 1 cm"
        );
        sweep_base = sweep_base.max(baseline);
    }
    assert!(
        sweep_base > SLIDE_BUDGET_MM,
        "baseline must skate ({sweep_base} mm) or the warp proves nothing"
    );
    // Replay: a twin chain over the same script locks identically.
    let run = || {
        let script = [(4.0f32, [0.0f32, 1.0]); 60];
        let (warp, receipts, outs, _) = replay_chain(&script, 30, 4.0, 4.0);
        let mut locker = FootLocker::with_defaults();
        let mut locked = Vec::new();
        for (t, out) in outs.iter().enumerate() {
            let tick = u32::try_from(t).expect("tick fits u32");
            for foot in 0..2u8 {
                let down = planted_from_contacts(&out.contacts, tick, foot);
                locked.push(
                    locker
                        .update(usize::from(foot), [0.1, 0.03, 0.05], down)
                        .expect("lock"),
                );
            }
        }
        (warp, receipts, locked)
    };
    let (w1, r1, l1) = run();
    let (w2, r2, l2) = run();
    assert_eq!(w1, w2);
    assert_eq!(r1, r2);
    assert_eq!(l1, l2);
    assert_eq!(feet_hash(&l1), feet_hash(&l2));
}

#[test]
fn strafe_135_skateless_and_capped() {
    for (label, wish) in [("45", [SQ, SQ]), ("90", [1.0f32, 0.0]), ("135", [SQ, -SQ])] {
        let (worst, _) = chain_slide(1.5, wish, 90);
        println!("warp strafe {label} deg: worst={worst} mm");
        assert!(
            worst <= SLIDE_BUDGET_MM_PIVOT,
            "{label} deg strafe skates {worst} mm"
        );
    }
    // The warp turns substantially toward a 90 degree wish.
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    for _ in 0..90 {
        warp.orient_toward([1.0, 0.0]).expect("strafe step");
    }
    assert!(warp.warp_yaw() > 1.2, "90 deg wish turns the mesh");
    assert!(
        warp.warp_yaw() <= STRAFE_WARP_MAX_RAD + 1e-3,
        "warp caps at 135 deg"
    );
    // A full reversal saturates exactly at the cap instead of spinning.
    let mut full = GroundWarp::new();
    full.sync_facing([0.0, 1.0]).expect("sync");
    for _ in 0..90 {
        full.orient_toward([0.0, -1.0]).expect("reversal step");
    }
    assert!(
        (full.warp_yaw().abs() - STRAFE_WARP_MAX_RAD).abs() < 1e-4,
        "180 deg wish saturates at the cap: {}",
        full.warp_yaw()
    );
    // Twin strafes match bit for bit.
    let strafe = || {
        let mut w = GroundWarp::new();
        w.sync_facing([0.0, 1.0]).expect("sync");
        let steps: Vec<f32> = (0..30)
            .map(|_| w.orient_toward([SQ, -SQ]).expect("step"))
            .collect();
        (w, steps)
    };
    assert_eq!(strafe(), strafe());
}

#[test]
fn aligned_wish_takes_no_step() {
    // Facing already on the wish: the exact-reversal break must not fire.
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    assert_eq!(warp.orient_toward([0.0, 3.0]).expect("aligned"), 0.0);
    assert_eq!(warp.warp_yaw(), 0.0);
    // Once converged on a lateral wish, later steps stay at zero.
    for _ in 0..90 {
        warp.orient_toward([1.0, 0.0]).expect("converge");
    }
    let settled = warp.warp_yaw();
    assert!(settled > 1.5, "turned toward +X: {settled}");
    for _ in 0..30 {
        let step = warp.orient_toward([1.0, 0.0]).expect("hold");
        assert!(step.abs() < 1e-6, "converged facing jitters by {step}");
    }
    // A reversal breaks toward +yaw, once per window cap.
    let mut flip = GroundWarp::new();
    flip.sync_facing([0.0, 1.0]).expect("sync");
    assert_eq!(
        flip.orient_toward([0.0, -1.0]).expect("reverse"),
        ORIENT_STEP_CRUISE_RAD
    );
}

#[test]
fn transient_kind_sets_the_turn_cap() {
    let pivot = {
        let mut w = GroundWarp::new();
        w.sync_facing([0.0, 1.0]).expect("sync");
        w.begin_transient(LocoState::Pivot, 10, 1.5, 1.5)
            .expect("pivot window");
        w.orient_toward([0.0, -1.0]).expect("pivot step")
    };
    let start = {
        let mut w = GroundWarp::new();
        w.sync_facing([0.0, 1.0]).expect("sync");
        w.begin_transient(LocoState::Start, 10, 1.5, 1.5)
            .expect("start window");
        w.orient_toward([0.0, -1.0]).expect("start step")
    };
    assert_eq!(pivot, ORIENT_STEP_PIVOT_RAD);
    assert_eq!(start, ORIENT_STEP_CRUISE_RAD);
    assert!(pivot > start, "pivots turn faster than cruise");
}

#[test]
fn distance_match_holds_the_window() {
    // Capsule covers 4 m/s for 12 ticks; the cursor advances at the same
    // 4 m/s, so clip travel equals capsule travel over the window.
    let mut warp = GroundWarp::new();
    warp.begin_transient(LocoState::Pivot, 12, 1.5, 4.0)
        .expect("window");
    assert!(
        (warp.rate() - 4.0 / 1.5).abs() < 1e-6,
        "rate={}",
        warp.rate()
    );
    assert!(
        (warp.stride() - 1.0).abs() < 1e-6,
        "stride={}",
        warp.stride()
    );
    assert!(!warp.settled());
    assert_eq!(warp.window_ticks(), 12);
    assert!((warp.target_speed() - 1.5).abs() < 1e-6);
    for _ in 0..12 {
        let clip_step = warp.tick([0.0, 4.0 * TICK_DT]).expect("tick");
        assert!(
            (clip_step - 4.0 * TICK_DT).abs() < 1e-6,
            "cursor at capsule speed"
        );
    }
    assert!(warp.settled());
    assert_eq!(warp.remaining_ticks(), 0);
    let gap_mm = (warp.clip_travel_m() - warp.capsule_travel_m()).abs() * MM_PER_M;
    assert!(gap_mm < 1.0, "window misses by {gap_mm} mm");
    assert!((warp.capsule_travel_m() - 12.0 * 4.0 * TICK_DT).abs() < 1e-4);
    // Ticks past the window leave the accounts alone.
    let (capsule, clip) = (warp.capsule_travel_m(), warp.clip_travel_m());
    warp.tick([0.0, 4.0 * TICK_DT]).expect("late tick");
    assert_eq!(
        (warp.capsule_travel_m(), warp.clip_travel_m()),
        (capsule, clip)
    );
    // Stationary ends hold rate 1.0: no division, no teleport.
    let mut still = GroundWarp::new();
    still
        .begin_transient(LocoState::Stop, 12, 1.5, 0.0)
        .expect("stop");
    assert_eq!(still.rate(), 1.0);
    assert_eq!(speed_rate(0.0, 4.0).expect("still clip"), 1.0);
}

#[test]
fn stride_bends_only_where_the_rate_saturates() {
    // Inside the rate envelope the stride stays exactly 1.
    let mut warp = GroundWarp::new();
    warp.begin_transient(LocoState::Start, 10, 1.5, 3.0)
        .expect("in envelope");
    assert_eq!(warp.stride(), 1.0);
    // Past it the rate clamps and the stride takes the residual.
    warp.begin_transient(LocoState::Start, 10, 0.5, 7.0)
        .expect("past envelope");
    assert_eq!(warp.rate(), RATE_MAX);
    assert_eq!(warp.stride(), STRIDE_MAX);
}

#[test]
fn root_passes_through_bit_identical() {
    // Authority split: the warp never moves the capsule intent.
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    warp.begin_transient(LocoState::Start, 20, 1.5, 0.5)
        .expect("window");
    let mut machine = Locomotion::new();
    for t in 0..30 {
        let plain = machine
            .clone()
            .update(0.5, [0.0, 1.0], t, None)
            .expect("loco");
        let mut warped = machine.update(0.5, [0.0, 1.0], t, None).expect("loco");
        let capsule = capsule_speed(&warped);
        warp.warp_output(&mut warped, capsule, [0.0, 1.0], &LEGS)
            .expect("warp");
        assert_eq!(warped.root.dp, plain.root.dp, "root delta untouched t={t}");
        assert_eq!(
            warped.root.dyaw, plain.root.dyaw,
            "root yaw untouched t={t}"
        );
        assert_eq!(warped.contacts, plain.contacts, "contacts untouched t={t}");
    }
}

#[test]
fn turning_wish_moves_the_pose_straight_wish_does_not() {
    let mut machine = Locomotion::new();
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    let wish = [SQ, SQ];
    let mut out = machine.update(2.0, wish, 0, None).expect("loco");
    let plain = out.pose.clone();
    let capsule = capsule_speed(&out);
    let receipt = warp
        .warp_output(&mut out, capsule, wish, &LEGS)
        .expect("warp");
    assert!(receipt.dyaw > 0.0, "wish is toward +X from +Z");
    assert_ne!(out.pose, plain, "turning wish yaws the pose");
    // Quaternions stay unit after the yaw premultiply.
    for q in &out.pose.rotations {
        let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
        assert!((n2 - 1.0).abs() < 1e-4, "rotation drifted off unit: {n2}");
    }
    let mut aligned = Locomotion::new();
    let mut steady = GroundWarp::new();
    steady.sync_facing([0.0, 1.0]).expect("sync");
    let mut out = aligned.update(2.0, [0.0, 1.0], 0, None).expect("loco");
    let plain = out.pose.clone();
    let capsule = capsule_speed(&out);
    steady
        .warp_output(&mut out, capsule, [0.0, 1.0], &LEGS)
        .expect("warp");
    assert_eq!(out.pose, plain, "an aligned wish leaves the pose alone");
}

#[test]
fn replay_exact_x2() {
    let script = [
        (0.0f32, [0.0f32, 1.0]),
        (1.5, [0.0, 1.0]),
        (1.5, [1.0, 0.0]),
        (4.0, [0.2, 0.9]),
        (4.0, [-0.2, -0.9]),
        (0.0, [0.0, 1.0]),
        (2.0, [0.7, -0.7]),
    ];
    let long: Vec<(f32, [f32; 2])> = script.iter().cycle().take(70).copied().collect();
    let run = || {
        let (warp, receipts, outs, machine) = replay_chain(&long, 20, 1.5, 1.5);
        let roots: Vec<[f32; 2]> = outs.iter().map(|out| out.root.dp).collect();
        (warp, receipts, roots, machine)
    };
    let first = run();
    let second = run();
    assert_eq!(first.0, second.0);
    assert_eq!(first.1, second.1);
    assert_eq!(first.2, second.2);
    assert_eq!(first.3, second.3);
    assert_ne!(first.1[10], first.1[11], "receipts differ tick to tick");
}

/// Locomotion output at `tick` for a unit wish, for the rejection checks.
fn sample_output(tick: u32) -> LocoOutput {
    Locomotion::new()
        .update(1.5, [0.0, 1.0], tick, None)
        .expect("loco")
}

#[test]
fn failures_are_typed_never_panics() {
    assert_eq!(speed_rate(f32::NAN, 1.0), Err(WarpError::NonFinite("clip")));
    assert_eq!(
        speed_rate(1.0, f32::INFINITY),
        Err(WarpError::NonFinite("capsule"))
    );
    assert_eq!(speed_rate(-1.0, 1.0), Err(WarpError::NegativeSpeed(-1.0)));
    assert_eq!(
        speed_rate(1.0, MAX_WISH_SPEED + 1.0),
        Err(WarpError::SpeedTooHigh(MAX_WISH_SPEED + 1.0))
    );
    assert_eq!(speed_rate(0.5, 7.0).expect("wide"), RATE_MAX);
    assert_eq!(stride_for(1.5, 1.5, 1.0).expect("exact"), 1.0);
    assert_eq!(stride_for(1.5, 1.5, 0.0), Err(WarpError::BadScale(0.0)));
    let mut warp = GroundWarp::new();
    assert_eq!(warp.sync_facing([0.0, 0.0]), Err(WarpError::BadWishDir));
    assert_eq!(
        warp.sync_facing([f32::NAN, 0.0]),
        Err(WarpError::NonFinite("wish dir"))
    );
    assert_eq!(warp.orient_toward([0.0, 0.0]), Err(WarpError::BadWishDir));
    assert_eq!(
        warp.begin_transient(LocoState::Start, 0, 1.0, 1.0),
        Err(WarpError::BadWindow(0))
    );
    assert_eq!(
        warp.tick([f32::NAN, 0.0]),
        Err(WarpError::NonFinite("root delta"))
    );
    assert_eq!(
        WarpError::BadJoint(99).to_string(),
        "warp: leg joint 99 out of range"
    );
}

#[test]
fn rejected_warp_output_mutates_nothing() {
    let mut warp = GroundWarp::new();
    warp.sync_facing([0.0, 1.0]).expect("sync");
    warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
        .expect("window");
    let good = sample_output(0);
    let before_warp = warp.clone();
    let mut bad_dp = good.clone();
    bad_dp.root = RootDelta {
        dp: [f32::NAN, 0.0],
        ..bad_dp.root
    };
    let mut ragged = good.clone();
    ragged.pose.rotations.pop();
    let mut empty = good.clone();
    empty.pose = LocalPose {
        positions: Vec::new(),
        rotations: Vec::new(),
    };
    let mut nan_pose = good.clone();
    nan_pose.pose.positions[1][0] = f32::NAN;
    let ragged_err = WarpError::LengthMismatch {
        expected: good.pose.positions.len(),
        got: good.pose.rotations.len() - 1,
    };
    let cases: [(LocoOutput, f32, [f32; 2], &[usize], WarpError); 7] = [
        (good.clone(), 1.5, [0.0, 0.0], &LEGS, WarpError::BadWishDir),
        (
            good.clone(),
            1.5,
            [0.0, 1.0],
            &[3, 99],
            WarpError::BadJoint(99),
        ),
        (
            good.clone(),
            -1.0,
            [0.0, 1.0],
            &LEGS,
            WarpError::NegativeSpeed(-1.0),
        ),
        (
            bad_dp,
            1.5,
            [0.0, 1.0],
            &LEGS,
            WarpError::NonFinite("root delta"),
        ),
        (ragged, 1.5, [0.0, 1.0], &LEGS, ragged_err),
        (empty, 1.5, [0.0, 1.0], &LEGS, WarpError::EmptyPose),
        (
            nan_pose,
            1.5,
            [0.0, 1.0],
            &LEGS,
            WarpError::NonFinite("pose positions"),
        ),
    ];
    for (mut target, speed, wish, legs, expect) in cases {
        // Debug text, not `==`: NaN lanes never compare equal to themselves.
        let snapshot = format!("{target:?}");
        assert_eq!(
            warp.warp_output(&mut target, speed, wish, legs),
            Err(expect)
        );
        assert_eq!(
            format!("{target:?}"),
            snapshot,
            "{expect:?} touched the output"
        );
        assert_eq!(warp, before_warp, "{expect:?} touched the warp");
    }
}

#[test]
fn steady_tick_uses_no_heap() {
    // Calibration: the probe must see a real allocation.
    let ((), heap) = measure(|| {
        std::hint::black_box(vec![0u8; 64]);
    });
    assert!(heap.calls >= 1, "alloc probe is blind");
    // The scalar tick path allocates nothing.
    let ((), heap) = measure(|| {
        let mut warp = GroundWarp::new();
        warp.begin_transient(LocoState::Pivot, 10, 1.5, 4.0)
            .expect("window");
        for _ in 0..10 {
            std::hint::black_box(warp.tick([0.01, 0.02]).expect("tick"));
            std::hint::black_box(warp.orient_toward([0.2, 0.9]).expect("orient"));
            std::hint::black_box(speed_rate(1.5, 4.0).expect("rate"));
        }
    });
    assert_eq!(heap.calls, 0, "steady warp tick allocated");
    // The full consumer entry over a caller-owned output allocates nothing.
    let mut out = sample_output(0);
    let mut warp = GroundWarp::new();
    warp.begin_transient(LocoState::Start, 10, 1.5, 1.5)
        .expect("window");
    let ((), heap) = measure(|| {
        std::hint::black_box(
            warp.warp_output(&mut out, 1.5, [0.0, 1.0], &LEGS)
                .expect("warp"),
        );
    });
    assert_eq!(heap.calls, 0, "warp_output allocated");
}
