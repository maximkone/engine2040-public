//! ragdoll_look: a floored fighter falls as a ragdoll in the window (RZ2w).
//!
//! The sim decides who is floored (`GaitPhase::Fallen` from `LiveMatch::balance`, or a
//! `FightCue::Knockdown`). From that tick this layer hands the standing pose to the float
//! physical layer (`engine-physfx`), steps it once per accepted tick, and the fighter look
//! reads the bones back instead of the flat death clip. When the sim says the fighter is back on
//! its feet, the last ragdoll pose blends into the clip through the same [`Inertial`] offsets a
//! clip switch uses (LF9), so the get-up has no pop.
//!
//! The physical skeleton is a reduced one: 15 bones of the CT7 skeleton (pelvis, torso, head,
//! two legs of three, two arms of three) each get a capsule from the one table [`BONES`]. Every
//! other joint (shoulders, fingers, toes, heels, weapon sockets) rides rigidly on its nearest
//! physical ancestor. A capsule sits along its bone, centered between the bone's joint and its
//! tip. The skinned body takes the bone rotations of the ragdoll and the pelvis position; limb
//! lengths come from the standing pose, so the mesh never stretches.
//!
//! Consumer: `FighterLook::observe_live_balances` polls the gaits and calls the tick;
//! `FighterLook::inertial_pose` reads the bones back for a ragdolled slot, so the full skin, the
//! joints-only pose and the window draw the same body. Data owner: the authority owns who fell
//! (integers, hashed); this module owns only the picture. Units: meters, seconds, Y up, impulses
//! in newton-seconds, unit quaternions `[x, y, z, w]`. Determinism: presentation only. The sim
//! never reads a float from here and nothing here is hashed, so the checkpoint hashes are the
//! same with the layer on or off (`ragdoll_look_hashes_equal_on_and_off`). The same ticks give the
//! same bones twice on one machine. Errors: a refused spawn (`FxError`) leaves the slot on the
//! flat death pose it had before and is counted in [`FighterLook::ragdoll_refused`]. Budget: no
//! allocation and no work before the first knockout; then at most the tier's ragdoll cap
//! (Comfort 8, Potato 2) of 15 bodies each, one `PhysFx::step` per accepted tick.

use engine_anim::contract::{Joint, LocalPose, Skeleton};
use engine_anim::inertial::Inertial;
use engine_arena::GaitPhase;
use engine_audio::FightCue;
use engine_core::quat_mul;
use engine_physfx::{BoneShape, Error as FxError, Key, PhysFx, Rig as FxRig, Tier, Transform};

use super::fighter_look_inertia::{apply_offsets, begin_switch, NO_OFFSET};
use super::{rig_index, slot_root, Act, FighterLook, Rig, SlotAnim, MAX_LOOK_FIGHTERS, TICK_HZ};

/// Tier a look starts on: the comfortable desktop tier (eight ragdolls fit).
pub const DEFAULT_TIER: Tier = Tier::Comfort;
/// Physical bones per ragdoll.
pub const BONE_COUNT: usize = BONES.len();
/// Half-width of the sand slab the bodies land on, meters (the pit is 40 m across).
const FLOOR_HALF_M: f32 = 50.0;
/// Half-thickness of that slab, meters.
const FLOOR_THICK_M: f32 = 0.5;
/// Newton-seconds of root impulse per permille of damage.
const NS_PER_PERMILLE: f32 = 0.1;
/// How much of the push goes up, as a share of its planar length.
const KICK_LIFT: f32 = 0.3;
/// Damage assumed for a fall no `Hit` cue explains, in permille.
const DEFAULT_DAMAGE_PERMILLE: u16 = 250;
/// Largest damage that counts, in permille.
const MAX_DAMAGE_PERMILLE: u16 = 600;
/// Fastest start velocity handed to the physical layer, m/s.
const MAX_START_MS: f32 = 10.0;
/// Ticks a ragdoll lies before the sim's recovery may stand it up (0.75 s; the sim's own fall
/// and catch take 45 ticks).
const MIN_LYING_TICKS: u32 = 45;
/// Halflife of the get-up blend in seconds: the time the gap between the last ragdoll pose and the
/// clip halves in. Longer than a clip switch (0.08 s) because the gap is a lying body against a
/// standing one, and a short halflife would pop (the view law judges a joint 10 cm off its own
/// constant-velocity prediction in one frame).
const GETUP_HALFLIFE_S: f32 = 0.25;
/// Ticks after which the get-up offsets are dropped: ten halflives (2.5 s at 60 Hz), a millimeter
/// per meter left.
const GETUP_TICKS: u32 = 150;
/// Age marking a slot with no get-up in flight.
const NO_GETUP: u32 = u32::MAX;
/// A `Hit` this many ticks old or newer explains a fall that has no cue of its own.
const HIT_EXPLAINS_TICKS: u32 = 30;
/// Ticks a body keeps following the sim's slide, level, after its tether ended: the sim lets
/// the momentum drift out while the body falls and lands.
const LAND_CARRY_TICKS: u32 = 40;
/// A body that was towed may get up once its root is slower than this, m/s.
const LANDED_MS: f32 = 1.2;

/// One physical bone: the CT7 joint it follows, its physical parent (an index of this table,
/// parent first), the CT7 joint at its tip (the capsule centers halfway there) or a fixed length
/// when it has none, and the capsule. The radii are kept under the clearance of the standing and
/// guard poses so no two unjoined capsules start inside each other.
struct Bone {
    joint: &'static str,
    parent: Option<usize>,
    tip: Option<&'static str>,
    fixed_len_m: f32,
    radius_m: f32,
    half_height_m: f32,
}

/// The one bone-shape table of the CT7 skeleton. Lengths are the standing bone lengths of the
/// hero body (the variants differ by millimeters): a capsule is `2 * (half_height + radius)` long.
const BONES: [Bone; 15] = [
    Bone {
        joint: "pelvis",
        parent: None,
        tip: Some("torso"),
        fixed_len_m: 0.17,
        radius_m: 0.09,
        half_height_m: 0.0,
    },
    Bone {
        joint: "torso",
        parent: Some(0),
        tip: Some("head"),
        fixed_len_m: 0.50,
        radius_m: 0.115,
        half_height_m: 0.13,
    },
    Bone {
        joint: "head",
        parent: Some(1),
        tip: None,
        fixed_len_m: 0.24,
        radius_m: 0.09,
        half_height_m: 0.03,
    },
    Bone {
        joint: "leg_upper_l",
        parent: Some(0),
        tip: Some("leg_lower_l"),
        fixed_len_m: 0.44,
        radius_m: 0.07,
        half_height_m: 0.148,
    },
    Bone {
        joint: "leg_lower_l",
        parent: Some(3),
        tip: Some("foot_l"),
        fixed_len_m: 0.44,
        radius_m: 0.055,
        half_height_m: 0.164,
    },
    Bone {
        joint: "foot_l",
        parent: Some(4),
        tip: Some("toes_l"),
        fixed_len_m: 0.18,
        radius_m: 0.04,
        half_height_m: 0.05,
    },
    Bone {
        joint: "leg_upper_r",
        parent: Some(0),
        tip: Some("leg_lower_r"),
        fixed_len_m: 0.44,
        radius_m: 0.07,
        half_height_m: 0.148,
    },
    Bone {
        joint: "leg_lower_r",
        parent: Some(6),
        tip: Some("foot_r"),
        fixed_len_m: 0.44,
        radius_m: 0.055,
        half_height_m: 0.164,
    },
    Bone {
        joint: "foot_r",
        parent: Some(7),
        tip: Some("toes_r"),
        fixed_len_m: 0.18,
        radius_m: 0.04,
        half_height_m: 0.05,
    },
    Bone {
        joint: "arm_upper_l",
        parent: Some(1),
        tip: Some("arm_lower_l"),
        fixed_len_m: 0.25,
        radius_m: 0.04,
        half_height_m: 0.085,
    },
    Bone {
        joint: "arm_lower_l",
        parent: Some(9),
        tip: Some("palm_l"),
        fixed_len_m: 0.20,
        radius_m: 0.035,
        half_height_m: 0.065,
    },
    Bone {
        joint: "palm_l",
        parent: Some(10),
        tip: Some("middle_lower_l"),
        fixed_len_m: 0.10,
        radius_m: 0.03,
        half_height_m: 0.02,
    },
    Bone {
        joint: "arm_upper_r",
        parent: Some(1),
        tip: Some("arm_lower_r"),
        fixed_len_m: 0.25,
        radius_m: 0.04,
        half_height_m: 0.085,
    },
    Bone {
        joint: "arm_lower_r",
        parent: Some(12),
        tip: Some("palm_r"),
        fixed_len_m: 0.20,
        radius_m: 0.035,
        half_height_m: 0.065,
    },
    Bone {
        joint: "palm_r",
        parent: Some(13),
        tip: Some("middle_lower_r"),
        fixed_len_m: 0.10,
        radius_m: 0.03,
        half_height_m: 0.02,
    },
];

/// The physical rig: the reduced skeleton and one capsule per bone, validated once. `None` only
/// if the table above is wrong (a test pins that it is not).
fn physical_rig() -> Option<FxRig> {
    let skeleton = Skeleton {
        joints: BONES
            .iter()
            .map(|bone| Joint {
                name: bone.joint.to_string(),
                parent: bone.parent,
            })
            .collect(),
    };
    let shapes = BONES
        .iter()
        .map(|bone| BoneShape {
            radius: bone.radius_m,
            half_height: bone.half_height_m,
        })
        .collect();
    FxRig::new(skeleton, shapes).ok()
}

/// How the physical bones sit in one CT7 skeleton, found by joint name once at load.
#[derive(Clone, Debug, PartialEq)]
struct Layout {
    /// CT7 joint of each physical bone.
    joint_of: Vec<usize>,
    /// CT7 tip joint of each physical bone, if it has one.
    tip_of: Vec<Option<usize>>,
    /// Physical bone of each CT7 joint, if it has one.
    bone_of: Vec<Option<usize>>,
}

impl Layout {
    /// The layout of `skeleton`, or `None` when a named joint is missing or the root is not the
    /// pelvis. Cold path.
    fn find(skeleton: &Skeleton) -> Option<Self> {
        let at = |name: &str| skeleton.joints.iter().position(|joint| joint.name == name);
        let mut joint_of = Vec::with_capacity(BONE_COUNT);
        let mut tip_of = Vec::with_capacity(BONE_COUNT);
        let mut bone_of = vec![None; skeleton.len()];
        for (index, bone) in BONES.iter().enumerate() {
            let joint = at(bone.joint)?;
            let tip = match bone.tip {
                Some(name) => Some(at(name)?),
                None => None,
            };
            joint_of.push(joint);
            tip_of.push(tip);
            bone_of[joint] = Some(index);
        }
        Some(Self {
            joint_of,
            tip_of,
            bone_of,
        })
    }
}

/// What the sim's blow gives the fall: a planar direction away from the attacker and the damage
/// in permille, and the slot as the look held it just before the fall (its act, clip clock and
/// switch offsets), which is the pose the viewer saw last. Plain data.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Kick {
    dir: [f32; 2],
    damage: u16,
    standing: SlotAnim,
}

/// One live ragdoll of a slot.
#[derive(Clone, Debug)]
struct Body {
    /// Look rig (skeleton variant) the bones were taken from.
    rig: usize,
    /// The standing pose it started from: the rigid joints and the limb lengths come from here.
    seed: LocalPose,
    /// Capsule center along its bone, per physical bone, meters.
    center: [f32; BONE_COUNT],
    /// Ticks stepped since the spawn.
    age: u32,
    /// Height of the root capsule at the spawn, meters: where a tether holds the body up from.
    hold_y: f32,
}

/// The ragdoll layer of a [`FighterLook`]: one float world, one body per fallen slot.
pub(super) struct RagdollLook {
    on: bool,
    tier: Tier,
    /// The world, built at the first knockout.
    fx: Option<PhysFx>,
    rig: Option<FxRig>,
    /// One layout per look rig, `None` where the skeleton does not carry the table.
    layouts: Vec<Option<Layout>>,
    bodies: [Option<Body>; MAX_LOOK_FIGHTERS],
    pending: [Option<Kick>; MAX_LOOK_FIGHTERS],
    /// True from a slot's fall until the sim says it is up again, so one fall spawns once.
    latched: [bool; MAX_LOOK_FIGHTERS],
    /// The sim says the slot may stand up.
    stand: [bool; MAX_LOOK_FIGHTERS],
    /// One offset set per look slot for the get-up blend, built at load with the longer halflife;
    /// empty if a rig is past the lane cap (the get-up is then a plain cut).
    getup: Vec<Inertial>,
    /// Ticks since each slot's get-up began, [`NO_GETUP`] when none is in flight.
    getup_age: [u32; MAX_LOOK_FIGHTERS],
    /// The body of the slot was made by a tether, not by a fall: it gets up once it has landed.
    flight: [bool; MAX_LOOK_FIGHTERS],
    /// A tether carries the slot's body this tick.
    towed: [bool; MAX_LOOK_FIGHTERS],
    /// Ticks of level carry left after the tether ended.
    landing: [u32; MAX_LOOK_FIGHTERS],
    stepped: Option<u32>,
    spawned: u32,
    refused: u32,
}

impl RagdollLook {
    /// An empty layer on `tier`; the layouts are found once per rig skeleton.
    fn new(layouts: Vec<Option<Layout>>, getup: Vec<Inertial>) -> Self {
        Self {
            on: true,
            tier: DEFAULT_TIER,
            fx: None,
            rig: physical_rig(),
            layouts,
            bodies: std::array::from_fn(|_| None),
            pending: [None; MAX_LOOK_FIGHTERS],
            latched: [false; MAX_LOOK_FIGHTERS],
            stand: [false; MAX_LOOK_FIGHTERS],
            getup,
            getup_age: [NO_GETUP; MAX_LOOK_FIGHTERS],
            flight: [false; MAX_LOOK_FIGHTERS],
            towed: [false; MAX_LOOK_FIGHTERS],
            landing: [0; MAX_LOOK_FIGHTERS],
            stepped: None,
            spawned: 0,
            refused: 0,
        }
    }

    /// A layer that never spawns: for the tests' looks that have no rigs.
    #[cfg(test)]
    pub(super) fn empty() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    fn live(&self) -> usize {
        self.bodies.iter().flatten().count()
    }

    fn has(&self, slot: usize) -> bool {
        self.bodies.get(slot).is_some_and(Option::is_some)
    }

    /// Drops the world and every body; the layer starts over on the same settings.
    fn clear(&mut self) {
        self.fx = None;
        self.bodies = std::array::from_fn(|_| None);
        self.pending = [None; MAX_LOOK_FIGHTERS];
        self.latched = [false; MAX_LOOK_FIGHTERS];
        self.stand = [false; MAX_LOOK_FIGHTERS];
        self.getup_age = [NO_GETUP; MAX_LOOK_FIGHTERS];
        self.flight = [false; MAX_LOOK_FIGHTERS];
        self.towed = [false; MAX_LOOK_FIGHTERS];
        self.landing = [0; MAX_LOOK_FIGHTERS];
        self.stepped = None;
    }

    /// True when the slot's body may stand up as far as a tether is concerned: no tether holds
    /// it, its level carry is over and, for a body a tether made, its root has slowed down.
    fn landed(&self, slot: usize) -> bool {
        if self.towed[slot] || self.landing[slot] > 0 {
            return false;
        }
        if !self.flight[slot] {
            return true;
        }
        let speed = u8::try_from(slot)
            .ok()
            .and_then(|slot| self.fx.as_ref()?.ragdoll_speed(slot));
        speed.is_none_or(|speed| speed < LANDED_MS)
    }

    /// Drags the slot's root toward `to` at `velocity`; `lift` holds it up, else it may fall.
    fn carry(&mut self, slot: usize, to: [f32; 3], velocity: [f32; 3], lift: bool) {
        if let (Ok(slot), Some(fx)) = (u8::try_from(slot), self.fx.as_mut()) {
            // A refusal (non-finite track) leaves the body free; the next tick tries again.
            let _ = fx.carry_ragdoll(slot, to, velocity, lift);
        }
    }

    /// The world, made on first use: the sand slab and nothing else.
    fn world(&mut self) -> Option<&mut PhysFx> {
        if self.fx.is_none() {
            let mut fx = PhysFx::new(self.tier).with_clock(wall_micros);
            fx.add_static_box(
                [FLOOR_HALF_M, FLOOR_THICK_M, FLOOR_HALF_M],
                [0.0, -FLOOR_THICK_M, 0.0],
            )
            .ok()?;
            self.fx = Some(fx);
        }
        self.fx.as_mut()
    }
}

/// Microseconds on the wall clock since the first call: the step cost the receipt prints. Evidence
/// only; nothing hashed reads it.
fn wall_micros() -> u64 {
    u64::try_from(engine_clock::now().as_micros()).unwrap_or(u64::MAX)
}

// --- small float helpers; the look's own are private to their modules ---------------------------

fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

fn unit(q: [f32; 4]) -> [f32; 4] {
    let norm = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    if norm.is_finite() && norm > 1e-6 {
        q.map(|c| c / norm)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let p = quat_mul(quat_mul(q, [v[0], v[1], v[2], 0.0]), conj(q));
    [p[0], p[1], p[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The turn about +Y by `yaw` that `root_matrix` makes: forward +Z goes to `(sin, 0, cos)`.
fn yaw_quat(yaw: f32) -> [f32; 4] {
    let (s, c) = (yaw * 0.5).sin_cos();
    [0.0, s, 0.0, c]
}

/// World joint places and turns of a local pose, parents first.
fn forward(skeleton: &Skeleton, pose: &LocalPose) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let mut pos = vec![[0.0f32; 3]; skeleton.len()];
    let mut rot = vec![[0.0f32, 0.0, 0.0, 1.0]; skeleton.len()];
    for (index, joint) in skeleton.joints.iter().enumerate() {
        match joint.parent {
            Some(parent) => {
                pos[index] = add(pos[parent], rotate(rot[parent], pose.positions[index]));
                rot[index] = unit(quat_mul(rot[parent], pose.rotations[index]));
            }
            None => {
                pos[index] = pose.positions[index];
                rot[index] = unit(pose.rotations[index]);
            }
        }
    }
    (pos, rot)
}

/// Planar unit direction of `(dx, dz)`; zero for a degenerate or non-finite one.
fn planar_unit(dx: f32, dz: f32) -> [f32; 2] {
    let len = dx.hypot(dz);
    if len.is_finite() && len > 1e-4 {
        [dx / len, dz / len]
    } else {
        [0.0, 0.0]
    }
}

/// Everything a spawn reads: the standing local pose in the slot's root frame `(pos, yaw)`, the
/// fighter's own velocity and the blow.
struct Start<'a> {
    rig: usize,
    skeleton: &'a Skeleton,
    pose: &'a LocalPose,
    root: ([f32; 3], f32),
    velocity: [f32; 3],
    kick: Kick,
}

impl RagdollLook {
    /// Spawns the ragdoll of `slot` from `start`.
    fn spawn(&mut self, slot: usize, start: &Start<'_>) -> Result<(), FxError> {
        let Start {
            rig,
            skeleton,
            pose,
            root,
            velocity,
            kick,
        } = *start;
        let layout = self
            .layouts
            .get(rig)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or(FxError::BadCaps)?;
        let (wp, wq) = forward(skeleton, pose);
        let turn = yaw_quat(root.1);
        let world_pos: Vec<[f32; 3]> = wp.iter().map(|p| add(root.0, rotate(turn, *p))).collect();
        let world_rot: Vec<[f32; 4]> = wq.iter().map(|q| unit(quat_mul(turn, *q))).collect();
        // Capsule centers: halfway from the bone's joint to its tip, along the bone's own Y.
        let mut center = [0.0f32; BONE_COUNT];
        let mut place: Vec<Transform> = Vec::with_capacity(BONE_COUNT);
        for (index, bone) in BONES.iter().enumerate() {
            let joint = layout.joint_of[index];
            let len = match layout.tip_of[index] {
                Some(tip) => length(sub(world_pos[tip], world_pos[joint])),
                None => bone.fixed_len_m,
            };
            center[index] = 0.5 * len.clamp(0.04, 1.5);
            place.push(Transform {
                pos: add(
                    world_pos[joint],
                    rotate(world_rot[joint], [0.0, center[index], 0.0]),
                ),
                rot: world_rot[joint],
            });
        }
        let mut local = LocalPose {
            positions: vec![[0.0; 3]; BONE_COUNT],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; BONE_COUNT],
        };
        for (index, bone) in BONES.iter().enumerate() {
            match bone.parent {
                Some(parent) => {
                    let up = conj(place[parent].rot);
                    local.positions[index] = rotate(up, sub(place[index].pos, place[parent].pos));
                    local.rotations[index] = unit(quat_mul(up, place[index].rot));
                }
                None => {
                    local.positions[index] = place[index].pos;
                    local.rotations[index] = place[index].rot;
                }
            }
        }
        let mass_kick = f32::from(kick.damage.min(MAX_DAMAGE_PERMILLE)) * NS_PER_PERMILLE;
        let impulse = [
            kick.dir[0] * mass_kick,
            mass_kick * KICK_LIFT,
            kick.dir[1] * mass_kick,
        ];
        let shape = self.rig.take().ok_or(FxError::BadCaps)?;
        let result = match (u8::try_from(slot), self.world()) {
            (Ok(slot), Some(fx)) => fx.spawn_ragdoll(slot, &shape, &local, velocity, impulse),
            _ => Err(FxError::BadCaps),
        };
        self.rig = Some(shape);
        result?;
        // A new fall ends any get-up still in flight.
        self.getup_age[slot] = NO_GETUP;
        self.bodies[slot] = Some(Body {
            rig,
            seed: pose.clone(),
            center,
            age: 0,
            hold_y: place[0].pos[1],
        });
        self.spawned = self.spawned.saturating_add(1);
        Ok(())
    }

    /// Steps the world one tick and forgets slots whose bones the world recycled.
    fn step(&mut self) {
        let Some(fx) = self.fx.as_mut() else {
            return;
        };
        fx.step();
        let mut alive = [false; MAX_LOOK_FIGHTERS];
        if let Ok(placed) = fx.transforms(1.0) {
            for one in placed {
                if let Key::Bone { slot, .. } = one.key {
                    if let Some(flag) = alive.get_mut(usize::from(slot)) {
                        *flag = true;
                    }
                }
            }
        }
        for (body, alive) in self.bodies.iter_mut().zip(alive) {
            match body {
                Some(live) if alive => live.age = live.age.saturating_add(1),
                _ => *body = None,
            }
        }
    }

    /// The slot's bones at `alpha` as a local pose in the slot's root frame `(pos, yaw)`, or
    /// `None` when it has no ragdoll (or the world recycled it).
    fn pose(
        &self,
        slot: usize,
        skeleton: &Skeleton,
        alpha: f32,
        root: ([f32; 3], f32),
    ) -> Option<LocalPose> {
        let body = self.bodies.get(slot)?.as_ref()?;
        let layout = self.layouts.get(body.rig)?.as_ref()?;
        let placed = self.fx.as_ref()?.transforms(alpha).ok()?;
        let mut bones: [Option<Transform>; BONE_COUNT] = [None; BONE_COUNT];
        for one in placed {
            if let Key::Bone { slot: s, bone } = one.key {
                if usize::from(s) == slot {
                    *bones.get_mut(usize::from(bone))? = Some(one.at);
                }
            }
        }
        let back = conj(yaw_quat(root.1));
        let mut out = body.seed.clone();
        // Root-frame place and turn of every joint so far. The joints above the pelvis and all
        // the non-physical ones keep their standing local transforms; each physical bone takes
        // the turn of its capsule, and only the pelvis takes a place from physics, so the limbs
        // keep their standing lengths.
        let mut turn = vec![[0.0f32, 0.0, 0.0, 1.0]; skeleton.len()];
        let mut place = vec![[0.0f32; 3]; skeleton.len()];
        for (joint, entry) in skeleton.joints.iter().enumerate() {
            let bone = layout.bone_of[joint];
            let rot = match (bone, entry.parent) {
                (Some(bone), _) => unit(quat_mul(back, bones[bone]?.rot)),
                (None, Some(parent)) => unit(quat_mul(turn[parent], body.seed.rotations[joint])),
                (None, None) => body.seed.rotations[joint],
            };
            let at = if bone == Some(0) {
                // The pelvis follows its capsule, whose center sits `center` up its own Y.
                let center = rotate(back, sub(bones[0]?.pos, root.0));
                sub(center, rotate(rot, [0.0, body.center[0], 0.0]))
            } else if let Some(parent) = entry.parent {
                add(
                    place[parent],
                    rotate(turn[parent], body.seed.positions[joint]),
                )
            } else {
                body.seed.positions[joint]
            };
            if bone.is_some() {
                out.rotations[joint] = match entry.parent {
                    Some(parent) => unit(quat_mul(conj(turn[parent]), rot)),
                    None => rot,
                };
            }
            if bone == Some(0) {
                out.positions[joint] = match entry.parent {
                    Some(parent) => rotate(conj(turn[parent]), sub(at, place[parent])),
                    None => at,
                };
            }
            turn[joint] = rot;
            place[joint] = at;
        }
        let finite = out.rotations.iter().flatten().all(|c| c.is_finite())
            && out.positions.iter().flatten().all(|c| c.is_finite());
        finite.then_some(out)
    }
}

impl FighterLook {
    /// Switches the ragdoll layer. Off, a floored fighter lies down on the flat death pose as
    /// before RZ2w. Turning it off drops every ragdoll. Presentation only.
    pub fn set_ragdoll(&mut self, on: bool) {
        self.ragdoll.on = on;
        if !on {
            self.ragdoll.clear();
        }
    }

    /// Sets the hardware tier whose caps bound the ragdolls (potato 2, web 4, comfort 8, high
    /// 16). Drops the ragdolls alive: call it at start up, not mid-fight.
    pub fn set_ragdoll_tier(&mut self, tier: Tier) {
        self.ragdoll.tier = tier;
        self.ragdoll.clear();
    }

    /// Slots drawn as a ragdoll now.
    #[must_use]
    pub fn ragdolls(&self) -> usize {
        self.ragdoll.live()
    }

    /// Ragdolls spawned since load.
    #[must_use]
    pub fn ragdoll_spawns(&self) -> u32 {
        self.ragdoll.spawned
    }

    /// Spawns the physical layer refused since load (a cap too small for one body, bad numbers).
    #[must_use]
    pub fn ragdoll_refused(&self) -> u32 {
        self.ragdoll.refused
    }

    /// The world's counters as the one `physfx:` receipt line, or `None` before the first
    /// knockout.
    #[must_use]
    pub fn ragdoll_receipt(&self) -> Option<String> {
        self.ragdoll.fx.as_ref().map(|fx| fx.stats().receipt_line())
    }

    /// True while `slot` is drawn from its ragdoll.
    pub(super) fn ragdolled(&self, slot: usize) -> bool {
        self.ragdoll.has(slot)
    }

    /// True while `slot` lies by physics (a ragdoll) or is still rising out of one: the sand
    /// rest then only lifts a sunk body and never pulls a flying or rising one down.
    pub(super) fn ragdoll_rests(&self, slot: usize) -> bool {
        self.ragdoll.has(slot)
            || self
                .ragdoll
                .getup_age
                .get(slot)
                .is_some_and(|age| *age < GETUP_TICKS)
    }

    /// The layouts of a freshly loaded look's rigs, in rig order.
    pub(super) fn ragdoll_for_rigs(rigs: &[Rig]) -> RagdollLook {
        let layouts = rigs.iter().map(|rig| Layout::find(&rig.skeleton)).collect();
        let sets: Option<Vec<Inertial>> = (0..MAX_LOOK_FIGHTERS)
            .map(|slot| {
                let rig = rigs.get(rig_index(rigs.len(), slot))?;
                Inertial::new(2 * rig.skeleton.len(), GETUP_HALFLIFE_S).ok()
            })
            .collect();
        RagdollLook::new(layouts, sets.unwrap_or_default())
    }

    /// A `Knockdown` cue floors `victim`: queue its ragdoll with the blow of this tick's `Hit`
    /// on it, if any. Called from the cue fold.
    pub(super) fn ragdoll_note_knockdown(&mut self, attacker: u8, victim: u8, cues: &[FightCue]) {
        let index = usize::from(victim);
        if !self.ragdoll.on || index >= MAX_LOOK_FIGHTERS || self.ragdoll.has(index) {
            return;
        }
        let damage = cues
            .iter()
            .find_map(|cue| match *cue {
                FightCue::Hit {
                    victim: v, damage, ..
                } if v == victim => Some(damage),
                _ => None,
            })
            .unwrap_or(DEFAULT_DAMAGE_PERMILLE);
        let dir = match (self.slots.get(usize::from(attacker)), self.slots.get(index)) {
            (Some(a), Some(v)) if a.seen && v.seen => {
                planar_unit(v.pos[0] - a.pos[0], v.pos[2] - a.pos[2])
            }
            _ => [0.0, 0.0],
        };
        let standing = self.slots[index];
        self.ragdoll.pending[index] = Some(Kick {
            dir,
            damage,
            standing,
        });
        self.ragdoll.latched[index] = true;
    }

    /// The sim's gait for `slot` this tick: `Fallen` queues a ragdoll once per fall; a gait that
    /// is not `Fallen` lets a ragdoll that has lain long enough stand up. Returns the gait the
    /// look should show: while a slot rises out of its ragdoll it shows `Steady`, so the
    /// sim's kneel and lean steps do not move the pose the get-up blends toward.
    pub(super) fn ragdoll_fold_gait(&mut self, slot: u8, gait: GaitPhase) -> GaitPhase {
        let index = usize::from(slot);
        if !self.ragdoll.on || index >= MAX_LOOK_FIGHTERS {
            return gait;
        }
        if gait == GaitPhase::Fallen {
            if !self.ragdoll.latched[index] && !self.ragdoll.has(index) {
                // No cue explains this fall: push along the last blow if it is fresh.
                let anim = self.slots[index];
                let fresh = self.tick.wrapping_sub(anim.hit_start) <= HIT_EXPLAINS_TICKS;
                let dir = if fresh { anim.hit_push } else { [0.0, 0.0] };
                self.ragdoll.pending[index] = Some(Kick {
                    dir,
                    damage: DEFAULT_DAMAGE_PERMILLE,
                    standing: anim,
                });
                self.ragdoll.latched[index] = true;
            }
            self.ragdoll.stand[index] = false;
            // The slot is floored when its ragdoll spawns (it may wait out a hitstop), never
            // before: until then it keeps the pose it shows.
            let waits = self.ragdoll.pending[index].is_some();
            return if waits {
                self.slots[index].balance
            } else {
                gait
            };
        }
        self.ragdoll.latched[index] = false;
        let lying = self.ragdoll.bodies[index]
            .as_ref()
            .is_some_and(|body| body.age >= MIN_LYING_TICKS);
        self.ragdoll.stand[index] =
            lying && self.slots[index].act != Act::Down && self.ragdoll.landed(index);
        if self.ragdoll.stand[index] {
            self.slots[index].balance = GaitPhase::Steady;
        }
        let rising = self.ragdoll_rests(index) && !self.ragdoll.has(index);
        if self.ragdoll.stand[index] || rising {
            GaitPhase::Steady
        } else {
            gait
        }
    }

    /// Once per accepted tick: spawn what was queued, step the world one tick and stand up the
    /// slots the sim has recovered. Safe to call more than once per tick: the world steps once.
    pub(super) fn ragdoll_tick(&mut self) {
        if !self.ragdoll.on {
            return;
        }
        let fresh = self.ragdoll.stepped != Some(self.tick);
        self.ragdoll.stepped = Some(self.tick);
        for index in 0..MAX_LOOK_FIGHTERS {
            let Some(kick) = self.ragdoll.pending[index] else {
                continue;
            };
            if self.slots[index].held {
                // Hitstop (LF10): the pair's poses stand still for the freeze, so the fall waits
                // for it. The held slot is the frozen pose, which is what the fall starts from.
                self.ragdoll.pending[index] = Some(Kick {
                    standing: self.slots[index],
                    ..kick
                });
                continue;
            }
            self.ragdoll.pending[index] = None;
            self.ragdoll_spawn(index, kick);
        }
        if fresh {
            // The get-up clock runs with the clip clock: a held slot's age stands still (LF10).
            for (age, slot) in self.ragdoll.getup_age.iter_mut().zip(&self.slots) {
                if *age != NO_GETUP && !slot.held {
                    *age = if *age + 1 < GETUP_TICKS {
                        *age + 1
                    } else {
                        NO_GETUP
                    };
                }
            }
            // A ragdoll in hitstop (LF10) stands still with the pair it belongs to: the world
            // waits out the freeze, so the held pose is one pose.
            let held = (0..MAX_LOOK_FIGHTERS).any(|i| self.ragdoll.has(i) && self.slots[i].held);
            if self.ragdoll.live() > 0 && !held {
                self.ragdoll.step();
            }
        }
        for index in 0..MAX_LOOK_FIGHTERS {
            if std::mem::take(&mut self.ragdoll.stand[index]) {
                self.ragdoll_stand_up(index);
            }
        }
    }

    /// Hands the standing pose of `index` to the world and floors the slot: whatever the world
    /// answers, the slot lies down (on its ragdoll, or flat if it was refused).
    fn ragdoll_spawn(&mut self, index: usize, kick: Kick) {
        self.ragdoll_try_spawn(index, kick);
        self.floor_slot(u8::try_from(index).unwrap_or(u8::MAX));
    }

    fn ragdoll_try_spawn(&mut self, index: usize, kick: Kick) {
        let rig_index = self.rig_for_slot(index);
        let Some(rig) = self.rigs.get(rig_index) else {
            return;
        };
        // The pose the viewer saw last: the slot as it stood before the fall, at the start of
        // this tick. The sim's floor verdict and the death act are what the flat pose reads, so
        // the seed leaves them out.
        let mut standing = kick.standing;
        if !standing.seen {
            return;
        }
        // A pose frozen by hitstop is read as its tick ends; from here the clock runs free.
        standing.held = false;
        if standing.balance == GaitPhase::Fallen {
            standing.balance = GaitPhase::Steady;
        }
        if standing.act == Act::Down {
            standing.act = Act::Idle;
            standing.act_start = self.tick;
        }
        let Ok(pose) = self.inertial_pose(rig, index, &standing, 0.0) else {
            return;
        };
        let root = slot_root(&standing, 0.0);
        let velocity = [
            (standing.pos[0] - standing.prev_pos[0]) * TICK_HZ,
            0.0,
            (standing.pos[2] - standing.prev_pos[2]) * TICK_HZ,
        ]
        .map(|v| {
            if v.is_finite() {
                v.clamp(-MAX_START_MS, MAX_START_MS)
            } else {
                0.0
            }
        });
        let start = Start {
            rig: rig_index,
            skeleton: &rig.skeleton,
            pose: &pose,
            root,
            velocity,
            kick,
        };
        let result = self.ragdoll.spawn(index, &start);
        if result.is_err() {
            self.ragdoll.refused = self.ragdoll.refused.saturating_add(1);
        }
    }

    /// The get-up: the last two ragdoll frames and the pose the clip shows next start the same
    /// kind of offsets a clip switch does (LF9, here with a longer halflife), so the body rises
    /// out of the ragdoll pose with no pop. The slot's balance already left `Fallen` (set when
    /// the stand-up was granted).
    fn ragdoll_stand_up(&mut self, index: usize) {
        let rig_index = self.rig_for_slot(index);
        // No clip switch is in flight under the get-up: its layer carries the whole blend.
        self.slots[index].iz_age = NO_OFFSET;
        let slot = self.slots[index];
        let shown = self.rigs.get(rig_index).and_then(|rig| {
            let before = self
                .ragdoll
                .pose(index, &rig.skeleton, 0.0, slot_root(&slot, 0.0))?;
            let now = self
                .ragdoll
                .pose(index, &rig.skeleton, 1.0, slot_root(&slot, 1.0))?;
            Some((before, now))
        });
        // The ragdoll is gone from the next frame on, whatever the blend does.
        self.ragdoll.bodies[index] = None;
        self.ragdoll.flight[index] = false;
        let Some((before, now)) = shown else {
            return;
        };
        let arriving = self.rigs.get(rig_index).and_then(|rig| {
            let now = self.inertial_pose_base(rig, index, &slot, 0.0).ok()?;
            let next = self.inertial_pose_base(rig, index, &slot, 1.0).ok()?;
            Some((now, next))
        });
        let started = arriving.and_then(|(arriving_now, arriving_next)| {
            let set = self.ragdoll.getup.get_mut(index)?;
            begin_switch(set, [&before, &now], 1.0, [&arriving_now, &arriving_next]).ok()
        });
        self.ragdoll.getup_age[index] = if started.is_some() { 0 } else { NO_GETUP };
    }

    /// Adds the get-up offsets of `index` onto `pose` while one is in flight. The age is the
    /// ticks since the get-up began plus the frame's share. Errors: the set's refusal.
    pub(super) fn ragdoll_getup_offsets(
        &self,
        index: usize,
        pose: &mut LocalPose,
        alpha: f32,
    ) -> Result<(), super::LookError> {
        let age = self
            .ragdoll
            .getup_age
            .get(index)
            .copied()
            .unwrap_or(NO_GETUP);
        if age >= GETUP_TICKS {
            return Ok(());
        }
        let Some(set) = self.ragdoll.getup.get(index) else {
            return Ok(());
        };
        let held = self.slots.get(index).is_some_and(|slot| slot.held);
        let alpha = if held { 0.0 } else { alpha };
        let ticks = f32::from(u16::try_from(age).unwrap_or(u16::MAX));
        apply_offsets(set, pose, (ticks + alpha).max(0.0) / TICK_HZ)
            .map_err(|err| super::LookError::Skin(err.to_string()))
    }

    /// A tether carries these bodies this tick (`Some(lift)`, meters the arc holds the body above
    /// its standing height) and lets go of the rest. A carried slot with no ragdoll gets one at
    /// the next tick (it keeps the pose the viewer saw); a ragdoll follows the sim's own track
    /// with a soft spring, so the limbs trail and swing; when the tether lets go the body keeps
    /// following the slide, level, for [`LAND_CARRY_TICKS`], falls, lands, and stands up through
    /// the usual get-up. Call it once per accepted tick before [`Self::observe_live_balances`].
    /// Presentation only: the sim never reads this.
    pub fn observe_carries(&mut self, tows: &[Option<f32>; MAX_LOOK_FIGHTERS]) {
        if !self.ragdoll.on {
            return;
        }
        for (index, tow) in tows.iter().enumerate() {
            let slot = self.slots[index];
            if !slot.seen {
                continue;
            }
            let track = [slot.pos[0], 0.0, slot.pos[2]];
            let speed = [
                (slot.pos[0] - slot.prev_pos[0]) * TICK_HZ,
                0.0,
                (slot.pos[2] - slot.prev_pos[2]) * TICK_HZ,
            ];
            match *tow {
                Some(lift) => {
                    self.ragdoll.towed[index] = true;
                    if !self.ragdoll.has(index) {
                        if self.ragdoll.pending[index].is_none() {
                            self.ragdoll.pending[index] = Some(Kick {
                                dir: [0.0, 0.0],
                                damage: 0,
                                standing: slot,
                            });
                            self.ragdoll.flight[index] = true;
                        }
                        continue;
                    }
                    let hold = self.ragdoll.bodies[index]
                        .as_ref()
                        .map_or(0.0, |b| b.hold_y);
                    let up = if self.ragdoll.flight[index] {
                        lift
                    } else {
                        0.0
                    };
                    self.ragdoll
                        .carry(index, [track[0], hold + up, track[2]], speed, true);
                }
                None => {
                    if std::mem::take(&mut self.ragdoll.towed[index]) {
                        self.ragdoll.landing[index] = LAND_CARRY_TICKS;
                    }
                    if self.ragdoll.landing[index] > 0 {
                        self.ragdoll.landing[index] -= 1;
                        self.ragdoll.carry(index, track, speed, false);
                    }
                }
            }
        }
    }

    /// Where each ragdolled slot's root bone is now, meters: what a rope ties to.
    #[must_use]
    pub fn ragdoll_roots(&self) -> [Option<[f32; 3]>; MAX_LOOK_FIGHTERS] {
        std::array::from_fn(|index| {
            let slot = u8::try_from(index).ok()?;
            self.ragdoll.has(index).then_some(())?;
            self.ragdoll.fx.as_ref()?.ragdoll_root(slot)
        })
    }

    /// The bones of a ragdolled `slot` at `alpha` in its root frame, or `None`.
    pub(super) fn ragdoll_pose(
        &self,
        index: usize,
        rig: &Rig,
        slot: &super::SlotAnim,
        alpha: f32,
    ) -> Option<LocalPose> {
        if !self.ragdoll.on || !self.ragdoll.has(index) {
            return None;
        }
        // A held slot reads the end of its tick, like its clip clock does.
        self.ragdoll.pose(
            index,
            &rig.skeleton,
            super::clip_alpha(slot, alpha),
            slot_root(slot, alpha),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_truth::{BodyTruth, ViewLaws, GROUND_Y};
    use crate::FighterView;
    use engine_render::v1::Camera;
    use std::path::PathBuf;

    fn content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    fn camera() -> Camera {
        Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        )
    }

    /// `n` fighters on a ring of 3 m, each facing the middle.
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

    fn hit(attacker: u8, victim: u8) -> FightCue {
        FightCue::Hit {
            attacker,
            victim,
            damage: 300,
        }
    }

    fn knock(attacker: u8, victim: u8) -> FightCue {
        FightCue::Knockdown { attacker, victim }
    }

    /// One scripted tick without a live match: fold the tick, then the ragdoll tick.
    fn tick(
        look: &mut FighterLook,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
    ) {
        look.observe_tick(tick, views, hp, cues);
        look.ragdoll_tick();
    }

    fn joints(look: &FighterLook, fighters: usize, alpha: f32) -> Vec<BodyTruth> {
        look.pose_frame(fighters, alpha)
            .expect("CPU pose")
            .into_iter()
            .map(|body| body.truth)
            .collect()
    }

    fn speed_ms(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(p, q)| length(sub(*p, *q)) * TICK_HZ)
            .fold(0.0, f32::max)
    }

    #[test]
    fn ragdoll_look_table_builds_and_fits_every_ct7_skeleton() {
        let rig = physical_rig().expect("the bone table builds a rig");
        assert_eq!(rig.bones(), BONE_COUNT);
        for (index, bone) in BONES.iter().enumerate() {
            if let Some(parent) = bone.parent {
                assert!(parent < index, "{} lists its parent first", bone.joint);
            }
            let capsule = 2.0 * (bone.half_height_m + bone.radius_m);
            assert!(
                (capsule - bone.fixed_len_m).abs() < 0.07,
                "{}: capsule {capsule:.3} m against bone {:.3} m",
                bone.joint,
                bone.fixed_len_m
            );
        }
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        assert!(!look.rigs.is_empty());
        for (index, rig) in look.rigs.iter().enumerate() {
            let layout = look.ragdoll.layouts[index]
                .as_ref()
                .unwrap_or_else(|| panic!("rig {} lacks a table bone", rig.variant));
            assert_eq!(layout.joint_of.len(), BONE_COUNT);
            // The table's parent is the nearest physical ancestor in the CT7 chain.
            for (bone, parent) in BONES.iter().map(|b| b.parent).enumerate() {
                let mut up = rig.skeleton.joints[layout.joint_of[bone]].parent;
                let mut found = None;
                while let Some(joint) = up {
                    if let Some(p) = layout.bone_of[joint] {
                        found = Some(p);
                        break;
                    }
                    up = rig.skeleton.joints[joint].parent;
                }
                assert_eq!(found, parent, "{} bone {bone}", rig.variant);
            }
        }
    }

    /// What a scripted fall measured on slot 1.
    struct Fall {
        joints: Vec<Vec<[f32; 3]>>,
        lowest_joint_m: f32,
        lowest_joint: usize,
        lowest_mesh_m: f32,
        fastest_ms: f32,
        awake_at: Vec<usize>,
    }

    /// Slot 1 is struck dead at tick `start` by slot 0 and falls; the run goes `ticks` ticks on.
    fn fall(look: &mut FighterLook, start: u32, ticks: u32, views: &[FighterView]) -> Fall {
        let n = views.len();
        let mut hp = vec![1000u16; n];
        let mut out = Fall {
            joints: Vec::new(),
            lowest_joint_m: f32::MAX,
            lowest_joint: 0,
            lowest_mesh_m: f32::MAX,
            fastest_ms: 0.0,
            awake_at: Vec::new(),
        };
        for t in 0..start + ticks {
            let cues = if t == start {
                hp[1] = 0;
                vec![hit(0, 1), knock(0, 1)]
            } else {
                Vec::new()
            };
            tick(look, t, views, &hp, &cues);
            if t < start {
                continue;
            }
            for alpha in [0.0f32, 0.5, 1.0] {
                let truth = joints(look, n, alpha);
                let body = truth.iter().find(|b| b.slot == 1).expect("slot 1 drawn");
                if t > start {
                    // World height: the model-space joint plus the body's root, which carries
                    // the sand rest (a sunk body is lifted).
                    for (index, joint) in body.joints.iter().enumerate() {
                        let y = joint[1] + body.root[1];
                        if y < out.lowest_joint_m {
                            out.lowest_joint_m = y;
                            out.lowest_joint = index;
                        }
                    }
                    out.lowest_mesh_m = out.lowest_mesh_m.min(body.min[1]);
                }
                if alpha == 1.0 {
                    if let Some(last) = out.joints.last() {
                        out.fastest_ms = out.fastest_ms.max(speed_ms(&body.joints, last));
                    }
                    out.joints.push(body.joints.clone());
                }
            }
            out.awake_at
                .push(look.ragdoll.fx.as_ref().map_or(0, |fx| fx.stats().awake));
        }
        out
    }

    #[test]
    fn ragdoll_look_scripted_knockout_settles_in_3s_above_the_sand() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(2);
        let run = fall(&mut look, 10, 240, &views);
        assert_eq!(look.ragdolls(), 1, "the dead fighter is a ragdoll");
        assert_eq!(look.ragdoll_spawns(), 1);
        assert_eq!(look.ragdoll_refused(), 0);
        // Settled: asleep within 3 s (180 ticks) of the knockout, and still there after.
        let settle = run
            .awake_at
            .iter()
            .position(|awake| *awake == 0)
            .expect("the ragdoll fell asleep");
        let low_name = &look.rigs[look.rig_for_slot(1)].skeleton.joints[run.lowest_joint].name;
        println!(
            "ragdoll_look: settled after {settle} ticks, fastest joint {:.1} m/s, lowest joint {low_name} {:.1} cm, lowest mesh {:.1} cm, {}",
            run.fastest_ms,
            run.lowest_joint_m * 100.0,
            run.lowest_mesh_m * 100.0,
            look.ragdoll_receipt().unwrap_or_default()
        );
        assert!(settle <= 180, "asleep after {settle} ticks");
        let rest = &run.joints[settle..];
        for pair in rest.windows(2) {
            assert!(
                speed_ms(&pair[0], &pair[1]) < 0.02,
                "a settled ragdoll stands still"
            );
        }
        assert!(
            run.lowest_joint_m >= -0.02,
            "every joint above the sand: lowest {:.1} cm",
            run.lowest_joint_m * 100.0
        );
        assert!(
            run.lowest_mesh_m >= GROUND_Y - 1e-4,
            "no skin under the sand: lowest {:.1} cm",
            run.lowest_mesh_m * 100.0
        );
        assert!(
            run.fastest_ms < 25.0,
            "no explosion: {:.1} m/s",
            run.fastest_ms
        );
        // The body lies down on the sand.
        let last = run.joints.last().expect("frames");
        let top_m = last.iter().map(|j| j[1]).fold(f32::MIN, f32::max);
        assert!(
            top_m < 0.8,
            "highest joint {top_m:.2} m: the body lies down"
        );
        // The same script twice gives the same bones.
        let mut again = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let twice = fall(&mut again, 10, 240, &views);
        assert_eq!(run.joints, twice.joints, "ragdoll x2 equal");
    }

    #[test]
    fn ragdoll_look_falls_from_every_kind_of_pose_without_blowing_up() {
        // The fall starts mid-run, mid-swing and standing: the capsules of any of them start
        // apart, so the solver never has to shove a limb out of another.
        for (name, speed_m, swing) in [
            ("idle", 0.0f32, false),
            ("run", 0.06, false),
            ("swing", 0.0, true),
        ] {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            let mut views = ring(2);
            let mut hp = [1000u16; 2];
            let mut fastest = 0.0f32;
            let mut previous: Option<Vec<[f32; 3]>> = None;
            for t in 0..260u32 {
                if t < 20 {
                    views[1].pos_m[0] += speed_m;
                }
                let cues = match t {
                    14 if swing => vec![hit(1, 0)],
                    20 => {
                        hp[1] = 0;
                        vec![hit(0, 1), knock(0, 1)]
                    }
                    _ => Vec::new(),
                };
                tick(&mut look, t, &views, &hp, &cues);
                let truth = joints(&look, 2, 1.0);
                let body = truth.iter().find(|b| b.slot == 1).expect("drawn");
                if t > 20 {
                    if let Some(last) = &previous {
                        fastest = fastest.max(speed_ms(&body.joints, last));
                    }
                    assert!(body.nonfinite == 0, "{name}: finite at tick {t}");
                }
                previous = Some(body.joints.clone());
            }
            println!("ragdoll_look: {name} fall, fastest joint {fastest:.1} m/s");
            assert_eq!(look.ragdolls(), 1, "{name}");
            assert!(fastest < 25.0, "{name}: {fastest:.1} m/s");
        }
    }

    #[test]
    fn ragdoll_look_eight_knockouts_stay_under_the_tier_cap() {
        for (tier, want) in [(Tier::Comfort, 8usize), (Tier::Web, 4), (Tier::Potato, 2)] {
            let caps = tier.caps();
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            look.set_ragdoll_tier(tier);
            let views = ring(8);
            let mut hp = [1000u16; 8];
            let mut peak_bodies = 0;
            for t in 0..240u32 {
                let mut cues = Vec::new();
                if t == 10 {
                    for victim in 0..8u8 {
                        hp[usize::from(victim)] = 0;
                        cues.push(knock((victim + 1) % 8, victim));
                    }
                }
                tick(&mut look, t, &views, &hp, &cues);
                assert!(look.ragdolls() <= caps.ragdolls, "{tier:?} tick {t}");
                if let Some(fx) = look.ragdoll.fx.as_ref() {
                    peak_bodies = peak_bodies.max(fx.stats().bodies);
                    assert!(fx.stats().bodies <= caps.bodies, "{tier:?} tick {t}");
                }
            }
            println!(
                "ragdoll_look: {tier:?} ragdolls={} (cap {}), bodies peak {peak_bodies} (cap {}), {}",
                look.ragdolls(),
                caps.ragdolls,
                caps.bodies,
                look.ragdoll_receipt().unwrap_or_default()
            );
            assert_eq!(look.ragdolls(), want.min(caps.ragdolls), "{tier:?}");
            assert_eq!(look.ragdoll_spawns(), 8, "{tier:?}: all eight knocked out");
            assert_eq!(look.ragdoll_refused(), 0, "{tier:?}");
            // Every body still draws: a slot the cap recycled falls back to the flat death pose.
            let truth = joints(&look, 8, 0.5);
            assert_eq!(truth.len(), 8, "{tier:?}");
            assert!(
                truth.iter().all(|b| b.nonfinite == 0 && b.min[1] >= -0.01),
                "{tier:?}"
            );
            let drawn = truth.iter().filter(|b| b.ragdolled).count();
            assert_eq!(
                drawn,
                look.ragdolls(),
                "{tier:?}: the receipt counts the drawn ragdolls"
            );
        }
    }

    #[test]
    fn ragdoll_look_standing_pose_overlaps_nothing_on_the_15_bone_rig() {
        // Every CT7 variant is floored out of its standing pose: no two unjoined capsules start
        // inside each other, so the solver has nothing to blast apart and the spawn filters no
        // pair. The receipt also carries one mass ratio per joint (QR2, QR4, QR5).
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(8);
        let mut hp = [1000u16; 8];
        for t in 0..20u32 {
            let mut cues = Vec::new();
            if t == 10 {
                for victim in 0..8u8 {
                    hp[usize::from(victim)] = 0;
                    cues.push(knock((victim + 1) % 8, victim));
                }
            }
            tick(&mut look, t, &views, &hp, &cues);
        }
        let fx = look
            .ragdoll
            .fx
            .as_ref()
            .expect("the knockouts built the world");
        for slot in 0..8u8 {
            let receipt = fx.ragdoll_receipt(slot).expect("every fighter fell");
            println!("ragdoll_look: slot {slot} {}", receipt.receipt_line());
            assert_eq!(
                receipt.overlaps, 0,
                "slot {slot}: the standing pose overlaps"
            );
            assert_eq!(receipt.filtered, 0, "slot {slot}");
            assert_eq!(
                receipt.mass_ratio.len(),
                BONE_COUNT - 1,
                "one ratio per joint"
            );
            assert!(receipt.mass_ratio.iter().all(|r| r.is_finite() && *r > 0.0));
        }
    }

    #[test]
    fn ragdoll_look_off_keeps_the_flat_death_pose() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.set_ragdoll(false);
        let views = ring(2);
        let run = fall(&mut look, 10, 60, &views);
        assert_eq!(look.ragdolls(), 0);
        assert_eq!(look.ragdoll_spawns(), 0);
        assert!(
            look.ragdoll_receipt().is_none(),
            "no world before a knockout"
        );
        let truth = joints(&look, 2, 1.0);
        assert!(truth.iter().all(|b| !b.ragdolled));
        let last = run.joints.last().expect("frames");
        assert!(last.iter().all(|j| j[1] < 0.9), "lies flat as before");
    }

    #[test]
    fn ragdoll_look_get_up_blends_back_through_inertial_without_a_pop() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(2);
        let mut laws = ViewLaws::default();
        let hp = [1000u16; 2];
        let (mut stood, mut previous) = (None, None::<Vec<[f32; 3]>>);
        let mut spawned_at = None;
        let (mut lying_top_m, mut jump_cm, mut lying_step_cm) = (0.0f32, 0.0f32, 0.0f32);
        for t in 0..420u32 {
            // The sim floors the gait for two seconds, then it recovers.
            let cues = if t == 10 {
                vec![hit(0, 1), knock(0, 1)]
            } else {
                Vec::new()
            };
            look.observe_tick(t, &views, &hp, &cues);
            // The sim's own timing: prone 30 ticks, catching 15, then steady.
            let gait = match t {
                10..=39 => GaitPhase::Fallen,
                40..=54 => GaitPhase::Catching,
                _ => GaitPhase::Steady,
            };
            let shown = look.ragdoll_fold_gait(1, gait);
            look.observe_balance(1, shown);
            look.ragdoll_tick();
            for alpha in [0.0f32, 0.5] {
                let truth = joints(&look, 2, alpha);
                laws.note(&camera(), f64::from(t) + f64::from(alpha), &truth);
                let body = truth.iter().find(|b| b.slot == 1).expect("drawn");
                if look.ragdolled(1) {
                    spawned_at.get_or_insert(t);
                    lying_top_m = body.joints.iter().map(|j| j[1]).fold(f32::MIN, f32::max);
                    if let Some(last) = &previous {
                        let step = speed_ms(&body.joints, last) / TICK_HZ * 100.0;
                        lying_step_cm = lying_step_cm.max(step);
                    }
                } else if stood.is_none() && spawned_at.is_some() {
                    stood = Some(t);
                    if let Some(last) = &previous {
                        jump_cm = speed_ms(&body.joints, last) / TICK_HZ * 100.0;
                    }
                }
                previous = Some(body.joints.clone());
            }
        }
        let stood = stood.expect("the sim's recovery stands the ragdoll up");
        let line = laws.line().expect("frames folded");
        println!(
            "ragdoll_look: stood up at tick {stood}, joint jump {jump_cm:.1} cm, lying top {lying_top_m:.2} m, {line}"
        );
        let spawned_at = spawned_at.expect("the fall spawned a ragdoll");
        assert!(
            stood >= spawned_at + MIN_LYING_TICKS,
            "lies at least {MIN_LYING_TICKS} ticks from {spawned_at}, stood {stood}"
        );
        assert_eq!(look.ragdolls(), 0);
        assert!(lying_top_m < 0.8);
        // The body may still be sliding when the sim stands it up (it lies 45 ticks at least);
        // the first frame out of the ragdoll moves no more than the frames before it did.
        assert!(
            jump_cm <= lying_step_cm * 1.5 + 2.0,
            "the first frame out of the ragdoll moved a joint {jump_cm:.1} cm; lying frames moved {lying_step_cm:.1} cm"
        );
        assert!(
            line.contains(" pops=0 "),
            "no clip snap on the way up: {line}"
        );
        let truth = joints(&look, 2, 0.5);
        let body = truth.iter().find(|b| b.slot == 1).expect("drawn");
        let top = body.joints.iter().map(|j| j[1]).fold(f32::MIN, f32::max);
        assert!(top > 1.2, "stands again, head at {top:.2} m");
    }

    /// The scripted get-up of the test above, sampled at alpha 0 and 0.5 of every tick: for each
    /// sample, whether slot 1 was still a ragdoll and where its joints were.
    fn get_up_samples() -> Vec<(bool, Vec<[f32; 3]>)> {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = ring(2);
        let hp = [1000u16; 2];
        let mut samples = Vec::new();
        for t in 0..420u32 {
            let cues = if t == 10 {
                vec![hit(0, 1), knock(0, 1)]
            } else {
                Vec::new()
            };
            look.observe_tick(t, &views, &hp, &cues);
            let gait = match t {
                10..=39 => GaitPhase::Fallen,
                40..=54 => GaitPhase::Catching,
                _ => GaitPhase::Steady,
            };
            let shown = look.ragdoll_fold_gait(1, gait);
            look.observe_balance(1, shown);
            look.ragdoll_tick();
            for alpha in [0.0f32, 0.5] {
                let truth = joints(&look, 2, alpha);
                let body = truth.iter().find(|b| b.slot == 1).expect("drawn");
                samples.push((body.ragdolled, body.joints.clone()));
            }
        }
        samples
    }

    #[test]
    fn ragdoll_look_get_up_first_frame_is_within_the_pop_law_of_the_ragdoll_it_leaves() {
        // The view law skips the first frame out of a ragdoll (physics moved the joints, so it
        // has no velocity to predict from). The prediction is made here from the ragdoll's own
        // last two frames: with the 0.25 s halflife the first get-up frame must land within
        // POP_M of it, the 10 cm the halflife's doc comment states.
        let samples = get_up_samples();
        let lying = samples
            .iter()
            .position(|s| s.0)
            .expect("the fall spawned a ragdoll");
        let up = lying
            + samples[lying..]
                .iter()
                .position(|s| !s.0)
                .expect("it stood up");
        assert!(up >= lying + 2, "two ragdoll frames to predict from");
        let miss = |at: usize| -> f64 {
            let (now, a, b) = (&samples[at].1, &samples[at - 1].1, &samples[at - 2].1);
            (0..now.len())
                .map(|j| {
                    (0..3)
                        .map(|k| f64::from(now[j][k]) - f64::from(2.0 * a[j][k] - b[j][k]))
                        .map(|d| d * d)
                        .sum::<f64>()
                        .sqrt()
                })
                .fold(0.0, f64::max)
        };
        let first = miss(up);
        println!(
            "ragdoll_look: get-up halflife {GETUP_HALFLIFE_S} s, first frame {:.1} cm off its prediction, second {:.1} cm",
            first * 100.0,
            miss(up + 1) * 100.0
        );
        assert!(
            first < crate::view_truth::POP_M,
            "the first get-up frame is {:.1} cm off the ragdoll's own prediction",
            first * 100.0
        );
    }

    #[test]
    fn ragdoll_look_hashes_equal_on_and_off() {
        // The window's session: 8 fighters, 900 ticks of brains fighting. The look reads the
        // tick after it ran and never writes back, so the authority checkpoints must be the
        // same bytes with the ragdoll layer on and off.
        let run = |on: bool| {
            let config = crate::GameConfig {
                seed: 2040,
                ticks: 900,
                fighters: 8,
                half_extent_m: 20.0,
            };
            let mut app = crate::App::new(config)
                .and_then(crate::App::into_duel)
                .expect("duel builds");
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            look.set_ragdoll(on);
            let mut peak = 0;
            let mut laws = ViewLaws::default();
            for t in 0..900u32 {
                let out = app.tick_empty();
                look.observe_tick(t, app.fighter_views(), app.hp(), &out.cues);
                look.observe_live_balances(app.live_match(), 8);
                peak = peak.max(look.ragdolls());
                let truth = look.pose_frame_joints(8, 0.5).expect("joints");
                laws.note(&camera(), f64::from(t) + 0.5, &truth);
            }
            (
                app.checkpoints().to_vec(),
                peak,
                look.ragdoll_spawns(),
                laws.line(),
            )
        };
        let (on, peak, spawns, line) = run(true);
        let (off, none, zero, off_line) = run(false);
        let pops = |line: &Option<String>| -> u32 {
            line.as_deref()
                .and_then(|l| l.split(" pops=").nth(1))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse().ok())
                .expect("the line names pops")
        };
        let (pops_on, pops_off) = (pops(&line), pops(&off_line));
        println!(
            "ragdoll_look: {} checkpoints equal on/off, peak {peak} ragdolls, {spawns} spawns
  on : {}
  off: {}",
            on.len(),
            line.clone().unwrap_or_default(),
            off_line.unwrap_or_default()
        );
        assert!(!on.is_empty(), "the run took checkpoints");
        assert_eq!(
            on, off,
            "authority checkpoints are the same with the layer on and off"
        );
        assert!(
            spawns > 0 && peak > 0,
            "the fight floored bodies: {spawns} spawns"
        );
        assert_eq!((none, zero), (0, 0), "off spawns nothing");
        // The ragdolls and their rises add a few joint pops at most to a brawl that has
        // the same brawl without them, almost all in the run act.
        assert!(
            pops_on <= pops_off + 8,
            "pops with the layer on {pops_on}, off {pops_off}"
        );
        assert!(
            line.is_some_and(|l| l.contains(" ragdolls=") && !l.ends_with(" ragdolls=0")),
            "the arena view line counts the ragdolls drawn"
        );
    }

    #[test]
    fn ragdoll_look_tow_lifts_a_body_along_the_track_then_it_lands_and_gets_up() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut views = ring(2);
        let hp = [1000u16; 2];
        let mut peak_m = 0.0f32;
        let mut gap_m = 0.0f32;
        let mut up_at = None;
        for t in 0..420u32 {
            let towed = (5..40).contains(&t);
            if towed {
                views[1].pos_m[0] += 0.1;
            }
            look.observe_tick(t, &views, &hp, &[]);
            let mut tows = [None; MAX_LOOK_FIGHTERS];
            tows[1] = towed.then_some(0.6);
            look.observe_carries(&tows);
            let shown = look.ragdoll_fold_gait(1, GaitPhase::Steady);
            assert_eq!(shown, GaitPhase::Steady);
            look.ragdoll_tick();
            if let Some(root) = look.ragdoll_roots()[1] {
                if towed && t > 25 {
                    peak_m = peak_m.max(root[1]);
                    gap_m = gap_m.max((root[0] - views[1].pos_m[0]).abs());
                }
            } else if t > 40 && up_at.is_none() {
                up_at = Some(t);
            }
            assert!(look.ragdolls() <= 1);
        }
        assert_eq!(look.ragdoll_refused(), 0);
        assert_eq!(look.ragdoll_spawns(), 1, "one body for the whole flight");
        assert!(peak_m > 1.4, "the arc holds the body up: {peak_m}");
        assert!(gap_m < 0.8, "and it follows the sim's track: {gap_m}");
        let up = up_at.expect("the body landed and got up");
        assert!(up >= 80, "not before the level carry ran out: {up}");
        assert!(look.ragdolls() == 0 && !look.ragdolled(1));
    }
}
