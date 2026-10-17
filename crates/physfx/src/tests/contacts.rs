//! contacts: bones that are not joined push each other apart, inside one ragdoll and between two
//! (PX9). Rapier gives every collider the same group, and `insert_joints` turns contacts off only
//! between a bone and its parent, so these tests pin that rule from the outside.

use super::*;

/// Deepest overlap a settled or falling pair may show, meters.
const SLOP: f32 = 0.01;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `v` turned by the unit quaternion `q`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let c = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let t = [2.0 * c[0], 2.0 * c[1], 2.0 * c[2]];
    let tc = [
        u[1] * t[2] - u[2] * t[1],
        u[2] * t[0] - u[0] * t[2],
        u[0] * t[1] - u[1] * t[0],
    ];
    [
        v[0] + q[3] * t[0] + tc[0],
        v[1] + q[3] * t[1] + tc[1],
        v[2] + q[3] * t[2] + tc[2],
    ]
}

/// The capsule axis of a placed bone: its two end points.
fn axis(placed: &Placed, half_height: f32) -> ([f32; 3], [f32; 3]) {
    let up = rotate(placed.at.rot, [0.0, half_height, 0.0]);
    let p = placed.at.pos;
    (sub(p, up), [p[0] + up[0], p[1] + up[1], p[2] + up[2]])
}

/// Distance between the segments `p1..q1` and `p2..q2` (Ericson 5.1.9).
fn segment_distance(p1: [f32; 3], q1: [f32; 3], p2: [f32; 3], q2: [f32; 3]) -> f32 {
    let (d1, d2, r) = (sub(q1, p1), sub(q2, p2), sub(p1, p2));
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let c = dot(d1, r);
    let b = dot(d1, d2);
    let denom = a * e - b * b;
    let mut s = if denom > 1.0e-9 {
        ((b * f - c * e) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    let on1 = [p1[0] + d1[0] * s, p1[1] + d1[1] * s, p1[2] + d1[2] * s];
    let on2 = [p2[0] + d2[0] * t, p2[1] + d2[1] * t, p2[2] + d2[2] * t];
    dot(sub(on1, on2), sub(on1, on2)).sqrt()
}

/// Deepest overlap among bone pairs `(i of a, j of b)` that the rig does not join, meters.
fn deepest(a: &[Placed], b: &[Placed], same_ragdoll: bool) -> (f32, usize, usize) {
    let mut worst = (f32::MIN, 0, 0);
    for (i, pa) in a.iter().enumerate() {
        for (j, pb) in b.iter().enumerate() {
            let joined = same_ragdoll && (i == j || BONES[i].1 == Some(j) || BONES[j].1 == Some(i));
            if joined || (same_ragdoll && j < i) {
                continue;
            }
            let (ra, rb) = (BONES[i].3, BONES[j].3);
            let (p1, q1) = axis(pa, ra.1);
            let (p2, q2) = axis(pb, rb.1);
            let depth = ra.0 + rb.0 - segment_distance(p1, q1, p2, q2);
            if depth > worst.0 {
                worst = (depth, i, j);
            }
        }
    }
    worst
}

/// How deep unjoined bones sank into each other in one fall: the worst moment (depth, bone
/// a, bone b, tick) and the settled end.
struct Fall {
    peak: (f32, usize, usize, u32),
    rest: f32,
}

/// One seeded fall: height, launch speed and hit all come from `seed`; 300 ticks.
fn seeded_fall(seed: u64) -> Fall {
    // xorshift64*: a seeded stream in -1..=1, no dependency beyond the crate under test.
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut unit = move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / 8_388_608.0 - 1.0
    };
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    let start = moved(&pose, [0.0, 0.3 + unit().abs() * 1.2, 0.0]);
    let speed = [unit() * 3.0, 0.0, unit() * 3.0];
    let hit = [unit() * 120.0, unit().abs() * 60.0, unit() * 120.0];
    fx.spawn_ragdoll(0, &rig, &start, speed, hit)
        .expect("ragdoll");
    let mut fall = Fall {
        peak: (f32::MIN, 0, 0, 0),
        rest: 0.0,
    };
    for tick in 0..300 {
        fx.step();
        let bones = bones_of(&fx, 0);
        let (depth, i, j) = deepest(&bones, &bones, true);
        if depth > fall.peak.0 {
            fall.peak = (depth, i, j, tick);
        }
        fall.rest = depth;
    }
    fall
}

#[test]
fn fifty_seeded_falls_leave_no_bone_pair_overlapping_over_a_centimeter() {
    let falls: Vec<Fall> = (0..50).map(seeded_fall).collect();
    let name = |i: usize| BONES[i].0;
    let (seed, worst) = falls
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.peak.0.total_cmp(&b.1.peak.0))
        .expect("falls");
    let (depth, i, j, tick) = worst.peak;
    let rest = falls.iter().map(|f| f.rest).fold(f32::MIN, f32::max);
    assert!(
        depth <= SLOP && rest <= SLOP,
        "seed {seed}: {} and {} overlap by {depth} m at tick {tick}; worst settled overlap {rest} m",
        name(i),
        name(j)
    );
}

/// A fighter on the floor and a second one dropped on top of it, 400 ticks.
fn pile() -> PhysFx {
    let (rig, pose) = humanoid();
    let mut fx = arena(Tier::Comfort);
    let lying = moved(&pose, [0.0, 0.2, 0.0]);
    fx.spawn_ragdoll(0, &rig, &lying, [0.0; 3], [0.0, 0.0, 40.0])
        .expect("first");
    step_n(&mut fx, 120);
    fx.spawn_ragdoll(1, &rig, &moved(&pose, [0.05, 1.8, 0.0]), [0.0; 3], [0.0; 3])
        .expect("second");
    step_n(&mut fx, 400);
    fx
}

#[test]
fn two_ragdolls_landing_on_each_other_separate() {
    let fx = pile();
    let (below, above) = (bones_of(&fx, 0), bones_of(&fx, 1));
    let (depth, ..) = deepest(&below, &above, false);
    assert!(depth <= SLOP, "the two fighters overlap by {depth} m");
    let apart = sub(below[0].at.pos, above[0].at.pos);
    assert!(
        dot(apart, apart).sqrt() > 0.2,
        "the second fighter must rest on the first, not inside it: {apart:?}"
    );
}

#[test]
fn the_pile_replays_exactly() {
    assert_eq!(
        pile().transforms(1.0).expect("transforms"),
        pile().transforms(1.0).expect("transforms")
    );
}
