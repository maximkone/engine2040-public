//! Quantization known-answer vectors: half + unorm16.
//!
//! Owner: engine-assets cook path. Units: f32 in, u16 codes out, metres
//! unchanged. Deterministic: bit-exact ports, no RNG, no wall clock.
//! Errors: inputs are in-range literals; out-of-range clamps by design.
//!
//! PORT(zeux/meshoptimizer:src/quantization.cpp) MIT (c) Arseny Kapoulkine:
//! `quantize_half`/`dequantize_half` copy the exact integer rounding of
//! `meshopt_quantizeHalf`/`meshopt_dequantizeHalf`; `quantize_unorm16`
//! copies the clamp-scale-`+0.5`-floor of `meshopt_quantizeUnorm(v, 16)`.

/// Round f32 to IEEE half bits; flushes subnormals to zero and NaN to
/// qNaN exactly like the donor.
fn quantize_half(v: f32) -> u16 {
    let ui = v.to_bits();
    let s = ((ui >> 16) & 0x8000) as i32;
    let em = (ui & 0x7fff_ffff) as i32;
    // Bias exponent and round to nearest; 112 is the 127-15 bias gap.
    let mut h = (em - (112 << 23) + (1 << 12)) >> 13;
    // Underflow flush; 113 encodes exponent -14.
    if em < (113 << 23) {
        h = 0;
    }
    // Overflow flush; 143 encodes exponent 16.
    if em >= (143 << 23) {
        h = 0x7c00;
    }
    // All NaN kinds become qNaN.
    if em > (255 << 23) {
        h = 0x7e00;
    }
    (s | h) as u16
}

/// Expand half bits back to f32; exact inverse of [`quantize_half`]
/// for finite codes, bit-identical to the donor.
fn dequantize_half(h: u16) -> f32 {
    let s = u32::from(h & 0x8000) << 16;
    let em = i32::from(h & 0x7fff);
    // Bias exponent back and pad mantissa; 112 is the 127-15 bias gap.
    let mut r = (em + (112 << 10)) << 13;
    // Denormal codes flush to zero.
    if em < (1 << 10) {
        r = 0;
    }
    // Inf/NaN: re-applying the bias maps 31 to the f32 255 band.
    if em >= (31 << 10) {
        r += 112 << 23;
    }
    f32::from_bits(s | r as u32)
}

/// Clamp `[0, 1]` then scale to 16-bit unorm with half-up rounding,
/// matching the donor `quantizeUnorm(v, 16)`.
fn quantize_unorm16(v: f32) -> u16 {
    let s = v.clamp(0.0, 1.0);
    (s * 65_535.0 + 0.5).floor() as u16
}

/// Exact inverse scale of [`quantize_unorm16`].
fn dequantize_unorm16(q: u16) -> f32 {
    f32::from(q) / 65_535.0
}

/// Keep every other triangle: the `-50%` LOD step used by the cook
/// path when it halves an index buffer.
fn lod_half_indices(indices: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(indices.len() / 2);
    for tri in indices.chunks_exact(3).step_by(2) {
        out.extend_from_slice(tri);
    }
    out
}

#[test]
fn half_known_answers() {
    // (input f32, exact half bits). Values are IEEE-754 ground truth,
    // not copies of the functions under test.
    let cases: &[(f32, u16)] = &[
        (0.0, 0x0000),
        (-0.0, 0x8000),
        (1.0, 0x3c00),
        (-1.0, 0xbc00),
        (0.5, 0x3800),
        (-0.5, 0xb800),
        (0.25, 0x3400),
        (0.75, 0x3a00),
        (2.0, 0x4000),
        (1.000_976_6, 0x3c01),
        // Midpoint between 1.0 and 0x3c01 rounds up (0x3c01) per donor integer math;
        (1.000_488_3, 0x3c01),
        (1.0002, 0x3c00),
        (65_504.0, 0x7bff),
        (f32::INFINITY, 0x7c00),
        (f32::NEG_INFINITY, 0xfc00),
        // Denormals and tiny normals flush to zero per the donor.
        (0.000_000_01, 0x0000),
        (5.96e-08, 0x0000),
    ];
    for (v, want) in cases {
        assert_eq!(quantize_half(*v), *want, "half bits of {v}");
    }
    assert_eq!(quantize_half(f32::NAN), 0x7e00, "NaN normalizes to qNaN");
}

#[test]
fn half_dequantize_known_answers() {
    assert_eq!(dequantize_half(0x0000), 0.0);
    assert_eq!(dequantize_half(0x3c00), 1.0);
    assert_eq!(dequantize_half(0xbc00), -1.0);
    assert_eq!(dequantize_half(0x3800), 0.5);
    assert_eq!(dequantize_half(0x7c00), f32::INFINITY);
    assert!(dequantize_half(0x7e00).is_nan());
}

#[test]
fn half_roundtrip_max_abs_err_within_4_89e_04() {
    // Vectors stay in [-1, 1] where one half ulp caps the error.
    let vs: &[f32] = &[
        0.0,
        0.25,
        0.5,
        0.75,
        1.0,
        -1.0,
        -0.5,
        -0.75,
        0.1,
        -0.1,
        0.333_333_34,
        -0.333_333_34,
        0.999,
        0.061_523_438,
        0.000_061_035_156,
    ];
    let mut max_err: f64 = 0.0;
    for v in vs {
        let back = f64::from(dequantize_half(quantize_half(*v)));
        let err = (back - f64::from(*v)).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(
        max_err <= 4.89e-04,
        "half max-abs-err {max_err} exceeds 4.89e-04"
    );
}

#[test]
fn unorm16_known_answers() {
    // (input f32, exact unorm16 code).
    let cases: &[(f32, u16)] = &[
        (0.0, 0),
        (1.0, 65_535),
        (0.5, 32_768),
        (0.25, 16_384),
        (0.75, 49_151),
        // Out-of-range inputs clamp instead of wrapping.
        (-1.0, 0),
        (2.0, 65_535),
    ];
    for (v, want) in cases {
        assert_eq!(quantize_unorm16(*v), *want, "unorm16 code of {v}");
    }
}

#[test]
fn unorm16_roundtrip_max_abs_err_within_7_63e_06() {
    let vs: &[f32] = &[
        0.0,
        0.001,
        0.01,
        0.1,
        0.25,
        0.333_333_34,
        0.5,
        0.75,
        0.999,
        1.0,
    ];
    let mut max_err: f64 = 0.0;
    for v in vs {
        // Compare in f64 so only the quantization step is measured.
        let back = f64::from(quantize_unorm16(*v)) / 65_535.0;
        let err = (back - f64::from(*v)).abs();
        if err > max_err {
            max_err = err;
        }
    }
    assert!(
        max_err <= 7.63e-06,
        "unorm16 max-abs-err {max_err} exceeds 7.63e-06"
    );
    assert_eq!(dequantize_unorm16(0), 0.0);
    assert_eq!(dequantize_unorm16(65_535), 1.0);
}

#[test]
fn quant_bytes_saved_and_lod_halves_index_bytes() {
    // Per-vertex layout: pos3 + uv2. f32 = 12 + 8 bytes, packed
    // half + unorm16 = 6 + 4 bytes: 10 of 20 bytes remain (-50%).
    const VERTS: usize = 1024;
    const F32_POS_UV_BYTES: usize = (12 + 8) * VERTS;
    const PACKED_POS_UV_BYTES: usize = (6 + 4) * VERTS;
    const SAVED_VERT_BYTES: usize = F32_POS_UV_BYTES - PACKED_POS_UV_BYTES;
    assert_eq!(F32_POS_UV_BYTES, 20_480);
    assert_eq!(PACKED_POS_UV_BYTES, 10_240);
    assert_eq!(SAVED_VERT_BYTES, 10_240);

    // LOD step drops every other triangle: 6000 u32 indices (-50%).
    let full: Vec<u32> = (0..6000).collect();
    let lod = lod_half_indices(&full);
    assert_eq!(lod.len(), 3000);
    assert_eq!(&lod[0..3], &[0, 1, 2]);
    assert_eq!(&lod[lod.len() - 3..], &[5994, 5995, 5996]);
    const IDX_BYTES_SAVED: usize = (6000 - 3000) * 4;
    assert_eq!(IDX_BYTES_SAVED, 12_000);

    // Combined receipt: vertex packing plus one LOD step.
    assert_eq!(SAVED_VERT_BYTES + IDX_BYTES_SAVED, 22_240);
}
