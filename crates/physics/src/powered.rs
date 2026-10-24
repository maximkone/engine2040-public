//! powered: per-bone powered ragdoll with partial hit masks and get-up.
//!
//! Consumer: arena hit reactions and deaths (maps combat events to hits and
//! masks; this crate never imports arena or combat). Owner: this module owns
//! the per-bone bodies, drive params, mask and get-up state. Units: meters,
//! seconds, ticks; rotations are unit quaternions `[x, y, z, w]`. Errors are
//! typed [`RagdollError`]; bad caller input never panics.
//!
//! Determinism (HASH-LAW tier note): one fixed `dt` per step, bodies hashed in
//! skeleton order, rapier3d `enhanced-determinism` without SIMD/parallel. The
//! glue uses only comparisons and `+ - * /` plus `sqrt` (quat normalize) and
//! `round` (hash): no transcendentals. Drive targets reuse preallocated
//! buffers, so `step` performs no steady-state allocation.
#![forbid(unsafe_code)]

use super::pd::stable_gains;
use super::ragdoll::{
    drain_narrow_contacts, insert_joints, joint_depths, normalize_quat, normalized_rotation,
    quat_conj, read_sim_transforms, rotate_vec, BoneShape,
};
use super::rapier::{
    ordered_linvel, ordered_translation, position_in_bounds, quantize, sanitize3, BodyState,
    ParamsError, RagdollParams, FIXED_DT, GRAVITY_Y, MAX_LEN,
};
use super::servo::{self, Rig, Servo, ServoTuning};
use engine_anim::contract::{validate_pose, validate_skeleton, AnimError, LocalPose, Skeleton};
use engine_core::{quat_mul, Fnv64};
use rapier3d::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet,
    RigidBodyBuilder, RigidBodyHandle, RigidBodySet, SoftBodySet,
};
use rapier3d::geometry::{BroadPhaseBvh, ColliderBuilder, ColliderSet, NarrowPhase};
use rapier3d::math::{Rotation, Vector};
use rapier3d::pipeline::PhysicsPipeline;
use std::fmt::{Display, Formatter};

/// Get-up budget: a started get-up blends back to animation within this many
/// fixed steps (1 s at 60 Hz). [`PoweredRagdoll::start_getup`] plus
/// [`GETUP_BUDGET_TICKS`] steps always lands back in [`RagdollMode::Animated`].
pub const GETUP_BUDGET_TICKS: u64 = 60;
/// Drive gain upper bound, 1/s.
const MAX_DRIVE_GAIN: f32 = 1_000.0;
/// Drive speed-cap upper bound, m/s and rad/s.
const MAX_DRIVE_CAP: f32 = 1_000.0;
/// Small linear damping so free bones settle to rest on the floor.
const LINEAR_DAMPING: f32 = 0.05;
/// Small angular damping so chained bones stop oscillating.
const ANGULAR_DAMPING: f32 = 0.05;
/// Contact friction for bone colliders.
const FRICTION: f32 = 0.9;

/// Per-bone PD velocity-servo drive params: a driven bone's linear velocity
/// is `clamp_magnitude(P * offset - D * linvel, max_speed)` toward its
/// animated world target each step, and likewise its angular velocity from
/// the quaternion error vector with `angular_gain`/`angular_damping` capped
/// by `max_spin`. The D term damps on measured bone velocity (for a fixed
/// driver target the pose-error derivative equals the negated velocity, so
/// no error history is stored); a degenerate step `dt` answers the P term
/// only (see [`super::pd`]). A zero P gain leaves that axis untouched, so
/// all-zero P gains are a limp bone that still counts as driven for masking
/// purposes. P and D gains are 1/s, caps are m/s and rad/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveParams {
    /// Position-to-velocity P gain, finite in `0..=MAX_DRIVE_GAIN`.
    pub linear_gain: f32,
    /// Orientation-to-spin P gain, finite in `0..=MAX_DRIVE_GAIN`.
    pub angular_gain: f32,
    /// Linear D gain on measured velocity, finite in `0..=MAX_DRIVE_GAIN`.
    pub linear_damping: f32,
    /// Angular D gain on measured spin, finite in `0..=MAX_DRIVE_GAIN`.
    pub angular_damping: f32,
    /// Linear speed cap (vector magnitude), finite in `(0, MAX_DRIVE_CAP]`.
    pub max_speed: f32,
    /// Angular speed cap (vector magnitude), finite in `(0, MAX_DRIVE_CAP]`.
    pub max_spin: f32,
}

impl DriveParams {
    /// Default drive: holds a human-scale chain aloft against gravity with
    /// centimetre-scale static error and recovers a fallen chain inside the
    /// get-up budget (see the tracking and get-up tests). The D gains settle
    /// a knocked chain with fewer ticks than P-only at the same overshoot
    /// (see the `pd_drive` legs).
    pub fn default_drive() -> Self {
        Self {
            linear_gain: 12.0,
            angular_gain: 10.0,
            linear_damping: 1.0,
            angular_damping: 1.0,
            max_speed: 8.0,
            max_spin: 12.0,
        }
    }
}

/// Get-up state machine. `Animated` means every bone is driven and the output
/// pose equals the animated input exactly; `Partial`/`Full` mean some/all
/// bones simulate freely; `GettingUp` ramps every bone back over
/// [`GETUP_BUDGET_TICKS`] steps, then lands in `Animated`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RagdollMode {
    /// Fully driven; output equals the animated pose exactly.
    Animated,
    /// Some bones free (mask has both values).
    Partial,
    /// Every bone free (mask all set).
    Full,
    /// Blending back to animation; drives re-engaged on every bone.
    GettingUp,
}

/// Typed errors for the powered ragdoll. User input never panics.
#[derive(Clone, Debug, PartialEq)]
pub enum RagdollError {
    /// Skeleton or pose failed the animation contract; the [`AnimError`] names it.
    Anim(AnimError),
    /// Mask length differs from the skeleton; expected + got carried.
    BadMask {
        /// Expected bone count.
        expected: usize,
        /// Mask length.
        got: usize,
    },
    /// Bone index is out of range for this skeleton.
    BadBone(usize),
    /// Drive params invalid; bone index carried.
    BadDrive(usize),
    /// Hit impulse must have finite components.
    NonFiniteImpulse,
    /// `step` needs a finite `dt` greater than zero.
    InvalidDt,
    /// `start_getup` needs a ragdolled body; already fully animated.
    NotRagdolled,
    /// A position component was non-finite or out of range.
    NonFinitePosition,
    /// Capsule radius must be finite and in `(0, MAX_LEN]`.
    InvalidRadius,
    /// Capsule half-height must be finite and in `[0, MAX_LEN]`.
    InvalidHeight,
    /// Static-box half-extents must be finite and each in `(0, MAX_LEN]`.
    InvalidHalfExtents,
    /// Snapshot does not match this ragdoll (counts, mask, mode or pose).
    SnapshotMismatch,
    /// The ragdoll params builder refused the table.
    Params(ParamsError),
}

impl Display for RagdollError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RagdollError::Anim(e) => write!(f, "invalid skeleton/pose: {e:?}"),
            RagdollError::BadMask { expected, got } => {
                write!(f, "bad mask: need {expected} entries, got {got}")
            }
            RagdollError::BadBone(b) => write!(f, "bad bone: index {b} out of range"),
            RagdollError::BadDrive(b) => write!(f, "bad drive params on bone {b}"),
            RagdollError::NonFiniteImpulse => {
                f.write_str("invalid impulse: need finite components")
            }
            RagdollError::InvalidDt => f.write_str("invalid dt: need finite dt > 0"),
            RagdollError::NotRagdolled => f.write_str("not ragdolled: get-up needs a downed body"),
            RagdollError::NonFinitePosition => {
                f.write_str("invalid position: need finite values within +-1000")
            }
            RagdollError::InvalidRadius => {
                f.write_str("invalid radius: need finite radius in (0, 1000]")
            }
            RagdollError::InvalidHeight => {
                f.write_str("invalid half-height: need finite value in [0, 1000]")
            }
            RagdollError::InvalidHalfExtents => {
                f.write_str("invalid half-extents: need finite values in (0, 1000]")
            }
            RagdollError::SnapshotMismatch => f.write_str("snapshot does not match this ragdoll"),
            RagdollError::Params(e) => write!(f, "ragdoll params refused: {e}"),
        }
    }
}

impl std::error::Error for RagdollError {}

impl From<AnimError> for RagdollError {
    fn from(e: AnimError) -> Self {
        RagdollError::Anim(e)
    }
}

impl From<ParamsError> for RagdollError {
    fn from(e: ParamsError) -> Self {
        RagdollError::Params(e)
    }
}

/// Owned snapshot: per-bone kinematic state plus every field that affects
/// future output (mask, mode, get-up countdown, counters, driver pose).
#[derive(Clone, Debug, PartialEq)]
pub struct PoweredSnapshot {
    /// One entry per bone, skeleton order.
    pub bodies: Vec<BodyState>,
    /// Animated driver pose at snapshot time.
    pub animated: LocalPose,
    /// Ragdoll mask at snapshot time (`true` = free bone).
    pub mask: Vec<bool>,
    /// Mode at snapshot time.
    pub mode: RagdollMode,
    /// Get-up steps left (meaningful only for `GettingUp`).
    pub getup_remaining: u64,
    /// Fixed steps taken since spawn.
    pub tick: u64,
    /// Applied-hit counter.
    pub hits: u64,
    /// Per-bone CCD switches at snapshot time.
    pub ccd_flags: Vec<bool>,
    /// Total contact pairs observed at snapshot time.
    pub contact_events: u64,
    /// PID state of the linear servo, one per bone (see [`Servo`]).
    pub servo_lin: Vec<Servo>,
    /// PID state of the angular servo, one per bone.
    pub servo_ang: Vec<Servo>,
    /// Rig whose tuning row was active at snapshot time.
    pub rig: Rig,
}

/// One observed contact pair from the last [`PoweredRagdoll::step`]: the
/// Jolt `ContactListener` pattern reimplemented on Rapier's narrow phase.
/// The step only reads contacts into a pending list; the caller drains it
/// after the step and mutates game state there (hits, breaks, sound),
/// never inside the step. `b` is `None` against static geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoweredContact {
    /// First bone index in skeleton order (the smaller one for bone pairs).
    pub a: usize,
    /// Second bone index, or `None` against static geometry.
    pub b: Option<usize>,
}

impl PoweredContact {
    /// Order-preserving constructor for the shared narrow-phase drain.
    fn ordered(a: usize, b: Option<usize>) -> Self {
        Self { a, b }
    }

    /// Skeleton-order sort key for the shared narrow-phase drain.
    fn sort_key(&self) -> (usize, Option<usize>) {
        (self.a, self.b)
    }
}

/// Per-bone powered ragdoll: driven bones track the animated pose with a
/// velocity servo, masked bones simulate freely, and a get-up blends back to
/// within [`GETUP_BUDGET_TICKS`] steps.
pub struct PoweredRagdoll {
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
    /// Per-bone drive params, skeleton order.
    drives: Vec<DriveParams>,
    /// Partial-body mask, skeleton order: `true` = free (ragdolled) bone.
    mask: Vec<bool>,
    /// Get-up state machine.
    mode: RagdollMode,
    /// Get-up steps left (meaningful only for `GettingUp`).
    getup_remaining: u64,
    /// Fixed steps taken since spawn (folded into the hash).
    tick: u64,
    /// Applied-hit counter (folded into the hash).
    hits: u64,
    /// Scratch drive targets, preallocated at spawn so `step` never allocates.
    target_pos: Vec<[f32; 3]>,
    /// Scratch drive targets, preallocated at spawn so `step` never allocates.
    target_rot: Vec<[f32; 4]>,
    /// Per-bone continuous collision: `true` rides the CCD solver so fast
    /// weapons and limbs stop at thin shields instead of tunneling. On at
    /// spawn for every bone; folded into the hash, carried by snapshots.
    ccd_flags: Vec<bool>,
    /// Contacts read during the last step, skeleton order, drained by the
    /// caller after the step (never mutated inside it). Observation only:
    /// cleared on snapshot restore, not hashed (the counter below is).
    pending: Vec<PoweredContact>,
    /// Total contact pairs observed since spawn (folded into the hash,
    /// carried by snapshots).
    contact_events: u64,
    /// PID state of the linear servo, one per bone, skeleton order. Carried by
    /// snapshots so a restored run resumes bit for bit; never hashed.
    servo_lin: Vec<Servo>,
    /// PID state of the angular servo, one per bone, skeleton order.
    servo_ang: Vec<Servo>,
    /// The rig whose tuning row this body runs (see [`ServoTuning`]).
    rig: Rig,
}

impl PoweredRagdoll {
    /// Spawn one dynamic body per skeleton joint at the animated world
    /// transform, joined parent-to-child with depth-limited spherical joints.
    /// `capsule_dims` and `drives` carry one entry per joint. Every bone starts
    /// driven, so the mode is [`RagdollMode::Animated`] and [`bone_poses`](
    /// PoweredRagdoll::bone_poses) returns `pose` exactly until a hit or mask
    /// frees bones.
    pub fn spawn(
        skeleton: &Skeleton,
        pose: &LocalPose,
        capsule_dims: &[BoneShape],
        drives: &[DriveParams],
    ) -> Result<Self, RagdollError> {
        validate_spawn_args(skeleton, pose, capsule_dims, drives)?;
        let radii: Vec<f32> = capsule_dims.iter().map(|d| d.radius).collect();
        let half_heights: Vec<f32> = capsule_dims.iter().map(|d| d.half_height).collect();
        let params = RagdollParams::for_rig(skeleton, &radii, &half_heights)?;
        let (world_pos, world_rot) = world_transforms(skeleton, pose);
        let depths = joint_depths(skeleton);
        let integration = params.integration(FIXED_DT);
        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();
        let mut impulse_joints = ImpulseJointSet::new();
        let mut order = Vec::with_capacity(skeleton.len());
        let ccd_flags = params.ccd_flags();
        for (i, (p, r)) in world_pos.iter().zip(world_rot.iter()).enumerate() {
            let body = RigidBodyBuilder::dynamic()
                .translation(Vector::new(p[0], p[1], p[2]))
                .linear_damping(LINEAR_DAMPING)
                .angular_damping(ANGULAR_DAMPING)
                // Sleeping is disabled so kinematic state (pos/rot/vel) is
                // the complete future-affecting state and restore is exact.
                .can_sleep(false)
                // The one CCD policy: a dynamic bone rides the solver so fast
                // limbs stop at thin shields (see `set_ccd`).
                .ccd_enabled(ccd_flags.get(i).copied().unwrap_or(false))
                .build();
            let handle = bodies.insert(body);
            // Orientation is set directly as a quaternion: the builder takes
            // axis-angle (which needs `acos` to derive), so this avoids trig.
            if let Some(b) = bodies.get_mut(handle) {
                b.set_rotation(normalized_rotation(r), false);
            }
            // The density carries the bone's share of the body mass.
            let density = params
                .density_for(i, capsule_dims[i].radius, capsule_dims[i].half_height)
                .unwrap_or(1.0);
            let collider =
                ColliderBuilder::capsule_y(capsule_dims[i].half_height, capsule_dims[i].radius)
                    .density(density)
                    .friction(FRICTION)
                    .restitution(0.0)
                    .build();
            colliders.insert_with_parent(collider, handle, &mut bodies);
            order.push(handle);
        }
        insert_joints(
            skeleton,
            &world_pos,
            &world_rot,
            &depths,
            &order,
            &mut impulse_joints,
        );
        let n = skeleton.len();
        Ok(Self {
            pipeline: PhysicsPipeline::new(),
            gravity: Vector::new(0.0, GRAVITY_Y, 0.0),
            params: integration,
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
            drives: drives.to_vec(),
            mask: vec![false; n],
            mode: RagdollMode::Animated,
            getup_remaining: 0,
            tick: 0,
            hits: 0,
            target_pos: vec![[0.0; 3]; n],
            target_rot: vec![[0.0, 0.0, 0.0, 1.0]; n],
            ccd_flags,
            pending: Vec::with_capacity(32),
            contact_events: 0,
            servo_lin: vec![Servo::new(); n],
            servo_ang: vec![Servo::new(); n],
            rig: Rig::default(),
        })
    }

    /// The rig whose PID row drives this body.
    pub fn rig(&self) -> Rig {
        self.rig
    }

    /// Switch the rig row: the servo history is dropped, so the next step
    /// starts from the pose with a clean integral and slew state.
    pub fn set_rig(&mut self, rig: Rig) {
        if self.rig == rig {
            return;
        }
        self.rig = rig;
        for s in self.servo_lin.iter_mut().chain(self.servo_ang.iter_mut()) {
            s.reset();
        }
    }

    /// Feed the animated driver pose for this tick. Driven bones track it via
    /// their servo params; validation matches spawn.
    pub fn set_animated(&mut self, pose: &LocalPose) -> Result<(), RagdollError> {
        validate_pose(&self.skeleton, pose)?;
        self.animated = pose.clone();
        Ok(())
    }

    /// Drive params for one bone.
    pub fn drive(&self, bone: usize) -> Option<DriveParams> {
        self.drives.get(bone).copied()
    }

    /// Retune one bone's drive (arena difficulty or per-limb weight). Invalid
    /// params or an out-of-range bone change nothing and report the cause.
    pub fn set_drive(&mut self, bone: usize, drive: DriveParams) -> Result<(), RagdollError> {
        if bone >= self.drives.len() {
            return Err(RagdollError::BadBone(bone));
        }
        check_drive(&drive).map_err(|()| RagdollError::BadDrive(bone))?;
        self.drives[bone] = drive;
        Ok(())
    }

    /// Current partial-body mask, skeleton order (`true` = free bone).
    pub fn mask(&self) -> &[bool] {
        &self.mask
    }

    /// Install a partial-body mask: `true` frees a bone, `false` drives it.
    /// All-false animates, all-true is a full ragdoll. A manual mask overrides
    /// an in-flight get-up (the body stays where the caller puts it).
    pub fn set_mask(&mut self, mask: &[bool]) -> Result<(), RagdollError> {
        if mask.len() != self.mask.len() {
            return Err(RagdollError::BadMask {
                expected: self.mask.len(),
                got: mask.len(),
            });
        }
        self.mask.copy_from_slice(mask);
        self.mode = derive_mode(&self.mask);
        self.getup_remaining = 0;
        Ok(())
    }

    /// Current get-up state.
    pub fn mode(&self) -> RagdollMode {
        self.mode
    }

    /// Get-up steps left (zero unless [`RagdollMode::GettingUp`]).
    pub fn getup_remaining(&self) -> u64 {
        self.getup_remaining
    }

    /// Fixed steps taken since spawn.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Applied-hit counter.
    pub fn hit_count(&self) -> u64 {
        self.hits
    }

    /// Per-bone CCD switch (`None` past the end of the skeleton). Every
    /// bone spawns with CCD on; the caller switches slow bones off to save
    /// solver work and fast weapons back on before a swing.
    pub fn ccd_enabled(&self, bone: usize) -> Option<bool> {
        self.ccd_flags.get(bone).copied()
    }

    /// Flip one bone's CCD switch. An out-of-range bone changes nothing and
    /// reports [`RagdollError::BadBone`].
    pub fn set_ccd(&mut self, bone: usize, enabled: bool) -> Result<(), RagdollError> {
        let flag = self
            .ccd_flags
            .get_mut(bone)
            .ok_or(RagdollError::BadBone(bone))?;
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
    pub fn set_bone_linvel(&mut self, bone: usize, vel: [f32; 3]) -> Result<(), RagdollError> {
        if vel.iter().any(|c| !c.is_finite()) {
            return Err(RagdollError::NonFiniteImpulse);
        }
        let handle = *self.order.get(bone).ok_or(RagdollError::BadBone(bone))?;
        let Some(body) = self.bodies.get_mut(handle) else {
            return Err(RagdollError::BadBone(bone));
        };
        body.set_linvel(Vector::new(vel[0], vel[1], vel[2]), true);
        Ok(())
    }

    /// Contacts read during the last step, skeleton order. Read-only: the
    /// caller drains them with [`PoweredRagdoll::take_contacts`] after the
    /// step and applies game effects there.
    pub fn contacts(&self) -> &[PoweredContact] {
        &self.pending
    }

    /// Drain the last step's contacts (Jolt listener pattern): the step
    /// reads, the caller takes the list and mutates game state after it.
    /// Deterministic skeleton order; empty when nothing touched.
    pub fn take_contacts(&mut self) -> Vec<PoweredContact> {
        core::mem::take(&mut self.pending)
    }

    /// Drain the last step's contacts into applied hits: the wired consumer
    /// of the listener drain (the contact/joint follow-up pass calls this
    /// after the step instead of reading [`PoweredRagdoll::take_contacts`]
    /// raw). Each drained contact strikes bone `a` with the caller-supplied
    /// `impulse` at `tick`; pair contacts strike the first bone only —
    /// splitting pairs across both bones is the follow-up joint-table work.
    /// The caller gates strike windows (weapon bones, not resting feet):
    /// this call strikes every drained contact. A non-finite impulse drains
    /// nothing and reports [`RagdollError::NonFiniteImpulse`]. Returns
    /// contacts applied.
    pub fn drain_contacts_into_hits(
        &mut self,
        impulse: [f32; 3],
        tick: u64,
    ) -> Result<usize, RagdollError> {
        if impulse.iter().any(|c| !c.is_finite()) {
            return Err(RagdollError::NonFiniteImpulse);
        }
        let contacts = self.take_contacts();
        let mut applied = 0;
        for contact in &contacts {
            self.apply_hit(contact.a, impulse, tick)?;
            applied += 1;
        }
        Ok(applied)
    }

    /// Total contact pairs observed since spawn.
    pub fn contact_events(&self) -> u64 {
        self.contact_events
    }

    /// Global blend weight behind [`bone_poses`](PoweredRagdoll::bone_poses):
    /// `0` animated, `1` ragdolled, ramping down through a get-up.
    pub fn blend_weight(&self) -> f32 {
        match self.mode {
            RagdollMode::Animated => 0.0,
            RagdollMode::Partial | RagdollMode::Full => 1.0,
            RagdollMode::GettingUp => self.getup_remaining as f32 / GETUP_BUDGET_TICKS as f32,
        }
    }

    /// Per-bone output weight (`None` for an out-of-range bone).
    pub fn bone_weight(&self, bone: usize) -> Option<f32> {
        if bone >= self.mask.len() {
            return None;
        }
        match self.mode {
            RagdollMode::Animated => Some(0.0),
            RagdollMode::Partial | RagdollMode::Full => {
                Some(if self.mask[bone] { 1.0 } else { 0.0 })
            }
            RagdollMode::GettingUp => Some(self.blend_weight()),
        }
    }

    /// Inject a hit impulse on one bone and free its subtree (partial ragdoll
    /// on hit): the struck bone plus every descendant simulate freely while
    /// the rest keep tracking. A hit interrupts an in-flight get-up.
    /// Non-finite impulse components and out-of-range bones are rejected; the
    /// world is unchanged then.
    pub fn apply_hit(
        &mut self,
        bone: usize,
        impulse: [f32; 3],
        tick: u64,
    ) -> Result<(), RagdollError> {
        if bone >= self.order.len() {
            return Err(RagdollError::BadBone(bone));
        }
        if impulse.iter().any(|c| !c.is_finite()) {
            return Err(RagdollError::NonFiniteImpulse);
        }
        let handle = self.order[bone];
        let Some(body) = self.bodies.get_mut(handle) else {
            return Err(RagdollError::BadBone(bone));
        };
        body.apply_impulse(Vector::new(impulse[0], impulse[1], impulse[2]), true);
        for i in 0..self.mask.len() {
            if is_descendant(&self.skeleton, i, bone) {
                self.mask[i] = true;
            }
        }
        self.mode = derive_mode(&self.mask);
        self.getup_remaining = 0;
        // The velocity change already carries the impulse; the counter only
        // records that a hit happened so the hash sees the event order. The
        // caller tick is arena bookkeeping, not sim state.
        self.hits = self.hits.wrapping_add(1);
        let _ = tick;
        Ok(())
    }

    /// Start the get-up: every drive re-engages and the output pose blends
    /// back to animation over [`GETUP_BUDGET_TICKS`] steps. Needs a downed
    /// body; calling it while fully animated reports [`NotRagdolled`](
    /// RagdollError::NotRagdolled) and changes nothing.
    pub fn start_getup(&mut self) -> Result<(), RagdollError> {
        if self.mode == RagdollMode::Animated {
            return Err(RagdollError::NotRagdolled);
        }
        for m in &mut self.mask {
            *m = false;
        }
        self.mode = RagdollMode::GettingUp;
        self.getup_remaining = GETUP_BUDGET_TICKS;
        Ok(())
    }

    /// Advance the simulation by one fixed step: driven bones get their servo
    /// velocities toward the animated targets (all bones during a get-up),
    /// then the world integrates. Only finite `dt > 0` is accepted; callers
    /// use [`FIXED_DT`]. Contacts are read into the pending list at the end
    /// of the step; the caller drains them after the step and mutates game
    /// state there, never inside it.
    pub fn step(&mut self, dt: f32) -> Result<(), RagdollError> {
        if !dt.is_finite() || dt <= 0.0 {
            return Err(RagdollError::InvalidDt);
        }
        write_targets(
            &self.skeleton,
            &self.animated,
            &mut self.target_pos,
            &mut self.target_rot,
        );
        let getting_up = self.mode == RagdollMode::GettingUp;
        for i in 0..self.order.len() {
            if !getting_up && self.mask[i] {
                continue;
            }
            self.drive_bone(i, dt);
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
        drain_narrow_contacts(
            &self.narrow,
            &self.colliders,
            &self.order,
            &mut self.pending,
            &mut self.contact_events,
            PoweredContact::ordered,
            PoweredContact::sort_key,
        );
        self.tick = self.tick.wrapping_add(1);
        if getting_up {
            self.getup_remaining = self.getup_remaining.saturating_sub(1);
            if self.getup_remaining == 0 {
                self.mode = RagdollMode::Animated;
            }
        }
        Ok(())
    }

    /// Output pose: driven bones return the animated input exactly (weight
    /// `0`), free bones return the simulated local pose (weight `1`), and a
    /// get-up blends every bone under the ramp weight.
    pub fn bone_poses(&self) -> LocalPose {
        if self.mode == RagdollMode::Animated {
            return self.animated.clone();
        }
        let w_global = self.blend_weight();
        let rest_pos = world_transforms(&self.skeleton, &self.animated).0;
        let (sim_pos, sim_rot) = read_sim_transforms(&self.bodies, &self.order, &rest_pos);
        let n = self.skeleton.len();
        let mut positions = Vec::with_capacity(n);
        let mut rotations = Vec::with_capacity(n);
        for (i, joint) in self.skeleton.joints.iter().enumerate() {
            let w = match self.mode {
                RagdollMode::GettingUp => w_global,
                _ => {
                    if self.mask[i] {
                        1.0
                    } else {
                        0.0
                    }
                }
            };
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
            if w <= 0.0 {
                positions.push(self.animated.positions[i]);
                rotations.push(self.animated.rotations[i]);
                continue;
            }
            if w >= 1.0 {
                positions.push(lp);
                rotations.push(lq);
                continue;
            }
            let ap = self.animated.positions[i];
            let aq = self.animated.rotations[i];
            positions.push([
                ap[0] + (lp[0] - ap[0]) * w,
                ap[1] + (lp[1] - ap[1]) * w,
                ap[2] + (lp[2] - ap[2]) * w,
            ]);
            // Shortest-path nlerp: negate when antipodal, then normalize
            // (one `sqrt`, allowed sim math). Linear in `w`.
            let mut bq = lq;
            let dot = aq[0] * bq[0] + aq[1] * bq[1] + aq[2] * bq[2] + aq[3] * bq[3];
            if dot < 0.0 {
                bq = [-bq[0], -bq[1], -bq[2], -bq[3]];
            }
            rotations.push(normalize_quat(&[
                aq[0] + (bq[0] - aq[0]) * w,
                aq[1] + (bq[1] - aq[1]) * w,
                aq[2] + (bq[2] - aq[2]) * w,
                aq[3] + (bq[3] - aq[3]) * w,
            ]));
        }
        LocalPose {
            positions,
            rotations,
        }
    }

    /// Add a static box (the floor). Mirrors the Rapier adapter's validation; returns an error
    /// for bad input and adds nothing then.
    pub fn add_static_box(
        &mut self,
        half_extents: [f32; 3],
        pos: [f32; 3],
    ) -> Result<(), RagdollError> {
        check_pos(pos)?;
        for h in half_extents {
            if !h.is_finite() || h == 0.0 || !(0.0..=MAX_LEN).contains(&h) {
                return Err(RagdollError::InvalidHalfExtents);
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

    /// Canonical hash over bones in skeleton order plus mask, mode, counters,
    /// driver pose and drive params. Each float is scaled by
    /// [`HASH_SCALE`](super::rapier::HASH_SCALE),
    /// rounded to `i64` and folded as little-endian bytes (FNV-1a). No float
    /// formatting, no transcendental calls.
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
        for m in &self.mask {
            h.write_bool(*m);
        }
        h.write_u8(mode_tag(self.mode));
        h.write_u64(self.getup_remaining);
        h.write_u64(self.tick);
        h.write_u64(self.hits);
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
        for d in &self.drives {
            for v in [
                d.linear_gain,
                d.angular_gain,
                d.linear_damping,
                d.angular_damping,
                d.max_speed,
                d.max_spin,
            ] {
                h.write_i64(quantize(v));
            }
        }
        h.finish()
    }

    /// Copy out the full future-affecting state.
    pub fn snapshot(&self) -> PoweredSnapshot {
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
        PoweredSnapshot {
            bodies,
            animated: self.animated.clone(),
            mask: self.mask.clone(),
            mode: self.mode,
            getup_remaining: self.getup_remaining,
            tick: self.tick,
            hits: self.hits,
            ccd_flags: self.ccd_flags.clone(),
            contact_events: self.contact_events,
            servo_lin: self.servo_lin.clone(),
            servo_ang: self.servo_ang.clone(),
            rig: self.rig,
        }
    }

    /// Restore a snapshot. Counts, mask length, mode countdown and pose must
    /// match; resuming with the same fixed `dt` sequence is identical to the
    /// recorded run, and the restored snapshot compares equal to the saved one.
    /// The pending contact list is observation, not sim state: it is cleared,
    /// and the next step refills it.
    pub fn restore(&mut self, snapshot: PoweredSnapshot) -> Result<(), RagdollError> {
        if snapshot.bodies.len() != self.order.len() || snapshot.mask.len() != self.mask.len() {
            return Err(RagdollError::SnapshotMismatch);
        }
        if snapshot.ccd_flags.len() != self.order.len() {
            return Err(RagdollError::SnapshotMismatch);
        }
        if snapshot.servo_lin.len() != self.order.len()
            || snapshot.servo_ang.len() != self.order.len()
        {
            return Err(RagdollError::SnapshotMismatch);
        }
        validate_pose(&self.skeleton, &snapshot.animated)
            .map_err(|_| RagdollError::SnapshotMismatch)?;
        match snapshot.mode {
            RagdollMode::GettingUp
                if snapshot.getup_remaining == 0
                    || snapshot.getup_remaining > GETUP_BUDGET_TICKS =>
            {
                return Err(RagdollError::SnapshotMismatch);
            }
            RagdollMode::Animated | RagdollMode::Partial | RagdollMode::Full
                if snapshot.getup_remaining != 0 =>
            {
                return Err(RagdollError::SnapshotMismatch);
            }
            _ => {}
        }
        for (handle, s) in self.order.iter().zip(snapshot.bodies.iter()) {
            let Some(body) = self.bodies.get_mut(*handle) else {
                return Err(RagdollError::SnapshotMismatch);
            };
            body.set_translation(Vector::new(s.pos[0], s.pos[1], s.pos[2]), false);
            // Rotation bits round-trip exactly: `from_xyzw` is a plain
            // constructor, so no re-normalize (that can shift the last ulp).
            // Non-finite components fall back to identity.
            let q = if s.rot.iter().all(|c| c.is_finite()) {
                Rotation::from_xyzw(s.rot[0], s.rot[1], s.rot[2], s.rot[3])
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
        self.mask = snapshot.mask;
        self.mode = snapshot.mode;
        self.getup_remaining = snapshot.getup_remaining;
        self.tick = snapshot.tick;
        self.hits = snapshot.hits;
        for (i, on) in snapshot.ccd_flags.iter().enumerate() {
            self.ccd_flags[i] = *on;
            if let Some(body) = self.bodies.get_mut(self.order[i]) {
                body.enable_ccd(*on);
            }
        }
        self.contact_events = snapshot.contact_events;
        self.servo_lin = snapshot.servo_lin;
        self.servo_ang = snapshot.servo_ang;
        self.rig = snapshot.rig;
        self.pending.clear();
        Ok(())
    }

    /// Fixed-point PID servo toward the animated target: P and I on the
    /// offset, D on the measured rate, a bounded integral and a slew cap on
    /// the commanded velocity, per 3-vector then one magnitude clamp (see
    /// [`crate::servo`]). The integer law removes the rest buzz the capped
    /// float PD left and drops the steady-state droop; a degenerate step `dt`
    /// answers the P term only and non-finite terms sanitize to zero, so a
    /// huge pose can never inject NaN into the world. A zero P gain leaves
    /// that axis untouched, so all-zero P gains are a limp bone.
    fn drive_bone(&mut self, i: usize, dt: f32) {
        let drive = self.drives[i];
        let target = self.target_pos[i];
        let target_rot = self.target_rot[i];
        let tuning = ServoTuning::for_rig(self.rig);
        // The integer law takes the step in microseconds; a step outside the
        // law's window (invalid, or past 1 s) reads as P-only inside `step`.
        let dt_us = servo::to_micro(dt);
        let Some(body) = self.bodies.get(self.order[i]) else {
            return;
        };
        let (p, lv, r, av) = {
            let r = body.rotation();
            (
                body.translation(),
                body.linvel(),
                [r.x, r.y, r.z, r.w],
                body.angvel(),
            )
        };
        // Linear servo: P and I on the position offset, D on measured velocity,
        // capped as one vector. Rapier integrates the assigned velocity, so
        // the loop is stable for any bone mass once its gains sit inside
        // `stable_gains`.
        let mut lin = None;
        if drive.linear_gain > 0.0 {
            // Inside the servo's stability region whatever the table says (see [`stable_gains`]).
            let (gain, damping) = stable_gains(drive.linear_gain, drive.linear_damping, dt);
            let err = [
                servo::to_micro(target[0] - p.x),
                servo::to_micro(target[1] - p.y),
                servo::to_micro(target[2] - p.z),
            ];
            let vel = [
                servo::to_micro(lv.x),
                servo::to_micro(lv.y),
                servo::to_micro(lv.z),
            ];
            let out = self.servo_lin[i].step(
                err,
                vel,
                dt_us,
                servo::Gains {
                    p: gain,
                    d: damping,
                },
                &tuning,
                cap_micro(drive.max_speed),
            );
            lin = Some(servo::from_vector(out, drive.max_speed));
        }
        // Angular servo: the error-quaternion vector part points along the
        // shortest correction axis with magnitude `sin(angle/2)` (plain
        // quaternion `+ - *`, no angle extraction), damped by measured spin.
        let mut ang = None;
        if drive.angular_gain > 0.0 {
            let mut qe = quat_mul(target_rot, quat_conj(&r));
            if qe[3] < 0.0 {
                qe = [-qe[0], -qe[1], -qe[2], -qe[3]];
            }
            // The error vector is sin(angle/2), so the real loop gain is half the table's;
            // clamping with the full gain is the conservative side of the same edge.
            let (gain, damping) = stable_gains(drive.angular_gain, drive.angular_damping, dt);
            let err = [
                servo::to_micro(qe[0]),
                servo::to_micro(qe[1]),
                servo::to_micro(qe[2]),
            ];
            let vel = [
                servo::to_micro(av.x),
                servo::to_micro(av.y),
                servo::to_micro(av.z),
            ];
            let out = self.servo_ang[i].step(
                err,
                vel,
                dt_us,
                servo::Gains {
                    p: gain,
                    d: damping,
                },
                &tuning,
                cap_micro(drive.max_spin),
            );
            ang = Some(servo::from_vector(out, drive.max_spin));
        }
        let Some(body) = self.bodies.get_mut(self.order[i]) else {
            return;
        };
        if let Some(v) = lin {
            body.set_linvel(Vector::new(v[0], v[1], v[2]), true);
        }
        if let Some(w) = ang {
            body.set_angvel(Vector::new(w[0], w[1], w[2]), true);
        }
    }
}

/// A speed cap (m/s or rad/s) as the servo's integer cap, µm/s.
fn cap_micro(cap: f32) -> i32 {
    i32::try_from(servo::to_micro(cap).clamp(0, i64::from(i32::MAX))).unwrap_or(0)
}

/// Mode implied by a mask: no free bones animates, all free is a full ragdoll.
fn derive_mode(mask: &[bool]) -> RagdollMode {
    if mask.iter().all(|m| !m) {
        RagdollMode::Animated
    } else if mask.iter().all(|m| *m) {
        RagdollMode::Full
    } else {
        RagdollMode::Partial
    }
}

/// Stable tag for hashing (never persisted; see [`PoweredSnapshot`]).
fn mode_tag(mode: RagdollMode) -> u8 {
    match mode {
        RagdollMode::Animated => 0,
        RagdollMode::Partial => 1,
        RagdollMode::Full => 2,
        RagdollMode::GettingUp => 3,
    }
}

/// True when `bone` equals `root` or descends from it via parent links.
/// Parents-first skeleton order keeps the walk bounded by the bone index.
fn is_descendant(skeleton: &Skeleton, bone: usize, root: usize) -> bool {
    let mut cur = bone;
    loop {
        if cur == root {
            return true;
        }
        match skeleton.joints.get(cur).and_then(|j| j.parent) {
            Some(p) => cur = p,
            None => return false,
        }
    }
}

/// Animated world transforms written into caller buffers (no allocation):
/// positions compose by rotated offset plus parent position, rotations by
/// quaternion multiply. Only `+ - *` math.
fn write_targets(
    skeleton: &Skeleton,
    pose: &LocalPose,
    pos: &mut [[f32; 3]],
    rot: &mut [[f32; 4]],
) {
    for (i, joint) in skeleton.joints.iter().enumerate() {
        let (pp, pr) = match joint.parent {
            Some(p) => (pos[p], rot[p]),
            None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
        };
        let rp = rotate_vec(&pose.positions[i], &pr);
        pos[i] = [pp[0] + rp[0], pp[1] + rp[1], pp[2] + rp[2]];
        rot[i] = quat_mul(pr, pose.rotations[i]);
    }
}

/// Animated world transforms, parent-first. Only `+ - *` math; callers
/// validate lengths and parent order first.
fn world_transforms(skeleton: &Skeleton, pose: &LocalPose) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let mut pos = vec![[0.0f32; 3]; skeleton.len()];
    let mut rot = vec![[0.0f32, 0.0, 0.0, 1.0]; skeleton.len()];
    write_targets(skeleton, pose, &mut pos, &mut rot);
    (pos, rot)
}

/// Drive params carry finite P/D gains with positive speed caps.
fn check_drive(d: &DriveParams) -> Result<(), ()> {
    for g in [
        d.linear_gain,
        d.angular_gain,
        d.linear_damping,
        d.angular_damping,
    ] {
        if !g.is_finite() || !(0.0..=MAX_DRIVE_GAIN).contains(&g) {
            return Err(());
        }
    }
    for c in [d.max_speed, d.max_spin] {
        if !c.is_finite() || c <= 0.0 || c > MAX_DRIVE_CAP {
            return Err(());
        }
    }
    Ok(())
}

/// Spawn args share one shape check: skeleton and pose pass the animation
/// contract, capsule and drive slices match the joint count, capsules carry
/// finite dimensions and drives carry valid gains. Units: meters for capsule
/// dims, 1/s and m/s for gains. Errors: [`RagdollError`] names the bad input.
fn validate_spawn_args(
    skeleton: &Skeleton,
    pose: &LocalPose,
    capsule_dims: &[BoneShape],
    drives: &[DriveParams],
) -> Result<(), RagdollError> {
    validate_skeleton(skeleton)?;
    validate_pose(skeleton, pose)?;
    if capsule_dims.len() != skeleton.len() {
        return Err(RagdollError::Anim(AnimError::LengthMismatch {
            expected: skeleton.len(),
            got: capsule_dims.len(),
        }));
    }
    if drives.len() != skeleton.len() {
        return Err(RagdollError::Anim(AnimError::LengthMismatch {
            expected: skeleton.len(),
            got: drives.len(),
        }));
    }
    for d in capsule_dims {
        if !d.radius.is_finite() || d.radius == 0.0 || !(0.0..=MAX_LEN).contains(&d.radius) {
            return Err(RagdollError::InvalidRadius);
        }
        if !d.half_height.is_finite() || !(0.0..=MAX_LEN).contains(&d.half_height) {
            return Err(RagdollError::InvalidHeight);
        }
    }
    for (i, d) in drives.iter().enumerate() {
        check_drive(d).map_err(|()| RagdollError::BadDrive(i))?;
    }
    Ok(())
}

fn check_pos(pos: [f32; 3]) -> Result<(), RagdollError> {
    if position_in_bounds(pos) {
        Ok(())
    } else {
        Err(RagdollError::NonFinitePosition)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ragdoll::{swing_limit, SWING_MIN};
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

    fn chain_drives() -> Vec<DriveParams> {
        vec![DriveParams::default_drive(); 4]
    }

    fn powered_on_floor() -> PoweredRagdoll {
        let mut r = PoweredRagdoll::spawn(
            &chain_skeleton(),
            &chain_pose(),
            &chain_shapes(),
            &chain_drives(),
        )
        .expect("spawn chain");
        r.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        r
    }

    fn step_n(r: &mut PoweredRagdoll, n: usize) {
        for _ in 0..n {
            r.step(FIXED_DT).expect("fixed step");
        }
    }

    fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
        ((a[0] - b[0]) * (a[0] - b[0])
            + (a[1] - b[1]) * (a[1] - b[1])
            + (a[2] - b[2]) * (a[2] - b[2]))
            .sqrt()
    }

    fn animated_world_targets() -> Vec<[f32; 3]> {
        world_transforms(&chain_skeleton(), &chain_pose()).0
    }

    #[test]
    fn spawn_is_animated_and_drives_track_pose() {
        let mut r = powered_on_floor();
        assert_eq!(r.mode(), RagdollMode::Animated);
        assert_eq!(r.mask(), &[false; 4]);
        assert_eq!(r.blend_weight(), 0.0);
        // Blend 0 returns the animated driver pose exactly, not approximately.
        assert_eq!(r.bone_poses(), chain_pose());
        // Powered tracking holds the chain aloft: after two seconds every bone
        // sits near its animated world target instead of piling on the floor.
        step_n(&mut r, 120);
        let targets = animated_world_targets();
        for (i, target) in targets.iter().enumerate() {
            let p = r.bone_position(i).expect("bone");
            assert!(
                dist3(p, *target) < 0.3,
                "driven bone {i} must track: {p:?} vs {target:?}"
            );
        }
        assert_eq!(r.mode(), RagdollMode::Animated);
        assert_eq!(r.bone_poses(), chain_pose());
    }

    #[test]
    fn per_bone_drive_params_take_effect() {
        // Single-bone twins: identical except the drive. The driven bone holds
        // its target against gravity; the limp bone (zero gains, so the servo
        // never touches it) falls to the floor. A jointed chain would hang
        // from its anchors instead, so one bone isolates the drive effect.
        let skeleton = Skeleton {
            joints: vec![Joint {
                name: "solo".into(),
                parent: None,
            }],
        };
        let pose = LocalPose {
            positions: vec![[0.0, 3.0, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]],
        };
        let shapes = vec![BoneShape {
            radius: 0.15,
            half_height: 0.2,
        }];
        let drives = vec![DriveParams::default_drive()];
        let mut stiff =
            PoweredRagdoll::spawn(&skeleton, &pose, &shapes, &drives).expect("spawn solo");
        let mut limp =
            PoweredRagdoll::spawn(&skeleton, &pose, &shapes, &drives).expect("spawn limp");
        for r in [&mut stiff, &mut limp] {
            r.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
                .expect("floor");
        }
        limp.set_drive(
            0,
            DriveParams {
                linear_gain: 0.0,
                angular_gain: 0.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                max_speed: 8.0,
                max_spin: 12.0,
            },
        )
        .expect("limp bone");
        assert_eq!(
            limp.drive(0),
            Some(DriveParams {
                linear_gain: 0.0,
                angular_gain: 0.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                max_speed: 8.0,
                max_spin: 12.0,
            })
        );
        assert_eq!(limp.drive(99), None);
        step_n(&mut stiff, 90);
        step_n(&mut limp, 90);
        let held = stiff.bone_position(0).expect("driven bone");
        assert!(
            (held[1] - 3.0).abs() < 0.2,
            "driven bone must hold its target: {held:?}"
        );
        let fallen = limp.bone_position(0).expect("limp bone");
        assert!(
            fallen[1] < 1.0,
            "limp bone must fall to the floor: {fallen:?}"
        );
        // The limp twin still counts as driven for masking: it reports
        // animated output while its sim body rests on the floor.
        assert_eq!(limp.mode(), RagdollMode::Animated);
        assert_ne!(stiff.state_hash(), limp.state_hash());
    }

    #[test]
    fn hit_frees_subtree_as_partial_ragdoll() {
        // Twin runs: only A takes a +X hit on bone 1. Bone 1 plus descendants
        // go free while the driven root keeps tracking in both twins.
        let mut a = powered_on_floor();
        let mut b = powered_on_floor();
        step_n(&mut a, 30);
        step_n(&mut b, 30);
        a.apply_hit(1, [0.06, 0.0, 0.0], 30).expect("hit bone 1");
        assert_eq!(a.mask(), &[false, true, true, true]);
        assert_eq!(a.mode(), RagdollMode::Partial);
        assert_eq!(a.blend_weight(), 1.0);
        assert_eq!(a.hit_count(), 1);
        assert_eq!(a.bone_weight(0), Some(0.0));
        assert_eq!(a.bone_weight(2), Some(1.0));
        assert_eq!(a.bone_weight(99), None);
        step_n(&mut a, 60);
        step_n(&mut b, 60);
        let targets = animated_world_targets();
        // Both roots keep tracking the animated target.
        for r in [&a, &b] {
            let root = r.bone_position(0).expect("root");
            assert!(
                dist3(root, targets[0]) < 0.4,
                "driven root must hold: {root:?}"
            );
        }
        // The struck bone travels with the impulse; descendants displace
        // through the joints. Rotation about the anchors can fling a
        // descendant opposite in X, so descendants assert 3D displacement.
        let p1a = a.bone_position(1).expect("bone 1a");
        let p1b = b.bone_position(1).expect("bone 1b");
        assert!(
            p1a[0] - p1b[0] > 0.05,
            "struck bone must travel with the hit: {p1a:?} vs {p1b:?}"
        );
        for i in 2..4 {
            let pa = a.bone_position(i).expect("bone a");
            let pb = b.bone_position(i).expect("bone b");
            assert!(
                dist3(pa, pb) > 0.1,
                "freed bone {i} must displace down-chain: {pa:?} vs {pb:?}"
            );
            assert!(pa.iter().all(|c| c.is_finite()));
        }
        // Partial output mixes per bone: driven bones read animated exactly.
        let out = a.bone_poses();
        assert_eq!(out.positions[0], chain_pose().positions[0]);
        assert_ne!(out.positions[2], chain_pose().positions[2]);
    }

    #[test]
    fn full_mask_reports_full_ragdoll() {
        let mut r = powered_on_floor();
        r.set_mask(&[true; 4]).expect("full mask");
        assert_eq!(r.mode(), RagdollMode::Full);
        assert_eq!(r.bone_weight(0), Some(1.0));
        step_n(&mut r, 600);
        // Untethered, the whole chain falls below its spawn heights and rests
        // above the floor instead of tunneling.
        let tops = [3.0, 3.5, 4.0, 4.5];
        for (i, top) in tops.iter().enumerate() {
            let p = r.bone_position(i).expect("bone");
            assert!(p[1] < top - 1.0, "free bone {i} must fall: {p:?}");
            assert!(p[1] > -0.5, "free bone {i} must not tunnel: {p:?}");
            assert!(p.iter().all(|c| c.is_finite()));
        }
        // Clearing the mask by hand animates again without a get-up countdown.
        r.set_mask(&[false; 4]).expect("clear mask");
        assert_eq!(r.mode(), RagdollMode::Animated);
        assert_eq!(r.getup_remaining(), 0);
        assert_eq!(r.bone_poses(), chain_pose());
    }

    #[test]
    fn getup_blends_back_within_budget() {
        let mut r = powered_on_floor();
        r.apply_hit(0, [0.05, 0.0, 0.0], 0).expect("knock down");
        assert_eq!(r.mode(), RagdollMode::Full);
        step_n(&mut r, 60);
        r.start_getup().expect("get up");
        assert_eq!(r.mode(), RagdollMode::GettingUp);
        assert_eq!(r.getup_remaining(), GETUP_BUDGET_TICKS);
        assert_eq!(r.mask(), &[false; 4]);
        // Halfway through the weight is mid-ramp and the mode still counts as
        // getting up; the pose is between sim and animated, never NaN.
        step_n(&mut r, (GETUP_BUDGET_TICKS / 2) as usize);
        assert_eq!(r.mode(), RagdollMode::GettingUp);
        let mid = r.blend_weight();
        assert!(
            mid > 0.25 && mid < 0.75,
            "mid-get-up weight must ramp: {mid}"
        );
        for q in r.bone_poses().rotations.iter() {
            assert!(q.iter().all(|c| c.is_finite()));
        }
        // The remaining budget lands exactly back in animation.
        step_n(&mut r, (GETUP_BUDGET_TICKS / 2) as usize);
        assert_eq!(r.tick(), 60 + GETUP_BUDGET_TICKS);
        assert_eq!(r.mode(), RagdollMode::Animated);
        assert_eq!(r.getup_remaining(), 0);
        assert_eq!(r.blend_weight(), 0.0);
        assert_eq!(r.bone_poses(), chain_pose());
        // Drives pulled the bodies back, so the sim matches the pose too.
        let targets = animated_world_targets();
        for (i, target) in targets.iter().enumerate() {
            let p = r.bone_position(i).expect("bone");
            assert!(
                dist3(p, *target) < 0.6,
                "recovered bone {i} must sit near its target: {p:?}"
            );
        }
    }

    #[test]
    fn getup_needs_a_downed_body_and_hits_interrupt_it() {
        let mut r = powered_on_floor();
        assert_eq!(r.start_getup(), Err(RagdollError::NotRagdolled));
        assert_eq!(r.mode(), RagdollMode::Animated);
        r.set_mask(&[false, false, true, true]).expect("partial");
        r.start_getup().expect("get up");
        step_n(&mut r, 10);
        // A fresh hit interrupts the get-up and re-ragdolls the subtree.
        r.apply_hit(2, [0.04, 0.0, 0.0], 10).expect("late hit");
        assert_eq!(r.mode(), RagdollMode::Partial);
        assert_eq!(r.getup_remaining(), 0);
        assert_eq!(r.mask(), &[false, false, true, true]);
        // A manual mask likewise cancels the countdown.
        r.start_getup().expect("get up again");
        r.set_mask(&[true; 4]).expect("manual full");
        assert_eq!(r.mode(), RagdollMode::Full);
        assert_eq!(r.getup_remaining(), 0);
    }

    #[test]
    fn determinism_twice_identical_through_getup() {
        // Same spawn plus same drive/hit/get-up script twice: identical hashes
        // and identical output poses, including across the budget boundary.
        let script = || {
            let mut r = powered_on_floor();
            step_n(&mut r, 10);
            r.apply_hit(1, [0.05, 0.0, 0.0], 10).expect("hit");
            step_n(&mut r, 20);
            r.start_getup().expect("get up");
            step_n(&mut r, GETUP_BUDGET_TICKS as usize);
            (r.state_hash(), r.bone_poses())
        };
        let (h1, p1) = script();
        let (h2, p2) = script();
        assert_eq!(h1, h2);
        assert_eq!(p1, p2);
        // A different hit script hashes differently (the hash sees hits).
        let mut r = powered_on_floor();
        step_n(&mut r, 10);
        r.apply_hit(2, [0.05, 0.0, 0.0], 10).expect("other hit");
        step_n(&mut r, 20);
        assert_ne!(r.state_hash(), h1);
    }

    #[test]
    fn snapshot_restore_exact_mid_getup() {
        let mut r = powered_on_floor();
        r.apply_hit(1, [0.05, 0.0, 0.0], 0).expect("hit");
        step_n(&mut r, 20);
        r.start_getup().expect("get up");
        step_n(&mut r, 15);
        let snap = r.snapshot();
        assert_eq!(snap.mode, RagdollMode::GettingUp);
        step_n(&mut r, 45);
        let (after_hash, after_pose, after_mode) = (r.state_hash(), r.bone_poses(), r.mode());
        assert_eq!(after_mode, RagdollMode::Animated);
        r.restore(snap.clone()).expect("restore");
        // Restored state matches the snapshot exactly, not approximately.
        assert_eq!(r.snapshot(), snap);
        assert_eq!(r.mode(), RagdollMode::GettingUp);
        step_n(&mut r, 45);
        assert_eq!(r.state_hash(), after_hash);
        assert_eq!(r.bone_poses(), after_pose);
        assert_eq!(r.mode(), RagdollMode::Animated);
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
        fn legacy(r: &PoweredRagdoll) -> u64 {
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
            for m in &r.mask {
                h ^= u64::from(*m);
                h = h.wrapping_mul(FNV64_PRIME);
            }
            h ^= u64::from(mode_tag(r.mode));
            h = h.wrapping_mul(FNV64_PRIME);
            for b in r
                .getup_remaining
                .to_le_bytes()
                .into_iter()
                .chain(r.tick.to_le_bytes())
                .chain(r.hits.to_le_bytes())
            {
                h ^= u64::from(b);
                h = h.wrapping_mul(FNV64_PRIME);
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
            for d in &r.drives {
                for v in [
                    d.linear_gain,
                    d.angular_gain,
                    d.linear_damping,
                    d.angular_damping,
                    d.max_speed,
                    d.max_spin,
                ] {
                    fold(&mut h, &quantize(v).to_le_bytes());
                }
            }
            h
        }
        // Animated spawn plus a masked mid-getup state cover every tag path.
        let fresh = powered_on_floor();
        assert_eq!(fresh.state_hash(), legacy(&fresh));
        let mut r = powered_on_floor();
        r.apply_hit(1, [0.05, 0.0, 0.0], 0).expect("hit");
        step_n(&mut r, 20);
        r.start_getup().expect("get up");
        step_n(&mut r, 15);
        assert_eq!(r.mode(), RagdollMode::GettingUp);
        assert_eq!(r.state_hash(), legacy(&r));
    }

    #[test]
    fn invalid_input_rejected_without_panic() {
        let skeleton = chain_skeleton();
        let pose = chain_pose();
        let shapes = chain_shapes();
        let drives = chain_drives();
        // Bad skeletons and poses name the animation-contract cause.
        assert!(PoweredRagdoll::spawn(&Skeleton::default(), &pose, &shapes, &drives).is_err());
        let mut short = pose.clone();
        short.positions.pop();
        assert!(PoweredRagdoll::spawn(&skeleton, &short, &shapes, &drives).is_err());
        // Shape and drive count mismatches name the expected length.
        assert!(PoweredRagdoll::spawn(&skeleton, &pose, &shapes[..2], &drives).is_err());
        assert!(PoweredRagdoll::spawn(&skeleton, &pose, &shapes, &drives[..2]).is_err());
        // Bad capsule dimensions.
        let mut bad_shapes = shapes.clone();
        bad_shapes[0].radius = 0.0;
        assert_eq!(
            PoweredRagdoll::spawn(&skeleton, &pose, &bad_shapes, &drives).map(|_| ()),
            Err(RagdollError::InvalidRadius)
        );
        bad_shapes = shapes.clone();
        bad_shapes[1].half_height = f32::NAN;
        assert_eq!(
            PoweredRagdoll::spawn(&skeleton, &pose, &bad_shapes, &drives).map(|_| ()),
            Err(RagdollError::InvalidHeight)
        );
        // Bad per-bone drive params name the bone.
        for (i, bad) in [
            DriveParams {
                linear_gain: -1.0,
                ..DriveParams::default_drive()
            },
            DriveParams {
                angular_gain: f32::INFINITY,
                ..DriveParams::default_drive()
            },
            DriveParams {
                max_speed: 0.0,
                ..DriveParams::default_drive()
            },
            DriveParams {
                max_spin: f32::NAN,
                ..DriveParams::default_drive()
            },
            DriveParams {
                linear_damping: -1.0,
                ..DriveParams::default_drive()
            },
            DriveParams {
                angular_damping: f32::INFINITY,
                ..DriveParams::default_drive()
            },
        ]
        .iter()
        .enumerate()
        {
            let mut bad_drives = drives.clone();
            bad_drives[1] = *bad;
            assert_eq!(
                PoweredRagdoll::spawn(&skeleton, &pose, &shapes, &bad_drives).map(|_| ()),
                Err(RagdollError::BadDrive(1)),
                "drive case {i}"
            );
        }
        let mut r = powered_on_floor();
        // Bad bones, impulses, masks and dt.
        assert_eq!(
            r.apply_hit(99, [0.01, 0.0, 0.0], 0),
            Err(RagdollError::BadBone(99))
        );
        assert_eq!(
            r.apply_hit(1, [f32::INFINITY, 0.0, 0.0], 0),
            Err(RagdollError::NonFiniteImpulse)
        );
        assert_eq!(
            r.set_mask(&[true; 2]),
            Err(RagdollError::BadMask {
                expected: 4,
                got: 2
            })
        );
        assert_eq!(
            r.set_drive(
                9,
                DriveParams {
                    linear_gain: 1.0,
                    angular_gain: 1.0,
                    linear_damping: 1.0,
                    angular_damping: 1.0,
                    max_speed: 1.0,
                    max_spin: 1.0,
                }
            ),
            Err(RagdollError::BadBone(9))
        );
        assert_eq!(
            r.set_drive(
                0,
                DriveParams {
                    linear_gain: f32::NAN,
                    angular_gain: 1.0,
                    linear_damping: 1.0,
                    angular_damping: 1.0,
                    max_speed: 1.0,
                    max_spin: 1.0,
                }
            ),
            Err(RagdollError::BadDrive(0))
        );
        assert_eq!(r.step(0.0), Err(RagdollError::InvalidDt));
        assert_eq!(r.step(f32::NAN), Err(RagdollError::InvalidDt));
        assert!(r.add_static_box([0.0, 0.5, 0.5], [0.0, 0.0, 0.0]).is_err());
        assert_eq!(r.bone_position(99), None);
        assert_eq!(r.bone_linvel(99), None);
        // Mismatched snapshots restore nothing: body count, mask length,
        // get-up countdown consistency and driver pose length.
        let mut snap = r.snapshot();
        snap.bodies.pop();
        assert_eq!(r.restore(snap), Err(RagdollError::SnapshotMismatch));
        let mut snap = r.snapshot();
        snap.mask.pop();
        assert_eq!(r.restore(snap), Err(RagdollError::SnapshotMismatch));
        let mut snap = r.snapshot();
        snap.mode = RagdollMode::GettingUp;
        snap.getup_remaining = 0;
        assert_eq!(r.restore(snap), Err(RagdollError::SnapshotMismatch));
        let mut snap = r.snapshot();
        snap.mode = RagdollMode::GettingUp;
        snap.getup_remaining = GETUP_BUDGET_TICKS + 1;
        assert_eq!(r.restore(snap), Err(RagdollError::SnapshotMismatch));
        let mut snap = r.snapshot();
        snap.getup_remaining = 5;
        assert_eq!(r.restore(snap), Err(RagdollError::SnapshotMismatch));
        let mut snap = r.snapshot();
        snap.animated.positions.pop();
        assert_eq!(r.restore(snap), Err(RagdollError::SnapshotMismatch));
    }

    #[test]
    fn swing_limits_derive_from_depth() {
        // Contract probe: limits come from skeleton hierarchy depth alone.
        assert!((swing_limit(1) - 0.75).abs() < 1e-6);
        assert!((swing_limit(2) - 0.6).abs() < 1e-6);
        assert!(swing_limit(1) > swing_limit(3));
        assert!(swing_limit(100) >= SWING_MIN);
        assert_eq!(joint_depths(&chain_skeleton()), vec![0, 1, 2, 3]);
        assert!(is_descendant(&chain_skeleton(), 3, 1));
        assert!(is_descendant(&chain_skeleton(), 1, 1));
        assert!(!is_descendant(&chain_skeleton(), 0, 1));
        assert!(!is_descendant(&chain_skeleton(), 1, 3));
    }

    #[test]
    fn mirrored_hits_stay_mirrored() {
        // Twin runs mirrored across the YZ plane: the chain spawns symmetric
        // about X = 0, so a +X hit on bone 1 in A and the negated -X hit in B
        // must keep every bone mirrored (X negated, Y/Z equal). The check
        // runs through driven roots, freed subtrees and the get-up path so a
        // mirrored checkpoint restores a mirrored future.
        let mut a = powered_on_floor();
        let mut b = powered_on_floor();
        step_n(&mut a, 30);
        step_n(&mut b, 30);
        a.apply_hit(1, [0.05, 0.0, 0.0], 30).expect("hit A");
        b.apply_hit(1, [-0.05, 0.0, 0.0], 30).expect("hit B");
        assert_eq!(a.snapshot().mask, b.snapshot().mask);
        step_n(&mut a, 60);
        step_n(&mut b, 60);
        // The hits actually moved the struck bones apart in X, so the mirror
        // relation below is exercised, not vacuous.
        let ax = a.bone_position(1).expect("bone A")[0];
        let bx = b.bone_position(1).expect("bone B")[0];
        assert!(ax > 0.02, "A must travel +X: {ax}");
        assert!(bx < -0.02, "B must travel -X: {bx}");
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
        // Mirrored checkpoints resume mirrored: restore A from a mirror of
        // B's snapshot and both twins hash-track their own line.
        // Reflection across the YZ plane negates X of positions and linear
        // velocities; a rotation `(x, y, z, w)` mirrors to `(x, -y, -z, w)`
        // (axis X flips with the plane, Y/Z counter-rotate) and angular
        // velocity, a pseudovector, to `(wx, -wy, -wz)`.
        let mut snapshot_b = b.snapshot();
        for body in &mut snapshot_b.bodies {
            body.pos[0] = -body.pos[0];
            body.linvel[0] = -body.linvel[0];
            body.rot[1] = -body.rot[1];
            body.rot[2] = -body.rot[2];
            body.angvel[1] = -body.angvel[1];
            body.angvel[2] = -body.angvel[2];
        }
        // The servo history mirrors with the checkpoint: the linear command
        // and integral are odd in X, the angular ones in Y and Z (a
        // pseudovector under the same reflection).
        for s in &mut snapshot_b.servo_lin {
            *s = s.mirrored([true, false, false]);
        }
        for s in &mut snapshot_b.servo_ang {
            *s = s.mirrored([false, true, true]);
        }
        a.restore(snapshot_b).expect("restore mirrored");
        // The mirrored checkpoint resumes a mirrored future: stepping both
        // twins keeps every bone mirrored, so the checkpoint carries the
        // same trajectory, reflected.
        step_n(&mut a, 30);
        step_n(&mut b, 30);
        assert_eq!(a.snapshot().mask, b.snapshot().mask);
        assert_eq!(a.mode(), b.mode());
        for i in 0..4 {
            let pa = a.bone_position(i).expect("bone A");
            let pb = b.bone_position(i).expect("bone B");
            // 2 mm at a 4 m height: the integer servo rounds its command to
            // the micro-unit and slews it, so contact-phase twins on the floor
            // drift a hair more than the float PD did (measured 1.1 mm here).
            assert!(
                (pa[0] + pb[0]).abs() < 2e-3
                    && (pa[1] - pb[1]).abs() < 2e-3
                    && (pa[2] - pb[2]).abs() < 2e-3,
                "resumed bone {i} must stay mirrored: {pa:?} vs {pb:?}"
            );
        }
    }

    #[test]
    fn step_allocations_audited_and_pinned() {
        // Allocation audit: our tick glue allocates nothing — drive
        // targets are preallocated at spawn and `state_hash` folds on the
        // stack — while `PhysicsPipeline::step` from rapier3d 0.35 keeps
        // per-step locals (substep moved/quarantine lists, user-change wake
        // lists) that no caller configuration removes. Measured 2026-09-30
        // on all phases (driven aloft, falling, rested): 4 heap calls and
        // ~145 bytes per step, steady-state, so the pin below guards the
        // budget instead of asserting an unreachable zero.
        let mut r = powered_on_floor();
        step_n(&mut r, 30);
        r.apply_hit(1, [0.05, 0.0, 0.0], 30).expect("hit");
        let (hash_before, tick_allocs) = engine_core::alloc_probe::measure(|| {
            step_n(&mut r, 60);
            r.state_hash()
        });
        assert!(
            tick_allocs.calls <= 300 && tick_allocs.bytes <= 12_000,
            "tick budget: 60 steps must stay near 4 calls/step inside rapier, got {tick_allocs:?}"
        );
        let (_, hash_allocs) = engine_core::alloc_probe::measure(|| r.state_hash());
        assert_eq!(
            hash_allocs.calls, 0,
            "our hash must not allocate, got {hash_allocs:?}"
        );
        // The probed run is real work: the struck bone displaced and the mode
        // is partial, so the budget above guards a live tick, not a no-op.
        assert_eq!(r.mode(), RagdollMode::Partial);
        assert_eq!(r.state_hash(), hash_before);
        let p = r.bone_position(1).expect("bone");
        assert!(p[0] > 0.02, "struck bone must travel: {p:?}");
    }

    #[test]
    fn ccd_fast_blade_stopped_by_thin_shield() {
        // Accept on the powered path: a freed blade fired at 40 m/s at
        // a 5 cm shield stops at the plate (CCD on) and the listener sees
        // the hit; the same script twice hashes and drains identically.
        use engine_core::Rng;
        fn swing(seed: u64) -> ([f32; 3], Vec<PoweredContact>, u64) {
            let mut rng = Rng::new(0x000B_1ADE).fork(seed);
            let dy = (rng.next_u64() % 1000) as f32 / 1000.0 - 0.5;
            let skeleton = Skeleton {
                joints: vec![Joint {
                    name: "blade".into(),
                    parent: None,
                }],
            };
            let pose = LocalPose {
                positions: vec![[-2.0, 1.0 + dy * 0.2, 0.0]],
                rotations: vec![[0.0, 0.0, 0.0, 1.0]],
            };
            let shapes = vec![BoneShape {
                radius: 0.02,
                half_height: 0.30,
            }];
            let drives = vec![DriveParams::default_drive()];
            let mut r = PoweredRagdoll::spawn(&skeleton, &pose, &shapes, &drives).expect("blade");
            r.add_static_box([0.025, 1.0, 1.0], [0.0, 1.0, 0.0])
                .expect("shield");
            r.set_mask(&[true]).expect("free blade");
            assert_eq!(r.ccd_enabled(0), Some(true));
            r.set_bone_linvel(0, [40.0, 0.0, 0.0]).expect("fire blade");
            let mut saw: Vec<PoweredContact> = Vec::new();
            for _ in 0..12 {
                r.step(FIXED_DT).expect("fixed step");
                saw.extend(r.take_contacts());
            }
            (r.bone_position(0).expect("blade pos"), saw, r.state_hash())
        }
        use engine_core::Fnv64;
        let mut digest_a = Fnv64::new();
        let mut digest_b = Fnv64::new();
        for seed in 0..1_000u64 {
            let (pa, ca, ha) = swing(seed);
            let (pb, cb, hb) = swing(seed);
            assert_eq!(pa, pb, "seed {seed} must repeat: {pa:?} vs {pb:?}");
            assert_eq!(ca, cb, "seed {seed} contacts must repeat");
            assert_eq!(ha, hb, "seed {seed} hash must repeat");
            assert!(
                pa[0] < 0.0,
                "seed {seed}: blade tunneled past the shield: {pa:?}"
            );
            assert!(!ca.is_empty(), "seed {seed}: listener must see the hit");
            assert!(ca.iter().all(|c| c.a == 0 && c.b.is_none()));
            for v in [pa[0], pa[1], pa[2]] {
                digest_a.write_i64(quantize(v));
                digest_b.write_i64(quantize(v));
            }
        }
        assert_eq!(digest_a.finish(), digest_b.finish());
    }

    #[test]
    fn ccd_toggle_and_contact_drain_reject_bad_bones() {
        // Failure case: CCD and velocity setters name out-of-range bones and
        // change nothing; an empty step drains empty.
        let mut r = powered_on_floor();
        assert_eq!(r.ccd_enabled(0), Some(true));
        assert_eq!(r.ccd_enabled(99), None);
        assert_eq!(r.set_ccd(99, false), Err(RagdollError::BadBone(99)));
        assert_eq!(
            r.set_bone_linvel(99, [1.0, 0.0, 0.0]),
            Err(RagdollError::BadBone(99))
        );
        assert_eq!(
            r.set_bone_linvel(0, [f32::NAN, 0.0, 0.0]),
            Err(RagdollError::NonFiniteImpulse)
        );
        r.set_ccd(1, false).expect("slow bone off");
        assert_eq!(r.ccd_enabled(1), Some(false));
        r.set_ccd(1, true).expect("fast limb back on");
        assert_eq!(r.contacts(), &[]);
        assert_eq!(r.take_contacts(), vec![]);
        assert_eq!(r.contact_events(), 0);
    }

    #[test]
    fn contact_drain_feeds_hits_after_step() {
        // The wired consumer: after a blade-vs-shield step the drain turns
        // every contact into one applied hit; a bad impulse drains nothing.
        // The contact/joint follow-up pass calls this after the step
        // instead of reading `take_contacts` raw.
        use engine_core::Rng;
        fn script(seed: u64) -> (usize, u64, u64) {
            let mut rng = Rng::new(0xD2A1).fork(seed);
            let dy = (rng.next_u64() % 1000) as f32 / 1000.0 - 0.5;
            let skeleton = Skeleton {
                joints: vec![Joint {
                    name: "blade".into(),
                    parent: None,
                }],
            };
            let pose = LocalPose {
                positions: vec![[-2.0, 1.0 + dy * 0.2, 0.0]],
                rotations: vec![[0.0, 0.0, 0.0, 1.0]],
            };
            let shapes = vec![BoneShape {
                radius: 0.02,
                half_height: 0.30,
            }];
            let drives = vec![DriveParams::default_drive()];
            let mut r = PoweredRagdoll::spawn(&skeleton, &pose, &shapes, &drives).expect("blade");
            r.add_static_box([0.025, 1.0, 1.0], [0.0, 1.0, 0.0])
                .expect("shield");
            r.set_mask(&[true]).expect("free blade");
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
                Err(RagdollError::NonFiniteImpulse)
            );
            assert!(!r.contacts().is_empty(), "bad impulse drains nothing");
            let applied = r
                .drain_contacts_into_hits([0.05, 0.0, 0.0], 12)
                .expect("drain into hits");
            assert!(applied >= 1, "one shield contact strikes once");
            assert_eq!(r.hit_count(), applied as u64);
            assert!(r.contacts().is_empty(), "drain empties the pending list");
            (applied, r.hit_count(), r.state_hash())
        }
        let (a, ha, xa) = script(7);
        let (b, hb, xb) = script(7);
        assert_eq!((a, ha, xa), (b, hb, xb), "drain script must repeat");
    }
}
