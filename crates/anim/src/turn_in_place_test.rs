//! turn_in_place tests: the ALS activation ladder, the clip choice and the
//! stored play rate.

use super::*;

/// Feed `count` ticks of one view and hand back the last answer.
fn hold(gate: &mut TurnGate, yaw_deg: f32, speed_deg_s: f32, count: u32) -> Option<Turn> {
    let mut out = None;
    for _ in 0..count {
        out = gate.step(yaw_deg, speed_deg_s, false);
    }
    out
}

#[test]
fn a_slow_90_degree_view_turn_fires_after_its_delay_and_a_moving_body_never_turns() {
    // ALS `ViewYawAngleToActivationDelay{0.0f, 0.75f}` over 45..180 degrees is
    // 0.25 s at 90 degrees, so 15 ticks at 60 Hz.
    let delay = delay_ticks(90.0);
    // 15 of 60 ticks is the curve's 0.25 s.
    assert_eq!(delay, 15, "90 degrees sits a quarter into the delay curve");
    assert_eq!(TICKS_PER_SECOND * 15 / 60, 15);

    let mut gate = TurnGate::new();
    for tick in 1..delay {
        assert_eq!(gate.step(90.0, 10.0, false), None, "early tick {tick}");
        assert_eq!(gate.timer_ticks(), tick, "the timer counts integer ticks");
    }
    assert_eq!(
        gate.step(90.0, 10.0, false),
        Some(Turn {
            clip: TurnClip::Right90,
            yaw_deg: 90.0,
        }),
        "the turn fires on its delay tick"
    );
    assert_eq!(gate.timer_ticks(), 0, "a fired turn re-arms the timer");

    // A moving body never turns, however long the view is held.
    for tick in 1..=120 {
        assert_eq!(gate.step(90.0, 10.0, true), None, "moving tick {tick}");
        assert_eq!(gate.timer_ticks(), 0, "a moving body keeps the timer at 0");
    }

    // A view still swinging re-arms the timer: the conditions must hold.
    let mut gate = TurnGate::new();
    assert_eq!(
        gate.step(90.0, 200.0, false),
        None,
        "a fast view does not turn"
    );
    assert_eq!(gate.timer_ticks(), 0);
    assert_eq!(gate.step(30.0, 10.0, false), None, "inside the threshold");
    assert_eq!(gate.timer_ticks(), 0);

    // Total for non-finite input.
    let mut gate = TurnGate::new();
    assert_eq!(gate.step(f32::NAN, 10.0, false), None);
    assert_eq!(gate.timer_ticks(), 0);
    assert_eq!(gate.step(90.0, f32::NAN, false), None);
    assert_eq!(gate.timer_ticks(), 0);
    assert_eq!(gate.step(f32::INFINITY, 10.0, false), None);
    assert_eq!(gate.timer_ticks(), 0);
}

#[test]
fn a_181_degree_turn_picks_the_180_clip() {
    assert_eq!(
        delay_ticks(181.0),
        MAX_DELAY_TICKS,
        "181 clamps to the 180 key"
    );

    let mut gate = TurnGate::new();
    assert_eq!(hold(&mut gate, 181.0, 5.0, MAX_DELAY_TICKS - 1), None);
    assert_eq!(
        hold(&mut gate, 181.0, 5.0, 1),
        Some(Turn {
            clip: TurnClip::Right180,
            yaw_deg: 181.0,
        })
    );

    // The other shoulder, and the 90 clips on both.
    let mut left = TurnGate::new();
    assert_eq!(
        hold(&mut left, -181.0, 5.0, MAX_DELAY_TICKS),
        Some(Turn {
            clip: TurnClip::Left180,
            yaw_deg: -181.0,
        })
    );
    let mut right90 = TurnGate::new();
    assert_eq!(
        hold(&mut right90, 90.0, 5.0, delay_ticks(90.0)),
        Some(Turn {
            clip: TurnClip::Right90,
            yaw_deg: 90.0,
        })
    );
    let mut left90 = TurnGate::new();
    assert_eq!(
        hold(&mut left90, -90.0, 5.0, delay_ticks(90.0)),
        Some(Turn {
            clip: TurnClip::Left90,
            yaw_deg: -90.0,
        })
    );

    assert!(TurnClip::Left180.is_180() && TurnClip::Right180.is_180());
    assert!(!TurnClip::Right90.is_180());
    assert!(TurnClip::Left90.turns_left() && !TurnClip::Right90.turns_left());
}

#[test]
fn a_bigger_turn_waits_longer() {
    // ALS's `ViewYawAngleToActivationDelay{0.0f, 0.75f}` grows from 0 s at the
    // 45 degree threshold to 0.75 s at 180: the bigger the turn, the longer the
    // view must hold still before the body follows.
    assert_eq!(delay_ticks(VIEW_YAW_ANGLE_THRESHOLD_DEG), 0);
    assert_eq!(delay_ticks(46.0), 1);
    assert!(delay_ticks(46.0) < delay_ticks(90.0));
    assert!(delay_ticks(90.0) < delay_ticks(TURN_180_ANGLE_THRESHOLD_DEG));
    assert!(delay_ticks(TURN_180_ANGLE_THRESHOLD_DEG) < MAX_DELAY_TICKS);
    assert_eq!(delay_ticks(180.0), MAX_DELAY_TICKS);
    assert_eq!(
        delay_ticks(400.0),
        MAX_DELAY_TICKS,
        "past 180 the curve is flat"
    );

    // The 90 fires inside 15 ticks; the 180 is still waiting there.
    let mut small = TurnGate::new();
    assert!(hold(&mut small, 90.0, 5.0, 15).is_some());
    let mut big = TurnGate::new();
    assert_eq!(
        hold(&mut big, 180.0, 5.0, 15),
        None,
        "the big turn still waits"
    );
    assert!(
        big.timer_ticks() > 0,
        "and it is accumulating, not re-armed"
    );
}

#[test]
fn a_faster_view_flick_plays_the_turn_clip_faster_and_the_rate_never_pops() {
    // ALS `ReferenceViewYawSpeed{180, 460}` and `PlayRate{1.15, 3.0}`.
    let low = turn_rate(0.0, ROTATE_REFERENCE_DEG_S, ROTATE_PLAY_RATE);
    let at_low_ref = turn_rate(180.0, ROTATE_REFERENCE_DEG_S, ROTATE_PLAY_RATE);
    let at_high_ref = turn_rate(460.0, ROTATE_REFERENCE_DEG_S, ROTATE_PLAY_RATE);
    let over = turn_rate(900.0, ROTATE_REFERENCE_DEG_S, ROTATE_PLAY_RATE);
    assert!((low - 1.15).abs() < 1e-6);
    assert!((at_low_ref - 1.15).abs() < 1e-6);
    assert!((at_high_ref - 3.0).abs() < 1e-6);
    assert!((over - 3.0).abs() < 1e-6, "past the top the map is flat");
    let mid = turn_rate(320.0, ROTATE_REFERENCE_DEG_S, ROTATE_PLAY_RATE);
    assert!(
        (at_low_ref..=at_high_ref).contains(&mid),
        "the map is monotone"
    );
    // Total: a non-finite speed or an empty reference range reads the base.
    assert!((turn_rate(f32::NAN, ROTATE_REFERENCE_DEG_S, ROTATE_PLAY_RATE) - 1.15).abs() < 1e-6);
    assert!((turn_rate(300.0, [5.0, 5.0], ROTATE_PLAY_RATE) - 1.15).abs() < 1e-6);

    let mut rate = TurnRate::new();
    let mut last = rate.rate();
    for tick in 0..40 {
        let now = rate.step(460.0);
        assert!(
            (now - last).abs() <= RATE_MAX_STEP_PER_TICK + 1e-6,
            "tick {tick} popped the rate: {last} -> {now}"
        );
        last = now;
    }
    assert!(
        (rate.rate() - 3.0).abs() < 1e-6,
        "a held flick reaches the top rate"
    );
    for _ in 0..40 {
        rate.step(0.0);
    }
    assert!(
        (rate.rate() - 1.15).abs() < 1e-6,
        "and eases back to the base"
    );
    // A non-finite target cannot move it.
    let held = rate.step_towards(f32::NAN);
    assert!((held - rate.rate()).abs() < 1e-6);
}

#[test]
fn the_same_view_turns_twice_give_the_same_turns() {
    fn run() -> ([Option<Turn>; 4], f32) {
        let mut gate = TurnGate::new();
        let mut turns = [None; 4];
        let mut rate = TurnRate::new();
        let mut last = 0.0;
        for (slot, yaw) in turns.iter_mut().zip([60.0, 90.0, 130.0, 180.0]) {
            *slot = hold(&mut gate, yaw, 20.0, delay_ticks(yaw));
            gate.reset();
            last = rate.step(240.0);
        }
        (turns, last)
    }
    assert_eq!(run(), run());
    let (turns, _) = run();
    assert_eq!(turns[1].map(|t| t.clip), Some(TurnClip::Right90));
    assert_eq!(turns[2].map(|t| t.clip), Some(TurnClip::Right180));
}
