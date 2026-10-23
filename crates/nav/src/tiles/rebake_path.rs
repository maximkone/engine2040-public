//! tiles::rebake_path: tile rebake methods, split from tiles.rs to keep the
//! file under the readable-size cap. Owner: nav crate; the tick calls
//! `rebake_budget` (version flips only) and an offline geometry lane calls
//! `rebake_budget_with` to refresh each flipped tile's cells.

use super::{tile_id_at, TileId, TiledNav};

impl TiledNav {
    /// Rebakes the lowest dirty tile, if any, without allocating.
    ///
    /// Overflowed tiles keep their dirty flag and are skipped, so one
    /// stuck counter never blocks the rest of the row.
    pub fn rebake_one(&mut self) -> Option<TileId> {
        self.rebake_one_with(&mut |_| {})
    }

    /// [`TiledNav::rebake_one`] that also hands the flipped tile to `lane`.
    ///
    /// The lane sees the tile once, after its version bumps, so an offline
    /// geometry lane (a mesh tile bake) refreshes exactly the cells the tick
    /// versioned. The tick calls [`TiledNav::rebake_one`], whose lane is
    /// empty: the tick keeps version flips only.
    pub fn rebake_one_with(&mut self, lane: &mut impl FnMut(TileId)) -> Option<TileId> {
        for index in 0..self.registry.tile_count() {
            let tile = tile_id_at(index);
            if !matches!(self.registry.is_dirty(tile), Ok(true)) {
                continue;
            }
            if self.registry.rebake(tile).is_ok() {
                self.stale = true;
                lane(tile);
                return Some(tile);
            }
        }
        None
    }

    /// Rebakes up to `max_tiles` dirty tiles in ascending order without
    /// allocating; returns how many rebaked. The tick calls this with its
    /// per-tick tile budget, so a breach opens within its tick budget.
    pub fn rebake_budget(&mut self, max_tiles: usize) -> usize {
        self.rebake_budget_with(max_tiles, &mut |_| {})
    }

    /// [`TiledNav::rebake_budget`] that also hands every flipped tile to
    /// `lane` in ascending order; returns how many rebaked. An offline
    /// geometry lane uses this to rebake exactly the tiles the tick
    /// versioned, while the tick itself calls [`TiledNav::rebake_budget`].
    pub fn rebake_budget_with(&mut self, max_tiles: usize, lane: &mut impl FnMut(TileId)) -> usize {
        let mut done = 0;
        while done < max_tiles {
            if self.rebake_one_with(lane).is_none() {
                break;
            }
            done += 1;
        }
        done
    }
}
