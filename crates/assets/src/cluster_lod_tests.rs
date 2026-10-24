//! Tests for the cluster-LOD hierarchy: monotone errors, an exact cover at
//! every cut, and the same bytes twice.

use super::{build_clod_tree, ClodConfig, ClodTree};
use crate::cooked::CookedMesh;
use crate::meshlet::build_meshlets;

/// A wavy `n x n` quad grid in the XZ plane: simplification leaves the surface,
/// so a group's measured error is above zero and can be ordered.
fn wavy_grid(n: usize) -> CookedMesh {
    let side = n + 1;
    let mut positions = Vec::with_capacity(side * side);
    for row in 0..side {
        for col in 0..side {
            let x = col as f32 * 0.25;
            let z = row as f32 * 0.25;
            let y = 0.2 * (x * 1.5).sin() * (z * 1.5).cos();
            positions.push([x, y, z]);
        }
    }
    let mut normals = Vec::with_capacity(positions.len());
    let mut uv0 = Vec::with_capacity(positions.len());
    let mut tangents = Vec::with_capacity(positions.len());
    for row in 0..side {
        for col in 0..side {
            normals.push([0.0, 1.0, 0.0]);
            uv0.push([col as f32 / n as f32, row as f32 / n as f32]);
            tangents.push([1.0, 0.0, 0.0, 1.0]);
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6);
    for row in 0..n {
        for col in 0..n {
            let a = (row * side + col) as u32;
            let b = a + 1;
            let c = a + side as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    CookedMesh {
        positions,
        normals,
        uv0,
        tangents,
        indices,
    }
}

fn tree_of(mesh: &CookedMesh) -> ClodTree {
    build_clod_tree(mesh, ClodConfig::default()).expect("wavy grid builds a hierarchy")
}

#[test]
fn cluster_lod_error_never_decreases_up_the_tree() {
    let mesh = wavy_grid(16);
    let tree = tree_of(&mesh);
    assert!(tree.nodes.len() > 1, "the grid must merge into a hierarchy");
    for node in &tree.nodes {
        if let Some(parent) = node.parent {
            let parent = &tree.nodes[usize::try_from(parent).expect("parent id fits")];
            assert!(
                parent.bounds.error_m >= node.bounds.error_m,
                "parent error {} below child error {}",
                parent.bounds.error_m,
                node.bounds.error_m
            );
            assert!(
                parent.bounds.error_m.is_finite() && node.bounds.error_m.is_finite(),
                "an error must be a finite number of meters"
            );
        }
    }
    assert!(
        tree.nodes
            .iter()
            .any(|node| node.bounds.error_m > 0.0 && node.level > 0),
        "simplifying a wavy grid must record a positive error"
    );
}

#[test]
fn cluster_lod_cut_at_a_threshold_covers_every_triangle_once() {
    let mesh = wavy_grid(16);
    let tree = tree_of(&mesh);
    let total = u32::try_from(mesh.indices.len() / 3).expect("grid size fits");
    assert!(
        tree.cut(-1.0).is_empty(),
        "a threshold under every error draws nothing"
    );
    for error_m in [0.0f32, 0.0005, 0.01, 0.1, 1.0, f32::MAX] {
        let mut triangles = tree.cut_triangles(error_m);
        assert!(!triangles.is_empty(), "cut at {error_m} drew nothing");
        triangles.sort_unstable();
        let expected: Vec<u32> = (0..total).collect();
        assert_eq!(
            triangles, expected,
            "cut at {error_m} must cover every source triangle exactly once"
        );
    }
    assert!(
        tree.cut_drawn_triangles(f32::MAX) < tree.cut_drawn_triangles(0.0),
        "a loose threshold must draw fewer triangles than level 0"
    );
}

#[test]
fn cluster_lod_level_zero_is_the_meshlet_output() {
    let mesh = wavy_grid(16);
    let tree = tree_of(&mesh);
    let cooked = build_meshlets(&mesh.positions, &mesh.indices).expect("meshlets cook");
    let leaves: Vec<_> = tree.nodes.iter().filter(|node| node.level == 0).collect();
    assert_eq!(leaves.len(), cooked.len(), "one leaf per meshlet");
    let mut covered: Vec<u32> = leaves
        .iter()
        .flat_map(|node| node.triangles.clone())
        .collect();
    covered.sort_unstable();
    let expected: Vec<u32> = (0..u32::try_from(mesh.indices.len() / 3).expect("size")).collect();
    assert_eq!(covered, expected, "leaves partition every source triangle");
    assert!(tree.level_count() >= 2, "the grid must build a group level");
}

#[test]
fn cluster_lod_same_input_gives_equal_bytes() {
    let mesh = wavy_grid(12);
    let first = tree_of(&mesh);
    let second = tree_of(&mesh);
    assert_eq!(first, second, "two builds must match node for node");
    assert_eq!(
        first.to_bytes(),
        second.to_bytes(),
        "two builds must emit the same payload bytes"
    );
    assert!(first.to_bytes().starts_with(b"E2CLOD01"));
}

#[test]
fn cluster_lod_node_bounds_cover_their_own_triangles() {
    let mesh = wavy_grid(12);
    let tree = tree_of(&mesh);
    for (id, node) in tree.nodes.iter().enumerate() {
        for position in &node.positions {
            assert!(
                node.bounds.contains_point(*position),
                "node {id} bounds miss its own vertex {position:?}"
            );
        }
    }
}

#[test]
fn cluster_lod_projected_error_shrinks_with_distance() {
    let tree = tree_of(&wavy_grid(12));
    let group = tree
        .nodes
        .iter()
        .filter(|node| node.level > 0)
        .max_by(|a, b| a.bounds.error_m.total_cmp(&b.bounds.error_m))
        .expect("the grid has a group");
    let projection = 1.0 / (35.0f32.to_radians() * 0.5).tan();
    let near = group
        .bounds
        .projected_error_px([0.0, 2.0, 0.0], projection, 0.1, 720);
    let far = group
        .bounds
        .projected_error_px([0.0, 40.0, 0.0], projection, 0.1, 720);
    assert!(near.is_finite() && far.is_finite());
    assert!(
        near > far,
        "a group at 40 m must project under the same group at 2 m"
    );
}

#[test]
fn cluster_lod_empty_mesh_is_refused() {
    let empty = CookedMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        uv0: Vec::new(),
        tangents: Vec::new(),
        indices: Vec::new(),
    };
    assert!(build_clod_tree(&empty, ClodConfig::default()).is_err());
}
