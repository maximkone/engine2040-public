//! HS23 fallback proof tests.
//!
//! Kani is not installed on the shared PC, so these tests mirror the
//! authored harness properties on deterministic grids and keep the harness
//! inventory honest. The Kani runs themselves stay an open gate.

use super::*;
use crate::tape::MAX_TAPE_BYTES;
use engine_combat::anatomy::JointId;
use engine_combat::cut::{
    chance_q16, edge_reaches, effective_q16, roll_q16, saturate_q16, BladeSwing, CutTables,
    JointTarget, MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, MAX_WEAR_Q16,
};
use engine_core::fixed::Fixed;
use engine_destruction::{Bond, BondedCollection, BondedDesc, Chunk3, LOAD_Q16_ONE};

const HARNESS_FILES: [(&str, &str); 4] = [
    ("q16", include_str!("../kani/q16.rs")),
    ("cut", include_str!("../kani/cut.rs")),
    ("bond", include_str!("../kani/bond.rs")),
    ("tape", include_str!("../kani/tape.rs")),
];

fn harness_docs(src: &str) -> Vec<Vec<&str>> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() != "#[kani::proof]" {
            continue;
        }
        let mut docs = Vec::new();
        let mut j = i;
        while j > 0 {
            let prev = lines[j - 1].trim();
            if let Some(rest) = prev.strip_prefix("///") {
                docs.push(rest);
                j -= 1;
            } else if prev.starts_with("#[") {
                j -= 1;
            } else {
                break;
            }
        }
        out.push(docs);
    }
    out
}

#[test]
fn hs23_has_five_harnesses_per_kernel() {
    let mut total = 0;
    for (name, src) in HARNESS_FILES {
        let count = harness_docs(src).len();
        assert_eq!(count, 5, "kernel {name} must have five harnesses");
        total += count;
    }
    assert_eq!(total, 20);
}

#[test]
fn hs23_every_harness_names_kernel_and_bound() {
    for (name, src) in HARNESS_FILES {
        for (index, docs) in harness_docs(src).into_iter().enumerate() {
            assert!(
                docs.iter()
                    .any(|line| line.trim_start().starts_with("Kernel:")),
                "{name} harness {index} has no Kernel line"
            );
            assert!(
                docs.iter()
                    .any(|line| line.trim_start().starts_with("Bound:")),
                "{name} harness {index} has no Bound line"
            );
        }
    }
}

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn round_q16(p: i128) -> i128 {
    const HALF: i128 = 1 << 15;
    if p >= 0 {
        (p + HALF) >> 16
    } else {
        -((HALF - p) >> 16)
    }
}

#[test]
fn hs23_q16_properties_hold_on_a_seeded_grid() {
    let mut state = 0xC0FF_EE01_u64;
    for _ in 0..2_000 {
        let a = lcg(&mut state) as i64;
        let b = lcg(&mut state) as i64;
        let fa = Fixed::from_bits(a);
        let fb = Fixed::from_bits(b);
        let want_mul = i64::try_from(round_q16(i128::from(a) * i128::from(b))).ok();
        assert_eq!(fa.checked_mul(fb).map(Fixed::to_bits), want_mul);
        let divisor = b | 1;
        let want_div = i64::try_from((i128::from(a) << 16) / i128::from(divisor)).ok();
        assert_eq!(
            fa.checked_div(Fixed::from_bits(divisor))
                .map(Fixed::to_bits),
            want_div
        );

        let raw = (lcg(&mut state) >> 1) as i64;
        let root = Fixed::from_bits(raw).sqrt().to_bits().unsigned_abs();
        let n = u128::try_from(raw).expect("positive") * 65_536;
        let r = u128::from(root);
        assert!(r * r <= n);
        assert!((r + 1) * (r + 1) > n);

        let t = (lcg(&mut state) % 131_073) as i64 - 65_536;
        let blend = fa.lerp(fb, Fixed::from_bits(t));
        if t <= 0 {
            assert_eq!(blend.to_bits(), a);
        }
        if t >= Fixed::ONE.to_bits() {
            assert_eq!(blend.to_bits(), b);
        }

        let scalar = lcg(&mut state) as i64;
        assert_eq!(
            fa.reference_cmp_scalar(scalar),
            i128::from(a).cmp(&(i128::from(scalar) * i128::from(Fixed::SCALE)))
        );
    }
}

fn coord(state: &mut u64) -> i32 {
    i32::try_from(lcg(state) % 400_001).expect("fits i32") - 200_000
}

fn probe_swing(state: &mut u64) -> BladeSwing {
    BladeSwing {
        attacker: 0,
        victim: 1,
        swing: lcg(state) as u32,
        p0_mm: [coord(state), coord(state), coord(state)],
        p1_mm: [coord(state), coord(state), coord(state)],
        blade_half_mm: (lcg(state) % 501) as u32,
        energy_q16: (lcg(state) % u64::from(MAX_ENERGY_Q16 + 1)) as u32,
        sharpness_q16: (lcg(state) % u64::from(MAX_SHARPNESS_Q16 + 1)) as u32,
        wear_q16: (lcg(state) % u64::from(MAX_WEAR_Q16 + 1)) as u32,
    }
}

#[test]
fn hs23_cut_properties_hold_on_a_seeded_grid() {
    let tables = CutTables::v1();
    let mut state = 0xC07_0A11_u64;
    let max_effective = u64::from(MAX_ENERGY_Q16) * u64::from(MAX_SHARPNESS_Q16) / 65_536;
    for _ in 0..2_000 {
        let swing = probe_swing(&mut state);
        let target = JointTarget {
            joint: JointId::ElbowR,
            q0_mm: [coord(&mut state), coord(&mut state), coord(&mut state)],
            q1_mm: [coord(&mut state), coord(&mut state), coord(&mut state)],
            radius_mm: (lcg(&mut state) % 501) as u32,
        };
        let first = edge_reaches(&swing, &target, &tables);
        assert_eq!(first, edge_reaches(&swing, &target, &tables));
        assert!(effective_q16(&swing) <= max_effective);
        assert!(chance_q16(effective_q16(&swing), lcg(&mut state), 262_144) <= 65_536);
        assert_eq!(
            roll_q16(&swing, JointId::ElbowR),
            roll_q16(&swing, JointId::ElbowR)
        );
        assert!(roll_q16(&swing, JointId::ElbowR) <= 65_535);
        assert_eq!(saturate_q16(u64::MAX), u32::MAX);
        assert_eq!(saturate_q16(7), 7);
    }

    let origin = [0, 0, 0];
    let contact = BladeSwing {
        p0_mm: origin,
        p1_mm: origin,
        blade_half_mm: 0,
        energy_q16: 65_536,
        sharpness_q16: 65_536,
        wear_q16: 0,
        attacker: 0,
        victim: 1,
        swing: 1,
    };
    let capsule = JointTarget {
        joint: JointId::ElbowR,
        q0_mm: origin,
        q1_mm: origin,
        radius_mm: 1,
    };
    assert!(edge_reaches(&contact, &capsule, &tables));

    let far = [6_001, 0, 0];
    let travel = BladeSwing {
        p0_mm: origin,
        p1_mm: far,
        ..contact
    };
    let far_capsule = JointTarget {
        q0_mm: far,
        q1_mm: far,
        ..capsule
    };
    assert!(!edge_reaches(&travel, &far_capsule, &tables));
}

fn chain() -> BondedCollection {
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
    BondedCollection::new(&desc).expect("three-chunk chain is valid")
}

#[test]
fn hs23_bond_properties_hold_on_chain() {
    let mut field = chain();
    assert_eq!(
        field.loads_q16(),
        &[4 * LOAD_Q16_ONE, 3 * LOAD_Q16_ONE, 2 * LOAD_Q16_ONE]
    );
    assert_eq!(
        field.masses_q16(),
        &[LOAD_Q16_ONE, LOAD_Q16_ONE, 2 * LOAD_Q16_ONE]
    );
    assert_eq!(field.weakest_link(), Some(1));
    assert_eq!(field.max_load_ratio_q16(), 3 * LOAD_Q16_ONE);
    assert_eq!(field.load_ratio_q16(1), Some(3 * LOAD_Q16_ONE));
    assert_eq!(field.load_ratio_q16(2), Some(LOAD_Q16_ONE));

    assert_eq!(field.set_capacity_q16(0), 0);
    assert!(!field.is_overloaded(1));
    assert!(!field.is_overloaded(2));

    assert_eq!(field.set_capacity_q16(3 * LOAD_Q16_ONE - 1), 0);
    assert!(field.is_overloaded(1));
    assert!(!field.is_overloaded(2));
    assert!(!field.is_overloaded(0), "the anchor is never overloaded");

    assert_eq!(
        field.set_capacity_q16(3 * LOAD_Q16_ONE),
        3 * LOAD_Q16_ONE - 1
    );
    assert!(!field.is_overloaded(1));
    field.set_capacity_q16(LOAD_Q16_ONE - 1);
    assert!(field.is_overloaded(2));
    field.set_capacity_q16(LOAD_Q16_ONE);
    assert!(!field.is_overloaded(2));
}

fn valid_tape() -> Tape {
    Tape {
        config: Config {
            seed: 2040,
            ticks: 1,
        },
        inputs: vec![Input {
            tick: 0,
            body: 7,
            vx: 1.5,
            vy: -0.25,
        }],
        checkpoints: vec![Checkpoint {
            legacy: 0x1234,
            state: 0x5678,
        }],
    }
}

#[test]
fn hs23_tape_decode_roundtrips_and_rejects_corruption() {
    let tape = valid_tape();
    let bytes = tape.encode().expect("valid tape encodes");
    assert_eq!(Tape::decode(&bytes).expect("valid tape decodes"), tape);

    for len in 0..bytes.len() {
        assert!(
            Tape::decode(&bytes[..len]).is_err(),
            "prefix {len} must fail"
        );
    }
    assert!(Tape::decode(&vec![0u8; MAX_TAPE_BYTES + 1]).is_err());

    for index in 0..bytes.len() {
        let mut bad = bytes.clone();
        bad[index] ^= 0x5A;
        if let Ok(decoded) = Tape::decode(&bad) {
            assert_eq!(
                decoded.encode().expect("decoded tape re-encodes"),
                bad,
                "byte {index} changed but re-encode differs"
            );
        }
    }

    let mut state = 0x7A9E_u64;
    for _ in 0..256 {
        let mut bytes = [0u8; 60];
        for chunk in bytes.chunks_mut(8) {
            let value = lcg(&mut state).to_le_bytes();
            chunk.copy_from_slice(&value[..chunk.len()]);
        }
        if let Ok(decoded) = Tape::decode(&bytes) {
            assert!(decoded.config.ticks >= 1 && decoded.config.ticks <= 2);
            assert_eq!(decoded.checkpoints.len(), decoded.config.ticks as usize);
            assert!(decoded.inputs.len() + decoded.config.ticks as usize == 2);
            assert_eq!(
                decoded
                    .encode()
                    .expect("decoded tape re-encodes")
                    .as_slice(),
                bytes.as_slice()
            );
        }
    }
}
