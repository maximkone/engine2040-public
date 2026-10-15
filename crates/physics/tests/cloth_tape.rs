//! Integration leg: the public cloth-tape API runs a small deterministic
//! tape from outside the crate (the collider-coupling harnesses will call
//! it the same way). Units: raw Q16.16 units, ticks at 60 Hz.
//!
//! This binary installs the counting allocator so the per-tick probe below
//! observes real heap use; without it the maxima would read zero.

use engine_core::alloc_probe;
use engine_physics::cloth::{
    capsule_penetration_max, cloth_digest, run_cloth_tape, self_separation_min, step_cloth,
    CapsuleCollider, ClothError, ClothParams, ClothTapeConfig, ClothWorld, CLOTH_TAPE_SEED,
    TAPE_BAR_RADIUS_RAW, TAPE_COLS, TAPE_ROWS, TAPE_SPACING,
};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

#[test]
fn small_cloth_tape_deterministic_and_bounded() {
    let config = ClothTapeConfig {
        seed: CLOTH_TAPE_SEED,
        drapes: 2,
        steps_per_drape: 30,
    };
    assert_eq!(config.ticks(), 3 * 30);
    let a = run_cloth_tape(&config).expect("tape runs");
    let b = run_cloth_tape(&config).expect("tape reruns");
    assert_eq!(a.digest, b.digest, "small tape must repeat: {a} vs {b}");
    assert_eq!(a, b);
    assert_eq!(a.ticks, config.ticks());
    println!("{a}");
}

#[test]
fn small_cloth_tape_step_allocates_nothing() {
    let mut w = ClothWorld::new_grid(
        TAPE_COLS,
        TAPE_ROWS,
        TAPE_SPACING,
        [0, 10 * 65536, 0],
        ClothParams::default_cape(),
    )
    .expect("cape grid");
    step_cloth(&mut w, 1);
    let ((), allocs) = alloc_probe::measure(|| {
        step_cloth(&mut w, 1);
        let cap = engine_physics::cloth::CapsuleCollider::new(
            [-16000, 10 * 65536 - 6 * TAPE_SPACING, 0],
            [16000, 10 * 65536 - 6 * TAPE_SPACING, 0],
            TAPE_BAR_RADIUS_RAW,
        )
        .expect("tape bar");
        let _ = capsule_penetration_max(&w, &[cap]);
        let _ = self_separation_min(&w);
        let _ = cloth_digest(&w);
    });
    assert_eq!(allocs.calls, 0, "tape step must not allocate: {allocs:?}");
}

#[test]
fn small_cloth_tape_refuses_bad_input() {
    let mut config = ClothTapeConfig {
        seed: CLOTH_TAPE_SEED,
        drapes: 0,
        steps_per_drape: 30,
    };
    assert_eq!(
        run_cloth_tape(&config).expect_err("empty tape must refuse"),
        ClothError::BadParam("drapes")
    );
    config.drapes = 51;
    assert_eq!(
        run_cloth_tape(&config).expect_err("oversize tape must refuse"),
        ClothError::BadParam("drapes")
    );
    config = ClothTapeConfig {
        seed: CLOTH_TAPE_SEED,
        drapes: 2,
        steps_per_drape: 0,
    };
    assert_eq!(
        run_cloth_tape(&config).expect_err("empty steps must refuse"),
        ClothError::BadParam("steps_per_drape")
    );
    config.steps_per_drape = 241;
    assert_eq!(
        run_cloth_tape(&config).expect_err("oversize steps must refuse"),
        ClothError::BadParam("steps_per_drape")
    );
    assert_eq!(
        CapsuleCollider::new([0, 0, 0], [1000, 0, 0], 0).expect_err("zero radius must refuse"),
        ClothError::BadParam("capsule radius")
    );
    assert_eq!(
        CapsuleCollider::new([0, 40 * 65536, 0], [1000, 0, 0], 100)
            .expect_err("far endpoint must refuse"),
        ClothError::BadParam("capsule endpoint")
    );
    let tune = ClothParams::default_cape();
    assert_eq!(
        ClothWorld::new_grid(0, TAPE_ROWS, TAPE_SPACING, [0, 10 * 65536, 0], tune)
            .expect_err("empty grid must refuse"),
        ClothError::BadDims
    );
    assert_eq!(
        ClothWorld::new_grid(TAPE_COLS, TAPE_ROWS, 0, [0, 10 * 65536, 0], tune)
            .expect_err("empty spacing must refuse"),
        ClothError::BadSpacing
    );
}
