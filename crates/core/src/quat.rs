//! quat: the one quaternion product shared by animation, physics, assets,
//! scene and editor code. Quaternions are `[x, y, z, w]` arrays. The product
//! fixes its operation order here, so every caller composes rotations to the
//! same bits: no fused multiply-add and no reordering, ever.
use crate::fixed::Fixed;

/// Hamilton product `a * b` of two `[x, y, z, w]` quaternions: the result
/// rotates by `b` first, then by `a`.
#[must_use]
pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

/// Hamilton product `a * b` of two `[x, y, z, w]` quaternions in Q47.16
/// fixed point: the result rotates by `b` first, then by `a`.
///
/// Units: each component counts 1/65536 units ([`Fixed`]).
/// Determinism: integer-only with the same left-to-right order as
/// [`quat_mul`]; bit-identical on every platform. Wraps on overflow
/// like the `Fixed` operators. The `f32` product stays until the anim,
/// physics, assets, scene and editor callers migrate in a later wave.
#[must_use]
pub fn quat_mul_fixed(a: [Fixed; 4], b: [Fixed; 4]) -> [Fixed; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::{quat_mul, quat_mul_fixed};
    use crate::fixed::Fixed;

    const I: [f32; 4] = [1.0, 0.0, 0.0, 0.0];
    const J: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
    const K: [f32; 4] = [0.0, 0.0, 1.0, 0.0];
    const ONE: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    #[test]
    fn hamilton_units_follow_ij_equals_k() {
        assert_eq!(quat_mul(I, J), K);
        assert_eq!(quat_mul(J, I), [0.0, 0.0, -1.0, 0.0]);
        assert_eq!(quat_mul(I, I), [0.0, 0.0, 0.0, -1.0]);
        assert_eq!(quat_mul(ONE, K), K);
        assert_eq!(quat_mul(K, ONE), K);
    }

    #[test]
    fn product_bits_are_pinned() {
        // Expected bits come from the same left-to-right order evaluated with
        // f32 rounding after every multiply, add and subtract.
        let got = quat_mul([0.1, -0.2, 0.3, 0.9], [-0.4, 0.5, 0.25, 0.7]).map(f32::to_bits);
        assert_eq!(got, [0xbefa_e148, 0x3e28_f5c0, 0x3ecf_5c29, 0x3f31_eb86]);
    }

    fn fx(raw: [i64; 4]) -> [Fixed; 4] {
        raw.map(Fixed::from_bits)
    }

    fn bits(q: [Fixed; 4]) -> [i64; 4] {
        q.map(|v| v.to_bits())
    }

    #[test]
    fn fixed_identity_composes() {
        let one = fx([0, 0, 0, Fixed::SCALE]);
        let k = fx([0, 0, Fixed::SCALE, 0]);
        assert_eq!(quat_mul_fixed(one, k), k);
        assert_eq!(quat_mul_fixed(k, one), k);
        assert_eq!(quat_mul_fixed(one, one), one);
    }

    #[test]
    fn fixed_bits_are_pinned() {
        // a = [1, 2, 3, 4], b = [5, 6, 7, 8] as whole units; each product
        // term is exact, so the pinned bits prove the Hamilton order.
        let a = [
            Fixed::from_int(1),
            Fixed::from_int(2),
            Fixed::from_int(3),
            Fixed::from_int(4),
        ];
        let b = [
            Fixed::from_int(5),
            Fixed::from_int(6),
            Fixed::from_int(7),
            Fixed::from_int(8),
        ];
        let got = bits(quat_mul_fixed(a, b));
        assert_eq!(
            got,
            [
                24 * Fixed::SCALE,
                48 * Fixed::SCALE,
                48 * Fixed::SCALE,
                -6 * Fixed::SCALE,
            ]
        );
    }

    #[test]
    fn fixed_deterministic_x2() {
        let a = fx([6554, -13_107, 19_661, 58_983]);
        let b = fx([-26_214, 32_768, 16_384, 45_875]);
        assert_eq!(quat_mul_fixed(a, b), quat_mul_fixed(a, b));
    }

    #[test]
    fn fixed_matches_float_oracle_within_2_lsb() {
        // f32 is the test oracle only; the product above stays integer-only.
        let a = fx([6554, -13_107, 19_661, 58_983]);
        let b = fx([-26_214, 32_768, 16_384, 45_875]);
        let got = bits(quat_mul_fixed(a, b));
        let af: [f32; 4] = a.map(|v| v.to_bits() as f32 / Fixed::SCALE as f32);
        let bf: [f32; 4] = b.map(|v| v.to_bits() as f32 / Fixed::SCALE as f32);
        let oracle = quat_mul(af, bf);
        for i in 0..4 {
            let expect = (oracle[i] * Fixed::SCALE as f32).round() as i64;
            let diff = (got[i] - expect).abs();
            assert!(diff <= 2, "lane {i}: got {} oracle {expect}", got[i]);
        }
    }
}
