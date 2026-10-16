//! stage_bodies: the CPU pose step of the fighter look, with no GPU.
//!
//! SP61 splits `FighterLook::stage_bodies` into a CPU pose step (clip
//! sample, skin, `BodyTruth`) and the upload. This module is the CPU half:
//! small pure helpers with no `Presenter`, no device and no clock, so
//! `FighterLook::load`, `observe_tick` and `pose_frame` run headless.
//! The clip sample plus vertex skin stay in `fighter_look`; the truth rows
//! plus laws stay in `view_truth`. One home per concern: pose math lives
//! here, nowhere else.
//!
//! Units: meters in, meters out, radians for yaw. Determinism: pure `f32`
//! in, same `f32` out, no allocation. Errors: none; bad inputs read as
//! `None`, `false` or `0.0`, never a panic. Budget: no heap, a few dozen
//! flops per joint, so the joints-only frame stays far under the skin cost.

/// Sub-tick interpolation clamped to `0..=1`; non-finite reads as `0.0`.
///
/// Same rule as the frame pose: a bad alpha never moves a root.
#[must_use]
pub fn clamp_alpha(alpha: f32) -> f32 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Place one model-space joint through an AN1 row-vector root matrix:
/// turn about +Y by yaw, then move to the root position.
///
/// Layout matches `fighter_look::root_matrix`: the translation sits in the
/// last row, so `out[k] = p[0]*m[0][k] + p[1]*m[1][k] + p[2]*m[2][k] +
/// m[3][k]`. A non-finite component passes through untouched; callers that
/// fold bounds skip it in [`joints_bounds`].
#[must_use]
pub fn place_joint(point: [f32; 3], root: &[[f32; 4]; 4]) -> [f32; 3] {
    std::array::from_fn(|k| {
        point[0] * root[0][k] + point[1] * root[1][k] + point[2] * root[2][k] + root[3][k]
    })
}

/// Bounds of joints padded by the per-rig radius, with no vertex skin.
///
/// The joints-only mode: world-space joints (already placed through
/// [`place_joint`]) grown by `radius_m` on every axis. `None` when no
/// finite joint folds in. A non-finite or negative radius reads as `0.0`.
/// The padded joints bounds hold the full-vertex bounds within 10 cm; see
/// [`bounds_hold`] for the check.
#[must_use]
pub fn joints_bounds(joints: &[[f32; 3]], radius_m: f32) -> Option<([f32; 3], [f32; 3])> {
    let pad = if radius_m.is_finite() && radius_m > 0.0 {
        radius_m
    } else {
        0.0
    };
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut folded = false;
    for joint in joints {
        if !joint.iter().all(|v| v.is_finite()) {
            continue;
        }
        folded = true;
        for axis in 0..3 {
            lo[axis] = lo[axis].min(joint[axis] - pad);
            hi[axis] = hi[axis].max(joint[axis] + pad);
        }
    }
    folded.then_some((lo, hi))
}

/// True when `inner` sits inside `outer` grown by `pad_m` on every axis.
///
/// The joints-within-10cm check: the vertex bounds must sit inside the
/// joints bounds plus the pad. A non-finite face reads as outside (loud,
/// never a silent pass).
#[must_use]
pub fn bounds_hold(
    outer_min: [f32; 3],
    outer_max: [f32; 3],
    inner_min: [f32; 3],
    inner_max: [f32; 3],
    pad_m: f32,
) -> bool {
    outer_min
        .iter()
        .zip(inner_min.iter())
        .all(|(outer, inner)| *outer - pad_m <= *inner)
        && inner_max
            .iter()
            .zip(outer_max.iter())
            .all(|(inner, outer)| *inner <= *outer + pad_m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The AN1 row-vector root used across the look: turn about +Y, then
    /// move. Body forward +Z maps to `(sin yaw, 0, cos yaw)`.
    fn root_matrix(pos: [f32; 3], yaw: f32) -> [[f32; 4]; 4] {
        let (s, c) = yaw.sin_cos();
        [
            [c, 0.0, -s, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [s, 0.0, c, 0.0],
            [pos[0], pos[1], pos[2], 1.0],
        ]
    }

    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < 1e-6)
    }

    #[test]
    fn root_placement_maps_forward_onto_yaw_then_moves() {
        let root = root_matrix([1.0, 2.0, 3.0], std::f32::consts::FRAC_PI_2);
        let at = place_joint([0.0, 0.0, 1.0], &root);
        assert!(
            near(at, [2.0, 2.0, 3.0]),
            "row-vector root placement, got {at:?}"
        );
        let id = root_matrix([0.0; 3], 0.0);
        assert!(
            near(place_joint([1.0, 2.0, 3.0], &id), [1.0, 2.0, 3.0]),
            "identity root holds the joint"
        );
    }

    #[test]
    fn joints_bounds_pad_one_joint_on_every_axis() {
        let (lo, hi) = joints_bounds(&[[1.0, 2.0, 3.0]], 0.5).expect("one joint bounds");
        assert!(near(lo, [0.5, 1.5, 2.5]), "pad down, got {lo:?}");
        assert!(near(hi, [1.5, 2.5, 3.5]), "pad up, got {hi:?}");
    }

    #[test]
    fn joints_bounds_skip_nonfinite_and_empty() {
        assert_eq!(joints_bounds(&[], 0.4), None, "no joints, no bounds");
        assert_eq!(
            joints_bounds(&[[f32::NAN, 0.0, 0.0]], 0.4),
            None,
            "non-finite joint stays out"
        );
        let (lo, hi) = joints_bounds(
            &[[f32::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 2.0, 0.0]],
            0.0,
        )
        .expect("finite joints fold");
        assert!(near(lo, [0.0, 1.0, 0.0]), "got {lo:?}");
        assert!(near(hi, [0.0, 2.0, 0.0]), "got {hi:?}");
    }

    #[test]
    fn joints_bounds_hold_vertex_bounds_within_10cm_and_repeat() {
        // A standing body: joints up the middle, skin about 30 cm wide.
        let joints = [[0.0, 0.2, 0.0], [0.0, 1.0, 0.0], [0.0, 1.7, 0.0]];
        let vertex_min = [-0.3, 0.0, -0.3];
        let vertex_max = [0.3, 1.8, 0.3];
        let first = joints_bounds(&joints, 0.4).expect("joints bound");
        let second = joints_bounds(&joints, 0.4).expect("joints bound repeats");
        assert_eq!(first, second, "same joints give same bounds twice");
        assert!(
            bounds_hold(first.0, first.1, vertex_min, vertex_max, 0.10),
            "padded joints hold the skin within 10 cm: {first:?} vs {vertex_min:?}..{vertex_max:?}"
        );
        let tight = joints_bounds(&joints, 0.0).expect("unpadded bounds");
        assert!(
            !bounds_hold(tight.0, tight.1, vertex_min, vertex_max, 0.10),
            "no radius cannot hold 30 cm of skin"
        );
    }

    #[test]
    fn bounds_hold_pad_semantics_are_exact() {
        let outer_min = [0.0; 3];
        let outer_max = [1.0; 3];
        assert!(
            bounds_hold(outer_min, outer_max, [0.0; 3], [1.0; 3], 0.0),
            "exact fit holds with no pad"
        );
        assert!(
            !bounds_hold(outer_min, outer_max, [-0.001, 0.0, 0.0], [1.0; 3], 0.0),
            "1 mm outside fails with no pad"
        );
        assert!(
            bounds_hold(outer_min, outer_max, [-0.001, 0.0, 0.0], [1.0; 3], 0.01),
            "1 mm outside holds with a 1 cm pad"
        );
        assert!(
            !bounds_hold(outer_min, outer_max, [f32::NAN, 0.0, 0.0], [1.0; 3], 1.0),
            "a non-finite face never passes"
        );
    }

    #[test]
    fn clamp_alpha_holds_edges() {
        assert_eq!(clamp_alpha(0.5), 0.5);
        assert_eq!(clamp_alpha(2.0), 1.0);
        assert_eq!(clamp_alpha(-1.0), 0.0);
        assert_eq!(clamp_alpha(f32::NAN), 0.0, "non-finite never moves a root");
        assert_eq!(
            clamp_alpha(f32::INFINITY),
            0.0,
            "infinite never moves a root"
        );
    }
}
