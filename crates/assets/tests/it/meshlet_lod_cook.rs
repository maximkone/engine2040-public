//! VG2 spike: the LOD DAG halves triangles within error, caps clusters,
//! repeats x2 and stays KTX2-only.
//!
//! Owner: `engine-assets`; consumer: the mesh cook proving arena meshes
//! land clusters plus a DAG inside the error bound. Units: counts in,
//! cooked levels out. Determinism: fixed grids only, so reruns agree.

use engine_assets::import::{validate_ktx2, KTX2_MAGIC};
use engine_assets::meshlet::{
    cook_meshlet_lod, MeshletError, LOD_MAX_ERROR, MAX_MESHLET_TRIANGLES, MAX_MESHLET_VERTICES,
};

/// Flat grid positions plus row-major quad indices for `cells` cells.
fn grid(cells: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
    let side = cells + 1;
    let mut positions = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            positions.push([x as f32, y as f32, 0.0]);
        }
    }
    let mut indices = Vec::new();
    for y in 0..cells {
        for x in 0..cells {
            let a = (y * side + x) as u32;
            let b = (y * side + x + 1) as u32;
            let c = ((y + 1) * side + x) as u32;
            let d = ((y + 1) * side + x + 1) as u32;
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    (positions, indices)
}

/// Smallest valid KTX2: magic plus a 64x64 4-level header.
fn ktx2() -> Vec<u8> {
    let mut bytes = vec![0_u8; 80];
    bytes[0..12].copy_from_slice(&KTX2_MAGIC);
    bytes[20..24].copy_from_slice(&64_u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&64_u32.to_le_bytes());
    bytes[36..40].copy_from_slice(&4_u32.to_le_bytes());
    bytes
}

#[test]
fn lod_dag_halves_triangles_caps_clusters_and_repeats_x2() {
    let (positions, indices) = grid(16);
    let lod0_tris = indices.len() / 3;
    let tex = ktx2();
    assert!(validate_ktx2("arena/albedo.ktx2", &tex).is_ok());
    let textures = [("arena/albedo.ktx2", tex.as_slice())];
    let first =
        cook_meshlet_lod(&positions, &indices, &textures, LOD_MAX_ERROR).expect("grid cooks");
    let second =
        cook_meshlet_lod(&positions, &indices, &textures, LOD_MAX_ERROR).expect("grid recooks");
    assert_eq!(first, second, "cook x2 identical");
    assert_eq!(first.levels.len(), 4, "one level per LOD ratio");
    let lod1 = &first.levels[1];
    assert!(
        lod1.indices.len() * 2 <= indices.len(),
        "LOD1 index length at most half"
    );
    assert!(
        lod1.indices.len() / 3 * 100 <= lod0_tris * 55,
        "LOD1 keeps at most 55pct"
    );
    assert!(lod1.error <= LOD_MAX_ERROR, "LOD1 inside the error bound");
    for level in &first.levels {
        let mut covered = 0_usize;
        for meshlet in &level.meshlets {
            assert!(meshlet.vertices.len() <= MAX_MESHLET_VERTICES);
            assert!(meshlet.triangles.len() <= MAX_MESHLET_TRIANGLES);
            covered += meshlet.triangles.len();
        }
        assert_eq!(
            covered,
            level.indices.len() / 3,
            "clusters cover every triangle"
        );
    }
}

#[test]
fn lod_cook_refuses_non_ktx2_and_blown_error_bound() {
    let (positions, indices) = grid(8);
    let tex = ktx2();
    let err = cook_meshlet_lod(
        &positions,
        &indices,
        &[("a.png", b"tiny".as_slice())],
        LOD_MAX_ERROR,
    )
    .expect_err("non-KTX2 refuses");
    assert!(matches!(err, MeshletError::NonKtx2 { .. }));
    assert!(validate_ktx2("a.png", b"tiny").is_err());
    let err = cook_meshlet_lod(&positions, &indices, &[("a.ktx2", tex.as_slice())], 0.0)
        .expect_err("zero bound refuses any collapse");
    assert!(matches!(err, MeshletError::ErrorOverBound { .. }));
}
