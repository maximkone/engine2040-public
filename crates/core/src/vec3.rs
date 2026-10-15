//! vec3: the one 3-vector cross product and squared distance shared by
//! animation, physics, playtest and render code. Vectors are `[x, y, z]`
//! arrays of `f32`. The operation order is fixed here, so every caller gets
//! the same bits: no fused multiply-add and no reordering, ever.

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
