//! Priority stack picks the live shot and eases the handoff twice alike.
use engine_game::camera::{frame_pair, DirectorParams, Vec3};
use engine_game::camera_stack::{Shot, Stack, StackError, HANDOFF_TICKS};

fn stack_all_live() -> Stack {
    let mut stack = Stack::new();
    for shot in [
        Shot::new(0, 0, true),
        Shot::new(1, 5, true),
        Shot::new(2, 10, true),
    ] {
        stack.push(shot).expect("table fits three shots");
    }
    stack
}

fn frames_from_pair() -> (
    engine_game::camera::CameraFrame,
    engine_game::camera::CameraFrame,
) {
    let focus = Vec3::new(1.0, 0.0, 2.0);
    let other = Vec3::new(-1.5, 0.5, 0.0);
    let near = frame_pair(focus, Some(other), &DirectorParams::default()).expect("pair frames");
    let far_params = DirectorParams {
        base_dist_m: 8.0,
        ..DirectorParams::default()
    };
    let far = frame_pair(focus, Some(other), &far_params).expect("wide pair frames");
    let from = engine_game::camera::CameraFrame {
        tick: 0,
        eye_m: near.eye_m,
        look_m: near.look_m,
        shake_m: Vec3::ZERO,
        lock: Some(1),
        trauma_permille: 0,
    };
    let to = engine_game::camera::CameraFrame {
        tick: HANDOFF_TICKS,
        eye_m: far.eye_m,
        look_m: far.look_m,
        shake_m: Vec3::ZERO,
        lock: Some(1),
        trauma_permille: 0,
    };
    assert_ne!(from.eye_m, to.eye_m, "shots frame apart so the blend moves");
    (from, to)
}

#[test]
fn priority_pick_falls_back_in_table_order() {
    assert_eq!(stack_all_live().pick().expect("top shot wins").id, 2);
    let mut no_top = Stack::new();
    for shot in [
        Shot::new(0, 0, true),
        Shot::new(1, 5, true),
        Shot::new(2, 10, false),
    ] {
        no_top.push(shot).expect("table fits");
    }
    assert_eq!(no_top.pick().expect("falls back to next live").id, 1);
    let mut tied = Stack::new();
    for shot in [Shot::new(0, 7, true), Shot::new(1, 7, true)] {
        tied.push(shot).expect("table fits");
    }
    assert_eq!(tied.pick().expect("tie keeps order").id, 0);
    let mut dead = Stack::new();
    for shot in [
        Shot::new(0, 0, false),
        Shot::new(1, 5, false),
        Shot::new(2, 10, false),
    ] {
        dead.push(shot).expect("table fits");
    }
    assert_eq!(
        dead.pick().expect_err("dead stack refuses"),
        StackError::Dead
    );
}

#[test]
fn handoff_midpoint_eases_and_replays() {
    let (from, to) = frames_from_pair();
    let mid = Stack::blend(&from, &to, HANDOFF_TICKS / 2);
    let weight = Stack::blend_weight(HANDOFF_TICKS / 2);
    assert!(
        (weight as i32 - 500).abs() <= 8,
        "mid weight 500 +- 8, got {weight}"
    );
    let full = (to.eye_m - from.eye_m).len();
    let part = (mid.eye_m - from.eye_m).len();
    assert!(full > 1e-6, "shots frame apart");
    let permille = part / full * 1000.0;
    assert!(
        (permille - 500.0).abs() <= 8.0,
        "mid eye 500 +- 8 permille, got {permille}"
    );
    let again = Stack::blend(&from, &to, HANDOFF_TICKS / 2);
    assert_eq!(mid.to_bytes(), again.to_bytes(), "frame bytes replay twice");
}
