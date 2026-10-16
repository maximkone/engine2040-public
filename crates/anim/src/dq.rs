//! dq: dual-quaternion skinning option for twist-heavy joints (shoulders,
//! upper arms). Linear blend skinning collapses volume near a half-turn
//! twist (candy-wrapper crease); blending rigid transforms as unit dual
//! quaternions follows the shortest screw motion and keeps the volume.
//! Std-only; deterministic (`+ - * /`, `sqrt` only, skeleton order, no
//! clock); the blend/compose path allocates nothing (caller-owned buffers).
//! Units: meters, unit quaternions `[x, y, z, w]`. Errors: [`DqError`].
//! Consumer: render `v1_skin::skinning_uniform` accepts either layout via
//! [`skinning_with_mode`]; [`SkinMode::Linear`] delegates to
//! `contract::skinning_matrices`, so the default linear path is bit-identical.

use core::fmt;

use engine_core::quat_mul;

use super::contract::{skinning_matrices, validate_pose, AnimError, LocalPose, Skeleton};

/// Unit dual quaternion: a rigid transform as `real + ε dual`, with
/// `real` a unit rotation and `dual = 0.5 * t * real` for translation `t`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DualQuat {
    /// Unit rotation quaternion `[x, y, z, w]`.
    pub real: [f32; 4],
    /// Dual part encoding translation, `[x, y, z, w]`.
    pub dual: [f32; 4],
}

impl DualQuat {
    /// Identity transform: unit real part, zero dual part.
    pub const IDENTITY: DualQuat = DualQuat {
        real: [0.0, 0.0, 0.0, 1.0],
        dual: [0.0, 0.0, 0.0, 0.0],
    };
}

/// Dual-quaternion failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DqError {
    /// Nothing to blend (empty stack with no weights).
    EmptyStack,
    /// Stack and weight counts differ; stack + weight counts carried.
    LengthMismatch {
        /// Dual-quaternion count.
        expected: usize,
        /// Weight count.
        got: usize,
    },
    /// A blend weight is not finite; weight index carried.
    NonFiniteWeight(usize),
}

impl fmt::Display for DqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DqError::EmptyStack => write!(f, "nothing to blend"),
            DqError::LengthMismatch { expected, got } => {
                write!(f, "stack/weight count differ: {expected} vs {got}")
            }
            DqError::NonFiniteWeight(index) => {
                write!(f, "blend weight {index} is not finite")
            }
        }
    }
}

/// Skinning path selector. `Linear` is the default path, bit-identical
/// to `contract::skinning_matrices`; `DualQuaternion` is the opt-in twist
/// path with the same matrix layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SkinMode {
    /// Default linear blend skinning (unchanged default).
    Linear,
    /// Dual-quaternion blend skinning (shoulders/twists option).
    DualQuaternion,
}

fn quat_conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

fn quat_norm(q: [f32; 4]) -> f32 {
    (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt()
}

fn rotate(point: &[f32; 3], quat: &[f32; 4]) -> [f32; 3] {
    let q = quat_mul(*quat, [point[0], point[1], point[2], 0.0]);
    let r = quat_mul(q, quat_conj(*quat));
    [r[0], r[1], r[2]]
}

/// Build the unit dual quaternion for a rigid joint transform: rotation
/// `rotation` (normalized here, so validated poses within tolerance stay
/// exact) plus translation `translation` in meters.
#[must_use]
pub fn rigid_to_dual(translation: [f32; 3], rotation: [f32; 4]) -> DualQuat {
    let norm = quat_norm(rotation);
    let real = if norm.is_finite() && norm > 0.0 {
        [
            rotation[0] / norm,
            rotation[1] / norm,
            rotation[2] / norm,
            rotation[3] / norm,
        ]
    } else {
        DualQuat::IDENTITY.real
    };
    let t = [translation[0], translation[1], translation[2], 0.0];
    let half = quat_mul(t, real);
    DualQuat {
        real,
        dual: [0.5 * half[0], 0.5 * half[1], 0.5 * half[2], 0.5 * half[3]],
    }
}

/// Normalize a dual quaternion by its real-part length; a zero or
/// non-finite real part yields identity (only reachable from invalid
/// input, never from validated poses).
#[must_use]
pub fn dual_normalize(dq: &DualQuat) -> DualQuat {
    let norm = quat_norm(dq.real);
    if !norm.is_finite() || norm <= 0.0 {
        return DualQuat::IDENTITY;
    }
    DualQuat {
        real: [
            dq.real[0] / norm,
            dq.real[1] / norm,
            dq.real[2] / norm,
            dq.real[3] / norm,
        ],
        dual: [
            dq.dual[0] / norm,
            dq.dual[1] / norm,
            dq.dual[2] / norm,
            dq.dual[3] / norm,
        ],
    }
}

/// Blend a stack of unit dual quaternions into `out`: antipodal correction
/// against the first item (negate when the real-part dot is negative, so
/// the blend follows the short screw), weighted sum, then normalize.
/// Allocates nothing; `items` and `weights` must share a length.
pub fn blend_into(out: &mut DualQuat, items: &[DualQuat], weights: &[f32]) -> Result<(), DqError> {
    let Some(first) = items.first() else {
        return Err(DqError::EmptyStack);
    };
    if items.len() != weights.len() {
        return Err(DqError::LengthMismatch {
            expected: items.len(),
            got: weights.len(),
        });
    }
    let mut real = [0.0f32; 4];
    let mut dual = [0.0f32; 4];
    for (index, (item, weight)) in items.iter().zip(weights.iter()).enumerate() {
        if !weight.is_finite() {
            return Err(DqError::NonFiniteWeight(index));
        }
        let dot = first.real[0] * item.real[0]
            + first.real[1] * item.real[1]
            + first.real[2] * item.real[2]
            + first.real[3] * item.real[3];
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        for lane in 0..4 {
            real[lane] += sign * weight * item.real[lane];
            dual[lane] += sign * weight * item.dual[lane];
        }
    }
    *out = dual_normalize(&DualQuat { real, dual });
    Ok(())
}

/// Compose parent and local unit dual quaternions into a world transform
/// (`parent * local`). Inputs from [`rigid_to_dual`] or [`blend_into`] are
/// unit, so the result is unit up to float rounding; no allocation.
#[must_use]
pub fn dual_concat(parent: &DualQuat, local: &DualQuat) -> DualQuat {
    let real = quat_mul(parent.real, local.real);
    let a = quat_mul(parent.real, local.dual);
    let b = quat_mul(parent.dual, local.real);
    DualQuat {
        real,
        dual: [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]],
    }
}

/// Convert a unit dual quaternion to the render skinning matrix layout:
/// row-major `[[f32; 4]; 4]` with translation in the last row, so render
/// `v1_skin::skinning_uniform` accepts it unchanged. Normalizes first, so
/// blended inputs stay rigid; allocates nothing.
#[must_use]
pub fn dual_to_matrix(dq: &DualQuat) -> [[f32; 4]; 4] {
    let unit = dual_normalize(dq);
    let x = rotate(&[1.0, 0.0, 0.0], &unit.real);
    let y = rotate(&[0.0, 1.0, 0.0], &unit.real);
    let z = rotate(&[0.0, 0.0, 1.0], &unit.real);
    let t = quat_mul(unit.dual, quat_conj(unit.real));
    [
        [x[0], x[1], x[2], 0.0],
        [y[0], y[1], y[2], 0.0],
        [z[0], z[1], z[2], 0.0],
        [2.0 * t[0], 2.0 * t[1], 2.0 * t[2], 1.0],
    ]
}

/// Compose a pose into world-space unit dual quaternions, parents first in
/// skeleton order, writing into caller-owned `out` (no allocation).
/// Validation matches `contract::skinning_matrices`, so failures agree.
pub fn skinning_duals_into(
    skeleton: &Skeleton,
    pose: &LocalPose,
    out: &mut [DualQuat],
) -> Result<(), AnimError> {
    validate_pose(skeleton, pose)?;
    if out.len() != skeleton.len() {
        return Err(AnimError::LengthMismatch {
            expected: skeleton.len(),
            got: out.len(),
        });
    }
    for (index, joint) in skeleton.joints.iter().enumerate() {
        let local = rigid_to_dual(pose.positions[index], pose.rotations[index]);
        out[index] = match joint.parent {
            Some(parent) => dual_concat(&out[parent], &local),
            None => local,
        };
    }
    Ok(())
}

/// Dual-quaternion skinning matrices in the render layout (one matrix per
/// joint, translation in the last row). Allocates the output vec once;
/// per-tick callers should reuse [`skinning_duals_into`] plus
/// [`dual_to_matrix`] instead.
pub fn skinning_matrices_dq(
    skeleton: &Skeleton,
    pose: &LocalPose,
) -> Result<Vec<[[f32; 4]; 4]>, AnimError> {
    validate_pose(skeleton, pose)?;
    let mut duals = Vec::with_capacity(skeleton.len());
    duals.resize(skeleton.len(), DualQuat::IDENTITY);
    skinning_duals_into(skeleton, pose, &mut duals)?;
    Ok(duals.iter().map(dual_to_matrix).collect())
}

/// Skin with the selected path. `Linear` delegates to
/// `contract::skinning_matrices` (bit-identical default); `DualQuaternion`
/// blends through unit dual quaternions for twist-heavy joints.
pub fn skinning_with_mode(
    skeleton: &Skeleton,
    pose: &LocalPose,
    mode: SkinMode,
) -> Result<Vec<[[f32; 4]; 4]>, AnimError> {
    match mode {
        SkinMode::Linear => skinning_matrices(skeleton, pose),
        SkinMode::DualQuaternion => skinning_matrices_dq(skeleton, pose),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> (Skeleton, LocalPose) {
        let skeleton = Skeleton {
            joints: vec![
                super::super::contract::Joint {
                    name: "root".into(),
                    parent: None,
                },
                super::super::contract::Joint {
                    name: "shoulder".into(),
                    parent: Some(0),
                },
                super::super::contract::Joint {
                    name: "arm".into(),
                    parent: Some(1),
                },
            ],
        };
        let pose = LocalPose {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.5, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 3],
        };
        (skeleton, pose)
    }

    /// Row-vector transform matching the render layout (translation last row).
    fn xform(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
        [
            v[0] * m[0][0] + v[1] * m[1][0] + v[2] * m[2][0] + m[3][0],
            v[0] * m[0][1] + v[1] * m[1][1] + v[2] * m[2][1] + m[3][1],
            v[0] * m[0][2] + v[1] * m[1][2] + v[2] * m[2][2] + m[3][2],
        ]
    }

    fn len(v: [f32; 3]) -> f32 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    #[test]
    fn dq_option() {
        // Candy-wrapper fixture: a vertex one meter off the twist axis,
        // bound half to identity and half to a half-turn about X.
        let straight = rigid_to_dual([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
        let twisted = rigid_to_dual([0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]);
        let items = [straight, twisted];
        let weights = [0.5, 0.5];
        let mut blended = DualQuat::IDENTITY;
        blend_into(&mut blended, &items, &weights).unwrap();

        // Linear matrix blend collapses: the crease this option removes.
        let linear = {
            let a = dual_to_matrix(&straight);
            let b = dual_to_matrix(&twisted);
            let mut m = [[0.0f32; 4]; 4];
            for row in 0..4 {
                for col in 0..4 {
                    m[row][col] = 0.5 * a[row][col] + 0.5 * b[row][col];
                }
            }
            m
        };
        let creased = xform(&linear, [0.0, 1.0, 0.0]);
        assert!(
            len(creased) < 0.05,
            "fixture must crease linearly, got {creased:?}"
        );

        // Dual-quaternion blend keeps the volume: a quarter-turn about X.
        let kept = xform(&dual_to_matrix(&blended), [0.0, 1.0, 0.0]);
        assert!(
            (len(kept) - 1.0).abs() < 1e-5,
            "DQ blend must keep unit length, got {kept:?}"
        );
        assert!(
            (kept[0]).abs() < 1e-4 && (kept[1]).abs() < 1e-4 && (kept[2] - 1.0).abs() < 1e-4,
            "DQ blend must quarter-turn [0,1,0] to [0,0,1], got {kept:?}"
        );

        // Default path is untouched: Linear delegates bit-identically.
        let (skeleton, pose) = chain();
        assert_eq!(
            skinning_with_mode(&skeleton, &pose, SkinMode::Linear).unwrap(),
            skinning_matrices(&skeleton, &pose).unwrap()
        );

        // Same inputs twice give the same dual matrices.
        assert_eq!(
            skinning_matrices_dq(&skeleton, &pose).unwrap(),
            skinning_matrices_dq(&skeleton, &pose).unwrap()
        );
    }

    #[test]
    fn dq_option_matches_linear_when_untwisted() {
        let (skeleton, pose) = chain();
        let linear = skinning_matrices(&skeleton, &pose).unwrap();
        let dual = skinning_matrices_dq(&skeleton, &pose).unwrap();
        assert_eq!(linear.len(), dual.len());
        let mut worst = 0.0f32;
        for (a, b) in linear.iter().zip(dual.iter()) {
            for row in 0..4 {
                for col in 0..4 {
                    worst = worst.max((a[row][col] - b[row][col]).abs());
                }
            }
        }
        assert!(
            worst < 1e-5,
            "untwisted DQ must match linear, worst {worst}"
        );
        // Root at origin, shoulder/arm offset +1 m / +0.5 m in Y each.
        assert!((dual[2][3][1] - 1.5).abs() < 1e-5);
    }

    #[test]
    fn dq_option_rejects_bad_input() {
        let empty = Skeleton::default();
        let pose = LocalPose {
            positions: vec![],
            rotations: vec![],
        };
        assert_eq!(
            skinning_matrices_dq(&empty, &pose),
            Err(AnimError::EmptySkeleton)
        );
        assert_eq!(
            skinning_with_mode(&empty, &pose, SkinMode::DualQuaternion),
            Err(AnimError::EmptySkeleton)
        );
        let (skeleton, good) = chain();
        let mut short = vec![DualQuat::IDENTITY; 2];
        assert_eq!(
            skinning_duals_into(&skeleton, &good, &mut short),
            Err(AnimError::LengthMismatch {
                expected: 3,
                got: 2
            })
        );
        let mut out = DualQuat::IDENTITY;
        assert_eq!(blend_into(&mut out, &[], &[]), Err(DqError::EmptyStack));
        let items = [DualQuat::IDENTITY; 2];
        assert_eq!(
            blend_into(&mut out, &items, &[1.0]),
            Err(DqError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        assert_eq!(
            blend_into(&mut out, &items, &[0.5, f32::NAN]),
            Err(DqError::NonFiniteWeight(1))
        );
        // Display names every failure without allocating the message path.
        assert_eq!(format!("{}", DqError::EmptyStack), "nothing to blend");
    }

    #[test]
    fn dq_option_zero_alloc() {
        let (skeleton, pose) = chain();
        let mut duals = vec![DualQuat::IDENTITY; skeleton.len()];
        let items = [DualQuat::IDENTITY; 2];
        let weights = [0.5, 0.5];
        let mut blended = DualQuat::IDENTITY;
        let ((), count) = engine_core::alloc_probe::measure(|| {
            skinning_duals_into(&skeleton, &pose, &mut duals).unwrap();
            blend_into(&mut blended, &items, &weights).unwrap();
            let _ = dual_to_matrix(&blended);
        });
        assert_eq!(count.calls, 0, "per-tick DQ path must not allocate");
    }
}
