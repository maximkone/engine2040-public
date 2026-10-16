//! physics_substep: schedule + hash-stability regression for `substep.rs`.
//!
//! Owner: `engine-physics` substep schedule. Units: meters, seconds, 60 Hz
//! ticks. Determinism: fixed substep count with `PHASE_ORDER`, index order,
//! `+ - * /` and `sqrt` only. Errors: `ScheduleError`, never a panic.
//!
//! What `tests/contacts.rs` does not cover: phase-order fingerprint and
//! state-hash stability across identical tapes, substep/iteration moves of
//! the hash, joint insertion-order invariance, config-edge validation, and
//! `dt`/empty/out-of-range edges. That file pins contact separation and
//! compliance isolation only.

use engine_physics::rigid::FIXED_DT;
use engine_physics::substep::{
    Joint, Phase, Schedule, ScheduleConfig, ScheduleError, DEFAULT_GRAVITY, DEFAULT_ITERATIONS,
    DEFAULT_SUBSTEPS, MAX_GRAVITY, MAX_ITERATIONS, MAX_REST_M, MAX_SUBSTEPS, PHASE_ORDER,
};
use engine_physics::Body;

const DT: f32 = FIXED_DT;

/// Two bodies linked at rest 1.0 m, high off the ground.
fn linked_pair() -> Vec<Body> {
    vec![
        Body::new(0.0, 3.0, 0.4, 0.0, 0.25, 0.25),
        Body::new(1.0, 3.2, -0.2, 0.0, 0.25, 0.25),
    ]
}

fn schedule_with(substeps: usize, iterations: usize, gravity: f32) -> Schedule {
    let cfg = ScheduleConfig::new(substeps, iterations, gravity).expect("valid cfg");
    Schedule::with_config(cfg)
}

fn run_tape(substeps: usize, ticks: usize) -> (Vec<Body>, Schedule) {
    let mut schedule = schedule_with(substeps, DEFAULT_ITERATIONS, DEFAULT_GRAVITY);
    schedule
        .add_joint(Joint::new(0, 1, 1.0).expect("valid joint"))
        .expect("add joint");
    let mut bodies = linked_pair();
    for _ in 0..ticks {
        schedule.step(&mut bodies, DT);
    }
    (bodies, schedule)
}

#[test]
fn phase_order_pinned_and_in_schedule_hash() {
    assert_eq!(
        PHASE_ORDER,
        [
            Phase::Integrate,
            Phase::SolveContacts,
            Phase::SolveJoints,
            Phase::UpdateSleep,
        ],
        "explicit order: integrate, contacts, joints, sleep"
    );
    let base = Schedule::new();
    assert_eq!(base.substeps(), DEFAULT_SUBSTEPS);
    let fingerprint = base.schedule_hash();
    let again = Schedule::new();
    assert_eq!(fingerprint, again.schedule_hash());
    let two = schedule_with(2, DEFAULT_ITERATIONS, DEFAULT_GRAVITY);
    assert_ne!(fingerprint, two.schedule_hash());
    let many_iters = schedule_with(DEFAULT_SUBSTEPS, DEFAULT_ITERATIONS + 1, DEFAULT_GRAVITY);
    assert_ne!(fingerprint, many_iters.schedule_hash());
}

#[test]
fn identical_tapes_share_state_hash_while_substeps_move_it() {
    let (b1, s1) = run_tape(DEFAULT_SUBSTEPS, 120);
    let (b2, s2) = run_tape(DEFAULT_SUBSTEPS, 120);
    assert_eq!(
        s1.state_hash(&b1),
        s2.state_hash(&b2),
        "same tape twice hashes equal"
    );
    let (b3, s3) = run_tape(2, 120);
    assert_ne!(
        s1.state_hash(&b1),
        s3.state_hash(&b3),
        "substep count recorded in state hash"
    );
    let (b4, s4) = run_tape(DEFAULT_SUBSTEPS, 121);
    assert_eq!(
        s1.state_hash(&b1),
        s4.state_hash(&b4),
        "settled tape holds its hash one tick on"
    );
}

#[test]
fn joint_insertion_order_does_not_move_hash() {
    fn with_order(first: Joint, second: Joint) -> (Vec<Body>, Schedule) {
        let mut schedule = Schedule::new();
        schedule.add_joint(first).expect("add first");
        schedule.add_joint(second).expect("add second");
        let mut bodies = vec![
            Body::new(0.0, 3.0, 0.0, 0.0, 0.25, 0.25),
            Body::new(1.0, 3.0, 0.0, 0.0, 0.25, 0.25),
            Body::new(2.0, 3.0, 0.0, 0.0, 0.25, 0.25),
        ];
        for _ in 0..60 {
            schedule.step(&mut bodies, DT);
        }
        (bodies, schedule)
    }
    let j02 = Joint::new(0, 2, 2.0).expect("valid joint");
    let j12 = Joint::new(1, 2, 1.0).expect("valid joint");
    let (forward_b, forward) = with_order(j02, j12);
    let (reverse_b, reverse) = with_order(j12, j02);
    assert_eq!(forward.joints(), reverse.joints());
    assert_eq!(
        [(0_usize, 2_usize), (1_usize, 2_usize)],
        [
            (forward.joints()[0].a, forward.joints()[0].b),
            (forward.joints()[1].a, forward.joints()[1].b),
        ],
        "joints stay in canonical (a, b) order"
    );
    assert_eq!(
        forward.state_hash(&forward_b),
        reverse.state_hash(&reverse_b),
        "insertion order never moves the hash"
    );
}

#[test]
fn config_edges_refused_or_clamped() {
    assert_eq!(
        ScheduleConfig::new(0, DEFAULT_ITERATIONS, DEFAULT_GRAVITY).unwrap_err(),
        ScheduleError::BadSubsteps(0)
    );
    assert_eq!(
        ScheduleConfig::new(MAX_SUBSTEPS + 1, DEFAULT_ITERATIONS, DEFAULT_GRAVITY).unwrap_err(),
        ScheduleError::BadSubsteps(MAX_SUBSTEPS + 1)
    );
    assert_eq!(
        ScheduleConfig::new(DEFAULT_SUBSTEPS, 0, DEFAULT_GRAVITY).unwrap_err(),
        ScheduleError::BadIterations(0)
    );
    assert_eq!(
        ScheduleConfig::new(DEFAULT_SUBSTEPS, MAX_ITERATIONS + 1, DEFAULT_GRAVITY).unwrap_err(),
        ScheduleError::BadIterations(MAX_ITERATIONS + 1)
    );
    assert!(matches!(
        ScheduleConfig::new(DEFAULT_SUBSTEPS, DEFAULT_ITERATIONS, f32::NAN).unwrap_err(),
        ScheduleError::BadGravity(_)
    ));
    assert!(matches!(
        ScheduleConfig::new(DEFAULT_SUBSTEPS, DEFAULT_ITERATIONS, f32::INFINITY).unwrap_err(),
        ScheduleError::BadGravity(_)
    ));
    assert!(matches!(
        ScheduleConfig::new(DEFAULT_SUBSTEPS, DEFAULT_ITERATIONS, MAX_GRAVITY + 1.0).unwrap_err(),
        ScheduleError::BadGravity(_)
    ));
    ScheduleConfig::new(MAX_SUBSTEPS, MAX_ITERATIONS, MAX_GRAVITY).expect("max edge valid");
    ScheduleConfig::new(1, 1, -MAX_GRAVITY).expect("min edge valid");
    assert_eq!(
        Joint::new(1, 1, 1.0).unwrap_err(),
        ScheduleError::BadJoint(1, 1)
    );
    assert_eq!(
        Joint::new(0, 1, 0.0).unwrap_err(),
        ScheduleError::BadRest(0.0)
    );
    assert!(matches!(
        Joint::new(0, 1, MAX_REST_M + 1.0).unwrap_err(),
        ScheduleError::BadRest(_)
    ));
    for bad in [f32::NAN, f32::INFINITY, -1.0] {
        let joint = Joint::with_compliance(0, 1, 1.0, bad).expect("sanitized joint");
        assert_eq!(joint.compliance, 0.0, "compliance {bad} becomes rigid");
    }
}

#[test]
fn dt_and_empty_edges_step_nothing() {
    let mut schedule = Schedule::new();
    schedule.step(&mut [], DT);
    assert_eq!(schedule.sleeping_count(), 0);
    let mut bodies = linked_pair();
    let before = bodies
        .iter()
        .map(|b| (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()))
        .collect::<Vec<_>>();
    let hash_before = schedule.state_hash(&bodies);
    for bad_dt in [f32::NAN, f32::INFINITY, 0.0, -DT] {
        schedule.step(&mut bodies, bad_dt);
    }
    let after = bodies
        .iter()
        .map(|b| (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()))
        .collect::<Vec<_>>();
    assert_eq!(before, after, "bad dt leaves bodies bit-identical");
    assert_eq!(hash_before, schedule.state_hash(&bodies));
    assert_eq!(schedule.sleeping_count(), 0);
    let mut sparse = Schedule::new();
    sparse
        .add_joint(Joint::new(0, 9, 1.0).expect("valid joint"))
        .expect("add joint");
    let mut two = linked_pair();
    sparse.step(&mut two, DT);
    assert!(two.iter().all(|b| b.x.is_finite() && b.y.is_finite()));
}
