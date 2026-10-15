//! injured: single-loss joint masks plus support polygon after limb loss.
//!
//! Consumer: [`layers::apply_override`](super::layers::apply_override) reads the
//! rebuilt [`MaskTable`] weights (lost joint dropped, stance kept) and
//! [`hit_react::apply_hit_react`](super::hit_react::apply_hit_react) composes the
//! rebuilt table with its own mask through [`compose`]; the double-loss rebuild
//! extends this module's single-loss composition. Owner: `engine-anim`; this
//! module owns [`Loss`], [`Rebuilt`] and [`InjuredError`] only, reusing the graph
//! [`Pose`] width plus [`MaskTable`] and the `hit_react` joint ids without
//! mirroring them. Units: mask bytes (0 drops the joint, 255 full drive),
//! meters planar `[x, z]`; seeds are `u64` stream keys. Determinism: validated
//! integers in, `+ - * /` plus comparisons only, joint order, no clock; same loss
//! plus same seed gives the same bytes twice. Errors: [`InjuredError`], never a
//! panic on caller data. Budget: one linear pass over [`FIGHTER_JOINTS`] lanes per
//! rebuild; rebuilds run once per loss event and the table is reused every tick,
//! so steady-state ticks allocate nothing.

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
    /// A mask build failed (over-wide rig); source carried.
    Layer(super::graph::GraphError),
}

impl std::fmt::Display for InjuredError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadJoint(j) => write!(f, "injured: joint {j} is past the rig"),
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
}
