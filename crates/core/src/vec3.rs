//! vec3: the one 3-vector cross product, squared distance and integer 2D
//! orientation shared by animation, physics, nav, playtest and render code.
//! Vectors are `[x, y, z]` arrays of `f32`. The operation order is fixed
//! here, so every caller gets the same bits: no fused multiply-add and no
//! reordering, ever.

/// Cross product `a x b` of two `[x, y, z]` vectors.
///
/// Units: the output has the product of the input units.
/// Determinism: fixed `*` and `-` order, no allocation.
#[inline]
#[must_use]
pub fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Squared Euclidean distance between two `[x, y, z]` points.
///
/// Units: metres squared when the inputs are metres. Determinism: fixed
/// `*` and `+` order, no square root, no allocation.
#[inline]
#[must_use]
pub fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}

/// 2D cross product `(b - o) x (c - o)` of integer points: positive when `c`
/// lies left of the ray `o -> b`, zero when the three are collinear.
///
/// Units: the product of the input units. Determinism: integer only, no
/// allocation; exact in `i64` while every coordinate stays below 2^30.
#[inline]
#[must_use]
pub fn orient2(o: (i32, i32), b: (i32, i32), c: (i32, i32)) -> i64 {
    (i64::from(b.0) - i64::from(o.0)) * (i64::from(c.1) - i64::from(o.1))
        - (i64::from(b.1) - i64::from(o.1)) * (i64::from(c.0) - i64::from(o.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orient2_signs_left_right_and_collinear() {
        assert!(orient2((0, 0), (10, 0), (5, 5)) > 0);
        assert!(orient2((0, 0), (10, 0), (5, -5)) < 0);
        assert_eq!(orient2((0, 0), (10, 10), (-3, -3)), 0);
    }

    #[test]
    fn orient2_is_exact_near_the_coordinate_bound() {
        let big = 1 << 29;
        assert_eq!(
            orient2((-big, -big), (big, -big), (big, big)),
            4 * i64::from(big) * i64::from(big)
        );
    }

    #[test]
    fn cross3_takes_each_component_exactly() {
        // (2*6 - 3*5, 3*4 - 1*6, 1*5 - 2*4): every product and difference is distinct.
        assert_eq!(cross3([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), [-3.0, 6.0, -3.0]);
    }

    #[test]
    fn dist2_sums_each_squared_difference() {
        // Differences -3, -4, -6: 9 + 16 + 36.
        assert_eq!(dist2([1.0, 2.0, 3.0], [4.0, 6.0, 9.0]), 61.0);
    }

    #[test]
    fn orient2_subtracts_the_origin_from_every_point() {
        // Off the origin the differences are not the raw coordinates:
        // (3, 4) x (2, 7) = 21 - 8.
        assert_eq!(orient2((1, 2), (4, 6), (3, 9)), 13);
    }
}
