//! clinch: ragdolls that share a place part without sinking into each other (NS3).
//!
//! Consumer: the clinch leg of [`ragdoll_tape`](super::ragdoll_tape), which parts its two
//! fighters after every step; any code that keeps several ragdolls in separate worlds can call
//! the same function. Each fighter has a world of its own (Rapier never sees another fighter's
//! bones), so this pass is the only contact between them. Units: meters, kilograms, seconds.
//! Determinism: one `sqrt` per bone pair, `+ - * /` otherwise; fighters are visited in id order
//! and bones in skeleton order, so the order the caller passes them in never reaches the
//! result. Errors: none; a bone with a non-finite pose is skipped. Budget:
//! `PASSES * fighters^2 * bones^2 / 2` capsule tests, no allocation.
#![forbid(unsafe_code)]

use super::ragdoll::{BoneCapsule, Ragdoll};

/// Sweeps per call. Parting one pair can push a bone into a third, so one sweep is not enough.
pub const PASSES: usize = 4;
/// Overlap left on purpose, meters, so a settled pair reads as touching and not as a jitter
/// between pushed-out and pushed-in.
pub const SLOP_M: f32 = 0.002;
/// Axes closer than this, meters, have no usable direction.
const MIN_DIST_M: f32 = 1.0e-6;

/// One fighter in a clinch: a stable id and its ragdoll.
pub struct Member<'a> {
    /// Orders the fighters; two fighters in one clinch must not share an id.
    pub id: u8,
    /// The fighter's bones.
    pub ragdoll: &'a mut Ragdoll,
}

/// What one [`part_members`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClinchStats {
    /// Bone pairs pushed apart, summed over the sweeps.
    pub parted: u32,
    /// Deepest overlap the first sweep met, meters; `0.0` when nothing touched.
    pub deepest_m: f32,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn along(p: [f32; 3], d: [f32; 3], t: f32) -> [f32; 3] {
    [p[0] + d[0] * t, p[1] + d[1] * t, p[2] + d[2] * t]
}

/// The closest points of the segments `p1..q1` and `p2..q2` (Ericson, Real-Time Collision
/// Detection 5.1.9). A zero-length segment is a point.
fn closest_points(
    (p1, q1): ([f32; 3], [f32; 3]),
    (p2, q2): ([f32; 3], [f32; 3]),
) -> ([f32; 3], [f32; 3]) {
    let (d1, d2, r) = (sub(q1, p1), sub(q2, p2), sub(p1, p2));
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (s, t) = if a <= f32::EPSILON && e <= f32::EPSILON {
        (0.0, 0.0)
    } else if a <= f32::EPSILON {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = dot(d1, r);
        if e <= f32::EPSILON {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = dot(d1, d2);
            let denom = a * e - b * b;
            let mut s = if denom > f32::EPSILON * a * e {
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
            (s, t)
        }
    };
    (along(p1, d1, s), along(p2, d2, t))
}

/// How far the capsules sink into each other, meters, with the unit direction from `b` to `a`.
/// Negative when they are apart. Axes that cross have no direction of their own: the centers
/// give it, and a coincident pair parts along +X, so the answer is always defined.
fn overlap(a: &BoneCapsule, b: &BoneCapsule) -> (f32, [f32; 3]) {
    let (pa, pb) = closest_points((a.a, a.b), (b.a, b.b));
    let gap = sub(pa, pb);
    let dist = dot(gap, gap).sqrt();
    let depth = a.radius + b.radius - dist;
    if dist > MIN_DIST_M {
        return (depth, [gap[0] / dist, gap[1] / dist, gap[2] / dist]);
    }
    let centers = sub(
        along(a.a, sub(a.b, a.a), 0.5),
        along(b.a, sub(b.b, b.a), 0.5),
    );
    let reach = dot(centers, centers).sqrt();
    if reach > MIN_DIST_M {
        (
            depth,
            [centers[0] / reach, centers[1] / reach, centers[2] / reach],
        )
    } else {
        (depth, [1.0, 0.0, 0.0])
    }
}

/// The deepest overlap between a bone of `a` and a bone of `b`, meters; `0.0` when none touch.
pub fn deepest_overlap(a: &Ragdoll, b: &Ragdoll) -> f32 {
    let mut deepest = 0.0f32;
    for i in 0..a.bone_count() {
        let Some(ca) = a.bone_capsule(i) else {
            continue;
        };
        for j in 0..b.bone_count() {
            if let Some(cb) = b.bone_capsule(j) {
                deepest = deepest.max(overlap(&ca, &cb).0);
            }
        }
    }
    deepest
}

/// Parts the bone pair `(i of a, j of b)` when it overlaps beyond [`SLOP_M`]: the lighter bone
/// moves further, and an approach along the contact normal is cancelled so the next step does
/// not push the pair back together. Returns the overlap met, `None` when the pair was fine.
fn part_pair(a: &mut Ragdoll, i: usize, b: &mut Ragdoll, j: usize) -> Option<f32> {
    let (ca, cb) = (a.bone_capsule(i)?, b.bone_capsule(j)?);
    let (depth, n) = overlap(&ca, &cb);
    if depth <= SLOP_M {
        return None;
    }
    let give = depth - SLOP_M;
    let inv_sum = ca.inv_mass + cb.inv_mass;
    let (wa, wb) = (ca.inv_mass / inv_sum, cb.inv_mass / inv_sum);
    let closing = dot(sub(a.bone_linvel(i)?, b.bone_linvel(j)?), n);
    let stop = if closing < 0.0 { -closing } else { 0.0 };
    let by = |w: f32, sign: f32| {
        let k = sign * w;
        (
            [n[0] * give * k, n[1] * give * k, n[2] * give * k],
            [n[0] * stop * k, n[1] * stop * k, n[2] * stop * k],
        )
    };
    let (shift_a, dv_a) = by(wa, 1.0);
    let (shift_b, dv_b) = by(wb, -1.0);
    // Both bones were read just above, so neither call can refuse.
    a.part_bone(i, shift_a, dv_a).ok()?;
    b.part_bone(j, shift_b, dv_b).ok()?;
    Some(depth)
}

/// Parts every bone of every fighter from the bones of the others. Members are put in id order
/// first, then each pair `(low id, high id)` is swept bone by bone in skeleton order, so the
/// slice order the caller passes never changes the result.
pub fn part_members(members: &mut [Member<'_>]) -> ClinchStats {
    members.sort_unstable_by_key(|m| m.id);
    let mut stats = ClinchStats::default();
    for pass in 0..PASSES {
        for low in 0..members.len() {
            let (head, rest) = members.split_at_mut(low + 1);
            let a = &mut *head[low].ragdoll;
            for other in rest {
                let b = &mut *other.ragdoll;
                for i in 0..a.bone_count() {
                    for j in 0..b.bone_count() {
                        if let Some(depth) = part_pair(a, i, b, j) {
                            stats.parted += 1;
                            if pass == 0 {
                                stats.deepest_m = stats.deepest_m.max(depth);
                            }
                        }
                    }
                }
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capsule(a: [f32; 3], b: [f32; 3], radius: f32) -> BoneCapsule {
        BoneCapsule {
            a,
            b,
            radius,
            inv_mass: 1.0,
        }
    }

    #[test]
    fn closest_points_cover_crossing_parallel_and_point_segments() {
        // Crossing axes touch at their middles.
        let (p, q) = closest_points(
            ([-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            ([0.0, 0.5, -1.0], [0.0, 0.5, 1.0]),
        );
        assert_eq!((p, q), ([0.0, 0.0, 0.0], [0.0, 0.5, 0.0]));
        // Parallel axes pick any pair at the same distance.
        let (p, q) = closest_points(
            ([0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.3, 0.0, 0.0], [0.3, 1.0, 0.0]),
        );
        assert!((sub(p, q)[0] + 0.3).abs() < 1e-6 && sub(p, q)[1].abs() < 1e-6);
        // A zero-length segment is a point on the other segment's clamp.
        let (p, q) = closest_points(([0.0; 3], [0.0; 3]), ([2.0, 1.0, 0.0], [4.0, 1.0, 0.0]));
        assert_eq!((p, q), ([0.0; 3], [2.0, 1.0, 0.0]));
    }

    #[test]
    fn overlap_is_signed_and_always_has_a_direction() {
        let a = capsule([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.15);
        let near = capsule([0.2, 0.0, 0.0], [0.2, 1.0, 0.0], 0.15);
        let (depth, n) = overlap(&a, &near);
        assert!(
            (depth - 0.1).abs() < 1e-6 && n == [-1.0, 0.0, 0.0],
            "{depth} {n:?}"
        );
        let far = capsule([1.0, 0.0, 0.0], [1.0, 1.0, 0.0], 0.15);
        assert!(overlap(&a, &far).0 < 0.0);
        // Crossing axes: the centers give the direction.
        let cross = capsule([-0.5, 0.5, 0.0], [0.5, 0.5, 0.0], 0.15);
        let (depth, n) = overlap(&a, &cross);
        assert!(depth > 0.29 && n.iter().all(|c| c.is_finite()));
        // The same capsule twice parts along +X.
        assert_eq!(overlap(&a, &a).1, [1.0, 0.0, 0.0]);
    }
}
