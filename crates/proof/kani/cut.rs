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

/// Kernel: `edge_reaches` position envelope.
/// Bound: a swing and capsule at `i32::MAX` on the x axis, outside the
/// position envelope.
#[kani::proof]
pub fn cut_edge_reach_rejects_outside_envelope() {
    let outside = [i32::MAX, 0, 0];
    let swing = swing(outside, outside, 0);
    let target = target(outside, 1);
    assert!(!edge_reaches(&swing, &target, &CutTables::v1()));
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
