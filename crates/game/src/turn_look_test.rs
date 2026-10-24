//! turn_look tests: the view yaw, the gate it feeds and the eased root offset.

use super::*;

/// Sustained view yaw for `ticks`: how the look sees a held aim.
fn hold(turn: &mut AimTurn, view_deg: f32, moving: bool, ticks: u32) -> ViewTurn {
    let mut last = turn.step(view_deg, moving);
    for _ in 1..ticks {
        last = turn.step(view_deg, moving);
    }
    last
}

#[test]
fn turn_in_place_the_tick_rate_float_matches_the_gate() {
    // The speed below is degrees per tick times this float, so it must be the
    // gate's own tick rate.
    assert_eq!(TICKS_PER_SECOND, 60);
    assert_eq!(TICKS_PER_SECOND_F, 60.0);
}

#[test]
fn turn_in_place_the_view_yaw_is_the_clamped_bearing_to_the_rival() {
    // A body facing +z at the origin, rivals all around it. Positive is right.
    let me = [0.0, 0.0];
    assert!((aim_yaw_deg(0.0, me, [1.0, 0.0]) - 90.0).abs() < 1.0e-4);
    assert!((aim_yaw_deg(0.0, me, [-1.0, 0.0]) + 90.0).abs() < 1.0e-4);
    assert!(
        aim_yaw_deg(0.0, me, [0.0, 2.0]).abs() < 1.0e-6,
        "dead ahead"
    );
    let behind = aim_yaw_deg(0.0, me, [0.0, -2.0]);
    assert_eq!(
        behind.abs(),
        AIM_CLAMP_DEG,
        "behind clamps to the aim limit"
    );
    // Turning the body turns the view with it.
    let quarter = aim_yaw_deg(90.0, me, [0.0, 2.0]);
    assert!(
        (quarter + 90.0).abs() < 1.0e-4,
        "a body turned right sees the same rival on its left: {quarter}"
    );
    assert_eq!(aim_yaw_deg(f32::NAN, me, [1.0, 0.0]), 0.0);
    assert_eq!(aim_yaw_deg(0.0, me, me), 0.0);
}

#[test]
fn turn_in_place_the_root_catches_a_slow_view_and_the_clip_fires() {
    let mut turn = AimTurn::new();
    assert_eq!(turn.root_offset_deg(), 0.0);

    // The aim snaps to 90 in one tick, which reads as a fast view: the timer
    // starts on the next tick, so ALS's 15 tick delay for 90 degrees lands on
    // tick 16. The root then eases onto the view over the blend.
    let mut fired = None;
    let mut tick_of_fire = 0;
    for tick in 1..=30u32 {
        let out = turn.step(90.0, false);
        if out.fired.is_some() {
            fired = out.fired;
            tick_of_fire = tick;
        }
    }
    assert_eq!(fired, Some(TurnClip::Right90), "the gate plays Right90");
    assert_eq!(tick_of_fire, 16, "ALS's 90 degree delay is 15 ticks");
    assert!(
        (turn.root_offset_deg() - 90.0).abs() < 1.0e-4,
        "the root caught the view: {}",
        turn.root_offset_deg()
    );

    // The left shoulder mirrors, and the aim limit caps what the root owes:
    // a view behind the body is a 70 degree view, so the 90 clip plays and the
    // root stops at the clamp.
    let mut left = AimTurn::new();
    assert_eq!(
        hold(&mut left, -90.0, false, 30).fired,
        Some(TurnClip::Left90)
    );
    assert!((left.root_offset_deg() + 90.0).abs() < 1.0e-4);
    let mut far = AimTurn::new();
    assert_eq!(
        hold(&mut far, -170.0, false, 30).fired,
        Some(TurnClip::Left90)
    );
    assert!(
        (far.root_offset_deg() + AIM_CLAMP_DEG).abs() < 1.0e-4,
        "the root stops at the aim limit: {}",
        far.root_offset_deg()
    );
}

#[test]
fn turn_in_place_a_moving_body_never_turns_and_the_run_repeats() {
    let mut turn = AimTurn::new();
    for _ in 0..60u32 {
        let out = turn.step(90.0, true);
        assert_eq!(out.fired, None, "a moving body never turns");
    }
    assert_eq!(turn.root_offset_deg(), 0.0, "and its root never swings");

    // The same script twice gives the same offsets and clips: the wire is a
    // pure function of the aim it is fed, so a replay cannot drift.
    fn script() -> Vec<(f32, Option<TurnClip>)> {
        let mut turn = AimTurn::new();
        let mut seen = Vec::new();
        for tick in 0..48u32 {
            let view = match tick {
                0..=15 => 60.0,
                16..=31 => -60.0,
                _ => 10.0,
            };
            let out = turn.step(view, false);
            seen.push((out.root_offset_deg, out.fired));
        }
        seen
    }
    assert_eq!(script(), script(), "x2 equal");
}
