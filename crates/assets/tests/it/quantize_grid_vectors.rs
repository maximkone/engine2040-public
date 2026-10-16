// PORT(zeux/meshoptimizer:src/quantization.cpp) MIT (c) Arseny Kapoulkine:
// Idea reused, code fresh: meshopt_quantizeFloat clamp-scale-round grid
// plus the computePositionExponent pattern of fitting one shared
// center/half-extent over the position AABB, then quantizing each
// normalized component to an N-bit signed grid.

//! N-bit grid-quantize parity vectors: cooked position bytes shrink.
//!
//! Owner: engine-assets cook path. Units: metres in, signed grid codes out.
//! Deterministic: integer grid math plus IEEE round/divide only, no RNG.
//! Errors: out-of-range components clamp to the grid edge; NaN maps to 0.

/// Signed grid scale for bits in 2..=16: (2^(bits-1)) - 1.
fn snorm_scale(bits: u32) -> f32 {
    assert!((2..=16).contains(&bits), "bits must be 2..=16");
    // Integer shift keeps the scale exact; the only float op is the final cast.
    let levels: u32 = (1_u32 << (bits - 1)) - 1;
    levels as f32
}

/// Fresh port of the meshopt_quantizeFloat idea: clamp v in [-1, 1],
/// scale by the N-bit grid, round to nearest (half away), clamp to the grid.
fn quantize_float_grid(v: f32, bits: u32) -> i32 {
    let scale = snorm_scale(bits);
    if v.is_nan() {
        return 0;
    }
    if !v.is_finite() {
        return if v > 0.0 {
            scale as i32
        } else {
            -(scale as i32)
        };
    }
    let clamped = v.clamp(-1.0, 1.0);
    let rounded = (clamped * scale).round();
    let hi = scale;
    if rounded >= hi {
        hi as i32
    } else if rounded <= -hi {
        -(hi as i32)
    } else {
        rounded as i32
    }
}

/// Exact inverse scale of quantize_float_grid.
fn dequantize_float_grid(q: i32, bits: u32) -> f32 {
    let scale = snorm_scale(bits);
    q as f32 / scale
}

/// Shared grid over a position AABB, mirroring the donor exponent pattern:
/// one center plus one half-extent covering the longest axis.
struct Grid {
    center: [f32; 3],
    half_extent: f32,
    bits: u32,
}

/// Fit Grid over min/max (finite literals from the cook path).
/// A degenerate AABB falls back to a tiny extent so division stays finite.
fn compute_grid(min: [f32; 3], max: [f32; 3], bits: u32) -> Grid {
    assert!((2..=16).contains(&bits), "bits must be 2..=16");
    let mut center = [0.0_f32; 3];
    let mut extent: f32 = 0.0;
    for axis in 0..3 {
        center[axis] = (min[axis] + max[axis]) * 0.5;
        let half = (max[axis] - min[axis]) * 0.5;
        if half > extent {
            extent = half;
        }
    }
    // Degenerate input is real cook data (single-point stub): keep it finite.
    if extent.is_nan() || extent <= 0.0 {
        extent = 1.0e-6;
    }
    Grid {
        center,
        half_extent: extent,
        bits,
    }
}

/// Quantize one position through grid: normalize, then grid-quantize.
fn quantize_position(p: [f32; 3], grid: &Grid) -> [i32; 3] {
    let mut out = [0_i32; 3];
    for axis in 0..3 {
        let n = (p[axis] - grid.center[axis]) / grid.half_extent;
        out[axis] = quantize_float_grid(n, grid.bits);
    }
    out
}

/// Expand one grid position back to metres.
fn dequantize_position(q: [i32; 3], grid: &Grid) -> [f32; 3] {
    let mut out = [0.0_f32; 3];
    for axis in 0..3 {
        out[axis] =
            grid.center[axis] + dequantize_float_grid(q[axis], grid.bits) * grid.half_extent;
    }
    out
}

/// Worst-case world error of one grid step: half a grid cell in metres.
fn half_step_world(grid: &Grid) -> f32 {
    grid.half_extent / snorm_scale(grid.bits) * 0.5
}

fn max_abs_err(a: &[f32], b: &[f32]) -> f64 {
    let mut max: f64 = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        let err = f64::from(*x - *y).abs();
        if err > max {
            max = err;
        }
    }
    max
}

#[test]
fn snorm_known_answers() {
    // Exact grid codes: scale 32767 for 16 bits, 127 for 8 bits.
    assert_eq!(quantize_float_grid(0.0, 16), 0);
    assert_eq!(quantize_float_grid(1.0, 16), 32_767);
    assert_eq!(quantize_float_grid(-1.0, 16), -32_767);
    // 0.5 * 32767 = 16383.5 rounds away from zero to 16384.
    assert_eq!(quantize_float_grid(0.5, 16), 16_384);
    assert_eq!(quantize_float_grid(-0.5, 16), -16_384);
    assert_eq!(quantize_float_grid(1.0, 8), 127);
    assert_eq!(quantize_float_grid(-1.0, 8), -127);
    assert_eq!(quantize_float_grid(0.5, 8), 64);
    // Out-of-range clamps instead of wrapping.
    assert_eq!(quantize_float_grid(2.0, 16), 32_767);
    assert_eq!(quantize_float_grid(-2.0, 16), -32_767);
    // Non-finite inputs stay finite codes.
    assert_eq!(quantize_float_grid(f32::NAN, 16), 0);
    assert_eq!(quantize_float_grid(f32::INFINITY, 16), 32_767);
    assert_eq!(quantize_float_grid(f32::NEG_INFINITY, 16), -32_767);
    // Inverse scale is exact at the grid points.
    assert_eq!(dequantize_float_grid(0, 16), 0.0);
    assert_eq!(dequantize_float_grid(32_767, 16), 1.0);
    assert_eq!(dequantize_float_grid(-32_767, 16), -1.0);
    println!("snorm grid answers ok: scale16 32767, scale8 127");
}

#[test]
fn positions_48b_to_24b_per_quad() {
    // Four verts of a unit quad: 4 * 12 B f32 -> 4 * 6 B i16 grid.
    let positions: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let verts: usize = positions.len();
    let before: usize = verts * 12;
    // 16-bit grid packs each component into 2 bytes.
    let after: usize = verts * 3 * 2;
    assert_eq!(before, 48);
    assert_eq!(after, 24);
    assert!(after < before);

    let grid = compute_grid([0.0, 0.0, 0.0], [1.0, 1.0, 0.0], 16);
    let bound = half_step_world(&grid) + 1.0e-6;
    for p in positions {
        let q = quantize_position(p, &grid);
        let back = dequantize_position(q, &grid);
        for axis in 0..3 {
            let err = (back[axis] - p[axis]).abs();
            assert!(err <= bound, "axis {axis} err {err} over {bound}");
        }
    }
    println!("bytes-shrunk {before} -> {after}");
}

#[test]
fn roundtrip_error_within_half_step() {
    let grid = compute_grid([-2.0, -2.0, -2.0], [2.0, 2.0, 2.0], 16);
    let bound = f64::from(half_step_world(&grid)) + 1.0e-6;
    let fixture: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [1.0, -1.0, 0.5],
        [-1.5, 1.5, -0.25],
        [2.0, 2.0, 2.0],
        [-2.0, -2.0, -2.0],
        [0.333_333_34, -0.75, 1.25],
        [0.1, -0.1, 0.05],
        [-0.9, 0.9, -1.1],
    ];
    let mut flat_before = Vec::with_capacity(24);
    let mut flat_after = Vec::with_capacity(24);
    for p in fixture {
        let back = dequantize_position(quantize_position(p, &grid), &grid);
        flat_before.extend_from_slice(&p);
        flat_after.extend_from_slice(&back);
    }
    let max_err = max_abs_err(&flat_before, &flat_after);
    assert!(
        max_err <= bound,
        "max err {max_err} exceeds half-step {bound}"
    );
    println!("max err {max_err} within half-step {bound}");
}

#[test]
fn bit_width_trades_size_for_error() {
    let min = [0.0, 0.0, 0.0];
    let max = [1.0, 1.0, 1.0];
    let grid8 = compute_grid(min, max, 8);
    let grid16 = compute_grid(min, max, 16);
    let fixture: [[f32; 3]; 6] = [
        [0.0, 0.0, 0.0],
        [0.2, 0.4, 0.6],
        [0.33, 0.66, 0.99],
        [0.123, 0.456, 0.789],
        [1.0, 1.0, 1.0],
        [0.5, 0.25, 0.75],
    ];
    let mut before8 = Vec::with_capacity(18);
    let mut after8 = Vec::with_capacity(18);
    let mut before16 = Vec::with_capacity(18);
    let mut after16 = Vec::with_capacity(18);
    for p in fixture {
        let b8 = dequantize_position(quantize_position(p, &grid8), &grid8);
        let b16 = dequantize_position(quantize_position(p, &grid16), &grid16);
        before8.extend_from_slice(&p);
        after8.extend_from_slice(&b8);
        before16.extend_from_slice(&p);
        after16.extend_from_slice(&b16);
    }
    let err8 = max_abs_err(&before8, &after8);
    let err16 = max_abs_err(&before16, &after16);
    assert!(err16 < err8, "16-bit {err16} should beat 8-bit {err8}");
    assert!(
        err16 <= f64::from(half_step_world(&grid16)) + 1.0e-6,
        "16-bit {err16} over its bound"
    );
    // 8-bit codes never leave the [-127, 127] grid.
    for p in fixture {
        for c in quantize_position(p, &grid8) {
            assert!((-127..=127).contains(&c), "8-bit code {c} out of grid");
        }
    }
    println!("bit-width trade: err8 {err8} err16 {err16}");
}

#[test]
fn determinism_x2_identical() {
    let grid = compute_grid([-1.0, -1.0, -1.0], [3.0, 5.0, 7.0], 12);
    let fixture: [[f32; 3]; 4] = [
        [0.5, -1.25, 3.75],
        [1.0, 0.1, -0.1],
        [2.5, 4.25, 6.5],
        [-0.5, 0.0, 7.0],
    ];
    for p in fixture {
        let a = quantize_position(p, &grid);
        let b = quantize_position(p, &grid);
        assert_eq!(a, b);
        assert_eq!(dequantize_position(a, &grid), dequantize_position(b, &grid));
    }
    println!(
        "deterministic grid codes identical: {} verts",
        fixture.len()
    );
}

#[test]
fn clamp_and_degenerate_aabb() {
    // Out-of-range components clamp to the grid edge: finite, no wrap.
    let grid = compute_grid([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 16);
    let far = quantize_position([10.0, -10.0, 2.0], &grid);
    assert_eq!(far[0], 32_767);
    assert_eq!(far[1], -32_767);
    assert_eq!(far[2], 32_767);
    let back = dequantize_position(far, &grid);
    for v in back {
        assert!(v.is_finite(), "clamped decode must stay finite");
    }
    // Degenerate AABB (single point) stays finite and exact at the center.
    let single = compute_grid([1.0, 2.0, 3.0], [1.0, 2.0, 3.0], 16);
    assert!(single.half_extent > 0.0);
    assert!(single.half_extent.is_finite());
    let q = quantize_position([1.0, 2.0, 3.0], &single);
    assert_eq!(q, [0, 0, 0]);
    let round = dequantize_position(q, &single);
    assert_eq!(round, [1.0, 2.0, 3.0]);
    println!("clamp + degenerate grid ok");
}
