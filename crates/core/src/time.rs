//! time: integer 60Hz accumulator, the only sim clock.
//! Owner: engine-core. Units: ns, thirds-of-ns, ticks.
//! Determinism: tick index u64 is truth; wall quantized at poll only,
//! never hashed. Errors: `TimeError`, no unwrap on input.
//! Budget: O(1) per push (at most 5 steps), 24 bytes state, zero heap alloc.

use crate::tick::MAX_STEPS;
use std::fmt;
use std::time::Duration;

/// Nanoseconds per second.
pub const NS_PER_SEC: u64 = 1_000_000_000;
/// Whole-nanosecond part of the exact 60Hz step (1e9/60 = 16_666_666 + 2/3).
pub const STEP_NS: u64 = 16_666_666;
/// Carried fractional remainder of the exact 60Hz step, in thirds of a ns.
pub const STEP_REM_THIRDS: u8 = 2;
/// Exact 60Hz step in thirds of a ns: 16_666_666*3 + 2 = 50_000_000.
pub const STEP_THIRDS: u64 = STEP_NS * 3 + STEP_REM_THIRDS as u64;
/// Max accepted frame dt in ns (0.1s, matches `tick::MAX_FRAME_DT`).
pub const MAX_FRAME_NS: u64 = 100_000_000;

/// Typed time errors; invalid external input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// Wall seconds were negative.
    NegativeDt,
    /// Wall seconds were NaN or infinite.
    NonFiniteDt,
    /// Tick index or buffered ns would overflow u64.
    Overflow,
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NegativeDt => write!(f, "frame dt must be >= 0"),
            Self::NonFiniteDt => write!(f, "frame dt must be finite"),
            Self::Overflow => write!(f, "tick index or buffer would overflow"),
        }
    }
}

impl std::error::Error for TimeError {}

/// Integer fixed-step accumulator: buffered ns plus carried thirds.
///
/// The exact 60Hz step is 16_666_666ns + 2/3ns. The buffer holds
/// `acc_ns * 3 + rem_thirds` thirds of a ns, so three steps consume
/// exactly 50_000_000ns with zero drift. Input is clamped at
/// `MAX_FRAME_NS` and at most `MAX_STEPS` steps are taken per push,
/// with the anti-spiral cap. When the step cap is
/// hit, banked whole steps are forgiven (only a sub-step remainder is
/// kept), so a hitch burst never builds catch-up debt. `tick` is the
/// truth; wall input never enters a hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntAcc {
    acc_ns: u64,
    rem_thirds: u8,
    tick: u64,
}

impl IntAcc {
    /// Build an empty 60Hz accumulator.
    pub fn new() -> Self {
        Self {
            acc_ns: 0,
            rem_thirds: 0,
            tick: 0,
        }
    }

    /// Ticks taken so far; the authoritative clock.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Buffered whole nanoseconds (presentation only, never hashed).
    pub fn buffered_ns(&self) -> u64 {
        self.acc_ns
    }

    /// Buffered fractional thirds of a ns, always 0..=2 (never hashed).
    pub fn buffered_thirds(&self) -> u8 {
        self.rem_thirds
    }

    /// Blend factor for render interpolation in [0, 1].
    /// Presentation only: it must never enter a simulation hash.
    pub fn alpha(&self) -> f32 {
        let num = self.acc_ns as f64 * 3.0 + f64::from(self.rem_thirds);
        (num / STEP_THIRDS as f64).clamp(0.0, 1.0) as f32
    }

    /// Push already-quantized nanoseconds, return steps due.
    /// Clamps at `MAX_FRAME_NS`; errors only on u64 overflow.
    /// Anti-spiral: when the `MAX_STEPS` cap is hit, banked whole steps
    /// are forgiven and only a sub-step remainder is kept, so slow frames
    /// cannot stockpile catch-up debt.
    pub fn push_ns(&mut self, dt_ns: u64) -> Result<u32, TimeError> {
        let clamped = dt_ns.min(MAX_FRAME_NS);
        let mut total: u128 =
            u128::from(self.acc_ns) * 3 + u128::from(self.rem_thirds) + u128::from(clamped) * 3;
        let mut n: u32 = 0;
        while total >= u128::from(STEP_THIRDS) && n < MAX_STEPS {
            total -= u128::from(STEP_THIRDS);
            n += 1;
        }
        let next = self
            .tick
            .checked_add(u64::from(n))
            .ok_or(TimeError::Overflow)?;
        if total / 3 > u128::from(u64::MAX) {
            return Err(TimeError::Overflow);
        }
        if n == MAX_STEPS {
            total %= u128::from(STEP_THIRDS);
        }
        self.tick = next;
        self.acc_ns = u64::try_from(total / 3).expect("total/3 checked against u64::MAX");
        self.rem_thirds = u8::try_from(total % 3).expect("remainder 0..3 fits in u8");
        Ok(n)
    }

    /// Push a wall-clock `Duration` quantized to whole ns (poll boundary).
    /// Clamps at `MAX_FRAME_NS`; sub-ns precision cannot exist in `Duration`.
    pub fn push_duration(&mut self, dt: Duration) -> Result<u32, TimeError> {
        let ns = u64::try_from(dt.as_nanos().min(u128::from(MAX_FRAME_NS)))
            .expect("clamped at MAX_FRAME_NS fits in u64");
        self.push_ns(ns)
    }

    /// Push wall-clock seconds, quantizing to ns at this poll boundary.
    /// Rejects NaN/infinite/negative; clamps huge inputs at `MAX_FRAME_NS`.
    pub fn push_secs(&mut self, dt_secs: f32) -> Result<u32, TimeError> {
        self.push_ns(quantize_wall_secs(dt_secs)?)
    }
}

impl Default for IntAcc {
    fn default() -> Self {
        Self::new()
    }
}

/// Checked f64 bits to u64 by truncation toward zero.
/// Takes the `to_bits` of the wall value so the warm tick+hash path stays integer-only.
/// Rejects non-finite and negative input; overflow past u64 fails.
/// Bit decode keeps every in-range integer exact without a cast.
fn f64_to_u64_checked(bits: u64) -> Result<u64, TimeError> {
    let exp_raw = (bits >> 52) & 0x7ff;
    if exp_raw == 0x7ff {
        return Err(TimeError::NonFiniteDt);
    }
    if (bits >> 63) == 1 {
        let mant_raw = bits & 0x000f_ffff_ffff_ffff;
        if exp_raw != 0 || mant_raw != 0 {
            return Err(TimeError::NegativeDt);
        }
    }
    if exp_raw == 0 {
        return Ok(0);
    }
    let exp_u16 = u16::try_from(exp_raw).map_err(|_| TimeError::Overflow)?;
    let exp = i32::from(exp_u16)
        .checked_sub(1023)
        .ok_or(TimeError::Overflow)?;
    if exp < 0 {
        return Ok(0);
    }
    let mant = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
    if exp <= 52 {
        let shift = u32::try_from(52i32.checked_sub(exp).ok_or(TimeError::Overflow)?)
            .map_err(|_| TimeError::Overflow)?;
        Ok(mant >> shift)
    } else {
        let shift = u32::try_from(exp.checked_sub(52i32).ok_or(TimeError::Overflow)?)
            .map_err(|_| TimeError::Overflow)?;
        // `checked_shl` only guards the shift amount; value overflow
        // past u64 must refuse, never wrap, so callers keep checked.
        if shift >= 64u32 || mant > u64::MAX >> shift {
            return Err(TimeError::Overflow);
        }
        Ok(mant << shift)
    }
}

/// Poll boundary: quantize wall-clock seconds to whole ns (round half up).
/// Rejects NaN/infinite/negative; saturates huge inputs at `MAX_FRAME_NS`.
pub fn quantize_wall_secs(dt_secs: f32) -> Result<u64, TimeError> {
    if !dt_secs.is_finite() {
        return Err(TimeError::NonFiniteDt);
    }
    if dt_secs < 0.0 {
        return Err(TimeError::NegativeDt);
    }
    let rounded = f64::from(dt_secs) * 1_000_000_000.0;
    // 100M fits in u32; every u32 value holds exact in f64.
    let bound = f64::from(u32::try_from(MAX_FRAME_NS).map_err(|_| TimeError::Overflow)?);
    let clamped = rounded.round().min(bound).max(0.0);
    f64_to_u64_checked(clamped.to_bits())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;

    #[test]
    fn step_constants_exact() {
        assert_eq!(STEP_NS * 60 + 40, NS_PER_SEC);
        assert_eq!(STEP_THIRDS, 50_000_000);
        assert_eq!(STEP_REM_THIRDS, 2);
        assert_eq!(MAX_FRAME_NS, 100_000_000);
        assert_eq!(crate::TICK_HZ, 60);
    }

    #[test]
    fn soak_10m_int_zero_drift() {
        // 10M x 10ms = 100_000s = exactly 6_000_000 ticks; 10ms is exact in ns.
        let mut acc = IntAcc::new();
        let mut steps: u64 = 0;
        for _ in 0..10_000_000 {
            steps += u64::from(acc.push_ns(10_000_000).expect("10ms push cannot overflow"));
        }
        assert_eq!(steps, 6_000_000, "integer steps must be exact");
        assert_eq!(acc.tick(), 6_000_000);
        assert_eq!(acc.buffered_ns(), 0, "integer residual must be zero");
        assert_eq!(acc.buffered_thirds(), 0, "integer remainder must be zero");
    }

    #[test]
    fn int_acc_exact_steps_no_drift() {
        // Integer ns is the only sim clock.
        // 6000 x 10ms is exactly 60s = 3600 ticks with zero residual.
        let mut acc = IntAcc::new();
        assert_eq!(acc.tick(), 0);
        assert_eq!(acc.buffered_ns(), 0);
        assert_eq!(acc.buffered_thirds(), 0);
        let mut steps: u64 = 0;
        for _ in 0..6000 {
            steps += u64::from(acc.push_ns(10_000_000).expect("10ms bounded"));
        }
        assert_eq!(steps, 3_600);
        assert_eq!(acc.tick(), 3_600);
        assert_eq!(acc.buffered_ns(), 0, "integer residual must be zero");
        assert_eq!(acc.buffered_thirds(), 0, "integer remainder must be zero");
        assert_eq!(acc.alpha(), 0.0);
        assert_eq!(crate::TICK_HZ, 60);
        assert_eq!(crate::MAX_STEPS, 5);
    }

    #[test]
    fn same_tape_twice_identical() {
        let tape: Vec<u64> = (0..600).map(|i| 10_000_000 + (i % 7) * 1_000_000).collect();
        let run = |tape: &[u64]| {
            let mut acc = IntAcc::new();
            let mut state = crate::TickState::new(2040);
            let mut per_push = Vec::with_capacity(tape.len());
            for &dt in tape {
                let n = acc
                    .push_duration(Duration::from_nanos(dt))
                    .expect("bounded tape cannot overflow");
                let base = acc.tick() - u64::from(n);
                for i in 0..n {
                    state.step(base + u64::from(i));
                }
                per_push.push(n);
            }
            (
                per_push,
                acc.tick(),
                acc.buffered_ns(),
                acc.buffered_thirds(),
                state.hash(),
            )
        };
        let (p1, t1, b1, r1, h1) = run(&tape);
        let (p2, t2, b2, r2, h2) = run(&tape);
        assert_eq!((p1, t1, b1, r1, h1), (p2, t2, b2, r2, h2));
        assert_ne!(h1, 2040, "ticks must advance the hash");
    }

    #[test]
    fn wall_never_hashed_tick_is_truth() {
        // 1000 x 1ms and 100 x 10ms both total 1s = 60 ticks; hashes must match.
        let drive = |dts: &[Duration]| {
            let mut acc = IntAcc::new();
            let mut state = crate::TickState::new(7);
            for &dt in dts {
                let n = acc.push_duration(dt).expect("bounded tape cannot overflow");
                let base = acc.tick() - u64::from(n);
                for i in 0..n {
                    state.step(base + u64::from(i));
                }
            }
            (acc.tick(), state.hash())
        };
        let a: Vec<Duration> = vec![Duration::from_millis(1); 1000];
        let b: Vec<Duration> = vec![Duration::from_millis(10); 100];
        let (ta, ha) = drive(&a);
        let (tb, hb) = drive(&b);
        assert_eq!(ta, 60);
        assert_eq!(tb, 60);
        assert_eq!(ha, hb, "same ticks, different wall chunking, same hash");
    }

    #[test]
    fn core_hash_inputs_frozen_and_deterministic() {
        // Core inputs the app golden depends on stay frozen; same tape
        // replays x2 identical. The full golden hash is gated by the app
        // proof (golden x2), never by this unit test.
        assert_eq!(crate::TICK_HZ, 60);
        assert_eq!(crate::DT.to_bits(), (1.0f32 / 60.0f32).to_bits());
        assert_eq!(crate::MAX_STEPS, 5);
        assert_eq!(crate::MAX_FRAME_DT.to_bits(), 0.1f32.to_bits());
        assert_eq!(crate::TickState::new(2040).hash(), 2040);
        assert_eq!(crate::Rng::new(2040).next_u64(), 2189151099318);
        let h1 = crate::run_ticks(600, 2040, |t, _| t.0);
        let h2 = crate::run_ticks(600, 2040, |t, _| t.0);
        assert_eq!(h1.len(), 600);
        assert_eq!(h1, h2);
        assert_ne!(h1.first(), h1.last());
    }

    #[test]
    fn rejects_bad_wall_input() {
        let mut acc = IntAcc::new();
        assert_eq!(acc.push_secs(f32::NAN), Err(TimeError::NonFiniteDt));
        assert_eq!(acc.push_secs(f32::INFINITY), Err(TimeError::NonFiniteDt));
        assert_eq!(acc.push_secs(-1.0), Err(TimeError::NegativeDt));
        assert_eq!(acc.tick(), 0, "rejected input mutates nothing");
        // Huge inputs clamp, never panic or overflow.
        assert_eq!(acc.push_secs(10.0), Ok(5));
        assert_eq!(acc.push_ns(u64::MAX), Ok(5));
        assert_eq!(quantize_wall_secs(0.0), Ok(0));
        assert_eq!(quantize_wall_secs(-0.0), Ok(0));
    }

    #[test]
    fn slow_frames_forgive_backlog() {
        // 60 x 100ms hitches: 5 steps each, no banked catch-up debt.
        let mut acc = IntAcc::new();
        let mut steps: u64 = 0;
        for _ in 0..60 {
            let n = acc.push_ns(MAX_FRAME_NS).expect("bounded");
            assert_eq!(n, 5);
            steps += u64::from(n);
        }
        assert_eq!(steps, 300);
        assert_eq!(acc.tick(), 300);
        assert!(
            acc.buffered_ns() < 17_000_000,
            "backlog must stay under one step, got {}ns",
            acc.buffered_ns()
        );
        // Same tape twice: forgiveness is deterministic.
        let mut acc2 = IntAcc::new();
        for _ in 0..60 {
            acc2.push_ns(MAX_FRAME_NS).expect("bounded");
        }
        assert_eq!(
            (acc2.tick(), acc2.buffered_ns(), acc2.buffered_thirds()),
            (acc.tick(), acc.buffered_ns(), acc.buffered_thirds())
        );
    }

    #[test]
    fn tick_overflow_refuses_without_mutation() {
        let mut acc = IntAcc::new();
        acc.tick = u64::MAX - 1;
        acc.acc_ns = 0;
        acc.rem_thirds = 0;
        // 100ms needs 6 steps but caps at 5: MAX-1 + 5 overflows.
        assert_eq!(acc.push_ns(MAX_FRAME_NS), Err(TimeError::Overflow));
        assert_eq!(acc.tick, u64::MAX - 1);
        assert_eq!((acc.acc_ns, acc.rem_thirds), (0, 0));
    }

    #[test]
    fn no_heap_in_push_steady_state() {
        let ((), heap) = measure(|| {
            let mut acc = IntAcc::new();
            for i in 0..100_000u64 {
                let dt = 1_000_000 + (i % 7) * 100_000;
                let n = acc.push_ns(dt).expect("bounded");
                std::hint::black_box(n);
            }
            std::hint::black_box(acc.tick());
        });
        assert_eq!(heap.calls, 0, "integer push must not allocate");
        let ((), poll) = measure(|| {
            let mut acc = IntAcc::new();
            for _ in 0..10_000 {
                let n = acc.push_secs(0.001).expect("bounded");
                std::hint::black_box(n);
            }
        });
        assert_eq!(poll.calls, 0, "wall poll path must not allocate");
    }

    #[test]
    fn alpha_presentation_only_bounds() {
        let mut acc = IntAcc::new();
        assert_eq!(acc.alpha(), 0.0);
        acc.push_ns(1_000_000).expect("bounded");
        let a = acc.alpha();
        assert!(a > 0.0 && a < 1.0, "partial buffer blends, alpha={a}");
        let (tick, hash_before) = (acc.tick(), {
            let mut s = crate::TickState::new(3);
            s.step(acc.tick());
            s.hash()
        });
        assert_eq!(acc.alpha(), a, "alpha is a pure read");
        assert_eq!(acc.tick(), tick);
        let _ = hash_before;
    }

    #[test]
    fn quantize_wall_at_poll() {
        // 1/60 as f32 widens past the true step, so it rounds up one ns.
        assert_eq!(quantize_wall_secs(1.0 / 60.0), Ok(16_666_668));
        assert_eq!(quantize_wall_secs(0.001), Ok(1_000_000));
        assert_eq!(quantize_wall_secs(10.0), Ok(MAX_FRAME_NS));
    }

    #[test]
    fn f64_to_u64_checked_refuses_overflow_past_u64() {
        // Boundary past u64::MAX must refuse, never wrap to zero;
        // in-range 100M stays exact for the poll boundary.
        assert_eq!(
            f64_to_u64_checked(1.844_674_407_370_955_2e19f64.to_bits()),
            Err(TimeError::Overflow)
        );
        assert_eq!(
            f64_to_u64_checked(1.0e20f64.to_bits()),
            Err(TimeError::Overflow)
        );
        assert_eq!(
            f64_to_u64_checked(100_000_000.0f64.to_bits()),
            Ok(100_000_000)
        );
    }
}
