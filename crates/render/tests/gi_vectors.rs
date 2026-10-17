//! gi_vectors: L2 SH coefficient vector gates for the lookup-only runtime.
//!
//! Owner: render lane; consumer: `GiRuntime::irradiance` plus the RL2 IBL
//! `eval_sh` path, which share `gi::sh_basis`. Units: unit directions in,
//! basis weights out. Determinism: fixed L2 order, same inputs give the same
//! nine weights on every run. Errors: none; degenerate dirs stay finite.
//!
//! Shape adapted from PORT(google/filament@main:libs/ibl/include/ibl/CubemapSH.h)
//! Apache-2.0 `computeSH` (cubemap bands to float3[9] L2 coeffs); expected
//! vectors below are fresh constants derived from the L2 basis, not copied.

use engine_core::alloc_probe;
use engine_render::gi::{sh_basis, sh_irradiance, RUNTIME_OPS_PER_SAMPLE, SH_COEFFS};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

/// L2 order: L00, L1y, L10, L11, L2-2, L2-1, L20, L21, L22.
const PLUS_X: [f32; 9] = [
    0.282095, 0.0, 0.0, -0.488603, 0.0, 0.0, -0.315392, 0.0, 0.546274,
];
const PLUS_Y: [f32; 9] = [
    0.282095, -0.488603, 0.0, 0.0, 0.0, 0.0, -0.315392, 0.0, -0.546274,
];
const PLUS_Z: [f32; 9] = [0.282095, 0.0, 0.488603, 0.0, 0.0, 0.0, 0.630784, 0.0, 0.0];
const DIRS: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
/// Lookup budget: index plus lerp plus the 27 multiply-add eval, capped at 64.
const LOOKUP_OP_BUDGET: u32 = 64;
/// Match tolerance for one coefficient.
const COEFF_TOL: f32 = 1e-6;

fn expected(axis: usize) -> [f32; 9] {
    if axis == 0 {
        PLUS_X
    } else if axis == 1 {
        PLUS_Y
    } else {
        PLUS_Z
    }
}

fn matches_nine(got: [f32; 9], want: [f32; 9]) -> u32 {
    let mut hits: u32 = 0;
    let mut i: usize = 0;
    while i < SH_COEFFS {
        let d = got[i] - want[i];
        let a = if d < 0.0 { -d } else { d };
        if a <= COEFF_TOL {
            hits += 1;
        }
        i += 1;
    }
    hits
}

fn all_finite(v: [f32; 9]) -> bool {
    let mut i: usize = 0;
    while i < v.len() {
        if !v[i].is_finite() {
            return false;
        }
        i += 1;
    }
    true
}

#[test]
fn nine_coeffs_match_on_three_axes() {
    assert_eq!(SH_COEFFS, 9);
    let mut total: u32 = 0;
    let mut axis: usize = 0;
    while axis < DIRS.len() {
        let got = sh_basis(DIRS[axis]);
        assert!(all_finite(got));
        let hits = matches_nine(got, expected(axis));
        assert_eq!(hits, 9, "axis mismatch");
        total += hits;
        axis += 1;
    }
    assert_eq!(total, 27, "three axes carry 27 matching weights");
}

#[test]
fn lookup_budget_is_sixty_four_ops() {
    assert_eq!(SH_COEFFS, 9);
    assert_eq!(RUNTIME_OPS_PER_SAMPLE, 64);
    // Compile-time law: the lookup may never outgrow its op budget. A const
    // assertion fails the build, so it cannot pass vacuously at run time.
    const _: () = assert!(
        RUNTIME_OPS_PER_SAMPLE <= LOOKUP_OP_BUDGET,
        "lookup must stay within 64 ops"
    );
    // Eval touches 9 coeffs times 3 channels: 27 multiply-adds.
    let mul_adds: u32 = SH_COEFFS as u32 * 3;
    assert_eq!(mul_adds, 27);
    assert!(mul_adds < LOOKUP_OP_BUDGET);
    // Single-coeff SH evaluates to the L00 weight on every axis.
    let mut sh = [[0.0f32; 3]; SH_COEFFS];
    sh[0] = [1.0, 1.0, 1.0];
    let mut axis: usize = 0;
    while axis < DIRS.len() {
        let e = sh_irradiance(&sh, DIRS[axis]);
        let mut c: usize = 0;
        while c < 3 {
            let d = e[c] - 0.282095;
            let a = if d < 0.0 { -d } else { d };
            assert!(a <= COEFF_TOL, "L00-only SH reads 0.282095");
            c += 1;
        }
        axis += 1;
    }
}

#[test]
fn lookup_path_allocates_nothing() {
    let sh = [[0.5f32; 3]; SH_COEFFS];
    let ((), heap) = alloc_probe::measure(|| {
        let mut i: usize = 0;
        while i < 32 {
            let b = sh_basis(DIRS[i % DIRS.len()]);
            let e = sh_irradiance(&sh, DIRS[i % DIRS.len()]);
            std::hint::black_box((b, e));
            i += 1;
        }
    });
    assert_eq!(heap.calls, 0, "SH lookup must not allocate");
    assert_eq!(heap.bytes, 0, "SH lookup must not allocate");
}

#[test]
fn degenerate_direction_stays_finite() {
    // Zero-length input is guarded by the normalizer, never NaN.
    let b = sh_basis([0.0, 0.0, 0.0]);
    assert!(all_finite(b));
    let zero = [[0.0f32; 3]; SH_COEFFS];
    let e = sh_irradiance(&zero, [0.0, 0.0, 0.0]);
    assert_eq!(e, [0.0, 0.0, 0.0]);
    assert!(e.iter().all(|x| x.is_finite()));
}
