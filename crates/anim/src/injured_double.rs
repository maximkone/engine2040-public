//! injured_double: double-loss joint masks after two limbs are lost.
//!
//! Consumer: [`super::injured`] single-loss composition is the fallback this
//! module extends; [`rebuild_double`] intersects two single rebuilds through
//! [`super::injured::compose`] and narrows the contacts, then
//! [`apply_double_override`] drives
//! [`apply_override`](super::layers::apply_override) with the rebuilt table.
//! Owner: `engine-anim`; this module owns [`DoubleRebuilt`] and [`DoubleError`]
//! only, reusing the [`Loss`](super::injured::Loss) descriptor plus
//! [`MaskTable`](super::layers::MaskTable) without mirroring them. Units: mask
//! bytes (0 drops the joint, 255 full drive), meters planar `[x, z]`; seeds are
//! `u64` stream keys. Determinism: validated integers in, `+ - * /` plus
//! comparisons only, joint order, no clock; same pair plus same seed gives the
//! same bytes twice. Errors: [`DoubleError`], never a panic on caller data.
//! Budget: two single rebuilds plus one linear mask pass per loss event; the
//! table is reused every tick, so steady-state ticks allocate nothing.

use super::graph::{GraphError, Pose};
use super::injured::{stance_contains, Loss};
use super::layers::{apply_override, MaskTable};
use engine_core::Fnv64;

/// Q8 left-foot contact, mirrors the single-loss bind: `-0.2 * 256` is `-51`.
const FOOT_L_Q8: [i16; 2] = [-51, 26];
/// Q8 right-foot contact, mirrors the single-loss bind: `0.2 * 256` is `51`.
const FOOT_R_Q8: [i16; 2] = [51, -26];

/// What rejected a double-loss call. Refusals change nothing.
#[derive(Clone, PartialEq, Debug)]
pub enum DoubleError {
    /// Joint past the fighter rig; culprit carried.
    BadJoint(usize),
    /// Both losses name the same joint; culprit carried.
    SameJoint(usize),
    /// Not exactly two losses; got carried. BX5 owns at most two losses.
    TooMany {
        /// How many losses the caller passed.
        got: usize,
    },
    /// A mask build failed; source carried.
    Layer(GraphError),
}

impl std::fmt::Display for DoubleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadJoint(j) => write!(f, "injured_double: joint {j} is past the rig"),
            Self::SameJoint(j) => write!(f, "injured_double: joint {j} lost twice"),
            Self::TooMany { got } => write!(
                f,
                "injured_double (BX5): refuses {got} losses, at most two joints"
            ),
            Self::Layer(e) => write!(f, "injured_double layer: {e}"),
        }
    }
}

impl std::error::Error for DoubleError {}

impl From<GraphError> for DoubleError {
    fn from(e: GraphError) -> Self {
        Self::Layer(e)
    }
}

impl From<super::injured::InjuredError> for DoubleError {
    fn from(e: super::injured::InjuredError) -> Self {
        match e {
            super::injured::InjuredError::BadJoint(j) => Self::BadJoint(j),
            super::injured::InjuredError::Layer(source) => Self::Layer(source),
        }
    }
}

/// Rebuilt state after two losses: the mask plus the narrowed support polygon.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DoubleRebuilt {
    mask: MaskTable,
    left_down: bool,
    right_down: bool,
    com_q8: [i16; 2],
}

impl DoubleRebuilt {
    /// Rebuilt mask: both lost joints read 0, every kept joint stays driven.
    pub fn mask(&self) -> &MaskTable {
        &self.mask
    }

    /// Kept foot contacts `[left, right]` after both losses.
    pub fn contacts(&self) -> [bool; 2] {
        [self.left_down, self.right_down]
    }

    /// Center-of-mass projection, planar meters: midpoint of the kept feet.
    pub fn com(&self) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for (axis, slot) in out.iter_mut().enumerate() {
            *slot = f32::from(self.com_q8[axis]) / 256.0;
        }
        out
    }

    /// True while the COM sits inside the kept-feet box (holds by build).
    pub fn support_holds(&self) -> bool {
        stance_contains(self.left_down, self.right_down, self.com())
    }
}

/// Fixed-point COM from kept feet: integer midpoint in Q8, no float drift.
/// Empty stance (both legs lost) reads the origin; support then fails loudly.
#[expect(
    clippy::manual_unwrap_or_default,
    reason = "explicit zero keeps the COM total on wide targets"
)]
fn com_q8_for(left_down: bool, right_down: bool) -> [i16; 2] {
    let mut total_x: i32 = 0;
    let mut total_z: i32 = 0;
    let mut kept: i32 = 0;
    if left_down {
        total_x += i32::from(FOOT_L_Q8[0]);
        total_z += i32::from(FOOT_L_Q8[1]);
        kept += 1;
    }
    if right_down {
        total_x += i32::from(FOOT_R_Q8[0]);
        total_z += i32::from(FOOT_R_Q8[1]);
        kept += 1;
    }
    if kept == 0 {
        return [0, 0];
    }
    let mid_x = total_x / kept;
    let mid_z = total_z / kept;
    // Foot Q8 lanes plus their midpoint always fit i16 by build.
    let x = match i16::try_from(mid_x) {
        Ok(v) => v,
        Err(_) => 0,
    };
    let z = match i16::try_from(mid_z) {
        Ok(v) => v,
        Err(_) => 0,
    };
    [x, z]
}

/// Rebuild the mask plus narrowed polygon for two losses at `seed`: both lost
/// joints read 0, kept joints keep their seeded drive, contacts narrow to feet
/// both singles keep, and the COM is the midpoint of the kept feet. Same pair
/// plus same seed gives the same bytes twice.
pub fn rebuild_double(first: Loss, second: Loss, seed: u64) -> Result<DoubleRebuilt, DoubleError> {
    if first.joint() == second.joint() {
        return Err(DoubleError::SameJoint(first.joint()));
    }
    let a = super::injured::rebuild_single(first, seed)?;
    let b = super::injured::rebuild_single(second, seed)?;
    let mask = super::injured::compose(a.mask(), b.mask())?;
    let kept = a.contacts();
    let other = b.contacts();
    let left_down = kept[0] && other[0];
    let right_down = kept[1] && other[1];
    Ok(DoubleRebuilt {
        mask,
        left_down,
        right_down,
        com_q8: com_q8_for(left_down, right_down),
    })
}

/// Rebuild from a loss slice: exactly two losses rebuild, anything else is
/// refused loudly naming BX5. The triple-loss gate lives here.
pub fn rebuild_many(losses: &[Loss], seed: u64) -> Result<DoubleRebuilt, DoubleError> {
    match losses {
        [first, second] => rebuild_double(*first, *second, seed),
        _ => Err(DoubleError::TooMany { got: losses.len() }),
    }
}

/// Rebuild from raw joint ids; a past-the-rig id refuses and writes nothing.
pub fn rebuild_double_from_joints(
    first: usize,
    second: usize,
    seed: u64,
) -> Result<DoubleRebuilt, DoubleError> {
    let a = Loss::from_joint(first)?;
    let b = Loss::from_joint(second)?;
    rebuild_double(a, b, seed)
}

/// Apply a two-loss event through the rebuild into the layers path.
///
/// `first`/`second` are lost rig joints, `seed` the stream key; the rebuilt
/// mask drives [`apply_override`] into caller-owned `out` and the rebuilt
/// state returns for reuse every tick. Same pair plus same seed gives the same
/// bytes twice; a bad pair refuses and writes nothing.
pub fn apply_double_override(
    base: &Pose,
    over: &Pose,
    first: usize,
    second: usize,
    seed: u64,
    out: &mut Pose,
) -> Result<DoubleRebuilt, DoubleError> {
    let rebuilt = rebuild_double_from_joints(first, second, seed)?;
    apply_override(base, over, rebuilt.mask(), out)?;
    Ok(rebuilt)
}

#[expect(
    clippy::manual_unwrap_or_default,
    reason = "explicit zero keeps digest total"
)]
/// Stable digest of a double rebuild: mask bytes, contacts and COM bits. Same
/// pair plus same seed hashes the same; different pairs hash apart.
pub fn digest(rebuilt: &DoubleRebuilt) -> u64 {
    let mut h = Fnv64::new();
    // Rig width always fits u64; fallback keeps the digest total.
    let len_u64 = match u64::try_from(rebuilt.mask.len()) {
        Ok(v) => v,
        Err(_) => 0,
    };
    h.write_u64(len_u64);
    for w in rebuilt.mask.weights() {
        h.write_u8(*w);
    }
    h.write_bool(rebuilt.left_down);
    h.write_bool(rebuilt.right_down);
    for q in rebuilt.com_q8 {
        h.write(&q.to_le_bytes());
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::super::hit_react::{ARM_L, ARM_R, FIGHTER_JOINTS, LEG_L, LEG_R, PELVIS, SPINE};
    use super::*;

    const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn fighter_base() -> Pose {
        let mut pose = Pose::zero(FIGHTER_JOINTS);
        let feet = [
            [0.0, 1.0, 0.0],
            [-0.2, 0.5, 0.1],
            [0.2, 0.5, -0.1],
            [-0.3, 1.4, 0.0],
            [0.3, 1.4, 0.0],
            [0.0, 1.2, 0.0],
        ];
        for (slot, spot) in feet.iter().enumerate() {
            pose.positions[slot] = *spot;
            pose.rotations[slot] = IDENTITY;
        }
        pose
    }

    #[test]
    fn double_loss_masks_rebuild_x1000() {
        const FIXTURES: [(Loss, Loss, u64); 6] = [
            (Loss::LegL, Loss::ArmL, 101),
            (Loss::LegR, Loss::ArmR, 102),
            (Loss::LegL, Loss::ArmR, 103),
            (Loss::LegR, Loss::ArmL, 104),
            (Loss::ArmL, Loss::ArmR, 105),
            (Loss::Spine, Loss::Pelvis, 106),
        ];
        let hit = super::super::hit_react::hit_mask();
        for (first, second, seed) in FIXTURES {
            let lost_a = first.joint();
            let lost_b = second.joint();
            let mut distinct = 1u32;
            let mut prev = digest(&rebuild_double(first, second, seed).expect("fixture rebuilds"));
            for i in 0..1000u64 {
                let s = seed.wrapping_add(i);
                let a = rebuild_double(first, second, s).expect("fixture rebuilds");
                let b = rebuild_double(first, second, s).expect("fixture rebuilds");
                assert_eq!(digest(&a), digest(&b), "seed {s}: bytes x2");
                assert_eq!(a, b, "seed {s}: struct x2");
                assert_eq!(a.mask().weight(lost_a), 0, "first loss is dropped");
                assert_eq!(a.mask().weight(lost_b), 0, "second loss is dropped");
                for kept in 0..FIGHTER_JOINTS {
                    if kept == lost_a || kept == lost_b {
                        continue;
                    }
                    assert!(
                        a.mask().weight(kept) >= super::super::injured::STANCE_MIN,
                        "kept joint {kept} stays driven"
                    );
                }
                assert!(
                    a.com()[0].is_finite() && a.com()[1].is_finite(),
                    "no NaN COM"
                );
                assert!(a.support_holds(), "seed {s}: support holds");
                assert!(
                    !stance_contains(a.contacts()[0], a.contacts()[1], [5.0, 5.0]),
                    "far point fails the polygon"
                );
                let both = super::super::injured::compose(a.mask(), &hit).expect("masks compose");
                assert_eq!(both.weight(lost_a), 0, "first stays dropped under hit");
                assert_eq!(both.weight(lost_b), 0, "second stays dropped under hit");
                let d = digest(&a);
                if d != prev {
                    distinct += 1;
                    prev = d;
                }
            }
            assert!(distinct >= 2, "seeds move the bytes");
        }
        assert!(
            !stance_contains(true, true, [f32::NAN, 0.0]),
            "NaN never counts as supported"
        );
        let triple = [Loss::LegL, Loss::LegR, Loss::ArmL];
        let refused = rebuild_many(&triple, 7).unwrap_err();
        assert_eq!(refused, DoubleError::TooMany { got: 3 });
        assert!(refused.to_string().contains("BX5"));
    }

    #[test]
    fn double_refuses_bad_input_names_cause() {
        assert_eq!(
            rebuild_double(Loss::LegL, Loss::LegL, 3).unwrap_err(),
            DoubleError::SameJoint(LEG_L)
        );
        assert_eq!(
            rebuild_double_from_joints(FIGHTER_JOINTS, ARM_L, 3).unwrap_err(),
            DoubleError::BadJoint(FIGHTER_JOINTS)
        );
        assert_eq!(
            rebuild_double_from_joints(LEG_L, 999, 3).unwrap_err(),
            DoubleError::BadJoint(999)
        );
        let triple = [Loss::ArmL, Loss::ArmR, Loss::Spine];
        let err = rebuild_many(&triple, 3).unwrap_err();
        assert!(err.to_string().contains("BX5"));
        assert!(err.to_string().contains('3'));
        let single = [Loss::ArmL];
        assert!(rebuild_many(&single, 3)
            .unwrap_err()
            .to_string()
            .contains("BX5"));
        let full = MaskTable::full(FIGHTER_JOINTS).expect("full mask");
        let narrow = MaskTable::full(3).expect("narrow mask");
        assert!(super::super::injured::compose(&full, &narrow).is_err());
        assert!(DoubleError::BadJoint(9).to_string().contains('9'));
        assert!(DoubleError::SameJoint(LEG_R)
            .to_string()
            .contains(&LEG_R.to_string()));
    }

    #[test]
    fn double_override_wires_rebuild_into_layers() {
        let base = fighter_base();
        let mut over = base.clone();
        for p in &mut over.positions {
            p[0] += 1.0;
        }
        let mut first = base.clone();
        let rebuilt = apply_double_override(&base, &over, LEG_L, ARM_L, 77, &mut first)
            .expect("leg plus arm routes through the layers path");
        assert_eq!(rebuilt.mask().weight(LEG_L), 0);
        assert_eq!(rebuilt.mask().weight(ARM_L), 0);
        assert_eq!(first.positions[LEG_L], base.positions[LEG_L]);
        assert_eq!(first.positions[ARM_L], base.positions[ARM_L]);
        assert_ne!(first.positions[LEG_R], base.positions[LEG_R]);
        assert!(rebuilt.support_holds());
        let mut second = base.clone();
        let again = apply_double_override(&base, &over, LEG_L, ARM_L, 77, &mut second)
            .expect("same pair replays");
        assert_eq!(digest(&rebuilt), digest(&again));
        assert_eq!(first, second);
        let mut third = base.clone();
        let moved = apply_double_override(&base, &over, LEG_L, ARM_L, 79, &mut third)
            .expect("fresh seed rebuilds");
        assert_eq!(moved.mask().weight(LEG_L), 0);
        assert_eq!(moved.mask().weight(ARM_L), 0);
        assert_ne!(digest(&rebuilt), digest(&moved));
        let mut untouched = base.clone();
        assert_eq!(
            apply_double_override(&base, &over, FIGHTER_JOINTS, ARM_L, 77, &mut untouched)
                .unwrap_err(),
            DoubleError::BadJoint(FIGHTER_JOINTS)
        );
        assert_eq!(untouched, base);
        let mut same = base.clone();
        assert_eq!(
            apply_double_override(&base, &over, ARM_R, ARM_R, 77, &mut same).unwrap_err(),
            DoubleError::SameJoint(ARM_R)
        );
        assert_eq!(same, base);
        assert_eq!(PELVIS, 0);
        assert_eq!(SPINE, 5);
    }
}
