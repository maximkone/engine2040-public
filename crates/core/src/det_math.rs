//! det_math: integer-only trig and exp, always compiled in.
//! Owner: engine-core (std-only; no float, no alloc).
//! Units: Fixed Q47.16 raw i64; angles radians, exp unitless.
//! Determinism: i128 integer math only; identical bits on every machine.
//! Accuracy: `det_sin`, `det_cos` and `det_exp` are correctly rounded (nearest
//! raw unit) apart from inputs whose true value sits within about 1e-11 of a
//! rounding tie; the exhaustive tests below compare every raw input of a
//! period (and of the whole exp range) with an `f64` reference. `det_atan2za`
//! is the CORDIC path of `Fixed::atan2`, not correctly rounded.
//! Domain: angles clamp to +-2^40 raw (about 1.7e7 rad) so the reduction fits
//! i128. Errors: saturates outward; no panics. Budget: zero heap; no tables.
use crate::fixed::Fixed;

/// Pi in Q96 (96 fractional bits), re-derived by `constants_match_series`.
const PI_Q96: i128 = 248_902_613_312_231_085_230_521_944_622;
const TAU_Q96: i128 = 2 * PI_Q96;
/// Pi is even in Q96, so half of it is exact.
const HALF_PI_Q96: i128 = PI_Q96 / 2;
/// ln 2 in Q96, re-derived by `constants_match_series`.
const LN2_Q96: i128 = 54_916_777_467_707_473_351_141_471_128;
/// Fractional bits of the series arithmetic (products stay below 2^127).
const SERIES_BITS: u32 = 56;
const ONE: i128 = 1 << SERIES_BITS;
/// Q96 to series Q56, and series Q56 to Fixed Q16.
const TO_SERIES: i128 = 1 << (96 - SERIES_BITS);
const FROM_SERIES: i128 = 1 << (SERIES_BITS - 16);
/// Raw angle clamp: keeps `k * TAU_Q96` inside i128.
const LIMIT_RAW: i64 = 1 << 40;

/// Saturate i128 to i64 for narrowings.
fn sat_i128(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// Round-to-nearest division with a positive divisor; ties away from zero.
fn div_round(a: i128, b: i128) -> i128 {
    if a >= 0 {
        a.wrapping_add(b / 2) / b
    } else {
        a.wrapping_sub(b / 2) / b
    }
}

/// Series-Q56 multiply with round-to-nearest.
fn mul_series(a: i128, b: i128) -> i128 {
    div_round(a.wrapping_mul(b), ONE)
}

/// Sine Taylor through r^25 on |r| <= pi/2; the cut term is below 2^-60.
fn sin_series(r: i128) -> i128 {
    let r2 = mul_series(r, r);
    let mut term = r;
    let mut sum = r;
    for n in (1_i128..25).step_by(2) {
        term = div_round(term.wrapping_mul(r2), (n + 1) * (n + 2) * ONE).wrapping_neg();
        sum = sum.wrapping_add(term);
    }
    sum
}

/// Exp Taylor through r^22 on |r| <= ln2 / 2; the cut term is below 2^-70.
fn exp_series(r: i128) -> i128 {
    let mut term = ONE;
    let mut sum = ONE;
    for n in 1_i128..23 {
        term = div_round(term.wrapping_mul(r), n * ONE);
        sum = sum.wrapping_add(term);
    }
    sum
}

/// Sine of a Q96 angle: exact reduction by whole turns, fold to +-pi/2, series.
fn sin_q96(angle: i128) -> Fixed {
    let turns = div_round(angle, TAU_Q96);
    let mut r = angle.wrapping_sub(turns.wrapping_mul(TAU_Q96));
    if r > HALF_PI_Q96 {
        r = PI_Q96.wrapping_sub(r);
    } else if r < -HALF_PI_Q96 {
        r = (-PI_Q96).wrapping_sub(r);
    }
    let s = sin_series(div_round(r, TO_SERIES));
    Fixed::from_bits(sat_i128(div_round(s, FROM_SERIES)))
}

/// A Fixed angle as Q96, clamped to the supported domain.
fn angle_q96(x: Fixed) -> i128 {
    i128::from(x.0.clamp(-LIMIT_RAW, LIMIT_RAW)) << 80
}

/// Deterministic sine in radians, correctly rounded.
pub fn det_sin(x: Fixed) -> Fixed {
    sin_q96(angle_q96(x))
}

/// Deterministic cosine in radians, correctly rounded: the quarter turn is
/// added to the exact angle, never to a rounded Fixed.
pub fn det_cos(x: Fixed) -> Fixed {
    sin_q96(angle_q96(x).wrapping_add(HALF_PI_Q96))
}

/// Deterministic pair; same units and guarantees as the single forms.
pub fn det_sin_cos(x: Fixed) -> (Fixed, Fixed) {
    (det_sin(x), det_cos(x))
}

/// Deterministic exp, correctly rounded; saturates outward past about +-21.1 (30.5 ln 2).
pub fn det_exp(x: Fixed) -> Fixed {
    let xq = angle_q96(x);
    let k = div_round(xq, LN2_Q96);
    if k > 30 {
        return Fixed::MAX;
    }
    if k < -30 {
        return Fixed::ZERO;
    }
    let r = div_round(xq.wrapping_sub(k.wrapping_mul(LN2_Q96)), TO_SERIES);
    let e = exp_series(r);
    let raw = if k >= 0 {
        div_round(e << k, FROM_SERIES)
    } else {
        div_round(e, FROM_SERIES << k.unsigned_abs())
    };
    Fixed::from_bits(sat_i128(raw))
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
            assert!((got - want).abs() <= 1, "sin {got} vs {want}");
        }
        for (x, want) in coss {
            let got = det_cos(x).to_bits();
            assert!((got - want).abs() <= 1, "cos {got} vs {want}");
        }
        for (x, want) in exps {
            let got = det_exp(x).to_bits();
            assert_eq!(got, want, "exp {x:?}");
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

    /// Pi and ln 2 in Q110, from integer series that share nothing with the
    /// constants above (Machin's atan formula and 2 atanh(1/3)).
    fn series_constants() -> (i128, i128) {
        const ONE110: i128 = 1 << 110;
        let atan_inv = |n: i128| {
            let mut term = ONE110 / n;
            let mut sum = term;
            let mut k = 1;
            while term > 0 {
                term /= n * n;
                let t = term / (2 * k + 1);
                sum += if k % 2 == 0 { t } else { -t };
                k += 1;
            }
            sum
        };
        let pi = 16 * atan_inv(5) - 4 * atan_inv(239);
        let (mut term, mut ln2, mut k) = (ONE110 / 3, ONE110 / 3, 1);
        while term > 0 {
            term /= 9;
            ln2 += term / (2 * k + 1);
            k += 1;
        }
        (pi, 2 * ln2)
    }

    #[test]
    fn constants_match_series() {
        let (pi, ln2) = series_constants();
        assert!((PI_Q96 - div_round(pi, 1 << 14)).abs() <= 1, "pi in Q96");
        assert!((LN2_Q96 - div_round(ln2, 1 << 14)).abs() <= 1, "ln2 in Q96");
        assert_eq!(TAU_Q96, PI_Q96 * 2);
        assert_eq!(HALF_PI_Q96 * 2, PI_Q96, "pi is even in Q96");
    }

    /// `want` rounded to the raw unit, or `None` when the exact value sits too
    /// close to a tie for an f64 reference to decide (its error is a few 1e-16
    /// of the value, so large results leave more ties undecided).
    fn nearest_raw(want: f64) -> Option<i64> {
        let scaled = want * 65_536.0;
        let rounded = scaled.round();
        if (scaled - rounded).abs() > 0.5 - 1e-9 - scaled.abs() * 4e-16 {
            None
        } else {
            Some(rounded as i64)
        }
    }

    /// Count the inputs whose result is not the correctly rounded value of
    /// `reference` (ties the reference cannot decide are counted apart).
    fn wrong_and_ties(
        inputs: std::ops::RangeInclusive<i64>,
        got: impl Fn(Fixed) -> Fixed,
        reference: impl Fn(f64) -> f64,
    ) -> (u32, u32) {
        let (mut wrong, mut ties) = (0, 0);
        for raw in inputs {
            match nearest_raw(reference(raw as f64 / 65_536.0)) {
                Some(want) if want != got(Fixed::from_bits(raw)).to_bits() => wrong += 1,
                Some(_) => {}
                None => ties += 1,
            }
        }
        (wrong, ties)
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "libm is the independent test oracle here, never part of the sim"
    )]
    fn sin_cos_correctly_rounded_over_two_periods() {
        // Every raw input of two periods: both signs, both folds, the wrap.
        let span = -411_776..=411_776;
        let (wrong, ties) = wrong_and_ties(span.clone(), det_sin, f64::sin);
        assert_eq!((wrong, ties < 8), (0, true), "sin ties {ties}");
        let (wrong, ties) = wrong_and_ties(span, det_cos, f64::cos);
        assert_eq!((wrong, ties < 8), (0, true), "cos ties {ties}");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "libm is the independent test oracle here, never part of the sim"
    )]
    fn exp_correctly_rounded_over_the_whole_range() {
        // 30.5 ln 2 = 21.14 is the saturation edge; every raw input inside it.
        let (wrong, ties) = wrong_and_ties(-1_385_000..=1_385_000, det_exp, f64::exp);
        assert_eq!((wrong, ties < 10_000), (0, true), "exp ties {ties}");
        assert_eq!(
            det_exp(Fixed::from_int(22)),
            Fixed::MAX,
            "saturates outward"
        );
        assert_eq!(det_exp(Fixed::from_int(-22)), Fixed::ZERO);
        assert_eq!(det_exp(Fixed::MIN), Fixed::ZERO);
        assert_eq!(det_exp(Fixed::MAX), Fixed::MAX);
    }

    #[test]
    fn huge_angles_clamp_without_panic() {
        for raw in [i64::MIN, i64::MAX, LIMIT_RAW, -LIMIT_RAW, LIMIT_RAW + 1] {
            let x = Fixed::from_bits(raw);
            assert!(det_sin(x).to_bits().abs() <= 65_536);
            assert!(det_cos(x).to_bits().abs() <= 65_536);
        }
        assert_eq!(
            det_sin(Fixed::from_bits(i64::MAX)),
            det_sin(Fixed::from_bits(LIMIT_RAW))
        );
    }

    /// FNV digest of all four functions over `n` seeded inputs.
    fn corpus_digest(n: u64) -> u64 {
        let rng = crate::CounterRng::new(2040);
        let mut h = crate::Fnv64::new();
        for i in 0..n {
            let a = (rng.sample(i) % 1_647_105) as i64 - 823_552;
            let e = (rng.sample(i.wrapping_add(1_000_003)) % 2_726_001) as i64 - 1_363_000;
            h.write_i64(det_sin(Fixed::from_bits(a)).to_bits());
            h.write_i64(det_cos(Fixed::from_bits(a)).to_bits());
            h.write_i64(det_exp(Fixed::from_bits(e)).to_bits());
            h.write_i64(det_atan2za(Fixed::from_bits(a), Fixed::from_bits(e)).to_bits());
        }
        h.finish()
    }

    /// The 1M-input corpus digest, recorded on Windows x86-64 (MSVC) and
    /// Linux x86-64 (GitHub runner); it must read the same on every machine.
    const CORPUS_1M_DIGEST: u64 = 967_702_734_373_494_140;

    #[test]
    fn corpus_1m_bit_identical_twice_and_pinned() {
        let first = corpus_digest(1_000_000);
        assert_eq!(first, corpus_digest(1_000_000), "twice per build");
        assert_eq!(first, CORPUS_1M_DIGEST, "pinned across toolchains");
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
