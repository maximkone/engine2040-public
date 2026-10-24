//! geom: the small ray and box maths the scene report needs, on plain arrays.
//!
//! Consumer: `scene_report` (camera sight lines, sun rays, contact gaps). View
//! only: nothing here reaches the simulation or its hash. Units: meters.
//! Determinism: pure `f32` in, same `f32` out, fixed loop order. Errors: none;
//! a non-finite input reads as a miss, never a panic.

use engine_render::v1::Camera;

/// A world point.
pub type P3 = [f32; 3];

/// `a - b`.
#[must_use]
pub fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `a + b`.
#[must_use]
pub fn add(a: P3, b: P3) -> P3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `a * k`.
#[must_use]
pub fn scale(a: P3, k: f32) -> P3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Dot product.
#[must_use]
pub fn dot(a: P3, b: P3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Length of `a`.
#[must_use]
pub fn len(a: P3) -> f32 {
    dot(a, a).sqrt()
}

/// Distance between two points.
#[must_use]
pub fn dist(a: P3, b: P3) -> f32 {
    len(sub(a, b))
}

/// Unit vector, or `None` for a zero or non-finite input.
#[must_use]
pub fn unit(a: P3) -> Option<P3> {
    let l = len(a);
    (l.is_finite() && l > 1e-9).then(|| scale(a, 1.0 / l))
}

/// True when every component is finite.
#[must_use]
pub fn finite(a: P3) -> bool {
    a.iter().all(|v| v.is_finite())
}

/// Where world point `p` lands for `camera`: NDC `[x, y]` (`-1..=1` inside
/// the picture, y up) and the depth in front of the eye. `None` for a point
/// behind the near plane or an unusable camera. The same maths as
/// `view_truth::screen_rect`, for one point.
#[must_use]
pub fn project(camera: &Camera, p: P3) -> Option<([f32; 2], f32)> {
    let view = camera.view_matrix().ok()?;
    let half = (camera.fov_y * 0.5).tan();
    let mut at = camera.eye;
    (at.x, at.y, at.z) = (p[0], p[1], p[2]);
    let local = view.transform_point3(at);
    let depth = -local.z;
    if !(depth > camera.near && half > 0.0 && camera.aspect > 0.0 && depth.is_finite()) {
        return None;
    }
    Some((
        [
            local.x / (depth * half * camera.aspect),
            local.y / (depth * half),
        ],
        depth,
    ))
}

/// True when an NDC point is inside the picture.
#[must_use]
pub fn in_picture(ndc: [f32; 2]) -> bool {
    ndc[0].abs() <= 1.0 && ndc[1].abs() <= 1.0
}

/// Parameter `t` in `0..=1` where the segment `from..to` first enters the
/// box, or `None` when it misses. A segment that starts inside the box
/// reports `Some(0.0)`.
#[must_use]
pub fn seg_box(from: P3, to: P3, min: P3, max: P3) -> Option<f32> {
    if !(finite(min) && finite(max) && finite(from) && finite(to)) {
        return None;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        let delta = to[axis] - from[axis];
        if delta.abs() < 1e-9 {
            if from[axis] < min[axis] || from[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = (
            (min[axis] - from[axis]) / delta,
            (max[axis] - from[axis]) / delta,
        );
        lo = lo.max(a.min(b));
        hi = hi.min(a.max(b));
        if lo > hi {
            return None;
        }
    }
    Some(lo)
}

/// True when `p` is strictly inside the box.
#[must_use]
pub fn inside(p: P3, min: P3, max: P3) -> bool {
    (0..3).all(|k| min[k] < p[k] && p[k] < max[k])
}

/// Distance from `p` to the box; zero inside.
#[must_use]
pub fn point_box(p: P3, min: P3, max: P3) -> f32 {
    let d: P3 = std::array::from_fn(|k| (min[k] - p[k]).max(0.0).max(p[k] - max[k]));
    len(d)
}

/// Möller-Trumbore on both winding sides: the segment `from..to` against one
/// triangle, `t` in `0..=1` along the segment.
#[must_use]
pub fn seg_tri(from: P3, to: P3, tri: [P3; 3]) -> Option<f32> {
    let cross = |a: P3, b: P3| -> P3 {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dir = sub(to, from);
    let e1 = sub(tri[1], tri[0]);
    let e2 = sub(tri[2], tri[0]);
    let p = cross(dir, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(from, tri[0]);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(dir, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot(e2, q) * inv;
    (t.is_finite() && (0.0..=1.0).contains(&t)).then_some(t)
}

/// Closest approach of the segment `from..to` to the segment `a..b`: the
/// distance and the parameter along `from..to` where it happens.
#[must_use]
pub fn seg_seg(from: P3, to: P3, a: P3, b: P3) -> (f32, f32) {
    let d1 = sub(to, from);
    let d2 = sub(b, a);
    let r = sub(from, a);
    let (aa, ee, ff) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
    let (mut s, mut t);
    if aa <= 1e-12 && ee <= 1e-12 {
        return (dist(from, a), 0.0);
    }
    if aa <= 1e-12 {
        s = 0.0;
        t = (ff / ee).clamp(0.0, 1.0);
    } else {
        let c = dot(d1, r);
        if ee <= 1e-12 {
            t = 0.0;
            s = (-c / aa).clamp(0.0, 1.0);
        } else {
            let bb = dot(d1, d2);
            let denom = aa * ee - bb * bb;
            s = if denom > 1e-12 {
                ((bb * ff - c * ee) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            t = (bb * s + ff) / ee;
            if t < 0.0 {
                t = 0.0;
                s = (-c / aa).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((bb - c) / aa).clamp(0.0, 1.0);
            }
        }
    }
    let p1 = add(from, scale(d1, s));
    let p2 = add(a, scale(d2, t));
    (dist(p1, p2), s)
}

/// Distance from `p` to the segment `a..b`.
#[must_use]
pub fn point_seg(p: P3, a: P3, b: P3) -> f32 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 <= 1e-12 {
        return dist(p, a);
    }
    let t = (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0);
    dist(p, add(a, scale(ab, t)))
}

/// The eight corners of a box, x fastest.
#[must_use]
pub fn corners(min: P3, max: P3) -> [P3; 8] {
    std::array::from_fn(|i| {
        [
            if i & 1 == 0 { min[0] } else { max[0] },
            if i & 2 == 0 { min[1] } else { max[1] },
            if i & 4 == 0 { min[2] } else { max[2] },
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_enters_a_box_at_the_right_parameter_and_misses_beside_it() {
        let hit = seg_box([0.0, 1.0, -5.0], [0.0, 1.0, 5.0], [-1.0; 3], [1.0; 3]);
        assert!((hit.expect("hits") - 0.4).abs() < 1e-5, "{hit:?}");
        assert_eq!(
            seg_box([3.0, 1.0, -5.0], [3.0, 1.0, 5.0], [-1.0; 3], [1.0; 3]),
            None
        );
        assert_eq!(
            seg_box([0.0, 0.0, 0.0], [0.0, 0.0, 5.0], [-1.0; 3], [1.0; 3]),
            Some(0.0),
            "a start inside the box enters at zero"
        );
        assert_eq!(seg_box([f32::NAN; 3], [0.0; 3], [-1.0; 3], [1.0; 3]), None);
    }

    #[test]
    fn a_segment_hits_a_triangle_from_both_sides_and_stops_short() {
        let tri = [[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        assert!(seg_tri([0.0, 0.5, 2.0], [0.0, 0.5, -2.0], tri).is_some());
        assert!(seg_tri([0.0, 0.5, -2.0], [0.0, 0.5, 2.0], tri).is_some());
        assert_eq!(
            seg_tri([0.0, 0.5, 2.0], [0.0, 0.5, 1.0], tri),
            None,
            "short"
        );
    }

    #[test]
    fn segment_distances_find_the_gap_and_the_parameter() {
        let (gap, at) = seg_seg(
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [5.0, 1.0, -3.0],
            [5.0, 1.0, 3.0],
        );
        assert!(
            (gap - 1.0).abs() < 1e-5 && (at - 0.5).abs() < 1e-5,
            "{gap} {at}"
        );
        assert!((point_seg([0.0, 2.0, 0.0], [-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]) - 2.0).abs() < 1e-6);
        assert!((point_box([3.0, 0.0, 0.0], [-1.0; 3], [1.0; 3]) - 2.0).abs() < 1e-6);
        assert_eq!(point_box([0.0; 3], [-1.0; 3], [1.0; 3]), 0.0);
    }

    #[test]
    fn a_point_ahead_projects_inside_and_one_behind_does_not() {
        let camera = Camera::new([0.0, 1.5, 6.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0);
        let (ndc, depth) = project(&camera, [0.0, 1.0, 0.0]).expect("ahead");
        assert!(in_picture(ndc) && depth > 5.0, "{ndc:?} {depth}");
        assert!(project(&camera, [0.0, 1.0, 12.0]).is_none(), "behind");
    }
}
