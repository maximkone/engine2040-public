//! motion_warp tests: one lunge clip warped onto many distances. Each test
//! goes red without the warp: the raw clip ends where it ends.

use super::contract::{RootDelta, RootTable};
use super::motion_warp::{
    lunge_target, root_after, slide_mm, warp_root, WarpError, WarpLimits, WarpTarget, WarpWindow,
};

/// Clip length in ticks: 6 wind-up, 20 lunge window, 14 recovery.
const TICKS: u32 = 40;
/// The warp window: the burst of the lunge.
const WINDOW: WarpWindow = WarpWindow {
    start_tick: 6,
    end_tick: 26,
};

/// A lunge clip: slow wind-up, a bell-shaped burst forward (about 1.2 m, a
/// little sideways drift, a few degrees of yaw), then a slow recovery.
fn lunge_clip() -> RootTable {
    let deltas = (0..TICKS)
        .map(|tick| {
            let (dp, dyaw) = if (WINDOW.start_tick..WINDOW.end_tick).contains(&tick) {
                let u = (tick - WINDOW.start_tick) as f32 / 20.0;
                let bell = (u * (1.0 - u)) * 6.0 / 20.0 * 1.2; // integrates to ~1.2 m
                ([0.08 * bell, bell], 0.002 + 0.001 * u)
            } else if tick < WINDOW.start_tick {
                ([0.0, -0.002], 0.0)
            } else {
                ([0.0, 0.004], -0.001)
            };
            RootDelta { tick, dp, dyaw }
        })
        .collect();
    RootTable { deltas }
}

fn win(table: &RootTable) -> &[RootDelta] {
    &table.deltas[WINDOW.start_tick as usize..WINDOW.end_tick as usize]
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn target(dp: [f32; 2]) -> WarpTarget {
    WarpTarget { dp, dyaw: None }
}

/// Targets from a short poke to a long lunge, straight and off-angle.
const TARGETS: [[f32; 2]; 7] = [
    [0.0, 0.5],
    [0.0, 0.9],
    [0.05, 1.6],
    [0.0, 2.2],
    [0.4, 1.3],
    [-0.5, 1.1],
    [0.3, 0.8],
];

#[test]
fn warp_root_ends_exactly_at_the_target_at_the_window_end() {
    let clip = lunge_clip();
    let raw_end = root_after(win(&clip));
    for goal in TARGETS {
        let (warped, _) = warp_root(&clip, WINDOW, target(goal), WarpLimits::default()).unwrap();
        let end = root_after(win(&warped));
        assert!(
            dist(end, goal) < 1.0e-5,
            "target {goal:?}: the window ended at {end:?}"
        );
        // Without the warp the same clip ends elsewhere: the test has teeth.
        assert!(dist(raw_end, goal) > 0.05, "{goal:?}");
    }
    // Hard limits too: the skew is held back, the landing is not.
    let tight = WarpLimits {
        scale_min: 0.9,
        scale_max: 1.1,
        min_turn_cos: 0.95,
    };
    let (warped, report) = warp_root(&clip, WINDOW, target([0.0, 2.2]), tight).unwrap();
    assert!(report.clamped, "the limits held the skew back");
    assert!(report.residual_m > 0.3, "the spread covers the rest");
    assert!(dist(root_after(win(&warped)), [0.0, 2.2]) < 1.0e-5);
}

#[test]
fn warp_root_leaves_every_tick_outside_the_window_untouched() {
    let clip = lunge_clip();
    let (warped, _) = warp_root(&clip, WINDOW, target([0.2, 1.8]), WarpLimits::default()).unwrap();
    assert_eq!(warped.deltas.len(), clip.deltas.len());
    for (a, b) in clip.deltas.iter().zip(&warped.deltas) {
        if (WINDOW.start_tick..WINDOW.end_tick).contains(&a.tick) {
            continue;
        }
        assert_eq!(a.tick, b.tick);
        assert_eq!(a.dp[0].to_bits(), b.dp[0].to_bits(), "tick {}", a.tick);
        assert_eq!(a.dp[1].to_bits(), b.dp[1].to_bits(), "tick {}", a.tick);
        assert_eq!(a.dyaw.to_bits(), b.dyaw.to_bits(), "tick {}", a.tick);
    }
    // Before the window the root is where the clip puts it; after it the
    // clip's own recovery plays on from the warped landing spot.
    let pre = WINDOW.start_tick as usize;
    assert_eq!(
        root_after(&warped.deltas[..pre]),
        root_after(&clip.deltas[..pre])
    );
    let post = WINDOW.end_tick as usize;
    let tail_raw = root_after(&clip.deltas[post..]);
    let tail_warped = root_after(&warped.deltas[post..]);
    assert!(dist(tail_raw, tail_warped) < 1.0e-6);
}

#[test]
fn warp_root_velocity_is_continuous_at_the_window_edges() {
    let clip = lunge_clip();
    let goal = [0.3, 1.9];
    let (warped, _) = warp_root(&clip, WINDOW, target(goal), WarpLimits::default()).unwrap();
    let (raw, new) = (win(&clip), win(&warped));
    let miss = dist(root_after(raw), goal);
    // The change the warp makes to the speed: the first and the last tick of
    // the window move as the clip does (a sliver of the miss), the middle
    // carries the real correction.
    let change = |i: usize| dist(raw[i].dp, new[i].dp);
    let mid = (0..raw.len()).map(change).fold(0.0f32, f32::max);
    assert!(mid > 0.02, "the warp changes the lunge, got {mid}");
    for i in [0, raw.len() - 1] {
        assert!(
            change(i) < 0.02 * mid.max(miss),
            "edge tick {i} jumps {} against a {mid} peak",
            change(i)
        );
    }
    // And the step to the neighbour outside the window is no bigger than
    // the clip's own step there, plus the sliver.
    let step = |a: &RootDelta, b: &RootDelta| dist(a.dp, b.dp);
    let into = step(&warped.deltas[5], &warped.deltas[6]);
    let into_raw = step(&clip.deltas[5], &clip.deltas[6]);
    let out = step(&warped.deltas[25], &warped.deltas[26]);
    let out_raw = step(&clip.deltas[25], &clip.deltas[26]);
    assert!(
        (into - into_raw).abs() < 0.02 * mid.max(miss),
        "{into} {into_raw}"
    );
    assert!(
        (out - out_raw).abs() < 0.02 * mid.max(miss),
        "{out} {out_raw}"
    );
}

#[test]
fn warp_root_to_the_clips_own_end_changes_nothing() {
    let clip = lunge_clip();
    let own = root_after(win(&clip));
    let yaw: f32 = win(&clip).iter().map(|d| d.dyaw).sum();
    let (warped, report) = warp_root(
        &clip,
        WINDOW,
        WarpTarget {
            dp: own,
            dyaw: Some(yaw),
        },
        WarpLimits::default(),
    )
    .unwrap();
    for (a, b) in clip.deltas.iter().zip(&warped.deltas) {
        assert!(dist(a.dp, b.dp) < 2.0e-6, "tick {}", a.tick);
        assert!((a.dyaw - b.dyaw).abs() < 2.0e-6, "tick {}", a.tick);
    }
    assert!((report.scale - 1.0).abs() < 1.0e-3 && report.residual_m < 1.0e-4);
    assert!(!report.clamped);
}

#[test]
fn warp_root_stretches_and_turns_the_stride_not_just_the_end() {
    let clip = lunge_clip();
    // A longer lunge: the burst is stretched in the middle of the window.
    let (long, report) =
        warp_root(&clip, WINDOW, target([0.0, 2.4]), WarpLimits::default()).unwrap();
    assert!(report.scale > 1.5, "stride scale {}", report.scale);
    let peak = |t: &RootTable| win(t).iter().map(|d| d.dp[1]).fold(0.0f32, f32::max);
    assert!(peak(&long) > 1.5 * peak(&clip));
    // A sidestepping target: the burst heads off to that side mid-window.
    let (side, report) =
        warp_root(&clip, WINDOW, target([0.9, 1.0]), WarpLimits::default()).unwrap();
    assert!(report.turn_cos < 0.99, "turn cos {}", report.turn_cos);
    let mid = win(&side)[10];
    assert!(mid.dp[0] > 0.04, "mid-window drift {}", mid.dp[0]);
    assert!(mid.dp[0] / mid.dp[1] > 3.0 * (win(&clip)[10].dp[0] / win(&clip)[10].dp[1]));
}

#[test]
fn warp_root_yaw_lands_on_its_target_and_eases_at_the_edges() {
    let clip = lunge_clip();
    let want = 0.6f32;
    let (warped, _) = warp_root(
        &clip,
        WINDOW,
        WarpTarget {
            dp: [0.0, 1.0],
            dyaw: Some(want),
        },
        WarpLimits::default(),
    )
    .unwrap();
    let total: f32 = win(&warped).iter().map(|d| d.dyaw).sum();
    assert!((total - want).abs() < 1.0e-5, "yaw total {total}");
    let (raw, new) = (win(&clip), win(&warped));
    let peak = (0..raw.len())
        .map(|i| (raw[i].dyaw - new[i].dyaw).abs())
        .fold(0.0f32, f32::max);
    assert!(peak > 0.01);
    for i in [0, raw.len() - 1] {
        assert!((raw[i].dyaw - new[i].dyaw).abs() < 0.03 * peak, "edge {i}");
    }
}

#[test]
fn warp_root_never_draws_more_foot_slide_than_the_raw_clip() {
    // The authority moves the fighter at a steady pace over the window to the
    // target. The clip's stride either disagrees with that (raw) or covers it
    // (warped); the slide meter reads the gap.
    let clip = lunge_clip();
    for goal in TARGETS {
        let authority: Vec<RootDelta> = (0..WINDOW.end_tick - WINDOW.start_tick)
            .map(|k| RootDelta {
                tick: WINDOW.start_tick + k,
                dp: [goal[0] / 20.0, goal[1] / 20.0],
                dyaw: 0.0,
            })
            .collect();
        let (warped, _) = warp_root(&clip, WINDOW, target(goal), WarpLimits::default()).unwrap();
        let raw_slide = slide_mm(win(&clip), &authority);
        let warped_slide = slide_mm(win(&warped), &authority);
        assert!(
            warped_slide <= raw_slide + 0.5,
            "target {goal:?}: raw {raw_slide} mm, warped {warped_slide} mm"
        );
        // At the window end the clip and the authority agree to a hundredth of a mm.
        let a_end = root_after(&authority);
        assert!(dist(root_after(win(&warped)), a_end) < 1.0e-5);
    }
    // A pure stretch (same direction, longer lunge) cuts the slide, not just ties it.
    let goal = [0.0, 2.0];
    let authority: Vec<RootDelta> = (0..20)
        .map(|k| RootDelta {
            tick: 6 + k,
            dp: [0.0, 0.1],
            dyaw: 0.0,
        })
        .collect();
    let (warped, _) = warp_root(&clip, WINDOW, target(goal), WarpLimits::default()).unwrap();
    assert!(slide_mm(win(&warped), &authority) < 0.7 * slide_mm(win(&clip), &authority));
}

#[test]
fn warp_root_is_bit_exact_on_replay() {
    let clip = lunge_clip();
    let a = warp_root(&clip, WINDOW, target([0.2, 1.4]), WarpLimits::default()).unwrap();
    let b = warp_root(&clip, WINDOW, target([0.2, 1.4]), WarpLimits::default()).unwrap();
    assert_eq!(a.0, b.0);
    assert_eq!(a.1, b.1);
}

#[test]
fn warp_root_refuses_hostile_input_and_returns_no_table() {
    let clip = lunge_clip();
    let ok = target([0.0, 1.0]);
    let l = WarpLimits::default();
    assert_eq!(
        warp_root(&clip, WINDOW, target([f32::NAN, 1.0]), l).unwrap_err(),
        WarpError::BadValue("target")
    );
    assert_eq!(
        warp_root(&clip, WINDOW, target([0.0, 1.0e9]), l).unwrap_err(),
        WarpError::BadValue("target")
    );
    let short = WarpWindow {
        start_tick: 6,
        end_tick: 8,
    };
    assert_eq!(
        warp_root(&clip, short, ok, l).unwrap_err(),
        WarpError::BadWindow(2)
    );
    let past = WarpWindow {
        start_tick: 30,
        end_tick: 50,
    };
    assert_eq!(
        warp_root(&clip, past, ok, l).unwrap_err(),
        WarpError::WindowGap(40)
    );
    let backwards = WarpWindow {
        start_tick: 20,
        end_tick: 10,
    };
    assert_eq!(
        warp_root(&clip, backwards, ok, l).unwrap_err(),
        WarpError::BadWindow(0)
    );
    let bad = WarpLimits {
        scale_min: 0.0,
        ..l
    };
    assert_eq!(
        warp_root(&clip, WINDOW, ok, bad).unwrap_err(),
        WarpError::BadLimits
    );
    let mut holed = clip.clone();
    holed.deltas.remove(10);
    assert_eq!(
        warp_root(&holed, WINDOW, ok, l).unwrap_err(),
        WarpError::WindowGap(10)
    );
}

#[test]
fn warp_root_handles_a_clip_that_stands_still() {
    // No path to scale or turn: the whole miss goes through the spread and
    // the root still lands on the target.
    let still = RootTable {
        deltas: (0..TICKS)
            .map(|tick| RootDelta {
                tick,
                dp: [0.0; 2],
                dyaw: 0.0,
            })
            .collect(),
    };
    let (warped, report) =
        warp_root(&still, WINDOW, target([0.1, 0.7]), WarpLimits::default()).unwrap();
    assert!(dist(root_after(win(&warped)), [0.1, 0.7]) < 1.0e-5);
    assert!((report.residual_m - dist([0.1, 0.7], [0.0, 0.0])).abs() < 1.0e-5);
}

#[test]
fn lunge_target_stops_a_blade_reach_short_of_the_opponent() {
    let t = lunge_target([0.0, 0.0], [0.0, 3.0], 1.2).unwrap();
    assert!(dist(t, [0.0, 1.8]) < 1.0e-6);
    let diag = lunge_target([1.0, 1.0], [4.0, 5.0], 1.0).unwrap();
    assert!(dist(diag, [2.4, 3.2]) < 1.0e-5, "{diag:?}");
    // Already inside reach: no step, never backs off.
    assert_eq!(lunge_target([0.0, 0.0], [0.0, 1.0], 1.2), Some([0.0, 0.0]));
    assert_eq!(lunge_target([0.0, 0.0], [f32::NAN, 1.0], 1.2), None);
    assert_eq!(lunge_target([0.0, 0.0], [0.0, 1.0], -1.0), None);
    // The blade arrives: the warped root meets the reach point.
    let clip = lunge_clip();
    let goal = lunge_target([0.0, 0.0], [0.4, 2.6], 1.0).unwrap();
    let (warped, _) = warp_root(&clip, WINDOW, target(goal), WarpLimits::default()).unwrap();
    assert!(dist(root_after(win(&warped)), goal) < 1.0e-5);
}
