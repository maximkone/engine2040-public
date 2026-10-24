//! Parent/child inertia stabilization for the ragdoll, beside
//! [`crate::ragdoll::stabilize_masses`] (SW162). Mass ratios are clamped by
//! that pass; a heavy child on a light parent still out-torques the solver, so
//! this pass raises the parent's principal moment toward the sum of its
//! children's, capped at [`INERTIA_RATIO_MAX`] times its own (Jolt's
//! `cMaxInertiaIncrease`): a hand 40x the forearm's moment halves per pass, and
//! a gap inside the ceiling closes in one. Client-local ragdoll only: nothing
//! here reaches the golden fold.

use engine_anim::contract::Skeleton;

/// Ceiling on a parent's inertia rise when its children sum higher, as a
/// multiplier of the parent's own moment (Jolt's `cMaxInertiaIncrease`).
/// Units: ratio, dimensionless. Determinism: exact `f32` compare.
pub const INERTIA_RATIO_MAX: f32 = 2.0;

/// Stabilize parent/child inertia the way `stabilize_masses` does mass:
/// `moments` holds each bone's principal moment in skeleton order (kg m^2).
/// Walking the leaves first, a parent rises to at least the sum of its
/// children's stabilized moments, never above [`INERTIA_RATIO_MAX`] times its
/// own, so a heavy child cannot out-torque a light parent inside the solver.
/// Masses are not touched; the caller rebuilds each tensor from the result.
/// Length mismatch or empty input does nothing (never panics); non-finite or
/// non-positive moments sanitize to `1.0` first. Pure and deterministic on
/// every platform (`+ *` and comparisons only).
pub fn stabilize_inertia(moments: &mut [f32], skeleton: &Skeleton) {
    if moments.is_empty() || moments.len() != skeleton.len() {
        return;
    }
    for m in moments.iter_mut() {
        if !m.is_finite() || *m <= 0.0 {
            *m = 1.0;
        }
    }
    // Skeleton order is parents first, so a reverse walk visits every child
    // before its parent: the sum a parent sees is made of stabilized children.
    let mut child_sum = vec![0.0f32; moments.len()];
    for i in (0..skeleton.len()).rev() {
        let own = moments[i];
        let sum = child_sum[i];
        if sum > own {
            moments[i] = sum.min(INERTIA_RATIO_MAX * own);
        }
        if let Some(parent) = skeleton.joints[i].parent {
            if parent < moments.len() {
                child_sum[parent] += moments[i];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::Joint;

    /// Four-bone chain: root, mid, tip (forearm), end (hand).
    fn chain_skeleton() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                },
                Joint {
                    name: "mid".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "tip".into(),
                    parent: Some(1),
                },
                Joint {
                    name: "end".into(),
                    parent: Some(2),
                },
            ],
        }
    }

    /// SW162: a heavy hand on a light forearm (40x) is the shape the PX42
    /// shove exposes; the inertia pass raises the forearm and never the mass.
    #[test]
    fn stabilize_raises_a_heavy_hand_over_a_light_arm() {
        let skeleton = chain_skeleton();
        let before = vec![1.0f32, 1.0, 0.01, 0.4];
        let hand_before = before[3] / before[2];
        assert!(
            hand_before > INERTIA_RATIO_MAX,
            "the hand starts over the bound: {hand_before}"
        );
        let mass_before = vec![2.0f32, 1.5, 1.0, 0.9];
        let total_mass_before: f32 = mass_before.iter().sum();
        let mut moments = before.clone();
        stabilize_inertia(&mut moments, &skeleton);
        // A 40x gap can only halve in one pass: the forearm takes exactly the
        // Jolt ceiling (2x its own moment), never more.
        assert!(
            (moments[2] - INERTIA_RATIO_MAX * before[2]).abs() < 1e-6,
            "the forearm rose to exactly the ceiling: {}",
            moments[2]
        );
        let hand_after = moments[3] / moments[2];
        assert!(
            (hand_after - hand_before / INERTIA_RATIO_MAX).abs() < 1e-3,
            "one pass halves the ratio: {hand_before} -> {hand_after}"
        );
        // A gap inside the ceiling closes fully: 1.5x comes back to 1.0.
        let mut near = vec![1.0f32, 1.0, 0.01, 0.015];
        stabilize_inertia(&mut near, &skeleton);
        let near_after = near[3] / near[2];
        assert!(
            (near_after - 1.0).abs() < 1e-6,
            "a gap inside the ceiling closes: {near_after}"
        );
        // A parent already above its children's sum keeps its own moment.
        assert_eq!(moments[0], before[0], "the root has no heavy child");
        // The pass never touches mass: the slice and its total are unchanged.
        assert_eq!(mass_before.iter().sum::<f32>(), total_mass_before);
        // Bytes x2: the same input gives the same f32 bits.
        let mut again = before.clone();
        stabilize_inertia(&mut again, &skeleton);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
        assert_eq!(bits(&moments), bits(&again));
        // Failure cases do nothing and never panic.
        let mut short = vec![1.0f32, 2.0];
        stabilize_inertia(&mut short, &skeleton);
        assert_eq!(short, vec![1.0, 2.0]);
        let mut bad = vec![f32::NAN, 0.0, -1.0, f32::INFINITY];
        stabilize_inertia(&mut bad, &skeleton);
        assert!(bad.iter().all(|m| m.is_finite() && *m > 0.0));
        println!(
            "STABILIZE_INERTIA hand_before={hand_before:.3} hand_after={hand_after:.3} forearm={:.4} total_mass={total_mass_before:.4}",
            moments[2]
        );
    }
}
