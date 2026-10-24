//! cluster_cull: per-cluster bounds and a normal cone, so a cluster the camera
//! cannot see never reaches the vertex stage.
//!
//! A drawn mesh is all or nothing: culling happens at the mesh, so the back of a
//! tunic and the far side of a boulder still cost vertex work. A cluster (the
//! 64-vertex meshlet the cook already builds) carries a bounding sphere and a
//! normal cone — an axis and the cosine of its half angle — baked once from its
//! triangles. Two tests then reject it: the sphere lies fully outside one
//! frustum plane, or the cone plus the sphere turns away from the eye.
//!
//! The cone test is the exact support maximum of `cos * along + sin * perp +
//! radius` against the eye vector, so it can only refuse a cluster whose every
//! triangle already faces away; the oracle test below checks that against a
//! brute-force per-triangle pass over 64 cameras.
//!
//! Units: metres for centres, radii and axes, cosine for the half angle. Pure
//! data and arithmetic in a fixed order, so the same clusters and camera give
//! the same kept list twice. No allocation beyond the returned list. Caller:
//! the forward pass one day; the arena's kit still draws whole meshes.
//!
//! Note on layering: `engine-assets` also bakes meshlet bounds
//! (`assets::meshlet::MeshletBounds`, culled by `assets::meshlet_cull`). This
//! module is the render-side twin because `engine-assets` is only a dependency
//! of the render crate behind `v1`; the two must agree, and the oracle test
//! here is what pins the maths.

use crate::draw::{frustum_planes_packed, sphere_visible_cpu, Camera, DrawError};

/// One cluster's baked bounds: the sphere that holds it and the cone that holds
/// its face normals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterBounds {
    /// Sphere centre in metres (vertex centroid).
    pub center_m: [f32; 3],
    /// Sphere radius in metres, from the centre to the furthest vertex.
    pub radius_m: f32,
    /// Unit cone axis: the area-weighted mean face normal.
    pub cone_axis: [f32; 3],
    /// Cosine of the cone's half angle (`1.0` is a flat cluster).
    pub cone_cos: f32,
}

/// Why a cluster was kept out of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CullVerdict {
    /// The cluster may be seen: draw it.
    Visible,
    /// Its sphere lies fully outside one frustum plane.
    OutsideFrustum,
    /// Its cone turns away from the eye: every face is back-facing.
    FacingAway,
}

impl CullVerdict {
    /// True only for [`CullVerdict::Visible`].
    #[must_use]
    pub fn is_visible(self) -> bool {
        matches!(self, Self::Visible)
    }
}

/// One frame's cull inputs: the six packed frustum planes and the eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullView {
    planes: [[f32; 4]; 6],
    eye_m: [f32; 3],
}

impl CullView {
    /// The view of one camera, through the draw module's own plane build so the
    /// mesh cull and the cluster cull share one frustum.
    ///
    /// Errors: what `frustum_planes_packed` refuses (a degenerate camera).
    pub fn from_camera(camera: &Camera) -> Result<Self, DrawError> {
        Ok(Self {
            planes: frustum_planes_packed(camera)?,
            eye_m: camera.pos_m,
        })
    }

    /// Eye position in metres.
    #[must_use]
    pub fn eye_m(&self) -> [f32; 3] {
        self.eye_m
    }

    /// Verdict for one cluster: frustum first, then the cone.
    #[must_use]
    pub fn verdict(&self, bounds: &ClusterBounds) -> CullVerdict {
        let radius = bounds.radius_m.max(0.0);
        if !sphere_visible_cpu(bounds.center_m, radius, &self.planes) {
            return CullVerdict::OutsideFrustum;
        }
        if cone_culled(bounds, self.eye_m) {
            return CullVerdict::FacingAway;
        }
        CullVerdict::Visible
    }

    /// True when the cluster may be seen.
    #[must_use]
    pub fn visible(&self, bounds: &ClusterBounds) -> bool {
        self.verdict(bounds).is_visible()
    }
}

/// True when every face of the cluster turns away from `eye`.
///
/// The support maximum of the cone with the sphere: the cluster is invisible
/// only when its whole cone, widened by the sphere, sits behind the eye's
/// tangent plane. Non-finite bounds are kept, never culled.
#[must_use]
pub fn cone_culled(bounds: &ClusterBounds, eye: [f32; 3]) -> bool {
    let v = [
        eye[0] - bounds.center_m[0],
        eye[1] - bounds.center_m[1],
        eye[2] - bounds.center_m[2],
    ];
    let axis_len2 = bounds.cone_axis[0] * bounds.cone_axis[0]
        + bounds.cone_axis[1] * bounds.cone_axis[1]
        + bounds.cone_axis[2] * bounds.cone_axis[2];
    if !v.iter().all(|lane| lane.is_finite())
        || !axis_len2.is_finite()
        || axis_len2 <= 1e-12
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
    let along = axis[0] * v[0] + axis[1] * v[1] + axis[2] * v[2];
    let perp = [
        v[0] - axis[0] * along,
        v[1] - axis[1] * along,
        v[2] - axis[2] * along,
    ];
    let perp_len = (perp[0] * perp[0] + perp[1] * perp[1] + perp[2] * perp[2])
        .max(0.0)
        .sqrt();
    let support = cosine * along + sine * perp_len + bounds.radius_m.max(0.0);
    support < 0.0
}

/// Baked bounds for one cluster of `indices` into `positions`.
///
/// The sphere centre is the vertex centroid and the radius its furthest vertex,
/// so every point of the cluster is inside. The cone axis is the area-weighted
/// face-normal sum and `cone_cos` is the smallest `dot(axis, face normal)` over
/// the cluster's triangles, so every face normal is inside the cone. One
/// degenerate triangle is skipped; a cluster with no usable triangle, or with
/// no vertices, gives `None`.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the centroid is accumulated in f64 for stability and narrowed once to the f32 mesh units the bounds are published in"
)]
pub fn bake_cluster(positions: &[[f32; 3]], indices: &[u32]) -> Option<ClusterBounds> {
    let tri_count = indices.len() / 3;
    if tri_count == 0 {
        return None;
    }
    // Centre: centroid of the vertices the cluster names, each once.
    let mut used: Vec<u32> = Vec::new();
    for index in indices.iter().take(tri_count * 3) {
        if !used.contains(index) {
            used.push(*index);
        }
    }
    if used.is_empty() {
        return None;
    }
    let mut sum = [0.0f64; 3];
    for index in &used {
        let point = positions.get(usize::try_from(*index).ok()?)?;
        for axis in 0..3 {
            sum[axis] += f64::from(point[axis]);
        }
    }
    let count = used.len() as f64;
    let center = [
        (sum[0] / count) as f32,
        (sum[1] / count) as f32,
        (sum[2] / count) as f32,
    ];
    let mut radius = 0.0f32;
    for index in &used {
        let point = positions.get(usize::try_from(*index).ok()?)?;
        let d = [
            point[0] - center[0],
            point[1] - center[1],
            point[2] - center[2],
        ];
        radius = radius.max((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
    }
    // Axis: area-weighted normal sum.
    let mut axis = [0.0f64; 3];
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(tri_count);
    for tri in indices.chunks_exact(3) {
        let (Some(a), Some(b), Some(c)) = (
            positions.get(usize::try_from(tri[0]).ok()?)?,
            positions.get(usize::try_from(tri[1]).ok()?)?,
            positions.get(usize::try_from(tri[2]).ok()?)?,
        ) else {
            continue;
        };
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let len = (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        if !len.is_finite() || len <= 1e-12 {
            continue;
        }
        let normal = [cross[0] / len, cross[1] / len, cross[2] / len];
        normals.push(normal);
        for lane in 0..3 {
            axis[lane] += f64::from(cross[lane]);
        }
    }
    let axis_len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let cone_axis = if axis_len.is_finite() && axis_len > 1e-12 {
        [
            (axis[0] / axis_len) as f32,
            (axis[1] / axis_len) as f32,
            (axis[2] / axis_len) as f32,
        ]
    } else {
        // No usable face: an empty cone cannot cull anything.
        return Some(ClusterBounds {
            center_m: center,
            radius_m: radius,
            cone_axis: [0.0, 0.0, 1.0],
            cone_cos: -1.0,
        });
    };
    let mut cone_cos = 1.0f32;
    for normal in &normals {
        let dot = cone_axis[0] * normal[0] + cone_axis[1] * normal[1] + cone_axis[2] * normal[2];
        cone_cos = cone_cos.min(dot);
    }
    Some(ClusterBounds {
        center_m: center,
        radius_m: radius,
        cone_axis,
        cone_cos: cone_cos.clamp(-1.0, 1.0),
    })
}

/// Every cluster the view keeps, in input order. The list is deterministic: the
/// same bounds and camera give the same bytes twice.
#[must_use]
pub fn cull_clusters(view: &CullView, clusters: &[ClusterBounds]) -> Vec<u32> {
    let mut kept = Vec::with_capacity(clusters.len());
    for (index, bounds) in clusters.iter().enumerate() {
        if view.visible(bounds) {
            kept.push(u32::try_from(index).unwrap_or(u32::MAX));
        }
    }
    kept
}

#[cfg(test)]
#[path = "cluster_cull_tests.rs"]
mod tests;
