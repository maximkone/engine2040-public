//! Meshlet bound vectors: count lower bounds plus cap checks.
//!
//! Owner: `engine-assets`; consumer: the mesh cook keeps clusters under
//! the 64v/124t limits. Units: counts in, meshlet lists out. Determinism:
//! fixed grids and arithmetic only, so reruns agree. Errors: cook errors
//! surface through `expect` with the invariant named.
//!
//! Own-code bounds derived from in-tree `meshlet.rs` limits; no donor
//! bytes. Prior MeshForge lead stayed UNVERIFIED and is rejected here.

use engine_assets::meshlet::{
    build_meshlets, build_meshlets_naive, MAX_MESHLET_TRIANGLES, MAX_MESHLET_VERTICES,
};

/// Fewest meshlets that can hold `tri_count` triangles when every cluster
/// packs to its triangle limit.
fn lower_bound(tri_count: usize) -> usize {
    tri_count.div_ceil(MAX_MESHLET_TRIANGLES)
}

/// Flat grid positions plus row-major quad indices for `cells` cells.
fn grid(cells: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
    let side = cells.saturating_add(1);
    let mut positions = Vec::with_capacity(side.saturating_mul(side));
    for y in 0..side {
        for x in 0..side {
            positions.push([x as f32, y as f32, 0.0]);
        }
    }
    let at = |x: usize, y: usize| u32::try_from(y.saturating_mul(side).saturating_add(x));
    let mut indices = Vec::new();
    for y in 0..cells {
        for x in 0..cells {
            let a = at(x, y).expect("grid vertex fits u32");
            let b = at(x.saturating_add(1), y).expect("grid vertex fits u32");
            let c = at(x, y.saturating_add(1)).expect("grid vertex fits u32");
            let d = at(x.saturating_add(1), y.saturating_add(1)).expect("grid vertex fits u32");
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    (positions, indices)
}

/// Disjoint triangles with no shared vertices; vertex pressure dominates.
fn disjoint_tris(count: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = Vec::with_capacity(count.saturating_mul(3));
    let mut indices = Vec::with_capacity(count.saturating_mul(3));
    for tri in 0..count {
        let base = u32::try_from(tri.saturating_mul(3)).expect("disjoint index fits u32");
        let origin = tri as f32;
        positions.push([origin, 0.0, 0.0]);
        positions.push([origin + 1.0, 0.0, 0.0]);
        positions.push([origin, 1.0, 0.0]);
        indices.extend_from_slice(&[base, base.saturating_add(1), base.saturating_add(2)]);
    }
    (positions, indices)
}

/// Every cluster respects both caps and covers exactly its input.
fn assert_caps_and_cover(positions: &[[f32; 3]], indices: &[u32], tri_count: usize, label: &str) {
    for meshlets in [
        build_meshlets_naive(positions, indices).expect("naive cook accepts valid mesh"),
        build_meshlets(positions, indices).expect("cone cook accepts valid mesh"),
    ] {
        assert!(!meshlets.is_empty(), "{label} emits at least one meshlet");
        assert!(
            meshlets.len() >= lower_bound(tri_count),
            "{label} stays above its lower bound"
        );
        let mut covered = 0_usize;
        for meshlet in &meshlets {
            assert!(
                meshlet.vertices.len() <= MAX_MESHLET_VERTICES,
                "{label} caps vertices"
            );
            assert!(
                meshlet.triangles.len() <= MAX_MESHLET_TRIANGLES,
                "{label} caps triangles"
            );
            assert!(
                !meshlet.triangles.is_empty(),
                "{label} keeps no empty meshlet"
            );
            covered = covered.saturating_add(meshlet.triangles.len());
        }
        assert_eq!(covered, tri_count, "{label} covers every triangle once");
    }
}

#[test]
fn lower_bound_vectors_match_ceil_arithmetic() {
    for (tris, want) in [(0_usize, 0_usize), (1, 1), (124, 1), (125, 2), (248, 2)] {
        assert_eq!(lower_bound(tris), want, "bound for {tris} triangles");
    }
}

#[test]
fn forty_k_tris_need_at_least_323_meshlets() {
    assert_eq!(lower_bound(40_000), 323, "40k-triangle lower bound");
}

#[test]
fn grid_clusters_respect_caps_and_cover() {
    let (positions, indices) = grid(12);
    let tri_count = indices.len() / 3;
    assert_caps_and_cover(&positions, &indices, tri_count, "grid");
}

#[test]
fn disjoint_clusters_respect_vertex_cap_and_cover() {
    let (positions, indices) = disjoint_tris(300);
    let tri_count = indices.len() / 3;
    assert_eq!(tri_count, 300, "disjoint fixture holds 300 triangles");
    assert_caps_and_cover(&positions, &indices, tri_count, "disjoint");
}
