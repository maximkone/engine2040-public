//! ragdoll_contacts: post-solver capsule contacts for a limp ragdoll.
//!
//! Consumer: [`crate::ragdoll::Ragdoll::step`], after Rapier's pipeline and the
//! floor/spacing seam. A joint can drive two non-adjacent bones through each
//! other inside one tick; this module projects the surfaces apart, stops the
//! approaching normal velocity, and drains the slow crawl a long chain can
//! hold. Parent/child pairs stay skipped: their contacts are off by design and
//! their rest pose nests them.
//!
//! Determinism: index loops, `+ - * /`, `sqrt` and comparisons only; no
//! allocation and no hash-map order. Positions and velocities are written in
//! skeleton order, so replay and mirror runs stay deterministic.

use crate::ragdoll::{normalize_quat, rotate_vec, CouplingParams, CouplingStats};
use engine_anim::contract::Skeleton;
use rapier3d::dynamics::{RigidBodyHandle, RigidBodySet};
use rapier3d::math::Vector;

/// Slop the capsule contact projection may leave, meters. Below the 1 cm
/// PX9 bar with room for float drift.
const CAPSULE_CONTACT_SLOP_M: f32 = 0.008;
/// Gauss-Seidel sweeps of the capsule contact projection per tick. A pair
/// moved by one sweep can overlap another, so a few sweeps settle the chain.
const CAPSULE_CONTACT_SWEEPS: usize = 8;
/// A long chain can hold a summed speed above this while every bone is under
/// [`REST_DAMPING_MAX_BONE_MPS`]: a slow joint/contact crawl, not a fall. The
/// rest assist is for long chains only; the four-bone clinch and mirror
/// fixtures keep their shipped dynamics so their controls still bite.
const REST_DAMPING_MIN_BONES: usize = 8;
/// Summed speed the rest assist may leave, meters per second.
const REST_DAMPING_MIN_TOTAL_MPS: f32 = 0.8;
/// Below this per-bone speed a crawl is rest, not a hit.
const REST_DAMPING_MAX_BONE_MPS: f32 = 0.5;
/// Velocity kept per rest-damped step. Strong enough to beat the joint drive
/// that would otherwise hold a slow crawl.
const REST_DAMPING_KEEP: f32 = 0.85;

/// The two ends of a capsule's axis: `center -+ R * (0, half_height, 0)`.
pub(crate) fn capsule_axis(
    center: [f32; 3],
    rot: [f32; 4],
    half_height: f32,
) -> ([f32; 3], [f32; 3]) {
    let up = rotate_vec(&[0.0, half_height, 0.0], &normalize_quat(&rot));
    (
        [center[0] - up[0], center[1] - up[1], center[2] - up[2]],
        [center[0] + up[0], center[1] + up[1], center[2] + up[2]],
    )
}

/// Closest points of two segments `p1..q1` and `p2..q2` and their distance
/// (Ericson, Real-Time Collision Detection 5.1.9). One `sqrt`; no allocation.
pub(crate) fn closest_segments(
    p1: [f32; 3],
    q1: [f32; 3],
    p2: [f32; 3],
    q2: [f32; 3],
) -> ([f32; 3], [f32; 3], f32) {
    let d1 = [q1[0] - p1[0], q1[1] - p1[1], q1[2] - p1[2]];
    let d2 = [q2[0] - p2[0], q2[1] - p2[1], q2[2] - p2[2]];
    let r = [p1[0] - p2[0], p1[1] - p2[1], p1[2] - p2[2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let c = dot(d1, r);
    let b = dot(d1, d2);
    let denom = a * e - b * b;
    let mut s = if denom > 1e-9 {
        ((b * f - c * e) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = if e > 1e-9 { (b * s + f) / e } else { 0.0 };
    if t < 0.0 {
        t = 0.0;
        s = if a > 1e-9 {
            (-c / a).clamp(0.0, 1.0)
        } else {
            0.0
        };
    } else if t > 1.0 {
        t = 1.0;
        s = if a > 1e-9 {
            ((b - c) / a).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    let on1 = [p1[0] + d1[0] * s, p1[1] + d1[1] * s, p1[2] + d1[2] * s];
    let on2 = [p2[0] + d2[0] * t, p2[1] + d2[1] * t, p2[2] + d2[2] * t];
    let gap = [on1[0] - on2[0], on1[1] - on2[1], on1[2] - on2[2]];
    (on1, on2, dot(gap, gap).sqrt())
}

/// Surface overlap and separation axis of one bone pair: positive overlap is
/// the depth the two capsule surfaces share, the axis points from bone `i` to
/// bone `j`. `None` for bad handles or non-finite transforms; a zero center
/// delta falls back to +X so the axis is always defined and deterministic.
fn capsule_pair_overlap(
    bodies: &RigidBodySet,
    order: &[RigidBodyHandle],
    radii: &[f32],
    half_heights: &[f32],
    i: usize,
    j: usize,
) -> Option<(f32, [f32; 3])> {
    let bi = bodies.get(*order.get(i)?)?;
    let bj = bodies.get(*order.get(j)?)?;
    let ri = *radii.get(i)?;
    let rj = *radii.get(j)?;
    let hi = *half_heights.get(i)?;
    let hj = *half_heights.get(j)?;
    let pi = [bi.translation().x, bi.translation().y, bi.translation().z];
    let pj = [bj.translation().x, bj.translation().y, bj.translation().z];
    let qi = [
        bi.rotation().x,
        bi.rotation().y,
        bi.rotation().z,
        bi.rotation().w,
    ];
    let qj = [
        bj.rotation().x,
        bj.rotation().y,
        bj.rotation().z,
        bj.rotation().w,
    ];
    if pi.iter().chain(pj.iter()).any(|c| !c.is_finite()) {
        return None;
    }
    let (a1, a2) = capsule_axis(pi, qi, hi);
    let (b1, b2) = capsule_axis(pj, qj, hj);
    let (on_i, on_j, dist) = closest_segments(a1, a2, b1, b2);
    if !dist.is_finite() {
        return None;
    }
    // Unit axis from bone i to bone j. A zero segment distance falls back to
    // the center delta, then +X, so the direction is always defined.
    let axis = if dist > 1e-6 {
        [
            (on_j[0] - on_i[0]) / dist,
            (on_j[1] - on_i[1]) / dist,
            (on_j[2] - on_i[2]) / dist,
        ]
    } else {
        let d = [pj[0] - pi[0], pj[1] - pi[1], pj[2] - pi[2]];
        let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if d2 > 1e-12 {
            let dl = d2.sqrt();
            [d[0] / dl, d[1] / dl, d[2] / dl]
        } else {
            [1.0, 0.0, 0.0]
        }
    };
    Some((ri + rj - dist, axis))
}

/// Capsule surface separation after the solver: a bone pair that still
/// overlaps by more than [`CAPSULE_CONTACT_SLOP_M`] parts along its closest
/// surface axis, split between the two bodies and capped by the coupling
/// per-step budget. Parent/child pairs are skipped. Index loops only; no
/// allocation.
pub(crate) fn resolve_capsule_contacts(
    bodies: &mut RigidBodySet,
    order: &[RigidBodyHandle],
    radii: &[f32],
    half_heights: &[f32],
    skeleton: &Skeleton,
    params: &CouplingParams,
    stats: &mut CouplingStats,
) {
    if !params.enabled {
        return;
    }
    let n = order
        .len()
        .min(radii.len())
        .min(half_heights.len())
        .min(skeleton.len());
    for _ in 0..CAPSULE_CONTACT_SWEEPS {
        let mut corrected = false;
        for i in 0..n {
            for j in (i + 1)..n {
                if skeleton.joints[i].parent == Some(j) || skeleton.joints[j].parent == Some(i) {
                    continue;
                }
                let Some((overlap, axis)) =
                    capsule_pair_overlap(bodies, order, radii, half_heights, i, j)
                else {
                    continue;
                };
                let need = overlap - CAPSULE_CONTACT_SLOP_M;
                if need <= 0.0 {
                    continue;
                }
                let each = need * 0.5;
                let half = each.min(params.max_step_m);
                if half < each {
                    stats.truncations = stats.truncations.wrapping_add(1);
                    stats.penetration_events = stats.penetration_events.wrapping_add(1);
                }
                // No wake: bones never sleep, and `false` keeps the pass out of
                // Rapier's user-change lists (see the floor pass in `rapier`).
                if let Some(a) = bodies.get_mut(order[i]) {
                    let t = a.translation();
                    a.set_translation(
                        Vector::new(
                            t.x - axis[0] * half,
                            t.y - axis[1] * half,
                            t.z - axis[2] * half,
                        ),
                        false,
                    );
                }
                if let Some(b) = bodies.get_mut(order[j]) {
                    let t = b.translation();
                    b.set_translation(
                        Vector::new(
                            t.x + axis[0] * half,
                            t.y + axis[1] * half,
                            t.z + axis[2] * half,
                        ),
                        false,
                    );
                }
                if half > stats.max_correction_m {
                    stats.max_correction_m = half;
                }
                corrected = true;
            }
        }
        if !corrected {
            break;
        }
    }
    project_capsule_velocities(bodies, order, radii, half_heights, skeleton);
}

/// Remove the approaching normal velocity of a touching, non-joined capsule
/// pair. The position pass alone leaves the joint free to drive the pair back
/// together next tick; zeroing the normal approach makes the contact hold
/// without touching the tangential motion. Inverse mass splits the impulse,
/// so the heavy bone moves less. Index loops only; no allocation.
fn project_capsule_velocities(
    bodies: &mut RigidBodySet,
    order: &[RigidBodyHandle],
    radii: &[f32],
    half_heights: &[f32],
    skeleton: &Skeleton,
) {
    let n = order
        .len()
        .min(radii.len())
        .min(half_heights.len())
        .min(skeleton.len());
    for i in 0..n {
        for j in (i + 1)..n {
            if skeleton.joints[i].parent == Some(j) || skeleton.joints[j].parent == Some(i) {
                continue;
            }
            let Some((overlap, axis)) =
                capsule_pair_overlap(bodies, order, radii, half_heights, i, j)
            else {
                continue;
            };
            if overlap <= 0.0 {
                continue;
            }
            let (rel, inv_i, inv_j) = {
                let (Some(bi), Some(bj)) = (bodies.get(order[i]), bodies.get(order[j])) else {
                    continue;
                };
                let vi = bi.linvel();
                let vj = bj.linvel();
                let rel =
                    (vj.x - vi.x) * axis[0] + (vj.y - vi.y) * axis[1] + (vj.z - vi.z) * axis[2];
                let mi = bi.mass();
                let mj = bj.mass();
                let (inv_i, inv_j) = if mi > 0.0 && mj > 0.0 {
                    (1.0 / mi, 1.0 / mj)
                } else {
                    (1.0, 1.0)
                };
                (rel, inv_i, inv_j)
            };
            if rel >= 0.0 {
                continue;
            }
            let k = -rel / (inv_i + inv_j);
            if let Some(a) = bodies.get_mut(order[i]) {
                let v = a.linvel();
                a.set_linvel(
                    Vector::new(
                        v.x - axis[0] * k * inv_i,
                        v.y - axis[1] * k * inv_i,
                        v.z - axis[2] * k * inv_i,
                    ),
                    false,
                );
            }
            if let Some(b) = bodies.get_mut(order[j]) {
                let v = b.linvel();
                b.set_linvel(
                    Vector::new(
                        v.x + axis[0] * k * inv_j,
                        v.y + axis[1] * k * inv_j,
                        v.z + axis[2] * k * inv_j,
                    ),
                    false,
                );
            }
        }
    }
}

/// A long chain that is slow per bone can still hold a summed speed above the
/// rest bound: many small joint/contact corrections keep it crawling. Drain a
/// little linear and angular velocity per step while every bone is under
/// [`REST_DAMPING_MAX_BONE_MPS`] and the sum is above
/// [`REST_DAMPING_MIN_TOTAL_MPS`]. Runs only for chains longer than
/// [`REST_DAMPING_MIN_BONES`], so the four-bone clinch and mirror fixtures keep
/// their shipped dynamics. Index loops only; no allocation.
pub(crate) fn damp_resting_crawl(bodies: &mut RigidBodySet, order: &[RigidBodyHandle]) {
    if order.len() <= REST_DAMPING_MIN_BONES {
        return;
    }
    let mut total = 0.0f32;
    let mut max = 0.0f32;
    for handle in order {
        let Some(body) = bodies.get(*handle) else {
            continue;
        };
        let v = body.linvel();
        let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
        total += speed;
        max = max.max(speed);
    }
    if total <= REST_DAMPING_MIN_TOTAL_MPS || max >= REST_DAMPING_MAX_BONE_MPS {
        return;
    }
    for handle in order {
        let Some(body) = bodies.get_mut(*handle) else {
            continue;
        };
        let v = body.linvel();
        body.set_linvel(
            Vector::new(
                v.x * REST_DAMPING_KEEP,
                v.y * REST_DAMPING_KEEP,
                v.z * REST_DAMPING_KEEP,
            ),
            false,
        );
        let w = body.angvel();
        body.set_angvel(
            Vector::new(
                w.x * REST_DAMPING_KEEP,
                w.y * REST_DAMPING_KEEP,
                w.z * REST_DAMPING_KEEP,
            ),
            false,
        );
    }
}
