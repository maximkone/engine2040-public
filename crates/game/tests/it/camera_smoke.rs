//! Camera smoke: the public framing API validates, frames and stays deterministic.
use engine_game::camera::camera_rig::FightCamSpring;
use engine_game::camera::{arena_boom_obstacles, frame_pair, DirectorParams, Vec3, ARENA_WALL_BOX};
use engine_game::camera_arm::{Aabb, FLOOR_LEN};

#[test]
fn defaults_validate() {
    DirectorParams::default()
        .validate()
        .expect("defaults valid");
    DirectorParams::arena().validate().expect("arena valid");
    DirectorParams::traversal()
        .validate()
        .expect("traversal valid");
}

#[test]
fn bad_fov_rejected() {
    let bad = DirectorParams {
        fov_y_rad: 0.0,
        ..DirectorParams::default()
    };
    assert!(bad.validate().is_err(), "zero fov fails");
    assert!(frame_pair(Vec3::new(1.0, 0.0, 0.0), None, &bad).is_err());
}

#[test]
fn frame_pair_deterministic() {
    let params = DirectorParams::default();
    let focus = Vec3::new(1.0, 0.0, 2.0);
    let other = Vec3::new(-1.5, 0.5, 0.0);
    let a = frame_pair(focus, Some(other), &params).expect("pair frames");
    let b = frame_pair(focus, Some(other), &params).expect("pair reframes");
    assert_eq!(a, b, "same inputs frame the same pair");
    assert_eq!(
        a.look_m,
        Vec3::new(-0.5, 0.25, 0.8),
        "look is 60% of the way from the attacker to the defender, at mean height"
    );
    let solo = frame_pair(focus, None, &params).expect("solo frames");
    assert_eq!(solo.look_m, focus, "solo look is the focus");
    assert_ne!(a.eye_m, solo.eye_m, "pair and solo differ");
}

fn springarm_params() -> DirectorParams {
    DirectorParams {
        boom_up: 0.0,
        boom_back: 1.0,
        ..DirectorParams::default()
    }
}

fn springarm_eye(look: Vec3, yaw: f32, dist: f32, obstacles: &[Aabb]) -> Vec3 {
    let mut spring = FightCamSpring::new(0.25).expect("spring builds");
    spring.reset(yaw, dist, look).expect("reset");
    spring.eye_m(&springarm_params(), obstacles, 1.0 / 60.0)
}

#[test]
fn camera_springarm_blocked_stops_short() {
    let look = Vec3::new(0.0, 1.5, 0.0);
    let boxes = arena_boom_obstacles();
    let first = springarm_eye(look, core::f32::consts::FRAC_PI_2, 5.0, &boxes);
    let second = springarm_eye(look, core::f32::consts::FRAC_PI_2, 5.0, &boxes);
    assert_eq!(first, second, "same sweep gives same eye twice");
    let len = (first - look).len();
    assert!((len - 2.69).abs() < 1e-4, "blocked eye 2.69 m, got {len}");
    assert!(
        first.x <= ARENA_WALL_BOX.min[0],
        "eye never passes the wall face"
    );
}

#[test]
fn camera_springarm_unblocked_keeps_full_length() {
    let look = Vec3::new(0.0, 1.5, 0.0);
    let empty: [Aabb; 0] = [];
    let first = springarm_eye(look, core::f32::consts::FRAC_PI_2, 5.0, &empty);
    let second = springarm_eye(look, core::f32::consts::FRAC_PI_2, 5.0, &empty);
    assert_eq!(first, second, "same sweep gives same eye twice");
    assert_eq!((first - look).len(), 5.0, "unblocked eye keeps 5.0 m");
}

#[test]
fn camera_springarm_inside_parks_at_floor() {
    let look = Vec3::new(4.0, 1.5, 0.0);
    let boxes = arena_boom_obstacles();
    let first = springarm_eye(look, core::f32::consts::FRAC_PI_2, 5.0, &boxes);
    let second = springarm_eye(look, core::f32::consts::FRAC_PI_2, 5.0, &boxes);
    assert_eq!(first, second, "same sweep gives same eye twice");
    let len = (first - look).len();
    assert!(
        (len - FLOOR_LEN).abs() < 1e-4,
        "inside parks at floor, got {len}"
    );
}
