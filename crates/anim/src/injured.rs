//! injured: single-loss joint masks, support polygon and staged loss gait.
//!
//! Consumer: [`layers::apply_override`](super::layers::apply_override) reads the
//! rebuilt [`MaskTable`] weights (lost joint dropped, stance kept) and
//! [`hit_react::apply_hit_react`](super::hit_react::apply_hit_react) composes the
//! rebuilt table with its own mask through [`compose`]; the double-loss rebuild
//! extends this module's single-loss composition. A loss also picks a
//! [`Gait`] (hop, crawl, fall, one-arm guard) and a [`Staged`] ramp that reaches
//! a legal state inside [`LEGAL_TICKS`]. Owner: `engine-anim`; this
//! module owns [`Loss`], [`Rebuilt`], [`Gait`], [`Staged`] and [`InjuredError`]
//! only, reusing the graph
//! [`Pose`] width plus [`MaskTable`] and the `hit_react` joint ids without
//! mirroring them. Units: mask bytes (0 drops the joint, 255 full drive),
//! meters planar `[x, z]`; seeds are `u64` stream keys. Determinism: validated
//! integers in, `+ - * /` plus comparisons only, joint order, no clock; same loss
//! plus same seed gives the same bytes twice. Errors: [`InjuredError`], never a
//! panic on caller data. Budget: one linear pass over [`FIGHTER_JOINTS`] lanes per
//! rebuild and per staged tick; rebuilds run once per loss event, a staged tick
//! allocates nothing, and only [`Staged::to_mask`] materializes a table, so
//! steady-state ticks allocate nothing.

use super::graph::Pose;
use super::hit_react::{ARM_L, ARM_R, FIGHTER_JOINTS, LEG_L, LEG_R, PELVIS, SPINE};
use super::layers::{MaskTable, MASK_ONE};
use engine_core::{splitmix64, Fnv64};

/// Kept joints always stay driven at least half: a rebuilt mask never shows a
/// T-pose by construction.
pub const STANCE_MIN: u8 = 128;
/// Left-foot ground contact, planar `[x, z]` meters (matches the flinch bind).
const FOOT_L: [f32; 2] = [-0.2, 0.1];
/// Right-foot ground contact, planar `[x, z]` meters (matches the flinch bind).
const FOOT_R: [f32; 2] = [0.2, -0.1];
/// Support test slack, meters: the rebuilt COM sits exactly on the kept
/// contacts, so a small box holds it while a far point fails.
pub const STANCE_R: f32 = 0.05;

/// Which joint a single loss took. One loss only; pairs belong to the
/// double-loss rebuild.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Loss {
    /// Pelvis lost: torso dropped, both feet stay planted.
    Pelvis,
    /// Left leg lost: only the right foot stays planted.
    LegL,
    /// Right leg lost: only the left foot stays planted.
    LegR,
    /// Left arm lost: both feet stay planted.
    ArmL,
    /// Right arm lost: both feet stay planted.
    ArmR,
    /// Spine lost: upper body dropped, both feet stay planted.
    Spine,
}

impl Loss {
    /// Joint index of the lost joint, in the fighter rig order.
    pub fn joint(self) -> usize {
        match self {
            Self::Pelvis => PELVIS,
            Self::LegL => LEG_L,
            Self::LegR => LEG_R,
            Self::ArmL => ARM_L,
            Self::ArmR => ARM_R,
            Self::Spine => SPINE,
        }
    }

    /// Build from a raw joint index; anything past the rig is refused with the
    /// culprit carried and writes nothing.
    pub fn from_joint(joint: usize) -> Result<Self, InjuredError> {
        match joint {
            PELVIS => Ok(Self::Pelvis),
            LEG_L => Ok(Self::LegL),
            LEG_R => Ok(Self::LegR),
            ARM_L => Ok(Self::ArmL),
            ARM_R => Ok(Self::ArmR),
            SPINE => Ok(Self::Spine),
            other => Err(InjuredError::BadJoint(other)),
        }
    }
}

/// What rejected an injured-mask call. Refusals change nothing.
#[derive(Clone, PartialEq, Debug)]
pub enum InjuredError {
    /// Joint past the fighter rig; culprit carried.
    BadJoint(usize),
    /// Not one or two losses; got carried. BX5 owns at most two losses.
    LossCount {
        /// How many losses the caller passed.
        got: usize,
    },
    /// A mask build failed (over-wide rig); source carried.
    Layer(super::graph::GraphError),
}

impl std::fmt::Display for InjuredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadJoint(j) => write!(f, "injured: joint {j} is past the rig"),
            Self::LossCount { got } => {
                write!(f, "injured (BX5): refuses {got} losses, at most two joints")
            }
            Self::Layer(e) => write!(f, "injured layer: {e}"),
        }
    }
}

impl std::error::Error for InjuredError {}

impl From<super::graph::GraphError> for InjuredError {
    fn from(e: super::graph::GraphError) -> Self {
        Self::Layer(e)
    }
}

/// Rebuilt state after one loss: the mask plus the support polygon it stands on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rebuilt {
    mask: MaskTable,
    left_down: bool,
    right_down: bool,
    com_q8: [i16; 2],
}

impl Rebuilt {
    /// Rebuilt mask: lost joint reads 0, every kept joint stays driven.
    pub fn mask(&self) -> &MaskTable {
        &self.mask
    }

    /// Kept foot contacts `[left, right]` after the loss.
    pub fn contacts(&self) -> [bool; 2] {
        [self.left_down, self.right_down]
    }

    /// Center-of-mass projection, planar meters: the midpoint of the kept feet.
    pub fn com(&self) -> [f32; 2] {
        [
            f32::from(self.com_q8[0]) / 256.0,
            f32::from(self.com_q8[1]) / 256.0,
        ]
    }

    /// True while the COM sits inside the kept-feet box (holds by construction).
    pub fn support_holds(&self) -> bool {
        stance_contains(self.left_down, self.right_down, self.com())
    }
}

/// Base drive weight for one kept joint from the seed stream: always at least
/// half drive, so per-seed stance varies without ever going limp.
fn base_weight(seed: u64, joint: usize) -> u8 {
    // Rig joints always fit u64; the default keeps the seed stream total.
    let lane = u64::try_from(joint).unwrap_or_default();
    let mixed = seed.wrapping_add(lane.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let wide = (splitmix64(mixed) >> 33) & 0xFF;
    // Masked to one byte, so this never misses; the default keeps drive total.
    let byte = u8::try_from(wide).unwrap_or_default();
    STANCE_MIN + (byte % STANCE_MIN)
}

/// Q8 of [`FOOT_L`]: `-0.2 * 256` rounds to `-51`, `0.1 * 256` rounds to `26`.
const FOOT_L_Q8: [i16; 2] = [-51, 26];
/// Q8 of [`FOOT_R`]: `0.2 * 256` rounds to `51`, `-0.1 * 256` rounds to `-26`.
const FOOT_R_Q8: [i16; 2] = [51, -26];

/// Fixed-point COM from kept feet: exact midpoint in Q8, no float drift.
fn com_q8(left_down: bool, right_down: bool) -> [i16; 2] {
    let mut xs = [0i16; 2];
    let mut count = 0usize;
    if left_down {
        xs[count] = FOOT_L_Q8[0];
        xs[count + 1] = FOOT_L_Q8[1];
        count += 1;
    }
    if right_down {
        let qx = FOOT_R_Q8[0];
        let qz = FOOT_R_Q8[1];
        if count == 0 {
            xs = [qx, qz];
            count = 1;
        } else {
            xs[0] = (xs[0] + qx) / 2;
            xs[1] = (xs[1] + qz) / 2;
            count = 2;
        }
    }
    debug_assert!(count > 0, "single loss always keeps one foot");
    xs
}

/// Rebuild the mask plus support polygon for one loss at `seed`: the lost joint
/// reads 0, every kept joint keeps its seeded drive, and the COM is the midpoint
/// of the kept feet. Same loss plus same seed gives the same bytes twice.
pub fn rebuild_single(loss: Loss, seed: u64) -> Result<Rebuilt, InjuredError> {
    let lost = loss.joint();
    let mut weights = [MASK_ONE; FIGHTER_JOINTS];
    for (joint, w) in weights.iter_mut().enumerate() {
        if joint != lost {
            *w = base_weight(seed, joint);
        }
    }
    weights[lost] = 0;
    let mask = MaskTable::from_vec(weights.to_vec())?;
    let left_down = loss != Loss::LegL;
    let right_down = loss != Loss::LegR;
    Ok(Rebuilt {
        mask,
        left_down,
        right_down,
        com_q8: com_q8(left_down, right_down),
    })
}

/// Apply one loss event through the rebuild into the layers path.
///
/// `joint` is the lost rig joint, `seed` the stream key; the rebuilt mask
/// drives [`apply_override`](super::layers::apply_override) into caller-owned
/// `out` and the rebuilt state returns for reuse every tick. Same joint plus
/// same seed gives the same bytes twice; a bad joint refuses and writes
/// nothing. Loss routing feeds the saved pair here; the pair-loss rebuild
/// extends this path.
pub fn apply_loss_override(
    base: &Pose,
    over: &Pose,
    joint: usize,
    seed: u64,
    out: &mut Pose,
) -> Result<Rebuilt, InjuredError> {
    let loss = Loss::from_joint(joint)?;
    let rebuilt = rebuild_single(loss, seed)?;
    super::layers::apply_override(base, over, rebuilt.mask(), out)?;
    Ok(rebuilt)
}

/// True while `com` sits inside the kept-feet box plus slack: the point test the
/// rebuilt polygon answers, shared so fixtures and the selector agree.
pub fn stance_contains(left_down: bool, right_down: bool, com: [f32; 2]) -> bool {
    if !com[0].is_finite() || !com[1].is_finite() {
        return false;
    }
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    let mut any = false;
    for (down, foot) in [(left_down, FOOT_L), (right_down, FOOT_R)] {
        if !down {
            continue;
        }
        any = true;
        for axis in 0..2 {
            if foot[axis] < lo[axis] {
                lo[axis] = foot[axis];
            }
            if foot[axis] > hi[axis] {
                hi[axis] = foot[axis];
            }
        }
    }
    if !any {
        return false;
    }
    com[0] >= lo[0] - STANCE_R
        && com[0] <= hi[0] + STANCE_R
        && com[1] >= lo[1] - STANCE_R
        && com[1] <= hi[1] + STANCE_R
}

/// Ticks a fighter has to reach a legal state after a loss: the BX5 bar, and
/// the longest settle a staged gait may take.
pub const LEGAL_TICKS: u8 = 20;
/// Shortest settle: a plan ramps for at least this many ticks before it can
/// count as legal, so a loss never snaps the pose in one frame.
pub const SETTLE_MIN: u8 = 8;
/// Deepest drive a kept joint may take after a loss, per rig joint: past its
/// limit the joint hyperextends, so the staged ramp never passes it. Arms cap
/// lower than legs because a lone arm swings wider than a leg carries weight.
/// Index order matches [`FIGHTER_JOINTS`] and every lane is at least
/// [`STANCE_MIN`], so a legal ramp never shows a T-pose.
pub const JOINT_LIMIT: [u8; FIGHTER_JOINTS] = [255, 240, 240, 216, 216, 232];
/// Grounded gaits bear on the trunk, so the COM must stay inside this planar
/// band, meters `[x, z]`: the hip and shoulder spread of the dropped torso.
const TRUNK_HALF: [f32; 2] = [0.3, 0.2];

/// Locomotion a fighter falls back on after a loss: which limbs still carry it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gait {
    /// Nothing lost: both feet planted, both arms free.
    Stand,
    /// One leg left: the fighter hops on the kept foot.
    Hop,
    /// One arm left: both feet planted, the kept arm holds the guard.
    Guard,
    /// Spine or pelvis lost: the torso drops and the floor bears it.
    Fall,
    /// Both legs lost: the fighter crawls on the kept arms and trunk.
    Crawl,
}

impl Gait {
    /// Gait after one loss: a leg hops, an arm guards, the torso falls.
    pub fn for_loss(loss: Loss) -> Self {
        Self::for_losses(&[loss])
    }

    /// Gait after one or two losses, joint order irrelevant: a torso loss
    /// always drops the body, two lost legs crawl, a lone leg hops, a lone arm
    /// guards, and losing only both arms still stands on both feet.
    pub fn for_losses(losses: &[Loss]) -> Self {
        let mut legs = 0u8;
        let mut arms = 0u8;
        let mut torso = false;
        for loss in losses {
            match loss {
                Loss::LegL | Loss::LegR => legs += 1,
                Loss::ArmL | Loss::ArmR => arms += 1,
                Loss::Pelvis | Loss::Spine => torso = true,
            }
        }
        if torso {
            return Self::Fall;
        }
        match (legs, arms) {
            (0, 0) => Self::Stand,
            (0, 1) => Self::Guard,
            (0, _) => Self::Stand,
            (1, _) => Self::Hop,
            _ => Self::Crawl,
        }
    }

    /// True while the floor bears the body instead of the feet; the COM then
    /// has to stay over the trunk band, not over a foot box.
    pub fn grounded(self) -> bool {
        matches!(self, Self::Fall | Self::Crawl)
    }

    /// Ticks for one locomotion cycle at 60 Hz: a hop is one push and one
    /// landing, a crawl drags both arms through. A fall has no cycle (0).
    pub fn cycle_ticks(self) -> u8 {
        match self {
            Self::Stand => 32,
            Self::Hop => 24,
            Self::Guard => 32,
            Self::Fall => 0,
            Self::Crawl => 40,
        }
    }

    /// Stable byte for digests; never a discriminant cast.
    pub fn code(self) -> u8 {
        match self {
            Self::Stand => 0,
            Self::Hop => 1,
            Self::Guard => 2,
            Self::Fall => 3,
            Self::Crawl => 4,
        }
    }
}

/// True while `gait` can bear `com` on the contacts it kept: upright gaits need
/// the COM inside the foot box, grounded gaits inside the trunk band. Shared so
/// the staged plan and fixtures answer the same question.
pub fn gait_supported(gait: Gait, contacts: [bool; 2], com: [f32; 2]) -> bool {
    if !com[0].is_finite() || !com[1].is_finite() {
        return false;
    }
    if gait.grounded() {
        return com[0].abs() <= TRUNK_HALF[0] && com[1].abs() <= TRUNK_HALF[1];
    }
    stance_contains(contacts[0], contacts[1], com)
}

/// Seeded settle length in `[SETTLE_MIN, LEGAL_TICKS]`: the loss tick never
/// snaps to the gait pose, and every seed lands inside the legal bar.
fn settle_for(seed: u64) -> u8 {
    let span = u64::from(LEGAL_TICKS - SETTLE_MIN) + 1;
    let wide = (splitmix64(seed ^ 0x5E77_1E00_0000_0001) >> 41) % span;
    // Modulo by 13 keeps this under LEGAL_TICKS, so the narrowing never misses.
    SETTLE_MIN + u8::try_from(wide).unwrap_or_default()
}

/// Integer ramp from `start` to `end` over `settle` ticks, inclusive at the end:
/// `end` from tick `settle` on, never past it. `start <= end` by build.
fn ramp(start: u8, end: u8, tick: u8, settle: u8) -> u8 {
    if tick >= settle || settle == 0 {
        return end;
    }
    let span = u16::from(end - start);
    let walked = (span * u16::from(tick)) / u16::from(settle);
    // `walked <= span`, and the sum fits a byte by build.
    start + u8::try_from(walked).unwrap_or_default()
}

/// Staged switch from the standing mask to a loss gait: every tick drives the
/// kept joints further out of the T-pose toward their capped target, and the
/// plan reports legal at or before [`LEGAL_TICKS`]. The ramp is fixed-point
/// bytes, so same losses plus same seed replay the same ticks twice.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Staged {
    gait: Gait,
    lost: [bool; FIGHTER_JOINTS],
    target: [u8; FIGHTER_JOINTS],
    weights: [u8; FIGHTER_JOINTS],
    supported: bool,
    settle_ticks: u8,
    elapsed: u8,
}

impl Staged {
    /// Stage one loss: the rebuilt mask supplies the targets, the gait says how
    /// the fighter carries itself, and the ramp starts at the safe floor.
    pub fn single(loss: Loss, seed: u64) -> Result<Self, InjuredError> {
        let rebuilt = rebuild_single(loss, seed)?;
        Self::from_parts(
            &[loss],
            rebuilt.mask(),
            rebuilt.contacts(),
            rebuilt.com(),
            seed,
        )
    }

    /// Stage one or two losses from an already rebuilt mask: the mask supplies
    /// per-joint targets (capped at [`JOINT_LIMIT`]), the contacts plus COM
    /// answer whether the gait can carry the body. A zero or three-loss call, a
    /// non-finite COM or a wrong-width mask refuses and stages nothing.
    pub fn from_parts(
        losses: &[Loss],
        mask: &MaskTable,
        contacts: [bool; 2],
        com: [f32; 2],
        seed: u64,
    ) -> Result<Self, InjuredError> {
        if losses.is_empty() || losses.len() > 2 {
            return Err(InjuredError::LossCount { got: losses.len() });
        }
        if mask.len() != FIGHTER_JOINTS {
            return Err(InjuredError::Layer(
                super::graph::GraphError::LengthMismatch {
                    expected: FIGHTER_JOINTS,
                    got: mask.len(),
                },
            ));
        }
        let gait = Gait::for_losses(losses);
        let mut lost = [false; FIGHTER_JOINTS];
        for loss in losses {
            lost[loss.joint()] = true;
        }
        let mut target = [0u8; FIGHTER_JOINTS];
        let mut weights = [0u8; FIGHTER_JOINTS];
        for joint in 0..FIGHTER_JOINTS {
            target[joint] = mask.weight(joint).min(JOINT_LIMIT[joint]);
            weights[joint] = if lost[joint] { 0 } else { STANCE_MIN };
        }
        Ok(Self {
            gait,
            lost,
            target,
            weights,
            supported: gait_supported(gait, contacts, com),
            settle_ticks: settle_for(seed),
            elapsed: 0,
        })
    }

    /// Gait this plan switched to.
    pub fn gait(&self) -> Gait {
        self.gait
    }

    /// Ticks ramped so far; never past [`LEGAL_TICKS`].
    pub fn elapsed(&self) -> u8 {
        self.elapsed
    }

    /// Ticks this plan ramps before the pose reaches its gait targets.
    pub fn settle_ticks(&self) -> u8 {
        self.settle_ticks
    }

    /// Joint drive this tick: a lost joint reads 0, a kept joint sits between
    /// [`STANCE_MIN`] and its [`JOINT_LIMIT`] lane.
    pub fn weight(&self, joint: usize) -> u8 {
        match self.weights.get(joint) {
            Some(w) => *w,
            None => 0,
        }
    }

    /// All joint drives this tick, in rig order.
    pub fn weights(&self) -> &[u8; FIGHTER_JOINTS] {
        &self.weights
    }

    /// True while the plan holds a legal state: settled inside the bar, the
    /// gait carries the body, no lost joint drives and no kept joint passes
    /// its limit.
    pub fn is_legal(&self) -> bool {
        if self.elapsed < self.settle_ticks || !self.supported {
            return false;
        }
        self.weights.iter().enumerate().all(|(joint, w)| {
            if self.lost[joint] {
                *w == 0
            } else {
                *w >= STANCE_MIN && *w <= JOINT_LIMIT[joint]
            }
        })
    }

    /// Advance one tick and re-ramp; returns [`Self::is_legal`] for this tick.
    /// Pure integer work: no allocation, no clock.
    pub fn tick(&mut self) -> bool {
        if self.elapsed < LEGAL_TICKS {
            self.elapsed += 1;
        }
        for joint in 0..FIGHTER_JOINTS {
            self.weights[joint] = if self.lost[joint] {
                0
            } else {
                ramp(
                    STANCE_MIN,
                    self.target[joint],
                    self.elapsed,
                    self.settle_ticks,
                )
            };
        }
        self.is_legal()
    }

    /// Tick up to [`LEGAL_TICKS`] and report how many ticks the legal state
    /// took; `None` means the plan never settled (a broken gait or support).
    pub fn run_to_legal(&mut self) -> Option<u8> {
        for _ in 0..LEGAL_TICKS {
            if self.tick() {
                return Some(self.elapsed);
            }
        }
        None
    }

    /// Materialize this tick's drives as a [`MaskTable`] for
    /// [`apply_override`](super::layers::apply_override): one table per call,
    /// built on the loss event and reused while the ramp bytes hold.
    pub fn to_mask(&self) -> Result<MaskTable, InjuredError> {
        Ok(MaskTable::from_vec(self.weights.to_vec())?)
    }
}

/// Stable digest of a staged tick: gait, ramp bytes, elapsed and settle. Same
/// losses plus same seed hash the same at the same tick.
pub fn digest_staged(staged: &Staged) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(u64::from(staged.gait.code()));
    for w in staged.weights {
        h.write_u64(u64::from(w));
    }
    h.write_u64(u64::from(staged.elapsed));
    h.write_u64(u64::from(staged.settle_ticks));
    h.write_bool(staged.supported);
    h.finish()
}

/// Intersect two masks joint-wise (per-joint minimum): how the rebuilt table
/// meets the hit-react mask so a lost joint stays dropped under any layer.
/// Widths must agree; steady-state ticks reuse the output table.
pub fn compose(a: &MaskTable, b: &MaskTable) -> Result<MaskTable, super::graph::GraphError> {
    if a.len() != b.len() {
        return Err(super::graph::GraphError::LengthMismatch {
            expected: a.len(),
            got: b.len(),
        });
    }
    let weights: Vec<u8> = a
        .weights()
        .iter()
        .zip(b.weights().iter())
        .map(|(x, y)| (*x).min(*y))
        .collect();
    MaskTable::from_vec(weights)
}

/// Stable digest of a rebuild: mask bytes, contacts and COM bits. Same loss plus
/// same seed hashes the same; different losses hash apart.
pub fn digest(rebuilt: &Rebuilt) -> u64 {
    let mut h = Fnv64::new();
    // Rig width always fits u64; the default keeps the digest total.
    let len_u64 = u64::try_from(rebuilt.mask.len()).unwrap_or_default();
    h.write_u64(len_u64);
    for w in rebuilt.mask.weights() {
        h.write_u64(u64::from(*w));
    }
    h.write_u64(u64::from(rebuilt.left_down));
    h.write_u64(u64::from(rebuilt.right_down));
    for q in rebuilt.com_q8 {
        h.write_u64(u64::from(u16::from_ne_bytes(q.to_ne_bytes())));
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Pose;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn loss_of(joint: usize) -> Loss {
        Loss::from_joint(joint).expect("fixture joint is on the rig")
    }

    fn bind_pose() -> Pose {
        Pose {
            positions: vec![
                [0.0, 1.0, 0.0],
                [-0.2, 0.5, 0.1],
                [0.2, 0.5, -0.1],
                [-0.3, 1.4, 0.0],
                [0.3, 1.4, 0.0],
                [0.0, 1.2, 0.0],
            ],
            rotations: vec![IDENTITY; FIGHTER_JOINTS],
        }
    }

    #[test]
    fn single_loss_masks_rebuild_x1000() {
        const FIXTURES: [(usize, u64); 15] = [
            (LEG_L, 11),
            (LEG_R, 12),
            (ARM_L, 13),
            (ARM_R, 14),
            (SPINE, 15),
            (PELVIS, 16),
            (LEG_L, 21),
            (LEG_R, 22),
            (ARM_L, 23),
            (ARM_R, 24),
            (SPINE, 25),
            (PELVIS, 26),
            (LEG_L, 31),
            (ARM_R, 32),
            (SPINE, 33),
        ];
        let hit = super::super::hit_react::hit_mask();
        for (joint, seed) in FIXTURES {
            let loss = loss_of(joint);
            let mut distinct = 1u32;
            let mut prev = digest(&rebuild_single(loss, seed).expect("fixture rebuilds"));
            for i in 0..1000u64 {
                let s = seed.wrapping_add(i);
                let a = rebuild_single(loss, s).expect("fixture rebuilds");
                let b = rebuild_single(loss, s).expect("fixture rebuilds");
                assert_eq!(digest(&a), digest(&b), "joint {joint} seed {s}: bytes x2");
                assert_eq!(a, b, "joint {joint} seed {s}: struct x2");
                assert_eq!(a.mask().weight(joint), 0, "lost joint {joint} is dropped");
                for kept in 0..FIGHTER_JOINTS {
                    if kept == joint {
                        continue;
                    }
                    assert!(
                        a.mask().weight(kept) >= STANCE_MIN,
                        "kept joint {kept} stays driven for loss {joint}"
                    );
                }
                assert!(
                    a.com()[0].is_finite() && a.com()[1].is_finite(),
                    "no NaN COM"
                );
                assert!(a.support_holds(), "loss {joint} seed {s}: support holds");
                assert!(
                    !stance_contains(a.contacts()[0], a.contacts()[1], [5.0, 5.0]),
                    "far point fails the polygon"
                );
                // The gait plan settles inside the BX5 bar, and no ramped lane
                // sits over its limit or on a T-pose.
                let mut plan = Staged::single(loss, s).expect("gait plan stages");
                assert_eq!(
                    plan.run_to_legal(),
                    Some(plan.settle_ticks()),
                    "loss {joint} seed {s}: legal inside 20 ticks"
                );
                assert_eq!(plan.gait(), Gait::for_loss(loss));
                for (lane, w) in plan.weights().iter().enumerate() {
                    if lane == joint {
                        assert_eq!(*w, 0, "loss {joint} seed {s}: lost lane stays 0");
                    } else {
                        assert!(
                            *w >= STANCE_MIN && *w <= JOINT_LIMIT[lane],
                            "loss {joint} seed {s}: lane {lane} inside its limit"
                        );
                    }
                }
                assert_eq!(
                    plan.to_mask().expect("staged mask builds").weight(joint),
                    0,
                    "loss {joint} seed {s}: staged mask drops the lane"
                );
                let both = compose(a.mask(), &hit).expect("masks compose");
                assert_eq!(both.weight(joint), 0, "lost joint stays dropped under hit");
                let d = digest(&a);
                if d != prev {
                    distinct += 1;
                    prev = d;
                }
            }
            assert!(distinct >= 2, "joint {joint}: seeds move the bytes");
        }
        assert!(
            !stance_contains(true, true, [f32::NAN, 0.0]),
            "NaN never counts as supported"
        );
    }

    #[test]
    fn single_loss_refuses_bad_joint() {
        assert_eq!(
            Loss::from_joint(FIGHTER_JOINTS),
            Err(InjuredError::BadJoint(FIGHTER_JOINTS))
        );
        assert_eq!(Loss::from_joint(999), Err(InjuredError::BadJoint(999)));
        let full = MaskTable::full(FIGHTER_JOINTS).expect("full mask");
        let narrow = MaskTable::full(3).expect("narrow mask");
        assert!(compose(&full, &narrow).is_err());
    }

    #[test]
    fn rebuilt_mask_drives_layers_apply() {
        let rebuilt = rebuild_single(Loss::ArmL, 7).expect("arm loss rebuilds");
        let base = bind_pose();
        let mut over = base.clone();
        for p in &mut over.positions {
            p[0] += 1.0;
        }
        let mut out = base.clone();
        super::super::layers::apply_override(&base, &over, rebuilt.mask(), &mut out)
            .expect("rebuilt mask feeds the layers path");
        assert_eq!(out.positions[ARM_L], base.positions[ARM_L]);
        assert_ne!(out.positions[ARM_R], base.positions[ARM_R]);
    }

    #[test]
    fn loss_override_wires_rebuild_into_layers() {
        // Fresh seeds the earlier fixtures never used: 99 and 101.
        let base = bind_pose();
        let mut over = base.clone();
        for p in &mut over.positions {
            p[0] += 1.0;
        }
        let mut first = base.clone();
        let rebuilt = apply_loss_override(&base, &over, LEG_L, 99, &mut first)
            .expect("leg loss routes through the layers path");
        assert_eq!(rebuilt.mask().weight(LEG_L), 0);
        assert_eq!(first.positions[LEG_L], base.positions[LEG_L]);
        assert_ne!(first.positions[LEG_R], base.positions[LEG_R]);
        assert!(rebuilt.support_holds());
        let mut second = base.clone();
        let again =
            apply_loss_override(&base, &over, LEG_L, 99, &mut second).expect("same pair replays");
        assert_eq!(digest(&rebuilt), digest(&again));
        assert_eq!(first, second);
        // A different seed moves the kept weights but keeps the lost joint at 0.
        let mut third = base.clone();
        let moved =
            apply_loss_override(&base, &over, LEG_L, 101, &mut third).expect("fresh seed rebuilds");
        assert_eq!(moved.mask().weight(LEG_L), 0);
        assert_ne!(digest(&rebuilt), digest(&moved));
        // Failure case: a bad joint refuses and writes nothing.
        let mut untouched = base.clone();
        assert_eq!(
            apply_loss_override(&base, &over, FIGHTER_JOINTS, 99, &mut untouched).unwrap_err(),
            InjuredError::BadJoint(FIGHTER_JOINTS)
        );
        assert_eq!(untouched, base);
    }

    /// Walk a plan tick by tick and return the ticks it took to turn legal.
    fn walk_to_legal(plan: &mut Staged) -> Option<u8> {
        for tick in 1..=LEGAL_TICKS {
            let legal_now = plan.tick();
            assert_eq!(plan.elapsed(), tick, "elapsed follows the tick");
            if legal_now {
                return Some(tick);
            }
        }
        None
    }

    #[test]
    fn injured_hop_reaches_legal_state_within_20_ticks() {
        // 60 seeds per lost leg: the hop settles inside the bar on every one,
        // then repeats byte for byte on a second plan.
        for (loss, kept) in [(Loss::LegL, 1usize), (Loss::LegR, 0usize)] {
            let lost = loss.joint();
            for seed in 300..360u64 {
                let rebuilt = rebuild_single(loss, seed).expect("leg loss rebuilds");
                assert!(rebuilt.contacts()[kept], "seed {seed}: kept foot down");
                assert!(!rebuilt.contacts()[1 - kept], "seed {seed}: lost leg up");
                let mut hop = Staged::single(loss, seed).expect("hop stages");
                assert_eq!(hop.gait(), Gait::Hop);
                assert!(hop.settle_ticks() >= SETTLE_MIN && hop.settle_ticks() <= LEGAL_TICKS);
                // The loss tick holds the safe floor: no T-pose, no snap.
                assert!(!hop.is_legal(), "seed {seed}: not legal before a tick");
                assert_eq!(hop.weight(lost), 0, "seed {seed}: lost leg stays dropped");
                for lane in 0..FIGHTER_JOINTS {
                    if lane != lost {
                        assert_eq!(hop.weight(lane), STANCE_MIN, "seed {seed}: floor start");
                    }
                }
                let ticks = walk_to_legal(&mut hop);
                assert_eq!(
                    ticks,
                    Some(hop.settle_ticks()),
                    "seed {seed}: legal at settle"
                );
                assert!(hop.is_legal(), "seed {seed}: hop is legal");
                // The ramp finishes inside the first hop cycle, so the push is
                // never taken on a half-ramped pose.
                assert!(hop.settle_ticks() <= Gait::Hop.cycle_ticks());
                for (lane, w) in hop.weights().iter().enumerate() {
                    if lane == lost {
                        assert_eq!(*w, 0, "seed {seed}: lost lane stays 0");
                    } else {
                        assert!(
                            *w >= STANCE_MIN && *w <= JOINT_LIMIT[lane],
                            "seed {seed}: lane {lane} inside its limit"
                        );
                    }
                }
                // The staged table feeds the layers path with the leg dropped.
                let mask = hop.to_mask().expect("staged mask builds");
                assert_eq!(mask.weight(lost), 0);
                let base = bind_pose();
                let mut over = base.clone();
                for p in &mut over.positions {
                    p[0] += 1.0;
                }
                let mut out = base.clone();
                super::super::layers::apply_override(&base, &over, &mask, &mut out)
                    .expect("staged mask drives the layers path");
                assert_eq!(out.positions[lost], base.positions[lost]);
                // Same loss plus same seed replays the same ticks.
                let mut again = Staged::single(loss, seed).expect("hop stages again");
                let _ = walk_to_legal(&mut again);
                assert_eq!(digest_staged(&hop), digest_staged(&again));
                assert_eq!(hop, again);
            }
        }
    }

    #[test]
    fn injured_one_arm_guard_reaches_legal_state_within_20_ticks() {
        let mut capped = 0u32;
        for (loss, kept_arm) in [(Loss::ArmL, ARM_R), (Loss::ArmR, ARM_L)] {
            let lost = loss.joint();
            for seed in 400..460u64 {
                let rebuilt = rebuild_single(loss, seed).expect("arm loss rebuilds");
                assert_eq!(
                    rebuilt.contacts(),
                    [true, true],
                    "both feet carry the guard"
                );
                let mut guard = Staged::single(loss, seed).expect("guard stages");
                assert_eq!(guard.gait(), Gait::Guard);
                assert_eq!(walk_to_legal(&mut guard), Some(guard.settle_ticks()));
                assert!(guard.is_legal(), "seed {seed}: guard is legal");
                assert_eq!(guard.weight(lost), 0, "seed {seed}: lost arm stays dropped");
                let kept = guard.weight(kept_arm);
                assert!(
                    kept >= STANCE_MIN && kept <= JOINT_LIMIT[kept_arm],
                    "seed {seed}: guard arm inside its limit"
                );
                if kept == JOINT_LIMIT[kept_arm] {
                    capped += 1;
                }
            }
        }
        // The arm cap bites: some seed drives the lone arm into its limit.
        assert!(capped > 0, "the guard arm cap is reached by some seed");
    }

    #[test]
    fn injured_torso_loss_falls_to_legal_state_within_20_ticks() {
        for loss in [Loss::Pelvis, Loss::Spine] {
            let lost = loss.joint();
            for seed in 500..560u64 {
                let rebuilt = rebuild_single(loss, seed).expect("torso loss rebuilds");
                let mut fall = Staged::single(loss, seed).expect("fall stages");
                assert_eq!(fall.gait(), Gait::Fall);
                assert!(fall.gait().grounded(), "the floor bears a dropped torso");
                assert!(Gait::Fall.cycle_ticks() == 0, "a fall has no gait cycle");
                assert!(
                    gait_supported(Gait::Fall, rebuilt.contacts(), rebuilt.com()),
                    "seed {seed}: the trunk stays over the band"
                );
                assert_eq!(walk_to_legal(&mut fall), Some(fall.settle_ticks()));
                assert_eq!(fall.weight(lost), 0, "seed {seed}: lost torso lane stays 0");
                assert!(
                    !gait_supported(Gait::Fall, rebuilt.contacts(), [0.9, 0.0]),
                    "a trunk far off the band is never supported"
                );
            }
        }
    }

    #[test]
    fn gait_names_the_limbs_that_still_carry() {
        assert_eq!(Gait::for_loss(Loss::LegL), Gait::Hop);
        assert_eq!(Gait::for_loss(Loss::LegR), Gait::Hop);
        assert_eq!(Gait::for_loss(Loss::ArmL), Gait::Guard);
        assert_eq!(Gait::for_loss(Loss::ArmR), Gait::Guard);
        assert_eq!(Gait::for_loss(Loss::Spine), Gait::Fall);
        assert_eq!(Gait::for_loss(Loss::Pelvis), Gait::Fall);
        assert_eq!(Gait::for_losses(&[]), Gait::Stand);
        assert_eq!(Gait::for_losses(&[Loss::LegL, Loss::LegR]), Gait::Crawl);
        assert_eq!(Gait::for_losses(&[Loss::ArmL, Loss::ArmR]), Gait::Stand);
        assert_eq!(Gait::for_losses(&[Loss::LegL, Loss::ArmL]), Gait::Hop);
        assert_eq!(Gait::for_losses(&[Loss::ArmL, Loss::Spine]), Gait::Fall);
        assert_eq!(Gait::for_losses(&[Loss::Pelvis, Loss::LegR]), Gait::Fall);
        // Every gait but the fall has a cycle, and codes stay stable bytes.
        for (gait, code) in [
            (Gait::Stand, 0u8),
            (Gait::Hop, 1),
            (Gait::Guard, 2),
            (Gait::Fall, 3),
            (Gait::Crawl, 4),
        ] {
            assert_eq!(gait.code(), code);
            assert_eq!(gait.grounded(), cycle_is_grounded(gait));
        }
        assert!(Gait::Hop.cycle_ticks() > 0);
        assert!(Gait::Crawl.cycle_ticks() > Gait::Hop.cycle_ticks());
    }

    fn cycle_is_grounded(gait: Gait) -> bool {
        matches!(gait, Gait::Fall | Gait::Crawl)
    }

    #[test]
    fn staged_refuses_bad_input_names_cause() {
        let mask = rebuild_single(Loss::ArmL, 7)
            .expect("arm loss rebuilds")
            .mask()
            .clone();
        let com = [0.0, 0.0];
        // Zero and three losses are outside BX5's one-or-two world.
        let none: [Loss; 0] = [];
        assert_eq!(
            Staged::from_parts(&none, &mask, [true, true], com, 7).unwrap_err(),
            InjuredError::LossCount { got: 0 }
        );
        assert_eq!(
            Staged::from_parts(
                &[Loss::ArmL, Loss::ArmR, Loss::LegL],
                &mask,
                [true, true],
                com,
                7
            )
            .unwrap_err(),
            InjuredError::LossCount { got: 3 }
        );
        let narrow = MaskTable::full(3).expect("narrow mask");
        assert_eq!(
            Staged::from_parts(&[Loss::ArmL], &narrow, [true, true], com, 7).unwrap_err(),
            InjuredError::Layer(crate::graph::GraphError::LengthMismatch {
                expected: FIGHTER_JOINTS,
                got: 3,
            })
        );
        // A non-finite COM stages but never turns legal, so it cannot fake a
        // supported gait.
        let mut lost_ground =
            Staged::from_parts(&[Loss::ArmL], &mask, [true, true], [f32::NAN, 0.0], 7)
                .expect("a NaN COM stages, it just never holds");
        assert_eq!(lost_ground.run_to_legal(), None);
        assert!(!lost_ground.is_legal());
        assert!(InjuredError::LossCount { got: 3 }.to_string().contains('3'));
        assert!(InjuredError::LossCount { got: 3 }
            .to_string()
            .contains("BX5"));
    }
}
