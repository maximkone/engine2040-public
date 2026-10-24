//! pd: PD velocity-servo drive law for the powered ragdoll.
//!
//! Consumer: `crate::powered::PoweredRagdoll` feeds each driven bone
//! through `pd_raw` per axis and caps the 3-vector with
//! `clamp_vector`. Owner: this module owns the scalar law, the magnitude
//! clamp and the dt guard. Units: meters, seconds, 1/s gains, m/s and rad/s
//! caps. Errors: none; every entry sanitizes non-finite input to a finite
//! output, so the servo can never inject NaN into the world.
//!
//! Determinism: only comparisons and `+ - * /` plus one `sqrt` in the
//! magnitude clamp; no transcendentals, no allocation, no history state.
//! The D term damps on measured bone velocity: for a fixed driver target
//! the pose-error derivative equals the negated velocity, so no error
//! history is stored and the snapshot/hash shape never changes.
//!
//! Steal note: the shape (D term on the error, clamped output, dt fallback
//! to P-only) follows the `active-ragdoll-rs` PD servo idea, reimplemented
//! here in scalar form; no donor bytes are copied.
#![forbid(unsafe_code)]

/// Step below this (seconds) is degenerate: the D term is skipped and the
/// servo answers the P term only. Covers zero, negative, NaN, infinite and
/// sub-step noise.
pub const PD_DT_FLOOR: f32 = 1e-6;

/// True when `dt` can carry a D term: finite and at least [`PD_DT_FLOOR`].
pub fn dt_valid(dt: f32) -> bool {
    dt.is_finite() && dt >= PD_DT_FLOOR
}

/// PX46: PD position-motor stiffness on a powered bone joint, N.m per radian.
/// The motor holds the build-pose angle the velocity servo used to chase, so a
/// stand does not need twelve solver passes.
pub const PD_MOTOR_STIFFNESS_NM: f32 = 60.0;
/// PX46: PD position-motor damping on a powered bone joint, N.m.s per radian.
pub const PD_MOTOR_DAMPING_NM_S: f32 = 6.0;
/// PX46: torque ceiling of one powered joint motor, N.m; a spear thrust still
/// bends the joint past it instead of teleporting the bone.
pub const PD_MOTOR_MAX_NM: f32 = 60.0;
/// PX46: extra PGS iterations a powered bone body adds to its solver island.
/// Four base passes plus these eight give a bone island the twelve visits the
/// velocity servo needed; the passive ragdoll build is untouched.
pub const PD_BONE_ADDITIONAL_PGS: usize = 8;

/// Fraction of the stability edge [`stable_gains`] allows: the slowest mode then shrinks by at
/// least a tenth every tick instead of ringing on the edge.
pub const STABLE_MARGIN: f32 = 0.9;

/// The gains of the velocity servo `powered.rs` runs, clamped inside its stability region.
///
/// The world takes the commanded velocity `v' = kp * e - kd * v` and integrates it, so for one
/// bone the tick map on `(error, velocity)` has trace `1 - kp*dt - kd` and determinant `-kd`.
/// The Jury test settles it only while `kp * dt < 2` and `kd < (2 - kp * dt) / 2` (and
/// `kd < 1`). Past the edge the velocity flips sign every tick and grows until the speed cap
/// holds the bone buzzing: the shipped D of 1.0 at P 12 and 60 Hz sat past it (edge 0.9).
/// Both gains are clamped to [`STABLE_MARGIN`] of their edge, so no drive table, however
/// tuned, can build an unstable servo. A degenerate `dt` (P only, see [`dt_valid`]) returns
/// the gains unchanged; non-positive gains clamp to zero.
pub fn stable_gains(kp: f32, kd: f32, dt: f32) -> (f32, f32) {
    let (kp, kd) = (sanitize(kp).max(0.0), sanitize(kd).max(0.0));
    if !dt_valid(dt) {
        return (kp, kd);
    }
    let kp = kp.min(STABLE_MARGIN * 2.0 / dt);
    let edge = ((2.0 - kp * dt) / 2.0).min(1.0);
    (kp, kd.min(STABLE_MARGIN * edge))
}

/// Raw servo axis without the cap: `kp * error` damped by `kd * vel`.
/// Degenerate `dt` (see [`dt_valid`]) answers the P term only; non-finite
/// intermediates sanitize to zero, so the output is always finite.
pub fn pd_raw(error: f32, vel: f32, kp: f32, kd: f32, dt: f32) -> f32 {
    let p = sanitize(kp * error);
    if !dt_valid(dt) {
        return p;
    }
    sanitize(p - sanitize(kd * vel))
}

/// One capped servo axis: `pd_raw` clamped to `±max`. A bad cap (NaN,
/// infinite, zero or negative) clamps to zero; the output is always finite.
pub fn pd_axis(error: f32, vel: f32, kp: f32, kd: f32, dt: f32, max: f32) -> f32 {
    clamp1(pd_raw(error, vel, kp, kd, dt), max)
}

/// Magnitude clamp for a 3-vector servo output: inside the ball the vector
/// passes through; outside it scales to length `max` with its direction
/// kept (one `sqrt`, allowed sim math). Non-finite components sanitize to
/// zero first; a bad cap yields zero. The output is always finite with
/// length at most `max`.
pub fn clamp_vector(v: [f32; 3], max: f32) -> [f32; 3] {
    let cap = if max.is_finite() && max > 0.0 {
        max
    } else {
        return [0.0; 3];
    };
    let (x, y, z) = (sanitize(v[0]), sanitize(v[1]), sanitize(v[2]));
    let n2 = x * x + y * y + z * z;
    if !n2.is_finite() {
        return [0.0; 3];
    }
    if n2 <= cap * cap {
        [x, y, z]
    } else {
        let s = cap / n2.sqrt();
        [sanitize(x * s), sanitize(y * s), sanitize(z * s)]
    }
}

/// Clamp to `±max` with comparisons only; bad caps clamp to zero and NaN
/// maps to zero, so the result is always finite.
fn clamp1(v: f32, max: f32) -> f32 {
    let cap = if max.is_finite() && max > 0.0 {
        max
    } else {
        0.0
    };
    if v < -cap {
        -cap
    } else if v > cap {
        cap
    } else if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Replace non-finite drive terms with zero so a huge pose or velocity can
/// never inject NaN into the world.
fn sanitize(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::laws;

    /// A consumer-shaped servo axis: the bone position and the measured
    /// velocity it carries into the next tick (the body's linvel).
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Servo {
        x: f32,
        v: f32,
    }

    /// Fixed tick, 60 Hz, as the powered ragdoll steps.
    const TICK_DT: f32 = 1.0 / 60.0;

    /// One tick as `powered.rs` `drive_bone` runs it: the servo answers from
    /// the measured velocity, then the world integrates the commanded one.
    /// The target is zero.
    fn servo_tick(s: &mut Servo, kp: f32, kd: f32, max: f32) {
        let (kp, kd) = stable_gains(kp, kd, TICK_DT);
        s.v = pd_axis(-s.x, s.v, kp, kd, TICK_DT, max);
        s.x += s.v * TICK_DT;
    }

    /// Shipped linear drive (`DriveParams::default_drive`, powered.rs): P 12,
    /// D 1.0, cap 8 m/s.
    const SHIPPED: (f32, f32, f32) = (12.0, 1.0, 8.0);

    /// Damping inside the discrete stability edge at 60 Hz with P 12: the
    /// lagged D term needs `kd < (2 - kp * dt) / 2`, which is 0.9 here.
    const SETTLING: (f32, f32, f32) = (12.0, 0.5, 8.0);

    /// Error magnitude, meters. Slack for these laws is f32 rounding at unit
    /// scale (about 1e-7), so 1e-6 is the honest floor, not a tuned one.
    fn error(s: &Servo) -> f64 {
        f64::from(s.x.abs())
    }

    #[test]
    fn pd_drive_p_only_matches_plain_gain_when_still() {
        // No motion, valid dt: PD equals the plain P term on every axis sign.
        for error in [-2.0, -0.1, 0.0, 0.5, 3.0] {
            let capped = (12.0f32 * error).clamp(-8.0, 8.0);
            assert_eq!(pd_axis(error, 0.0, 12.0, 3.0, 1.0 / 60.0, 8.0), capped);
        }
        assert_eq!(pd_raw(0.5, 0.0, 12.0, 3.0, 1.0 / 60.0), 6.0);
    }

    #[test]
    fn pd_drive_damping_brakes_motion_toward_target() {
        // Racing toward the target, damping brakes below P; fleeing, it
        // pushes back harder than P. A plausible defect: adding the D term
        // with the wrong sign, which pumps energy in instead.
        let p = 12.0f32 * 1.0;
        let braking = pd_raw(1.0, 2.0, 12.0, 3.0, 1.0 / 60.0);
        assert_eq!(braking, p - 6.0);
        let chasing = pd_raw(1.0, -2.0, 12.0, 3.0, 1.0 / 60.0);
        assert_eq!(chasing, p + 6.0);
        assert!(braking < p && p < chasing);
    }

    #[test]
    fn pd_drive_degenerate_dt_returns_p_only_without_nan() {
        // Dividing an error rate by a zero dt is the NaN source; every
        // degenerate dt must answer the plain P term, finite always.
        for dt in [
            0.0,
            -1.0 / 60.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e-9,
        ] {
            assert!(!dt_valid(dt));
            for (error, vel) in [(1.0, 0.5), (-2.0, -3.0), (1e30, 1e30), (-1e30, 1e30)] {
                let got = pd_axis(error, vel, 12.0, 3.0, dt, 8.0);
                assert!(got.is_finite(), "dt {dt} error {error} vel {vel}: {got}");
                let mut p_only = 12.0f32 * error;
                if !p_only.is_finite() {
                    p_only = 0.0;
                }
                let p_only = p_only.clamp(-8.0, 8.0);
                assert_eq!(got, p_only, "dt {dt} must fall back to P-only");
            }
            assert!(pd_raw(1.0, 5.0, 12.0, 3.0, dt).is_finite());
        }
        assert!(dt_valid(1.0 / 60.0));
        assert!(!dt_valid(PD_DT_FLOOR * 0.5));
    }

    #[test]
    fn pd_drive_magnitude_clamp_caps_diagonal_output() {
        // Per-component clamps leak sqrt(3) x the cap on diagonals; the
        // vector clamp holds the ball and keeps the direction.
        assert_eq!(clamp_vector([3.0, 4.0, 0.0], 2.5), [1.5, 2.0, 0.0]);
        assert_eq!(clamp_vector([1.0, 1.0, 1.0], 8.0), [1.0, 1.0, 1.0]);
        assert_eq!(clamp_vector([0.0, 0.0, 0.0], 8.0), [0.0, 0.0, 0.0]);
        assert_eq!(clamp_vector([1.0, 0.0, 0.0], 0.0), [0.0, 0.0, 0.0]);
        assert_eq!(clamp_vector([1.0, 0.0, 0.0], f32::NAN), [0.0, 0.0, 0.0]);
        let bad = clamp_vector([f32::NAN, 1.0, f32::INFINITY], 8.0);
        assert!(bad.iter().all(|c| c.is_finite()));
        let mag = (bad[0] * bad[0] + bad[1] * bad[1] + bad[2] * bad[2]).sqrt();
        assert!(mag <= 8.0 + 1e-5);
    }

    #[test]
    fn pd_drive_servo_path_alloc_free() {
        // The tick path borrows nothing and boxes nothing: 90 servo answers
        // plus clamps must not touch the heap.
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            let mut acc = 0.0f32;
            for i in 0..90u8 {
                let e = 1.0 - f32::from(i) * 0.01;
                let v = clamp_vector(
                    [
                        pd_raw(e, 0.5, 12.0, 3.0, 1.0 / 60.0),
                        pd_raw(-e, -0.5, 12.0, 3.0, 1.0 / 60.0),
                        pd_raw(e * 0.5, 0.25, 10.0, 2.0, 1.0 / 60.0),
                    ],
                    8.0,
                );
                acc += v[0] + v[1] + v[2];
            }
            assert!(acc.is_finite());
        });
        assert_eq!(allocs.calls, 0, "servo path must not allocate: {allocs:?}");
    }

    #[test]
    fn pd_drive_digest_twice_identical() {
        // Same law plus same script hashes one way; a different D gain
        // moves the digest.
        fn digest(kd: f32) -> u64 {
            use engine_core::Fnv64;
            let mut h = Fnv64::new();
            let (mut p, mut v) = (1.0f32, 0.0f32);
            let dt = 1.0 / 60.0;
            for _ in 0..90 {
                let f = pd_axis(-p, v, 12.0, kd, dt, 8.0);
                v += f * dt;
                p += v * dt;
                h.write_i64((p * 1_000_000.0).round() as i64);
                h.write_i64((v * 1_000_000.0).round() as i64);
            }
            h.finish()
        }
        assert_eq!(digest(3.0), digest(3.0));
        assert_ne!(digest(3.0), digest(0.0));
    }

    #[test]
    fn pd_servo_at_its_target_stays_at_rest() {
        let (kp, kd, max) = SHIPPED;
        let at_target = Servo { x: 0.0, v: 0.0 };
        laws::rest_stays_rest(at_target, 600, |s| servo_tick(s, kp, kd, max))
            .expect("a servo at its target with zero velocity must not move");
    }

    #[test]
    fn pd_servo_from_rest_error_never_grows_and_never_overshoots() {
        let (kp, kd, max) = SETTLING;
        let start = Servo { x: 1.0, v: 0.0 };
        laws::energy_never_rises(start, 600, |s| servo_tick(s, kp, kd, max), error, 1e-6)
            .expect("settling damping must not let the error grow from rest");
        let mut s = start;
        for tick in 1..=600 {
            servo_tick(&mut s, kp, kd, max);
            assert!(
                s.x >= 0.0,
                "overshoot past the target at tick {tick}: {s:?}"
            );
        }
    }

    #[test]
    fn pd_servo_potential_never_rises_from_rest() {
        // The servo owns no mass, so its 0.5 v^2 term is not its to conserve:
        // the spring term 0.5 kp e^2 alone is the servo's own energy. Adding
        // 0.5 v^2 would rise on the first tick (0.06 to 0.76 at x 0.1, kd 0.5),
        // because the command sets the velocity from nothing.
        let (kp, kd, max) = SETTLING;
        let spring = move |s: &Servo| 0.5 * f64::from(kp) * f64::from(s.x).powi(2);
        laws::energy_never_rises(
            Servo { x: 1.0, v: 0.0 },
            600,
            |s| servo_tick(s, kp, kd, max),
            spring,
            1e-6,
        )
        .expect("the spring term must not rise from rest");
    }

    #[test]
    fn pd_servo_mirrored_error_drives_mirrored() {
        let (kp, kd, max) = SHIPPED;
        let east = Servo { x: 0.3, v: 0.0 };
        let west = Servo { x: -0.3, v: 0.0 };
        laws::mirror(
            east,
            west,
            600,
            |s, _| servo_tick(s, kp, kd, max),
            |a, b| a.x == -b.x && a.v == -b.v,
        )
        .expect("the servo is odd in error and velocity, so mirrored starts stay mirrored");
    }

    #[test]
    fn pd_servo_unclamped_shipped_damping_buzzes_at_its_cap() {
        // The bug the clamp closes: D 1.0 at P 12 and 60 Hz is past the edge (0.9), so the
        // sign-flipping mode grows about 10% a tick until the cap holds the bone buzzing.
        let (kp, kd, max) = SHIPPED;
        let mut s = Servo { x: 1.0, v: 0.0 };
        let mut ring = 0.0f32;
        for tick in 0..300 {
            s.v = pd_axis(-s.x, s.v, kp, kd, TICK_DT, max);
            s.x += s.v * TICK_DT;
            if tick >= 240 {
                ring = ring.max(s.v.abs());
            }
        }
        assert_eq!(
            ring, max,
            "the unclamped servo should ring at its cap: {s:?}"
        );
    }

    #[test]
    fn pd_servo_at_shipped_damping_settles_from_rest() {
        // Through `stable_gains`, as `drive_bone` runs it: D 1.0 clamps to 0.81, the error
        // never passes its start, and the bone is within a millimetre inside two seconds.
        let (kp, kd, max) = SHIPPED;
        let (_, kd_run) = stable_gains(kp, kd, TICK_DT);
        assert!(
            kd_run < 0.9 && kd_run > 0.8,
            "shipped D clamps just inside the edge: {kd_run}"
        );
        let mut s = Servo { x: 1.0, v: 0.0 };
        for tick in 0..240 {
            servo_tick(&mut s, kp, kd, max);
            assert!(
                s.x.abs() <= 1.0,
                "tick {tick}: the error passed its start: {s:?}"
            );
        }
        assert!(s.x.abs() < 1e-3 && s.v.abs() < 1e-2, "not settled: {s:?}");
    }

    #[test]
    fn pd_servo_admitted_large_gain_never_grows_its_error() {
        // P 150 at 60 Hz (kp*dt 2.5) diverged; clamped to kp*dt 1.8 the error turns to -0.8 of itself
        // each tick, sign flipping, never growing.
        let start = Servo { x: 0.01, v: 0.0 };
        laws::energy_never_rises(start, 30, |s| servo_tick(s, 150.0, 0.0, 8.0), error, 1e-6)
            .expect("an admitted gain inside the drive range must not diverge");
        for (kp, kd) in [(1_000.0, 1_000.0), (12.0, 1.0), (59.9, 0.95), (0.0, 0.0)] {
            let (p, d) = stable_gains(kp, kd, TICK_DT);
            let a = p * TICK_DT;
            assert!(
                a < 2.0 && d < (2.0 - a) / 2.0 && d < 1.0,
                "({kp}, {kd}) -> ({p}, {d})"
            );
        }
        assert_eq!(
            stable_gains(12.0, 0.5, TICK_DT),
            (12.0, 0.5),
            "stable gains pass through"
        );
    }
}
