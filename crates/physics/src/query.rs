//! query: ray casts and swept-sphere casts against cooked static colliders.
//!
//! Consumer: the door verbs `physics.raycast` and `physics.shapecast`. Data owner:
//! [`crate::collider`] (the shapes) and [`crate::layers`] (the scope). Units: meters,
//! Y up; directions are normalized here, distances are along the unit direction.
//! Determinism: `+ - * /`, `sqrt` and comparisons in `f32`, colliders and triangles in
//! stored order, ties go to the earlier one; the same world and the same cast always
//! give the same hit on every machine. Errors: `CastError` names the bad argument;
//! a clean miss is `Ok(None)`. Budget: one pass over the in-scope triangles per cast,
//! with a bounding-box skip per collider; no allocation.
//!
//! Every shape is a triangle soup and every triangle is two-sided, so a cast that
//! starts inside a closed collider reports the wall it meets going out. A sphere that
//! already touches a triangle at the start reports distance zero and the push-out
//! direction. A swept sphere is tested against each triangle as a plane, three edge
//! cylinders and three corner spheres, which is exact for the whole triangle soup.

use core::fmt;

use engine_core::fixed::Fixed;
use engine_core::vec3::cross3;

use crate::collider::Collider;

type V3 = [f32; 3];

/// Why a cast was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastError {
    /// An origin, direction, radius or distance is NaN or infinite.
    NonFinite,
    /// The direction has no length.
    ZeroDirection,
    /// The distance is not above zero.
    BadDistance,
    /// The radius is negative.
    BadRadius,
    /// The scope names no layer bit.
    Unscoped,
}

impl fmt::Display for CastError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let words = match self {
            Self::NonFinite => "a cast argument is not finite",
            Self::ZeroDirection => "the cast direction has no length",
            Self::BadDistance => "the cast distance must be above zero",
            Self::BadRadius => "the cast radius must not be negative",
            Self::Unscoped => "the cast scope names no layer",
        };
        f.write_str(words)
    }
}

impl std::error::Error for CastError {}

/// The nearest thing a cast met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Index of the collider in the slice cast against.
    pub collider: usize,
    /// Distance travelled along the unit direction (the sphere centre for a sphere).
    pub distance: f32,
    /// Contact point on the surface.
    pub point: [f32; 3],
    /// Unit surface normal facing the cast.
    pub normal: [f32; 3],
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: V3) -> Option<V3> {
    let len = dot(a, a).sqrt();
    (len > 1.0e-9).then(|| scale(a, 1.0 / len))
}

/// A candidate contact: distance, point, unit normal.
type Contact = (f32, V3, V3);

/// Keep the earlier of two candidates unless the new one is strictly nearer.
fn nearer(best: &mut Option<Contact>, found: Option<Contact>) {
    if let Some(c) = found {
        if best.is_none_or(|b| c.0 < b.0) {
            *best = Some(c);
        }
    }
}

/// Ray `origin + t * dir` against one triangle (Moller-Trumbore, two-sided).
fn ray_triangle(o: V3, d: V3, tri: &[V3; 3], max_t: f32) -> Option<Contact> {
    let e1 = sub(tri[1], tri[0]);
    let e2 = sub(tri[2], tri[0]);
    let p = cross3(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1.0e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(o, tri[0]);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross3(s, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    if !(0.0..=max_t).contains(&t) {
        return None;
    }
    let n = unit(cross3(e1, e2))?;
    let facing = if dot(n, d) < 0.0 { n } else { scale(n, -1.0) };
    Some((t, add(o, scale(d, t)), facing))
}

/// Closest point on a triangle to `p` (Ericson, real-time collision detection 5.1.5).
fn closest_on_triangle(p: V3, tri: &[V3; 3]) -> V3 {
    let [a, b, c] = *tri;
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add(a, scale(ab, d1 / (d1 - d3)));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add(a, scale(ac, d2 / (d2 - d6)));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return add(b, scale(sub(c, b), w));
    }
    let denom = 1.0 / (va + vb + vc);
    add(a, add(scale(ab, vb * denom), scale(ac, vc * denom)))
}

/// True when `p` (on the triangle's plane) lies inside the triangle.
fn inside_triangle(p: V3, tri: &[V3; 3], n: V3) -> bool {
    (0..3).all(|i| {
        let (a, b) = (tri[i], tri[(i + 1) % 3]);
        dot(cross3(sub(b, a), sub(p, a)), n) >= 0.0
    })
}

/// First time the sphere centre path `o + t d` is `r` from the segment `p..q`, if the
/// contact lies within the segment.
fn sweep_edge(o: V3, d: V3, r: f32, p: V3, q: V3, max_t: f32) -> Option<Contact> {
    let e = sub(q, p);
    let ee = dot(e, e);
    if ee <= 0.0 {
        return None;
    }
    let m = sub(o, p);
    let d_perp = sub(d, scale(e, dot(d, e) / ee));
    let m_perp = sub(m, scale(e, dot(m, e) / ee));
    let a = dot(d_perp, d_perp);
    if a < 1.0e-9 {
        return None;
    }
    let b = dot(m_perp, d_perp);
    let c = dot(m_perp, m_perp) - r * r;
    if c < 0.0 {
        return None;
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / a;
    if !(0.0..=max_t).contains(&t) {
        return None;
    }
    let centre = add(o, scale(d, t));
    let u = dot(sub(centre, p), e) / ee;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let on_edge = add(p, scale(e, u));
    let n = unit(sub(centre, on_edge))?;
    Some((t, on_edge, n))
}

/// First time the sphere centre path `o + t d` is `r` from the point `v`.
fn sweep_vertex(o: V3, d: V3, r: f32, v: V3, max_t: f32) -> Option<Contact> {
    let m = sub(o, v);
    let b = dot(m, d);
    let c = dot(m, m) - r * r;
    if c < 0.0 {
        return None;
    }
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    if !(0.0..=max_t).contains(&t) {
        return None;
    }
    let n = unit(sub(add(o, scale(d, t)), v))?;
    Some((t, v, n))
}

/// Sphere of radius `r` swept along `o + t d` against one two-sided triangle.
fn sphere_triangle(o: V3, d: V3, r: f32, tri: &[V3; 3], max_t: f32) -> Option<Contact> {
    let n = unit(cross3(sub(tri[1], tri[0]), sub(tri[2], tri[0])))?;
    let closest = closest_on_triangle(o, tri);
    let gap = sub(o, closest);
    if dot(gap, gap) < r * r {
        // Already touching: no travel, pushed out along the gap (or against the cast).
        let normal = unit(gap).unwrap_or_else(|| scale(d, -1.0));
        return Some((0.0, closest, normal));
    }
    let mut best = None;
    let side = dot(n, sub(o, tri[0]));
    let facing = if side >= 0.0 { n } else { scale(n, -1.0) };
    let approach = dot(facing, d);
    if approach < -1.0e-7 {
        let t = (side.abs() - r) / -approach;
        if (0.0..=max_t).contains(&t) {
            let touch = sub(add(o, scale(d, t)), scale(facing, r));
            if inside_triangle(touch, tri, n) {
                nearer(&mut best, Some((t, touch, facing)));
            }
        }
    }
    for i in 0..3 {
        nearer(
            &mut best,
            sweep_edge(o, d, r, tri[i], tri[(i + 1) % 3], max_t),
        );
    }
    for v in tri {
        nearer(&mut best, sweep_vertex(o, d, r, *v, max_t));
    }
    best
}

/// True when the ray `o + t d`, `t <= max_t`, can touch the box grown by `grow`.
fn ray_near_box(o: V3, d: V3, lo: V3, hi: V3, grow: f32, max_t: f32) -> bool {
    let (mut t0, mut t1) = (0.0f32, max_t);
    for axis in 0..3 {
        let (min, max) = (lo[axis] - grow, hi[axis] + grow);
        if d[axis].abs() < 1.0e-9 {
            if o[axis] < min || o[axis] > max {
                return false;
            }
        } else {
            let (a, b) = ((min - o[axis]) / d[axis], (max - o[axis]) / d[axis]);
            t0 = t0.max(a.min(b));
            t1 = t1.min(a.max(b));
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Validate the cast arguments; returns the unit direction.
fn check(
    origin: [f32; 3],
    dir: [f32; 3],
    radius: f32,
    max_distance: f32,
    scope: u32,
) -> Result<V3, CastError> {
    if origin.iter().chain(&dir).any(|v| !v.is_finite())
        || !radius.is_finite()
        || !max_distance.is_finite()
    {
        return Err(CastError::NonFinite);
    }
    if radius < 0.0 {
        return Err(CastError::BadRadius);
    }
    if max_distance <= 0.0 {
        return Err(CastError::BadDistance);
    }
    if scope & crate::layers::ALL == 0 {
        return Err(CastError::Unscoped);
    }
    unit(dir).ok_or(CastError::ZeroDirection)
}

/// Sweep a sphere of `radius` from `origin` along `dir` (any length) for at most
/// `max_distance` meters against the colliders whose layer is in `scope`; the nearest
/// hit, if any. `distance` is how far the sphere centre travels before contact. A zero
/// radius is a ray.
pub fn sphere_cast(
    colliders: &[Collider],
    origin: [f32; 3],
    dir: [f32; 3],
    radius: f32,
    max_distance: f32,
    scope: u32,
) -> Result<Option<Hit>, CastError> {
    let d = check(origin, dir, radius, max_distance, scope)?;
    let mut best: Option<(usize, Contact)> = None;
    for (index, collider) in colliders.iter().enumerate() {
        if collider.layer & scope == 0 {
            continue;
        }
        if let Some((lo, hi)) = collider.shape.bounds() {
            if !ray_near_box(origin, d, lo, hi, radius, max_distance) {
                continue;
            }
        }
        let limit = best.map_or(max_distance, |(_, c)| c.0);
        let mut found: Option<Contact> = None;
        for tri in collider.shape.triangles() {
            let hit = if radius > 0.0 {
                sphere_triangle(origin, d, radius, &tri, limit)
            } else {
                ray_triangle(origin, d, &tri, limit)
            };
            nearer(&mut found, hit);
        }
        if let Some(c) = found {
            if best.is_none_or(|(_, b)| c.0 < b.0) {
                best = Some((index, c));
            }
        }
    }
    Ok(best.map(|(collider, (distance, point, normal))| Hit {
        collider,
        distance,
        point,
        normal,
    }))
}

/// Cast a ray from `origin` along `dir` (any length) for at most `max_distance`
/// meters against the colliders whose layer is in `scope`; the nearest hit, if any.
pub fn ray_cast(
    colliders: &[Collider],
    origin: [f32; 3],
    dir: [f32; 3],
    max_distance: f32,
    scope: u32,
) -> Result<Option<Hit>, CastError> {
    sphere_cast(colliders, origin, dir, 0.0, max_distance, scope)
}

// ---- Q16 swept capsule casts (PX20) -------------------------------------------------
//
// The same query in `Fixed` (Q16.16) for the integer simulation: a swept sphere (radius
// zero is a segment) against capsules, every hit with its time of impact as a Q16
// fraction of the move, sorted nearest first. Integer `+ - *` and `sqrt` only, no `f32`;
// targets are independent, so the answer does not depend on their order.

type F3 = [Fixed; 3];

/// Largest offset, in meters, from the sweep's start that [`swept_cast`] works in: the move
/// itself and every capsule near its path (keeps every Q16 product far from overflow). The
/// sweep runs in the frame of its start, so world position is free: a cast 5 km from the
/// origin answers exactly as the same cast at the origin.
pub const SWEPT_RANGE_M: i64 = 256;
/// Largest sphere or capsule radius, in meters, that [`swept_cast`] accepts.
pub const SWEPT_RADIUS_M: i64 = 64;

/// Why a Q16 cast was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweptError {
    /// The move, or a capsule its path can reach, spans beyond [`SWEPT_RANGE_M`] from the
    /// start, or a radius is negative or beyond [`SWEPT_RADIUS_M`].
    OutOfRange,
}

impl fmt::Display for SweptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a swept cast coordinate or radius is out of range")
    }
}

impl std::error::Error for SweptError {}

/// A capsule a sweep can hit: the segment `a..b` grown by `radius` (`a == b` is a sphere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CastCapsule {
    /// Caller id, unique per cast; breaks ties between equal times of impact.
    pub id: u32,
    /// One end of the axis, meters.
    pub a: [Fixed; 3],
    /// The other end of the axis, meters.
    pub b: [Fixed; 3],
    /// Radius around the axis, meters.
    pub radius: Fixed,
}

/// One capsule a sweep met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SweptHit {
    /// The [`CastCapsule::id`] that was hit.
    pub id: u32,
    /// Time of impact, a fraction of the move: zero when the sweep starts touching,
    /// one when contact comes only at the end.
    pub toi: Fixed,
    /// Unit direction from the capsule towards the sphere centre at contact.
    pub normal: [Fixed; 3],
}

fn fsub(a: F3, b: F3) -> F3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn fadd(a: F3, b: F3) -> F3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn fscale(a: F3, s: Fixed) -> F3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn fdot(a: F3, b: F3) -> Fixed {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn funit(a: F3) -> Option<F3> {
    let len = fdot(a, a).sqrt();
    (len.0 > 0).then(|| a.map(|v| v / len))
}

/// `dot / axis_sq` as a Q16 fraction, from exact Q32 products (`axis_sq` above zero).
fn along(p: F3, e: F3, axis_sq: i128) -> Fixed {
    let dot: i128 = p
        .iter()
        .zip(&e)
        .map(|(x, y)| i128::from(x.0) * i128::from(y.0))
        .sum();
    Fixed(i64::try_from((dot << 16) / axis_sq).unwrap_or(i64::MAX))
}

/// Normal at a contact: away from the touched point, else against the move.
fn contact_normal(centre: F3, touched: F3, d: F3) -> F3 {
    let against = fscale(d, Fixed::NEG_ONE);
    funit(fsub(centre, touched))
        .or_else(|| funit(against))
        .unwrap_or([Fixed::ZERO, Fixed::ONE, Fixed::ZERO])
}

/// First `t` in `0..=1` where the point `o + t d` is `r` from the point `v`.
fn sweep_point(o: F3, d: F3, v: F3, r: Fixed) -> Option<(Fixed, F3)> {
    let (a, m) = (fdot(d, d), fsub(o, v));
    let (b, c) = (fdot(m, d), fdot(m, m) - r * r);
    let disc = b * b - a * c;
    if a.0 == 0 || b.0 >= 0 || c.0 < 0 || disc.0 < 0 {
        return None;
    }
    let t = (-b - disc.sqrt()).checked_div(a)?;
    (t.0 >= 0 && t <= Fixed::ONE).then(|| (t, contact_normal(fadd(o, fscale(d, t)), v, d)))
}

/// First `t` in `0..=1` where `o + t d` is `r` from the axis `a + u e`, `0 <= u <= 1`.
fn sweep_side(o: F3, d: F3, (a, e, axis_sq): (F3, F3, i128), r: Fixed) -> Option<(Fixed, F3)> {
    let m = fsub(o, a);
    let d_off = fsub(d, fscale(e, along(d, e, axis_sq)));
    let m_off = fsub(m, fscale(e, along(m, e, axis_sq)));
    let (qa, b) = (fdot(d_off, d_off), fdot(m_off, d_off));
    let c = fdot(m_off, m_off) - r * r;
    let disc = b * b - qa * c;
    if qa.0 == 0 || b.0 >= 0 || c.0 < 0 || disc.0 < 0 {
        return None;
    }
    let t = (-b - disc.sqrt()).checked_div(qa)?;
    let centre = fadd(o, fscale(d, t));
    let u = along(fsub(centre, a), e, axis_sq);
    let in_range = t.0 >= 0 && t <= Fixed::ONE && u.0 >= 0 && u <= Fixed::ONE;
    in_range.then(|| (t, contact_normal(centre, fadd(a, fscale(e, u)), d)))
}

/// The earliest contact of the sphere path `o + t d` with one capsule grown by `radius`.
fn sweep_capsule(o: F3, d: F3, radius: Fixed, cap: &CastCapsule) -> Option<(Fixed, F3)> {
    let r = radius + cap.radius;
    let e = fsub(cap.b, cap.a);
    let axis_sq: i128 = e.iter().map(|v| i128::from(v.0) * i128::from(v.0)).sum();
    // A capsule axis shorter than 1/256 m is a sphere: its direction is below Q16 noise.
    let long = axis_sq >= 1 << 16;
    let u = if long {
        along(fsub(o, cap.a), e, axis_sq).clamp(Fixed::ZERO, Fixed::ONE)
    } else {
        Fixed::ZERO
    };
    let closest = fadd(cap.a, fscale(e, u));
    let gap = fsub(o, closest);
    if fdot(gap, gap) <= r * r {
        return Some((Fixed::ZERO, contact_normal(o, closest, d)));
    }
    let mut best: Option<(Fixed, F3)> = None;
    let sides = long
        .then(|| sweep_side(o, d, (cap.a, e, axis_sq), r))
        .flatten();
    let ends = [Some(cap.a), long.then_some(cap.b)];
    let found = std::iter::once(sides)
        .chain(ends.into_iter().flatten().map(|v| sweep_point(o, d, v, r)))
        .flatten();
    for hit in found {
        if best.is_none_or(|b| hit.0 < b.0) {
            best = Some(hit);
        }
    }
    best
}

fn within(points: &[F3], limit: i64) -> bool {
    points
        .iter()
        .flatten()
        .all(|v| v.0.unsigned_abs() <= limit.unsigned_abs() * Fixed::SCALE.unsigned_abs())
}

/// True when the capsule `a..b` grown by `reach` cannot meet the move `0..d` because their
/// boxes are apart on some axis (all in the frame of the sweep's start).
fn apart(d: F3, a: F3, b: F3, reach: Fixed) -> bool {
    (0..3).any(|axis| {
        let (move_lo, move_hi) = (d[axis].min(Fixed::ZERO), d[axis].max(Fixed::ZERO));
        let cap_lo = a[axis].min(b[axis]).saturating_sub(reach);
        let cap_hi = a[axis].max(b[axis]).saturating_add(reach);
        cap_hi < move_lo || cap_lo > move_hi
    })
}

/// Sweep a sphere of `radius` (zero is a segment) from `from` to `to` against `targets`;
/// `out` is cleared and filled with every capsule met, nearest time of impact first,
/// ties by [`CastCapsule::id`]. The order of `targets` never changes `out`.
///
/// The sweep runs in the frame of `from`: every product in it reads an offset (`o - v`,
/// `to - from`), never an absolute coordinate, so moving the whole scene changes no bit of
/// the answer. A capsule far from the path is skipped, not refused, so a big map pays nothing
/// for its far fighters.
///
/// # Errors
/// [`SweptError::OutOfRange`] when the move spans beyond [`SWEPT_RANGE_M`], a capsule that
/// can reach the path spans beyond it from `from`, or a radius is outside
/// `0..=`[`SWEPT_RADIUS_M`].
pub fn swept_cast(
    from: [Fixed; 3],
    to: [Fixed; 3],
    radius: Fixed,
    targets: &[CastCapsule],
    out: &mut Vec<SweptHit>,
) -> Result<(), SweptError> {
    out.clear();
    let radius_ok = |r: Fixed| (0..=SWEPT_RADIUS_M * Fixed::SCALE).contains(&r.0);
    let d = fsub(to, from);
    if !within(&[d], SWEPT_RANGE_M) || !radius_ok(radius) {
        return Err(SweptError::OutOfRange);
    }
    let origin = [Fixed::ZERO; 3];
    for cap in targets {
        if !radius_ok(cap.radius) {
            out.clear();
            return Err(SweptError::OutOfRange);
        }
        let local = CastCapsule {
            a: fsub(cap.a, from),
            b: fsub(cap.b, from),
            ..*cap
        };
        if !within(&[local.a, local.b], SWEPT_RANGE_M) {
            if apart(d, local.a, local.b, radius + cap.radius) {
                continue;
            }
            out.clear();
            return Err(SweptError::OutOfRange);
        }
        if let Some((toi, normal)) = sweep_capsule(origin, d, radius, &local) {
            out.push(SweptHit {
                id: cap.id,
                toi,
                normal,
            });
        }
    }
    out.sort_unstable_by_key(|h| (h.toi.0, h.id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collider::tests::cube_hull;
    use crate::collider::Shape;
    use crate::layers;

    fn cube(id: &str, layer: u32) -> Collider {
        Collider {
            id: id.to_owned(),
            layer,
            shape: cube_hull(),
        }
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1.0e-4)
    }

    #[test]
    fn a_ray_hits_the_near_face_with_its_point_and_normal() {
        let world = [cube("COL_cube", layers::WORLD)];
        let hit = ray_cast(
            &world,
            [-3.0, 0.5, 0.5],
            [2.0, 0.0, 0.0],
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("a hit");
        assert_eq!(hit.collider, 0);
        assert!(
            (hit.distance - 3.0).abs() < 1.0e-5,
            "length of dir does not matter"
        );
        assert!(close(hit.point, [0.0, 0.5, 0.5]), "{:?}", hit.point);
        assert!(close(hit.normal, [-1.0, 0.0, 0.0]), "{:?}", hit.normal);
    }

    #[test]
    fn a_ray_respects_range_aim_nearest_and_scope() {
        let world = [
            cube("COL_far", layers::WORLD),
            Collider {
                id: "COL_near".to_owned(),
                layer: layers::PROP,
                shape: match cube_hull() {
                    Shape::Hull { points, faces } => Shape::Hull {
                        points: points.iter().map(|p| [p[0] - 4.0, p[1], p[2]]).collect(),
                        faces,
                    },
                    Shape::Mesh { .. } => unreachable!("cube_hull is a hull"),
                },
            },
        ];
        let from = [-8.0, 0.5, 0.5];
        let both = layers::WORLD | layers::PROP;
        // The prop at x -4..-3 is nearer than the cube at x 0..1.
        let hit = ray_cast(&world, from, [1.0, 0.0, 0.0], 20.0, both)
            .expect("valid cast")
            .expect("hit");
        assert_eq!(hit.collider, 1);
        assert!((hit.distance - 4.0).abs() < 1.0e-5);
        // Scope filters by layer: only the world cube answers.
        let hit = ray_cast(&world, from, [1.0, 0.0, 0.0], 20.0, layers::WORLD)
            .expect("valid cast")
            .expect("hit");
        assert_eq!(hit.collider, 0);
        assert!((hit.distance - 8.0).abs() < 1.0e-5);
        // Too short, off to the side, and pointing away all miss.
        assert_eq!(ray_cast(&world, from, [1.0, 0.0, 0.0], 3.0, both), Ok(None));
        assert_eq!(
            ray_cast(&world, [-8.0, 2.0, 0.5], [1.0, 0.0, 0.0], 20.0, both),
            Ok(None)
        );
        assert_eq!(
            ray_cast(&world, from, [-1.0, 0.0, 0.0], 20.0, both),
            Ok(None)
        );
        // Same cast twice, same answer.
        assert_eq!(
            ray_cast(&world, from, [1.0, 0.0, 0.0], 20.0, both),
            ray_cast(&world, from, [1.0, 0.0, 0.0], 20.0, both)
        );
    }

    #[test]
    fn a_ray_hits_a_mesh_and_a_ray_from_inside_reports_the_wall_it_leaves_by() {
        let Shape::Hull { points, faces } = cube_hull() else {
            unreachable!("cube is a hull");
        };
        let mesh = Collider {
            id: "COL_mesh".to_owned(),
            layer: layers::WORLD,
            shape: Shape::Mesh {
                vertices: points,
                indices: faces.iter().flatten().map(|&i| u32::from(i)).collect(),
            },
        };
        let hit = ray_cast(
            core::slice::from_ref(&mesh),
            [0.5, 5.0, 0.5],
            [0.0, -1.0, 0.0],
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("hit");
        assert!(close(hit.point, [0.5, 1.0, 0.5]) && close(hit.normal, [0.0, 1.0, 0.0]));
        let out = ray_cast(
            core::slice::from_ref(&mesh),
            [0.5, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("hit");
        assert!((out.distance - 0.5).abs() < 1.0e-5);
        assert!(
            close(out.normal, [-1.0, 0.0, 0.0]),
            "the wall faces the ray's origin side"
        );
    }

    #[test]
    fn bad_cast_arguments_are_refused_by_name() {
        let world = [cube("COL_cube", layers::WORLD)];
        let ok = [0.0, 1.0, 0.0];
        let scope = layers::WORLD;
        assert_eq!(
            ray_cast(&world, [f32::NAN, 0.0, 0.0], ok, 5.0, scope),
            Err(CastError::NonFinite)
        );
        assert_eq!(
            ray_cast(&world, [0.0; 3], [0.0; 3], 5.0, scope),
            Err(CastError::ZeroDirection)
        );
        assert_eq!(
            ray_cast(&world, [0.0; 3], ok, 0.0, scope),
            Err(CastError::BadDistance)
        );
        assert_eq!(
            ray_cast(&world, [0.0; 3], ok, f32::INFINITY, scope),
            Err(CastError::NonFinite)
        );
        assert_eq!(
            ray_cast(&world, [0.0; 3], ok, 5.0, 0),
            Err(CastError::Unscoped)
        );
        assert_eq!(
            ray_cast(&world, [0.0; 3], ok, 5.0, 1 << 9),
            Err(CastError::Unscoped)
        );
        assert_eq!(
            sphere_cast(&world, [0.0; 3], ok, -1.0, 5.0, scope),
            Err(CastError::BadRadius)
        );
    }

    #[test]
    fn a_swept_sphere_stops_one_radius_short_of_a_face() {
        let world = [cube("COL_cube", layers::WORLD)];
        let hit = sphere_cast(
            &world,
            [-3.0, 0.5, 0.5],
            [1.0, 0.0, 0.0],
            0.5,
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("a hit");
        assert!(
            (hit.distance - 2.5).abs() < 1.0e-5,
            "centre travels to x = -0.5"
        );
        assert!(close(hit.point, [0.0, 0.5, 0.5]), "{:?}", hit.point);
        assert!(close(hit.normal, [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn a_swept_sphere_rolls_over_an_edge_and_catches_a_corner() {
        let world = [cube("COL_cube", layers::WORLD)];
        // Centre 0.3 above the top edge: the rounded edge is met at x = -0.4.
        let edge = sphere_cast(
            &world,
            [-3.0, 1.3, 0.5],
            [1.0, 0.0, 0.0],
            0.5,
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("edge hit");
        assert!((edge.distance - 2.6).abs() < 1.0e-4, "{}", edge.distance);
        assert!(close(edge.point, [0.0, 1.0, 0.5]), "{:?}", edge.point);
        assert!(close(edge.normal, [-0.8, 0.6, 0.0]), "{:?}", edge.normal);
        // Aimed past the corner (0, 1, 1): only the corner sphere is met.
        let corner = sphere_cast(
            &world,
            [-3.0, 1.3, 1.3],
            [1.0, 0.0, 0.0],
            0.5,
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("corner hit");
        assert!(
            (corner.distance - (3.0 - 0.07f32.sqrt())).abs() < 1.0e-4,
            "{}",
            corner.distance
        );
        assert!(close(corner.point, [0.0, 1.0, 1.0]));
        assert!(
            close(corner.normal, [-0.07f32.sqrt() / 0.5, 0.6, 0.6]),
            "{:?}",
            corner.normal
        );
        // A path that clears the cube by more than the radius misses.
        let miss = sphere_cast(
            &world,
            [-3.0, 1.6, 0.5],
            [1.0, 0.0, 0.0],
            0.5,
            10.0,
            layers::WORLD,
        );
        assert_eq!(miss, Ok(None));
    }

    #[test]
    fn a_sphere_that_starts_touching_reports_zero_and_the_way_out() {
        let world = [cube("COL_cube", layers::WORLD)];
        let hit = sphere_cast(
            &world,
            [-0.3, 0.5, 0.5],
            [0.0, 0.0, 1.0],
            0.5,
            10.0,
            layers::WORLD,
        )
        .expect("valid cast")
        .expect("a hit");
        assert_eq!(hit.distance, 0.0);
        assert!(close(hit.normal, [-1.0, 0.0, 0.0]), "{:?}", hit.normal);
        // A zero radius is the ray cast.
        assert_eq!(
            sphere_cast(
                &world,
                [-3.0, 0.5, 0.5],
                [1.0, 0.0, 0.0],
                0.0,
                10.0,
                layers::WORLD
            ),
            ray_cast(
                &world,
                [-3.0, 0.5, 0.5],
                [1.0, 0.0, 0.0],
                10.0,
                layers::WORLD
            )
        );
    }

    fn q(v: f32) -> Fixed {
        Fixed((v * 65536.0).round() as i64)
    }

    fn q3(v: [f32; 3]) -> F3 {
        v.map(q)
    }

    fn cap(id: u32, a: [f32; 3], b: [f32; 3], radius: f32) -> CastCapsule {
        CastCapsule {
            id,
            a: q3(a),
            b: q3(b),
            radius: q(radius),
        }
    }

    fn run(from: [f32; 3], to: [f32; 3], radius: f32, targets: &[CastCapsule]) -> Vec<SweptHit> {
        let mut out = Vec::new();
        swept_cast(q3(from), q3(to), q(radius), targets, &mut out).expect("in range");
        out
    }

    fn flt(v: Fixed) -> f32 {
        v.0 as f32 / 65536.0
    }

    #[test]
    fn swept_cast_meets_a_sphere_a_side_and_an_end_cap() {
        // Sphere of radius 1 at the origin, swept point radius 0.5, path x -10 -> 10:
        // contact at x = -1.5, so t = 8.5 / 20.
        let sphere = [cap(1, [0.0; 3], [0.0; 3], 1.0)];
        let hit = run([-10.0, 0.0, 0.0], [10.0, 0.0, 0.0], 0.5, &sphere);
        assert_eq!(hit.len(), 1);
        assert!(
            (flt(hit[0].toi) - 0.425).abs() < 1.0e-3,
            "{}",
            flt(hit[0].toi)
        );
        assert!((flt(hit[0].normal[0]) + 1.0).abs() < 1.0e-2);
        // Capsule along y from -2 to 2: a sweep at mid height meets the side at x = -1.5 ...
        let tall = [cap(2, [0.0, -2.0, 0.0], [0.0, 2.0, 0.0], 1.0)];
        let side = run([-10.0, 0.5, 0.0], [10.0, 0.5, 0.0], 0.5, &tall);
        assert!((flt(side[0].toi) - 0.425).abs() < 1.0e-3);
        assert!((flt(side[0].normal[0]) + 1.0).abs() < 1.0e-2);
        // ... and one coming down onto the top meets the rounded end cap at y = 3.5.
        let cap_hit = run([0.0, 10.0, 0.0], [0.0, -10.0, 0.0], 0.5, &tall);
        assert!((flt(cap_hit[0].toi) - 0.325).abs() < 1.0e-3);
        assert!((flt(cap_hit[0].normal[1]) - 1.0).abs() < 1.0e-2);
        // A path that clears the radii misses; one that starts inside reports zero.
        assert!(run([-10.0, 5.0, 0.0], [10.0, 5.0, 0.0], 0.5, &tall).is_empty());
        let inside = run([0.5, 0.0, 0.0], [10.0, 0.0, 0.0], 0.5, &tall);
        assert_eq!(inside[0].toi, Fixed::ZERO);
        assert!((flt(inside[0].normal[0]) - 1.0).abs() < 1.0e-2);
    }

    #[test]
    fn swept_cast_sorts_by_time_of_impact_then_id_and_refuses_far_points() {
        let row = [
            cap(9, [6.0, 0.0, 0.0], [6.0, 0.0, 0.0], 0.5),
            cap(4, [2.0, 0.0, 0.0], [2.0, 0.0, 0.0], 0.5),
            cap(7, [2.0, 3.0, 0.0], [2.0, 3.0, 0.0], 0.5),
            cap(1, [2.0, -3.0, 0.0], [2.0, -3.0, 0.0], 0.5),
        ];
        let hits = run([0.0; 3], [10.0, 0.0, 0.0], 0.0, &row);
        let ids: Vec<u32> = hits.iter().map(|h| h.id).collect();
        assert_eq!(ids, [4, 9]);
        // Two equal times of impact keep id order whatever the target order.
        let twins = [row[2], row[3]];
        let tall = run(
            [2.0, 0.0, 0.0],
            [2.0, 8.0, 0.0],
            0.0,
            &[
                twins[1],
                cap(2, [2.0, 3.0, 0.0], [2.0, 3.0, 0.0], 0.5),
                twins[0],
            ],
        );
        assert_eq!(tall.iter().map(|h| h.id).collect::<Vec<_>>(), [2, 7]);
        let mut out = Vec::new();
        let far = [q(300.0), Fixed::ZERO, Fixed::ZERO];
        assert_eq!(
            swept_cast([Fixed::ZERO; 3], far, Fixed::ZERO, &[], &mut out),
            Err(SweptError::OutOfRange)
        );
        assert_eq!(
            swept_cast([Fixed::ZERO; 3], [Fixed::ONE; 3], q(-1.0), &[], &mut out),
            Err(SweptError::OutOfRange)
        );
    }

    #[test]
    fn swept_cast_answers_the_same_anywhere_on_a_big_map() {
        // Law: physics does not care where the origin is. The same throw 5 km out (a Rome
        // chapter map, an urban-war city) meets the same capsules at the same times with the
        // same normals, bit for bit; before, any point past 256 m refused the whole cast and
        // the spear flew through everyone.
        let row = [
            cap(9, [6.0, 0.6, 0.0], [6.0, 0.6, 0.0], 0.5),
            cap(4, [2.0, 0.3, 0.0], [2.0, 1.8, 0.0], 0.4),
            cap(7, [4.0, -1.0, 0.2], [4.5, 1.0, -0.3], 0.35),
        ];
        let (from, to) = (q3([0.0, 1.0, 0.0]), q3([10.0, 0.5, 0.1]));
        let home = run_fixed(from, to, &row);
        assert_eq!(home.len(), 3, "the fixture must hit all three: {home:?}");
        for shift in [[5_000, -300, 4_000], [-20_000, 0, 77]] {
            let s = shift.map(Fixed::from_int);
            let far_row = row.map(|c| CastCapsule {
                a: fadd(c.a, s),
                b: fadd(c.b, s),
                ..c
            });
            let far = run_fixed(fadd(from, s), fadd(to, s), &far_row);
            assert_eq!(far, home, "shift {shift:?} changed the answer");
        }
        // A fighter 10 km away cannot meet a 10 m throw: skipped, never a refusal.
        let mut lonely = row.to_vec();
        lonely.push(cap(3, [10_000.0, 0.0, 0.0], [10_000.0, 2.0, 0.0], 0.4));
        assert_eq!(run_fixed(from, to, &lonely), home);
        // One that straddles the path but spans past the range is still refused, not guessed.
        let mut wall = row.to_vec();
        wall.push(cap(5, [5.0, 1.0, -400.0], [5.0, 1.0, 400.0], 0.4));
        let mut out = Vec::new();
        assert_eq!(
            swept_cast(from, to, q(0.08), &wall, &mut out),
            Err(SweptError::OutOfRange)
        );
        assert!(out.is_empty());
    }

    /// [`swept_cast`] on Q16 points with a 0.08 m sphere.
    fn run_fixed(from: F3, to: F3, targets: &[CastCapsule]) -> Vec<SweptHit> {
        let mut out = Vec::new();
        swept_cast(from, to, q(0.08), targets, &mut out).expect("cast in range");
        out
    }

    /// Distance from `p` to the segment `a..b`, in `f32` (the test's own reference).
    fn dist_to_axis(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> f32 {
        let (e, m) = (sub(b, a), sub(p, a));
        let ee = dot(e, e);
        let u = if ee > 0.0 {
            (dot(m, e) / ee).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let gap = sub(m, scale(e, u));
        dot(gap, gap).sqrt()
    }

    /// 1,000 seeded casts, each checked against a dense march and a permuted target list;
    /// returns a digest of every hit.
    fn thousand_seeded_casts() -> u64 {
        let mut rng = engine_core::Rng::new(2040);
        let mut pick =
            |lo: f32, hi: f32| lo + (rng.next_u32() % 10_000) as f32 / 10_000.0 * (hi - lo);
        let mut digest = Vec::new();
        for _ in 0..1000 {
            let from = [pick(-6.0, 6.0), pick(-6.0, 6.0), pick(-6.0, 6.0)];
            let to = [pick(-6.0, 6.0), pick(-6.0, 6.0), pick(-6.0, 6.0)];
            let radius = pick(0.0, 0.6);
            let targets: Vec<CastCapsule> = (0..8)
                .map(|id| {
                    let a = [pick(-5.0, 5.0), pick(-5.0, 5.0), pick(-5.0, 5.0)];
                    let b = if id % 3 == 0 {
                        a
                    } else {
                        [
                            a[0] + pick(-2.0, 2.0),
                            a[1] + pick(-2.0, 2.0),
                            a[2] + pick(-2.0, 2.0),
                        ]
                    };
                    cap(id, a, b, pick(0.1, 0.8))
                })
                .collect();
            let hits = run(from, to, radius, &targets);
            // Reversed and rotated target lists give the identical answer.
            let mut shuffled = targets.clone();
            shuffled.reverse();
            shuffled.rotate_left(3);
            assert_eq!(hits, run(from, to, radius, &shuffled));
            assert!(hits
                .windows(2)
                .all(|w| (w[0].toi.0, w[0].id) < (w[1].toi.0, w[1].id)));
            for t in &targets {
                let (a, b) = (t.a.map(flt), t.b.map(flt));
                let reach = radius + flt(t.radius);
                let at = |s: f32| {
                    let p = [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * s);
                    dist_to_axis(p, a, b) - reach
                };
                // Deepest approach over a dense march (1/2048 of a move up to 20 m).
                let march = (0..=2048).map(|i| (i, at(i as f32 / 2048.0)));
                let first_inside = march.clone().find(|(_, g)| *g < -0.02).map(|(i, _)| i);
                match hits.iter().find(|h| h.id == t.id) {
                    None => assert!(
                        first_inside.is_none(),
                        "missed a hit the march found: {t:?}"
                    ),
                    Some(h) => {
                        let toi = flt(h.toi);
                        assert!(
                            at(toi).abs() < 0.05 || toi == 0.0,
                            "contact off the surface: {}",
                            at(toi)
                        );
                        let before = (toi * 2048.0) as i32;
                        assert!(
                            first_inside.is_none_or(|i| i + 4 >= before),
                            "tunnelled before the reported hit"
                        );
                    }
                }
            }
            digest.extend(hits.iter().flat_map(|h| [u64::from(h.id), h.toi.0 as u64]));
        }
        engine_core::Rng::digest(&digest)
    }

    #[test]
    fn swept_cast_thousand_seeded_casts_match_a_dense_march_and_hash_the_same_twice() {
        let first = thousand_seeded_casts();
        assert_eq!(first, thousand_seeded_casts());
        assert_ne!(
            first,
            engine_core::Rng::digest(&[]),
            "the seeds must hit something"
        );
    }
}
