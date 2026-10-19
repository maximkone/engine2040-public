//! limb_look: limbs that come off, drawn from the authority cut event (RZ3).
//!
//! The authority names the severed joint in a 16-byte [`Cut`]. From that tick this
//! layer hands one limb to the float physical layer (`engine-physfx`) as its own
//! rigid body with the fighter's velocity, draws a closed stump cap on the
//! remaining body, and stamps the cut's blood decal and spray. The body keeps
//! animating: the cut never edits the pose, so the joint still plays its clip and
//! only the skin rows of the chain that left are collapsed onto the stump. The cut
//! event stays the truth; physics only paints the fallen limb.
//!
//! Consumer: `FighterLook::observe_cuts` (from the window and the tests) queues the
//! cuts, `FighterLook::limb_tick` steps the world once per accepted tick, and the
//! frame reads [`LimbCapture`] for the drawn limb and cap poses. Data owner: the
//! authority owns which joint was cut (integers, hashed); this module owns only the
//! picture. Units: meters, seconds, Y up, `[x, y, z, w]` unit quaternions. The gore
//! setting is presentation only: `Off` draws no limb and no stump, `Reduced` draws
//! the limb, the stump and the blood decal, `Full` adds the spray. None of it is
//! hashed, so one match gives one golden at every setting.
//! Errors: a refused spawn (`FxError`) leaves the joint attached and is counted in
//! [`FighterLook::limb_refused`]; a bad cut event is refused the same way.
//! Budget: no allocation before the first cut; then one body per cut under the
//! tier's body cap, one `PhysFx::step` per accepted tick, and a remapped skin only
//! on slots that lost a limb.

use engine_combat::gore::GoreSetting;
use engine_combat::{Cut, JointId, JOINT_TOTAL};
use engine_physfx::{BoneShape, Error as FxError, Key, Material, PhysFx, Tier, Transform};
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter};
use engine_render::stump::{place_mesh, stump_cap, StumpCap};
use engine_render::v1::{MeshData, PbrMaterial};
use engine_render::v1_skin::{SkinWeights, SkinnedFrame, SkinningUniform};

use super::{mul, FighterLook, LookError, Rig, RigPart, MAX_LOOK_FIGHTERS, TICK_HZ};

/// Tier the limb world starts on: the comfortable desktop tier. Lower it on weak
/// hardware with [`FighterLook::set_limb_tier`]; the body cap follows the tier.
pub const LIMB_TIER: Tier = Tier::Comfort;
/// Half-width of the sand slab the limbs land on, meters.
const FLOOR_HALF_M: f32 = 50.0;
/// Half-thickness of that slab, meters.
const FLOOR_THICK_M: f32 = 0.5;
/// Fastest start velocity handed to the physical layer, m/s.
const MAX_START_MS: f32 = 10.0;
/// Segments of one stump cap or limb cylinder.
const CAP_SEGMENTS: u32 = 12;
/// Depth of a stump cap along the limb axis, meters.
const CAP_DEPTH_M: f32 = 0.06;
/// Flesh color of a falling limb.
const FLESH_COLOR: [f32; 3] = [0.42, 0.12, 0.10];
/// Weight of one vertex that must sit on the leaving chain for that vertex to
/// leave with it; below it the vertex stays on the body.
const LEAVE_WEIGHT: f32 = 0.5;
/// Most limb bodies one slot's cut mask needs (one per severable joint).
const MAX_LIMBS: usize = JOINT_TOTAL - 1;

/// The CT7 joint that leaves the body for one cut joint, in [`JointId`] wire
/// order. The pelvis never detaches. Waist and chest both take the torso (the
/// CT7 body has no separate chest joint).
const CUT_BONE: [&str; JOINT_TOTAL] = [
    "",            // Pelvis
    "torso",       // Waist
    "torso",       // Chest
    "head",        // Neck
    "arm_upper_l", // ShoulderL
    "arm_upper_r", // ShoulderR
    "arm_lower_l", // ElbowL
    "arm_lower_r", // ElbowR
    "palm_l",      // WristL
    "palm_r",      // WristR
    "leg_upper_l", // HipL
    "leg_upper_r", // HipR
    "leg_lower_l", // KneeL
    "leg_lower_r", // KneeR
    "foot_l",      // AnkleL
    "foot_r",      // AnkleR
];
/// Capsule radius of one cut joint's limb, meters, in [`JointId`] wire order.
const CUT_RADIUS_M: [f32; JOINT_TOTAL] = [
    0.0,   // Pelvis
    0.115, // Waist
    0.115, // Chest
    0.09,  // Neck
    0.045, // ShoulderL
    0.045, // ShoulderR
    0.04,  // ElbowL
    0.04,  // ElbowR
    0.03,  // WristL
    0.03,  // WristR
    0.075, // HipL
    0.075, // HipR
    0.06,  // KneeL
    0.06,  // KneeR
    0.045, // AnkleL
    0.045, // AnkleR
];

/// Mesh id base for the limb and stump draws (clear of the body-part range).
pub const LIMB_MESH_ID_BASE: u64 = 0xF16D_0000;

/// Mesh key of one cut's limb or stump cap: `base + slot * 16 + joint`.
#[must_use]
pub fn limb_mesh_key(slot: usize, joint: u8) -> MeshKey {
    let index = slot * JOINT_TOTAL + usize::from(joint);
    let id = LIMB_MESH_ID_BASE + u64::try_from(index).unwrap_or(u64::MAX - LIMB_MESH_ID_BASE);
    let mut digest = [0x5Cu8; 32];
    digest[0] = u8::try_from(slot).unwrap_or(u8::MAX);
    digest[1] = joint;
    MeshKey::new(id, digest)
}

/// One severed limb that left a body.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Limb {
    /// Wake-up id given to the physical layer.
    part: u32,
    /// Victim slot.
    slot: u8,
    /// Cut joint wire value.
    joint: u8,
    /// CT7 joint where the chain left.
    bone: usize,
    /// Capsule radius, meters.
    radius_m: f32,
    /// Capsule half-height along its own Y, meters.
    half_height_m: f32,
}

/// One drawn limb body at a frame, as numbers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LimbPose {
    /// Victim slot.
    pub slot: u8,
    /// Cut joint wire value.
    pub joint: u8,
    /// CT7 joint name the chain left from.
    pub bone: &'static str,
    /// Body center, meters.
    pub pos: [f32; 3],
    /// Body orientation, unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
    /// Capsule radius, meters.
    pub radius_m: f32,
    /// Capsule half-height along its own Y, meters.
    pub half_height_m: f32,
}

/// One stump cap on a remaining body at a frame, as numbers.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StumpPose {
    /// Victim slot.
    pub slot: u8,
    /// Cut joint wire value.
    pub joint: u8,
    /// CT7 joint name the cap sits on.
    pub bone: &'static str,
    /// Cap center, meters.
    pub pos: [f32; 3],
    /// Cap orientation, unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
    /// Cap radius, meters.
    pub radius_m: f32,
}

/// What one frame draws of the cuts: the fallen limb bodies, the stump caps, the
/// blood decals and the spray. Plain data, presentation only.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct LimbCapture {
    /// Fallen limb bodies drawn.
    pub limbs: usize,
    /// Stump caps drawn.
    pub stumps: usize,
    /// Blood decals stamped.
    pub decals: usize,
    /// Blood sprays emitted.
    pub sprays: usize,
    /// One row per fallen limb.
    pub limb_poses: Vec<LimbPose>,
    /// One row per stump cap.
    pub stump_poses: Vec<StumpPose>,
}

impl LimbCapture {
    /// The one-line receipt the tests print: `limb_look: limbs=.. stumps=.. decals=.. sprays=..`.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "limb_look: limbs={} stumps={} decals={} sprays={}",
            self.limbs, self.stumps, self.decals, self.sprays
        )
    }
}

/// The limb layer of a [`FighterLook`]: one float world, one body per cut.
pub(super) struct LimbLook {
    on: bool,
    gore: GoreSetting,
    tier: Tier,
    /// The world, built at the first cut.
    fx: Option<PhysFx>,
    /// Cut joints per slot: bit `joint` set once that joint's chain left.
    cut: [u16; MAX_LOOK_FIGHTERS],
    live: Vec<Limb>,
    /// One part id per spawn; the physical layer names the body by it.
    next_part: u32,
    spawned: u32,
    refused: u32,
    stepped: Option<u32>,
    /// Resident limb and stump keys uploaded for the last frame.
    published: Vec<MeshKey>,
}

impl LimbLook {
    /// A layer with no world yet: no work before the first cut.
    pub(super) fn new() -> Self {
        Self {
            on: true,
            gore: GoreSetting::Full,
            tier: LIMB_TIER,
            fx: None,
            cut: [0; MAX_LOOK_FIGHTERS],
            live: Vec::new(),
            next_part: 1,
            spawned: 0,
            refused: 0,
            stepped: None,
            published: Vec::new(),
        }
    }

    /// A layer that never spawns: for the tests' looks that have no rigs.
    #[cfg(test)]
    pub(super) fn empty() -> Self {
        Self::new()
    }

    fn live_count(&self) -> usize {
        self.live.len()
    }

    pub(super) fn stumped(&self, slot: usize) -> bool {
        self.cut.get(slot).is_some_and(|mask| *mask != 0)
    }

    fn mask(&self, slot: usize) -> u16 {
        self.cut.get(slot).copied().unwrap_or(0)
    }

    /// Drops the world and every body; the layer starts over on the same settings.
    fn clear(&mut self) {
        self.fx = None;
        self.live.clear();
        self.cut = [0; MAX_LOOK_FIGHTERS];
        self.stepped = None;
        self.published.clear();
    }
}

/// Microseconds on the wall clock since the first call: the receipt line's step cost.
fn wall_micros() -> u64 {
    u64::try_from(engine_clock::now().as_micros()).unwrap_or(u64::MAX)
}

/// The CT7 joint index of `name`, or `None` when the rig has no such joint.
fn joint_index(rig: &Rig, name: &str) -> Option<usize> {
    rig.skeleton
        .joints
        .iter()
        .position(|joint| joint.name == name)
}

/// Root of the leaving chain for one cut joint: the authority's wire value to the
/// CT7 joint that leaves. `None` for the pelvis or a value past the table.
fn cut_root(rig: &Rig, joint: u8) -> Option<usize> {
    let id = JointId::from_u8(joint)?;
    if !id.is_severable() {
        return None;
    }
    joint_index(rig, CUT_BONE[usize::from(joint)])
}

/// True when `joint` sits under `root` (or is `root`) in the rig skeleton.
fn under(rig: &Rig, joint: usize, root: usize) -> bool {
    let mut at = Some(joint);
    for _ in 0..=rig.skeleton.len() {
        let Some(index) = at else {
            return false;
        };
        if index == root {
            return true;
        }
        at = rig
            .skeleton
            .joints
            .get(index)
            .and_then(|joint| joint.parent);
    }
    false
}

/// For every joint in `rig`, the root of the leaving chain it belongs to, or
/// `usize::MAX` when it stays. `mask` is a bit per [`JointId`] wire value.
fn leaving(rig: &Rig, mask: u16) -> Vec<usize> {
    let mut roots: Vec<usize> = Vec::new();
    for joint in 0..JOINT_TOTAL {
        if mask & (1 << joint) == 0 {
            continue;
        }
        if let Some(root) = cut_root(rig, u8::try_from(joint).unwrap_or(u8::MAX)) {
            roots.push(root);
        }
    }
    let mut owner = vec![usize::MAX; rig.skeleton.len()];
    for (index, spot) in owner.iter_mut().enumerate() {
        if let Some(root) = roots.iter().copied().find(|root| under(rig, index, *root)) {
            *spot = root;
        }
    }
    owner
}

/// Remap `skin` so every vertex weighted mostly onto a leaving chain collapses
/// onto that chain's stump: the remaining body animates exactly as before and
/// the limb leaves no stretched vertices behind. `owner` is [`leaving`].
fn remap_skin(owner: &[usize], skin: &SkinWeights) -> SkinWeights {
    let mut joints = skin.joints.clone();
    let mut weights = skin.weights.clone();
    for (vertex, row) in joints.iter_mut().enumerate() {
        let Some(weight) = weights.get(vertex) else {
            continue;
        };
        let mut gone = 0.0f32;
        let mut stump = usize::MAX;
        for influence in 0..row.len() {
            let joint = usize::from(row[influence]);
            let root = owner.get(joint).copied().unwrap_or(usize::MAX);
            if root != usize::MAX {
                gone += weight[influence];
                if stump == usize::MAX {
                    stump = root;
                }
            }
        }
        if gone > LEAVE_WEIGHT {
            *row = [u16::try_from(stump).unwrap_or(0); 4];
            weights[vertex] = [1.0, 0.0, 0.0, 0.0];
        }
    }
    SkinWeights { joints, weights }
}

/// A quaternion turning `+Y` onto `axis`; the identity for a degenerate axis and
/// a half turn about X for the opposite one. `+ - * /` and `sqrt` only.
fn quat_from_y(axis: [f32; 3]) -> [f32; 4] {
    let len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if !len.is_finite() || len < 1e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let a = [axis[0] / len, axis[1] / len, axis[2] / len];
    if a[1] < -0.999_999 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    // cross(+Y, a) = (a2, 0, -a0); q = [cross, 1 + dot].
    let q = [a[2], 0.0, -a[0], 1.0 + a[1]];
    let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if norm.is_finite() && norm > 1e-6 {
        [q[0] / norm, q[1] / norm, q[2] / norm, q[3] / norm]
    } else {
        [1.0, 0.0, 0.0, 0.0]
    }
}

/// One spawn's whole input, gathered before the layer mutates itself.
struct Start {
    slot: usize,
    joint: u8,
    bone: usize,
    radius_m: f32,
    half_height_m: f32,
    at: Transform,
    velocity: [f32; 3],
}

impl FighterLook {
    /// Switches the limb layer. Off, a cut draws nothing as before RZ3.
    /// Presentation only.
    pub fn set_limbs(&mut self, on: bool) {
        self.limbs.on = on;
        if !on {
            self.limbs.clear();
        }
    }

    /// Sets the gore level the limb layer honors. `Off` draws no limb, no stump
    /// and no blood (the authority still decided the cut); `Reduced` draws the
    /// limb, the stump and the decal; `Full` adds the spray. Presentation only:
    /// every level steps the same authority and keeps the golden.
    pub fn set_gore(&mut self, gore: GoreSetting) {
        self.limbs.gore = gore;
        if gore == GoreSetting::Off {
            self.limbs.clear();
        }
    }

    /// The gore level in force.
    #[must_use]
    pub fn gore(&self) -> GoreSetting {
        self.limbs.gore
    }

    /// Sets the hardware tier whose body cap bounds the limbs. Drops the world:
    /// call it at start up, not mid-fight.
    pub fn set_limb_tier(&mut self, tier: Tier) {
        self.limbs.tier = tier;
        self.limbs.clear();
    }

    /// Fallen limbs alive.
    #[must_use]
    pub fn limbs(&self) -> usize {
        self.limbs.live_count()
    }

    /// Limbs spawned since load.
    #[must_use]
    pub fn limb_spawns(&self) -> u32 {
        self.limbs.spawned
    }

    /// Cuts the physical layer refused since load (a bad joint, a cap too small).
    #[must_use]
    pub fn limb_refused(&self) -> u32 {
        self.limbs.refused
    }

    /// The world's counters as the one `physfx:` receipt line, or `None` before
    /// the first cut.
    #[must_use]
    pub fn limb_receipt(&self) -> Option<String> {
        self.limbs.fx.as_ref().map(|fx| fx.stats().receipt_line())
    }

    /// Cut joints of `slot` as a bit per [`JointId`] wire value; zero while whole.
    #[must_use]
    pub fn cut_joints(&self, slot: u8) -> u16 {
        self.limbs.mask(usize::from(slot))
    }

    /// True while any joint of `slot` has left the body.
    #[must_use]
    pub fn stumped(&self, slot: u8) -> bool {
        self.limbs.stumped(usize::from(slot))
    }

    /// Takes one tick's authority cuts and detaches the named chains. Call it once
    /// per accepted tick after `observe_tick` (the slot state it reads is that
    /// tick's). Presentation only: the cut is already the authority's truth.
    pub fn observe_cuts(&mut self, cuts: &[Cut]) {
        if !self.limbs.on || self.limbs.gore == GoreSetting::Off {
            return;
        }
        let mut starts: Vec<Start> = Vec::with_capacity(cuts.len());
        for cut in cuts {
            let slot = usize::from(cut.victim);
            let Some(anim) = self.slots.get(slot).copied() else {
                self.limbs.refused = self.limbs.refused.saturating_add(1);
                continue;
            };
            if !anim.seen {
                self.limbs.refused = self.limbs.refused.saturating_add(1);
                continue;
            }
            let rig = &self.rigs[self.rig_for_slot(slot)];
            let Some(root) = cut_root(rig, cut.joint) else {
                self.limbs.refused = self.limbs.refused.saturating_add(1);
                continue;
            };
            if self.limbs.cut[slot] & (1 << cut.joint) != 0 {
                // One joint detaches once.
                self.limbs.refused = self.limbs.refused.saturating_add(1);
                continue;
            }
            let Ok(sample) = self.slot_sample(rig, slot, &anim, 0.0) else {
                self.limbs.refused = self.limbs.refused.saturating_add(1);
                continue;
            };
            let matrix = mul(&sample.world[root], &sample.root);
            let pos = [matrix[3][0], matrix[3][1], matrix[3][2]];
            // The bone runs from its joint to its first child in the pose; a leaf
            // runs along its own joint's +Y.
            let tip = rig
                .skeleton
                .joints
                .iter()
                .position(|joint| joint.parent == Some(root))
                .map(|child| {
                    let m = mul(&sample.world[child], &sample.root);
                    [m[3][0] - pos[0], m[3][1] - pos[1], m[3][2] - pos[2]]
                })
                .unwrap_or_else(|| {
                    let row = sample.world[root];
                    [row[1][0], row[1][1], row[1][2]]
                });
            let len = (tip[0] * tip[0] + tip[1] * tip[1] + tip[2] * tip[2]).sqrt();
            let len = if len.is_finite() && len > 0.02 {
                len
            } else {
                0.12
            };
            let radius = CUT_RADIUS_M[usize::from(cut.joint)].min(len * 0.5);
            let half_len = len * 0.5;
            let at = Transform {
                pos: [
                    pos[0] + tip[0] * 0.5,
                    pos[1] + tip[1] * 0.5,
                    pos[2] + tip[2] * 0.5,
                ],
                rot: quat_from_y(tip),
            };
            let velocity = [
                (anim.pos[0] - anim.prev_pos[0]) * TICK_HZ,
                0.0,
                (anim.pos[2] - anim.prev_pos[2]) * TICK_HZ,
            ]
            .map(|v| {
                if v.is_finite() {
                    v.clamp(-MAX_START_MS, MAX_START_MS)
                } else {
                    0.0
                }
            });
            starts.push(Start {
                slot,
                joint: cut.joint,
                bone: root,
                radius_m: radius,
                half_height_m: (half_len - radius).max(0.0),
                at,
                velocity,
            });
        }
        for start in starts {
            self.spawn_limb(start);
        }
    }

    /// Hands one chain to the physical layer; a refusal leaves the joint attached.
    fn spawn_limb(&mut self, start: Start) {
        let part = self.limbs.next_part;
        if self.limbs.fx.is_none() {
            // The sand slab, built at the first cut: no work before one.
            let mut fx = PhysFx::new(self.limbs.tier).with_clock(wall_micros);
            if fx
                .add_static_box(
                    [FLOOR_HALF_M, FLOOR_THICK_M, FLOOR_HALF_M],
                    [0.0, -FLOOR_THICK_M, 0.0],
                )
                .is_err()
            {
                self.limbs.refused = self.limbs.refused.saturating_add(1);
                return;
            }
            self.limbs.fx = Some(fx);
        }
        let shape = BoneShape {
            radius: start.radius_m,
            half_height: start.half_height_m,
        };
        let spawned = match self.limbs.fx.as_mut() {
            Some(fx) => fx.spawn_limb_of(Material::Flesh, part, shape, start.at, start.velocity),
            None => Err(FxError::BadCaps),
        };
        if spawned.is_err() {
            self.limbs.refused = self.limbs.refused.saturating_add(1);
            return;
        }
        self.limbs.next_part = self.limbs.next_part.saturating_add(1);
        self.limbs.cut[start.slot] |= 1 << start.joint;
        self.limbs.live.push(Limb {
            part,
            slot: u8::try_from(start.slot).unwrap_or(u8::MAX),
            joint: start.joint,
            bone: start.bone,
            radius_m: start.radius_m,
            half_height_m: start.half_height_m,
        });
        self.limbs.spawned = self.limbs.spawned.saturating_add(1);
    }

    /// Steps the limb world one tick and forgets the bodies it recycled. Safe to
    /// call more than once per tick: the world steps once.
    pub fn limb_tick(&mut self) {
        if !self.limbs.on || self.limbs.fx.is_none() {
            return;
        }
        if self.limbs.stepped == Some(self.tick) {
            return;
        }
        self.limbs.stepped = Some(self.tick);
        let mut alive: Vec<u32> = Vec::new();
        if let Some(fx) = self.limbs.fx.as_mut() {
            fx.step();
            if let Ok(placed) = fx.transforms(1.0) {
                for one in placed {
                    if let Key::Limb { part } = one.key {
                        alive.push(part);
                    }
                }
            }
        }
        if self.limbs.live.len() > MAX_LIMBS {
            // The world itself recycles under its cap; keep the newest, so the
            // picture never draws a body the world already forgot.
            let excess = self.limbs.live.len() - MAX_LIMBS;
            self.limbs.live.drain(..excess);
        }
        self.limbs.live.retain(|limb| alive.contains(&limb.part));
    }

    /// The root of the leaving chain each rig joint belongs to, or `None` while
    /// the slot is whole.
    pub(super) fn limb_owner(&self, slot: usize, rig: &Rig) -> Option<Vec<usize>> {
        let mask = self.limbs.mask(slot);
        if mask == 0 {
            return None;
        }
        Some(leaving(rig, mask))
    }
    /// One body part's posed vertices, with a stumped slot's leaving chain
    /// collapsed onto its stump so the limb leaves the body. Loud on a refusal.
    pub(super) fn skin_part(
        &self,
        rig: &Rig,
        part: &RigPart,
        owner: Option<&[usize]>,
        uniform: &SkinningUniform,
    ) -> Result<SkinnedFrame, LookError> {
        let posed = match owner {
            Some(owner) => {
                let rows = remap_skin(owner, &part.skin);
                rig.pipe.render_skinned(&part.mesh, Some(&rows), uniform)
            }
            None => rig
                .pipe
                .render_skinned(&part.mesh, Some(&part.skin), uniform),
        };
        posed.map_err(|err| LookError::Skin(format!("{err:?}")))
    }

    /// Everything the frame draws of the cuts at sub-tick `alpha`: the fallen
    /// limbs from the physical layer, the stump caps on the animated bodies, and
    /// the blood the gore level allows. Presentation only; never hashed.
    #[must_use]
    pub fn limb_capture(&self, alpha: f32) -> LimbCapture {
        let mut out = LimbCapture::default();
        if !self.limbs.on || self.limbs.gore == GoreSetting::Off {
            return out;
        }
        out.decals = self.limbs.live.len();
        if self.limbs.gore == GoreSetting::Full {
            out.sprays = self.limbs.live.len();
        }
        if let Some(fx) = self.limbs.fx.as_ref() {
            if let Ok(placed) = fx.transforms(alpha) {
                for one in placed {
                    let Key::Limb { part } = one.key else {
                        continue;
                    };
                    let Some(limb) = self.limbs.live.iter().find(|limb| limb.part == part) else {
                        continue;
                    };
                    out.limb_poses.push(LimbPose {
                        slot: limb.slot,
                        joint: limb.joint,
                        bone: CUT_BONE[usize::from(limb.joint)],
                        pos: one.at.pos,
                        rot: one.at.rot,
                        radius_m: limb.radius_m,
                        half_height_m: limb.half_height_m,
                    });
                }
            }
        }
        // The cap rides the body the cut left behind: it reads the same pose the
        // frame draws, so the joint keeps animating under it.
        for limb in &self.limbs.live {
            let slot = usize::from(limb.slot);
            let Some(anim) = self.slots.get(slot) else {
                continue;
            };
            let rig = &self.rigs[self.rig_for_slot(slot)];
            let Ok(sample) = self.slot_sample(rig, slot, anim, alpha) else {
                continue;
            };
            let matrix = mul(&sample.world[limb.bone], &sample.root);
            out.stump_poses.push(StumpPose {
                slot: limb.slot,
                joint: limb.joint,
                bone: CUT_BONE[usize::from(limb.joint)],
                pos: [matrix[3][0], matrix[3][1], matrix[3][2]],
                rot: [0.0, 0.0, 0.0, 1.0],
                radius_m: limb.radius_m,
            });
        }
        out.stumps = out.stump_poses.len();
        out.limbs = out.limb_poses.len();
        out
    }

    /// The cap mesh of one cut joint, cold-built once per joint: a closed stub of
    /// that joint's radius. `None` for the pelvis or an unknown joint.
    #[must_use]
    pub fn stump_mesh(&self, joint: u8) -> Option<StumpCap> {
        let radius = *CUT_RADIUS_M.get(usize::from(joint))?;
        if radius <= 0.0 {
            return None;
        }
        stump_cap(radius, CAP_DEPTH_M, CAP_SEGMENTS).ok()
    }

    /// Upload the world-space limb and cap meshes for this frame's `alpha`, so
    /// [`FighterLook::push_limb_draws`] can draw them. Called from `stage_bodies`.
    pub(super) fn publish_limbs(&mut self, presenter: &mut Presenter, alpha: f32) {
        self.limbs.published.clear();
        if !self.limbs.on || self.limbs.gore == GoreSetting::Off {
            return;
        }
        let capture = self.limb_capture(alpha);
        for pose in &capture.limb_poses {
            let depth = (pose.half_height_m * 2.0).clamp(0.02, 0.6);
            let Ok(base) = stump_cap(pose.radius_m, depth, CAP_SEGMENTS) else {
                continue;
            };
            let Ok(placed) = place_mesh(&base, pose.pos, pose.rot) else {
                continue;
            };
            let key = limb_mesh_key(usize::from(pose.slot), pose.joint);
            let mesh = MeshData {
                positions: placed.positions,
                normals: placed.normals,
                uvs: placed.uvs,
                indices: placed.indices,
                tangents: Vec::new(),
            };
            if presenter.upload_mesh(key, &mesh).is_ok() {
                self.limbs.published.push(key);
            }
        }
        for pose in &capture.stump_poses {
            let Ok(base) = stump_cap(pose.radius_m, CAP_DEPTH_M, CAP_SEGMENTS) else {
                continue;
            };
            let Ok(placed) = place_mesh(&base, pose.pos, pose.rot) else {
                continue;
            };
            let key = limb_mesh_key(usize::from(pose.slot), pose.joint);
            let mesh = MeshData {
                positions: placed.positions,
                normals: placed.normals,
                uvs: placed.uvs,
                indices: placed.indices,
                tangents: Vec::new(),
            };
            if presenter.upload_mesh(key, &mesh).is_ok() {
                self.limbs.published.push(key);
            }
        }
    }

    /// Push one flesh draw per limb body and stump cap this frame published, plus
    /// the body parts as usual. Returns draws. A key the presenter does not hold
    /// draws nothing, so a failed publish never fails the frame.
    #[must_use]
    pub fn push_limb_draws(&self, list: &mut DrawList, resident: &[MeshKey]) -> usize {
        let flesh = PbrMaterial {
            base_color: FLESH_COLOR,
            metallic: 0.0,
            roughness: 0.7,
            exposure: 1.0,
        };
        let mut pushed = 0usize;
        for key in &self.limbs.published {
            if !resident.contains(key) {
                continue;
            }
            list.items.push(DrawItem {
                mesh: *key,
                material: flesh.clone(),
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
            pushed += 1;
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FighterView;
    use engine_audio::FightCue;
    use engine_physfx::Tier;
    use std::path::PathBuf;

    fn content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    /// A ring of `n` fighters facing the middle, like the arena match.
    fn ring(n: u8) -> Vec<FighterView> {
        (0..n)
            .map(|slot| {
                let a = f32::from(slot) / f32::from(n) * std::f32::consts::TAU;
                FighterView {
                    slot,
                    pos_m: [3.0 * a.sin(), 0.0, 3.0 * a.cos()],
                    yaw: a + std::f32::consts::PI,
                }
            })
            .collect()
    }

    /// One cut naming `joint` on `victim`.
    fn cut(swing: u32, victim: u8, joint: u8) -> Cut {
        Cut {
            swing,
            attacker: victim.wrapping_add(1),
            victim,
            joint,
            flags: 0,
            energy_q16: 600_000,
            protection_q16: 589_824,
        }
    }

    /// The elbow of the left arm (wire 6).
    const ELBOW_L: u8 = 6;
    /// The right shoulder (wire 5).
    const SHOULDER_R: u8 = 5;

    fn run_ticks(
        look: &mut FighterLook,
        views: &[FighterView],
        ticks: u32,
        cut_at: Option<(u32, Cut)>,
    ) {
        let hp = vec![1000u16; views.len()];
        for t in 0..ticks {
            let cues: Vec<FightCue> = Vec::new();
            look.observe_tick(t, views, &hp, &cues);
            if let Some((at, c)) = cut_at {
                if t == at {
                    look.observe_cuts(&[c]);
                }
            }
            look.limb_tick();
        }
    }

    #[test]
    fn limb_look_scripted_cut_detaches_exactly_the_named_part() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(2);
        run_ticks(&mut look, &views, 20, Some((10, cut(1, 1, ELBOW_L))));
        assert_eq!(look.limbs(), 1, "one chain left");
        assert_eq!(look.limb_spawns(), 1);
        assert_eq!(look.limb_refused(), 0);
        assert_eq!(look.cut_joints(1), 1 << ELBOW_L, "the named joint only");
        assert_eq!(look.cut_joints(0), 0, "the attacker is whole");
        assert!(look.stumped(1) && !look.stumped(0));
        let capture = look.limb_capture(0.5);
        assert_eq!(capture.limbs, 1);
        assert_eq!(capture.stumps, 1);
        assert_eq!(capture.limb_poses[0].bone, "arm_lower_l");
        assert_eq!(capture.stump_poses[0].bone, "arm_lower_l");
        assert!(capture.stump_poses[0].radius_m > 0.0);
        // A different joint on the same victim leaves a second chain.
        look.observe_cuts(&[cut(2, 1, SHOULDER_R)]);
        look.limb_tick();
        assert_eq!(look.limbs(), 2);
        assert_eq!(look.cut_joints(1), (1 << ELBOW_L) | (1 << SHOULDER_R));
        // The same joint detaches once: the repeat is refused, nothing spawns.
        look.observe_cuts(&[cut(3, 1, ELBOW_L)]);
        assert_eq!(look.limbs(), 2);
        assert_eq!(look.limb_refused(), 1);
        // The pelvis never detaches.
        look.observe_cuts(&[cut(4, 1, 0)]);
        assert_eq!(look.limbs(), 2);
        assert_eq!(look.limb_refused(), 2);
    }

    #[test]
    fn limb_look_remaining_body_keeps_animating_after_a_cut() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(2);
        run_ticks(&mut look, &views, 20, None);
        let before = look.pose_frame_joints(2, 0.5).expect("CPU pose");
        let before_body = before.iter().find(|b| b.slot == 1).expect("slot 1 drawn");
        let before_joints = before_body.joints.len();
        let before_min = before_body.min;
        // Cut the left forearm at tick 20, then keep ticking.
        let hp = [1000u16; 2];
        for t in 20..60u32 {
            look.observe_tick(t, &views, &hp, &[]);
            if t == 20 {
                look.observe_cuts(&[cut(1, 1, ELBOW_L)]);
            }
            look.limb_tick();
        }
        let after = look.pose_frame_joints(2, 0.5).expect("CPU pose");
        let after_body = after.iter().find(|b| b.slot == 1).expect("slot 1 drawn");
        assert_eq!(
            after_body.joints.len(),
            before_joints,
            "the rig is unchanged"
        );
        assert_eq!(after_body.nonfinite, 0);
        // The stump follows the animating joint.
        let a = look.limb_capture(0.0);
        let joints_a = look
            .pose_frame_joints(2, 0.0)
            .expect("CPU pose")
            .into_iter()
            .find(|b| b.slot == 1)
            .expect("slot 1")
            .joints;
        for t in 60..90u32 {
            look.observe_tick(t, &views, &hp, &[]);
            look.limb_tick();
        }
        let b = look.limb_capture(0.0);
        let joints_b = look
            .pose_frame_joints(2, 0.0)
            .expect("CPU pose")
            .into_iter()
            .find(|b| b.slot == 1)
            .expect("slot 1")
            .joints;
        assert_eq!((a.stumps, b.stumps), (1, 1));
        assert!(joints_a != joints_b, "the remaining body still animates");
        assert!(a.stump_poses[0].pos.iter().all(|v| v.is_finite()));
        // The full skin collapses the leaving chain: the drawn body loses the
        // forearm's vertices, so its lowest vertex rises (an arm hangs low).
        let full = look.pose_frame(2, 0.5).expect("CPU skin");
        let full_body = full
            .iter()
            .find(|b| b.truth.slot == 1)
            .expect("slot 1 skin");
        assert!(
            full_body.truth.min[1] >= before_min[1] - 1e-4,
            "no stretched vertices"
        );
        assert!(look.stumped(1));
    }

    #[test]
    fn limb_look_six_simultaneous_cuts_stay_under_the_physfx_cap() {
        for tier in [Tier::Potato, Tier::Comfort] {
            let caps = tier.caps();
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            look.set_limb_tier(tier);
            let views = ring(8);
            let cuts: Vec<Cut> = (0..6u8)
                .map(|victim| cut(u32::from(victim), victim, ELBOW_L))
                .collect();
            run_ticks(&mut look, &views, 20, None);
            look.observe_cuts(&cuts);
            look.limb_tick();
            assert_eq!(look.limbs(), 6, "{tier:?}: every cut is a body");
            assert_eq!(look.limb_refused(), 0, "{tier:?}");
            let receipt = look.limb_receipt().expect("the world exists");
            println!("limb_look: {tier:?} {receipt} cap_bodies={}", caps.bodies);
            let bodies = look
                .limbs
                .fx
                .as_ref()
                .map(|fx| fx.stats().bodies)
                .expect("world");
            assert!(
                bodies <= caps.bodies,
                "{tier:?}: {bodies} under {}",
                caps.bodies
            );
            for slot in 0..6u8 {
                assert!(look.stumped(slot), "{tier:?} slot {slot}");
            }
        }
    }

    #[test]
    fn limb_look_gore_off_draws_no_limb_and_no_stump() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.set_gore(GoreSetting::Off);
        assert_eq!(look.gore(), GoreSetting::Off);
        let views = ring(2);
        run_ticks(&mut look, &views, 20, Some((10, cut(1, 1, ELBOW_L))));
        assert_eq!(look.limbs(), 0, "Off draws no limb");
        assert_eq!(look.limb_spawns(), 0);
        assert_eq!(look.limb_receipt(), None, "no world is built");
        let capture = look.limb_capture(0.5);
        assert_eq!(capture.limbs, 0);
        assert_eq!(capture.stumps, 0);
        assert_eq!(capture.decals, 0);
        assert_eq!(capture.sprays, 0);
        assert!(!look.stumped(1), "the body is untouched under Off");
        // Reduced draws the limb and the stump and the decal, no spray.
        let mut reduced = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        reduced.set_gore(GoreSetting::Reduced);
        run_ticks(&mut reduced, &views, 20, Some((10, cut(1, 1, ELBOW_L))));
        let reduced_capture = reduced.limb_capture(0.5);
        assert_eq!((reduced_capture.limbs, reduced_capture.stumps), (1, 1));
        assert_eq!(reduced_capture.decals, 1);
        assert_eq!(reduced_capture.sprays, 0, "Reduced drops the spray");
        // Full adds the spray on the same cut.
        let mut full = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        full.set_gore(GoreSetting::Full);
        run_ticks(&mut full, &views, 20, Some((10, cut(1, 1, ELBOW_L))));
        let full_capture = full.limb_capture(0.5);
        assert_eq!((full_capture.limbs, full_capture.stumps), (1, 1));
        assert_eq!(full_capture.decals, 1);
        assert_eq!(full_capture.sprays, 1, "Full keeps the spray");
        assert!(full_capture.line().contains("stumps=1"));
    }

    #[test]
    fn limb_look_capture_mid_cut_shows_the_stump() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(2);
        run_ticks(&mut look, &views, 20, Some((10, cut(1, 1, ELBOW_L))));
        // A capture a few ticks after the cut: the stump sits on the arm.
        let capture = look.limb_capture(0.5);
        assert_eq!(capture.stumps, 1, "the capture shows the stump");
        let cap = capture.stump_poses[0];
        assert_eq!((cap.slot, cap.joint), (1, ELBOW_L));
        assert!(cap.pos.iter().all(|v| v.is_finite()), "finite cap pose");
        assert!(
            (0.01..=0.3).contains(&cap.radius_m),
            "radius {}",
            cap.radius_m
        );
        assert_eq!(cap.rot, [0.0, 0.0, 0.0, 1.0]);
        // The cap mesh the render owner builds closes the hole at that radius.
        let mesh = look.stump_mesh(ELBOW_L).expect("a cap builds");
        assert!(mesh.is_closed(), "the stump cap is watertight");
        let reach = mesh.reach_m([0.0; 3], [0.0, 1.0, 0.0]).expect("finite");
        assert!((reach - cap.radius_m).abs() < 1e-5, "reach {reach}");
        // The falling limb is a body of its own, above the sand.
        assert_eq!(capture.limb_poses.len(), 1);
        let limb = capture.limb_poses[0];
        assert!(limb.pos.iter().all(|v| v.is_finite()));
        assert!(limb.pos[1] > 0.0, "the limb starts above the ground");
        // A capture before the cut shows nothing.
        let mut fresh = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        run_ticks(&mut fresh, &views, 9, None);
        assert_eq!(fresh.limb_capture(0.5).stumps, 0);
        assert_eq!(fresh.limb_capture(0.5).limbs, 0);
    }

    #[test]
    fn limb_look_golden_hashes_equal_for_off_reduced_and_full() {
        // The window's session: 8 fighters under the v1 duel for 240 ticks with a
        // scripted cut every 40 ticks. The layer only reads the tick, so the
        // authority checkpoints are the same bytes at every gore level.
        let run = |gore: GoreSetting| {
            let mut app = crate::App::new(crate::GameConfig {
                seed: 2040,
                ticks: 240,
                fighters: 8,
                half_extent_m: 20.0,
            })
            .and_then(crate::App::into_duel)
            .expect("duel builds");
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            look.set_gore(gore);
            let mut spawns = 0;
            for t in 0..240u32 {
                let out = app.tick_empty();
                look.observe_tick(t, app.fighter_views(), app.hp(), &out.cues);
                if t % 40 == 20 {
                    let victim = u8::try_from(t / 40).unwrap_or(0) % 8;
                    look.observe_cuts(&[cut(t, victim, ELBOW_L)]);
                }
                look.observe_live_balances(app.live_match(), 8);
                look.limb_tick();
                spawns = look.limb_spawns();
            }
            (app.checkpoints().to_vec(), spawns, look.limb_capture(0.5))
        };
        let (off, off_spawns, off_capture) = run(GoreSetting::Off);
        let (reduced, reduced_spawns, reduced_capture) = run(GoreSetting::Reduced);
        let (full, full_spawns, full_capture) = run(GoreSetting::Full);
        println!(
            "limb_look: {} checkpoints equal, off spawns={off_spawns}, reduced spawns={reduced_spawns}, full spawns={full_spawns}, {}",
            off.len(),
            full_capture.line()
        );
        assert!(!off.is_empty(), "the run took checkpoints");
        assert_eq!(
            off, reduced,
            "authority checkpoints are the same at Reduced"
        );
        assert_eq!(off, full, "authority checkpoints are the same at Full");
        assert_eq!(off_spawns, 0, "Off draws nothing");
        assert_eq!(off_capture.limbs, 0);
        assert!(
            reduced_spawns > 0 && reduced_capture.stumps > 0,
            "Reduced cut"
        );
        assert_eq!(full_spawns, reduced_spawns, "the same cuts at every level");
    }
}
