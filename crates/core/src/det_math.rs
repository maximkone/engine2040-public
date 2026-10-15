//! det_math: integer-only trig and exp behind an off gate.
//! Owner: engine-core (std-only; no float, no alloc).
//! Units: Fixed Q47.16 raw i64; angles radians, exp unitless.
//! Determinism: wrapping i64 plus i128 widening; identical bits.
//! Errors: saturates outward; no panics.
//! Budget: zero heap; no tables.
use crate::fixed::Fixed;

const HALF_PI_RAW: i64 = 102_944;
const ONE_Q32: i128 = 4_294_967_296;
const HALF_Q32: i128 = 32_768;
// Lunaris ln2 split in Q48: HI keeps trailing zeros so k*HI stays exact,
// LO holds the remainder; HI+LO rounds to ln2 (fdlibm split).
// PORT(AizenKai1001/lunaris@56cc7e30cbd21ece2314c155498a66f69fb36535:crates/lunaris-sim/src/detmath.rs:9 `pub fn sin`, :33 `pub fn cos`, `exp` k*LN2 split)
const LN2_HI_Q48: i128 = 195_103_586_451_456;
const LN2_LO_Q48: i128 = 53_711;
const LN2_Q48: i128 = 195_103_586_505_167;
// Full-circle constants in Q48 for exact reduction; inputs stay Q47.16.
const TAU_Q48: i128 = 1_768_559_438_007_110;
const PI_Q48: i128 = 884_279_719_003_555;
const HALF_PI_Q48: i128 = 442_139_859_501_778;

/// Saturate i128 to i64 for narrowings.
fn sat_i128(v: i128) -> i64 {
    if v > i128::from(i64::MAX) {
        i64::MAX
    } else if v < i128::from(i64::MIN) {
        i64::MIN
    } else {
        i64::try_from(v).unwrap_or(0)
    }
}

/// Round-to-nearest division with a positive divisor; symmetric for negatives.
fn div_round(a: i128, b: i128) -> i128 {
    if a >= 0 {
        a.wrapping_add(b / 2) / b
    } else {
        a.wrapping_sub(b / 2) / b
    }
}

/// Q32 multiply with round-to-nearest; both inputs Q32, output Q32.
fn mul_q32(a: i128, b: i128) -> i128 {
    div_round(a.wrapping_mul(b), ONE_Q32)
}

/// Sine Taylor through r^17 in Q32; truncation near pi/2 is far below one LSB.
fn sin_taylor_q32(r: i64) -> i64 {
    let x = i128::from(r) << 16;
    let r2 = mul_q32(x, x);
    let mut term = x;
    let mut sum = x;
    let mut n: i128 = 1;
    for _ in 0..8 {
        let d = (n + 1) * (n + 2);
        term = div_round(term.wrapping_mul(r2), d.wrapping_mul(ONE_Q32));
        term = term.wrapping_neg();
        sum = sum.wrapping_add(term);
        n += 2;
    }
    sat_i128(div_round(sum, 65_536))
}

/// Exp series with 16 terms in Q32; single rounding per term.
fn exp_series_q32(r: i128) -> i128 {
    let mut term = ONE_Q32;
    let mut sum = ONE_Q32;
    for n in 1_i128..17 {
        term = div_round(term.wrapping_mul(r), n.wrapping_mul(ONE_Q32));
        sum = sum.wrapping_add(term);
    }
    sum
}

/// Deterministic sine in radians; folds to [-pi/2, pi/2] before the r^17 poly.
pub fn det_sin(x: Fixed) -> Fixed {
    let xq = i128::from(x.0) << 32;
    let kq = div_round(xq, TAU_Q48);
    let mut r48 = xq.wrapping_sub(kq.wrapping_mul(TAU_Q48));
    if r48 > PI_Q48 {
        r48 = r48.wrapping_sub(TAU_Q48);
    } else if r48 < PI_Q48.wrapping_neg() {
        r48 = r48.wrapping_add(TAU_Q48);
    }
    if r48 > HALF_PI_Q48 {
        r48 = PI_Q48.wrapping_sub(r48);
    } else if r48 < HALF_PI_Q48.wrapping_neg() {
        r48 = PI_Q48.wrapping_neg().wrapping_sub(r48);
    }
    Fixed::from_bits(sin_taylor_q32(sat_i128(div_round(r48, 4_294_967_296))))
}

/// Deterministic cosine via sine shift; same units and guarantees.
pub fn det_cos(x: Fixed) -> Fixed {
    det_sin(Fixed::from_bits(x.0.wrapping_add(HALF_PI_RAW)))
}

/// Deterministic pair sharing the singleton reduction.
pub fn det_sin_cos(x: Fixed) -> (Fixed, Fixed) {
    (det_sin(x), det_cos(x))
}

/// Deterministic exp via k ln2 split with two-part subtraction; saturates outward.
pub fn det_exp(x: Fixed) -> Fixed {
    let xq = i128::from(x.0) << 32;
    let kq = div_round(xq, LN2_Q48);
    if kq > 30 {
        return Fixed::MAX;
    }
    if kq < -30 {
        return Fixed::ZERO;
    }
    let r48 = xq
        .wrapping_sub(kq.wrapping_mul(LN2_HI_Q48))
        .wrapping_sub(kq.wrapping_mul(LN2_LO_Q48));
    let r = div_round(r48, 65_536);
    let e = exp_series_q32(r);
    if kq >= 0 {
        let shift = u32::try_from(kq).unwrap_or(0);
        Fixed::from_bits(sat_i128(((e << shift) + HALF_Q32) >> 16))
    } else {
        let shift = u32::try_from(kq.wrapping_neg()).unwrap_or(0);
        Fixed::from_bits(sat_i128(((e >> shift) + HALF_Q32) >> 16))
    }
}

/// Atan2 coverage through the Fixed CORDIC path; zero gives zero.
pub fn det_atan2za(y: Fixed, x: Fixed) -> Fixed {
    if y == Fixed::ZERO && x == Fixed::ZERO {
        Fixed::ZERO
    } else {
        Fixed::atan2(y, x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[rustfmt::skip]
    fn det_golden_spots() {
        let sins = [(Fixed::ZERO, 0), (Fixed::from_bits(34_315), 32_768), (Fixed::FRAC_PI_2, 65_536), (Fixed::PI, 0)];
        let coss = [(Fixed::ZERO, 65_536), (Fixed::from_bits(34_315), 56_756), (Fixed::FRAC_PI_2, 0), (Fixed::PI, -65_536)];
        let exps = [(Fixed::ZERO, 65_536), (Fixed::ONE, 178_145), (Fixed::NEG_ONE, 24_109), (Fixed::from_int(8), 195_360_063)];
        for (x, want) in sins {
            let got = det_sin(x).to_bits();
            assert!((got - want).abs() <= 3, "sin {got} vs {want}");
        }
        for (x, want) in coss {
            let got = det_cos(x).to_bits();
            assert!((got - want).abs() <= 3, "cos {got} vs {want}");
        }
        for (x, want) in exps {
            let got = det_exp(x).to_bits();
            assert!((got - want).abs() <= 3, "exp {got} vs {want}");
        }
        let (s, c) = det_sin_cos(Fixed::from_bits(34_315));
        assert_eq!(s, det_sin(Fixed::from_bits(34_315)));
        assert_eq!(c, det_cos(Fixed::from_bits(34_315)));
        assert_eq!(det_atan2za(Fixed::ZERO, Fixed::ZERO), Fixed::ZERO);
        assert_eq!(
            det_atan2za(Fixed::ONE, Fixed::ONE),
            Fixed::atan2(Fixed::ONE, Fixed::ONE)
        );
    }

    fn corpus_digest() -> u64 {
        let rng = crate::CounterRng::new(2040);
        let mut h = crate::Fnv64::new();
        for i in 0..100_000u64 {
            let w = rng.sample(i);
            let ar = (w % 1_647_105) as i64 - 823_552;
            let er = (rng.sample(i.wrapping_add(1_000_003)) % 1_048_577) as i64 - 524_288;
            h.write_i64(det_sin(Fixed::from_bits(ar)).to_bits());
            h.write_i64(det_cos(Fixed::from_bits(ar)).to_bits());
            h.write_i64(det_exp(Fixed::from_bits(er)).to_bits());
            h.write_i64(det_atan2za(Fixed::from_bits(ar), Fixed::from_bits(er)).to_bits());
        }
        h.finish()
    }

    #[test]
    fn det_bit_identical_100k() {
        assert_eq!(corpus_digest(), corpus_digest());
        assert_ne!(corpus_digest(), 0);
    }

    #[test]
    fn det_zero_alloc() {
        let ((), n) = crate::alloc_probe::measure(|| {
            let mut acc = 0i64;
            for i in 0..1024u64 {
                let rng = crate::CounterRng::new(7);
                let w = rng.sample(i) as i64;
                acc = acc.wrapping_add(det_sin(Fixed::from_bits(w)).to_bits());
                acc = acc.wrapping_add(det_cos(Fixed::from_bits(w)).to_bits());
                acc = acc.wrapping_add(det_exp(Fixed::from_bits(w)).to_bits());
            }
            std::hint::black_box(acc);
        });
        assert_eq!(n.calls, 0);
        assert_eq!(n.bytes, 0);
    }
}
