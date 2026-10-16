//! PD drive-law leg: the servo behind the powered ragdoll answers from
//! outside the crate (the arena hit pipeline drives the same surface:
//! [`pd_axis`] per axis, [`clamp_vector`] on the output, [`dt_valid`] on
//! the step). Units: meters, seconds, 1/s gains, m/s and rad/s caps.
//! Determinism: one error/velocity sequence maps to one output; the digest
//! below re-verifies the recovery twice.
//!
//! The fallen-chain comparison lives behind the `rapier` feature next to
//! the powered ragdoll; the scalar law legs below run on the default
//! (std-only) build.

use engine_core::Fnv64;
use engine_physics::pd::{clamp_vector, dt_valid, pd_axis, pd_raw, PD_DT_FLOOR};

/// Scalar force-mass abstraction of a knocked bone pulled home by the
/// servo at a fixed step: `v += F * dt`, `p += v * dt`, target zero.
/// Returns `(settle_ticks, overshoot_past_target)`. Never settles past 600.
fn settle_ticks(kp: f32, kd: f32, cap: f32, dt: f32) -> (u32, f32) {
    let (mut p, mut v) = (1.0f32, 0.0f32);
    let mut deepest = 0.0f32;
    let mut calm = 0u32;
    for t in 0..600u32 {
        let f = pd_axis(-p, v, kp, kd, dt, cap);
        v += f * dt;
        p += v * dt;
        if p < deepest {
            deepest = p;
        }
        if p.abs() < 0.02 && v.abs() < 0.1 {
            calm += 1;
            if calm >= 12 {
                return (t + 1, -deepest);
            }
        } else {
            calm = 0;
        }
    }
    (600, -deepest)
}

#[test]
fn pd_drive_degenerate_dt_returns_p_only_without_nan() {
    // A plausible defect: dividing the error rate by a zero dt injects NaN
    // into the world. Every degenerate dt must answer the plain P term.
    let dts = [
        0.0,
        -1.0 / 60.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        1e-9,
    ];
    for dt in dts {
        assert!(!dt_valid(dt), "dt {dt} must read degenerate");
        for (error, vel) in [(1.0, 0.5), (-2.0, -3.0), (1e30, 1e30), (-1e30, 1e30)] {
            let got = pd_axis(error, vel, 12.0, 3.0, dt, 8.0);
            assert!(got.is_finite(), "dt {dt} error {error} vel {vel}: {got}");
            let p_only = (12.0f32 * error).clamp(-8.0, 8.0);
            let p_only = if p_only.is_finite() { p_only } else { 0.0 };
            assert_eq!(got, p_only, "dt {dt} must fall back to P-only");
        }
    }
    assert!(dt_valid(1.0 / 60.0));
    assert!(!dt_valid(PD_DT_FLOOR * 0.5));
}

#[test]
fn pd_drive_magnitude_clamp_caps_diagonal_output() {
    // Per-component clamps leak sqrt(3) x the cap on diagonals; the vector
    // clamp holds the ball. A plausible defect: normalizing with a zero or
    // NaN length.
    let capped = clamp_vector([3.0, 4.0, 0.0], 2.5);
    assert_eq!(capped, [1.5, 2.0, 0.0]);
    assert_eq!(clamp_vector([1.0, 1.0, 1.0], 8.0), [1.0, 1.0, 1.0]);
    let bad = clamp_vector([f32::NAN, 1.0, f32::INFINITY], 8.0);
    assert!(bad.iter().all(|c| c.is_finite()));
    let mag = (bad[0] * bad[0] + bad[1] * bad[1] + bad[2] * bad[2]).sqrt();
    assert!(mag <= 8.0 + 1e-5);
    assert_eq!(clamp_vector([1.0, 0.0, 0.0], 0.0), [0.0, 0.0, 0.0]);
    assert_eq!(clamp_vector([0.0, 0.0, 0.0], 8.0), [0.0, 0.0, 0.0]);
    // Raw law without the cap: P minus velocity damping, D skipped on bad dt.
    assert_eq!(pd_raw(1.0, 0.5, 12.0, 3.0, 1.0 / 60.0), 12.0 - 1.5);
    assert_eq!(pd_raw(1.0, 0.5, 12.0, 3.0, 0.0), 12.0);
}

#[test]
fn pd_drive_settles_faster_than_p_only_at_matched_overshoot() {
    // Fallen-chain abstraction: same P gain and cap, PD adds velocity
    // damping. The damped run must settle in at least 15% fewer ticks and
    // never overshoot past the P-only run.
    let dt = 1.0 / 60.0;
    let (p_ticks, p_over) = settle_ticks(12.0, 0.0, 8.0, dt);
    let (pd_ticks, pd_over) = settle_ticks(12.0, 4.0, 8.0, dt);
    assert!(pd_ticks < 600, "PD run must settle inside the budget");
    assert!(
        pd_ticks * 100 <= p_ticks * 85,
        "PD {pd_ticks} must beat P-only {p_ticks} by 15%"
    );
    assert!(
        pd_over <= p_over + 1e-6,
        "PD overshoot {pd_over} must not exceed P-only {p_over}"
    );
}

#[test]
fn pd_drive_digest_twice_identical() {
    // Same law plus same script hashes one way; a different D gain moves it.
    let digest = |kd: f32| {
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
    };
    assert_eq!(digest(4.0), digest(4.0));
    assert_ne!(digest(4.0), digest(0.0));
}

/// Fallen-chain leg on the real ragdoll: knock the chain flat, get up, and
/// count fixed steps until every bone rests near its animated target. The
/// PD default must close in at least 15% fewer ticks than P-only at the same
/// gains and caps, with the digest identical across twin runs.
#[cfg(feature = "rapier")]
mod chain {
    use engine_anim::contract::{Joint, LocalPose, Skeleton};
    use engine_physics::powered::{DriveParams, PoweredRagdoll};
    use engine_physics::ragdoll::BoneShape;
    use engine_physics::rapier::FIXED_DT;

    fn chain_skeleton() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                },
                Joint {
                    name: "mid".into(),
                    parent: Some(0),
                },
                Joint {
                    name: "tip".into(),
                    parent: Some(1),
                },
                Joint {
                    name: "end".into(),
                    parent: Some(2),
                },
            ],
        }
    }

    fn chain_pose() -> LocalPose {
        LocalPose {
            positions: vec![
                [0.0, 3.0, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 0.0],
                [0.0, 0.5, 0.0],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 4],
        }
    }

    fn spawn(drives: &[DriveParams]) -> PoweredRagdoll {
        let shapes = vec![
            BoneShape {
                radius: 0.15,
                half_height: 0.2,
            };
            4
        ];
        let mut r = PoweredRagdoll::spawn(&chain_skeleton(), &chain_pose(), &shapes, drives)
            .expect("spawn chain");
        r.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        r
    }

    /// Fallen-chain recovery: hold aloft, knock bone 1 so its subtree falls
    /// and swings free while the root keeps driving, then count fixed steps
    /// until every free bone calms below 0.3 m/s for 20 straight ticks.
    /// Returns `(ticks, peak_free_speed)`. Cap 1200.
    fn settle_free(drives: &[DriveParams]) -> (u32, f32) {
        let mut r = spawn(drives);
        for _ in 0..30 {
            r.step(FIXED_DT).expect("hold");
        }
        r.apply_hit(1, [0.35, 0.0, 0.0], 30).expect("hit");
        let mut calm = 0u32;
        let mut peak = 0.0f32;
        for t in 0..1200u32 {
            r.step(FIXED_DT).expect("swing");
            let mut worst = 0.0f32;
            for i in 1..4 {
                let v = r.bone_linvel(i).expect("vel");
                let s = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                worst = worst.max(s);
            }
            peak = peak.max(worst);
            if worst < 0.3 {
                calm += 1;
                if calm >= 20 {
                    return (t + 1, peak);
                }
            } else {
                calm = 0;
            }
        }
        (1200, peak)
    }

    #[test]
    fn pd_drive_fallen_chain_recovers_faster() {
        // The P-only root slams between its caps and pumps the hanging
        // subtree; the PD root glides, so the free bones calm sooner at no
        // extra overshoot. Same P gains and caps both sides: only the D
        // terms differ.
        let pd = vec![DriveParams::default_drive(); 4];
        let mut p_only = pd.clone();
        for d in &mut p_only {
            d.linear_damping = 0.0;
            d.angular_damping = 0.0;
        }
        let (p_ticks, p_peak) = settle_free(&p_only);
        let (pd_ticks, pd_peak) = settle_free(&pd);
        assert!(pd_ticks < 1200, "PD chain must settle inside the budget");
        assert!(
            pd_ticks * 100 <= p_ticks * 85,
            "PD {pd_ticks} must beat P-only {p_ticks} by 15%"
        );
        assert!(
            pd_peak <= p_peak + 0.05,
            "PD peak {pd_peak} must not exceed P-only {p_peak}"
        );
        // Twins through the PD fall hash twice, poses included.
        let run = || {
            let mut r = spawn(&pd);
            for _ in 0..30 {
                r.step(FIXED_DT).expect("hold");
            }
            r.apply_hit(1, [0.35, 0.0, 0.0], 30).expect("hit");
            for _ in 0..90 {
                r.step(FIXED_DT).expect("swing");
            }
            (r.state_hash(), r.bone_poses())
        };
        let (h1, p1) = run();
        let (h2, p2) = run();
        assert_eq!(h1, h2);
        assert_eq!(p1, p2);
    }
}
