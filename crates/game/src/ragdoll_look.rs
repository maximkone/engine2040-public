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

use super::fighter_look_inertia::{apply_offsets, begin_switch};
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
/// Ticks a ragdoll lies before the sim's recovery may stand it up (1.5 s).
const MIN_LYING_TICKS: u32 = 90;
/// Halflife of the get-up blend in seconds: the time the gap between the last ragdoll pose and the
/// clip halves in. Longer than a clip switch (0.08 s) because the gap is a lying body against a
/// standing one, and a short halflife would pop (the view law judges a joint 10 cm off its own
/// constant-velocity prediction in one frame).
const GETUP_HALFLIFE_S: f32 = 0.25;
/// Ticks after which the get-up offsets are dropped: ten halflives, a millimeter per meter left.
const GETUP_TICKS: u32 = (GETUP_HALFLIFE_S * 10.0 * TICK_HZ) as u32;
/// Age marking a slot with no get-up in flight.
const NO_GETUP: u32 = u32::MAX;
/// A `Hit` this many ticks old or newer explains a fall that has no cue of its own.
const HIT_EXPLAINS_TICKS: u32 = 30;

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
            stepped: None,
            spawned: 0,
            refused: 0,
        }
    }

    /// A layer that never spawns: for a look with no rigs.
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
        self.stepped = None;
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
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    u64::try_from(
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_micros(),
    )
    .unwrap_or(u64::MAX)
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
        self.bodies[slot] = Some(Body {
            rig,
            seed: pose.clone(),
            center,
            age: 0,
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
    /// is not `Fallen` lets a ragdoll that has lain long enough stand up.
    pub(super) fn ragdoll_fold_gait(&mut self, slot: u8, gait: GaitPhase) {
        let index = usize::from(slot);
        if !self.ragdoll.on || index >= MAX_LOOK_FIGHTERS {
            return;
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
            return;
        }
        self.ragdoll.latched[index] = false;
        let lying = self.ragdoll.bodies[index]
            .as_ref()
            .is_some_and(|body| body.age >= MIN_LYING_TICKS);
        self.ragdoll.stand[index] = lying && self.slots[index].act != Act::Down;
        if self.ragdoll.stand[index] {
            self.slots[index].balance = gait;
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
            if let Some(kick) = self.ragdoll.pending[index].take() {
                self.ragdoll_spawn(index, kick);
            }
        }
        if fresh {
            for age in &mut self.ragdoll.getup_age {
                if *age != NO_GETUP {
                    *age = if *age + 1 < GETUP_TICKS {
                        *age + 1
                    } else {
                        NO_GETUP
                    };
                }
            }
            if self.ragdoll.live() > 0 {
                self.ragdoll.step();
            }
        }
        for index in 0..MAX_LOOK_FIGHTERS {
            if std::mem::take(&mut self.ragdoll.stand[index]) {
                self.ragdoll_stand_up(index);
            }
        }
    }

    /// Hands the standing pose of `index` to the world.
    fn ragdoll_spawn(&mut self, index: usize, kick: Kick) {
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
        let Some((before, now)) = shown else {
            return;
        };
        let arriving = self.rigs.get(rig_index).and_then(|rig| {
            let now = self.inertial_pose(rig, index, &slot, 0.0).ok()?;
            let next = self.inertial_pose(rig, index, &slot, 1.0).ok()?;
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
        let ticks = f32::from(u16::try_from(age).unwrap_or(u16::MAX));
        apply_offsets(set, pose, (ticks + alpha).max(0.0) / TICK_HZ)
            .map_err(|err| super::LookError::Skin(err.to_string()))
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
        self.ragdoll
            .pose(index, &rig.skeleton, alpha, slot_root(slot, alpha))
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
                    for (index, joint) in body.joints.iter().enumerate() {
                        if joint[1] < out.lowest_joint_m {
                            out.lowest_joint_m = joint[1];
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
        let (mut lying_top_m, mut jump_cm) = (0.0f32, 0.0f32);
        for t in 0..420u32 {
            // The sim floors the gait for two seconds, then it recovers.
            let cues = if t == 10 {
                vec![hit(0, 1), knock(0, 1)]
            } else {
                Vec::new()
            };
            look.observe_tick(t, &views, &hp, &cues);
            let gait = if (10..130).contains(&t) {
                GaitPhase::Fallen
            } else {
                GaitPhase::Steady
            };
            look.ragdoll_fold_gait(1, gait);
            look.ragdoll_tick();
            for alpha in [0.0f32, 0.5] {
                let truth = joints(&look, 2, alpha);
                laws.note(&camera(), f64::from(t) + f64::from(alpha), &truth);
                let body = truth.iter().find(|b| b.slot == 1).expect("drawn");
                if look.ragdolled(1) {
                    lying_top_m = body.joints.iter().map(|j| j[1]).fold(f32::MIN, f32::max);
                } else if stood.is_none() && t > 10 {
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
        assert!(
            stood >= 10 + MIN_LYING_TICKS,
            "lies at least {MIN_LYING_TICKS} ticks, stood {stood}"
        );
        assert_eq!(look.ragdolls(), 0);
        assert!(lying_top_m < 0.8);
        assert!(
            jump_cm < 5.0,
            "the first frame out of the ragdoll moved a joint {jump_cm:.1} cm"
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
        assert!(
            line.is_some_and(|l| l.contains(" ragdolls=") && !l.ends_with(" ragdolls=0")),
            "the arena view line counts the ragdolls drawn"
        );
    }
}

#[cfg(test)]
mod debug_tmp {
    use super::*;
    #[test]
    fn ragdoll_look_debug_pops_tmp() {
        let content =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters");
        let config = crate::GameConfig {
            seed: 2040,
            ticks: 900,
            fighters: 8,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config)
            .and_then(crate::App::into_duel)
            .expect("duel");
        let mut look = FighterLook::load(&content).expect("load");
        let mut prev: Vec<Option<(f64, Vec<[f32; 3]>, Option<Vec<[f64; 3]>>)>> = vec![None; 8];
        for t in 0..900u32 {
            let out = app.tick_empty();
            look.observe_tick(t, app.fighter_views(), app.hp(), &out.cues);
            look.observe_live_balances(app.live_match(), 8);
            for alpha in [0.25f32, 0.75] {
                let time = f64::from(t) + f64::from(alpha);
                for body in look.pose_frame_joints(8, alpha).expect("joints") {
                    let s = usize::from(body.slot);
                    let slot = look.slots[s];
                    if s == 7 && (330..340).contains(&t) {
                        let names = &look.rigs[look.rig_for_slot(s)].skeleton.joints;
                        let at = |n: &str| {
                            let i = names.iter().position(|j| j.name == n).unwrap();
                            body.joints[i]
                        };
                        println!(
                            "T t={t} a={alpha} act={:?} bal={:?} rag={} age={} toes_l={:.2?} knee_l={:.2?} pelvis={:.2?} head={:.2?}",
                            slot.act, slot.balance, look.ragdolled(s), look.ragdoll.getup_age[s],
                            at("toes_l"), at("leg_lower_l"), at("pelvis"), at("head")
                        );
                    }
                    if let Some((pt, pj, pv)) = prev[s].take() {
                        let dt = time - pt;
                        let mut popped = false;
                        if let (Some(v), false) = (&pv, body.ragdolled) {
                            let mut worst = (0.0f64, 0usize);
                            for (i, ((now, then), vv)) in
                                body.joints.iter().zip(&pj).zip(v).enumerate()
                            {
                                let m = (0..3)
                                    .map(|k| {
                                        (f64::from(now[k]) - (f64::from(then[k]) + vv[k] * dt))
                                            .powi(2)
                                    })
                                    .sum::<f64>()
                                    .sqrt();
                                if m > worst.0 {
                                    worst = (m, i);
                                }
                            }
                            if worst.0 > 0.10 {
                                popped = true;
                                println!(
                                    "POP t={t} a={alpha} slot={s} act={:?} bal={:?} ragdolled={} rests={} getup_age={} miss={:.0}cm joint={}",
                                    slot.act,
                                    slot.balance,
                                    look.ragdolled(s),
                                    look.ragdoll_rests(s),
                                    look.ragdoll.getup_age[s],
                                    worst.0 * 100.0,
                                    look.rigs[look.rig_for_slot(s)].skeleton.joints[worst.1].name
                                );
                            }
                        }
                        let vel = if popped || body.ragdolled {
                            None
                        } else {
                            Some(
                                body.joints
                                    .iter()
                                    .zip(&pj)
                                    .map(|(n, p)| {
                                        std::array::from_fn(|k| {
                                            (f64::from(n[k]) - f64::from(p[k])) / dt
                                        })
                                    })
                                    .collect(),
                            )
                        };
                        prev[s] = Some((time, body.joints.clone(), vel));
                    } else {
                        prev[s] = Some((time, body.joints.clone(), None));
                    }
                }
            }
        }
    }
}
