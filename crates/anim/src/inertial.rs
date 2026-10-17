//! inertial: inertialized clip switches over joint positions.
//!
//! At a switch each joint keeps the offset and the offset velocity between
//! what the viewer last saw and the arriving clip, and a critically damped
//! spring decays both to zero. Position and speed stay continuous across the
//! cut, so a fast joint does not snap. Owner: `engine-anim`; consumer: the
//! fighter look, which passes the shown pose and the arriving clip's pose and
//! velocity at each switch tick and reads the decayed offsets at any age
//! between sim ticks ([`Inertial::offsets_after`]). Units: meters, m/s,
//! seconds. Determinism:
//! basic IEEE operations only (no libm, no fused multiply-add), so the same
//! inputs give the same bits on every platform. Presentation only: floats
//! never enter a hash. Errors: [`InertialError`]; a refused call changes
//! nothing. Heap: the set allocates in `new`, never per tick.

use core::f32::consts::LN_2;

/// Shortest accepted halflife in seconds.
pub const MIN_HALFLIFE_S: f32 = 0.001;
/// Longest accepted halflife in seconds.
pub const MAX_HALFLIFE_S: f32 = 1.0;
/// Longest accepted frame step in seconds; a longer step is refused.
pub const MAX_DT_S: f32 = 1.0;
/// Longest accepted time since a switch for [`Inertial::offsets_after`], in
/// seconds; a longer age is refused (a settled switch needs no call).
pub const MAX_AGE_S: f32 = 60.0;
/// Most joints one set holds.
pub const MAX_JOINTS: usize = 256;
/// Largest accepted joint coordinate in meters, either sign.
pub const MAX_COORD_M: f32 = 1000.0;
/// Largest accepted joint speed in meters per second, either sign.
pub const MAX_SPEED_M_S: f32 = 1000.0;
/// Past this argument e^-x is below f32's normal range and reads as zero.
const EXP_NEG_CUTOFF: f32 = 88.0;

/// Inertial failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InertialError {
    /// Halflife not finite or outside the accepted span; value carried.
    BadHalflife(f32),
    /// Step not finite or outside `0..=MAX_DT_S`; value carried.
    BadStep(f32),
    /// Age not finite or outside `0..=MAX_AGE_S`; value carried.
    BadAge(f32),
    /// Joint count past [`MAX_JOINTS`]; value carried.
    TooManyJoints(usize),
    /// Lane count differs from the set's joint count.
    LengthMismatch { expected: usize, got: usize },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Finite float past the named lane's bound.
    OutOfRange(&'static str),
}

impl core::fmt::Display for InertialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InertialError::BadHalflife(h) => write!(
                f,
                "inertial: halflife {h} s outside {MIN_HALFLIFE_S}..={MAX_HALFLIFE_S} s"
            ),
            InertialError::BadStep(dt) => {
                write!(f, "inertial: step {dt} s outside 0..={MAX_DT_S} s")
            }
            InertialError::BadAge(age) => {
                write!(f, "inertial: age {age} s outside 0..={MAX_AGE_S} s")
            }
            InertialError::TooManyJoints(n) => write!(f, "inertial: {n} joints past {MAX_JOINTS}"),
            InertialError::LengthMismatch { expected, got } => {
                write!(f, "inertial: lane count {got} differs from {expected}")
            }
            InertialError::NonFinite(lane) => write!(f, "inertial: non-finite float in {lane}"),
            InertialError::OutOfRange(lane) => write!(f, "inertial: {lane} past its bound"),
        }
    }
}

impl std::error::Error for InertialError {}

/// Per-joint inertial state for one fighter: the offset and offset velocity
/// of each joint from its arriving clip pose. The rate is fixed at
/// construction, `2 ln 2 / halflife`, the decay rate of a critically damped
/// spring with that halflife.
#[derive(Clone, Debug, PartialEq)]
pub struct Inertial {
    off: Vec<[f32; 3]>,
    off_v: Vec<[f32; 3]>,
    rate: f32,
}

impl Inertial {
    /// Zero offsets for `joints` joints with the given halflife in seconds.
    /// Errors: [`InertialError::BadHalflife`] outside the accepted span,
    /// [`InertialError::TooManyJoints`] past [`MAX_JOINTS`].
    pub fn new(joints: usize, halflife_s: f32) -> Result<Self, InertialError> {
        if !halflife_s.is_finite() || !(MIN_HALFLIFE_S..=MAX_HALFLIFE_S).contains(&halflife_s) {
            return Err(InertialError::BadHalflife(halflife_s));
        }
        if joints > MAX_JOINTS {
            return Err(InertialError::TooManyJoints(joints));
        }
        Ok(Self {
            off: vec![[0.0; 3]; joints],
            off_v: vec![[0.0; 3]; joints],
            rate: 2.0 * LN_2 / halflife_s,
        })
    }

    /// Cut to a new clip. `shown` and `shown_vel` are the joint positions and
    /// velocities the viewer saw last, `arriving` and `arriving_vel` the new
    /// clip's at this tick. After the call the offsets are shown minus
    /// arriving, so a cut in the middle of a blend starts from what was shown.
    /// Errors: lane lengths, non-finite or out-of-bound floats; a refused call
    /// changes nothing.
    pub fn switch(
        &mut self,
        shown: &[[f32; 3]],
        shown_vel: &[[f32; 3]],
        arriving: &[[f32; 3]],
        arriving_vel: &[[f32; 3]],
    ) -> Result<(), InertialError> {
        let n = self.off.len();
        same_len(n, shown.len())?;
        same_len(n, shown_vel.len())?;
        same_len(n, arriving.len())?;
        same_len(n, arriving_vel.len())?;
        check_lanes(shown, "shown", MAX_COORD_M)?;
        check_lanes(shown_vel, "shown_vel", MAX_SPEED_M_S)?;
        check_lanes(arriving, "arriving", MAX_COORD_M)?;
        check_lanes(arriving_vel, "arriving_vel", MAX_SPEED_M_S)?;
        for (((off, off_v), (s, sv)), (a, av)) in self
            .off
            .iter_mut()
            .zip(self.off_v.iter_mut())
            .zip(shown.iter().zip(shown_vel))
            .zip(arriving.iter().zip(arriving_vel))
        {
            *off = sub(*s, *a);
            *off_v = sub(*sv, *av);
        }
        Ok(())
    }

    /// Advance `dt_s` seconds and write `arriving + offset` to `out`. A zero
    /// step writes the shown pose back exactly.
    /// Errors: non-finite or out-of-range `dt_s` or `arriving`, lane lengths;
    /// a refused call changes nothing and writes nothing.
    pub fn step(
        &mut self,
        arriving: &[[f32; 3]],
        dt_s: f32,
        out: &mut [[f32; 3]],
    ) -> Result<(), InertialError> {
        if !dt_s.is_finite() || !(0.0..=MAX_DT_S).contains(&dt_s) {
            return Err(InertialError::BadStep(dt_s));
        }
        let n = self.off.len();
        same_len(n, arriving.len())?;
        same_len(n, out.len())?;
        check_lanes(arriving, "arriving", MAX_COORD_M)?;
        let rate = self.rate;
        let decay = exp_neg(rate * dt_s);
        for ((off, off_v), (target, shown)) in self
            .off
            .iter_mut()
            .zip(self.off_v.iter_mut())
            .zip(arriving.iter().zip(out.iter_mut()))
        {
            for (((o, v), t), s) in off
                .iter_mut()
                .zip(off_v.iter_mut())
                .zip(target)
                .zip(shown.iter_mut())
            {
                let (next_off, next_v) = spring_lane(*o, *v, rate, dt_s, decay);
                *o = next_off;
                *v = next_v;
                *s = *t + next_off;
            }
        }
        Ok(())
    }

    /// The offsets `age_s` seconds after the switch, written to `out` without
    /// advancing the set: the pure form of [`Inertial::step`], so a frame at
    /// any moment between two sim ticks reads the same state. Offsets only,
    /// no arriving pose added. After `step` has run, `age_s` counts from that
    /// step.
    /// Errors: `age_s` not finite or outside `0..=MAX_AGE_S`, lane length;
    /// a refused call writes nothing.
    pub fn offsets_after(&self, age_s: f32, out: &mut [[f32; 3]]) -> Result<(), InertialError> {
        if !age_s.is_finite() || !(0.0..=MAX_AGE_S).contains(&age_s) {
            return Err(InertialError::BadAge(age_s));
        }
        same_len(self.off.len(), out.len())?;
        let decay = exp_neg(self.rate * age_s);
        for ((off, off_v), lane) in self.off.iter().zip(&self.off_v).zip(out.iter_mut()) {
            for ((o, v), slot) in off.iter().zip(off_v).zip(lane.iter_mut()) {
                *slot = spring_lane(*o, *v, self.rate, age_s, decay).0;
            }
        }
        Ok(())
    }

    /// Largest |offset| over all joints and components, in meters.
    pub fn max_offset(&self) -> f32 {
        self.off
            .iter()
            .flatten()
            .fold(0.0, |worst, v| worst.max(v.abs()))
    }
}

/// Component-wise `a - b`.
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn same_len(expected: usize, got: usize) -> Result<(), InertialError> {
    if expected == got {
        Ok(())
    } else {
        Err(InertialError::LengthMismatch { expected, got })
    }
}

/// Every float finite and within `bound` in absolute value.
fn check_lanes(lanes: &[[f32; 3]], name: &'static str, bound: f32) -> Result<(), InertialError> {
    for lane in lanes {
        for v in lane {
            if !v.is_finite() {
                return Err(InertialError::NonFinite(name));
            }
            if v.abs() > bound {
                return Err(InertialError::OutOfRange(name));
            }
        }
    }
    Ok(())
}

/// One exact step of a critically damped spring on one scalar lane, where
/// `decay` = e^-(rate dt). The velocity term folds the offset velocity into
/// `rate * off`, so a zero offset velocity decays without overshoot.
fn spring_lane(off: f32, off_v: f32, rate: f32, dt: f32, decay: f32) -> (f32, f32) {
    let j1 = off_v + rate * off;
    (decay * (off + j1 * dt), decay * (off_v - j1 * rate * dt))
}

/// e^-x for x >= 0 from basic IEEE operations only, so every platform rounds
/// the same way (this crate is a simulation crate and `f32::exp` is a
/// disallowed method). Halves x to at most one (exact, a power of two), sums
/// ten Taylor terms, then squares back up.
fn exp_neg(x: f32) -> f32 {
    if x >= EXP_NEG_CUTOFF {
        return 0.0;
    }
    let mut r = x;
    let mut squarings = 0u32;
    while r > 1.0 {
        r *= 0.5;
        squarings += 1;
    }
    let mut term = 1.0f32;
    let mut sum = 1.0f32;
    for n in 1u8..=10 {
        term *= -r / f32::from(n);
        sum += term;
    }
    for _ in 0..squarings {
        sum *= sum;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::{exp_neg, Inertial, InertialError, MAX_AGE_S, MAX_DT_S, MAX_JOINTS};
    use core::f32::consts::{E, LN_2};
    use engine_core::alloc_probe::measure;
    use engine_core::DT;

    /// Halflife for the decay tests: five halflives at 60 Hz are nine ticks,
    /// the fighter crossfade length.
    const HALF: f32 = 0.03;
    const ZERO2: [[f32; 3]; 2] = [[0.0; 3]; 2];

    /// Largest per-component gap between two pose lanes.
    fn gap(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
        a.iter()
            .zip(b)
            .flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| (x - y).abs()))
            .fold(0.0, f32::max)
    }

    /// Bound on one lane's error after a switch with `offset` and
    /// `velocity`: the offset plus the largest excursion of
    /// e^-rate t (offset + a t), with a = velocity + rate * offset.
    fn spring_bound(offset: f32, velocity: f32) -> f32 {
        let rate = 2.0 * LN_2 / HALF;
        let a = velocity + rate * offset;
        offset.abs() + a.abs() / (E * rate)
    }

    #[test]
    fn switch_frame_shows_the_old_pose() {
        let shown = [[0.3, 1.2, -0.1], [0.0, 0.9, 0.4]];
        let arriving = [[-0.2, 1.0, 0.0], [0.5, 0.5, 0.5]];
        let mut iz = Inertial::new(2, HALF).expect("fixture rate");
        iz.switch(
            &shown,
            &[[1.0, 0.0, 0.0], [0.0, -2.0, 0.5]],
            &arriving,
            &ZERO2,
        )
        .expect("fixture switch");
        let mut out = [[0.0; 3]; 2];
        iz.step(&arriving, 0.0, &mut out).expect("zero step");
        assert!(
            gap(&out, &shown) <= 1e-5,
            "switch frame {out:?} vs old pose {shown:?}"
        );
    }

    #[test]
    fn offset_decays_to_under_a_millimeter_in_five_halflives() {
        let arriving = ZERO2;
        let shown = [[0.1, 0.0, 0.0], [0.0, -0.1, 0.0]];
        let mut iz = Inertial::new(2, HALF).expect("fixture rate");
        iz.switch(&shown, &ZERO2, &arriving, &ZERO2)
            .expect("fixture switch");
        let mut out = [[0.0; 3]; 2];
        for _ in 0..9 {
            iz.step(&arriving, DT, &mut out).expect("fixture step");
        }
        let left = gap(&out, &arriving);
        assert!(left < 1e-3, "five halflives left {left} m from 0.1 m");
        for _ in 0..21 {
            iz.step(&arriving, DT, &mut out).expect("fixture step");
        }
        let settled = gap(&out, &arriving);
        assert!(settled < 1e-5, "after 30 ticks {settled} m left");
    }

    #[test]
    fn zero_velocity_error_never_grows() {
        // With no offset velocity the decay is monotone: no tick's error
        // passes the error at the switch.
        let arriving = ZERO2;
        let shown = [[0.1, 0.05, -0.08], [-0.02, 0.0, 0.1]];
        let mut iz = Inertial::new(2, HALF).expect("fixture rate");
        iz.switch(&shown, &ZERO2, &arriving, &ZERO2)
            .expect("fixture switch");
        let at_switch = gap(&shown, &arriving);
        let mut out = [[0.0; 3]; 2];
        let mut last = at_switch;
        for tick in 0..60 {
            iz.step(&arriving, DT, &mut out).expect("fixture step");
            let now = gap(&out, &arriving);
            assert!(now <= last + 1e-7, "tick {tick}: error {now} past {last}");
            last = now;
        }
    }

    #[test]
    fn carried_velocity_error_stays_under_the_spring_bound() {
        // A joint moving away from its arriving pose carries on for a moment:
        // the error passes its value at the switch (5 cm here, peaking near
        // 8.7 cm), stays under the spring bound, and still decays to zero.
        let arriving = [[0.0; 3]];
        let mut iz = Inertial::new(1, HALF).expect("fixture rate");
        iz.switch(
            &[[0.05, 0.0, 0.0]],
            &[[6.0, 0.0, 0.0]],
            &arriving,
            &[[0.0; 3]],
        )
        .expect("fixture switch");
        let bound = spring_bound(0.05, 6.0);
        let mut out = [[0.0; 3]];
        let mut peak = 0.0f32;
        for tick in 0..120 {
            iz.step(&arriving, DT, &mut out).expect("fixture step");
            let now = gap(&out, &arriving);
            assert!(now <= bound + 1e-6, "tick {tick}: error {now} past {bound}");
            peak = peak.max(now);
        }
        assert!(peak > 0.07, "the carry must show: peak {peak}");
        assert!(
            gap(&out, &arriving) < 1e-5,
            "settle {}",
            gap(&out, &arriving)
        );
    }

    #[test]
    fn sixty_and_thirty_hertz_stepping_agree() {
        // The offset decays exactly, so 18 ticks at 60 Hz and 9 ticks at 30 Hz
        // land on the same pose while the arriving clip moves.
        let arriving_at = |t: f32| [[0.8 * t, 1.0, -0.3 * t], [0.0, 0.2 * t, 0.0]];
        let shown = [[0.1, 1.05, 0.02], [0.3, 0.2, 0.0]];
        let shown_vel = [[1.0, 0.0, 0.0], [0.0, -0.5, 0.2]];
        let arriving_vel = [[0.8, 0.0, -0.3], [0.0, 0.2, 0.0]];
        let mut fast = Inertial::new(2, HALF).expect("fixture rate");
        fast.switch(&shown, &shown_vel, &arriving_at(0.0), &arriving_vel)
            .expect("fixture switch");
        let mut slow = fast.clone();
        let (mut out_fast, mut out_slow) = ([[0.0; 3]; 2], [[0.0; 3]; 2]);
        for k in 1..=18u32 {
            let t = k as f32 * DT;
            fast.step(&arriving_at(t), DT, &mut out_fast)
                .expect("60 Hz step");
        }
        for k in 1..=9u32 {
            let t = k as f32 / 30.0;
            slow.step(&arriving_at(t), 1.0 / 30.0, &mut out_slow)
                .expect("30 Hz step");
        }
        let split = gap(&out_fast, &out_slow);
        assert!(split <= 1e-4, "60 Hz and 30 Hz split by {split}");
    }

    #[test]
    fn second_switch_mid_blend_starts_from_what_was_shown() {
        // The shown pose already carries the first offset, so the second
        // offset is shown minus the new arriving pose: nothing is counted twice.
        let rest = [[0.0; 3]];
        let mut iz = Inertial::new(1, HALF).expect("fixture rate");
        iz.switch(&rest, &[[1.0, 0.0, 0.0]], &rest, &[[0.0; 3]])
            .expect("first switch");
        let mut shown = rest;
        let mut before = shown;
        for _ in 0..3 {
            before = shown;
            iz.step(&rest, DT, &mut shown).expect("first blend");
        }
        let shown_vel = [[(shown[0][0] - before[0][0]) / DT, 0.0, 0.0]];
        let next = [[0.2, 0.0, 0.0]];
        iz.switch(&shown, &shown_vel, &next, &[[0.0; 3]])
            .expect("second switch");
        let now = gap(&shown, &next);
        assert!(
            (iz.max_offset() - now).abs() <= 1e-6,
            "offset {} vs gap {now}",
            iz.max_offset()
        );
        let mut out = [[0.0; 3]];
        iz.step(&next, 0.0, &mut out).expect("switch frame");
        assert!(
            gap(&out, &shown) <= 1e-5,
            "switch frame {out:?} vs shown {shown:?}"
        );
        let bound = spring_bound(shown[0][0] - next[0][0], shown_vel[0][0]);
        for tick in 0..60 {
            iz.step(&next, DT, &mut out).expect("second blend");
            let err = gap(&out, &next);
            assert!(err <= bound + 1e-6, "tick {tick}: error {err} past {bound}");
        }
        assert!(gap(&out, &next) < 1e-5, "settle {}", gap(&out, &next));
    }

    #[test]
    fn hostile_input_is_refused_and_changes_nothing() {
        assert!(matches!(
            Inertial::new(1, f32::NAN),
            Err(InertialError::BadHalflife(_))
        ));
        assert_eq!(Inertial::new(1, 0.0), Err(InertialError::BadHalflife(0.0)));
        assert_eq!(Inertial::new(1, 2.0), Err(InertialError::BadHalflife(2.0)));
        assert_eq!(
            Inertial::new(MAX_JOINTS + 1, HALF),
            Err(InertialError::TooManyJoints(MAX_JOINTS + 1))
        );

        let mut iz = Inertial::new(2, HALF).expect("fixture rate");
        iz.switch(&[[0.1, 0.0, 0.0], [0.0, 0.1, 0.0]], &ZERO2, &ZERO2, &ZERO2)
            .expect("fixture switch");
        let kept = iz.clone();
        let ok = ZERO2;
        assert_eq!(
            iz.switch(&[[f32::NAN, 0.0, 0.0], [0.0; 3]], &ok, &ok, &ok),
            Err(InertialError::NonFinite("shown"))
        );
        assert_eq!(
            iz.switch(&[[1.0e9, 0.0, 0.0], [0.0; 3]], &ok, &ok, &ok),
            Err(InertialError::OutOfRange("shown"))
        );
        assert_eq!(
            iz.switch(&ok, &[[1.0e4, 0.0, 0.0], [0.0; 3]], &ok, &ok),
            Err(InertialError::OutOfRange("shown_vel"))
        );
        assert_eq!(
            iz.switch(&ok, &ok, &[[0.0; 3]], &ok),
            Err(InertialError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        assert_eq!(
            iz.switch(&ok, &ok, &ok, &[[f32::INFINITY, 0.0, 0.0], [0.0; 3]]),
            Err(InertialError::NonFinite("arriving_vel"))
        );

        let mut out = [[7.0; 3]; 2];
        let mut short = [[7.0; 3]];
        assert_eq!(
            iz.step(&[[f32::NAN, 0.0, 0.0], [0.0; 3]], DT, &mut out),
            Err(InertialError::NonFinite("arriving"))
        );
        assert!(matches!(
            iz.step(&ok, f32::NAN, &mut out),
            Err(InertialError::BadStep(_))
        ));
        assert_eq!(
            iz.step(&ok, -DT, &mut out),
            Err(InertialError::BadStep(-DT))
        );
        assert_eq!(
            iz.step(&ok, MAX_DT_S + 1.0, &mut out),
            Err(InertialError::BadStep(MAX_DT_S + 1.0))
        );
        assert_eq!(
            iz.step(&ok, DT, &mut short),
            Err(InertialError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        assert_eq!(out, [[7.0; 3]; 2], "refusals write nothing");
        assert_eq!(iz, kept, "refusals change no state");
        let text = InertialError::NonFinite("shown").to_string();
        assert!(text.contains("shown"), "{text}");
    }

    #[test]
    fn replay_is_bit_exact() {
        let run = || {
            let mut iz = Inertial::new(2, HALF).expect("fixture rate");
            let mut out = [[0.0; 3]; 2];
            let mut bits = Vec::new();
            let arriving = |t: f32| [[0.6 * t, 0.9, 0.1], [0.0, -0.2 * t, 0.3]];
            iz.switch(
                &[[0.2, 0.9, 0.1], [0.0, 0.1, 0.3]],
                &[[2.0, 0.0, 0.0], [0.0, 0.5, 0.0]],
                &arriving(0.0),
                &[[0.6, 0.0, 0.0], [0.0, -0.2, 0.0]],
            )
            .expect("fixture switch");
            for k in 1..=40u32 {
                let t = k as f32 * DT;
                if k == 20 {
                    let shown_vel = [[0.1, 0.0, 0.0], [0.0; 3]];
                    iz.switch(
                        &out,
                        &shown_vel,
                        &arriving(t),
                        &[[0.6, 0.0, 0.0], [0.0, -0.2, 0.0]],
                    )
                    .expect("second switch");
                }
                iz.step(&arriving(t), DT, &mut out).expect("fixture step");
                bits.extend(out.iter().flatten().map(|v| v.to_bits()));
            }
            bits
        };
        assert_eq!(run(), run(), "same script, same bits");
    }

    #[test]
    fn steady_state_allocates_nothing() {
        let rest = [[0.0; 3]; 8];
        let mut iz = Inertial::new(8, HALF).expect("fixture rate");
        iz.switch(&[[0.1, 0.0, 0.0]; 8], &[[1.0, 0.0, 0.0]; 8], &rest, &rest)
            .expect("fixture switch");
        let mut out = [[0.0; 3]; 8];
        for _ in 0..8 {
            iz.step(&rest, DT, &mut out).expect("warmup step");
        }
        let ((), heap) = measure(|| {
            for _ in 0..240 {
                iz.step(&rest, DT, &mut out).expect("steady step");
                std::hint::black_box(&out);
            }
        });
        assert_eq!(heap.calls, 0, "inertial steps must not allocate");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test-only libm oracle checks exp_neg; production stays on basic IEEE operations"
    )]
    fn exp_neg_matches_libm_within_a_part_per_100k() {
        for x in [
            0.0f32, 0.01, 0.5, 1.0, 2.0, 5.0, 7.0, 10.0, 20.0, 40.0, 60.0, 80.0,
        ] {
            let want = (-x).exp();
            let got = exp_neg(x);
            assert!(
                (got - want).abs() <= 1e-5 * want,
                "e^-{x}: got {got} want {want}"
            );
        }
        assert_eq!(exp_neg(100.0), 0.0, "past the cutoff the answer is zero");
    }

    #[test]
    fn fast_joint_cut_to_rest_misses_less_than_a_raw_cut() {
        // A joint moving 6 m/s is cut to a resting clip. A raw cut misses its
        // constant-velocity prediction by the full 0.10 m (6 m/s over one tick);
        // with the carried velocity the miss is about 5 cm at this halflife.
        let mut iz = Inertial::new(1, HALF).expect("fixture rate");
        iz.switch(&[[0.0; 3]], &[[6.0, 0.0, 0.0]], &[[0.0; 3]], &[[0.0; 3]])
            .expect("fixture switch");
        let mut out = [[0.0; 3]];
        iz.step(&[[0.0; 3]], DT, &mut out).expect("fixture step");
        let miss = 6.0 * DT - out[0][0];
        assert!(miss > 0.0 && miss < 0.06, "carried miss {miss} m");
    }

    #[test]
    fn offsets_after_reads_the_same_state_stepping_reaches() {
        // The pure read at age k * DT is the offset k steps would leave, at
        // zero age it is the switch offset itself, and no read moves the set.
        let shown = [[0.3, 1.2, -0.1], [0.0, 0.9, 0.4]];
        let mut iz = Inertial::new(2, HALF).expect("fixture rate");
        iz.switch(
            &shown,
            &[[1.0, 0.0, 0.0], [0.0, -2.0, 0.5]],
            &ZERO2,
            &[[0.5, 0.0, 0.0], [0.0; 3]],
        )
        .expect("fixture switch");
        let kept = iz.clone();
        let mut read = [[0.0; 3]; 2];
        iz.offsets_after(0.0, &mut read).expect("zero age");
        assert_eq!(read, shown, "age zero is shown minus arriving");
        let mut stepped = iz.clone();
        let mut out = [[0.0; 3]; 2];
        for k in 1..=30u32 {
            stepped.step(&ZERO2, DT, &mut out).expect("fixture step");
            iz.offsets_after(k as f32 * DT, &mut read)
                .expect("fixture read");
            let split = gap(&read, &out);
            assert!(split <= 1e-5, "tick {k}: read vs stepped differ by {split}");
        }
        let mut again = [[0.0; 3]; 2];
        iz.offsets_after(0.25, &mut read).expect("read");
        iz.offsets_after(0.25, &mut again).expect("read again");
        assert_eq!(read, again, "the same age reads the same bits");
        assert_eq!(iz, kept, "reads change no state");
    }

    #[test]
    fn offsets_after_refuses_a_bad_age_or_lane_count() {
        let mut iz = Inertial::new(2, HALF).expect("fixture rate");
        iz.switch(&[[0.1, 0.0, 0.0], [0.0, 0.1, 0.0]], &ZERO2, &ZERO2, &ZERO2)
            .expect("fixture switch");
        let mut out = [[7.0; 3]; 2];
        for bad in [f32::NAN, f32::INFINITY, -DT, MAX_AGE_S + 1.0] {
            assert!(
                matches!(
                    iz.offsets_after(bad, &mut out),
                    Err(InertialError::BadAge(_))
                ),
                "age {bad} must be refused"
            );
        }
        let mut short = [[7.0; 3]];
        assert_eq!(
            iz.offsets_after(DT, &mut short),
            Err(InertialError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        assert_eq!(out, [[7.0; 3]; 2], "refusals write nothing");
        assert_eq!(short, [[7.0; 3]], "refusals write nothing");
        iz.offsets_after(MAX_AGE_S, &mut out).expect("longest age");
        assert_eq!(out, ZERO2, "a minute on, the offset is gone");
        let text = InertialError::BadAge(-1.0).to_string();
        assert!(text.contains("age"), "{text}");
    }
}
