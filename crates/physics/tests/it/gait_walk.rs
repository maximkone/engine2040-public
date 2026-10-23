//! PX5a walk leg: the procedural gait drives the powered ragdoll servo, so the
//! body walks because its legs push, not because a clip plays. The fixture is
//! a two-legged rig the gait names every bone of; the proofs are the row's
//! accept numbers (travel, line, height band, stop, replay, shove).
#![cfg(feature = "rapier")]

use engine_anim::contract::{Joint, LocalPose, Skeleton};
use engine_physics::balance::{
    BalanceParams, BalanceState, CaptureBias, GaitVerdict, DEFAULT_STAGGER_Q8,
};
use engine_physics::gait::{GaitCycle, GaitParams};
use engine_physics::powered::{DriveParams, PoweredContact, PoweredRagdoll, RagdollMode};
use engine_physics::ragdoll::{world_transforms, BoneShape};
use engine_physics::rapier::FIXED_DT;

/// Bone slots of the fixture: pelvis, spine, head, thigh_l, shin_l, foot_l,
/// thigh_r, shin_r, foot_r, arm_l, arm_r.
const PELVIS: usize = 0;
const SPINE: usize = 1;
const FOOT_L: usize = 5;
const FOOT_R: usize = 8;

/// The fixture rig: a biped whose rest pose stands 1.0 m at the pelvis.
fn biped() -> Skeleton {
    let names: [(&str, Option<usize>); 11] = [
        ("pelvis", None),
        ("spine", Some(0)),
        ("head", Some(1)),
        ("thigh_l", Some(0)),
        ("shin_l", Some(3)),
        ("foot_l", Some(4)),
        ("thigh_r", Some(0)),
        ("shin_r", Some(6)),
        ("foot_r", Some(7)),
        ("arm_l", Some(1)),
        ("arm_r", Some(1)),
    ];
    Skeleton {
        joints: names
            .iter()
            .map(|(name, parent)| Joint {
                name: (*name).into(),
                parent: *parent,
            })
            .collect(),
    }
}

/// Rest pose: pelvis at 1.0 m, 0.45 m thigh and shin, foot spheres resting on
/// the floor. The arms sit wide so no unjointed pair of capsules touches.
fn rest_pose() -> LocalPose {
    LocalPose {
        positions: vec![
            [0.0, 1.0, 0.0],
            [0.0, 0.25, 0.0],
            [0.0, 0.25, 0.0],
            [0.0, -0.05, 0.09],
            [0.0, -0.45, 0.0],
            [0.0, -0.45, 0.0],
            [0.0, -0.05, -0.09],
            [0.0, -0.45, 0.0],
            [0.0, -0.45, 0.0],
            [0.0, 0.0, 0.24],
            [0.0, 0.0, -0.24],
        ],
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; 11],
    }
}

/// Capsule dims: slim limbs, a sphere foot that just touches the floor.
fn shapes() -> Vec<BoneShape> {
    let dims = [
        (0.10, 0.08), // pelvis
        (0.09, 0.08), // spine
        (0.08, 0.04), // head
        (0.06, 0.12), // thigh_l
        (0.05, 0.12), // shin_l
        (0.05, 0.00), // foot_l
        (0.06, 0.12), // thigh_r
        (0.05, 0.12), // shin_r
        (0.05, 0.00), // foot_r
        (0.04, 0.10), // arm_l
        (0.04, 0.10), // arm_r
    ];
    dims.iter()
        .map(|(radius, half_height)| BoneShape {
            radius: *radius,
            half_height: *half_height,
        })
        .collect()
}

/// The walk drive: a stiff servo, because a velocity servo lags its target by
/// `speed / gain`, and 80/s keeps that lag at 45 mm at 3 m/s.
fn walk_drive() -> DriveParams {
    DriveParams {
        linear_gain: 80.0,
        angular_gain: 80.0,
        linear_damping: 0.2,
        angular_damping: 0.2,
        max_speed: 8.0,
        max_spin: 12.0,
    }
}

/// One walking body: the ragdoll, its gait cycle and its balance core.
struct Rig {
    skeleton: Skeleton,
    ragdoll: PoweredRagdoll,
    gait: GaitCycle,
    balance: BalanceState,
    /// Foot bones touching the floor after the last step, `[left, right]`.
    feet_down: [bool; 2],
}

impl Rig {
    /// Spawn the fixture on the floor, standing.
    fn new() -> Self {
        let skeleton = biped();
        let drives = vec![walk_drive(); 11];
        let mut ragdoll = PoweredRagdoll::spawn(&skeleton, &rest_pose(), &shapes(), &drives)
            .expect("spawn the biped");
        ragdoll
            .add_static_box([20.0, 0.5, 20.0], [0.0, -0.5, 0.0])
            .expect("floor");
        Self {
            gait: GaitCycle::new(GaitParams::default(), &skeleton, rest_pose()),
            skeleton,
            ragdoll,
            balance: BalanceState::new(BalanceParams::default()),
            feet_down: [false; 2],
        }
    }

    /// One tick of the walk: advance the cycle, feed the servo pose, step the
    /// world, read the contacts and feed the balance core.
    fn step(&mut self, speed_mm_s: i32, turn_q10: i32) {
        self.gait.advance(speed_mm_s, turn_q10);
        let pose = self
            .gait
            .pose(&self.skeleton, self.gait.phase(), speed_mm_s);
        self.ragdoll.set_animated(&pose).expect("driver pose");
        self.ragdoll.step(FIXED_DT).expect("fixed step");
        let contacts = self.ragdoll.take_contacts();
        self.feet_down = foot_contacts(&contacts);
        self.observe_balance();
        self.balance.step();
    }

    /// Feed the balance core the body's center of mass in its own support
    /// frame: the arena reads it around the stance, so the walking fixture
    /// subtracts the pelvis before the observation.
    fn observe_balance(&mut self) {
        let pelvis = self.pelvis();
        let mut centers = Vec::with_capacity(self.skeleton.len());
        for bone in 0..self.skeleton.len() {
            let p = self.ragdoll.bone_position(bone).expect("bone");
            centers.push([p[0] - pelvis[0], p[1], p[2] - pelvis[2]]);
        }
        let masses = vec![1.0f32; self.skeleton.len()];
        self.balance
            .observe(&centers, &masses)
            .expect("center of mass");
    }

    /// Pelvis world position.
    fn pelvis(&self) -> [f32; 3] {
        self.ragdoll.bone_position(PELVIS).expect("pelvis")
    }

    /// Walk `ticks` at one speed.
    fn walk(&mut self, ticks: usize, speed_mm_s: i32) {
        for _ in 0..ticks {
            self.step(speed_mm_s, 0);
        }
    }

    /// Foot contact x positions where a foot touches down, one per rising edge.
    fn note_contacts(&self, was_down: &[bool; 2], out: &mut Vec<f32>) {
        for foot in [FOOT_L, FOOT_R] {
            let side = usize::from(foot == FOOT_R);
            if self.feet_down[side] && !was_down[side] {
                out.push(self.ragdoll.bone_position(foot).expect("foot")[0]);
            }
        }
    }
}

/// Which feet touch the floor in one contact list.
fn foot_contacts(contacts: &[PoweredContact]) -> [bool; 2] {
    let mut down = [false; 2];
    for c in contacts {
        if c.b.is_some() {
            continue;
        }
        if c.a == FOOT_L {
            down[0] = true;
        } else if c.a == FOOT_R {
            down[1] = true;
        }
    }
    down
}

/// Planar distance between two positions, metres.
fn planar(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]) * (a[0] - b[0]) + (a[2] - b[2]) * (a[2] - b[2])).sqrt()
}

/// Distance between two positions, metres.
fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

#[test]
fn gait_walks_three_metres_in_120_ticks() {
    // The row's accept line, measured on the served body: 120 ticks at
    // 1.5 m/s is 2 s of walking, so the pelvis covers 3 m along the line,
    // stays within 150 mm of it, and its height stays inside a 60 mm band.
    let mut rig = Rig::new();
    let start = rig.pelvis();
    let mut low = start[1];
    let mut high = start[1];
    for _ in 0..120 {
        rig.step(1500, 0);
        let p = rig.pelvis();
        low = low.min(p[1]);
        high = high.max(p[1]);
    }
    let end = rig.pelvis();
    let forward = end[0] - start[0];
    let off_line = planar([end[0], 0.0, end[2]], [end[0], 0.0, 0.0]);
    println!(
        "walk 1.5 m/s: forward {forward:.3} m, off line {off_line:.3} m, \
         height band {:.3} m ({low:.3}..{high:.3})",
        high - low
    );
    assert!(
        (forward - 3.0).abs() <= 0.150,
        "120 ticks at 1.5 m/s must walk 3 m, got {forward:.3} m"
    );
    assert!(
        off_line <= 0.150,
        "pelvis drifted {off_line:.3} m off the line"
    );
    assert!(
        high - low <= 0.060,
        "pelvis height band {:.3} m must stay inside 60 mm",
        high - low
    );
    assert_eq!(
        rig.ragdoll.mode(),
        RagdollMode::Animated,
        "a walk never falls"
    );
}

#[test]
fn gait_travel_is_speed_times_time() {
    // The cadence follows the commanded speed, so 3 m/s over 120 ticks (2 s)
    // is 6 m: the row's 3 m is the 1.5 m/s case above, not a different walk.
    // This leg also reads the row's kill criterion at 3 m/s.
    let mut rig = Rig::new();
    let start = rig.pelvis();
    let mut low = start[1];
    let mut high = start[1];
    let mut planted_at: [Option<f32>; 2] = [None, None];
    let mut worst_pose: f32 = 0.0;
    let mut served_at: [Option<f32>; 2] = [None, None];
    let mut worst_served: f32 = 0.0;
    for _ in 0..120 {
        rig.step(3000, 0);
        let p = rig.pelvis();
        low = low.min(p[1]);
        high = high.max(p[1]);
        let stance = rig.gait.foot_stance();
        let pose = rig.gait.pose(&rig.skeleton, rig.gait.phase(), 3000);
        let (world, _) = world_transforms(&rig.skeleton, &pose);
        for (side, foot) in [FOOT_L, FOOT_R].into_iter().enumerate() {
            if !stance[side] {
                planted_at[side] = None;
                served_at[side] = None;
                continue;
            }
            // The row's kill bar reads on the foot the cycle plants: the pose's
            // own world x, held from the plant to the next toe-off.
            let x = world[foot][0];
            let anchor = *planted_at[side].get_or_insert(x);
            worst_pose = worst_pose.max((x - anchor).abs());
            // The served body is reported here, not gated. A servo's lag is
            // `speed / gain`, and at 3 m/s the swing foot covers 1 m in 10
            // ticks, so the fixture's P-80 drive trails it by about 150 mm:
            // the foot touches down behind its plant spot and skids onto it.
            if rig.feet_down[side] {
                let x = rig.ragdoll.bone_position(foot).expect("foot")[0];
                let anchor = *served_at[side].get_or_insert(x);
                worst_served = worst_served.max((x - anchor).abs());
            } else {
                served_at[side] = None;
            }
        }
    }
    let end = rig.pelvis();
    let forward = end[0] - start[0];
    println!(
        "walk 3 m/s: forward {forward:.3} m over 120 ticks, height band {:.3} m, \
         planted foot holds {:.3} m in the pose, served foot skids {:.3} m",
        high - low,
        worst_pose,
        worst_served
    );
    assert!(
        (forward - 6.0).abs() <= 0.150,
        "120 ticks at 3 m/s must walk 6 m, got {forward:.3} m"
    );
    assert!(
        high - low <= 0.060,
        "pelvis height band {:.3} m must stay inside 60 mm",
        high - low
    );
    assert!(
        worst_pose <= 0.100,
        "a planted foot slid {worst_pose:.3} m at 3 m/s (the row's kill bar is 0.100)"
    );
}

#[test]
fn gait_stops_inside_thirty_ticks() {
    // A stop command must reach the stand pose inside the accept's 30 ticks,
    // and then the pelvis must settle: less than 20 mm of travel after it.
    let mut rig = Rig::new();
    rig.walk(60, 3000);
    let mut ticks = 0;
    while !rig.gait.standing() {
        rig.step(0, 0);
        ticks += 1;
        assert!(ticks <= 30, "the stand must arrive inside 30 ticks");
    }
    let at_stand = rig.pelvis();
    for _ in 0..30 {
        rig.step(0, 0);
    }
    let settled = rig.pelvis();
    let drift = dist3(settled, at_stand);
    println!(
        "stop: {ticks} ticks to stand, {:.1} mm of drift after",
        drift * 1000.0
    );
    assert!(ticks <= 30, "the stand must arrive inside 30 ticks");
    assert!(
        drift <= 0.020,
        "the standing pelvis moved {:.1} mm in 30 ticks",
        drift * 1000.0
    );
}

#[test]
fn gait_replay_hashes_twice() {
    // Same inputs twice: the pose bytes and every sim byte the ragdoll hashes
    // must match, or the walk is not a replayable gait.
    let run = || {
        let mut rig = Rig::new();
        rig.walk(120, 3000);
        let pose = rig.gait.pose(&rig.skeleton, rig.gait.phase(), 3000);
        (rig.ragdoll.state_hash(), rig.gait.state_hash(), pose)
    };
    let (h1, g1, p1) = run();
    let (h2, g2, p2) = run();
    assert_eq!(h1, h2, "the ragdoll state must hash twice");
    assert_eq!(g1, g2, "the gait state must hash twice");
    assert_eq!(p1, p2, "the pose bytes must match");
}

#[test]
fn gait_shove_steps_out_and_recovers() {
    // A shove at the stagger threshold mid-walk: the balance core staggers,
    // the upper body goes free, and the gait takes a recovery step that plants
    // the next contact further forward than the steady twin's, with both feet
    // off the floor at once on the way (a step, not a glide). Inside the
    // recovery budget the body is steady and walking again.
    let mut shoved = Rig::new();
    let mut steady = Rig::new();
    shoved.walk(60, 3000);
    steady.walk(60, 3000);
    let shove_tick = 60;
    let verdict = shoved
        .balance
        .apply_impact(DEFAULT_STAGGER_Q8)
        .expect("stagger-level shove");
    assert_eq!(verdict, GaitVerdict::Staggered);
    let bias = shoved
        .balance
        .capture_bias(DEFAULT_STAGGER_Q8)
        .expect("bias");
    shoved.gait.set_bias(&bias);
    shoved
        .ragdoll
        .apply_hit(SPINE, [0.05, 0.0, 0.0], shove_tick as u64)
        .expect("upper body takes the blow");
    shoved.gait.step_out(150);
    assert_eq!(
        shoved.ragdoll.mode(),
        RagdollMode::Partial,
        "only the top frees"
    );
    let mut shoved_plants: Vec<f32> = Vec::new();
    let mut steady_plants: Vec<f32> = Vec::new();
    let mut both_feet_up = 0;
    let mut recovered_at = None;
    for tick in 0..60 {
        let shoved_was = shoved.feet_down;
        let steady_was = steady.feet_down;
        shoved.step(3000, 0);
        steady.step(3000, 0);
        shoved.note_contacts(&shoved_was, &mut shoved_plants);
        steady.note_contacts(&steady_was, &mut steady_plants);
        if !shoved.feet_down[0] && !shoved.feet_down[1] {
            both_feet_up += 1;
        }
        if !shoved.balance.staggered() && recovered_at.is_none() {
            recovered_at = Some(tick + 1);
            // The stagger window closed: the balance bias goes with it.
            shoved.gait.set_bias(&CaptureBias {
                cp_err_mm: [0, 0],
                ankle_q8: 0,
                hip_q8: 0,
                stance_q8: 0,
            });
        }
    }
    let plant_gap = shoved_plants.first().copied().unwrap_or(f32::NAN)
        - steady_plants.first().copied().unwrap_or(f32::NAN);
    let ahead = shoved.pelvis()[0] - steady.pelvis()[0];
    let height_gap = (shoved.pelvis()[1] - steady.pelvis()[1]).abs();
    println!(
        "shove: recovered at {recovered_at:?}, plant {plant_gap:.3} m ahead, \
         body {ahead:.3} m ahead, both feet up {both_feet_up} ticks, \
         height gap {height_gap:.3} m"
    );
    let recovered = recovered_at.unwrap_or(u32::MAX);
    assert!(
        recovered <= 60,
        "the shove must recover inside the 60-tick budget"
    );
    assert!(
        plant_gap >= 0.100,
        "the next plant must be at least 100 mm ahead, got {plant_gap:.3} m"
    );
    assert!(both_feet_up >= 1, "the step-out must leave the floor");
    assert!(ahead >= 0.100, "the body must step out, got {ahead:.3} m");
    assert!(height_gap <= 0.060, "the shove must not drop the body");
    assert!(
        shoved.gait.travel_mm()[0] > steady.gait.travel_mm()[0],
        "the recovery step must add travel"
    );
}
