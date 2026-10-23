//! mesh::tiled: per-tile integer-mm bake lane for the navmesh.
//! Owner: the mesh module; consumers are the offline tile rebake lanes.
//! Units: cells, integer mm, tile ids. The window is a tile plus
//! [`TILE_BORDER`] cells each side; only the tile's own cells are written,
//! and the assembled mesh stays byte-equal to the whole-soup bake.

use super::{
    assemble, grid_u32, grid_usize, quant_to_tri_f, sample_height, tri_cell_span, BakeConfig,
    NavError, NavMesh, Prepared, MAX_SIDE, NO_POLY,
};

/// Border cells a tile bake includes around its own edge. The tile writes
/// only its interior; the ring is the neighbor context the tile window is
/// built in (the Recast `borderSize` shape), so edge cells sit against the
/// same whole-soup triangles as the tile center.
pub const TILE_BORDER: u32 = 2;

/// Tile layout of one [`TiledBake`]: edge plus overlap ring, the tile grid,
/// and which triangles each tile must scan.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TileLayout {
    edge: u32,
    border: u32,
    cols: u32,
    rows: u32,
    lanes: Vec<Vec<u32>>,
}

impl TileLayout {
    fn new(prepared: &Prepared, edge: u32, border: u32) -> Result<Self, NavError> {
        if edge == 0 || edge > MAX_SIDE {
            return Err(NavError::NotCanonical("tile edge"));
        }
        if border > MAX_SIDE {
            return Err(NavError::NotCanonical("tile border"));
        }
        let cols = prepared.width.div_ceil(edge);
        let rows = prepared.height.div_ceil(edge);
        let mut lanes = vec![Vec::new(); grid_usize(cols) * grid_usize(rows)];
        for (index, quant) in prepared.walkable.iter().enumerate() {
            let tri_f = quant_to_tri_f(quant);
            let (lo_x, hi_x) = tri_cell_span(
                [tri_f[0][0], tri_f[1][0], tri_f[2][0]],
                prepared.origin_mm[0],
                prepared.cell_mm,
                prepared.width,
            );
            let (lo_z, hi_z) = tri_cell_span(
                [tri_f[0][2], tri_f[1][2], tri_f[2][2]],
                prepared.origin_mm[1],
                prepared.cell_mm,
                prepared.height,
            );
            let index = grid_u32(index);
            for ty in lo_z / edge..=hi_z / edge {
                for tx in lo_x / edge..=hi_x / edge {
                    lanes[grid_usize(ty * cols + tx)].push(index);
                }
            }
        }
        Ok(Self {
            edge,
            border,
            cols,
            rows,
            lanes,
        })
    }

    fn tile_count(&self) -> u32 {
        self.cols * self.rows
    }

    /// Own-cell bounds of one tile: `x0..x1`, `z0..z1`, clipped to the grid.
    fn bounds(&self, tile: u32, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let (tx, ty) = (tile % self.cols, tile / self.cols);
        let (x0, z0) = (tx * self.edge, ty * self.edge);
        (
            x0,
            z0,
            (x0 + self.edge).min(width),
            (z0 + self.edge).min(height),
        )
    }
}

/// Per-tile integer-mm bake lane.
///
/// The soup is quantized and spanned once, exactly like
/// [`bake`](super::bake), then each tile rasterizes its own cells inside a
/// window of its edge plus [`TILE_BORDER`] cells each side that shares the
/// whole-bake origin and cell size. Cells sample the same sorted triangles
/// in the same order, so the assembled mesh is byte-equal to
/// [`bake`](super::bake) of the same soup, and a [`TiledBake::rebake`] of a
/// tile subset leaves every other tile's [`TiledBake::tile_bytes`]
/// untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TiledBake {
    prepared: Prepared,
    layout: TileLayout,
    owner: Vec<u32>,
    heights: Vec<i32>,
    nav: NavMesh,
}

impl TiledBake {
    /// Bakes the soup tile by tile at `edge` cells per tile with the
    /// [`TILE_BORDER`] overlap ring.
    ///
    /// Errors match [`bake`](super::bake); a zero or oversized tile edge
    /// refuses as [`NavError::NotCanonical`].
    pub fn bake(tris: &[[[f32; 3]; 3]], config: &BakeConfig, edge: u32) -> Result<Self, NavError> {
        Self::bake_with_border(tris, config, edge, TILE_BORDER)
    }

    /// [`TiledBake::bake`] with an explicit overlap ring in cells.
    pub fn bake_with_border(
        tris: &[[[f32; 3]; 3]],
        config: &BakeConfig,
        edge: u32,
        border: u32,
    ) -> Result<Self, NavError> {
        let prepared = Prepared::new(tris, config)?;
        let layout = TileLayout::new(&prepared, edge, border)?;
        let cells = grid_usize(prepared.width) * grid_usize(prepared.height);
        let mut owner = vec![NO_POLY; cells];
        let mut heights = vec![0i32; cells];
        for tile in 0..layout.tile_count() {
            raster_tile(&prepared, &layout, tile, &mut owner, &mut heights);
        }
        if owner.iter().all(|poly| *poly == NO_POLY) {
            return Err(NavError::NoWalkable);
        }
        let nav = assemble(
            prepared.cell_mm,
            prepared.origin_mm,
            prepared.width,
            prepared.height,
            owner.clone(),
            heights.clone(),
        );
        Ok(Self {
            prepared,
            layout,
            owner,
            heights,
            nav,
        })
    }

    /// Tiles in the layout, `cols * rows`.
    pub fn tile_count(&self) -> u32 {
        self.layout.tile_count()
    }

    /// Tile layout `(edge, border, cols, rows)` in cells.
    pub fn tile_layout(&self) -> (u32, u32, u32, u32) {
        (
            self.layout.edge,
            self.layout.border,
            self.layout.cols,
            self.layout.rows,
        )
    }

    /// Assembled whole-grid navmesh, rebuilt after each rebake. Equal to
    /// [`bake`](super::bake) of the same soup.
    pub fn nav(&self) -> &NavMesh {
        &self.nav
    }

    /// Canonical bytes of one tile's own cells, row-major: a walkable flag
    /// then the height in integer millimeters per cell.
    ///
    /// Errors: [`NavError::NotCanonical`] for a tile past the layout.
    pub fn tile_bytes(&self, tile: u32) -> Result<Vec<u8>, NavError> {
        if tile >= self.layout.tile_count() {
            return Err(NavError::NotCanonical("tile id"));
        }
        let (x0, z0, x1, z1) = self
            .layout
            .bounds(tile, self.prepared.width, self.prepared.height);
        let width = grid_usize(self.prepared.width);
        let mut out = Vec::with_capacity(grid_usize((x1 - x0) * (z1 - z0)) * 5);
        for z in z0..z1 {
            for x in x0..x1 {
                let index = grid_usize(z) * width + grid_usize(x);
                out.push(u8::from(self.owner[index] != NO_POLY));
                out.extend_from_slice(&self.heights[index].to_le_bytes());
            }
        }
        Ok(out)
    }

    /// Rebakes exactly `tiles`, in the order given, and rebuilds the
    /// assembled mesh; returns how many tiles it rebaked.
    ///
    /// A tile past the layout refuses before any cell changes
    /// ([`NavError::NotCanonical`]). Cells of tiles not named are never
    /// written, so their [`TiledBake::tile_bytes`] stay identical.
    pub fn rebake(&mut self, tiles: &[u32]) -> Result<usize, NavError> {
        if tiles.iter().any(|tile| *tile >= self.layout.tile_count()) {
            return Err(NavError::NotCanonical("tile id"));
        }
        for tile in tiles {
            raster_tile(
                &self.prepared,
                &self.layout,
                *tile,
                &mut self.owner,
                &mut self.heights,
            );
        }
        if !tiles.is_empty() {
            self.nav = assemble(
                self.prepared.cell_mm,
                self.prepared.origin_mm,
                self.prepared.width,
                self.prepared.height,
                self.owner.clone(),
                self.heights.clone(),
            );
        }
        Ok(tiles.len())
    }
}

/// Rasterizes one tile's own cells from its lane, sampling at the shared
/// integer-mm cell centers so the overlap ring never moves a tile value.
fn raster_tile(
    prepared: &Prepared,
    layout: &TileLayout,
    tile: u32,
    owner: &mut [u32],
    heights: &mut [i32],
) {
    let (x0, z0, x1, z1) = layout.bounds(tile, prepared.width, prepared.height);
    // The window is the tile plus its overlap ring, the geometry the tile
    // lane is built in; only the tile's own cells are written below.
    let wx0 = x0.saturating_sub(layout.border);
    let wz0 = z0.saturating_sub(layout.border);
    let wx1 = (x1 + layout.border).min(prepared.width);
    let wz1 = (z1 + layout.border).min(prepared.height);
    let (ww, wh) = (grid_usize(wx1 - wx0), grid_usize(wz1 - wz0));
    let mut sum = vec![0i64; ww * wh];
    let mut count = vec![0u32; ww * wh];
    for index in &layout.lanes[grid_usize(tile)] {
        let tri_f = quant_to_tri_f(&prepared.walkable[grid_usize(*index)]);
        let (lo_x, hi_x) = tri_cell_span(
            [tri_f[0][0], tri_f[1][0], tri_f[2][0]],
            prepared.origin_mm[0],
            prepared.cell_mm,
            prepared.width,
        );
        let (lo_z, hi_z) = tri_cell_span(
            [tri_f[0][2], tri_f[1][2], tri_f[2][2]],
            prepared.origin_mm[1],
            prepared.cell_mm,
            prepared.height,
        );
        let (lo_x, hi_x) = (lo_x.max(wx0), hi_x.min(wx1 - 1));
        let (lo_z, hi_z) = (lo_z.max(wz0), hi_z.min(wz1 - 1));
        if lo_x > hi_x || lo_z > hi_z {
            continue;
        }
        for cz in lo_z..=hi_z {
            for cx in lo_x..=hi_x {
                let center = [
                    f64::from(prepared.origin_mm[0])
                        + (f64::from(cx) + 0.5) * f64::from(prepared.cell_mm),
                    f64::from(prepared.origin_mm[1])
                        + (f64::from(cz) + 0.5) * f64::from(prepared.cell_mm),
                ];
                if let Some(height_mm) = sample_height(&tri_f, center) {
                    let local = grid_usize(cz - wz0) * ww + grid_usize(cx - wx0);
                    sum[local] += i64::from(height_mm);
                    count[local] += 1;
                }
            }
        }
    }
    for z in z0..z1 {
        for x in x0..x1 {
            let local = grid_usize(z - wz0) * ww + grid_usize(x - wx0);
            let index = grid_usize(z) * grid_usize(prepared.width) + grid_usize(x);
            if count[local] == 0 {
                owner[index] = NO_POLY;
                heights[index] = 0;
            } else {
                owner[index] = 0;
                // `mean` averages values that were each an `i32`, so it is
                // inside `i32` as well.
                let mean = sum[local] / i64::from(count[local]);
                heights[index] = i32::try_from(mean).unwrap_or_default();
            }
        }
    }
}
