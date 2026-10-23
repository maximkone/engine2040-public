//! fighter_look: the CT7 fighters as skinned, textured bodies in the presenter.
//!
//! Consumer: the `arena` binary (window frames plus their offscreen
//! captures, free play and the staged fight). Data owner: this module owns
//! the presentation anim state per slot (act, clip clock, hit flash, facing)
//! and the fight camera; bodies and clips come from
//! `engine_assets::body_import`, clip retarget from `engine_assets::retarget`,
//! linear blend skinning from `engine_render::v1_skin`, bundle identity from
//! [`crate::fighter_bind`] (body bytes re-hashed against the CT7 pins).
//! Units: meters, radians, fixed ticks at 60 Hz, clip seconds.
//! Determinism: presentation only. It reads views, hp and cues after each
//! tick and never writes match state, so the tick hash cannot move.
//! Errors: typed [`LookError`]; a missing or re-cooked body fails at load,
//! a refused upload fails the frame.
//! Budget: cold load about 1 s (4 bodies, 6 clips, 16 PNG decodes); per
//! frame CPU skinning of about 3.9k vertices per fighter plus one mesh
//! upload per body part (4 per fighter) whose 48 B vertices changed since
//! its last upload ([`upload_slot_parts`]).

use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::Instant;

use engine_anim::contract::{skinning_matrices, LocalPose, Skeleton};
use engine_anim::hit_react::{lean_upper, view_envelope, DIP_M, PEAK_M};
use engine_anim::inertial::Inertial;
use engine_anim::pose::sample_pose_with_rots;
use engine_arena::cloth::{Cloth, ClothCollider, ClothGrid, ClothParams, ClothTier};
use engine_arena::GaitPhase;
use engine_assets::asset_source::{AssetSource, FsSource};
use engine_assets::body_import::{
    import_clip_source, import_skinned_body, ClipSource, SkinnedBody,
};
use engine_assets::retarget::{retarget_clip, BoneMap};
use engine_assets::skin_import::ImportedClip;
use engine_core::vec3::cross3;
use engine_core::Fnv64;
use engine_render::contact_feet::{ContactFoot, CONTACT_FEET_MAX};
use engine_render::draw::{trail_strip_verts, TrailStripVert};
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter, TextureKey, TextureUpload};
use engine_render::v1::{self, Camera, MeshData, PbrMaterial};
use engine_render::v1_skin::{SkinPipeline, SkinWeights};
use sha2::{Digest, Sha256};

use crate::anim_bridge::FighterSlot;
use crate::fight_ring::FightRing;
use crate::fighter_bind::{self, FighterBinding};
use crate::stage_bodies::place_joint;
use crate::view_truth::BodyTruth;
use crate::weapon_trail::{TrailPoint, TrailRing, TRAIL_LEN};

/// Fold plus bridge bulk, moved whole into the child module below so this
/// file stays under the size cap: [`FighterLook::observe_tick`] and the act
/// fold live there, on the same paths as before. Presentation only.
#[path = "fighter_look_acts.rs"]
mod fighter_look_acts;

use self::fighter_look_acts::{bridge_act, bridge_slots};

/// Rollback rewind (EG39): one checkpoint per folded tick, the restore and
/// the fading correction offset the frame adds to the drawn root.
/// Presentation only.
#[path = "fighter_look_rewind.rs"]
mod fighter_look_rewind;

use self::fighter_look_rewind::{empty_checkpoints, Correction, LookCheckpoint};

/// Inertialized clip switches (LF9): the per-slot offset sets, the switch
/// fold and the frame read. Presentation only.
#[path = "fighter_look_inertia.rs"]
mod fighter_look_inertia;

/// Planted feet (LF12): the per-slot plants, the tick fold that steps them and
/// the frame step that holds them on the sand. Presentation only.
#[path = "fighter_look_feet.rs"]
mod fighter_look_feet;

use self::fighter_look_feet::{foot_slides, Feet, Legs};

/// Last-stage joint clamp (AM24): caps a presented joint's miss against the
/// pop law's prediction. The angular limiter lanes live here too.
#[path = "fighter_look_clamp.rs"]
mod fighter_look_clamp;

use self::fighter_look_clamp::LimitLane;

/// The throwing arm of the spear grapple (SG3): the per-slot throw clock, the
/// layer over the last pose and the palm positions a held spear ties to.
/// Presentation only.
#[path = "fighter_look_throw.rs"]
mod fighter_look_throw;

pub use self::fighter_look_throw::ArmState;
use self::fighter_look_throw::{throw_arm, Throw};

/// Real-body proofs for the sand rest, the hitstop hold, clip switches and
/// planted feet (a test-only file).
#[path = "fighter_look_proofs.rs"]
mod fighter_look_proofs;

/// The duel's over-the-shoulder shot (GP14 framing): two seen bodies fill the
/// picture instead of backing off to two specks. Presentation only.
#[path = "fighter_look_shoulder.rs"]
mod fighter_look_shoulder;

/// The window's stateful fight camera (GP43): the shot through the halflife
/// springs, the shake, roll and lens punch added after. Presentation only.
#[path = "fighter_look_cam.rs"]
mod fighter_look_cam;

pub use self::fighter_look_cam::CamFollow;

/// Ragdoll look (RZ2w): a floored fighter is handed to the float physical layer and drawn from
/// its bones; the get-up blends back through the inertial offsets. Presentation only.
#[path = "ragdoll_look.rs"]
mod ragdoll_look;

use self::ragdoll_look::RagdollLook;
pub use self::ragdoll_look::RagdollStump;

/// The realistic body family (Rocketbox bodies with their own clips) beside
/// the CT7 bodies: same look, its own bind and per-rig clip sets.
#[path = "fighter_look_family.rs"]
mod fighter_look_family;
/// Limb look (RZ3): an authority cut event hands the named joint's chain to the
/// float physical layer and draws a stump cap where it left. Presentation only.
#[path = "limb_look.rs"]
mod limb_look;

use self::limb_look::LimbLook;
pub use self::limb_look::{limb_mesh_key, LimbCapture, LimbPose, StumpPose, LIMB_MESH_ID_BASE};

/// The cape layer (RZ7): one CPU Verlet sheet per look slot, pinned at the
/// shoulders of the shown pose and stepped once per accepted tick. The sheets
/// come from `engine-physics`' cloth module through the `engine-arena` re-export
/// above, so this lane cloths a fighter without a physics edge.
///
/// Consumer: [`FighterLook::observe_tick`] lays and steps a cape per seen slot
/// each tick; [`FighterLook::present_into`] uploads the sheets and pushes one
/// draw each. Data owner: this layer; the pose and the seen slots come from
/// `fighter_look`. Units: meters, seconds, y up. Determinism: a cape is a pure
/// fold of the tick's pose plus its own previous state, so the same ticks give
/// the same sheet twice; the sim never reads one and nothing here is hashed, so
/// the golden cannot move. Errors: a refused pose, tune or collider skips the
/// slot's cape (no draw) instead of failing the frame.
///
/// Tune of every cape: the comfortable tier's sheet, light drag, no tearing.
const CAPE_TIER: ClothTier = ClothTier::Comfort;
/// Cape size in meters: width across the shoulder line, then the hang.
const CAPE_SIZE_M: [f32; 2] = [0.5, 0.95];
/// How far behind the shoulder line the cape's top row hangs, meters: the CT7
/// shoulder joints sit slightly forward of the body's middle, so this clears
/// the torso and hangs the sheet on the back.
const CAPE_BACK_M: f32 = 0.28;
/// Radius of the one-way body capsule a cape must keep clear, meters.
const CAPE_BODY_RADIUS_M: f32 = 0.16;
/// Skin added to a ragdoll bone's radius when it pushes a cape, meters: the
/// drawn body is fuller than the ragdoll capsule, so the cloth rests on the
/// surface instead of inside a thigh.
const CAPE_LIMB_SKIN_M: f32 = 0.02;
/// A rival's body pushes this fighter's cape once their roots are this close
/// on the ground plane, meters: a body leaning into a cape moves it.
const CAPE_NEIGHBOR_M: f32 = 1.6;
/// Most limb capsules one rig gives its cape: room for the ragdoll's table.
const MAX_CAPE_LIMBS: usize = 16;
/// Fastest body speed, m/s, the cape's own airflow follows: a teleport or a
/// respawn never blows a sheet across the arena.
const CAPE_WIND_MAX_M_S: f32 = 12.0;
/// Slowest body speed, m/s, that counts as running air: below it the cape's
/// wind stays zero and a standing sheet stays asleep.
const CAPE_WIND_MIN_M_S: f32 = 0.05;
/// Deep red of the cape cloth.
const CAPE_COLOR: [f32; 3] = [0.40, 0.06, 0.08];
/// Roughness of the cape cloth.
const CAPE_ROUGHNESS: f32 = 0.9;
/// Mesh id base for cape draws (clear of the body-part, texture and limb keys).
const CAPE_MESH_ID_BASE: u64 = 0xF16E_0000;

/// Mesh key of one slot's cape.
fn cape_mesh_key(slot: usize) -> MeshKey {
    let id = CAPE_MESH_ID_BASE + u64::try_from(slot).unwrap_or(u64::MAX - CAPE_MESH_ID_BASE);
    let mut digest = [0x5Du8; 32];
    digest[0] = u8::try_from(slot).unwrap_or(u8::MAX);
    MeshKey::new(id, digest)
}

/// A small particle index as a float, for a row fraction; cloth grids hold at
/// most 32 particles per side, so nothing saturates.
fn small_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// The CT7 joints one rig's cape pins and collides with.
#[derive(Clone, Copy, PartialEq, Debug)]
struct CapeJoints {
    /// Left and right shoulder joints: the top row spans between them.
    shoulders: [usize; 2],
    /// Chest and pelvis joints: the one-way body capsule runs between them.
    torso: [usize; 2],
    /// Limb capsules from the ragdoll bone table: joint, tip joint and radius
    /// in meters. The first `limb_count` entries are real.
    limbs: [(usize, usize, f32); MAX_CAPE_LIMBS],
    /// How many entries of `limbs` the rig has.
    limb_count: usize,
}

impl CapeJoints {
    /// The cape joints of a skeleton found by name; `None` when a shoulder or
    /// the torso is missing. A limb whose joint or tip is missing is skipped.
    fn find(skeleton: &Skeleton) -> Option<Self> {
        let at = |name: &str| skeleton.joints.iter().position(|joint| joint.name == name);
        let mut limbs = [(0, 0, 0.0); MAX_CAPE_LIMBS];
        let mut limb_count = 0;
        for (joint, tip, radius) in ragdoll_look::bone_capsules() {
            let (Some(a), Some(b)) = (at(joint), at(tip)) else {
                continue;
            };
            let Some(entry) = limbs.get_mut(limb_count) else {
                break;
            };
            *entry = (a, b, radius + CAPE_LIMB_SKIN_M);
            limb_count += 1;
        }
        Some(Self {
            shoulders: [at("arm_upper_l")?, at("arm_upper_r")?],
            torso: [at("torso")?, at("pelvis")?],
            limbs,
            limb_count,
        })
    }

    /// The limb capsules this rig really has.
    fn limbs(&self) -> &[(usize, usize, f32)] {
        &self.limbs[..self.limb_count.min(MAX_CAPE_LIMBS)]
    }
}

/// One slot's body for this tick's capes: the shoulders the top row pins to,
/// the facing, the ground point, and the range of `Capes::bodies` holding the
/// slot's own colliders.
#[derive(Clone, Copy, PartialEq, Debug)]
struct CapePose {
    shoulders: [[f32; 3]; 2],
    /// Sine and cosine of the slot's yaw.
    yaw_sc: (f32, f32),
    /// The slot root on the ground, meters.
    root: [f32; 3],
    /// Start and end of the slot's colliders in `Capes::bodies`.
    bodies: (usize, usize),
}

/// The cape layer of a [`FighterLook`]: one Verlet sheet per look slot.
struct Capes {
    /// Per rig: the joints its capes pin to, `None` when the rig has none.
    joints: Vec<Option<CapeJoints>>,
    /// Per slot: this tick's body, `None` for an unseen or unposed slot.
    poses: [Option<CapePose>; MAX_LOOK_FIGHTERS],
    /// Every posed slot's colliders this tick (torso and limbs), slot order.
    bodies: Vec<ClothCollider>,
    /// The colliders one cape steps against: its own body plus near rivals.
    near: Vec<ClothCollider>,
    /// Per slot: the sheet, `None` before the slot is first seen.
    sheets: Vec<Option<Cloth>>,
    /// Per slot: the body's backward direction, the side a cape hangs on.
    outward: [[f32; 3]; MAX_LOOK_FIGHTERS],
    /// Top-row scratch, one pin point per column: no allocation per tick.
    row: Vec<[f32; 3]>,
    /// Slots the last tick laid a sheet on.
    stepped: usize,
    /// Cape keys uploaded for the last frame, one per drawn cape.
    published: Vec<MeshKey>,
}

impl Capes {
    /// The layer for `rigs`: the cape joints of every rig, cold.
    fn for_rigs(rigs: &[Rig]) -> Self {
        Self::with_joints(
            rigs.iter()
                .map(|rig| CapeJoints::find(&rig.skeleton))
                .collect(),
        )
    }

    /// A layer with no rigs: the tests' looks that draw no bodies.
    #[cfg(test)]
    fn empty() -> Self {
        Self::with_joints(Vec::new())
    }

    /// A cold layer over per-rig cape joints.
    fn with_joints(joints: Vec<Option<CapeJoints>>) -> Self {
        Self {
            joints,
            poses: [None; MAX_LOOK_FIGHTERS],
            bodies: Vec::new(),
            near: Vec::new(),
            sheets: vec![None; MAX_LOOK_FIGHTERS],
            outward: [[0.0, 0.0, -1.0]; MAX_LOOK_FIGHTERS],
            row: Vec::new(),
            stepped: 0,
            published: Vec::new(),
        }
    }
}

/// Model-space point `joint` in world meters under the slot's `root`.
fn joint_world(world: &[[[f32; 4]; 4]], root: &[[f32; 4]; 4], joint: usize) -> [f32; 3] {
    let m = mul(&world[joint], root);
    [m[3][0], m[3][1], m[3][2]]
}

/// Write the cape's top row into `out`: `cols` points spanning `left` to
/// `right`, every one moved by `back`. Pure; `out` is reused by the caller.
fn cape_row_into(
    out: &mut Vec<[f32; 3]>,
    left: [f32; 3],
    right: [f32; 3],
    back: [f32; 3],
    cols: usize,
) {
    out.clear();
    let last = small_f32(cols.saturating_sub(1).max(1));
    for c in 0..cols {
        let t = small_f32(c) / last;
        out.push([
            left[0] + (right[0] - left[0]) * t + back[0],
            left[1] + (right[1] - left[1]) * t + back[1],
            left[2] + (right[2] - left[2]) * t + back[2],
        ]);
    }
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// One unit normal of the cloth at vertex `(c, r)`, turned onto `outward`'s
/// side so the cape's face points away from the body it hangs behind.
fn cape_normal(
    at: impl Fn(usize, usize) -> [f32; 3],
    cols: usize,
    rows: usize,
    c: usize,
    r: usize,
    outward: [f32; 3],
) -> [f32; 3] {
    let right = sub3(at((c + 1).min(cols - 1), r), at(c.saturating_sub(1), r));
    let down = sub3(at(c, (r + 1).min(rows - 1)), at(c, r.saturating_sub(1)));
    let mut n = cross3(right, down);
    let len2 = dot3(n, n);
    if len2.is_finite() && len2 > 1e-12 {
        let inv = 1.0 / len2.sqrt();
        n = [n[0] * inv, n[1] * inv, n[2] * inv];
    } else {
        n = [0.0, 1.0, 0.0];
    }
    if dot3(n, outward) < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    n
}

/// The cape's triangle mesh at its current positions: unit uvs over the grid,
/// normals on `outward`'s side. Pure; the sheet is a `cols` by `rows` grid hung
/// from its pinned top row.
fn cape_mesh(cloth: &Cloth, grid: ClothGrid, outward: [f32; 3]) -> MeshData {
    let (cols, rows) = (grid.cols, grid.rows);
    let positions = cloth.positions().to_vec();
    let at = |c: usize, r: usize| positions[r * cols + c];
    let mut normals = Vec::with_capacity(positions.len());
    let mut uvs = Vec::with_capacity(positions.len());
    for r in 0..rows {
        for c in 0..cols {
            normals.push(cape_normal(at, cols, rows, c, r, outward));
            uvs.push([
                small_f32(c) / small_f32(cols - 1),
                small_f32(r) / small_f32(rows - 1),
            ]);
        }
    }
    let mut indices = Vec::with_capacity((cols - 1) * (rows - 1) * 6);
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let a = u32::try_from(r * cols + c).unwrap_or(u32::MAX);
            let b = a + 1;
            let d = a + u32::try_from(cols).unwrap_or(u32::MAX);
            let e = d + 1;
            indices.extend_from_slice(&[a, b, d, b, e, d]);
        }
    }
    MeshData {
        positions,
        normals,
        uvs,
        indices,
        tangents: Vec::new(),
    }
}

impl FighterLook {
    /// Lay and step every seen slot's cape on the pose of the tick just folded.
    /// Call it once per accepted tick, after the act fold (`observe_tick`).
    /// Presentation only; the sim never reads a cape.
    fn cape_tick(&mut self) {
        self.capes.stepped = 0;
        self.capes.bodies.clear();
        for slot in 0..MAX_LOOK_FIGHTERS {
            self.capes.poses[slot] = self.cape_pose(slot);
        }
        for slot in 0..MAX_LOOK_FIGHTERS {
            self.cape_slot(slot);
        }
    }

    /// One slot's body for this tick's capes: sample the pose once, read the
    /// shoulders and push the torso and limb capsules onto `Capes::bodies`.
    /// `None` for an unseen slot, a rig with no cape joints or a refused pose.
    /// A refused capsule is left out; the rest of the body still collides.
    fn cape_pose(&mut self, slot: usize) -> Option<CapePose> {
        let anim = self.slots[slot];
        if !anim.seen {
            return None;
        }
        let rig_index = self.rig_for_slot(slot);
        let joints = self.capes.joints.get(rig_index).copied().flatten()?;
        let sample = {
            let rig = &self.rigs[rig_index];
            self.slot_sample(rig, slot, &anim, 1.0)
        }
        .ok()?;
        let at = |joint: usize| joint_world(&sample.world, &sample.root, joint);
        let start = self.capes.bodies.len();
        let torso =
            ClothCollider::capsule(at(joints.torso[1]), at(joints.torso[0]), CAPE_BODY_RADIUS_M);
        let limbs = joints
            .limbs()
            .iter()
            .map(|&(a, b, radius)| ClothCollider::capsule(at(a), at(b), radius));
        self.capes
            .bodies
            .extend(std::iter::once(torso).chain(limbs).flatten());
        Some(CapePose {
            shoulders: [at(joints.shoulders[0]), at(joints.shoulders[1])],
            yaw_sc: sample.yaw.sin_cos(),
            root: anim.pos,
            bodies: (start, self.capes.bodies.len()),
        })
    }

    /// One slot's cape for this tick: build the sheet on first sight, pin its
    /// top row to the shoulders and step it once against the slot's own body
    /// and every rival body within [`CAPE_NEIGHBOR_M`]. An unseen slot, a rig
    /// with no cape joints or a refused pose leaves no cape.
    fn cape_slot(&mut self, slot: usize) {
        let anim = self.slots[slot];
        if !anim.seen {
            self.capes.sheets[slot] = None;
            return;
        }
        let Some(pose) = self.capes.poses[slot] else {
            return;
        };
        let [left, right] = pose.shoulders;
        let (s, c) = pose.yaw_sc;
        // The body faces `(sin yaw, 0, cos yaw)`; the cape hangs on the other side.
        let back = [-s * CAPE_BACK_M, 0.0, -c * CAPE_BACK_M];
        let grid = CAPE_TIER.knobs().cape;
        cape_row_into(&mut self.capes.row, left, right, back, grid.cols);
        let mid = self.capes.row[grid.cols / 2];
        if self.capes.sheets[slot].is_none() {
            let params = ClothParams::for_tier(CAPE_TIER);
            match Cloth::new(grid, CAPE_SIZE_M, mid, params) {
                Ok(sheet) => self.capes.sheets[slot] = Some(sheet),
                Err(_) => return,
            }
        }
        // Every posed body close enough to lean into the sheet, the slot's own
        // (distance zero) included, in slot order: the same poses give the same
        // collider list.
        let mut near = std::mem::take(&mut self.capes.near);
        near.clear();
        for other in self.capes.poses.iter().flatten() {
            let (dx, dz) = (other.root[0] - pose.root[0], other.root[2] - pose.root[2]);
            if dx * dx + dz * dz <= CAPE_NEIGHBOR_M * CAPE_NEIGHBOR_M {
                near.extend_from_slice(&self.capes.bodies[other.bodies.0..other.bodies.1]);
            }
        }
        self.cape_step(slot, &anim, (s, c), pose.root[1], &near);
        self.capes.near = near;
    }

    /// Pin slot `slot`'s sheet to this tick's top row, lay its floor at the slot
    /// root's height, blow the body's own wind through it and step it once
    /// against `near`.
    fn cape_step(
        &mut self,
        slot: usize,
        anim: &SlotAnim,
        (s, c): (f32, f32),
        floor_y: f32,
        near: &[ClothCollider],
    ) {
        let row = std::mem::take(&mut self.capes.row);
        let Some(sheet) = self.capes.sheets[slot].as_mut() else {
            self.capes.row = row;
            return;
        };
        let pinned = sheet.pin_top_row(&row).is_ok();
        self.capes.row = row;
        if pinned {
            // The floor under the slot: a hem that crouches or falls lies on the
            // ground instead of hanging through it.
            if sheet.set_ground(Some(floor_y)).is_err() {
                return;
            }
            // The air a running fighter moves through: the body's own wind,
            // backward at its speed, so the cape streams behind it. Standing
            // still gives zero wind and the sheet stays asleep. Presentation
            // only; the speed is the tick's recorded travel, never the hash.
            let (dx, dz) = (
                anim.pos[0] - anim.prev_pos[0],
                anim.pos[2] - anim.prev_pos[2],
            );
            let speed = (dx * dx + dz * dz).sqrt() * TICK_HZ;
            let wind = if speed.is_finite() && speed > CAPE_WIND_MIN_M_S {
                let speed = speed.min(CAPE_WIND_MAX_M_S);
                [-s * speed, 0.0, -c * speed]
            } else {
                [0.0; 3]
            };
            if sheet.set_wind(wind).is_err() {
                return;
            }
            // One 60 Hz Verlet step, the tick the pose belongs to, against
            // every near body; a body with no valid capsule clears nothing.
            sheet.step(near);
            self.capes.outward[slot] = [-s, 0.0, -c];
            self.capes.stepped += 1;
        }
    }

    /// Upload the cape mesh of every seen slot that has a sheet, so
    /// [`Self::push_cape_draws`] can draw it. Called from `stage_bodies`.
    fn publish_capes(&mut self, presenter: &mut Presenter) {
        self.capes.published.clear();
        let grid = CAPE_TIER.knobs().cape;
        for slot in 0..MAX_LOOK_FIGHTERS {
            if !self.slots[slot].seen {
                continue;
            }
            let Some(sheet) = self.capes.sheets[slot].as_ref() else {
                continue;
            };
            let key = cape_mesh_key(slot);
            let mesh = cape_mesh(sheet, grid, self.capes.outward[slot]);
            if presenter.upload_mesh(key, &mesh).is_ok() {
                self.capes.published.push(key);
            }
        }
    }

    /// Push one cloth draw per cape mesh this frame published. Returns draws. A
    /// key the presenter does not hold draws nothing, so a failed publish never
    /// fails the frame.
    fn push_cape_draws(&self, list: &mut DrawList, resident: &[MeshKey]) -> usize {
        let material = PbrMaterial {
            base_color: CAPE_COLOR,
            metallic: 0.0,
            roughness: CAPE_ROUGHNESS,
            exposure: 1.0,
        };
        let mut pushed = 0usize;
        for key in &self.capes.published {
            if !resident.contains(key) {
                continue;
            }
            list.items.push(DrawItem {
                mesh: *key,
                material: material.clone(),
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
            pushed += 1;
        }
        pushed
    }

    /// Seen fighters the last tick wore a cape for, and the capes the last
    /// frame drew: the receipt pair a capture reports.
    #[must_use]
    pub fn capes(&self) -> (usize, usize) {
        (self.capes.stepped, self.capes.published.len())
    }

    /// The cape sheet of `slot` at its current particle positions, or `None`
    /// when that slot has none: the capture and the tests read it.
    #[must_use]
    pub fn cape_positions(&self, slot: usize) -> Option<&[[f32; 3]]> {
        self.capes
            .sheets
            .get(slot)
            .and_then(|sheet| sheet.as_ref())
            .map(Cloth::positions)
    }
}

/// Mesh id base for skinned body parts (clear of the cube, dressing, kit
/// and staged-box keys): `base + slot * 16 + part`.
pub const LOOK_MESH_ID_BASE: u64 = 0xF16B_0000;
/// Texture id base for body base-color maps: `base + rig * 16 + part`.
pub const LOOK_TEXTURE_ID_BASE: u64 = 0xF16C_0000;
/// Most fighters drawn (the match cap).
pub const MAX_LOOK_FIGHTERS: usize = 8;
/// Steepest fight-camera look down: rise per metre of eye-to-focus reach.
const MAX_PITCH_SLOPE: f32 = 0.75;
/// Rise of the fight eye per metre of shot distance. A pack shot twenty-two
/// metres out sat 6.8 m up at 0.22 (a drone over the pit, measured on
/// 2026-10-20); 0.06 keeps it near head height while the fit still sees over
/// the near bodies. Directed captures keep their own orbit and height.
const FIGHT_RISE_SLOPE: f32 = 0.06;
/// Fixed ticks per second (clip clocks advance with match ticks).
/// Single owner of the rate is `engine_core::TICK_HZ`.
const TICK_HZ: f32 = engine_core::TICK_HZ as f32;
/// Ticks between swing start and the impact cue (about mid-swing).
const IMPACT_TICKS: u32 = 12;
/// Ticks a hit victim stays tinted: 0.25 s, as long as the light-attack
/// tell that caused the hit, so one hit reads on screen and in the tally.
const FLASH_TICKS: u32 = 15;
/// Share of the green channel a flashing body keeps.
const FLASH_GREEN_KEEP: f32 = 0.35;
/// Share of the blue channel a flashing body keeps.
const FLASH_BLUE_KEEP: f32 = 0.3;
/// Exposure a flashing body draws at.
const FLASH_EXPOSURE: f32 = 1.6;
/// Farthest drawn attacker-to-victim gap that still tints the victim.
/// Rules v1 lands every swing on every living foe at any range, so without
/// this gate all eight bodies strobe red from the first exchange on.
const TINT_REACH_M: f32 = 3.0;
/// Planar speed above which a fighter reads as running (meters per tick).
const RUN_SPEED_M_PER_TICK: f32 = 0.012;
/// Reference planar speed where the run clip plays at its authored rate
/// (meters per tick; presentation only, never in the tick hash).
const RUN_REF_M_PER_TICK: f32 = 0.05;
/// Largest facing turn per tick in radians (about 14 degrees).
const MAX_TURN_PER_TICK: f32 = 0.25;
/// Seconds of angular budget a limiter lane takes when it is first seeded:
/// at the look's cap this accepts any pose whole, so a first frame never
/// ramps in from identity. Presentation only.
const SEED_S: f32 = 1.0;
/// Ticks short of a tick's end the limiter reads its wanted pose: the same
/// hair the inertial switch fold uses, so a swing clip that runs out on this
/// tick is not sampled a tick before the offsets smooth that cut.
const LAG_TICKS: f32 = 0.001;
/// Most parts per body (atlas quadrants); keys reserve 16 per slot.
const MAX_PARTS: usize = 16;
/// Pelvis height for a fallen slot: lies flat, below 0.4 m (EG45).
/// Presentation only; the sim hash never reads it.
const BALANCE_FALLEN_PELVIS_M: f32 = 0.25;
/// Pelvis height for a catching slot: kneels (EG45). Presentation only.
const BALANCE_CATCH_PELVIS_M: f32 = 0.45;
/// Forward lean for a stumbling slot in meters (EG45). Presentation only.
const BALANCE_STUMBLE_LEAN_M: f32 = 0.18;
/// Largest sand shift in meters, up or down (LF17, view only): the fallen
/// pelvis (EG45) puts the start of the death clip about 70 cm under the sand
/// and its rest pose hovers about 10 cm over it, so the cap sits above both
/// and still stops a bad pose from flying or diving.
const SAND_LIFT_MAX_M: f32 = 1.0;
/// View-only shake added per in-reach Hit, in permille (LF11, view only).
/// Presentation only; the sim hash never reads it.
const SHAKE_HIT_PERMILLE: u32 = 400;
/// View-only shake added per in-reach Knockdown, in permille (LF11).
/// Presentation only; the sim hash never reads it.
const SHAKE_KNOCKDOWN_PERMILLE: u32 = 700;
/// Clamp for the fight-camera trauma, in permille (LF11).
const SHAKE_MAX_PERMILLE: u32 = 1000;
/// Trauma decayed per tick: 1000 per second at 60 Hz needs 16.67 per tick,
/// so 17 (the ceiling) clears full trauma in 59 ticks, inside 1.0 s.
/// Integer-only, so the same ticks give the same trauma twice.
const SHAKE_DECAY_PERMILLE_PER_TICK: u32 = 17;
/// Shake cap in meters at full trauma (LF11): 0.12 m, the arena director
/// default, so the fight camera matches the staged rig at full trauma.
const SHAKE_MAX_M: f32 = 0.12;

/// Clip files under `glb/clips/`, in [`ClipRole`] order.
const CLIP_FILES: [&str; 6] = [
    "human_male_idle_combat_main_hand_melee.glb",
    "human_male_run.glb",
    "human_male_cast_main_hand_melee.glb",
    "human_male_cast_main_hand_melee_2.glb",
    "human_male_cast_main_hand_melee_3.glb",
    "human_male_death.glb",
];

/// Logical path of one baked clip inside the fighters content tree
/// (`glb/clips/<file>`).
#[must_use]
fn clip_logical_path(file: &str) -> String {
    format!("glb/clips/{file}")
}

/// Load and frame failures.
#[derive(Clone, PartialEq, Debug)]
pub enum LookError {
    /// Bundle bind refused (manifest or body file).
    Bind(String),
    /// A body's bytes differ from the pinned CT7 SHA-256.
    ShaMismatch(String),
    /// Body or clip import failed.
    Import(String),
    /// Skinning or mesh validation failed.
    Skin(String),
    /// The presenter refused an upload.
    Upload(String),
    /// The fight ring (floor, tiers, textures) refused to build or stage.
    Ring(String),
}

impl core::fmt::Display for LookError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bind(msg) => write!(f, "fighter look bind: {msg}"),
            Self::ShaMismatch(name) => write!(f, "fighter look: {name} is not the CT7 bytes"),
            Self::Import(msg) => write!(f, "fighter look import: {msg}"),
            Self::Skin(msg) => write!(f, "fighter look skin: {msg}"),
            Self::Upload(msg) => write!(f, "fighter look upload: {msg}"),
            Self::Ring(msg) => write!(f, "fighter look ring: {msg}"),
        }
    }
}

impl std::error::Error for LookError {}

/// What a fighter is doing on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Guard stance, looping.
    Idle,
    /// Moving across the sand, looping.
    Run,
    /// One sword swing (variant cycles per swing).
    Attack(u8),
    /// Knocked down: death clip, held on its last frame.
    Down,
}

/// An act's name in the view truth (`idle`, `run`, `attack`, `down`).
fn act_label(act: Act) -> &'static str {
    match act {
        Act::Idle => "idle",
        Act::Run => "run",
        Act::Attack(_) => "attack",
        Act::Down => "down",
    }
}

/// One body part ready to skin: bind mesh, rows, texture and factors.
struct RigPart {
    mesh: MeshData,
    skin: SkinWeights,
    texture: Option<(TextureKey, TextureUpload)>,
    material: PbrMaterial,
}

/// One variant: skeleton, inverse binds, parts and its retargeted clips.
struct Rig {
    variant: String,
    skeleton: Skeleton,
    inverse_binds: Vec<[[f32; 4]; 4]>,
    pelvis: usize,
    /// Torso joint: the hit-react lean turns it about the push axis (AM23).
    torso: usize,
    /// Head joint: the lean adds a snap and the metric reads its travel.
    head: usize,
    bind_pelvis: [f32; 3],
    /// Skin radius in meters: the farthest any bind-pose vertex sits from
    /// its nearest bind joint, plus a 10 cm margin. The joints-only pose
    /// pads placed joints by this radius so its bounds hold the skinned
    /// vertex bounds; cold-computed at load, read-only on the hot path.
    radius_m: f32,
    pipe: SkinPipeline,
    parts: Vec<RigPart>,
    /// Clips in [`CLIP_FILES`] order, on this rig's skeleton.
    clips: Vec<ImportedClip>,
    /// Both legs by joint name; `None` plants no feet (LF12).
    legs: Option<Legs>,
    /// The right arm by joint name; `None` throws nothing (SG3).
    throw_arm: Option<engine_anim::ik::Limb>,
}

/// Presentation state for one slot.
#[derive(Clone, Copy, PartialEq, Debug)]
struct SlotAnim {
    act: Act,
    act_start: u32,
    /// The bridge's fade, mirrored in step with it: the leaving clip, its
    /// frozen time and the ticks left. Since LF9 the pose reads none of it (a
    /// clip switch rides on the `iz_age` offsets); the wire test and the
    /// hitstop hold proof pin the mirror.
    prev_clip: usize,
    prev_time: f32,
    fade_left: u32,
    swings: u8,
    prev_pos: [f32; 3],
    pos: [f32; 3],
    prev_yaw: f32,
    yaw: f32,
    flash_until: u32,
    seen: bool,
    /// Planar meters per tick, set beside the speed pick in `fold_acts`.
    /// Presentation only; never enters the tick hash.
    run_speed: f32,
    /// Seconds of run clip played when the run act's current tick began: a
    /// phase summed tick by tick at each tick's own rate, so a speed change
    /// bends the stride rate and never moves the phase. Zero outside the run
    /// act. Presentation only; never enters the tick hash.
    run_phase_s: f32,
    /// Integer balance gait for the look (EG45): steady reads as before,
    /// stumbling leans, fallen lies flat, catching kneels. Presentation
    /// only; the sim digest never reads this.
    balance: GaitPhase,
    /// First tick the additive flinch shows on (`u32::MAX` reads as none).
    /// Presentation only; the sim hash never reads it.
    hit_start: u32,
    /// World-space planar push away from the attacker at `hit_start`,
    /// unit length or zero. Presentation only.
    hit_push: [f32; 2],
    /// True while the sim's hitstop holds this slot's pose (LF10): the clip
    /// clock, the blend, the flinch and the facing stand still, and every
    /// frame of the freeze samples the pose of its tick. Presentation only.
    held: bool,
    /// Free ticks since the last clip switch started its inertial offsets
    /// (LF9); [`fighter_look_inertia::NO_OFFSET`] when none is in flight. The
    /// clock runs with the clip clock, so a held slot's age stands still.
    /// Presentation only.
    iz_age: u32,
    /// The slot's planted feet (LF12): where each foot is held on the sand.
    /// They stand still with the clip clock in hitstop. Presentation only.
    feet: Feet,
    /// The spear throw's clock (SG3): ticks since the spear left the hand.
    /// Presentation only.
    throw: Throw,
}

impl SlotAnim {
    const fn new() -> Self {
        Self {
            act: Act::Idle,
            act_start: 0,
            prev_clip: 0,
            prev_time: 0.0,
            fade_left: 0,
            swings: 0,
            prev_pos: [0.0; 3],
            pos: [0.0; 3],
            prev_yaw: 0.0,
            yaw: 0.0,
            flash_until: 0,
            seen: false,
            run_speed: 0.0,
            run_phase_s: 0.0,
            balance: GaitPhase::Steady,
            hit_start: u32::MAX,
            hit_push: [0.0, 0.0],
            held: false,
            iz_age: fighter_look_inertia::NO_OFFSET,
            feet: Feet::new(),
            throw: Throw::NONE,
        }
    }

    /// Carry the run clip's phase over `ticks` ticks at the rate this slot's
    /// recorded speed gives: the speed it showed over the ticks that ended.
    /// Any other act has no run phase to carry. Pure; no allocation.
    fn march_run_phase(&mut self, ticks: u32) {
        if self.act == Act::Run {
            let ticks = f32::from(u16::try_from(ticks).unwrap_or(u16::MAX));
            self.run_phase_s += ticks / TICK_HZ * run_phase_scale(self.run_speed);
        }
    }

    /// Run clip seconds `alpha` ticks into this slot's tick: the carried
    /// phase plus that share of the tick at the tick's own rate. Never
    /// negative, so a look back from before the run began reads its first
    /// frame.
    fn run_clip_s(&self, alpha: f32) -> f32 {
        (self.run_phase_s + alpha / TICK_HZ * run_phase_scale(self.run_speed)).max(0.0)
    }
}

/// Playback scale for the run clip from planar speed: speed over
/// [`RUN_REF_M_PER_TICK`], clamped to half through double rate. Pure
/// function of speed; the run phase sums one scale per tick
/// ([`SlotAnim::march_run_phase`]), so a changed speed changes the stride
/// rate and the phase stays put. Non-finite speeds hold the authored rate.
fn run_phase_scale(speed_m_per_tick: f32) -> f32 {
    if !speed_m_per_tick.is_finite() {
        return 1.0;
    }
    (speed_m_per_tick / RUN_REF_M_PER_TICK).clamp(0.5, 2.0)
}

/// Sub-tick alpha for the clip clock (LF10): a held slot samples the end of
/// its tick on every frame of the freeze, so no frame between two sim ticks
/// plays on. The end of the tick is where the last free frame stopped and
/// where the first free frame after the freeze starts, so the pose neither
/// rewinds going in nor jumps coming out. Position and facing keep the
/// caller's alpha; a held slot's do not move. Pure.
#[must_use]
fn clip_alpha(slot: &SlotAnim, alpha: f32) -> f32 {
    if slot.held {
        1.0
    } else {
        alpha
    }
}

/// Pelvis height for one balance gait (EG45): steady and stumbling keep
/// the clip height, fallen lies flat below 0.4 m, catching kneels.
/// Pure, no allocation; non-finite clip heights fall back to the gait
/// target (steady falls back to 0.0) so no NaN ever reaches the skin.
#[must_use]
fn balance_pelvis_y_m(gait: GaitPhase, clip_y: f32) -> f32 {
    match gait {
        GaitPhase::Fallen => BALANCE_FALLEN_PELVIS_M,
        GaitPhase::Catching => BALANCE_CATCH_PELVIS_M,
        GaitPhase::Steady | GaitPhase::Stumbling => {
            if clip_y.is_finite() {
                clip_y
            } else {
                0.0
            }
        }
    }
}

/// Forward lean for one balance gait in meters (EG45): only stumbling
/// leans its root; every other gait holds its ground. Pure.
#[must_use]
fn balance_lean_m(gait: GaitPhase) -> f32 {
    match gait {
        GaitPhase::Stumbling => BALANCE_STUMBLE_LEAN_M,
        GaitPhase::Steady | GaitPhase::Fallen | GaitPhase::Catching => 0.0,
    }
}

/// Shift that rests a body's lowest vertex on the sand (LF17, view only): the
/// depth below the sand lifts it, a hover above the sand lowers it, clamped to
/// `-SAND_LIFT_MAX_M..=SAND_LIFT_MAX_M`. Pure `f32` in/out; a non-finite
/// height reads as 0.
#[must_use]
fn sand_rest_shift_m(lowest_y: f32) -> f32 {
    if !lowest_y.is_finite() {
        return 0.0;
    }
    (crate::view_truth::GROUND_Y - lowest_y).clamp(-SAND_LIFT_MAX_M, SAND_LIFT_MAX_M)
}

/// Full below-sand depth, clamped `0.0..=SAND_LIFT_MAX_M` (LF17, view only).
/// Pure `f32` in/out; non-finite or above sand reads as 0, never lowers. The
/// joints-only pose uses it: its padded bounds sit under the true vertices,
/// so a hover read from them would be false.
#[must_use]
fn sand_lift_m(lowest_y: f32) -> f32 {
    sand_rest_shift_m(lowest_y).max(0.0)
}

/// True for a body that lies on the sand: the death clip (`Down`) or the sim's
/// `Fallen` verdict. Only these rest both ways; every other body is lifted
/// only when its lowest vertex is under the sand, never pulled down (LF17,
/// view only).
#[must_use]
fn lies_on_sand(slot: &SlotAnim) -> bool {
    slot.act == Act::Down || slot.balance == GaitPhase::Fallen
}

/// Whole microseconds of wall clock, saturating. Evidence only: the fighter
/// skin split ([`SkinSplit`]) reports through it and nothing hashed reads it.
fn micros(elapsed: std::time::Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Body parts left resident for the frame's draws, split into the CPU pose
/// half and the GPU upload half of the fighter skin wall.
///
/// Consumer: [`FighterLook::present_into`] returns it inside
/// [`PresentReport`] and `crates/game/src/bin/arena.rs` folds both halves
/// into the `arena frame-split` line beside the untouched `fighters` lump.
/// Data owner: this module, read-only for its consumers. Units: parts,
/// microseconds of wall clock sampled with [`engine_clock::now`] only (no map, poll or
/// wait). Determinism: evidence only; the counters never enter a draw, the
/// view truth or hashed state. A zero means no `SkinnedFrame`s reached the
/// upload path this call, never a second timing law.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SkinSplit {
    /// Parts the frame's draws read this call: one per part with finite
    /// posed vertices, whether it was written or already resident.
    pub uploaded: usize,
    /// `uploaded`'s CPU half: clip sample, skin, pose fold, the sand rest and
    /// the upload digest ([`FighterLook::pose_frame`] plus the per-part fold).
    pub pose_us: u64,
    /// `uploaded`'s GPU half: the staged value plus `Presenter::upload_mesh`
    /// per part (validation, tangent rebuild, byte pack, buffer write). A
    /// part whose digest is unchanged runs neither and adds nothing.
    pub upload_us: u64,
}

/// One presented frame's cost report: the draws pushed for the frame and the
/// split of the fighter skin wall behind them.
///
/// Consumer: `crates/game/src/bin/arena.rs` (`ArenaApp::push_fighters`) adds
/// `skin.pose_us`/`skin.upload_us` to its `fighters_pose_us`/
/// `fighters_upload_us` counters, printed as `fighters_pose=`/
/// `fighters_upload=` on the `arena frame-split` line. Units: draws, parts,
/// microseconds of wall clock. Determinism: evidence only; the draws stay the
/// frame's draw list and the counters never enter hashed state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PresentReport {
    /// Draws pushed this frame: the staged ring plus the skinned body parts.
    pub draws: usize,
    /// Parts left resident for those draws and the two wall halves.
    pub skin: SkinSplit,
}

/// The value [`upload_slot_parts`] hands to `Presenter::upload_mesh`: posed
/// positions and normals over the rig part's own uvs and indices.
///
/// `Presenter::upload_mesh` revalidates its input and rebuilds the tangents
/// from those four arrays on every call, and never reads the value's
/// `tangents`; running `v1::upload_mesh` here as well repeated the whole
/// validate plus tangent pass on every changed part. `tangents` stays empty on
/// purpose: nothing but the presenter reads this value, and a bad vertex
/// still refuses there, before any buffer is written.
fn staged_part_mesh(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    rig_mesh: &MeshData,
) -> MeshData {
    MeshData {
        positions,
        normals,
        uvs: rig_mesh.uvs.clone(),
        indices: rig_mesh.indices.clone(),
        tangents: Vec::new(),
    }
}

/// True when every part of `slot_index` is resident as a skinned mesh on the
/// slot's own joint block (staged by [`FighterLook::stage_skinned_meshes`]),
/// so a frame only has to write that block. Pure reads of the presenter.
fn slot_is_gpu_skinned(presenter: &Presenter, slot_index: usize, rig: &Rig) -> bool {
    !rig.parts.is_empty()
        && (0..rig.parts.len()).all(|part_index| {
            presenter.skin_block_of(&look_mesh_key(slot_index, part_index)) == Some(slot_index)
        })
}

/// The GPU-skin frame write for one slot: its joint matrices (sand rest
/// folded in) into the slot's block, nothing else. The `parts` count reads
/// as uploaded (they are resident and drawn); the single block write is the
/// frame's whole upload half. A refusal (a non-finite matrix) is loud.
fn write_slot_skin(
    presenter: &mut Presenter,
    slot_index: usize,
    parts: usize,
    matrices: &[[[f32; 4]; 4]],
) -> Result<SkinSplit, LookError> {
    let started = engine_clock::now();
    presenter
        .write_skin_block(slot_index, matrices)
        .map_err(|err| LookError::Upload(err.to_string()))?;
    Ok(SkinSplit {
        uploaded: parts,
        pose_us: 0,
        upload_us: micros(engine_clock::since(started)),
    })
}

/// Upload one slot's skinned parts with the sand rest applied (LF17, view
/// only): lifted positions for the draw, normals ride unchanged, tangents
/// recomputed from the lifted positions plus UVs. The GPU vertex is 48 B
/// (pos + normal + uv + tangent, `MeshData::vertex_bytes`); the in-place
/// `refresh_mesh_vertices` path writes 32 B (pos + normal + uv) and leaves
/// the 48 B stride misaligned past the first vertex, stretching every other
/// fighter triangle across the screen (1209Z streaks). Until that path carries
/// tangents, every *changed* skinned frame re-uploads the full mesh (correct
/// stride) so fighters render with limbs and armor.
///
/// The upload is skipped when the frame's lifted positions plus skinned
/// normals hash to the part's resident digest (`digests`, keyed by
/// `slot * MAX_PARTS + part`): the resident bytes already are the frame's
/// bytes, so a frozen body re-uploads nothing and a moved one re-uploads
/// whole. `uploaded` counts the parts left resident for the frame either way.
/// Returns that count beside the two halves they cost: the lift plus digest
/// fold into `pose_us`, the [`staged_part_mesh`] plus `Presenter::upload_mesh`
/// step into `upload_us` ([`SkinSplit`]).
/// [found2 | 7.6.2 Rendering with Normal Maps]
fn upload_slot_parts(
    presenter: &mut Presenter,
    slot_index: usize,
    rig: &Rig,
    frames: Vec<engine_render::v1_skin::SkinnedFrame>,
    lift: f32,
    digests: &mut [Option<u64>],
) -> Result<SkinSplit, LookError> {
    let mut split = SkinSplit::default();
    for (part_index, posed) in frames.into_iter().enumerate() {
        let part = &rig.parts[part_index];
        let lifted_started = engine_clock::now();
        // Digest the exact bytes the pair would upload: the lifted positions
        // and the skinned normals (uvs and indices belong to the rig and
        // never change). Hashing rides the lift pass, so a changed frame pays
        // one pass over the vertices it already touches.
        let mut hasher = Fnv64::new();
        let lifted: Vec<[f32; 3]> = if lift != 0.0 {
            let mut out = Vec::with_capacity(posed.positions.len());
            for position in &posed.positions {
                let moved = [position[0], position[1] + lift, position[2]];
                for component in moved {
                    hasher.write_f32(component);
                }
                out.push(moved);
            }
            out
        } else {
            for position in &posed.positions {
                for component in position {
                    hasher.write_f32(*component);
                }
            }
            posed.positions
        };
        for normal in &posed.normals {
            for component in normal {
                hasher.write_f32(*component);
            }
        }
        let digest = hasher.finish();
        split.pose_us = split
            .pose_us
            .saturating_add(micros(engine_clock::since(lifted_started)));
        let slot_part = slot_index * MAX_PARTS + part_index;
        if digests.get(slot_part) == Some(&Some(digest)) {
            // Resident bytes already are this frame's bytes: no upload.
            split.uploaded += 1;
            continue;
        }
        let key = look_mesh_key(slot_index, part_index);
        // 48 B stride fix: always re-upload with recomputed tangents; never
        // refresh 32 B in place (misaligned past vertex 0, streaks).
        let upload_started = engine_clock::now();
        let mesh = staged_part_mesh(lifted, posed.normals, &part.mesh);
        presenter
            .upload_mesh(key, &mesh)
            .map_err(|err| LookError::Upload(err.to_string()))?;
        split.upload_us = split
            .upload_us
            .saturating_add(micros(engine_clock::since(upload_started)));
        if let Some(resident) = digests.get_mut(slot_part) {
            *resident = Some(digest);
        }
        split.uploaded += 1;
    }
    Ok(split)
}

// Note: the authoritative read is `engine-arena` `Sim::balance` (already
// landed with EG34) via `LiveMatch::balance`; the window polls it each
// tick through `FighterLook::observe_live_balances`. The cue mirror
// (`Hit` stumbles, `Knockdown` floors) stays for the same-tick swing
// start, so the same blows lean the same roots.

/// Contact blob radius under a standing fighter, meters.
const CONTACT_STAND_RADIUS_M: f32 = 0.85;
/// Contact blob radius under a floored fighter, meters.
const CONTACT_DOWN_RADIUS_M: f32 = 1.3;

/// Camera shot for the fight camera.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shot {
    /// Close on the hero (slot 0).
    Hero,
    /// Wide establishing view of the arena with the fight in it.
    Wide,
    /// Mid shot fitting every fighter.
    Action,
}

/// One presented frame's look inputs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookFrame {
    /// Fighters in the match.
    pub fighters: usize,
    /// Sub-tick interpolation in `0..=1`.
    pub alpha: f32,
    /// Camera shot for this frame.
    pub shot: Shot,
    /// Presented frame index (slow camera orbit).
    pub frame: u32,
}

/// Shot for presented frame `frame` of a `frames`-long capture run: a
/// directed run (capture camera path) splits into thirds, hero close-up,
/// then the wide arena, then the fight; free play keeps the fight shot.
#[must_use]
pub fn shot_for_frame(frame: u32, frames: Option<u32>, directed: bool) -> Shot {
    match frames {
        Some(total) if directed && total >= 3 => {
            let third = total / 3;
            if frame < third {
                Shot::Hero
            } else if frame < third * 2 {
                Shot::Wide
            } else {
                Shot::Action
            }
        }
        _ => Shot::Action,
    }
}

/// Whether a world point lands in `camera`'s picture: in front of the near
/// plane and inside the field of view. False for a camera that cannot build
/// a view.
#[cfg(test)]
fn in_picture(camera: &Camera, at: [f32; 3]) -> bool {
    count_in_picture(camera, std::iter::once(at)) == 1
}

/// How many of `points` land in `camera`'s picture (see [`in_picture`]): the
/// one owner of the picture test, with a single view build for the batch.
fn count_in_picture(camera: &Camera, points: impl Iterator<Item = [f32; 3]>) -> usize {
    let Ok(view) = camera.view_matrix() else {
        return 0;
    };
    let half = (camera.fov_y * 0.5).tan();
    points
        .filter(|at| {
            let mut point = camera.eye;
            (point.x, point.y, point.z) = (at[0], at[1], at[2]);
            let local = view.transform_point3(point);
            let depth = -local.z;
            depth > camera.near
                && local.y.abs() <= depth * half
                && local.x.abs() <= depth * half * camera.aspect
        })
        .count()
}

/// Degrees `camera` looks down below the horizon (negative looks up).
#[must_use]
pub fn look_down_deg(camera: &Camera) -> f32 {
    let dir = camera.target - camera.eye;
    (-dir.y).atan2(dir.x.hypot(dir.z)).to_degrees()
}

/// Metres the eye can back away from `focus` along `azimuth` before it
/// leaves the square pit of half-width `limit`.
fn pit_room(focus: [f32; 3], azimuth: f32, limit: f32) -> f32 {
    let along = |at: f32, dir: f32| {
        if dir > 1e-4 {
            (limit - at) / dir
        } else if dir < -1e-4 {
            (-limit - at) / dir
        } else {
            f32::MAX
        }
    };
    along(focus[0], azimuth.sin())
        .min(along(focus[2], azimuth.cos()))
        .max(0.0)
}

/// Orbit side and reach for a fight eye `distance` from `focus`: the wanted
/// azimuth while the pit has room for it, else the mirror side-on view or a
/// diagonal that does, else the roomiest of them pulled in along its own
/// line. Clamping x and z apart once parked the eye over a duel by the wall,
/// looking straight down at the sand (perf window, 2026-10-11).
fn pit_orbit(focus: [f32; 3], azimuth: f32, distance: f32, limit: f32) -> (f32, f32) {
    use std::f32::consts::{FRAC_PI_4, PI};
    let mut best = (azimuth, 0.0f32);
    for turn in [
        0.0,
        PI,
        FRAC_PI_4,
        -FRAC_PI_4,
        3.0 * FRAC_PI_4,
        -3.0 * FRAC_PI_4,
    ] {
        let side = azimuth + turn;
        let room = pit_room(focus, side, limit);
        if room >= distance {
            return (side, distance);
        }
        if room > best.1 {
            best = (side, room);
        }
    }
    best
}

/// One fight framing: what the eye looks at and where it orbits.
#[derive(Clone, Copy)]
struct ShotSpec {
    /// Floor point the eye aims at, meters.
    focus: [f32; 3],
    /// Wanted eye distance from `focus` along the floor, meters.
    distance: f32,
    /// Eye height; `0.0` derives it from `distance`.
    height: f32,
    /// Height of the aim point, meters.
    look_y: f32,
    /// Wanted orbit azimuth, radians.
    azimuth: f32,
}

/// The fight lens for `spec`: eye kept inside the pit (`limit`) and under the
/// pitch cap, both ends moved by `shake`. `base` comes back when the pose
/// cannot validate. Pure presentation.
fn shot_camera(base: &Camera, spec: ShotSpec, limit: f32, shake: [f32; 3]) -> Camera {
    let height = if spec.height > 0.0 {
        spec.height
    } else {
        1.6 + spec.distance * FIGHT_RISE_SLOPE
    };
    let (azimuth, reach) = pit_orbit(spec.focus, spec.azimuth, spec.distance, limit);
    let eye = [
        spec.focus[0] + azimuth.sin() * reach + shake[0],
        height.min(spec.look_y + reach * MAX_PITCH_SLOPE) + shake[1],
        spec.focus[2] + azimuth.cos() * reach + shake[2],
    ];
    let target = [
        spec.focus[0] + shake[0],
        spec.look_y + shake[1],
        spec.focus[2] + shake[2],
    ];
    let mut camera = base.clone();
    camera.eye = eye.into();
    camera.target = target.into();
    camera.up = [0.0, 1.0, 0.0].into();
    camera.fov_y = 60.0f32.to_radians();
    camera.near = 0.1;
    camera.far = camera.far.max(120.0);
    if camera.validate().is_ok() {
        camera
    } else {
        base.clone()
    }
}

/// Nearest group-shot eye distance tried, meters.
const WIDE_START_M: f32 = 8.0;
/// Step between tried group-shot eye distances, meters.
const WIDE_STEP_M: f32 = 1.0;
/// Farthest group-shot eye distance, meters (inside the 120 m far plane).
const WIDE_MAX_M: f32 = 48.0;
/// Orbit azimuths tried per group shot: the wanted one and its mirrors.
const WIDE_ORBITS: u8 = 16;
/// Spare pit room (meters) that makes an orbit comfortable: the wanted orbit
/// keeps the shot while it holds this much, so a body drifting at the edge of
/// the pit does not flip the eye across the arena.
const WIDE_SLACK_M: f32 = 3.0;
/// Halvings that tighten a found eye distance below [`WIDE_STEP_M`].
const WIDE_REFINE: u8 = 4;

/// A wider eye for the seen bodies (`seen`, floor positions) when `current`
/// drops one. Orbits nearest the `wanted` azimuth go first, each walked out
/// from [`WIDE_START_M`] while the pit has room for the eye; the first orbit
/// that holds every body with [`WIDE_SLACK_M`] to spare wins, else the one
/// with the most spare room. When no orbit holds them all, the eye that holds
/// the most, if more than `current` does. `None` keeps `current`. Pure
/// presentation.
fn wide_group_camera(
    base: &Camera,
    seen: &[[f32; 3]],
    current: &Camera,
    limit: f32,
    wanted: f32,
    shake: [f32; 3],
) -> Option<Camera> {
    let chests = || seen.iter().map(|at| [at[0], 1.0, at[2]]);
    let kept = count_in_picture(current, chests());
    if kept >= seen.len() {
        return None;
    }
    let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
    for at in seen {
        low = [low[0].min(at[0]), low[1].min(at[2])];
        high = [high[0].max(at[0]), high[1].max(at[2])];
    }
    let focus = [0.5 * (low[0] + high[0]), 0.0, 0.5 * (low[1] + high[1])];
    let eye_at = |azimuth: f32, distance: f32| {
        let spec = ShotSpec {
            focus,
            distance,
            height: 0.0,
            look_y: 1.0,
            azimuth,
        };
        shot_camera(base, spec, limit, shake)
    };
    let mut whole: Option<(f32, Camera)> = None;
    let mut part: Option<(usize, Camera)> = None;
    for orbit in 0..WIDE_ORBITS {
        let steps = f32::from(orbit.div_ceil(2));
        let side = if orbit % 2 == 1 { steps } else { -steps };
        let azimuth = wanted + side * std::f32::consts::FRAC_PI_8;
        let room = pit_room(focus, azimuth, limit).min(WIDE_MAX_M);
        let mut distance = WIDE_START_M;
        while distance <= room {
            let camera = eye_at(azimuth, distance);
            let held = count_in_picture(&camera, chests());
            if held >= seen.len() {
                // Tighten between the last miss and this hit, so the eye
                // glides with the bodies instead of stepping a whole meter.
                let (mut near, mut hit) = (distance - WIDE_STEP_M, (distance, camera));
                let halvings = if distance > WIDE_START_M {
                    WIDE_REFINE
                } else {
                    0
                };
                for _ in 0..halvings {
                    let mid = 0.5 * (near + hit.0);
                    let tried = eye_at(azimuth, mid);
                    if count_in_picture(&tried, chests()) >= seen.len() {
                        hit = (mid, tried);
                    } else {
                        near = mid;
                    }
                }
                let slack = room - hit.0;
                if slack >= WIDE_SLACK_M {
                    return Some(hit.1);
                }
                if whole.as_ref().is_none_or(|(most, _)| slack > *most) {
                    whole = Some((slack, hit.1));
                }
                break;
            }
            if held > kept && part.as_ref().is_none_or(|(most, _)| held > *most) {
                part = Some((held, camera));
            }
            distance += WIDE_STEP_M;
        }
    }
    whole
        .map(|(_, camera)| camera)
        .or_else(|| part.map(|(_, camera)| camera))
}

/// The fighter look: rigs, per-slot anim state and the last tick seen.
pub struct FighterLook {
    rigs: Vec<Rig>,
    /// Sand pit plus stone tiers drawn under the fighters once staged.
    ring: Option<FightRing>,
    slots: Vec<SlotAnim>,
    /// One bridge slot per look slot: the production owner of the act pick
    /// plus the fade driver. Driven in `fold_acts` from the same views, hp
    /// and cues; sampling stays in `slot_pose`. Presentation only.
    bridge: Vec<FighterSlot>,
    /// Blade trail ring, view only: `start_swing` pushes attacking slots,
    /// the next slice's strip reads `pairs`. Never hashed.
    trails: TrailRing,
    tick: u32,
    /// Ticks one swing clip plays (from the hero's first swing clip).
    swing_ticks: u32,
    bundle_shas: Vec<String>,
    /// Every body the last presented frame drew, as numbers.
    truth: Vec<BodyTruth>,
    /// View-only camera trauma in permille `0..=1000` (LF11): in-reach Hits
    /// add 400, in-reach Knockdowns add 700, clamped at 1000, decayed 1000
    /// per second. Presentation only; the sim hash never reads it.
    shake_trauma_permille: u32,
    /// Last uploaded vertex digest per `slot * MAX_PARTS + part`, `None`
    /// before that part's first upload. The upload pair is skipped when a
    /// frame's lifted positions plus skinned normals hash to the resident
    /// entry, so a frozen body re-uploads nothing while a moved one
    /// re-uploads whole (48 B stride, never a 32 B refresh). Presentation
    /// only; never hashed and never drawn.
    uploaded_digest: Vec<Option<u64>>,
    /// One inertial offset set per look slot (LF9), sized to that slot's rig
    /// and built once at load: a clip switch writes it, every frame reads it.
    /// Empty for a look with no rigs. Presentation only; never hashed.
    inertia: Vec<Inertial>,
    /// One last-stage rotation limiter per look slot, sized to that slot's rig
    /// and built once at load: `observe_tick` advances it on the tick's final
    /// pose, `slot_sample` reads this tick's start and end rotations on every
    /// frame, so no bone turns faster than 1080 deg/s (18 deg a 60 Hz tick).
    /// Empty for a look with no rigs. Presentation only; never hashed.
    limit: Vec<LimitLane>,
    /// The ragdoll layer (RZ2w): one float world, a body per floored slot. Presentation only;
    /// the sim never reads it and nothing in it is hashed.
    ragdoll: RagdollLook,
    /// The limb layer (RZ3): one float world, a body per authority cut, plus the
    /// stump caps and the blood the gore level allows. Presentation only; the sim
    /// never reads it and nothing in it is hashed.
    limbs: LimbLook,
    /// The cape layer (RZ7): one CPU Verlet sheet per seen slot, pinned at the
    /// shoulders. Presentation only; the sim never reads it and nothing in it is
    /// hashed.
    capes: Capes,
    /// The slot the fight camera rides behind: the window's human (0, or 1 for
    /// a network guest). View only.
    hero: usize,
    /// Rewind ring (EG39): the last 16 folded ticks, so a resim starts from
    /// the state before its window and replays the cues once instead of
    /// counting them twice. Presentation only.
    checkpoints: Vec<LookCheckpoint>,
    /// Next ring slot to overwrite when no entry already names the tick.
    rewind_cursor: usize,
    /// Fading visual correction from the last rewind; `None` at rest.
    correction: Option<Correction>,
}

/// Neutral name for the presentation look, so crates below the viewer (the
/// web shell) import the type without a genre word (GE2 ledger; see net's
/// `BodySnap` for the same pattern).
pub type BodyLook = FighterLook;

/// One upload-digest slot per `slot * MAX_PARTS + part`, all empty: the
/// [`FighterLook::uploaded_digest`] shape, one owner for the constructor and
/// the test literals that spell the struct out.
fn empty_upload_digest() -> Vec<Option<u64>> {
    vec![None; MAX_LOOK_FIGHTERS * MAX_PARTS]
}

/// Variant index for a slot among `rigs` variants: the hero on variant 0,
/// opponents cycle through the rest. The one owner of the slot-to-rig rule.
fn rig_index(rigs: usize, slot: usize) -> usize {
    if slot == 0 || rigs < 2 {
        return 0;
    }
    1 + (slot - 1) % (rigs - 1)
}

/// Fighters content dir (`content/fighters`), resolved like the kit dir:
/// walk up from the executable to the tree holding the manifest, else the
/// launch directory.
#[must_use]
pub fn fighters_content_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let mut anchor = exe.parent().map(Path::to_path_buf);
        while let Some(dir) = anchor {
            let candidate = dir.join("content/fighters");
            if candidate.join(fighter_bind::MANIFEST_NAME).is_file() {
                return candidate;
            }
            anchor = dir.parent().map(Path::to_path_buf);
        }
    }
    PathBuf::from("content/fighters")
}

fn sha_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn digest32(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Same-name bone pairs from the target rig onto the clip rig.
fn same_name_map(source: &Skeleton, target: &Skeleton) -> Result<BoneMap, LookError> {
    let pairs: Vec<(&str, &str)> = target
        .joints
        .iter()
        .filter(|joint| source.joints.iter().any(|other| other.name == joint.name))
        .map(|joint| (joint.name.as_str(), joint.name.as_str()))
        .collect();
    BoneMap::build(source, target, &pairs).map_err(|err| LookError::Import(format!("{err:?}")))
}

/// Farthest a bind vertex sits from its nearest bind joint, in meters.
/// Pure `f32` in, same `f32` out; a non-finite vertex reads as infinity so
/// the caller skips it, never a panic.
fn nearest_joint_d(joints: &[[f32; 3]], vertex: [f32; 3]) -> f32 {
    let mut nearest = f32::MAX;
    for joint in joints.iter() {
        let dx = vertex[0] - joint[0];
        let dy = vertex[1] - joint[1];
        let dz = vertex[2] - joint[2];
        nearest = nearest.min(dx * dx + dy * dy + dz * dz);
    }
    nearest.sqrt()
}

/// Skin radius of one rig in meters: the farthest any bind vertex sits from
/// its nearest bind joint, plus 10 cm, floored at 0.5 m. The joints-only
/// pose pads placed joints by this radius so its bounds hold the skinned
/// vertex bounds. Cold path only (once per rig at load); pure `f32` in,
/// same `f32` out; a non-finite vertex never moves the radius.
fn skin_radius_m(body: &SkinnedBody) -> f32 {
    let mut radius = 0.5f32;
    for part in body.parts.iter() {
        for vertex in part.mesh.positions.iter() {
            let d = nearest_joint_d(&body.bind.positions, *vertex);
            if d.is_finite() {
                radius = radius.max(d + 0.10);
            }
        }
    }
    radius
}

/// Posed bound of one skinned body in world meters: the axis-box centre over
/// every posed vertex plus the farthest vertex gap from it, or `None` when no
/// finite vertex folds in. This is the bound a draw carries, so culling
/// follows the pose and a limb the bind pose never had stays inside it
/// (Eberly 3.2); the vertices ride the body's sand rest like the drawn
/// mesh does. Meters in, meters out, no heap, and the same frames give the
/// same bound bytes twice.
fn posed_bound_m(body: &PosedBody) -> Option<([f32; 3], f32)> {
    let lift = if body.lift.is_finite() {
        body.lift
    } else {
        0.0
    };
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut folded = false;
    for frame in body.frames.iter() {
        for vertex in frame.positions.iter() {
            if !vertex.iter().all(|v| v.is_finite()) {
                continue;
            }
            folded = true;
            let at = [vertex[0], vertex[1] + lift, vertex[2]];
            for (axis, value) in at.iter().enumerate() {
                lo[axis] = lo[axis].min(*value);
                hi[axis] = hi[axis].max(*value);
            }
        }
    }
    if !folded {
        return None;
    }
    let centre: [f32; 3] = std::array::from_fn(|axis| (lo[axis] + hi[axis]) * 0.5);
    let mut radius2 = 0.0f32;
    for frame in body.frames.iter() {
        for vertex in frame.positions.iter() {
            if !vertex.iter().all(|v| v.is_finite()) {
                continue;
            }
            let (dx, dy, dz) = (
                vertex[0] - centre[0],
                vertex[1] + lift - centre[1],
                vertex[2] - centre[2],
            );
            radius2 = radius2.max(dx * dx + dy * dy + dz * dz);
        }
    }
    Some((centre, radius2.sqrt()))
}

/// Retarget every clip source onto the body skeleton, in file order.
/// Cold path only; a missing clip or a failed map fails loud.
fn retarget_clips(
    sources: &[ClipSource],
    body: &SkinnedBody,
) -> Result<Vec<ImportedClip>, LookError> {
    let mut clips = Vec::with_capacity(sources.len());
    for source in sources {
        let map = same_name_map(&source.skeleton, &body.skeleton)?;
        let clip = source
            .clips
            .first()
            .ok_or_else(|| LookError::Import("clip file holds no clip".to_string()))?;
        let moved = retarget_clip(
            clip,
            &source.skeleton,
            &source.bind,
            &body.skeleton,
            &body.bind,
            &map,
        )
        .map_err(|err| LookError::Import(format!("retarget {}: {err:?}", clip.name)))?;
        clips.push(moved);
    }
    Ok(clips)
}

fn load_rig(
    binding: &FighterBinding,
    rig_index: usize,
    sources: &[ClipSource],
    source: &dyn AssetSource,
) -> Result<Rig, LookError> {
    let bytes = source
        .read(&binding.body_glb)
        .map_err(|err| LookError::Bind(format!("{}: {err}", binding.variant)))?;
    if sha_hex(&bytes) != binding.body_glb_sha256 {
        return Err(LookError::ShaMismatch(binding.variant.clone()));
    }
    let body = import_skinned_body(&bytes)
        .map_err(|err| LookError::Import(format!("{}: {err}", binding.variant)))?;
    if body.parts.len() > MAX_PARTS {
        return Err(LookError::Import(format!(
            "{}: {} parts over {MAX_PARTS}",
            binding.variant,
            body.parts.len()
        )));
    }
    let mut pipe = SkinPipeline::new();
    pipe.upload_skeleton(body.skeleton.len())
        .map_err(|err| LookError::Skin(format!("{err:?}")))?;
    let mut parts = Vec::with_capacity(body.parts.len());
    for (index, part) in body.parts.iter().enumerate() {
        let mesh = v1::upload_mesh(
            &part.mesh.positions,
            &part.mesh.normals,
            &part.mesh.uv0,
            &part.mesh.indices,
        )
        .map_err(|err| LookError::Skin(format!("{} part {index}: {err}", binding.variant)))?;
        let skin = pipe
            .upload_weights(&part.skin.joints, &part.skin.weights)
            .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        let texture = match &part.base_color_image {
            Some(image) => {
                let upload = TextureUpload::from_encoded(image)
                    .map_err(|err| LookError::Import(format!("{err}")))?;
                let id = LOOK_TEXTURE_ID_BASE
                    + u64::try_from(rig_index * MAX_PARTS + index)
                        .map_err(|_| LookError::Import("texture id".to_string()))?;
                Some((TextureKey::new(id, digest32(image)), upload))
            }
            None => None,
        };
        let factor = part.material.base_color_factor;
        let material = PbrMaterial {
            base_color: [factor[0], factor[1], factor[2]],
            metallic: part.material.metallic_factor,
            roughness: part.material.roughness_factor,
            exposure: 1.0,
        };
        material
            .validate()
            .map_err(|err| LookError::Import(format!("material: {err}")))?;
        parts.push(RigPart {
            mesh,
            skin,
            texture,
            material,
        });
    }
    let pelvis = body
        .skeleton
        .joints
        .iter()
        .position(|joint| joint.name == "pelvis")
        .ok_or_else(|| LookError::Import(format!("{}: no pelvis joint", binding.variant)))?;
    let torso = body
        .skeleton
        .joints
        .iter()
        .position(|joint| joint.name == "torso")
        .ok_or_else(|| LookError::Import(format!("{}: no torso joint", binding.variant)))?;
    let head = body
        .skeleton
        .joints
        .iter()
        .position(|joint| joint.name == "head")
        .ok_or_else(|| LookError::Import(format!("{}: no head joint", binding.variant)))?;
    let clips = retarget_clips(sources, &body)?;
    Ok(Rig {
        radius_m: skin_radius_m(&body),
        legs: Legs::find(&body.skeleton, &body.bind.positions),
        throw_arm: throw_arm(&body.skeleton),
        variant: binding.variant.clone(),
        bind_pelvis: body.bind.positions[pelvis],
        skeleton: body.skeleton,
        inverse_binds: body.inverse_binds,
        pelvis,
        torso,
        head,
        pipe,
        parts,
        clips,
    })
}

/// One line per staged rig: the variant, its joints, its parts, its clips and
/// the digest of the bytes it was built from. Two looks that staged the same
/// rig read the same line, so a browser rig and a native rig can be compared
/// without a window.
fn rig_summary(rig: &Rig) -> String {
    let mut digest = engine_core::Fnv64::new();
    digest.write(rig.variant.as_bytes());
    for joint in &rig.skeleton.joints {
        digest.write(joint.name.as_bytes());
        match joint.parent {
            Some(parent) => digest.write_u64(u64::try_from(parent).unwrap_or(u64::MAX)),
            None => digest.write_u64(u64::MAX),
        }
    }
    for bind in &rig.inverse_binds {
        for row in bind {
            for value in row {
                digest.write_f32(*value);
            }
        }
    }
    for part in &rig.parts {
        digest.write_u64(u64::try_from(part.mesh.positions.len()).unwrap_or(u64::MAX));
        for value in &part.mesh.positions {
            digest.write_f32(value[0]);
            digest.write_f32(value[1]);
            digest.write_f32(value[2]);
        }
        for value in &part.mesh.normals {
            digest.write_f32(value[0]);
            digest.write_f32(value[1]);
            digest.write_f32(value[2]);
        }
        for value in &part.mesh.uvs {
            digest.write_f32(value[0]);
            digest.write_f32(value[1]);
        }
        for value in &part.mesh.indices {
            digest.write_u32(*value);
        }
        for row in &part.skin.joints {
            for joint in row {
                digest.write_u16(*joint);
            }
        }
        for row in &part.skin.weights {
            for weight in row {
                digest.write_f32(*weight);
            }
        }
        for value in part.material.base_color {
            digest.write_f32(value);
        }
        digest.write_f32(part.material.metallic);
        digest.write_f32(part.material.roughness);
    }
    for clip in &rig.clips {
        digest.write(clip.name.as_bytes());
        digest.write_f32(clip.duration);
    }
    format!(
        "{} joints={} parts={} clips={} digest={:016x}",
        rig.variant,
        rig.skeleton.joints.len(),
        rig.parts.len(),
        rig.clips.len(),
        digest.finish()
    )
}

/// Multiply two AN1 row-vector matrices: `p * a * b`.
fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (k, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][k] + a[i][1] * b[1][k] + a[i][2] * b[2][k] + a[i][3] * b[3][k];
        }
    }
    out
}

/// A slot's root at sub-tick `alpha`: the position lerped from the last tick
/// to this one, with the balance lean (EG45) stepped along the facing, and the
/// facing lerped the short way. Pure; presentation only.
fn slot_root(slot: &SlotAnim, alpha: f32) -> ([f32; 3], f32) {
    let mut pos: [f32; 3] =
        std::array::from_fn(|i| slot.prev_pos[i] + (slot.pos[i] - slot.prev_pos[i]) * alpha);
    let yaw = slot.prev_yaw + angle_delta(slot.prev_yaw, slot.yaw) * alpha;
    // Balance lean (EG45): a stumbling root steps forward along its
    // facing; every other gait holds. Presentation only.
    let lean = balance_lean_m(slot.balance);
    if lean != 0.0 && yaw.is_finite() {
        let (s, c) = yaw.sin_cos();
        if s.is_finite() && c.is_finite() {
            pos[0] += lean * s;
            pos[2] += lean * c;
        }
    }
    (pos, yaw)
}

/// Root placement in the AN1 row-vector layout: turn about +Y by `yaw`
/// (body forward +Z maps to `(sin yaw, 0, cos yaw)`), then move to `pos`.
fn root_matrix(pos: [f32; 3], yaw: f32) -> [[f32; 4]; 4] {
    let (s, c) = yaw.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [pos[0], pos[1], pos[2], 1.0],
    ]
}

/// Shortest signed angle from `from` to `to`, in `(-PI, PI]`.
fn angle_delta(from: f32, to: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut d = (to - from) % tau;
    if d > std::f32::consts::PI {
        d -= tau;
    } else if d <= -std::f32::consts::PI {
        d += tau;
    }
    d
}

/// Mesh key for one slot's body part.
#[must_use]
pub fn look_mesh_key(slot: usize, part: usize) -> MeshKey {
    let index = slot * MAX_PARTS + part;
    let id = LOOK_MESH_ID_BASE + u64::try_from(index).unwrap_or(u64::MAX - LOOK_MESH_ID_BASE);
    let mut digest = [0x5Bu8; 32];
    digest[0] = u8::try_from(slot).unwrap_or(u8::MAX);
    digest[1] = u8::try_from(part).unwrap_or(u8::MAX);
    MeshKey::new(id, digest)
}

/// The hit-flash material for a body part: red-shifted albedo (the shader
/// multiplies it into the base-color map) and a brighter exposure. Pure; a
/// finite valid input stays finite and valid.
#[must_use]
fn flash_material(base: &PbrMaterial) -> PbrMaterial {
    let mut flash = base.clone();
    flash.base_color = [
        base.base_color[0],
        base.base_color[1] * FLASH_GREEN_KEEP,
        base.base_color[2] * FLASH_BLUE_KEEP,
    ];
    flash.exposure = FLASH_EXPOSURE;
    flash
}

/// GPU skin availability probe behind the window path.
///
/// Consumer: [`FighterLook::load_and_stage`] (cold window entry) calls this
/// so the built `GpuSkinned` path stays constructed in the live frame. Data
/// owner: `engine-render` owns the GPU device and pipeline; this file owns
/// the call only. Units: none. Determinism: presentation only; the answer
/// never enters hashed state and the CPU skin vectors below stay the single
/// render path. Errors: never fails; a missing adapter or a device refusal
/// reads as `None` (headless skips, never fails). Budget: one adapter
/// request per staged match, zero per tick or frame.
#[cfg(not(target_arch = "wasm32"))]
fn gpu_skin_adapter() -> Option<String> {
    match engine_render::v1_skin::GpuSkinned::init() {
        Ok(gpu) => Some(gpu.adapter_name().to_string()),
        Err(_) => None,
    }
}

/// The browser keeps no HDRI in the wasm and has no synchronous adapter
/// request, so the probe reports none (QW4).
#[cfg(target_arch = "wasm32")]
fn gpu_skin_adapter() -> Option<String> {
    None
}

/// One CPU-posed body with its skinned parts, before any GPU upload.
///
/// Consumer: [`FighterLook::stage_bodies`] uploads `frames` with `lift`;
/// the headless pose step returns these with no `Presenter`. Units: meters
/// for bounds, seconds for clip time. Determinism: pure fold of the tick
/// plus `alpha`; the same inputs pose the same truth twice. Errors: none
/// here; the posing calls fail loud on a missing rig instead of panicking.
struct PosedBody {
    truth: BodyTruth,
    frames: Vec<engine_render::v1_skin::SkinnedFrame>,
    /// Joint matrices (`inverse bind * joint * root`, AN1 rows) with the down
    /// lift already in their translation row: the GPU skin path writes them
    /// as the body's one joint block, the frames above only feed the bounds.
    matrices: Vec<[[f32; 4]; 4]>,
    /// Sand rest in meters (LF17): positive lifts a sunk dead body, negative
    /// lowers a hovering one, zero for every standing body.
    lift: f32,
    /// Bound this draw carries in world meters: the posed-vertex bound
    /// ([`posed_bound_m`]) floored at the rig's bind radius, so a cull reads
    /// the pose and never the bind-pose sphere. `None` when no posed vertex
    /// is finite. Consumer: [`FighterLook::stage_bodies`], the frame's draw
    /// path, replacing the bind sphere a render draw cull would read.
    bound: Option<([f32; 3], f32)>,
}

#[cfg(test)]
impl PosedBody {
    fn truth(&self) -> &BodyTruth {
        &self.truth
    }
}

/// Sampled pose shared by the full and joints-only paths: clip pick, world
/// joint matrices and the interpolated root. Both modes fold from here, so
/// their slots, joints, clip and clip time can never drift apart.
struct SlotSample {
    clip: usize,
    time: f32,
    world: Vec<[[f32; 4]; 4]>,
    pos: [f32; 3],
    yaw: f32,
    root: [[f32; 4]; 4],
    /// How far each fully planted foot is drawn from its held spot, in
    /// meters (LF12); `None` for a foot that is not at full plant.
    slides: [Option<f32>; 2],
    /// Head travel from the rotation flinch, meters (AM23); 0 when silent.
    react_head_m: f32,
}

/// One lane per look slot, sized to that slot's rig, like the inertial sets.
/// Cold path; a rig past the limiter's shape fails the load.
fn limit_lanes(rigs: &[Rig]) -> Result<Vec<LimitLane>, LookError> {
    (0..MAX_LOOK_FIGHTERS)
        .map(|slot| {
            let rig = rigs
                .get(rig_index(rigs.len(), slot))
                .ok_or_else(|| LookError::Skin("limit lanes: no rig".to_string()))?;
            Ok(LimitLane::new(&rig.skeleton))
        })
        .collect()
}

impl FighterLook {
    /// Ride the fight camera behind `slot` (the window's human) instead of
    /// slot 0: the shoulder shot, the hero close-up and the duel framing all
    /// follow it. A slot past the look's range keeps the last one. View only.
    pub fn set_hero(&mut self, slot: u8) {
        if usize::from(slot) < MAX_LOOK_FIGHTERS {
            self.hero = usize::from(slot);
        }
    }

    /// Pinned bundle SHAs of the bound variants, manifest order.
    #[must_use]
    pub fn bundle_shas(&self) -> &[String] {
        &self.bundle_shas
    }

    /// How many rigs the look staged (one per bound variant).
    #[must_use]
    pub fn rig_count(&self) -> usize {
        self.rigs.len()
    }

    /// One line per staged rig: the variant, its joints, its parts and its
    /// clips, plus the digest of the bytes it was built from. Two looks that
    /// staged the same rig read the same line, so a browser rig and a native
    /// rig can be compared without a window.
    #[must_use]
    pub fn rig_summaries(&self) -> Vec<String> {
        self.rigs.iter().map(rig_summary).collect()
    }

    /// Variant drawn for a slot: the hero on slot 0, opponents cycle after.
    #[must_use]
    pub fn rig_for_slot(&self, slot: usize) -> usize {
        rig_index(self.rigs.len(), slot)
    }

    /// Variant name drawn for a slot.
    #[must_use]
    pub fn variant_for_slot(&self, slot: usize) -> &str {
        self.rigs
            .get(self.rig_for_slot(slot))
            .map_or("", |rig| rig.variant.as_str())
    }

    /// Upload every base-color map. Cold path; a refused upload is loud.
    pub fn stage_textures(&self, presenter: &mut Presenter) -> Result<usize, LookError> {
        let mut staged = 0usize;
        for rig in &self.rigs {
            for part in &rig.parts {
                if let Some((key, upload)) = &part.texture {
                    presenter
                        .upload_texture(*key, upload)
                        .map_err(|err| LookError::Upload(err.to_string()))?;
                    staged += 1;
                }
            }
        }
        Ok(staged)
    }

    /// Apply one authoritative balance verdict to `slot` (EG45): the gait
    /// read from `Sim::balance` (`engine-arena`) via `LiveMatch::balance`
    /// and polled by [`observe_live_balances`](Self::observe_live_balances).
    /// A bad slot is ignored; a current `Fallen` never downgrades (keeps
    /// flat); every other gait applies (`Steady` resets, `Stumbling` leans
    /// via `balance_lean_m`, `Catching` kneels via `balance_pelvis_y_m`,
    /// `Fallen` floors via `balance_pelvis_y_m` plus the flat `Down` pose
    /// in [`slot_pose`](Self::slot_pose)). Presentation only; the sim
    /// digest never reads this.
    pub fn observe_balance(&mut self, slot: u8, gait: GaitPhase) {
        let Some(anim) = self.slots.get_mut(usize::from(slot)) else {
            return;
        };
        if anim.balance == GaitPhase::Fallen && gait != GaitPhase::Fallen {
            return;
        }
        anim.balance = gait;
    }

    /// Poll the authoritative integer balance gaits into the pose (EG45):
    /// slots `0..fighters.min(MAX_LOOK_FIGHTERS)` read
    /// `LiveMatch::balance` and feed [`observe_balance`](Self::observe_balance)
    /// with `state.gait()`. A `None` (bad slot) is ignored; the Fallen-sticky
    /// rule stays inside `observe_balance` (a current `Fallen` never
    /// downgrades). Presentation only; the sim digest never reads this.
    ///
    /// Consumer: the window (the next slice owns the per-tick call). Data
    /// owner: `engine-arena` owns the per-slot states. Units: slot index.
    /// Determinism: same live plus same fighters gives the same pose twice.
    pub fn observe_live_balances(&mut self, live: &engine_arena::LiveMatch, fighters: usize) {
        for index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let Ok(slot) = u8::try_from(index) else {
                continue;
            };
            if let Some(state) = live.balance(slot) {
                // RZ2w: a fall queues a ragdoll (with the standing slot, so before the floor
                // verdict is mirrored); a recovery lets it stand up.
                let shown = self.ragdoll_fold_gait(slot, state.gait());
                self.observe_balance(slot, shown);
            }
        }
        // Once per accepted tick (the window calls this right after `observe_tick`): spawn
        // what the tick floored, step the float world one tick, stand up what recovered.
        self.ragdoll_tick();
    }

    /// Fold view-only shake trauma (LF11): saturating add, clamped at
    /// [`SHAKE_MAX_PERMILLE`]. Presentation only; the sim hash never reads it.
    fn add_shake_trauma(&mut self, amount_permille: u32) {
        self.shake_trauma_permille = self
            .shake_trauma_permille
            .saturating_add(amount_permille)
            .min(SHAKE_MAX_PERMILLE);
    }

    /// Drawn slots under `fighters` that wear the hit tint now, and all
    /// drawn slots: the arena receipt's share of red bodies.
    #[must_use]
    pub fn tinted(&self, fighters: usize) -> (usize, usize) {
        let drawn = self
            .slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|s| s.seen);
        drawn.fold((0, 0), |(red, all), slot| {
            (red + usize::from(self.tick < slot.flash_until), all + 1)
        })
    }

    /// Both slots drawn and their feet within [`TINT_REACH_M`] on the sand.
    fn in_reach(&self, attacker: u8, victim: u8) -> bool {
        let (Some(a), Some(v)) = (
            self.slots.get(usize::from(attacker)),
            self.slots.get(usize::from(victim)),
        ) else {
            return false;
        };
        a.seen && v.seen && (a.pos[0] - v.pos[0]).hypot(a.pos[2] - v.pos[2]) <= TINT_REACH_M
    }

    /// Blade base-plus-tip for `pos` plus `yaw`, view only: the base rides
    /// 1 m above the feet, the tip leads 0.9 m along the facing plus 0.1 m
    /// up. Pure; non-finite input reads as non-finite output, which the
    /// ring drops instead of storing.
    fn blade_points(pos: [f32; 3], yaw: f32) -> ([f32; 3], [f32; 3]) {
        let base = [pos[0], pos[1] + 1.0, pos[2]];
        let (s, c) = yaw.sin_cos();
        let tip = [base[0] + s * 0.9, base[1] + 0.1, base[2] + c * 0.9];
        (base, tip)
    }

    fn start_swing(&mut self, attacker: u8, tick: u32) {
        let index = usize::from(attacker);
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        if slot.act == Act::Down {
            return;
        }
        if let Some(slot) = self.slots.get_mut(index) {
            // A swing already in flight keeps playing; a new one restarts.
            let variant = slot.swings % 3;
            slot.swings = slot.swings.wrapping_add(1);
            slot.act = Act::Attack(variant);
            slot.act_start = tick.saturating_sub(IMPACT_TICKS);
        }
        // Trail samples land in `feed_trails` once per tick, so one swing
        // start never pushes twice. Never hashed.
    }

    /// Feed the blade trail ring once per tick, view only: age once, then
    /// push current blade points for each seen attacking slot (cap 12,
    /// oldest drops) and clear every other slot. Non-finite stores nothing.
    /// Never hashed. Caller: `observe_tick` after the act fold.
    fn feed_trails(&mut self) {
        self.trails.age_tick();
        for index in 0..MAX_LOOK_FIGHTERS {
            let (seen, attacking, pos, yaw) = match self.slots.get(index) {
                Some(slot) => (
                    slot.seen,
                    matches!(slot.act, Act::Attack(_)),
                    slot.pos,
                    slot.yaw,
                ),
                None => continue,
            };
            if seen && attacking {
                let (base_m, tip_m) = Self::blade_points(pos, yaw);
                self.trails.push(index, base_m, tip_m);
            } else {
                self.trails.clear_idle(index);
            }
        }
    }

    /// Trail pairs for `slot`, oldest first; empty when idle. View only,
    /// never hashed. Caller: [`Self::trail_strip`].
    #[must_use]
    pub fn trail_pairs(&self, slot: usize) -> &[TrailPoint] {
        self.trails.pairs(slot)
    }

    /// LF14: fold every busy slot's ring into the frame's sword ribbon: one
    /// triangle-strip vertex list (`base`, `tip` per pair, oldest first, alpha
    /// by age) plus the vert count of each slot's strip, so the draw never
    /// bridges two fighters. An idle slot adds nothing, so a calm frame
    /// returns two empty lists and stages no draw. View only, never hashed;
    /// the only allocation is the returned pair of lists.
    #[must_use]
    pub fn trail_strip(&self) -> (Vec<TrailStripVert>, Vec<u32>) {
        let mut verts = Vec::new();
        let mut lens = Vec::new();
        for slot in 0..MAX_LOOK_FIGHTERS {
            let pairs = self.trails.pairs(slot);
            if pairs.is_empty() {
                continue;
            }
            let mut base = [[0.0f32; 3]; TRAIL_LEN];
            let mut tip = [[0.0f32; 3]; TRAIL_LEN];
            let mut age = [0u8; TRAIL_LEN];
            let count = pairs.len().min(TRAIL_LEN);
            for (index, point) in pairs.iter().take(count).enumerate() {
                base[index] = point.base_m;
                tip[index] = point.tip_m;
                age[index] = point.age_ticks;
            }
            let strip = trail_strip_verts(&base[..count], &tip[..count], &age[..count]);
            if strip.is_empty() {
                continue;
            }
            lens.push(u32::try_from(strip.len()).unwrap_or(0));
            verts.extend(strip);
        }
        (verts, lens)
    }

    /// Current act for a slot (receipts and tests): the bridge pick mapped
    /// through `bridge_act` for seen slots, `None` otherwise. The
    /// `SlotAnim.act` mirror stays as the sampling cache for `slot_pose`.
    #[must_use]
    pub fn act(&self, slot: usize) -> Option<Act> {
        let seen = self.slots.get(slot).is_some_and(|s| s.seen);
        if !seen {
            return None;
        }
        self.bridge.get(slot).map(|b| bridge_act(b.pick().clip))
    }

    /// Clip index plus clip time for a slot's act at the frame's sub-tick
    /// `alpha`; a finished swing falls back to idle or run (resolved here, so
    /// the frame never shows a frozen last swing frame). The run clip plays
    /// the phase the slot carried tick by tick through [`run_phase_scale`]
    /// ([`SlotAnim::run_clip_s`]), so a speed change never jumps it; the other
    /// acts read elapsed time and ignore speed.
    fn clip_time(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> (usize, f32) {
        let since =
            f32::from(u16::try_from(self.tick.saturating_sub(slot.act_start)).unwrap_or(u16::MAX));
        let t = (since + alpha) / TICK_HZ;
        let duration = |index: usize| rig.clips.get(index).map_or(0.0, |clip| clip.duration);
        match slot.act {
            Act::Idle => (0, t),
            Act::Run => (1, slot.run_clip_s(alpha)),
            Act::Attack(variant) => {
                let index = 2 + usize::from(variant.min(2));
                if t < duration(index) {
                    (index, t)
                } else {
                    (0, t - duration(index))
                }
            }
            // Hold the last death frame (sampling wraps past the end).
            Act::Down => (5, t.min(duration(5) * 0.999)),
        }
    }

    /// Clip index plus clip time a slot shows at sub-tick `alpha`: the act's
    /// clip through [`clip_time`](Self::clip_time), or the flat `Down` last
    /// frame once the sim has floored the slot (EG45). Pure.
    fn clip_pick(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> (usize, f32) {
        if slot.balance == GaitPhase::Fallen {
            let duration = rig.clips.get(5).map_or(0.0, |clip| clip.duration);
            (5, duration * 0.999)
        } else {
            let at = clip_alpha(slot, alpha);
            self.clip_time(rig, slot, at)
        }
    }

    /// Clip pose for one slot at sub-tick `alpha`: the current act's clip
    /// sampled at the speed-scaled run time, with no switch offset (those
    /// ride on top in [`inertial_pose`](Self::inertial_pose)). Pure function
    /// of the tick plus the recorded speed; the pelvis stays pinned in place (the
    /// match owns travel, the clip keeps its bob). The balance gait then
    /// lowers the pelvis for a fall or a catch (EG45, presentation only):
    /// a `Fallen` gait forces the flat `Down` last frame at once, so the
    /// floored slot lies flat within one frame; `Catching` kneels and
    /// `Stumbling` leans while keeping the act's clip; `Steady` keeps the
    /// pose byte-identical.
    /// A `Down` pose starts here at [`BALANCE_FALLEN_PELVIS_M`]; the LF17
    /// sand rest ([`sand_rest_shift_m`]) only shifts the body after the CPU
    /// skin folds the posed bounds, never inside this pose. Last, the additive
    /// flinch (EG18/AM23) pushes the pelvis away from the recorded attacker
    /// and leans the torso plus head over about 450 ms: blend in, hold, fade.
    fn slot_pose(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> Result<LocalPose, LookError> {
        let mut pose = self.slot_pose_base(rig, slot, alpha)?;
        self.apply_hit_flinch(rig, &mut pose, slot, alpha)?;
        Ok(pose)
    }

    /// [`slot_pose`](Self::slot_pose) before the flinch: the clip sample,
    /// the in-place pelvis and the balance gait. The flinch reads this as its
    /// base; the rotation metric measures only the head travel the lean adds.
    fn slot_pose_base(
        &self,
        rig: &Rig,
        slot: &SlotAnim,
        alpha: f32,
    ) -> Result<LocalPose, LookError> {
        let (clip_index, time) = self.clip_pick(rig, slot, alpha);
        let clip = rig
            .clips
            .get(clip_index)
            .ok_or_else(|| LookError::Skin("clip missing".to_string()))?;
        let mut pose: LocalPose =
            sample_pose_with_rots(&rig.skeleton, &clip.positions, &clip.rotations, time)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        // In-place clips: the match owns travel, the clip keeps its bob.
        pose.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        pose.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        pose.positions[rig.pelvis][1] =
            balance_pelvis_y_m(slot.balance, pose.positions[rig.pelvis][1]);
        Ok(pose)
    }

    /// Presentation flinch weight for this slot at `alpha`; no hit reads zero.
    /// A held slot samples the end of its tick, like its clip, so the impact
    /// pose stays one pose at every sub-tick frame of the freeze (LF10).
    fn hit_weight(&self, slot: &SlotAnim, alpha: f32) -> f32 {
        if slot.hit_start == u32::MAX {
            return 0.0;
        }
        view_envelope(
            self.tick.wrapping_sub(slot.hit_start),
            clip_alpha(slot, alpha),
        )
    }

    /// Add the hit flinch to `pose`: pelvis push plus dip, then the
    /// real-skeleton upper-chain lean. Returns the head travel from the
    /// rotation half in meters (AM23); 0 when the flinch is silent. View only.
    fn apply_hit_flinch(
        &self,
        rig: &Rig,
        pose: &mut LocalPose,
        slot: &SlotAnim,
        alpha: f32,
    ) -> Result<f32, LookError> {
        let weight = self.hit_weight(slot, alpha);
        if weight <= 0.0 || (slot.hit_push[0] == 0.0 && slot.hit_push[1] == 0.0) {
            return Ok(0.0);
        }
        let planar = 0.5 * PEAK_M * weight;
        let dip = 0.5 * DIP_M * weight;
        pose.positions[rig.pelvis][0] += slot.hit_push[0] * planar;
        pose.positions[rig.pelvis][2] += slot.hit_push[1] * planar;
        pose.positions[rig.pelvis][1] -= dip;
        let head_m = lean_upper(
            &rig.skeleton,
            pose,
            rig.torso,
            rig.head,
            slot.hit_push,
            weight,
        )
        .map_err(|err| LookError::Skin(err.to_string()))?;
        Ok(head_m)
    }

    /// Head travel from the rotation flinch at this sample, meters (AM23).
    /// Re-samples the base only while a flinch is live, so the metric reads
    /// the same lean [`slot_pose`](Self::slot_pose) applies. A missing pose
    /// reads 0 instead of failing the frame.
    fn react_head_m(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> f32 {
        if self.hit_weight(slot, alpha) <= 0.0 {
            return 0.0;
        }
        let Ok(mut pose) = self.slot_pose_base(rig, slot, alpha) else {
            return 0.0;
        };
        self.apply_hit_flinch(rig, &mut pose, slot, alpha)
            .unwrap_or(0.0)
    }

    /// Create every slot's body-part vertex buffers once, from the bind
    /// pose, so no fighter ever creates a GPU buffer inside a frame: a
    /// slot that appears late already has its buffers and just rewrites
    /// them. Returns parts staged. Cold path; loud on refusal.
    pub fn stage_meshes(&self, presenter: &mut Presenter) -> Result<usize, LookError> {
        let mut staged = 0usize;
        for slot_index in 0..MAX_LOOK_FIGHTERS {
            let rig = &self.rigs[self.rig_for_slot(slot_index)];
            for (part_index, part) in rig.parts.iter().enumerate() {
                presenter
                    .upload_mesh(look_mesh_key(slot_index, part_index), &part.mesh)
                    .map_err(|err| LookError::Upload(err.to_string()))?;
                staged += 1;
            }
        }
        Ok(staged)
    }

    /// The GPU-skin twin of [`Self::stage_meshes`]: create every slot's
    /// part buffers once as bind-pose skinned vertices (pos, normal, uv,
    /// joints, weights), each slot on its own joint block. A staged frame
    /// then writes one joint block per body and uploads no vertices, and the
    /// window forward pass blends the skin in the vertex stage. Returns
    /// parts staged. Cold path; loud on refusal.
    pub fn stage_skinned_meshes(&mut self, presenter: &mut Presenter) -> Result<usize, LookError> {
        let mut staged = 0usize;
        for slot_index in 0..MAX_LOOK_FIGHTERS {
            let rig = &self.rigs[self.rig_for_slot(slot_index)];
            for (part_index, part) in rig.parts.iter().enumerate() {
                presenter
                    .upload_skinned_mesh(
                        look_mesh_key(slot_index, part_index),
                        &part.mesh,
                        &part.skin,
                        slot_index,
                    )
                    .map_err(|err| LookError::Upload(err.to_string()))?;
                staged += 1;
            }
        }
        // The resident bytes are the bind pose again: no CPU digest holds.
        self.uploaded_digest.fill(None);
        Ok(staged)
    }

    /// Skin every seen fighter at sub-tick `alpha` and write its parts into
    /// their resident vertex buffers, skipping a part whose bytes are already
    /// resident. Returns parts left resident; the wall halves behind them are
    /// [`Self::present_into`]'s report ([`SkinSplit`]). A skinning or upload
    /// refusal is loud.
    pub fn stage_frame(
        &mut self,
        presenter: &mut Presenter,
        fighters: usize,
        alpha: f32,
    ) -> Result<usize, LookError> {
        self.stage_bodies(presenter, fighters, alpha)
            .map(|(split, _)| split.uploaded)
    }

    /// Sample one seen slot at sub-tick `alpha`: clip pick, world joint
    /// matrices and interpolated root. Pure fold of the tick plus `alpha`;
    /// the balance lean (EG45) rides along, presentation only. A `Fallen`
    /// gait forces the flat `Down` last frame (like [`slot_pose`](Self::slot_pose)),
    /// so the floored slot draws flat within one frame. The last-stage
    /// angular-speed cap ([`limit_pose`](Self::limit_pose)) guards the bone
    /// rotations and [`clamp_sample_positions`](Self::clamp_sample_positions)
    /// guards their presented positions, so both the full and the joints-only
    /// pose carry both.
    fn slot_sample(
        &self,
        rig: &Rig,
        index: usize,
        slot: &SlotAnim,
        alpha: f32,
    ) -> Result<SlotSample, LookError> {
        self.slot_sample_raw(rig, index, slot, alpha, true)
    }

    /// [`slot_sample`](Self::slot_sample) with the final-pose position clamp
    /// optional: `limit_tick` samples the raw pose first, then clamps it
    /// against the previous tick's sample. Presentation only.
    fn slot_sample_raw(
        &self,
        rig: &Rig,
        index: usize,
        slot: &SlotAnim,
        alpha: f32,
        clamp_positions: bool,
    ) -> Result<SlotSample, LookError> {
        let (clip, time) = self.clip_pick(rig, slot, alpha);
        let mut pose = self.inertial_pose(rig, index, slot, alpha)?;
        // Last-stage angular-speed cap: each bone turns no faster than
        // `DEFAULT_ANG_LIMIT_DEG_PER_SEC`, its state advanced once in
        // `observe_tick`. Before the feet plant and the throw layer, so the
        // IK still lands the ankles on their held spots and the throw arm
        // keeps its cue timeline; both have their own bounded per-tick motion.
        // A held slot reads the end of its tick, like its clip clock.
        let at = clip_alpha(slot, alpha);
        let ramp = matches!(slot.balance, GaitPhase::Steady | GaitPhase::Stumbling);
        self.limit_pose(index, &mut pose.rotations, at, ramp);
        let (mut pos, yaw) = slot_root(slot, alpha);
        // LF12: plant the feet last, on top of the inertial offsets, so a clip
        // switch stays continuous and the feet hold their spot on the sand.
        let contacts = self.plant_feet(rig, slot, &mut pose, yaw, alpha)?;
        // SG3: the spear throw is a masked override on the final pose, last of all.
        self.lay_throw(rig, index, slot, &mut pose, alpha)?;
        let mut world = skinning_matrices(&rig.skeleton, &pose)
            .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        // AM24: the final position clamp runs after every pose layer, so the
        // drawn joints, their bounds and the foot-slide truth all read one
        // pose that never jumps past the pop law's miss budget.
        if clamp_positions {
            self.clamp_sample_positions(index, &mut world, alpha);
        }
        // EG39: a rollback rewind adds its fading error vector to the drawn
        // root after the pop clamp, so the clamp never eats the correction and
        // the body eases from where the viewer last saw it to the confirmed
        // root. The sim never reads this root.
        let correction = self.correction_offset(index);
        for (axis, offset) in correction.iter().enumerate() {
            pos[axis] += *offset;
        }
        let slides = foot_slides(rig.legs, &world, &contacts, (pos, yaw));
        let root = root_matrix(pos, yaw);
        let react_head_m = self.react_head_m(rig, slot, alpha);
        Ok(SlotSample {
            clip,
            time,
            world,
            pos,
            yaw,
            root,
            slides,
            react_head_m,
        })
    }

    /// CPU pose step of [`Self::stage_frame`]: clip sample, skin and truth
    /// per seen fighter, with no `Presenter`, so `load`, `observe_tick` and
    /// this run headless. A seen slot with no rig fails loud; unseen slots
    /// pose nothing. The same inputs pose the same bodies twice; the sim
    /// hash never reads them. Each posed body carries its bound from the
    /// posed vertices, floored at the rig's bind radius.
    fn pose_frame(&self, fighters: usize, alpha: f32) -> Result<Vec<PosedBody>, LookError> {
        let alpha = crate::stage_bodies::clamp_alpha(alpha);
        let mut posed = Vec::with_capacity(fighters.min(MAX_LOOK_FIGHTERS));
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = self
                .rigs
                .get(self.rig_for_slot(slot_index))
                .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
            let sample = self.slot_sample(rig, slot_index, &slot, alpha)?;
            let pos = sample.pos;
            let yaw = sample.yaw;
            let root = sample.root;
            let mut matrices: Vec<[[f32; 4]; 4]> = sample
                .world
                .iter()
                .zip(rig.inverse_binds.iter())
                .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
                .collect();
            let uniform = rig
                .pipe
                .skinning_uniform(&matrices)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
            let joints = sample
                .world
                .iter()
                .map(|m| [m[3][0], m[3][1], m[3][2]])
                .collect();
            let mut body = BodyTruth::new(
                u8::try_from(slot_index).unwrap_or(u8::MAX),
                act_label(slot.act),
                pos,
                yaw,
                joints,
            );
            body.clip = u8::try_from(sample.clip).unwrap_or(u8::MAX);
            body.clip_s = sample.time;
            body.tinted = self.tick < slot.flash_until;
            body.foot_slide_m = sample.slides;
            body.ragdolled = self.ragdolled(slot_index);
            body.react_head_m = sample.react_head_m;
            let mut frames = Vec::with_capacity(rig.parts.len());
            let owner = self.limb_owner(slot_index, rig);
            for part in rig.parts.iter() {
                // RZ3: a part on a stumped slot draws with the leaving chain's
                // vertices collapsed onto its stump, so the limb leaves the body.
                let posed = self.skin_part(rig, part, owner.as_deref(), &uniform)?;
                body.fold_vertices(&posed.positions);
                if posed.used_error_material {
                    body.error_parts = body.error_parts.saturating_add(1);
                }
                frames.push(posed);
            }
            // Sand rest, view only: no body is drawn under the sand.
            // A dead body sits on it both ways (up from a sink, down from a
            // hover, LF17); a ragdoll and a living body are lifted only, so a
            // run's flight phase is never pulled down to the sand.
            let lift = if !body.has_bounds() {
                0.0
            } else if self.ragdoll_rests(slot_index) {
                sand_lift_m(body.min[1])
            } else if lies_on_sand(&slot) {
                sand_rest_shift_m(body.min[1])
            } else {
                sand_lift_m(body.min[1])
            };
            if lift != 0.0 {
                body.root[1] += lift;
                body.min[1] += lift;
                body.max[1] += lift;
                // The blend weights sum to one, so lifting every joint's
                // translation row lifts each skinned vertex by `lift`, the
                // same shift `upload_slot_parts` adds to the CPU positions.
                for matrix in &mut matrices {
                    matrix[3][1] += lift;
                }
            }
            let mut drawn = PosedBody {
                truth: body,
                frames,
                matrices,
                lift,
                bound: None,
            };
            // One owner of the bound mint: the posed vertices, floored at
            // the rig's own bind radius so a bound never shrinks under the
            // sphere it replaces.
            drawn.bound =
                posed_bound_m(&drawn).map(|(centre, radius)| (centre, radius.max(rig.radius_m)));
            posed.push(drawn);
        }
        Ok(posed)
    }

    /// Joints-only pose: the same clip sample, root placement and tint as
    /// the full pose, but bounds come from the placed joints padded by the
    /// rig's skin radius, with no vertex skin and no `Presenter`, so a frame
    /// costs a few dozen flops per joint. It poses the same slots, joints,
    /// clip and clip time as the full pose, and its bounds hold the vertex
    /// bounds (see the `bounds_hold` check in tests). A seen slot with no
    /// rig fails loud; unseen slots pose nothing; the same inputs repeat
    /// bit for bit.
    pub fn pose_frame_joints(
        &self,
        fighters: usize,
        alpha: f32,
    ) -> Result<Vec<BodyTruth>, LookError> {
        let alpha = crate::stage_bodies::clamp_alpha(alpha);
        let mut out = Vec::with_capacity(fighters.min(MAX_LOOK_FIGHTERS));
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = self
                .rigs
                .get(self.rig_for_slot(slot_index))
                .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
            let sample = self.slot_sample(rig, slot_index, &slot, alpha)?;
            let pos = sample.pos;
            let yaw = sample.yaw;
            let root = sample.root;
            let joints: Vec<[f32; 3]> = sample
                .world
                .iter()
                .map(|m| [m[3][0], m[3][1], m[3][2]])
                .collect();
            let placed: Vec<[f32; 3]> = joints
                .iter()
                .map(|joint| place_joint(*joint, &root))
                .collect();
            let mut body = BodyTruth::new(
                u8::try_from(slot_index).unwrap_or(u8::MAX),
                act_label(slot.act),
                pos,
                yaw,
                joints,
            );
            body.clip = u8::try_from(sample.clip).unwrap_or(u8::MAX);
            body.clip_s = sample.time;
            body.tinted = self.tick < slot.flash_until;
            body.foot_slide_m = sample.slides;
            body.ragdolled = self.ragdolled(slot_index);
            body.react_head_m = sample.react_head_m;
            if let Some((lo, hi)) = crate::stage_bodies::joints_bounds(&placed, rig.radius_m) {
                body.min = lo;
                body.max = hi;
            }
            // LF17 sand rest, view only: the lift half of the full pose's rule.
            let lift = if (lies_on_sand(&self.slots[slot_index]) || self.ragdoll_rests(slot_index))
                && body.has_bounds()
            {
                sand_lift_m(body.min[1])
            } else {
                0.0
            };
            if lift > 0.0 {
                body.root[1] += lift;
                body.min[1] += lift;
                body.max[1] += lift;
            }
            out.push(body);
        }
        Ok(out)
    }

    /// Headless bout folded into one `arena view:` receipt line, with no
    /// `Presenter` and no window: steps a bout, observes each tick and poses
    /// the full CPU skin, so the bounds are the exact posed vertices and the
    /// sand-rest rule matches the window's line. Zero ticks fold no frame and
    /// read as `""`; otherwise the view laws line. The same seed gives the
    /// same line twice; the sim hash never reads it.
    /// Caller: `crates/endpoint/src/packs/view.rs` (SP62 `view.laws` pops this
    /// line); public so that door can reach it without a `Presenter`.
    pub fn view_run(seed: u64, ticks: u32, fighters: u8) -> Result<String, LookError> {
        Self::view_run_detail(seed, ticks, fighters).map(|(line, _)| line)
    }

    /// The same headless bout as [`view_run`](Self::view_run), with the worst
    /// pop named: the slot, the joint, the tick and how far that joint missed
    /// its prediction. The receipt line is byte-identical to `view_run`'s; the
    /// name is what the fix needs (AM24).
    pub fn view_run_detail(
        seed: u64,
        ticks: u32,
        fighters: u8,
    ) -> Result<(String, Option<crate::view_truth::PopDetail>), LookError> {
        if ticks == 0 {
            return Ok((String::new(), None));
        }
        let config = crate::GameConfig {
            seed,
            ticks,
            fighters,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).map_err(|err| LookError::Bind(format!("{err:?}")))?;
        let mut look = Self::load(&fighters_content_dir())?;
        let camera = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let mut laws = crate::view_truth::ViewLaws::default();
        for tick in 0..ticks {
            let outcome = app.tick_empty();
            // LF10: poll the app's mirrored hitstop the same way the window
            // twin does; the look's cue-derived freeze follows the same
            // 4-tick law, so the hold agrees. Presentation only, never hashed.
            let remaining = app.hitstop_remaining();
            let holding = crate::App::hitstop_holds(remaining);
            debug_assert!(
                remaining <= engine_combat::fight::HITSTOP_TICKS,
                "hitstop mirror stays within the sim freeze"
            );
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            debug_assert_eq!(
                holding,
                look.bridge.iter().any(|slot| tick < slot.hold_until()),
                "app mirror and look freeze agree at tick {tick}"
            );
            look.observe_live_balances(app.live_match(), usize::from(fighters));
            let bodies: Vec<crate::view_truth::BodyTruth> = look
                .pose_frame(usize::from(fighters), 0.5)?
                .into_iter()
                .map(|posed| posed.truth)
                .collect();
            laws.note(&camera, f64::from(tick) + 0.5, &bodies);
        }
        Ok((laws.line().unwrap_or_default(), laws.pop_detail()))
    }

    /// [`Self::stage_frame`] plus each skinned body as numbers (bounds of
    /// the exact posed vertices, model-space joints, clip, tint): the view
    /// truth costs one pass over vertices the CPU skin already produced.
    fn stage_bodies(
        &mut self,
        presenter: &mut Presenter,
        fighters: usize,
        alpha: f32,
    ) -> Result<(SkinSplit, Vec<BodyTruth>), LookError> {
        // Split: the CPU pose step lives in `pose_frame`; this keeps the
        // upload, so every existing caller poses and draws exactly as before.
        let pose_started = engine_clock::now();
        let posed = self.pose_frame(fighters, alpha)?;
        // RZ3: bake this frame's fallen limbs and stump caps into resident keys
        // before anything reads the resident list below.
        self.publish_limbs(presenter, alpha);
        // RZ7: upload each seen slot's cape sheet for this frame.
        self.publish_capes(presenter);
        let mut split = SkinSplit {
            uploaded: 0,
            pose_us: micros(engine_clock::since(pose_started)),
            upload_us: 0,
        };
        let mut truth = Vec::with_capacity(posed.len());
        for body in posed {
            let slot_index = usize::from(body.truth.slot);
            let rig_index = self.rig_for_slot(slot_index);
            let rig = self
                .rigs
                .get(rig_index)
                .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
            // The carried bound is what a cull reads, so it must agree with
            // the folded vertex box: one emptiness story for both owners.
            debug_assert_eq!(
                body.bound.is_some(),
                body.truth.has_bounds(),
                "posed bound and folded AABB agree on a finite body"
            );
            // No finite posed vertex means nothing to draw: the slot keeps
            // its resident mesh instead of writing NaN vertices over it.
            if body.bound.is_none() {
                truth.push(body.truth);
                continue;
            }
            // A body whose parts were staged as skinned meshes draws from
            // bind-pose vertices plus one joint block; the CPU-skinned frames
            // above then only fed its bounds. Any other body re-uploads its
            // posed vertices.
            let parts = if slot_is_gpu_skinned(presenter, slot_index, rig) {
                // A stumped slot draws from the collapsed CPU vertices, not from
                // the joint block: drop the skin block so the presenter draws the
                // mesh the upload below writes.
                if self.limbs.stumped(slot_index) {
                    for (part_index, part) in rig.parts.iter().enumerate() {
                        presenter
                            .upload_mesh(look_mesh_key(slot_index, part_index), &part.mesh)
                            .map_err(|err| LookError::Upload(err.to_string()))?;
                    }
                    upload_slot_parts(
                        presenter,
                        slot_index,
                        rig,
                        body.frames,
                        body.lift,
                        &mut self.uploaded_digest,
                    )?
                } else {
                    write_slot_skin(presenter, slot_index, rig.parts.len(), &body.matrices)?
                }
            } else {
                upload_slot_parts(
                    presenter,
                    slot_index,
                    rig,
                    body.frames,
                    body.lift,
                    &mut self.uploaded_digest,
                )?
            };
            split.uploaded += parts.uploaded;
            split.pose_us = split.pose_us.saturating_add(parts.pose_us);
            split.upload_us = split.upload_us.saturating_add(parts.upload_us);
            truth.push(body.truth);
        }
        Ok((split, truth))
    }

    /// Every body the last [`Self::present_into`] drew, as numbers.
    #[must_use]
    pub fn view_truth(&self) -> &[BodyTruth] {
        &self.truth
    }

    /// Sim time the last presented frame showed, in ticks: the last tick
    /// folded plus the interpolation `alpha`.
    #[must_use]
    pub fn sim_time(&self, alpha: f32) -> f64 {
        f64::from(self.tick) + f64::from(alpha)
    }

    /// Push one textured draw per resident body part; hit victims draw
    /// tinted for [`FLASH_TICKS`]. Missing keys draw nothing. Returns draws.
    pub fn push_draws(&self, list: &mut DrawList, resident: &[MeshKey], fighters: usize) -> usize {
        let mut pushed = 0usize;
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = &self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = &self.rigs[self.rig_for_slot(slot_index)];
            let flash = self.tick < slot.flash_until;
            for (part_index, part) in rig.parts.iter().enumerate() {
                let key = look_mesh_key(slot_index, part_index);
                if !resident.contains(&key) {
                    continue;
                }
                let material = if flash {
                    flash_material(&part.material)
                } else {
                    part.material.clone()
                };
                list.items.push(DrawItem {
                    mesh: key,
                    material,
                    base_color: part.texture.as_ref().map(|(key, _)| *key),
                    normal_map: None,
                    orm: None,
                    emissive: None,
                });
                pushed += 1;
            }
        }
        pushed
    }

    /// Window and capture entry: load from [`fighters_content_dir`],
    /// upload every base-color map, then build and stage the fight ring
    /// for a match clamped at `half_extent_m`. Cold path; loud on refusal.
    pub fn load_and_stage(
        presenter: &mut Presenter,
        half_extent_m: f32,
    ) -> Result<Self, LookError> {
        // Keep the built GPU skin path constructed in the live window frame.
        let _ = gpu_skin_adapter();
        // `E2040_BODIES=rocketbox` draws the realistic family instead of the
        // CT7 bodies; the default never changes.
        let mut look = if std::env::var("E2040_BODIES").as_deref() == Ok("rocketbox") {
            Self::load_rocketbox(&fighters_content_dir().join(fighter_bind::ROCKETBOX_DIR))?
        } else {
            Self::load(&fighters_content_dir())?
        };
        look.stage_textures(presenter)?;
        // The window draws the skin on the GPU: bind-pose vertices plus one
        // joint block per body. `E2040_CPU_SKIN` keeps the CPU-skinned
        // upload for A/B captures and as the fallback (the pose, bounds and
        // draw order are the same either way).
        if std::env::var_os("E2040_CPU_SKIN").is_some() {
            look.stage_meshes(presenter)?;
        } else {
            look.stage_skinned_meshes(presenter)?;
        }
        let ring = FightRing::load(&crate::arena_kit::kit_content_dir(), half_extent_m)
            .map_err(|err| LookError::Ring(err.to_string()))?;
        ring.stage(presenter)
            .map_err(|err| LookError::Ring(err.to_string()))?;
        look.ring = Some(ring);
        Ok(look)
    }

    /// One frame for the presenter: skin and upload every fighter, push the
    /// textured draws and aim the list camera for the frame's shot. Returns
    /// the frame's draws beside the fighter skin wall split ([`PresentReport`]);
    /// refusals are loud.
    pub fn present_into(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        frame: &LookFrame,
    ) -> Result<PresentReport, LookError> {
        let (skin, truth) = self.stage_bodies(presenter, frame.fighters, frame.alpha)?;
        self.truth = truth;
        // LF14: the swing ribbon the ring collected, staged for the frame's HDR
        // pass. A calm tick's ring is empty, so an idle slot draws no strip;
        // the strip is view only and never enters the hash.
        let (trail_verts, trail_lens) = self.trail_strip();
        presenter.set_trail_strips(&trail_verts, &trail_lens);
        // The grounding shade follows the bodies: one blob under every seen fighter.
        presenter.set_contact_feet(&self.contact_feet(frame.fighters));
        let resident = presenter.mesh_keys();
        if let Some(ring) = &self.ring {
            ring.push_draws(list, &resident);
        }
        let draws = self.push_draws(list, &resident, frame.fighters);
        let draws = draws + self.push_limb_draws(list, &resident);
        let draws = draws + self.push_cape_draws(list, &resident);
        if draws > 0 {
            list.camera = self.fight_camera(&list.camera, frame.shot, frame.fighters, frame.frame);
        }
        Ok(PresentReport { draws, skin })
    }

    /// Contact blobs for the sand under the seen fighters, slot order; unseen
    /// slots stay dead (radius 0). A floored fighter lies longer, so its blob
    /// is wider. Presentation only: where bodies stand, never fixed spots.
    #[must_use]
    fn contact_feet(&self, fighters: usize) -> [ContactFoot; CONTACT_FEET_MAX] {
        let dead = ContactFoot {
            x_m: 0.0,
            z_m: 0.0,
            radius_m: 0.0,
        };
        let mut feet = [dead; CONTACT_FEET_MAX];
        let seen = self.slots.iter().take(fighters.min(MAX_LOOK_FIGHTERS));
        for (slot, foot) in seen.zip(feet.iter_mut()).filter(|(slot, _)| slot.seen) {
            *foot = ContactFoot {
                x_m: slot.pos[0],
                z_m: slot.pos[2],
                radius_m: if slot.act == Act::Down {
                    CONTACT_DOWN_RADIUS_M
                } else {
                    CONTACT_STAND_RADIUS_M
                },
            };
        }
        feet
    }

    /// Seen fighters whose chest (1 m up) is in `camera`'s picture: the
    /// window's framing check, 0 when the shot shows nobody.
    #[must_use]
    pub fn fighters_in_view(&self, camera: &Camera, fighters: usize) -> usize {
        let chests = self
            .slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|slot| slot.seen)
            .map(|slot| [slot.pos[0], 1.0, slot.pos[2]]);
        count_in_picture(camera, chests)
    }

    /// Fighters under `fighters` the look has folded at least one tick for:
    /// the denominator the framing receipt reads `fighters_in_view` against.
    #[must_use]
    pub fn fighters_seen(&self, fighters: usize) -> usize {
        self.slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|slot| slot.seen)
            .count()
    }

    /// The hero's duel: midpoint, half separation and axis yaw toward the
    /// nearest other fighter (standing rivals first). `None` without a
    /// seen hero plus rival.
    fn duel(&self, fighters: usize) -> Option<([f32; 3], f32, f32)> {
        let hero = self.slots.get(self.hero).filter(|slot| slot.seen)?;
        let rival = self
            .slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .enumerate()
            .filter(|(index, slot)| *index != self.hero && slot.seen)
            .map(|(_, slot)| slot)
            .min_by(|a, b| {
                let key = |slot: &SlotAnim| {
                    let (dx, dz) = (slot.pos[0] - hero.pos[0], slot.pos[2] - hero.pos[2]);
                    (slot.act == Act::Down, dx * dx + dz * dz)
                };
                let (ka, kb) = (key(a), key(b));
                ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            })?;
        let (dx, dz) = (rival.pos[0] - hero.pos[0], rival.pos[2] - hero.pos[2]);
        let half = 0.5 * (dx * dx + dz * dz).sqrt();
        let mid = [hero.pos[0] + 0.5 * dx, 0.0, hero.pos[2] + 0.5 * dz];
        Some((mid, half, dx.atan2(dz)))
    }

    /// Farthest the camera eye may sit from the pit center on either floor
    /// axis, meters: the pit wall less a 0.6 m margin; unbounded without a ring.
    fn pit_limit_m(&self) -> f32 {
        self.ring
            .as_ref()
            .map_or(f32::MAX, |ring| ring.wall_m - 0.6)
    }

    /// View-only shake sample for the fight camera (LF11): one
    /// [`crate::camera::shake_offset`] sample for this tick's trauma, as an
    /// `[x, y, z]` vector added to both eye and target. Pure; zero trauma
    /// gives `[0.0; 3]`, so calm shots pass through untouched.
    fn fight_shake_vec(&self) -> [f32; 3] {
        let shake = crate::camera::shake_offset(self.shake_trauma_permille, self.tick, SHAKE_MAX_M);
        [shake.x, shake.y, shake.z]
    }

    /// Wide establishing camera aimed at `center`: keeps the base eye, aims at
    /// the fight, then moves both by `shake` before validate. Pure presentation.
    fn wide_camera(base: &Camera, center: [f32; 3], shake: [f32; 3]) -> Camera {
        let mut camera = base.clone();
        camera.target = [center[0], 1.0, center[2]].into();
        let eye_arr = camera.eye.to_array();
        camera.eye = [
            eye_arr[0] + shake[0],
            eye_arr[1] + shake[1],
            eye_arr[2] + shake[2],
        ]
        .into();
        let tgt_arr = camera.target.to_array();
        camera.target = [
            tgt_arr[0] + shake[0],
            tgt_arr[1] + shake[1],
            tgt_arr[2] + shake[2],
        ]
        .into();
        if camera.validate().is_ok() {
            camera
        } else {
            base.clone()
        }
    }

    /// Fight camera for `shot`: aims at the hero, the whole fight, or a
    /// wide arena view, orbiting slowly with `frame`. Pure presentation.
    /// View-only shake (LF11): the same [`crate::camera::shake_offset`]
    /// offset moves both eye and target, so the frame translates while the
    /// view direction holds; zero trauma leaves the shot untouched.
    #[must_use]
    pub fn fight_camera(&self, base: &Camera, shot: Shot, fighters: usize, frame: u32) -> Camera {
        self.fight_camera_with(base, shot, fighters, frame, self.fight_shake_vec())
    }

    /// The shot of [`Self::fight_camera`] with `shake` moving eye and target
    /// together; `[0.0; 3]` is the calm shot the stateful follower smooths
    /// (`fighter_look_cam`) before it adds the shake back.
    #[must_use]
    fn fight_camera_with(
        &self,
        base: &Camera,
        shot: Shot,
        fighters: usize,
        frame: u32,
        shake: [f32; 3],
    ) -> Camera {
        // The hero first (the shoulder shot rides the first body), then the rest in slot order.
        let fighters = fighters.min(MAX_LOOK_FIGHTERS);
        let seen: Vec<[f32; 3]> = std::iter::once(self.hero)
            .filter(|hero| *hero < fighters)
            .chain((0..fighters).filter(|index| *index != self.hero))
            .filter_map(|index| self.slots.get(index))
            .filter(|slot| slot.seen)
            .map(|slot| slot.pos)
            .collect();
        let Some(first) = seen.first().copied() else {
            return base.clone();
        };
        let count = f32::from(u8::try_from(seen.len()).unwrap_or(u8::MAX));
        let mut center = [0.0f32; 3];
        for pos in &seen {
            center[0] += pos[0] / count;
            center[2] += pos[2] / count;
        }
        let mut radius = 0.0f32;
        for pos in &seen {
            let (dx, dz) = (pos[0] - center[0], pos[2] - center[2]);
            radius = radius.max((dx * dx + dz * dz).sqrt());
        }
        // The live duel: exactly two bodies in the fight shot get the shoulder
        // shot, close and readable at any separation; the side-on fit below
        // stays for bouts of three or more and when the pit blocks the shot.
        if shot == Shot::Action {
            if let Some(camera) = self.shoulder_duel(base, &seen, shake) {
                return camera;
            }
        }
        let turn = f32::from(u16::try_from(frame % 3600).unwrap_or(0)) * 0.0025;
        let (focus, distance, height, look_y, azimuth) = match shot {
            Shot::Hero => {
                let hero = self.slots[self.hero];
                let at = if hero.seen { hero.pos } else { first };
                (at, 3.4, 1.55, 1.15, hero.yaw + 0.55 + turn * 0.2)
            }
            Shot::Action => match self.duel(fighters) {
                // Side-on to the hero's duel: both fighters across frame.
                // Three-to-four seen fall through to the group fit below.
                Some((mid, half, axis)) if seen.len() <= 2 || seen.len() > 4 => (
                    mid,
                    (half * 2.2 + 3.2).clamp(3.5, 7.0),
                    0.0,
                    1.0,
                    axis + std::f32::consts::FRAC_PI_2,
                ),
                _ => (center, (radius * 1.9 + 4.5).clamp(5.0, 16.0), 0.0, 1.0, 0.9),
            },
            Shot::Wide => {
                return Self::wide_camera(base, center, shake);
            }
        };
        // Keep the eye inside the pit so the wall never blocks the shot.
        let limit = self.pit_limit_m();
        // View-only shake (LF11): one sample moves both eye and target
        // before validate, so the same ticks give the same offsets.
        let spec = ShotSpec {
            focus,
            distance,
            height,
            look_y,
            azimuth,
        };
        let camera = shot_camera(base, spec, limit, shake);
        // The fight shot shows the fight: when the close framing drops a
        // live body, a wider eye that holds more of them takes over. The
        // wanted azimuth is fixed: a per-frame turn made even a still pack
        // orbit the pit, which read as camera drift in the live window.
        if shot == Shot::Action {
            let wanted = 0.9;
            if let Some(wider) = wide_group_camera(base, &seen, &camera, limit, wanted, shake) {
                return wider;
            }
        }
        camera
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use engine_audio::FightCue;

    use crate::FighterView;

    fn content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    fn view(slot: u8, x: f32, z: f32) -> FighterView {
        FighterView {
            slot,
            pos_m: [x, 0.0, z],
            yaw: 0.0,
        }
    }

    #[test]
    fn root_matrix_turns_forward_onto_yaw() {
        let m = root_matrix([1.0, 0.0, 2.0], std::f32::consts::FRAC_PI_2);
        // Forward +Z at yaw 90 degrees points along +X; translation last row.
        let forward = [m[2][0], m[2][1], m[2][2]];
        assert!((forward[0] - 1.0).abs() < 1e-6 && forward[2].abs() < 1e-6);
        assert_eq!(m[3], [1.0, 0.0, 2.0, 1.0]);
        let id = mul(&m, &root_matrix([0.0; 3], 0.0));
        assert_eq!(id, m);
    }

    #[test]
    fn directed_runs_split_into_hero_wide_action_thirds() {
        assert_eq!(shot_for_frame(0, Some(120), true), Shot::Hero);
        assert_eq!(shot_for_frame(39, Some(120), true), Shot::Hero);
        assert_eq!(shot_for_frame(40, Some(120), true), Shot::Wide);
        assert_eq!(shot_for_frame(80, Some(120), true), Shot::Action);
        assert_eq!(shot_for_frame(0, Some(120), false), Shot::Action);
        assert_eq!(shot_for_frame(0, None, true), Shot::Action);
    }

    #[test]
    fn fight_camera_eye_backs_off_a_wall_instead_of_hovering() {
        // A duel 1 m off the +z wall with the side-on eye wanted toward it:
        // per-axis clamping parked that eye 1 m from the focus and 4.5 m up.
        let (limit, focus) = (9.4, [0.0, 0.0, 8.4]);
        let (azimuth, reach) = pit_orbit(focus, 0.0, 9.0, limit);
        assert!((reach - 9.0).abs() < 1e-4, "full distance on the open side");
        let eye_z = focus[2] + azimuth.cos() * reach;
        assert!(eye_z.abs() <= limit + 1e-3, "eye inside the pit: {eye_z}");
        assert!(focus[2] - eye_z > 8.0, "eye backs into the pit: {eye_z}");
        assert_eq!(pit_orbit([0.0; 3], 0.3, 9.0, f32::MAX), (0.3, 9.0));
    }

    #[test]
    fn fight_camera_framing_sees_a_side_on_duel_and_not_a_top_down_one() {
        let side = Camera::new(
            [0.0, 4.5, 9.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        assert!(in_picture(&side, [-3.0, 1.0, 0.0]) && in_picture(&side, [3.0, 1.0, 0.0]));
        assert!(!in_picture(&side, [0.0, 1.0, 12.0]), "behind the eye");
        assert!(look_down_deg(&side) > 20.0 && look_down_deg(&side) < 25.0);
        // The 2026-10-11 shot: eye 1 m off the duel's middle, 4.5 m up, fighters 5 m either side.
        let top = Camera::new(
            [0.0, 4.5, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        assert!(!in_picture(&top, [-5.0, 1.0, 0.0]) && !in_picture(&top, [5.0, 1.0, 0.0]));
        assert!(look_down_deg(&top) > 70.0);
    }

    /// Group-shot camera state without bodies: seven seen slots across
    /// +-6 m, hero unseen so `Shot::Action` frames the whole bout through
    /// [`FighterLook::fight_camera`], the path the widening changed.
    fn spread_look() -> FighterLook {
        let mut slots = vec![SlotAnim::new(); MAX_LOOK_FIGHTERS];
        for (slot, x) in slots
            .iter_mut()
            .skip(1)
            .zip([-6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0])
        {
            slot.seen = true;
            slot.pos = [x, 0.0, 0.0];
        }
        FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots,
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
            inertia: Vec::new(),
            limit: Vec::new(),
            ragdoll: RagdollLook::empty(),
            limbs: LimbLook::empty(),
            capes: Capes::empty(),
            hero: 0,
            checkpoints: empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
        }
    }

    #[test]
    fn fight_camera_widens_group_shot_to_60deg_and_frames_the_bout() {
        let mut look = spread_look();
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let cam = look.fight_camera(&base, Shot::Action, 8, 0);
        assert!(
            (cam.fov_y.to_degrees() - 60.0).abs() < 1e-4,
            "group shot rides the widened lens, got {} deg",
            cam.fov_y.to_degrees()
        );
        assert_eq!(
            look.fighters_in_view(&cam, 8),
            7,
            "the whole spread stays in the 60 deg picture"
        );
        // The duel branch rides the same lens: hero plus nearest rival
        // framed side-on at 60 deg.
        look.slots[0].seen = true;
        look.slots[0].pos = [20.0, 0.0, 0.0];
        let duel_cam = look.fight_camera(&base, Shot::Action, 8, 0);
        assert!(
            (duel_cam.fov_y.to_degrees() - 60.0).abs() < 1e-4,
            "duel shot rides the widened lens, got {} deg",
            duel_cam.fov_y.to_degrees()
        );
        assert!(
            in_picture(&duel_cam, [20.0, 1.0, 0.0]) && in_picture(&duel_cam, [7.0, 1.0, 0.0]),
            "hero plus nearest rival stay in the duel picture"
        );
    }

    #[test]
    fn fight_camera_frames_four_with_hero_seen() {
        // Two pairs: the duel fit held two, the group fit holds all four.
        let mut slots = vec![SlotAnim::new(); MAX_LOOK_FIGHTERS];
        for (slot, at) in slots.iter_mut().zip([
            [-2.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [5.0, 0.0, 2.0],
            [6.0, 0.0, 2.0],
        ]) {
            slot.seen = true;
            slot.pos = at;
        }
        let look = FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots,
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
            inertia: Vec::new(),
            limit: Vec::new(),
            ragdoll: RagdollLook::empty(),
            limbs: LimbLook::empty(),
            capes: Capes::empty(),
            hero: 0,
            checkpoints: empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
        };
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let cam = look.fight_camera(&base, Shot::Action, 4, 0);
        assert_eq!(
            look.fighters_in_view(&cam, 4),
            4,
            "both pairs stay in the 60 deg picture"
        );
    }

    /// `spread_look` re-seated on `bodies` (floor x/z, slot order, all seen).
    fn look_at(bodies: &[[f32; 2]]) -> FighterLook {
        let mut look = spread_look();
        for slot in &mut look.slots {
            slot.seen = false;
        }
        for (slot, at) in look.slots.iter_mut().zip(bodies) {
            slot.seen = true;
            slot.pos = [at[0], 0.0, at[1]];
        }
        look
    }

    /// Where the eight bodies of the standing 8-fighter run (seed 2040) stood
    /// at ticks 0 and 250: the hero pinned at the +x wall, the rest across a
    /// 36 by 30 m field.
    const ARENA_SPREADS: [[[f32; 2]; 8]; 2] = [
        [
            [19.2, 3.4],
            [-16.6, -11.2],
            [9.7, -2.2],
            [8.9, 11.5],
            [-1.3, 16.2],
            [-2.9, 17.9],
            [18.1, 15.0],
            [16.9, 15.0],
        ],
        [
            [19.2, 3.4],
            [-9.6, -8.6],
            [16.5, 1.0],
            [13.3, 13.1],
            [-2.1, 17.8],
            [-3.2, 18.3],
            [14.9, 12.6],
            [14.2, 13.5],
        ],
    ];

    fn arena_base() -> Camera {
        Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        )
    }

    #[test]
    fn two_fighters_in_the_fight_shot_get_the_shoulder_camera() {
        // The 2026-10-18 demo capture: two bodies 15 m apart were two specks
        // at the screen edges. The duel rides the hero's shoulder instead.
        let look = look_at(&[[-8.0, 1.0], [7.0, -2.0]]);
        let cam = look.fight_camera(&arena_base(), Shot::Action, 2, 0);
        assert_eq!(look.fighters_in_view(&cam, 2), 2, "both bodies in picture");
        let eye = cam.eye.to_array();
        assert!((eye[1] - 1.9).abs() < 0.01, "shoulder-high eye: {eye:?}");
        assert!(
            (eye[0] + 8.0).hypot(eye[2] - 1.0) < 6.0,
            "eye rides the hero, not the far side of the pit: {eye:?}"
        );
        // A third body hands the shot back to the group fit (high boom).
        let three = look_at(&[[-8.0, 1.0], [7.0, -2.0], [0.0, 9.0]]);
        let group = three.fight_camera(&arena_base(), Shot::Action, 3, 0);
        assert!(
            group.eye.y > 2.5,
            "three bodies keep the group fit: {:?}",
            group.eye
        );
        // The directed run's hero close-up never takes the duel shot.
        let hero = look.fight_camera(&arena_base(), Shot::Hero, 2, 0);
        assert!(
            hero.eye.y < 1.9,
            "the hero shot keeps its own boom: {:?}",
            hero.eye
        );
    }

    #[test]
    fn a_network_guest_gets_the_shoulder_of_its_own_slot() {
        // The guest plays slot 1: the camera rides behind slot 1, so W walks
        // at the rival instead of away from it.
        let mut look = look_at(&[[-8.0, 1.0], [7.0, -2.0]]);
        look.set_hero(1);
        let cam = look.fight_camera(&arena_base(), Shot::Action, 2, 0);
        assert_eq!(look.fighters_in_view(&cam, 2), 2, "both bodies in picture");
        let eye = cam.eye.to_array();
        assert!(
            (eye[0] - 7.0).hypot(eye[2] + 2.0) < 6.0,
            "eye rides slot 1: {eye:?}"
        );
        assert!(
            (eye[0] + 8.0).hypot(eye[2] - 1.0) > 9.0,
            "not slot 0: {eye:?}"
        );
        look.set_hero(99);
        let kept = look.fight_camera(&arena_base(), Shot::Action, 2, 0);
        assert_eq!(
            kept.eye.to_array(),
            eye,
            "an out-of-range slot keeps the hero"
        );
    }

    #[test]
    fn contact_blobs_sit_under_the_seen_fighters_and_nowhere_else() {
        // The live window drew dark discs at 24 fixed spots and none under the
        // fighters. The feed is the bodies' own floor positions, slot order.
        let mut look = look_at(&[[-8.0, 1.0], [7.0, -2.0], [0.0, 9.0]]);
        look.slots[1].act = Act::Down;
        let feet = look.contact_feet(2);
        assert_eq!((feet[0].x_m, feet[0].z_m), (-8.0, 1.0), "the hero's feet");
        assert_eq!((feet[1].x_m, feet[1].z_m), (7.0, -2.0), "the rival's feet");
        assert_eq!(feet[0].radius_m, CONTACT_STAND_RADIUS_M);
        assert!(
            feet[1].radius_m > feet[0].radius_m,
            "a floored body lies wider"
        );
        // A third body outside `fighters` and every unseen slot stay dead.
        assert!(feet[2..].iter().all(|foot| foot.radius_m == 0.0));
        let all = look.contact_feet(8);
        assert_eq!(
            (all[2].x_m, all[2].z_m),
            (0.0, 9.0),
            "three bodies, three blobs"
        );
        let mut gone = look_at(&[[-8.0, 1.0], [7.0, -2.0]]);
        gone.slots[0].seen = false;
        let feet = gone.contact_feet(8);
        assert_eq!(feet[0].radius_m, 0.0, "an unseen fighter casts nothing");
        assert_eq!((feet[1].x_m, feet[1].z_m), (7.0, -2.0));
        assert_eq!(
            CONTACT_FEET_MAX, MAX_LOOK_FIGHTERS,
            "one blob per look slot"
        );
    }

    #[test]
    fn fight_camera_frames_every_live_body_at_the_arena_spread() {
        // The hero's duel shot held 2 of 8 here; the fight shot holds all.
        for spread in &ARENA_SPREADS {
            let look = look_at(spread);
            let cam = look.fight_camera(&arena_base(), Shot::Action, 8, 0);
            assert_eq!(look.fighters_seen(8), 8);
            assert_eq!(look.fighters_in_view(&cam, 8), 8, "every body in picture");
        }
    }

    #[test]
    fn fight_camera_stays_near_head_height_and_does_not_orbit() {
        // The live fight shot on a spread pack: the eye stays near head
        // height (the old 0.22 rise put a 22 m pack shot at 6.8 m, a drone
        // over the pit) and a still pack does not orbit with the frame
        // counter. Both measured on 2026-10-20 from --view-brief.
        for spread in &ARENA_SPREADS {
            let look = look_at(spread);
            let mut azimuth: Option<f32> = None;
            for tick in [0u32, 30, 90, 150] {
                let cam = look.fight_camera(&arena_base(), Shot::Action, 8, tick);
                let eye = cam.eye.to_array();
                let focus = cam.target.to_array();
                let reach = (eye[0] - focus[0]).hypot(eye[2] - focus[2]);
                assert!(
                    eye[1] <= 1.6 + 0.07 * reach + 1.0e-2,
                    "tick {tick}: eye {eye:?} rises above 1.6 + 0.07 * {reach} m"
                );
                let bearing = eye[0].atan2(eye[2]);
                if let Some(prev) = azimuth {
                    assert!(
                        (bearing - prev).abs() < 1.0e-3,
                        "tick {tick}: the still pack orbited {} rad",
                        bearing - prev
                    );
                } else {
                    azimuth = Some(bearing);
                }
            }
        }
    }

    #[test]
    fn wide_group_camera_holds_the_arena_spread_from_inside_the_pit() {
        // The pit wall sits 21 m out, the eye 0.6 m inside it.
        let limit = 20.4;
        for spread in &ARENA_SPREADS {
            let look = look_at(spread);
            let seen: Vec<[f32; 3]> = look.slots.iter().take(8).map(|s| s.pos).collect();
            let close = look.fight_camera(&arena_base(), Shot::Action, 2, 0);
            let cam = wide_group_camera(&arena_base(), &seen, &close, limit, 0.9, [0.0; 3])
                .expect("a wider eye holds the spread");
            assert_eq!(look.fighters_in_view(&cam, 8), 8);
            let eye = cam.eye.to_array();
            assert!(
                eye[0].abs() <= limit + 1e-3 && eye[2].abs() <= limit + 1e-3,
                "eye {eye:?} left the pit"
            );
        }
    }

    #[test]
    fn arena_run_keeps_all_eight_bodies_in_the_fight_shot_inside_the_pit() {
        // The window's own session, idle hero: at every tick of 300 the
        // fight shot (pit wall 21 m, eye 0.6 m inside it) holds all eight.
        let config = crate::GameConfig {
            seed: 2040,
            ticks: 300,
            fighters: 8,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::with_world(config, crate::arena_kit::kit_match_world_or_empty())
            .and_then(crate::App::into_duel)
            .expect("arena session builds");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let human = engine_input::FighterId::new(crate::HUMAN_SLOT).expect("human slot");
        let (mut last_eye, mut max_step): (Option<[f32; 3]>, f32) = (None, 0.0);
        for tick in 0..300 {
            let idle = engine_input::FighterInput {
                tick,
                fighter: human,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: engine_input::Buttons::NONE,
            };
            let out = app.tick(idle);
            look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
            let seen: Vec<[f32; 3]> = look.slots.iter().take(8).map(|s| s.pos).collect();
            assert_eq!(look.fighters_seen(8), 8, "tick {tick}: every body folded");
            let close = look.fight_camera(&arena_base(), Shot::Action, 2, tick);
            let cam = wide_group_camera(&arena_base(), &seen, &close, 20.4, 0.9, [0.0; 3])
                .expect("a wider eye holds the field");
            assert_eq!(look.fighters_in_view(&cam, 8), 8, "tick {tick}");
            let eye = cam.eye.to_array();
            if let Some(prev) = last_eye {
                let step = ((eye[0] - prev[0]).powi(2) + (eye[2] - prev[2]).powi(2)).sqrt();
                max_step = max_step.max(step);
            }
            last_eye = Some(eye);
            assert!(
                eye[0].abs() <= 20.4 + 1e-3 && eye[2].abs() <= 20.4 + 1e-3,
                "tick {tick}: eye {eye:?} left the pit"
            );
        }
        // The eye glides with the bodies; a flip to another orbit would jump
        // tens of meters in one tick.
        assert!(max_step < 1.0, "eye jumped {max_step} m in one tick");
    }

    #[test]
    fn wide_group_camera_keeps_a_shot_that_already_holds_everyone() {
        let look = look_at(&[[0.0, 0.0], [2.0, 0.0]]);
        let seen: Vec<[f32; 3]> = look.slots.iter().take(2).map(|s| s.pos).collect();
        let cam = look.fight_camera(&arena_base(), Shot::Action, 2, 0);
        assert_eq!(look.fighters_in_view(&cam, 2), 2);
        assert!(wide_group_camera(&arena_base(), &seen, &cam, 20.4, 0.9, [0.0; 3]).is_none());
    }

    #[test]
    fn wide_group_camera_needs_room_for_the_eye() {
        // A pit smaller than the nearest wide eye cannot widen the shot: the
        // close framing stays, whatever it drops.
        let look = look_at(&ARENA_SPREADS[0]);
        let seen: Vec<[f32; 3]> = look.slots.iter().take(8).map(|s| s.pos).collect();
        let close = look.fight_camera(&arena_base(), Shot::Action, 2, 0);
        assert!(wide_group_camera(&arena_base(), &seen, &close, 2.0, 0.9, [0.0; 3]).is_none());
    }

    #[test]
    fn fight_camera_without_seen_fighters_returns_base_untouched() {
        // Failure path: nobody seen, the base camera (lens included)
        // passes through instead of framing an empty bout.
        let look = FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
            inertia: Vec::new(),
            limit: Vec::new(),
            ragdoll: RagdollLook::empty(),
            limbs: LimbLook::empty(),
            capes: Capes::empty(),
            hero: 0,
            checkpoints: empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
        };
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let cam = look.fight_camera(&base, Shot::Action, 8, 0);
        assert_eq!(cam, base, "empty bout keeps the base camera");
    }

    #[test]
    fn fight_camera_eye_stays_in_the_pit_from_every_corner() {
        for limit in [9.4f32, 4.0] {
            for focus in [
                [limit, 0.0, limit],
                [-limit, 0.0, limit],
                [limit, 0.0, -limit],
                [-limit, 0.0, -limit],
            ] {
                for step in 0..16u8 {
                    let wanted = f32::from(step) * std::f32::consts::FRAC_PI_8;
                    let (azimuth, reach) = pit_orbit(focus, wanted, 9.0, limit);
                    let (x, z) = (
                        focus[0] + azimuth.sin() * reach,
                        focus[2] + azimuth.cos() * reach,
                    );
                    assert!(
                        x.abs() <= limit + 1e-3 && z.abs() <= limit + 1e-3,
                        "eye ({x}, {z}) left the pit"
                    );
                    // The roomiest side always reaches across most of the pit.
                    assert!(
                        reach >= 9.0f32.min(limit * 1.4),
                        "reach {reach} at {wanted} in {limit}"
                    );
                    let rise = 4.5f32.min(1.0 + reach * MAX_PITCH_SLOPE) - 1.0;
                    assert!(
                        rise / reach <= MAX_PITCH_SLOPE + 1e-4,
                        "never a top-down look"
                    );
                }
            }
        }
    }

    #[test]
    fn angle_delta_takes_the_short_way() {
        let d = angle_delta(3.0, -3.0);
        assert!(d > 0.0 && d < 0.3, "wraps through PI, got {d}");
        assert!((angle_delta(0.0, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn look_keys_never_collide_across_slots_and_parts() {
        let mut keys = Vec::new();
        for slot in 0..MAX_LOOK_FIGHTERS {
            for part in 0..4 {
                keys.push(look_mesh_key(slot, part));
            }
        }
        let total = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), total);
        assert!(keys.iter().all(|key| key.id >= LOOK_MESH_ID_BASE));
    }

    #[test]
    fn ct7_bodies_load_with_clips_and_textures() {
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        assert_eq!(look.rigs.len(), 4);
        assert_eq!(
            look.bundle_shas(),
            &fighter_bind::EXPECTED_BUNDLE_SHA256[..]
        );
        assert_eq!(look.variant_for_slot(0), "hero");
        assert_ne!(look.variant_for_slot(1), "hero");
        for rig in &look.rigs {
            assert_eq!(rig.clips.len(), CLIP_FILES.len());
            assert!(rig.parts.iter().all(|part| part.texture.is_some()));
        }
    }

    #[test]
    fn rest_pose_skins_onto_the_bind_mesh_at_the_root() {
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let rig = &look.rigs[0];
        // Bind pose through skinning must give the bind mesh moved by root.
        let bind = engine_anim::contract::LocalPose {
            positions: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].positions.tracks[j].values[0])
                .collect(),
            rotations: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].rotations[j].values[0])
                .collect(),
        };
        let world = skinning_matrices(&rig.skeleton, &bind).expect("pose");
        let root = root_matrix([3.0, 0.0, -2.0], 0.0);
        let matrices: Vec<[[f32; 4]; 4]> = world
            .iter()
            .zip(rig.inverse_binds.iter())
            .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
            .collect();
        let uniform = rig.pipe.skinning_uniform(&matrices).expect("uniform");
        let part = &rig.parts[0];
        let posed = rig
            .pipe
            .render_skinned(&part.mesh, Some(&part.skin), &uniform)
            .expect("skin");
        // Idle frame 0 is close to bind: every vertex lands near its bind
        // spot plus the root offset, and the body stays on its feet.
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for p in &posed.positions {
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
        assert!(max_y > 1.0 && max_y < 2.2, "standing height, got {max_y}");
        assert!(min_y > -0.2, "feet stay above the sand, got {min_y}");
        let (min_x, max_x) = posed
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p[0]), hi.max(p[0]))
            });
        assert!(
            min_x > 2.0 && max_x < 4.0,
            "root moved the body to x=3, got {min_x}..{max_x}"
        );
    }

    #[test]
    fn cues_drive_swings_tint_and_knockdown() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        assert_eq!(look.act(0), Some(Act::Idle));
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(20, &views, &[1000, 900], &[hit]);
        assert!(matches!(look.act(0), Some(Act::Attack(_))));
        assert!(look.slots[1].flash_until > 20);
        // Facing turns toward the rival (+X from slot 0).
        assert!(look.slots[0].yaw > 0.0);
        look.observe_tick(21, &views, &[1000, 0], &[]);
        assert_eq!(look.act(1), Some(Act::Down));
        // Running: a slot that moves faster than the run floor runs.
        let moved = [view(0, -0.9, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(80, &moved, &[1000, 0], &[]);
        assert!(matches!(look.act(0), Some(Act::Attack(_) | Act::Run)));
    }

    #[test]
    fn hitstop_hold_freezes_impact_pair_while_others_advance() {
        // LF10: during the sim's hitstop the attacker's and victim's clip
        // clocks stay put while every other slot advances; afterwards all
        // advance. Headless: clip phase reads as `tick - act_start`.
        let mut look = balance_look_two();
        look.slots[2].seen = true;
        look.slots[2].pos = [0.0, 0.0, 2.0];
        look.slots[2].prev_pos = [0.0, 0.0, 2.0];
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0), view(2, 0.0, 2.0)];
        let hp = [1000, 1000, 1000];
        for t in 0..10u32 {
            look.observe_tick(t, &views, &hp, &[]);
        }
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(10, &views, &hp, &[hit]);
        let since =
            |look: &FighterLook, s: usize| look.tick.saturating_sub(look.slots[s].act_start);
        let frozen = [since(&look, 0), since(&look, 1)];
        let free = since(&look, 2);
        // Three quiet ticks of hitstop plus the impact tick (four held ticks
        // matching the App hitstop count 4,3,2,1): the impact pair holds.
        for t in 11..14u32 {
            look.observe_tick(t, &views, &hp, &[]);
            assert_eq!(since(&look, 0), frozen[0], "attacker holds at tick {t}");
            assert_eq!(since(&look, 1), frozen[1], "victim holds at tick {t}");
            assert_eq!(
                since(&look, 2),
                free.saturating_add(t.saturating_sub(10)),
                "bystander advances at tick {t}"
            );
        }
        // Hold spent: every clock advances again.
        look.observe_tick(14, &views, &hp, &[]);
        assert_eq!(
            since(&look, 0),
            frozen[0].saturating_add(1),
            "attacker resumes"
        );
        assert_eq!(
            since(&look, 1),
            frozen[1].saturating_add(1),
            "victim resumes"
        );
        assert_eq!(
            since(&look, 2),
            free.saturating_add(4),
            "bystander keeps advancing"
        );
        // Same script twice gives the same clocks.
        let mut again = balance_look_two();
        again.slots[2].seen = true;
        again.slots[2].pos = [0.0, 0.0, 2.0];
        again.slots[2].prev_pos = [0.0, 0.0, 2.0];
        for t in 0..10u32 {
            again.observe_tick(t, &views, &hp, &[]);
        }
        again.observe_tick(10, &views, &hp, &[hit]);
        for t in 11..15u32 {
            again.observe_tick(t, &views, &hp, &[]);
        }
        assert_eq!(since(&again, 0), since(&look, 0), "attacker replays");
        assert_eq!(since(&again, 1), since(&look, 1), "victim replays");
        assert_eq!(since(&again, 2), since(&look, 2), "bystander replays");
    }

    pub(super) fn balance_look_two() -> FighterLook {
        let mut slots = vec![SlotAnim::new(); MAX_LOOK_FIGHTERS];
        for (s, x) in slots.iter_mut().take(2).zip([-1.0, 1.0]) {
            s.seen = true;
            s.pos = [x, 0.0, 0.0];
            s.prev_pos = s.pos;
        }
        FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots,
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
            inertia: Vec::new(),
            limit: Vec::new(),
            ragdoll: RagdollLook::empty(),
            limbs: LimbLook::empty(),
            capes: Capes::empty(),
            hero: 0,
            checkpoints: empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
        }
    }

    #[test]
    fn balance_look_floored_lies_flat() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 2,
        };
        // Sim digest stays equal with the look on and off.
        let run_hashes = |with_look: bool| {
            let mut sim =
                LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
            let mut look = balance_look_two();
            let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
            let mut hashes = Vec::new();
            for t in 0..10u32 {
                if let Some(tick) = sim.step(&[]) {
                    hashes.push(tick.checkpoint.hash);
                }
                if with_look {
                    look.observe_tick(t, &views, &[1000, 1000], &[]);
                    look.observe_live_balances(&sim, 2);
                }
            }
            hashes
        };
        assert_eq!(run_hashes(false), run_hashes(true));
        assert_eq!(
            run_hashes(true),
            run_hashes(true),
            "same look twice hashes twice"
        );
        let mut sim =
            LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
        for _ in 0..10u32 {
            let _ = sim.step(&[]);
        }
        assert_eq!(sim.tick(), 10, "scripted v2 bout runs 10 ticks");
        // Authoritative mirror: both slots read `Sim::balance` via `LiveMatch`.
        let mut look = balance_look_two();
        look.observe_live_balances(&sim, 2);
        for index in 0..2usize {
            let slot = u8::try_from(index).expect("slot fits");
            let want = sim.balance(slot).expect("slot live").gait();
            assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors sim");
        }
        // One floored slot (authoritative `Fallen`): root below 0.4 m within
        // 10 frames, standing slot unchanged.
        let standing_before = look.slots[0];
        look.observe_balance(1, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen);
        assert!(balance_pelvis_y_m(GaitPhase::Fallen, 0.9) < 0.4);
        assert_eq!(look.slots[0], standing_before, "standing keeps pose");
        // Flat pose with real rigs: `Fallen` forces the `Down` last frame.
        let rigged = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let rig = &rigged.rigs[rigged.rig_for_slot(1)];
        for frame in 0..10u32 {
            let sample = look
                .slot_sample(rig, 1, &look.slots[1], 0.5)
                .expect("floored sample");
            assert_eq!(sample.clip, 5, "floored draws flat on frame {frame}");
            let pose = look
                .slot_pose(rig, &look.slots[1], 0.5)
                .expect("floored pose samples");
            assert!(
                pose.positions[rig.pelvis][1] < 0.4,
                "floored root below 0.4 m on frame {frame}"
            );
        }
        // Standing keeps the act's clip and pelvis.
        let calm_sample = look
            .slot_sample(rig, 0, &look.slots[0], 0.5)
            .expect("standing sample");
        assert_ne!(calm_sample.clip, 5, "standing never draws flat");
        assert_eq!(balance_pelvis_y_m(GaitPhase::Steady, 0.9), 0.9);
        assert_eq!(balance_lean_m(GaitPhase::Steady), 0.0);
        look.stumble_slot(99);
        look.floor_slot(99);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
    }

    #[test]
    fn balance_look_authoritative_verdict_lies_flat_and_kneels() {
        let mut look = balance_look_two();
        look.observe_balance(1, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen);
        assert!(
            balance_pelvis_y_m(GaitPhase::Fallen, 0.9) < 0.4,
            "fallen lies flat below 0.4 m"
        );
        look.observe_balance(0, GaitPhase::Catching);
        assert_eq!(balance_pelvis_y_m(GaitPhase::Catching, 0.9), 0.45);
        assert_eq!(balance_lean_m(GaitPhase::Catching), 0.0);
        look.observe_balance(1, GaitPhase::Stumbling);
        assert_eq!(
            look.slots[1].balance,
            GaitPhase::Fallen,
            "fallen never downgrades"
        );
        look.observe_balance(99, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
        let mut again = balance_look_two();
        again.observe_balance(1, GaitPhase::Fallen);
        again.observe_balance(0, GaitPhase::Catching);
        again.observe_balance(1, GaitPhase::Stumbling);
        again.observe_balance(99, GaitPhase::Fallen);
        assert_eq!(look.slots[0].balance, again.slots[0].balance);
        assert_eq!(look.slots[1].balance, again.slots[1].balance);
    }

    #[test]
    fn balance_look_standing_keeps_pose() {
        assert_eq!(balance_pelvis_y_m(GaitPhase::Steady, 0.9), 0.9);
        assert_eq!(balance_lean_m(GaitPhase::Steady), 0.0);
        assert_eq!(balance_pelvis_y_m(GaitPhase::Steady, f32::NAN), 0.0);
        let mut look = balance_look_two();
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        assert_eq!(look.slots[0].balance, GaitPhase::Steady);
        assert_eq!(look.slots[0].pos, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn balance_poll_mirrors_authoritative_gaits() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 2,
        };
        let mut live =
            LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
        for _ in 0..10u32 {
            let _ = live.step(&[]);
        }
        let mut look = balance_look_two();
        look.observe_live_balances(&live, 2);
        for index in 0..2usize {
            let slot = u8::try_from(index).expect("slot fits");
            let want = live.balance(slot).expect("slot live").gait();
            assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors sim");
        }
        let mut again = balance_look_two();
        again.observe_live_balances(&live, 2);
        assert_eq!(look.slots[0].balance, again.slots[0].balance);
        assert_eq!(look.slots[1].balance, again.slots[1].balance);
        look.observe_balance(1, GaitPhase::Fallen);
        look.observe_balance(99, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
        look.observe_live_balances(&live, 2);
        let want0 = live.balance(0).expect("slot live").gait();
        assert_eq!(look.slots[0].balance, want0, "steady slot still mirrors");
        assert_eq!(
            look.slots[1].balance,
            GaitPhase::Fallen,
            "fallen never downgrades"
        );
        look.observe_live_balances(&live, 99);
        assert_eq!(look.slots[0].balance, want0, "wide poll keeps the mirror");
    }

    #[test]
    fn balance_look_live_poll_lies_flat() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 2,
        };
        let run = || {
            let mut live =
                LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
            for _ in 0..10u32 {
                let _ = live.step(&[]);
            }
            let mut look = balance_look_two();
            look.observe_live_balances(&live, 2);
            for index in 0..2usize {
                let slot = u8::try_from(index).expect("slot fits");
                let want = live.balance(slot).expect("slot live").gait();
                assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors live");
            }
            assert_eq!(
                balance_pelvis_y_m(GaitPhase::Steady, 0.9),
                0.9,
                "standing keeps pose"
            );
            assert_eq!(balance_lean_m(GaitPhase::Steady), 0.0);
            look.observe_balance(1, GaitPhase::Fallen);
            look.observe_live_balances(&live, 2);
            assert_eq!(
                look.slots[1].balance,
                GaitPhase::Fallen,
                "live poll keeps the floor"
            );
            let floored = balance_pelvis_y_m(GaitPhase::Fallen, 0.9);
            assert!(floored < 0.4, "floored live poll lies flat, got {floored}");
            look.observe_balance(0, GaitPhase::Catching);
            assert_eq!(
                balance_pelvis_y_m(GaitPhase::Catching, 0.9),
                0.45,
                "catching kneels"
            );
            assert_eq!(balance_lean_m(GaitPhase::Catching), 0.0, "catching holds");
            look.observe_balance(99, GaitPhase::Fallen);
            assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
            (look.slots[0].balance, look.slots[1].balance, floored)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "live poll x2 equal");
        assert!(first.2 < 0.4, "fallen root below 0.4 m twice");
    }

    #[test]
    fn app_live_match_feeds_look_mirror() {
        let config = crate::GameConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("session builds");
        for _ in 0..10u32 {
            let _ = app.tick_empty();
        }
        let mut look = balance_look_two();
        look.observe_live_balances(app.live_match(), 2);
        for index in 0..2usize {
            let slot = u8::try_from(index).expect("slot fits");
            let want = app.live_match().balance(slot).expect("slot live").gait();
            assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors app");
        }
        let mut again = balance_look_two();
        again.observe_live_balances(app.live_match(), 2);
        assert_eq!(look.slots[0].balance, again.slots[0].balance);
        assert_eq!(look.slots[1].balance, again.slots[1].balance);
    }

    #[test]
    fn sand_lift_rests_the_lowest_vertex_and_never_pushes_down() {
        // The lift reads the pose-skinned bounds of every act. Above sand and
        // non-finite never lift.
        assert_eq!(sand_lift_m(0.05), 0.0, "above sand never pushes down");
        assert_eq!(
            sand_lift_m(0.0),
            0.0,
            "resting exactly on sand needs no lift"
        );
        assert_eq!(sand_lift_m(f32::NAN), 0.0, "non-finite never lifts");
        assert_eq!(sand_lift_m(f32::INFINITY), 0.0, "infinite never lifts");
        assert_eq!(sand_lift_m(f32::NEG_INFINITY), 0.0, "infinite never lifts");
    }

    #[test]
    fn view_truth_down_lift_rests_the_fallen_start_pose_at_once() {
        // The product floor: the upright start of the death clip with the
        // fallen pelvis sank 56.1 cm; the lift must bring it back to the
        // sand on the very first tick (it was 0 there before, then 20 cm).
        let sunk = -0.561;
        let lift = sand_lift_m(sunk);
        assert!(
            (lift - 0.561).abs() < 1e-6,
            "full depth lifts at once, got {lift}"
        );
        assert!(
            (sunk + lift).abs() * 100.0 < 3.0,
            "the lifted body sits within the 3 cm sunk floor"
        );
    }

    #[test]
    fn product_sunk_56cm_down_rest_reports_green() {
        // Product 05142ee3 frame 169 slot 0 down min -0.561, sunk 56.1 floor 3.
        // The sand rest (LF17, view only) puts the lowest vertex of a dead body
        // on the sand from the first frame of the fall: no ease-in, because a
        // body 56 cm under the sand on its first frames is the bug. Pure
        // positions plus view laws, no window. Caller: `FighterLook::pose_frame`
        // (the live rest reads this shift).
        use crate::view_truth::{BodyTruth, ViewLaws};
        let cam = Camera::new([0.0, 1.5, 6.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0);
        let mut raw = BodyTruth::new(
            0,
            "down",
            [19.185, 0.0, 3.438],
            -2.111,
            vec![[0.0, 1.0, 0.0]],
        );
        raw.fold_vertices(&[[18.672, -0.561, 3.107], [19.493, 1.111, 3.872]]);
        assert!(
            (raw.min[1] + 0.561).abs() < 1e-6,
            "pose min is the sunk depth"
        );
        let mut before = ViewLaws::default();
        before.note(&cam, 169.0, &[raw.clone()]);
        assert!(
            before.line().unwrap().contains("sunk_cm_max=56.1"),
            "unlifted sunk 56.1, got {}",
            before.line().unwrap()
        );
        let shift = sand_rest_shift_m(raw.min[1]);
        assert!(
            (shift - 0.561).abs() < 1e-6,
            "the first frame lifts the full depth, got {shift}"
        );
        assert!(
            (raw.min[1] + shift).abs() * 100.0 < 1.0,
            "the rested body sits within 1 cm of the sand"
        );
        let mut fixed = raw;
        fixed.root[1] += shift;
        fixed.min[1] += shift;
        fixed.max[1] += shift;
        let mut after = ViewLaws::default();
        after.note(&cam, 169.0, &[fixed]);
        assert!(
            after.line().unwrap().contains("sunk_cm_max=0.0"),
            "lifted sunk 0, got {}",
            after.line().unwrap()
        );
    }

    #[test]
    fn sand_rest_shift_lowers_a_hover_and_caps_both_ways() {
        // The settled death pose hovers about 9.5 cm over the sand on the
        // real body; the rest lowers it, and a bad pose never flies or dives.
        assert!((sand_rest_shift_m(0.095) + 0.095).abs() < 1e-6);
        assert!((sand_rest_shift_m(-0.13) - 0.13).abs() < 1e-6);
        assert_eq!(sand_rest_shift_m(0.0), 0.0, "on the sand needs no shift");
        assert_eq!(sand_rest_shift_m(-5.0), SAND_LIFT_MAX_M);
        assert_eq!(sand_rest_shift_m(5.0), -SAND_LIFT_MAX_M);
        assert_eq!(sand_rest_shift_m(f32::NAN), 0.0, "non-finite never shifts");
        assert_eq!(sand_rest_shift_m(f32::INFINITY), 0.0);
        assert_eq!(sand_rest_shift_m(f32::NEG_INFINITY), 0.0);
        assert_eq!(sand_lift_m(0.095), 0.0, "the joints path never lowers");
    }

    #[test]
    fn view_truth_sand_lift_matches_sink_and_caps_at_one_meter() {
        assert!((sand_lift_m(-0.13) - 0.13).abs() < 1e-6);
        assert_eq!(sand_lift_m(-5.0), SAND_LIFT_MAX_M, "a bad pose never flies");
    }

    #[test]
    fn view_truth_down_lift_stable_x2() {
        assert_eq!(sand_lift_m(-0.13), sand_lift_m(-0.13));
        assert_eq!(sand_lift_m(-0.561), sand_lift_m(-0.561));
    }

    #[test]
    fn balance_look_digest_equal_x2() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 10,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 1,
        };
        let run = |with_look: bool| {
            let mut sim = LiveMatch::with_rules(&cfg, ArenaRules::V1).expect("v1 builds");
            let mut look = balance_look_two();
            let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
            let mut hashes = Vec::new();
            for t in 0..10u32 {
                if let Some(tick) = sim.step(&[]) {
                    hashes.push(tick.checkpoint.hash);
                }
                if with_look {
                    look.observe_tick(t, &views, &[1000, 1000], &[]);
                }
            }
            hashes
        };
        assert_eq!(run(false), run(true));
        assert_eq!(run(true), run(true), "same look twice hashes twice");
    }

    #[test]
    fn a_hit_from_across_the_pit_swings_but_never_tints_the_victim() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let far = [view(0, -4.0, 0.0), view(1, 4.0, 0.0)];
        look.observe_tick(0, &far, &[1000, 1000], &[]);
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(20, &far, &[1000, 900], &[hit]);
        assert!(matches!(look.act(0), Some(Act::Attack(_))));
        assert_eq!(look.slots[1].flash_until, 0, "8 m away: no tint");
        assert_eq!(look.tinted(2), (0, 2));
        let near = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(40, &near, &[1000, 800], &[hit]);
        assert_eq!(look.slots[1].flash_until, 40 + FLASH_TICKS);
        assert_eq!(look.tinted(2), (1, 2));
    }

    /// `(slot, part, plain material, drawn material)` for every part of the
    /// first `fighters` slots, in the order `push_draws` emits them, with
    /// all parts resident.
    fn drawn_parts(
        look: &FighterLook,
        fighters: usize,
    ) -> Vec<(usize, usize, PbrMaterial, PbrMaterial)> {
        let mut order = Vec::new();
        for slot in 0..fighters {
            for part in 0..look.rigs[look.rig_for_slot(slot)].parts.len() {
                order.push((slot, part));
            }
        }
        let resident: Vec<MeshKey> = order.iter().map(|&(s, p)| look_mesh_key(s, p)).collect();
        let scene = engine_render::v1::default_scene(64, 64).expect("default scene is valid");
        let mut list = DrawList {
            camera: scene.camera,
            light: scene.light,
            items: Vec::new(),
        };
        let pushed = look.push_draws(&mut list, &resident, fighters);
        assert_eq!(pushed, order.len(), "one draw per resident part");
        order
            .into_iter()
            .zip(list.items)
            .map(|((slot, part), item)| {
                let plain = look.rigs[look.rig_for_slot(slot)].parts[part]
                    .material
                    .clone();
                (slot, part, plain, item.material)
            })
            .collect()
    }

    #[test]
    fn tint_reaches_the_drawn_parts() {
        // The flag is not a receipt: the draw the frame pushes for a flashing
        // victim carries the red-shifted material, and only that victim's.
        assert_eq!(FLASH_TICKS, 15, "a hit reads as long as its tell");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let near = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &near, &[1000, 1000], &[]);
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(40, &near, &[1000, 900], &[hit]);
        let parts = drawn_parts(&look, 2);
        assert!(parts.iter().any(|p| p.0 == 1), "the victim is drawn");
        for (slot, part, plain, drawn) in &parts {
            if *slot == 1 {
                assert_eq!(drawn, &flash_material(plain), "victim part {part}");
                assert!(drawn.exposure > plain.exposure, "brighter");
                assert!(
                    drawn.base_color[1] < plain.base_color[1],
                    "green drops, so the body reads red"
                );
                assert!(drawn.validate().is_ok(), "the presenter accepts it");
            } else {
                assert_eq!(drawn, plain, "the attacker is never tinted");
            }
        }
        // The window is exactly FLASH_TICKS wide: the last flashing tick
        // still draws tinted, the next one draws the plain material.
        look.observe_tick(40 + FLASH_TICKS - 1, &near, &[1000, 900], &[]);
        assert_eq!(look.tinted(2), (1, 2));
        assert!(drawn_parts(&look, 2).iter().any(|p| p.2 != p.3));
        look.observe_tick(40 + FLASH_TICKS, &near, &[1000, 900], &[]);
        assert_eq!(look.tinted(2), (0, 2), "the flash is spent");
        for (slot, part, plain, drawn) in drawn_parts(&look, 2) {
            assert_eq!(plain, drawn, "slot {slot} part {part} plain again");
        }
    }

    #[test]
    fn flash_material_shifts_red_and_stays_valid() {
        for base in [[1.0, 1.0, 1.0], [0.2, 0.5, 0.9], [0.0, 0.0, 0.0]] {
            let plain = PbrMaterial {
                base_color: base,
                metallic: 0.1,
                roughness: 0.6,
                exposure: 1.0,
            };
            let flash = flash_material(&plain);
            assert_eq!(flash.base_color[0], base[0], "red is kept");
            assert!(flash.base_color[1] <= base[1] && flash.base_color[2] <= base[2]);
            assert_eq!(flash.exposure, FLASH_EXPOSURE);
            assert_eq!((flash.metallic, flash.roughness), (0.1, 0.6));
            assert!(flash.validate().is_ok(), "{base:?} stays a valid material");
        }
    }

    /// Tinted body-frames over drawn body-frames for a bout of `fighters`
    /// at `window` ticks per hit, one frame per tick (the arena's tally).
    /// Reads the last in-reach hit tick off `flash_until`, so any window up
    /// to [`FLASH_TICKS`] comes from one run.
    fn bout_tint_share(seed: u64, ticks: u32, fighters: u8, windows: &[u32]) -> Vec<(f64, u32)> {
        let config = crate::GameConfig {
            seed,
            ticks,
            fighters,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("app builds");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut drawn = 0u32;
        let mut tinted = vec![0u32; windows.len()];
        let mut hits = 0u32;
        let mut last_hit = [None::<u32>; MAX_LOOK_FIGHTERS];
        for tick in 0..ticks {
            let outcome = app.tick_empty();
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            look.observe_live_balances(app.live_match(), usize::from(fighters));
            let slots = usize::from(fighters).min(MAX_LOOK_FIGHTERS);
            for (state, last) in look.slots.iter().zip(last_hit.iter_mut()).take(slots) {
                if !state.seen {
                    continue;
                }
                drawn += 1;
                if state.flash_until > 0 {
                    let at = state.flash_until - FLASH_TICKS;
                    if *last != Some(at) {
                        *last = Some(at);
                        hits += 1;
                    }
                }
                for (count, window) in tinted.iter_mut().zip(windows) {
                    *count += u32::from(last.is_some_and(|at| tick < at + window));
                }
            }
        }
        assert!(drawn > 0, "the bout draws bodies");
        tinted
            .into_iter()
            .map(|count| (f64::from(count) / f64::from(drawn), hits))
            .collect()
    }

    #[test]
    fn tint_share_of_a_brawl_stays_under_the_strobe_floor() {
        // The strobe floor (`tools/product-bar.json` `tinted_share_max`,
        // 0.25 of drawn bodies): a readable 15-tick flash must not turn a
        // brawl into a strobe. Same bout, windows of 6 and 15 ticks.
        for fighters in [4u8, 8] {
            for seed in [2040u64, 7] {
                let share = bout_tint_share(seed, 300, fighters, &[6, FLASH_TICKS]);
                let (short, long) = (share[0].0, share[1].0);
                eprintln!(
                    "tint share fighters={fighters} seed={seed} hits={}: 6 ticks {short:.3}, {FLASH_TICKS} ticks {long:.3}",
                    share[0].1
                );
                assert!(long >= short, "a longer flash never tints less");
                assert!(
                    long < 0.25,
                    "{fighters} fighters seed {seed}: share {long:.3} is a strobe"
                );
            }
        }
    }

    /// EG47: one routed skip for every GPU test here. No adapter maps to
    /// `NoAdapter`, so the test prints one `SKIPPED <test>` line and
    /// `E2040_REQUIRE_GPU=1` turns the skip into a failure.
    fn presenter_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(presenter) => Ok(presenter),
            Err(engine_render::present::PresentError::NoAdapter) => Err(
                engine_render::test_gpu::NoAdapter("no wgpu adapter on this machine".into()),
            ),
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        engine_render::test_gpu::require_or_skip(test, probe)
    }

    /// A staged look on its own offscreen presenter: textures plus either
    /// the CPU-skinned or the GPU-skinned part buffers. `None` without an
    /// adapter (the test reports the skip).
    fn staged_look(test: &str, gpu_skin: bool) -> Option<(FighterLook, Presenter)> {
        let mut presenter = presenter_or_skip(test)?;
        presenter.resize(192, 108).expect("valid size applies");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.stage_textures(&mut presenter).expect("textures stage");
        if gpu_skin {
            look.stage_skinned_meshes(&mut presenter)
                .expect("skinned bodies stage");
        } else {
            look.stage_meshes(&mut presenter).expect("bodies stage");
        }
        Some((look, presenter))
    }

    /// One GPU-skinned frame creates no bind group and, the first time a seen
    /// slot's cape pins, only that cape's two cloth buffers (RZ7, cold). Every
    /// other frame and every body part rewrites resident buffers. `cloth`
    /// carries the published count between frames.
    fn assert_frame_creates(
        until: u32,
        before: (u64, u64),
        after: (u64, u64),
        published: u64,
        cloth: &mut u64,
    ) {
        let fresh = published.saturating_sub(*cloth);
        *cloth = published;
        assert_eq!(
            after.1, before.1,
            "a GPU-skinned frame creates no bind group (tick {until})"
        );
        assert_eq!(
            after.0 - before.0,
            2 * fresh,
            "a frame creates only the cloth of a newly pinned cape (tick {until})"
        );
    }

    /// One scripted tick of a four-fighter ring: a hit on slot 1 at tick 20
    /// (flash), slot 2 knocked down at tick 21 and held down after. Call it
    /// for every tick from 0, in order.
    fn brawl_step(look: &mut FighterLook, tick: u32) {
        let ring = [
            view(0, -1.0, 0.0),
            view(1, 1.0, 0.0),
            view(2, 0.0, 1.8),
            view(3, 0.0, -1.8),
        ];
        match tick {
            20 => look.observe_tick(
                tick,
                &ring,
                &[1000, 900, 1000, 1000],
                &[FightCue::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: 100,
                }],
            ),
            21 => {
                look.observe_tick(
                    tick,
                    &ring,
                    &[1000, 900, 0, 1000],
                    &[FightCue::Knockdown {
                        attacker: 3,
                        victim: 2,
                    }],
                );
                // The sim's balance verdict: the victim lies on the sand,
                // the pose whose first frames sink under it.
                look.observe_balance(2, GaitPhase::Fallen);
            }
            t if t > 21 => look.observe_tick(t, &ring, &[1000, 900, 0, 1000], &[]),
            t => look.observe_tick(t, &ring, &[1000; 4], &[]),
        }
    }

    #[test]
    fn skin_window_frame_matches_the_cpu_skinned_frame() {
        let Some((mut cpu_look, mut cpu)) = staged_look("skin window frame (cpu)", false) else {
            return;
        };
        let Some((mut gpu_look, mut gpu)) = staged_look("skin window frame (gpu)", true) else {
            return;
        };
        let frame = LookFrame {
            fighters: 4,
            alpha: 0.5,
            shot: Shot::Action,
            frame: 0,
        };
        let scene = engine_render::v1::default_scene(192, 108).expect("default scene is valid");
        let list = || DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: Vec::new(),
        };
        let parts: usize = (0..4)
            .map(|slot| gpu_look.rigs[gpu_look.rig_for_slot(slot)].parts.len())
            .sum();
        // Ticks that show a flash (24), the deepest sand lift (26) and a body
        // at rest on the sand (75), each compared frame for frame.
        let mut cloth = 0u64;
        for until in 0..=75u32 {
            brawl_step(&mut cpu_look, until);
            brawl_step(&mut gpu_look, until);
            if ![24, 26, 75].contains(&until) {
                continue;
            }
            let (mut cpu_list, mut gpu_list) = (list(), list());
            let cpu_report = cpu_look
                .present_into(&mut cpu, &mut cpu_list, &frame)
                .expect("cpu frame stages");
            let before = gpu.gpu_create_totals();
            let gpu_report = gpu_look
                .present_into(&mut gpu, &mut gpu_list, &frame)
                .expect("gpu frame stages");
            let now = gpu.gpu_create_totals();
            let published = u64::try_from(gpu_look.capes().1).unwrap_or(u64::MAX);
            assert_frame_creates(until, before, now, published, &mut cloth);
            assert_eq!(cpu_report.draws, gpu_report.draws, "same draws");
            assert_eq!(gpu_report.skin.uploaded, parts, "every part is resident");
            assert!(gpu_report.skin.pose_us > 0, "the bounds still pose on CPU");
            // The truth the camera, the view laws and the lift read is the
            // same vertex box either way.
            assert_eq!(cpu_look.view_truth().len(), gpu_look.view_truth().len());
            for (a, b) in cpu_look.view_truth().iter().zip(gpu_look.view_truth()) {
                assert_eq!((a.min, a.max, a.root), (b.min, b.max, b.root));
            }
            if until == 26 {
                assert!(
                    gpu_look.view_truth().iter().any(|body| body.root[1] > 0.02),
                    "the downed body rides a sand lift at tick {until}"
                );
            }
            let cpu_out = cpu.render_offscreen(&cpu_list).expect("cpu renders");
            let gpu_out = gpu.render_offscreen(&gpu_list).expect("gpu renders");
            assert!(cpu_out.receipt.is_clean() && gpu_out.receipt.is_clean());
            let decode = |png: Option<Vec<u8>>| {
                TextureUpload::from_encoded(&png.expect("capture has pixels"))
                    .expect("capture is a PNG")
                    .rgba
            };
            let (a, b) = (decode(cpu_out.png), decode(gpu_out.png));
            assert_eq!(a.len(), b.len());
            let mut worst = 0u8;
            let mut over = 0usize;
            for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
                let d = pa
                    .iter()
                    .zip(pb)
                    .map(|(x, y)| x.abs_diff(*y))
                    .max()
                    .unwrap_or(0);
                worst = worst.max(d);
                over += usize::from(d > 8);
            }
            eprintln!(
                "skin window frame tick {until}: worst channel diff {worst}, pixels over 8: {over} of {}",
                a.len() / 4
            );
            assert!(
                over * 100 <= a.len() / 4,
                "tick {until}: under 1% of pixels differ by more than 8 levels, got {over} (worst {worst})"
            );
        }
    }

    #[test]
    fn tint_reddens_the_victim_the_window_draws() {
        // The hit tint is a picture, not only a flag: the same pose drawn
        // with the flash on is redder on the victim's pixels than with it off.
        let Some((mut look, mut presenter)) = staged_look("tint pixels", true) else {
            return;
        };
        for tick in 0..=24u32 {
            brawl_step(&mut look, tick);
        }
        assert_eq!(look.tinted(4), (1, 4), "slot 1 flashes at tick 24");
        // A bigger frame: four bodies in a mid shot are small, and the
        // flash changes only the victim's pixels.
        presenter.resize(384, 216).expect("valid size applies");
        let scene = engine_render::v1::default_scene(384, 216).expect("default scene is valid");
        let frame = LookFrame {
            fighters: 4,
            alpha: 0.5,
            shot: Shot::Action,
            frame: 0,
        };
        let mut shoot = |look: &mut FighterLook| -> Vec<u8> {
            let mut list = DrawList {
                camera: scene.camera.clone(),
                light: scene.light.clone(),
                items: Vec::new(),
            };
            look.present_into(&mut presenter, &mut list, &frame)
                .expect("frame stages");
            let png = presenter
                .render_offscreen(&list)
                .expect("frame renders")
                .png
                .expect("capture has pixels");
            TextureUpload::from_encoded(&png)
                .expect("capture is a PNG")
                .rgba
        };
        let tinted = shoot(&mut look);
        look.slots[1].flash_until = 0;
        let plain = shoot(&mut look);
        // Redness is red over the mean of green and blue, summed on the
        // pixels the flash changed.
        let (mut changed, mut red_on, mut red_off) = (0u32, 0.0f64, 0.0f64);
        for (a, b) in tinted.chunks_exact(4).zip(plain.chunks_exact(4)) {
            if a[..3] == b[..3] {
                continue;
            }
            changed += 1;
            let redness = |p: &[u8]| f64::from(p[0]) - 0.5 * (f64::from(p[1]) + f64::from(p[2]));
            red_on += redness(a);
            red_off += redness(b);
        }
        eprintln!("tint pixels: {changed} changed, redness {red_off:.0} -> {red_on:.0}");
        assert!(changed > 150, "the victim's body changed, got {changed}");
        assert!(
            red_on > red_off + f64::from(changed),
            "the flash reddens the body: {red_off:.0} -> {red_on:.0} over {changed} pixels"
        );
    }

    #[test]
    fn skin_window_joint_blocks_follow_the_pose_and_the_lift() {
        let Some((mut look, presenter)) = staged_look("skin window blocks", true) else {
            return;
        };
        for slot in 0..MAX_LOOK_FIGHTERS {
            let rig = &look.rigs[look.rig_for_slot(slot)];
            for part in 0..rig.parts.len() {
                assert_eq!(
                    presenter.skin_block_of(&look_mesh_key(slot, part)),
                    Some(slot),
                    "slot {slot} part {part} draws through its own block"
                );
            }
            assert!(slot_is_gpu_skinned(&presenter, slot, rig));
        }
        // The matrices the frame writes carry the lift in their last row.
        // Tick 26 is where the fallen start pose sinks deepest under the
        // sand, so the rest lifts it a long way there.
        for tick in 0..=26u32 {
            brawl_step(&mut look, tick);
        }
        let posed = look.pose_frame(4, 0.5).expect("CPU pose");
        let downed = posed
            .iter()
            .find(|body| body.truth.slot == 2)
            .expect("slot 2 is posed");
        assert!(
            downed.lift > 0.02,
            "the downed body is lifted, got {} m",
            downed.lift
        );
        let upright = posed
            .iter()
            .find(|body| body.truth.slot == 0)
            .expect("slot 0 is posed");
        // Since 01572b00 the sand rest covers every body, lift-only: a
        // standing pose whose foot dips is nudged up to the sand by its own
        // dip and never rides the downed body's lift or gets pushed down.
        assert!(
            upright.lift >= 0.0,
            "an upright lift never pushes down, got {} m",
            upright.lift
        );
        assert!(
            upright.lift <= 0.05,
            "an upright body is nudged by its own dip only, got {} m",
            upright.lift
        );
        assert!(
            upright.lift == 0.0 || upright.truth.min[1].abs() < 1e-6,
            "a lifted upright body rests on the sand: lift {} m, lowest {} m",
            upright.lift,
            upright.truth.min[1]
        );
        let rig = &look.rigs[look.rig_for_slot(2)];
        let slot = look.slots[2];
        let sample = look.slot_sample(rig, 2, &slot, 0.5).expect("sample");
        for (matrix, (joint, inverse)) in downed
            .matrices
            .iter()
            .zip(sample.world.iter().zip(rig.inverse_binds.iter()))
        {
            let base = mul(&mul(inverse, joint), &sample.root);
            assert_eq!(matrix[3][1], base[3][1] + downed.lift);
            assert_eq!(matrix[3][0], base[3][0]);
            assert_eq!(matrix[0], base[0]);
        }
        // A CPU-staged presenter is never taken for a GPU-skinned one.
        let Some((cpu_look, cpu)) = staged_look("skin window blocks (cpu)", false) else {
            return;
        };
        let rig = &cpu_look.rigs[cpu_look.rig_for_slot(0)];
        assert!(!slot_is_gpu_skinned(&cpu, 0, rig));
        assert!(
            presenter.mesh_count() >= MAX_LOOK_FIGHTERS,
            "every slot owns its parts"
        );
    }

    #[test]
    fn fighter_anim_same_tick_gives_same_clip_and_phase() {
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let hp = [1000u16, 1000u16];
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        let mut first = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut second = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        for tick in 0..=10u32 {
            let cues = if tick == 5 { vec![hit] } else { vec![] };
            first.observe_tick(tick, &views, &hp, &cues);
            second.observe_tick(tick, &views, &hp, &cues);
        }
        for alpha in [0.25, 0.5] {
            let rig = &first.rigs[first.rig_for_slot(0)];
            let other = &second.rigs[second.rig_for_slot(0)];
            let a = first
                .slot_pose(rig, &first.slots[0], alpha)
                .expect("pose samples");
            let b = second
                .slot_pose(other, &second.slots[0], alpha)
                .expect("pose samples");
            assert_eq!(a, b);
        }
    }

    #[test]
    fn fighter_anim_bridge_drives_look_pick() {
        use crate::anim_bridge::{BridgeClip, BridgeTuning};
        // Glue tuning builds from the look's speed constants; bad values refuse.
        let tuning = BridgeTuning::new(super::RUN_SPEED_M_PER_TICK, super::RUN_REF_M_PER_TICK)
            .expect("look tuning builds");
        assert!(
            BridgeTuning::new(0.0, super::RUN_REF_M_PER_TICK).is_err(),
            "zero threshold refuses"
        );
        assert!(
            BridgeTuning::new(super::RUN_SPEED_M_PER_TICK, f32::NAN).is_err(),
            "NaN ref refuses"
        );
        let _ = tuning;
        let run = || {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            let hp = [1000u16, 1000u16];
            let hit = FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 100,
            };
            // Tick 0 still: idle on both sides of the wire.
            let still = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(0, &still, &hp, &[]);
            assert_eq!(look.act(0), Some(Act::Idle));
            assert_eq!(look.bridge[0].pick().clip, BridgeClip::Idle);
            assert_eq!(look.slots[0].fade_left, look.bridge[0].fade().fade_left);
            // Steady 0.10 m per tick: run on both sides, fading together.
            for tick in 1..=4u32 {
                let x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
                let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
                look.observe_tick(tick, &views, &hp, &[]);
            }
            assert_eq!(look.act(0), Some(Act::Run));
            assert_eq!(look.bridge[0].pick().clip, BridgeClip::Run);
            assert!(
                look.bridge[0].fade().blending(),
                "run switch restarts the fade"
            );
            assert_eq!(
                look.slots[0].fade_left,
                look.bridge[0].fade().fade_left,
                "look reads the bridge fade"
            );
            assert_eq!(
                usize::from(look.bridge[0].fade().prev_clip),
                look.slots[0].prev_clip,
                "leaving clip rides the wire"
            );
            // A Hit starts the attack variant on both sides; the look keeps
            // the landed impact backdate while the clip matches.
            let held = [view(0, -0.6, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(5, &held, &hp, &[hit]);
            assert!(matches!(look.act(0), Some(Act::Attack(0))));
            assert_eq!(look.bridge[0].pick().clip, BridgeClip::Attack0);
            assert_eq!(
                look.slots[0].fade_left,
                look.bridge[0].fade().fade_left,
                "attack restarts the fade through the wire"
            );
            // Zero hp floors both sides.
            look.observe_tick(6, &held, &[1000, 0], &[]);
            assert_eq!(look.act(1), Some(Act::Down));
            assert_eq!(look.bridge[1].pick().clip, BridgeClip::Down);
            (look.act(0), look.bridge[0].pick(), look.slots[0].fade_left)
        };
        assert_eq!(
            run(),
            run(),
            "same cues give the same bridge-driven pick twice"
        );
    }

    #[test]
    fn fighter_anim_bridge_owns_look_act_reads_bridge() {
        use crate::anim_bridge::BridgeClip;
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let hp = [1000u16, 1000u16];
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        // Idle on a still tick.
        let still = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &still, &hp, &[]);
        assert_eq!(look.act(0), Some(Act::Idle));
        assert_eq!(look.bridge[0].pick().clip, BridgeClip::Idle);
        // Steady motion runs.
        for tick in 1..=4u32 {
            let x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(tick, &views, &hp, &[]);
        }
        assert_eq!(look.act(0), Some(Act::Run));
        assert_eq!(look.bridge[0].pick().clip, BridgeClip::Run);
        // One attack.
        let held = [view(0, -0.6, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(5, &held, &hp, &[hit]);
        assert!(matches!(look.act(0), Some(Act::Attack(0))));
        assert_eq!(look.bridge[0].pick().clip, BridgeClip::Attack0);
        // A diverged mirror still reports the bridge pick.
        look.slots[0].act = Act::Down;
        assert_eq!(
            look.act(0),
            Some(Act::Attack(0)),
            "act reads the bridge, not the mirror"
        );
        // An unseen slot stays silent even though its bridge idles.
        assert_eq!(look.act(2), None);
        assert_eq!(look.bridge[2].pick().clip, BridgeClip::Idle);
    }

    #[test]
    fn fighter_anim_switch_midpoint_carries_the_offset() {
        // LF9: an idle to run switch cuts to the run clip on the switch tick
        // and carries the old pose as a decaying offset. The switch frame
        // shows the old pose, mid-decay differs from both clips, and past the
        // settle age the pose is the plain run clip, bit for bit.
        let hp = [1000u16, 1000u16];
        let mut calm = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut mover = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let joints_of = |look: &FighterLook, plain: bool| -> Vec<[f32; 3]> {
            let rig = &look.rigs[look.rig_for_slot(0)];
            if plain {
                let pose = look
                    .slot_pose(rig, &look.slots[0], 0.0)
                    .expect("plain pose");
                let world = skinning_matrices(&rig.skeleton, &pose).expect("plain joints");
                return world.iter().map(|m| [m[3][0], m[3][1], m[3][2]]).collect();
            }
            look.pose_frame_joints(2, 0.0)
                .expect("joints pose")
                .into_iter()
                .find(|body| body.slot == 0)
                .expect("slot 0 is posed")
                .joints
        };
        let worst = |a: &[[f32; 3]], b: &[[f32; 3]]| -> f32 {
            a.iter()
                .zip(b)
                .map(|(p, q)| {
                    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
                })
                .fold(0.0f32, f32::max)
        };
        let step = |calm: &mut FighterLook, mover: &mut FighterLook, tick: u32| {
            let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            calm.observe_tick(tick, &[view(0, -1.0, 0.0), view(1, 1.0, 0.0)], &hp, &[]);
            mover.observe_tick(tick, &[view(0, x, 0.0), view(1, 1.0, 0.0)], &hp, &[]);
        };
        step(&mut calm, &mut mover, 0);
        step(&mut calm, &mut mover, 1);
        assert_eq!(mover.slots[0].act, Act::Run, "steady motion runs");
        assert_eq!(calm.slots[0].act, Act::Idle, "a still fighter idles");
        assert_eq!(mover.slots[0].iz_age, 0, "the switch tick starts the clock");
        let at_switch = worst(&joints_of(&mover, false), &joints_of(&calm, false));
        assert!(
            at_switch < 1e-4,
            "switch frame is the old pose, off {at_switch} m"
        );
        let offset = worst(&joints_of(&mover, false), &joints_of(&mover, true));
        assert!(
            offset > 0.05,
            "the old pose sits {offset} m from the run clip"
        );
        for tick in 2..=5u32 {
            step(&mut calm, &mut mover, tick);
        }
        assert_eq!(
            mover.slots[0].fade_left, 5,
            "the bridge fade still marches beside the offsets"
        );
        let mid = joints_of(&mover, false);
        assert!(
            worst(&mid, &joints_of(&mover, true)) > 0.01,
            "four ticks in, the offset is still carried"
        );
        assert!(
            worst(&mid, &joints_of(&calm, false)) > 0.01,
            "four ticks in, the pose has left the idle clip"
        );
        let settle = fighter_look_inertia::SETTLE_TICKS;
        for tick in 6..=(1 + 2 * settle) {
            step(&mut calm, &mut mover, tick);
            if tick == 1 + settle / 2 {
                let left = worst(&joints_of(&mover, false), &joints_of(&mover, true));
                assert!(
                    left < offset * 0.1,
                    "half the window on, {left} of {offset} m left"
                );
            }
        }
        let rig = &mover.rigs[mover.rig_for_slot(0)];
        let slot = mover.slots[0];
        assert_eq!(
            mover.inertial_pose(rig, 0, &slot, 0.5).expect("shown pose"),
            mover.slot_pose(rig, &slot, 0.5).expect("clip pose"),
            "past the settle age the shown pose is the clip pose, bit for bit"
        );
    }

    #[test]
    fn fighter_anim_crossfade_replays_x2() {
        let run = || {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            let hp = [1000u16, 1000u16];
            let mut poses = Vec::new();
            for tick in 0..=10u32 {
                let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
                let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
                look.observe_tick(tick, &views, &hp, &[]);
                let rig = &look.rigs[look.rig_for_slot(0)];
                let slot = look.slots[0];
                poses.push(look.inertial_pose(rig, 0, &slot, 0.5).expect("pose"));
                poses.push(look.inertial_pose(rig, 0, &slot, 0.0).expect("pose"));
            }
            poses
        };
        assert_eq!(run(), run(), "same ticks give same blend twice");
    }

    #[test]
    fn fighter_anim_run_phase_scales_with_speed() {
        let mut slow = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut fast = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        for tick in 0..=20u32 {
            let slow_x = -1.0 + 0.025 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let fast_x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let slow_views = [view(0, slow_x, 0.0), view(1, 5.0, 0.0)];
            let fast_views = [view(0, fast_x, 0.0), view(1, 5.0, 0.0)];
            slow.observe_tick(tick, &slow_views, &[1000, 1000], &[]);
            fast.observe_tick(tick, &fast_views, &[1000, 1000], &[]);
        }
        assert_eq!(slow.act(0), Some(Act::Run));
        assert_eq!(fast.act(0), Some(Act::Run));
        let slow_slot = slow.slots[0];
        let fast_slot = fast.slots[0];
        assert!((slow_slot.run_speed - 0.025).abs() < 1e-6);
        assert!((fast_slot.run_speed - 0.10).abs() < 1e-6);
        let slow_rig = &slow.rigs[slow.rig_for_slot(0)];
        let fast_rig = &fast.rigs[fast.rig_for_slot(0)];
        let (_, slow_t) = slow.clip_time(slow_rig, &slow_slot, 0.5);
        let (_, fast_t) = fast.clip_time(fast_rig, &fast_slot, 0.5);
        assert!(
            fast_t > slow_t,
            "fast {fast_t} must lead slow {slow_t} at the same tick"
        );
        let (_, slow_again) = slow.clip_time(slow_rig, &slow_slot, 0.5);
        let (_, fast_again) = fast.clip_time(fast_rig, &fast_slot, 0.5);
        assert_eq!(slow_t, slow_again);
        assert_eq!(fast_t, fast_again);
    }

    #[test]
    fn run_phase_marches_only_in_the_run_act_and_survives_bad_speeds() {
        let tick_s = 1.0 / TICK_HZ;
        let marched = |act: Act, speed: f32, ticks: u32| {
            let mut slot = SlotAnim::new();
            slot.act = act;
            slot.run_speed = speed;
            slot.march_run_phase(ticks);
            slot.run_phase_s
        };
        let close = |got: f32, want: f32| (got - want).abs() < 1.0e-6;
        let at_ref = marched(Act::Run, RUN_REF_M_PER_TICK, 6);
        assert!(close(at_ref, 6.0 * tick_s), "authored rate: {at_ref} s");
        assert!(close(marched(Act::Run, 0.0, 6), 3.0 * tick_s), "half rate");
        assert!(
            close(marched(Act::Run, 1.0, 6), 12.0 * tick_s),
            "double rate"
        );
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let held = marched(Act::Run, bad, 6);
            assert!(
                close(held, 6.0 * tick_s),
                "speed {bad} holds the authored rate: {held}"
            );
        }
        assert_eq!(
            marched(Act::Run, RUN_REF_M_PER_TICK, 0),
            0.0,
            "no ticks, no phase"
        );
        for act in [Act::Idle, Act::Attack(1), Act::Down] {
            assert_eq!(
                marched(act, RUN_REF_M_PER_TICK, 6),
                0.0,
                "{act:?} carries no run phase"
            );
        }
        let mut run = SlotAnim::new();
        run.act = Act::Run;
        assert_eq!(run.run_clip_s(-3.0), 0.0, "a look back never goes negative");
    }

    #[test]
    fn fighter_anim_hit_react_flinches_and_decays() {
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let hp = [1000u16, 1000u16];
        let mut hit = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut calm = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        for tick in 0..=19u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        // Same ticks give the same pose before any hit.
        for alpha in [0.0, 0.5] {
            let slot_h = hit.slots[1];
            let rig_h = &hit.rigs[hit.rig_for_slot(1)];
            let pose_h = hit.slot_pose(rig_h, &slot_h, alpha).expect("pose samples");
            let slot_c = calm.slots[1];
            let rig_c = &calm.rigs[calm.rig_for_slot(1)];
            let pose_c = calm.slot_pose(rig_c, &slot_c, alpha).expect("pose samples");
            assert_eq!(pose_h, pose_c, "pre-hit poses replay at alpha {alpha}");
        }
        let cue = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        // Same hitstop hold on both sides (LF10): `Parried` freezes the same
        // impact pair without a flinch, so past-window poses compare hold to
        // hold and only the flinch can differ; its ramp peaks one tick in.
        let calm_hold = FightCue::Parried {
            attacker: 0,
            victim: 1,
        };
        hit.observe_tick(20, &views, &hp, &[cue]);
        calm.observe_tick(20, &views, &hp, &[calm_hold]);
        // Hitstop freezes the flinch four ticks; sample its full lean after
        // the three-tick rise.
        for tick in 21..=27u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        let slot_h = hit.slots[1];
        let rig_h = &hit.rigs[hit.rig_for_slot(1)];
        let flinch = hit.slot_pose(rig_h, &slot_h, 0.5).expect("pose samples");
        let slot_c = calm.slots[1];
        let rig_c = &calm.rigs[calm.rig_for_slot(1)];
        let base = calm.slot_pose(rig_c, &slot_c, 0.5).expect("pose samples");
        let pelvis = rig_h.pelvis;
        let dx = flinch.positions[pelvis][0] - base.positions[pelvis][0];
        let dz = flinch.positions[pelvis][2] - base.positions[pelvis][2];
        let dy = flinch.positions[pelvis][1] - base.positions[pelvis][1];
        assert!(
            dx * dx + dz * dz > 1e-10,
            "a Hit pushes the victim pelvis off its base: dx={dx} dz={dz}"
        );
        assert!(dy < 0.0, "a Hit dips the victim pelvis: dy={dy}");
        // The push points away from the attacker (victim at +X of attacker).
        assert!(dx > 0.0, "flinch runs away from the attacker: dx={dx}");
        // AM23: the rotation half leans the real skeleton and the head reads
        // at least 8 cm of travel, the metric `react_head_cm_max` prints.
        let head_m = hit.react_head_m(rig_h, &slot_h, 0.5);
        assert!(head_m >= 0.08, "rotation flinch head travel: {head_m}");
        assert_ne!(
            flinch.rotations[rig_h.torso], base.rotations[rig_c.torso],
            "the torso leans off its base"
        );
        // Mid-window the flinch has decayed but still reads.
        for tick in 28..=40u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        let slot_h = hit.slots[1];
        let rig_h = &hit.rigs[hit.rig_for_slot(1)];
        let mid = hit.slot_pose(rig_h, &slot_h, 0.5).expect("pose samples");
        let slot_c = calm.slots[1];
        let rig_c = &calm.rigs[calm.rig_for_slot(1)];
        let base_mid = calm.slot_pose(rig_c, &slot_c, 0.5).expect("pose samples");
        let mdx = mid.positions[pelvis][0] - base_mid.positions[pelvis][0];
        let mdz = mid.positions[pelvis][2] - base_mid.positions[pelvis][2];
        assert!(
            mdx * mdx + mdz * mdz < dx * dx + dz * dz,
            "the flinch decays inside the window: mid {mdx},{mdz} vs hit {dx},{dz}"
        );
        assert!(
            mdx * mdx + mdz * mdz > 1e-12,
            "the flinch still reads mid-window"
        );
        // Past the 450 ms window the base returns exactly.
        for tick in 41..=51u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        let slot_h = hit.slots[1];
        let rig_h = &hit.rigs[hit.rig_for_slot(1)];
        let past = hit.slot_pose(rig_h, &slot_h, 0.5).expect("pose samples");
        let slot_c = calm.slots[1];
        let rig_c = &calm.rigs[calm.rig_for_slot(1)];
        let base_past = calm.slot_pose(rig_c, &slot_c, 0.5).expect("pose samples");
        // Balance may still read stumbling, but the flinch offset is gone:
        // both sides froze the same hitstop hold (Hit vs Parried), and
        // stumble never moves `slot_pose`, so poses match exactly.
        assert_eq!(past, base_past, "past the window the flinch is silent");
        // Same Hit ticks give the same flinch twice.
        let run = || {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            for tick in 0..=20u32 {
                look.observe_tick(tick, &views, &hp, &[]);
            }
            look.observe_tick(21, &views, &hp, &[cue]);
            let slot = look.slots[1];
            let rig = &look.rigs[look.rig_for_slot(1)];
            look.slot_pose(rig, &slot, 0.5).expect("pose samples")
        };
        assert_eq!(run(), run(), "same Hit tick gives the same flinch twice");
    }

    #[test]
    fn staged_bodies_rewrite_their_buffers_in_place_every_frame() {
        let Some(mut presenter) =
            presenter_or_skip("staged_bodies_rewrite_their_buffers_in_place_every_frame")
        else {
            return;
        };
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let staged = look.stage_meshes(&mut presenter).expect("bodies stage");
        assert!(staged >= MAX_LOOK_FIGHTERS, "every slot owns its parts");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        // RZ7 capes: the first stage publishes each seen fighter's cloth
        // mesh (a vertex and an index buffer), cold. That is the only
        // creation a stage may make; every later one rewrites in place.
        look.stage_bodies(&mut presenter, 2, 0.5)
            .expect("warm frame stages");
        let created = presenter.gpu_create_totals();
        // A moved pose is what re-sends a part (an upload digest skips only
        // the parts whose bytes are already resident), so every step here
        // steps the tick and both roots: each staged frame is a changed one.
        let mut truth = Vec::new();
        for step in 1..=5u8 {
            let moved = [
                view(0, -1.0 + f32::from(step) * 0.1, 0.0),
                view(1, 1.0 - f32::from(step) * 0.1, 0.0),
            ];
            look.observe_tick(u32::from(step), &moved, &[1000, 1000], &[]);
            let (split, bodies) = look
                .stage_bodies(&mut presenter, 2, 0.5)
                .expect("frame stages");
            assert!(split.uploaded >= 2, "both fighters write their parts");
            assert!(split.upload_us > 0, "a moved pose runs the upload");
            truth = bodies;
        }
        // The full 48 B image (pos + normal + uv + tangent, tangents rebuilt
        // from the posed vertices) rewrites each resident buffer in place: a
        // changed frame writes every part and creates no GPU buffer, and the
        // 32 B refresh that misaligned past vertex 0 (1209Z streaks) is not
        // what runs.
        assert_eq!(
            presenter.gpu_create_totals(),
            created,
            "steady frames rewrite the staged buffers, never create new ones"
        );
        // No streak: every posed vertex stays within 5 m of its slot root
        // (a stretched triangle would read as tens of meters).
        assert_eq!(truth.len(), 2, "both seen fighters report a body");
        for body in &truth {
            let dx = body.max[0] - body.min[0];
            let dy = body.max[1] - body.min[1];
            let dz = body.max[2] - body.min[2];
            assert!(
                dx < 5.0 && dy < 5.0 && dz < 5.0,
                "fighter bounds stay compact, got [{dx:.2}, {dy:.2}, {dz:.2}]"
            );
        }
        // A fighter that shows up later already owns its body buffers; only
        // its cloth is published now, cold.
        let three = [view(0, -1.0, 0.0), view(1, 1.0, 0.0), view(2, 0.0, 3.0)];
        look.observe_tick(6, &three, &[1000, 1000, 1000], &[]);
        let cloth_before = look.capes().1;
        let (split, _) = look
            .stage_bodies(&mut presenter, 3, 0.5)
            .expect("late fighter stages");
        assert!(split.upload_us > 0, "the late fighter writes its parts");
        let cloth = u64::try_from(look.capes().1.saturating_sub(cloth_before)).unwrap_or(u64::MAX);
        let late = presenter.gpu_create_totals();
        assert_eq!(
            late,
            (created.0 + 2 * cloth, created.1),
            "the late slot's body rewrites resident buffers; only its cloth is new"
        );
        // The frame after that rewrites the new cloth in place too.
        look.observe_tick(7, &three, &[1000; 3], &[]);
        look.stage_bodies(&mut presenter, 3, 0.5)
            .expect("steady frame stages");
        assert_eq!(
            presenter.gpu_create_totals(),
            late,
            "steady frames after a late slot never create"
        );
    }

    /// What the presenter was handed before [`staged_part_mesh`]: the caller
    /// validated and rebuilt the tangents too, then the presenter did the same
    /// again. Reference arm only; returns the microseconds the pair took.
    fn upload_parts_validated_first(
        presenter: &mut Presenter,
        slot_index: usize,
        rig: &Rig,
        frames: Vec<engine_render::v1_skin::SkinnedFrame>,
        lift: f32,
    ) -> u64 {
        let mut spent = 0u64;
        for (part_index, posed) in frames.into_iter().enumerate() {
            let part = &rig.parts[part_index];
            let started = Instant::now();
            let lifted: Vec<[f32; 3]> = if lift != 0.0 {
                posed
                    .positions
                    .iter()
                    .map(|p| [p[0], p[1] + lift, p[2]])
                    .collect()
            } else {
                posed.positions
            };
            let mesh = v1::upload_mesh(&lifted, &posed.normals, &part.mesh.uvs, &part.mesh.indices)
                .expect("posed part validates");
            presenter
                .upload_mesh(look_mesh_key(slot_index, part_index), &mesh)
                .expect("posed part uploads");
            spent = spent.saturating_add(micros(started.elapsed()));
        }
        spent
    }

    #[test]
    fn skinned_upload_draws_the_same_pixels_without_the_caller_side_pass() {
        // The staged value carries no tangents: the presenter must rebuild
        // them itself, so the picture must equal the one the validated pair
        // drew, and both must differ from the bind pose they replaced.
        let Some(mut presenter) =
            presenter_or_skip("skinned_upload_draws_the_same_pixels_without_the_caller_side_pass")
        else {
            return;
        };
        presenter.resize(160, 120).expect("valid size applies");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.stage_textures(&mut presenter).expect("textures stage");
        look.stage_meshes(&mut presenter).expect("bodies stage");
        look.observe_tick(0, &[view(0, -1.0, 0.0), view(1, 1.0, 0.0)], &[1000; 2], &[]);
        look.observe_tick(1, &[view(0, -0.8, 0.0), view(1, 0.8, 0.0)], &[1000; 2], &[]);
        let posed = look.pose_frame(2, 0.5).expect("CPU pose");
        assert_eq!(posed.len(), 2, "both seen slots pose");
        let scene = engine_render::v1::default_scene(160, 120).expect("default scene is valid");
        let draw = |look: &FighterLook, presenter: &mut Presenter| -> Vec<u8> {
            let mut list = DrawList {
                camera: scene.camera.clone(),
                light: scene.light.clone(),
                items: Vec::new(),
            };
            let resident = presenter.mesh_keys();
            assert!(look.push_draws(&mut list, &resident, 2) > 0, "bodies draw");
            list.camera = look.fight_camera(&list.camera, Shot::Action, 2, 0);
            presenter
                .render_offscreen(&list)
                .expect("offscreen renders")
                .png
                .expect("a live frame has pixels")
        };
        let bind = draw(&look, &mut presenter);
        let mut digests = empty_upload_digest();
        for body in &posed {
            let slot = usize::from(body.truth.slot);
            let rig = &look.rigs[look.rig_for_slot(slot)];
            upload_slot_parts(
                &mut presenter,
                slot,
                rig,
                body.frames.clone(),
                body.lift,
                &mut digests,
            )
            .expect("staged parts upload");
        }
        let staged = draw(&look, &mut presenter);
        look.stage_meshes(&mut presenter)
            .expect("bind pose returns");
        assert_eq!(draw(&look, &mut presenter), bind, "the bind pose is back");
        for body in &posed {
            let slot = usize::from(body.truth.slot);
            let rig = &look.rigs[look.rig_for_slot(slot)];
            upload_parts_validated_first(&mut presenter, slot, rig, body.frames.clone(), body.lift);
        }
        let validated = draw(&look, &mut presenter);
        assert_ne!(staged, bind, "the posed upload changes the picture");
        assert!(staged == validated, "same posed bytes reach the submit");
    }

    #[test]
    fn skinned_upload_spends_less_than_the_validated_first_pair() {
        // F2P: with `v1::upload_mesh` run in the caller as well, the staged
        // arm repeats the validate plus tangent pass the presenter already
        // owns, so its best round costs the same as the reference arm (7589
        // us vs 7482 us measured) and the 92% bound below fails. With the cut
        // the best round costs 55-80% of the reference on a busy machine.
        let Some(mut presenter) =
            presenter_or_skip("skinned_upload_spends_less_than_the_validated_first_pair")
        else {
            return;
        };
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.stage_meshes(&mut presenter).expect("bodies stage");
        let views: Vec<FighterView> = (0..8u8)
            .map(|slot| view(slot, -3.5 + f32::from(slot), 0.0))
            .collect();
        look.observe_tick(0, &views, &[1000; 8], &[]);
        let posed = look.pose_frame(8, 0.5).expect("CPU pose");
        assert_eq!(posed.len(), 8, "eight seen slots pose");
        let (mut reference_best, mut staged_best) = (u64::MAX, u64::MAX);
        for round in 0..30u32 {
            let reference_arm = |presenter: &mut Presenter| -> u64 {
                let mut spent = 0u64;
                for body in &posed {
                    let slot = usize::from(body.truth.slot);
                    let rig = &look.rigs[look.rig_for_slot(slot)];
                    spent = spent.saturating_add(upload_parts_validated_first(
                        presenter,
                        slot,
                        rig,
                        body.frames.clone(),
                        body.lift,
                    ));
                }
                spent
            };
            let staged_arm = |presenter: &mut Presenter| -> u64 {
                let mut spent = 0u64;
                for body in &posed {
                    let slot = usize::from(body.truth.slot);
                    let rig = &look.rigs[look.rig_for_slot(slot)];
                    let mut digests = empty_upload_digest();
                    let split = upload_slot_parts(
                        presenter,
                        slot,
                        rig,
                        body.frames.clone(),
                        body.lift,
                        &mut digests,
                    )
                    .expect("staged parts upload");
                    spent = spent.saturating_add(split.upload_us);
                }
                spent
            };
            // Alternate who goes first so neither arm owns the warm cache.
            let (reference, staged) = if round % 2 == 0 {
                let reference = reference_arm(&mut presenter);
                (reference, staged_arm(&mut presenter))
            } else {
                let staged = staged_arm(&mut presenter);
                (reference_arm(&mut presenter), staged)
            };
            if round > 0 {
                reference_best = reference_best.min(reference);
                staged_best = staged_best.min(staged);
            }
        }
        eprintln!("staged upload best {staged_best} us vs validated pair best {reference_best} us");
        assert!(
            staged_best * 25 <= reference_best * 23,
            "the staged value skips the caller-side pass: {staged_best} us vs {reference_best} us"
        );
    }

    #[test]
    fn skinned_upload_skips_resident_parts_and_counts_both_halves() {
        // F2P: without the upload digest every part re-runs the
        // `v1::upload_mesh`/`Presenter::upload_mesh` pair on every present, so
        // `second.skin.upload_us` is non-zero and the assertion below fails.
        // The two halves must both count on a staged rig, must sit inside the
        // wall they are cut from, and the half the cut owns must drop on an
        // identical second call while `uploaded` still reports the parts the
        // frame needs - and a moved pose must re-upload whole, never leaving
        // the resident bytes of a previous tick on screen.
        let Some(mut presenter) =
            presenter_or_skip("skinned_upload_skips_resident_parts_and_counts_both_halves")
        else {
            return;
        };
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.stage_meshes(&mut presenter).expect("bodies stage");
        let views = [view(0, -1.0, 0.0)];
        look.observe_tick(0, &views, &[1000], &[]);
        let scene = engine_render::v1::default_scene(64, 64).expect("default scene is valid");
        let list = || DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: Vec::new(),
        };
        let frame = LookFrame {
            fighters: 1,
            alpha: 0.5,
            shot: Shot::Action,
            frame: 0,
        };
        let parts = look.rigs[look.rig_for_slot(0)].parts.len();
        // Warm the skin, the tangent rebuild and the allocator off the clock.
        let warm = LookFrame {
            alpha: 0.25,
            ..frame
        };
        look.present_into(&mut presenter, &mut list(), &warm)
            .expect("warm present");
        // One tick later the fighter stands a step on: the changed placed
        // vertices must re-upload whole, and the identical repeat must not.
        look.observe_tick(1, &[view(0, -0.8, 0.0)], &[1000], &[]);
        let wall = Instant::now();
        let first = look
            .present_into(&mut presenter, &mut list(), &frame)
            .expect("first present");
        let wall_ns = wall.elapsed().as_nanos();
        let second = look
            .present_into(&mut presenter, &mut list(), &frame)
            .expect("repeat present");
        assert!(
            first.skin.pose_us > 0 && first.skin.upload_us > 0,
            "both halves count on a staged rig, got {:?}",
            first.skin
        );
        let split_ns =
            u128::from(first.skin.pose_us) * 1_000 + u128::from(first.skin.upload_us) * 1_000;
        assert!(
            split_ns <= wall_ns,
            "the split never exceeds the wall it was cut from: {split_ns} ns vs {wall_ns} ns"
        );
        assert_eq!(
            first.skin.uploaded, parts,
            "one staged fighter writes its parts"
        );
        assert_eq!(
            second.skin.uploaded, parts,
            "the repeated frame still reports the parts it needs"
        );
        // The cut: an unchanged digest runs no `upload_mesh` pair, so the
        // second identical call adds zero upload time while the first frame
        // of a moved pose still uploads whole.
        assert_eq!(
            second.skin.upload_us, 0,
            "an unchanged digest uploads nothing, got {:?}",
            second.skin
        );
        // A skipped part is never a stale one: the next tick moves the body,
        // so its parts leave the resident digest and the pair runs again.
        look.observe_tick(2, &[view(0, -0.6, 0.0)], &[1000], &[]);
        let moved = look
            .present_into(&mut presenter, &mut list(), &frame)
            .expect("moved present");
        assert!(
            moved.skin.upload_us > 0 && moved.skin.uploaded == parts,
            "a moved pose re-uploads its parts, got {:?}",
            moved.skin
        );
    }

    #[test]
    fn gpu_skin_probe_runs_twice_with_same_answer() {
        // Behavior plus headless failure path: on a machine without a GPU
        // the probe reads as `None` (skip, never fail); with a GPU it reads
        // as `Some` adapter name. Either way it never panics and repeats.
        let first = super::gpu_skin_adapter();
        let second = super::gpu_skin_adapter();
        assert_eq!(first, second, "probe is stable across calls");
    }

    #[test]
    fn gpu_skin_probe_leaves_cpu_skin_vectors_unchanged() {
        // The live window entry probes the GPU path while the CPU skin stays
        // the single render path: the skinned vertex digest below must read
        // the same before and after the probe.
        use sha2::{Digest, Sha256};
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let rig = &look.rigs[0];
        let bind = engine_anim::contract::LocalPose {
            positions: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].positions.tracks[j].values[0])
                .collect(),
            rotations: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].rotations[j].values[0])
                .collect(),
        };
        let world = skinning_matrices(&rig.skeleton, &bind).expect("pose");
        let root = root_matrix([0.0, 0.0, 0.0], 0.0);
        let matrices: Vec<[[f32; 4]; 4]> = world
            .iter()
            .zip(rig.inverse_binds.iter())
            .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
            .collect();
        let digest_of = |matrices: &[[[f32; 4]; 4]]| {
            let uniform = rig.pipe.skinning_uniform(matrices).expect("uniform");
            let part = &rig.parts[0];
            let posed = rig
                .pipe
                .render_skinned(&part.mesh, Some(&part.skin), &uniform)
                .expect("skin");
            let mut hasher = Sha256::new();
            for p in &posed.positions {
                for c in p {
                    hasher.update(c.to_le_bytes());
                }
            }
            hasher.finalize().to_vec()
        };
        let before = digest_of(&matrices);
        let _ = super::gpu_skin_adapter();
        let after = digest_of(&matrices);
        assert_eq!(before, after, "probe must not move the skin vectors");
    }

    #[test]
    fn shake_trauma_hit_adds_and_settles_within_one_second() {
        // View-only shake: an in-reach Hit adds 400 permille, then the
        // 1000-per-second decay clears it within 60 ticks (1.0 s at 60 Hz).
        let mut look = balance_look_two();
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        assert_eq!(look.shake_trauma_permille, 0, "no impacts yet: no shake");
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(10, &views, &[1000, 900], &[hit]);
        assert_eq!(
            look.shake_trauma_permille, SHAKE_HIT_PERMILLE,
            "one in-reach Hit adds 400 permille"
        );
        // The fight camera carries the shake: same offset `shake_offset`
        // reports for this trauma plus tick, on both eye and target.
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let shaken = look.fight_camera(&base, Shot::Action, 2, 0);
        let expected = crate::camera::shake_offset(SHAKE_HIT_PERMILLE, look.tick, SHAKE_MAX_M);
        assert_ne!(
            (expected.x, expected.y, expected.z),
            (0.0, 0.0, 0.0),
            "400 permille at tick 10 shakes (non-zero offset)"
        );
        // A calm twin (same slots, same tick, no Hit) frames the same shot
        // without the offset: both eye and target move by `expected`.
        let mut calm = balance_look_two();
        calm.observe_tick(0, &views, &[1000, 1000], &[]);
        calm.observe_tick(10, &views, &[1000, 900], &[]);
        let unshaken = calm.fight_camera(&base, Shot::Action, 2, 0);
        let eye_delta = shaken.eye - unshaken.eye;
        let target_delta = shaken.target - unshaken.target;
        for (got, want, axis) in [
            (eye_delta.x, expected.x, "eye x"),
            (eye_delta.y, expected.y, "eye y"),
            (eye_delta.z, expected.z, "eye z"),
            (target_delta.x, expected.x, "target x"),
            (target_delta.y, expected.y, "target y"),
            (target_delta.z, expected.z, "target z"),
        ] {
            assert!(
                (got - want).abs() < 1e-6,
                "{axis} carries the shake offset: got {got}, want {want}"
            );
        }
        for t in 11..=70u32 {
            look.observe_tick(t, &views, &[1000, 900], &[]);
        }
        assert_eq!(
            look.shake_trauma_permille, 0,
            "400 permille decays to 0 within 60 ticks (1.0 s)"
        );
        let rest = crate::camera::shake_offset(0, look.tick, SHAKE_MAX_M);
        assert_eq!((rest.x, rest.y, rest.z), (0.0, 0.0, 0.0));
        // Settled: the shaken shot matches the calm shot exactly.
        let settled = look.fight_camera(&base, Shot::Action, 2, 0);
        let mut calm_late = balance_look_two();
        for t in 0..=70u32 {
            calm_late.observe_tick(t, &views, &[1000, 900], &[]);
        }
        let calm_cam = calm_late.fight_camera(&base, Shot::Action, 2, 0);
        assert_eq!(settled.eye, calm_cam.eye, "settled eye matches calm");
        assert_eq!(
            settled.target, calm_cam.target,
            "settled target matches calm"
        );
    }

    #[test]
    fn shake_trauma_knockdown_clamps_and_repeats() {
        // Two in-reach Knockdowns (700 + 700) clamp at 1000, and the same
        // cue script plus the same ticks give the same camera twice.
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let kd = FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        };
        let mut first = balance_look_two();
        let mut second = balance_look_two();
        for look in [&mut first, &mut second] {
            look.observe_tick(0, &views, &[1000, 1000], &[]);
            look.observe_tick(10, &views, &[1000, 0], &[kd]);
            look.observe_tick(11, &views, &[1000, 0], &[kd]);
        }
        assert_eq!(
            first.shake_trauma_permille, SHAKE_MAX_PERMILLE,
            "700 + 700 clamps at 1000, got {}",
            first.shake_trauma_permille
        );
        assert_eq!(first.shake_trauma_permille, second.shake_trauma_permille);
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let a = first.fight_camera(&base, Shot::Action, 2, 7);
        let b = second.fight_camera(&base, Shot::Action, 2, 7);
        assert_eq!(a.eye, b.eye, "same ticks give same shaken eye");
        assert_eq!(a.target, b.target, "same ticks give same shaken target");
    }

    #[test]
    fn shake_trauma_out_of_reach_and_non_impacts_never_shake() {
        // Failure paths: a Hit from 8 m away never shakes (same gate as the
        // hit tint), and Blocked/Parried/Dodged/StaminaOut never shake even
        // in reach.
        let mut look = balance_look_two();
        let far = [view(0, -4.0, 0.0), view(1, 4.0, 0.0)];
        look.observe_tick(0, &far, &[1000, 1000], &[]);
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(20, &far, &[1000, 900], &[hit]);
        assert_eq!(
            look.shake_trauma_permille, 0,
            "8 m away: no shake (out of reach)"
        );
        let near = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(40, &near, &[1000, 1000], &[]);
        for cue in [
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 10,
            },
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
            FightCue::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightCue::StaminaOut { fighter: 0 },
        ] {
            look.observe_tick(41, &near, &[1000, 1000], &[cue]);
            assert_eq!(
                look.shake_trauma_permille, 0,
                "non-impact {cue:?} never shakes"
            );
        }
    }

    /// True when `inner` sits inside `outer` grown by `pad` on every axis.
    /// A non-finite face reads as outside (loud, never a silent pass).
    fn bounds_hold(
        outer_min: [f32; 3],
        outer_max: [f32; 3],
        inner_min: [f32; 3],
        inner_max: [f32; 3],
        pad: f32,
    ) -> bool {
        outer_min
            .iter()
            .zip(inner_min.iter())
            .all(|(outer, inner)| *outer - pad <= *inner)
            && inner_max
                .iter()
                .zip(outer_max.iter())
                .all(|(inner, outer)| *inner <= *outer + pad)
    }

    /// One synthetic skinned frame over `positions`: the fold input of
    /// [`posed_bound_m`].
    fn bound_frame(positions: &[[f32; 3]]) -> engine_render::v1_skin::SkinnedFrame {
        engine_render::v1_skin::SkinnedFrame {
            positions: positions.to_vec(),
            normals: vec![[0.0, 1.0, 0.0]; positions.len()],
            tangents: vec![engine_render::v1_skin::DUMMY_TANGENT; positions.len()],
            used_error_material: false,
            diagnostics: Vec::new(),
        }
    }

    /// One synthetic posed body over `positions`, with no bound computed yet.
    fn bound_body(positions: &[[f32; 3]]) -> PosedBody {
        PosedBody {
            truth: BodyTruth::new(0, "idle", [0.0; 3], 0.0, Vec::new()),
            frames: vec![bound_frame(positions)],
            matrices: Vec::new(),
            lift: 0.0,
            bound: None,
        }
    }

    /// Gap between two world points in meters.
    fn gap_m(a: [f32; 3], b: [f32; 3]) -> f32 {
        let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    #[test]
    fn posed_bound_follows_a_posed_arm_outside_the_bind_sphere() {
        // Accept case: the bind sphere is 0.5 m around the trunk and the
        // swing holds the arm 1.5 m out past it. A bind-pose sphere culls
        // that arm; the posed bound holds it.
        let bind_radius_m = 0.5f32;
        let (trunk, arm) = ([0.0, 0.9, 0.0], [1.5, 1.2, 0.0]);
        let body = bound_body(&[[-0.3, 0.0, -0.3], [0.3, 1.8, 0.3], arm]);
        let (centre, radius) = posed_bound_m(&body).expect("finite vertices bind");
        assert!(
            gap_m(centre, arm) <= radius + 1e-6,
            "the posed arm stays inside the bound, got {} over {radius}",
            gap_m(centre, arm)
        );
        assert!(
            gap_m(trunk, arm) > bind_radius_m,
            "the arm leaves the sphere"
        );
        assert!(radius >= bind_radius_m, "the bound clears the sphere");
    }

    #[test]
    fn posed_bound_replays_same_bytes_twice() {
        let body = bound_body(&[[-0.3, 0.0, -0.3], [1.5, 1.2, 0.0], [0.3, 1.8, 0.3]]);
        let first = posed_bound_m(&body).expect("finite vertices bind");
        let second = posed_bound_m(&body).expect("finite vertices bind again");
        assert_eq!(
            (first.0.map(f32::to_bits), first.1.to_bits()),
            (second.0.map(f32::to_bits), second.1.to_bits()),
            "the same posed vertices give the same bound bytes twice"
        );
    }

    #[test]
    fn posed_bound_refuses_a_nan_vertex() {
        // A NaN vertex never widens the bound and never panics; a body whose
        // every vertex is non-finite has no bound at all.
        assert_eq!(
            posed_bound_m(&bound_body(&[[f32::NAN, 0.0, 0.0]])),
            None,
            "no finite vertex, no bound"
        );
        let mixed = bound_body(&[[0.0, 0.0, 0.0], [f32::NAN, 90.0, 0.0], [1.0, 2.0, 0.0]]);
        let (centre, radius) = posed_bound_m(&mixed).expect("finite vertices bind");
        assert_eq!(
            centre,
            [0.5, 1.0, 0.0],
            "the NaN vertex never moves the centre"
        );
        assert!(
            centre.iter().all(|v| v.is_finite()) && radius.is_finite(),
            "the bound stays finite"
        );
        assert!(
            radius <= gap_m(centre, [1.0, 2.0, 0.0]) + 1e-6,
            "the NaN vertex never widens the bound, got {radius}"
        );
    }

    #[test]
    fn posed_bound_never_shrinks_below_the_bind_radius() {
        // The caller floors the posed radius at the rig's bind radius
        // (`Rig::radius_m` = `skin_radius_m` of the bind body, floor 0.5 m),
        // so a bound never shrinks under today's value.
        let bind_radius_m = 0.5f32;
        let tiny = bound_body(&[[-0.01, 0.0, -0.01], [0.01, 0.04, 0.01]]);
        let (_, radius) = posed_bound_m(&tiny).expect("finite vertices bind");
        assert!(radius < bind_radius_m, "a tiny pose sits inside the sphere");
        assert_eq!(radius.max(bind_radius_m), bind_radius_m, "the floor holds");
        // End to end: every slot a real posed frame carries sits at or above
        // its rig's bind radius, never under it.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        let posed = look.pose_frame(2, 0.5).expect("CPU pose");
        assert_eq!(posed.len(), 2, "both seen slots pose");
        for body in &posed {
            let rig = &look.rigs[look.rig_for_slot(usize::from(body.truth.slot))];
            let (centre, radius) = body.bound.expect("a posed body carries a bound");
            assert!(centre.iter().all(|v| v.is_finite()), "centre stays finite");
            assert!(
                radius >= rig.radius_m,
                "slot {} bound {radius} under the bind radius {}",
                body.truth.slot,
                rig.radius_m
            );
        }
    }

    /// Rotate `v` by the unit quaternion `q`; the test's own arm vector.
    fn rotate_by(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
        let p = engine_core::quat_mul(
            engine_core::quat_mul(q, [v[0], v[1], v[2], 0.0]),
            [-q[0], -q[1], -q[2], q[3]],
        );
        [p[0], p[1], p[2]]
    }

    fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    #[test]
    fn ang_limit_look_output_takes_ten_ticks_for_180_deg() {
        // AM24: a 180 deg step on one bone, driven through the look's own
        // limiter tail. `advance_limit` is what `observe_tick` calls;
        // `limit_pose` is what `slot_sample` reads on every frame. A half
        // metre arm turns the cap into metres: 18 deg a tick is a 0.156 m
        // chord, so the arm covers the arc in ten chords, nine leave it short.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let joints = look.rigs[0].skeleton.len();
        let identity = vec![[0.0f32, 0.0, 0.0, 1.0]; joints];
        let mut wanted = identity.clone();
        wanted[0] = [0.0, 1.0, 0.0, 0.0];
        let arm = [0.0, 0.0, 0.5];
        assert_eq!(
            look.limit[0].limiter.limit_deg_per_sec(),
            1080.0,
            "the look runs the 180 deg in 10 ticks cap by default"
        );
        // First sight lands whole: the seed spends a full second of budget.
        look.advance_limit(0, &identity, 1.0 / TICK_HZ);
        let mut rots = identity.clone();
        look.limit_pose(0, &mut rots, 1.0, true);
        assert_eq!(rots, identity, "a first pose is shown whole, no ramp");
        let mut tip = rotate_by(rots[0], arm);
        for tick in 1..=9u32 {
            look.advance_limit(0, &wanted, 1.0 / TICK_HZ);
            let mut rots = identity.clone();
            look.limit_pose(0, &mut rots, 1.0, true);
            let next = rotate_by(rots[0], arm);
            let chord = distance(tip, next);
            assert!(
                (0.150..=0.161).contains(&chord),
                "tick {tick} turned {chord} m at 0.5 m, want 18 deg (0.156 m)"
            );
            tip = next;
        }
        assert!(
            distance(tip, [0.0, 0.0, -0.5]) > 0.15,
            "nine ticks are not a 180 deg step"
        );
        look.advance_limit(0, &wanted, 1.0 / TICK_HZ);
        let mut rots = identity.clone();
        look.limit_pose(0, &mut rots, 1.0, true);
        let miss = distance(rotate_by(rots[0], arm), [0.0, 0.0, -0.5]);
        assert!(
            miss <= 0.005,
            "ten ticks land the 180 deg step, {miss} m off"
        );
        // Halfway through a tick the frame reads the blend, not the end: turn
        // the lane back toward identity and read the half tick.
        look.advance_limit(0, &identity, 1.0 / TICK_HZ);
        let to = look.limit[0].to[0];
        let mut half = identity.clone();
        look.limit_pose(0, &mut half, 0.5, true);
        assert_ne!(half[0], to, "the sub-tick frame stays inside the tick");
        assert_ne!(
            half[0],
            [0.0, 1.0, 0.0, 0.0],
            "the sub-tick frame has left the tick start"
        );
    }

    #[test]
    fn view_run_2040_600_has_no_joint_pops() {
        // AM24 accept: the last-stage cap takes the seeded 600-tick line from
        // 31 joint pops (worst 12.1 cm, all in the run clip) to none, with the
        // sand and skin counters unchanged.
        let line = FighterLook::view_run(2040, 600, 2).expect("headless view runs");
        assert!(
            line.contains(" nonfinite=0 error_parts=0 sunk_cm_max=0.0 "),
            "{line}"
        );
        assert!(line.contains(" pops=0 pop_acts=none "), "{line}");
    }

    #[test]
    fn view_run_2040_300_eight_fighters_has_no_joint_pops() {
        // AM24 accept: the flinch rises over three ticks, so the seeded
        // 8-fighter 300-tick line reads no joint pops (was 2 at 13.1 cm,
        // both on `index_lower_r` under `act=attack`).
        let line = FighterLook::view_run(2040, 300, 8).expect("headless view runs");
        println!("{line}");
        assert!(line.contains(" pops=0 pop_acts=none "), "{line}");
    }

    #[test]
    fn view_run_repeats_its_arena_view_line_x2_with_no_gpu() {
        // SP61 accept: a seeded 600-tick 8-fighter run prints the same
        // `arena view:` line twice. No `Presenter` is built anywhere here,
        // so the load plus observe plus CPU pose path runs with no GPU.
        let first = FighterLook::view_run(2040, 600, 8).expect("headless view runs");
        let second = FighterLook::view_run(2040, 600, 8).expect("headless view repeats");
        println!("{first}");
        println!("{second}");
        assert!(!first.is_empty(), "600 ticks fold frames");
        assert!(
            first.starts_with("arena view:"),
            "receipt line, got {first}"
        );
        assert_eq!(first, second, "same seed gives the same view line twice");
    }

    #[test]
    fn view_run_seed_repeats_and_joints_hold_bounds() {
        // Headless run with no `Presenter`: the same seed prints the same
        // `arena view:` line twice, joints-only holds the vertex bounds
        // within 10 cm, and joints-only clears 2000 frames/s.
        let first = FighterLook::view_run(2040, 60, 2).expect("headless view runs");
        let second = FighterLook::view_run(2040, 60, 2).expect("headless view repeats");
        assert!(!first.is_empty(), "ticks fold frames");
        assert!(
            first.starts_with("arena view:"),
            "receipt line, got {first}"
        );
        assert_eq!(first, second, "same seed repeats bit for bit");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let config = crate::GameConfig {
            seed: 99,
            ticks: 20,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("bout builds");
        for tick in 0..20u32 {
            let outcome = app.tick_empty();
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            let full = look.pose_frame(2, 0.5).expect("CPU pose");
            let joints = look.pose_frame_joints(2, 0.5).expect("joints pose");
            assert_eq!(full.len(), joints.len(), "same slots both ways");
            for (posed, joint) in full.iter().map(PosedBody::truth).zip(&joints) {
                assert!(posed.has_bounds() && joint.has_bounds(), "both modes bound");
                assert!(
                    bounds_hold(joint.min, joint.max, posed.min, posed.max, 0.10),
                    "joints hold vertex within 10 cm"
                );
            }
        }
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(100, &views, &[1000, 1000], &[]);
        for _ in 0..20u32 {
            look.pose_frame_joints(2, 0.5).expect("warm joints pose");
        }
        let frames = 200u32;
        let start = std::time::Instant::now();
        for _ in 0..frames {
            look.pose_frame_joints(2, 0.5).expect("timed joints pose");
        }
        let fps = f64::from(frames) / start.elapsed().as_secs_f64();
        assert!(fps >= 2000.0, "joints-only hits 2000 frames/s");
    }

    #[test]
    fn view_run_joints_bounds_hold_vertex_bounds_within_10cm() {
        // SP61 accept: joints-only bounds (joints padded by the per-rig
        // radius, no vertex skin) hold the full-vertex bounds within 10 cm,
        // over a real bout (idle, run, attack and down acts), and repeat.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let config = crate::GameConfig {
            seed: 99,
            ticks: 120,
            fighters: 4,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("bout builds");
        for tick in 0..120u32 {
            let outcome = app.tick_empty();
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            let full = look.pose_frame(4, 0.5).expect("CPU pose");
            let joints = look.pose_frame_joints(4, 0.5).expect("joints pose");
            let again = look.pose_frame_joints(4, 0.5).expect("joints pose repeats");
            assert_eq!(joints, again, "same tick gives same joints truth");
            assert_eq!(
                full.len(),
                joints.len(),
                "same slots pose both ways at tick {tick}"
            );
            for (posed, joint) in full.iter().map(PosedBody::truth).zip(&joints) {
                assert_eq!(posed.slot, joint.slot, "slot order matches");
                assert_eq!(posed.joints, joint.joints, "same clip sample feeds both");
                assert_eq!(posed.clip, joint.clip, "same clip feeds both");
                assert_eq!(posed.clip_s, joint.clip_s, "same clip time feeds both");
                assert!(
                    posed.has_bounds() && joint.has_bounds(),
                    "both modes bound tick {tick} slot {}",
                    posed.slot
                );
                assert!(
                    bounds_hold(joint.min, joint.max, posed.min, posed.max, 0.10),
                    "joints bounds hold vertex bounds within 10 cm at tick {tick} slot {}: joints {:?}..{:?} vs vertex {:?}..{:?}",
                    posed.slot,
                    joint.min,
                    joint.max,
                    posed.min,
                    posed.max,
                );
            }
        }
    }

    #[test]
    fn view_run_joints_only_hits_2000_frames_per_second() {
        // SP61 accept: joints-only poses at least 2000 frames/s for 8
        // fighters on the reference CPU. Bounds come from joints, never the
        // vertex skin, so the frame cost stays far under the skin cost.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views: Vec<FighterView> = (0..8u8)
            .map(|slot| view(slot, f32::from(slot) - 3.5, 0.0))
            .collect();
        look.observe_tick(0, &views, &[1000u16; 8], &[]);
        // Warm the pose (clip tables, allocator) off the clock.
        for _ in 0..20u32 {
            assert_eq!(
                look.pose_frame_joints(8, 0.5)
                    .expect("warm joints pose")
                    .len(),
                8,
                "every seen slot poses"
            );
        }
        let frames = 400u32;
        let mut posed = 0usize;
        let start = std::time::Instant::now();
        for _ in 0..frames {
            posed += look
                .pose_frame_joints(8, 0.5)
                .expect("timed joints pose")
                .len();
        }
        let fps = f64::from(frames) / start.elapsed().as_secs_f64();
        println!("joints-only 8-fighter pose: {fps:.0} frames/s over {frames} frames");
        assert_eq!(posed, 8 * usize::try_from(frames).unwrap_or(0));
        assert!(
            fps >= 2000.0,
            "joints-only hits 2000 frames/s, got {fps:.0}"
        );
    }

    #[test]
    fn view_run_joints_share_the_vertex_root_placement() {
        // Render-math checklist #1: the AN1 row-vector root maps body +Z
        // onto +X at 90 deg yaw, then moves to the root position. The
        // joints-only bounds ride this same placement as the vertex path.
        let root = root_matrix([1.0, 2.0, 3.0], std::f32::consts::FRAC_PI_2);
        let at = place_joint([0.0, 0.0, 1.0], &root);
        assert!(
            (at[0] - 2.0).abs() < 1e-6 && (at[1] - 2.0).abs() < 1e-6 && (at[2] - 3.0).abs() < 1e-6,
            "row-vector root placement, got {at:?}"
        );
    }

    #[test]
    fn trail_feed_stages_12_frames_oldest_first_and_clears_idle() {
        use crate::weapon_trail::{alpha_for_age, TRAIL_LEN};
        use engine_audio::FightCue;
        // F2P: an attacking slot staged 12 frames holds the last-12 ribbon
        // oldest first; today only `start_swing` pushes once, never ages or
        // clears, so this fails with one unaged pair.
        let mut look = balance_look_two();
        look.swing_ticks = 30;
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        for tick in 0..12u32 {
            let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
            let cues = if tick == 0 { vec![hit] } else { vec![] };
            look.observe_tick(tick, &views, &[1000, 1000], &cues);
        }
        let held = look.trail_pairs(0);
        assert_eq!(held.len(), TRAIL_LEN, "12 staged frames hold 12 pairs");
        assert!(
            held[0].base_m[0] < held[TRAIL_LEN - 1].base_m[0],
            "oldest first with rising x: {:?} vs {:?}",
            held[0].base_m,
            held[TRAIL_LEN - 1].base_m
        );
        let mut ages: Vec<u8> = held.iter().map(|p| p.age_ticks).collect();
        ages.sort_unstable();
        let expect: Vec<u8> = (0..12u8).collect();
        assert_eq!(ages, expect, "ribbon holds ages 0..=11");
        assert_eq!(held[TRAIL_LEN - 1].age_ticks, 0, "newest is age 0");
        assert_eq!(held[0].age_ticks, 11, "oldest is age 11");
        assert!(
            alpha_for_age(0) > alpha_for_age(5) && alpha_for_age(5) > alpha_for_age(11),
            "alpha falls with age"
        );
        assert!(look.trail_pairs(1).is_empty(), "idle slot holds none");
        // Leaving Attack clears the ribbon.
        for tick in 12..60u32 {
            let views = [view(0, -0.4, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(tick, &views, &[1000, 1000], &[]);
        }
        assert!(
            look.trail_pairs(0).is_empty(),
            "leaving Attack clears the trail"
        );
    }

    #[test]
    fn trail_strip_folds_the_swing_and_draws_none_when_idle() {
        // LF14: the strip the draw reads. A calm look stages no vert and no
        // strip count, so the frame issues no draw; a mid-swing look stages
        // one strip for the attacker only, two verts a pair, fading by age.
        use engine_audio::FightCue;
        let mut look = balance_look_two();
        look.swing_ticks = 30;
        let (verts, lens) = look.trail_strip();
        assert!(verts.is_empty(), "no swing stages no vert");
        assert!(lens.is_empty(), "no swing stages no strip");
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        for tick in 0..6u32 {
            let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
            let cues = if tick == 0 { vec![hit] } else { vec![] };
            look.observe_tick(tick, &views, &[1000, 1000], &cues);
        }
        let pairs = look.trail_pairs(0).len();
        assert!(pairs > 1, "the swing ring filled");
        let (verts, lens) = look.trail_strip();
        assert_eq!(lens.len(), 1, "one strip, the attacker's only");
        assert_eq!(
            u32::try_from(verts.len()).unwrap_or(0),
            lens[0],
            "the count is the flat list's length"
        );
        assert_eq!(verts.len(), pairs * 2, "one base plus one tip a pair");
        assert!(
            verts[0].alpha < verts[verts.len() - 1].alpha,
            "oldest first, so alpha rises toward the newest pair"
        );
        assert!(
            verts.iter().all(|vert| vert.alpha.is_finite()),
            "the strip carries finite alpha"
        );
    }

    #[test]
    fn view_run_pose_without_rigs_fails_loud_never_panics() {
        // Failure paths: a seen slot with no rig fails (never a panic), an
        // unseen bout poses nothing, and zero ticks folds no frame.
        let look = balance_look_two();
        assert!(
            look.pose_frame(2, 0.0).is_err(),
            "seen slot with no rig fails"
        );
        assert!(
            look.pose_frame_joints(2, 0.0).is_err(),
            "joints mode fails too"
        );
        let empty = FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
            inertia: Vec::new(),
            limit: Vec::new(),
            ragdoll: RagdollLook::empty(),
            limbs: LimbLook::empty(),
            capes: Capes::empty(),
            hero: 0,
            checkpoints: empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
        };
        assert_eq!(
            empty
                .pose_frame(8, 0.0)
                .expect("unseen poses nothing")
                .len(),
            0
        );
        assert_eq!(
            empty
                .pose_frame_joints(8, 0.0)
                .expect("unseen joints pose nothing")
                .len(),
            0
        );
        assert_eq!(
            FighterLook::view_run(2040, 0, 8).expect("zero ticks"),
            "",
            "no frame folded, no line"
        );
    }

    #[test]
    fn skinned_upload_keeps_48b_stride_with_finite_tangents() {
        // 45f7596e streak: `refresh_mesh_vertices` packs 32 B per vertex
        // (pos + normal + uv) into a 48 B buffer (pos + normal + uv +
        // tangent), so every vertex past the first lands misaligned and each
        // fighter triangle stretches across the sand. The frame path must
        // re-upload the full mesh so `vertex_bytes` stays 48 B per vertex
        // with finite tangents. [found2 | 7.6.2 Rendering with Normal Maps]
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        let posed = look.pose_frame(2, 0.5).expect("CPU pose");
        assert_eq!(posed.len(), 2, "both seen slots pose");
        for body in &posed {
            let slot = usize::from(body.truth.slot);
            let rig = &look.rigs[look.rig_for_slot(slot)];
            assert!(body.lift.is_finite(), "lift stays finite");
            for (part_index, frame) in body.frames.iter().enumerate() {
                let part = &rig.parts[part_index];
                assert_eq!(
                    frame.positions.len(),
                    part.mesh.positions.len(),
                    "skin keeps vertex count"
                );
                let lifted: Vec<[f32; 3]> = if body.lift > 0.0 {
                    frame
                        .positions
                        .iter()
                        .map(|p| [p[0], p[1] + body.lift, p[2]])
                        .collect()
                } else {
                    frame.positions.clone()
                };
                assert!(
                    lifted.iter().flatten().all(|c| c.is_finite()),
                    "lifted positions stay finite"
                );
                let mesh =
                    v1::upload_mesh(&lifted, &frame.normals, &part.mesh.uvs, &part.mesh.indices)
                        .expect("skinned part re-uploads");
                assert_eq!(
                    mesh.tangents.len(),
                    lifted.len(),
                    "tangents ride every vertex"
                );
                assert!(
                    mesh.tangents.iter().flatten().all(|c| c.is_finite()),
                    "tangents stay finite"
                );
                assert_eq!(
                    mesh.vertex_bytes().len(),
                    lifted.len() * 48,
                    "48 B stride holds every vertex"
                );
                let refresh_len = lifted.len() * 32;
                assert_ne!(
                    refresh_len,
                    mesh.vertex_bytes().len(),
                    "32 B refresh would misalign past vertex 0"
                );
            }
        }
    }

    /// Two views: slot 0 at `z` with the rival straight ahead at +Z 40 m, so
    /// the fighter faces the way it runs and slot 1 stands still.
    fn cape_views(z: f32) -> [FighterView; 2] {
        [view(0, 0.0, z), view(1, 0.0, 40.0)]
    }

    /// World point `p` in a slot's own frame (the yaw turn undone, the root at
    /// the origin): the cape trails behind when this z goes negative.
    fn cape_body_local(p: [f32; 3], pos: [f32; 3], yaw: f32) -> [f32; 3] {
        let d = sub3(p, pos);
        let (s, c) = yaw.sin_cos();
        [d[0] * c - d[2] * s, d[1], d[0] * s + d[2] * c]
    }

    /// The body-frame hem of a slot's cape.
    fn cape_hem_local(look: &FighterLook, slot: usize) -> [f32; 3] {
        let sheet = look
            .cape_positions(slot)
            .unwrap_or_else(|| panic!("slot {slot} wears a cape"));
        let at = look.slots[slot];
        cape_body_local(sheet[sheet.len() - 1], at.pos, at.yaw)
    }

    #[test]
    fn capes_pin_to_the_shoulders_and_trail_a_running_fighter() {
        // RZ7 accept: a cape is pinned at each fighter's shoulders, hangs behind
        // the body and swings while the fighter runs. Presentation only: nothing
        // here reads or writes sim state.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let grid = CAPE_TIER.knobs().cape;
        let hp = [1000u16; 2];
        // Stand still two seconds: the sheet settles at rest and sleeps.
        for tick in 0..120u32 {
            look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
        }
        assert_eq!(look.capes().0, 2, "one cape per seen fighter");
        let sheet = look.capes.sheets[0].as_ref().expect("slot 0 cape");
        assert_eq!(sheet.pinned_count(), grid.cols, "the top row stays pinned");
        assert!(
            sheet.positions().iter().flatten().all(|v| v.is_finite()),
            "the resting cape is finite"
        );
        let hem = sheet.positions()[sheet.particle_count() - 1];
        let top_mid = sheet.positions()[grid.cols / 2];
        assert!(
            top_mid[1] - hem[1] > 0.5,
            "the hem hangs under the shoulder row: {top_mid:?} to {hem:?}"
        );
        let rest = cape_hem_local(&look, 0);
        assert!(rest[1] < 0.8, "the hem hangs low in the body: {rest:?}");
        assert!(rest[2] < -0.1, "the cape hangs behind the body: {rest:?}");
        // Run 3 m/s along +Z for 90 ticks (1.5 s, 4.5 m).
        let mut z = 0.0f32;
        for tick in 120..210u32 {
            look.observe_tick(tick, &cape_views(z), &hp, &[]);
            z += 3.0 / TICK_HZ;
        }
        let sheet = look.capes.sheets[0].as_ref().expect("slot 0 cape");
        let hem = sheet.positions()[sheet.particle_count() - 1];
        let top_mid = sheet.positions()[grid.cols / 2];
        let running = cape_hem_local(&look, 0);
        assert!(
            running.iter().all(|v| v.is_finite()),
            "the running cape is finite: {running:?}"
        );
        assert!(
            top_mid[1] - hem[1] > 0.4,
            "the hem keeps hanging while the fighter runs: {top_mid:?} to {hem:?}"
        );
        assert!(
            running[2] < -0.3,
            "the cape trails the running body: {running:?}"
        );
        let d = sub3(running, rest);
        let moved = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        println!("cape: rest hem {rest:?}, running hem {running:?}, moved {moved:.3} m");
        assert!(
            moved > 0.15,
            "the cape moves with the run: rest {rest:?}, running {running:?}"
        );
    }

    /// Distance in meters from `p` to the segment `a`-`b`.
    fn segment_distance(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
        let (ab, ap) = (sub3(b, a), sub3(p, a));
        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        let t = if len2 > 0.0 {
            ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = sub3(ap, [ab[0] * t, ab[1] * t, ab[2] * t]);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    #[test]
    fn a_rival_pressed_behind_pushes_the_cape_instead_of_passing_through() {
        // Collision by default: a cape meets every body near it, not only its
        // wearer's torso. The rival stands 0.35 m behind slot 0, so its torso
        // shell fills the space the cape hangs in. After one second no free
        // cape particle sits more than 2 cm inside that shell; with only the
        // wearer's torso as a collider the sheet hung straight through it.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, 0.0, 0.0), view(1, 0.0, -0.35)];
        let hp = [1000u16; 2];
        for tick in 0..60u32 {
            look.observe_tick(tick, &views, &hp, &[]);
        }
        let anim = look.slots[1];
        let rig_index = look.rig_for_slot(1);
        let joints = look.capes.joints[rig_index].expect("CT7 has cape joints");
        let sample = look
            .slot_sample(&look.rigs[rig_index], 1, &anim, 1.0)
            .expect("slot 1 poses");
        let at = |joint: usize| joint_world(&sample.world, &sample.root, joint);
        let (a, b) = (at(joints.torso[1]), at(joints.torso[0]));
        let sheet = look.capes.sheets[0].as_ref().expect("slot 0 cape");
        let deepest = sheet
            .positions()
            .iter()
            .enumerate()
            .filter(|(i, _)| !sheet.is_pinned(*i))
            .map(|(_, p)| CAPE_BODY_RADIUS_M - segment_distance(*p, a, b))
            .fold(f32::MIN, f32::max);
        println!("cape vs rival torso: deepest particle {deepest:.3} m inside the shell");
        assert!(
            deepest < 0.02,
            "the cape passes through the rival: {deepest:.3} m deep"
        );
        assert!(
            look.capes.near.len() > 2 * joints.limbs().len(),
            "the cape stepped against both bodies, limbs included: {}",
            look.capes.near.len()
        );
    }

    #[test]
    fn cape_mesh_covers_the_grid_with_unit_normals_and_unit_uvs() {
        // The drawn sheet: one vertex per particle, two triangles per cell,
        // outward unit normals and unit uvs. The renderer revalidates it.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.observe_tick(0, &cape_views(0.0), &[1000, 1000], &[]);
        let grid = CAPE_TIER.knobs().cape;
        let sheet = look.capes.sheets[0].as_ref().expect("slot 0 cape");
        let mesh = cape_mesh(sheet, grid, look.capes.outward[0]);
        assert_eq!(mesh.positions.len(), grid.particles());
        assert_eq!(mesh.normals.len(), grid.particles());
        assert_eq!(mesh.indices.len(), (grid.cols - 1) * (grid.rows - 1) * 6);
        assert_eq!(mesh.uvs.len(), grid.particles());
        for n in &mesh.normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "unit normal, got {n:?}");
        }
        assert!(mesh.uvs.iter().flatten().all(|u| (0.0..=1.0).contains(u)));
        v1::upload_mesh(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.indices)
            .expect("the cape mesh validates");
    }

    #[test]
    fn capes_never_reach_the_authority_checkpoints() {
        // P2P: the cape layer is presentation only. Two identical 8-fighter
        // bouts, one with the capes stepped and one with the cape joints
        // cleared, take the same checkpoint bytes: the cloth is never hashed.
        let run = |capes: bool| {
            let config = crate::GameConfig {
                seed: 2040,
                ticks: 180,
                fighters: 8,
                half_extent_m: 20.0,
            };
            let mut app =
                crate::App::with_world(config, crate::arena_kit::kit_match_world_or_empty())
                    .and_then(crate::App::into_duel)
                    .expect("arena session builds");
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            if !capes {
                look.capes.joints.clear();
            }
            for tick in 0..180u32 {
                let out = app.tick_empty();
                look.observe_tick(tick, app.fighter_views(), app.hp(), &out.cues);
            }
            (app.checkpoints().to_vec(), look.capes().0)
        };
        let (with_capes, stepped) = run(true);
        let (without, none) = run(false);
        assert!(!with_capes.is_empty(), "the bout took checkpoints");
        assert_eq!(stepped, 8, "eight capes step in a full bout");
        assert_eq!(none, 0, "no cape joints, no cape");
        assert_eq!(with_capes, without, "the cape layer never moves the sim");
    }
}
