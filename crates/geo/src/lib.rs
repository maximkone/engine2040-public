//! geo: cook-time mesh tools an AI can call (decision D4): booleans, UV unwrap and pack,
//! seam-locked LOD, collision decomposition.
//!
//! Callers and edges (from `crates/geo/Cargo.toml:30-35`): above `engine-core` digests, plus
//! `engine-assets` only behind the off-by-default `kit` feature (kit tests read the arena glTF
//! files); every donor library sits behind its own feature, off by default
//! (`crates/geo/Cargo.toml:15-28`). The only in-repo consumer is the door `geo` pack
//! (`crates/endpoint/Cargo.toml:57-58`, feature `geo`, off by default); off `default-members`
//! (root `Cargo.toml:14-17`). No tick crate depends on it: cook-time only, never in the tick graph.
//!
//! Invariants: results are pure functions of the input mesh and the options (no clock, no thread
//! count, no random seed the caller did not pass), so a cook run twice gives the same bytes; the
//! digest of every output is `GeoMesh::digest`. Cook-time `f32` is allowed here (see
//! `primitives`, and `parry3d` with `f32` in `crates/geo/Cargo.toml:35`); nothing here runs in
//! the fixed tick. Errors: [`GeoError`] for bad meshes, `recipe::RecipeError` for bad recipes.
//!
//! Budgets: 64 recipe ops, 200_000 triangles, 32 name chars
//! (`crates/geo/src/recipe.rs:18-22`). Entry points: [`GeoMesh`], [`GeoError`], [`EdgeReport`],
//! [`mesh`], [`ops`], [`primitives`], [`recipe`], and one module per feature (`boolean`, `uv`,
//! `lod`, `collision`).
//!
//! Donors (licenses read from each crate's own file on 2026-10-04, all on the `deny.toml` list):
//! `manifold-rust` (Apache-2.0, port of Manifold), `xatlas` (MIT, vendored source behind the
//! `uv` feature), `meshopt` (MIT OR Apache-2.0, meshoptimizer), `parry3d` (Apache-2.0, V-HACD).

pub mod mesh;
/// Native mesh ops (weld, mirror, array, flip, normals): std only, no feature.
pub mod ops;
pub mod primitives;
pub mod recipe;

#[cfg(feature = "bake")]
pub mod bake;
#[cfg(feature = "boolean")]
pub mod boolean;
#[cfg(feature = "collision")]
pub mod collision;
#[cfg(feature = "lod")]
pub mod lod;
#[cfg(feature = "uv")]
pub mod uv;

pub use mesh::{EdgeReport, GeoError, GeoMesh};

#[cfg(test)]
mod wiring_tests {
    //! Root contract: what the callers see through `engine_geo::`.
    //!
    //! Why this module exists: `crates/geo/src/lib.rs` changed 3 times in
    //! 30 days yet held zero `#[test]`, and the crate has no `tests/it/`
    //! directory, so a dropped `pub mod` or `pub use` would still compile
    //! the crate but break every caller at their `use` line (the endpoint
    //! `geo` pack, the kit tests). Each test below names the root path, so
    //! a removed or feature-gated item fails here first. The behavior
    //! asserts go through the same root path: they fail when the behavior
    //! breaks, not when the internals move.

    use super::*;
    use crate::{mesh, primitives, recipe};

    /// Unit tetra with outward faces (volume exactly 1/6).
    fn tetra() -> GeoMesh {
        GeoMesh::from_positions(
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3],
        )
    }

    #[test]
    fn root_paths_resolve_for_callers() {
        // Every entry point stays reachable through the crate root.
        let mesh = crate::GeoMesh::from_positions(vec![[0.0; 3]], vec![]);
        assert_eq!(mesh.triangle_count(), 0);
        let report: crate::EdgeReport = crate::EdgeReport::default();
        assert!(!report.is_closed());
        let _: crate::GeoError = crate::GeoError::Empty;
        assert_eq!(crate::recipe::MAX_OPS, 64);
        assert_eq!(crate::recipe::MAX_TRIANGLES, 200_000);
        assert_eq!(crate::recipe::MAX_NAME, 32);
        let cube = crate::primitives::cuboid([1.0, 1.0, 1.0]);
        assert_eq!(cube.triangle_count(), 12);
        // The validate signature callers program against stays put.
        let _: fn(&crate::mesh::GeoMesh) -> Result<(), crate::GeoError> =
            crate::mesh::GeoMesh::validate;
        let _ = mesh::GeoError::NonFinite;
        let _ = primitives::cuboid_faces([1.0, 1.0, 1.0]);
        let _ = recipe::RecipeError::TooManyOps;
    }

    #[test]
    fn tetra_is_closed_with_exact_volume_and_bounds() {
        let mesh = tetra();
        mesh.validate().expect("unit tetra validates");
        assert_eq!(mesh.triangle_count(), 4);
        assert_eq!(
            mesh.edge_report(),
            EdgeReport {
                edges: 6,
                open_edges: 0
            }
        );
        assert!(mesh.edge_report().is_closed());
        assert_eq!(mesh.volume(), 1.0 / 6.0);
        assert_eq!(mesh.bounds(), ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
        assert_eq!(mesh.weld_map(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn validate_rejects_bad_meshes_loudly() {
        // Normals length mismatch names its array.
        let mut mesh = tetra();
        mesh.normals = vec![[0.0, 0.0, 1.0]];
        assert_eq!(mesh.validate(), Err(GeoError::Malformed("normals length")));
        // UV length mismatch names its array.
        let mut mesh = tetra();
        mesh.uvs = vec![[0.0, 0.0], [1.0, 1.0]];
        assert_eq!(mesh.validate(), Err(GeoError::Malformed("uvs length")));
        // Index count must come in triples.
        let mesh = GeoMesh::from_positions(vec![[0.0; 3]], vec![0]);
        assert_eq!(
            mesh.validate(),
            Err(GeoError::Malformed("index count not a multiple of 3"))
        );
        // Out-of-range indices never reach the census.
        let mesh = GeoMesh::from_positions(vec![[0.0; 3]], vec![0, 0, 7]);
        assert_eq!(
            mesh.validate(),
            Err(GeoError::Malformed("index out of range"))
        );
        // NaN positions fail even when every length lines up.
        let mesh = GeoMesh::from_positions(
            vec![[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![0, 1, 2],
        );
        assert_eq!(mesh.validate(), Err(GeoError::NonFinite));
    }

    #[test]
    fn error_display_strings_stay_caller_readable() {
        assert_eq!(
            GeoError::Empty.to_string(),
            "geo: mesh has no triangles".to_string()
        );
        assert_eq!(
            GeoError::NonFinite.to_string(),
            "geo: mesh has a non-finite value"
        );
        assert_eq!(GeoError::NotClosed.to_string(), "geo: mesh is not closed");
        assert_eq!(
            GeoError::Malformed("normals length").to_string(),
            "geo: malformed mesh: normals length"
        );
        assert_eq!(
            GeoError::Donor("boom".to_string()).to_string(),
            "geo: donor library: boom"
        );
    }

    #[test]
    fn digest_is_deterministic_and_sensitive() {
        let a = tetra();
        // Same bytes twice give the same receipt (the cook-twice invariant).
        assert_eq!(a.digest(), tetra().digest());
        assert_ne!(a.digest(), 0);
        // One flipped index bit changes the receipt.
        let mut moved = tetra();
        moved.indices[11] = 2;
        assert_ne!(a.digest(), moved.digest());
        // Normals ride the digest too, not just positions.
        let mut normal = tetra();
        normal.normals = vec![[0.0, 0.0, 1.0]; 4];
        assert_ne!(a.digest(), normal.digest());
    }

    #[test]
    fn cuboid_through_root_is_a_closed_box() {
        let cube = primitives::cuboid([2.0, 2.0, 2.0]);
        cube.validate().expect("cuboid validates");
        assert!(cube.edge_report().is_closed());
        assert!((cube.volume() - 8.0).abs() < 1e-9);
        assert_eq!(cube.bounds(), ([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]));
    }
}
