//! soft_float: a 32-bit soft float, IEEE-754 binary32 in integer math.
//! Owner: engine-core (std-only; no float, no alloc, no tables).
//! Units: unitless reals; the bits are the IEEE-754 binary32 layout.
//! Determinism: integer math only (u32, u64, u128, i64); the same input gives
//! the same bits on every machine, where a hardware float path does not.
//! Accuracy: binary32 semantics with round to nearest even. `add`, `sub`,
//! `mul`, `div`, `floor`, `ceil`, `fmod` and `sqrt` are correctly rounded, so
//! they give the same bits as `f32`. `inv_sqrt` is the reciprocal of the
//! correctly rounded square root. `log2` stays within one unit in the last
//! place of the platform's own `f32::log2`.
//! Domain: the whole binary32 range, subnormals included; NaN and the
//! infinities follow IEEE-754.
//! Errors: no panics; a division by zero gives an infinity and `0/0` a NaN.
//! Budget: zero heap; every operation is a fixed number of integer steps.
use core::cmp::Ordering;

/// The exponent of a zero or a subnormal: every finite value is a multiple of
/// `2^MIN_EXP`, so this is the grid the subnormals sit on.
const MIN_EXP: i64 = -149;
/// The offset from the exponent field to the exponent of the value:
/// `value = sig * 2^(exp_field - EXP_BIAS)`.
const EXP_BIAS: i64 = 150;
/// The largest unbiased exponent of a finite value.
const MAX_EXP: i64 = 104;
/// The 23 fraction bits.
const FRAC_MASK: u32 = 0x007f_ffff;
/// The 8 exponent bits.
const EXP_MASK: u32 = 0x0000_00ff;
/// The sign bit.
const SIGN_MASK: u32 = 0x8000_0000;
/// The exponent field of an infinity or a NaN.
const SPECIAL_EXP: u32 = 0xff;
/// The quiet bit of a NaN fraction.
const QUIET_BIT: u32 = 0x0040_0000;
/// The leading significand bit of a normal value.
const HIDDEN_ONE: u32 = 0x0080_0000;
/// The significand of one: `2^23`.
const ONE_SIG: u64 = 1 << 23;
/// Fractional bits of the fixed point the logarithm series runs in.
const Q60: u32 = 60;
/// One in the logarithm's fixed point.
const ONE_Q60: u128 = 1 << Q60;
/// `2 / ln 2` in the logarithm's fixed point, rounded to nearest even.
const TWO_OVER_LN2_Q60: u128 = 3_326_628_274_461_080_623;
/// Terms of the logarithm series. `z` is under a third, so the tail after
/// this many terms is under `2^-63` of the result.
const LOG_TERMS: u32 = 17;
/// The exponent gap under which the smaller operand of a sum still reaches
/// the rounding bit of the larger one.
///
/// A gap of 27 puts the smaller operand under an eighth of the ulp of the
/// larger one. An eighth is what a power of two needs: the float below a
/// power of two sits half a step closer than the float above it, so the
/// midpoint there is under a quarter of an ulp away.
const ADD_GAP: i64 = 27;

/// A 32-bit soft float: sign, 8-bit biased exponent, 23-bit fraction.
///
/// The layout and the semantics are IEEE-754 binary32, computed with integer
/// math only, so every machine gives the same bits. The value is a 24-bit
/// signed significand (the sign, the implicit leading one and the 23 stored
/// fraction bits) times a power of two from the 8-bit exponent.
///
/// Equality is bit equality, so a NaN equals a NaN with the same bits and the
/// type stays hashable; use [`SoftFloat::is_nan`] for the IEEE check. The
/// order is total, the way `f32::total_cmp` orders: the negative NaN comes
/// first, the positive NaN comes last, and a negative zero sits below a
/// positive zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SoftFloat {
    neg: bool,
    exp: u32,
    frac: u32,
}

impl SoftFloat {
    /// Positive zero.
    pub const ZERO: Self = Self {
        neg: false,
        exp: 0,
        frac: 0,
    };
    /// Negative zero.
    pub const NEG_ZERO: Self = Self {
        neg: true,
        exp: 0,
        frac: 0,
    };
    /// One.
    pub const ONE: Self = Self {
        neg: false,
        exp: 127,
        frac: 0,
    };
    /// Minus one.
    pub const NEG_ONE: Self = Self {
        neg: true,
        exp: 127,
        frac: 0,
    };
    /// Two.
    pub const TWO: Self = Self {
        neg: false,
        exp: 128,
        frac: 0,
    };
    /// One half.
    pub const HALF: Self = Self {
        neg: false,
        exp: 126,
        frac: 0,
    };
    /// The largest finite value.
    pub const MAX: Self = Self {
        neg: false,
        exp: 254,
        frac: FRAC_MASK,
    };
    /// The smallest positive value: the smallest subnormal.
    pub const MIN_POSITIVE: Self = Self {
        neg: false,
        exp: 0,
        frac: 1,
    };
    /// The largest subnormal.
    pub const MAX_SUBNORMAL: Self = Self {
        neg: false,
        exp: 0,
        frac: FRAC_MASK,
    };
    /// Positive infinity.
    pub const INFINITY: Self = Self {
        neg: false,
        exp: SPECIAL_EXP,
        frac: 0,
    };
    /// Negative infinity.
    pub const NEG_INFINITY: Self = Self {
        neg: true,
        exp: SPECIAL_EXP,
        frac: 0,
    };
    /// A quiet NaN.
    pub const NAN: Self = Self {
        neg: false,
        exp: SPECIAL_EXP,
        frac: QUIET_BIT,
    };

    /// The IEEE-754 binary32 bit pattern of this value.
    pub const fn to_bits(self) -> u32 {
        (if self.neg { SIGN_MASK } else { 0 }) | (self.exp << 23) | self.frac
    }

    /// The float with this IEEE-754 binary32 bit pattern.
    ///
    /// Every bit pattern is a value: a signalling NaN stays a NaN, and the
    /// subnormals keep their own exponent.
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            neg: (bits & SIGN_MASK) != 0,
            exp: (bits >> 23) & EXP_MASK,
            frac: bits & FRAC_MASK,
        }
    }

    /// The float with this whole number as its value.
    ///
    /// A number too big for the binary32 range is not refused: the value
    /// rounds to the nearest float, as a float literal would.
    pub fn from_int(value: i32) -> Self {
        let neg = value < 0;
        let magnitude = value.unsigned_abs();
        Self::from_bits(pack(neg, u128::from(magnitude), 0, false))
    }

    /// True when the value is a NaN.
    pub const fn is_nan(self) -> bool {
        self.exp == SPECIAL_EXP && self.frac != 0
    }

    /// True when the value is an infinity.
    pub const fn is_infinite(self) -> bool {
        self.exp == SPECIAL_EXP && self.frac == 0
    }

    /// True when the value is finite, a zero or a subnormal included.
    pub const fn is_finite(self) -> bool {
        self.exp != SPECIAL_EXP
    }

    /// True when the value is a zero, either sign.
    pub const fn is_zero(self) -> bool {
        self.exp == 0 && self.frac == 0
    }

    /// True when the value is a subnormal, a zero excluded.
    pub const fn is_subnormal(self) -> bool {
        self.exp == 0 && self.frac != 0
    }

    /// True when the sign bit is set. Negative zero and negative NaN count.
    pub const fn is_negative(self) -> bool {
        self.neg
    }

    /// The sum, correctly rounded to the nearest binary32.
    ///
    /// A NaN operand gives a NaN, an infinity beats a finite value, and two
    /// infinities of opposite signs give a NaN. A zero of either sign adds
    /// nothing but its own sign rules.
    pub fn add(self, other: Self) -> Self {
        if self.is_nan() {
            return self.quiet();
        }
        if other.is_nan() {
            return other.quiet();
        }
        if self.is_infinite() || other.is_infinite() {
            return add_infinities(self, other);
        }
        if self.is_zero() && other.is_zero() {
            // Only two zeros of opposite signs differ: the sum is a positive
            // zero under round to nearest.
            return if self.neg == other.neg {
                self
            } else {
                Self::ZERO
            };
        }
        if self.is_zero() {
            return other;
        }
        if other.is_zero() {
            return self;
        }
        let a = Parts::split(self.to_bits());
        let b = Parts::split(other.to_bits());
        let (big, small, big_value) = if a.exp >= b.exp {
            (a, b, self)
        } else {
            (b, a, other)
        };
        let gap = big.exp - small.exp;
        if gap >= ADD_GAP {
            // The smaller operand is under a quarter of the ulp of the larger
            // one, so the sum rounds back to the larger operand.
            return big_value;
        }
        // Both operands are put on the smaller exponent, so the sum is exact.
        let wide = u128::from(big.sig) << shift_of(gap);
        let narrow = u128::from(small.sig);
        let (neg, num) = if big.neg == small.neg {
            (big.neg, wide + narrow)
        } else if wide > narrow {
            (big.neg, wide - narrow)
        } else if wide < narrow {
            (small.neg, narrow - wide)
        } else {
            // Exact cancellation: the sum is a positive zero under round to
            // nearest.
            return Self::ZERO;
        };
        Self::from_bits(pack(neg, num, small.exp, false))
    }

    /// The difference, correctly rounded to the nearest binary32.
    pub fn sub(self, other: Self) -> Self {
        self.add(other.negated())
    }

    /// The product, correctly rounded to the nearest binary32.
    pub fn mul(self, other: Self) -> Self {
        if self.is_nan() {
            return self.quiet();
        }
        if other.is_nan() {
            return other.quiet();
        }
        let neg = self.neg != other.neg;
        if self.is_infinite() || other.is_infinite() {
            if (self.is_infinite() && other.is_zero()) || (other.is_infinite() && self.is_zero()) {
                return Self::NAN;
            }
            return Self::from_bits(infinity_bits(neg));
        }
        if self.is_zero() || other.is_zero() {
            return Self::from_bits(sign_bits(neg));
        }
        let a = Parts::split(self.to_bits());
        let b = Parts::split(other.to_bits());
        Self::from_bits(pack(
            neg,
            u128::from(a.sig) * u128::from(b.sig),
            a.exp + b.exp,
            false,
        ))
    }

    /// The quotient, correctly rounded to the nearest binary32.
    ///
    /// A zero divisor gives an infinity, a zero over zero gives a NaN, and an
    /// infinity over an infinity gives a NaN.
    pub fn div(self, other: Self) -> Self {
        if self.is_nan() {
            return self.quiet();
        }
        if other.is_nan() {
            return other.quiet();
        }
        let neg = self.neg != other.neg;
        if self.is_infinite() {
            if other.is_infinite() {
                return Self::NAN;
            }
            return Self::from_bits(infinity_bits(neg));
        }
        if other.is_infinite() {
            return Self::from_bits(sign_bits(neg));
        }
        if other.is_zero() {
            if self.is_zero() {
                return Self::NAN;
            }
            return Self::from_bits(infinity_bits(neg));
        }
        if self.is_zero() {
            return Self::from_bits(sign_bits(neg));
        }
        let a = Parts::split(self.to_bits());
        let b = Parts::split(other.to_bits());
        // The dividend is lifted to a full significand so the quotient keeps
        // its bits whatever the divisor is.
        let (sig, exp) = lift(a.sig, a.exp);
        // The quotient is taken with 39 spare bits, so the rounding bit of
        // the result is far above the bits the division drops.
        let scaled = u128::from(sig) << 39;
        let divisor = u128::from(b.sig);
        let quotient = scaled / divisor;
        let rest = scaled % divisor;
        Self::from_bits(pack(neg, quotient, exp - b.exp - 39, rest != 0))
    }

    /// The largest integer that is not above this value.
    ///
    /// A value that is already an integer, an infinity and a NaN keep their
    /// own bits; a negative value rounds away from zero, so `floor(-0.5)` is
    /// `-1`.
    pub fn floor(self) -> Self {
        if self.is_nan() || self.is_infinite() || self.is_zero() {
            return self;
        }
        let p = Parts::split(self.to_bits());
        if p.exp >= 0 {
            return self;
        }
        let shift = shift_of64(-p.exp);
        let whole = p.sig >> shift;
        let rest = p.sig & mask_bits64(shift);
        // A positive value keeps the whole number; a negative one rounds away
        // from zero, so a value with a part loses one more.
        let magnitude = if p.neg && rest != 0 { whole + 1 } else { whole };
        Self::from_bits(pack(p.neg, u128::from(magnitude), 0, false))
    }

    /// The smallest integer that is not below this value.
    ///
    /// A value that is already an integer, an infinity and a NaN keep their
    /// own bits; a positive value rounds away from zero, so `ceil(0.5)` is
    /// `1`, and a negative one rounds toward zero, so `ceil(-0.5)` is a
    /// negative zero.
    pub fn ceil(self) -> Self {
        if self.is_nan() || self.is_infinite() || self.is_zero() {
            return self;
        }
        let p = Parts::split(self.to_bits());
        if p.exp >= 0 {
            return self;
        }
        let shift = shift_of64(-p.exp);
        let whole = p.sig >> shift;
        let rest = p.sig & mask_bits64(shift);
        let magnitude = if !p.neg && rest != 0 {
            whole + 1
        } else {
            whole
        };
        Self::from_bits(pack(p.neg, u128::from(magnitude), 0, false))
    }

    /// The remainder of the division, with the sign of this value.
    ///
    /// The quotient is truncated toward zero, so the remainder takes the sign
    /// of the dividend and its magnitude is under the divisor's. The result
    /// is exact: a remainder is always a representable value.
    pub fn fmod(self, other: Self) -> Self {
        if self.is_nan() || other.is_nan() {
            return Self::NAN;
        }
        if self.is_infinite() || other.is_zero() {
            return Self::NAN;
        }
        if self.is_zero() {
            return self;
        }
        let a = Parts::split(self.to_bits());
        let b = Parts::split(other.to_bits());
        if a.exp < b.exp {
            // This value is the smaller one, so it is already the remainder.
            return self;
        }
        let gap = a.exp - b.exp;
        let mut rest = a.sig % b.sig;
        for _ in 0..gap {
            rest = (rest * 2) % b.sig;
        }
        Self::from_bits(pack(a.neg, u128::from(rest), b.exp, false))
    }

    /// The square root, correctly rounded to the nearest binary32.
    ///
    /// A negative value gives a NaN, a negative zero keeps its sign, and an
    /// infinity keeps its own.
    pub fn sqrt(self) -> Self {
        if self.is_nan() {
            return self.quiet();
        }
        if self.is_zero() || self.is_infinite() {
            return self;
        }
        if self.neg {
            return Self::NAN;
        }
        let p = Parts::split(self.to_bits());
        // An even exponent splits the root into a root and a power of two.
        let (sig, exp) = if p.exp % 2 == 0 {
            (p.sig, p.exp)
        } else {
            (p.sig * 2, p.exp - 1)
        };
        // The root of the significand is an integer root of a scaled square,
        // so no float ever takes part.
        let scaled = u128::from(sig) << 80;
        let root = isqrt(scaled);
        let exact = root * root == scaled;
        Self::from_bits(pack(false, root, exp / 2 - 40, !exact))
    }

    /// The reciprocal of the square root.
    ///
    /// This is the reciprocal of the correctly rounded square root, so it
    /// gives the same bits as `1.0 / x.sqrt()`.
    pub fn inv_sqrt(self) -> Self {
        Self::ONE.div(self.sqrt())
    }

    /// The binary logarithm.
    ///
    /// The result is the correctly rounded value of `log2(x)`, to well under
    /// one unit in the last place. A negative value gives a NaN, a zero gives
    /// minus infinity, and an infinity gives itself.
    pub fn log2(self) -> Self {
        if self.is_nan() {
            return self.quiet();
        }
        if self.neg {
            return if self.is_zero() {
                Self::NEG_INFINITY
            } else {
                Self::NAN
            };
        }
        if self.is_zero() {
            return Self::NEG_INFINITY;
        }
        if self.is_infinite() {
            return self;
        }
        let p = Parts::split(self.to_bits());
        // The significand is lifted to the range [1, 2) so the fraction of
        // the logarithm is the fraction of a mantissa.
        let (sig, exp) = lift(p.sig, p.exp);
        let whole = exp + 23;
        let fraction = log2_fraction(sig);
        let (neg, magnitude) = if whole >= 0 {
            (
                false,
                (u128::try_from(whole).unwrap_or(0) << Q60) + fraction,
            )
        } else {
            let below = u128::try_from(-whole).unwrap_or(1) - 1;
            (true, (below << Q60) + (ONE_Q60 - fraction))
        };
        Self::from_bits(pack(neg, magnitude, -60, false))
    }

    /// This value with the sign flipped.
    pub const fn negated(self) -> Self {
        Self {
            neg: !self.neg,
            exp: self.exp,
            frac: self.frac,
        }
    }

    /// This value without its sign.
    pub const fn abs(self) -> Self {
        Self {
            neg: false,
            exp: self.exp,
            frac: self.frac,
        }
    }

    /// The quiet form of a NaN; any other value keeps its own bits.
    const fn quiet(self) -> Self {
        if !self.is_nan() {
            return self;
        }
        Self {
            neg: self.neg,
            exp: self.exp,
            frac: self.frac | QUIET_BIT,
        }
    }

    /// The magnitude key of the total order: the exponent field, then the
    /// fraction bits.
    const fn magnitude_key(self) -> u32 {
        (self.exp << 23) | self.frac
    }
}

impl PartialOrd for SoftFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SoftFloat {
    /// The total order: the negative NaN first, then the negative values by
    /// growing magnitude, then the positive values by shrinking magnitude,
    /// then the positive NaN.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.neg, other.neg) {
            (false, false) => self.magnitude_key().cmp(&other.magnitude_key()),
            (true, true) => other.magnitude_key().cmp(&self.magnitude_key()),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
        }
    }
}

/// A finite value as `(-1)^neg * sig * 2^exp`.
#[derive(Clone, Copy)]
struct Parts {
    neg: bool,
    sig: u64,
    exp: i64,
}

impl Parts {
    /// Split finite bits into the sign, the integer significand and the
    /// exponent. A subnormal keeps the grid exponent and has no leading one.
    fn split(bits: u32) -> Parts {
        let exp_field = (bits >> 23) & EXP_MASK;
        let frac = bits & FRAC_MASK;
        if exp_field == 0 {
            Parts {
                neg: (bits & SIGN_MASK) != 0,
                sig: u64::from(frac),
                exp: MIN_EXP,
            }
        } else {
            Parts {
                neg: (bits & SIGN_MASK) != 0,
                sig: u64::from(frac | HIDDEN_ONE),
                exp: i64::from(exp_field) - EXP_BIAS,
            }
        }
    }
}

/// The sum of two values when at least one of them is an infinity.
fn add_infinities(a: SoftFloat, b: SoftFloat) -> SoftFloat {
    if a.is_infinite() && b.is_infinite() {
        return if a.neg == b.neg { a } else { SoftFloat::NAN };
    }
    if a.is_infinite() {
        a
    } else {
        b
    }
}

/// Lift a significand into `[2^23, 2^24)` and give the exponent that keeps
/// the value. A subnormal gains its leading bits.
fn lift(sig: u64, exp: i64) -> (u64, i64) {
    if sig >= ONE_SIG {
        return (sig, exp);
    }
    let shift = 24 - bit_len64(sig);
    (sig << shift, exp - i64::from(shift))
}

/// Round `num * 2^exp` to the nearest binary32 and pack the bits.
///
/// `num` is the exact magnitude and `sticky` says that nonzero bits were
/// dropped below its least significant bit. The rounding is to nearest even,
/// with the IEEE-754 rules for the subnormal grid and for the overflow.
fn pack(neg: bool, num: u128, exp: i64, sticky: bool) -> u32 {
    if num == 0 {
        return sign_bits(neg);
    }
    let len = bit_len(num);
    let top = exp + i64::from(len) - 1;
    if top < MIN_EXP + 23 {
        return pack_subnormal(neg, num, exp, sticky);
    }
    let (mut sig, mut new_exp) = if len > 24 {
        let shift = len - 24;
        let kept = num >> shift;
        let rest = num & mask_bits(shift);
        let half = 1u128 << (shift - 1);
        let round_bit = (rest & half) != 0;
        let below = (rest & (half - 1)) != 0;
        let kept = if round_bit && (sticky || below || (kept & 1) != 0) {
            kept + 1
        } else {
            kept
        };
        (kept, exp + i64::from(shift))
    } else {
        (num << (24 - len), exp - i64::from(24 - len))
    };
    if sig == 1 << 24 {
        sig >>= 1;
        new_exp += 1;
    }
    if new_exp > MAX_EXP {
        return infinity_bits(neg);
    }
    let field = u32::try_from(new_exp + EXP_BIAS).unwrap_or(SPECIAL_EXP);
    let frac = u32::try_from(sig - u128::from(ONE_SIG)).unwrap_or(0);
    sign_bits(neg) | (field << 23) | frac
}

/// Round a value under the smallest normal onto the subnormal grid.
///
/// The grid step is `2^MIN_EXP`. A value that rounds up to the smallest
/// normal leaves the subnormal range and becomes that normal.
fn pack_subnormal(neg: bool, num: u128, exp: i64, sticky: bool) -> u32 {
    let (mut units, rest, half) = if exp >= MIN_EXP {
        (num << shift_of(exp - MIN_EXP), 0u128, 0u128)
    } else {
        let shift = shift_of(MIN_EXP - exp);
        (num >> shift, num & mask_bits(shift), 1u128 << (shift - 1))
    };
    if half != 0 {
        let round_bit = (rest & half) != 0;
        let below = (rest & (half - 1)) != 0;
        if round_bit && (sticky || below || (units & 1) != 0) {
            units += 1;
        }
    }
    if units == u128::from(ONE_SIG) {
        // The rounding carried into the smallest normal.
        return sign_bits(neg) | (1 << 23);
    }
    let frac = u32::try_from(units).unwrap_or(0);
    sign_bits(neg) | frac
}

/// The bits of a zero with the given sign.
fn sign_bits(neg: bool) -> u32 {
    if neg {
        SIGN_MASK
    } else {
        0
    }
}

/// The bits of an infinity with the given sign.
fn infinity_bits(neg: bool) -> u32 {
    sign_bits(neg) | (SPECIAL_EXP << 23)
}

/// The number of significant bits of a value.
fn bit_len(v: u128) -> u32 {
    128 - v.leading_zeros()
}

/// The number of significant bits of a 64-bit value.
fn bit_len64(v: u64) -> u32 {
    64 - v.leading_zeros()
}

/// A shift count that never reaches the width, so no shift can overflow.
fn shift_of(gap: i64) -> u32 {
    u32::try_from(gap).unwrap_or(0).min(127)
}

/// A shift count that never reaches the width of a 64-bit value.
fn shift_of64(gap: i64) -> u32 {
    u32::try_from(gap).unwrap_or(0).min(63)
}

/// The low `shift` bits set.
fn mask_bits(shift: u32) -> u128 {
    if shift == 0 {
        0
    } else if shift >= 128 {
        u128::MAX
    } else {
        (1u128 << shift) - 1
    }
}

/// The low `shift` bits of a 64-bit value.
fn mask_bits64(shift: u32) -> u64 {
    if shift == 0 {
        0
    } else if shift >= 64 {
        u64::MAX
    } else {
        (1u64 << shift) - 1
    }
}

/// The integer square root: the largest `r` with `r * r <= n`.
fn isqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut rest = n;
    let mut root = 0u128;
    let mut step = 1u128 << 126;
    while step > rest {
        step >>= 2;
    }
    while step != 0 {
        let trial = root + step;
        if rest >= trial {
            rest -= trial;
            root = (root >> 1) + step;
        } else {
            root >>= 1;
        }
        step >>= 2;
    }
    root
}

/// The fraction of the binary logarithm of a significand in `[1, 2)`.
///
/// The result is in the fixed point of [`Q60`] fractional bits. The series is
/// the fast one, `atanh(z)`, with `z` under a third, so the terms fall by a
/// ninth each and the tail is under the last bit of the fixed point.
fn log2_fraction(sig: u64) -> u128 {
    // z = (m - 1) / (m + 1) with m = sig / 2^23.
    let numerator = u128::from(sig - ONE_SIG) << Q60;
    let denominator = u128::from(sig + ONE_SIG);
    let z = div_round(numerator, denominator);
    let square = (z * z) >> Q60;
    let mut term = z;
    let mut sum = z;
    let mut k = 1u32;
    while k <= LOG_TERMS {
        term = (term * square) >> Q60;
        sum += div_round(term, u128::from(2 * k + 1));
        k += 1;
    }
    // log2(m) = 2 * atanh(z) / ln 2.
    div_round(sum * TWO_OVER_LN2_Q60, ONE_Q60)
}

/// A division rounded to nearest even.
fn div_round(numerator: u128, denominator: u128) -> u128 {
    let quotient = numerator / denominator;
    let rest = numerator % denominator;
    let twice = rest * 2;
    if twice > denominator || (twice == denominator && (quotient & 1) != 0) {
        quotient + 1
    } else {
        quotient
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;

    /// The oracle: the platform float with these bits.
    fn oracle(bits: u32) -> f32 {
        f32::from_bits(bits)
    }

    /// The bits of the platform float, for the other direction.
    fn bits_of(value: f32) -> u32 {
        value.to_bits()
    }

    /// A monotonic key for a platform float, so the distance in steps is a
    /// plain subtraction.
    fn step_key(value: f32) -> u32 {
        let bits = value.to_bits();
        if (bits & SIGN_MASK) != 0 {
            !bits
        } else {
            bits | SIGN_MASK
        }
    }

    /// The distance in representable steps between two platform floats.
    fn step_distance(a: f32, b: f32) -> u32 {
        step_key(a).abs_diff(step_key(b))
    }

    /// A spread of bit patterns: the edges, the specials and seeded values.
    fn spread() -> Vec<u32> {
        let edges = [
            0x0000_0000,
            0x8000_0000,
            0x0000_0001,
            0x8000_0001,
            0x007f_ffff,
            0x0080_0000,
            0x3f80_0000,
            0xbf80_0000,
            0x3f00_0000,
            0x4000_0000,
            0x7f7f_ffff,
            0xff7f_ffff,
            0x7f80_0000,
            0xff80_0000,
            0x7fc0_0000,
            0x7f80_0001,
            0x0080_0001,
            0x3380_0000,
            0x4b00_0000,
            0x2f80_0000,
            0x2fc0_0000,
            0x0000_0002,
            0x007f_fffe,
        ];
        let rng = crate::CounterRng::new(2040);
        let mut out: Vec<u32> = edges.to_vec();
        for index in 0..512 {
            out.push(u32::try_from(rng.sample(index)).unwrap_or(0));
        }
        // Seeded values with a random exponent and a random mantissa: these
        // are the ones that walk the rounding paths.
        for index in 0..512 {
            let word = rng.sample(index + 1024);
            let exp = u32::try_from((word >> 20) % 254).unwrap_or(127) + 1;
            let frac = u32::try_from(word & u64::from(FRAC_MASK)).unwrap_or(0);
            let sign = u32::try_from((word >> 40) & 1).unwrap_or(0) << 31;
            out.push(sign | (exp << 23) | frac);
        }
        out
    }

    /// The float `2^power`.
    fn pow2(power: i32) -> SoftFloat {
        let field = u32::try_from(power + 127).unwrap_or(127);
        SoftFloat::from_bits(field << 23)
    }

    #[test]
    fn the_bits_round_trip_through_the_exponent_range() {
        for field in 0..=255u32 {
            for frac in [0u32, 1, 0x003f_ffff, 0x0040_0000, 0x007f_ffff] {
                for sign in [0u32, SIGN_MASK] {
                    let bits = sign | (field << 23) | frac;
                    assert_eq!(
                        SoftFloat::from_bits(bits).to_bits(),
                        bits,
                        "bits {bits:#010x} did not survive the round trip"
                    );
                }
            }
        }
    }

    #[test]
    fn the_operations_do_not_care_about_the_input_order() {
        let sample = spread();
        for bits in &sample {
            let a = SoftFloat::from_bits(*bits);
            for other_bits in sample.iter().step_by(37) {
                let b = SoftFloat::from_bits(*other_bits);
                assert_eq!(
                    a.add(b).to_bits(),
                    b.add(a).to_bits(),
                    "add is not commutative for {bits:#010x}"
                );
                assert_eq!(
                    a.mul(b).to_bits(),
                    b.mul(a).to_bits(),
                    "mul is not commutative for {bits:#010x}"
                );
                // The same call twice gives the same bits.
                assert_eq!(a.add(b).to_bits(), a.add(b).to_bits());
                assert_eq!(a.mul(b).to_bits(), a.mul(b).to_bits());
            }
        }
    }

    #[test]
    fn add_sub_mul_div_match_the_f32_oracle() {
        let sample = spread();
        for bits in &sample {
            for other_bits in sample.iter().step_by(13) {
                let a = SoftFloat::from_bits(*bits);
                let b = SoftFloat::from_bits(*other_bits);
                let fa = oracle(*bits);
                let fb = oracle(*other_bits);
                let cases = [
                    ("add", a.add(b).to_bits(), fa + fb),
                    ("sub", a.sub(b).to_bits(), fa - fb),
                    ("mul", a.mul(b).to_bits(), fa * fb),
                    ("div", a.div(b).to_bits(), fa / fb),
                ];
                for (name, got, want) in cases {
                    if want.is_nan() {
                        assert!(
                            f32::from_bits(got).is_nan(),
                            "{name} of {bits:#010x} and {other_bits:#010x} did not give a NaN"
                        );
                    } else {
                        assert_eq!(
                            got,
                            bits_of(want),
                            "{name} of {bits:#010x} and {other_bits:#010x} gave different bits"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn floor_ceil_and_fmod_match_the_f32_oracle() {
        let sample = spread();
        for bits in &sample {
            for other_bits in sample.iter().step_by(17) {
                let a = SoftFloat::from_bits(*bits);
                let b = SoftFloat::from_bits(*other_bits);
                let fa = oracle(*bits);
                let fb = oracle(*other_bits);
                if fa.is_finite() {
                    assert_eq!(
                        a.floor().to_bits(),
                        bits_of(fa.floor()),
                        "floor {bits:#010x}"
                    );
                    assert_eq!(a.ceil().to_bits(), bits_of(fa.ceil()), "ceil {bits:#010x}");
                }
                if fa.is_finite() && fb.is_finite() && fb != 0.0 {
                    assert_eq!(
                        a.fmod(b).to_bits(),
                        bits_of(fa % fb),
                        "fmod {bits:#010x} and {other_bits:#010x}"
                    );
                }
            }
        }
    }

    #[test]
    fn sqrt_and_inv_sqrt_match_the_f32_oracle() {
        for bits in spread() {
            let a = SoftFloat::from_bits(bits);
            let fa = oracle(bits);
            if fa.is_finite() && fa >= 0.0 {
                assert_eq!(a.sqrt().to_bits(), bits_of(fa.sqrt()), "sqrt {bits:#010x}");
                assert_eq!(
                    a.inv_sqrt().to_bits(),
                    bits_of(1.0 / fa.sqrt()),
                    "inv_sqrt {bits:#010x}"
                );
            }
        }
    }

    #[test]
    fn log2_sits_next_to_the_f32_oracle() {
        for bits in spread() {
            let a = SoftFloat::from_bits(bits);
            let fa = oracle(bits);
            if fa.is_finite() && fa > 0.0 {
                let got = a.log2();
                let want = fa.log2();
                assert!(
                    step_distance(oracle(got.to_bits()), want) <= 1,
                    "log2 of {bits:#010x} landed {got:?} beside {want}"
                );
            }
        }
    }

    #[test]
    fn log2_is_exact_on_the_powers_of_two() {
        for power in -30i32..=30 {
            assert_eq!(
                pow2(power).log2().to_bits(),
                SoftFloat::from_int(power).to_bits(),
                "log2 of 2^{power}"
            );
        }
    }

    #[test]
    fn nan_and_infinity_follow_ieee() {
        let nan = SoftFloat::NAN;
        let inf = SoftFloat::INFINITY;
        assert!(nan.is_nan());
        assert!(!nan.is_finite());
        assert!(inf.is_infinite());
        assert!(nan.add(SoftFloat::ONE).is_nan());
        assert!(SoftFloat::ONE.add(nan).is_nan());
        assert!(nan.mul(SoftFloat::ONE).is_nan());
        assert!(inf.add(inf).is_infinite());
        assert!(inf.add(SoftFloat::NEG_INFINITY).is_nan());
        assert!(inf.sub(inf).is_nan());
        assert!(inf.mul(SoftFloat::ZERO).is_nan());
        assert_eq!(SoftFloat::ONE.div(SoftFloat::ZERO).to_bits(), inf.to_bits());
        assert!(SoftFloat::ZERO.div(SoftFloat::ZERO).is_nan());
        assert_eq!(
            SoftFloat::ZERO.sub(SoftFloat::ZERO).to_bits(),
            SoftFloat::ZERO.to_bits(),
            "a zero minus a zero is a positive zero"
        );
        assert_eq!(
            SoftFloat::NEG_ZERO.to_bits(),
            SoftFloat::from_bits(0x8000_0000).to_bits(),
            "the negative zero keeps its sign"
        );
    }

    #[test]
    fn the_subnormals_are_exact() {
        let smallest = SoftFloat::MIN_POSITIVE;
        assert_eq!(smallest.to_bits(), 0x0000_0001);
        assert!(smallest.is_subnormal());
        assert_eq!(smallest.add(smallest).to_bits(), 0x0000_0002);
        assert_eq!(
            smallest.mul(SoftFloat::TWO).to_bits(),
            0x0000_0002,
            "twice the smallest subnormal"
        );
        assert_eq!(
            SoftFloat::MAX_SUBNORMAL
                .add(SoftFloat::MIN_POSITIVE)
                .to_bits(),
            0x0080_0000,
            "the largest subnormal plus the smallest one is the smallest normal"
        );
        assert_eq!(
            SoftFloat::MAX_SUBNORMAL.mul(SoftFloat::HALF).to_bits(),
            0x0040_0000,
            "half of the largest subnormal is a tie and rounds to even"
        );
    }

    #[test]
    fn the_rounding_is_to_nearest_even() {
        // Half an ulp of one: the spacing of the floats around one is
        // `2^-23`, so half of it is `2^-24`.
        let half_ulp = SoftFloat::from_bits(0x3380_0000);
        assert_eq!(
            SoftFloat::ONE.add(half_ulp).to_bits(),
            0x3f80_0000,
            "one and a half ulp is a tie and the even significand keeps it"
        );
        // One ulp up has an odd significand, so the same tie rounds up.
        let odd = SoftFloat::from_bits(0x3f80_0001);
        assert_eq!(
            odd.add(half_ulp).to_bits(),
            0x3f80_0002,
            "the tie rounds up to the even significand"
        );
        // A hair over the tie rounds up, not down.
        let over = SoftFloat::from_bits(0x3390_0000);
        assert_eq!(
            SoftFloat::ONE.add(over).to_bits(),
            0x3f80_0001,
            "over the tie rounds up"
        );
    }

    #[test]
    fn the_order_is_total() {
        let mut values = vec![
            SoftFloat::NEG_INFINITY,
            SoftFloat::NEG_ONE,
            SoftFloat::NEG_ZERO,
            SoftFloat::ZERO,
            SoftFloat::ONE,
            SoftFloat::INFINITY,
            SoftFloat::NAN,
        ];
        values.sort();
        let order: Vec<u32> = values.iter().map(|v| v.to_bits()).collect();
        assert_eq!(
            order,
            vec![
                0xff80_0000,
                0xbf80_0000,
                0x8000_0000,
                0x0000_0000,
                0x3f80_0000,
                0x7f80_0000,
                0x7fc0_0000,
            ],
            "the total order of the sample"
        );
    }

    #[test]
    fn the_neighbours_give_the_same_bits_as_the_oracle() {
        // The rounding paths live between two neighbours, so the pairs of
        // adjacent bit patterns are walked on purpose.
        let rng = crate::CounterRng::new(2040);
        for index in 0..1024u64 {
            let word = rng.sample(index + 4096);
            let exp = u32::try_from((word >> 20) % 250).unwrap_or(127) + 2;
            let frac = u32::try_from(word & u64::from(FRAC_MASK)).unwrap_or(0);
            let bits = (exp << 23) | frac;
            for pair in [bits, bits.wrapping_add(1), bits.wrapping_sub(1)] {
                let a = SoftFloat::from_bits(pair);
                let b = SoftFloat::from_bits(pair.wrapping_add(1));
                let fa = oracle(pair);
                let fb = oracle(pair.wrapping_add(1));
                assert_eq!(
                    a.add(b).to_bits(),
                    bits_of(fa + fb),
                    "the sum of two neighbours at {pair:#010x}"
                );
                assert_eq!(
                    a.mul(b).to_bits(),
                    bits_of(fa * fb),
                    "the product of two neighbours at {pair:#010x}"
                );
                assert_eq!(
                    a.div(b).to_bits(),
                    bits_of(fa / fb),
                    "the quotient of two neighbours at {pair:#010x}"
                );
                assert_eq!(
                    a.sqrt().to_bits(),
                    bits_of(fa.sqrt()),
                    "the root of a neighbour at {pair:#010x}"
                );
            }
        }
    }

    #[test]
    fn the_operations_never_allocate() {
        let sample = spread();
        let ((), heap) = measure(|| {
            for bits in &sample {
                let a = SoftFloat::from_bits(*bits);
                let b = SoftFloat::from_bits(*bits ^ 0x3f80_0000);
                std::hint::black_box(a.add(b));
                std::hint::black_box(a.sub(b));
                std::hint::black_box(a.mul(b));
                std::hint::black_box(a.div(b));
                std::hint::black_box(a.floor());
                std::hint::black_box(a.ceil());
                std::hint::black_box(a.fmod(b));
                std::hint::black_box(a.sqrt());
                std::hint::black_box(a.inv_sqrt());
                std::hint::black_box(a.log2());
            }
        });
        assert_eq!(heap.calls, 0, "the soft float must not allocate");
    }

    #[test]
    fn the_production_half_names_no_float() {
        // The soft float itself must not use a platform float: the whole
        // point is that its bits do not depend on the machine. The oracle
        // lives in the test half only, so the scan reads the code with the
        // comments taken out and the words split apart.
        let source = include_str!("soft_float.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or("");
        let code = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<&str>>()
            .join("\n");
        for word in ["f32", "f64"] {
            let named = code
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|token| token == word);
            assert!(!named, "the production half names {word}");
        }
    }
}
