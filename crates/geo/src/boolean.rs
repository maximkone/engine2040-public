//! Mesh booleans on `manifold-rust` (pure Rust port of the Manifold kernel, Apache-2.0).
//!
//! Inputs may carry split vertices (UV seams): they are welded by exact position first, since
//! the kernel wants one vertex per corner of a closed 2-manifold. Outputs carry positions only:
//! a cut creates faces that have no UVs, so the caller unwraps the result afterwards (`uv`).
//! Port note: no donor code is copied; the crate is a dependency (Apache-2.0).

use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, MeshGL64};

use crate::mesh::{narrow, GeoError, GeoMesh};

/// Which boolean to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    /// A or B.
    Union,
    /// A minus B.
    Difference,
    /// A and B.
    Intersection,
}

fn to_manifold(mesh: &GeoMesh) -> Result<Manifold, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let welded = mesh.welded();
    if !welded.edge_report().is_closed() {
        return Err(GeoError::NotClosed);
    }
    let mut gl = MeshGL64 {
        num_prop: 3,
        ..MeshGL64::default()
    };
    for p in &welded.positions {
        gl.vert_properties.extend(p.map(f64::from));
    }
    gl.tri_verts = welded.indices.iter().map(|&i| u64::from(i)).collect();
    let manifold = Manifold::from_mesh_gl64(&gl);
    match manifold.status() {
        Error::NoError => Ok(manifold),
        other => Err(GeoError::Donor(format!("input mesh rejected: {other:?}"))),
    }
}

fn from_manifold(manifold: &Manifold) -> Result<GeoMesh, GeoError> {
    match manifold.status() {
        Error::NoError => {}
        other => return Err(GeoError::Donor(format!("boolean failed: {other:?}"))),
    }
    let gl = manifold.get_mesh_gl64(-1);
    let stride = usize::try_from(gl.num_prop).unwrap_or(3).max(3);
    let positions: Vec<[f32; 3]> = gl
        .vert_properties
        .chunks_exact(stride)
        .map(|c| [narrow(c[0]), narrow(c[1]), narrow(c[2])])
        .collect();
    let indices = gl
        .tri_verts
        .iter()
        .map(|&i| u32::try_from(i).unwrap_or(u32::MAX))
        .collect();
    Ok(GeoMesh::from_positions(positions, indices))
}

/// Run a boolean of two closed meshes. The result is closed, outward, welded, positions only;
/// an empty result (disjoint intersection, A inside B for a difference) returns `GeoError::Empty`.
pub fn boolean(a: &GeoMesh, b: &GeoMesh, op: BoolOp) -> Result<GeoMesh, GeoError> {
    let (ma, mb) = (to_manifold(a)?, to_manifold(b)?);
    let result = match op {
        BoolOp::Union => ma.union(&mb),
        BoolOp::Difference => ma.difference(&mb),
        BoolOp::Intersection => ma.intersection(&mb),
    };
    let mesh = from_manifold(&result)?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid, cylinder, l_prism, torus, uv_sphere};
    use engine_core::Rng;

    /// Analytic volume of the overlap of two axis-aligned boxes.
    fn box_overlap(a: ([f32; 3], [f32; 3]), b: ([f32; 3], [f32; 3])) -> f64 {
        (0..3)
            .map(|k| f64::from((a.1[k].min(b.1[k]) - a.0[k].max(b.0[k])).max(0.0)))
            .product()
    }

    #[test]
    fn box_pairs_match_analytic_volume() {
        // Six pairs of offset boxes: union, difference and intersection volumes are exact.
        let offsets = [
            [0.5, 0.0, 0.0],
            [0.5, 0.5, 0.0],
            [0.25, 0.75, 0.5],
            [1.5, 0.0, 0.0],
            [0.0, 0.0, 0.9],
            [0.2, 0.2, 0.2],
        ];
        for off in offsets {
            let a = cuboid([2.0, 2.0, 2.0]);
            let b = cuboid([2.0, 2.0, 2.0]).placed([1.0; 3], 0, off);
            let overlap = box_overlap(a.bounds(), b.bounds());
            let union = boolean(&a, &b, BoolOp::Union).unwrap();
            let diff = boolean(&a, &b, BoolOp::Difference).unwrap();
            assert!(
                (union.volume() - (16.0 - overlap)).abs() < 1e-4 * 16.0,
                "union {off:?}"
            );
            assert!(
                (diff.volume() - (8.0 - overlap)).abs() < 1e-4 * 8.0,
                "diff {off:?}"
            );
            for r in [&union, &diff] {
                assert_eq!(r.open_edge_count(), 0, "closed {off:?}");
            }
            if overlap > 0.0 {
                let inter = boolean(&a, &b, BoolOp::Intersection).unwrap();
                assert!(
                    (inter.volume() - overlap).abs() < 1e-4 * 8.0,
                    "inter {off:?}"
                );
                assert_eq!(inter.open_edge_count(), 0);
            } else {
                assert_eq!(boolean(&a, &b, BoolOp::Intersection), Err(GeoError::Empty));
            }
        }
    }

    fn solid(kind: u64) -> GeoMesh {
        match kind % 5 {
            0 => cuboid([2.0, 1.5, 1.0]),
            1 => uv_sphere(1.0, 10, 14),
            2 => cylinder(0.8, 2.0, 18),
            3 => torus(1.0, 0.4, 20, 10),
            _ => l_prism(2.0, 1.0, 1.0),
        }
    }

    /// 20 seeded pairs of solids: every result is closed (each edge shared by exactly 2
    /// triangles) and the whole run repeats to the same digests.
    #[test]
    fn twenty_seeded_pairs_stay_closed_and_repeat() {
        let run = || -> Vec<u64> {
            let mut digests = Vec::new();
            for seed in 0..20_u64 {
                let mut rng = Rng::new(0x6e0_0000 + seed);
                let a = solid(rng.next_u64());
                let scale = [
                    0.75 + rng.next_range(0, 6) as f32 * 0.25,
                    0.75 + rng.next_range(0, 6) as f32 * 0.25,
                    0.75 + rng.next_range(0, 6) as f32 * 0.25,
                ];
                let turns = rng.next_range(0, 4) as u32;
                let off = [
                    rng.next_range(0, 9) as f32 * 0.25 - 1.0,
                    rng.next_range(0, 9) as f32 * 0.25 - 1.0,
                    rng.next_range(0, 9) as f32 * 0.25 - 1.0,
                ];
                let b = solid(rng.next_u64()).placed(scale, turns, off);
                for op in [BoolOp::Union, BoolOp::Difference] {
                    let r =
                        boolean(&a, &b, op).unwrap_or_else(|e| panic!("seed {seed} {op:?}: {e}"));
                    r.validate().unwrap();
                    assert_eq!(r.open_edge_count(), 0, "seed {seed} {op:?} open edges");
                    assert!(r.volume() > 0.0, "seed {seed} {op:?} volume");
                    digests.push(r.digest());
                }
            }
            digests
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn open_or_malformed_input_is_refused() {
        let mut open = cuboid([1.0; 3]);
        open.indices.truncate(30);
        let b = cuboid([1.0; 3]);
        assert_eq!(boolean(&open, &b, BoolOp::Union), Err(GeoError::NotClosed));
        let empty = GeoMesh::default();
        assert_eq!(boolean(&empty, &b, BoolOp::Union), Err(GeoError::Empty));
        let mut bad = cuboid([1.0; 3]);
        bad.indices[0] = 99;
        assert!(matches!(
            boolean(&bad, &b, BoolOp::Union),
            Err(GeoError::Malformed(_))
        ));
    }

    #[test]
    fn seamed_input_is_welded_first() {
        // A 24-vertex box (UV seams on every edge) booleans like the 8-vertex one.
        let seamed = crate::primitives::cuboid_faces([2.0; 3]);
        let b = cuboid([2.0; 3]).placed([1.0; 3], 0, [1.0, 0.0, 0.0]);
        let r = boolean(&seamed, &b, BoolOp::Union).unwrap();
        assert!((r.volume() - 12.0).abs() < 1e-4);
        assert_eq!(r.open_edge_count(), 0);
    }
}
