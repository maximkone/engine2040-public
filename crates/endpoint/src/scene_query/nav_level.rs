//! AD20 step 2: the level's own nav bake is the door's reach view.
//!
//! `level.build` bakes the landed level's walkable tiles (`build_level::bake_nav`)
//! and this adapter turns those tiles into the [`NavMesh`] the session holds, so
//! `scene.query reachable` answers from the level the AI just built instead of
//! refusing `needs_nav_view`. Data owner: `engine-nav` owns the mesh and the
//! polygon adapter (`engine_nav::reach::reach_view`), `build_level` owns the
//! walk flags. Units: meters out of the 100 cm level tile.
//!
//! Determinism: tiles come out of a `BTreeMap`, so the triangle order and the
//! baked mesh are the same for the same level. A level with no walkable tile
//! bakes nothing, so the refusal stays honest rather than answering from an
//! empty world.

use engine_nav::mesh::{BakeConfig, NavMesh};
use engine_spatial::NavView;

use crate::build_level::NAV_TILE_CM;
use crate::mcp::ServerState;

/// One upward floor quad per walkable level tile, in tile order. A tile spans
/// `[tx, tx + 1] x [tz, tz + 1]` meters, the same square the level's tile
/// bake judged, and the two triangles wind upward so the bake keeps them.
#[expect(
    clippy::cast_precision_loss,
    reason = "a level tile index is a small integer (±6), exact in f32"
)]
fn tile_quads(view: &NavView) -> Vec<[[f32; 3]; 3]> {
    let side = NAV_TILE_CM as f32 / 100.0;
    let mut tris = Vec::with_capacity(view.tiles.len() * 2);
    for (&(tx, tz), tile) in &view.tiles {
        if !tile.walk {
            continue;
        }
        let x0 = tx as f32 * side;
        let z0 = tz as f32 * side;
        let (x1, z1) = (x0 + side, z0 + side);
        tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z0]]);
        tris.push([[x1, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
    }
    tris
}

/// The level's walkable tiles as one baked nav mesh; `None` when the level
/// baked no walkable tile, so the session keeps answering `needs_nav_view`.
pub fn level_mesh(view: &NavView) -> Option<NavMesh> {
    let tris = tile_quads(view);
    if tris.is_empty() {
        return None;
    }
    NavMesh::bake(&tris, &BakeConfig::default()).ok()
}

impl ServerState {
    /// Hold the landed level's nav bake. `level.build` calls this after the
    /// batch lands, so every later `reachable` reads the level the AI built.
    pub fn hand_level_nav(&mut self, view: &NavView) {
        self.nav = level_mesh(view);
    }
}
