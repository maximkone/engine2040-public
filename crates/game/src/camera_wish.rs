//! camera_wish: WASD and the stick, relative to where the camera looks.
//!
//! Owner: game lane, input mapping. Consumer: the `arena` window
//! (`bin/arena_frame`), which turns the local wish the bindings and the pad
//! emit into the world wish the sim takes. The bindings give `(forward,
//! right)` (`mx` forward, `mz` right, `-100..=100`), but the sim reads `mx`
//! and `mz` as world x and z (`engine_arena`'s `loco_wish`: world-planar), so
//! with the live over-the-shoulder shot W walked along +x whichever way the
//! picture pointed (2026-10-18: at spawn W ran into the pit wall). This module
//! rotates the local wish by the camera's ground-plane forward, so W walks
//! where the camera looks, S back, A and D strafe.
//!
//! Units: wishes are percent of top speed (`-100..=100`), headings are unit
//! `[x, z]` floor vectors, camera positions are meters. Determinism: pure
//! float math over the camera and the wish, then one integer quantize; the
//! sim sees only the integers, exactly as it sees a keyboard wish. Scope:
//! input mapping only. The sim, `MatchSpec`, the tape format and every hashed
//! byte are untouched, and a tape, the staged script and the spear demo keep
//! their world axes (the window applies this to the live keyboard and pad
//! path alone). Errors: none; a heading that cannot be read leaves the wish as
//! it came. Budget: a handful of float ops, no heap.

use crate::pad::quantize_axis;
use engine_input::fighter::WISH_RANGE;

/// Shortest floor-plane look (meters) that still names a heading: a camera
/// looking straight down or up has none.
const MIN_GROUND_M: f32 = 1.0e-3;

/// The camera's heading on the floor: the unit `[x, z]` from `eye` toward
/// `target` with the height dropped, so tilting the camera never turns the
/// walk. `None` when the camera looks (nearly) straight up or down, or a
/// coordinate is not finite; the caller keeps its last heading then. Pure.
#[must_use]
pub fn ground_forward(eye: [f32; 3], target: [f32; 3]) -> Option<[f32; 2]> {
    let (dx, dz) = (target[0] - eye[0], target[2] - eye[2]);
    let len = dx.hypot(dz);
    (len.is_finite() && len > MIN_GROUND_M).then(|| [dx / len, dz / len])
}

/// Turn a local wish `(forward, right)` into the world wish `(mx, mz)` for a
/// camera whose floor heading is `forward` (see [`ground_forward`]): W is the
/// heading itself, D the heading turned a quarter toward the camera's right
/// (`[-z, x]` in the sim's x east, z south floor, seen from above). The result
/// never leaves `-100..=100` and never exceeds full speed: a diagonal is
/// scaled to one full step, not 41 percent past it, and a half-deflected stick
/// stays half speed. Zero stays zero; a heading that is not finite returns the
/// wish unrotated. Pure over the heading and the wish.
#[must_use]
pub fn world_wish(forward: [f32; 2], local: (i8, i8)) -> (i8, i8) {
    if local == (0, 0) || !forward.iter().all(|c| c.is_finite()) {
        return local;
    }
    let full = f32::from(WISH_RANGE);
    let (ahead, side) = (f32::from(local.0), f32::from(local.1));
    let wx = ahead * forward[0] - side * forward[1];
    let wz = ahead * forward[1] + side * forward[0];
    let len = wx.hypot(wz);
    let k = if len > full { full / len } else { 1.0 };
    (quantize_axis(wx * k / full), quantize_axis(wz * k / full))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_camera_looking_down_plus_z_makes_w_go_plus_z() {
        let forward = ground_forward([0.0, 2.0, 0.0], [0.0, 2.0, 10.0]).expect("heading");
        assert_eq!(forward, [0.0, 1.0]);
        // W (forward 100), S, D (right 100), A: the sim's world axes.
        assert_eq!(world_wish(forward, (100, 0)), (0, 100), "W goes +z");
        assert_eq!(world_wish(forward, (-100, 0)), (0, -100), "S goes -z");
        // Looking down +z the camera's right hand is -x.
        assert_eq!(world_wish(forward, (0, 100)), (-100, 0), "D goes -x");
        assert_eq!(world_wish(forward, (0, -100)), (100, 0), "A goes +x");
    }

    #[test]
    fn the_four_quarter_turns_rotate_the_keys_and_x_forward_is_the_old_mapping() {
        // Looking down +x the rotation is the identity: the old world mapping
        // (W is +x, D is +z) is the special case of a camera facing +x. Only
        // a corner past full speed is scaled back to one full step.
        let plus_x = [1.0, 0.0];
        for local in [(100, 0), (-100, 0), (0, 100), (0, -100), (30, -70)] {
            assert_eq!(world_wish(plus_x, local), local, "{local:?}");
        }
        assert_eq!(
            world_wish(plus_x, (100, 100)),
            (71, 71),
            "W+D, not 141 percent"
        );
        assert_eq!(world_wish([0.0, -1.0], (100, 0)), (0, -100), "W looking -z");
        assert_eq!(world_wish([0.0, -1.0], (0, 100)), (100, 0), "D looking -z");
        assert_eq!(world_wish([-1.0, 0.0], (100, 0)), (-100, 0), "W looking -x");
        assert_eq!(world_wish([-1.0, 0.0], (0, 100)), (0, -100), "D looking -x");
    }

    #[test]
    fn a_diagonal_is_one_full_step_and_a_half_stick_stays_half() {
        let forward = [0.0, 1.0];
        let (mx, mz) = world_wish(forward, (100, 100));
        let len = f32::from(mx).hypot(f32::from(mz));
        assert!(
            (len - 100.0).abs() <= 1.5,
            "W+D is full speed, not 141: {len}"
        );
        assert!(
            mx < 0 && mz > 0,
            "W+D looking +z goes forward and right: {mx},{mz}"
        );
        // A half-deflected stick (forward 50) keeps its half speed.
        assert_eq!(world_wish([1.0, 0.0], (50, 0)), (50, 0));
        assert_eq!(world_wish([0.0, 1.0], (50, 0)), (0, 50));
    }

    #[test]
    fn no_heading_and_no_wish_leave_the_wish_alone() {
        assert_eq!(world_wish([0.0, 1.0], (0, 0)), (0, 0), "rest stays rest");
        // A heading that cannot be read never zeroes or scrambles the wish.
        assert_eq!(world_wish([f32::NAN, 0.0], (100, 0)), (100, 0));
        assert_eq!(world_wish([0.0, f32::INFINITY], (0, -100)), (0, -100));
    }

    #[test]
    fn every_wish_and_heading_stays_inside_the_wish_range() {
        for step in 0..64u8 {
            let turn = f32::from(step) * core::f32::consts::TAU / 64.0;
            let forward = [turn.sin(), turn.cos()];
            for ahead in (-100i8..=100).step_by(25) {
                for side in (-100i8..=100).step_by(25) {
                    let (mx, mz) = world_wish(forward, (ahead, side));
                    let len = f32::from(mx).hypot(f32::from(mz));
                    assert!(len <= 100.5 + 1.0, "{ahead},{side} at {turn}: {mx},{mz}");
                    assert!((-100..=100).contains(&mx) && (-100..=100).contains(&mz));
                }
            }
        }
    }

    #[test]
    fn the_heading_drops_the_tilt_and_refuses_a_camera_with_none() {
        // Looking down at 45 degrees along +x: the heading is +x all the same.
        let tilted = ground_forward([0.0, 5.0, 0.0], [5.0, 0.0, 0.0]).expect("heading");
        assert_eq!(tilted, [1.0, 0.0]);
        // Straight down has no heading; so does a broken coordinate.
        assert_eq!(ground_forward([1.0, 5.0, 1.0], [1.0, 0.0, 1.0]), None);
        assert_eq!(ground_forward([0.0, 1.0, 0.0], [f32::NAN, 1.0, 1.0]), None);
        assert_eq!(
            ground_forward([0.0, 1.0, 0.0], [1.0, 1.0, f32::INFINITY]),
            None
        );
    }

    /// The handedness against the renderer's own view matrix (the `v1` camera
    /// needs the `app` leg, so the headless build keeps the formula tests above).
    #[cfg(feature = "app")]
    mod renderer_truth {
        use super::*;
        use engine_render::v1::Camera;

        /// A level camera at a fixed spot looking `heading_deg` around from +z
        /// toward +x (0 looks down +z, 90 down +x).
        fn camera_facing(heading_deg: f32) -> Camera {
            let (sin, cos) = heading_deg.to_radians().sin_cos();
            let eye = [3.0, 1.8, -2.0];
            let target = [eye[0] + sin * 10.0, eye[1], eye[2] + cos * 10.0];
            Camera::new(eye, target, [0.0, 1.0, 0.0], 60.0f32.to_radians())
        }

        /// Where the floor point `at` lands for `camera`: `(right, ahead)` in
        /// meters along the camera's own axes (the renderer's view matrix, not
        /// this module's formula).
        fn on_screen(camera: &Camera, at: [f32; 2]) -> (f32, f32) {
            let view = camera.view_matrix().expect("a level camera builds");
            let mut point = camera.eye;
            (point.x, point.z) = (at[0], at[1]);
            let local = view.transform_point3(point);
            (local.x, -local.z)
        }

        #[test]
        fn all_eight_directions_land_where_the_renderer_says_at_any_heading() {
            // Local wishes as the bindings emit them: (forward, right) from W S
            // A D and their four corners. The expected screen side comes from
            // the renderer's own view matrix, so this pins the handedness.
            let keys: [(&str, (i8, i8)); 8] = [
                ("W", (100, 0)),
                ("S", (-100, 0)),
                ("D", (0, 100)),
                ("A", (0, -100)),
                ("W+D", (100, 100)),
                ("W+A", (100, -100)),
                ("S+D", (-100, 100)),
                ("S+A", (-100, -100)),
            ];
            for heading_deg in [0.0, 20.0, 45.0, 90.0, 135.0, 180.0, -90.0, -135.0, 270.0] {
                let camera = camera_facing(heading_deg);
                let forward = ground_forward(camera.eye.to_array(), camera.target.to_array())
                    .expect("level camera");
                let eye = [camera.eye.x, camera.eye.z];
                for (name, local) in keys {
                    let (mx, mz) = world_wish(forward, local);
                    // One full-speed step is 4 m here; 100 is full speed.
                    let step = [f32::from(mx) / 100.0 * 4.0, f32::from(mz) / 100.0 * 4.0];
                    let (right, ahead) = on_screen(&camera, [eye[0] + step[0], eye[1] + step[1]]);
                    let moved = right.hypot(ahead);
                    assert!(
                        (moved - 4.0).abs() < 0.2,
                        "{name} at {heading_deg}: one full step is 4 m, got {moved}"
                    );
                    let (want_right, want_ahead) = (f32::from(local.1), f32::from(local.0));
                    let scale = want_right.hypot(want_ahead);
                    assert!(
                        (right / moved - want_right / scale).abs() < 0.05,
                        "{name} at {heading_deg}: right {right} of {moved}"
                    );
                    assert!(
                        (ahead / moved - want_ahead / scale).abs() < 0.05,
                        "{name} at {heading_deg}: ahead {ahead} of {moved}"
                    );
                }
            }
        }
    }
}
