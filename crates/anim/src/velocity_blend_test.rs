//! velocity_blend tests: the four amplitudes, the half-life ease and the
//! blend-space pair they feed.

use super::*;
use crate::space::{bake, weights_into, BlendSpace, SpaceSample, WEIGHT_SCALE};

/// The one blend space these tests query: rest, a 2 m/s forward run and a
/// 2 m/s right strafe (Q10 speed, decidegrees heading).
fn gait_space() -> BlendSpace {
    bake(vec![
        SpaceSample::new(0, 0, 10).unwrap(),
        SpaceSample::new(2048, 0, 11).unwrap(),
        SpaceSample::new(2048, 900, 12).unwrap(),
    ])
    .unwrap()
}

fn weights(space: &BlendSpace, params: crate::space::SpaceParams) -> [u32; 3] {
    let mut out = [0u32; 3];
    weights_into(space, params, &mut out).unwrap();
    out
}

#[test]
fn a_pure_strafe_sets_only_the_right_axis() {
    let mut blend = VelocityBlend::new();
    let target = VelocityBlend::target([1.5, 0.0]);

    blend.snap(target);
    assert!(
        (blend.right() - 1.0).abs() < 1e-6,
        "the magnitude clamps to one"
    );
    assert_eq!(
        (blend.forward(), blend.backward(), blend.left()),
        (0.0, 0.0, 0.0)
    );

    // One half-life closes exactly half the gap; a long step closes most of it
    // and never overshoots.
    let mut eased = VelocityBlend::new();
    eased.step(target, DEFAULT_HALF_LIFE_S, DEFAULT_HALF_LIFE_S);
    assert!(
        (eased.right() - 0.5).abs() < 1e-5,
        "half the gap after one half-life"
    );
    eased.step(target, DEFAULT_HALF_LIFE_S, DEFAULT_HALF_LIFE_S * 9.0);
    assert!(eased.right() > 0.9, "nine tenths after a long step");
    assert!(eased.right() <= 1.0, "the ease never overshoots");
    assert!((eased.forward() + eased.backward() + eased.left()).abs() < 1e-6);

    // A zero half-life or a non-finite step snaps rather than easing.
    let mut snapped = VelocityBlend::new();
    snapped.step(target, 0.0, 0.016);
    assert!((snapped.right() - 1.0).abs() < 1e-6);
    let mut total = VelocityBlend::new();
    total.step(VelocityBlend::target([f32::NAN, f32::INFINITY]), 0.1, 0.016);
    assert_eq!(total, VelocityBlend::new(), "bad input reads as rest");
}

#[test]
fn a_diagonal_splits_both_axes() {
    let target = VelocityBlend::target([0.8, 0.8]);
    let mut blend = VelocityBlend::new();
    blend.snap(target);
    assert!((blend.right() - 0.8).abs() < 1e-6);
    assert!((blend.forward() - 0.8).abs() < 1e-6);
    assert_eq!((blend.left(), blend.backward()), (0.0, 0.0));

    // Each side is its own amplitude: running backwards and left is not the
    // same as running forwards and right.
    let back_left = VelocityBlend::target([-1.0, -1.0]);
    assert!((back_left.left() - 1.0).abs() < 1e-6);
    assert!((back_left.backward() - 1.0).abs() < 1e-6);
    assert_eq!((back_left.right(), back_left.forward()), (0.0, 0.0));
}

#[test]
fn the_same_axes_twice_pick_the_same_samples() {
    let space = gait_space();

    fn run(space: &BlendSpace) -> ([u32; 3], [u32; 3], i32, i32) {
        let mut blend = VelocityBlend::new();
        blend.snap(VelocityBlend::target([0.0, 1.0]));
        let forward = weights(space, blend.space_params(2.0).unwrap());
        let mut strafing = VelocityBlend::new();
        strafing.snap(VelocityBlend::target([1.0, 0.0]));
        let right = weights(space, strafing.space_params(2.0).unwrap());
        (
            forward,
            right,
            blend.space_params(2.0).unwrap().angle_q(),
            strafing.space_params(2.0).unwrap().angle_q(),
        )
    }

    let (forward, right, forward_deg, right_deg) = run(&space);
    assert_eq!(
        run(&space),
        (forward, right, forward_deg, right_deg),
        "x2 equal"
    );
    assert_eq!(
        forward,
        [0, WEIGHT_SCALE, 0],
        "a forward run takes the run sample"
    );
    assert_eq!(
        right,
        [0, 0, WEIGHT_SCALE],
        "a right strafe takes the strafe sample"
    );
    assert_eq!(forward_deg, 0);
    assert_eq!(right_deg, 900);

    // The diamond heading is exact on the four axes and close between them.
    assert_eq!(blend_deg([0.0, 1.0]), 0.0);
    assert_eq!(blend_deg([1.0, 0.0]), 90.0);
    assert_eq!(blend_deg([0.0, -1.0]), 180.0);
    assert_eq!(blend_deg([-1.0, 0.0]), -90.0);
    assert_eq!(blend_deg([1.0, 1.0]), 45.0);
    let off_axis = blend_deg([0.5, 1.0]).abs();
    assert!(
        (off_axis - 26.565).abs() <= DIAMOND_MAX_ERROR_DEG,
        "the heading stays inside its stated error: {off_axis}"
    );
}

/// Heading of one body-frame direction through the public blend pair.
fn blend_deg(vel: [f32; 2]) -> f32 {
    let mut blend = VelocityBlend::new();
    blend.snap(VelocityBlend::target(vel));
    f32::from(i16::try_from(blend.space_params(1.0).unwrap().angle_q()).unwrap()) / 10.0
}
