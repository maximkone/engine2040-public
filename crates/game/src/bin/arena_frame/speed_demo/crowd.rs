//! crowd (HC6, docs/SPRINT-34-hero-city.md): people on the sidewalks who go full ragdoll.
//!
//! Walkers are cheap: each one walks a loop around its block with a procedural gait on a
//! 14-bone mannequin ([`BONES`]). A hit faster than [`WAKE_MPS`] hands that walker's exact
//! pose to the float physical layer (`engine-physfx`) as a ragdoll with the hit's velocity,
//! so the body that flies is the body that was walking. Knees and elbows are hinges, the rest
//! are balls with the rig's swing limits; buildings near the hit become static boxes so a
//! thrown body hits the wall instead of passing through it. The physical layer's tier caps
//! how many ragdolls are awake (Comfort 8); the oldest goes back to sleep for good when a new
//! one wakes. Drawing: every bone is a limb from its joint to its child joint, all walkers and
//! ragdolls of one shirt colour merged into one mesh per frame ([`Crowd::meshes`]).
//! Presentation only: nothing here reads into the sim. Units: meters, seconds, newton-seconds;
//! y up; rotations are unit quaternions `[x, y, z, w]`; a walker's local forward is +z.

use engine_anim::contract::{Joint, LocalPose, Skeleton};
use engine_core::quat_mul;
use engine_physfx::{BoneShape, Hinge, Key, PhysFx, Rig, Tier};

use super::num::{add, axis_quat, below, f32_of, len, rotate, scale, sub, u32_of, unit_f32};

/// A hit at least this fast wakes a walker as a ragdoll, m/s.
pub(super) const WAKE_MPS: f32 = 6.0;
/// Walkers farther than this from the camera are not drawn, meters.
pub(super) const DRAW_RANGE_M: f32 = 150.0;
/// Shirt colours: walkers and ragdolls merge into one mesh per colour.
pub(super) const SHIRTS: usize = 4;
/// One stride (two steps) in meters; the gait phase turns once per stride.
const STRIDE_M: f32 = 1.5;
/// Pelvis height of a standing walker, meters.
const PELVIS_M: f32 = 0.95;
/// Statics the crowd may add for buildings near hits (under every tier's static cap).
const WALL_BUDGET: usize = 48;
/// Fastest velocity handed to the physical layer, m/s (its own limit is 100).
const MAX_FLING_MPS: f32 = 60.0;

/// One mannequin bone: name, parent, offset from the parent joint in the parent's frame
/// (meters, standing), physics capsule (radius, half-height; meters), drawn limb end in the
/// bone's own frame (meters) and drawn half-width (meters).
type BoneRow = (
    &'static str,
    Option<usize>,
    [f32; 3],
    (f32, f32),
    [f32; 3],
    f32,
);

/// The mannequin, parents first. Standing pose: every local rotation is identity.
const BONES: [BoneRow; 14] = [
    (
        "pelvis",
        None,
        [0.0, 0.0, 0.0],
        (0.13, 0.05),
        [0.0, 0.12, 0.0],
        0.15,
    ),
    (
        "spine",
        Some(0),
        [0.0, 0.12, 0.0],
        (0.12, 0.05),
        [0.0, 0.22, 0.0],
        0.14,
    ),
    (
        "chest",
        Some(1),
        [0.0, 0.22, 0.0],
        (0.15, 0.08),
        [0.0, 0.25, 0.0],
        0.17,
    ),
    (
        "head",
        Some(2),
        [0.0, 0.27, 0.0],
        (0.11, 0.04),
        [0.0, 0.24, 0.0],
        0.1,
    ),
    (
        "upper_arm_l",
        Some(2),
        [-0.2, 0.2, 0.0],
        (0.05, 0.1),
        [0.0, -0.28, 0.0],
        0.05,
    ),
    (
        "lower_arm_l",
        Some(4),
        [0.0, -0.28, 0.0],
        (0.04, 0.1),
        [0.0, -0.26, 0.0],
        0.04,
    ),
    (
        "upper_arm_r",
        Some(2),
        [0.2, 0.2, 0.0],
        (0.05, 0.1),
        [0.0, -0.28, 0.0],
        0.05,
    ),
    (
        "lower_arm_r",
        Some(6),
        [0.0, -0.28, 0.0],
        (0.04, 0.1),
        [0.0, -0.26, 0.0],
        0.04,
    ),
    (
        "thigh_l",
        Some(0),
        [-0.1, -0.03, 0.0],
        (0.08, 0.15),
        [0.0, -0.44, 0.0],
        0.075,
    ),
    (
        "shin_l",
        Some(8),
        [0.0, -0.44, 0.0],
        (0.06, 0.15),
        [0.0, -0.44, 0.0],
        0.06,
    ),
    (
        "foot_l",
        Some(9),
        [0.0, -0.44, 0.0],
        (0.06, 0.03),
        [0.0, -0.05, 0.2],
        0.05,
    ),
    (
        "thigh_r",
        Some(0),
        [0.1, -0.03, 0.0],
        (0.08, 0.15),
        [0.0, -0.44, 0.0],
        0.075,
    ),
    (
        "shin_r",
        Some(11),
        [0.0, -0.44, 0.0],
        (0.06, 0.15),
        [0.0, -0.44, 0.0],
        0.06,
    ),
    (
        "foot_r",
        Some(12),
        [0.0, -0.44, 0.0],
        (0.06, 0.03),
        [0.0, -0.05, 0.2],
        0.05,
    ),
];
/// Bone indices the gait drives.
const THIGH_L: usize = 8;
const SHIN_L: usize = 9;
const THIGH_R: usize = 11;
const SHIN_R: usize = 12;
const UPPER_ARM_L: usize = 4;
const LOWER_ARM_L: usize = 5;
const UPPER_ARM_R: usize = 6;
const LOWER_ARM_R: usize = 7;
/// Side axis: rotations about +x swing a hanging limb backward (toward -z).
const SIDE: [f32; 3] = [1.0, 0.0, 0.0];
const UP: [f32; 3] = [0.0, 1.0, 0.0];
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// One sidewalk loop: the rectangle `min..max` (x, z) a walker circles, meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Loop {
    pub min: [f32; 2],
    pub max: [f32; 2],
}

impl Loop {
    fn perimeter(&self) -> f32 {
        2.0 * ((self.max[0] - self.min[0]) + (self.max[1] - self.min[1]))
    }

    /// The point `s` meters along the loop (counter-clockwise from `min`) and the unit
    /// direction of travel there.
    fn at(&self, s: f32) -> ([f32; 2], [f32; 2]) {
        let (w, d) = (self.max[0] - self.min[0], self.max[1] - self.min[1]);
        let s = s.rem_euclid(self.perimeter().max(1e-3));
        if s < w {
            ([self.min[0] + s, self.min[1]], [1.0, 0.0])
        } else if s < w + d {
            ([self.max[0], self.min[1] + s - w], [0.0, 1.0])
        } else if s < 2.0 * w + d {
            ([self.max[0] - (s - w - d), self.max[1]], [-1.0, 0.0])
        } else {
            ([self.min[0], self.max[1] - (s - 2.0 * w - d)], [0.0, -1.0])
        }
    }
}

/// A walker on its loop.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Walker {
    path: Loop,
    s_m: f32,
    speed: f32,
    phase: f32,
    shirt: usize,
    /// Ground height under the loop, meters.
    ground_y: f32,
}

/// A walker who was hit: its ragdoll slot and shirt.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fallen {
    slot: u8,
    shirt: usize,
}

/// One mesh of the crowd for a frame: plain vectors, world space.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct CrowdMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// Everyone on the sidewalks.
pub(super) struct Crowd {
    walkers: Vec<Walker>,
    fallen: Vec<Fallen>,
    fx: PhysFx,
    rig: Rig,
    next_slot: u8,
    slots: u8,
    walls: usize,
    /// Walkers woken as ragdolls so far (the exit line counts them).
    pub woken: u32,
}

impl Crowd {
    /// `count` walkers spread over `loops` (seeded), the ground at `ground_y` under each loop.
    pub(super) fn new(
        loops: &[(Loop, f32)],
        count: usize,
        seed: u64,
        tier: Tier,
    ) -> Result<Self, String> {
        let rig = mannequin_rig()?;
        let mut fx = PhysFx::new(tier);
        fx.add_static_box([1000.0, 0.5, 1000.0], [0.0, -0.5, 0.0])
            .map_err(|e| format!("crowd ground: {e}"))?;
        let mut rng = seed;
        let mut next = || {
            rng = rng.wrapping_add(engine_core::hash::GOLDEN_GAMMA);
            engine_core::splitmix64(rng)
        };
        let mut walkers = Vec::with_capacity(count);
        for _ in 0..count.min(loops.len().saturating_mul(16)) {
            let (path, ground_y) = loops[below(next(), loops.len())];
            walkers.push(Walker {
                path,
                s_m: unit_f32(next()) * path.perimeter(),
                speed: 1.1 + 0.6 * unit_f32(next()),
                phase: unit_f32(next()) * std::f32::consts::TAU,
                shirt: below(next(), SHIRTS),
                ground_y,
            });
        }
        let slots = u8::try_from(tier.caps().ragdolls.max(1)).unwrap_or(u8::MAX);
        Ok(Self {
            walkers,
            fallen: Vec::new(),
            fx,
            rig,
            next_slot: 0,
            slots,
            walls: 1,
            woken: 0,
        })
    }

    /// One 60 Hz tick: walkers walk, ragdolls fall.
    pub(super) fn step(&mut self, dt: f32) {
        for w in &mut self.walkers {
            w.s_m += w.speed * dt;
            w.phase = (w.phase + w.speed * dt / STRIDE_M * std::f32::consts::TAU)
                .rem_euclid(std::f32::consts::TAU);
        }
        if !self.fallen.is_empty() {
            self.fx.step();
        }
    }

    /// Something moving at `vel` (m/s) passes `at` with reach `radius_m`: every walker inside
    /// wakes as a ragdoll flung along it when it is fast enough. `walls` are the building boxes
    /// near the hit (min, max); they become statics so bodies hit them. Returns how many woke.
    pub(super) fn hit(
        &mut self,
        at: [f32; 3],
        radius_m: f32,
        vel: [f32; 3],
        walls: &[([f32; 3], [f32; 3])],
    ) -> u32 {
        if len(vel) < WAKE_MPS {
            return 0;
        }
        let mut woke = 0;
        let mut i = 0;
        while i < self.walkers.len() {
            let w = self.walkers[i];
            let pelvis = walker_pelvis(&w);
            if len(sub(pelvis, at)) <= radius_m && self.wake(&w, vel) {
                self.walkers.swap_remove(i);
                woke += 1;
            } else {
                i += 1;
            }
        }
        if woke > 0 {
            self.add_walls(walls);
        }
        woke
    }

    /// Hand one walker's pose to the physical layer. False when the layer refuses it.
    fn wake(&mut self, w: &Walker, vel: [f32; 3]) -> bool {
        let pose = walker_pose(w);
        let speed = len(vel).min(MAX_FLING_MPS);
        let dir = scale(vel, 1.0 / len(vel).max(1e-3));
        // The body takes most of the hitter's speed, lifted a little so it leaves the ground.
        let fling = add(scale(dir, speed * 0.7), [0.0, speed * 0.15, 0.0]);
        let slot = self.next_slot;
        if self
            .fx
            .spawn_ragdoll(slot, &self.rig, &pose, fling, [0.0; 3])
            .is_err()
        {
            return false;
        }
        self.fallen.retain(|f| f.slot != slot);
        self.fallen.push(Fallen {
            slot,
            shirt: w.shirt,
        });
        self.next_slot = (slot + 1) % self.slots;
        self.woken += 1;
        true
    }

    fn add_walls(&mut self, walls: &[([f32; 3], [f32; 3])]) {
        for (min, max) in walls {
            if self.walls >= WALL_BUDGET {
                return;
            }
            let half = scale(sub(*max, *min), 0.5);
            let centre = add(*min, half);
            if self.fx.add_static_box(half, centre).is_ok() {
                self.walls += 1;
            }
        }
    }

    /// Walkers still walking.
    pub(super) fn walking(&self) -> usize {
        self.walkers.len()
    }

    /// Ragdolls the physical layer holds now.
    pub(super) fn awake(&self) -> usize {
        self.fallen.len()
    }

    /// The pelvis of every ragdoll now, meters (tests and the camera read it).
    pub(super) fn ragdoll_roots(&self) -> Vec<[f32; 3]> {
        let Ok(placed) = self.fx.transforms(1.0) else {
            return Vec::new();
        };
        placed
            .iter()
            .filter(|p| matches!(p.key, Key::Bone { bone: 0, .. }))
            .map(|p| p.at.pos)
            .collect()
    }

    /// This frame's crowd as one mesh per shirt colour, walkers within [`DRAW_RANGE_M`] of
    /// `eye` plus every ragdoll; `alpha` blends the ragdolls' last two ticks.
    pub(super) fn meshes(&self, eye: [f32; 3], alpha: f32) -> Vec<CrowdMesh> {
        let mut out = vec![CrowdMesh::default(); SHIRTS];
        for w in &self.walkers {
            if len(sub(walker_pelvis(w), eye)) > DRAW_RANGE_M {
                continue;
            }
            let pose = walker_pose(w);
            let (pos, rot) = world_bones(&pose);
            push_body(&mut out[w.shirt], &pos, &rot);
        }
        if let Ok(placed) = self.fx.transforms(alpha.clamp(0.0, 1.0)) {
            for f in &self.fallen {
                let mut pos = [[0.0; 3]; BONES.len()];
                let mut rot = [IDENTITY; BONES.len()];
                let mut seen = 0;
                for p in &placed {
                    if let Key::Bone { slot, bone } = p.key {
                        let b = usize::from(bone);
                        if slot == f.slot && b < BONES.len() {
                            pos[b] = p.at.pos;
                            rot[b] = p.at.rot;
                            seen += 1;
                        }
                    }
                }
                if seen == BONES.len() {
                    push_body(&mut out[f.shirt], &pos, &rot);
                }
            }
        }
        out
    }
}

/// The mannequin's physical rig: the skeleton, one capsule per bone, knees and elbows hinged.
fn mannequin_rig() -> Result<Rig, String> {
    let skeleton = Skeleton {
        joints: BONES
            .iter()
            .map(|b| Joint {
                name: b.0.to_string(),
                parent: b.1,
            })
            .collect(),
    };
    let shapes = BONES
        .iter()
        .map(|b| BoneShape {
            radius: b.3 .0,
            half_height: b.3 .1,
        })
        .collect();
    let knee = |bone| Hinge {
        bone,
        min: 0.0,
        max: 2.4,
    };
    let elbow = |bone| Hinge {
        bone,
        min: -2.5,
        max: 0.0,
    };
    Rig::new(skeleton, shapes)
        .and_then(|rig| {
            rig.with_hinges(&[
                knee(SHIN_L),
                knee(SHIN_R),
                elbow(LOWER_ARM_L),
                elbow(LOWER_ARM_R),
            ])
        })
        .map_err(|e| format!("crowd rig: {e}"))
}

/// Where a walker's pelvis is now, meters.
fn walker_pelvis(w: &Walker) -> [f32; 3] {
    let (p, _) = w.path.at(w.s_m);
    let bob = 0.03 * w.phase.cos().abs();
    [p[0], w.ground_y + PELVIS_M + bob, p[1]]
}

/// A walker's pose: the root holds the world placement, the limbs swing with the gait phase.
fn walker_pose(w: &Walker) -> LocalPose {
    let (_, dir) = w.path.at(w.s_m);
    let heading = axis_quat(UP, dir[0].atan2(dir[1]));
    let mut positions: Vec<[f32; 3]> = BONES.iter().map(|b| b.2).collect();
    let mut rotations = vec![IDENTITY; BONES.len()];
    positions[0] = walker_pelvis(w);
    rotations[0] = heading;
    let (s, c) = w.phase.sin_cos();
    // Legs swing opposite each other; a leg folds its knee as it swings through.
    rotations[THIGH_L] = axis_quat(SIDE, -0.45 * s);
    rotations[THIGH_R] = axis_quat(SIDE, 0.45 * s);
    rotations[SHIN_L] = axis_quat(SIDE, 0.1 + 0.6 * (-c).max(0.0));
    rotations[SHIN_R] = axis_quat(SIDE, 0.1 + 0.6 * c.max(0.0));
    // Arms swing against their legs, elbows a little bent.
    rotations[UPPER_ARM_L] = axis_quat(SIDE, 0.35 * s);
    rotations[UPPER_ARM_R] = axis_quat(SIDE, -0.35 * s);
    rotations[LOWER_ARM_L] = axis_quat(SIDE, -0.3);
    rotations[LOWER_ARM_R] = axis_quat(SIDE, -0.3);
    LocalPose {
        positions,
        rotations,
    }
}

/// World joint positions and rotations of a pose whose root holds the world placement.
fn world_bones(pose: &LocalPose) -> ([[f32; 3]; 14], [[f32; 4]; 14]) {
    let mut pos = [[0.0; 3]; BONES.len()];
    let mut rot = [IDENTITY; BONES.len()];
    for (i, bone) in BONES.iter().enumerate() {
        match bone.1 {
            None => {
                pos[i] = pose.positions[i];
                rot[i] = pose.rotations[i];
            }
            Some(p) => {
                pos[i] = add(pos[p], rotate(rot[p], pose.positions[i]));
                rot[i] = quat_mul(rot[p], pose.rotations[i]);
            }
        }
    }
    (pos, rot)
}

/// Append one body: every bone a four-sided limb from its joint to its drawn end.
fn push_body(mesh: &mut CrowdMesh, pos: &[[f32; 3]; 14], rot: &[[f32; 4]; 14]) {
    for (i, bone) in BONES.iter().enumerate() {
        let from = pos[i];
        let to = add(from, rotate(rot[i], bone.4));
        let side = rotate(rot[i], [bone.5, 0.0, 0.0]);
        let front = rotate(rot[i], [0.0, 0.0, bone.5]);
        push_limb(mesh, from, to, side, front);
    }
}

/// A box limb from `a` to `b` with half-widths `side` and `front` (world vectors).
fn push_limb(mesh: &mut CrowdMesh, a: [f32; 3], b: [f32; 3], side: [f32; 3], front: [f32; 3]) {
    let corners = |c: [f32; 3]| {
        [
            add(c, add(side, front)),
            add(c, sub(side, front)),
            sub(c, add(side, front)),
            add(sub(c, side), front),
        ]
    };
    let (lo, hi) = (corners(a), corners(b));
    let faces = [
        [lo[0], lo[1], hi[1], hi[0]],
        [lo[1], lo[2], hi[2], hi[1]],
        [lo[2], lo[3], hi[3], hi[2]],
        [lo[3], lo[0], hi[0], hi[3]],
        [hi[0], hi[1], hi[2], hi[3]],
        [lo[3], lo[2], lo[1], lo[0]],
    ];
    for quad in faces {
        let n = super::num::unit(super::num::cross(
            sub(quad[1], quad[0]),
            sub(quad[3], quad[0]),
        ));
        let base = u32_of(mesh.positions.len());
        for (k, p) in quad.iter().enumerate() {
            mesh.positions.push(*p);
            mesh.normals.push(n);
            mesh.uvs.push([f32_of(k % 2), f32_of(k / 2)]);
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn street() -> Vec<(Loop, f32)> {
        vec![(
            Loop {
                min: [-20.0, -10.0],
                max: [20.0, 10.0],
            },
            0.0,
        )]
    }

    /// The walk keeps every walker on its loop with its feet near the ground, and the gait
    /// swings the legs.
    #[test]
    fn walkers_walk_their_loop_on_their_feet() {
        let mut crowd = Crowd::new(&street(), 10, 7, Tier::Comfort).expect("crowd");
        for _ in 0..600 {
            crowd.step(DT);
        }
        for w in &crowd.walkers {
            let (pos, rot) = world_bones(&walker_pose(w));
            let toe_l = add(pos[10], rotate(rot[10], BONES[10].4));
            let low = pos
                .iter()
                .chain([&toe_l])
                .map(|p| p[1])
                .fold(f32::MAX, f32::min);
            assert!((-0.1..0.2).contains(&low), "lowest point {low}");
            let p = walker_pelvis(w);
            let on_x = (p[0] + 20.0).abs() < 0.01 || (p[0] - 20.0).abs() < 0.01;
            let on_z = (p[2] + 10.0).abs() < 0.01 || (p[2] - 10.0).abs() < 0.01;
            assert!(on_x || on_z, "off the loop at {p:?}");
        }
    }

    /// A fast pass wakes the walkers near it; a slow one wakes nobody.
    #[test]
    fn a_fast_hit_wakes_ragdolls_and_a_slow_one_does_not() {
        let mut crowd = Crowd::new(&street(), 12, 3, Tier::Comfort).expect("crowd");
        let p = walker_pelvis(&crowd.walkers[0]);
        assert_eq!(crowd.hit(p, 3.0, [3.0, 0.0, 0.0], &[]), 0, "too slow");
        let woke = crowd.hit(p, 3.0, [30.0, 0.0, 0.0], &[]);
        assert!(woke >= 1, "the walker at the hit wakes");
        assert_eq!(crowd.awake(), usize::try_from(woke).unwrap_or(0));
        assert_eq!(crowd.walking(), 12 - usize::try_from(woke).unwrap_or(0));
    }

    /// A flung body flies along the hit, comes down, and never sinks into the ground.
    #[test]
    fn a_flung_body_flies_along_the_hit_and_lands_on_the_ground() {
        let mut crowd = Crowd::new(&street(), 1, 5, Tier::Comfort).expect("crowd");
        let start = walker_pelvis(&crowd.walkers[0]);
        assert_eq!(crowd.hit(start, 2.0, [25.0, 0.0, 0.0], &[]), 1);
        let mut lowest = f32::MAX;
        for _ in 0..300 {
            crowd.step(DT);
            for root in crowd.ragdoll_roots() {
                lowest = lowest.min(root[1]);
            }
        }
        let end = crowd.ragdoll_roots()[0];
        assert!(
            end[0] - start[0] > 5.0,
            "flung along +x: {start:?} -> {end:?}"
        );
        assert!(end[1] < 0.6, "it came down: {end:?}");
        assert!(lowest > -0.05, "never through the ground: {lowest}");
    }

    /// A wall in the way stops the body: it ends on the near side.
    #[test]
    fn a_wall_stops_a_flung_body() {
        let mut crowd = Crowd::new(&street(), 1, 5, Tier::Comfort).expect("crowd");
        let start = walker_pelvis(&crowd.walkers[0]);
        let wall = (
            [start[0] + 4.0, 0.0, start[2] - 10.0],
            [start[0] + 6.0, 30.0, start[2] + 10.0],
        );
        assert_eq!(crowd.hit(start, 2.0, [30.0, 0.0, 0.0], &[wall]), 1);
        for _ in 0..240 {
            crowd.step(DT);
        }
        let end = crowd.ragdoll_roots()[0];
        assert!(end[0] < start[0] + 4.0, "stopped by the wall: {end:?}");
    }

    /// Past the tier's cap the oldest ragdoll makes room; the count never passes the cap.
    #[test]
    fn the_tier_caps_the_awake_ragdolls() {
        let mut crowd = Crowd::new(&street(), 40, 9, Tier::Potato).expect("crowd");
        for _ in 0..40 {
            let Some(w) = crowd.walkers.first().copied() else {
                break;
            };
            crowd.hit(walker_pelvis(&w), 0.5, [20.0, 0.0, 0.0], &[]);
            crowd.step(DT);
        }
        assert!(crowd.woken >= 10, "many woke: {}", crowd.woken);
        assert!(crowd.awake() <= Tier::Potato.caps().ragdolls);
        assert_eq!(crowd.ragdoll_roots().len(), crowd.awake());
    }

    /// Drawing: one mesh per shirt, every body six quads per bone, far walkers culled.
    #[test]
    fn the_crowd_draws_one_mesh_per_shirt_and_culls_far_walkers() {
        let crowd = Crowd::new(&street(), 10, 7, Tier::Comfort).expect("crowd");
        let near = crowd.meshes([0.0, 2.0, 0.0], 1.0);
        assert_eq!(near.len(), SHIRTS);
        let quads: usize = near.iter().map(|m| m.indices.len() / 6).sum();
        assert_eq!(quads, 10 * BONES.len() * 6);
        let far = crowd.meshes([5000.0, 2.0, 0.0], 1.0);
        assert!(far.iter().all(|m| m.positions.is_empty()));
    }
}
