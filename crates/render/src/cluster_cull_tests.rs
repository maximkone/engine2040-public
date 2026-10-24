//! Tests for [`super`]: the oracle against a brute-force per-triangle pass.

use super::{bake_cluster, cull_clusters, ClusterBounds, CullVerdict, CullView};
use crate::draw::Camera;

/// A tessellated sphere cut into the eight octant clusters a cook would make.
struct Fixture {
    positions: Vec<[f32; 3]>,
    /// Triangle index ranges, one entry per octant cluster.
    clusters: Vec<Vec<u32>>,
    bounds: Vec<ClusterBounds>,
}

fn sphere(rings: usize, segments: usize) -> Vec<[f32; 3]> {
    let mut positions = Vec::new();
    for ring in 0..=rings {
        let theta = std::f32::consts::PI * ring as f32 / rings as f32;
        for segment in 0..=segments {
            let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
            positions.push([
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            ]);
        }
    }
    positions
}

fn triangle_normal(positions: &[[f32; 3]], tri: &[u32]) -> Option<[f32; 3]> {
    let a = positions.get(usize::try_from(tri[0]).ok()?)?;
    let b = positions.get(usize::try_from(tri[1]).ok()?)?;
    let c = positions.get(usize::try_from(tri[2]).ok()?)?;
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !len.is_finite() || len <= 1e-12 {
        return None;
    }
    Some([n[0] / len, n[1] / len, n[2] / len])
}

fn fixture() -> Fixture {
    let rings = 8usize;
    let segments = 12usize;
    let positions = sphere(rings, segments);
    let stride = u32::try_from(segments + 1).expect("stride fits");
    let mut clusters: Vec<Vec<u32>> = vec![Vec::new(); 8];
    let octant = |positions: &[[f32; 3]], tri: &[u32]| -> usize {
        let mut sum = [0.0f32; 3];
        for index in tri {
            if let Some(point) = positions.get(usize::try_from(*index).unwrap_or(usize::MAX)) {
                for axis in 0..3 {
                    sum[axis] += point[axis] / 3.0;
                }
            }
        }
        usize::from(sum[0] > 0.0)
            | (usize::from(sum[1] > 0.0) << 1)
            | (usize::from(sum[2] > 0.0) << 2)
    };
    for ring in 0..u32::try_from(rings).expect("fits") {
        for segment in 0..u32::try_from(segments).expect("fits") {
            let a = ring * stride + segment;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            for tri in [[a, c, b], [b, c, d]] {
                let lane = octant(&positions, &tri);
                clusters[lane].extend_from_slice(&tri);
            }
        }
    }
    let clusters: Vec<Vec<u32>> = clusters.into_iter().filter(|t| !t.is_empty()).collect();
    let bounds: Vec<ClusterBounds> = clusters
        .iter()
        .map(|tri| bake_cluster(&positions, tri).expect("bakes"))
        .collect();
    Fixture {
        positions,
        clusters,
        bounds,
    }
}

fn camera(pos_m: [f32; 3], forward: [f32; 3]) -> Camera {
    Camera {
        pos_m,
        forward,
        up: [0.0, 1.0, 0.0],
        fov_y_rad: std::f32::consts::FRAC_PI_2,
        aspect: 16.0 / 9.0,
        near_m: 0.1,
        far_m: 100.0,
    }
}

/// Deterministic camera ring: 64 positions on a sphere of `radius`, each
/// looking at the origin. A small LCG keeps the sample fixed without a crate.
fn orbit_cameras(radius: f32, count: usize) -> Vec<Camera> {
    let mut state = 0x5EED_2040u32;
    let mut next = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let u = next() * 2.0 - 1.0;
        let phi = next() * std::f32::consts::TAU;
        let w = (1.0 - u * u).max(0.0).sqrt();
        let pos = [radius * w * phi.cos(), radius * u, radius * w * phi.sin()];
        let len = (pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt();
        let forward = [-pos[0] / len, -pos[1] / len, -pos[2] / len];
        out.push(camera(pos, forward));
    }
    out
}

/// Every triangle of one cluster is back-facing from `eye`.
fn all_triangles_face_away(positions: &[[f32; 3]], tri_indices: &[u32], eye: [f32; 3]) -> bool {
    for tri in tri_indices.chunks_exact(3) {
        let Some(normal) = triangle_normal(positions, tri) else {
            continue;
        };
        let mut centroid = [0.0f32; 3];
        for index in tri {
            if let Some(point) = positions.get(usize::try_from(*index).unwrap_or(usize::MAX)) {
                for axis in 0..3 {
                    centroid[axis] += point[axis] / 3.0;
                }
            }
        }
        let to_eye = [
            eye[0] - centroid[0],
            eye[1] - centroid[1],
            eye[2] - centroid[2],
        ];
        if normal[0] * to_eye[0] + normal[1] * to_eye[1] + normal[2] * to_eye[2] > 1e-5 {
            return false;
        }
    }
    true
}

#[test]
fn cluster_cull_oracle_never_rejects_a_front_facing_cluster() {
    let scene = fixture();
    assert_eq!(scene.clusters.len(), 8, "the fixture has eight octants");
    // The fixture's own winding must be outward, or the cull means nothing.
    for (lane, tri) in scene.clusters.iter().enumerate() {
        for chunk in tri.chunks_exact(3) {
            let normal = triangle_normal(&scene.positions, chunk).expect("wound");
            let mut centroid = [0.0f32; 3];
            for index in chunk {
                if let Some(p) = scene
                    .positions
                    .get(usize::try_from(*index).unwrap_or(usize::MAX))
                {
                    for axis in 0..3 {
                        centroid[axis] += p[axis] / 3.0;
                    }
                }
            }
            let outward =
                normal[0] * centroid[0] + normal[1] * centroid[1] + normal[2] * centroid[2];
            assert!(outward > 0.0, "cluster {lane} winds inward");
        }
    }
    let mut rejected_total = 0usize;
    for camera in orbit_cameras(3.0, 64) {
        let view = CullView::from_camera(&camera).expect("camera");
        for (lane, bounds) in scene.bounds.iter().enumerate() {
            if view.verdict(bounds) != CullVerdict::FacingAway {
                continue;
            }
            rejected_total += 1;
            assert!(
                all_triangles_face_away(&scene.positions, &scene.clusters[lane], view.eye_m()),
                "cluster {lane} was cone-culled but a triangle faces the eye at {:?}",
                view.eye_m()
            );
        }
    }
    assert!(
        rejected_total > 0,
        "the cone must reject something in 64 orbit views"
    );
}

#[test]
fn cluster_cull_off_screen_camera_keeps_no_cluster() {
    let scene = fixture();
    // The sphere sits at the origin; this camera stands above it looking away.
    let camera = camera([0.0, 40.0, 0.0], [0.0, 1.0, 0.0]);
    let view = CullView::from_camera(&camera).expect("camera");
    let kept = cull_clusters(&view, &scene.bounds);
    assert!(
        kept.is_empty(),
        "kept {kept:?} with the arena behind the camera"
    );
    assert!(
        scene
            .bounds
            .iter()
            .all(|b| view.verdict(b) == CullVerdict::OutsideFrustum),
        "the frustum, not the cone, must remove them"
    );
}

#[test]
fn cluster_cull_backfacing_camera_keeps_at_most_a_quarter() {
    let scene = fixture();
    // Inside the sphere every octant faces outward, away from the eye.
    let camera = camera([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let view = CullView::from_camera(&camera).expect("camera");
    let kept = cull_clusters(&view, &scene.bounds);
    assert!(
        kept.len() <= scene.bounds.len() / 4,
        "kept {} of {} clusters from inside the sphere",
        kept.len(),
        scene.bounds.len()
    );
    for index in &kept {
        let bounds = &scene.bounds[usize::try_from(*index).unwrap_or(0)];
        assert!(
            view.verdict(bounds).is_visible(),
            "a kept cluster must be visible"
        );
    }
}

#[test]
fn cluster_cull_keeps_the_same_list_twice() {
    let scene = fixture();
    let mut cameras = orbit_cameras(3.0, 8);
    cameras.push(camera([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]));
    for camera in cameras {
        let view = CullView::from_camera(&camera).expect("camera");
        let first = cull_clusters(&view, &scene.bounds);
        let second = cull_clusters(&view, &scene.bounds);
        assert_eq!(first, second, "the kept list is byte-identical twice");
        let bytes = |list: &[u32]| {
            let mut out = Vec::with_capacity(list.len() * 4);
            for value in list {
                out.extend_from_slice(&value.to_le_bytes());
            }
            out
        };
        assert_eq!(
            bytes(&first),
            bytes(&second),
            "same bytes, not just same length"
        );
    }
}

#[test]
fn cluster_cull_bake_covers_every_vertex_and_normal() {
    let scene = fixture();
    for (lane, (bounds, tris)) in scene.bounds.iter().zip(&scene.clusters).enumerate() {
        let mut farthest = 0.0f32;
        let mut worst_angle = 1.0f32;
        for tri in tris.chunks_exact(3) {
            for index in tri {
                if let Some(point) = scene
                    .positions
                    .get(usize::try_from(*index).unwrap_or(usize::MAX))
                {
                    let d = [
                        point[0] - bounds.center_m[0],
                        point[1] - bounds.center_m[1],
                        point[2] - bounds.center_m[2],
                    ];
                    farthest = farthest.max((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt());
                }
            }
            if let Some(normal) = triangle_normal(&scene.positions, tri) {
                worst_angle = worst_angle.min(
                    bounds.cone_axis[0] * normal[0]
                        + bounds.cone_axis[1] * normal[1]
                        + bounds.cone_axis[2] * normal[2],
                );
            }
        }
        assert!(
            farthest <= bounds.radius_m + 1e-5,
            "cluster {lane} radius {} misses a vertex at {farthest} m",
            bounds.radius_m
        );
        assert!(
            worst_angle >= bounds.cone_cos - 1e-5,
            "cluster {lane} cone {} misses a normal at {worst_angle}",
            bounds.cone_cos
        );
    }
}
