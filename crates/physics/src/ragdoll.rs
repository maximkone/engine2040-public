//! ragdoll: deterministic active ragdoll with hit reactions.
//!
//! Consumer: arena match integration (the arena schedules it; this crate never
//! imports arena). Data owner: this module owns the per-bone Rapier bodies,
//! the spherical joint set, the blend weight and the animated driver pose.
//! Units: meters, seconds, Y-up; rotations are unit quaternions `[x, y, z, w]`.
//! Errors: [`Error`] is typed; invalid caller input never panics.
//!
//! A [`Ragdoll`] spawns one dynamic body per skeleton joint, joined
//! parent-to-child with spherical impulse joints whose swing limits derive
//! deterministically from joint depth (the skeleton contract carries names
//! plus parents only, so depth is the whole limit source: depth 1 swings widest, deeper joints get
//! tighter by [`SWING_STEP`] down to [`SWING_MIN`]). The caller feeds the
//! animated pose per tick via [`Ragdoll::set_animated`]; [`Ragdoll::bone_poses`]
//! blends that animated input with the simulated bones under the
//! [`Ragdoll::blend_to_ragdoll`] weight (`0` = fully animated, `1` = fully
//! simulated). Hit reactions arrive as plain [`HitEvent`] values the caller
//! maps from combat events (this module never imports combat); each hit
//! applies a center-of-mass impulse to one bone.
//!
//! Determinism scope (HASH-LAW tier note): one fixed `dt` per
//! [`Ragdoll::step`] call, bodies and joints created and hashed in skeleton
//! order, rapier3d with `enhanced-determinism` and without SIMD/parallel
//! features. The glue in this module uses only comparisons, `+ - * /`,
//! `sqrt` (quaternion normalize) and `round` (hash) plus integer math: no
//! float formatting, no `sin`/`cos`/`exp`/`powf` family calls. Rapier
//! internals use their own math under their `enhanced-determinism` guarantee;
//! that internal float use is the arena integration's scope, not part of
//! this glue.
//!
//! Coupling seam: Divide and Truncate (arXiv 2604.15513v1, idea
//! only, reimplemented) as the animated-vs-sim guard. Each [`Ragdoll::step`]
//! divides every penetration correction into [`CouplingParams::substeps`]
//! equal parts and truncates each part to [`CouplingParams::max_step_m`],
//! so a fast bone stops at the floor or at its neighbor instead of
//! tunneling through. One-way only: sim bodies move, the animated driver
//! and the floor never feel a force back.
#![forbid(unsafe_code)]

use super::rapier::{
    ordered_linvel, ordered_translation, position_in_bounds, quantize, sanitize3, BodyState,
    FIXED_DT, GRAVITY_Y, MAX_LEN,
};
use engine_anim::contract::{validate_pose, validate_skeleton, AnimError, LocalPose, Skeleton};
use engine_core::{quat_mul, Fnv64};
use rapier3d::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, JointAxis, MultibodyJointSet,
    RigidBodyBuilder, RigidBodyHandle, RigidBodySet, SoftBodySet, SphericalJointBuilder,
};
use rapier3d::geometry::{BroadPhaseBvh, ColliderBuilder, ColliderSet, NarrowPhase};
use rapier3d::math::{Rotation, Vector};
use rapier3d::pipeline::PhysicsPipeline;
use std::fmt::{Display, Formatter};

/// Swing limit for depth-1 joints (children of the root), radians.
const SWING_ROOT: f32 = 0.9;
/// Swing limit shrink per extra depth level, radians.
const SWING_STEP: f32 = 0.15;
/// Tightest swing limit any joint gets, radians.
pub(crate) const SWING_MIN: f32 = 0.2;
/// Small linear damping so a full ragdoll settles to rest on the floor.
const LINEAR_DAMPING: f32 = 0.05;
/// Small angular damping so chained bones stop oscillating.
const ANGULAR_DAMPING: f32 = 0.05;
/// Contact friction for bone colliders.
const FRICTION: f32 = 0.9;
/// Stand budget: the knock/drive/stand fixture ramps the blend back to the
/// animated pose inside this many fixed steps (1 s at 60 Hz).
pub const COUPLING_GETUP_BUDGET_TICKS: u64 = 60;
/// Divide-count ceiling for the truncate pass.
pub const COUPLING_MAX_SUBSTEPS: u8 = 8;
/// Default divide count: each correction runs in four equal parts.
const COUPLING_DEFAULT_SUBSTEPS: u8 = 4;
/// Default per-part truncation, meters: four parts stop 0.2 m per tick,
/// past a terminal fall step from 3 m (about 0.13 m) with margin.
const COUPLING_DEFAULT_MAX_STEP: f32 = 0.05;
/// Default floor plane height, meters (matches the tape floor box top).
const COUPLING_DEFAULT_FLOOR_Y: f32 = 0.0;
/// Default minimum bone-center separation, meters. Rest neighbors sit
/// 0.5 m apart, so this only bites on real interpenetration.
const COUPLING_DEFAULT_SPACING: f32 = 0.10;

/// Typed errors for the ragdoll. User input never panics.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Skeleton or pose failed the animation contract; the [`AnimError`] names it.
    Anim(AnimError),
    /// Capsule radius must be finite and in `(0, MAX_LEN]`.
    InvalidRadius,
    /// Capsule half-height must be finite and in `[0, MAX_LEN]`.
    InvalidHeight,
    /// Static-box half-extents must be finite and each in `(0, MAX_LEN]`.
    InvalidHalfExtents,
    /// A position component was non-finite or out of range.
    NonFinitePosition,
    /// Bone index is out of range for this skeleton.
    BadBone(usize),
    /// Blend weight must be finite and in `0..=1`.
    BadBlend,
    /// Hit impulse must have finite components.
    NonFiniteImpulse,
    /// `step` needs a finite `dt` greater than zero.
    InvalidDt,
    /// Divide count must be in `1..=COUPLING_MAX_SUBSTEPS`.
    BadSubsteps,
    /// A coupling bound is out of range (per-part step, floor height or
    /// minimum spacing); nothing is changed then.
    BadCouplingBound,
    /// Snapshot body count (or animated pose length) does not match.
    SnapshotMismatch,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Anim(e) => write!(f, "invalid skeleton/pose: {e:?}"),
            Error::InvalidRadius => f.write_str("invalid radius: need finite radius in (0, 1000]"),
            Error::InvalidHeight => {
                f.write_str("invalid half-height: need finite value in [0, 1000]")
            }
            Error::InvalidHalfExtents => {
                f.write_str("invalid half-extents: need finite values in (0, 1000]")
            }
            Error::NonFinitePosition => {
                f.write_str("invalid position: need finite values within +-1000")
            }
            Error::BadBone(b) => write!(f, "bad bone: index {b} out of range"),
            Error::BadBlend => f.write_str("invalid blend: need finite amount in 0..=1"),
            Error::NonFiniteImpulse => f.write_str("invalid impulse: need finite components"),
            Error::InvalidDt => f.write_str("invalid dt: need finite dt > 0"),
            Error::BadSubsteps => f.write_str("invalid substeps: need 1..=8 divide parts"),
            Error::BadCouplingBound => {
                f.write_str("invalid coupling bound: need finite step, floor and spacing")
            }
            Error::SnapshotMismatch => f.write_str("snapshot does not match this ragdoll"),
        }
    }
}

impl std::error::Error for Error {}

impl From<AnimError> for Error {
    fn from(e: AnimError) -> Self {
        Error::Anim(e)
    }
}

/// Capsule dimensions for one bone: `[radius, half_height]` packed as a typed
/// pair so call order is explicit. `half_height` is the cylinder mid-section
/// half-height (total height is `2 * half_height + 2 * radius`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneShape {
    /// Capsule radius, meters, in `(0, MAX_LEN]`.
    pub radius: f32,
    /// Cylinder mid-section half-height, meters, in `[0, MAX_LEN]`.
    pub half_height: f32,
}

/// One bone capsule in world space, as [`Ragdoll::bone_capsule`] reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneCapsule {
    /// Low end of the capsule axis, meters.
    pub a: [f32; 3],
    /// High end of the capsule axis, meters.
    pub b: [f32; 3],
    /// Capsule radius, meters.
    pub radius: f32,
    /// One over the body mass, 1/kg.
    pub inv_mass: f32,
}

/// Plain hit event the caller maps from combat data. This type is
/// local on purpose: the physics lane never imports combat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitEvent {
    /// Struck bone index in skeleton order.
    pub bone: usize,
    /// Center-of-mass impulse, newton-seconds.
    pub impulse: [f32; 3],
    /// Caller tick the hit belongs to (recorded in the hit counter order).
    pub tick: u64,
}

/// One observed contact pair from the last [`Ragdoll::step`]: the Jolt
/// `ContactListener` pattern reimplemented on Rapier's narrow phase. The
/// step only reads contacts into a pending list; the caller drains it after
/// the step and mutates game state there (hits, breaks, sound), never
/// inside the step. `b` is `None` against static geometry (floor, shield).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoneContact {
    /// First bone index in skeleton order (the smaller one for bone pairs).
    pub a: usize,
    /// Second bone index, or `None` against static geometry.
    pub b: Option<usize>,
}

impl BoneContact {
    /// Order-preserving constructor for the shared narrow-phase drain.
    fn ordered(a: usize, b: Option<usize>) -> Self {
        Self { a, b }
    }

    /// Skeleton-order sort key for the shared narrow-phase drain.
    fn sort_key(&self) -> (usize, Option<usize>) {
        (self.a, self.b)
    }
}

/// Divide-and-Truncate coupling params: the animated-vs-sim guard that runs
/// inside every [`Ragdoll::step`]. A needed correction is divided into
/// `substeps` equal parts and each part is truncated to `max_step_m`, so
/// the seam can stop at most `substeps * max_step_m` meters of penetration
/// per tick. Floor and spacing probes are one-way: sim bodies move, the
/// driver pose and the floor never feel a force back. Units: meters;
/// determinism: plain value, folded into [`Ragdoll::state_hash`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CouplingParams {
    /// Divide count in `1..=COUPLING_MAX_SUBSTEPS`; more parts smooth the
    /// stop but cost more pairwise checks per tick.
    pub substeps: u8,
    /// Per-part truncation, meters, finite in `(0, 1]`.
    pub max_step_m: f32,
    /// Floor plane height, meters, finite within `+-MAX_LEN`. A bone
    /// center may never rest below `floor_top_y + radius`.
    pub floor_top_y: f32,
    /// Minimum bone-center separation, meters, finite in `[0, MAX_LEN]`.
    /// Rest neighbors sit 0.5 m apart, so small values only bite on real
    /// interpenetration.
    pub min_spacing_m: f32,
    /// Master switch. Disabled, the seam measures nothing and moves nothing,
    /// so the world integrates exactly as before.
    pub enabled: bool,
}

impl CouplingParams {
    /// Default seam: enabled, four parts of at most 5 cm (0.2 m per tick),
    /// floor at `0.0`, 10 cm minimum spacing.
    pub fn default_coupling() -> Self {
        Self {
            substeps: COUPLING_DEFAULT_SUBSTEPS,
            max_step_m: COUPLING_DEFAULT_MAX_STEP,
            floor_top_y: COUPLING_DEFAULT_FLOOR_Y,
            min_spacing_m: COUPLING_DEFAULT_SPACING,
            enabled: true,
        }
    }

    /// Disabled seam: [`Ragdoll::step`] skips the pass entirely.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default_coupling()
        }
    }
}

impl Default for CouplingParams {
    fn default() -> Self {
        Self::default_coupling()
    }
}

/// Coupling counters: what the seam saw and what it could not fix.
/// `penetration_events` counts residual penetrations left after the pass
/// (zero means the seam stopped everything that tick); `truncations` counts
/// corrections the per-part cap shortened; `max_correction_m` is the
/// largest single correction applied so far. All three are folded into
/// [`Ragdoll::state_hash`] and carried by [`RagdollSnapshot`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CouplingStats {
    /// Residual penetrations left after the pass (floor or spacing).
    pub penetration_events: u64,
    /// Corrections shortened by the per-part cap.
    pub truncations: u64,
    /// Largest single correction applied, meters.
    pub max_correction_m: f32,
}

/// Owned snapshot: per-bone kinematic state plus the blend extras that affect
/// future [`Ragdoll::bone_poses`] output.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollSnapshot {
    /// One entry per bone, skeleton order.
    pub bodies: Vec<BodyState>,
    /// Animated driver pose at snapshot time.
    pub animated: LocalPose,
    /// Blend weight at snapshot time.
    pub blend: f32,
    /// Step counter at snapshot time.
    pub tick: u64,
    /// Coupling counters at snapshot time.
    pub coupling_stats: CouplingStats,
    /// Per-bone CCD switches at snapshot time.
    pub ccd_flags: Vec<bool>,
    /// Total contact pairs observed at snapshot time.
    pub contact_events: u64,
}

/// Deterministic active ragdoll: one dynamic body per skeleton joint.
pub struct Ragdoll {
    pipeline: PhysicsPipeline,
    gravity: Vector,
    params: IntegrationParameters,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
    ccd: CCDSolver,
    /// Body handles in skeleton order; fixes iteration/hash order.
    order: Vec<RigidBodyHandle>,
    /// Owned skeleton: joint parents drive the sim-local conversion.
    skeleton: Skeleton,
    /// Latest animated driver pose from the caller.
    animated: LocalPose,
    /// Blend weight: `0` = animated, `1` = simulated.
    blend: f32,
    /// Fixed steps taken since spawn (folded into the hash).
    tick: u64,
    /// Applied-hit counter (folded into the hash).
    hits: u64,
    /// Coupling seam params (folded into the hash).
    coupling: CouplingParams,
    /// Coupling counters (folded into the hash, carried by snapshots).
    coupling_stats: CouplingStats,
    /// Capsule radius per bone, skeleton order: the floor probe's yardstick.
    radii: Vec<f32>,
    /// Capsule cylinder half-height per bone, skeleton order: with the radius it fixes the
    /// capsule axis the clinch pass parts.
    half_heights: Vec<f32>,
    /// Per-bone continuous collision: `true` rides the CCD solver so fast
    /// weapons and limbs stop at thin shields instead of tunneling. On at
    /// spawn for every bone; folded into the hash, carried by snapshots.
    ccd_flags: Vec<bool>,
    /// Contacts read during the last step, skeleton order, drained by the
    /// caller after the step (never mutated inside it). Observation only:
    /// cleared on snapshot restore, not hashed (the counter below is).
    pending: Vec<BoneContact>,
    /// Total contact pairs observed since spawn (folded into the hash,
    /// carried by snapshots).
    contact_events: u64,
}

impl Ragdoll {
    /// Spawn one dynamic body per skeleton joint at the animated world
    /// transform, joined parent-to-child with depth-limited spherical joints.
    /// `capsule_dims` carries one [`BoneShape`] per joint. Blend starts at
    /// `0`, so [`Ragdoll::bone_poses`] returns `pose` exactly until the caller
    /// raises the weight.
    pub fn spawn(
        skeleton: &Skeleton,
        pose: &LocalPose,
        capsule_dims: &[BoneShape],
    ) -> Result<Self, Error> {
        validate_skeleton(skeleton).map_err(Error::Anim)?;
        validate_pose(skeleton, pose).map_err(Error::Anim)?;
        if capsule_dims.len() != skeleton.len() {
            return Err(Error::Anim(AnimError::LengthMismatch {
                expected: skeleton.len(),
                got: capsule_dims.len(),
            }));
        }
        for d in capsule_dims {
            if !d.radius.is_finite() || d.radius == 0.0 || !(0.0..=MAX_LEN).contains(&d.radius) {
                return Err(Error::InvalidRadius);
            }
            if !d.half_height.is_finite() || !(0.0..=MAX_LEN).contains(&d.half_height) {
                return Err(Error::InvalidHeight);
            }
        }
        let (world_pos, world_rot) = world_transforms(skeleton, pose);
        let depths = joint_depths(skeleton);

        let params = IntegrationParameters {
            dt: FIXED_DT,
            ..Default::default()
        };
        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();
        let mut impulse_joints = ImpulseJointSet::new();
        let mut order = Vec::with_capacity(skeleton.len());
        for (i, (p, r)) in world_pos.iter().zip(world_rot.iter()).enumerate() {
            let body = RigidBodyBuilder::dynamic()
                .translation(Vector::new(p[0], p[1], p[2]))
                .linear_damping(LINEAR_DAMPING)
                .angular_damping(ANGULAR_DAMPING)
                // Sleeping is disabled so kinematic state (pos/rot/vel) is
                // the complete future-affecting state and restore is exact.
                .can_sleep(false)
                // CCD is on: fast limbs stop at thin shields (see `set_ccd`).
                .ccd_enabled(true)
                .build();
            let handle = bodies.insert(body);
            // Orientation comes from the animated world rotation. The builder
            // takes axis-angle (which needs `acos` to derive), so rotation is
            // set directly as a quaternion instead: no trig in this glue.
            if let Some(b) = bodies.get_mut(handle) {
                b.set_rotation(normalized_rotation(r), false);
            }
            let collider =
                ColliderBuilder::capsule_y(capsule_dims[i].half_height, capsule_dims[i].radius)
                    .density(1.0)
                    .friction(FRICTION)
                    .restitution(0.0)
                    .build();
            colliders.insert_with_parent(collider, handle, &mut bodies);
            order.push(handle);
        }
        // Parent-to-child spherical joints, anchored at the midpoint of the
        // two rest positions so chained bodies start unstressed. Contacts
        // between joined bodies stay off so overlapping bone capsules never
        // self-collide; limits come from joint depth (the only hierarchy signal).
        insert_joints(
            skeleton,
            &world_pos,
            &world_rot,
            &depths,
            &order,
            &mut impulse_joints,
        );
        Ok(Self {
            pipeline: PhysicsPipeline::new(),
            gravity: Vector::new(0.0, GRAVITY_Y, 0.0),
            params,
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies,
            colliders,
            impulse_joints,
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            order,
            skeleton: skeleton.clone(),
            animated: pose.clone(),
            blend: 0.0,
            tick: 0,
            hits: 0,
            coupling: CouplingParams::default_coupling(),
            coupling_stats: CouplingStats::default(),
            radii: capsule_dims.iter().map(|d| d.radius).collect(),
            half_heights: capsule_dims.iter().map(|d| d.half_height).collect(),
            ccd_flags: vec![true; skeleton.len()],
            pending: Vec::with_capacity(32),
            contact_events: 0,
        })
    }

    /// Feed the animated driver pose for this tick. The caller (arena
    /// integration) samples its animation graph and passes the result here
    /// before [`Ragdoll::step`]; validation matches spawn.
    pub fn set_animated(&mut self, pose: &LocalPose) -> Result<(), Error> {
        validate_pose(&self.skeleton, pose).map_err(Error::Anim)?;
        self.animated = pose.clone();
        Ok(())
    }

    /// Set the ragdoll weight: `0` = fully animated, `1` = fully simulated.
    /// Values outside `0..=1` (or non-finite) are rejected; the weight is
    /// unchanged then.
    pub fn blend_to_ragdoll(&mut self, amount: f32) -> Result<(), Error> {
        if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
            return Err(Error::BadBlend);
        }
        self.blend = amount;
        Ok(())
    }

    /// Current blend weight.
    pub fn blend(&self) -> f32 {
        self.blend
    }

    /// Current coupling seam params.
    pub fn coupling(&self) -> CouplingParams {
        self.coupling
    }

    /// Install coupling seam params. Out-of-range divide counts or bounds
    /// are rejected and the seam is unchanged then.
    pub fn set_coupling(&mut self, params: CouplingParams) -> Result<(), Error> {
        check_coupling(&params)?;
        self.coupling = params;
        Ok(())
    }

    /// Coupling counters: residuals, truncations and largest correction.
    pub fn coupling_stats(&self) -> CouplingStats {
        self.coupling_stats
    }

    /// Residual penetrations left after the seam pass. Zero means every
    /// correction fit inside the per-tick authority that step.
    pub fn penetration_events(&self) -> u64 {
        self.coupling_stats.penetration_events
    }

    /// Per-bone CCD switch (`None` past the end of the skeleton). Every
    /// bone spawns with CCD on; the caller switches slow bones off to save
    /// solver work and fast weapons back on before a swing.
    pub fn ccd_enabled(&self, bone: usize) -> Option<bool> {
        self.ccd_flags.get(bone).copied()
    }

    /// Flip one bone's CCD switch. An out-of-range bone changes nothing and
    /// reports [`Error::BadBone`].
    pub fn set_ccd(&mut self, bone: usize, enabled: bool) -> Result<(), Error> {
        let flag = self.ccd_flags.get_mut(bone).ok_or(Error::BadBone(bone))?;
        *flag = enabled;
        let handle = self.order[bone];
        if let Some(body) = self.bodies.get_mut(handle) {
            body.enable_ccd(enabled);
        }
        Ok(())
    }

    /// Drive one bone's linear velocity directly (a swung weapon's tip
    /// speed): components must be finite, the bone must exist. The world is
    /// unchanged on error.
    pub fn set_bone_linvel(&mut self, bone: usize, vel: [f32; 3]) -> Result<(), Error> {
        if vel.iter().any(|c| !c.is_finite()) {
            return Err(Error::NonFiniteImpulse);
        }
        let handle = *self.order.get(bone).ok_or(Error::BadBone(bone))?;
        let Some(body) = self.bodies.get_mut(handle) else {
            return Err(Error::BadBone(bone));
        };
        body.set_linvel(Vector::new(vel[0], vel[1], vel[2]), true);
        Ok(())
    }

    /// Contacts read during the last step, skeleton order. Read-only: the
    /// caller drains them with [`Ragdoll::take_contacts`] after the step
    /// and applies game effects there.
    pub fn contacts(&self) -> &[BoneContact] {
        &self.pending
    }

    /// Drain the last step's contacts (Jolt listener pattern): the step
    /// reads, the caller takes the list and mutates game state after it.
    /// Deterministic skeleton order; empty when nothing touched.
    pub fn take_contacts(&mut self) -> Vec<BoneContact> {
        core::mem::take(&mut self.pending)
    }

    /// Drain the last step's contacts into applied hits: the wired consumer
    /// of the listener drain (the contact/joint follow-up pass calls this
    /// after the step instead of reading [`Ragdoll::take_contacts`] raw).
    /// Each drained contact strikes bone `a` with the caller-supplied
    /// `impulse` at `tick`; pair contacts strike the first bone only —
    /// splitting pairs across both bones is the follow-up joint-table work.
    /// The caller gates strike windows (weapon bones, not resting feet):
    /// this call strikes every drained contact. A non-finite impulse drains
    /// nothing and reports [`Error::NonFiniteImpulse`]. Returns contacts
    /// applied.
    pub fn drain_contacts_into_hits(
        &mut self,
        impulse: [f32; 3],
        tick: u64,
    ) -> Result<usize, Error> {
        if impulse.iter().any(|c| !c.is_finite()) {
            return Err(Error::NonFiniteImpulse);
        }
        let contacts = self.take_contacts();
        let mut applied = 0;
        for contact in &contacts {
            self.apply_hit_event(HitEvent {
                bone: contact.a,
                impulse,
                tick,
            })?;
            applied += 1;
        }
        Ok(applied)
    }

    /// Total contact pairs observed since spawn.
    pub fn contact_events(&self) -> u64 {
        self.contact_events
    }

    /// Fixed steps taken since spawn.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Inject a hit impulse on one bone: `apply_hit(bone, impulse, tick)` per
    /// the packet. Non-finite impulse components and out-of-range bones are
    /// rejected; the world is unchanged then.
    pub fn apply_hit(&mut self, bone: usize, impulse: [f32; 3], tick: u64) -> Result<(), Error> {
        self.apply_hit_event(HitEvent {
            bone,
            impulse,
            tick,
        })
    }

    /// Inject a [`HitEvent`] mapped by the caller from combat data.
    pub fn apply_hit_event(&mut self, hit: HitEvent) -> Result<(), Error> {
        let handle = *self.order.get(hit.bone).ok_or(Error::BadBone(hit.bone))?;
        if hit.impulse.iter().any(|c| !c.is_finite()) {
            return Err(Error::NonFiniteImpulse);
        }
        let Some(body) = self.bodies.get_mut(handle) else {
            return Err(Error::BadBone(hit.bone));
        };
        body.apply_impulse(
            Vector::new(hit.impulse[0], hit.impulse[1], hit.impulse[2]),
            true,
        );
        // The velocity change already carries the impulse; the counter only
        // records that a hit happened so the hash sees the event order.
        self.hits = self.hits.wrapping_add(1);
        let _ = hit.tick;
        Ok(())
    }

    /// Advance the simulation by one fixed step. Only finite `dt > 0` is
    /// accepted; callers use [`FIXED_DT`]. After the world integrates, the
    /// coupling seam divides every penetration correction into equal parts
    /// and truncates each part, so bones stop at the floor or at their
    /// neighbor instead of tunneling through. Contacts are read into the
    /// pending list at the end of the step; the caller drains them after
    /// the step and mutates game state there, never inside it.
    pub fn step(&mut self, dt: f32) -> Result<(), Error> {
        if !dt.is_finite() || dt <= 0.0 {
            return Err(Error::InvalidDt);
        }
        self.params.dt = dt;
        let hooks = ();
        let events = ();
        self.pipeline.step(
            self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &hooks,
            &events,
        );
        resolve_coupling(
            &mut self.bodies,
            &self.order,
            &self.radii,
            &self.coupling,
            &mut self.coupling_stats,
        );
        drain_narrow_contacts(
            &self.narrow,
            &self.colliders,
            &self.order,
            &mut self.pending,
            &mut self.contact_events,
            BoneContact::ordered,
            BoneContact::sort_key,
        );
        self.tick = self.tick.wrapping_add(1);
        Ok(())
    }

    /// Blended local pose: animated driver mixed with simulated bones under
    /// the current weight. At weight `0` this returns the animated pose
    /// exactly; at weight `1` the simulated local pose exactly.
    pub fn bone_poses(&self) -> LocalPose {
        if self.blend <= 0.0 {
            return self.animated.clone();
        }
        let rest_pos = world_transforms(&self.skeleton, &self.animated).0;
        let (sim_pos, sim_rot) = read_sim_transforms(&self.bodies, &self.order, &rest_pos);
        let n = self.skeleton.len();
        let mut positions = Vec::with_capacity(n);
        let mut rotations = Vec::with_capacity(n);
        for (i, joint) in self.skeleton.joints.iter().enumerate() {
            // Simulated local transform: parent-relative, or world for root.
            let (lp, lq) = match joint.parent {
                None => (sim_pos[i], sim_rot[i]),
                Some(p) => {
                    let pc = quat_conj(&sim_rot[p]);
                    let d = [
                        sim_pos[i][0] - sim_pos[p][0],
                        sim_pos[i][1] - sim_pos[p][1],
                        sim_pos[i][2] - sim_pos[p][2],
                    ];
                    (rotate_vec(&d, &pc), quat_mul(pc, sim_rot[i]))
                }
            };
            let lq = normalize_quat(&lq);
            if self.blend >= 1.0 {
                positions.push(lp);
                rotations.push(lq);
                continue;
            }
            let ap = self.animated.positions[i];
            let aq = self.animated.rotations[i];
            let t = self.blend;
            positions.push([
                ap[0] + (lp[0] - ap[0]) * t,
                ap[1] + (lp[1] - ap[1]) * t,
                ap[2] + (lp[2] - ap[2]) * t,
            ]);
            // Shortest-path nlerp: negate when antipodal, then normalize
            // (one `sqrt`, allowed sim math). Linear in `t`, so pose-space
            // error grows monotonically with the weight; see the blend test.
            let mut bq = lq;
            let dot = aq[0] * bq[0] + aq[1] * bq[1] + aq[2] * bq[2] + aq[3] * bq[3];
            if dot < 0.0 {
                bq = [-bq[0], -bq[1], -bq[2], -bq[3]];
            }
            rotations.push(normalize_quat(&[
                aq[0] + (bq[0] - aq[0]) * t,
                aq[1] + (bq[1] - aq[1]) * t,
                aq[2] + (bq[2] - aq[2]) * t,
                aq[3] + (bq[3] - aq[3]) * t,
            ]));
        }
        LocalPose {
            positions,
            rotations,
        }
    }

    /// Add a static box (the floor). Mirrors the Rapier adapter's validation; returns an error
    /// for bad input and adds nothing then.
    pub fn add_static_box(&mut self, half_extents: [f32; 3], pos: [f32; 3]) -> Result<(), Error> {
        check_pos(pos)?;
        for h in half_extents {
            if !h.is_finite() || h == 0.0 || !(0.0..=MAX_LEN).contains(&h) {
                return Err(Error::InvalidHalfExtents);
            }
        }
        let body = RigidBodyBuilder::fixed()
            .translation(Vector::new(pos[0], pos[1], pos[2]))
            .build();
        let handle = self.bodies.insert(body);
        let collider = ColliderBuilder::cuboid(half_extents[0], half_extents[1], half_extents[2])
            .friction(FRICTION)
            .restitution(0.0)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);
        Ok(())
    }

    /// Number of bones (one body each).
    pub fn bone_count(&self) -> usize {
        self.order.len()
    }

    /// World center of one bone by index.
    pub fn bone_position(&self, bone: usize) -> Option<[f32; 3]> {
        ordered_translation(&self.bodies, &self.order, bone)
    }

    /// Linear velocity of one bone by index.
    pub fn bone_linvel(&self, bone: usize) -> Option<[f32; 3]> {
        ordered_linvel(&self.bodies, &self.order, bone)
    }

    /// World capsule of one bone: the two ends of its axis, its radius and its inverse mass.
    /// `None` past the end or for a body with a non-finite pose.
    pub fn bone_capsule(&self, bone: usize) -> Option<BoneCapsule> {
        let body = self.bodies.get(*self.order.get(bone)?)?;
        let t = body.translation();
        let r = body.rotation();
        let up = rotate_vec(
            &[0.0, *self.half_heights.get(bone)?, 0.0],
            &normalize_quat(&[r.x, r.y, r.z, r.w]),
        );
        let (c, radius) = ([t.x, t.y, t.z], *self.radii.get(bone)?);
        let capsule = BoneCapsule {
            a: [c[0] - up[0], c[1] - up[1], c[2] - up[2]],
            b: [c[0] + up[0], c[1] + up[1], c[2] + up[2]],
            radius,
            inv_mass: body.mass().recip(),
        };
        let finite = capsule.a.iter().chain(&capsule.b).all(|v| v.is_finite());
        (finite && capsule.inv_mass.is_finite()).then_some(capsule)
    }

    /// Move one bone by `shift` meters and change its velocity by `dv`, the two halves of
    /// parting it from another fighter. Bones never sleep, so nothing is woken: the pass stays
    /// out of Rapier's user-change lists like the floor and spacing passes.
    pub fn part_bone(&mut self, bone: usize, shift: [f32; 3], dv: [f32; 3]) -> Result<(), Error> {
        if shift.iter().chain(&dv).any(|c| !c.is_finite()) {
            return Err(Error::NonFinitePosition);
        }
        let handle = *self.order.get(bone).ok_or(Error::BadBone(bone))?;
        let body = self.bodies.get_mut(handle).ok_or(Error::BadBone(bone))?;
        let (t, v) = (body.translation(), body.linvel());
        body.set_translation(
            Vector::new(t.x + shift[0], t.y + shift[1], t.z + shift[2]),
            false,
        );
        body.set_linvel(Vector::new(v.x + dv[0], v.y + dv[1], v.z + dv[2]), false);
        Ok(())
    }

    /// Canonical hash over bones in skeleton order plus blend extras, hit
    /// and step counters and the coupling seam params and counters. Each
    /// float is scaled by [`HASH_SCALE`](super::rapier::HASH_SCALE), rounded to `i64` and folded as
    /// little-endian bytes (FNV-1a). No float formatting, no transcendental
    /// calls.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for handle in &self.order {
            let Some(body) = self.bodies.get(*handle) else {
                continue;
            };
            let t = body.translation();
            let r = body.rotation();
            let lv = body.linvel();
            let av = body.angvel();
            for v in [
                t.x, t.y, t.z, r.x, r.y, r.z, r.w, lv.x, lv.y, lv.z, av.x, av.y, av.z,
            ] {
                h.write_i64(quantize(v));
            }
        }
        h.write_i64(quantize(self.blend));
        h.write_u64(self.tick);
        h.write_u64(self.hits);
        h.write_u64(self.coupling_stats.penetration_events);
        h.write_u64(self.coupling_stats.truncations);
        h.write_i64(quantize(self.coupling_stats.max_correction_m));
        h.write_u8(self.coupling.substeps);
        h.write_bool(self.coupling.enabled);
        for v in [
            self.coupling.max_step_m,
            self.coupling.floor_top_y,
            self.coupling.min_spacing_m,
        ] {
            h.write_i64(quantize(v));
        }
        for c in &self.ccd_flags {
            h.write_bool(*c);
        }
        h.write_u64(self.contact_events);
        for p in self.animated.positions.iter().flatten() {
            h.write_i64(quantize(*p));
        }
        for q in self.animated.rotations.iter().flatten() {
            h.write_i64(quantize(*q));
        }
        h.finish()
    }

    /// Copy out the full future-affecting state.
    pub fn snapshot(&self) -> RagdollSnapshot {
        let mut bodies = Vec::with_capacity(self.order.len());
        for handle in &self.order {
            if let Some(body) = self.bodies.get(*handle) {
                let t = body.translation();
                let r = body.rotation();
                let lv = body.linvel();
                let av = body.angvel();
                bodies.push(BodyState {
                    pos: [t.x, t.y, t.z],
                    rot: [r.x, r.y, r.z, r.w],
                    linvel: [lv.x, lv.y, lv.z],
                    angvel: [av.x, av.y, av.z],
                });
            }
        }
        RagdollSnapshot {
            bodies,
            animated: self.animated.clone(),
            blend: self.blend,
            tick: self.tick,
            coupling_stats: self.coupling_stats,
            ccd_flags: self.ccd_flags.clone(),
            contact_events: self.contact_events,
        }
    }

    /// Restore a snapshot. Counts and pose lengths must match; resuming with
    /// the same fixed `dt` sequence is identical to the recorded run, and the
    /// restored snapshot compares equal to the saved one. The pending
    /// contact list is observation, not sim state: it is cleared, and the
    /// next step refills it.
    pub fn restore(&mut self, snapshot: RagdollSnapshot) -> Result<(), Error> {
        if snapshot.bodies.len() != self.order.len() {
            return Err(Error::SnapshotMismatch);
        }
        if snapshot.ccd_flags.len() != self.order.len() {
            return Err(Error::SnapshotMismatch);
        }
        validate_pose(&self.skeleton, &snapshot.animated).map_err(|_| Error::SnapshotMismatch)?;
        for (handle, s) in self.order.iter().zip(snapshot.bodies.iter()) {
            let Some(body) = self.bodies.get_mut(*handle) else {
                return Err(Error::SnapshotMismatch);
            };
            body.set_translation(Vector::new(s.pos[0], s.pos[1], s.pos[2]), false);
            // Snapshot quaternions came from Rapier normalized; normalize on
            // restore (one `sqrt`, allowed sim math) to guard hand-built
            // snapshots. Non-finite components fall back to identity.
            let q = if s.rot.iter().all(|c| c.is_finite()) {
                Rotation::from_xyzw(s.rot[0], s.rot[1], s.rot[2], s.rot[3]).normalize()
            } else {
                Rotation::IDENTITY
            };
            body.set_rotation(q, false);
            let lv = sanitize3(s.linvel);
            let av = sanitize3(s.angvel);
            body.set_linvel(Vector::new(lv[0], lv[1], lv[2]), false);
            body.set_angvel(Vector::new(av[0], av[1], av[2]), false);
            body.reset_forces(false);
            body.wake_up(true);
        }
        self.animated = snapshot.animated;
        self.blend = snapshot.blend;
        self.tick = snapshot.tick;
        self.coupling_stats = snapshot.coupling_stats;
        for (i, on) in snapshot.ccd_flags.iter().enumerate() {
            self.ccd_flags[i] = *on;
            if let Some(body) = self.bodies.get_mut(self.order[i]) {
                body.enable_ccd(*on);
            }
        }
        self.contact_events = snapshot.contact_events;
        self.pending.clear();
        Ok(())
    }
}

/// Coupling params carry a divide count in range and finite bounds.
fn check_coupling(params: &CouplingParams) -> Result<(), Error> {
    if params.substeps == 0 || params.substeps > COUPLING_MAX_SUBSTEPS {
        return Err(Error::BadSubsteps);
    }
    if !params.max_step_m.is_finite() || params.max_step_m <= 0.0 || params.max_step_m > 1.0 {
        return Err(Error::BadCouplingBound);
    }
    if !params.floor_top_y.is_finite() || params.floor_top_y.abs() > MAX_LEN {
        return Err(Error::BadCouplingBound);
    }
    if !params.min_spacing_m.is_finite()
        || params.min_spacing_m < 0.0
        || params.min_spacing_m > MAX_LEN
    {
        return Err(Error::BadCouplingBound);
    }
    Ok(())
}

/// Bone index behind a body handle, skeleton order (`None` for static
/// geometry). Linear scan: bone counts are small and the order is fixed.
/// Shared by the plain and powered ragdolls.
pub(crate) fn bone_index(order: &[RigidBodyHandle], handle: RigidBodyHandle) -> Option<usize> {
    order.iter().position(|h| *h == handle)
}

/// Contact-listener read at the end of a ragdoll step: every active
/// narrow-phase pair with at least one bone side becomes one contact via
/// `mk`, sorted by `key` so skeleton order fixes the output on every PC
/// and in replay. Shared by the plain and powered ragdolls (their contact
/// types differ but the read is one pattern). The pass only reads the narrow
/// phase and refills caller-owned buffers; game-state mutation happens after
/// the caller drains the list, never inside the step.
pub(crate) fn drain_narrow_contacts<C>(
    narrow: &NarrowPhase,
    colliders: &ColliderSet,
    order: &[RigidBodyHandle],
    pending: &mut Vec<C>,
    counter: &mut u64,
    mk: fn(usize, Option<usize>) -> C,
    key: fn(&C) -> (usize, Option<usize>),
) {
    pending.clear();
    for pair in narrow.contact_pairs() {
        if !pair.has_any_active_contact() {
            continue;
        }
        let ia = colliders
            .get(pair.collider1)
            .and_then(|c| c.parent())
            .and_then(|h| bone_index(order, h));
        let ib = colliders
            .get(pair.collider2)
            .and_then(|c| c.parent())
            .and_then(|h| bone_index(order, h));
        match (ia, ib) {
            (Some(a), Some(b)) => {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                // Joined neighbors never self-collide (contacts off between
                // jointed bodies), so any bone pair here is a real touch.
                pending.push(mk(lo, Some(hi)));
            }
            (Some(a), None) | (None, Some(a)) => {
                pending.push(mk(a, None));
            }
            (None, None) => continue,
        }
    }
    pending.sort_unstable_by_key(key);
    *counter = counter.wrapping_add(u64::try_from(pending.len()).unwrap_or(u64::MAX));
}

/// Divide-and-Truncate pass: floor lift plus neighbor separation, each
/// correction divided into `substeps` equal parts with every part truncated
/// to `max_step_m`. Index loops over caller-owned slices only, so the pass
/// performs no steady-state allocation; math is `+ - * /` plus one `sqrt`
/// per bone pair, all deterministic in skeleton order.
fn resolve_coupling(
    bodies: &mut RigidBodySet,
    order: &[RigidBodyHandle],
    radii: &[f32],
    params: &CouplingParams,
    stats: &mut CouplingStats,
) {
    if !params.enabled {
        return;
    }
    resolve_floor(bodies, order, radii, params, stats);
    resolve_spacing(bodies, order, params, stats);
}

/// Floor lift: a bone center below `floor_top_y + radius` rises back to the
/// plane in divided, truncated parts, and downward velocity dies there so
/// the next step does not re-tunnel. A correction the per-tick authority
/// cannot cover stays counted as a residual penetration event.
fn resolve_floor(
    bodies: &mut RigidBodySet,
    order: &[RigidBodyHandle],
    radii: &[f32],
    params: &CouplingParams,
    stats: &mut CouplingStats,
) {
    let parts = f32::from(params.substeps);
    for (i, handle) in order.iter().enumerate() {
        let Some(radius) = radii.get(i).copied() else {
            continue;
        };
        let Some(body) = bodies.get_mut(*handle) else {
            continue;
        };
        let y = body.translation().y;
        if !y.is_finite() {
            continue;
        }
        let need = params.floor_top_y + radius - y;
        if need <= 0.0 {
            continue;
        }
        let each = need / parts;
        let clamped = if each > params.max_step_m {
            params.max_step_m
        } else {
            each
        };
        // One condition guards both counters: `clamped < each` means the
        // per-tick authority (`max_step_m * substeps`) cannot cover `need`,
        // so the cap shortened the move and a residual stays behind. When
        // the parts cover the need, `(need / parts) * parts` lands on the
        // plane up to float rounding and no event is counted.
        if clamped < each {
            stats.truncations = stats.truncations.wrapping_add(1);
            stats.penetration_events = stats.penetration_events.wrapping_add(1);
        }
        let applied = clamped * parts;
        let t = body.translation();
        // Sleeping is disabled on every bone, so no wake is needed: `false`
        // keeps the pass out of Rapier's user-change lists and the tick
        // inside its allocation budget (see the audit test below).
        body.set_translation(Vector::new(t.x, t.y + applied, t.z), false);
        let lv = body.linvel();
        if lv.y < 0.0 {
            body.set_linvel(Vector::new(lv.x, 0.0, lv.z), false);
        }
        if applied > stats.max_correction_m {
            stats.max_correction_m = applied;
        }
    }
}

/// Neighbor separation: bone centers closer than `min_spacing_m` part along
/// the pair axis in divided, truncated halves, so clinched bones never sit
/// inside each other. A degenerate zero-distance pair parts along +X.
/// Same single-condition counters as the floor pass: a shortened move
/// leaves a counted residual.
fn resolve_spacing(
    bodies: &mut RigidBodySet,
    order: &[RigidBodyHandle],
    params: &CouplingParams,
    stats: &mut CouplingStats,
) {
    let parts = f32::from(params.substeps);
    let n = order.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let (pi, pj) = match (bodies.get(order[i]), bodies.get(order[j])) {
                (Some(a), Some(b)) => {
                    let ta = a.translation();
                    let tb = b.translation();
                    ([ta.x, ta.y, ta.z], [tb.x, tb.y, tb.z])
                }
                _ => continue,
            };
            if pi.iter().chain(pj.iter()).any(|c| !c.is_finite()) {
                continue;
            }
            let d = [pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]];
            let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if !d2.is_finite() {
                continue;
            }
            // One `sqrt` per pair, allowed sim math.
            let dist = d2.sqrt();
            let need = params.min_spacing_m - dist;
            if need <= 0.0 {
                continue;
            }
            // Unit axis from `i` to `j`; a zero-distance pair parts along +X
            // so the direction is always defined and deterministic.
            let axis = if dist > 0.0 {
                [d[0] / dist, d[1] / dist, d[2] / dist]
            } else {
                [1.0, 0.0, 0.0]
            };
            let each = need * 0.5 / parts;
            let clamped = if each > params.max_step_m {
                params.max_step_m
            } else {
                each
            };
            if clamped < each {
                stats.truncations = stats.truncations.wrapping_add(1);
                stats.penetration_events = stats.penetration_events.wrapping_add(1);
            }
            let half = clamped * parts;
            // No wake: bones never sleep, and `false` keeps the pass out of
            // Rapier's user-change lists (see the floor pass and the audit).
            if let Some(a) = bodies.get_mut(order[i]) {
                let t = a.translation();
                a.set_translation(
                    Vector::new(
                        t.x - axis[0] * half,
                        t.y - axis[1] * half,
                        t.z - axis[2] * half,
                    ),
                    false,
                );
            }
            if let Some(b) = bodies.get_mut(order[j]) {
                let t = b.translation();
                b.set_translation(
                    Vector::new(
                        t.x + axis[0] * half,
                        t.y + axis[1] * half,
                        t.z + axis[2] * half,
                    ),
                    false,
                );
            }
            if half > stats.max_correction_m {
                stats.max_correction_m = half;
            }
        }
    }
}

/// Parent-to-child spherical joints anchored at the rest-pose midpoint of
/// each pair, with depth-limited swing and contacts off between joined
/// bodies. Split from `spawn` while it is open so the constructor stays
/// short; the math is unchanged. Public so the float-only physical layer
/// (`engine-physfx`) joins its ragdoll bones the same way this ragdoll does.
pub fn insert_joints(
    skeleton: &Skeleton,
    world_pos: &[[f32; 3]],
    world_rot: &[[f32; 4]],
    depths: &[u32],
    order: &[RigidBodyHandle],
    impulse_joints: &mut ImpulseJointSet,
) {
    for (i, joint) in skeleton.joints.iter().enumerate() {
        let Some(parent) = joint.parent else {
            continue;
        };
        let mid = [
            (world_pos[parent][0] + world_pos[i][0]) * 0.5,
            (world_pos[parent][1] + world_pos[i][1]) * 0.5,
            (world_pos[parent][2] + world_pos[i][2]) * 0.5,
        ];
        let a1 = to_local(&world_pos[parent], &world_rot[parent], &mid);
        let a2 = to_local(&world_pos[i], &world_rot[i], &mid);
        let limit = swing_limit(depths[i]);
        let data = SphericalJointBuilder::new()
            .local_anchor1(Vector::new(a1[0], a1[1], a1[2]))
            .local_anchor2(Vector::new(a2[0], a2[1], a2[2]))
            .contacts_enabled(false)
            .limits(JointAxis::AngX, [-limit, limit])
            .limits(JointAxis::AngY, [-limit, limit])
            .limits(JointAxis::AngZ, [-limit, limit])
            .build();
        impulse_joints.insert(order[parent], order[i], data, true);
    }
}

/// Depth of each joint (root is 0). Parents-first order from skeleton
/// validation means the parent depth is already known when a child is visited.
pub fn joint_depths(skeleton: &Skeleton) -> Vec<u32> {
    let mut depths = vec![0u32; skeleton.len()];
    for (i, joint) in skeleton.joints.iter().enumerate() {
        if let Some(p) = joint.parent {
            depths[i] = depths[p].saturating_add(1);
        }
    }
    depths
}

/// Swing limit for a joint at `depth`: widest at the root's children, tighter
/// deeper, never below [`SWING_MIN`]. The only skeleton signal is hierarchy, so
/// depth is the whole limit source; the values are plain stored numbers, no
/// trigonometric calls.
pub(crate) fn swing_limit(depth: u32) -> f32 {
    let shrink = SWING_STEP * depth as f32;
    let limit = SWING_ROOT - shrink;
    if limit < SWING_MIN {
        SWING_MIN
    } else {
        limit
    }
}

/// Child-to-parent mass ratio floor: stabilize keeps `child / parent` here or above.
/// Units: ratio, dimensionless. Determinism: exact `f32` compare, no rounding.
pub const MASS_RATIO_LO: f32 = 0.8;
/// Child-to-parent mass ratio ceiling, same units and determinism as [`MASS_RATIO_LO`].
pub const MASS_RATIO_HI: f32 = 1.2;
/// Friction motor torque for hinge joints (knees, elbows), newton-meters.
pub const JOINT_FRICTION_HINGE_NM: f32 = 10.0;
/// Friction motor torque for spherical joints, newton-meters.
pub const JOINT_FRICTION_SPHERICAL_NM: f32 = 5.0;
/// Spring rate for hinge joints, hertz.
pub const JOINT_SPRING_HINGE_HZ: f32 = 2.0;
/// Spring rate for spherical joints, hertz.
pub const JOINT_SPRING_SPHERICAL_HZ: f32 = 0.5;
/// Symmetric twist half-range for spherical joints, radians.
pub const JOINT_TWIST_SPHERICAL_RAD: f32 = 0.8;

/// Per-bone joint kind: hinge bends one way (knees, elbows), spherical cones plus twists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointKind {
    /// Single-axis hinge: flexion in `[0, swing_limit]`, no twist.
    Hinge,
    /// Ball joint: swing cone plus symmetric twist.
    Spherical,
}

/// Per-bone joint spec: kind plus limits plus drive tuning. Units: radians, N.m, Hz.
/// Determinism: plain values, never hashed (measurement beside spawn, not wired).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointSpec {
    /// Hinge or spherical.
    pub kind: JointKind,
    /// Cone half-angle (spherical) or flexion cap (hinge), radians.
    pub swing_limit: f32,
    /// Twist range low, radians; hinge locks to `0`.
    pub twist_min: f32,
    /// Twist range high, radians; hinge locks to `0`.
    pub twist_max: f32,
    /// Friction motor torque, N.m, in `5..=10`.
    pub friction_torque: f32,
    /// Spring rate, Hz, in `0.5..=2`.
    pub spring_hz: f32,
}

impl JointSpec {
    /// Clamp a hinge flexion angle into `[0, swing_limit]`. Non-finite input maps
    /// to `0`. Math is comparisons only, deterministic on every platform.
    pub fn clamp_hinge(&self, angle: f32) -> f32 {
        if !angle.is_finite() || !self.swing_limit.is_finite() {
            return 0.0;
        }
        if angle < 0.0 {
            0.0
        } else if angle > self.swing_limit {
            self.swing_limit
        } else {
            angle
        }
    }

    /// Clamp a twist angle into `[twist_min, twist_max]`. Non-finite input maps
    /// to `twist_min`. Comparisons only, deterministic on every platform.
    pub fn clamp_twist(&self, angle: f32) -> f32 {
        if !angle.is_finite() {
            return self.twist_min;
        }
        if angle < self.twist_min {
            self.twist_min
        } else if angle > self.twist_max {
            self.twist_max
        } else {
            angle
        }
    }
}

/// Per-bone joint table beside the depth-only swing limits: one [`JointSpec`]
/// per bone in skeleton order (root row unused, parent is `None`). Measurement
/// only: `spawn` does not read it yet (follow-up wiring packet), so it never
/// reaches the hash.
#[derive(Clone, Debug, PartialEq)]
pub struct JointTable {
    /// One entry per bone, skeleton order.
    pub rows: Vec<JointSpec>,
}

impl JointTable {
    /// Humanoid default: hinge knees and elbows (name contains `knee` or
    /// `elbow`), cone plus twist elsewhere. Swing reuses [`swing_limit`] by
    /// depth, friction `5..=10` N.m (hinge `10`, spherical `5`), spring
    /// `0.5..=2` Hz (hinge `2`, spherical `0.5`), hinge twist locked,
    /// spherical twist `+-0.8` rad. Allocates once for the table (never per
    /// tick); math is comparisons plus the depth-limit multiply in `swing_limit`.
    pub fn humanoid(skeleton: &Skeleton) -> Self {
        let depths = joint_depths(skeleton);
        let mut rows = Vec::with_capacity(skeleton.len());
        for (i, joint) in skeleton.joints.iter().enumerate() {
            let depth = depths.get(i).copied().unwrap_or(0);
            let swing = swing_limit(depth);
            if is_hinge_name(&joint.name) {
                rows.push(JointSpec {
                    kind: JointKind::Hinge,
                    swing_limit: swing,
                    twist_min: 0.0,
                    twist_max: 0.0,
                    friction_torque: JOINT_FRICTION_HINGE_NM,
                    spring_hz: JOINT_SPRING_HINGE_HZ,
                });
            } else {
                rows.push(JointSpec {
                    kind: JointKind::Spherical,
                    swing_limit: swing,
                    twist_min: -JOINT_TWIST_SPHERICAL_RAD,
                    twist_max: JOINT_TWIST_SPHERICAL_RAD,
                    friction_torque: JOINT_FRICTION_SPHERICAL_NM,
                    spring_hz: JOINT_SPRING_SPHERICAL_HZ,
                });
            }
        }
        Self { rows }
    }

    /// Row count (one per bone).
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Spec for one bone, `None` past the end.
    pub fn spec(&self, bone: usize) -> Option<JointSpec> {
        self.rows.get(bone).copied()
    }
}

/// True for hinge bones: name contains `knee` or `elbow` (contract names are
/// lowercase `[a-z0-9_]`). Pure string match, no allocation.
fn is_hinge_name(name: &str) -> bool {
    name.contains("knee") || name.contains("elbow")
}

/// Stabilize child-to-parent mass ratios into `[0.8, 1.2]` preserving total
/// mass, skeleton order (parents first). Pure: slices in, slices out, no
/// allocation, `+ - * /` plus comparisons only, stable skeleton order,
/// deterministic on every platform. Length mismatch or empty input does
/// nothing (never panics); non-finite or non-positive masses sanitize to `1.0`
/// before the pass. Pass: children absorb parents first (a fixed parent never
/// moves again, so deeper links stay fixed; one shared parent can fit every
/// disparate child, while one parent value cannot fit two disparate children).
/// Then one uniform rescale restores the total (uniform scale keeps every
/// ratio, up to float rounding).
/// Units: kilograms; errors: none (infallible, measurement beside spawn).
pub fn stabilize_masses(masses: &mut [f32], skeleton: &Skeleton) {
    if masses.is_empty() || masses.len() != skeleton.len() {
        return;
    }
    for m in masses.iter_mut() {
        if !m.is_finite() || *m <= 0.0 {
            *m = 1.0;
        }
    }
    let mut total_before = 0.0f32;
    for m in masses.iter() {
        total_before += *m;
    }
    if !total_before.is_finite() || total_before <= 0.0 {
        return;
    }
    for i in 0..skeleton.len() {
        let Some(parent) = skeleton.joints[i].parent else {
            continue;
        };
        if parent >= masses.len() {
            continue;
        }
        let child = masses[i];
        let par = masses[parent];
        if !child.is_finite() || !par.is_finite() || par <= 0.0 || child <= 0.0 {
            continue;
        }
        let ratio = child / par;
        if ratio < MASS_RATIO_LO {
            masses[i] = par * MASS_RATIO_LO;
        } else if ratio > MASS_RATIO_HI {
            masses[i] = par * MASS_RATIO_HI;
        }
    }
    let mut total_after = 0.0f32;
    for m in masses.iter() {
        total_after += *m;
    }
    if !total_after.is_finite() || total_after <= 0.0 {
        return;
    }
    let scale = total_before / total_after;
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    for m in masses.iter_mut() {
        *m *= scale;
    }
}

/// Animated world transforms, parent-first: positions compose by rotated
/// offset plus parent position, rotations by quaternion multiply. Only
/// `+ - *` math; callers validate lengths and parent order first.
pub fn world_transforms(skeleton: &Skeleton, pose: &LocalPose) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let mut pos = vec![[0.0f32; 3]; skeleton.len()];
    let mut rot = vec![[0.0f32, 0.0, 0.0, 1.0]; skeleton.len()];
    for (i, joint) in skeleton.joints.iter().enumerate() {
        let (pp, pr) = match joint.parent {
            Some(p) => (pos[p], rot[p]),
            None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
        };
        let rp = rotate_vec(&pose.positions[i], &pr);
        pos[i] = [pp[0] + rp[0], pp[1] + rp[1], pp[2] + rp[2]];
        rot[i] = quat_mul(pr, pose.rotations[i]);
    }
    (pos, rot)
}

/// Simulated world transforms read off `order`'s bodies in skeleton order,
/// shared by the plain and powered ragdolls. Non-finite components (which
/// Rapier never produces, but hand-driven or corrupt states could) fall back
/// to `rest_pos`, the animated rest transform, so a bad body never poisons
/// the blended pose with NaN; a missing body reads as its rest position and
/// the identity rotation. `rest_pos` holds one entry per handle in `order`.
pub(crate) fn read_sim_transforms(
    bodies: &RigidBodySet,
    order: &[RigidBodyHandle],
    rest_pos: &[[f32; 3]],
) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let mut pos = Vec::with_capacity(order.len());
    let mut rot = Vec::with_capacity(order.len());
    for (i, handle) in order.iter().enumerate() {
        match bodies.get(*handle) {
            Some(body) => {
                let t = body.translation();
                let r = body.rotation();
                let p = [t.x, t.y, t.z];
                let q = [r.x, r.y, r.z, r.w];
                pos.push(if p.iter().all(|c| c.is_finite()) {
                    p
                } else {
                    rest_pos[i]
                });
                rot.push(normalize_quat(&q));
            }
            None => {
                pos.push(rest_pos[i]);
                rot.push([0.0, 0.0, 0.0, 1.0]);
            }
        }
    }
    (pos, rot)
}

/// Express a world point in a body-local frame (center plus unit rotation).
/// Inverse rotation is the conjugate: no division, no trig.
pub(crate) fn to_local(center: &[f32; 3], quat: &[f32; 4], point: &[f32; 3]) -> [f32; 3] {
    let d = [
        point[0] - center[0],
        point[1] - center[1],
        point[2] - center[2],
    ];
    rotate_vec(&d, &quat_conj(quat))
}

pub(crate) fn quat_conj(q: &[f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

pub(crate) fn rotate_vec(v: &[f32; 3], q: &[f32; 4]) -> [f32; 3] {
    let t = quat_mul(*q, [v[0], v[1], v[2], 0.0]);
    let r = quat_mul(t, quat_conj(q));
    [r[0], r[1], r[2]]
}

/// Normalize a quaternion (one `sqrt`, allowed sim math); zero or non-finite
/// input falls back to identity so bad data never injects NaN.
pub(crate) fn normalize_quat(q: &[f32; 4]) -> [f32; 4] {
    if q.iter().any(|c| !c.is_finite()) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let n2 = q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3];
    if n2 <= 0.0 || !n2.is_finite() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let n = n2.sqrt();
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

/// Rapier rotation from an animated quaternion, normalized the same way.
pub(crate) fn normalized_rotation(q: &[f32; 4]) -> Rotation {
    let nq = normalize_quat(q);
    Rotation::from_xyzw(nq[0], nq[1], nq[2], nq[3]).normalize()
}

fn check_pos(pos: [f32; 3]) -> Result<(), Error> {
    if position_in_bounds(pos) {
        Ok(())
    } else {
        Err(Error::NonFinitePosition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::Joint;

    /// Four-bone vertical chain floating above the floor: root at y=3, each
    /// child offset +0.5 in Y.
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

    fn chain_pose() -> LocalPose {
        LocalPose {
            positions: vec![
                [0.0, 3.0, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 0.0],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 4],
        }
    }

    fn chain_shapes() -> Vec<BoneShape> {
        vec![
            BoneShape {
                radius: 0.15,
                half_height: 0.2,
            };
            4
        ]
    }

    fn ragdoll_on_floor() -> Ragdoll {
        let mut r =
            Ragdoll::spawn(&chain_skeleton(), &chain_pose(), &chain_shapes()).expect("spawn chain");
        r.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        r
    }

    fn step_n(r: &mut Ragdoll, n: usize) {
        for _ in 0..n {
            r.step(FIXED_DT).expect("fixed step");
        }
    }

    #[test]
    fn balance_hash_equal_with_ragdoll_on_and_off() {
        // EG34 split: the Rapier ragdoll is client-local, seeded from each
        // balance verdict and blended by its own weight; the authoritative
        // balance hash is equal with the ragdoll running or absent.
        use crate::balance::{knockdown_script, GaitVerdict};
        let off = knockdown_script(&mut |_, _| {});
        let mut ragdoll = ragdoll_on_floor();
        let spawned = ragdoll.state_hash();
        let on = knockdown_script(&mut |tick, verdict| {
            let blend = match verdict {
                GaitVerdict::Steady => 0.0,
                GaitVerdict::Staggered => 0.3,
                GaitVerdict::Stumbling => 0.6,
                GaitVerdict::Fallen => 1.0,
            };
            ragdoll.blend_to_ragdoll(blend).expect("blend in range");
            ragdoll
                .apply_hit_event(HitEvent {
                    bone: 1,
                    impulse: [40.0 * blend, 0.0, 0.0],
                    tick,
                })
                .expect("bone in range");
            step_n(&mut ragdoll, 10);
        });
        assert_eq!(on, off, "ragdoll on and off: equal balance digests");
        assert_ne!(ragdoll.state_hash(), spawned, "the client ragdoll moved");
    }
    fn speed(v: [f32; 3]) -> f32 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    /// Pose-space distance between two local poses (positions plus quat
    /// components, plain `+ - *` sums so the test asserts on the blend math,
    /// not on platform transcendentals).
    fn pose_error(a: &LocalPose, b: &LocalPose) -> f32 {
        let mut e = 0.0f32;
        for (x, y) in a.positions.iter().zip(b.positions.iter()) {
            e += (x[0] - y[0]).abs() + (x[1] - y[1]).abs() + (x[2] - y[2]).abs();
        }
        for (x, y) in a.rotations.iter().zip(b.rotations.iter()) {
            e += (x[0] - y[0]).abs()
                + (x[1] - y[1]).abs()
                + (x[2] - y[2]).abs()
                + (x[3] - y[3]).abs();
        }
        e
    }

    #[test]
    fn spawn_matches_animated_pose_at_blend_zero() {
        let r = ragdoll_on_floor();
        assert_eq!(r.blend(), 0.0);
        assert_eq!(r.bone_count(), 4);
        // Blend 0 returns the animated driver pose exactly, not approximately.
        assert_eq!(r.bone_poses(), chain_pose());
        // Bodies start at the animated world transforms: root at y=3, each
        // child +0.5 above its parent.
        for (i, want) in [3.0, 3.5, 4.0, 4.5].iter().enumerate() {
            let p = r.bone_position(i).expect("bone");
            assert!(
                (p[1] - want).abs() < 1e-6,
                "bone {i}: y={} want {want}",
                p[1]
            );
            assert!(p[0].abs() < 1e-6 && p[2].abs() < 1e-6, "{p:?}");
        }
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn fold(h: &mut u64, bytes: &[u8]) {
            for b in bytes {
                *h ^= u64::from(*b);
                *h = h.wrapping_mul(FNV64_PRIME);
            }
        }
        fn legacy(r: &Ragdoll) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            for handle in &r.order {
                let Some(body) = r.bodies.get(*handle) else {
                    continue;
                };
                let t = body.translation();
                let rv = body.rotation();
                let lv = body.linvel();
                let av = body.angvel();
                for v in [
                    t.x, t.y, t.z, rv.x, rv.y, rv.z, rv.w, lv.x, lv.y, lv.z, av.x, av.y, av.z,
                ] {
                    fold(&mut h, &quantize(v).to_le_bytes());
                }
            }
            fold(&mut h, &quantize(r.blend).to_le_bytes());
            for b in r.tick.to_le_bytes().into_iter().chain(r.hits.to_le_bytes()) {
                h ^= u64::from(b);
                h = h.wrapping_mul(FNV64_PRIME);
            }
            for b in r
                .coupling_stats
                .penetration_events
                .to_le_bytes()
                .into_iter()
                .chain(r.coupling_stats.truncations.to_le_bytes())
            {
                h ^= u64::from(b);
                h = h.wrapping_mul(FNV64_PRIME);
            }
            fold(
                &mut h,
                &quantize(r.coupling_stats.max_correction_m).to_le_bytes(),
            );
            h ^= u64::from(r.coupling.substeps);
            h = h.wrapping_mul(FNV64_PRIME);
            h ^= u64::from(r.coupling.enabled as u8);
            h = h.wrapping_mul(FNV64_PRIME);
            for v in [
                r.coupling.max_step_m,
                r.coupling.floor_top_y,
                r.coupling.min_spacing_m,
            ] {
                fold(&mut h, &quantize(v).to_le_bytes());
            }
            for c in &r.ccd_flags {
                h ^= u64::from(u8::from(*c));
                h = h.wrapping_mul(FNV64_PRIME);
            }
            for b in r.contact_events.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(FNV64_PRIME);
            }
            for p in r.animated.positions.iter().flatten() {
                fold(&mut h, &quantize(*p).to_le_bytes());
            }
            for q in r.animated.rotations.iter().flatten() {
                fold(&mut h, &quantize(*q).to_le_bytes());
            }
            h
        }
        // Fresh spawn plus a stepped ragdoll cover zero and nonzero counters.
        let fresh = ragdoll_on_floor();
        assert_eq!(fresh.state_hash(), legacy(&fresh));
        let mut r = ragdoll_on_floor();
        r.blend_to_ragdoll(1.0).expect("full ragdoll");
        step_n(&mut r, 30);
        assert_eq!(r.state_hash(), legacy(&r));
    }

    #[test]
    fn full_ragdoll_falls_and_rests() {
        let mut r = ragdoll_on_floor();
        r.blend_to_ragdoll(1.0).expect("full ragdoll");
        step_n(&mut r, 600);
        // The chain fell: every bone is below its spawn height.
        for (i, top) in [3.0, 3.5, 4.0, 4.5].iter().enumerate() {
            let p = r.bone_position(i).expect("bone");
            assert!(p[1] < top - 1.0, "bone {i} must fall: {p:?}");
            assert!(p[1].is_finite() && p[0].is_finite() && p[2].is_finite());
            // Bones rest above the floor, never through it.
            assert!(p[1] > -0.5, "bone {i} must not tunnel: {p:?}");
        }
        // ... and came to rest: residual speeds are small.
        let mut total = 0.0f32;
        for i in 0..4 {
            total += speed(r.bone_linvel(i).expect("vel"));
        }
        assert!(total < 1.0, "ragdoll must rest, total speed={total}");
    }

    #[test]
    fn hit_impulse_displaces_struck_bone_down_chain() {
        // Twin runs: settle both to rest, then only A takes a +X hit on bone
        // 1. Measuring early (20 steps) captures joint drag before the chain
        // can whip back on its limits.
        let mut a = ragdoll_on_floor();
        let mut b = ragdoll_on_floor();
        for r in [&mut a, &mut b] {
            r.blend_to_ragdoll(1.0).expect("full ragdoll");
        }
        step_n(&mut a, 150);
        step_n(&mut b, 150);
        a.apply_hit(1, [0.05, 0.0, 0.0], 150).expect("hit bone 1");
        step_n(&mut a, 20);
        step_n(&mut b, 20);
        // The struck bone travels with the impulse; descendants are displaced
        // through the joints. Rotation about the joint anchors can fling a
        // descendant opposite in X (here bone 3 swings down instead of +X),
        // so descendants assert 3D displacement magnitude, not direction.
        let p1a = a.bone_position(1).expect("bone 1a");
        let p1b = b.bone_position(1).expect("bone 1b");
        assert!(
            p1a[0] - p1b[0] > 0.1,
            "struck bone must travel with the hit: {p1a:?} vs {p1b:?}"
        );
        for i in 2..4 {
            let pa = a.bone_position(i).expect("bone a");
            let pb = b.bone_position(i).expect("bone b");
            let dist = (pa[0] - pb[0]).abs() + (pa[1] - pb[1]).abs() + (pa[2] - pb[2]).abs();
            assert!(dist > 0.1, "bone {i} must displace down-chain: {dist}");
        }
        // The hit ran one way only: the untouched twin barely moved in X.
        let root_b = b.bone_position(0).expect("root b");
        assert!(root_b[0].abs() < 0.3, "{root_b:?}");
    }

    #[test]
    fn blend_zero_to_one_interpolates_monotonically() {
        // Assertion documented: positions lerp and rotations nlerp per bone,
        // both linear in the weight, so the pose-space L1 distance from the
        // animated pose is non-decreasing in the weight up to float rounding
        // (tolerance 1e-4). Weight 0 is exact; weight 1 is the full sim pose.
        let mut r = ragdoll_on_floor();
        r.blend_to_ragdoll(1.0).expect("full ragdoll");
        step_n(&mut r, 120);
        r.blend_to_ragdoll(0.0).expect("back to animated");
        let animated = chain_pose();
        assert_eq!(r.bone_poses(), animated);
        let mut prev = 0.0f32;
        let mut full = 0.0f32;
        for w in [0.0, 0.25, 0.5, 0.75, 1.0] {
            r.blend_to_ragdoll(w).expect("weight");
            let e = pose_error(&r.bone_poses(), &animated);
            assert!(
                e + 1e-4 >= prev,
                "error must not decrease: w={w} e={e} prev={prev}"
            );
            prev = e;
            full = e;
        }
        assert!(full > 0.5, "sim must diverge from animated: {full}");
    }

    #[test]
    fn determinism_twice_identical() {
        // Same spawn plus same impulse script twice: identical hashes and
        // identical blended poses.
        let script = || {
            let mut r = ragdoll_on_floor();
            r.blend_to_ragdoll(0.75).expect("blend");
            step_n(&mut r, 20);
            r.apply_hit(2, [0.0, 0.0, 0.04], 20).expect("hit");
            step_n(&mut r, 40);
            r.apply_hit(0, [-0.03, 0.0, 0.0], 60).expect("hit root");
            step_n(&mut r, 60);
            (r.state_hash(), r.bone_poses())
        };
        let (h1, p1) = script();
        let (h2, p2) = script();
        assert_eq!(h1, h2);
        assert_eq!(p1, p2);
    }

    #[test]
    fn snapshot_restore_exact() {
        let mut r = ragdoll_on_floor();
        r.blend_to_ragdoll(1.0).expect("full ragdoll");
        step_n(&mut r, 40);
        r.apply_hit(1, [0.02, 0.0, 0.0], 40).expect("hit");
        let snap = r.snapshot();
        step_n(&mut r, 80);
        let (after_hash, after_pose) = (r.state_hash(), r.bone_poses());
        r.restore(snap.clone()).expect("restore");
        // Restored state matches the snapshot exactly, not approximately.
        assert_eq!(r.snapshot(), snap);
        step_n(&mut r, 80);
        assert_eq!(r.state_hash(), after_hash);
        assert_eq!(r.bone_poses(), after_pose);
    }

    #[test]
    fn mirrored_hits_stay_mirrored() {
        // Twin runs mirrored across the YZ plane: both chains settle from the
        // same X-symmetric spawn, then A takes a +X hit on bone 1 and B the
        // negated -X hit, so every bone must stay mirrored (X negated, Y/Z
        // equal) through the jointed fall.
        let mut a = ragdoll_on_floor();
        let mut b = ragdoll_on_floor();
        for r in [&mut a, &mut b] {
            r.blend_to_ragdoll(1.0).expect("full ragdoll");
        }
        step_n(&mut a, 150);
        step_n(&mut b, 150);
        a.apply_hit(1, [0.05, 0.0, 0.0], 150).expect("hit A");
        b.apply_hit(1, [-0.05, 0.0, 0.0], 150).expect("hit B");
        step_n(&mut a, 20);
        step_n(&mut b, 20);
        // The hits actually split the struck bones in X, so the mirror
        // relation below is exercised, not vacuous.
        let ax = a.bone_position(1).expect("bone A")[0];
        let bx = b.bone_position(1).expect("bone B")[0];
        assert!(ax - bx > 0.05, "twins must split in X: {ax} vs {bx}");
        for i in 0..4 {
            let pa = a.bone_position(i).expect("bone A");
            let pb = b.bone_position(i).expect("bone B");
            assert!(
                (pa[0] + pb[0]).abs() < 1e-3
                    && (pa[1] - pb[1]).abs() < 1e-3
                    && (pa[2] - pb[2]).abs() < 1e-3,
                "bone {i} must stay mirrored: {pa:?} vs {pb:?}"
            );
        }
        // A mirrored checkpoint resumes a mirrored future: restore A from a
        // mirror of B's snapshot (YZ reflection negates X of positions and
        // linear velocities, maps rotation `(x, y, z, w)` to `(x, -y, -z, w)`
        // and angular velocity to `(wx, -wy, -wz)`), step both, and every
        // bone is mirrored again.
        let mut snapshot_b = b.snapshot();
        for body in &mut snapshot_b.bodies {
            body.pos[0] = -body.pos[0];
            body.linvel[0] = -body.linvel[0];
            body.rot[1] = -body.rot[1];
            body.rot[2] = -body.rot[2];
            body.angvel[1] = -body.angvel[1];
            body.angvel[2] = -body.angvel[2];
        }
        a.restore(snapshot_b).expect("restore mirrored");
        step_n(&mut a, 20);
        step_n(&mut b, 20);
        for i in 0..4 {
            let pa = a.bone_position(i).expect("bone A");
            let pb = b.bone_position(i).expect("bone B");
            assert!(
                (pa[0] + pb[0]).abs() < 1e-3
                    && (pa[1] - pb[1]).abs() < 1e-3
                    && (pa[2] - pb[2]).abs() < 1e-3,
                "resumed bone {i} must stay mirrored: {pa:?} vs {pb:?}"
            );
        }
    }

    #[test]
    fn step_allocations_audited_and_pinned() {
        // Allocation audit: our tick glue allocates nothing — `step` plus
        // the coupling pass only write Rapier bodies through index handles
        // and `state_hash` folds on the stack — while `PhysicsPipeline::step`
        // from rapier3d 0.35 keeps per-step locals (substep moved/quarantine
        // lists, contact manifolds while piling) that no caller
        // configuration removes. The seam's kinematic lifts route through
        // those same rapier lists, so a piling chain measures ~9 heap calls
        // and ~760 bytes per step, steady-state (was ~6 calls and ~590
        // bytes before the seam; both runs pin 523 calls / 45440 bytes for
        // 60 steps exactly, so the cost is deterministic, not a leak). The
        // pin below guards that budget instead of asserting an unreachable
        // zero; the hash probe after it still demands zero from our code.
        let mut r = ragdoll_on_floor();
        r.blend_to_ragdoll(1.0).expect("full ragdoll");
        step_n(&mut r, 30);
        r.apply_hit(1, [0.02, 0.0, 0.0], 30).expect("hit");
        let (hash_before, tick_allocs) = engine_core::alloc_probe::measure(|| {
            step_n(&mut r, 60);
            r.state_hash()
        });
        assert!(
            tick_allocs.calls <= 600 && tick_allocs.bytes <= 52_000,
            "tick budget: 60 steps must stay near 9 calls/step inside rapier, got {tick_allocs:?}"
        );
        let (_, hash_allocs) = engine_core::alloc_probe::measure(|| r.state_hash());
        assert_eq!(
            hash_allocs.calls, 0,
            "our hash must not allocate, got {hash_allocs:?}"
        );
        // The probed run is real work: the body fell and the hash advanced
        // with the tick, so the budget above guards a live tick, not a no-op.
        assert_eq!(r.state_hash(), hash_before);
        let top = [3.0, 3.5, 4.0, 4.5];
        let mut fell = false;
        for (i, t) in top.iter().enumerate() {
            let p = r.bone_position(i).expect("bone");
            fell |= p[1] < t - 0.1;
        }
        assert!(fell, "chain must fall during the probed steps");
    }

    #[test]
    fn invalid_input_rejected_without_panic() {
        let skeleton = chain_skeleton();
        let pose = chain_pose();
        let shapes = chain_shapes();
        // Empty skeleton, pose length mismatch, shape count mismatch.
        assert!(Ragdoll::spawn(&Skeleton::default(), &pose, &shapes).is_err());
        let mut short = pose.clone();
        short.positions.pop();
        assert!(Ragdoll::spawn(&skeleton, &short, &shapes).is_err());
        assert!(Ragdoll::spawn(&skeleton, &pose, &shapes[..2]).is_err());
        // Bad capsule dimensions.
        let mut bad = shapes.clone();
        bad[0].radius = 0.0;
        assert_eq!(
            Ragdoll::spawn(&skeleton, &pose, &bad).map(|_| ()),
            Err(Error::InvalidRadius)
        );
        bad = shapes.clone();
        bad[1].half_height = f32::NAN;
        assert_eq!(
            Ragdoll::spawn(&skeleton, &pose, &bad).map(|_| ()),
            Err(Error::InvalidHeight)
        );
        let mut r = ragdoll_on_floor();
        // Bad bones, impulses, blend weights and dt.
        assert_eq!(
            r.apply_hit(99, [0.01, 0.0, 0.0], 0),
            Err(Error::BadBone(99))
        );
        assert_eq!(
            r.apply_hit(1, [f32::INFINITY, 0.0, 0.0], 0),
            Err(Error::NonFiniteImpulse)
        );
        assert_eq!(r.blend_to_ragdoll(f32::NAN), Err(Error::BadBlend));
        assert_eq!(r.blend_to_ragdoll(1.5), Err(Error::BadBlend));
        assert_eq!(r.blend_to_ragdoll(-0.1), Err(Error::BadBlend));
        assert_eq!(r.step(0.0), Err(Error::InvalidDt));
        assert_eq!(r.step(f32::NAN), Err(Error::InvalidDt));
        assert!(r.add_static_box([0.0, 0.5, 0.5], [0.0, 0.0, 0.0]).is_err());
        // Mismatched snapshot restores nothing; animated-length mismatch too.
        assert_eq!(
            r.restore(RagdollSnapshot {
                bodies: vec![],
                animated: chain_pose(),
                blend: 1.0,
                tick: 0,
                coupling_stats: CouplingStats::default(),
                ccd_flags: vec![],
                contact_events: 0,
            }),
            Err(Error::SnapshotMismatch)
        );
        let mut snap = r.snapshot();
        snap.animated.positions.pop();
        assert_eq!(r.restore(snap), Err(Error::SnapshotMismatch));
        // HitEvent entry point accepts the same local event the caller maps.
        assert!(r
            .apply_hit_event(HitEvent {
                bone: 2,
                impulse: [0.0, 0.0, 0.01],
                tick: 7,
            })
            .is_ok());
        assert!(r
            .apply_hit_event(HitEvent {
                bone: 9,
                impulse: [0.0, 0.0, 0.01],
                tick: 7,
            })
            .is_err());
        assert_eq!(r.bone_position(99), None);
        // Bad coupling params change nothing: zero substeps, over-ceiling
        // substeps, non-positive or over-wide steps, wild floor or spacing.
        let good = r.coupling();
        for bad in [
            CouplingParams {
                substeps: 0,
                ..good
            },
            CouplingParams {
                substeps: COUPLING_MAX_SUBSTEPS + 1,
                ..good
            },
            CouplingParams {
                max_step_m: 0.0,
                ..good
            },
            CouplingParams {
                max_step_m: 2.0,
                ..good
            },
            CouplingParams {
                max_step_m: f32::NAN,
                ..good
            },
            CouplingParams {
                floor_top_y: f32::INFINITY,
                ..good
            },
            CouplingParams {
                min_spacing_m: -0.1,
                ..good
            },
            CouplingParams {
                min_spacing_m: f32::NAN,
                ..good
            },
        ] {
            assert!(r.set_coupling(bad).is_err());
        }
        assert_eq!(r.coupling(), good);
        assert!(r
            .set_coupling(CouplingParams {
                substeps: 2,
                max_step_m: 0.02,
                floor_top_y: 0.0,
                min_spacing_m: 0.05,
                enabled: true,
            })
            .is_ok());
        assert_eq!(r.coupling().substeps, 2);
        r.set_coupling(good).expect("restore seam");
    }

    #[test]
    fn swing_limits_derive_from_depth() {
        // Contract probe: limits come from skeleton hierarchy depth alone.
        assert!((swing_limit(1) - 0.75).abs() < 1e-6);
        assert!((swing_limit(2) - 0.6).abs() < 1e-6);
        assert!(swing_limit(1) > swing_limit(3));
        assert!(swing_limit(100) >= SWING_MIN);
        // Depths follow the parent links: 0, 1, 2, 3 along the chain.
        assert_eq!(joint_depths(&chain_skeleton()), vec![0, 1, 2, 3]);
    }

    /// Weight ramp for the stand phase: full ragdoll until the budget
    /// window, then a linear blend back to the animated pose.
    fn stand_weight(tick: u64) -> f32 {
        let stand_start = 600 - COUPLING_GETUP_BUDGET_TICKS;
        if tick < stand_start {
            return 1.0;
        }
        let step = tick - stand_start;
        let done = step + 1;
        let num = f32::from(u16::try_from(done).unwrap_or(u16::MAX));
        let den = f32::from(u16::try_from(COUPLING_GETUP_BUDGET_TICKS).unwrap_or(u16::MAX));
        1.0 - num / den
    }

    /// Lowest bone-center clearance above the seam floor plane, meters;
    /// negative means a center sits inside the floor's bone-radius band.
    fn min_clearance(r: &Ragdoll) -> f32 {
        let mut best = f32::INFINITY;
        for i in 0..r.bone_count() {
            if let Some(p) = r.bone_position(i) {
                let clearance = p[1] - r.coupling().floor_top_y - 0.15;
                if clearance < best {
                    best = clearance;
                }
            }
        }
        best
    }

    #[test]
    fn coupling_knock_drive_stand_600_hash_x2_zero_penetration() {
        // Acceptance fixture: 600 ticks knock (hit at tick 30) / drive
        // (full ragdoll fall) / stand (blend back over the budget window).
        // Same script twice gives the same hash and pose; the seam leaves
        // zero residual penetration events; the stand lands back on the
        // animated pose inside the budget, else the asserts print numbers.
        fn script() -> (u64, LocalPose, CouplingStats, f32) {
            let mut r = ragdoll_on_floor();
            r.blend_to_ragdoll(1.0).expect("full ragdoll");
            for t in 0..600u64 {
                if t == 30 {
                    r.apply_hit(1, [0.06, 0.0, 0.0], 30).expect("knock");
                }
                r.blend_to_ragdoll(stand_weight(t)).expect("drive/stand");
                r.step(FIXED_DT).expect("fixed step");
            }
            let clearance = min_clearance(&r);
            (
                r.state_hash(),
                r.bone_poses(),
                r.coupling_stats(),
                clearance,
            )
        }
        let (h1, p1, s1, c1) = script();
        let (h2, p2, s2, c2) = script();
        assert_eq!(h1, h2, "seam fixture must hash x2: {h1:016x} vs {h2:016x}");
        assert_eq!(p1, p2, "seam fixture poses must match x2");
        assert_eq!(s1, s2, "seam counters must match x2: {s1:?} vs {s2:?}");
        assert_eq!(
            s1.penetration_events, 0,
            "zero residual penetration, stats={s1:?} clearance={c1}"
        );
        assert_eq!(c1, c2, "clearance must repeat: {c1} vs {c2}");
        assert!(
            c1 >= -1e-4,
            "bones rest on the floor band, never inside it: clearance={c1}"
        );
        // The stand used exactly the budget window and landed back on the
        // animated pose: blend weight hit zero on the last tick.
        assert_eq!(
            stand_weight(599),
            0.0,
            "stand must reach animated on the last tick"
        );
        assert_eq!(
            p1,
            chain_pose(),
            "stood pose must equal the animated driver exactly"
        );
    }

    #[test]
    fn tiny_authority_counts_truncations_and_residuals() {
        // Failure-proof twin: the same fall with a 1 um per-part cap cannot
        // cover real corrections (the default seam moves up to tens of um
        // per tick), so truncations and residual penetration events both
        // read nonzero and repeat x2. This keeps the zero in the acceptance
        // fixture honest: the counters move.
        fn script() -> CouplingStats {
            let mut r = ragdoll_on_floor();
            r.set_coupling(CouplingParams {
                substeps: 1,
                max_step_m: 0.000001,
                floor_top_y: 0.0,
                min_spacing_m: 0.10,
                enabled: true,
            })
            .expect("tiny seam");
            r.blend_to_ragdoll(1.0).expect("full ragdoll");
            r.apply_hit(1, [0.06, 0.0, 0.0], 0).expect("knock");
            step_n(&mut r, 300);
            r.coupling_stats()
        }
        let a = script();
        let b = script();
        assert_eq!(a, b, "tiny-seam counters must repeat: {a:?} vs {b:?}");
        assert!(a.truncations > 0, "tiny authority must truncate: {a:?}");
        assert!(
            a.penetration_events > 0,
            "tiny authority must leave residuals: {a:?}"
        );
        assert!(
            a.max_correction_m > 0.0,
            "tiny authority still moves bones: {a:?}"
        );
    }

    #[test]
    fn disabled_seam_measures_nothing_and_moves_nothing() {
        // Disabled, the pass is a no-op: counters stay zero and the hash
        // matches a run that never enabled the seam path.
        let mut r = ragdoll_on_floor();
        r.set_coupling(CouplingParams::disabled())
            .expect("disable seam");
        r.blend_to_ragdoll(1.0).expect("full ragdoll");
        step_n(&mut r, 120);
        assert_eq!(r.coupling_stats(), CouplingStats::default());
        assert_eq!(r.penetration_events(), 0);
    }

    #[test]
    fn ccd_fast_blade_stopped_by_thin_shield() {
        // Accept: a 40 m/s blade never passes a 5 cm shield in 1,000
        // seeded swings. The step moves 0.667 m, far past the 5 cm plate;
        // Rapier sweeps fast bodies against fixed colliders, and every bone
        // rides with CCD on, so each swing stops at the plate and the
        // contact listener sees the hit. Contacts are read during the step
        // and drained after it (Jolt listener pattern: the step never
        // mutates game state, the caller does). The start offset varies a
        // full step length per seed, so discrete phases differ across swings.
        use engine_core::{Fnv64, Rng};
        fn swing(seed: u64) -> ([f32; 3], bool) {
            let mut rng = Rng::new(0xCC_D0E).fork(seed);
            let frac = |n: u64| (n % 1000) as f32 / 1000.0 - 0.5;
            let dx = frac(rng.next_u64());
            let dy = frac(rng.next_u64());
            let dz = frac(rng.next_u64());
            let skeleton = Skeleton {
                joints: vec![Joint {
                    name: "blade".into(),
                    parent: None,
                }],
            };
            let pose = LocalPose {
                positions: vec![[-2.0 + dx * 0.667, 0.5 + dy * 0.2, dz * 0.2]],
                rotations: vec![[0.0, 0.0, 0.0, 1.0]],
            };
            let shapes = vec![BoneShape {
                radius: 0.02,
                half_height: 0.30,
            }];
            let mut r = Ragdoll::spawn(&skeleton, &pose, &shapes).expect("blade");
            r.add_static_box([0.025, 1.0, 1.0], [0.0, 0.5, 0.0])
                .expect("shield");
            r.blend_to_ragdoll(1.0).expect("free blade");
            // Fast limbs and weapons ride with CCD on.
            assert_eq!(r.ccd_enabled(0), Some(true));
            r.set_bone_linvel(0, [40.0, 0.0, 0.0]).expect("fire blade");
            let mut saw_contact = false;
            for _ in 0..12 {
                r.step(FIXED_DT).expect("fixed step");
                // Read during the step, mutate after: drain here, apply game
                // effects after the drain (the step itself changed nothing
                // but sim state plus the pending list).
                let hits = r.take_contacts();
                saw_contact |= !hits.is_empty();
                for c in &hits {
                    assert_eq!(c.a, 0, "blade must be bone 0: {c:?}");
                }
            }
            (r.bone_position(0).expect("blade pos"), saw_contact)
        }
        let mut digest_a = Fnv64::new();
        let mut digest_b = Fnv64::new();
        for seed in 0..1_000u64 {
            let (pa, ca) = swing(seed);
            let (pb, cb) = swing(seed);
            assert_eq!(pa, pb, "seed {seed} must repeat: {pa:?} vs {pb:?}");
            assert_eq!(ca, cb, "seed {seed} contacts must repeat");
            assert!(
                pa[0] < 0.0,
                "seed {seed}: blade tunneled past the shield: {pa:?}"
            );
            assert!(ca, "seed {seed}: listener must see the shield hit");
            for v in [pa[0], pa[1], pa[2]] {
                digest_a.write_i64(quantize(v));
                digest_b.write_i64(quantize(v));
            }
        }
        assert_eq!(digest_a.finish(), digest_b.finish());
    }

    #[test]
    fn ccd_bullet_blade_hits_moving_limb() {
        // Core: the per-bone CCD switch ("bullet") sweeps fast weapons
        // against *moving* limbs, not just fixed walls. Two unjoined roots
        // share one pipeline: a blade fired at 40 m/s at a slow limb rod.
        // With the bullet on, the blade dumps momentum into the limb (limb
        // shoved, blade slowed, listener sees the pair). With it off, the
        // same swings jump clean past (no contact, limb undisplaced, blade
        // still fast). Rapier never sweeps bullet-vs-bullet, so the limb
        // rides with CCD off: the blade sweeps it, not the reverse.
        use engine_core::Rng;
        fn swing(seed: u64, bullet: bool) -> ([f32; 3], [f32; 3], [f32; 3], bool, u64) {
            let mut rng = Rng::new(0x8B11E7).fork(seed);
            let frac = |n: u64| (n % 1000) as f32 / 1000.0 - 0.5;
            let dx = frac(rng.next_u64());
            let dy = frac(rng.next_u64());
            let dz = frac(rng.next_u64());
            let skeleton = Skeleton {
                joints: vec![
                    Joint {
                        name: "blade".into(),
                        parent: None,
                    },
                    Joint {
                        name: "limb".into(),
                        parent: None,
                    },
                ],
            };
            let y = 1.0 + dy * 0.2;
            let z = dz * 0.2;
            let pose = LocalPose {
                positions: vec![[-2.0 + dx * 0.667, y, z], [0.0, y, z]],
                rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
            };
            let shapes = vec![
                BoneShape {
                    radius: 0.02,
                    half_height: 0.30,
                },
                BoneShape {
                    radius: 0.05,
                    half_height: 0.40,
                },
            ];
            let mut r = Ragdoll::spawn(&skeleton, &pose, &shapes).expect("pair");
            r.blend_to_ragdoll(1.0).expect("free pair");
            assert_eq!(r.ccd_enabled(0), Some(true));
            assert_eq!(r.ccd_enabled(1), Some(true));
            // The limb is slow: CCD off, so only the blade's bullet sweep
            // can catch the pair.
            r.set_ccd(1, false).expect("slow limb off");
            if !bullet {
                r.set_ccd(0, false).expect("control without bullet");
            }
            r.set_bone_linvel(0, [40.0, 0.0, 0.0]).expect("fire blade");
            let mut saw_pair = false;
            for _ in 0..12 {
                r.step(FIXED_DT).expect("fixed step");
                let hits = r.take_contacts();
                saw_pair |= hits.iter().any(|c| c.a == 0 && c.b == Some(1));
            }
            (
                r.bone_position(0).expect("blade pos"),
                r.bone_position(1).expect("limb pos"),
                r.bone_linvel(0).expect("blade vel"),
                saw_pair,
                r.state_hash(),
            )
        }
        // Bullet on: all 1,000 swings hit the limb twice identically.
        for seed in 0..1_000u64 {
            let (ba, la, va, ca, ha) = swing(seed, true);
            let (bb, lb, vb, cb, hb) = swing(seed, true);
            assert_eq!((ba, la, va), (bb, lb, vb), "seed {seed} must repeat");
            assert_eq!((ca, ha), (cb, hb), "seed {seed} contacts+hash repeat");
            assert!(ca, "seed {seed}: listener must see the blade-limb pair");
            assert!(la[0] > 0.1, "seed {seed}: limb must be shoved +X: {la:?}");
            assert!(va[0] < 30.0, "seed {seed}: blade must slow down: {va:?}");
            let _ = (ba, bb);
        }
        // Control without the bullet: the same swings mostly tunnel.
        let mut tunneled = 0u32;
        for seed in 0..1_000u64 {
            let (_, limb, vel, saw, _) = swing(seed, false);
            if !saw && limb[0].abs() < 0.02 && vel[0] > 35.0 {
                tunneled += 1;
            }
        }
        assert!(
            tunneled > 500,
            "control without bullet must mostly tunnel, got {tunneled}/1000"
        );
    }

    #[test]
    fn ccd_toggle_and_contact_drain_reject_bad_bones() {
        // Failure case: CCD and velocity setters name out-of-range bones and
        // change nothing; draining twice yields the same events once.
        let mut r = ragdoll_on_floor();
        assert_eq!(r.ccd_enabled(0), Some(true));
        assert_eq!(r.ccd_enabled(99), None);
        assert_eq!(r.set_ccd(99, false), Err(Error::BadBone(99)));
        assert_eq!(
            r.set_bone_linvel(99, [1.0, 0.0, 0.0]),
            Err(Error::BadBone(99))
        );
        assert_eq!(
            r.set_bone_linvel(0, [f32::NAN, 0.0, 0.0]),
            Err(Error::NonFiniteImpulse)
        );
        r.set_ccd(1, false).expect("slow bone off");
        assert_eq!(r.ccd_enabled(1), Some(false));
        r.set_ccd(1, true).expect("fast limb back on");
        assert_eq!(r.take_contacts(), vec![]);
    }

    #[test]
    fn contact_drain_feeds_hits_after_step() {
        // The wired consumer: after a blade-vs-shield step the drain turns
        // every contact into one applied hit; a bad impulse drains nothing.
        // The contact/joint follow-up pass calls this after the step
        // instead of reading `take_contacts` raw.
        use engine_core::Rng;
        fn script(seed: u64) -> (usize, u64) {
            let mut rng = Rng::new(0xD2A1).fork(seed);
            let dy = (rng.next_u64() % 1000) as f32 / 1000.0 - 0.5;
            let skeleton = Skeleton {
                joints: vec![Joint {
                    name: "blade".into(),
                    parent: None,
                }],
            };
            let pose = LocalPose {
                positions: vec![[-2.0, 0.5 + dy * 0.2, 0.0]],
                rotations: vec![[0.0, 0.0, 0.0, 1.0]],
            };
            let shapes = vec![BoneShape {
                radius: 0.02,
                half_height: 0.30,
            }];
            let mut r = Ragdoll::spawn(&skeleton, &pose, &shapes).expect("blade");
            r.add_static_box([0.025, 1.0, 1.0], [0.0, 0.5, 0.0])
                .expect("shield");
            r.blend_to_ragdoll(1.0).expect("free blade");
            r.set_bone_linvel(0, [40.0, 0.0, 0.0]).expect("fire blade");
            for _ in 0..12 {
                r.step(FIXED_DT).expect("fixed step");
                if !r.contacts().is_empty() {
                    break;
                }
            }
            assert!(!r.contacts().is_empty(), "shield hit must pend");
            assert_eq!(
                r.drain_contacts_into_hits([f32::NAN, 0.0, 0.0], 12),
                Err(Error::NonFiniteImpulse)
            );
            assert!(!r.contacts().is_empty(), "bad impulse drains nothing");
            let applied = r
                .drain_contacts_into_hits([0.05, 0.0, 0.0], 12)
                .expect("drain into hits");
            assert!(applied >= 1, "one shield contact strikes once");
            assert!(r.contacts().is_empty(), "drain empties the pending list");
            (applied, r.state_hash())
        }
        let (a, ha) = script(7);
        let (b, hb) = script(7);
        assert_eq!((a, ha), (b, hb), "drain script must repeat");
    }

    /// 17-bone humanoid: pelvis, spine, chest, neck, head, two arms of three,
    /// two legs of three. Names carry `knee`/`elbow` so the table can hinge them.
    fn humanoid_skeleton() -> Skeleton {
        let rows: [(Option<usize>, &str); 17] = [
            (None, "pelvis"),
            (Some(0), "spine"),
            (Some(1), "chest"),
            (Some(2), "neck"),
            (Some(3), "head"),
            (Some(2), "shoulder_l"),
            (Some(5), "elbow_l"),
            (Some(6), "wrist_l"),
            (Some(2), "shoulder_r"),
            (Some(8), "elbow_r"),
            (Some(9), "wrist_r"),
            (Some(0), "hip_l"),
            (Some(11), "knee_l"),
            (Some(12), "ankle_l"),
            (Some(0), "hip_r"),
            (Some(14), "knee_r"),
            (Some(15), "ankle_r"),
        ];
        Skeleton {
            joints: rows
                .iter()
                .map(|(parent, name)| Joint {
                    name: (*name).into(),
                    parent: *parent,
                })
                .collect(),
        }
    }

    /// Local offsets for the humanoid (parent frame). Root entry is replaced
    /// by the seeded world position at spawn.
    fn humanoid_offsets() -> Vec<[f32; 3]> {
        vec![
            [0.0, 1.0, 0.0],
            [0.0, 0.2, 0.0],
            [0.0, 0.25, 0.0],
            [0.0, 0.25, 0.0],
            [0.0, 0.12, 0.0],
            [-0.2, 0.2, 0.0],
            [0.0, -0.3, 0.0],
            [0.0, -0.28, 0.0],
            [0.2, 0.2, 0.0],
            [0.0, -0.3, 0.0],
            [0.0, -0.28, 0.0],
            [-0.1, -0.05, 0.0],
            [0.0, -0.45, 0.0],
            [0.0, -0.42, 0.0],
            [0.1, -0.05, 0.0],
            [0.0, -0.45, 0.0],
            [0.0, -0.42, 0.0],
        ]
    }

    fn humanoid_pose(root: [f32; 3]) -> LocalPose {
        let mut positions = humanoid_offsets();
        positions[0] = root;
        LocalPose {
            positions,
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 17],
        }
    }

    fn humanoid_shapes() -> Vec<BoneShape> {
        vec![
            BoneShape {
                radius: 0.06,
                half_height: 0.1,
            };
            17
        ]
    }

    /// Deterministic unit float in `[0, 1)` from a stream word (tape idiom).
    fn unit01(word: u64) -> f32 {
        let top = word >> 40;
        let hi = u16::try_from(top >> 16).unwrap_or(u16::MAX);
        let lo = u16::try_from(top & 0xFFFF).unwrap_or(u16::MAX);
        (f32::from(hi) * 65_536.0 + f32::from(lo)) / 16_777_216.0
    }

    #[test]
    fn joint_table_humanoid_defaults() {
        let skeleton = humanoid_skeleton();
        let table = JointTable::humanoid(&skeleton);
        assert_eq!(table.len(), 17);
        assert!(!table.is_empty());
        // Hinge knees and elbows (4), cone plus twist elsewhere (13).
        let mut hinges = 0usize;
        for (i, joint) in skeleton.joints.iter().enumerate() {
            let spec = table.spec(i).expect("row per bone");
            let depth = joint_depths(&skeleton)[i];
            assert!(
                (spec.swing_limit - swing_limit(depth)).abs() < 1e-6,
                "bone {i} swing must reuse depth limit"
            );
            if joint.name.contains("knee") || joint.name.contains("elbow") {
                assert_eq!(spec.kind, JointKind::Hinge, "bone {i} must hinge");
                assert_eq!((spec.twist_min, spec.twist_max), (0.0, 0.0));
                assert_eq!(spec.friction_torque, JOINT_FRICTION_HINGE_NM);
                assert_eq!(spec.spring_hz, JOINT_SPRING_HINGE_HZ);
                hinges += 1;
            } else {
                assert_eq!(spec.kind, JointKind::Spherical, "bone {i} spherical");
                assert_eq!(
                    (spec.twist_min, spec.twist_max),
                    (-JOINT_TWIST_SPHERICAL_RAD, JOINT_TWIST_SPHERICAL_RAD)
                );
                assert_eq!(spec.friction_torque, JOINT_FRICTION_SPHERICAL_NM);
                assert_eq!(spec.spring_hz, JOINT_SPRING_SPHERICAL_HZ);
            }
            assert!(
                (5.0..=10.0).contains(&spec.friction_torque),
                "friction 5..10 N.m: {spec:?}"
            );
            assert!(
                (0.5..=2.0).contains(&spec.spring_hz),
                "spring 0.5..2 Hz: {spec:?}"
            );
        }
        assert_eq!(hinges, 4, "knees plus elbows hinge");
        assert_eq!(table.spec(99), None, "past-end bone is none");
        // Failure cases: non-finite hinge maps to 0, twist clamps both ends.
        let hinge = table.spec(6).expect("elbow_l");
        assert_eq!(hinge.clamp_hinge(f32::NAN), 0.0);
        assert_eq!(hinge.clamp_hinge(-0.5), 0.0);
        assert_eq!(hinge.clamp_hinge(99.0), hinge.swing_limit);
        let ball = table.spec(2).expect("chest");
        assert_eq!(ball.clamp_twist(-99.0), ball.twist_min);
        assert_eq!(ball.clamp_twist(99.0), ball.twist_max);
        assert_eq!(ball.clamp_twist(f32::INFINITY), ball.twist_min);
        // Same skeleton twice gives the same table.
        assert_eq!(table, JointTable::humanoid(&humanoid_skeleton()));
        println!("JOINT_TABLE hinges={hinges} rows=17 friction=5..10Nm spring=0.5..2Hz");
    }

    #[test]
    fn stabilize_clamps_ratio_preserving_total() {
        // Four-bone chain with wild masses: 0.2 vs 5.0 forces both clamps.
        let skeleton = chain_skeleton();
        let before = vec![0.2f32, 5.0, 0.2, 5.0];
        let total_before: f32 = before.iter().sum();
        let mut masses = before.clone();
        stabilize_masses(&mut masses, &skeleton);
        let total_after: f32 = masses.iter().sum();
        let rel = (total_after - total_before).abs() / total_before;
        assert!(
            rel < 1e-4,
            "total preserved: {total_before} vs {total_after}"
        );
        for i in 1..masses.len() {
            let parent = skeleton.joints[i].parent.expect("chain parent");
            let ratio = masses[i] / masses[parent];
            assert!(
                (MASS_RATIO_LO - 1e-4..=MASS_RATIO_HI + 1e-4).contains(&ratio),
                "link {parent}->{i} inside 0.8..1.2: {ratio}"
            );
        }
        // The pass moved something (not a no-op on wild input).
        let moved: f32 = masses
            .iter()
            .zip(before.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();
        assert!(moved > 0.1, "wild masses must move: {moved}");
        // Failure cases: length mismatch and empty do nothing, never panic.
        let mut short = vec![1.0f32, 2.0];
        stabilize_masses(&mut short, &skeleton);
        assert_eq!(short, vec![1.0, 2.0]);
        let mut empty: Vec<f32> = vec![];
        stabilize_masses(&mut empty, &skeleton);
        assert!(empty.is_empty());
        // Non-finite sanitizes to 1.0 then stabilizes inside the ratio.
        let mut bad = vec![1.0f32, f32::NAN, 1.0, f32::INFINITY];
        stabilize_masses(&mut bad, &skeleton);
        assert!(bad.iter().all(|m| m.is_finite() && *m > 0.0));
        // Same input twice gives the same output.
        let mut again = before.clone();
        stabilize_masses(&mut again, &skeleton);
        assert_eq!(masses, again);
        println!("STABILIZE total_before={total_before:.4} total_after={total_after:.4} moved={moved:.4}");
    }

    /// One seeded human fall: humanoid ragdoll steps plus synthetic hinge and
    /// mass measurements beside spawn (table and stabilize never reach spawn).
    fn measure_one_fall(seed: u64, fall: u64) -> (u64, u64, u64, f32, f32) {
        let mut rng = engine_core::Rng::new(seed).fork(fall);
        let skeleton = humanoid_skeleton();
        let table = JointTable::humanoid(&skeleton);
        // Seeded masses in 0.2..5.0 force the clamp, then stabilize.
        let mut masses: Vec<f32> = (0..skeleton.len())
            .map(|_| 0.2 + unit01(rng.next_u64()) * 4.8)
            .collect();
        let total_before: f32 = masses.iter().sum();
        stabilize_masses(&mut masses, &skeleton);
        let total_after: f32 = masses.iter().sum();
        let total_err = (total_after - total_before).abs() / total_before;
        let mut worst_ratio = 0.0f32;
        for i in 1..masses.len() {
            let parent = skeleton.joints[i].parent.unwrap_or(0);
            let ratio = masses[i] / masses[parent];
            let outside = if ratio < MASS_RATIO_LO {
                MASS_RATIO_LO - ratio
            } else if ratio > MASS_RATIO_HI {
                ratio - MASS_RATIO_HI
            } else {
                0.0
            };
            if outside > worst_ratio {
                worst_ratio = outside;
            }
        }
        // Seeded hinge frames: 30 frames of 4 hinges in -1.5..1.5 rad.
        let mut backward_before = 0u64;
        let mut backward_after = 0u64;
        for _ in 0..30 {
            for h in [6usize, 9, 12, 15] {
                let raw = unit01(rng.next_u64()) * 3.0 - 1.5;
                if raw < 0.0 {
                    backward_before += 1;
                }
                let clamped = table.spec(h).expect("hinge").clamp_hinge(raw);
                if clamped < 0.0 {
                    backward_after += 1;
                }
                assert!(clamped >= 0.0);
            }
        }
        // Real humanoid fall: seeded root offset, seeded shove, 30 fixed steps.
        let dx = unit01(rng.next_u64()) - 0.5;
        let dz = unit01(rng.next_u64()) - 0.5;
        let mut ragdoll =
            Ragdoll::spawn(&skeleton, &humanoid_pose([dx, 1.5, dz]), &humanoid_shapes())
                .expect("humanoid spawns");
        ragdoll
            .add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        ragdoll.blend_to_ragdoll(1.0).expect("limp");
        let shove = unit01(rng.next_u64()) * 0.12 - 0.06;
        for t in 0..30u32 {
            if t == 10 {
                let tick = u64::from(u16::try_from(t).unwrap_or(u16::MAX));
                ragdoll
                    .apply_hit(1, [shove, 0.0, 0.0], tick)
                    .expect("shove");
            }
            ragdoll.step(FIXED_DT).expect("fixed step");
        }
        (
            ragdoll.state_hash(),
            backward_before,
            backward_after,
            worst_ratio,
            total_err,
        )
    }

    /// Fifty seeded human falls: digest plus folded maxima. Pure measurement
    /// beside spawn (spawn/step untouched, hash untouched).
    fn measure_fifty(seed: u64) -> (u64, u64, u64, f32, f32) {
        let mut hashes = Vec::with_capacity(50);
        let mut before = 0u64;
        let mut after = 0u64;
        let mut worst_ratio = 0.0f32;
        let mut worst_total = 0.0f32;
        for fall in 0..50u64 {
            let (h, b0, b1, wr, te) = measure_one_fall(seed, fall);
            hashes.push(h);
            before += b0;
            after += b1;
            if wr > worst_ratio {
                worst_ratio = wr;
            }
            if te > worst_total {
                worst_total = te;
            }
        }
        (
            engine_core::Rng::digest(&hashes),
            before,
            after,
            worst_ratio,
            worst_total,
        )
    }

    #[test]
    fn fifty_seeded_human_falls_measured() {
        let (digest_a, before_a, after_a, ratio_a, total_a) = measure_fifty(2040);
        let (digest_b, before_b, after_b, ratio_b, total_b) = measure_fifty(2040);
        assert_eq!(digest_a, digest_b, "fifty falls must repeat");
        assert_eq!((before_a, after_a), (before_b, after_b));
        assert_eq!((ratio_a, total_a), (ratio_b, total_b));
        assert!(before_a > 0, "raw hinge frames must bite: {before_a}");
        assert_eq!(after_a, 0, "zero backward-hinge frames after clamp");
        assert!(ratio_a <= 1e-4, "every link inside ratio: {ratio_a}");
        assert!(total_a < 1e-4, "total preserved: {total_a}");
        println!(
            "HUMAN_FALLS falls=50 digest={digest_a:016x} backward_before={before_a} backward_after={after_a} worst_ratio_outside={ratio_a:.6} worst_total_rel={total_a:.6}"
        );
    }
}
