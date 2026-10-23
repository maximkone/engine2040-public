//! Kernel: cut-plane quantization in `engine_combat::cut`.
//!
//! The integer kernel is the contact solve behind `edge_reaches`: segment
//! parameters are quantized to `CUT_Q = 1024`, energy and odds in Q16, and
//! the kernel envelope is `CUT_POS_BOUND_MM`. The contact harnesses use a
//! tighter `+/-12_000` mm rectangle so the kernel's i64 dot products cannot
//! overflow. The public CSG `CutPlane` path is float-based and deliberately
//! is not a Kani kernel.

use engine_combat::anatomy::JointId;
use engine_combat::cut::{
    chance_q16, edge_reaches, effective_q16, BladeSwing, CutTables, JointTarget, MAX_ENERGY_Q16,
    MAX_SHARPNESS_Q16, MAX_WEAR_Q16,
};

fn any_i32x3() -> [i32; 3] {
    [kani::any(), kani::any(), kani::any()]
}

fn swing(p0: [i32; 3], p1: [i32; 3], blade_half_mm: u32) -> BladeSwing {
    BladeSwing {
        attacker: 0,
        victim: 1,
        swing: 7,
        p0_mm: p0,
        p1_mm: p1,
        blade_half_mm,
        energy_q16: 65_536,
        sharpness_q16: 65_536,
        wear_q16: 0,
    }
}

fn target(spine: [i32; 3], radius_mm: u32) -> JointTarget {
    JointTarget {
        joint: JointId::ElbowR,
        q0_mm: spine,
        q1_mm: spine,
        radius_mm,
    }
}

/// Kernel: `edge_reaches` contact gate (envelope, travel, capsule).
/// Bound: both points and both capsule ends inside `+/-12_000` mm (wider
/// than the 6_000 mm travel bound), blade half and capsule radius inside
/// `0..=500` mm.
#[kani::proof]
#[kani::unwind(4)]
pub fn cut_edge_reach_is_deterministic() {
    const COORD: i32 = 12_000;
    let p0 = any_i32x3();
    let p1 = any_i32x3();
    let q0 = any_i32x3();
    let q1 = any_i32x3();
    let blade_half: u32 = kani::any();
    let radius: u32 = kani::any();
    for axis in 0..3 {
        kani::assume((-COORD..=COORD).contains(&p0[axis]));
        kani::assume((-COORD..=COORD).contains(&p1[axis]));
        kani::assume((-COORD..=COORD).contains(&q0[axis]));
        kani::assume((-COORD..=COORD).contains(&q1[axis]));
    }
    kani::assume(blade_half <= 500);
    kani::assume(radius <= 500);
    let swing = swing(p0, p1, blade_half);
    let target = JointTarget {
        joint: JointId::ElbowR,
        q0_mm: q0,
        q1_mm: q1,
        radius_mm: radius,
    };
    let tables = CutTables::v1();
    assert_eq!(
        edge_reaches(&swing, &target, &tables),
        edge_reaches(&swing, &target, &tables)
    );
}

/// Kernel: `edge_reaches` single-swing travel bound.
/// Bound: a 6001 mm swing on a table that allows 6000 mm; capsule on the
/// far endpoint, radius 1 mm, inside the position envelope.
#[kani::proof]
pub fn cut_edge_reach_rejects_travel_past_bound() {
    let far = [6_001, 0, 0];
    let swing = swing([0, 0, 0], far, 0);
    let target = target(far, 1);
    assert!(!edge_reaches(&swing, &target, &CutTables::v1()));
}

/// Kernel: `edge_reaches` zero-length self contact.
/// Bound: blade point and capsule point coincide at the origin, radius
/// 1 mm, blade half 0 mm, travel 0 mm.
#[kani::proof]
pub fn cut_edge_reach_finds_coincident_geometry() {
    let origin = [0, 0, 0];
    let swing = swing(origin, origin, 0);
    let target = target(origin, 1);
    assert!(edge_reaches(&swing, &target, &CutTables::v1()));
}

/// Kernel: `effective_q16` energy quantization.
/// Bound: every validated V1 swing field (`energy <= 2097152`,
/// `sharpness <= 131072`, `wear <= 65536`); result at most 4194304.
#[kani::proof]
pub fn cut_effective_energy_stays_in_table_bound() {
    let energy: u32 = kani::any();
    let sharpness: u32 = kani::any();
    let wear: u32 = kani::any();
    kani::assume(energy <= MAX_ENERGY_Q16);
    kani::assume(sharpness <= MAX_SHARPNESS_Q16);
    kani::assume(wear <= MAX_WEAR_Q16);
    let mut swing = swing([0, 0, 0], [0, 0, 0], 0);
    swing.energy_q16 = energy;
    swing.sharpness_q16 = sharpness;
    swing.wear_q16 = wear;
    let bound = u64::from(MAX_ENERGY_Q16) * u64::from(MAX_SHARPNESS_Q16) / 65_536;
    assert!(effective_q16(&swing) <= bound);
}

/// Kernel: `chance_q16` sever ramp.
/// Bound: every `u64` energy and protection, every validated V1 span
/// (`65536..=524288` Q16); the odds stay in `0..=65536`.
#[kani::proof]
pub fn cut_chance_odds_stay_in_q16_range() {
    let effective: u64 = kani::any();
    let protection: u64 = kani::any();
    let span: u32 = kani::any();
    kani::assume((65_536..=524_288).contains(&span));
    assert!(chance_q16(effective, protection, span) <= 65_536);
}
