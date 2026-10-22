//! collide (HC9): the flyer and the car stop at building walls. A body that stepped into a
//! building box is put back on the face it came through and loses the part of its velocity
//! that pointed into the wall. The blade still cuts (it reaches past the body); this only
//! keeps the body itself out. Presentation only. Units: meters, m/s.

use super::num::{dot, scale, sub};

/// A box as `(min, max)` corners, meters.
pub(super) type Aabb = ([f32; 3], [f32; 3]);

/// Where a body ended up after meeting a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Contact {
    /// The corrected position, meters.
    pub pos: [f32; 3],
    /// The unit normal of the face it was put back on (points out of the building).
    pub normal: [f32; 3],
}

/// Push a body of `radius_m` at `pos`, which was at `prev` a moment ago, out of the first
/// box it overlaps. The face it leaves by is the one `prev` was outside of (the one it came
/// through), so a fast body is never thrown out the far side; a body that started inside
/// leaves by the nearest face. `None` when it overlaps nothing.
pub(super) fn push_out(
    prev: [f32; 3],
    pos: [f32; 3],
    radius_m: f32,
    boxes: &[Aabb],
) -> Option<Contact> {
    let mut at = pos;
    let mut hit: Option<[f32; 3]> = None;
    for (min, max) in boxes {
        if !(0..3).all(|a| at[a] > min[a] - radius_m && at[a] < max[a] + radius_m) {
            continue;
        }
        // Each axis: how far in, and the face to leave by (-1 low side, +1 high side).
        let mut best: Option<(f32, usize, f32)> = None;
        let mut best_any: Option<(f32, usize, f32)> = None;
        for a in 0..3 {
            let to_low = at[a] - (min[a] - radius_m);
            let to_high = (max[a] + radius_m) - at[a];
            let (depth, side) = if to_low < to_high {
                (to_low, -1.0)
            } else {
                (to_high, 1.0)
            };
            let came_low = prev[a] <= min[a] - radius_m;
            let came_high = prev[a] >= max[a] + radius_m;
            if (came_low || came_high) && best.is_none_or(|b| depth < b.0) {
                best = Some((depth, a, if came_low { -1.0 } else { 1.0 }));
            }
            if best_any.is_none_or(|b| depth < b.0) {
                best_any = Some((depth, a, side));
            }
        }
        let Some((_, axis, side)) = best.or(best_any) else {
            continue;
        };
        at[axis] = if side < 0.0 {
            min[axis] - radius_m
        } else {
            max[axis] + radius_m
        };
        let mut n = [0.0; 3];
        n[axis] = side;
        hit = Some(n);
    }
    hit.map(|normal| Contact { pos: at, normal })
}

/// `vel` without the part that points into a wall whose outward normal is `normal`.
pub(super) fn slide(vel: [f32; 3], normal: [f32; 3]) -> [f32; 3] {
    let into = dot(vel, normal).min(0.0);
    sub(vel, scale(normal, into))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WALL: Aabb = ([0.0, 0.0, 0.0], [20.0, 100.0, 20.0]);

    /// A body that stepped from the west into the wall is put back on the west face.
    #[test]
    fn a_body_stops_at_the_face_it_came_through() {
        let c = push_out([-5.0, 50.0, 10.0], [3.0, 50.0, 10.0], 1.5, &[WALL]).expect("hit");
        assert_eq!(c.normal, [-1.0, 0.0, 0.0]);
        assert!((c.pos[0] - (-1.5)).abs() < 1.0e-5, "{:?}", c.pos);
        assert_eq!(c.pos[1], 50.0);
    }

    /// Deep in (a fast step) it still leaves by the near side, never the far one.
    #[test]
    fn a_fast_body_is_not_thrown_out_the_far_side() {
        let c = push_out([-2.0, 50.0, 10.0], [17.0, 50.0, 10.0], 1.5, &[WALL]).expect("hit");
        assert_eq!(c.normal, [-1.0, 0.0, 0.0]);
    }

    /// Flying over the roof is not a hit; sinking onto it is put back on top.
    #[test]
    fn the_roof_is_a_floor() {
        assert!(push_out([10.0, 110.0, 10.0], [10.0, 105.0, 10.0], 1.5, &[WALL]).is_none());
        let c = push_out([10.0, 110.0, 10.0], [10.0, 99.0, 10.0], 1.5, &[WALL]).expect("hit");
        assert_eq!(c.normal, [0.0, 1.0, 0.0]);
        assert!((c.pos[1] - 101.5).abs() < 1.0e-5);
    }

    /// Clear air touches nothing, and sliding drops only the inward part.
    #[test]
    fn clear_air_is_free_and_sliding_keeps_the_rest() {
        assert!(push_out([-9.0, 5.0, 5.0], [-8.0, 5.0, 5.0], 1.5, &[WALL]).is_none());
        let v = slide([30.0, 4.0, -2.0], [-1.0, 0.0, 0.0]);
        assert_eq!(v, [0.0, 4.0, -2.0]);
        assert_eq!(
            slide([-30.0, 0.0, 0.0], [-1.0, 0.0, 0.0]),
            [-30.0, 0.0, 0.0]
        );
    }
}
