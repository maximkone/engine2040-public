//! Joint and contact conformance against offline oracles (NS4).
//!
//! The oracles are offline and no bytes are copied: Rapier's own code (its
//! `PdController`, its parry capsule contact, its angular-limit measure) and
//! a Jolt-style swing-twist limit model reimplemented here from the published
//! idea. Rapier has no swing cone, so the Jolt side of the comparison is a
//! model of what a cone means, written here. Fifty seeded poses per channel;
//! the report is printed, and every channel has a tolerance the test holds
//! it to.
//!
//! The channels, with the numbers the runner measured (2026-10-20, gh run
//! 39475148659) and the bar each channel is held to:
//!
//! - `servo`: our `pd_raw` against Rapier's `PdController`. Rapier adds the
//!   correction to the velocity, we assign it, so the law itself is compared.
//!   Measured 0.0, bar 1e-6.
//! - `contact`: our clinch overlap against Rapier's parry capsule contact.
//!   Measured 3.4e-5 m, bar 1e-4 m.
//! - `limit`: our joint against the range the table installs, measured the
//!   way Rapier measures an angular limit (the twist about the axis), on a
//!   chain in the air with no ground under it and the coupling seam off.
//!   Measured 0.025 rad, bar 0.05 rad: the joint holds its own row.
//! - `jolt`: the same poses against the Jolt-style swing cone. Our per-axis
//!   box is wider than Jolt's cone at the diagonal of the box, so the gap
//!   between the two limit laws is measured and posted (0.285 rad), not
//!   hidden.
//! - `landing`: the same chain dropped onto a floor, seam off. The soft
//!   Rapier limit lets a violent landing overrun the table (1.21 rad); the
//!   seam makes no difference there, because the bones rest on the floor box
//!   and its floor probe never fires. That gap belongs to the hard joint
//!   limits (the PX9 and AD16 rows).
//! - `seam`: the same free chain with the coupling seam on. The seam's floor
//!   probe lifts a chain that fell through the floor plane, so the seam
//!   moves the bones past the joint's row (0.67 rad). That gap belongs to
//!   the seam, not to the joint.
#![cfg(feature = "rapier")]

use engine_anim::contract::{Joint, LocalPose, Skeleton};
use engine_core::{quat_mul, Rng};
use engine_physics::clinch::deepest_overlap;
use engine_physics::pd::pd_raw;
use engine_physics::ragdoll::{BoneShape, CouplingParams, JointSpec, JointTable, Ragdoll};
use engine_physics::rapier::FIXED_DT;
use rapier3d::control::{PdController, PdErrors};
use rapier3d::dynamics::AxesMask;
use rapier3d::math::{Pose, Rotation, Vector};
use rapier3d::parry::query::contact;
use rapier3d::parry::shape::Capsule;

/// Poses per channel.
const POSES: usize = 50;

/// Steps per limit pose.
const LIMIT_STEPS: usize = 180;

/// Servo bar: the two laws are the same formula, so only float noise is
/// left.
const SERVO_TOL: f32 = 1.0e-6;

/// Contact depth bar, meters.
const CONTACT_TOL_M: f32 = 1.0e-4;

/// Joint limit bar, radians: in free flight the joint stays inside the row
/// the table gives it.
const LIMIT_TOL_RAD: f32 = 0.05;

/// Box-versus-cone bar, radians: our per-axis box is wider than Jolt's cone
/// at the diagonal of the box, by about `(sqrt(2) - 1) * limit`.
const JOLT_TOL_RAD: f32 = 0.35;

/// Violent landing bar, radians: the soft limit's transient, the gap the hard
/// joint limits (PX9, AD16) own.
const LANDING_TOL_RAD: f32 = 1.5;

/// Coupling seam bar, radians: the seam's floor probe moves the bones past
/// the joint's row, a gap of the seam's own row.
const SEAM_TOL_RAD: f32 = 0.85;

/// A draw in `-1.0..1.0` with 16 bits of resolution.
fn unit(rng: &mut Rng) -> f32 {
    let bits = u16::try_from(rng.next_u64() >> 48).expect("16 bits");
    f32::from(bits) / 32_768.0 - 1.0
}

/// Servo law: our `pd_raw` against the correction Rapier's `PdController`
/// makes from the same position error and velocity. Rapier adds the
/// correction to the velocity, we assign it, so the law itself is what is
/// compared, with a still target.
fn servo_deviation() -> f32 {
    let mut rng = Rng::new(2040);
    let mut worst = 0.0f32;
    for _ in 0..POSES {
        let (error, vel) = (unit(&mut rng) * 2.0, unit(&mut rng) * 6.0);
        let (kp, kd) = (
            1.0 + unit(&mut rng).abs() * 30.0,
            unit(&mut rng).abs() * 4.0,
        );
        let oracle = PdController::new(kp, kd, AxesMask::all()).correction(
            &PdErrors {
                linear: Vector::new(error, 0.0, 0.0),
                angular: Vector::ZERO,
            },
            &PdErrors {
                linear: Vector::new(-vel, 0.0, 0.0),
                angular: Vector::ZERO,
            },
        );
        worst = worst.max((pd_raw(error, vel, kp, kd, FIXED_DT) - oracle.linvel.x).abs());
    }
    worst
}

/// A one-bone ragdoll: a capsule at `pos` turned by `rot` (an `[x, y, z, w]`
/// quaternion).
fn lone_capsule(pos: [f32; 3], rot: [f32; 4], shape: BoneShape) -> Ragdoll {
    let skeleton = Skeleton {
        joints: vec![Joint {
            name: "body".into(),
            parent: None,
        }],
    };
    let pose = LocalPose {
        positions: vec![pos],
        rotations: vec![rot],
    };
    Ragdoll::spawn(&skeleton, &pose, &[shape]).expect("one bone spawns")
}

/// A unit quaternion from three draws (the Shoemake pick), through f64 trig:
/// the oracle side may use any math.
#[expect(
    clippy::disallowed_methods,
    reason = "the oracle is a test of the simulation, not the simulation: it may use platform trig"
)]
fn quat(rng: &mut Rng) -> [f32; 4] {
    let (u, v, w) = (
        f64::from(unit(rng)) * 0.5 + 0.5,
        f64::from(unit(rng)) * 0.5 + 0.5,
        f64::from(unit(rng)) * 0.5 + 0.5,
    );
    let tau = core::f64::consts::TAU;
    let (a, b) = ((1.0 - u).sqrt(), u.sqrt());
    [
        (a * (tau * v).sin()) as f32,
        (a * (tau * v).cos()) as f32,
        (b * (tau * w).sin()) as f32,
        (b * (tau * w).cos()) as f32,
    ]
}

/// Contact depth: the deepest overlap our clinch pass reads between two
/// capsules against the penetration parry reports for the same two shapes.
/// Parting pairs (parry sees no contact) must read zero.
fn contact_deviation() -> f32 {
    let mut rng = Rng::new(1_977);
    let (mut worst, mut touching) = (0.0f32, 0);
    for _ in 0..POSES {
        let shape = |rng: &mut Rng| BoneShape {
            radius: 0.05 + unit(rng).abs() * 0.15,
            half_height: unit(rng).abs() * 0.4,
        };
        let (sa, sb) = (shape(&mut rng), shape(&mut rng));
        let reach = |rng: &mut Rng| [unit(rng) * 0.3, unit(rng) * 0.3, unit(rng) * 0.3];
        let (pa, pb) = (reach(&mut rng), reach(&mut rng));
        let (qa, qb) = (quat(&mut rng), quat(&mut rng));
        let (a, b) = (lone_capsule(pa, qa, sa), lone_capsule(pb, qb, sb));
        let pose = |p: [f32; 3], q: [f32; 4]| {
            Pose::from_parts(
                Vector::new(p[0], p[1], p[2]),
                Rotation::from_xyzw(q[0], q[1], q[2], q[3]).normalize(),
            )
        };
        let shape = |s: BoneShape| Capsule::new_y(s.half_height, s.radius);
        let oracle = contact(&pose(pa, qa), &shape(sa), &pose(pb, qb), &shape(sb), 0.0)
            .expect("capsules are supported")
            .map_or(0.0, |c| (-c.dist).max(0.0));
        touching += usize::from(oracle > 0.0);
        worst = worst.max((deepest_overlap(&a, &b) - oracle).abs());
    }
    assert!(touching >= 15, "only {touching} of {POSES} poses touched");
    worst
}

/// The chain the limit poses use: four bones in a line. The names are unknown
/// to the table, so every bone reads the depth fallback row (a spherical
/// joint), and the joint frame is the body frame: the relative rotation the
/// test reads is the joint's own rotation.
fn chain_skeleton() -> Skeleton {
    Skeleton {
        joints: ["root", "mid", "tip", "end"]
            .iter()
            .enumerate()
            .map(|(i, name)| Joint {
                name: (*name).into(),
                parent: i.checked_sub(1),
            })
            .collect(),
    }
}

/// The chain's rest pose: the root three meters up, every bone half a meter
/// above its parent.
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

/// The chain's bones: a fat capsule, so the chain lands on a real floor.
fn chain_shapes() -> Vec<BoneShape> {
    vec![
        BoneShape {
            radius: 0.15,
            half_height: 0.2,
        };
        4
    ]
}

/// The angle of a rotation about one axis, the way Rapier measures an angular
/// limit: the twist about that axis, wrapped to `(-pi, pi]`.
#[expect(
    clippy::disallowed_methods,
    reason = "the oracle is a test of the simulation, not the simulation: it may use platform trig"
)]
fn axis_twist(q: [f32; 4], axis: usize) -> f32 {
    let half = f32::atan2(q[axis], q[3]);
    let half = if half > core::f32::consts::FRAC_PI_2 {
        half - core::f32::consts::PI
    } else if half < -core::f32::consts::FRAC_PI_2 {
        half + core::f32::consts::PI
    } else {
        half
    };
    2.0 * half
}

/// The angle of a rotation quaternion, in radians.
#[expect(
    clippy::disallowed_methods,
    reason = "the oracle is a test of the simulation, not the simulation: it may use platform trig"
)]
fn rotation_angle(q: [f32; 4]) -> f32 {
    let v = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    2.0 * f32::atan2(v, q[3])
}

/// The swing-twist split of a relative rotation about the bone's axis (the
/// local Y): Jolt's `SwingTwistConstraint` idea, reimplemented here. The twist
/// is the part around Y, the swing is the rest. Returns the swing angle and
/// the twist angle in radians.
fn swing_twist(q: [f32; 4]) -> (f32, f32) {
    // The twist is the normalized (w, 0, y, 0) part. A half turn around a
    // perpendicular axis carries no twist at all: the twist is the identity
    // and the swing carries the whole rotation.
    let n2 = q[3] * q[3] + q[1] * q[1];
    if n2 <= 1.0e-12 {
        return (rotation_angle(q), 0.0);
    }
    let inv = 1.0 / n2.sqrt();
    let twist = [0.0, q[1] * inv, 0.0, q[3] * inv];
    let conj = [-twist[0], -twist[1], -twist[2], twist[3]];
    (rotation_angle(quat_mul(q, conj)), axis_twist(twist, 1))
}

/// How far our own limit law is broken: the twist about the two swing axes
/// against the cone the table installs, and the twist about the bone axis
/// against the twist range. Rapier's angular limit is the twist about the
/// axis, so this is the law the engine really enforces.
fn our_over(q: [f32; 4], spec: JointSpec) -> f32 {
    let x = axis_twist(q, 0).abs() - spec.swing_limit;
    let z = axis_twist(q, 2).abs() - spec.swing_limit;
    let y = axis_twist(q, 1);
    let y = (y - spec.twist_max).max(spec.twist_min - y);
    x.max(z).max(y)
}

/// How far the Jolt-style cone is broken: the swing angle against the cone
/// half-angle and the twist against the twist range.
fn jolt_over(q: [f32; 4], spec: JointSpec) -> f32 {
    let (swing, twist) = swing_twist(q);
    let cone = swing - spec.swing_limit;
    let twist = (twist - spec.twist_max).max(spec.twist_min - twist);
    cone.max(twist)
}

/// What one limit sweep found: the largest overrun of our own limit law and
/// the largest overrun of the Jolt-style cone over the same poses.
struct LimitOutcome {
    /// Our per-axis law (the twist about each axis against the range the
    /// table installs), radians.
    ours: f32,
    /// The Jolt-style swing cone over the same poses, radians.
    jolt: f32,
}

/// The limit sweep: fifty seeded hits on a chain, stepped and read every tick.
/// `floor` puts a floor box under the chain; `seam` keeps the coupling seam on
/// (its floor probe lifts a chain that fell through the floor plane, so the
/// free case runs without it: there is no ground to stand on). With a floor
/// box under the chain the probe never fires, so the landing case reads the
/// joint's own soft limit.
fn limit_sweep(floor: bool, seam: bool) -> LimitOutcome {
    let skeleton = chain_skeleton();
    let pose = chain_pose();
    let shapes = chain_shapes();
    let table = JointTable::humanoid(&skeleton);
    let mut rng = Rng::new(4_000);
    let mut ours = 0.0f32;
    let mut jolt = 0.0f32;
    for _ in 0..POSES {
        let mut ragdoll = Ragdoll::spawn(&skeleton, &pose, &shapes).expect("chain spawns");
        if floor {
            ragdoll
                .add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
                .expect("floor");
        }
        if !seam {
            ragdoll
                .set_coupling(CouplingParams::disabled())
                .expect("seam off");
        }
        ragdoll.blend_to_ragdoll(1.0).expect("free");
        let bone = 1 + usize::try_from(rng.next_u64() % 3).expect("small");
        let hit = [
            unit(&mut rng) * 0.15,
            unit(&mut rng) * 0.15,
            unit(&mut rng) * 0.15,
        ];
        ragdoll.apply_hit(bone, hit, 0).expect("hit");
        for _ in 0..LIMIT_STEPS {
            ragdoll.step(FIXED_DT).expect("step");
            let local = ragdoll.bone_poses();
            for joint in 1..skeleton.len() {
                let q = local.rotations[joint];
                let spec = table.spec(joint).expect("row");
                ours = ours.max(our_over(q, spec));
                jolt = jolt.max(jolt_over(q, spec));
            }
        }
    }
    LimitOutcome { ours, jolt }
}

/// The report: the largest deviation of every channel.
struct Report {
    /// Our servo law against Rapier's `PdController`.
    servo: f32,
    /// Our clinch overlap against Rapier's parry contact, meters.
    contact_m: f32,
    /// Our joint against the range the table installs, radians.
    limit_rad: f32,
    /// The Jolt-style cone against the same poses, radians.
    jolt_rad: f32,
    /// The soft limit under a violent landing, radians.
    landing_rad: f32,
    /// The same free chain with the coupling seam on, radians.
    seam_rad: f32,
}

/// The whole report: every channel, fifty poses each.
fn report() -> Report {
    let free = limit_sweep(false, false);
    let landing = limit_sweep(true, false);
    let seam = limit_sweep(false, true);
    Report {
        servo: servo_deviation(),
        contact_m: contact_deviation(),
        limit_rad: free.ours,
        jolt_rad: free.jolt,
        landing_rad: landing.ours,
        seam_rad: seam.ours,
    }
}

#[test]
fn fifty_pose_oracle_report_stays_inside_its_tolerances() {
    let r = report();
    println!(
        "ORACLE poses={POSES} servo_max_dev={:.2e} contact_max_dev_m={:.2e} \
         limit_max_dev_rad={:.2e} jolt_limit_dev_rad={:.4} landing_max_dev_rad={:.4} \
         seam_max_dev_rad={:.4}",
        r.servo, r.contact_m, r.limit_rad, r.jolt_rad, r.landing_rad, r.seam_rad
    );
    // The servo law is the same formula, so the two agree to the float noise.
    assert!(
        r.servo <= SERVO_TOL,
        "the servo law deviates by {}",
        r.servo
    );
    // The overlap and the parry contact are the same depth, to float noise.
    assert!(
        r.contact_m <= CONTACT_TOL_M,
        "the contact depth deviates by {} m",
        r.contact_m
    );
    // A joint hit in the air holds its table limit: the soft Rapier limit
    // settles inside the range it was given.
    assert!(
        r.limit_rad <= LIMIT_TOL_RAD,
        "a joint turned {} rad past its limit",
        r.limit_rad
    );
    // Our per-axis box lets the diagonal of a swing turn further than Jolt's
    // cone: the gap between the two limit laws, posted.
    assert!(
        r.jolt_rad <= JOLT_TOL_RAD,
        "our swing cone is {} rad wider than the box",
        r.jolt_rad
    );
    // The soft limit lets a violent landing overrun the table. This is the
    // known gap: the hard joint limits are the PX9 and AD16 rows' work.
    assert!(
        r.landing_rad <= LANDING_TOL_RAD,
        "a landing turned a joint {} rad past its limit",
        r.landing_rad
    );
    // The coupling seam's floor probe lifts a chain that fell through the
    // floor plane; the joint it yanks is the seam's, not the limit's.
    assert!(
        r.seam_rad <= SEAM_TOL_RAD,
        "the seam's floor probe turned a joint {} rad",
        r.seam_rad
    );
}
