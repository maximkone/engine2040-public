//! Collision decomposition: a concave mesh into a few closed convex parts, on the V-HACD code
//! inside `parry3d` (Apache-2.0; the same Rapier collision stack the sim already links).
//!
//! Every part is rebuilt as a real convex hull of its points, capped at `max_hull_vertices` by
//! farthest-point sampling (deterministic: fixed start, ties by lowest index), so a part is
//! always closed, outward and small enough for the physics bridge. `parry3d` runs with its
//! `enhanced-determinism` feature (libm math), the setting `rapier3d` uses in this workspace.

use parry3d::math::Vector;
use parry3d::transformation::try_convex_hull;
use parry3d::transformation::vhacd::{VHACDParameters, VHACD};

use crate::mesh::{ix, to_u64, GeoError, GeoMesh};

/// Options for one decomposition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullOptions {
    /// Most convex parts to return.
    pub max_hulls: u32,
    /// V-HACD concavity bound (0.01 tight, 0.05 loose), relative to the shape.
    pub concavity: f32,
    /// Voxels along the longest side.
    pub resolution: u32,
    /// Most vertices in one part.
    pub max_hull_vertices: usize,
}

impl Default for HullOptions {
    fn default() -> Self {
        Self {
            max_hulls: 16,
            concavity: 0.02,
            resolution: 64,
            max_hull_vertices: 64,
        }
    }
}

fn vec3(p: [f32; 3]) -> Vector {
    Vector::new(p[0], p[1], p[2])
}

/// Keep at most `max` of the points: the extreme point on the lowest axis first, then the point
/// farthest from everything chosen so far.
fn farthest_points(points: &[Vector], max: usize) -> Vec<Vector> {
    if points.len() <= max {
        return points.to_vec();
    }
    let mut first = 0;
    for (i, p) in points.iter().enumerate() {
        if p.x < points[first].x {
            first = i;
        }
    }
    let mut chosen = vec![first];
    let mut dist: Vec<f32> = points
        .iter()
        .map(|p| (*p - points[first]).length_squared())
        .collect();
    while chosen.len() < max {
        let mut best = 0;
        for (i, &d) in dist.iter().enumerate() {
            if d > dist[best] {
                best = i;
            }
        }
        if dist[best] <= 0.0 {
            break;
        }
        chosen.push(best);
        for (i, p) in points.iter().enumerate() {
            let d = (*p - points[best]).length_squared();
            if d < dist[i] {
                dist[i] = d;
            }
        }
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| points[i]).collect()
}

fn hull_mesh(points: &[Vector], max_vertices: usize) -> Option<GeoMesh> {
    let limited = farthest_points(points, max_vertices.max(4));
    let (verts, tris) = try_convex_hull(&limited).ok()?;
    let positions = verts.iter().map(|v| [v.x, v.y, v.z]).collect();
    let indices = tris.iter().flatten().copied().collect();
    let mesh = GeoMesh::from_positions(positions, indices);
    (mesh.triangle_count() >= 4).then_some(mesh)
}

/// The convex hull of the whole mesh (the single-hull baseline the decomposition improves on).
pub fn single_hull(mesh: &GeoMesh, max_hull_vertices: usize) -> Result<GeoMesh, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let points: Vec<Vector> = mesh.positions.iter().copied().map(vec3).collect();
    hull_mesh(&points, max_hull_vertices).ok_or_else(|| GeoError::Donor("degenerate hull".into()))
}

/// Decompose `mesh` into closed convex parts.
pub fn decompose(mesh: &GeoMesh, options: &HullOptions) -> Result<Vec<GeoMesh>, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let points: Vec<Vector> = mesh.positions.iter().copied().map(vec3).collect();
    let tris: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    let params = VHACDParameters {
        concavity: options.concavity,
        resolution: options.resolution.max(8),
        max_convex_hulls: options.max_hulls.max(1),
        convex_hull_approximation: false,
        ..VHACDParameters::default()
    };
    let vhacd = VHACD::decompose(&params, &points, &tris, true);
    let mut parts = Vec::new();
    for (verts, _) in vhacd.compute_exact_convex_hulls(&points, &tris) {
        if let Some(part) = hull_mesh(&verts, options.max_hull_vertices) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        // A shape V-HACD could not split (tiny, flat): one hull is still a valid collider.
        parts.push(single_hull(mesh, options.max_hull_vertices)?);
    }
    parts.truncate(ix(options.max_hulls.max(1)));
    Ok(parts)
}

/// Digest of a whole decomposition (part count, then each part's digest), the cook receipt.
#[must_use]
pub fn digest(parts: &[GeoMesh]) -> u64 {
    let mut h = engine_core::Fnv64::new();
    h.write_u64(to_u64(parts.len()));
    for p in parts {
        h.write_u64(p.digest());
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid, l_prism, torus};

    fn assert_part_ok(part: &GeoMesh, max_vertices: usize) {
        part.validate().unwrap();
        assert_eq!(part.open_edge_count(), 0, "part is closed");
        assert!(part.volume() > 0.0, "outward");
        assert!(part.positions.len() <= max_vertices);
    }

    #[test]
    fn l_prism_splits_into_fewer_than_one_hull_of_error() {
        let l = l_prism(2.0, 1.0, 1.0);
        let one = single_hull(&l, 64).unwrap();
        assert!(
            one.volume() > 3.4,
            "single hull fills the notch: {}",
            one.volume()
        );
        let parts = decompose(&l, &HullOptions::default()).unwrap();
        assert!(
            parts.len() >= 2 && parts.len() <= 16,
            "{} parts",
            parts.len()
        );
        let total: f64 = parts.iter().map(GeoMesh::volume).sum();
        // The parts overlap a little but hug the L much closer than one hull does.
        assert!(
            total < one.volume(),
            "parts {total} vs hull {}",
            one.volume()
        );
        assert!(total > 2.9, "parts cover the L: {total}");
        for p in &parts {
            assert_part_ok(p, 64);
        }
    }

    #[test]
    fn convex_shape_stays_one_part() {
        let parts = decompose(&cuboid([1.0, 2.0, 3.0]), &HullOptions::default()).unwrap();
        assert_eq!(parts.len(), 1);
        assert_part_ok(&parts[0], 64);
        assert!((parts[0].volume() - 6.0).abs() < 0.5);
    }

    #[test]
    fn torus_obeys_the_part_cap_and_repeats() {
        let t = torus(1.0, 0.35, 24, 10);
        let options = HullOptions {
            max_hulls: 8,
            ..HullOptions::default()
        };
        let a = decompose(&t, &options).unwrap();
        let b = decompose(&t, &options).unwrap();
        assert_eq!(digest(&a), digest(&b));
        assert!(a.len() > 1 && a.len() <= 8, "{} parts", a.len());
        for p in &a {
            assert_part_ok(p, 64);
        }
    }

    #[test]
    fn vertex_cap_is_enforced() {
        let t = torus(1.0, 0.35, 24, 10);
        let options = HullOptions {
            max_hull_vertices: 12,
            ..HullOptions::default()
        };
        for p in decompose(&t, &options).unwrap() {
            assert_part_ok(&p, 12);
        }
    }

    #[test]
    fn empty_and_malformed_inputs_are_refused() {
        assert_eq!(
            decompose(&GeoMesh::default(), &HullOptions::default()),
            Err(GeoError::Empty)
        );
        let mut bad = cuboid([1.0; 3]);
        bad.indices[2] = 77;
        assert!(matches!(
            decompose(&bad, &HullOptions::default()),
            Err(GeoError::Malformed(_))
        ));
    }
}
