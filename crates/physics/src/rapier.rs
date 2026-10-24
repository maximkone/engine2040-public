//! rapier: Rapier adapter behind the opt-in `rapier` feature.
//!
//! Capsules plus static boxes on a fixed step, with a canonical state hash
//! and owned snapshot/restore for rollback. The headless default build stays
//! std-only; this module only exists with `features = ["rapier"]`.
//!
//! Determinism scope (HASH-LAW tier note): the world runs rapier3d with
//! `enhanced-determinism` and without SIMD/parallel features, one fixed `dt`
//! per [`World::step`] call, bodies iterated in spawn order. The glue in this
//! module uses only comparisons, `+ - * /`, `round` and integer math: no
//! formatting of floats, no `sin`/`cos`/`exp`/`powf` family calls. Rapier
//! internals use `+ - * /` and `sqrt` under their `enhanced-determinism`
//! guarantee; that internal float use is the arena integration's scope, not
//! part of this glue. Character controllers, arenas and network rollback
//! consume this API;
//! they own any further determinism claims.
//!
//! RL58 debug lines live beside the tick: [`World::collect_debug`] copies the
//! collider and joint outlines after a step into [`RapierDebugLines`]. The
//! copy reads the world and never steps, restores or hashes it. Callers push
//! their own post-tick sweep, hurt and nav lines through the same groups.
#![forbid(unsafe_code)]

use super::ragdoll::{CouplingParams, CouplingStats};
use crate::islands::{self, SnapshotError, STATE_FLOATS};
use crate::lod::{ccd_for, AgentKind, Ccd, LodConfig, Motion};
use engine_anim::contract::Skeleton;
use engine_core::Fnv64;
use rapier3d::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet,
    RigidBodyBuilder, RigidBodyHandle, RigidBodySet, SoftBodySet,
};
use rapier3d::geometry::{BroadPhaseBvh, ColliderBuilder, ColliderSet, NarrowPhase};
use rapier3d::math::{Rotation, Vector};
use rapier3d::pipeline::{
    DebugColor, DebugRenderBackend, DebugRenderMode, DebugRenderObject, DebugRenderPipeline,
    DebugRenderStyle, PhysicsPipeline,
};
use std::fmt::{Display, Formatter};

/// Fixed step, 60 Hz. Callers pass this to [`World::step`].
/// Single owner is `engine_core::FIXED_DT`.
pub use engine_core::FIXED_DT;
/// Gravity, m/s^2, -Y.
pub const GRAVITY_Y: f32 = -9.81;
/// Hash quantization: state floats are scaled by this and rounded to `i64`.
pub const HASH_SCALE: f64 = 1_000_000.0;
/// Upper bound for accepted lengths (radii, heights, half-extents, positions).
pub(crate) const MAX_LEN: f32 = 1_000.0;
/// Total body mass the ragdoll builder assumes, kilograms. The Forge ragdoll
/// wizard uses the same 80 kg default for its mass table.
pub const RAGDOLL_TOTAL_MASS_KG: f32 = 80.0;
/// Solver passes a ragdoll runs with (Rapier's `num_solver_iterations`).
/// Rapier 0.36 defaults to 4; the twelve-bone chain test picks this number:
/// at four passes the chain opens its joints by 2.6 cm under the spear
/// thrust, at eight by 1.2 cm and at twelve by 0.8 cm (the accept bar is
/// 5 cm, the shipped margin 1 cm).
pub const RAGDOLL_SOLVER_PASSES: usize = 12;
/// Ceiling on the solver passes a caller may ask for.
pub const RAGDOLL_MAX_SOLVER_PASSES: usize = 64;
/// How far the bone masses may miss the total, as a fraction of it.
pub const RAGDOLL_MASS_TOLERANCE: f32 = 0.01;

/// One captured debug line: endpoints in meters plus a linear RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebugLine {
    /// Start point in meters.
    pub a_m: [f32; 3],
    /// End point in meters.
    pub b_m: [f32; 3],
    /// Linear RGBA, each channel 0..=1.
    pub colour_rgba: [f32; 4],
}

/// The fixed groups the Rapier debug backend fills each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugGroup {
    /// Collider shapes and AABBs in the Rapier world.
    Colliders,
    /// Impulse and multibody joint frames.
    Joints,
    /// Combat sweep capsules, copied after the tick by the caller.
    Sweeps,
    /// Hurt capsules, copied after the tick by the caller.
    Hurt,
    /// Nav paths, copied after the tick by the caller.
    Nav,
}

impl DebugGroup {
    /// Every group in batch order.
    pub const ALL: [Self; 5] = [
        Self::Colliders,
        Self::Joints,
        Self::Sweeps,
        Self::Hurt,
        Self::Nav,
    ];

    /// Stable name the render recorder finds again.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Colliders => "colliders",
            Self::Joints => "joints",
            Self::Sweeps => "sweeps",
            Self::Hurt => "hurt",
            Self::Nav => "nav",
        }
    }

    /// Depth bias in thousandths of the depth range; negative sits closer.
    pub const fn depth_bias_permille(self) -> i32 {
        match self {
            Self::Colliders => -8,
            Self::Joints => -4,
            Self::Sweeps => 0,
            Self::Hurt => 2,
            Self::Nav => 4,
        }
    }

    /// Linear RGBA every line of this group carries.
    pub const fn colour_rgba(self) -> [f32; 4] {
        match self {
            Self::Colliders => [0.25, 0.85, 1.0, 1.0],
            Self::Joints => [1.0, 0.82, 0.25, 1.0],
            Self::Sweeps => [1.0, 0.50, 0.20, 1.0],
            Self::Hurt => [1.0, 0.25, 0.55, 1.0],
            Self::Nav => [0.40, 1.0, 0.45, 1.0],
        }
    }

    /// Index into [`RapierDebugLines::groups`].
    const fn index(self) -> usize {
        match self {
            Self::Colliders => 0,
            Self::Joints => 1,
            Self::Sweeps => 2,
            Self::Hurt => 3,
            Self::Nav => 4,
        }
    }
}

/// One captured group in batch order, with the bias and colour the live pass
/// applies to every line inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct RapierDebugGroup {
    name: &'static str,
    depth_bias_permille: i32,
    colour_rgba: [f32; 4],
    lines: Vec<DebugLine>,
}

impl RapierDebugGroup {
    /// Stable group name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Depth bias in thousandths of the depth range; negative sits closer.
    pub fn depth_bias_permille(&self) -> i32 {
        self.depth_bias_permille
    }

    /// Linear RGBA every line in the group carries.
    pub fn colour_rgba(&self) -> [f32; 4] {
        self.colour_rgba
    }

    /// The group's lines in capture order.
    pub fn lines(&self) -> &[DebugLine] {
        &self.lines
    }
}

/// Post-tick debug line copies for the live line pass and the recorder. Debug
/// only: the tick never reads this and it is never hashed.
#[derive(Clone, Debug, PartialEq)]
pub struct RapierDebugLines {
    groups: [RapierDebugGroup; 5],
}

impl RapierDebugLines {
    /// Five empty groups in [`DebugGroup::ALL`] order.
    pub fn new() -> Self {
        Self {
            groups: DebugGroup::ALL.map(|group| RapierDebugGroup {
                name: group.name(),
                depth_bias_permille: group.depth_bias_permille(),
                colour_rgba: group.colour_rgba(),
                lines: Vec::new(),
            }),
        }
    }

    /// Drop every line but keep the five groups.
    pub fn clear(&mut self) {
        for group in &mut self.groups {
            group.lines.clear();
        }
    }

    /// Append one line to `group`, using the group's colour. False when an
    /// endpoint is not finite; nothing is written then.
    pub fn push(&mut self, group: DebugGroup, a_m: [f32; 3], b_m: [f32; 3]) -> bool {
        if !a_m.iter().chain(b_m.iter()).all(|v| v.is_finite()) {
            return false;
        }
        let g = &mut self.groups[group.index()];
        let colour_rgba = g.colour_rgba;
        g.lines.push(DebugLine {
            a_m,
            b_m,
            colour_rgba,
        });
        true
    }

    /// Every group in batch order.
    pub fn groups(&self) -> &[RapierDebugGroup] {
        &self.groups
    }

    /// One group by name.
    pub fn group(&self, group: DebugGroup) -> &RapierDebugGroup {
        &self.groups[group.index()]
    }

    /// Total lines across all groups.
    pub fn line_count(&self) -> usize {
        self.groups.iter().map(|g| g.lines.len()).sum()
    }
}

impl Default for RapierDebugLines {
    fn default() -> Self {
        Self::new()
    }
}

/// Rapier calls this trait for every collider and joint line; the backend
/// keeps only those two and drops the modes the pipeline was not asked for.
impl DebugRenderBackend for RapierDebugLines {
    fn draw_line(
        &mut self,
        object: DebugRenderObject<'_>,
        a: Vector,
        b: Vector,
        _color: DebugColor,
    ) {
        let group = match object {
            DebugRenderObject::Collider(..) | DebugRenderObject::ColliderAabb(..) => {
                DebugGroup::Colliders
            }
            DebugRenderObject::ImpulseJoint(..) | DebugRenderObject::MultibodyJoint(..) => {
                DebugGroup::Joints
            }
            DebugRenderObject::RigidBody(..)
            | DebugRenderObject::ContactPair(..)
            | DebugRenderObject::SoftBody(..) => return,
        };
        self.push(group, [a.x, a.y, a.z], [b.x, b.y, b.z]);
    }
}

/// The pipeline the Rapier backend fills: collider shapes and joint frames
/// only, so a caller never sees contact or soft-body noise.
pub fn debug_render_pipeline() -> DebugRenderPipeline {
    DebugRenderPipeline::new(
        DebugRenderStyle::default(),
        DebugRenderMode::COLLIDER_SHAPES | DebugRenderMode::JOINTS,
    )
}

/// Typed errors for the Rapier adapter. User input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `step` needs a finite `dt` greater than zero.
    InvalidDt,
    /// Capsule radius must be finite and in `(0, MAX_LEN]`.
    InvalidRadius,
    /// Capsule cylinder height must be finite and in `[0, MAX_LEN]`.
    InvalidHeight,
    /// Static-box half-extents must be finite and each in `(0, MAX_LEN]`.
    InvalidHalfExtents,
    /// A position component was non-finite or out of range.
    NonFinitePosition,
    /// Snapshot body count does not match the world's dynamic body count.
    SnapshotMismatch,
    /// Island snapshot bytes were refused; the world is unchanged.
    SnapshotBytes(SnapshotError),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Error::InvalidDt => "invalid dt: need finite dt > 0",
            Error::InvalidRadius => "invalid radius: need finite radius in (0, 1000]",
            Error::InvalidHeight => "invalid height: need finite height in [0, 1000]",
            Error::InvalidHalfExtents => "invalid half-extents: need finite values in (0, 1000]",
            Error::NonFinitePosition => "invalid position: need finite values within +-1000",
            Error::SnapshotMismatch => "snapshot body count does not match world",
            Error::SnapshotBytes(e) => return write!(f, "island snapshot bytes refused: {e}"),
        };
        f.write_str(s)
    }
}

impl std::error::Error for Error {}

/// Typed errors of the ragdoll params builder. Bad caller input never panics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamsError {
    /// A mass fraction is not a finite number in `[0, 1]`.
    BadFraction(usize),
    /// The fractions miss 1 by more than [`RAGDOLL_MASS_TOLERANCE`].
    MassSum {
        /// The sum the table gave.
        got: f32,
    },
    /// The total mass is not a finite positive number of kilograms.
    BadTotalMass,
    /// The solver pass count is zero or above [`RAGDOLL_MAX_SOLVER_PASSES`].
    BadSolverPasses,
    /// The fraction table has no row for every bone of the skeleton.
    BoneCount {
        /// Bone count of the skeleton.
        want: usize,
        /// Row count of the table.
        got: usize,
    },
}

impl Display for ParamsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamsError::BadFraction(i) => {
                write!(
                    f,
                    "invalid mass fraction: row {i} is not a number in [0, 1]"
                )
            }
            ParamsError::MassSum { got } => write!(
                f,
                "mass fractions sum to {got}: need 1 within {RAGDOLL_MASS_TOLERANCE}"
            ),
            ParamsError::BadTotalMass => {
                f.write_str("invalid total mass: need a finite positive number of kilograms")
            }
            ParamsError::BadSolverPasses => write!(
                f,
                "invalid solver passes: need 1..={RAGDOLL_MAX_SOLVER_PASSES}"
            ),
            ParamsError::BoneCount { want, got } => {
                write!(
                    f,
                    "mass table has {got} rows, the skeleton has {want} bones"
                )
            }
        }
    }
}

impl std::error::Error for ParamsError {}

/// One named mass row: the bone names that pick it and the fraction of the
/// body mass that segment carries.
struct MassRow {
    /// Lower-case names; a bone takes the row whose name it contains.
    names: &'static [&'static str],
    /// Fraction of the total body mass.
    fraction: f32,
}

/// The segment masses of a human body as fractions of the total: the
/// Plagenhoef table the Forge ragdoll wizard ships (Plagenhoef 1983, retyped
/// there as the twelve rows of its twelve-bone plan). The rows name a pelvis,
/// two spine bones, a head, two upper arms, two forearms, two thighs and two
/// shanks; the table has no row for a hand, a foot or a neck.
///
/// A row with a short name must come before the row it is a prefix of:
/// `forearm` and `lower_arm` both contain `arm`, so the forearm row is first
/// or a forearm would weigh an upper arm.
const MASS_ROWS: &[MassRow] = &[
    MassRow {
        names: &["pelvis", "hips"],
        fraction: 0.135,
    },
    MassRow {
        names: &["spine1"],
        fraction: 0.131,
    },
    MassRow {
        names: &["spine2", "chest"],
        fraction: 0.155,
    },
    MassRow {
        names: &["head"],
        fraction: 0.0694,
    },
    MassRow {
        names: &["forearm", "lower_arm", "elbow"],
        fraction: 0.015,
    },
    MassRow {
        names: &["upper_arm", "shoulder", "arm"],
        fraction: 0.0263,
    },
    MassRow {
        names: &["upper_leg", "up_leg", "upleg", "thigh", "hip"],
        fraction: 0.105,
    },
    MassRow {
        names: &["shank", "shin", "knee", "leg"],
        fraction: 0.0433,
    },
];

/// The mass fraction of one bone, from its name. `None` when the table does
/// not name it.
fn named_fraction(name: &str) -> Option<f32> {
    let lower = name.to_ascii_lowercase();
    MASS_ROWS
        .iter()
        .find(|row| row.names.iter().any(|n| lower.contains(n)))
        .map(|row| row.fraction)
}

/// The mass fractions of one rig: one per bone, in skeleton order.
///
/// A bone the table names takes its published fraction. The mass the named
/// rows leave (the table names no hand, foot or neck) is shared by the bones
/// it does not name in proportion to their size, so a rig of unnamed bones
/// keeps the mass ratio of its shapes and no bone weighs nothing.
pub fn mass_fractions(skeleton: &Skeleton, radius: &[f32], half_height: &[f32]) -> Vec<f32> {
    let n = skeleton.len();
    let mut fractions = vec![0.0f32; n];
    let mut sum = 0.0f32;
    let mut rest_volume = 0.0f32;
    for (i, joint) in skeleton.joints.iter().enumerate() {
        match named_fraction(&joint.name) {
            Some(fraction) => {
                fractions[i] = fraction;
                sum += fraction;
            }
            None => {
                let r = radius.get(i).copied().unwrap_or(0.0);
                let h = half_height.get(i).copied().unwrap_or(0.0);
                rest_volume += capsule_volume(r, h);
            }
        }
    }
    if n == 0 {
        return fractions;
    }
    let left = 1.0 - sum;
    if rest_volume > 0.0 {
        for (i, fraction) in fractions.iter_mut().enumerate() {
            if *fraction != 0.0 {
                continue;
            }
            let r = radius.get(i).copied().unwrap_or(0.0);
            let h = half_height.get(i).copied().unwrap_or(0.0);
            *fraction = left * capsule_volume(r, h) / rest_volume;
        }
    } else {
        // Every bone is named: the leftover rides the root, the hips carry
        // the trunk.
        fractions[0] += left;
    }
    fractions
}

/// Volume of one capsule, cubic meters: the cylinder `PI r^2 * 2h` plus the
/// ball `4/3 PI r^3`. The same volume Rapier derives from the shape, so a
/// density over it gives the mass back.
fn capsule_volume(radius: f32, half_height: f32) -> f32 {
    let pi = core::f32::consts::PI;
    pi * radius * radius * 2.0 * half_height + 4.0 / 3.0 * pi * radius * radius * radius
}

/// The coupling seam passes: the floor lift and the neighbor separation
/// that keep a ragdoll's bones from sinking into the floor or into each
/// other. They move Rapier bodies only, so the adapter owns them; the
/// policy (the params and the counters) stays with the ragdoll.
///
/// Divide-and-Truncate pass: floor lift plus neighbor separation, each
/// correction divided into `substeps` equal parts with every part truncated
/// to `max_step_m`. Index loops over caller-owned slices only, so the pass
/// performs no steady-state allocation; math is `+ - * /` plus one `sqrt`
/// per bone pair, all deterministic in skeleton order.
pub(crate) fn resolve_coupling(
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
        // inside its allocation budget (see the ragdoll allocation audit).
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
            // Rapier's user-change lists (see the floor pass and the ragdoll
            // allocation audit).
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

/// One builder for every Rapier world: the solver passes, the bone mass table
/// and the CCD switches. The ragdoll, the powered ragdoll and this adapter
/// read it, so a rig cannot be built with one set of numbers and simulated
/// with another.
///
/// Masses: a bone weighs its fraction of the total, and the fraction table
/// comes from the bone names (the segment masses above). The table must sum
/// to 1 within [`RAGDOLL_MASS_TOLERANCE`]; a table that misses is refused,
/// never silently scaled.
///
/// Determinism: plain values read at spawn, never hashed (the Rapier ragdoll
/// is client-local, HASH-LAW "Ragdoll split").
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollParams {
    /// Solver passes per step.
    solver_passes: usize,
    /// Total body mass, kilograms.
    total_mass_kg: f32,
    /// Mass fraction per bone, skeleton order.
    fractions: Vec<f32>,
}

impl RagdollParams {
    /// The shipped table for one rig: the chosen solver passes, the
    /// [`RAGDOLL_TOTAL_MASS_KG`] total and the mass fractions of its bones.
    /// `radius` and `half_height` are the bone capsules in meters, skeleton
    /// order.
    pub fn for_rig(
        skeleton: &Skeleton,
        radius: &[f32],
        half_height: &[f32],
    ) -> Result<Self, ParamsError> {
        Self::new(
            mass_fractions(skeleton, radius, half_height),
            RAGDOLL_TOTAL_MASS_KG,
            RAGDOLL_SOLVER_PASSES,
        )
    }

    /// The params of a world without bones: the same solver passes and no
    /// mass table. The generic capsules of [`World`] carry no bone names, so
    /// they take no share of the body mass.
    pub fn world() -> Self {
        Self {
            solver_passes: RAGDOLL_SOLVER_PASSES,
            total_mass_kg: RAGDOLL_TOTAL_MASS_KG,
            fractions: Vec::new(),
        }
    }

    /// The builder from a caller table: one fraction per bone, the total mass
    /// and the solver passes. The fractions must sum to 1 within
    /// [`RAGDOLL_MASS_TOLERANCE`]; a table that misses is refused.
    pub fn new(
        fractions: Vec<f32>,
        total_mass_kg: f32,
        solver_passes: usize,
    ) -> Result<Self, ParamsError> {
        if solver_passes == 0 || solver_passes > RAGDOLL_MAX_SOLVER_PASSES {
            return Err(ParamsError::BadSolverPasses);
        }
        if !total_mass_kg.is_finite() || total_mass_kg <= 0.0 {
            return Err(ParamsError::BadTotalMass);
        }
        let mut sum = 0.0f32;
        for (i, fraction) in fractions.iter().enumerate() {
            if !fraction.is_finite() || !(0.0..=1.0).contains(fraction) {
                return Err(ParamsError::BadFraction(i));
            }
            sum += *fraction;
        }
        if (sum - 1.0).abs() > RAGDOLL_MASS_TOLERANCE {
            return Err(ParamsError::MassSum { got: sum });
        }
        Ok(Self {
            solver_passes,
            total_mass_kg,
            fractions,
        })
    }

    /// Solver passes per step.
    pub fn solver_passes(&self) -> usize {
        self.solver_passes
    }

    /// Total body mass, kilograms.
    pub fn total_mass_kg(&self) -> f32 {
        self.total_mass_kg
    }

    /// The mass fractions, one per bone, skeleton order.
    pub fn fractions(&self) -> &[f32] {
        &self.fractions
    }

    /// Bone count the table covers.
    pub fn bones(&self) -> usize {
        self.fractions.len()
    }

    /// The mass of one bone, kilograms. `None` past the end of the table.
    pub fn mass_kg(&self, bone: usize) -> Option<f32> {
        self.fractions.get(bone).map(|f| f * self.total_mass_kg)
    }

    /// The density of one bone, kg/m^3: its mass over the capsule volume.
    /// `None` past the end of the table or for a capsule without volume.
    pub fn density_for(&self, bone: usize, radius: f32, half_height: f32) -> Option<f32> {
        let mass = self.mass_kg(bone)?;
        let volume = capsule_volume(radius, half_height);
        (volume > 0.0).then(|| mass / volume)
    }

    /// The integration parameters at `dt`: the fixed step plus the solver
    /// passes of this table.
    pub fn integration(&self, dt: f32) -> IntegrationParameters {
        IntegrationParameters {
            dt,
            num_solver_iterations: self.solver_passes,
            ..Default::default()
        }
    }

    /// The CCD switch per bone, from the one policy: a dynamic bone rides the
    /// continuous-collision solver from spawn.
    pub fn ccd_flags(&self) -> Vec<bool> {
        let cfg = LodConfig::default();
        (0..self.fractions.len())
            .map(|_| ccd_for(AgentKind::Bone, Motion::Dynamic, 0.0, &cfg) == Ccd::On)
            .collect()
    }
}

/// Kinematic state of one dynamic body. Plain owned data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    /// Center position, meters.
    pub pos: [f32; 3],
    /// Rotation quaternion `(x, y, z, w)`.
    pub rot: [f32; 4],
    /// Linear velocity, m/s.
    pub linvel: [f32; 3],
    /// Angular velocity, rad/s.
    pub angvel: [f32; 3],
}

impl BodyState {
    /// The state as the [`STATE_FLOATS`] words the island snapshot carries.
    fn words(&self) -> [f32; STATE_FLOATS] {
        let [px, py, pz] = self.pos;
        let [rx, ry, rz, rw] = self.rot;
        let [lx, ly, lz] = self.linvel;
        let [ax, ay, az] = self.angvel;
        [px, py, pz, rx, ry, rz, rw, lx, ly, lz, ax, ay, az]
    }

    /// Inverse of [`Self::words`].
    fn from_words(w: &[f32; STATE_FLOATS]) -> Self {
        Self {
            pos: [w[0], w[1], w[2]],
            rot: [w[3], w[4], w[5], w[6]],
            linvel: [w[7], w[8], w[9]],
            angvel: [w[10], w[11], w[12]],
        }
    }
}

/// Owned snapshot of every dynamic body in spawn order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// One entry per dynamic body, spawn order.
    pub bodies: Vec<BodyState>,
}

/// One accepted spawn, kept so a reset can rebuild the world in the same order.
#[derive(Clone, Copy, Debug)]
enum Spawn {
    Capsule {
        pos: [f32; 3],
        radius: f32,
        height: f32,
    },
    Static {
        half: [f32; 3],
        pos: [f32; 3],
    },
}

/// Rapier world: dynamic capsules plus static boxes, fixed-stepped.
pub struct World {
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
    /// Dynamic body handles in spawn order; fixes iteration/hash order.
    order: Vec<RigidBodyHandle>,
    /// Every accepted spawn in order, statics included, to rebuild from.
    spawns: Vec<Spawn>,
}

impl World {
    /// New empty world with Earth gravity and the shared integration
    /// parameters (the fixed step plus the solver passes of the one builder).
    pub fn new() -> Self {
        let params = RagdollParams::world().integration(FIXED_DT);
        Self {
            pipeline: PhysicsPipeline::new(),
            gravity: Vector::new(0.0, GRAVITY_Y, 0.0),
            params,
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            order: Vec::new(),
            spawns: Vec::new(),
        }
    }

    /// Spawn a Y-up dynamic capsule. `height` is the cylinder mid-section
    /// height (total height is `height + 2 * radius`). Returns the body index.
    pub fn spawn_capsule(
        &mut self,
        pos: [f32; 3],
        radius: f32,
        height: f32,
    ) -> Result<usize, Error> {
        check_pos(pos)?;
        if !radius.is_finite() || radius == 0.0 || !(0.0..=MAX_LEN).contains(&radius) {
            return Err(Error::InvalidRadius);
        }
        if !height.is_finite() || !(0.0..=MAX_LEN).contains(&height) {
            return Err(Error::InvalidHeight);
        }
        self.spawns.push(Spawn::Capsule {
            pos,
            radius,
            height,
        });
        Ok(self.insert_capsule(pos, radius, height))
    }

    /// Insert a validated dynamic capsule; returns its body index.
    fn insert_capsule(&mut self, pos: [f32; 3], radius: f32, height: f32) -> usize {
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(pos[0], pos[1], pos[2]))
            // Sleeping is disabled so kinematic state (pos/rot/vel) is the
            // complete future-affecting state and restore is bit-identical.
            .can_sleep(false)
            .build();
        let handle = self.bodies.insert(body);
        let collider = ColliderBuilder::capsule_y(height * 0.5, radius)
            .density(1.0)
            .friction(0.8)
            .restitution(0.0)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);
        self.order.push(handle);
        self.order.len() - 1
    }

    /// Add a static box. Returns the static index (not hashed, never moves).
    pub fn add_static_box(
        &mut self,
        half_extents: [f32; 3],
        pos: [f32; 3],
    ) -> Result<usize, Error> {
        check_pos(pos)?;
        for h in half_extents {
            if !h.is_finite() || h == 0.0 || !(0.0..=MAX_LEN).contains(&h) {
                return Err(Error::InvalidHalfExtents);
            }
        }
        self.spawns.push(Spawn::Static {
            half: half_extents,
            pos,
        });
        Ok(self.insert_static(half_extents, pos))
    }

    /// Insert a validated static box; returns its static index.
    fn insert_static(&mut self, half_extents: [f32; 3], pos: [f32; 3]) -> usize {
        let body = RigidBodyBuilder::fixed()
            .translation(Vector::new(pos[0], pos[1], pos[2]))
            .build();
        let handle = self.bodies.insert(body);
        let collider = ColliderBuilder::cuboid(half_extents[0], half_extents[1], half_extents[2])
            .friction(0.8)
            .restitution(0.0)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);
        self.order.len()
    }

    /// Advance the world by one fixed step. Only finite `dt > 0` is accepted;
    /// callers use [`FIXED_DT`].
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
        Ok(())
    }

    /// Copy the collider and joint outlines into `out` after the tick. Reads
    /// only: no body, collider or solver state changes, so the tick hash cannot
    /// move. Clears `out` first and returns the lines captured.
    pub fn collect_debug(
        &self,
        pipeline: &mut DebugRenderPipeline,
        out: &mut RapierDebugLines,
    ) -> usize {
        out.clear();
        pipeline.render(
            out,
            &self.bodies,
            &self.colliders,
            &self.impulse_joints,
            &self.multibody_joints,
            &self.narrow,
            &self.soft_bodies,
        );
        out.line_count()
    }

    /// Canonical hash over dynamic bodies in spawn order. Each float is
    /// scaled by [`HASH_SCALE`], rounded to `i64` and folded as little-endian
    /// bytes (FNV-1a). No float formatting, no transcendental calls.
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
        h.finish()
    }

    /// Copy out every dynamic body's kinematic state in spawn order.
    pub fn snapshot(&self) -> Snapshot {
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
        Snapshot { bodies }
    }

    /// Restore a snapshot. Counts must match; resuming with the same fixed
    /// `dt` sequence is bit-identical to the recorded run.
    pub fn restore(&mut self, snapshot: Snapshot) -> Result<(), Error> {
        if snapshot.bodies.len() != self.order.len() {
            return Err(Error::SnapshotMismatch);
        }
        for (handle, s) in self.order.iter().zip(snapshot.bodies.iter()) {
            let Some(body) = self.bodies.get_mut(*handle) else {
                return Err(Error::SnapshotMismatch);
            };
            body.set_translation(Vector::new(s.pos[0], s.pos[1], s.pos[2]), false);
            // Rapier snapshots are unit quaternions up to float rounding.
            // Re-normalizing those would move the last bit and break the
            // byte-equal rollback, so only hand-built snapshots that are far
            // off unit length (one `sqrt`, allowed sim math) get normalized.
            // Non-finite components fall back to identity.
            let q = if s.rot.iter().all(|c| c.is_finite()) {
                let raw = Rotation::from_xyzw(s.rot[0], s.rot[1], s.rot[2], s.rot[3]);
                let len2 = s.rot.iter().map(|c| c * c).sum::<f32>();
                if (len2 - 1.0).abs() <= 1e-6 {
                    raw
                } else {
                    raw.normalize()
                }
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
        Ok(())
    }

    /// Snapshot bytes in canonical island order (see
    /// [`islands::island_snapshot`]): `coupled` lists the body pairs in
    /// contact, any order. The length is [`islands::snapshot_budget`], at
    /// most [`islands::FIGHTER_SNAPSHOT_BYTES`] per fighter plus the header.
    ///
    /// A snapshot is a sync point: the world is rebuilt around the same body
    /// states (see `Self::reset_to`), so the live run and a restore of these
    /// bytes step identically. Take snapshots at the same ticks on every peer.
    pub fn island_snapshot(&mut self, coupled: &[(usize, usize)]) -> Vec<u8> {
        let states = self.snapshot();
        let words: Vec<[f32; STATE_FLOATS]> = states.bodies.iter().map(BodyState::words).collect();
        let bytes = islands::island_snapshot(&words, coupled);
        // The body count matches by construction, so the restore cannot fail.
        let _ = self.reset_to(states);
        bytes
    }

    /// Roll every dynamic body back to [`Self::island_snapshot`] bytes, then
    /// rebuild around them like the snapshot did. Atomic: the bytes are fully
    /// parsed and validated first, so a refused snapshot leaves the world as
    /// it was.
    pub fn restore_island_snapshot(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let words = islands::parse_island_snapshot(bytes, self.order.len())
            .map_err(Error::SnapshotBytes)?;
        let bodies = words.iter().map(BodyState::from_words).collect();
        self.reset_to(Snapshot { bodies })
    }

    /// Rebuild the Rapier world from its spawn list, then put the dynamic
    /// bodies at `states`. Rapier keeps state between steps that a snapshot
    /// cannot hold: contact warm-start impulses and frozen lever arms, the
    /// broad-phase pair cache, island ids and active-set membership, CCD
    /// bookkeeping. A rebuilt world holds none of it, so both sides of a
    /// rollback step from the same place.
    fn reset_to(&mut self, states: Snapshot) -> Result<(), Error> {
        let spawns = std::mem::take(&mut self.spawns);
        *self = World::new();
        for spawn in &spawns {
            match *spawn {
                Spawn::Capsule {
                    pos,
                    radius,
                    height,
                } => {
                    self.insert_capsule(pos, radius, height);
                }
                Spawn::Static { half, pos } => {
                    self.insert_static(half, pos);
                }
            }
        }
        self.spawns = spawns;
        self.restore(states)
    }

    /// Number of dynamic bodies.
    pub fn body_count(&self) -> usize {
        self.order.len()
    }

    /// Center position of a dynamic body by index.
    pub fn body_position(&self, id: usize) -> Option<[f32; 3]> {
        ordered_translation(&self.bodies, &self.order, id)
    }

    /// Linear velocity of a dynamic body by index.
    pub fn body_linvel(&self, id: usize) -> Option<[f32; 3]> {
        ordered_linvel(&self.bodies, &self.order, id)
    }

    /// Teleport a dynamic body's center. The character controller applies its
    /// once-per-tick resolved displacement through here so the Rapier mirror
    /// stays in lockstep with the kinematic state. Returns false for an
    /// unknown body or an invalid position; the world is unchanged then.
    pub fn set_body_translation(&mut self, id: usize, pos: [f32; 3]) -> bool {
        if check_pos(pos).is_err() {
            return false;
        }
        let Some(handle) = self.order.get(id).copied() else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        body.set_translation(Vector::new(pos[0], pos[1], pos[2]), true);
        body.reset_forces(true);
        body.wake_up(true);
        true
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// Scale-round a float to `i64`; non-finite maps to a fixed sentinel.
pub(crate) fn quantize(v: f32) -> i64 {
    if !v.is_finite() {
        return i64::MIN;
    }
    let s = f64::from(v) * HASH_SCALE;
    if s >= 9.0e18 {
        i64::MAX
    } else if s <= -9.0e18 {
        i64::MIN + 1
    } else {
        s.round() as i64
    }
}

/// Replace non-finite components with zero so restore never injects NaN.
pub(crate) fn sanitize3(v: [f32; 3]) -> [f32; 3] {
    [
        if v[0].is_finite() { v[0] } else { 0.0 },
        if v[1].is_finite() { v[1] } else { 0.0 },
        if v[2].is_finite() { v[2] } else { 0.0 },
    ]
}

/// True when every component is finite and within [`MAX_LEN`]; the one
/// position gate every adapter in this crate validates through.
pub(crate) fn position_in_bounds(pos: [f32; 3]) -> bool {
    pos.iter().all(|c| c.is_finite() && c.abs() <= MAX_LEN)
}

/// Center of the body at `index` in insertion `order`: the one reader behind
/// every adapter's per-body position query. `None` past the end of `order`
/// or for a handle the set no longer holds.
pub(crate) fn ordered_translation(
    bodies: &RigidBodySet,
    order: &[RigidBodyHandle],
    index: usize,
) -> Option<[f32; 3]> {
    let t = bodies.get(*order.get(index)?)?.translation();
    Some([t.x, t.y, t.z])
}

/// Linear velocity of the body at `index` in insertion `order`, with the same
/// `None` cases as [`ordered_translation`].
pub(crate) fn ordered_linvel(
    bodies: &RigidBodySet,
    order: &[RigidBodyHandle],
    index: usize,
) -> Option<[f32; 3]> {
    let v = bodies.get(*order.get(index)?)?.linvel();
    Some([v.x, v.y, v.z])
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

    fn arena_with_capsule() -> World {
        let mut w = World::new();
        w.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        w.spawn_capsule([0.0, 4.0, 0.0], 0.4, 1.0).expect("capsule");
        w
    }

    fn step_n(w: &mut World, n: usize) {
        for _ in 0..n {
            w.step(FIXED_DT).expect("fixed step");
        }
    }

    #[test]
    fn hash_twice_identical() {
        let mut a = arena_with_capsule();
        let mut b = arena_with_capsule();
        step_n(&mut a, 120);
        step_n(&mut b, 120);
        assert_eq!(a.state_hash(), b.state_hash());
        // Same world hashes stably without stepping.
        assert_eq!(a.state_hash(), a.state_hash());
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn legacy(w: &World) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            for handle in &w.order {
                let Some(body) = w.bodies.get(*handle) else {
                    continue;
                };
                let t = body.translation();
                let r = body.rotation();
                let lv = body.linvel();
                let av = body.angvel();
                for v in [
                    t.x, t.y, t.z, r.x, r.y, r.z, r.w, lv.x, lv.y, lv.z, av.x, av.y, av.z,
                ] {
                    for b in quantize(v).to_le_bytes() {
                        h ^= u64::from(b);
                        h = h.wrapping_mul(FNV64_PRIME);
                    }
                }
            }
            h
        }
        let mut w = arena_with_capsule();
        assert_eq!(w.state_hash(), legacy(&w));
        step_n(&mut w, 120);
        assert_eq!(w.state_hash(), legacy(&w));
    }

    #[test]
    fn snapshot_restore_resimulate_identical() {
        let mut w = arena_with_capsule();
        step_n(&mut w, 60);
        let snap = w.snapshot();
        step_n(&mut w, 90);
        let after = w.state_hash();
        w.restore(snap.clone()).expect("restore");
        // Restored state matches the snapshot exactly.
        assert_eq!(w.snapshot(), snap);
        step_n(&mut w, 90);
        assert_eq!(w.state_hash(), after);
    }

    #[test]
    fn capsule_falls_to_floor_and_rests() {
        let mut w = arena_with_capsule();
        step_n(&mut w, 600);
        let pos = w.body_position(0).expect("body 0");
        let vel = w.body_linvel(0).expect("vel 0");
        // Floor top is y=0; capsule half-height is 0.5 (cylinder) + 0.4 (radius).
        let rest_y = 0.5 + 0.4;
        assert!(
            (pos[1] - rest_y).abs() < 0.1,
            "capsule y={} want {rest_y}",
            pos[1]
        );
        assert!(pos[0].abs() < 0.1 && pos[2].abs() < 0.1, "{pos:?}");
        let speed = vel[0].abs() + vel[1].abs() + vel[2].abs();
        assert!(speed < 0.2, "{vel:?}");
        assert!(pos.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn invalid_dt_rejected() {
        let mut w = arena_with_capsule();
        for bad in [0.0, -FIXED_DT, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(w.step(bad), Err(Error::InvalidDt), "dt={bad}");
        }
        assert!(w.step(FIXED_DT).is_ok());
    }

    #[test]
    fn invalid_spawn_input_rejected_without_panic() {
        let mut w = World::new();
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], 0.0, 1.0),
            Err(Error::InvalidRadius)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], -0.5, 1.0),
            Err(Error::InvalidRadius)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], f32::NAN, 1.0),
            Err(Error::InvalidRadius)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], 0.4, -1.0),
            Err(Error::InvalidHeight)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], 0.4, f32::INFINITY),
            Err(Error::InvalidHeight)
        );
        assert_eq!(
            w.spawn_capsule([0.0, f32::NAN, 0.0], 0.4, 1.0),
            Err(Error::NonFinitePosition)
        );
        assert_eq!(
            w.add_static_box([0.0, 0.5, 0.5], [0.0, 0.0, 0.0]),
            Err(Error::InvalidHalfExtents)
        );
        assert_eq!(
            w.add_static_box([f32::NAN, 0.5, 0.5], [0.0, 0.0, 0.0]),
            Err(Error::InvalidHalfExtents)
        );
        assert_eq!(w.body_count(), 0);
        // Mismatched snapshot restores nothing and reports the mismatch.
        assert_eq!(w.restore(Snapshot::default()), Ok(()));
        w.spawn_capsule([0.0, 2.0, 0.0], 0.4, 1.0).expect("capsule");
        assert_eq!(w.restore(Snapshot::default()), Err(Error::SnapshotMismatch));
        assert_eq!(w.body_position(99), None);
    }

    /// 64 fighters in 32 overlapping pairs on a floor: each pair pushes
    /// itself apart and spins, so rotations and velocities are all live.
    fn sixty_four_fighters() -> World {
        let mut w = World::new();
        w.add_static_box([40.0, 0.5, 40.0], [0.0, -0.5, 0.0])
            .expect("floor");
        for k in 0..32u8 {
            let (col, row) = (f32::from(k % 8), f32::from(k / 8));
            let (x, z) = (4.0 * col - 14.0, 4.0 * row - 6.0);
            w.spawn_capsule([x, 1.2, z], 0.4, 1.0).expect("left");
            let lift = 0.1 * f32::from(k % 3);
            w.spawn_capsule([x + 0.6, 1.2 + lift, z], 0.4, 1.0)
                .expect("right");
        }
        w
    }

    /// Fighters whose centers lie within 1.1 m of each other, ascending.
    fn clinching(w: &World) -> Vec<(usize, usize)> {
        let at: Vec<[f32; 3]> = (0..w.body_count())
            .map(|i| w.body_position(i).expect("body"))
            .collect();
        let mut pairs = Vec::new();
        for (i, a) in at.iter().enumerate() {
            for (j, b) in at.iter().enumerate().skip(i + 1) {
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 1.1 * 1.1 {
                    pairs.push((i, j));
                }
            }
        }
        pairs
    }

    #[test]
    fn snapshot_64_fighters_rollback_is_byte_equal_x2() {
        let mut w = sixty_four_fighters();
        step_n(&mut w, 40);
        let pairs = clinching(&w);
        assert!(pairs.len() >= 32, "pairs clinch: {}", pairs.len());
        let checkpoint = w.island_snapshot(&pairs);
        let islands = crate::islands::build_islands(64, &pairs);
        assert_eq!(
            checkpoint.len(),
            crate::islands::snapshot_budget(64, islands.len())
        );
        assert!(
            checkpoint.len()
                <= crate::islands::SNAPSHOT_HEADER_BYTES
                    + 64 * crate::islands::FIGHTER_SNAPSHOT_BYTES,
            "{} bytes over the budget",
            checkpoint.len()
        );
        step_n(&mut w, 90);
        let (end_hash, end_bytes) = (w.state_hash(), w.island_snapshot(&pairs));
        assert_ne!(end_bytes, checkpoint, "the fighters moved");
        for round in 0..2 {
            w.restore_island_snapshot(&checkpoint).expect("restores");
            assert_eq!(
                w.island_snapshot(&pairs),
                checkpoint,
                "restore then snapshot, round {round}"
            );
            step_n(&mut w, 90);
            assert_eq!(w.state_hash(), end_hash, "resim hash, round {round}");
            assert_eq!(
                w.island_snapshot(&pairs),
                end_bytes,
                "resim bytes, round {round}"
            );
        }
    }

    #[test]
    fn snapshot_island_bytes_refused_leave_the_world_unchanged() {
        let mut w = sixty_four_fighters();
        step_n(&mut w, 30);
        let pairs = clinching(&w);
        let good = w.island_snapshot(&pairs);
        step_n(&mut w, 10);
        let (hash, live) = (w.state_hash(), w.island_snapshot(&pairs));
        let mut nan = good.clone();
        let last = nan.len() - 4;
        nan[last..].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        let mut trailing = good.clone();
        trailing.push(7);
        let mut wrong_world = good.clone();
        wrong_world[4] = 63;
        for (what, bytes) in [
            ("truncated", good[..good.len() - 1].to_vec()),
            ("empty", Vec::new()),
            ("trailing", trailing),
            ("nan", nan),
            ("body count", wrong_world),
        ] {
            let result = w.restore_island_snapshot(&bytes);
            assert!(
                matches!(result, Err(Error::SnapshotBytes(_))),
                "{what}: {result:?}"
            );
            assert_eq!(w.state_hash(), hash, "{what} changed the world");
            assert_eq!(w.island_snapshot(&pairs), live, "{what} changed the bytes");
        }
        assert!(w.restore_island_snapshot(&good).is_ok());
        assert!(Error::SnapshotBytes(SnapshotError::Order)
            .to_string()
            .contains("refused"));
    }

    #[test]
    fn debug_lines_draw_colliders_and_joints_as_named_groups() {
        use rapier3d::dynamics::{GenericJoint, JointAxesMask};

        let mut w = arena_with_capsule();
        w.spawn_capsule([0.9, 3.5, 0.0], 0.35, 0.8)
            .expect("second capsule");
        let (first, second) = (w.order[0], w.order[1]);
        w.impulse_joints.insert(
            first,
            second,
            GenericJoint::new(JointAxesMask::empty()),
            true,
        );
        step_n(&mut w, 10);

        let mut pipeline = debug_render_pipeline();
        let mut lines = RapierDebugLines::new();
        let count = w.collect_debug(&mut pipeline, &mut lines);
        assert!(count > 0, "the world draws lines");
        assert_eq!(lines.groups().len(), 5, "five fixed groups");
        let names: Vec<_> = lines.groups().iter().map(|g| g.name()).collect();
        assert_eq!(names, vec!["colliders", "joints", "sweeps", "hurt", "nav"]);
        let colliders = lines.group(DebugGroup::Colliders);
        assert!(
            colliders.lines().len() >= 12,
            "the box and both capsules draw: {}",
            colliders.lines().len()
        );
        assert!(
            colliders.depth_bias_permille() < 0,
            "colliders sit closer than the beauty frame"
        );
        let joints = lines.group(DebugGroup::Joints);
        assert_eq!(joints.lines().len(), 3, "one joint draws three lines");
        assert!(lines.group(DebugGroup::Sweeps).lines().is_empty());
        assert!(
            lines.groups().iter().flat_map(|g| g.lines()).all(|l| l
                .a_m
                .iter()
                .chain(l.b_m.iter())
                .all(|v| v.is_finite())),
            "every captured coordinate is finite"
        );
    }

    #[test]
    fn debug_lines_leave_the_tick_hash_and_snapshot_unchanged() {
        let mut w = arena_with_capsule();
        step_n(&mut w, 30);
        let (hash, snapshot) = (w.state_hash(), w.snapshot());
        let mut pipeline = debug_render_pipeline();
        let (mut first, mut second) = (RapierDebugLines::new(), RapierDebugLines::new());
        w.collect_debug(&mut pipeline, &mut first);
        w.collect_debug(&mut pipeline, &mut second);
        assert_eq!(first, second, "the same world gives the same lines x2");
        assert_eq!(w.state_hash(), hash, "the copy moves no body");
        assert_eq!(w.snapshot(), snapshot, "the copy moves no snapshot");
        step_n(&mut w, 1);
        let mut after = RapierDebugLines::new();
        w.collect_debug(&mut pipeline, &mut after);
        assert_ne!(first, after, "the next copy is post-tick");
    }

    #[test]
    fn debug_lines_take_post_tick_sweep_hurt_and_nav_copies() {
        let mut lines = RapierDebugLines::new();
        assert!(lines.push(DebugGroup::Sweeps, [0.0; 3], [1.0, 0.0, 0.0]));
        assert!(lines.push(DebugGroup::Hurt, [0.0, 1.0, 0.0], [0.5, 1.0, 0.0]));
        assert!(lines.push(DebugGroup::Nav, [0.0, 0.0, 1.0], [0.0, 0.0, 2.0]));
        assert_eq!(lines.line_count(), 3);
        let sweep = lines.group(DebugGroup::Sweeps);
        assert_eq!(sweep.lines().len(), 1);
        assert_eq!(sweep.colour_rgba(), DebugGroup::Sweeps.colour_rgba());
        assert!(!lines.push(DebugGroup::Hurt, [f32::NAN, 0.0, 0.0], [0.0; 3]));
        assert!(!lines.push(DebugGroup::Nav, [0.0; 3], [0.0, f32::INFINITY, 0.0]));
        assert_eq!(lines.line_count(), 3, "refusals write nothing");
        lines.clear();
        assert_eq!(lines.line_count(), 0, "clear drops every line");
        assert_eq!(lines.groups().len(), 5, "clear keeps the groups");
    }
}
