//! Contacts pin: AABB contacts solve by sequential-impulse (Baumgarte
//! split plus accumulated velocity impulse), never by XPBD compliance
//! (`alpha / h^2`). Joints alone read `compliance`; contacts ignore it.
//!
//! Covers both engines: [`Solver`] (`step` / `step_with_xpbd`) and the
//! explicit [`Schedule`] (`SolveContacts` before `SolveJoints`).

use engine_physics::solver::{xpbd_delta_lambda, xpbd_h, Config, Solver, XpbdJoint};
use engine_physics::substep::{Joint, Schedule, ScheduleConfig};
use engine_physics::Body;

const DT: f32 = 1.0 / 60.0;

/// Two overlapping boxes, high off the ground so the grounded infinite-mass
/// rule stays out: `ox = 0.1`, `oy = 1.0`, min axis is x.
fn overlap_pair() -> Vec<Body> {
    vec![
        Body {
            x: 0.0,
            y: 5.0,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        },
        Body {
            x: 0.9,
            y: 5.0,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        },
    ]
}

fn solver_no_gravity() -> Solver {
    let mut cfg = Config::default();
    cfg.gravity = 0.0;
    Solver::with_config(cfg)
}

/// Min-axis penetration of the pair, or 0 when separated.
fn pair_pen(b: &[Body]) -> f32 {
    let ox = b[0].hx + b[1].hx - (b[1].x - b[0].x).abs();
    let oy = b[0].hy + b[1].hy - (b[1].y - b[0].y).abs();
    if ox <= 0.0 || oy <= 0.0 {
        0.0
    } else {
        ox.min(oy)
    }
}

#[test]
fn contacts_separate_hard_via_sequential_impulse() {
    // Baumgarte shape the contact must use: slop 0.5 mm, beta 0.8.
    assert_eq!(Config::default().slop, 0.0005);
    assert_eq!(Config::default().baumgarte, 0.8);
    // What XPBD compliance would do to the same 0.1 m error: soften it.
    let h = xpbd_h(DT, 4);
    let rigid = xpbd_delta_lambda(0.1, 2.0, 0.0, h);
    let soft = xpbd_delta_lambda(0.1, 2.0, 1.0, h);
    assert!(rigid < 0.0);
    assert!(
        soft > rigid && soft < 0.0,
        "compliance must soften the kick"
    );

    let run = || {
        let mut s = solver_no_gravity();
        let mut b = overlap_pair();
        for _ in 0..240 {
            s.step(&mut b, DT);
        }
        (b, s)
    };
    let (b1, s1) = run();
    let (b2, s2) = run();
    assert_eq!(
        s1.state_hash(&b1),
        s2.state_hash(&b2),
        "same seed + steps = same hash"
    );
    let pen = pair_pen(&b1);
    assert!(
        pen < 0.001,
        "sequential-impulse drives 0.1 m overlap to slop, got {pen:.6} m"
    );
    for m in &b1 {
        assert!(
            m.x.is_finite() && m.y.is_finite() && m.vx.is_finite() && m.vy.is_finite(),
            "no NaN escape: {m:?}"
        );
    }
    // Restitution 0: no bounce apart, no lateral drift.
    let dx = (b1[1].x - b1[0].x).abs();
    assert!(
        dx >= 1.0 - 0.002 && dx <= 1.0 + 0.05,
        "hard rest near 1.0 m apart, got {dx:.6} m"
    );
    assert!(b1[0].y.abs() < 5.0 + 0.05 && (b1[1].y - 5.0).abs() < 0.05);
}

#[test]
fn contacts_empty_joints_match_plain_step() {
    // No joints: the XPBD entry must answer the plain contact path bit-for-bit.
    fn plain() -> (Vec<Body>, u64) {
        let mut s = solver_no_gravity();
        let mut b = overlap_pair();
        for _ in 0..240 {
            s.step(&mut b, DT);
        }
        let h = s.state_hash(&b);
        (b, h)
    }
    fn xpbd_empty() -> (Vec<Body>, u64) {
        let mut s = solver_no_gravity();
        let mut b = overlap_pair();
        let mut joints: Vec<XpbdJoint> = vec![];
        for _ in 0..240 {
            s.step_with_xpbd(&mut b, DT, &mut joints);
        }
        let h = s.state_hash(&b);
        (b, h)
    }
    let (bp, hp) = plain();
    let (bx, hx) = xpbd_empty();
    assert_eq!(hp, hx, "empty-joint run must match plain step");
    assert_eq!(bp[0].x.to_bits(), bx[0].x.to_bits());
    assert_eq!(bp[1].x.to_bits(), bx[1].x.to_bits());
}

#[test]
fn contacts_ignore_xpbd_joint_compliance_solver() {
    // Bodies 0-1 contact; bodies 2-3 carry the joint far away (x ~ 10) so no
    // cross pair exists. Varying the joint compliance must move 2-3 only.
    fn fixture() -> Vec<Body> {
        vec![
            Body {
                x: 0.0,
                y: 5.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: 0.9,
                y: 5.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: 10.0,
                y: 5.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: 11.2,
                y: 5.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
        ]
    }
    fn run(compliance: f32) -> Vec<Body> {
        let mut s = solver_no_gravity();
        let mut b = fixture();
        // Rest 1.0 m in x, 0 in y; the pair starts 1.2 m apart (C = 0.2 m).
        let mut joints = vec![XpbdJoint::new(2, 3, 1000, 0, compliance)];
        for _ in 0..240 {
            s.step_with_xpbd(&mut b, DT, &mut joints);
        }
        b
    }
    let rigid = run(0.0);
    let soft = run(1.0);
    // Contacts ignore compliance: bit-identical contact pair.
    for i in [0, 1] {
        assert_eq!(
            (rigid[i].x.to_bits(), rigid[i].y.to_bits()),
            (soft[i].x.to_bits(), soft[i].y.to_bits()),
            "contact body {i} must not read joint compliance"
        );
    }
    assert!(
        pair_pen(&rigid) < 0.001,
        "contacts stay hard under a soft joint, pen {:.6} m",
        pair_pen(&rigid)
    );
    assert!(
        pair_pen(&soft) < 0.001,
        "contacts stay hard under a soft joint, pen {:.6} m",
        pair_pen(&soft)
    );
    // The joint itself obeys compliance: rigid closes, soft hangs open.
    let gap = |b: &[Body]| (b[3].x - b[2].x).abs();
    let rigid_drift = (gap(&rigid) - 1.0).abs();
    let soft_drift = (gap(&soft) - 1.0).abs();
    assert!(
        rigid_drift < 0.01,
        "rigid joint must close, drift {rigid_drift:.6} m"
    );
    assert!(
        soft_drift > rigid_drift + 0.05,
        "soft joint must hang open: soft {soft_drift:.6} m vs rigid {rigid_drift:.6} m"
    );
}

#[test]
fn schedule_contacts_ignore_joint_compliance() {
    // Same split on the explicit schedule: SolveContacts is Baumgarte, only
    // SolveJoints reads compliance.
    assert_eq!(engine_physics::substep::CONTACT_SLOP, 0.0005);
    assert_eq!(engine_physics::substep::CONTACT_BAUMGARTE, 0.8);
    fn bodies() -> Vec<Body> {
        vec![
            Body::new(0.0, 5.0, 0.0, 0.0, 0.5, 0.5),
            Body::new(0.9, 5.0, 0.0, 0.0, 0.5, 0.5),
            Body::new(10.0, 5.0, 0.0, 0.0, 0.5, 0.5),
            Body::new(11.2, 5.0, 0.0, 0.0, 0.5, 0.5),
        ]
    }
    fn run(compliance: f32) -> (Vec<Body>, Schedule) {
        let cfg = ScheduleConfig::new(4, 10, 0.0).expect("valid cfg");
        let mut schedule = Schedule::with_config(cfg);
        schedule
            .add_joint(Joint::with_compliance(2, 3, 1.0, compliance).expect("valid joint"))
            .expect("add joint");
        let mut b = bodies();
        for _ in 0..120 {
            schedule.step(&mut b, DT);
        }
        (b, schedule)
    }
    let (rigid_b, rigid) = run(0.0);
    let (soft_b, soft) = run(0.5);
    for i in [0, 1] {
        assert_eq!(
            (rigid_b[i].x.to_bits(), rigid_b[i].y.to_bits()),
            (soft_b[i].x.to_bits(), soft_b[i].y.to_bits()),
            "schedule contact body {i} must not read joint compliance"
        );
    }
    let pen = {
        let ox = rigid_b[0].hx + rigid_b[1].hx - (rigid_b[1].x - rigid_b[0].x).abs();
        let oy = rigid_b[0].hy + rigid_b[1].hy - (rigid_b[1].y - rigid_b[0].y).abs();
        ox.min(oy)
    };
    assert!(pen < 0.001, "schedule contacts stay hard, pen {pen:.6} m");
    let rigid_drift = rigid.joint_drift(&rigid_b);
    let soft_drift = soft.joint_drift(&soft_b);
    assert!(
        rigid_drift < 0.005,
        "rigid link must close, got {rigid_drift:.6} m"
    );
    assert!(
        soft_drift > rigid_drift,
        "compliance softens joints only: soft {soft_drift:.6} m vs rigid {rigid_drift:.6} m"
    );
}
