//! fighter_look_pov: the first-person slot's body, seen from its own eye (row POV2 and the look
//! audit of 2026-10-24, defect 2). The wearer's head leaves the picture, the head's real eye point
//! and a chest camera point ride the posed skeleton, and the posed vertices are kept for the view
//! laws, so the camera, the drawn body and the law read one pose.
//! Consumer: the `arena` window's legionary mode. Units: meters, world axes. Determinism:
//! presentation only; the pose and the sim hash never read it.
use super::{FighterLook, Rig};
use crate::fps_view::pov::{CoverAcc, PovBody, PovCover, PovLens};
use crate::stage_bodies::place_joint;
use crate::view_truth::BodyTruth;
use engine_render::v1_skin::SkinnedFrame;

/// The eye sits this far up the head from the head joint's origin, meters: the model's head joint
/// is at the jaw, and its eyes at about a hand's width above it.
const EYE_UP_M: f32 = 0.145;
/// The eye sits this far out of the face from the head joint's origin, meters.
const EYE_FWD_M: f32 = 0.085;
/// The wearer's own geometry keeps this far from its own eye point, meters: the
/// near plane sits at 0.08, and a real blade or hand inside that would cut the
/// face it belongs to. The pose pushes the carrying joints out to it.
pub(super) const HEAD_CLEAR_M: f32 = 0.10;
/// The chest camera sits this far up the torso from the torso joint's origin, meters.
const CHEST_UP_M: f32 = 0.30;
/// The chest camera sits this far out of the chest from the torso joint's origin, meters.
const CHEST_FWD_M: f32 = 0.13;
/// A hidden head is scaled to this much of itself and set down on its joint: a point, not a
/// zero matrix, so the skin's normals still normalize.
const HIDDEN_SCALE: f32 = 1.0e-3;

/// The two mount points of one posed wearer, world meters, before the sand rest.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Mounts {
    head_eye: [f32; 3],
    chest_cam: [f32; 3],
}

/// Where a joint's origin stood in the bind model: the inverse bind maps model to joint space
/// (`p * R + t`), so the origin is `-t * R^-1`, and `R` is a rotation.
fn bind_origin(inverse_bind: &[[f32; 4]; 4]) -> [f32; 3] {
    let t = inverse_bind[3];
    std::array::from_fn(|i| {
        -(t[0] * inverse_bind[i][0] + t[1] * inverse_bind[i][1] + t[2] * inverse_bind[i][2])
    })
}

/// Hide the wearer's head and read the mount points off the posed skeleton. `matrices` are the
/// body's skinning matrices (`inverse bind * joint * root`), changed in place: the head joint's
/// becomes a point on the joint, so every vertex that rides the head collapses onto it. The
/// camera sits at the eye point above that point, so nothing of the head is left to fill the view
/// from the inside. `None` for a rig without the joints (nothing changes).
pub(super) fn hide_head(rig: &Rig, matrices: &mut [[[f32; 4]; 4]]) -> Option<Mounts> {
    let (head, torso) = (rig.head, rig.torso);
    let head_skin = *matrices.get(head)?;
    let torso_skin = *matrices.get(torso)?;
    let head_origin = bind_origin(rig.inverse_binds.get(head)?);
    let torso_origin = bind_origin(rig.inverse_binds.get(torso)?);
    let mounts = Mounts {
        head_eye: place_joint(
            [
                head_origin[0],
                head_origin[1] + EYE_UP_M,
                head_origin[2] + EYE_FWD_M,
            ],
            &head_skin,
        ),
        chest_cam: place_joint(
            [
                torso_origin[0],
                torso_origin[1] + CHEST_UP_M,
                torso_origin[2] + CHEST_FWD_M,
            ],
            &torso_skin,
        ),
    };
    let at = place_joint(head_origin, &head_skin);
    matrices[head] = [
        [HIDDEN_SCALE, 0.0, 0.0, 0.0],
        [0.0, HIDDEN_SCALE, 0.0, 0.0],
        [0.0, 0.0, HIDDEN_SCALE, 0.0],
        [at[0], at[1], at[2], 1.0],
    ];
    Some(mounts)
}

/// The wearer's pose for the first person: the mount points and every posed vertex, lifted by
/// the sand rest `lift` (the same shift the upload adds), around the drawn `root`.
pub(super) fn pov_body(
    mounts: &Mounts,
    frames: &[SkinnedFrame],
    root: [f32; 3],
    lift: f32,
) -> PovBody {
    let up = |p: [f32; 3]| [p[0], p[1] + lift, p[2]];
    let total = frames.iter().map(|frame| frame.positions.len()).sum();
    let mut positions = Vec::with_capacity(total);
    for frame in frames {
        positions.extend(frame.positions.iter().map(|p| up(*p)));
    }
    PovBody {
        root_m: root,
        head_eye_m: up(mounts.head_eye),
        chest_cam_m: up(mounts.chest_cam),
        positions,
    }
}

/// The wearer's own geometry inside [`HEAD_CLEAR_M`] of its eye, pushed clear:
/// the joints that carry the nearest own vertex step out along eye -> vertex
/// until that vertex sits on the clearance, so the blade a wearer swings never
/// cuts the face it belongs to. `matrices` are the body's skinning matrices,
/// changed in place; the caller re-skins. Returns the push in meters (`None`
/// when nothing was inside). View only: the pose, the sim and the hash never
/// read it.
pub(super) fn clear_own_eye(
    rig: &Rig,
    eye: [f32; 3],
    frames: &[SkinnedFrame],
    matrices: &mut [[[f32; 4]; 4]],
) -> Option<f32> {
    let mut nearest: Option<(f32, [f32; 3], usize)> = None;
    for (part, frame) in frames.iter().enumerate() {
        for vertex in &frame.positions {
            let offset = [vertex[0] - eye[0], vertex[1] - eye[1], vertex[2] - eye[2]];
            let distance =
                (offset[0] * offset[0] + offset[1] * offset[1] + offset[2] * offset[2]).sqrt();
            if distance.is_finite() && nearest.is_none_or(|(best, _, _)| distance < best) {
                nearest = Some((distance, *vertex, part));
            }
        }
    }
    let (distance, vertex, part) = nearest?;
    if distance >= HEAD_CLEAR_M {
        return None;
    }
    let push = HEAD_CLEAR_M - distance;
    // A vertex exactly on the eye names no direction: lift the geometry instead.
    let dir = if distance > f32::MIN_POSITIVE {
        [
            (vertex[0] - eye[0]) / distance,
            (vertex[1] - eye[1]) / distance,
            (vertex[2] - eye[2]) / distance,
        ]
    } else {
        [0.0, 1.0, 0.0]
    };
    // Every joint the nearest part rides steps out once: a joint named by two
    // of its rows would otherwise be pushed twice.
    let skin = &rig.parts.get(part)?.skin;
    let mut moved: Vec<usize> = Vec::new();
    for (row, weights) in skin.weights.iter().enumerate() {
        let Some(row_joints) = skin.joints.get(row) else {
            continue;
        };
        for (lane, weight) in weights.iter().enumerate() {
            if *weight <= 0.0 {
                continue;
            }
            let Some(joint) = row_joints.get(lane).map(|joint| usize::from(*joint)) else {
                continue;
            };
            if moved.contains(&joint) {
                continue;
            }
            moved.push(joint);
            if let Some(matrix) = matrices.get_mut(joint) {
                for axis in 0..3 {
                    matrix[3][axis] += dir[axis] * push;
                }
            }
        }
    }
    Some(push)
}

impl FighterLook {
    /// Put the first-person eye in `slot` (or take it out of every slot with `None`): that body
    /// draws without its head and carries its own pose in [`BodyTruth::pov`]. View only; set it
    /// before the frame is posed.
    pub fn set_pov_slot(&mut self, slot: Option<u8>) {
        for (index, anim) in self.slots.iter_mut().enumerate() {
            anim.pov = slot.is_some_and(|wearer| usize::from(wearer) == index);
        }
    }

    /// What the wearer's own geometry does to the picture of `lens`: the near plane's cuts, the
    /// screen centre and the share of the picture covered, over the body as last presented.
    /// `None` when `slot` has no first-person pose.
    #[must_use]
    pub fn pov_cover(&self, slot: u8, lens: PovLens) -> Option<PovCover> {
        self.pov_cover_of(&self.truth, slot, lens)
    }

    /// [`Self::pov_cover`] over explicit bodies: the headless proofs pose without a presenter.
    #[must_use]
    pub(super) fn pov_cover_of(
        &self,
        bodies: &[BodyTruth],
        slot: u8,
        lens: PovLens,
    ) -> Option<PovCover> {
        let pov = bodies
            .iter()
            .find(|body| body.slot == slot)?
            .pov
            .as_deref()?;
        let rig = self.rigs.get(self.rig_for_slot(usize::from(slot)))?;
        let mut acc = CoverAcc::new(lens);
        let mut from = 0usize;
        for part in &rig.parts {
            let to = from + part.mesh.positions.len();
            acc.add_part(pov.positions.get(from..to)?, &part.mesh.indices);
            from = to;
        }
        Some(acc.finish())
    }
}

#[cfg(test)]
#[path = "fighter_look_pov_tests.rs"]
mod fighter_look_pov_tests;
