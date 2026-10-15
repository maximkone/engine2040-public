//! Camera smoke: the public framing API validates, frames and stays deterministic.
use engine_game::camera::{frame_pair, DirectorParams, Vec3};

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
    let mut bad = DirectorParams::default();
    bad.fov_y_rad = 0.0;
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
        Vec3::new(-0.25, 0.25, 1.0),
        "look is the midpoint"
    );
    let solo = frame_pair(focus, None, &params).expect("solo frames");
    assert_eq!(solo.look_m, focus, "solo look is the focus");
    assert_ne!(a.eye_m, solo.eye_m, "pair and solo differ");
}
