//! Kernel: bond load arithmetic in `engine_destruction::bonded`.
//!
//! Loads are Q16 units (`LOAD_Q16_ONE = 65536`); a chunk carries its own
//! mass plus every supported chunk below it. Capacity is a Q16 factor and
//! the overload gate is strict (`load > mass * capacity`). The fixture is
//! the three-chunk chain used by the crate tests: anchored `0`, middle `1`
//! (mass 1), tip `2` (mass 2).

use engine_destruction::{Bond, BondedCollection, BondedDesc, Chunk3, LOAD_Q16_ONE};

fn chain() -> Option<BondedCollection> {
    let desc = BondedDesc {
        chunks: vec![
            Chunk3 {
                x: 0.0,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            },
            Chunk3 {
                x: 1.0,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            },
            Chunk3 {
                x: 2.0,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 2.0,
            },
        ],
        bonds: vec![
            Bond {
                a: 0,
                b: 1,
                breaking_strain: 1.0,
            },
            Bond {
                a: 1,
                b: 2,
                breaking_strain: 1.0,
            },
        ],
        anchors: vec![0],
        break_threshold: 1.0,
    };
    BondedCollection::new(&desc).ok()
}

/// Kernel: support-tree load accumulation.
/// Bound: three chunks, two bonds, masses 1/1/2; every load covers its
/// own mass and every ratio is at least `1.0` Q16.
#[kani::proof]
#[kani::unwind(4)]
pub fn bond_load_covers_own_mass() {
    let Some(field) = chain() else {
        assert!(false, "three-chunk chain is valid");
        return;
    };
    for chunk in 0..3 {
        assert!(field.loads_q16()[chunk] >= field.masses_q16()[chunk]);
        let Some(ratio) = field.load_ratio_q16(chunk) else {
            assert!(false, "loaded chunk has a ratio");
            return;
        };
        assert!(ratio >= LOAD_Q16_ONE);
    }
}

/// Kernel: `ratio_q16`, `weakest_link`, `max_load_ratio_q16`.
/// Bound: the anchored/middle/tip chain; loads 4/3/2 mass units, so the
/// middle link is weakest at ratio 3 and the anchor is excluded.
#[kani::proof]
pub fn bond_weakest_link_is_max_ratio() {
    let Some(field) = chain() else {
        assert!(false, "three-chunk chain is valid");
        return;
    };
    assert_eq!(field.weakest_link(), Some(1));
    assert_eq!(field.max_load_ratio_q16(), 3 * LOAD_Q16_ONE);
    assert_eq!(field.load_ratio_q16(1), Some(3 * LOAD_Q16_ONE));
    assert_eq!(field.load_ratio_q16(2), Some(LOAD_Q16_ONE));
}

/// Kernel: `capacity_q16` off switch.
/// Bound: the chain with capacity `0`; no chunk is overloaded.
#[kani::proof]
#[kani::unwind(4)]
pub fn bond_capacity_zero_never_overloads() {
    let Some(mut field) = chain() else {
        assert!(false, "three-chunk chain is valid");
        return;
    };
    field.set_capacity_q16(0);
    for chunk in 0..3 {
        assert!(!field.is_overloaded(chunk));
    }
}

/// Kernel: `capacity_limit` strict overload threshold.
/// Bound: capacity in `1..=4` mass units on the chain; middle load 3 and
/// tip load 2 over mass 1 and 2, so each overloads exactly while capacity is
/// below its load-over-mass ratio (3 and 1).
#[kani::proof]
pub fn bond_capacity_threshold_is_strict() {
    let capacity: u64 = kani::any();
    kani::assume(capacity >= 1 && capacity <= 4 * LOAD_Q16_ONE);
    let Some(mut field) = chain() else {
        assert!(false, "three-chunk chain is valid");
        return;
    };
    field.set_capacity_q16(capacity);
    assert_eq!(field.is_overloaded(1), capacity < 3 * LOAD_Q16_ONE);
    assert_eq!(field.is_overloaded(2), capacity < LOAD_Q16_ONE);
    assert!(!field.is_overloaded(0), "the anchor is never overloaded");
}

/// Kernel: `set_capacity_q16` replacement contract.
/// Bound: every `u64` capacity; the previous factor is returned and the
/// getter reports the new factor.
#[kani::proof]
pub fn bond_set_capacity_returns_previous() {
    let capacity: u64 = kani::any();
    let Some(mut field) = chain() else {
        assert!(false, "three-chunk chain is valid");
        return;
    };
    assert_eq!(field.set_capacity_q16(capacity), 0);
    assert_eq!(field.capacity_q16(), capacity);
    let next: u64 = kani::any();
    assert_eq!(field.set_capacity_q16(next), capacity);
    assert_eq!(field.capacity_q16(), next);
}
