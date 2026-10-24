//! PX46: the PD joint motors hold a powered stand at four base solver passes.
//!
//! Test-only module. The stand is a twelve-bone chain pinned by a fixed root,
//! driven by the velocity servo plus the joint motors, with one spear impulse
//! at tick 300. It measures what the row asks for: the worst joint opening
//! (adjacent bone centres drifting from their rest distance), the pose error of
//! the still-driven bones, and that the same run twice gives the same bytes.

use super::*;
use rapier3d::dynamics::RigidBodyType;

/// Twelve bones, each half a metre below its parent, root at y = 3.
fn chain12() -> (Skeleton, LocalPose, Vec<BoneShape>, Vec<DriveParams>) {
    let n = 12;
    let joints = (0..n)
        .map(|i| engine_anim::contract::Joint {
            name: format!("b{i}").into(),
            parent: if i == 0 { None } else { Some(i - 1) },
        })
        .collect();
    let mut positions = vec![[0.0f32, -0.5, 0.0]; n];
    positions[0] = [0.0, 3.0, 0.0];
    let pose = LocalPose {
        positions,
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; n],
    };
    let shapes = vec![
        BoneShape {
            radius: 0.06,
            half_height: 0.1,
        };
        n
    ];
    let drives = vec![DriveParams::default_drive(); n];
    (Skeleton { joints }, pose, shapes, drives)
}

/// One 600-tick stand with the spear at tick 300: worst joint opening, worst
/// pose error of a driven bone, and the final translation bits.
fn stand(gap_at_tick_300: [f32; 3]) -> (f32, f32, Vec<u32>) {
    let (skeleton, pose, shapes, drives) = chain12();
    let mut r = PoweredRagdoll::spawn_with_passes(&skeleton, &pose, &shapes, &drives, 4)
        .expect("the stand spawns");
    // A stand has a floor under its feet: the root is pinned, the chain hangs.
    if let Some(root) = r.bodies.get_mut(r.order[0]) {
        root.set_body_type(RigidBodyType::Fixed, true);
    }
    let (world_pos, _) = world_transforms(&skeleton, &pose);
    let rest = |i: usize| {
        let a = world_pos[i];
        let b = world_pos[i - 1];
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let mut worst_gap = 0.0f32;
    for tick in 0..600u64 {
        if tick == 300 {
            r.apply_hit(6, gap_at_tick_300, tick).expect("the spear");
        }
        r.step(FIXED_DT).expect("a step");
        for i in 1..skeleton.len() {
            let (Some(a), Some(b)) = (r.bodies.get(r.order[i]), r.bodies.get(r.order[i - 1]))
            else {
                continue;
            };
            let (ta, tb) = (a.translation(), b.translation());
            let d = [ta.x - tb.x, ta.y - tb.y, ta.z - tb.z];
            let now = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let open = (now - rest(i)).abs();
            if open > worst_gap {
                worst_gap = open;
            }
        }
    }
    // The pose error of every bone still driven: freed bones (the spear's
    // subtree) are allowed to move, the driven ones must still track.
    let (world_pos, _) = world_transforms(&skeleton, &pose);
    let mut worst_err = 0.0f32;
    for i in 0..skeleton.len() {
        if r.mask[i] {
            continue;
        }
        let Some(body) = r.bodies.get(r.order[i]) else {
            continue;
        };
        let t = body.translation();
        let d = [
            t.x - world_pos[i][0],
            t.y - world_pos[i][1],
            t.z - world_pos[i][2],
        ];
        let err = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if err > worst_err {
            worst_err = err;
        }
    }
    let mut bits = Vec::with_capacity(skeleton.len());
    for i in 0..skeleton.len() {
        if let Some(body) = r.bodies.get(r.order[i]) {
            let t = body.translation();
            bits.push(t.x.to_bits());
            bits.push(t.y.to_bits());
            bits.push(t.z.to_bits());
        }
    }
    (worst_gap, worst_err, bits)
}

/// PX46: four base passes, the shipped one centimetre margin, a 600-tick stand
/// and one spear thrust; the same stand twice gives the same bytes.
#[test]
fn four_passes_hold_a_powered_stand_under_one_spear_thrust() {
    let (gap, err, bits) = stand([40.0, 0.0, 0.0]);
    println!("PX46_MOTORS gap={gap:.4} err={err:.4}");
    assert!(
        gap <= JOINT_GAP_BAR_M,
        "the stand opens a joint {gap:.4} m at four base passes, over the {JOINT_GAP_BAR_M} m bar"
    );
    assert!(
        err < POSE_ERROR_BAR_M,
        "a driven bone sits {err:.4} m off its target, over the {POSE_ERROR_BAR_M} m bar"
    );
    let (gap2, err2, bits2) = stand([40.0, 0.0, 0.0]);
    assert_eq!(gap.to_bits(), gap2.to_bits(), "the same stand twice");
    assert_eq!(err.to_bits(), err2.to_bits(), "the same stand twice");
    assert_eq!(bits, bits2, "the same bytes twice");
}

/// The row's bar: the shipped one centimetre joint margin, and the pose error
/// under two centimetres.
const JOINT_GAP_BAR_M: f32 = 0.01;
const POSE_ERROR_BAR_M: f32 = 0.02;
