//! fixed: Q47.16 deterministic math, CORDIC atan2 + LUT trig, LE digest.
use crate::checked::wrap_i128_to_i64;
use core::cmp::Ordering;

/// Signed Q47.16 fixed-point number: the raw `i64` counts 1/65536 units.
///
/// Integer-only, so every platform computes identical bits; simulation code
/// uses it wherever `f32` transcendentals would diverge. Operators wrap on
/// overflow (like `wrapping_*`) and `/` panics on a zero divisor; use the
/// `checked_*` forms on untrusted input. Angles are radians.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Fixed(pub i64);
#[rustfmt::skip]
const SIN_LUT: [i64; 65] = [0,1608,3216,4821,6424,8022,9616,11204,12785,14359,15924,17479,19024,20557,22078,23586,25080,26558,28020,29466,30893,32303,33692,35062,36410,37736,39040,40320,41576,42806,44011,45190,46341,47464,48559,49624,50660,51665,52639,53581,54491,55368,56212,57022,57798,58538,59244,59914,60547,61145,61705,62228,62714,63162,63572,63944,64277,64571,64827,65043,65220,65358,65457,65516,65536];
#[rustfmt::skip]
const ATAN_T: [i64; 16] = [51472,30386,16055,8150,4091,2047,1024,512,256,128,64,32,16,8,4,2];
const TAU_RAW: i64 = 411776;
const PI_RAW: i64 = 205888;
const HALF_PI_RAW: i64 = 102944;
impl Fixed {
    /// Raw units per 1.0.
    pub const SCALE: i64 = 65536;
    /// Fraction bits.
    pub const FRAC: u32 = 16;
    /// 0.0
    pub const ZERO: Self = Self(0);
    /// 1.0
    pub const ONE: Self = Self(65536);
    /// 0.5
    pub const HALF: Self = Self(32768);
    /// -1.0
    pub const NEG_ONE: Self = Self(-65536);
    /// Largest value, about 1.4e14.
    pub const MAX: Self = Self(i64::MAX);
    /// Smallest value, about -1.4e14.
    pub const MIN: Self = Self(i64::MIN);
    /// π rounded to 1/65536.
    pub const PI: Self = Self(PI_RAW);
    /// 2π rounded to 1/65536.
    pub const TAU: Self = Self(TAU_RAW);
    /// π/2 rounded to 1/65536.
    pub const FRAC_PI_2: Self = Self(HALF_PI_RAW);
    /// π/4 rounded to 1/65536.
    pub const FRAC_PI_4: Self = Self(51472);
    /// Wrap raw Q47.16 bits.
    pub const fn from_bits(b: i64) -> Self {
        Self(b)
    }
    /// Raw Q47.16 bits.
    pub const fn to_bits(self) -> i64 {
        self.0
    }
    /// Integer `v` as fixed point; wraps beyond about ±1.4e14.
    pub const fn from_int(v: i64) -> Self {
        Self(v.wrapping_mul(Self::SCALE))
    }
    /// Integer part, truncated toward zero (`-1.5` gives `-1`).
    pub fn to_int(self) -> i64 {
        self.0 / Self::SCALE
    }
    /// Little-endian raw bytes, the canonical wire and hash form.
    pub fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }
    /// Inverse of [`Fixed::to_le_bytes`].
    pub fn from_le_bytes(b: [u8; 8]) -> Self {
        Self(i64::from_le_bytes(b))
    }
    /// Sum, wrapping on overflow (the `+` operator).
    pub fn wrapping_add(self, o: Self) -> Self {
        Self(self.0.wrapping_add(o.0))
    }
    /// Difference, wrapping on overflow (the `-` operator).
    pub fn wrapping_sub(self, o: Self) -> Self {
        Self(self.0.wrapping_sub(o.0))
    }
    /// Product rounded to the nearest 1/65536, ties away from zero, wrapping on overflow
    /// (the `*` operator). See [`round_q16`].
    pub fn wrapping_mul(self, o: Self) -> Self {
        Self(wrap_i128_to_i64(round_q16(
            i128::from(self.0) * i128::from(o.0),
        )))
    }
    /// Quotient truncated toward zero, wrapping on overflow (the `/`
    /// operator).
    ///
    /// # Panics
    /// When `o` is zero; use [`Fixed::checked_div`] on untrusted input.
    pub fn wrapping_div(self, o: Self) -> Self {
        assert!(o.0 != 0, "fixed div by zero");
        Self(wrap_i128_to_i64(
            (i128::from(self.0) << 16) / i128::from(o.0),
        ))
    }
    /// Sum, `None` on overflow.
    pub fn checked_add(self, o: Self) -> Option<Self> {
        self.0.checked_add(o.0).map(Self)
    }
    /// Difference, `None` on overflow.
    pub fn checked_sub(self, o: Self) -> Option<Self> {
        self.0.checked_sub(o.0).map(Self)
    }
    /// Product rounded to the nearest 1/65536, ties away from zero, `None` on overflow.
    pub fn checked_mul(self, o: Self) -> Option<Self> {
        i64::try_from(round_q16(i128::from(self.0) * i128::from(o.0)))
            .ok()
            .map(Self)
    }
    /// Quotient truncated toward zero, `None` on a zero divisor or overflow.
    pub fn checked_div(self, o: Self) -> Option<Self> {
        if o.0 == 0 {
            return None;
        }
        i64::try_from((i128::from(self.0) << 16) / i128::from(o.0))
            .ok()
            .map(Self)
    }
    /// Sum clamped to [`Fixed::MIN`]..=[`Fixed::MAX`].
    pub fn saturating_add(self, o: Self) -> Self {
        Self(self.0.saturating_add(o.0))
    }
    /// Difference clamped to [`Fixed::MIN`]..=[`Fixed::MAX`].
    pub fn saturating_sub(self, o: Self) -> Self {
        Self(self.0.saturating_sub(o.0))
    }
    /// Absolute value; [`Fixed::MIN`] maps to [`Fixed::MAX`] instead of wrapping.
    pub fn abs(self) -> Self {
        if self.0 == i64::MIN {
            Self(i64::MAX)
        } else if self.0 < 0 {
            Self(-self.0)
        } else {
            self
        }
    }
    /// The smaller of two values.
    pub fn min(self, o: Self) -> Self {
        if self.0 < o.0 {
            self
        } else {
            o
        }
    }
    /// The larger of two values.
    pub fn max(self, o: Self) -> Self {
        if self.0 > o.0 {
            self
        } else {
            o
        }
    }
    /// `self` limited to `lo..=hi`; `lo` wins when `lo > hi`.
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        if self.0 < lo.0 {
            lo
        } else if self.0 > hi.0 {
            hi
        } else {
            self
        }
    }
    /// Two-term blend `self * (1 - t) + o * t` with `t` clamped to
    /// `ZERO..=ONE`; each product runs in `i128` then clamps to
    /// `MIN..=MAX`, and the terms join with saturating add. Endpoints are
    /// exact: `ZERO` gives `self`, `ONE` gives `o`. Integer-only, so every
    /// platform computes identical bits; no allocation.
    pub fn lerp(self, o: Self, t: Self) -> Self {
        let tc = t.clamp(Self::ZERO, Self::ONE);
        if tc == Self::ZERO {
            return self;
        }
        if tc == Self::ONE {
            return o;
        }
        let inv = Self::ONE.0 - tc.0;
        let a = saturating_i128_to_i64(round_q16(i128::from(self.0) * i128::from(inv)));
        let b = saturating_i128_to_i64(round_q16(i128::from(o.0) * i128::from(tc.0)));
        Self(a.saturating_add(b))
    }
    /// Square root rounded down to 1/65536; zero for zero and negatives.
    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Self(0);
        }
        // Guard above proves positivity; fallbacks below are unreachable
        // and keep tick callers total instead of panicking.
        let n = u128::try_from(self.0).unwrap_or(0) * u128::try_from(Self::SCALE).unwrap_or(65_536);
        Self(i64::try_from(isqrt(n)).unwrap_or(i64::MAX))
    }
    /// Sine of an angle in radians: 64-segment quarter-wave table with
    /// linear interpolation, any input range.
    pub fn sin(self) -> Self {
        Self(sin_raw(self.0.rem_euclid(TAU_RAW)))
    }
    /// Cosine of an angle in radians (the sine table shifted by π/2).
    pub fn cos(self) -> Self {
        Self(sin_raw((self.0 + HALF_PI_RAW).rem_euclid(TAU_RAW)))
    }
    /// `(sin, cos)` of an angle in radians.
    pub fn sin_cos(self) -> (Self, Self) {
        (self.sin(), self.cos())
    }
    /// Tangent; saturates to [`Fixed::MAX`] or [`Fixed::MIN`] where the
    /// cosine is exactly zero.
    pub fn tan(self) -> Self {
        let (s, c) = self.sin_cos();
        if c.0 == 0 {
            if s.0 >= 0 {
                Self::MAX
            } else {
                Self::MIN
            }
        } else {
            s / c
        }
    }
    /// Angle of `(x, y)` in `-π..=π` radians by 16-step CORDIC; `(0, 0)`
    /// gives zero.
    pub fn atan2(y: Self, x: Self) -> Self {
        if x.0 == 0 && y.0 == 0 {
            return Self::ZERO;
        }
        if x.0 == 0 {
            return if y.0 > 0 {
                Self::FRAC_PI_2
            } else {
                Self(-HALF_PI_RAW)
            };
        }
        if y.0 == 0 {
            return if x.0 > 0 { Self::ZERO } else { Self::PI };
        }
        if x.0 > 0 {
            return Self(atan_pos(y.0, x.0));
        }
        let nx = x.0.wrapping_neg();
        if nx <= 0 {
            return if y.0 >= 0 { Self::PI } else { Self(-PI_RAW) };
        }
        let a = atan_pos(y.0, nx);
        if y.0 >= 0 {
            Self(PI_RAW - a)
        } else {
            Self(-PI_RAW - a)
        }
    }
    /// FNV-1a 64 over each value's little-endian raw bytes, in order.
    pub fn digest(vals: &[Self]) -> u64 {
        let mut hasher = crate::hash::Fnv64::new();
        for v in vals {
            hasher.write_i64(v.0);
        }
        hasher.finish()
    }
    /// Reference ordering oracle for `Fixed` vs `Fixed`, independent of the derived `Ord`.
    ///
    /// Compares raw Q47.16 bits with explicit branches so oracle vectors and
    /// the seed corpus catch an inverted or truncated compare. Same bits give
    /// the same ordering on every platform; no allocation.
    /// PORT(alloy-rs/ruint@c764018b80dce8f0b5a0262cb97846e633acee31:src/cmp.rs) shape only (reference compare oracle), no donor bytes.
    pub fn reference_cmp(a: Self, b: Self) -> Ordering {
        if a.0 < b.0 {
            Ordering::Less
        } else if a.0 > b.0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
    /// Reference ordering oracle for `Fixed` vs an integer scalar, via `i128`.
    ///
    /// Compares `self.0` against `rhs * SCALE` in `i128`, so scalars whose
    /// scaled form would overflow `i64` still order correctly and never wrap.
    /// Fractions order strictly between their neighboring integers. Same
    /// inputs give the same ordering on every platform; no allocation.
    pub fn reference_cmp_scalar(self, rhs: i64) -> Ordering {
        cmp_raw_vs_scalar_i128(self.0, i128::from(rhs))
    }
}
/// Compare raw Q47.16 bits against an integer scalar in `i128`.
///
/// The scaled scalar uses checked multiply: an overflow means the scalar
/// magnitude exceeds every `Fixed` value, so the sign alone decides. Pure
/// integer math, identical on every platform; no allocation.
fn cmp_raw_vs_scalar_i128(raw: i64, scalar: i128) -> Ordering {
    match scalar.checked_mul(i128::from(Fixed::SCALE)) {
        Some(scaled) => i128::from(raw).cmp(&scaled),
        None => {
            if scalar < 0 {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
    }
}
// Scalar compare oracle ported from alloy-rs/ruint `impl_for_primitives!`
// (PartialEq/PartialOrd vs u8..usize,i8..isize) plus the reference-compare
// shape; reimplemented fresh over Q47.16 raw bits in i128, std-only,
// no donor bytes vendored.
// PORT(alloy-rs/ruint@c764018b80dce8f0b5a0262cb97846e633acee31:src/cmp.rs)
// PORT(alloy-rs/ruint@c764018b80dce8f0b5a0262cb97846e633acee31:proptest-regressions/)
macro_rules! impl_fixed_for_primitives {
    ($($t:ty),* $(,)?) => {
        $(
            impl PartialEq<$t> for Fixed {
                fn eq(&self, other: &$t) -> bool {
                    match i128::try_from(*other) {
                        Ok(s) => cmp_raw_vs_scalar_i128(self.0, s) == Ordering::Equal,
                        // Only huge u128 positives miss i128; never equal.
                        Err(_) => false,
                    }
                }
            }
            impl PartialOrd<$t> for Fixed {
                fn partial_cmp(&self, other: &$t) -> Option<Ordering> {
                    Some(match i128::try_from(*other) {
                        Ok(s) => cmp_raw_vs_scalar_i128(self.0, s),
                        // Only huge u128 positives miss i128; Fixed is less.
                        Err(_) => Ordering::Less,
                    })
                }
            }
        )*
    };
}
#[rustfmt::skip]
impl_fixed_for_primitives!(
    u8, u16, u32, u64, u128, usize,
    i8, i16, i32, i64, i128, isize,
);
/// A Q32 product brought back to Q16: rounded to the nearest 1/65536, ties away from zero.
///
/// Odd (`round_q16(-p) == -round_q16(p)`), so mirrored motion stays mirrored bit for bit, and
/// unbiased, so magnitudes do not creep. The old `p >> 16` floored: every negative product
/// lost an extra unit, so a round fired west at 900.37 m/s landed 2 mm farther than its
/// mirror after 132 ticks and 9 mm after 10 s, and a hair strand mirrored in a gust drifted
/// 1-4 units on its first tick (2026-10-21 physics-law sweep). `|p| <= 2^126` for a product
/// of two `i64`, so the negation never overflows.
const fn round_q16(p: i128) -> i128 {
    const HALF: i128 = 1 << 15;
    if p >= 0 {
        (p + HALF) >> 16
    } else {
        -((HALF - p) >> 16)
    }
}
fn saturating_i128_to_i64(v: i128) -> i64 {
    // Narrow through the i64 window: values outside hold the bound,
    // values inside keep exact bits. The fallback below is unreachable
    // after the clamp and keeps tick callers total instead of panicking.
    let windowed = v.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    i64::try_from(windowed).unwrap_or(0)
}
/// Integer square root of `n`, rounded down: the largest `r` with `r * r <= n`. Exact on every
/// platform, no float.
pub fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
fn lut(off: i64) -> i64 {
    if off <= 0 {
        return 0;
    }
    if off >= HALF_PI_RAW {
        return Fixed::SCALE;
    }
    let p = off * 64;
    // Guards above bound the quotient to 0..64; the fallback below is
    // unreachable and keeps tick callers total instead of panicking.
    let idx = usize::try_from(p / HALF_PI_RAW).unwrap_or(0);
    let fr = p % HALF_PI_RAW;
    let a = SIN_LUT[idx];
    let b = SIN_LUT[idx + 1];
    a + (b - a) * fr / HALF_PI_RAW
}
fn sin_raw(r: i64) -> i64 {
    match r / HALF_PI_RAW {
        0 => lut(r),
        1 => lut(PI_RAW - r),
        2 => -lut(r - PI_RAW),
        _ => -lut(TAU_RAW - r),
    }
}
fn atan_pos(y: i64, x: i64) -> i64 {
    let mut vx = x;
    let mut vy = y;
    let mut va = 0i64;
    let mut i = 0u32;
    while i < 16 {
        let sx = vx >> i;
        let sy = vy >> i;
        // Loop bound proves 0..16; the fallback below is unreachable
        // and keeps tick callers total instead of panicking.
        let a = ATAN_T[usize::try_from(i).unwrap_or(0)];
        if vy >= 0 {
            vx = vx.wrapping_add(sy);
            vy = vy.wrapping_sub(sx);
            va = va.wrapping_add(a);
        } else {
            vx = vx.wrapping_sub(sy);
            vy = vy.wrapping_add(sx);
            va = va.wrapping_sub(a);
        }
        i += 1;
    }
    va
}
impl std::ops::Add for Fixed {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        self.wrapping_add(o)
    }
}
impl std::ops::Sub for Fixed {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        self.wrapping_sub(o)
    }
}
impl std::ops::Mul for Fixed {
    type Output = Self;
    fn mul(self, o: Self) -> Self {
        self.wrapping_mul(o)
    }
}
impl std::ops::Div for Fixed {
    type Output = Self;
    fn div(self, o: Self) -> Self {
        self.wrapping_div(o)
    }
}
impl std::ops::Neg for Fixed {
    type Output = Self;
    fn neg(self) -> Self {
        Self(self.0.wrapping_neg())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn products_round_to_nearest_and_mirror_exactly() {
        // Ties and near-ties on both signs: 0.5 ulp rounds away from zero, 0.5 ulp minus one
        // rounds toward it, and a negated factor always negates the product.
        let half_ulp = Fixed::from_bits(1) * Fixed::from_bits(32_768);
        assert_eq!(half_ulp.to_bits(), 1);
        assert_eq!(
            (Fixed::from_bits(-1) * Fixed::from_bits(32_768)).to_bits(),
            -1
        );
        assert_eq!(
            (Fixed::from_bits(1) * Fixed::from_bits(32_767)).to_bits(),
            0
        );
        assert_eq!(
            (Fixed::from_bits(-1) * Fixed::from_bits(32_767)).to_bits(),
            0
        );
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..10_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let a = Fixed::from_bits(i64::from_le_bytes(seed.to_le_bytes()) >> 20);
            let b = Fixed::from_bits(i64::from_le_bytes(seed.rotate_left(29).to_le_bytes()) >> 30);
            assert_eq!((-a) * b, -(a * b), "{a:?} * {b:?}");
            assert_eq!(a.checked_mul(-b), (a * b).checked_mul(-Fixed::ONE));
            let exact = i128::from(a.to_bits()) * i128::from(b.to_bits());
            let err = i128::from((a * b).to_bits()) * 65_536 - exact;
            assert!(err.abs() <= 32_768, "{a:?} * {b:?} off by {err}/65536 ulp");
        }
    }

    #[test]
    fn a_mirrored_fall_stays_mirrored_to_the_bit() {
        // Semi-implicit Euler with drag, the shape every Q16 sim here runs: east and west
        // starts must stay exact negatives (the floored product broke this on tick 1).
        let dt = Fixed::from_bits(65_536 / 60);
        let drag = Fixed::from_bits(983);
        let step = |s: &mut (Fixed, Fixed), _: u32| {
            s.1 = s.1 - s.1 * drag;
            s.0 = s.0 + s.1 * dt;
        };
        let east = (Fixed::ZERO, Fixed::from_bits(59_008_811));
        let west = (Fixed::ZERO, Fixed::from_bits(-59_008_811));
        crate::laws::mirror(east, west, 600, step, |a, b| a.0 == -b.0 && a.1 == -b.1)
            .expect("a mirrored round stays mirrored");
    }
    #[test]
    fn q47_scale() {
        assert_eq!(Fixed::from_int(1).to_bits(), 65536);
        assert_eq!(Fixed::from_int(-2).to_bits(), -131072);
        assert_eq!(Fixed::from_bits(98304).to_int(), 1);
        assert_eq!(Fixed::from_bits(-98304).to_int(), -1);
        assert_eq!(
            Fixed::from_le_bytes(Fixed::from_int(3).to_le_bytes()),
            Fixed::from_int(3)
        );
    }
    #[test]
    fn arith_exact() {
        assert_eq!(
            Fixed::from_bits(98304) * Fixed::from_bits(147456),
            Fixed::from_bits(221184)
        );
        assert_eq!(
            Fixed::from_int(3) / Fixed::from_int(2),
            Fixed::from_bits(98304)
        );
        assert_eq!(Fixed::from_int(2) + Fixed::from_int(3), Fixed::from_int(5));
        assert_eq!(Fixed::from_int(2) - Fixed::from_int(5), Fixed::from_int(-3));
        assert!(Fixed::from_int(1).checked_div(Fixed::ZERO).is_none());
    }
    #[test]
    fn sqrt_exact() {
        assert_eq!(Fixed::from_int(4).sqrt(), Fixed::from_int(2));
        assert_eq!(Fixed::from_int(1).sqrt(), Fixed::ONE);
        assert_eq!(Fixed::ZERO.sqrt(), Fixed::ZERO);
        assert_eq!(Fixed::from_int(-1).sqrt(), Fixed::ZERO);
        assert_eq!(Fixed::from_int(2).sqrt().to_bits(), 92681);
    }
    #[test]
    fn trig_quadrants() {
        assert_eq!(Fixed::ZERO.sin(), Fixed::ZERO);
        assert_eq!(Fixed::FRAC_PI_2.sin(), Fixed::ONE);
        assert_eq!(Fixed::ZERO.cos(), Fixed::ONE);
        assert_eq!(Fixed::FRAC_PI_2.cos(), Fixed::ZERO);
        assert_eq!(Fixed::PI.sin(), Fixed::ZERO);
        assert_eq!(Fixed::PI.cos(), Fixed::NEG_ONE);
        assert_eq!(Fixed::TAU.sin(), Fixed::ZERO);
    }
    #[test]
    fn trig_30deg() {
        let s = Fixed::from_bits(34315).sin();
        let c = Fixed::from_bits(34315).cos();
        assert!(
            (s - Fixed::HALF).abs().to_bits() < 200,
            "sin30={}",
            s.to_bits()
        );
        assert!(
            (c - Fixed::from_bits(56756)).abs().to_bits() < 300,
            "cos30={}",
            c.to_bits()
        );
    }
    #[test]
    fn atan2_quadrants() {
        assert_eq!(Fixed::atan2(Fixed::ZERO, Fixed::ONE), Fixed::ZERO);
        assert_eq!(Fixed::atan2(Fixed::ONE, Fixed::ZERO), Fixed::FRAC_PI_2);
        assert_eq!(Fixed::atan2(Fixed::ZERO, Fixed::NEG_ONE), Fixed::PI);
        assert_eq!(
            Fixed::atan2(Fixed::NEG_ONE, Fixed::ZERO),
            Fixed::from_bits(-102944)
        );
        assert_eq!(Fixed::atan2(Fixed::ZERO, Fixed::ZERO), Fixed::ZERO);
        let d = (Fixed::atan2(Fixed::ONE, Fixed::ONE) - Fixed::FRAC_PI_4)
            .abs()
            .to_bits();
        assert!(d < 200, "atan45 diff={}", d);
        let e = (Fixed::atan2(Fixed::ONE, Fixed::NEG_ONE) - Fixed::from_bits(154416))
            .abs()
            .to_bits();
        assert!(e < 300, "atan135 diff={}", e);
    }
    #[test]
    fn digest_golden() {
        let v = [Fixed::from_int(1), Fixed::from_int(2)];
        assert_eq!(Fixed::digest(&v), 4601385034970360190);
        assert_eq!(Fixed::digest(&v), Fixed::digest(&v));
        assert_ne!(Fixed::digest(&v), Fixed::digest(&[Fixed::from_int(1)]));
    }
    #[test]
    fn lerp_endpoints_and_mid() {
        let a = Fixed::from_int(2);
        let b = Fixed::from_int(6);
        assert_eq!(a.lerp(b, Fixed::ZERO), a);
        assert_eq!(a.lerp(b, Fixed::ONE), b);
        assert_eq!(a.lerp(b, Fixed::HALF), Fixed::from_int(4));
        assert_eq!(a.lerp(b, Fixed::from_bits(16384)), Fixed::from_int(3));
    }
    #[test]
    fn lerp_clamps_t() {
        let a = Fixed::from_int(2);
        let b = Fixed::from_int(6);
        assert_eq!(a.lerp(b, Fixed::from_int(-1)), a);
        assert_eq!(a.lerp(b, Fixed::from_int(2)), b);
        assert_eq!(a.lerp(b, Fixed::from_bits(-1)), a);
        assert_eq!(a.lerp(b, Fixed::from_bits(65537)), b);
    }
    #[test]
    fn lerp_overflow_edges() {
        assert_eq!(Fixed::MIN.lerp(Fixed::MAX, Fixed::ZERO), Fixed::MIN);
        assert_eq!(Fixed::MIN.lerp(Fixed::MAX, Fixed::ONE), Fixed::MAX);
        // Old wrap form `self + (o - self) * t` gives MAX here; the
        // two-term blend gives the convex midpoint. The exact midpoint is
        // raw -0.5: the floored products gave -1, the nearest-rounded ones
        // give 0, the same in both directions.
        assert_eq!(Fixed::MIN.lerp(Fixed::MAX, Fixed::HALF), Fixed::ZERO);
        assert_eq!(Fixed::MAX.lerp(Fixed::MIN, Fixed::HALF), Fixed::ZERO);
    }
    #[test]
    fn saturating_i128_holds_bounds_where_cast_would_wrap() {
        // `as i64` keeps low bits past the bound (wraps to MIN); the
        // saturating helper holds the bound instead.
        assert_eq!(
            super::saturating_i128_to_i64(i128::from(i64::MAX) + 1),
            i64::MAX
        );
        assert_eq!(
            super::saturating_i128_to_i64(i128::from(i64::MIN) - 1),
            i64::MIN
        );
        assert_eq!(super::saturating_i128_to_i64(0), 0);
        assert_eq!(
            super::saturating_i128_to_i64(i128::from(i32::MAX)),
            i64::from(i32::MAX)
        );
    }
}
