//! The eight arena kit glTF files (`content/arena/src`) through every geo tool: the bars of
//! rows GEO2-GEO5 on real content. Needs feature `kit`; each test is also gated by its tool's feature.
#![cfg(feature = "kit")]

use engine_assets::mesh_import::import_gltf_path;
use engine_geo::GeoMesh;

const KIT: [&str; 8] = [
    "banner",
    "crate",
    "debris",
    "gate_tunnel",
    "sand_floor",
    "stands",
    "torch",
    "walls",
];

/// Every triangle primitive of every kit file, as `(file name, primitive index, mesh)`.
fn kit_meshes() -> Vec<(String, usize, GeoMesh)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena/src");
    let mut out = Vec::new();
    for name in KIT {
        let path = root.join(format!("{name}.gltf"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let import = import_gltf_path(&format!("{name}.gltf"), &bytes).unwrap();
        for (i, m) in import.meshes.iter().enumerate() {
            out.push((
                name.to_string(),
                i,
                GeoMesh {
                    positions: m.positions.clone(),
                    normals: m.normals.clone(),
                    uvs: m.uv0.clone(),
                    indices: m.indices.clone(),
                },
            ));
        }
    }
    out
}

#[test]
fn kit_loads_eight_files() {
    let kit = kit_meshes();
    let files: std::collections::BTreeSet<_> = kit.iter().map(|k| k.0.clone()).collect();
    assert_eq!(files.len(), 8);
    for (name, i, m) in &kit {
        m.validate().unwrap_or_else(|e| panic!("{name}/{i}: {e}"));
        println!(
            "kit {name}/{i}: {} verts {} tris closed={}",
            m.positions.len(),
            m.triangle_count(),
            m.edge_report().is_closed()
        );
    }
}

#[cfg(feature = "uv")]
#[test]
fn kit_unwrap_has_zero_overlapping_charts() {
    use engine_geo::uv::{overlapping_chart_pairs, unwrap, UnwrapOptions};
    let mut total_charts = 0;
    for (name, i, m) in kit_meshes() {
        let u = unwrap(&m, &UnwrapOptions::default()).unwrap_or_else(|e| panic!("{name}/{i}: {e}"));
        let overlaps = overlapping_chart_pairs(&u.mesh.uvs, &u.mesh.indices, &u.chart_of_triangle);
        println!(
            "uv {name}/{i}: {} charts, growth {:.2}, atlas {:?}, overlaps {overlaps}",
            u.chart_count,
            u.vertex_growth(m.positions.len()),
            u.atlas_size
        );
        assert_eq!(overlaps, 0, "{name}/{i}");
        assert!(u
            .mesh
            .uvs
            .iter()
            .flatten()
            .all(|&v| (0.0..=1.0).contains(&v)));
        total_charts += u.chart_count;
        // Same input, same bytes.
        let again = unwrap(&m, &UnwrapOptions::default()).unwrap();
        assert_eq!(u.mesh.digest(), again.mesh.digest(), "{name}/{i} repeat");
    }
    assert!(total_charts > 0);
}

#[cfg(feature = "lod")]
#[test]
fn kit_lod_keeps_every_seam_vertex() {
    use engine_geo::lod::{seam_vertices, simplify, LodOptions};
    let seam_keys = |m: &GeoMesh| -> Vec<([u32; 3], [u32; 2])> {
        let seams = seam_vertices(m, true, false);
        let mut keys: Vec<_> = (0..m.positions.len())
            .filter(|&i| seams[i])
            .map(|i| (m.positions[i].map(f32::to_bits), m.uvs[i].map(f32::to_bits)))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    };
    for (name, i, m) in kit_meshes() {
        let before = seam_keys(&m);
        let r = simplify(&m, &[], 0, 0.0, &LodOptions::default()).unwrap();
        let after = seam_keys(&r.mesh);
        let moved = before.iter().filter(|k| !after.contains(k)).count();
        println!(
            "lod {name}/{i}: {} -> {} tris, seams {} locked {} moved {moved}",
            r.triangles_before,
            r.triangles_after,
            before.len(),
            r.locked_vertices
        );
        assert_eq!(moved, 0, "{name}/{i} seam vertices moved");
        assert!(r.triangles_after <= r.triangles_before);
        let again = simplify(&m, &[], 0, 0.0, &LodOptions::default()).unwrap();
        assert_eq!(r.mesh.digest(), again.mesh.digest());
    }
}

#[cfg(feature = "lod")]
#[test]
fn engine_quadric_chain_moves_seam_vertices_the_control() {
    // The defect the lock removes: the engine's own `mesh_cook::simplify` averages UVs, so
    // seam vertices of the input are gone from its 50% level.
    use engine_assets::mesh_cook::simplify as old_simplify;
    use engine_assets::mesh_import::import_gltf_path;
    use engine_geo::lod::seam_vertices;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena/src");
    let mut moved_total = 0;
    for name in KIT {
        let bytes = std::fs::read(root.join(format!("{name}.gltf"))).unwrap();
        let import = import_gltf_path(&format!("{name}.gltf"), &bytes).unwrap();
        for m in &import.meshes {
            let geo = GeoMesh {
                positions: m.positions.clone(),
                normals: m.normals.clone(),
                uvs: m.uv0.clone(),
                indices: m.indices.clone(),
            };
            let seams = seam_vertices(&geo, true, false);
            let Ok(half) = old_simplify(m, (m.indices.len() / 6) as u32) else {
                continue;
            };
            let after: std::collections::BTreeSet<_> = half
                .positions
                .iter()
                .zip(&half.uv0)
                .map(|(p, u)| (p.map(f32::to_bits), u.map(f32::to_bits)))
                .collect();
            for (idx, _) in seams.iter().enumerate().filter(|(_, &s)| s) {
                let key = (
                    geo.positions[idx].map(f32::to_bits),
                    geo.uvs[idx].map(f32::to_bits),
                );
                if !after.contains(&key) {
                    moved_total += 1;
                }
            }
        }
    }
    println!("engine quadric chain: {moved_total} seam vertices moved or dropped at 50%");
}

#[cfg(feature = "collision")]
#[test]
fn kit_collision_has_at_most_16_closed_convex_parts() {
    use engine_geo::collision::{decompose, digest, HullOptions};
    for (name, i, m) in kit_meshes() {
        let parts =
            decompose(&m, &HullOptions::default()).unwrap_or_else(|e| panic!("{name}/{i}: {e}"));
        println!("hull {name}/{i}: {} parts", parts.len());
        assert!(
            !parts.is_empty() && parts.len() <= 16,
            "{name}/{i}: {} parts",
            parts.len()
        );
        for p in &parts {
            assert_eq!(p.open_edge_count(), 0, "{name}/{i} part open");
            assert!(p.volume() > 0.0);
        }
        let again = decompose(&m, &HullOptions::default()).unwrap();
        assert_eq!(digest(&parts), digest(&again));
    }
}

#[cfg(feature = "boolean")]
#[test]
fn kit_closed_meshes_survive_boolean_with_a_cutter() {
    use engine_geo::boolean::{boolean, BoolOp};
    use engine_geo::primitives::cuboid;
    let mut ran = 0;
    for (name, i, m) in kit_meshes() {
        if !m.welded().edge_report().is_closed() {
            continue;
        }
        let (lo, hi) = m.bounds();
        let centre = [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ];
        let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let cutter = cuboid([size[0] * 0.5, size[1] * 0.5, size[2] * 0.5]).placed(
            [1.0; 3],
            0,
            [
                centre[0] + size[0] * 0.3,
                centre[1] + size[1] * 0.3,
                centre[2],
            ],
        );
        for op in [BoolOp::Union, BoolOp::Difference] {
            let r = boolean(&m, &cutter, op).unwrap_or_else(|e| panic!("{name}/{i} {op:?}: {e}"));
            assert_eq!(r.open_edge_count(), 0, "{name}/{i} {op:?}");
            assert_eq!(r.digest(), boolean(&m, &cutter, op).unwrap().digest());
        }
        ran += 1;
    }
    println!("boolean ran on {ran} closed kit meshes");
}
