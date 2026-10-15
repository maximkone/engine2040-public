// Donor: zeux/meshoptimizer, MIT, https://github.com/zeux/meshoptimizer (src/quantization.cpp).
// Idea ported fresh in our style: f32-bits round-nearest half quantize plus dequantize.
// Underflow flushes to signed zero, overflow to inf, NaN to canonical qNaN.

//! Meshoptimizer-style half-quantize parity vectors: cooked mesh bytes shrink.

fn f32_to_f16_bits(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign: u16 = ((bits >> 16) & 0x8000) as u16;
    let exp: i32 = ((bits >> 23) & 0xff) as i32;
    let mant: u32 = bits & 0x007f_ffff;
    if exp == 0xff {
        if mant == 0 {
            return sign | 0x7c00;
        }
        return sign | 0x7e00;
    }
    let exp16: i32 = exp - 112;
    if exp16 >= 31 {
        return sign | 0x7c00;
    }
    if exp16 <= 0 {
        if exp16 < -10 {
            return sign;
        }
        let m: u32 = mant | 0x0080_0000;
        let shift: u32 = (14 - exp16) as u32;
        let half: u32 = 1u32 << (shift - 1);
        let mask: u32 = (1u32 << shift) - 1;
        let rem: u32 = m & mask;
        let mut q: u16 = (m >> shift) as u16;
        if rem > half || (rem == half && (q & 1) != 0) {
            q += 1;
        }
        return sign | q;
    }
    let e16: u16 = (exp16 as u16) << 10;
    let top: u16 = (mant >> 13) as u16;
    let rem: u32 = mant & 0x1fff;
    let mut q: u16 = top;
    if rem > 0x1000 || (rem == 0x1000 && (q & 1) == 1) {
        q += 1;
        if q == 0x0400 {
            if exp16 >= 30 {
                return sign | 0x7c00;
            }
            return sign | (((exp16 + 1) as u16) << 10);
        }
    }
    sign | e16 | q
}

fn f16_bits_to_f32(h: u16) -> f32 {
    let sign: u32 = ((h as u32) & 0x8000) << 16;
    let exp: u32 = ((h >> 10) & 0x1f) as u32;
    let mant: u32 = (h & 0x03ff) as u32;
    let fbits: u32 = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let mut m: u32 = mant;
            let mut e: i32 = -14;
            while (m & 0x0400) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x03ff;
            sign | (((e + 127) as u32) << 23) | (m << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | (mant << 13)
    } else {
        sign | ((exp + 112) << 23) | (mant << 13)
    };
    f32::from_bits(fbits)
}

fn quantize_positions(src: &[f32]) -> Vec<u16> {
    src.iter().map(|v| f32_to_f16_bits(*v)).collect()
}

fn dequantize_positions(src: &[u16]) -> Vec<f32> {
    src.iter().map(|h| f16_bits_to_f32(*h)).collect()
}

#[test]
fn roundtrip_max_error_within_half_ulp() {
    let fixture: [f32; 8] = [0.5, 1.0, -2.0, 100.0, 0.333_333_34, -0.75, 33.25, 1000.0];
    let bound: f32 = 1.0 / 2048.0;
    let mut max_rel: f32 = 0.0;
    for v in fixture {
        let d = f16_bits_to_f32(f32_to_f16_bits(v));
        let err = (d - v).abs() / v.abs();
        if err > max_rel {
            max_rel = err;
        }
        assert!(err <= bound, "rel err out of bound");
    }
    println!("max rel err {max_rel} within half-ULP bound {bound}");
}

#[test]
fn positions_12b_to_6b_per_vert() {
    let positions: [f32; 12] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let verts: usize = positions.len() / 3;
    let before: usize = verts * 12;
    let q = quantize_positions(&positions);
    let after: usize = q.len() * 2;
    assert_eq!(before, 48);
    assert_eq!(after, 24);
    assert!(after < before);
    let back = dequantize_positions(&q);
    for (a, b) in positions.iter().zip(back.iter()) {
        assert!((a - b).abs() <= 0.001, "drift too big");
    }
    println!("bytes-shrunk {before} -> {after}");
}

#[test]
fn determinism_x2_identical() {
    let positions: [f32; 9] = [0.5, -1.25, 3.75, 100.0, 0.1, -0.1, 7.5, 8.25, -9.5];
    let a = quantize_positions(&positions);
    let b = quantize_positions(&positions);
    assert_eq!(a, b);
    println!("deterministic halves identical: len {}", a.len());
}

#[test]
fn specials_flush_overflow_qnan() {
    assert_eq!(f32_to_f16_bits(1.0e-10_f32), 0x0000);
    assert_eq!(f32_to_f16_bits(-1.0e-10_f32), 0x8000);
    assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x7c00);
    assert_eq!(f32_to_f16_bits(1.0e10_f32), 0x7c00);
    let qnan = f32_to_f16_bits(f32::NAN);
    assert_eq!(qnan & 0x7c00, 0x7c00);
    assert_ne!(qnan & 0x03ff, 0);
    assert!(f16_bits_to_f32(qnan).is_nan());
    assert_eq!(f16_bits_to_f32(0x0000), 0.0);
    assert!(f16_bits_to_f32(0x7c00).is_infinite());
    println!("specials flush/overflow/qnan ok");
}
