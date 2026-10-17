//! mesh: navmesh bake plus point/area queries over walkable polygons.
//! Owner: nav crate; live consumers: ludus proof bake plus rebake.rs tile
//! bumps. Units: meters in and out,
//! grid cells inside, integer millimeters in bytes. Determinism: quantized
//! sort plus index-order flood fill, so identical geometry bakes identical
//! bytes. Errors: typed NavError, unreachable never guesses.

use std::collections::VecDeque;
use std::fmt;

use crate::cursor::{Cursor, Truncated};
/// Largest absolute vertex coordinate in meters; beyond it refuses.
pub const MAX_COORD_M: f32 = 4096.0;

/// Bake wire magic (`E2NAV1`) at the head of every blob.
const MESH_MAGIC: [u8; 6] = *b"E2NAV1";
/// Bake wire version; bump on any layout change, refuse anything else.
pub const MESH_VERSION: u16 = 1;
/// Millimeters per meter; the only float-to-int scale in the bake.
const MM_PER_M: f32 = 1000.0;
/// Largest triangle soup one bake accepts; larger inputs refuse loudly.
pub const MAX_TRIS: usize = 65_536;
/// Largest grid side in cells; wider spans refuse loudly.
pub const MAX_SIDE: u32 = 1024;
/// Largest cell count; denser grids refuse loudly.
pub const MAX_CELLS: usize = 1_048_576;
/// Largest blob one decode accepts; larger blobs refuse loudly.
pub const MAX_BYTES: usize = 16 << 20;
/// Largest area-query radius in meters; beyond it refuses.
pub const MAX_RADIUS_M: f32 = 1024.0;
/// Twice the area under which a triangle counts as degenerate, in m^2.
const DEGENERATE_AREA2: f32 = 1e-10;
/// Sentinel for "no polygon" in the cell lookup table.
const NO_POLY: u32 = u32::MAX;

/// Bake configuration. Units: meters for `cell_size`, cosine of the
/// steepest walkable slope for `walkable_normal_y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeConfig {
    /// Voxel size in meters, within `0.05..=5.0`.
    pub cell_size: f32,
    /// Minimum triangle normal Y kept as walkable, within `(0.0, 1.0]`.
    pub walkable_normal_y: f32,
}

impl Default for BakeConfig {
    fn default() -> Self {
        Self {
            cell_size: 0.25,
            walkable_normal_y: 0.7,
        }
    }
}

/// Stable polygon id; dense `0..count`, ordered by minimum cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PolyId(pub u32);

/// One walkable polygon: a connected walkable component with stable cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Polygon {
    id: PolyId,
    area: u8,
    cells: Vec<[u32; 2]>,
    neighbors: Vec<PolyId>,
}

impl Polygon {
    /// Stable id, dense `0..count`.
    pub fn id(&self) -> PolyId {
        self.id
    }
    /// Area kind (all zero for now; crowd and links own per-area costs).
    pub fn area(&self) -> u8 {
        self.area
    }
    /// Grid-local `(x, z)` cells, ascending.
    pub fn cells(&self) -> &[[u32; 2]] {
        &self.cells
    }
    /// Adjacent polygon ids, ascending, never self.
    pub fn neighbors(&self) -> &[PolyId] {
        &self.neighbors
    }
}

/// Baked navmesh: sorted polygons plus the cell tables queries read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavMesh {
    cell_mm: u32,
    origin_mm: [i32; 2],
    width: u32,
    height: u32,
    polygons: Vec<Polygon>,
    cell_poly: Vec<u32>,
    cell_h_mm: Vec<i32>,
}

/// Typed bake/query failures. Unreachable targets error, never guess.
#[derive(Clone, Debug, PartialEq)]
pub enum NavError {
    /// No triangles were given.
    EmptyInput,
    /// Triangle count over the bake bound.
    TooManyTriangles {
        /// Triangle count offered.
        got: usize,
        /// Bake bound refused past.
        max: usize,
    },
    /// Non-finite coordinate in the named triangle.
    NonFiniteTri {
        /// Triangle index in caller order.
        tri: usize,
    },
    /// Zero-area triangle in the named triangle.
    DegenerateTri {
        /// Triangle index in caller order.
        tri: usize,
    },
    /// Coordinate past the bake bound in the named triangle.
    VertexOutOfRange {
        /// Triangle index in caller order.
        tri: usize,
    },
    /// Cell size outside `0.05..=5.0` meters.
    BadCellSize {
        /// Cell size offered.
        got: f32,
    },
    /// Slope cosine outside `(0.0, 1.0]`.
    BadSlope {
        /// Cosine offered.
        got: f32,
    },
    /// Grid span over the bake bound.
    GridTooLarge {
        /// Grid width in cells.
        w: u32,
        /// Grid height in cells.
        h: u32,
        /// Side bound refused past.
        max_side: u32,
    },
    /// No walkable surface survived slope and raster checks.
    NoWalkable,
    /// Point lies outside the baked grid (NaN counts as outside).
    OutOfBounds,
    /// Point is inside the grid but on no walkable cell.
    NoPolygon,
    /// Walkable endpoints share no polygon path.
    Unreachable,
    /// Area radius is non-finite, negative, or over the bound.
    BadRadius {
        /// Radius offered in meters.
        got: f32,
    },
    /// Blob magic is not `E2NAV1`.
    BadMagic,
    /// Blob version is not the bake version.
    VersionMismatch {
        /// Version found.
        got: u16,
    },
    /// Blob ends mid-record.
    Truncated,
    /// Blob has bytes past the last polygon record.
    TrailingBytes {
        /// Leftover byte count.
        got: usize,
    },
    /// Blob breaks canonical order (ids, cells, or neighbors unsorted).
    NotCanonical(&'static str),
    /// Blob is larger than the decode bound.
    TooLarge {
        /// Byte count offered.
        got: usize,
    },
}

impl fmt::Display for NavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "navmesh: no triangles"),
            Self::TooManyTriangles { got, max } => {
                write!(f, "navmesh: {got} triangles over bound {max}")
            }
            Self::NonFiniteTri { tri } => write!(f, "navmesh: triangle {tri} not finite"),
            Self::DegenerateTri { tri } => write!(f, "navmesh: triangle {tri} has no area"),
            Self::VertexOutOfRange { tri } => {
                write!(f, "navmesh: triangle {tri} past coordinate bound")
            }
            Self::BadCellSize { got } => write!(f, "navmesh: cell size {got} outside 0.05..=5.0 m"),
            Self::BadSlope { got } => write!(f, "navmesh: slope cosine {got} outside (0.0, 1.0]"),
            Self::GridTooLarge { w, h, max_side } => {
                write!(f, "navmesh: grid {w}x{h} over side bound {max_side}")
            }
            Self::NoWalkable => write!(f, "navmesh: no walkable surface"),
            Self::OutOfBounds => write!(f, "navmesh: point outside the grid"),
            Self::NoPolygon => write!(f, "navmesh: point on no walkable cell"),
            Self::Unreachable => write!(f, "navmesh: no polygon path"),
            Self::BadRadius { got } => write!(f, "navmesh: radius {got} m refused"),
            Self::BadMagic => write!(f, "navmesh: bad magic"),
            Self::VersionMismatch { got } => write!(f, "navmesh: version {got} refused"),
            Self::Truncated => write!(f, "navmesh: blob truncated"),
            Self::TrailingBytes { got } => write!(f, "navmesh: {got} trailing bytes"),
            Self::NotCanonical(why) => write!(f, "navmesh: blob not canonical ({why})"),
            Self::TooLarge { got } => write!(f, "navmesh: {got} bytes over decode bound"),
        }
    }
}

impl std::error::Error for NavError {}

/// Bake walkable triangles into a navmesh.
///
/// Input is a triangle soup in meters (`[x, y, z]`, `+Y` up); triangle
/// order never affects the output because quantized vertices sort first.
/// Steep triangles are skipped (walls split regions); degenerate,
/// non-finite, or out-of-range triangles refuse with their caller index.
pub fn bake(tris: &[[[f32; 3]; 3]], config: &BakeConfig) -> Result<NavMesh, NavError> {
    check_config(config)?;
    if tris.is_empty() {
        return Err(NavError::EmptyInput);
    }
    if tris.len() > MAX_TRIS {
        return Err(NavError::TooManyTriangles {
            got: tris.len(),
            max: MAX_TRIS,
        });
    }
    // Quantize first so sort and bytes share one integer view.
    let mut quant = Vec::with_capacity(tris.len());
    for (index, tri) in tris.iter().enumerate() {
        quant.push(quantize_tri(tri, index)?);
    }
    let mut order: Vec<usize> = (0..quant.len()).collect();
    order.sort_by(|a, b| quant[*a].cmp(&quant[*b]));
    order.dedup_by(|a, b| quant[*a] == quant[*b]);
    // NaN would round to 0 mm under `as`. check_config already bounds the
    // size, so this only guards the rounding itself.
    let cell_mm = round_mm(f64::from(config.cell_size * MM_PER_M))
        .and_then(|mm| u32::try_from(mm).ok())
        .filter(|mm| *mm > 0)
        .ok_or(NavError::BadCellSize {
            got: config.cell_size,
        })?;
    let mut walkable: Vec<[i32; 9]> = Vec::with_capacity(order.len());
    for index in &order {
        let quant_tri = &quant[*index];
        if tri_normal_y(quant_tri) >= config.walkable_normal_y {
            walkable.push(*quant_tri);
        }
    }
    if walkable.is_empty() {
        return Err(NavError::NoWalkable);
    }
    let (origin_mm, width, height) = grid_span(&walkable, cell_mm)?;
    let (cell_poly, cell_h_mm) = rasterize(&walkable, origin_mm, width, height, cell_mm);
    if cell_poly.iter().all(|p| *p == NO_POLY) {
        return Err(NavError::NoWalkable);
    }
    Ok(assemble(
        cell_mm, origin_mm, width, height, cell_poly, cell_h_mm,
    ))
}

impl NavMesh {
    /// Bake through the free function with an explicit config.
    pub fn bake(tris: &[[[f32; 3]; 3]], config: &BakeConfig) -> Result<Self, NavError> {
        bake(tris, config)
    }
    /// Polygon count.
    pub fn polygon_count(&self) -> usize {
        self.polygons.len()
    }
    /// Polygons in stable id order.
    pub fn polygons(&self) -> &[Polygon] {
        &self.polygons
    }
    /// Polygon by id, if held.
    pub fn polygon(&self, id: PolyId) -> Option<&Polygon> {
        self.polygons.get(id.0 as usize)
    }
    /// Cell size in meters.
    pub fn cell_size_m(&self) -> f32 {
        self.cell_mm as f32 / MM_PER_M
    }
    /// Walkable area of polygon `id` in square meters, from the shoelace
    /// formula over its boundary edges. An edge shared with a cell of the same
    /// polygon is skipped, so holes subtract with no special case. None when
    /// `id` is not held.
    pub fn polygon_area_m2(&self, id: PolyId) -> Option<f64> {
        let poly = self.polygon(id)?;
        let mut edges = Vec::with_capacity(poly.cells().len() * 4);
        for &[cx, cz] in poly.cells() {
            let (x, z) = (f64::from(cx), f64::from(cz));
            let (ix, iz) = (i64::from(cx), i64::from(cz));
            // Counter-clockwise: each side is (from, to, cell across it).
            let sides = [
                ([x, z], [x + 1.0, z], [ix, iz - 1]),
                ([x + 1.0, z], [x + 1.0, z + 1.0], [ix + 1, iz]),
                ([x + 1.0, z + 1.0], [x, z + 1.0], [ix, iz + 1]),
                ([x, z + 1.0], [x, z], [ix - 1, iz]),
            ];
            for (from, to, across) in sides {
                if !self.owned_by(across, id) {
                    edges.push((from, to));
                }
            }
        }
        let cell_m = f64::from(self.cell_size_m());
        Some(shoelace_twice(&edges) * 0.5 * cell_m * cell_m)
    }
    /// True when grid cell `at` (possibly off-grid) belongs to polygon `id`.
    fn owned_by(&self, at: [i64; 2], id: PolyId) -> bool {
        let (Ok(x), Ok(z)) = (u32::try_from(at[0]), u32::try_from(at[1])) else {
            return false;
        };
        if x >= self.width || z >= self.height {
            return false;
        }
        let index = u64::from(z) * u64::from(self.width) + u64::from(x);
        usize::try_from(index)
            .ok()
            .and_then(|i| self.cell_poly.get(i))
            == Some(&id.0)
    }
    /// Grid dims in cells `(width, height)`.
    pub fn grid_dims(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    /// Walkable cell count.
    pub fn walkable_cells(&self) -> usize {
        self.cell_poly.iter().filter(|p| **p != NO_POLY).count()
    }
    /// Polygon holding `point` (`[x, y, z]` meters; `y` is ignored).
    /// Out-of-grid points (NaN included) report [`NavError::OutOfBounds`];
    /// grid points on no walkable cell report [`NavError::NoPolygon`].
    pub fn polygon_at(&self, point: [f32; 3]) -> Result<PolyId, NavError> {
        let (cx, cz) = self.cell_of(point)?;
        match self.cell_poly[cz as usize * self.width as usize + cx as usize] {
            NO_POLY => Err(NavError::NoPolygon),
            id => Ok(PolyId(id)),
        }
    }
    /// Waypoint path in meters from `from` to `to`, inclusive of both ends.
    /// Walks polygons for reachability, then cells for waypoints; any
    /// unreachable or off-mesh endpoint errors instead of guessing.
    pub fn query(&self, from: [f32; 3], to: [f32; 3]) -> Result<Vec<[f32; 3]>, NavError> {
        let (sx, sz) = self.cell_of(from)?;
        let (gx, gz) = self.cell_of(to)?;
        let start = self.cell_poly[sz as usize * self.width as usize + sx as usize];
        let goal = self.cell_poly[gz as usize * self.width as usize + gx as usize];
        if start == NO_POLY || goal == NO_POLY {
            return Err(NavError::NoPolygon);
        }
        if !self.polys_connected(PolyId(start), PolyId(goal)) {
            return Err(NavError::Unreachable);
        }
        let cells = self
            .cell_path((sx, sz), (gx, gz))
            .ok_or(NavError::Unreachable)?;
        Ok(cells.iter().map(|c| self.cell_center(*c)).collect())
    }
    /// Polygon ids whose cells fall within `radius_m` of `center` (XZ).
    /// Empty matches are `Ok(vec![])`; only bad radii refuse.
    pub fn query_area(&self, center: [f32; 3], radius_m: f32) -> Result<Vec<PolyId>, NavError> {
        if !radius_m.is_finite() || !(0.0..=MAX_RADIUS_M).contains(&radius_m) {
            return Err(NavError::BadRadius { got: radius_m });
        }
        if !center.iter().all(|v| v.is_finite()) {
            return Err(NavError::OutOfBounds);
        }
        let mut found = Vec::new();
        let (ccx, ccz) = (f64::from(center[0]), f64::from(center[2]));
        // Scan the disc in grid order so output order is deterministic.
        let (min_x, min_z) = self.world_to_cell_f(center[0] - radius_m, center[2] - radius_m);
        let (max_x, max_z) = self.world_to_cell_f(center[0] + radius_m, center[2] + radius_m);
        let r2 = f64::from(radius_m).powi(2);
        for cz in min_z..=max_z {
            for cx in min_x..=max_x {
                if cx < 0 || cz < 0 || cx >= i64::from(self.width) || cz >= i64::from(self.height) {
                    continue;
                }
                let idx = cz as usize * self.width as usize + cx as usize;
                let id = self.cell_poly[idx];
                if id == NO_POLY {
                    continue;
                }
                let c = self.cell_center((cx as u32, cz as u32));
                let (ddx, ddz) = (f64::from(c[0]) - ccx, f64::from(c[2]) - ccz);
                if ddx * ddx + ddz * ddz <= r2 {
                    found.push(PolyId(id));
                }
            }
        }
        found.sort();
        found.dedup();
        Ok(found)
    }
    /// Canonical versioned bytes; identical geometry bakes identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut size = 32usize;
        for poly in &self.polygons {
            size += 13 + poly.cells.len() * 12 + poly.neighbors.len() * 4;
        }
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&MESH_MAGIC);
        out.extend_from_slice(&MESH_VERSION.to_le_bytes());
        out.extend_from_slice(&self.cell_mm.to_le_bytes());
        out.extend_from_slice(&self.origin_mm[0].to_le_bytes());
        out.extend_from_slice(&self.origin_mm[1].to_le_bytes());
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&(self.polygons.len() as u32).to_le_bytes());
        for poly in &self.polygons {
            out.extend_from_slice(&poly.id.0.to_le_bytes());
            out.push(poly.area);
            out.extend_from_slice(&(poly.cells.len() as u32).to_le_bytes());
            for cell in &poly.cells {
                out.extend_from_slice(&cell[0].to_le_bytes());
                out.extend_from_slice(&cell[1].to_le_bytes());
                let idx = cell[1] as usize * self.width as usize + cell[0] as usize;
                out.extend_from_slice(&self.cell_h_mm[idx].to_le_bytes());
            }
            out.extend_from_slice(&(poly.neighbors.len() as u32).to_le_bytes());
            for neighbor in &poly.neighbors {
                out.extend_from_slice(&neighbor.0.to_le_bytes());
            }
        }
        out
    }
    /// Decode canonical bytes; any non-bake blob refuses with its reason.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavError> {
        if bytes.len() > MAX_BYTES {
            return Err(NavError::TooLarge { got: bytes.len() });
        }
        let mut read = Cursor::new(bytes);
        let mut magic = [0u8; 6];
        for slot in magic.iter_mut() {
            *slot = read.byte()?;
        }
        if magic != MESH_MAGIC {
            return Err(NavError::BadMagic);
        }
        let version = read.u16()?;
        if version != MESH_VERSION {
            return Err(NavError::VersionMismatch { got: version });
        }
        let cell_mm = read.u32()?;
        if !(50..=5000).contains(&cell_mm) {
            return Err(NavError::NotCanonical("cell size"));
        }
        let origin_mm = [read.i32()?, read.i32()?];
        let (width, height) = (read.u32()?, read.u32()?);
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(NavError::NotCanonical("grid dims"));
        }
        let cells = width as usize * height as usize;
        if cells > MAX_CELLS {
            return Err(NavError::NotCanonical("grid cells"));
        }
        let count = read.u32()? as usize;
        if count == 0 || count > cells {
            return Err(NavError::NotCanonical("polygon count"));
        }
        let mut polygons = Vec::with_capacity(count);
        let mut cell_poly = vec![NO_POLY; cells];
        let mut cell_h_mm = vec![0i32; cells];
        for expect in 0..count as u32 {
            let id = read.u32()?;
            if id != expect {
                return Err(NavError::NotCanonical("polygon ids"));
            }
            let area = read.byte()?;
            let cell_count = read.u32()? as usize;
            if cell_count == 0 || cell_count > cells {
                return Err(NavError::NotCanonical("cell count"));
            }
            let mut poly_cells = Vec::with_capacity(cell_count);
            for _ in 0..cell_count {
                let (cx, cz) = (read.u32()?, read.u32()?);
                let h_mm = read.i32()?;
                if cx >= width || cz >= height {
                    return Err(NavError::NotCanonical("cell coords"));
                }
                let idx = cz as usize * width as usize + cx as usize;
                if cell_poly[idx] != NO_POLY {
                    return Err(NavError::NotCanonical("cell owner"));
                }
                if !poly_cells.is_empty() && [cx, cz] <= poly_cells[poly_cells.len() - 1] {
                    return Err(NavError::NotCanonical("cell order"));
                }
                poly_cells.push([cx, cz]);
                cell_poly[idx] = id;
                cell_h_mm[idx] = h_mm;
            }
            let neighbors = read_neighbors(&mut read, id, count)?;
            polygons.push(Polygon {
                id: PolyId(id),
                area,
                cells: poly_cells,
                neighbors,
            });
        }
        if read.remaining() != 0 {
            return Err(NavError::TrailingBytes {
                got: read.remaining(),
            });
        }
        Ok(Self {
            cell_mm,
            origin_mm,
            width,
            height,
            polygons,
            cell_poly,
            cell_h_mm,
        })
    }
    /// Stable `u64` over the canonical bytes (bake SHA gate).
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }
    /// Grid cell of a world point, refusing off-grid points (NaN included).
    fn cell_of(&self, point: [f32; 3]) -> Result<(u32, u32), NavError> {
        if !point.iter().all(|v| v.is_finite()) {
            return Err(NavError::OutOfBounds);
        }
        let fx =
            (f64::from(point[0]) * 1000.0 - f64::from(self.origin_mm[0])) / f64::from(self.cell_mm);
        let fz =
            (f64::from(point[2]) * 1000.0 - f64::from(self.origin_mm[1])) / f64::from(self.cell_mm);
        if fx < 0.0 || fz < 0.0 {
            return Err(NavError::OutOfBounds);
        }
        let (cx, cz) = (fx.floor() as i64, fz.floor() as i64);
        if cx >= i64::from(self.width) || cz >= i64::from(self.height) {
            return Err(NavError::OutOfBounds);
        }
        Ok((cx as u32, cz as u32))
    }
    /// World center of a grid cell in meters, carrying its baked height.
    fn cell_center(&self, cell: (u32, u32)) -> [f32; 3] {
        let idx = cell.1 as usize * self.width as usize + cell.0 as usize;
        [
            (f64::from(self.origin_mm[0]) + (f64::from(cell.0) + 0.5) * f64::from(self.cell_mm))
                as f32
                / MM_PER_M,
            self.cell_h_mm[idx] as f32 / MM_PER_M,
            (f64::from(self.origin_mm[1]) + (f64::from(cell.1) + 0.5) * f64::from(self.cell_mm))
                as f32
                / MM_PER_M,
        ]
    }
    /// True when both polygons join through neighbor edges.
    fn polys_connected(&self, from: PolyId, to: PolyId) -> bool {
        if from == to {
            return true;
        }
        let mut seen = vec![false; self.polygons.len()];
        let mut queue = VecDeque::new();
        seen[from.0 as usize] = true;
        queue.push_back(from);
        while let Some(current) = queue.pop_front() {
            for next in &self.polygons[current.0 as usize].neighbors {
                if *next == to {
                    return true;
                }
                if !seen[next.0 as usize] {
                    seen[next.0 as usize] = true;
                    queue.push_back(*next);
                }
            }
        }
        false
    }
    /// Deterministic 4-dir cell path over walkable cells, inclusive.
    fn cell_path(&self, start: (u32, u32), goal: (u32, u32)) -> Option<Vec<(u32, u32)>> {
        let width = self.width as usize;
        let index = |c: (u32, u32)| c.1 as usize * width + c.0 as usize;
        if start == goal {
            return Some(vec![start]);
        }
        let mut parent: Vec<Option<usize>> = vec![None; width * self.height as usize];
        let mut queue = VecDeque::new();
        parent[index(start)] = Some(index(start));
        queue.push_back(start);
        while let Some((cx, cy)) = queue.pop_front() {
            for (nx, ny) in [
                (cx.wrapping_add(1), cy),
                (cx.wrapping_sub(1), cy),
                (cx, cy.wrapping_add(1)),
                (cx, cy.wrapping_sub(1)),
            ] {
                if nx >= self.width || ny >= self.height {
                    continue;
                }
                let next = index((nx, ny));
                if parent[next].is_some() || self.cell_poly[next] == NO_POLY {
                    continue;
                }
                parent[next] = Some(index((cx, cy)));
                queue.push_back((nx, ny));
            }
        }
        parent[index(goal)]?;
        let mut rev = vec![goal];
        let mut at = index(goal);
        while at != index(start) {
            at = parent[at].expect("parent chain holds to start");
            rev.push(((at % width) as u32, (at / width) as u32));
        }
        rev.reverse();
        Some(rev)
    }
    /// World position to unclamped grid coords for area scans.
    fn world_to_cell_f(&self, x_m: f32, z_m: f32) -> (i64, i64) {
        let fx = (f64::from(x_m) * 1000.0 - f64::from(self.origin_mm[0])) / f64::from(self.cell_mm);
        let fz = (f64::from(z_m) * 1000.0 - f64::from(self.origin_mm[1])) / f64::from(self.cell_mm);
        (fx.floor() as i64, fz.floor() as i64)
    }
}

impl From<Truncated> for NavError {
    fn from(_: Truncated) -> Self {
        Self::Truncated
    }
}

/// Stacked floors: each floor bakes alone so shared XZ never merges.
/// PORT(lunaris@56cc7e30:NAVIGATION bridge multi-level). No live caller:
/// rebake.rs `rebake_mesh_tile` takes a plain `NavMesh`, never this stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackedNav {
    levels: Vec<NavMesh>,
}

impl StackedNav {
    /// Bake one mesh per floor in order; floor plus [`bake`] order decide bytes.
    pub fn bake_stacked(
        level_tris: &[Vec<[[f32; 3]; 3]>],
        config: &BakeConfig,
    ) -> Result<Self, NavError> {
        if level_tris.is_empty() {
            return Err(NavError::EmptyInput);
        }
        if level_tris.len() > 64 {
            return Err(NavError::TooManyTriangles {
                got: level_tris.len(),
                max: 64,
            });
        }
        let mut levels = Vec::with_capacity(level_tris.len());
        for tris in level_tris {
            levels.push(bake(tris, config)?);
        }
        for lvl in &levels {
            MeshTiles::from_mesh(lvl, MESH_TILE_EDGE)?;
        }
        Ok(Self { levels })
    }
    /// Floor count.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }
    /// Floors in bake order.
    pub fn levels(&self) -> &[NavMesh] {
        &self.levels
    }
    /// Route ground to roof through `portal` XZ; same floor delegates to [`NavMesh::query`].
    /// Errors: [`NavError::OutOfBounds`] past the stack, else floor query.
    pub fn query_stacked(
        &self,
        from_level: usize,
        from: [f32; 3],
        to_level: usize,
        to: [f32; 3],
        portal: [f32; 3],
    ) -> Result<Vec<[f32; 3]>, NavError> {
        if from_level >= self.levels.len() || to_level >= self.levels.len() {
            return Err(NavError::OutOfBounds);
        }
        if from_level == to_level {
            return self.levels[from_level].query(from, to);
        }
        let mut path = self.levels[from_level].query(from, portal)?;
        if from_level < to_level {
            for mid in from_level + 1..to_level {
                let spine = self.levels[mid].query(portal, portal)?;
                path.extend(spine);
            }
        } else {
            for mid in (to_level + 1..from_level).rev() {
                let spine = self.levels[mid].query(portal, portal)?;
                path.extend(spine);
            }
        }
        let mut tail = self.levels[to_level].query(portal, to)?;
        if path.last() == tail.first() {
            tail.remove(0);
        }
        path.extend(tail);
        Ok(path)
    }
    /// Canonical bytes: count plus per-floor length plus floor bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let count = u32::try_from(self.levels.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&count.to_le_bytes());
        for lvl in &self.levels {
            let bytes = lvl.to_bytes();
            let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&bytes);
        }
        out
    }
    /// Decode canonical bytes; any non-stack blob refuses with its reason.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavError> {
        if bytes.len() > MAX_BYTES {
            return Err(NavError::TooLarge { got: bytes.len() });
        }
        let mut read = Cursor::new(bytes);
        let raw_count = read.u32()?;
        let count =
            usize::try_from(raw_count).map_err(|_| NavError::NotCanonical("level count"))?;
        if count == 0 || count > 64 {
            return Err(NavError::NotCanonical("level count"));
        }
        let mut levels = Vec::with_capacity(count);
        for _ in 0..count {
            let raw_len = read.u32()?;
            let len =
                usize::try_from(raw_len).map_err(|_| NavError::NotCanonical("level bytes"))?;
            if len == 0 || len > MAX_BYTES {
                return Err(NavError::NotCanonical("level bytes"));
            }
            let slice = read.take(len)?;
            levels.push(NavMesh::from_bytes(slice)?);
        }
        if read.remaining() != 0 {
            return Err(NavError::TrailingBytes {
                got: read.remaining(),
            });
        }
        Ok(Self { levels })
    }
    /// Stable u64 over the canonical bytes.
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }
}

/// Tile edge in cells for versioned mesh tiles; one NV1 generation per tile.
/// Built by `MeshTiles::from_mesh` (whose one call site discards the table);
/// nothing seeds a live tiling from it today.
pub const MESH_TILE_EDGE: u32 = 8;
/// Magic at the head of a tile-version blob; mesh bytes stay `E2NAV1`.
const TILE_MAGIC: [u8; 6] = *b"E2MTL1";

/// Per-tile NV1 generations stitched on shared edges; versions ride mesh bytes.
/// Owner nav, consumer rebake.rs follow-up. Units tile ids plus versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshTiles {
    edge: u32,
    cols: u32,
    rows: u32,
    versions: Vec<u32>,
}

impl MeshTiles {
    /// Tile layout over mesh; versions start at zero. Errors: bad edge/coverage.
    pub fn from_mesh(mesh: &NavMesh, edge: u32) -> Result<Self, NavError> {
        if edge == 0 || edge > MAX_SIDE {
            return Err(NavError::NotCanonical("tile edge"));
        }
        let (w, h) = mesh.grid_dims();
        let (cols, rows) = (w.div_ceil(edge), h.div_ceil(edge));
        let count = usize::try_from(cols)
            .map_err(|_| NavError::NotCanonical("tile count"))?
            .checked_mul(usize::try_from(rows).map_err(|_| NavError::NotCanonical("tile count"))?)
            .ok_or(NavError::NotCanonical("tile count"))?;
        if count == 0 || count > MAX_CELLS {
            return Err(NavError::NotCanonical("tile count"));
        }
        Ok(Self {
            edge,
            cols,
            rows,
            versions: vec![0; count],
        })
    }
    /// Version of one tile. Errors: past-the-table tile.
    pub fn version(&self, tile: u32) -> Result<u32, NavError> {
        let index = usize::try_from(tile).map_err(|_| NavError::NotCanonical("tile id"))?;
        self.versions
            .get(index)
            .copied()
            .ok_or(NavError::NotCanonical("tile id"))
    }
    /// Canonical bytes; identical tables bake identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&TILE_MAGIC);
        out.extend_from_slice(&MESH_VERSION.to_le_bytes());
        out.extend_from_slice(&self.edge.to_le_bytes());
        out.extend_from_slice(&self.cols.to_le_bytes());
        out.extend_from_slice(&self.rows.to_le_bytes());
        for v in &self.versions {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
    /// Decode canonical bytes; any non-tile blob refuses with its reason.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NavError> {
        if bytes.len() > MAX_BYTES {
            return Err(NavError::TooLarge { got: bytes.len() });
        }
        let mut read = Cursor::new(bytes);
        let mut magic = [0u8; 6];
        for s in magic.iter_mut() {
            *s = read.byte()?;
        }
        if magic != TILE_MAGIC {
            return Err(NavError::BadMagic);
        }
        let ver = read.u16()?;
        if ver != MESH_VERSION {
            return Err(NavError::VersionMismatch { got: ver });
        }
        let (edge, cols, rows) = (read.u32()?, read.u32()?, read.u32()?);
        if edge == 0 || edge > MAX_SIDE || cols == 0 || rows == 0 {
            return Err(NavError::NotCanonical("tile dims"));
        }
        let count = usize::try_from(cols)
            .map_err(|_| NavError::NotCanonical("tile count"))?
            .checked_mul(usize::try_from(rows).map_err(|_| NavError::NotCanonical("tile count"))?)
            .ok_or(NavError::NotCanonical("tile count"))?;
        if count == 0 || count > MAX_CELLS {
            return Err(NavError::NotCanonical("tile count"));
        }
        let mut versions = Vec::with_capacity(count);
        for _ in 0..count {
            versions.push(read.u32()?);
        }
        if read.remaining() != 0 {
            return Err(NavError::TrailingBytes {
                got: read.remaining(),
            });
        }
        Ok(Self {
            edge,
            cols,
            rows,
            versions,
        })
    }
}

/// Neighbor ids of polygon `id` out of `count`: strictly ascending, in
/// range and never the polygon itself, so adjacency has one canonical form.
fn read_neighbors(read: &mut Cursor<'_>, id: u32, count: usize) -> Result<Vec<PolyId>, NavError> {
    let neighbor_count = read.u32()? as usize;
    if neighbor_count >= count {
        return Err(NavError::NotCanonical("neighbor count"));
    }
    let mut neighbors: Vec<PolyId> = Vec::with_capacity(neighbor_count);
    for _ in 0..neighbor_count {
        let neighbor = PolyId(read.u32()?);
        if neighbor.0 >= count as u32 || neighbor.0 == id {
            return Err(NavError::NotCanonical("neighbor id"));
        }
        if neighbors.last().is_some_and(|last| neighbor <= *last) {
            return Err(NavError::NotCanonical("neighbor order"));
        }
        neighbors.push(neighbor);
    }
    Ok(neighbors)
}

/// Refuse bad configs before any geometry work.
fn check_config(config: &BakeConfig) -> Result<(), NavError> {
    if !config.cell_size.is_finite() || !(0.05..=5.0).contains(&config.cell_size) {
        return Err(NavError::BadCellSize {
            got: config.cell_size,
        });
    }
    if !config.walkable_normal_y.is_finite()
        || !(0.0..=1.0).contains(&config.walkable_normal_y)
        || config.walkable_normal_y <= 0.0
    {
        return Err(NavError::BadSlope {
            got: config.walkable_normal_y,
        });
    }
    Ok(())
}

/// Rounds a millimeter value to `i32`, or `None` when it is NaN, infinite or
/// outside `i32`. `as` would turn NaN into 0 and clamp the rest silently.
fn round_mm(value: f64) -> Option<i32> {
    let rounded = value.round();
    // Every comparison with NaN is false, so NaN never passes the range test.
    (f64::from(i32::MIN)..=f64::from(i32::MAX))
        .contains(&rounded)
        .then_some(rounded as i32)
}

/// Quantize one triangle to integer millimeters, refusing bad input.
fn quantize_tri(tri: &[[f32; 3]; 3], index: usize) -> Result<[i32; 9], NavError> {
    for vertex in tri {
        if !vertex.iter().all(|v| v.is_finite()) {
            return Err(NavError::NonFiniteTri { tri: index });
        }
        if !vertex.iter().all(|v| v.abs() <= MAX_COORD_M) {
            return Err(NavError::VertexOutOfRange { tri: index });
        }
    }
    if tri_area2(tri) <= DEGENERATE_AREA2 {
        return Err(NavError::DegenerateTri { tri: index });
    }
    let mut out = [0i32; 9];
    for (slot, value) in out.iter_mut().zip(tri.iter().flatten()) {
        // Finite and in bound after the checks above, so only NaN could fail
        // here, and it refuses instead of quantizing to 0.
        *slot =
            round_mm(f64::from(*value) * 1000.0).ok_or(NavError::NonFiniteTri { tri: index })?;
    }
    Ok(out)
}

/// Twice the triangle area in m^2 (zero means degenerate).
fn tri_area2(tri: &[[f32; 3]; 3]) -> f32 {
    let (ax, ay, az) = (
        tri[1][0] - tri[0][0],
        tri[1][1] - tri[0][1],
        tri[1][2] - tri[0][2],
    );
    let (bx, by, bz) = (
        tri[2][0] - tri[0][0],
        tri[2][1] - tri[0][1],
        tri[2][2] - tri[0][2],
    );
    let (cx, cy, cz) = (ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx);
    (cx * cx + cy * cy + cz * cz).sqrt()
}

/// Twice the signed area enclosed by directed edges (shoelace formula).
/// Counter-clockwise loops are positive. Exact for whole-number coordinates.
fn shoelace_twice(edges: &[([f64; 2], [f64; 2])]) -> f64 {
    edges.iter().map(|&(p, q)| p[0] * q[1] - q[0] * p[1]).sum()
}

/// Walkable slope of a quantized triangle as its normal Y in `[-1, 1]`.
fn tri_normal_y(quant: &[i32; 9]) -> f32 {
    let (ax, ay, az) = (
        f64::from(quant[3] - quant[0]),
        f64::from(quant[4] - quant[1]),
        f64::from(quant[5] - quant[2]),
    );
    let (bx, by, bz) = (
        f64::from(quant[6] - quant[0]),
        f64::from(quant[7] - quant[1]),
        f64::from(quant[8] - quant[2]),
    );
    let (cx, cy, cz) = (ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx);
    let len = (cx * cx + cy * cy + cz * cz).sqrt();
    if len <= 0.0 {
        return -1.0;
    }
    (cy / len) as f32
}

/// Grid origin and dims covering the walkable triangles, in integer mm.
fn grid_span(walkable: &[[i32; 9]], cell_mm: u32) -> Result<([i32; 2], u32, u32), NavError> {
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;
    let mut min_z = i32::MAX;
    let mut max_z = i32::MIN;
    for quant in walkable {
        for vertex in [
            [quant[0], quant[2]],
            [quant[3], quant[5]],
            [quant[6], quant[8]],
        ] {
            min_x = min_x.min(vertex[0]);
            max_x = max_x.max(vertex[0]);
            min_z = min_z.min(vertex[1]);
            max_z = max_z.max(vertex[1]);
        }
    }
    // Spans are taken in i64 so the subtraction cannot overflow. A side that
    // does not fit u32 saturates to u32::MAX, which the side bound refuses.
    let cell = i64::from(cell_mm);
    let width = u32::try_from((i64::from(max_x) - i64::from(min_x)) / cell + 1).unwrap_or(u32::MAX);
    let height =
        u32::try_from((i64::from(max_z) - i64::from(min_z)) / cell + 1).unwrap_or(u32::MAX);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(NavError::GridTooLarge {
            w: width,
            h: height,
            max_side: MAX_SIDE,
        });
    }
    if width as usize * height as usize > MAX_CELLS {
        return Err(NavError::GridTooLarge {
            w: width,
            h: height,
            max_side: MAX_SIDE,
        });
    }
    Ok(([min_x, min_z], width, height))
}

/// Rasterize walkable triangles onto the grid; heights average in tri order.
fn rasterize(
    walkable: &[[i32; 9]],
    origin_mm: [i32; 2],
    width: u32,
    height: u32,
    cell_mm: u32,
) -> (Vec<u32>, Vec<i32>) {
    let cells = width as usize * height as usize;
    let mut owner = vec![NO_POLY; cells];
    let mut height_sum = vec![0i64; cells];
    let mut height_count = vec![0u32; cells];
    for quant in walkable {
        let tri_f = quant_to_tri_f(quant);
        let (lo_x, hi_x) = tri_cell_span(
            [tri_f[0][0], tri_f[1][0], tri_f[2][0]],
            origin_mm[0],
            cell_mm,
            width,
        );
        let (lo_z, hi_z) = tri_cell_span(
            [tri_f[0][2], tri_f[1][2], tri_f[2][2]],
            origin_mm[1],
            cell_mm,
            height,
        );
        for cz in lo_z..=hi_z {
            for cx in lo_x..=hi_x {
                let center = [
                    f64::from(origin_mm[0]) + (f64::from(cx) + 0.5) * f64::from(cell_mm),
                    f64::from(origin_mm[1]) + (f64::from(cz) + 0.5) * f64::from(cell_mm),
                ];
                if let Some(height_mm) = sample_height(&tri_f, center) {
                    let idx = cz as usize * width as usize + cx as usize;
                    owner[idx] = 0;
                    height_sum[idx] += i64::from(height_mm);
                    height_count[idx] += 1;
                }
            }
        }
    }
    let heights = height_sum
        .iter()
        .zip(height_count.iter())
        .map(|(sum, count)| {
            if *count == 0 {
                0
            } else {
                (sum / i64::from(*count)) as i32
            }
        })
        .collect();
    (owner, heights)
}

/// Cell span overlapped by one triangle axis, clamped to the grid.
fn tri_cell_span(values_mm: [f64; 3], origin_mm: i32, cell_mm: u32, dims: u32) -> (u32, u32) {
    let lo = values_mm[0].min(values_mm[1]).min(values_mm[2]);
    let hi = values_mm[0].max(values_mm[1]).max(values_mm[2]);
    let to_cell = |v: f64| ((v - f64::from(origin_mm)) / f64::from(cell_mm)).floor() as i64;
    let lo_c = to_cell(lo).clamp(0, i64::from(dims) - 1);
    let hi_c = to_cell(hi).clamp(0, i64::from(dims) - 1);
    (lo_c as u32, hi_c.max(lo_c) as u32)
}

/// Quantized triangle back to float millimeters for raster tests.
fn quant_to_tri_f(quant: &[i32; 9]) -> [[f64; 3]; 3] {
    [
        [
            f64::from(quant[0]),
            f64::from(quant[1]),
            f64::from(quant[2]),
        ],
        [
            f64::from(quant[3]),
            f64::from(quant[4]),
            f64::from(quant[5]),
        ],
        [
            f64::from(quant[6]),
            f64::from(quant[7]),
            f64::from(quant[8]),
        ],
    ]
}

/// Height sample when the XZ center falls inside the triangle, else none.
/// A non-finite height gives none too, so the cell stays unwalkable.
fn sample_height(tri: &[[f64; 3]; 3], center_mm: [f64; 2]) -> Option<i32> {
    let (ax, az) = (tri[0][0], tri[0][2]);
    let (bx, bz) = (tri[1][0], tri[1][2]);
    let (cx, cz) = (tri[2][0], tri[2][2]);
    let denom = (bz - cz) * (ax - cx) + (cx - bx) * (az - cz);
    if denom.abs() < 1e-9 {
        return None;
    }
    let px = center_mm[0];
    let pz = center_mm[1];
    let u = ((bz - cz) * (px - cx) + (cx - bx) * (pz - cz)) / denom;
    let v = ((cz - az) * (px - cx) + (ax - cx) * (pz - cz)) / denom;
    let w = 1.0 - u - v;
    if u < -1e-9 || v < -1e-9 || w < -1e-9 {
        return None;
    }
    // A NaN center or corner height passes the bounds above; it leaves the
    // cell unsampled instead of rounding to a height of 0.
    round_mm(u * tri[0][1] + v * tri[1][1] + w * tri[2][1])
}

/// Label 4-connected walkable cells by BFS from the lowest unlabeled cell
/// index; returns the region per cell (`u32::MAX` for unwalkable cells) and
/// the region count. Index-order starts keep the labels deterministic.
fn flood_regions(owner: &[u32], cells: usize, width: u32, height: u32) -> (Vec<u32>, u32) {
    let mut region = vec![u32::MAX; cells];
    let mut region_count = 0u32;
    for start in 0..cells {
        if owner[start] == NO_POLY || region[start] != u32::MAX {
            continue;
        }
        let mut queue = VecDeque::new();
        region[start] = region_count;
        queue.push_back(start);
        while let Some(at) = queue.pop_front() {
            let (ax, az) = ((at % width as usize) as u32, (at / width as usize) as u32);
            for (nx, nz) in [
                (ax.wrapping_add(1), az),
                (ax.wrapping_sub(1), az),
                (ax, az.wrapping_add(1)),
                (ax, az.wrapping_sub(1)),
            ] {
                if nx >= width || nz >= height {
                    continue;
                }
                let next = nz as usize * width as usize + nx as usize;
                if owner[next] == NO_POLY || region[next] != u32::MAX {
                    continue;
                }
                region[next] = region_count;
                queue.push_back(next);
            }
        }
        region_count += 1;
    }
    (region, region_count)
}

/// Flood-fill walkable cells into stable polygons with sorted adjacency.
fn assemble(
    cell_mm: u32,
    origin_mm: [i32; 2],
    width: u32,
    height: u32,
    owner: Vec<u32>,
    heights: Vec<i32>,
) -> NavMesh {
    let cells = width as usize * height as usize;
    let (region, region_count) = flood_regions(&owner, cells, width, height);
    // Stable ids: regions ordered by minimum cell index.
    let mut minima = vec![usize::MAX; region_count as usize];
    for (cell, owns) in region.iter().enumerate() {
        if *owns != u32::MAX && cell < minima[*owns as usize] {
            minima[*owns as usize] = cell;
        }
    }
    let mut remap_order: Vec<u32> = (0..region_count).collect();
    remap_order.sort_by_key(|r| minima[*r as usize]);
    let mut remap = vec![0u32; region_count as usize];
    for (stable, raw) in remap_order.iter().enumerate() {
        remap[*raw as usize] = stable as u32;
    }
    let mut poly_cells: Vec<Vec<[u32; 2]>> = vec![Vec::new(); region_count as usize];
    for (cell, owns) in region.iter().enumerate() {
        if *owns == u32::MAX {
            continue;
        }
        let stable = remap[*owns as usize] as usize;
        poly_cells[stable].push([
            (cell % width as usize) as u32,
            (cell / width as usize) as u32,
        ]);
    }
    let mut neighbors: Vec<Vec<PolyId>> = vec![Vec::new(); region_count as usize];
    for (cell, owns) in region.iter().enumerate() {
        if *owns == u32::MAX {
            continue;
        }
        let (ax, az) = (
            (cell % width as usize) as u32,
            (cell / width as usize) as u32,
        );
        for (nx, nz) in [(ax.wrapping_add(1), az), (ax, az.wrapping_add(1))] {
            if nx >= width || nz >= height {
                continue;
            }
            let next = nz as usize * width as usize + nx as usize;
            if region[next] == u32::MAX || region[next] == *owns {
                continue;
            }
            let (a, b) = (remap[*owns as usize], remap[region[next] as usize]);
            neighbors[a as usize].push(PolyId(b));
            neighbors[b as usize].push(PolyId(a));
        }
    }
    let mut polygons = Vec::with_capacity(region_count as usize);
    let mut cell_poly = vec![NO_POLY; cells];
    for (stable, cells_of) in poly_cells.iter_mut().enumerate() {
        cells_of.sort_unstable();
        let mut edge = neighbors[stable].clone();
        edge.sort();
        edge.dedup();
        for cell in cells_of.iter() {
            cell_poly[cell[1] as usize * width as usize + cell[0] as usize] = stable as u32;
        }
        polygons.push(Polygon {
            id: PolyId(stable as u32),
            area: 0,
            cells: cells_of.clone(),
            neighbors: edge,
        });
    }
    NavMesh {
        cell_mm,
        origin_mm,
        width,
        height,
        polygons,
        cell_poly,
        cell_h_mm: heights,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BakeConfig {
        BakeConfig {
            cell_size: 0.5,
            walkable_normal_y: 0.7,
        }
    }

    /// Flat quad slab over `[x0, x1] x [z0, z1]` at height `y`, meters.
    /// Wound counter-clockwise seen from above so normals face `+Y`.
    fn slab(x0: f32, x1: f32, z0: f32, z1: f32, y: f32) -> Vec<[[f32; 3]; 3]> {
        vec![
            [[x0, y, z0], [x0, y, z1], [x1, y, z1]],
            [[x0, y, z0], [x1, y, z1], [x1, y, z0]],
        ]
    }

    #[test]
    fn bytes_identical_twice_and_round_trip() {
        let tris = slab(0.0, 4.0, 0.0, 4.0, 0.0);
        let first = bake(&tris, &config()).expect("floor bakes");
        let second = bake(&tris, &config()).expect("floor bakes");
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(first.digest(), second.digest());
        let back = NavMesh::from_bytes(&first.to_bytes()).expect("canonical decodes");
        assert_eq!(back, first);
        assert_eq!(back.to_bytes(), first.to_bytes());
    }

    #[test]
    fn shuffled_input_bakes_identical_bytes() {
        let mut tris = slab(0.0, 4.0, 0.0, 4.0, 0.0);
        tris.extend(slab(6.0, 8.0, 6.0, 8.0, 0.0));
        let forward = bake(&tris, &config()).expect("islands bake");
        let mut shuffled = tris.clone();
        shuffled.reverse();
        let backward = bake(&shuffled, &config()).expect("reversed bakes");
        assert_eq!(forward.to_bytes(), backward.to_bytes());
        assert_eq!(forward.polygon_count(), 2);
    }

    #[test]
    fn polygons_sorted_with_stable_ids() {
        // Far island first in input; ids still follow minimum cell order.
        let mut tris = slab(6.0, 8.0, 6.0, 8.0, 0.0);
        tris.extend(slab(0.0, 2.0, 0.0, 2.0, 0.0));
        let mesh = bake(&tris, &config()).expect("islands bake");
        assert_eq!(mesh.polygon_count(), 2);
        let ids: Vec<u32> = mesh.polygons().iter().map(|p| p.id().0).collect();
        assert_eq!(ids, vec![0, 1]);
        for poly in mesh.polygons() {
            let mut sorted = poly.cells().to_vec();
            sorted.sort_unstable();
            assert_eq!(poly.cells(), sorted.as_slice(), "cells sorted");
            assert!(poly.neighbors().is_empty(), "islands share no edge");
        }
        // Near slab owns the low cells, so it is polygon zero.
        assert_eq!(mesh.polygon_at([0.5, 0.0, 0.5]), Ok(PolyId(0)));
        assert_eq!(mesh.polygon_at([7.0, 0.0, 7.0]), Ok(PolyId(1)));
    }

    #[test]
    fn query_walks_connected_floor() {
        let mesh = bake(&slab(0.0, 4.0, 0.0, 4.0, 0.0), &config()).expect("floor bakes");
        let path = mesh
            .query([0.25, 0.0, 0.25], [3.75, 0.0, 3.75])
            .expect("floor connects");
        assert!(path.len() >= 2);
        let (first, last) = (path[0], path[path.len() - 1]);
        assert!((first[0] - 0.25).abs() <= 0.26 && (first[2] - 0.25).abs() <= 0.26);
        assert!((last[0] - 3.75).abs() <= 0.26 && (last[2] - 3.75).abs() <= 0.26);
        for step in &path {
            assert!(mesh.polygon_at(*step).is_ok(), "every step is walkable");
        }
        for window in path.windows(2) {
            let dx = (window[0][0] - window[1][0]).abs();
            let dz = (window[0][2] - window[1][2]).abs();
            assert!(dx + dz <= 0.51, "steps move one cell: {window:?}");
        }
    }

    #[test]
    fn query_ramps_carry_height() {
        // Ramp rising from y=0 at x=0 to y=1 at x=4, flat in z, facing +Y.
        let tris = vec![
            [[0.0, 0.0, 0.0], [4.0, 1.0, 2.0], [4.0, 1.0, 0.0]],
            [[0.0, 0.0, 0.0], [0.0, 0.0, 2.0], [4.0, 1.0, 2.0]],
        ];
        let mesh = bake(&tris, &config()).expect("ramp bakes");
        let path = mesh
            .query([0.25, 0.0, 1.0], [3.75, 0.0, 1.0])
            .expect("ramp connects");
        assert!(path.len() >= 2);
        assert!(
            path[path.len() - 1][1] > path[0][1] + 0.5,
            "waypoints climb the ramp: {path:?}"
        );
    }

    #[test]
    fn unreachable_never_guesses() {
        let mut tris = slab(0.0, 2.0, 0.0, 2.0, 0.0);
        tris.extend(slab(6.0, 8.0, 6.0, 8.0, 0.0));
        let mesh = bake(&tris, &config()).expect("islands bake");
        assert_eq!(
            mesh.query([0.5, 0.0, 0.5], [7.0, 0.0, 7.0]),
            Err(NavError::Unreachable)
        );
        // Off-grid goal refuses instead of snapping to the far island.
        assert_eq!(
            mesh.query([0.5, 0.0, 0.5], [50.0, 0.0, 50.0]),
            Err(NavError::OutOfBounds)
        );
        // Gap between islands is grid-inside but walkable nowhere.
        assert_eq!(mesh.polygon_at([4.0, 0.0, 4.0]), Err(NavError::NoPolygon));
        assert_eq!(
            mesh.polygon_at([50.0, 0.0, 50.0]),
            Err(NavError::OutOfBounds)
        );
        assert_eq!(
            mesh.polygon_at([f32::NAN, 0.0, 0.0]),
            Err(NavError::OutOfBounds)
        );
    }

    #[test]
    fn degenerate_and_empty_inputs_refuse() {
        assert_eq!(bake(&[], &config()), Err(NavError::EmptyInput));
        let flat = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]];
        assert_eq!(
            bake(&flat, &config()),
            Err(NavError::DegenerateTri { tri: 0 })
        );
        let nan = [[[f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]];
        assert_eq!(
            bake(&nan, &config()),
            Err(NavError::NonFiniteTri { tri: 0 })
        );
        // Second triangle degenerate reports its own caller index.
        let mut two = slab(0.0, 2.0, 0.0, 2.0, 0.0);
        two.push([[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]);
        assert_eq!(
            bake(&two, &config()),
            Err(NavError::DegenerateTri { tri: 2 })
        );
    }

    #[test]
    fn nan_inputs_never_round_to_zero() {
        // NaN and infinities have no integer; `as` would make NaN a zero.
        assert_eq!(round_mm(f64::NAN), None);
        assert_eq!(round_mm(f64::INFINITY), None);
        assert_eq!(round_mm(f64::from(i32::MAX) + 1.0), None);
        assert_eq!(round_mm(-2.4), Some(-2));
        assert_eq!(round_mm(2.5), Some(3));
        // Each bake entry point refuses NaN instead of baking a zero.
        let tris = slab(0.0, 1.0, 0.0, 1.0, 0.0);
        let nan_cell = BakeConfig {
            cell_size: f32::NAN,
            ..config()
        };
        assert!(matches!(
            bake(&tris, &nan_cell),
            Err(NavError::BadCellSize { got }) if got.is_nan()
        ));
        let nan_slope = BakeConfig {
            walkable_normal_y: f32::NAN,
            ..config()
        };
        assert!(matches!(
            bake(&tris, &nan_slope),
            Err(NavError::BadSlope { .. })
        ));
        let nan_z = [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, f32::NAN]]];
        assert_eq!(
            bake(&nan_z, &config()),
            Err(NavError::NonFiniteTri { tri: 0 })
        );
        // A NaN sample center or NaN corner height leaves the cell unsampled,
        // never a silent height of 0. The finite sample still answers.
        let flat = quant_to_tri_f(&[0, 500, 0, 1000, 500, 0, 0, 500, 1000]);
        assert_eq!(sample_height(&flat, [100.0, 100.0]), Some(500));
        assert_eq!(sample_height(&flat, [f64::NAN, 100.0]), None);
        let nan_corner = [
            [0.0, f64::NAN, 0.0],
            [1000.0, 500.0, 0.0],
            [0.0, 500.0, 1000.0],
        ];
        assert_eq!(sample_height(&nan_corner, [100.0, 100.0]), None);
    }

    #[test]
    fn grid_span_refuses_an_overflowing_span_and_sizes_a_narrow_one() {
        // x runs from i32::MIN to i32::MAX: the old i32 subtraction overflowed,
        // and the side it implies is past the bound either way.
        let wide = [[i32::MIN, 0, 0, i32::MAX, 0, 0, 0, 0, 0]];
        assert!(matches!(
            grid_span(&wide, 1),
            Err(NavError::GridTooLarge {
                max_side: MAX_SIDE,
                ..
            })
        ));
        // A 2000 mm span at 500 mm cells is exactly 5 cells across.
        let narrow = [[0, 0, 0, 2000, 0, 0, 0, 0, 0]];
        assert_eq!(grid_span(&narrow, 500), Ok(([0, 0], 5, 1)));
    }

    #[test]
    fn steep_only_geometry_has_nowhere_to_walk() {
        // Vertical quad wall: finite and sound, but nothing walkable.
        let wall = [[[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 2.0, 2.0]]];
        assert_eq!(bake(&wall, &config()), Err(NavError::NoWalkable));
    }

    #[test]
    fn bounds_refuse_loudly() {
        let tris = slab(0.0, 1.0, 0.0, 1.0, 0.0);
        assert_eq!(
            bake(
                &tris,
                &BakeConfig {
                    cell_size: 0.0,
                    ..config()
                }
            ),
            Err(NavError::BadCellSize { got: 0.0 })
        );
        assert_eq!(
            bake(
                &tris,
                &BakeConfig {
                    walkable_normal_y: 0.0,
                    ..config()
                }
            ),
            Err(NavError::BadSlope { got: 0.0 })
        );
        let big: Vec<[[f32; 3]; 3]> =
            vec![[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]; MAX_TRIS + 1];
        assert_eq!(
            bake(&big, &config()),
            Err(NavError::TooManyTriangles {
                got: MAX_TRIS + 1,
                max: MAX_TRIS
            })
        );
        // Two sound slabs 600 m apart need more cells than the side bound.
        let mut wide = slab(0.0, 1.0, 0.0, 1.0, 0.0);
        wide.extend(slab(600.0, 601.0, 0.0, 1.0, 0.0));
        assert!(matches!(
            bake(&wide, &config()),
            Err(NavError::GridTooLarge { .. })
        ));
        let mesh = bake(&tris, &config()).expect("slab bakes");
        assert_eq!(
            mesh.query_area([0.5, 0.0, 0.5], -1.0),
            Err(NavError::BadRadius { got: -1.0 })
        );
        assert!(matches!(
            mesh.query_area([0.5, 0.0, 0.5], f32::NAN),
            Err(NavError::BadRadius { .. })
        ));
    }

    #[test]
    fn wire_refusals_name_their_cause() {
        let mesh = bake(&slab(0.0, 2.0, 0.0, 2.0, 0.0), &config()).expect("slab bakes");
        let bytes = mesh.to_bytes();
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert_eq!(NavMesh::from_bytes(&bad_magic), Err(NavError::BadMagic));
        let mut bad_version = bytes.clone();
        bad_version[6] = 0x7f;
        assert!(matches!(
            NavMesh::from_bytes(&bad_version),
            Err(NavError::VersionMismatch { .. })
        ));
        assert_eq!(
            NavMesh::from_bytes(&bytes[..bytes.len() / 2]),
            Err(NavError::Truncated)
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            NavMesh::from_bytes(&trailing),
            Err(NavError::TrailingBytes { got: 1 })
        );
        // Flipping the first cell's x breaks canonical cell order/ownership.
        let mut bent = bytes.clone();
        bent[41] ^= 0x01;
        assert!(NavMesh::from_bytes(&bent).is_err());
    }

    #[test]
    fn area_query_covers_only_its_disc() {
        let mut tris = slab(0.0, 2.0, 0.0, 2.0, 0.0);
        tris.extend(slab(6.0, 8.0, 6.0, 8.0, 0.0));
        let mesh = bake(&tris, &config()).expect("islands bake");
        assert_eq!(mesh.query_area([1.0, 0.0, 1.0], 1.5), Ok(vec![PolyId(0)]));
        assert_eq!(
            mesh.query_area([1.0, 0.0, 1.0], 10.0),
            Ok(vec![PolyId(0), PolyId(1)])
        );
        assert_eq!(mesh.query_area([4.0, 0.0, 4.0], 0.4), Ok(vec![]));
    }

    #[test]
    fn wall_gap_splits_one_floor_into_two() {
        // Left and right slabs with a one-meter unwalkable gap between.
        let mut tris = slab(0.0, 2.0, 0.0, 4.0, 0.0);
        tris.extend(slab(3.0, 5.0, 0.0, 4.0, 0.0));
        let split = bake(&tris, &config()).expect("split floor bakes");
        assert_eq!(split.polygon_count(), 2);
        assert_eq!(
            split.query([0.5, 0.0, 2.0], [4.5, 0.0, 2.0]),
            Err(NavError::Unreachable)
        );
        // Bridging the gap rejoins one polygon and the query walks it.
        tris.extend(slab(2.0, 3.0, 1.5, 2.5, 0.0));
        let whole = bake(&tris, &config()).expect("bridged floor bakes");
        assert_eq!(whole.polygon_count(), 1);
        assert!(whole.query([0.5, 0.0, 2.0], [4.5, 0.0, 2.0]).is_ok());
    }

    /// Mesh of `width` x `height` cells of 0.5 m. `owners[i]` is the polygon
    /// of cell `i` (`NO_POLY` for none); polygons are built from the owners.
    fn mesh_with_owners(width: u32, height: u32, owners: &[u32]) -> NavMesh {
        let count = owners
            .iter()
            .filter(|o| **o != NO_POLY)
            .map(|o| *o as usize + 1)
            .max()
            .unwrap_or(0);
        let mut polygons: Vec<Polygon> = (0..count)
            .map(|i| Polygon {
                id: PolyId(i as u32),
                area: 0,
                cells: Vec::new(),
                neighbors: Vec::new(),
            })
            .collect();
        for (i, owner) in owners.iter().enumerate() {
            if *owner != NO_POLY {
                let cell = [(i % width as usize) as u32, (i / width as usize) as u32];
                polygons[*owner as usize].cells.push(cell);
            }
        }
        NavMesh {
            cell_mm: 500,
            origin_mm: [0, 0],
            width,
            height,
            polygons,
            cell_poly: owners.to_vec(),
            cell_h_mm: vec![0; owners.len()],
        }
    }

    #[test]
    fn shoelace_known_square_and_triangle() {
        // 4 x 4 square: area 16, so twice the area is 32.
        let square = [
            ([0.0, 0.0], [4.0, 0.0]),
            ([4.0, 0.0], [4.0, 4.0]),
            ([4.0, 4.0], [0.0, 4.0]),
            ([0.0, 4.0], [0.0, 0.0]),
        ];
        assert_eq!(shoelace_twice(&square), 32.0);
        // Triangle (0,0),(4,0),(0,3): area 6, twice is 12.
        let tri = [
            ([0.0, 0.0], [4.0, 0.0]),
            ([4.0, 0.0], [0.0, 3.0]),
            ([0.0, 3.0], [0.0, 0.0]),
        ];
        assert_eq!(shoelace_twice(&tri), 12.0);
        // Same loop clockwise flips the sign.
        let cw = [
            ([0.0, 0.0], [0.0, 3.0]),
            ([0.0, 3.0], [4.0, 0.0]),
            ([4.0, 0.0], [0.0, 0.0]),
        ];
        assert_eq!(shoelace_twice(&cw), -12.0);
    }

    #[test]
    fn polygon_area_block_and_hole() {
        // 2 wide, 3 tall, all polygon 0: 6 cells of 0.5 m, so 6 * 0.25 = 1.5 m2.
        let block = mesh_with_owners(2, 3, &[0; 6]);
        assert_eq!(block.polygon_area_m2(PolyId(0)), Some(1.5));
        // 3 x 3 ring around a hole: 8 cells, so 2.0 m2. The hole's inner
        // edges subtract without a special case.
        let ring = mesh_with_owners(3, 3, &[0, 0, 0, 0, NO_POLY, 0, 0, 0, 0]);
        assert_eq!(ring.polygon_area_m2(PolyId(0)), Some(2.0));
    }

    #[test]
    fn polygon_area_counts_only_own_cells() {
        // Two polygons side by side: each counts only its own cells.
        let split = mesh_with_owners(3, 1, &[0, 0, 1]);
        assert_eq!(split.polygon_area_m2(PolyId(0)), Some(0.5));
        assert_eq!(split.polygon_area_m2(PolyId(1)), Some(0.25));
        assert_eq!(split.polygon_area_m2(PolyId(9)), None);
    }

    #[test]
    fn stacked_four_floors_route_ground_to_roof() {
        // PORT(AizenKai1001/lunaris@56cc7e30cbd21ece2314c155498a66f69fb36535:docs/NAVIGATION.md bridge-over-road multi-level).
        let levels: Vec<Vec<[[f32; 3]; 3]>> = (0..4)
            .map(|f| slab(0.0, 4.0, 0.0, 4.0, f as f32 * 3.0))
            .collect();
        let stacked = StackedNav::bake_stacked(&levels, &config()).expect("stack bakes");
        assert_eq!(stacked.level_count(), 4, "stacked floors never merge");
        for lvl in stacked.levels() {
            assert_eq!(lvl.polygon_count(), 1);
        }
        let path = stacked
            .query_stacked(0, [0.5, 0.0, 3.5], 3, [3.5, 9.0, 0.5], [0.5, 0.0, 0.5])
            .expect("ground to roof routes");
        assert!(path.len() >= 2, "route has steps: {path:?}");
        assert!((path[0][1] - 0.0).abs() < 0.6, "starts at ground: {path:?}");
        assert!(
            (path[path.len() - 1][1] - 9.0).abs() < 0.6,
            "ends at roof: {path:?}"
        );
        assert_eq!(stacked.to_bytes(), stacked.to_bytes(), "bytes x2");
        let back = StackedNav::from_bytes(&stacked.to_bytes()).expect("round trip");
        assert_eq!(back.digest(), stacked.digest());
        let mut reg = crate::tiles::TileRegistry::new(16).expect("reg builds");
        let bump = crate::rebake::rebake_mesh_tile(
            &mut reg,
            &stacked.levels()[0],
            crate::tiles::TileId(0),
        );
        assert_eq!(bump, Ok(1));
    }

    #[test]
    fn stacked_bad_levels_refuse() {
        assert_eq!(
            StackedNav::bake_stacked(&[], &config()),
            Err(NavError::EmptyInput)
        );
        let levels = vec![slab(0.0, 1.0, 0.0, 1.0, 0.0)];
        let stacked = StackedNav::bake_stacked(&levels, &config()).expect("one floor bakes");
        assert_eq!(
            stacked.query_stacked(0, [0.5, 0.0, 0.5], 9, [0.5, 0.0, 0.5], [0.5, 0.0, 0.5]),
            Err(NavError::OutOfBounds)
        );
    }
}
