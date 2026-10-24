//! Tests of `camera_stack`: a higher-priority shot takes over after its blend ticks,
//! the eye holds inside the dead zone, a shot gone stale drops back to the next
//! priority, a body outside the zone pulls the eye, and the same offers twice give
//! the same eye.

use super::*;
use crate::camera_frame::{read, Rig, Subject};

/// One 60 Hz frame.
const TICK: f32 = 1.0 / 60.0;

fn rig(eye: [f32; 3], target: [f32; 3]) -> Rig {
    Rig {
        eye,
        target,
        fov_y: 0.9,
        aspect: 16.0 / 9.0,
        near: 0.1,
    }
}

fn shot(key: u8, priority: i16, rig: Rig) -> ShotOffer {
    ShotOffer { key, priority, rig }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// True when the bodies' middle in `pose`'s picture sits inside [`DEAD_ZONE`].
fn in_dead_zone(pose: &Rig, subjects: &[Subject]) -> bool {
    let read = read(pose, subjects).expect("the bodies are in front of the eye");
    let (cx, cy) = (
        (read.union.x0 + read.union.x1) * 0.5,
        (read.union.y0 + read.union.y1) * 0.5,
    );
    DEAD_ZONE.holds(cx, cy)
}

#[test]
fn a_higher_priority_shot_takes_over_after_its_blend_ticks() {
    let low = shot(1, 10, rig([0.0, 2.0, -6.0], [0.0, 1.0, 0.0]));
    let high = shot(2, 20, rig([4.0, 2.0, -4.0], [0.0, 1.0, 0.0]));
    let mut stack = CameraStack::default();
    assert_eq!(
        stack.step(0, TICK, &[low], &[]),
        Some(low.rig),
        "the first shot is the eye at once, with no cut to ease"
    );
    // The high shot joins the shelf: the eye cuts to it, eased over its blend ticks.
    let first = stack
        .step(1, TICK, &[low, high], &[])
        .expect("a shot is live");
    assert!(stack.cut.is_some(), "the high shot starts a cut");
    assert_ne!(first.eye, high.rig.eye, "the cut eases, it does not cut");
    let mut steps = 1;
    let mut tick = 2;
    let mut last = dist(first.eye, high.rig.eye);
    while stack.cut.is_some() {
        let pose = stack
            .step(tick, TICK, &[low, high], &[])
            .expect("a shot is live");
        steps += 1;
        tick += 1;
        if stack.cut.is_some() {
            assert_ne!(
                pose.eye, high.rig.eye,
                "not on the shot before its blend ends"
            );
            let now = dist(pose.eye, high.rig.eye);
            assert!(
                now < last,
                "the eye keeps closing on the shot: {now} < {last}"
            );
            last = now;
        }
    }
    assert_eq!(
        steps,
        u32::from(BLEND_TICKS),
        "the cut takes its blend ticks, no more"
    );
    assert_eq!(stack.active, Some(2), "the high shot is the active one");
    assert_eq!(
        stack.pose,
        Some(high.rig),
        "the cut landed on the shot exactly"
    );
}

#[test]
fn the_eye_holds_while_the_framed_bodies_stay_in_the_dead_zone() {
    let body = Subject::standing([0.0, 0.0, 0.0]);
    let home = rig([0.0, 1.8, -6.0], [0.0, 1.0, 0.0]);
    let mut stack = CameraStack::default();
    let held = stack
        .step(0, TICK, &[shot(1, 10, home)], &[body])
        .expect("a shot is live");
    assert!(
        in_dead_zone(&held, &[body]),
        "the body starts inside the dead zone"
    );
    let mut tick = 1;
    for drift in [0.03f32, 0.06, 0.09, 0.12, 0.15, 0.18, 0.21, 0.24] {
        // The shot's eye drifts with the fight; the body is still inside the box, so
        // the eye stays where it was, bit for bit.
        let drifted = shot(1, 10, rig([drift, 1.8, -6.0], [drift, 1.0, 0.0]));
        let pose = stack
            .step(tick, TICK, &[drifted], &[body])
            .expect("a shot is live");
        tick += 1;
        assert_eq!(pose, held, "the eye holds inside the dead zone");
        assert!(in_dead_zone(&pose, &[body]));
    }
    assert_eq!(stack.holds, 8, "every held tick is counted");
    assert_eq!(stack.moves, 0, "nothing pulled the eye");
}

#[test]
fn a_shot_gone_stale_drops_back_to_the_next_priority() {
    let high = shot(7, 50, rig([0.0, 3.0, -8.0], [0.0, 1.0, 0.0]));
    let low = shot(8, 10, rig([2.0, 1.6, -4.0], [0.0, 1.0, 0.0]));
    let mut stack = CameraStack::default();
    assert_eq!(stack.step(0, TICK, &[high, low], &[]), Some(high.rig));
    assert_eq!(stack.active, Some(7));
    // The director lets go of the high shot: only the low one is offered from now on.
    for tick in 1..=STALE_TICKS {
        stack.step(tick, TICK, &[low], &[]);
        assert_eq!(
            stack.active,
            Some(7),
            "tick {tick}: the stale allowance holds it"
        );
        assert_eq!(
            stack.pose,
            Some(high.rig),
            "the eye holds while nothing asks it to move"
        );
    }
    let dropped = stack
        .step(STALE_TICKS + 1, TICK, &[low], &[])
        .expect("the low shot is live");
    assert_eq!(
        stack.active,
        Some(8),
        "past the allowance the next priority wins"
    );
    assert_ne!(dropped.eye, low.rig.eye, "the drop eases, it does not cut");
    let mut tick = STALE_TICKS + 2;
    while stack.cut.is_some() {
        stack.step(tick, TICK, &[low], &[]);
        tick += 1;
    }
    assert_eq!(stack.pose, Some(low.rig), "the eye lands on the next shot");
    assert!(
        tick <= STALE_TICKS + 2 + u32::from(BLEND_TICKS),
        "the drop takes its blend ticks, no more"
    );
}

#[test]
fn the_eye_follows_once_the_bodies_leave_the_dead_zone() {
    let body = Subject::standing([0.0, 0.0, 0.0]);
    let outside = Subject::standing([5.0, 0.0, 0.0]);
    let home = shot(1, 10, rig([0.0, 1.8, -6.0], [0.0, 1.0, 0.0]));
    let away = shot(1, 10, rig([5.0, 1.8, -6.0], [5.0, 1.0, 0.0]));
    let mut stack = CameraStack::default();
    stack
        .step(0, TICK, &[home], &[body])
        .expect("a shot is live");
    assert!(!CameraStack::breached(
        &stack.pose.expect("held a pose"),
        &[body]
    ));
    // The body walks 5 m clear of the box: the eye eases toward the shot framing it.
    let mut last = 5.0;
    for tick in 1..=6 {
        let pose = stack
            .step(tick, TICK, &[away], &[outside])
            .expect("a shot is live");
        let now = (pose.eye[0] - 5.0).abs();
        assert!(now < last, "tick {tick}: the eye eases toward the body");
        last = now;
    }
    assert!(last < 2.0, "6 ticks close most of the gap: {last} m left");
    assert_eq!(stack.moves, 6, "every moving tick is counted");
    // Once the body is back inside the box the eye holds again, no breathing.
    let mut tick = 7;
    while stack.holds == 0 && tick <= 60 {
        stack
            .step(tick, TICK, &[away], &[outside])
            .expect("a shot is live");
        tick += 1;
    }
    let held = stack.pose.expect("a pose");
    assert!(stack.holds > 0, "the dead zone takes the eye back");
    assert!(in_dead_zone(&held, &[outside]));
    for tick in tick..=tick + 5 {
        let pose = stack
            .step(tick, TICK, &[away], &[outside])
            .expect("a shot is live");
        assert_eq!(pose, held, "the eye holds once the body is inside");
    }
}

#[test]
fn the_same_offers_twice_give_the_same_eye() {
    let body = Subject::standing([0.0, 0.0, 0.0]);
    let mut script = Vec::new();
    for n in 0..40u32 {
        let drift = f32::from(u8::try_from(n % 8).expect("8 steps")) * 0.05;
        let (key, priority) = if n < 20 { (1, 10) } else { (2, 20) };
        script.push(shot(
            key,
            priority,
            rig([drift, 2.0, -6.0], [drift, 1.0, 0.0]),
        ));
    }
    let run = |script: &[ShotOffer]| {
        let mut stack = CameraStack::default();
        let mut poses = Vec::new();
        for (tick, offer) in script.iter().enumerate() {
            let tick = u32::try_from(tick).expect("short script");
            poses.push(stack.step(tick, TICK, &[*offer], &[body]));
        }
        (poses, stack.holds, stack.moves)
    };
    assert_eq!(run(&script), run(&script), "same offers, same eye x2");
}

#[test]
fn a_shot_with_no_finite_eye_never_joins_the_shelf() {
    let mut stack = CameraStack::default();
    let bad = shot(1, 99, rig([f32::NAN, 2.0, -6.0], [0.0, 1.0, 0.0]));
    assert_eq!(
        stack.step(0, TICK, &[bad], &[]),
        None,
        "a shot with no finite eye is refused, not shown"
    );
    assert_eq!(stack.shelf.iter().flatten().count(), 0);
    let good = shot(2, 1, rig([0.0, 2.0, -6.0], [0.0, 1.0, 0.0]));
    assert_eq!(stack.step(1, TICK, &[good], &[]), Some(good.rig));
}
