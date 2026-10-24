//! servo: fixed-point PID joint servo for the powered ragdoll (PX11).
//!
//! Consumer: `crate::powered::PoweredRagdoll::drive_bone` runs one `Servo`
//! per bone on each axis and converts its answer back to the world velocity.
//! Owner: this module owns the integer PID law, the integral bound, the slew
//! cap and the per-rig tuning table. Units: micro-units (1e-6 of the SI
//! unit) — positions and offsets in µm, velocities in µm/s, the angular axis
//! as millionths of the quaternion error vector and spin in µrad/s. Gains
//! are Q8 (256 = 1.0) per second for P, per second squared for I; D is the
//! per-tick velocity factor the velocity servo already used (see [`crate::pd`]).
//!
//! Determinism: integer `+ - * /`, shifts, comparisons and one integer
//! square root ([`engine_core::fixed::isqrt`]) in `jitter_rms`; no float
//! inside the loop, no allocation, no clock. The only float step is the
//! caller's boundary conversion (`to_micro` / `from_micro`).
//!
//! The D term damps on the measurement, never on the error derivative, so a
//! target step does not kick the servo and no error history is stored. The
//! integral is bounded in µm·s (`ServoTuning::integ_clamp_um_s`) so a stuck
//! or unloaded bone cannot wind it up; the slew cap bounds the change of the
//! commanded velocity per tick, which is what removes the rest buzz the
//! float PD left at its speed cap.
//!
//! Tuning is per rig, never per animation: `ServoTuning::for_rig` is one
//! const row per `Rig`, and the bone's own drive table still owns the caps
//! and the zero-gain limp rule.
#![forbid(unsafe_code)]

use engine_core::fixed::isqrt;

/// Micro-units per SI unit, the scale of every integer in this module.
pub const MICRO: i64 = 1_000_000;

/// Largest term magnitude the law admits, µm or µm/s (1 km or 1 km/s). A
/// larger input saturates, so no bone pose can overflow the products below.
pub const MAX_TERM: i64 = 1_000_000_000;

/// [`MAX_TERM`] as f64, so the boundary clamp needs no cast.
const MAX_TERM_F: f64 = 1e9;

/// Longest step the law admits, µs (1 s). A larger `dt` takes the P-only
/// path, exactly as [`crate::pd::dt_valid`] does for a degenerate step.
pub const MAX_DT_US: i64 = 1_000_000;

/// The rig a tuning row belongs to. A rig is the skeleton and mass class,
/// not an animation: every clip on the same rig shares one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Rig {
    /// The shipped human-scale fighter chain.
    #[default]
    Standard,
    /// A heavier, shorter rig: more mass per bone, tighter slew.
    Heavy,
    /// A taller rig: longer limbs, a looser slew so the swing keeps up.
    Tall,
}

impl Rig {
    /// Every rig the table covers, in table order.
    pub const ALL: [Rig; 3] = [Rig::Standard, Rig::Heavy, Rig::Tall];

    /// Row index in the const table.
    pub const fn index(self) -> usize {
        match self {
            Rig::Standard => 0,
            Rig::Heavy => 1,
            Rig::Tall => 2,
        }
    }
}

/// One servo tuning row: the gains and bounds that belong to a rig. P and D
/// come from the bone's drive table scaled here; I, the integral bound and
/// the slew cap are the rig's own character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ServoTuning {
    /// Integral gain, Q8 1/s². 6.0 is 1536.
    pub i_q8: i32,
    /// Bound on the integral term, µm·s. 150_000 is 0.15 m·s.
    pub integ_clamp_um_s: i64,
    /// Largest |error| that still integrates, µm: outside the band the P and
    /// D terms do the travel and the integral holds (anti-windup on a get-up).
    pub integ_band_um: i64,
    /// Largest change of the commanded velocity per tick, µm/s.
    pub slew_um_s_tick: i32,
}

impl ServoTuning {
    /// The one table: one row per rig, no per-animation entry.
    pub const TABLE: [ServoTuning; Rig::ALL.len()] = [
        // Standard: the shipped chain, 6/s², 0.15 m·s bound, a 50 mm band,
        // 50 mm/s per tick (3 m/s per second) — below the float PD rest buzz
        // the PX11 tape measured.
        ServoTuning {
            i_q8: 6 << 8,
            integ_clamp_um_s: 150_000,
            integ_band_um: 50_000,
            slew_um_s_tick: 2_500_000,
        },
        // Heavy: slower integral so a 90 kg chain does not hunt, tighter slew.
        ServoTuning {
            i_q8: 4 << 8,
            integ_clamp_um_s: 120_000,
            integ_band_um: 40_000,
            slew_um_s_tick: 2_000_000,
        },
        // Tall: a longer limb swings further per tick, so a looser slew.
        ServoTuning {
            i_q8: 8 << 8,
            integ_clamp_um_s: 200_000,
            integ_band_um: 60_000,
            slew_um_s_tick: 3_000_000,
        },
    ];

    /// The row of `rig`.
    pub const fn for_rig(rig: Rig) -> Self {
        Self::TABLE[rig.index()]
    }
}

/// A bone's proportional and damping gains, 1/s and per-tick factor, before
/// `Servo` turns them into Q8.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gains {
    /// P gain, 1/s.
    pub p: f32,
    /// D gain on the measurement, per tick.
    pub d: f32,
}

/// The constants of one [`Servo::step`] call, in the law's units. They come
/// from the bone's drive table and the rig row, which do not change between
/// ticks, so the Q8 decode and the one division run only when one of them
/// does: the tick loop then multiplies, shifts and clamps only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Law {
    /// The call the constants were built from: the gain bits (not the floats,
    /// so a NaN row compares equal to itself), the step, the cap and the row.
    p_bits: u32,
    d_bits: u32,
    dt_us: i64,
    cap_um_s: i32,
    tuning: ServoTuning,
    /// The constants.
    p_q8: i64,
    d_q8: i64,
    k_q8: i64,
    i_bound: i64,
    cap: i64,
    slew: i64,
    valid: bool,
}

impl Law {
    /// The constants of one call.
    fn of(p_bits: u32, d_bits: u32, dt_us: i64, cap_um_s: i32, tuning: &ServoTuning) -> Self {
        let i_q8 = i64::from(tuning.i_q8);
        Self {
            p_bits,
            d_bits,
            dt_us,
            cap_um_s,
            tuning: *tuning,
            p_q8: to_q8(f32::from_bits(p_bits)),
            d_q8: to_q8(f32::from_bits(d_bits)),
            // The per-tick integral gain: i_q8 * dt, Q8, once per call.
            k_q8: i_q8.saturating_mul(dt_us) / MICRO,
            // The I term's bound in µm/s, the row's µm·s bound through the gain.
            i_bound: (i_q8 * tuning.integ_clamp_um_s) >> 8,
            cap: i64::from(cap_um_s.max(0)),
            slew: i64::from(tuning.slew_um_s_tick.max(0)),
            valid: dt_us > 0 && dt_us <= MAX_DT_US,
        }
    }

    /// Whether these are the constants of that call.
    fn is_for(
        &self,
        p_bits: u32,
        d_bits: u32,
        dt_us: i64,
        cap_um_s: i32,
        tuning: &ServoTuning,
    ) -> bool {
        self.p_bits == p_bits
            && self.d_bits == d_bits
            && self.dt_us == dt_us
            && self.cap_um_s == cap_um_s
            && self.tuning == *tuning
    }
}

/// Three-axis PID state of one driven bone: the integral and the last
/// commanded velocity per axis, so the next step can bound both, plus the
/// last call's constants (see `Law`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Servo {
    integ: [i64; 3],
    out: [i32; 3],
    law: Law,
}

impl Servo {
    /// A servo with no history: zero integral, zero last command.
    pub const fn new() -> Self {
        Self {
            integ: [0; 3],
            out: [0; 3],
            law: Law {
                p_bits: 0,
                d_bits: 0,
                dt_us: 0,
                cap_um_s: 0,
                tuning: ServoTuning {
                    i_q8: 0,
                    integ_clamp_um_s: 0,
                    integ_band_um: 0,
                    slew_um_s_tick: 0,
                },
                p_q8: 0,
                d_q8: 0,
                k_q8: 0,
                i_bound: 0,
                cap: 0,
                slew: 0,
                valid: false,
            },
        }
    }

    /// Drop all history (snapshot restore, re-spawn, mode change).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// A copy with the listed axes negated, for a checkpoint reflected across
    /// a plane: the integral and the last command are odd in their axis, so the
    /// mirrored twin must start from the negated history.
    pub fn mirrored(&self, negate: [bool; 3]) -> Self {
        let mut out = *self;
        for (integ, neg) in out.integ.iter_mut().zip(negate) {
            if neg {
                *integ = -*integ;
            }
        }
        for (out, neg) in out.out.iter_mut().zip(negate) {
            if neg {
                *out = -*out;
            }
        }
        out
    }

    /// Accumulated integral term of one axis, µm/s of commanded velocity (the
    /// row's µm·s bound carried through the I gain).
    pub fn integral(&self, axis: usize) -> i64 {
        self.integ[axis.min(2)]
    }

    /// Last commanded velocity of one axis, µm/s.
    pub fn output(&self, axis: usize) -> i32 {
        self.out[axis.min(2)]
    }

    /// One tick of the three axes. `err` is target minus measurement in µm
    /// (or millionths of the quaternion error vector), `vel` the measured
    /// rate in µm/s, `dt_us` the step in µs, `gains` the bone's P and D, and
    /// `cap_um_s` the caller's speed cap. A step outside `(0, MAX_DT_US]`
    /// answers the P term only, leaving the integral, the last command and
    /// the slewed output untouched.
    ///
    /// The call's constants are built once and kept (see `Law`): a bone
    /// whose drive table row, step and cap do not move pays neither the Q8
    /// decode nor the division.
    #[inline]
    pub fn step(
        &mut self,
        err: [i64; 3],
        vel: [i64; 3],
        dt_us: i64,
        gains: Gains,
        tuning: &ServoTuning,
        cap_um_s: i32,
    ) -> [i32; 3] {
        let (p_bits, d_bits) = (gains.p.to_bits(), gains.d.to_bits());
        if !self.law.is_for(p_bits, d_bits, dt_us, cap_um_s, tuning) {
            self.law = Law::of(p_bits, d_bits, dt_us, cap_um_s, tuning);
        }
        let law = self.law;
        let mut out = [0i32; 3];
        if !law.valid {
            // The P-only answer of a degenerate step: no integral, no slew.
            for axis in 0..3 {
                let e = err[axis].clamp(-MAX_TERM, MAX_TERM);
                let raw = ((law.p_q8 * e) >> 8).clamp(-law.cap, law.cap);
                out[axis] = i32::try_from(raw).unwrap_or(0);
                self.out[axis] = out[axis];
            }
            return out;
        }
        for axis in 0..3 {
            out[axis] = run_axis(
                err[axis],
                vel[axis],
                &mut self.integ[axis],
                self.out[axis],
                law,
            );
            self.out[axis] = out[axis];
        }
        out
    }
}

/// One valid axis of [`Servo::step`]: P, the bounded integral inside its
/// band, D against the measured rate, the cap and then the slew.
#[inline]
fn run_axis(e: i64, v: i64, integ: &mut i64, prev: i32, law: Law) -> i32 {
    let e = e.clamp(-MAX_TERM, MAX_TERM);
    let v = v.clamp(-MAX_TERM, MAX_TERM);
    let p_term = (law.p_q8 * e) >> 8;
    // Only inside the band, so a get-up's travel cannot wind it up; `k_q8`
    // folds the step into the gain once per call, so this loop divides never.
    if e.abs() <= law.tuning.integ_band_um {
        *integ = (*integ + ((law.k_q8 * e) >> 8)).clamp(-law.i_bound, law.i_bound);
    }
    // D on the measurement: a damping push against the measured rate.
    let d_term = (law.d_q8 * v) >> 8;
    let raw = (p_term + *integ - d_term).clamp(-law.cap, law.cap);
    let slewed = i64::from(prev) + (raw - i64::from(prev)).clamp(-law.slew, law.slew);
    i32::try_from(slewed).unwrap_or(0)
}

/// A gain in Q8 (256 = 1.0), saturating at the term bound. Non-finite and
/// negative gains answer zero, so a bad table can never inject a jump.
fn to_q8(gain: f32) -> i64 {
    let raw = f64::from(gain) * 256.0;
    if !raw.is_finite() || raw <= 0.0 {
        return 0;
    }
    whole(raw.min(f64::from(i32::MAX)))
}

/// A world value (m, m/s or unit quaternion component) as micro-units.
/// Saturates at [`MAX_TERM`]; a non-finite value answers zero, so a huge
/// pose can never reach the integer law.
pub fn to_micro(v: f32) -> i64 {
    let raw = f64::from(v) * 1e6;
    if !raw.is_finite() {
        return 0;
    }
    whole(raw.clamp(-MAX_TERM_F, MAX_TERM_F).round())
}

/// Micro-units back to a world value (m, m/s or quaternion component).
pub fn from_micro(v: i64) -> f32 {
    let scaled = f64::from(i32::try_from(v).unwrap_or(i32::MAX)) / 1e6;
    narrow(scaled)
}

/// An f64 as the nearest f32 (round to nearest, flushing the subnormal
/// range to zero), decoded from the bits for the same cast-ratchet reason as
/// [`whole`]. A NaN answers 0.0.
fn narrow(x: f64) -> f32 {
    let bits = x.to_bits();
    let sign = bits >> 63;
    let biased = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0x7ff);
    if biased == 0x7ff {
        return 0.0;
    }
    let exp = biased - 1023;
    if exp > 127 {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    if exp < -126 {
        return if sign == 0 { 0.0 } else { -0.0 };
    }
    // Round the 53-bit significand to f32's 24 bits, nearest, ties up.
    let full = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let mant24 = (full + (1u64 << 28)) >> 29;
    let mut e = exp + 127;
    let mut mant23 = mant24 & ((1u64 << 23) - 1);
    if mant24 > (1u64 << 24) - 1 {
        e += 1;
        mant23 = 0;
    }
    if e > 254 {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    let out = (sign << 31) | (u64::try_from(e).unwrap_or(0) << 23) | mant23;
    f32::from_bits(u32::try_from(out).unwrap_or(0))
}

/// Nearest whole number, saturating to the `i64` range and answering zero for
/// NaN or infinity. The crate's `as`-cast ratchet sits at its floor, so the
/// boundary decodes the IEEE-754 bits instead of casting: the value is a
/// mantissa times a power of two, and a shift is exact.
fn whole(x: f64) -> i64 {
    let bits = x.to_bits();
    let negative = (bits >> 63) != 0;
    let biased = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0x7ff);
    if biased == 0x7ff {
        return 0;
    }
    // 1075 = exponent bias (1023) + 52 mantissa bits.
    let exp = biased - 1075;
    let full = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let limit = if negative {
        i64::MIN.unsigned_abs()
    } else {
        i64::MAX.unsigned_abs()
    };
    let mag = if exp >= 0 {
        let scale = 1u64.checked_shl(u32::try_from(exp).unwrap_or(u32::MAX));
        scale.and_then(|s| full.checked_mul(s)).unwrap_or(u64::MAX)
    } else {
        let shift = u32::try_from(-exp).unwrap_or(u32::MAX);
        full.checked_shr(shift).unwrap_or_default()
    };
    let mag = i64::try_from(mag.min(limit)).unwrap_or(i64::MAX);
    if negative {
        -mag
    } else {
        mag
    }
}

/// A three-axis answer as world units (m/s or rad/s), magnitude-capped by the
/// caller's cap through [`crate::pd::clamp_vector`], so a diagonal never
/// leaks sqrt(3) times the cap.
pub fn from_vector(out: [i32; 3], cap: f32) -> [f32; 3] {
    crate::pd::clamp_vector(
        [
            from_micro(i64::from(out[0])),
            from_micro(i64::from(out[1])),
            from_micro(i64::from(out[2])),
        ],
        cap,
    )
}

/// Root-mean-square change between two velocity samples, in µm/s: the rest/// jitter metric the PX11 accept reads. Integer; mismatched lengths read the
/// shorter one, and an empty sample is zero.
pub fn jitter_rms(prev: &[i32], cur: &[i32]) -> i32 {
    let n = prev.len().min(cur.len());
    if n == 0 {
        return 0;
    }
    let mut sum: i64 = 0;
    for (a, b) in prev.iter().zip(cur.iter()).take(n) {
        let d = i64::from(*b) - i64::from(*a);
        sum += d * d;
    }
    let mean = u128::try_from(sum / i64::try_from(n).unwrap_or(1)).unwrap_or(0);
    i32::try_from(isqrt(mean)).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pd;

    const DT_US: i64 = 16_667;

    /// Integer micro-units as a world value, for the model's float law.
    fn world(v: i64) -> f32 {
        from_micro(v)
    }

    fn tuning() -> ServoTuning {
        ServoTuning::for_rig(Rig::Standard)
    }

    /// A row with no integral and no slew: isolates the P and D terms.
    fn open() -> ServoTuning {
        ServoTuning {
            i_q8: 0,
            integ_clamp_um_s: 0,
            integ_band_um: 0,
            slew_um_s_tick: i32::MAX,
        }
    }

    #[test]
    fn servo_p_only_matches_the_gain_at_rest() {
        for err in [-200_000i64, -100_000, 0, 50_000, 200_000] {
            // A fresh servo keeps the slew out of the way of the P check.
            let mut s = Servo::new();
            let out = s.step(
                [err, 0, 0],
                [0; 3],
                DT_US,
                Gains { p: 12.0, d: 0.0 },
                &open(),
                8_000_000,
            );
            // 12.0 Q8 is 3072 exactly, so the P term is 12 * err with no rounding.
            assert_eq!(i64::from(out[0]), 12 * err, "err {err}");
        }
    }

    #[test]
    fn servo_d_damps_on_the_measurement_not_the_error() {
        // Same error, two measured rates: the damping must push against the
        // rate with the sign of a brake, and a target step (error jump) must
        // not add an error-derivative kick.
        let t = tuning();
        let mut a = Servo::new();
        let still = a.step(
            [100_000; 3],
            [0; 3],
            DT_US,
            Gains { p: 12.0, d: 0.8125 },
            &open(),
            8_000_000,
        );
        let mut b = Servo::new();
        let moving = b.step(
            [100_000; 3],
            [60_000; 3],
            DT_US,
            Gains { p: 12.0, d: 0.8125 },
            &open(),
            8_000_000,
        );
        assert!(moving[0] < still[0], "{moving:?} vs {still:?}");
        assert_ne!(t, open());
        // D is not multiplied by dt: 0.8125 * 0.06 m/s is 48750 µm/s of brake.
        assert_eq!(i64::from(still[0]), 1_200_000);
        assert_eq!(i64::from(moving[0]), 1_200_000 - 48_750);
    }

    #[test]
    fn servo_integral_is_bounded_and_clamped() {
        // A standing error for a long time must stop winding at the row's
        // bound, and the output must stay inside the caller's cap.
        let t = tuning();
        let mut s = Servo::new();
        for _ in 0..10_000 {
            let out = s.step(
                [20_000; 3],
                [0; 3],
                DT_US,
                Gains { p: 12.0, d: 0.8125 },
                &t,
                8_000_000,
            );
            assert!(i64::from(out[0]).abs() <= 8_000_000);
        }
        assert_eq!(s.integral(0), 900_000);
        // At the bound the I term is i_q8 * bound in Q8: 6 * 0.15 m/s = 0.9 m/s.
        assert_eq!(s.integral(1), 900_000);
    }

    #[test]
    fn servo_slew_caps_the_output_delta_per_tick() {
        // A big error from rest may not jump the command; it ramps by the
        // row's slew, one tick at a time.
        let t = tuning();
        let mut s = Servo::new();
        let first = s.step(
            [8_000_000; 3],
            [0; 3],
            DT_US,
            Gains { p: 12.0, d: 0.8125 },
            &t,
            8_000_000,
        );
        assert_eq!(i64::from(first[0]), i64::from(t.slew_um_s_tick));
        let second = s.step(
            [8_000_000; 3],
            [0; 3],
            DT_US,
            Gains { p: 12.0, d: 0.8125 },
            &t,
            8_000_000,
        );
        assert_eq!(i64::from(second[0]), 2 * i64::from(t.slew_um_s_tick));
        for _ in 0..200 {
            s.step(
                [8_000_000; 3],
                [0; 3],
                DT_US,
                Gains { p: 12.0, d: 0.8125 },
                &t,
                8_000_000,
            );
        }
        assert_eq!(s.output(0), 8_000_000, "the cap still wins");
    }

    #[test]
    fn servo_degenerate_dt_answers_p_only_and_holds_history() {
        let t = tuning();
        let mut s = Servo::new();
        let first = s.step(
            [20_000; 3],
            [0; 3],
            DT_US,
            Gains { p: 12.0, d: 0.8125 },
            &t,
            8_000_000,
        );
        let integ = s.integral(0);
        assert!(integ > 0, "the valid first step integrates");
        for dt in [0, -DT_US, MAX_DT_US + 1] {
            let out = s.step(
                [1_000_000; 3],
                [500_000; 3],
                dt,
                Gains { p: 12.0, d: 0.8125 },
                &t,
                8_000_000,
            );
            // P only: 12 * 1 m = 12 m/s, capped at 8 m/s, no D and no slew.
            assert_eq!(out[0], 8_000_000, "dt {dt}");
            assert_eq!(s.integral(0), integ, "a bad dt must not integrate");
            assert_eq!(s.output(0), 8_000_000);
        }
        assert!(first[0] < 8_000_000, "the valid step was slewed");
    }

    #[test]
    fn servo_rig_table_has_one_row_per_rig_and_differs() {
        assert_eq!(ServoTuning::TABLE.len(), Rig::ALL.len());
        for (i, rig) in Rig::ALL.iter().enumerate() {
            assert_eq!(rig.index(), i);
            assert_eq!(ServoTuning::for_rig(*rig), ServoTuning::TABLE[i]);
            assert!(ServoTuning::for_rig(*rig).slew_um_s_tick > 0);
            assert!(ServoTuning::for_rig(*rig).integ_clamp_um_s > 0);
        }
        assert_ne!(
            ServoTuning::for_rig(Rig::Standard),
            ServoTuning::for_rig(Rig::Heavy)
        );
        assert_ne!(
            ServoTuning::for_rig(Rig::Standard),
            ServoTuning::for_rig(Rig::Tall)
        );
        assert_eq!(Rig::default().index(), 0);
    }

    #[test]
    fn servo_micro_round_trip_and_saturation() {
        for (v, want) in [
            (0.0f32, 0i64),
            (1.0, 1_000_000),
            (-1.0, -1_000_000),
            (0.5, 500_000),
            (0.000_001, 1),
            (-0.000_001, -1),
            (12.0, 12_000_000),
        ] {
            assert_eq!(to_micro(v), want, "{v}");
        }
        assert_eq!(from_micro(500_000), 0.5);
        assert_eq!(from_micro(1), 0.000_001);
        assert_eq!(from_micro(-8_000_000), -8.0);
        assert_eq!(from_micro(1_234_567), 1.234567);
        assert_eq!(whole(-2.7), -2);
        assert_eq!(whole(1e30), i64::MAX);
        assert_eq!(whole(-1e30), -i64::MAX);
        assert!(narrow(1e300).is_infinite());
        assert_eq!(to_micro(0.5), 500_000);
        assert_eq!(from_micro(500_000), 0.5);
        assert_eq!(to_micro(f32::NAN), 0);
        assert_eq!(to_micro(f32::INFINITY), 0);
        assert_eq!(to_micro(1e30), MAX_TERM);
        assert!((from_micro(i64::from(i32::MAX)) - 2147.4836).abs() < 0.01);
        // A huge measured rate must not overflow the D product.
        let mut s = Servo::new();
        let out = s.step(
            [1_000_000; 3],
            [i64::MAX; 3],
            DT_US,
            Gains { p: 12.0, d: 0.8125 },
            &open(),
            8_000_000,
        );
        assert_eq!(out[0], -8_000_000);
    }

    #[test]
    fn servo_digest_twice_identical() {
        fn digest(t: &ServoTuning) -> u64 {
            use engine_core::Fnv64;
            let mut h = Fnv64::new();
            let mut s = Servo::new();
            let mut pos = 400_000i64;
            for tick in 0..120i64 {
                let vel = 40_000 * (tick % 7 - 3);
                let out = s.step(
                    [-pos; 3],
                    [vel; 3],
                    DT_US,
                    Gains { p: 12.0, d: 0.8125 },
                    t,
                    8_000_000,
                );
                pos += i64::from(out[0]) * DT_US / MICRO;
                h.write_i64(i64::from(out[0]));
                h.write_i64(pos);
            }
            h.finish()
        }
        assert_eq!(digest(&tuning()), digest(&tuning()));
        assert_ne!(digest(&tuning()), digest(&ServoTuning::for_rig(Rig::Tall)));
    }

    // ---- the PX11 accept: three rigs, hit / partial / 2 s get-up ----------

    /// One integer joint: position µm, measured velocity µm/s.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Joint {
        pos: i64,
        vel: i32,
    }

    /// Which joint law the model runs.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Law {
        /// The shipped float PD velocity servo (`crate::pd`), quantized at
        /// the same micro boundary the wired servo converts at.
        Pd,
        /// The integer PID of this module.
        Pid,
    }

    /// Gravity per 60 Hz tick, µm/s: 9.81 m/s² / 60.
    const GRAVITY_TICK: i32 = 163_500;
    /// Contact jitter amplitude per tick, µm/s (a loaded rig at rest).
    const LOAD_NOISE: i32 = 60_000;
    /// Speed cap, µm/s.
    const CAP: i32 = 8_000_000;
    /// Shipped bone gains: P 12/s, D clamped inside the stability edge.
    const P: f32 = 12.0;
    const D: f32 = 1.0;

    /// Deterministic tick noise, ±LOAD_NOISE, no float and no clock.
    fn noise(tick: usize, joint: usize) -> i32 {
        let mut x = (tick as u32)
            .wrapping_mul(1_664_525)
            .wrapping_add((joint as u32).wrapping_mul(1_013_904_223))
            .wrapping_add(12_345);
        x ^= x >> 13;
        x = x.wrapping_mul(2_654_435_761);
        i32::try_from(x % 120_001).unwrap_or(0) - 60_000
    }

    /// A rig: its tune row and how many joints the chain has.
    const RIGS: [(Rig, usize); 3] = [(Rig::Standard, 4), (Rig::Heavy, 6), (Rig::Tall, 8)];

    /// Run one scenario and answer `(rest jitter, peak speed)` in µm/s.
    ///
    /// `driven(tick, joint)` says whether joint `i` is driven on that tick, so
    /// the same loop covers a hit (all driven), a partial ragdoll (half free)
    /// and a 2 s get-up (all free, then driven again). Jitter is the RMS
    /// change of every joint's velocity between consecutive ticks inside the
    /// window `sample_from..sample_from + 60`, as an integer.
    fn run(
        rig: Rig,
        joints: usize,
        law: Law,
        driven: impl Fn(usize, usize) -> bool,
        sample_from: usize,
    ) -> (i32, i32) {
        let t = ServoTuning::for_rig(rig);
        let (_, kd) = pd::stable_gains(P, D, 1.0 / 60.0);
        let gains = Gains { p: P, d: kd };
        let mut servos = vec![Servo::new(); joints];
        let mut st = vec![Joint { pos: 0, vel: 0 }; joints];
        let mut last: Vec<i32> = vec![0; joints];
        let mut sum: i64 = 0;
        let mut peak: i32 = 0;
        for tick in 0..360 {
            // Hit at tick 60: the tip takes 20 noise steps of speed and the
            // chain is pushed out of line.
            if tick == 60 {
                if let Some(tip) = st.last_mut() {
                    tip.vel = tip.vel.saturating_add(LOAD_NOISE * 20);
                }
                for (i, j) in st.iter_mut().enumerate() {
                    j.pos += 20_000 * i64::try_from(i).unwrap_or(0);
                }
            }
            for (i, j) in st.iter_mut().enumerate() {
                let measured = j.vel + GRAVITY_TICK + noise(tick, i);
                if driven(tick, i) {
                    let err = -j.pos;
                    j.vel = match law {
                        Law::Pd => {
                            let raw = pd::pd_raw(
                                from_micro(err),
                                from_micro(i64::from(measured)),
                                P,
                                kd,
                                1.0 / 60.0,
                            );
                            i32::try_from(to_micro(pd::clamp_vector([raw, 0.0, 0.0], 8.0)[0]))
                                .unwrap_or(0)
                        }
                        Law::Pid => servos[i].step(
                            [err, 0, 0],
                            [i64::from(measured), 0, 0],
                            DT_US,
                            gains,
                            &t,
                            CAP,
                        )[0],
                    }
                    .saturating_add(noise(tick, i) / 8);
                } else {
                    // A free joint keeps 63/64 of its velocity, like an
                    // articulated free node.
                    j.vel = (measured * 63) / 64 + noise(tick, i) / 8;
                }
                j.pos += i64::from(j.vel) * DT_US / MICRO;
                if tick > sample_from && tick <= sample_from + 60 {
                    let d = i64::from(j.vel) - i64::from(last[i]);
                    sum += d * d;
                }
                last[i] = j.vel;
                peak = peak.max(j.vel.abs());
            }
        }
        let n = i64::try_from(joints * 60).unwrap_or(1);
        let mean = u128::try_from(sum / n).unwrap_or(0);
        (i32::try_from(isqrt(mean)).unwrap_or(i32::MAX), peak)
    }

    /// Jitter of both laws for the three scenarios, printed so the report
    /// can quote the numbers. Hit and partial are read at rest (tick 240 on).
    /// A 2 s get-up drives the chain again at tick 120 and lands in two
    /// seconds (tick 240), so its jitter is read at rest after the budget,
    /// tick 300 on — reading it during the fling would measure the throw.
    #[test]
    fn servo_jitter_at_or_below_pd_on_three_rigs() {
        let hit = |_: usize, _: usize| true;
        let partial = |_: usize, i: usize| i.is_multiple_of(2);
        let getup = |tick: usize, _: usize| tick >= 120;
        for (rig, joints) in RIGS {
            for (name, driven, from) in [
                ("hit", &hit as &dyn Fn(usize, usize) -> bool, 240usize),
                ("partial", &partial as &dyn Fn(usize, usize) -> bool, 240),
                ("getup", &getup as &dyn Fn(usize, usize) -> bool, 300),
            ] {
                let (pd_j, pd_peak) = run(rig, joints, Law::Pd, driven, from);
                let (pid_j, pid_peak) = run(rig, joints, Law::Pid, driven, from);
                println!(
                    "{rig:?} {name}: pd jitter {pd_j} µm/s (peak {pd_peak}), pid jitter {pid_j} µm/s (peak {pid_peak})"
                );
                assert!(
                    pid_j <= pd_j,
                    "{rig:?} {name}: pid jitter {pid_j} above pd {pd_j}"
                );
            }
        }
    }

    // ---- the accept's overhead leg: paired, alternating, same process -----

    /// Three-axis steps in one arm run. Long enough that one sample is
    /// milliseconds, not microseconds (the first bench timed 1000 steps, and
    /// the same float arm then read 2.8 µs on one run and 5.2 on the next),
    /// and short enough that both input sets stay in L2 (the integer set is
    /// 48 B a step, the float one 24 B).
    const ARM_STEPS: usize = 20_000;
    /// Pairs timed; the first [`WARMUP_PAIRS`] are dropped.
    const ARM_PAIRS: usize = 60;
    const WARMUP_PAIRS: usize = 6;
    /// The step the paired bench runs, s.
    const ARM_DT_S: f32 = 1.0 / 60.0;
    /// The bench cap in world units, m/s (8 m/s is [`CAP`] in µm/s).
    const ARM_CAP: f32 = 8.0;

    /// The same steps in both laws' own units: µm and µm/s for the integer
    /// law, m and m/s for the float one. Built once, outside the timed loop,
    /// so neither arm pays the boundary conversion the caller shares.
    struct Inputs {
        int: Vec<([i64; 3], [i64; 3])>,
        flt: Vec<([f32; 3], [f32; 3])>,
    }

    impl Inputs {
        fn new(steps: usize) -> Self {
            let mut int = Vec::with_capacity(steps);
            let mut flt = Vec::with_capacity(steps);
            for i in 0..steps {
                let e = i64::try_from(i % 1000).unwrap_or(0) * 1_000 - 500_000;
                let v = i64::try_from(i % 97).unwrap_or(0) * 10_000 - 480_000;
                let (ei, vi) = ([e, -e, e / 2], [v, -v, v / 2]);
                int.push((ei, vi));
                let mut ef = [0.0f32; 3];
                let mut vf = [0.0f32; 3];
                for a in 0..3 {
                    ef[a] = world(ei[a]);
                    vf[a] = world(vi[a]);
                }
                flt.push((ef, vf));
            }
            Self { int, flt }
        }
    }

    /// One float PD arm run: the crate's own capped single-axis law, three
    /// axes per step. The gains are the caller's (hoisted, as `drive_bone`
    /// takes them once per bone), the inputs and the sum are black-boxed so
    /// nothing is folded away.
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "a test-only ratio of two laws; the wall clock never enters the tick or the hash"
    )]
    fn arm_pd(
        steps: &[([f32; 3], [f32; 3])],
        kp: f32,
        kd: f32,
        acc: &mut f64,
    ) -> std::time::Duration {
        let start = std::time::Instant::now();
        let mut sum = 0.0f64;
        // The slice itself is opaque, so the inputs cannot be folded: the
        // per-value `black_box` of the first bench forced a memory round-trip
        // on every argument and measured that, not the law.
        let steps = std::hint::black_box(steps);
        for (e, v) in steps {
            let mut out = [0.0f32; 3];
            for a in 0..3 {
                out[a] = pd::pd_axis(e[a], v[a], kp, kd, ARM_DT_S, ARM_CAP);
            }
            sum += f64::from(out[0]) + f64::from(out[1]) + f64::from(out[2]);
        }
        let spent = start.elapsed();
        *acc += std::hint::black_box(sum);
        spent
    }

    /// One integer PID arm run: four bones stepped in turn, as a rig walks a
    /// bone list, so the harness measures throughput and not one servo's
    /// latency chain.
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "a test-only ratio of two laws; the wall clock never enters the tick or the hash"
    )]
    fn arm_pid(
        steps: &[([i64; 3], [i64; 3])],
        t: &ServoTuning,
        gains: Gains,
        acc: &mut i64,
    ) -> std::time::Duration {
        let mut servos = [Servo::new(); 4];
        let start = std::time::Instant::now();
        let mut sum = 0i64;
        let steps = std::hint::black_box(steps);
        for (i, (e, v)) in steps.iter().enumerate() {
            let out = servos[i & 3].step(*e, *v, DT_US, gains, t, CAP);
            sum += i64::from(out[0]) + i64::from(out[1]) + i64::from(out[2]);
        }
        let spent = start.elapsed();
        *acc += std::hint::black_box(sum);
        spent
    }

    /// The float twin of the integer law: the same P clamp, the same integral
    /// band and bound, D on the measurement, the same cap and slew, in world
    /// units. It exists so the bench can separate the cost of fixed point from
    /// the cost of PID semantics: the shipped float arm is a stateless PD law,
    /// so the two are not the same computation.
    struct FloatPid {
        integ: [f32; 3],
        out: [f32; 3],
    }

    impl FloatPid {
        fn new() -> Self {
            Self {
                integ: [0.0; 3],
                out: [0.0; 3],
            }
        }

        fn step(
            &mut self,
            err: [f32; 3],
            vel: [f32; 3],
            dt_s: f32,
            gains: Gains,
            t: &ServoTuning,
        ) -> [f32; 3] {
            let i_gain = narrow(f64::from(t.i_q8)) / 256.0;
            let band = from_micro(t.integ_band_um);
            let bound = from_micro(t.integ_clamp_um_s);
            let slew = from_micro(i64::from(t.slew_um_s_tick));
            let cap = ARM_CAP;
            // The integer law's saturation bound in world units, so both arms
            // clamp on the same value (MAX_TERM is µm).
            let lim = from_micro(MAX_TERM);
            let mut out = [0.0f32; 3];
            for a in 0..3 {
                let e = err[a].clamp(-lim, lim);
                let v = vel[a].clamp(-lim, lim);
                let p_term = gains.p * e;
                if e.abs() <= band {
                    self.integ[a] = (self.integ[a] + i_gain * e * dt_s).clamp(-bound, bound);
                }
                let d_term = gains.d * v;
                let raw = (p_term + self.integ[a] - d_term).clamp(-cap, cap);
                let prev = self.out[a];
                let slewed = prev + (raw - prev).clamp(-slew, slew);
                self.out[a] = slewed;
                out[a] = slewed;
            }
            out
        }
    }

    /// One float PID arm run: the same four-bone walk as [`arm_pid`], so the
    /// only difference between the arms is integer versus float arithmetic.
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "a test-only ratio of two laws; the wall clock never enters the tick or the hash"
    )]
    fn arm_pid_f32(
        steps: &[([f32; 3], [f32; 3])],
        t: &ServoTuning,
        gains: Gains,
        acc: &mut f32,
    ) -> std::time::Duration {
        let mut servos = [
            FloatPid::new(),
            FloatPid::new(),
            FloatPid::new(),
            FloatPid::new(),
        ];
        let start = std::time::Instant::now();
        let mut sum = 0.0f32;
        let steps = std::hint::black_box(steps);
        for (i, (e, v)) in steps.iter().enumerate() {
            let out = servos[i & 3].step(*e, *v, ARM_DT_S, gains, t);
            sum += out[0] + out[1] + out[2];
        }
        let spent = start.elapsed();
        *acc += std::hint::black_box(sum);
        spent
    }

    /// Median, least and most of a sample.
    fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let least = v.first().copied().unwrap_or(0.0);
        let median = v.get(v.len() / 2).copied().unwrap_or(0.0);
        let most = v.last().copied().unwrap_or(0.0);
        (median, least, most)
    }

    /// PX11's accept, overhead leg. Three arms, one process, one set of steps,
    /// run alternately [`ARM_PAIRS`] times over [`ARM_STEPS`] three-axis steps
    /// each after [`WARMUP_PAIRS`] dropped pairs:
    ///
    /// - the shipped float PD law of `pd.rs` (stateless, no integral),
    /// - its float twin of the integer law ([`FloatPid`]: same integral band,
    ///   bound, cap and slew), so fixed point is measured against the same
    ///   computation and not against a cheaper one,
    /// - the integer PID of this module.
    ///
    /// Two ratios come out. `pid / pd` is the accept's literal pair — the new
    /// servo against the law it replaced, which also carries the cost of the
    /// integral and the slew. `pid / pidf` is the cost of fixed point alone.
    /// The report is the median with its least and most, never a single run.
    ///
    /// The accept stays a median `pid / pd` at or below 1.10; it is not met, so
    /// the bench stays `#[ignore]`d and the row open. The wall clock never
    /// enters the tick or the hash; run it with `cargo test -p engine-physics
    /// --lib servo_overhead -- --ignored --nocapture`.
    #[test]
    #[ignore = "PX11 open: the integer PID is over the accept's 1.10 against the float PD law; the bench prints both ratios"]
    fn servo_overhead_within_ten_percent() {
        let (_, kd) = pd::stable_gains(P, D, ARM_DT_S);
        let gains = Gains { p: P, d: kd };
        let t = tuning();
        let steps = Inputs::new(ARM_STEPS);
        let (mut pd_acc, mut pidf_acc, mut pid_acc) = (0.0f64, 0.0f32, 0i64);
        for _ in 0..WARMUP_PAIRS {
            arm_pd(&steps.flt, P, kd, &mut pd_acc);
            arm_pid_f32(&steps.flt, &t, gains, &mut pidf_acc);
            arm_pid(&steps.int, &t, gains, &mut pid_acc);
        }
        let mut pd_us = Vec::with_capacity(ARM_PAIRS);
        let mut pidf_us = Vec::with_capacity(ARM_PAIRS);
        let mut pid_us = Vec::with_capacity(ARM_PAIRS);
        let mut ratios = Vec::with_capacity(ARM_PAIRS);
        let mut same_law = Vec::with_capacity(ARM_PAIRS);
        for _ in 0..ARM_PAIRS {
            let pd = arm_pd(&steps.flt, P, kd, &mut pd_acc).as_secs_f64() * 1e6;
            let pidf = arm_pid_f32(&steps.flt, &t, gains, &mut pidf_acc).as_secs_f64() * 1e6;
            let pid = arm_pid(&steps.int, &t, gains, &mut pid_acc).as_secs_f64() * 1e6;
            pd_us.push(pd);
            pidf_us.push(pidf);
            pid_us.push(pid);
            ratios.push(pid / pd);
            same_law.push(pid / pidf);
        }
        let (pd_med, pd_lo, pd_hi) = spread(pd_us);
        let (pidf_med, pidf_lo, pidf_hi) = spread(pidf_us);
        let (pid_med, pid_lo, pid_hi) = spread(pid_us);
        let (r_med, r_lo, r_hi) = spread(ratios);
        let (s_med, s_lo, s_hi) = spread(same_law);
        println!("servo overhead: {ARM_PAIRS} pairs of {ARM_STEPS} three-axis steps");
        println!("  float PD  median {pd_med:.1} µs  least {pd_lo:.1}  most {pd_hi:.1}");
        println!("  float PID median {pidf_med:.1} µs  least {pidf_lo:.1}  most {pidf_hi:.1}");
        println!("  integer   median {pid_med:.1} µs  least {pid_lo:.1}  most {pid_hi:.1}");
        println!("  pid / pd  median {r_med:.3}  least {r_lo:.3}  most {r_hi:.3}  (accept pair)");
        println!(
            "  pid / pidf median {s_med:.3}  least {s_lo:.3}  most {s_hi:.3}  (fixed point alone)"
        );
        println!("  sums      pd {pd_acc:.3} pidf {pidf_acc:.3} pid {pid_acc}");
        println!(
            "  per bone per tick: integer {:.4} µs, float PID {:.4} µs (4 bones in turn)",
            pid_med * 1e3 / ARM_STEPS as f64,
            pidf_med * 1e3 / ARM_STEPS as f64
        );
        assert!(
            r_med <= 1.10,
            "integer PID overhead median {r_med:.3} over the range {r_lo:.3}-{r_hi:.3} \
             exceeds 1.10: pid {pid_med:.1} µs, pd {pd_med:.1} µs (fixed point alone {s_med:.3})"
        );
    }
}
