//! rebake: alias layer over tiles for older imports.
//! Owner: nav crate; tiles owns versions, dirty flags, and flow fields.
//! Units: tile ids plus ticks at 60 Hz, same as tiles.
//! This module only re-exports and bounds-checks, so prior paths keep
//! compiling while every rebake runs through tiles.

pub use crate::tiles::{
    BreakSet, TileError as RebakeError, TileId, TileRegistry, MAX_BREAK_TILES, MAX_TILES,
};

use crate::mesh::{NavMesh, StackedNav};
use crate::tiles::TileError;

/// Rebakes the tile a mesh polygon names.
///
/// Bounds come from the mesh polygon count, versions from the registry,
/// so the mesh never pretends to version itself. Errors:
/// [`TileError::BadTile`], [`TileError::VersionOverflow`].
pub fn rebake_mesh_tile(
    registry: &mut TileRegistry,
    mesh: &NavMesh,
    tile: TileId,
) -> Result<u32, TileError> {
    if tile.0 as usize >= mesh.polygon_count() {
        return Err(TileError::BadTile { tile: tile.0 });
    }
    registry.rebake(tile)
}

/// Rebakes one tile on one stacked level; wiring caller for [`StackedNav`].
/// Errors: [`TileError::BadTile`], [`TileError::VersionOverflow`].
pub fn rebake_stacked_level(
    registry: &mut TileRegistry,
    stacked: &StackedNav,
    level: usize,
    tile: TileId,
) -> Result<u32, TileError> {
    let mesh = stacked
        .levels()
        .get(level)
        .ok_or(TileError::BadTile { tile: tile.0 })?;
    rebake_mesh_tile(registry, mesh, tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor_mesh() -> NavMesh {
        let tris = [
            [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]],
            [[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
        ];
        NavMesh::bake(&tris, &crate::mesh::BakeConfig::default()).expect("floor bakes")
    }

    #[test]
    fn mesh_tile_rebake_bumps_through_the_registry() {
        let mesh = floor_mesh();
        let mut registry = TileRegistry::new(16).expect("registry builds");
        assert_eq!(rebake_mesh_tile(&mut registry, &mesh, TileId(0)), Ok(1));
        assert_eq!(registry.version(TileId(0)), Ok(1));
    }

    #[test]
    fn mesh_tile_past_the_mesh_refuses() {
        let mesh = floor_mesh();
        let mut registry = TileRegistry::new(16).expect("registry builds");
        assert_eq!(
            rebake_mesh_tile(&mut registry, &mesh, TileId(9)),
            Err(TileError::BadTile { tile: 9 })
        );
        assert_eq!(
            registry.version(TileId(0)),
            Ok(0),
            "refusals change nothing"
        );
    }

    #[test]
    fn aliases_name_the_tiles_owner() {
        let set = BreakSet::new(5, &[TileId(2), TileId(1)], 16).expect("set builds");
        assert_eq!(set.tiles, vec![TileId(1), TileId(2)]);
        assert_eq!(
            BreakSet::new(5, &[], 16),
            Err(RebakeError::EmptySet),
            "the old error name still reads"
        );
    }
}
