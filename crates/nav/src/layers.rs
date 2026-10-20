//! layers: layered navmesh — several walkable spans per column, stairs as
//! ramps between layers, canonical v2 bytes with v1 mesh decode.
//! Owner: nav crate; consumer: any caller that routes inside a building.
//! Units: meters in and out, integer millimeters inside and in bytes.
//! Determinism: quantized triangle sort plus index-order span merge, so the
//! same soup bakes the same bytes on every machine. A column keeps one span
//! per floor; only a climbable step links spans across columns, so stacked
//! floors never average into one height. Errors: typed [`LayerError`], an
//! unreachable floor never guesses.

use std::collections::VecDeque;
use std::fmt;

use crate::cursor::{Cursor, Truncated};
use crate::mesh::{
    self, grid_u32, grid_usize, NavError, NavMesh, MAX_BYTES, MAX_CELLS, MAX_SIDE, MAX_TRIS,
    MESH_VERSION,
};

/// Layered blob magic; plain mesh blobs stay `E2NAV1`.
const LAYERS_MAGIC: [u8; 6] = *b"E2LAY2";
/// Spans one column may hold; dense towers past it refuse loudly.
pub const MAX_SPANS_PER_COLUMN: usize = 16;
/// Largest climbable step between neighbouring spans, in millimeters.
pub const MAX_STEP_MM: i32 = 1_000;
/// Default climbable step, in millimeters.
pub const DEFAULT_STEP_MM: i32 = 400;
/// Heights closer than this in one column count as one baked surface.
const MERGE_MM: i32 = 50;
/// Largest absolute span height, matching the bake coordinate bound.
const MAX_TOP_MM: i32 = 4_096_000;
/// Layered header size: magic, version, cell, origin, dims, step, layers.
const HEADER_BYTES: usize = 36;

/// Bake configuration. Units: meters for `cell_size`, cosine of the steepest
/// walkable slope for `walkable_normal_y`, millimeters for `step_mm`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerConfig {
    /// Voxel size in meters, within `0.05..=5.0`.
    pub cell_size: f32,
    /// Minimum triangle normal Y kept as walkable, within `(0.0, 1.0]`.
    pub walkable_normal_y: f32,
    /// Largest height step between neighbouring spans, within
    /// `1..=MAX_STEP_MM` millimeters.
    pub step_mm: i32,
}

impl Default for LayerConfig {
    fn default() -> Self {
        Self {
            cell_size: 0.25,
            walkable_normal_y: 0.7,
            step_mm: DEFAULT_STEP_MM,
        }
    }
}

/// One walkable surface in a column: its height plus the connected walk
/// surface it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    top_mm: i32,
    layer: u32,
}

impl Span {
    /// Walkable height in millimeters.
    pub fn top_mm(&self) -> i32 {
        self.top_mm
    }
    /// Walkable height in meters.
    pub fn top_m(&self) -> f32 {
        engine_core::checked::f32_from_f64(f64::from(self.top_mm) / 1000.0)
    }
    /// Connected walk surface this span belongs to.
    pub fn layer(&self) -> u32 {
        self.layer
    }
}

/// Typed bake, query and decode failures. Unreachable targets error.
#[derive(Clone, Debug, PartialEq)]
pub enum LayerError {
    /// A plain mesh refusal, reused so both nav paths share one vocabulary.
    Nav(NavError),
    /// Step outside `1..=MAX_STEP_MM`.
    BadStep {
        /// Step offered in millimeters.
        got: i32,
    },
    /// One column held more spans than the table accepts.
    TooManySpans {
        /// Column index in grid order.
        column: usize,
        /// Span count found.
        got: usize,
    },
}

impl fmt::Display for LayerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nav(why) => write!(f, "nav layers: {why}"),
            Self::BadStep { got } => {
                write!(f, "nav layers: step {got} mm outside 1..={MAX_STEP_MM}")
            }
            Self::TooManySpans { column, got } => {
                write!(
                    f,
                    "nav layers: column {column} holds {got} spans over {MAX_SPANS_PER_COLUMN}"
                )
            }
        }
    }
}

impl std::error::Error for LayerError {}

impl From<NavError> for LayerError {
    fn from(why: NavError) -> Self {
        Self::Nav(why)
    }
}

impl From<Truncated> for LayerError {
    fn from(_: Truncated) -> Self {
        Self::Nav(NavError::Truncated)
    }
}

/// Layered navmesh: one span table for all columns plus one layer id per span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayeredNav {
    cell_mm: u32,
    origin_mm: [i32; 2],
    width: usize,
    height: usize,
    step_mm: i32,
    layers: u32,
    /// Column `i` owns spans `first[i]..first[i + 1]`; length `cells + 1`.
    first: Vec<usize>,
    spans: Vec<Span>,
}

/// Bake walkable triangles into a layered navmesh.
///
/// Input is a triangle soup in meters (`[x, y, z]`, `+Y` up). Every column
/// keeps one span per distinct height, so four floors over one footprint stay
/// four spans. Neighbouring spans link only when their height gap is at most
/// `config.step_mm`, which makes a stair ramp a walkable stair and a bare
/// floor edge a wall.
pub fn bake(tris: &[[[f32; 3]; 3]], config: &LayerConfig) -> Result<LayeredNav, LayerError> {
    check_config(config)?;
    if tris.is_empty() {
        return Err(NavError::EmptyInput.into());
    }
    if tris.len() > MAX_TRIS {
        return Err(NavError::TooManyTriangles {
            got: tris.len(),
            max: MAX_TRIS,
        }
        .into());
    }
    let mut quant = Vec::with_capacity(tris.len());
    for (index, tri) in tris.iter().enumerate() {
        quant.push(mesh::quantize_tri(tri, index)?);
    }
    quant.sort_unstable();
    quant.dedup();
    let walkable: Vec<[i32; 9]> = quant
        .into_iter()
        .filter(|tri| mesh::tri_normal_y(tri) >= config.walkable_normal_y)
        .collect();
    if walkable.is_empty() {
        return Err(NavError::NoWalkable.into());
    }
    let cell_mm = cell_mm(config)?;
    let (origin_mm, width, height) = mesh::grid_span(&walkable, cell_mm)?;
    let mut heights: Vec<Vec<i32>> = vec![Vec::new(); grid_usize(width) * grid_usize(height)];
    for tri in &walkable {
        rasterize(tri, origin_mm, width, height, cell_mm, &mut heights);
    }
    let (first, mut spans) = build_spans(heights)?;
    let layers = assign_layers(
        &mut spans,
        &first,
        grid_usize(width),
        grid_usize(height),
        config.step_mm,
    );
    Ok(LayeredNav {
        cell_mm,
        origin_mm,
        width: grid_usize(width),
        height: grid_usize(height),
        step_mm: config.step_mm,
        layers,
        first,
        spans,
    })
}

impl LayeredNav {
    /// Bake through the free function with an explicit config.
    pub fn bake(tris: &[[[f32; 3]; 3]], config: &LayerConfig) -> Result<Self, LayerError> {
        bake(tris, config)
    }
    /// Decode a plain mesh into a one-span-per-cell table; the v1 read path.
    /// Mesh callers keep their own routing; this only preserves their heights.
    pub fn from_mesh(mesh: &NavMesh, step_mm: i32) -> Result<Self, LayerError> {
        check_step(step_mm)?;
        let (raw_width, raw_height) = mesh.grid_dims();
        let (width, height) = (grid_usize(raw_width), grid_usize(raw_height));
        if width * height > MAX_CELLS {
            return Err(NavError::GridTooLarge {
                w: raw_width,
                h: raw_height,
                max_side: MAX_SIDE,
            }
            .into());
        }
        let mut first = Vec::with_capacity(width * height + 1);
        first.push(0);
        let mut spans = Vec::new();
        for cz in 0..height {
            for cx in 0..width {
                if let Some(top_mm) = mesh.cell_height_mm([grid_u32(cx), grid_u32(cz)]) {
                    spans.push(Span { top_mm, layer: 0 });
                }
                first.push(spans.len());
            }
        }
        if spans.is_empty() {
            return Err(NavError::NoWalkable.into());
        }
        let layers = assign_layers(&mut spans, &first, width, height, step_mm);
        Ok(Self {
            cell_mm: mesh.cell_size_mm(),
            origin_mm: mesh.origin_mm(),
            width,
            height,
            step_mm,
            layers,
            first,
            spans,
        })
    }
    /// Cell size in meters.
    pub fn cell_size_m(&self) -> f32 {
        engine_core::checked::f32_from_f64(f64::from(self.cell_mm) / 1000.0)
    }
    /// Grid dims in cells `(width, height)`.
    pub fn grid_dims(&self) -> (u32, u32) {
        (grid_u32(self.width), grid_u32(self.height))
    }
    /// Span count over every column.
    pub fn span_count(&self) -> usize {
        self.spans.len()
    }
    /// Connected walk surfaces; four floors without a stair make four.
    pub fn layer_count(&self) -> u32 {
        self.layers
    }
    /// Spans of the column holding `point`, ascending by height. Off-grid
    /// points refuse; an empty column returns an empty slice.
    pub fn spans_at(&self, point: [f32; 3]) -> Result<&[Span], LayerError> {
        let column = self.column_of(point)?;
        Ok(&self.spans[self.first[column]..self.first[column + 1]])
    }
    /// Span nearest to the point's height in its column; ties keep the lower.
    /// Off-grid points and columns with no walkable surface refuse.
    pub fn span_at(&self, point: [f32; 3]) -> Result<&Span, LayerError> {
        let index = self.nearest_span(point)?;
        Ok(&self.spans[index])
    }
    /// Waypoint path in meters from `from` to `to`.
    ///
    /// The two endpoints snap to the span nearest their `y`; every step moves
    /// one column and climbs at most `step_mm`, so a route over a stair exists
    /// only when the stair geometry links the floors. Unreachable floors
    /// report [`NavError::Unreachable`].
    pub fn query(&self, from: [f32; 3], to: [f32; 3]) -> Result<Vec<[f32; 3]>, LayerError> {
        let start = self.nearest_span(from)?;
        let goal = self.nearest_span(to)?;
        if start == goal {
            return Ok(vec![self.span_center(start)]);
        }
        let Some(chain) = self.span_path(start, goal) else {
            return Err(NavError::Unreachable.into());
        };
        Ok(chain
            .into_iter()
            .map(|span| self.span_center(span))
            .collect())
    }
    /// Canonical versioned bytes; identical geometry bakes identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let size = HEADER_BYTES + self.first.len() * 4 + self.spans.len() * 8;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&LAYERS_MAGIC);
        out.extend_from_slice(&MESH_VERSION.to_le_bytes());
        out.extend_from_slice(&self.cell_mm.to_le_bytes());
        out.extend_from_slice(&self.origin_mm[0].to_le_bytes());
        out.extend_from_slice(&self.origin_mm[1].to_le_bytes());
        out.extend_from_slice(&grid_u32(self.width).to_le_bytes());
        out.extend_from_slice(&grid_u32(self.height).to_le_bytes());
        out.extend_from_slice(&self.step_mm.to_le_bytes());
        out.extend_from_slice(&self.layers.to_le_bytes());
        for start in &self.first {
            out.extend_from_slice(&grid_u32(*start).to_le_bytes());
        }
        for span in &self.spans {
            out.extend_from_slice(&span.top_mm.to_le_bytes());
            out.extend_from_slice(&span.layer.to_le_bytes());
        }
        out
    }
    /// Decode canonical bytes. Accepts a layered `E2LAY2` blob and, as the v1
    /// read path, any plain `E2NAV1` mesh blob that still decodes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LayerError> {
        if bytes.len() > MAX_BYTES {
            return Err(NavError::TooLarge { got: bytes.len() }.into());
        }
        if bytes.len() < LAYERS_MAGIC.len() {
            return Err(NavError::Truncated.into());
        }
        if bytes[..LAYERS_MAGIC.len()] == LAYERS_MAGIC {
            decode_layered(bytes)
        } else if bytes[..mesh::MESH_MAGIC.len()] == mesh::MESH_MAGIC {
            Ok(Self::from_mesh(
                &NavMesh::from_bytes(bytes)?,
                DEFAULT_STEP_MM,
            )?)
        } else {
            Err(NavError::BadMagic.into())
        }
    }
    /// Stable `u64` over the canonical bytes (bake SHA gate).
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }
    /// Column index of a world point; off-grid points (NaN included) refuse.
    fn column_of(&self, point: [f32; 3]) -> Result<usize, LayerError> {
        if !point.iter().all(|value| value.is_finite()) {
            return Err(NavError::OutOfBounds.into());
        }
        let fx =
            (f64::from(point[0]) * 1000.0 - f64::from(self.origin_mm[0])) / f64::from(self.cell_mm);
        let fz =
            (f64::from(point[2]) * 1000.0 - f64::from(self.origin_mm[1])) / f64::from(self.cell_mm);
        if fx < 0.0 || fz < 0.0 {
            return Err(NavError::OutOfBounds.into());
        }
        let (cx, cz) = (cell_coord(fx), cell_coord(fz));
        if cx >= self.width || cz >= self.height {
            return Err(NavError::OutOfBounds.into());
        }
        Ok(cz * self.width + cx)
    }
    /// Span index nearest to the point's height in its column.
    fn nearest_span(&self, point: [f32; 3]) -> Result<usize, LayerError> {
        let column = self.column_of(point)?;
        let y_mm = mesh::round_mm(f64::from(point[1]) * 1000.0).ok_or(NavError::OutOfBounds)?;
        let (lo, hi) = (self.first[column], self.first[column + 1]);
        if lo == hi {
            return Err(NavError::NoPolygon.into());
        }
        let mut best = lo;
        for index in lo + 1..hi {
            let best_gap = (i64::from(self.spans[best].top_mm) - i64::from(y_mm)).abs();
            let gap = (i64::from(self.spans[index].top_mm) - i64::from(y_mm)).abs();
            if gap < best_gap {
                best = index;
            }
        }
        Ok(best)
    }
    /// Deterministic span path over climbable column neighbours, inclusive.
    fn span_path(&self, start: usize, goal: usize) -> Option<Vec<usize>> {
        let mut parent = vec![u32::MAX; self.spans.len()];
        let mut seen = vec![false; self.spans.len()];
        seen[start] = true;
        let mut queue = VecDeque::new();
        queue.push_back(start);
        while let Some(at) = queue.pop_front() {
            if at == goal {
                break;
            }
            let column = column_of(&self.first, at);
            for next_column in neighbors(column, self.width, self.height)
                .into_iter()
                .flatten()
            {
                let (lo, hi) = (self.first[next_column], self.first[next_column + 1]);
                for next in lo..hi {
                    if seen[next]
                        || !climbable(self.spans[at].top_mm, self.spans[next].top_mm, self.step_mm)
                    {
                        continue;
                    }
                    seen[next] = true;
                    parent[next] = u32::try_from(at).ok()?;
                    queue.push_back(next);
                }
            }
        }
        if !seen[goal] {
            return None;
        }
        let mut chain = vec![goal];
        let mut at = goal;
        while at != start {
            at = usize::try_from(parent[at]).ok()?;
            chain.push(at);
        }
        chain.reverse();
        Some(chain)
    }
    /// World center of the column holding `span`, at the span's height.
    fn span_center(&self, span: usize) -> [f32; 3] {
        let column = column_of(&self.first, span);
        let cx = column % self.width;
        let cz = column / self.width;
        [
            cell_center_m(self.origin_mm[0], cx, self.cell_mm),
            self.spans[span].top_m(),
            cell_center_m(self.origin_mm[1], cz, self.cell_mm),
        ]
    }
}

/// Validate the bake configuration before any triangle is touched.
fn check_config(config: &LayerConfig) -> Result<(), LayerError> {
    if !config.cell_size.is_finite() || !(0.05..=5.0).contains(&config.cell_size) {
        return Err(NavError::BadCellSize {
            got: config.cell_size,
        }
        .into());
    }
    if !config.walkable_normal_y.is_finite()
        || config.walkable_normal_y <= 0.0
        || config.walkable_normal_y > 1.0
    {
        return Err(NavError::BadSlope {
            got: config.walkable_normal_y,
        }
        .into());
    }
    check_step(config.step_mm)
}

/// Step bound shared by bake and decode.
fn check_step(step_mm: i32) -> Result<(), LayerError> {
    if !(1..=MAX_STEP_MM).contains(&step_mm) {
        return Err(LayerError::BadStep { got: step_mm });
    }
    Ok(())
}

/// Cell size in whole millimeters within the mesh bound.
fn cell_mm(config: &LayerConfig) -> Result<u32, LayerError> {
    mesh::round_mm(f64::from(config.cell_size * 1000.0))
        .and_then(|mm| u32::try_from(mm).ok())
        .filter(|mm| (50..=5000).contains(mm))
        .ok_or_else(|| {
            NavError::BadCellSize {
                got: config.cell_size,
            }
            .into()
        })
}

/// Non-negative float cell coordinate as a grid index.
fn cell_coord(value: f64) -> usize {
    let cell = engine_core::checked::saturating_f64_to_u64(value.floor());
    usize::try_from(cell).unwrap_or(usize::MAX)
}

/// World center of one cell axis in meters.
fn cell_center_m(origin_mm: i32, cell: usize, cell_mm: u32) -> f32 {
    let cell = f64::from(grid_u32(cell));
    engine_core::checked::f32_from_f64(
        (f64::from(origin_mm) + (cell + 0.5) * f64::from(cell_mm)) / 1000.0,
    )
}

/// Grid column index of cell `(cx, cz)`.
fn column_index(cx: u32, cz: u32, width: u32) -> usize {
    grid_usize(cz) * grid_usize(width) + grid_usize(cx)
}

/// Sample one walkable triangle into every column center it covers.
fn rasterize(
    tri: &[i32; 9],
    origin_mm: [i32; 2],
    width: u32,
    height: u32,
    cell_mm: u32,
    heights: &mut [Vec<i32>],
) {
    let tri_f = mesh::quant_to_tri_f(tri);
    let (lo_x, hi_x) = mesh::tri_cell_span(
        [tri_f[0][0], tri_f[1][0], tri_f[2][0]],
        origin_mm[0],
        cell_mm,
        width,
    );
    let (lo_z, hi_z) = mesh::tri_cell_span(
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
            if let Some(top_mm) = mesh::sample_height(&tri_f, center) {
                heights[column_index(cx, cz, width)].push(top_mm);
            }
        }
    }
}

/// Sort each column's samples and keep one span per distinct height.
fn build_spans(mut heights: Vec<Vec<i32>>) -> Result<(Vec<usize>, Vec<Span>), LayerError> {
    let mut first = Vec::with_capacity(heights.len() + 1);
    first.push(0);
    let mut spans = Vec::new();
    for (column, found) in heights.iter_mut().enumerate() {
        found.sort_unstable();
        let mut kept = 0usize;
        let mut last: Option<i32> = None;
        for top_mm in found.iter().copied() {
            if last
                .is_some_and(|height| i64::from(top_mm) - i64::from(height) <= i64::from(MERGE_MM))
            {
                continue;
            }
            spans.push(Span { top_mm, layer: 0 });
            last = Some(top_mm);
            kept += 1;
        }
        if kept > MAX_SPANS_PER_COLUMN {
            return Err(LayerError::TooManySpans { column, got: kept });
        }
        first.push(spans.len());
    }
    Ok((first, spans))
}

/// Label spans by connected walk surface: only a climbable step links them.
fn assign_layers(
    spans: &mut [Span],
    first: &[usize],
    width: usize,
    height: usize,
    step_mm: i32,
) -> u32 {
    let mut layer = vec![u32::MAX; spans.len()];
    let mut queue = VecDeque::new();
    let mut next_layer = 0u32;
    for start in 0..spans.len() {
        if layer[start] != u32::MAX {
            continue;
        }
        layer[start] = next_layer;
        queue.push_back(start);
        while let Some(at) = queue.pop_front() {
            let column = column_of(first, at);
            for next_column in neighbors(column, width, height).into_iter().flatten() {
                let (lo, hi) = (first[next_column], first[next_column + 1]);
                for next in lo..hi {
                    if layer[next] != u32::MAX
                        || !climbable(spans[at].top_mm, spans[next].top_mm, step_mm)
                    {
                        continue;
                    }
                    layer[next] = next_layer;
                    queue.push_back(next);
                }
            }
        }
        next_layer += 1;
    }
    for (span, id) in spans.iter_mut().zip(layer) {
        span.layer = id;
    }
    next_layer
}

/// True when two span heights are at most one step apart.
fn climbable(from_mm: i32, to_mm: i32, step_mm: i32) -> bool {
    (i64::from(from_mm) - i64::from(to_mm)).abs() <= i64::from(step_mm)
}

/// 4-dir column neighbours in grid order, `None` past an edge.
fn neighbors(column: usize, width: usize, height: usize) -> [Option<usize>; 4] {
    let (cx, cz) = (column % width, column / width);
    [
        if cx > 0 { Some(column - 1) } else { None },
        if cx + 1 < width {
            Some(column + 1)
        } else {
            None
        },
        if cz > 0 { Some(column - width) } else { None },
        if cz + 1 < height {
            Some(column + width)
        } else {
            None
        },
    ]
}

/// Column index of a span index, through the sorted `first` table.
fn column_of(first: &[usize], span: usize) -> usize {
    first.partition_point(|start| *start <= span) - 1
}

/// Decoded layered header; every table below reads its bounds from here.
struct Header {
    cell_mm: u32,
    origin_mm: [i32; 2],
    width: usize,
    height: usize,
    step_mm: i32,
    layers: u32,
}

/// Decode a layered `E2LAY2` blob, refusing anything non-canonical.
fn decode_layered(bytes: &[u8]) -> Result<LayeredNav, LayerError> {
    let mut read = Cursor::new(bytes);
    let header = decode_header(&mut read)?;
    let (first, mut spans, declared) = decode_table(&mut read, &header)?;
    if read.remaining() != 0 {
        return Err(NavError::TrailingBytes {
            got: read.remaining(),
        }
        .into());
    }
    let rebuilt = assign_layers(
        &mut spans,
        &first,
        header.width,
        header.height,
        header.step_mm,
    );
    if rebuilt != header.layers
        || spans
            .iter()
            .zip(&declared)
            .any(|(span, want)| span.layer != *want)
    {
        return Err(not_canonical("layer ids"));
    }
    Ok(LayeredNav {
        cell_mm: header.cell_mm,
        origin_mm: header.origin_mm,
        width: header.width,
        height: header.height,
        step_mm: header.step_mm,
        layers: header.layers,
        first,
        spans,
    })
}

/// Read and bound-check the fixed layered header.
fn decode_header(read: &mut Cursor<'_>) -> Result<Header, LayerError> {
    let mut magic = [0u8; 6];
    for slot in magic.iter_mut() {
        *slot = read.byte()?;
    }
    if magic != LAYERS_MAGIC {
        return Err(NavError::BadMagic.into());
    }
    let version = read.u16()?;
    if version != MESH_VERSION {
        return Err(NavError::VersionMismatch { got: version }.into());
    }
    let cell_mm = read.u32()?;
    if !(50..=5000).contains(&cell_mm) {
        return Err(not_canonical("cell size"));
    }
    let origin_mm = [read.i32()?, read.i32()?];
    let (raw_width, raw_height) = (read.u32()?, read.u32()?);
    if raw_width == 0 || raw_height == 0 || raw_width > MAX_SIDE || raw_height > MAX_SIDE {
        return Err(not_canonical("grid dims"));
    }
    let (width, height) = (grid_usize(raw_width), grid_usize(raw_height));
    if width * height > MAX_CELLS {
        return Err(not_canonical("grid cells"));
    }
    let step_mm = read.i32()?;
    if !(1..=MAX_STEP_MM).contains(&step_mm) {
        return Err(not_canonical("step"));
    }
    let layers = read.u32()?;
    Ok(Header {
        cell_mm,
        origin_mm,
        width,
        height,
        step_mm,
        layers,
    })
}

/// Decoded span table: column starts, spans, and the layer ids it declared.
type SpanTable = (Vec<usize>, Vec<Span>, Vec<u32>);

/// Read the span table, its declared layer ids and its canonical order.
fn decode_table(read: &mut Cursor<'_>, header: &Header) -> Result<SpanTable, LayerError> {
    let cells = header.width * header.height;
    let mut first = Vec::with_capacity(cells + 1);
    for _ in 0..=cells {
        first.push(grid_usize(read.u32()?));
    }
    if first[0] != 0 {
        return Err(not_canonical("span table start"));
    }
    for pair in first.windows(2) {
        if pair[1] < pair[0] || pair[1] - pair[0] > MAX_SPANS_PER_COLUMN {
            return Err(not_canonical("span table order"));
        }
    }
    let total = first[cells];
    if total == 0 || total > cells * MAX_SPANS_PER_COLUMN {
        return Err(not_canonical("span count"));
    }
    if header.layers == 0 || grid_usize(header.layers) > total {
        return Err(not_canonical("layer count"));
    }
    if read.remaining() < total * 8 {
        return Err(NavError::Truncated.into());
    }
    let mut spans = Vec::with_capacity(total);
    let mut declared = Vec::with_capacity(total);
    for _ in 0..total {
        let top_mm = read.i32()?;
        if !(-MAX_TOP_MM..=MAX_TOP_MM).contains(&top_mm) {
            return Err(not_canonical("span height"));
        }
        let layer = read.u32()?;
        if layer >= header.layers {
            return Err(not_canonical("layer id"));
        }
        spans.push(Span { top_mm, layer: 0 });
        declared.push(layer);
    }
    for column in 0..cells {
        let (lo, hi) = (first[column], first[column + 1]);
        let mut last: Option<i32> = None;
        for span in &spans[lo..hi] {
            if let Some(height) = last {
                if i64::from(span.top_mm) - i64::from(height) <= i64::from(MERGE_MM) {
                    return Err(not_canonical("span order"));
                }
            }
            last = Some(span.top_mm);
        }
    }
    Ok((first, spans, declared))
}

/// One non-canonical blob refusal naming the offending field.
fn not_canonical(why: &'static str) -> LayerError {
    LayerError::Nav(NavError::NotCanonical(why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{BakeConfig, NavMesh};

    fn config() -> LayerConfig {
        LayerConfig {
            cell_size: 0.5,
            walkable_normal_y: 0.7,
            step_mm: DEFAULT_STEP_MM,
        }
    }

    /// Flat quad slab over `[x0, x1] x [z0, z1]` at height `y`, meters.
    fn slab(x0: f32, x1: f32, z0: f32, z1: f32, y: f32) -> Vec<[[f32; 3]; 3]> {
        vec![
            [[x0, y, z0], [x0, y, z1], [x1, y, z1]],
            [[x0, y, z0], [x1, y, z1], [x1, y, z0]],
        ]
    }

    /// Quad ramp rising along `+x` from `y0` to `y1`, meters.
    fn ramp(x0: f32, x1: f32, z0: f32, z1: f32, y0: f32, y1: f32) -> Vec<[[f32; 3]; 3]> {
        vec![
            [[x0, y0, z0], [x0, y0, z1], [x1, y1, z1]],
            [[x0, y0, z0], [x1, y1, z1], [x1, y1, z0]],
        ]
    }

    /// Four floors three meters apart, each pair joined by a stair ramp.
    fn building() -> Vec<[[f32; 3]; 3]> {
        let mut tris = Vec::new();
        for floor in 0..4 {
            let y = floor as f32 * 3.0;
            tris.extend(slab(0.0, 12.0, 0.0, 4.0, y));
            if floor < 3 {
                tris.extend(ramp(4.0, 8.0, 0.0, 1.5, y, y + 3.0));
            }
        }
        tris
    }

    #[test]
    fn four_floor_building_routes_ground_to_roof() {
        let nav = bake(&building(), &config()).expect("building bakes");
        let tops: Vec<i32> = nav
            .spans_at([1.0, 0.0, 1.0])
            .expect("ground column is inside")
            .iter()
            .map(Span::top_mm)
            .collect();
        assert_eq!(tops, vec![0, 3_000, 6_000, 9_000], "one span per floor");
        let path = nav
            .query([1.0, 0.0, 1.0], [1.0, 9.0, 1.0])
            .expect("ground to roof routes");
        assert!(
            (path[0][1] - 0.0).abs() < 0.001,
            "starts at ground: {path:?}"
        );
        assert!(
            (path[path.len() - 1][1] - 9.0).abs() < 0.001,
            "ends at roof: {path:?}"
        );
        for step in path.windows(2) {
            let rise = step[1][1] - step[0][1];
            assert!(rise <= 0.401, "one stair step at a time: {step:?}");
        }
        assert!(
            path.windows(2)
                .any(|step| (step[1][1] - step[0][1] - 0.375).abs() < 0.08),
            "the stair ramp carries a 375 mm step: {path:?}"
        );
    }

    #[test]
    fn stacked_floors_never_merge() {
        let mut tris = Vec::new();
        for floor in 0..4 {
            tris.extend(slab(0.0, 12.0, 0.0, 4.0, floor as f32 * 3.0));
        }
        let nav = bake(&tris, &config()).expect("floors bake");
        assert_eq!(
            nav.layer_count(),
            4,
            "four floors with no stair are four walk surfaces"
        );
        assert_eq!(
            nav.spans_at([1.0, 0.0, 1.0])
                .expect("inside")
                .iter()
                .map(Span::top_mm)
                .collect::<Vec<_>>(),
            vec![0, 3_000, 6_000, 9_000],
            "the column keeps every floor"
        );
        assert_eq!(
            nav.query([1.0, 0.0, 1.0], [1.0, 9.0, 1.0]),
            Err(NavError::Unreachable.into()),
            "no stair never guesses a floor"
        );
        assert_eq!(
            nav.span_at([1.0, 5.9, 1.0])
                .expect("nearest is floor two")
                .top_mm(),
            6_000
        );
    }

    #[test]
    fn bytes_identical_twice_and_round_trip() {
        let first = bake(&building(), &config()).expect("bakes");
        let second = bake(&building(), &config()).expect("bakes");
        assert_eq!(first.to_bytes(), second.to_bytes(), "bytes x2");
        assert_eq!(first.digest(), second.digest(), "digest x2");
        let bytes = first.to_bytes();
        let back = LayeredNav::from_bytes(&bytes).expect("canonical decodes");
        assert_eq!(back, first);
        assert_eq!(back.to_bytes(), bytes);
        assert!(
            LayeredNav::from_bytes(&bytes[..bytes.len() - 1]).is_err(),
            "a truncated blob refuses"
        );
        let mut future = bytes.clone();
        future[6..8].copy_from_slice(&(MESH_VERSION + 1).to_le_bytes());
        assert_eq!(
            LayeredNav::from_bytes(&future),
            Err(NavError::VersionMismatch {
                got: MESH_VERSION + 1
            }
            .into())
        );
        let mut forged = bytes;
        let declared = u32::from_le_bytes([forged[32], forged[33], forged[34], forged[35]]);
        let last = forged.len() - 4;
        forged[last..].copy_from_slice(&declared.to_le_bytes());
        assert!(
            LayeredNav::from_bytes(&forged).is_err(),
            "a forged layer id refuses"
        );
    }

    #[test]
    fn v1_mesh_bytes_decode_as_one_span_per_cell() {
        let plain = NavMesh::bake(&slab(0.0, 4.0, 0.0, 4.0, 2.0), &BakeConfig::default())
            .expect("mesh bakes");
        let bytes = plain.to_bytes();
        assert_eq!(bytes[6..8], MESH_VERSION.to_le_bytes(), "mesh writes v2");
        let nav = LayeredNav::from_bytes(&bytes).expect("mesh bytes decode into spans");
        assert_eq!(nav.layer_count(), 1, "one floor is one walk surface");
        assert_eq!(
            nav.spans_at([1.0, 2.0, 1.0]).expect("inside").len(),
            1,
            "a plain mesh has one span per cell"
        );
        assert_eq!(nav.span_at([1.0, 2.0, 1.0]).expect("span").top_mm(), 2_000);
        let mut v1 = bytes;
        v1[6..8].copy_from_slice(&1u16.to_le_bytes());
        let old = NavMesh::from_bytes(&v1).expect("a v1 mesh still decodes");
        assert_eq!(
            old.to_bytes()[6..8],
            MESH_VERSION.to_le_bytes(),
            "v1 reads, v2 writes"
        );
        let layered = LayeredNav::from_bytes(&v1).expect("a v1 mesh decodes into spans");
        assert_eq!(layered.digest(), nav.digest(), "heights carry over");
    }

    #[test]
    fn bad_configs_and_off_grid_points_refuse() {
        let floor = slab(0.0, 2.0, 0.0, 2.0, 0.0);
        assert_eq!(bake(&[], &config()), Err(NavError::EmptyInput.into()));
        assert_eq!(
            bake(
                &floor,
                &LayerConfig {
                    step_mm: 0,
                    ..config()
                }
            ),
            Err(LayerError::BadStep { got: 0 })
        );
        assert!(matches!(
            bake(
                &floor,
                &LayerConfig {
                    step_mm: MAX_STEP_MM + 1,
                    ..config()
                }
            ),
            Err(LayerError::BadStep { .. })
        ));
        let nav = bake(&floor, &config()).expect("floor bakes");
        assert_eq!(
            nav.spans_at([9.0, 0.0, 9.0]),
            Err(NavError::OutOfBounds.into())
        );
        assert_eq!(
            nav.span_at([9.0, 0.0, 9.0]),
            Err(NavError::OutOfBounds.into())
        );
    }

    /// Twenty four-floor buildings bake inside the two-second budget. Wall
    /// clock only lives here; run alone with `--ignored` for the receipt.
    #[test]
    #[ignore = "wall-clock budget: run alone with --ignored on a quiet box"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn twenty_buildings_bake_under_two_seconds() {
        let mut tris = Vec::new();
        for building in 0..20 {
            let x = building as f32 * 20.0;
            for floor in 0..4 {
                let y = floor as f32 * 3.0;
                tris.extend(slab(x, x + 12.0, 0.0, 4.0, y));
                if floor < 3 {
                    tris.extend(ramp(x + 4.0, x + 8.0, 0.0, 1.5, y, y + 3.0));
                }
            }
        }
        let start = std::time::Instant::now();
        let nav = bake(&tris, &config()).expect("twenty buildings bake");
        let elapsed = start.elapsed();
        assert_eq!(nav.layer_count(), 20, "one walk surface per building");
        assert!(elapsed.as_millis() < 2_000, "bake took {elapsed:?}");
    }
}
