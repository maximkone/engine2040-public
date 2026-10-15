//! camera_arm: pillar-aware camera boom length, presentation only.
//!
//! Consumer: `crate::camera` (the fight director clamps its boom eye with
//! [`arm_length`] in a later hunk; this slice ships the pure sweep plus the
//! crate line). Data owner: this module owns nothing at runtime; callers pass
//! the pivot, direction, length cap, sphere radius and obstacle slice per
//! call. Units: meters. Determinism: pure function of the inputs, slice order
//! iteration with an order-independent minimum, same inputs give the same
//! length twice; never writes simulation state, never enters the tick hash.
//! Errors: total `f32` return, no `Result`; degenerate input (non-finite or
//! zero direction, non-positive or non-finite cap) returns the floor so the
//! frame never fails. Budget: O(boxes) time, zero heap bytes per call.

/// Shortest boom length in meters: a pivot inside an obstacle (or a
/// degenerate call) parks the eye here instead of inside geometry.
pub const MIN_ARM_M: f32 = 0.3;

/// Axis-aligned obstacle box in meters: `min` below `max` on every axis.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Aabb {
    /// Minimum corner in meters `[x, y, z]`.
    pub min: [f32; 3],
    /// Maximum corner in meters `[x, y, z]`.
    pub max: [f32; 3],
}

impl Aabb {
    /// Build a box from its corners; a badly ordered box (`min > max` on any
    /// axis) is kept as-is and skipped by [`arm_length`].
    #[must_use]
    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        Self { min, max }
    }

    /// True when every corner is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.min[0].is_finite()
            && self.min[1].is_finite()
            && self.min[2].is_finite()
            && self.max[0].is_finite()
            && self.max[1].is_finite()
            && self.max[2].is_finite()
    }

    /// True when `min` is at or below `max` on every axis.
    #[must_use]
    pub fn is_ordered(self) -> bool {
        self.min[0] <= self.max[0] && self.min[1] <= self.max[1] && self.min[2] <= self.max[2]
    }
}

/// Boom length in meters from `pivot` along `dir` (need not be unit): a sphere
/// sweep of `radius` against `boxes` inflated by that radius, slab-tested per
/// axis (Godot `SpringArm3D` shape). Returns the nearest hit distance, the
/// floor when the pivot starts inside an inflated box or the call is
/// degenerate, and `max_len` when nothing blocks the boom. Clamped to
/// `[MIN_ARM_M, max_len]`. Pure, allocates nothing, presentation only.
#[must_use]
pub fn arm_length(
    pivot: [f32; 3],
    dir: [f32; 3],
    max_len: f32,
    radius: f32,
    boxes: &[Aabb],
) -> f32 {
    if !pivot[0].is_finite() || !pivot[1].is_finite() || !pivot[2].is_finite() {
        return MIN_ARM_M;
    }
    if !dir[0].is_finite() || !dir[1].is_finite() || !dir[2].is_finite() {
        return MIN_ARM_M;
    }
    if !max_len.is_finite() || max_len <= 0.0 {
        return MIN_ARM_M;
    }
    let radius_m = if radius.is_finite() && radius > 0.0 {
        radius
    } else {
        0.0
    };
    let len_sq = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    if len_sq == 0.0 {
        return MIN_ARM_M;
    }
    let inv_len = 1.0 / len_sq.sqrt();
    let dir_m = [dir[0] * inv_len, dir[1] * inv_len, dir[2] * inv_len];
    let mut best = max_len;
    for bounds in boxes {
        if !bounds.is_finite() || !bounds.is_ordered() {
            continue;
        }
        let lo = [
            bounds.min[0] - radius_m,
            bounds.min[1] - radius_m,
            bounds.min[2] - radius_m,
        ];
        let hi = [
            bounds.max[0] + radius_m,
            bounds.max[1] + radius_m,
            bounds.max[2] + radius_m,
        ];
        let mut low_t = 0.0f32;
        let mut high_t = max_len;
        let mut missed = false;
        let mut i = 0;
        while i < 3 {
            let origin = pivot[i];
            let direction = dir_m[i];
            if direction == 0.0 {
                if origin < lo[i] || origin > hi[i] {
                    missed = true;
                    break;
                }
            } else {
                let mut near = (lo[i] - origin) / direction;
                let mut far = (hi[i] - origin) / direction;
                if near > far {
                    let swap = near;
                    near = far;
                    far = swap;
                }
                if near > low_t {
                    low_t = near;
                }
                if far < high_t {
                    high_t = far;
                }
                if low_t > high_t {
                    missed = true;
                    break;
                }
            }
            i += 1;
        }
        if missed {
            continue;
        }
        if low_t <= 0.0 {
            return MIN_ARM_M;
        }
        if low_t < best {
            best = low_t;
        }
    }
    best.max(MIN_ARM_M).min(max_len)
}

#[cfg(test)]
mod tests {
    use super::{arm_length, Aabb, MIN_ARM_M};

    fn pillar_at_face(face_m: f32) -> Aabb {
        Aabb::new([face_m, -5.0, -5.0], [face_m + 4.0, 5.0, 5.0])
    }

    #[test]
    fn camera_arm_blocks_at_inflated_face() {
        let boxes = [pillar_at_face(3.0)];
        let got = arm_length([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0, 0.3, &boxes);
        assert!((got - 2.7).abs() < 1e-4, "blocked at 2.7 m, got {got}");
    }

    #[test]
    fn camera_arm_open_space_returns_max() {
        let boxes: [Aabb; 0] = [];
        assert_eq!(
            arm_length([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 5.0, 0.3, &boxes),
            5.0
        );
    }

    #[test]
    fn camera_arm_pivot_inside_returns_floor() {
        let boxes = [Aabb::new([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0])];
        assert_eq!(
            arm_length([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0, 0.3, &boxes),
            MIN_ARM_M
        );
    }

    #[test]
    fn camera_arm_degenerate_direction_returns_floor() {
        let boxes: [Aabb; 0] = [];
        assert_eq!(
            arm_length([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 6.0, 0.3, &boxes),
            MIN_ARM_M
        );
    }

    #[test]
    fn camera_arm_hit_beyond_max_returns_max() {
        let boxes = [pillar_at_face(30.0)];
        assert_eq!(
            arm_length([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0, 0.3, &boxes),
            6.0
        );
    }

    #[test]
    fn camera_arm_same_inputs_give_same_length_twice() {
        let boxes = [
            pillar_at_face(3.0),
            Aabb::new([-9.0, -9.0, 4.0], [-5.0, 9.0, 9.0]),
        ];
        let first = arm_length([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0, 0.3, &boxes);
        let second = arm_length([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 6.0, 0.3, &boxes);
        assert_eq!(first, second);
    }
}
