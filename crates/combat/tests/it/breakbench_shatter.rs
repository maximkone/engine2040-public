// Donor: raylib-rs/physac-rs examples/physics_shatter, licence Zlib/MIT (licence verified), pattern only.
// Link: https://github.com/raylib-rs/physac-rs
// Pattern reused: one body breaks into seeded fragments whose masses sum to
// the whole. This file reimplements that idea in integers only: fixed-seed
// weights split a gram total, the remainder is dealt back so the sum is
// exact, and the seed is fed by `hit.simulate` so break and balance share
// one deterministic stream.

use engine_combat::anatomy::AnatomyTable;
use engine_combat::hitsim::{simulate, HitSpec};
use engine_combat::limbloss::WoundTable;
use engine_core::Rng;

// Fixed seed for the whole distribution; same input gives same report.
const FIXED_SEED: u64 = 2040;
// Shards one body breaks into; pinned so balance can read the shape.
const SHARD_COUNT: usize = 12;
// Whole-body mass in grams; the shards must sum back to this.
const TOTAL_MASS_G: u64 = 80_000;
// Clean hits per joint for the `hit.simulate` leg; small and exact.
const SAMPLES: u32 = 1_024;

// Seeded integer split of `total_g` into `count` positive shards.
//
// Each shard draws a weight `1..=1000` from a `shatter` substream, masses
// are floored shares of the total, and the leftover grams are dealt one by
// one from shard zero so the sum is exact. Pure integer math: same seed,
// same masses, on every PC.
fn shatter_masses(seed: u64, total_g: u64, count: usize) -> Vec<u64> {
    let mut rng = Rng::new(seed).fork_named("shatter");
    let mut weights = Vec::with_capacity(count);
    for _ in 0..count {
        weights.push(rng.next_u64() % 1_000 + 1);
    }
    let sum: u64 = weights.iter().copied().sum();
    let mut masses = Vec::with_capacity(count);
    let mut assigned: u64 = 0;
    for weight in &weights {
        let share = weight * total_g / sum;
        masses.push(share);
        assigned += share;
    }
    let mut remainder = total_g - assigned;
    let mut index: usize = 0;
    while remainder > 0 {
        masses[index % count] += 1;
        remainder -= 1;
        index += 1;
    }
    masses
}

#[test]
fn seeded_shatter_conserves_mass_and_pins_count() {
    let anatomy = AnatomyTable::rome_kit();
    let wound = WoundTable::v1();
    let spec = HitSpec {
        energy_q16: 20_u32 * 65_536,
        sharpness_q16: 90_000,
        wear_q16: 0,
        worn: 0,
        samples: SAMPLES,
        seed: FIXED_SEED,
    };
    let report = simulate(&anatomy, &wound, &spec).expect("fixed spec is in range");
    let rerun = simulate(&anatomy, &wound, &spec).expect("fixed spec is in range");
    assert_eq!(report, rerun, "same spec replays the same report");
    assert_eq!(report.samples, SAMPLES);
    assert_eq!(report.rows.len(), engine_combat::SEVERABLE_JOINT_COUNT);
    for row in report.rows {
        assert!(
            row.severed <= SAMPLES,
            "joint {} severed past samples",
            row.joint
        );
    }
    // The break leg feeds the shatter leg: one stream, one receipt.
    let shatter_seed = FIXED_SEED ^ report.digest;
    let shards = shatter_masses(shatter_seed, TOTAL_MASS_G, SHARD_COUNT);
    let shards_again = shatter_masses(shatter_seed, TOTAL_MASS_G, SHARD_COUNT);
    assert_eq!(shards, shards_again, "same seed shatters the same way");
    assert_eq!(shards.len(), SHARD_COUNT, "shard count is pinned");
    assert!(
        shards.iter().all(|mass| *mass > 0),
        "every shard keeps some mass"
    );
    let total: u64 = shards.iter().copied().sum();
    assert_eq!(total, TOTAL_MASS_G, "shard masses conserve the body");
    println!(
        "passed shatter shards={} total_g={} digest={}",
        shards.len(),
        total,
        report.digest
    );
}

#[test]
fn bad_spec_still_refuses() {
    let anatomy = AnatomyTable::rome_kit();
    let wound = WoundTable::v1();
    let bad = HitSpec {
        energy_q16: 20_u32 * 65_536,
        sharpness_q16: 90_000,
        wear_q16: 0,
        worn: 0,
        samples: 0,
        seed: FIXED_SEED,
    };
    assert!(
        simulate(&anatomy, &wound, &bad).is_err(),
        "empty run refuses"
    );
}
