//! layers: masked body layers over the animation graph pose.
//!
//! Consumer: arena/combat composes legs base, arms override plus additive
//! hit-react per fighter; render reads the final [`Pose`]. Owner:
//! `engine-anim`; this module consumes [`Pose`] plus [`GraphError`] from
//! `graph` and owns only [`MaskTable`]. Units: meters, unit quats
//! `[x, y, z, w]`; hit gain in Q8 (256 reads 1x). Determinism: joint
//! order, `+ - * /` plus `sqrt` only, no clock; replay-exact twice.
//! Errors: [`GraphError`], no new type. Budget: one linear pass
//! `O(joints)`, at most about 0.05 ms per fighter at 256 joints;
//! steady-state ticks allocate nothing (caller-owned `out`).

use super::contract::MAX_JOINTS;
use super::graph::{nlerp, GraphError, Pose};

/// Q8 unit gain: 256 reads 1x. Single owner for the engine-anim Q8
/// pair (`exert` plus `hit_react` re-export these, never redefine them).
pub const GAIN_Q8_ONE: u16 = 256;
/// Widest accepted hit gain: 768 reads 3x. Single owner; see above.
pub const GAIN_Q8_MAX: u16 = 768;
/// Mask value that takes the over pose exactly.
pub const MASK_ONE: u8 = 255;
/// Masked-eval tolerance in meters/quat lanes against the full-body reference.
pub const MASK_EVAL_TOL: f32 = 1e-5;

/// Per-joint blend mask as bytes: 0 keeps the base, 255 takes the layer.
///
/// Tables are data baked offline; eval only reads them in joint order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MaskTable {
    weights: Vec<u8>,
}

impl MaskTable {
    /// Build from explicit per-joint weights; empty or over-wide is refused.
    pub fn from_vec(weights: Vec<u8>) -> Result<Self, GraphError> {
        if weights.is_empty() {
            return Err(GraphError::EmptyPose);
        }
        if weights.len() > MAX_JOINTS {
            return Err(GraphError::LengthMismatch {
                expected: MAX_JOINTS,
                got: weights.len(),
            });
        }
        Ok(Self { weights })
    }

    /// Splat one weight across `joints`; zero or over-wide is refused.
    pub fn splat(joints: usize, weight: u8) -> Result<Self, GraphError> {
        if joints == 0 {
            return Err(GraphError::EmptyPose);
        }
        if joints > MAX_JOINTS {
            return Err(GraphError::LengthMismatch {
                expected: MAX_JOINTS,
                got: joints,
            });
        }
        Ok(Self {
            weights: vec![weight; joints],
        })
    }

    /// Full-take mask: every joint reads the layer.
    pub fn full(joints: usize) -> Result<Self, GraphError> {
        Self::splat(joints, MASK_ONE)
    }

    /// Zero mask: every joint keeps the base.
    pub fn zero(joints: usize) -> Result<Self, GraphError> {
        Self::splat(joints, 0)
    }

    /// Joint count.
    pub fn len(&self) -> usize {
        self.weights.len()
    }

    /// True when the table holds no joints (unreachable via constructors).
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Raw weights in joint order, for fixtures and mask-table postings.
    pub fn weights(&self) -> &[u8] {
        &self.weights
    }

    /// Weight for one joint; out-of-range reads 0 (keep the base).
    pub fn weight(&self, joint: usize) -> u8 {
        self.weights.get(joint).copied().unwrap_or(0)
    }
}

/// Paired-lane, non-empty, finite pose check; returns the joint count.
fn checked_joints(pose: &Pose) -> Result<usize, GraphError> {
    if pose.positions.len() != pose.rotations.len() {
        return Err(GraphError::LengthMismatch {
            expected: pose.positions.len(),
            got: pose.rotations.len(),
        });
    }
    let joints = pose.positions.len();
    if joints == 0 {
        return Err(GraphError::EmptyPose);
    }
    if joints > MAX_JOINTS {
        return Err(GraphError::LengthMismatch {
            expected: MAX_JOINTS,
            got: joints,
        });
    }
    if pose.positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(GraphError::NonFinite("layers.positions"));
    }
    if pose.rotations.iter().flatten().any(|v| !v.is_finite()) {
        return Err(GraphError::NonFinite("layers.rotations"));
    }
    Ok(joints)
}

/// Scratch shape check: paired lanes at `joints` width (contents unread).
fn checked_out(out: &Pose, joints: usize) -> Result<(), GraphError> {
    if out.positions.len() != joints || out.rotations.len() != joints {
        let got = if out.positions.len() != joints {
            out.positions.len()
        } else {
            out.rotations.len()
        };
        return Err(GraphError::LengthMismatch {
            expected: joints,
            got,
        });
    }
    Ok(())
}

fn checked_gain(gain_q8: u16) -> Result<(), GraphError> {
    if gain_q8 > GAIN_Q8_MAX {
        return Err(GraphError::BadWeight {
            node: String::from("hit-gain"),
            weight: f32::from(gain_q8),
        });
    }
    Ok(())
}

/// Override one joint: mask 0 keeps base, 255 takes over, else lerp/nlerp.
pub(crate) fn override_joint(
    base_p: [f32; 3],
    base_r: [f32; 4],
    over_p: [f32; 3],
    over_r: [f32; 4],
    mask: u8,
) -> ([f32; 3], [f32; 4]) {
    if mask == 0 {
        return (base_p, base_r);
    }
    if mask == MASK_ONE {
        return (over_p, over_r);
    }
    let t = f32::from(mask) * (1.0 / 255.0);
    (
        [
            base_p[0] + (over_p[0] - base_p[0]) * t,
            base_p[1] + (over_p[1] - base_p[1]) * t,
            base_p[2] + (over_p[2] - base_p[2]) * t,
        ],
        nlerp(base_r, over_r, t),
    )
}

/// Override blend: per-joint mask picks base versus over for positions and
/// shortest-arc quats. Endpoints are exact copies; steady-state allocates
/// nothing.
pub fn apply_override(
    base: &Pose,
    over: &Pose,
    mask: &MaskTable,
    out: &mut Pose,
) -> Result<(), GraphError> {
    let joints = checked_joints(base)?;
    let over_joints = checked_joints(over)?;
    if over_joints != joints {
        return Err(GraphError::LengthMismatch {
            expected: joints,
            got: over_joints,
        });
    }
    if mask.len() != joints {
        return Err(GraphError::LengthMismatch {
            expected: joints,
            got: mask.len(),
        });
    }
    checked_out(out, joints)?;
    for joint in 0..joints {
        let (pos, rot) = override_joint(
            base.positions[joint],
            base.rotations[joint],
            over.positions[joint],
            over.rotations[joint],
            mask.weights()[joint],
        );
        out.positions[joint] = pos;
        out.rotations[joint] = rot;
    }
    Ok(())
}

/// Additive blend: position offsets scaled by mask times Q8 gain; rotations
/// stay on the base (rotational hit-react rides the override path until a
/// dual-quat row layers quats). Mask 0 or gain 0 keeps the base exactly.
pub fn apply_additive(
    base: &Pose,
    add: &Pose,
    mask: &MaskTable,
    gain_q8: u16,
    out: &mut Pose,
) -> Result<(), GraphError> {
    let joints = checked_joints(base)?;
    let add_joints = checked_joints(add)?;
    if add_joints != joints {
        return Err(GraphError::LengthMismatch {
            expected: joints,
            got: add_joints,
        });
    }
    if mask.len() != joints {
        return Err(GraphError::LengthMismatch {
            expected: joints,
            got: mask.len(),
        });
    }
    checked_gain(gain_q8)?;
    checked_out(out, joints)?;
    for joint in 0..joints {
        let m = mask.weights()[joint];
        if m == 0 || gain_q8 == 0 {
            out.positions[joint] = base.positions[joint];
            out.rotations[joint] = base.rotations[joint];
            continue;
        }
        let s = f32::from(m) * f32::from(gain_q8) / 65280.0;
        out.positions[joint] = [
            base.positions[joint][0] + add.positions[joint][0] * s,
            base.positions[joint][1] + add.positions[joint][1] * s,
            base.positions[joint][2] + add.positions[joint][2] * s,
        ];
        out.rotations[joint] = base.rotations[joint];
    }
    Ok(())
}

/// How one ordered layer reads its pose.
/// PORT(FyroxEngine/Fyrox:fyrox-animation 1.0.1 layer plus mask): ordered masked
/// layers over the base; the full ABSM graph is rejected here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerMode {
    /// Masked override of positions plus shortest-arc quats.
    Override,
    /// Masked additive offsets at Q8 gain; rotations stay base.
    Additive,
}

/// One ordered masked layer, keyed by name (name, mode, pose, mask, gain).
/// Registration stays explicit name-keyed pushes; linkme slice rejected (std-only).
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a>(
    pub &'static str,
    pub LayerMode,
    pub &'a Pose,
    pub &'a MaskTable,
    pub u16,
);

/// Ordered masked stack over `base` in registration order.
/// Reuses `override_joint` plus the additive mask-times-gain scale; no new math.
/// Units meters/quats/Q8; joint order, `+ - * /` only, replay-exact; caller `out` only.
pub fn compose_stack(base: &Pose, layers: &[Layer<'_>], out: &mut Pose) -> Result<(), GraphError> {
    let joints = checked_joints(base)?;
    checked_out(out, joints)?;
    for layer in layers {
        let got = checked_joints(layer.2)?;
        if got != joints {
            return Err(GraphError::LengthMismatch {
                expected: joints,
                got,
            });
        }
        if layer.3.len() != joints {
            return Err(GraphError::LengthMismatch {
                expected: joints,
                got: layer.3.len(),
            });
        }
        if layer.1 == LayerMode::Additive {
            checked_gain(layer.4)?;
        }
    }
    for joint in 0..joints {
        let mut pos = base.positions[joint];
        let mut rot = base.rotations[joint];
        for layer in layers {
            match layer.1 {
                LayerMode::Override => {
                    let (p, r) = override_joint(
                        pos,
                        rot,
                        layer.2.positions[joint],
                        layer.2.rotations[joint],
                        layer.3.weights()[joint],
                    );
                    pos = p;
                    rot = r;
                }
                LayerMode::Additive => {
                    let m = layer.3.weights()[joint];
                    if m == 0 || layer.4 == 0 {
                        continue;
                    }
                    let s = f32::from(m) * f32::from(layer.4) / 65280.0;
                    pos = [
                        pos[0] + layer.2.positions[joint][0] * s,
                        pos[1] + layer.2.positions[joint][1] * s,
                        pos[2] + layer.2.positions[joint][2] * s,
                    ];
                }
            }
        }
        out.positions[joint] = pos;
        out.rotations[joint] = rot;
    }
    Ok(())
}

/// Full body composition in one linear pass: legs base, arms override by
/// `arms_mask`, then additive hit-react by `hit_mask` times `hit_gain_q8`.
///
/// Sampling stays with the graph banks; this composes sampled poses. All
/// widths must agree; steady-state ticks allocate nothing.
pub fn compose(
    base: &Pose,
    arms: &Pose,
    hit: &Pose,
    arms_mask: &MaskTable,
    hit_mask: &MaskTable,
    hit_gain_q8: u16,
    out: &mut Pose,
) -> Result<(), GraphError> {
    let layers = [
        Layer("arms", LayerMode::Override, arms, arms_mask, GAIN_Q8_ONE),
        Layer("hit", LayerMode::Additive, hit, hit_mask, hit_gain_q8),
    ];
    compose_stack(base, &layers, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::pose_bits;
    use engine_core::alloc_probe::measure;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    /// Six-joint fighter rig: 0 pelvis, 1/2 legs, 3/4 arms, 5 spine.
    fn legs_pose() -> Pose {
        Pose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, 0.5, 0.1],
                [0.2, 0.5, -0.1],
                [-0.3, 1.4, 0.0],
                [0.3, 1.4, 0.0],
                [0.0, 1.2, 0.0],
            ],
            rotations: vec![IDENTITY; 6],
        }
    }

    /// Combat upper body; legs lanes intentionally mirror the base.
    fn arms_pose() -> Pose {
        Pose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, 0.5, 0.1],
                [0.2, 0.5, -0.1],
                [-0.5, 1.5, 0.4],
                [0.5, 1.5, 0.4],
                [0.0, 1.2, 0.0],
            ],
            rotations: vec![IDENTITY; 6],
        }
    }

    /// Hit-react deltas; zero everywhere except pelvis and spine.
    fn hit_pose() -> Pose {
        Pose {
            positions: vec![
                [-0.1, 0.0, -0.2],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                [-0.15, 0.05, -0.3],
            ],
            rotations: vec![IDENTITY; 6],
        }
    }

    /// Posted mask fixture: arms take joints 3/4, hit takes 0/5.
    fn fixture_masks() -> (MaskTable, MaskTable) {
        let arms = MaskTable::from_vec(vec![0, 0, 0, 255, 255, 0]).expect("arms mask");
        let hit = MaskTable::from_vec(vec![255, 0, 0, 0, 0, 255]).expect("hit mask");
        (arms, hit)
    }

    fn assert_near(a: [f32; 3], b: [f32; 3], tol: f32) {
        for lane in 0..3 {
            assert!(
                (a[lane] - b[lane]).abs() <= tol,
                "lane {lane}: {a:?} vs {b:?} past {tol}"
            );
        }
    }

    #[test]
    fn masked_eval_matches_fullbody_reference() {
        let (arms_mask, hit_mask) = fixture_masks();
        // Mask-table posting: fixtures live here and print under --nocapture.
        println!(
            "AM3 masks: arms={:?} hit={:?} gain_q8={} tol={}",
            arms_mask.weights(),
            hit_mask.weights(),
            GAIN_Q8_ONE,
            MASK_EVAL_TOL
        );
        let base = legs_pose();
        let arms = arms_pose();
        let hit = hit_pose();
        let mut out = Pose::zero(6);
        compose(
            &base,
            &arms,
            &hit,
            &arms_mask,
            &hit_mask,
            GAIN_Q8_ONE,
            &mut out,
        )
        .expect("compose fixture");
        // Hand-rolled full-body reference, independent of compose internals.
        let reference: Vec<[f32; 3]> = vec![
            [-0.1, 1.0, -0.2],
            [-0.2, 0.5, 0.1],
            [0.2, 0.5, -0.1],
            [-0.5, 1.5, 0.4],
            [0.5, 1.5, 0.4],
            [-0.15, 1.25, -0.3],
        ];
        for (joint, want) in reference.iter().enumerate() {
            assert_near(out.positions[joint], *want, MASK_EVAL_TOL);
        }
        assert_eq!(out.rotations, vec![IDENTITY; 6]);
    }

    #[test]
    fn override_endpoints_are_exact() {
        let base = legs_pose();
        let over = arms_pose();
        let mut out = Pose::zero(6);
        apply_override(&base, &over, &MaskTable::zero(6).expect("zero"), &mut out)
            .expect("zero mask");
        assert_eq!(out.positions, base.positions);
        assert_eq!(out.rotations, base.rotations);
        apply_override(&base, &over, &MaskTable::full(6).expect("full"), &mut out)
            .expect("full mask");
        assert_eq!(out.positions, over.positions);
        assert_eq!(out.rotations, over.rotations);
    }

    #[test]
    fn partial_override_blends_positions_and_keeps_quats_unit() {
        let base = legs_pose();
        let over = arms_pose();
        let mask = MaskTable::splat(6, 128).expect("half mask");
        let mut out = Pose::zero(6);
        apply_override(&base, &over, &mask, &mut out).expect("half blend");
        // Independent midpoint check on joint 3: legs + (arms - legs) * 128/255.
        let t = 128.0f32 / 255.0;
        let want = [
            -0.3 + (-0.5 - -0.3) * t,
            1.4 + (1.5 - 1.4) * t,
            0.0 + (0.4 - 0.0) * t,
        ];
        assert_near(out.positions[3], want, MASK_EVAL_TOL);
        for rot in &out.rotations {
            let norm2 = rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3];
            assert!(
                (norm2 - 1.0).abs() <= MASK_EVAL_TOL,
                "quat {rot:?} not unit"
            );
        }
    }

    #[test]
    fn additive_scales_by_mask_and_gain() {
        let base = legs_pose();
        let add = hit_pose();
        let full = MaskTable::full(6).expect("full");
        let mut out = Pose::zero(6);
        // Zero gain keeps the base exactly even under a full mask.
        apply_additive(&base, &add, &full, 0, &mut out).expect("zero gain");
        assert_eq!(out.positions, base.positions);
        // Unit gain adds the full delta on joint 0.
        apply_additive(&base, &add, &full, GAIN_Q8_ONE, &mut out).expect("unit gain");
        assert_near(out.positions[0], [-0.1, 1.0, -0.2], MASK_EVAL_TOL);
        // Double gain doubles the offset on joint 5.
        apply_additive(&base, &add, &full, 512, &mut out).expect("double gain");
        assert_near(out.positions[5], [-0.3, 1.3, -0.6], MASK_EVAL_TOL);
        // Additive never bends rotations.
        assert_eq!(out.rotations, vec![IDENTITY; 6]);
    }

    #[test]
    fn replay_is_identical_twice() {
        let (arms_mask, hit_mask) = fixture_masks();
        let (base, arms, hit) = (legs_pose(), arms_pose(), hit_pose());
        let mut first = Pose::zero(6);
        let mut second = Pose::zero(6);
        for out in [&mut first, &mut second] {
            compose(&base, &arms, &hit, &arms_mask, &hit_mask, GAIN_Q8_ONE, out)
                .expect("replay compose");
        }
        assert_eq!(first, second);
    }

    #[test]
    fn ordered_stack_matches_compose_and_replays() {
        let (arms_mask, hit_mask) = fixture_masks();
        let (base, arms, hit) = (legs_pose(), arms_pose(), hit_pose());
        let layers = [
            Layer("arms", LayerMode::Override, &arms, &arms_mask, GAIN_Q8_ONE),
            Layer("hit", LayerMode::Additive, &hit, &hit_mask, GAIN_Q8_ONE),
        ];
        assert_eq!(layers[0].0, "arms");
        let mut stacked = Pose::zero(6);
        let mut direct = Pose::zero(6);
        compose_stack(&base, &layers, &mut stacked).expect("stacked");
        compose(
            &base,
            &arms,
            &hit,
            &arms_mask,
            &hit_mask,
            GAIN_Q8_ONE,
            &mut direct,
        )
        .expect("direct");
        assert_eq!(stacked, direct);
        assert_near(stacked.positions[0], [-0.1, 1.0, -0.2], MASK_EVAL_TOL);
        assert_near(stacked.positions[5], [-0.15, 1.25, -0.3], MASK_EVAL_TOL);
        assert_eq!(stacked.rotations, vec![IDENTITY; 6]);
        let mut again = Pose::zero(6);
        compose_stack(&base, &layers, &mut again).expect("replay");
        assert_eq!(stacked, again);
    }

    #[test]
    fn bad_shapes_are_refused() {
        let base = legs_pose();
        let mut out = Pose::zero(6);
        let mask6 = MaskTable::from_vec(vec![0; 6]).expect("mask6");
        // Pose width mismatch names the expected count.
        let mut wide = legs_pose();
        wide.positions.push([0.0; 3]);
        wide.rotations.push(IDENTITY);
        assert_eq!(
            apply_override(&base, &wide, &mask6, &mut out).unwrap_err(),
            GraphError::LengthMismatch {
                expected: 6,
                got: 7
            }
        );
        // Mask width mismatch is loud too.
        let mask5 = MaskTable::from_vec(vec![0; 5]).expect("mask5");
        assert_eq!(
            compose(&base, &base, &base, &mask5, &mask6, GAIN_Q8_ONE, &mut out).unwrap_err(),
            GraphError::LengthMismatch {
                expected: 6,
                got: 5
            }
        );
        // Empty and over-wide tables never build.
        assert_eq!(
            MaskTable::from_vec(Vec::new()).unwrap_err(),
            GraphError::EmptyPose
        );
        assert_eq!(MaskTable::splat(0, 0).unwrap_err(), GraphError::EmptyPose);
        assert_eq!(
            MaskTable::splat(MAX_JOINTS + 1, 0).unwrap_err(),
            GraphError::LengthMismatch {
                expected: MAX_JOINTS,
                got: MAX_JOINTS + 1
            }
        );
        // Non-finite lanes name their lane.
        let mut nan = legs_pose();
        nan.positions[1][0] = f32::NAN;
        assert_eq!(
            apply_override(&nan, &base, &mask6, &mut out).unwrap_err(),
            GraphError::NonFinite("layers.positions")
        );
        let mut inf = legs_pose();
        inf.rotations[0][3] = f32::INFINITY;
        assert_eq!(
            apply_additive(&base, &inf, &mask6, GAIN_Q8_ONE, &mut out).unwrap_err(),
            GraphError::NonFinite("layers.rotations")
        );
        // Over-wide gain is refused; the ceiling itself is accepted.
        assert_eq!(
            apply_additive(&base, &base, &mask6, GAIN_Q8_MAX + 1, &mut out).unwrap_err(),
            GraphError::BadWeight {
                node: String::from("hit-gain"),
                weight: (GAIN_Q8_MAX + 1) as f32
            }
        );
        apply_additive(&base, &base, &mask6, GAIN_Q8_MAX, &mut out).expect("ceiling gain");
        // Scratch width mismatch writes nothing.
        let mut narrow = Pose::zero(5);
        assert_eq!(
            compose(
                &base,
                &base,
                &base,
                &mask6,
                &mask6,
                GAIN_Q8_ONE,
                &mut narrow
            )
            .unwrap_err(),
            GraphError::LengthMismatch {
                expected: 6,
                got: 5
            }
        );
        // Zero-joint poses are refused, and OOB mask reads keep the base.
        assert_eq!(
            apply_override(&Pose::zero(0), &Pose::zero(0), &mask6, &mut out).unwrap_err(),
            GraphError::EmptyPose
        );
        assert_eq!(mask6.weight(99), 0);
    }

    #[test]
    fn steady_state_reuses_buffers() {
        let (arms_mask, hit_mask) = fixture_masks();
        let (base, arms, hit) = (legs_pose(), arms_pose(), hit_pose());
        let mut out = Pose::zero(6);
        compose(
            &base,
            &arms,
            &hit,
            &arms_mask,
            &hit_mask,
            GAIN_Q8_ONE,
            &mut out,
        )
        .expect("warmup");
        let pos_ptr = out.positions.as_ptr();
        let rot_ptr = out.rotations.as_ptr();
        let (pos_cap, rot_cap) = (out.positions.capacity(), out.rotations.capacity());
        for _ in 0..60 {
            compose(
                &base,
                &arms,
                &hit,
                &arms_mask,
                &hit_mask,
                GAIN_Q8_ONE,
                &mut out,
            )
            .expect("steady tick");
        }
        assert_eq!(out.positions.as_ptr(), pos_ptr);
        assert_eq!(out.rotations.as_ptr(), rot_ptr);
        assert_eq!(out.positions.capacity(), pos_cap);
        assert_eq!(out.rotations.capacity(), rot_cap);
        assert_eq!(out.positions.len(), 6);
    }

    /// One scripted layers tick: locomotion-style base swaps plus
    /// interrupt (override) and hit-react (additive) shapes. The gain
    /// script covers silent, unit, double and ceiling impulse lanes.
    fn am16_tick(
        base: &Pose,
        arms: &Pose,
        hit: &Pose,
        arms_mask: &MaskTable,
        hit_mask: &MaskTable,
        tick: u32,
        out: &mut Pose,
    ) {
        let gain = match tick % 4 {
            0 => GAIN_Q8_ONE,
            1 => 512,
            2 => 0,
            _ => GAIN_Q8_MAX,
        };
        match tick % 3 {
            0 => compose(base, arms, hit, arms_mask, hit_mask, gain, out).expect("compose"),
            1 => apply_additive(base, hit, hit_mask, gain, out).expect("additive"),
            _ => apply_override(base, arms, arms_mask, out).expect("override"),
        }
    }

    #[test]
    fn am16_alloc_probe_sees_a_real_allocation() {
        // Calibration: a blind counter must not fake the gate below.
        let ((), heap) = measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(heap.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn am16_layers_replay_x2_and_restore_equal() {
        let (arms_mask, hit_mask) = fixture_masks();
        let (base, arms, hit) = (legs_pose(), arms_pose(), hit_pose());
        let run = || {
            let mut out = Pose::zero(6);
            let mut digests = Vec::with_capacity(48);
            for tick in 0..48u32 {
                am16_tick(&base, &arms, &hit, &arms_mask, &hit_mask, tick, &mut out);
                digests.push(pose_bits(&out));
            }
            digests
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "masked eval must replay x2");
        assert_ne!(first[0], first[1], "gain script must move");
        // Snapshot mid-tape, run out, restore, run out again.
        let mut out = Pose::zero(6);
        for tick in 0..16u32 {
            am16_tick(&base, &arms, &hit, &arms_mask, &hit_mask, tick, &mut out);
        }
        let snapshot = out.clone();
        let mut reference = Vec::with_capacity(32);
        for tick in 16..48u32 {
            am16_tick(&base, &arms, &hit, &arms_mask, &hit_mask, tick, &mut out);
            reference.push(pose_bits(&out));
        }
        out.clone_from(&snapshot);
        let mut replay = Vec::with_capacity(32);
        for tick in 16..48u32 {
            am16_tick(&base, &arms, &hit, &arms_mask, &hit_mask, tick, &mut out);
            replay.push(pose_bits(&out));
        }
        assert_eq!(reference, replay, "restore-then-eval must equal");
    }

    #[test]
    fn am16_layers_allocate_nothing_per_tick() {
        let (arms_mask, hit_mask) = fixture_masks();
        let (base, arms, hit) = (legs_pose(), arms_pose(), hit_pose());
        let mut out = Pose::zero(6);
        for tick in 0..8u32 {
            am16_tick(&base, &arms, &hit, &arms_mask, &hit_mask, tick, &mut out);
        }
        let ((), heap) = measure(|| {
            for tick in 8..72u32 {
                am16_tick(&base, &arms, &hit, &arms_mask, &hit_mask, tick, &mut out);
                std::hint::black_box(pose_bits(&out));
            }
        });
        assert_eq!(
            heap.calls, 0,
            "masked eval must not allocate per tick after warmup"
        );
    }
}
