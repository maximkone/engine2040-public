//! num: the speed demo's one copy of its small vector math and named number
//! conversions, so the movers, the cut and the scene share them instead of each
//! keeping its own. Presentation only. Units: whatever the caller's vectors carry.

pub(super) use engine_core::checked::saturating_usize_to_u32 as u32_of;
use engine_core::checked::{f32_from_u64, saturating_f64_to_u64};
pub(super) use engine_core::vec3::cross3 as cross;

pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(super) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(super) fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub(super) fn len(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
/// `a` at unit length, or `fallback` when `a` is too short to have a direction.
pub(super) fn unit_or(a: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let l = len(a);
    if l > 1e-9 {
        scale(a, 1.0 / l)
    } else {
        fallback
    }
}
/// `a` at unit length, +x when it has no direction.
pub(super) fn unit(a: [f32; 3]) -> [f32; 3] {
    unit_or(a, [1.0, 0.0, 0.0])
}
/// Rotate `v` by the unit quaternion `q` = [x, y, z, w].
pub(super) fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let t = scale(cross(u, v), 2.0);
    add(add(v, scale(t, q[3])), cross(u, t))
}
/// The unit quaternion turning `angle` radians about the unit `axis`.
pub(super) fn axis_quat(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let (s, c) = (angle * 0.5).sin_cos();
    [axis[0] * s, axis[1] * s, axis[2] * s, c]
}
/// The mean of `points`; the origin for none.
pub(super) fn centroid(points: &[[f32; 3]]) -> [f32; 3] {
    if points.is_empty() {
        return [0.0; 3];
    }
    let s = points.iter().fold([0.0; 3], |acc, p| add(acc, *p));
    scale(s, 1.0 / f32_of(points.len()))
}

/// A count or index as a float (exact below 2^24, which every demo count is).
pub(super) fn f32_of(n: usize) -> f32 {
    f32_from_u64(u64::try_from(n).unwrap_or(u64::MAX))
}
/// A mesh index as a slice index.
pub(super) fn idx(i: u32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}
/// The grid cell `x` falls in: its floor, 0 for negatives and NaN.
pub(super) fn floor_idx(x: f32) -> usize {
    usize::try_from(saturating_f64_to_u64(f64::from(x.floor()))).unwrap_or(usize::MAX)
}
/// The top 24 bits of a random word as a float in `[0, 1)`.
pub(super) fn unit_f32(word: u64) -> f32 {
    f32_from_u64(word >> 40) * (1.0 / 16_777_216.0)
}
/// `word` reduced into `0..n` (n > 0).
pub(super) fn below(word: u64, n: usize) -> usize {
    let n = u64::try_from(n.max(1)).unwrap_or(u64::MAX);
    usize::try_from(word % n).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conversions_hold_their_ranges() {
        assert_eq!(f32_of(3), 3.0);
        assert_eq!(idx(7), 7);
        assert_eq!(u32_of(9), 9);
        assert_eq!(floor_idx(2.9), 2);
        assert_eq!(floor_idx(-1.5), 0);
        assert_eq!(floor_idx(f32::NAN), 0);
        assert!((0.0..1.0).contains(&unit_f32(u64::MAX)));
        assert_eq!(below(10, 3), 1);
        assert_eq!(centroid(&[[0.0; 3], [2.0, 4.0, 6.0]]), [1.0, 2.0, 3.0]);
        assert_eq!(unit([0.0; 3]), [1.0, 0.0, 0.0]);
    }
}
