//! destruction: offline pre-fracture bake. Runtime ships chunk data only.

pub mod bake;
pub mod bake3d;
pub mod bonded;
/// Debris budgets at 8-fighter scale: per-event caps, sleep islands,
/// distance LOD. Std-only.
pub mod budget;
/// Sparse bond topology over a bonded descriptor: sorted adjacency, anchor
/// set and anchor connectivity for the propagation leg. Std-only.
pub mod cluster;
/// Siege-wall baked fracture plus crack spread on the bonded path: cache
/// reads, fixed-point spread and the integer hash state. Std-only.
pub mod crack;
pub mod csg;
/// Bounds-checked byte reads shared by the snapshot decoders.
mod cursor;
/// Runtime Voronoi fracture at an impact point with a capped cell count, break
/// events on the bonded shape, and the cook cache that serves the same
/// cut without the cold clip. Std-only.
pub mod fracture;
/// World material table for deterministic breaks: fourteen Q16 rows with
/// toughness, brittleness, shard rule, sound, dust, spark, decal and burn
/// rate, plus bond and chunk material-id validators. Std-only.
pub mod material;
/// Shape matching for bonded debris: best-fit rotation per cluster, goal
/// positions with a dt-independent spring and a volume proxy, so chunks
/// hold their form under torque. Opt-in, never in the bonded tick. Std-only.
pub mod shape_match;
pub mod shards;
pub mod shards3d;
/// One canonical snapshot of destruction state (bonded collections plus the
/// debris ledger) for rollback resim across a collapse. Std-only.
pub mod snapshot;
/// Crack spread over the bonded graph: sorted flood, fixed-point strain,
/// break events shaped like the bonded collection's. Std-only.
pub mod spread;
/// Load-path analysis over a bonded collection (weakest link, predicted
/// collapse, report digest) plus the seeded oracle wall and frame
/// building fixtures. Std-only.
pub mod structure;

pub use bake::{bake_bytes, bake_hash, fracture, Chunk};
pub use bonded::{
    event_bytes, Bond, BondedCollection, BondedDesc, BreakError, BreakEvent, Chunk3, LOAD_Q16_ONE,
    NO_LAYER, NO_PARENT,
};
pub use crack::{CrackError, CrackOutcome, CrackWall};
pub use csg::{
    bake, blade_plane, brush_intersect, brush_subtract, csg_bake_bytes, csg_bake_digest, frac_name,
    frac_name_into, knife, shell_union, to_bonded_desc, CsgError, CutPlane, FractureChunk,
    PropMesh,
};
pub use fracture::{
    fracture_at, CacheKey, Fracture, FractureCache, FractureError, FractureSpec, MAX_CACHE_GRID,
    MAX_FRACTURE_CELLS,
};
pub use material::{
    validate as validate_material, validate_bond_materials, validate_chunk_materials, DecalKind,
    DustKind, MaterialError, MaterialId, MaterialRow, MaterialTable, ShardRule, SoundKind,
    SparkKind, MATERIAL_COUNT, MATERIAL_VERSION_V1, MAX_BRITTLENESS_Q16, MAX_BURN_Q16,
    MAX_TOUGHNESS_Q16, Q16_ONE,
};
pub use shape_match::{ShapeError, ShapeMatcher, SpringCoefficients};
pub use structure::{
    analyze, building, collapse_digest, oracle_wall, report_digest, seeded_wall_cut_digest,
    Building, BuildingSpec, ElementKind, StructureError, StructureReport,
};

// The one counting allocator of this test binary: zero-allocation tests read
// it through `engine_core::alloc_probe::measure`.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

/// Caller-visible pins for the crate-root re-exports: a caller includes
/// `engine_destruction` and uses these names, so the root must stay wired
/// and the behavior behind it must stay stable. Breaks when bake changes
/// shape, when the seed stops mattering, or when the shipped material kit
/// stops validating.
#[cfg(test)]
mod root_api_tests {
    use super::{
        bake_bytes, bake_hash, fracture, validate_bond_materials, validate_material, MaterialId,
        MaterialTable,
    };

    #[test]
    fn root_bake_is_deterministic_and_sized() {
        let first = fracture(0.0, 0.0, 10.0, 5.0, 8, 42);
        let second = fracture(0.0, 0.0, 10.0, 5.0, 8, 42);
        assert_eq!(first.len(), 8);
        assert_eq!(second.len(), 8);
        assert_eq!(bake_bytes(&first), bake_bytes(&second));
        assert_eq!(bake_hash(&first), bake_hash(&second));
        assert_eq!(bake_bytes(&first).len(), 8 * 20);
        for chunk in &first {
            let expect = chunk.hx * chunk.hy * 4.0;
            assert!((chunk.mass - expect).abs() < 1e-4, "mass follows hx*hy*4");
        }
        let fewer = fracture(0.0, 0.0, 10.0, 5.0, 7, 42);
        assert_eq!(fewer.len(), 7);
        assert_eq!(bake_bytes(&fewer).len(), 7 * 20);
        assert_ne!(bake_bytes(&first), bake_bytes(&fewer));
        let moved = fracture(5.0, 0.0, 10.0, 5.0, 8, 42);
        assert_ne!(bake_hash(&first), bake_hash(&moved));
    }

    #[test]
    fn root_siege_kit_validates_and_digests_stably() {
        let kit = MaterialTable::siege_kit();
        assert!(kit.validate().is_ok());
        assert!(validate_material(kit).is_ok());
        assert_eq!(kit.digest(), MaterialTable::siege_kit().digest());
        assert!(kit.is_combustible(MaterialId::Thatch));
        assert!(!kit.is_combustible(MaterialId::Stone));
        assert!(validate_bond_materials(&[MaterialId::Stone.as_u8()]).is_ok());
        assert!(validate_bond_materials(&[u8::MAX]).is_err());
    }
}
