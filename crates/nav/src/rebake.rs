//! rebake: mesh-tile version bumps against the tile registry.
//! Owner: nav crate; tiles owns versions and dirty flags, mesh owns polygons.
//! Units: tile ids plus ticks at 60 Hz, same as tiles.
//! WD8 live path: `rebake_mesh_tile` bounds-checks the mesh, then the
//! registry bumps exactly that tile, so break sets rebake only their tiles.

use crate::mesh::NavMesh;
use crate::tiles::{TileError, TileId, TileRegistry};

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
}
