//! meshlet_cull: conservative meshlet visibility filter (frustum plus cone).
//!
//! Owner: `engine-assets`; consumer: the render meshlet path picks which
//! cooked clusters to submit. Units: meters. Determinism: fixed plane order
//! and `f32` arithmetic only, so the same bounds give the same verdict.
//! Errors: none, every input maps to a verdict.
//!
//! Conservative by construction: a meshlet is culled only when its bounding
//! sphere lies fully outside one frustum plane, or when every face normal of
//! its cone turns away from the eye. A visible meshlet is never culled (the
//! no-false-negative law). Bounds come from
//! [`crate::meshlet::MeshletBounds`], cooked by `meshlet_bounds`.
//!
//! The cone cutoff is the exact maximum of `dot(normal, eye - point)` over
//! the bounding sphere and the normal cone, so the test is tight, not merely
//! safe: with `v = eye - center`, `c` the cone cosine and `s = sqrt(1 - c^2)`
//! the maximum is `c * dot(axis, v) + s * |v_perp| + radius`.
//!
//! Cluster-cut idea adapted from meshoptimizer (Zeux, MIT,
//! `zeux/meshoptimizer` meshlet cone cull): sphere-plus-cone bounds against a
//! view. No vendored bytes; reimplemented here.

use crate::meshlet::MeshletBounds;

/// Squared length at or below which a vector counts as degenerate.
const DEGENERATE_LEN2: f32 = 1e-24;
/// One clip plane per view: left, right, bottom, top, near, far.
const PLANE_COUNT: usize = 6;

/// Dot product of two 3-vectors.
fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// One clip plane: `dot(normal, point) + offset >= 0` is inside.
///
/// `normal` is unit length after [`CullPlane::new`]. A degenerate or
/// non-finite plane accepts every point, so a broken view can never cull.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullPlane {
    /// Unit inward normal.
    pub normal: [f32; 3],
    /// Plane offset in meters.
    pub offset: f32,
}

impl CullPlane {
    /// Normalized plane from a normal plus offset.
    ///
    /// A zero, non-finite or sub-epsilon normal yields an accept-all plane.
    pub fn new(normal: [f32; 3], offset: f32) -> Self {
        let len2 = dot(&normal, &normal);
        if !len2.is_finite() || !offset.is_finite() || len2 <= DEGENERATE_LEN2 {
            return Self {
                normal: [0.0, 0.0, 0.0],
                offset: f32::INFINITY,
            };
        }
        let inv = 1.0 / len2.sqrt();
        Self {
            normal: [normal[0] * inv, normal[1] * inv, normal[2] * inv],
            offset: offset * inv,
        }
    }

    /// Signed distance: positive inside, negative outside, meters.
    pub fn signed_distance(&self, point: [f32; 3]) -> f32 {
        dot(&self.normal, &point) + self.offset
    }
}

/// Six clip planes of one view, in left, right, bottom, top, near, far order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    planes: [CullPlane; PLANE_COUNT],
}

impl Frustum {
    /// Frustum from six already-built planes.
    pub fn from_planes(planes: [CullPlane; PLANE_COUNT]) -> Self {
        Self { planes }
    }

    /// Frustum from a row-major view-projection matrix.
    ///
    /// Clip depth is the GL convention `-w <= z <= w`; row `i` is `m[i]` and
    /// a world point is `m * point`. Every plane is normalized.
    pub fn from_view_projection(m: &[[f32; 4]; 4]) -> Self {
        let w = m[3];
        let sum = |row: [f32; 4], sign: f32| -> [f32; 4] {
            [
                w[0] + sign * row[0],
                w[1] + sign * row[1],
                w[2] + sign * row[2],
                w[3] + sign * row[3],
            ]
        };
        let plane = |row: [f32; 4]| CullPlane::new([row[0], row[1], row[2]], row[3]);
        Self {
            planes: [
                plane(sum(m[0], 1.0)),
                plane(sum(m[0], -1.0)),
                plane(sum(m[1], 1.0)),
                plane(sum(m[1], -1.0)),
                plane(sum(m[2], 1.0)),
                plane(sum(m[2], -1.0)),
            ],
        }
    }

    /// True when the sphere touches or lies inside every plane.
    ///
    /// A straddling sphere is kept: only a fully outside sphere is refused.
    pub fn contains_sphere(&self, center: [f32; 3], radius: f32) -> bool {
        self.planes.iter().all(|plane| {
            let distance = plane.signed_distance(center);
            distance.is_nan() || distance >= -radius
        })
    }
}

/// Why the filter kept a meshlet out, or [`CullVerdict::Visible`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CullVerdict {
    /// The meshlet may be seen: submit it.
    Visible,
    /// The bounding sphere lies fully outside one frustum plane.
    OutsideFrustum,
    /// Every face of the cone turns away from the eye.
    FacingAway,
}

impl CullVerdict {
    /// True only for [`CullVerdict::Visible`].
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// True when every face of the meshlet turns away from `eye`.
///
/// The test is the exact cone-plus-sphere maximum, so it never culls a
/// front-facing cluster. Non-finite bounds are kept, never culled.
pub fn cone_culled(bounds: &MeshletBounds, eye: [f32; 3]) -> bool {
    let v = [
        eye[0] - bounds.center[0],
        eye[1] - bounds.center[1],
        eye[2] - bounds.center[2],
    ];
    let axis_len2 = dot(&bounds.cone_axis, &bounds.cone_axis);
    if !v.iter().all(|lane| lane.is_finite())
        || !axis_len2.is_finite()
        || axis_len2 <= DEGENERATE_LEN2
        || !bounds.cone_cos.is_finite()
    {
        return false;
    }
    let inv = 1.0 / axis_len2.sqrt();
    let axis = [
        bounds.cone_axis[0] * inv,
        bounds.cone_axis[1] * inv,
        bounds.cone_axis[2] * inv,
    ];
    let cosine = bounds.cone_cos.clamp(-1.0, 1.0);
    let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
    let along = dot(&axis, &v);
    let perp = [
        v[0] - axis[0] * along,
        v[1] - axis[1] * along,
        v[2] - axis[2] * along,
    ];
    let support =
        cosine * along + sine * dot(&perp, &perp).max(0.0).sqrt() + bounds.radius_m.max(0.0);
    support < 0.0
}

/// Frustum plus eye, the two inputs every cull test needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullFilter {
    /// View volume of the frame.
    pub frustum: Frustum,
    /// Eye position in world meters.
    pub eye: [f32; 3],
}

impl CullFilter {
    /// Filter for one view.
    pub fn new(frustum: Frustum, eye: [f32; 3]) -> Self {
        Self { frustum, eye }
    }

    /// Verdict for one meshlet: frustum first, then cone.
    pub fn verdict(&self, bounds: &MeshletBounds) -> CullVerdict {
        if !self
            .frustum
            .contains_sphere(bounds.center, bounds.radius_m.max(0.0))
        {
            return CullVerdict::OutsideFrustum;
        }
        if cone_culled(bounds, self.eye) {
            return CullVerdict::FacingAway;
        }
        CullVerdict::Visible
    }

    /// True when the meshlet may be seen.
    pub fn visible(&self, bounds: &MeshletBounds) -> bool {
        self.verdict(bounds).is_visible()
    }
}

/// One verdict per meshlet, input order kept.
pub fn cull_meshlets(filter: &CullFilter, bounds: &[MeshletBounds]) -> Vec<CullVerdict> {
    bounds.iter().map(|one| filter.verdict(one)).collect()
}
