//! warp_ground starts/stops accept sweep: straight starts/stops at
//! 0.5-7 m/s slide under 1 cm and replay bit-exactly. Wires the
//! distance match against the foot slide meter; the locomotion
//! Start/Stop transient wiring lands in a later deepen.

use super::warp_ground::{
    match_rate, plan, warp_hash, DistanceWarp, WarpError, WarpKind, RATE_MAX, RATE_MIN,
    SLIDE_BUDGET_MM,
};
use crate::clip::TICK_DT;
use crate::feet::slide_mm;
use crate::locomotion::{START_TICKS, STOP_TICKS};
use engine_core::alloc_probe::measure;

/// Sweep speeds (m/s): endpoints plus mids of the 0.5-7 accept band.
const SPEEDS: [f32; 6] = [0.5, 1.0, 2.0, 3.5, 5.0, 7.0];
/// Baked-clip error model: the raw clip runs 15% fast with a 2 mm drag.
const CLIP_FAST: f32 = 1.15;
/// Baked-clip drag per tick in meters.
const CLIP_DRAG_M: f32 = 0.002;

/// Named `u32` to `f32` conversion for tick counters (all windows fit in
/// `u16`); saturates instead of wrapping so fixtures stay finite.
fn u32_f32(v: u32) -> f32 {
    f32::from(u16::try_from(v).unwrap_or(u16::MAX))
}

/// Capsule ramp step in meters for `tick` of `total` at `target` m/s.
/// Start ramps up, stop ramps down; linear, matches [`plan`].
fn capsule_step(kind: WarpKind, target: f32, tick: u32, total: u32) -> f32 {
    let speed = match kind {
        WarpKind::Start => target * (u32_f32(tick) + 1.0) / u32_f32(total),
        WarpKind::Stop => target * (u32_f32(total) - u32_f32(tick)) / u32_f32(total),
    };
    speed * TICK_DT
}

/// Raw baked anim step in meters: fast plus drag, the sampling error.
fn anim_step(raw_capsule: f32) -> f32 {
    raw_capsule * CLIP_FAST + CLIP_DRAG_M
}

/// Run one transient; returns warped steps plus per-tick planted foot x.
fn run(kind: WarpKind, target: f32) -> (Vec<f32>, Vec<[f32; 3]>) {
    let planned_res = plan(kind, target);
    assert!(planned_res.is_ok(), "fixture plan failed");
    let planned = match planned_res {
        Ok(p) => p,
        Err(_) => return (Vec::new(), Vec::new()),
    };
    let mut warp = DistanceWarp::new(planned);
    let cap = usize::try_from(planned.ticks).unwrap_or(0);
    let mut steps = Vec::with_capacity(cap);
    let mut feet = Vec::with_capacity(cap);
    for t in 0..planned.ticks {
        let c = capsule_step(kind, target, t, planned.ticks);
        let a = anim_step(c);
        let out_res = warp.tick(c, a);
        assert!(out_res.is_ok(), "fixture tick failed");
        let out = match out_res {
            Ok(o) => o,
            Err(_) => continue,
        };
        steps.push(out.warped_step_m);
        // Planted foot world x: anchor at 0 plus the running residual.
        let r = warp.warped_covered_m() - warp.capsule_covered_m();
        feet.push([r, 0.0, 0.0]);
    }
    (steps, feet)
}

/// Largest slide between a run's first and last foot positions in mm.
fn worst_slide(feet: &[[f32; 3]]) -> u32 {
    if feet.len() < 2 {
        return 0;
    }
    slide_mm(feet[0], feet[feet.len() - 1])
}

/// Raw (unwarped) drift in mm over the same script: proves the fixture
/// actually exercises the warp instead of passing on an easy script.
fn raw_drift_mm(kind: WarpKind, target: f32) -> u32 {
    let planned_res = plan(kind, target);
    assert!(planned_res.is_ok(), "fixture plan failed");
    let planned = match planned_res {
        Ok(p) => p,
        Err(_) => return 0,
    };
    let (mut raw_covered, mut capsule_covered) = (0.0f32, 0.0f32);
    for t in 0..planned.ticks {
        let c = capsule_step(kind, target, t, planned.ticks);
        raw_covered += anim_step(c);
        capsule_covered += c;
    }
    slide_mm([raw_covered, 0.0, 0.0], [capsule_covered, 0.0, 0.0])
}

#[test]
fn starts_slide_under_1cm_05_to_7() {
    for target in SPEEDS {
        let (steps, feet) = run(WarpKind::Start, target);
        let raw = raw_drift_mm(WarpKind::Start, target);
        assert!(
            raw > SLIDE_BUDGET_MM,
            "start {target} m/s raw drift {raw} mm must exceed the budget"
        );
        let worst = worst_slide(&feet);
        println!("warp start {target} m/s: raw={raw} mm warped worst={worst} mm");
        assert!(
            worst <= SLIDE_BUDGET_MM,
            "start {target} m/s slide {worst} mm past 1 cm"
        );
        assert_eq!(
            steps.len(),
            usize::try_from(START_TICKS).unwrap_or(usize::MAX)
        );
    }
}

#[test]
fn stops_slide_under_1cm_05_to_7() {
    for target in SPEEDS {
        let (steps, feet) = run(WarpKind::Stop, target);
        let raw = raw_drift_mm(WarpKind::Stop, target);
        assert!(
            raw > SLIDE_BUDGET_MM,
            "stop {target} m/s raw drift {raw} mm must exceed the budget"
        );
        let worst = worst_slide(&feet);
        println!("warp stop {target} m/s: raw={raw} mm warped worst={worst} mm");
        assert!(
            worst <= SLIDE_BUDGET_MM,
            "stop {target} m/s slide {worst} mm past 1 cm"
        );
        assert_eq!(
            steps.len(),
            usize::try_from(STOP_TICKS).unwrap_or(usize::MAX)
        );
    }
}

#[test]
fn plan_matches_the_ramp_total() {
    // Closed form vs independent per-tick sum: fails if the formula drifts.
    for kind in [WarpKind::Start, WarpKind::Stop] {
        for target in SPEEDS {
            let planned = plan(kind, target).expect("plan");
            let mut sum = 0.0f32;
            for t in 0..planned.ticks {
                sum += capsule_step(kind, target, t, planned.ticks);
            }
            assert!(
                (planned.capsule_distance_m - sum).abs() < 1e-4,
                "{kind:?} {target}: plan {} vs sum {sum}",
                planned.capsule_distance_m
            );
        }
    }
}

#[test]
fn replay_is_bit_exact_twice() {
    for target in SPEEDS {
        let (a_steps, a_feet) = run(WarpKind::Start, target);
        let (b_steps, b_feet) = run(WarpKind::Start, target);
        assert_eq!(a_steps, b_steps, "start {target} steps");
        assert_eq!(a_feet, b_feet, "start {target} feet");
        assert_eq!(warp_hash(&a_steps), warp_hash(&b_steps));
        let (c_steps, c_feet) = run(WarpKind::Stop, target);
        let (d_steps, d_feet) = run(WarpKind::Stop, target);
        assert_eq!(c_steps, d_steps, "stop {target} steps");
        assert_eq!(c_feet, d_feet, "stop {target} feet");
        assert_eq!(warp_hash(&c_steps), warp_hash(&d_steps));
    }
    assert_eq!(warp_hash(&[1.0, 2.0]), warp_hash(&[1.0, 2.0]));
    assert_ne!(warp_hash(&[1.0, 2.0]), warp_hash(&[1.0, 2.5]));
}

#[test]
fn failures_are_typed() {
    assert_eq!(
        plan(WarpKind::Start, -1.0),
        Err(WarpError::NegativeSpeed(-1.0))
    );
    assert_eq!(
        plan(WarpKind::Stop, 99.0),
        Err(WarpError::SpeedTooHigh(99.0))
    );
    assert_eq!(
        plan(WarpKind::Start, f32::NAN),
        Err(WarpError::NonFinite("target"))
    );
    assert_eq!(
        match_rate(f32::NAN, 0.05),
        Err(WarpError::NonFinite("capsule_step"))
    );
    assert_eq!(
        match_rate(0.05, f32::NAN),
        Err(WarpError::NonFinite("anim_step"))
    );
    assert_eq!(
        match_rate(-0.01, 0.05),
        Err(WarpError::BadStep("capsule_step"))
    );
    assert_eq!(match_rate(0.05, 0.0), Err(WarpError::BadStep("anim_step")));
    assert_eq!(
        match_rate(0.05, -0.02),
        Err(WarpError::BadStep("anim_step"))
    );
    // A rejected tick mutates nothing.
    let planned = plan(WarpKind::Start, 2.0).expect("plan");
    let mut warp = DistanceWarp::new(planned);
    let before = warp.clone();
    assert!(warp.tick(0.05, 0.0).is_err());
    assert_eq!(warp, before);
}

#[test]
fn rate_clamps_instead_of_teleporting() {
    assert_eq!(match_rate(10.0, 0.05).expect("clamp top"), RATE_MAX);
    assert_eq!(match_rate(0.0001, 0.5).expect("clamp bottom"), RATE_MIN);
    assert_eq!(match_rate(0.05, 0.05).expect("unity"), 1.0);
}

#[test]
fn pivots_strafes_and_stride_scope_flags_match_the_ground_warp() {
    // Scope flags read at runtime (black_box keeps the check live) and each
    // one is pinned to behavior, so a flag cannot claim what the warp lacks.
    use super::locomotion::LocoState;
    use super::warp_ground::{
        GroundWarp, ORIENT_STEP_CRUISE_RAD, ORIENT_STEP_PIVOT_RAD, STRAFE_WARP_MAX_RAD, STRIDE_MAX,
    };
    assert!(
        std::hint::black_box(super::warp_ground::PIVOT_SUPPORTED),
        "pivot windows warp"
    );
    assert!(
        std::hint::black_box(super::warp_ground::STRAFE_SUPPORTED),
        "strafes warp"
    );
    assert!(
        std::hint::black_box(super::warp_ground::STRIDE_SUPPORTED),
        "stride warps"
    );
    let mut pivot = GroundWarp::new();
    pivot
        .begin_transient(LocoState::Pivot, 10, 1.5, 1.5)
        .expect("pivot window");
    assert_eq!(
        pivot.orient_toward([0.0, -1.0]).expect("pivot step"),
        ORIENT_STEP_PIVOT_RAD
    );
    let mut strafe = GroundWarp::new();
    for _ in 0..90 {
        strafe.orient_toward([0.0, -1.0]).expect("strafe step");
    }
    assert!((strafe.warp_yaw() - STRAFE_WARP_MAX_RAD).abs() < 1e-4);
    assert!(ORIENT_STEP_PIVOT_RAD > ORIENT_STEP_CRUISE_RAD);
    let mut stride = GroundWarp::new();
    stride
        .begin_transient(LocoState::Start, 10, 0.5, 7.0)
        .expect("stride window");
    assert_eq!(stride.stride(), STRIDE_MAX);
    // WarpKind covers starts/stops only: both arms plan a live window.
    let kinds = std::hint::black_box([WarpKind::Start, WarpKind::Stop]);
    let mut seen = 0u32;
    for kind in kinds {
        let planned = plan(kind, 2.0).expect("plan");
        assert!(planned.ticks > 0);
        seen += 1;
    }
    assert_eq!(seen, 2);
}

#[test]
fn alloc_probe_sees_a_real_allocation() {
    let ((), heap) = measure(|| {
        std::hint::black_box(vec![0u8; 64]);
    });
    assert!(heap.calls >= 1, "alloc probe is blind");
}

#[test]
fn steady_tick_allocates_nothing() {
    let planned = plan(WarpKind::Start, 2.0).expect("plan");
    let mut warp = DistanceWarp::new(planned);
    let ((), heap) = measure(|| {
        for t in 0..planned.ticks {
            let c = capsule_step(WarpKind::Start, 2.0, t, planned.ticks);
            let out = warp.tick(c, anim_step(c)).expect("tick");
            std::hint::black_box(out);
        }
    });
    assert_eq!(heap.calls, 0, "distance tick must stay allocation-free");
}
