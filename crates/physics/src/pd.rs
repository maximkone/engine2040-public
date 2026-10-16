//! pd: PD velocity-servo drive law for the powered ragdoll.
//!
//! Consumer: [`crate::powered::PoweredRagdoll`] feeds each driven bone
//! through [`pd_raw`] per axis and caps the 3-vector with
//! [`clamp_vector`]. Owner: this module owns the scalar law, the magnitude
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

/// One capped servo axis: [`pd_raw`] clamped to `±max`. A bad cap (NaN,
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
}
