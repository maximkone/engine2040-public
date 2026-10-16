//! mesh_cook: engine-native mesh cook.
//!
//! Consumer: bundles pack [`MeshCookOutput`] LODs plus [`CollisionHull`]s,
//! and the modeling loop drives [`cook_gltf_bytes`] on outside-tool glTF.
//! Data owner: `engine-assets`. Units: triangles for budgets, meters for
//! positions and hulls. Determinism scope: the same mesh plus the same class
//! cooks bit-identical LODs, hulls and digests (index-order loops, sorted
//! edges, `+ - * /` and `sqrt` only; no hash-map iteration, no wall clock).
//! Errors: typed [`MeshCookError`]; degenerate inputs are refused, never
//! rescaled or guessed, while over-budget inputs simplify down to budget.
//!
//! Pipeline: [`cook_mesh`] validates one [`CookedMesh`], simplifies over-budget
//! input down to budget, builds the [`LOD_RATIOS`] quadric-simplification chain,
//! orders every LOD for the vertex cache and overdraw ([`optimize_index_order`],
//! offline only, byte-stable), and builds one containing hull per LOD;
//! [`cook_gltf_bytes`] runs that per imported
//! mesh and additionally collects `FRAC_<piece>` chunk nodes for destruction.
//!
//! `FRAC_` chunks are a naming and data convention for bonded destruction
//! collections, not a code dependency: this crate never imports
//! `engine-destruction`, and destruction reads only cooked bytes plus chunk
//! names.
//!
//! Conventions (from `docs/plan/content.md`): LOD suffixes `_LOD1.._LOD3`,
//! colliders `COL_`, fracture chunks `FRAC_<piece>`, sockets `SOCKET_<name>`;
//! triangle budgets are hero `<= 40k`, opponents `<= 25k`, props `<= 5k` at
//! LOD0 with at most 4 materials per character (materials ride the glTF import).

/// Re-exported cooked mesh type the vertex-cache vectors consume.
pub use super::cooked::CookedMesh;
use super::cooked::{len_word, validate_mesh, CookError};
use super::mesh_import::{import_gltf_bytes, MeshImportError, MAX_BYTES};
use super::wire::bound_chars;
use engine_core::Fnv64;
use std::fmt;

pub use super::collapse::simplify;

/// Hero LOD0 triangle budget (content pipeline convention).
pub const HERO_MAX_TRIS: u32 = 40_000;
/// Opponent LOD0 triangle budget (content pipeline convention).
pub const OPPONENT_MAX_TRIS: u32 = 25_000;
/// Prop LOD0 triangle budget (content pipeline convention).
pub const PROP_MAX_TRIS: u32 = 5_000;
/// LOD levels cooked per mesh: LOD0 plus three simplifications.
pub const MAX_LOD_LEVELS: usize = 4;
/// Fraction of the base triangle count each LOD level targets.
pub const LOD_RATIOS: [f32; MAX_LOD_LEVELS] = [1.0, 0.5, 0.25, 0.125];
/// Most support points one hull keeps; bounds the cooked payload.
pub const MAX_HULL_VERTS: usize = 64;
/// Node-name prefix marking a destruction fracture chunk.
pub const FRAC_PREFIX: &str = "FRAC_";
/// Longest accepted `FRAC_` node name; longer names are refused, never cut.
pub const MAX_FRAC_NAME_CHARS: usize = 128;
/// Longest accepted chunk piece token after the prefix.
pub const MAX_FRAC_PIECE_CHARS: usize = 64;
/// Most chunks one import collects; bounds the cooked payload.
pub const MAX_FRAC_CHUNKS: usize = 1024;
/// Collider-name prefix (content pipeline convention).
pub const COL_PREFIX: &str = "COL_";
/// Socket-name prefix (content pipeline convention).
pub const SOCKET_PREFIX: &str = "SOCKET_";
/// Bound for diagnostics carried in [`MeshCookError`].
pub const MAX_COOK_DETAIL_CHARS: usize = 128;
/// Triangles at or below this area (meters squared) count as zero-area.
const ZERO_AREA_M2: f32 = 1e-12;
/// Hull containment slack (meters) covering float rounding on the boundary.
const HULL_EPS_M: f32 = 1e-4;

/// Which content budget a mesh cooks against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshClass {
    /// Hero fighter: LOD0 holds at most [`HERO_MAX_TRIS`] triangles.
    Hero,
    /// Opponent variant: LOD0 holds at most [`OPPONENT_MAX_TRIS`] triangles.
    Opponent,
    /// Prop: LOD0 holds at most [`PROP_MAX_TRIS`] triangles.
    Prop,
}

impl MeshClass {
    /// LOD0 triangle budget for this class.
    pub fn max_tris(self) -> u32 {
        match self {
            Self::Hero => HERO_MAX_TRIS,
            Self::Opponent => OPPONENT_MAX_TRIS,
            Self::Prop => PROP_MAX_TRIS,
        }
    }

    /// Stable tag byte folded into cook digests.
    fn tag(self) -> u8 {
        match self {
            Self::Hero => 0,
            Self::Opponent => 1,
            Self::Prop => 2,
        }
    }
}

/// Why a mesh cook failed; every refused input maps to one variant.
#[derive(Clone, Debug, PartialEq)]
pub enum MeshCookError {
    /// No vertices at all.
    Empty,
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Index addresses no vertex, culprit index carried.
    IndexOutOfRange(u32),
    /// Geometry carries no cookable surface; bounded reason carried.
    Degenerate {
        /// Bounded reason (never echoes unbounded input).
        detail: String,
    },
    /// Triangle count exceeds the class budget; both counts carried.
    OverBudget {
        /// Triangles in the input mesh.
        tris: u32,
        /// Budget of the requested class.
        max: u32,
    },
    /// An input exceeds a cook bound; offending count carried.
    TooLarge {
        /// Offending byte or chunk count.
        count: usize,
    },
    /// glTF bytes failed to parse or name chunks; bounded reason carried.
    Parse {
        /// Bounded reason (never echoes unbounded input).
        detail: String,
    },
}

impl fmt::Display for MeshCookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "mesh cook found no vertices"),
            Self::NonFinite(field) => write!(f, "mesh cook non-finite {field}"),
            Self::IndexOutOfRange(index) => write!(f, "mesh cook index {index} out of range"),
            Self::Degenerate { detail } => {
                write!(f, "mesh cook degenerate: {}", bound_detail(detail))
            }
            Self::OverBudget { tris, max } => {
                write!(f, "mesh cook over budget: {tris} tris over {max}")
            }
            Self::TooLarge { count } => write!(f, "mesh cook too large: {count}"),
            Self::Parse { detail } => write!(f, "mesh cook parse: {}", bound_detail(detail)),
        }
    }
}

impl std::error::Error for MeshCookError {}

/// Truncate a carried diagnostic to [`MAX_COOK_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_COOK_DETAIL_CHARS)
}

/// Map cooked validation onto the cook error (mesh lanes only; the material
/// arms are defensive because [`validate_mesh`] never emits them).
pub(crate) fn map_cook_error(error: &CookError) -> MeshCookError {
    match error {
        CookError::EmptyMesh => MeshCookError::Empty,
        CookError::LengthMismatch { field, .. } => MeshCookError::Degenerate {
            detail: format!("length mismatch in {field}"),
        },
        CookError::IndexOutOfRange(index) => MeshCookError::IndexOutOfRange(*index),
        CookError::NonFinite(field) => MeshCookError::NonFinite(field),
        CookError::BadFactor { field, .. } => MeshCookError::Degenerate {
            detail: format!("bad {field} factor"),
        },
        CookError::DuplicateSlot(_) => MeshCookError::Degenerate {
            detail: "duplicate texture slot".to_string(),
        },
        CookError::TextureDecode(detail) => MeshCookError::Degenerate {
            detail: bound_detail(detail),
        },
        CookError::TextureTooLarge { count } => MeshCookError::TooLarge { count: *count },
        CookError::BadTextureDimensions { width, height } => MeshCookError::Degenerate {
            detail: format!("bad texture dimensions {width}x{height}"),
        },
    }
}

/// Map an import failure onto the cook error without echoing raw dumps.
fn map_import_error(error: MeshImportError) -> MeshCookError {
    match &error {
        MeshImportError::TooLarge { count } => MeshCookError::TooLarge { count: *count },
        _ => MeshCookError::Parse {
            detail: bound_detail(&error.to_string()),
        },
    }
}

/// Triangle count of one mesh (`indices.len() / 3`); units are triangles.
pub fn tri_count(mesh: &CookedMesh) -> u32 {
    (mesh.indices.len() / 3).min(u32::MAX as usize) as u32
}

/// Refuse a mesh over `max` triangles; exact-budget inputs pass.
pub fn check_budget(mesh: &CookedMesh, max: u32) -> Result<(), MeshCookError> {
    let tris = tri_count(mesh);
    if tris > max {
        return Err(MeshCookError::OverBudget { tris, max });
    }
    Ok(())
}

/// Largest triangle area in the mesh (meters squared), index order.
fn largest_area_m2(mesh: &CookedMesh) -> f32 {
    let mut worst = 0.0f32;
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let (Some(p0), Some(p1), Some(p2)) = (
            mesh.positions.get(a),
            mesh.positions.get(b),
            mesh.positions.get(c),
        ) else {
            continue;
        };
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        let area = 0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        if area.is_finite() && area > worst {
            worst = area;
        }
    }
    worst
}

/// Validate plus refuse zero-surface input: every triangle at or below
/// [`ZERO_AREA_M2`] means there is no shape to simplify or collide with.
pub(crate) fn validate_cookable(mesh: &CookedMesh) -> Result<(), MeshCookError> {
    validate_mesh(mesh).map_err(|e| map_cook_error(&e))?;
    if largest_area_m2(mesh) <= ZERO_AREA_M2 {
        return Err(MeshCookError::Degenerate {
            detail: format!("all {} triangles zero-area", tri_count(mesh)),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Quadric simplification lives in [`super::collapse`] (Garland-Heckbert error
// quadrics, midpoint collapses, one small step per function). [`simplify`] is
// re-exported above, so the cook path and its digest are unchanged.
// ---------------------------------------------------------------------------

/// Triangle target for `level` of a `base_tris` chain; at least one triangle.
pub fn lod_target(base_tris: u32, level: usize) -> u32 {
    let ratio = if level < LOD_RATIOS.len() {
        LOD_RATIOS[level]
    } else {
        LOD_RATIOS[LOD_RATIOS.len() - 1]
    };
    ((base_tris as f32) * ratio)
        .ceil()
        .clamp(1.0, u32::MAX as f32) as u32
}

/// Build the LOD chain: `lods[0]` is the base clone, each later level
/// simplifies the previous one toward its [`lod_target`]. Every level meets
/// its target whenever the mesh has collapsible edges; levels that already
/// meet their target clone through so the chain always holds `levels` meshes.
pub fn build_lod_chain(mesh: &CookedMesh, levels: usize) -> Result<Vec<CookedMesh>, MeshCookError> {
    validate_cookable(mesh)?;
    let levels = levels.clamp(1, MAX_LOD_LEVELS);
    let base_tris = tri_count(mesh);
    let mut lods: Vec<CookedMesh> = Vec::with_capacity(levels);
    lods.push(mesh.clone());
    for level in 1..levels {
        let target = lod_target(base_tris, level);
        let prev = lods[level - 1].clone();
        if tri_count(&prev) <= target {
            lods.push(prev);
        } else {
            lods.push(simplify(&prev, target)?);
        }
    }
    Ok(lods)
}

// ---------------------------------------------------------------------------
// Offline index ordering: vertex-cache plus overdraw passes.
// ---------------------------------------------------------------------------

/// Post-transform vertex-cache size the cook orders for, in entries. A small
/// modeled LRU matches the `16..=32` entry FIFOs of the target GPUs; the same
/// mesh always orders the same way for this size.
pub const VERTEX_CACHE_SIZE: usize = 32;
/// Triangles per overdraw cluster. Clusters sort front-to-back while the
/// triangle order inside each cluster keeps its cache-optimal sequence, so
/// one cluster never scatters wider than [`OVERDRAW_CLUSTER_TRIS`] triangles.
pub const OVERDRAW_CLUSTER_TRIS: usize = 128;

/// Cooked payload bytes of one mesh: attributes plus indices, in bytes.
/// Reordering only permutes or compacts, so this count never grows; the
/// parity harness asserts it stays equal on fully referenced meshes.
pub fn lod_bytes(mesh: &CookedMesh) -> usize {
    mesh.positions.len() * 12
        + mesh.normals.len() * 12
        + mesh.uv0.len() * 8
        + mesh.tangents.len() * 16
        + mesh.indices.len() * 4
}

/// `u32` index as `usize` for slicing; saturates past the pointer width
/// (unreachable on 32/64-bit targets, so live meshes resolve exactly and
/// absurd ones miss their `.get`, never wrap).
pub(crate) fn index_usize(index: u32) -> usize {
    usize::try_from(index).unwrap_or(usize::MAX)
}

/// `usize` count as `u32` for index buffers; saturates past `u32::MAX`
/// (meshes that large cannot exist in memory, so live code never saturates).
pub(crate) fn count_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// `u64` count as `f32` for miss ratios: exact on every input
/// (round-half-even on the mantissa; every `u64` magnitude fits the float
/// exponent range, so nothing saturates). Replaces a silent conversion
/// without changing one bit the keep-old guards decide on.
fn u64_to_f32(count: u64) -> f32 {
    if count == 0 {
        return 0.0;
    }
    // Top bit to bit 63, then round the top 24 bits half-even.
    let head = count.leading_zeros();
    let body = count.wrapping_shl(head);
    let top = u32::try_from(body >> 40).unwrap_or(u32::MAX);
    let tail = body & 0xFF_FFFF_FFFF;
    const HALF: u64 = 1 << 39;
    let round_up = tail > HALF || (tail == HALF && (top & 1) == 1);
    let mut mantissa = top.wrapping_add(u32::from(round_up));
    let mut biased = 63u32.wrapping_sub(head).wrapping_add(127);
    if mantissa > 0xFF_FFFF {
        mantissa = 0;
        biased = biased.wrapping_add(1);
    }
    f32::from_bits(biased.wrapping_shl(23) | (mantissa & 0x7F_FFFF))
}

/// Post-transform vertex-cache miss ratio of one index buffer, `0.0..=1.0`.
/// Units are misses per index reference. Simulates an LRU cache of
/// `cache_size` entries over the buffer in order, so reruns agree. Empty
/// buffers score `0.0`; a zero-size cache scores `1.0` on any reference.
pub fn vertex_cache_miss_ratio(indices: &[u32], cache_size: usize) -> f32 {
    if indices.is_empty() {
        return 0.0;
    }
    if cache_size == 0 {
        return 1.0;
    }
    let mut cache: Vec<u32> = Vec::new();
    let mut misses: u64 = 0;
    for index in indices {
        if let Some(hit) = cache.iter().position(|cached| cached == index) {
            cache.remove(hit);
            cache.push(*index);
        } else {
            misses += 1;
            if cache.len() >= cache_size {
                cache.remove(0);
            }
            cache.push(*index);
        }
    }
    let total = u64::try_from(indices.len()).unwrap_or(u64::MAX);
    u64_to_f32(misses) / u64_to_f32(total)
}

/// True when a mesh is safe to reorder: exact triangles, every corner inside
/// the vertex arrays, and attribute lanes matching the positions.
fn valid_for_order(mesh: &CookedMesh) -> bool {
    !mesh.positions.is_empty()
        && !mesh.indices.is_empty()
        && mesh.indices.len().is_multiple_of(3)
        && mesh.normals.len() == mesh.positions.len()
        && mesh.uv0.len() == mesh.positions.len()
        && mesh.tangents.len() == mesh.positions.len()
        && mesh
            .indices
            .iter()
            .all(|index| index_usize(*index) < mesh.positions.len())
}

/// Reference `vertex` through the modeled LRU cache, evicting the stalest
/// entry once the cache is full (`wrapping_add` keeps absurd meshes total).
fn cache_touch(
    cache: &mut Vec<u32>,
    in_cache: &mut [bool],
    stamp: &mut [u32],
    tick: &mut u32,
    vertex: usize,
    size: usize,
) {
    *tick = tick.wrapping_add(1);
    if let Some(slot) = stamp.get_mut(vertex) {
        *slot = *tick;
    }
    if in_cache.get(vertex) == Some(&true) {
        if let Some(hit) = cache
            .iter()
            .position(|cached| index_usize(*cached) == vertex)
        {
            cache.remove(hit);
            cache.push(count_u32(vertex));
        }
        return;
    }
    if cache.len() >= size {
        let mut victim = 0usize;
        let mut oldest = u32::MAX;
        for (slot, cached) in cache.iter().enumerate() {
            let age = stamp.get(index_usize(*cached)).copied().unwrap_or(u32::MAX);
            if age < oldest {
                oldest = age;
                victim = slot;
            }
        }
        let evicted = cache.remove(victim);
        if let Some(slot) = in_cache.get_mut(index_usize(evicted)) {
            *slot = false;
        }
    }
    cache.push(count_u32(vertex));
    if let Some(slot) = in_cache.get_mut(vertex) {
        *slot = true;
    }
}

/// Lowest-index live triangle touching `vertex`, or `None`.
fn live_tri_for(adjacency: &[Vec<u32>], live: &[bool], vertex: usize) -> Option<u32> {
    adjacency.get(vertex).and_then(|tris| {
        tris.iter()
            .copied()
            .find(|tri| live.get(index_usize(*tri)) == Some(&true))
    })
}

/// Mutable Tipsify walk state: triangle corners, liveness, valences,
/// adjacency, the modeled cache and the emit order. One struct keeps the
/// step functions small enough to review.
struct TipsifyState {
    /// Triangle corners in input order.
    corners: Vec<[u32; 3]>,
    /// Triangles still waiting for emission.
    live: Vec<bool>,
    /// Live-triangle count per vertex.
    valence: Vec<u32>,
    /// Live-triangle ids per vertex, input order.
    adjacency: Vec<Vec<u32>>,
    /// Modeled cache members.
    cache: Vec<u32>,
    /// Membership flags shadowing `cache`.
    in_cache: Vec<bool>,
    /// Last-touch tick per vertex.
    stamp: Vec<u32>,
    /// Touch clock; wraps instead of overflowing.
    tick: u32,
    /// One-ring neighbor stack for cache dead ends.
    dead: Vec<u32>,
    /// Emitted triangle ids.
    order: Vec<u32>,
    /// Input-order restart cursor.
    cursor: usize,
    /// Modeled cache size in entries.
    cache_size: usize,
}

impl TipsifyState {
    /// Build the walk state: valences and adjacency from the corners.
    fn new(corners: Vec<[u32; 3]>, vert_count: usize, cache_size: usize) -> Self {
        let tris = corners.len();
        let live = vec![true; tris];
        let mut valence = vec![0u32; vert_count];
        let mut adjacency: Vec<Vec<u32>> = vec![Vec::new(); vert_count];
        for (id, tri) in corners.iter().enumerate() {
            let tri_id = count_u32(id);
            for corner in tri {
                let vertex = index_usize(*corner);
                if let Some(slot) = valence.get_mut(vertex) {
                    *slot += 1;
                }
                if let Some(list) = adjacency.get_mut(vertex) {
                    list.push(tri_id);
                }
            }
        }
        Self {
            corners,
            live,
            valence,
            adjacency,
            cache: Vec::new(),
            in_cache: vec![false; vert_count],
            stamp: vec![0u32; vert_count],
            tick: 0,
            dead: Vec::new(),
            order: Vec::with_capacity(tris),
            cursor: 0,
            cache_size: cache_size.max(1),
        }
    }

    /// Next triangle to emit: most-recent cache vertex first, then the
    /// dead-end stack, then input order. Ties break on the lowest index.
    fn next(&mut self) -> Option<u32> {
        let mut members = self.cache.clone();
        members.sort_unstable_by(|a, b| {
            let old = self.stamp.get(index_usize(*a)).copied().unwrap_or(0);
            let young = self.stamp.get(index_usize(*b)).copied().unwrap_or(0);
            young.cmp(&old).then_with(|| a.cmp(b))
        });
        for member in members {
            let vertex = index_usize(member);
            if self.valence.get(vertex).copied().unwrap_or(0) == 0 {
                continue;
            }
            if let Some(tri) = live_tri_for(&self.adjacency, &self.live, vertex) {
                return Some(tri);
            }
        }
        while self.order.len() < self.corners.len() {
            let Some(vertex) = self.dead.pop() else {
                break;
            };
            let at = index_usize(vertex);
            if self.valence.get(at).copied().unwrap_or(0) == 0 {
                continue;
            }
            if let Some(tri) = live_tri_for(&self.adjacency, &self.live, at) {
                return Some(tri);
            }
        }
        while self.order.len() < self.corners.len() && self.cursor < self.corners.len() {
            let id = self.cursor;
            self.cursor += 1;
            if self.live.get(id) == Some(&true) {
                return Some(count_u32(id));
            }
        }
        None
    }

    /// Emit one triangle: retire it, touch its corners through the modeled
    /// cache, and stack its one-ring neighbors in index order.
    fn emit(&mut self, tri: u32) {
        let id = index_usize(tri);
        if let Some(slot) = self.live.get_mut(id) {
            *slot = false;
        }
        self.order.push(tri);
        if let Some(corners) = self.corners.get(id).copied() {
            for corner in corners {
                let vertex = index_usize(corner);
                if let Some(slot) = self.valence.get_mut(vertex) {
                    *slot = slot.saturating_sub(1);
                }
                cache_touch(
                    &mut self.cache,
                    &mut self.in_cache,
                    &mut self.stamp,
                    &mut self.tick,
                    vertex,
                    self.cache_size,
                );
                if let Some(ring) = self.adjacency.get(vertex).cloned() {
                    for other in ring {
                        let id = index_usize(other);
                        if self.live.get(id) != Some(&true) {
                            continue;
                        }
                        if let Some(shared) = self.corners.get(id) {
                            for peer in *shared {
                                let valence =
                                    self.valence.get(index_usize(peer)).copied().unwrap_or(0);
                                if valence > 0 {
                                    self.dead.push(peer);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Tipsify-school emit order (Sander-Zhang-Strain family, reimplemented from
/// the idea, no vendored code): the walk state above visits the modeled
/// cache most-recent-first with deterministic tie-breaks, so the same mesh
/// emits the same triangle order twice. Index-order loops only, `+ - * /`
/// on integer keys, no hash-map iteration, no wall clock.
fn tipsify_order(indices: &[u32], vert_count: usize, cache_size: usize) -> Vec<u32> {
    let corners: Vec<[u32; 3]> = indices
        .chunks_exact(3)
        .map(|tri| [tri[0], tri[1], tri[2]])
        .collect();
    let mut state = TipsifyState::new(corners, vert_count, cache_size);
    while state.order.len() < state.corners.len() {
        let Some(tri) = state.next() else {
            break;
        };
        state.emit(tri);
    }
    state.order
}

/// Rebuild one mesh in triangle `order`, compacting vertices to first-use
/// order. The triangle count and the surface are unchanged; unreferenced
/// vertices compact away, so LOD bytes never grow and hull containment
/// survives the pass.
fn reorder_compact(mesh: &CookedMesh, order: &[u32]) -> CookedMesh {
    let corners: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|tri| [tri[0], tri[1], tri[2]])
        .collect();
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for tri in order {
        if let Some(corner) = corners.get(index_usize(*tri)) {
            indices.extend_from_slice(corner);
        }
    }
    let mut remap = vec![u32::MAX; mesh.positions.len()];
    let mut next = 0u32;
    for index in &indices {
        if let Some(slot) = remap.get_mut(index_usize(*index)) {
            if *slot == u32::MAX {
                *slot = next;
                next += 1;
            }
        }
    }
    let mut order_verts: Vec<usize> = (0..mesh.positions.len())
        .filter(|vert| remap.get(*vert) != Some(&u32::MAX))
        .collect();
    order_verts.sort_unstable_by_key(|vert| remap.get(*vert).copied().unwrap_or(u32::MAX));
    let mut positions = Vec::with_capacity(order_verts.len());
    let mut normals = Vec::with_capacity(order_verts.len());
    let mut uv0 = Vec::with_capacity(order_verts.len());
    let mut tangents = Vec::with_capacity(order_verts.len());
    for old in order_verts {
        if let (Some(p), Some(n), Some(t), Some(g)) = (
            mesh.positions.get(old),
            mesh.normals.get(old),
            mesh.uv0.get(old),
            mesh.tangents.get(old),
        ) {
            positions.push(*p);
            normals.push(*n);
            uv0.push(*t);
            tangents.push(*g);
        }
    }
    for index in indices.iter_mut() {
        if let Some(mapped) = remap.get(index_usize(*index)) {
            *index = *mapped;
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

/// Reorder one mesh for the post-transform vertex cache, offline only. Runs
/// the Tipsify-school pass plus first-use compaction, then keeps the old
/// triangle order when the candidate scores worse, so the miss ratio never
/// regresses. Pure permutation plus compaction: the surface, the silhouette
/// and the seam error are identical to the input at equal or fewer bytes.
/// Out-of-range or ragged inputs clone through unchanged, never guessed.
pub fn optimize_vertex_cache(mesh: &CookedMesh) -> CookedMesh {
    if !valid_for_order(mesh) {
        return mesh.clone();
    }
    let order = tipsify_order(&mesh.indices, mesh.positions.len(), VERTEX_CACHE_SIZE);
    if order.len() * 3 != mesh.indices.len() {
        return mesh.clone();
    }
    let candidate = reorder_compact(mesh, &order);
    if vertex_cache_miss_ratio(&candidate.indices, VERTEX_CACHE_SIZE)
        <= vertex_cache_miss_ratio(&mesh.indices, VERTEX_CACHE_SIZE)
    {
        candidate
    } else {
        mesh.clone()
    }
}

/// Reorder one mesh for overdraw, offline only. Groups consecutive triangles
/// into [`OVERDRAW_CLUSTER_TRIS`] clusters and stable-sorts the clusters
/// front-to-back along the longest AABB axis (ties keep input order, so
/// reruns agree). Pure triangle permutation: attributes, counts, bytes and
/// the surface are untouched. Degenerate bounds clone through unchanged.
pub fn optimize_overdraw(mesh: &CookedMesh) -> CookedMesh {
    if !valid_for_order(mesh) {
        return mesh.clone();
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in &mesh.positions {
        for axis in 0..3 {
            if p[axis] < min[axis] {
                min[axis] = p[axis];
            }
            if p[axis] > max[axis] {
                max[axis] = p[axis];
            }
        }
    }
    let extent = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    if extent.iter().any(|e| !e.is_finite()) {
        return mesh.clone();
    }
    let mut axis = 0usize;
    for candidate in 1..3 {
        if extent[candidate] > extent[axis] {
            axis = candidate;
        }
    }
    let corners: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|tri| [tri[0], tri[1], tri[2]])
        .collect();
    let tris = corners.len();
    let clusters = tris.div_ceil(OVERDRAW_CLUSTER_TRIS);
    let mut key = vec![0.0f64; clusters];
    for (id, tri) in corners.iter().enumerate() {
        let cluster = id / OVERDRAW_CLUSTER_TRIS;
        let mut mid = 0.0f64;
        for corner in tri {
            if let Some(p) = mesh.positions.get(index_usize(*corner)) {
                mid += f64::from(p[axis]);
            }
        }
        if let Some(slot) = key.get_mut(cluster) {
            *slot += mid;
        }
    }
    for (cluster, slot) in key.iter_mut().enumerate() {
        let first = cluster * OVERDRAW_CLUSTER_TRIS;
        let count = tris.saturating_sub(first).clamp(1, OVERDRAW_CLUSTER_TRIS);
        // `count` is clamped to the cluster size, so the `u32` below never
        // saturates in practice; the fallback keeps absurd meshes total.
        let faces = u32::try_from(count.saturating_mul(3)).unwrap_or(u32::MAX);
        let mean = *slot / f64::from(faces);
        *slot = if mean.is_finite() { mean } else { 0.0 };
    }
    let mut ids: Vec<usize> = (0..clusters).collect();
    ids.sort_unstable_by(|a, b| key[*a].total_cmp(&key[*b]).then_with(|| a.cmp(b)));
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for cluster in ids {
        let first = cluster * OVERDRAW_CLUSTER_TRIS;
        let last = (first + OVERDRAW_CLUSTER_TRIS).min(tris);
        for tri in &corners[first..last] {
            indices.extend_from_slice(tri);
        }
    }
    let mut out = mesh.clone();
    out.indices = indices;
    out
}

/// Full offline index order: vertex-cache pass first, overdraw clusters on
/// top, keeping the better cache scorer at each step so the miss ratio never
/// regresses against the input. Runs at cook time only, never in the fixed
/// tick: scratch buffers grow with the mesh. Deterministic and byte-stable:
/// the same mesh orders the same way twice.
pub fn optimize_index_order(mesh: &CookedMesh) -> CookedMesh {
    let cached = optimize_vertex_cache(mesh);
    if !valid_for_order(&cached) {
        return cached;
    }
    let drawn = optimize_overdraw(&cached);
    if vertex_cache_miss_ratio(&drawn.indices, VERTEX_CACHE_SIZE)
        <= vertex_cache_miss_ratio(&cached.indices, VERTEX_CACHE_SIZE)
    {
        drawn
    } else {
        cached
    }
}

// ---------------------------------------------------------------------------
// Collision hulls: AABB plus bounding sphere plus support points, std math.
// ---------------------------------------------------------------------------

/// Collision hull for one LOD: the AABB, the bounding sphere and a bounded
/// support-point set, all in meters. Built to contain its mesh; prove it with
/// [`CollisionHull::contains_mesh`] (the acceptance gate checks every hull).
#[derive(Clone, Debug, PartialEq)]
pub struct CollisionHull {
    /// AABB minimum corner in meters.
    pub min: [f32; 3],
    /// AABB maximum corner in meters.
    pub max: [f32; 3],
    /// Bounding-sphere center in meters.
    pub center: [f32; 3],
    /// Bounding-sphere radius in meters.
    pub radius: f32,
    /// Extreme mesh vertices along fixed axes and diagonals (bounded).
    pub support: Vec<[f32; 3]>,
}

impl CollisionHull {
    /// Build the hull of one mesh: AABB over all vertices, sphere around the
    /// centroid grown to the farthest vertex, supports as the farthest vertex
    /// per fixed direction (lowest index wins ties, so reruns agree).
    pub fn build(mesh: &CookedMesh) -> Result<Self, MeshCookError> {
        validate_mesh(mesh).map_err(|e| map_cook_error(&e))?;
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut sum = [0.0f64; 3];
        let mut count = 0u64;
        for p in &mesh.positions {
            for i in 0..3 {
                if p[i] < min[i] {
                    min[i] = p[i];
                }
                if p[i] > max[i] {
                    max[i] = p[i];
                }
                sum[i] += f64::from(p[i]);
            }
            count += 1;
        }
        if count == 0 {
            return Err(MeshCookError::Empty);
        }
        let center = [
            (sum[0] / count as f64) as f32,
            (sum[1] / count as f64) as f32,
            (sum[2] / count as f64) as f32,
        ];
        // Grow the radius to the farthest vertex so containment holds by
        // construction (same distance math as `contains_point` below).
        let mut radius = 0.0f32;
        for p in &mesh.positions {
            let dx = p[0] - center[0];
            let dy = p[1] - center[1];
            let dz = p[2] - center[2];
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            if dist.is_finite() && dist > radius {
                radius = dist;
            }
        }
        // Fixed probe set: axes plus cube diagonals; deterministic order.
        let inv = 0.577_350_26_f32;
        let dirs: [[f32; 3]; 14] = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [inv, inv, inv],
            [inv, inv, -inv],
            [inv, -inv, inv],
            [inv, -inv, -inv],
            [-inv, inv, inv],
            [-inv, inv, -inv],
            [-inv, -inv, inv],
            [-inv, -inv, -inv],
        ];
        let mut support: Vec<[f32; 3]> = Vec::new();
        for dir in dirs {
            let mut best: Option<(usize, f32)> = None;
            for (i, p) in mesh.positions.iter().enumerate() {
                let dot = p[0] * dir[0] + p[1] * dir[1] + p[2] * dir[2];
                // Strict `>` keeps the lowest index on ties.
                if dot.is_finite() && best.is_none_or(|(_, b)| dot > b) {
                    best = Some((i, dot));
                }
            }
            if let Some((i, _)) = best {
                if let Some(p) = mesh.positions.get(i) {
                    if !support.iter().any(|q| q == p) && support.len() < MAX_HULL_VERTS {
                        support.push(*p);
                    }
                }
            }
        }
        Ok(Self {
            min,
            max,
            center,
            radius,
            support,
        })
    }

    /// True when `point` sits inside the AABB and the sphere (plus slack).
    pub fn contains_point(&self, point: &[f32; 3]) -> bool {
        for (i, v) in point.iter().enumerate() {
            if !v.is_finite() {
                return false;
            }
            if *v < self.min[i] - HULL_EPS_M || *v > self.max[i] + HULL_EPS_M {
                return false;
            }
        }
        let dx = point[0] - self.center[0];
        let dy = point[1] - self.center[1];
        let dz = point[2] - self.center[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        dist.is_finite() && dist <= self.radius + HULL_EPS_M
    }

    /// True when every vertex of `mesh` sits inside this hull.
    pub fn contains_mesh(&self, mesh: &CookedMesh) -> bool {
        mesh.positions.iter().all(|p| self.contains_point(p))
    }
}

// ---------------------------------------------------------------------------
// Names, FRAC chunks, and the full cook.
// ---------------------------------------------------------------------------

/// Cooked asset name for `level` of `base`: LOD0 keeps the base name, later
/// levels take the `_LODn` suffix convention.
pub fn lod_name(base: &str, level: usize) -> String {
    if level == 0 {
        base.to_string()
    } else {
        format!("{base}_LOD{level}")
    }
}

/// True for the `_LOD1.._LOD3` suffix convention (LOD0 keeps the base name).
pub fn is_lod_name(name: &str) -> bool {
    name.len() > 5
        && (name.ends_with("_LOD1") || name.ends_with("_LOD2") || name.ends_with("_LOD3"))
}

/// True for the `COL_` collider convention.
pub fn is_col_name(name: &str) -> bool {
    name.starts_with(COL_PREFIX) && name.len() > COL_PREFIX.len()
}

/// True for the `SOCKET_` socket convention.
pub fn is_socket_name(name: &str) -> bool {
    name.starts_with(SOCKET_PREFIX) && name.len() > SOCKET_PREFIX.len()
}

/// Piece token after the `FRAC_` prefix, or `None` when the name is bare.
pub fn frac_piece(name: &str) -> Option<&str> {
    name.strip_prefix(FRAC_PREFIX)
        .filter(|piece| !piece.is_empty())
}

/// True when `name` marks a destruction fracture chunk (`FRAC_` plus a piece
/// token).
pub fn is_frac_name(name: &str) -> bool {
    frac_piece(name).is_some()
}

/// Piece charset: ASCII alphanumerics plus `_` and `-`, within budget.
fn valid_piece(piece: &str) -> bool {
    !piece.is_empty()
        && piece.len() <= MAX_FRAC_PIECE_CHARS
        && piece
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Strict `FRAC_<piece>` check: prefix, charset and length, bounded errors.
pub fn validate_frac_name(name: &str) -> Result<&str, MeshCookError> {
    if name.chars().count() > MAX_FRAC_NAME_CHARS {
        return Err(MeshCookError::TooLarge {
            count: name.chars().count(),
        });
    }
    match frac_piece(name) {
        Some(piece) if valid_piece(piece) => Ok(piece),
        _ => Err(MeshCookError::Degenerate {
            detail: format!("bad chunk name {}", bound_detail(name)),
        }),
    }
}

/// One destruction fracture chunk: its node name plus its cooked mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct FracChunk {
    /// Node name (`FRAC_<piece>`, `#prim` suffix when a node holds several).
    pub name: String,
    /// Cooked chunk mesh in meters.
    pub mesh: CookedMesh,
}

/// Split already-cooked named meshes into chunks: `FRAC_`-prefixed entries
/// validate strictly and survive, every other name is skipped (body, `COL_`
/// colliders, sockets and LODs cook through the mesh path instead).
pub fn split_frac_chunks(
    named: Vec<(String, CookedMesh)>,
) -> Result<Vec<FracChunk>, MeshCookError> {
    let mut chunks = Vec::new();
    for (name, mesh) in named {
        if !is_frac_name(&name) {
            continue;
        }
        validate_frac_name(&name)?;
        validate_mesh(&mesh).map_err(|e| map_cook_error(&e))?;
        if chunks.len() >= MAX_FRAC_CHUNKS {
            return Err(MeshCookError::TooLarge {
                count: chunks.len() + 1,
            });
        }
        chunks.push(FracChunk { name, mesh });
    }
    Ok(chunks)
}

/// Collect `FRAC_` chunks from glTF bytes: imports the geometry through the
/// glTF importer, then maps `FRAC_<piece>` node names (document order) onto their mesh
/// primitives. Multi-primitive nodes emit one chunk per primitive with a
/// `#prim` suffix. Files without chunk nodes cook to an empty vec.
pub fn import_frac_chunks(bytes: &[u8]) -> Result<Vec<FracChunk>, MeshCookError> {
    if bytes.len() > MAX_BYTES {
        return Err(MeshCookError::TooLarge { count: bytes.len() });
    }
    if bytes.is_empty() {
        return Err(MeshCookError::Parse {
            detail: "empty input".to_string(),
        });
    }
    let imported = import_gltf_bytes(bytes).map_err(map_import_error)?;
    collect_frac_chunks(bytes, &imported)
}

/// Map `FRAC_` node names onto an already-run import (same primitive order as
/// [`import_gltf_bytes`], so node mesh indices resolve to cooked meshes).
fn collect_frac_chunks(
    bytes: &[u8],
    imported: &super::mesh_import::GltfImport,
) -> Result<Vec<FracChunk>, MeshCookError> {
    let parsed = gltf::Gltf::from_slice(bytes).map_err(|e| MeshCookError::Parse {
        detail: bound_detail(&format!("{e:?}")),
    })?;
    // Base cooked index per mesh index, in primitive order (the order the
    // importer pushes them, keyed by mesh so iteration order cannot drift).
    let mut base: Vec<usize> = vec![0; parsed.document.meshes().len()];
    let mut cursor = 0usize;
    for mesh in parsed.document.meshes() {
        base[mesh.index()] = cursor;
        cursor += mesh.primitives().len();
    }
    let mut chunks = Vec::new();
    for node in parsed.document.nodes() {
        let name = node.name().unwrap_or("");
        if !is_frac_name(name) {
            continue;
        }
        validate_frac_name(name)?;
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let prims = mesh.primitives().len();
        for prim in 0..prims {
            let Some(slot) = base.get(mesh.index()).and_then(|b| b.checked_add(prim)) else {
                continue;
            };
            let Some(cooked) = imported.meshes.get(slot) else {
                continue;
            };
            if chunks.len() >= MAX_FRAC_CHUNKS {
                return Err(MeshCookError::TooLarge {
                    count: chunks.len() + 1,
                });
            }
            let chunk_name = if prims == 1 {
                name.to_string()
            } else {
                format!("{name}#prim{prim}")
            };
            chunks.push(FracChunk {
                name: chunk_name,
                mesh: cooked.clone(),
            });
        }
    }
    Ok(chunks)
}

/// One cooked mesh: the LOD chain, one containing hull per LOD, and the digest
/// proving the cook (same mesh plus class cooks the same digest twice).
#[derive(Clone, Debug, PartialEq)]
pub struct MeshCookOutput {
    /// Class whose budget this mesh met.
    pub class: MeshClass,
    /// LOD0 triangle count.
    pub base_tris: u32,
    /// LOD chain; `lods[0]` is the base mesh.
    pub lods: Vec<CookedMesh>,
    /// One hull per LOD; `hulls[i]` contains `lods[i]`.
    pub hulls: Vec<CollisionHull>,
    /// Stable digest over the class, every LOD and every hull.
    pub digest: u64,
}

/// One cooked file: every mesh cooked plus the `FRAC_` chunk list.
#[derive(Clone, Debug, PartialEq)]
pub struct CookedAsset {
    /// One output per imported triangle mesh, document order.
    pub meshes: Vec<MeshCookOutput>,
    /// `FRAC_` chunks, node order.
    pub chunks: Vec<FracChunk>,
    /// Stable digest over every mesh digest plus every chunk.
    pub digest: u64,
}

/// Cook one mesh: validate, simplify over-budget input down to budget, build
/// the LOD chain and one containing hull per LOD. Every LOD leaves
/// vertex-cache and overdraw ordered ([`optimize_index_order`], offline only):
/// the same mesh plus class still cooks the same bytes twice, with the same
/// triangle counts and the same surface, only in a more GPU-friendly order.
/// Cook order follows the meshoptimizer (MIT, zeux/meshoptimizer) pattern:
/// simplify to budget first, then vertex-cache, then overdraw.
pub fn cook_mesh(mesh: &CookedMesh, class: MeshClass) -> Result<MeshCookOutput, MeshCookError> {
    validate_cookable(mesh)?;
    let max = class.max_tris();
    let base_mesh: CookedMesh = if tri_count(mesh) > max {
        let simplified = simplify(mesh, max)?;
        let simplified_tris = tri_count(&simplified);
        if simplified_tris > max {
            return Err(MeshCookError::OverBudget {
                tris: simplified_tris,
                max,
            });
        }
        simplified
    } else {
        mesh.clone()
    };
    let base_tris = tri_count(&base_mesh);
    let unordered = build_lod_chain(&base_mesh, MAX_LOD_LEVELS)?;
    let lods: Vec<CookedMesh> = unordered.iter().map(optimize_index_order).collect();
    let mut hulls = Vec::with_capacity(lods.len());
    for lod in &lods {
        hulls.push(CollisionHull::build(lod)?);
    }
    let mut out = MeshCookOutput {
        class,
        base_tris,
        lods,
        hulls,
        digest: 0,
    };
    out.digest = cook_digest(&out);
    Ok(out)
}

/// Cook glTF bytes against a class budget: every mesh cooks through
/// [`cook_mesh`] (over-budget meshes simplify down to budget), and
/// `FRAC_<piece>` nodes collect into chunks for destruction.
pub fn cook_gltf_bytes(bytes: &[u8], class: MeshClass) -> Result<CookedAsset, MeshCookError> {
    if bytes.len() > MAX_BYTES {
        return Err(MeshCookError::TooLarge { count: bytes.len() });
    }
    if bytes.is_empty() {
        return Err(MeshCookError::Parse {
            detail: "empty input".to_string(),
        });
    }
    let imported = import_gltf_bytes(bytes).map_err(map_import_error)?;
    let mut meshes = Vec::with_capacity(imported.meshes.len());
    for mesh in &imported.meshes {
        meshes.push(cook_mesh(mesh, class)?);
    }
    let chunks = collect_frac_chunks(bytes, &imported)?;
    let mut asset = CookedAsset {
        meshes,
        chunks,
        digest: 0,
    };
    asset.digest = asset_digest(&asset);
    Ok(asset)
}

/// Feed one cooked mesh by its bits: counts plus every attribute and index.
fn feed_mesh(state: &mut Fnv64, mesh: &CookedMesh) {
    state.write_u64(len_word(mesh.positions.len()));
    for array in mesh.positions.iter().chain(mesh.normals.iter()) {
        for v in array {
            state.write_f32(*v);
        }
    }
    for uv in &mesh.uv0 {
        state.write_f32(uv[0]);
        state.write_f32(uv[1]);
    }
    for tangent in &mesh.tangents {
        for v in tangent {
            state.write_f32(*v);
        }
    }
    state.write_u64(len_word(mesh.indices.len()));
    for index in &mesh.indices {
        state.write(&index.to_le_bytes());
    }
}

/// Feed one hull by its bits: corners, sphere and support points.
fn feed_hull(state: &mut Fnv64, hull: &CollisionHull) {
    for v in hull.min.iter().chain(hull.max.iter()) {
        state.write_f32(*v);
    }
    for v in &hull.center {
        state.write_f32(*v);
    }
    state.write_f32(hull.radius);
    state.write_u64(len_word(hull.support.len()));
    for p in &hull.support {
        for v in p {
            state.write_f32(*v);
        }
    }
}

/// Stable `u64` over one mesh cook: class tag, base count, every LOD and hull.
pub fn cook_digest(output: &MeshCookOutput) -> u64 {
    let mut state = Fnv64::new();
    state.write_u8(output.class.tag());
    state.write_u32(output.base_tris);
    state.write_u64(len_word(output.lods.len()));
    for lod in &output.lods {
        feed_mesh(&mut state, lod);
        state.write_u8(0xFA);
    }
    for hull in &output.hulls {
        feed_hull(&mut state, hull);
        state.write_u8(0xFB);
    }
    state.finish()
}

/// Stable `u64` over one file cook: every mesh digest plus every chunk name
/// and mesh. Identical bytes plus class give the identical digest twice.
pub fn asset_digest(asset: &CookedAsset) -> u64 {
    let mut state = Fnv64::new();
    state.write_u64(len_word(asset.meshes.len()));
    for mesh in &asset.meshes {
        state.write_u64(mesh.digest);
    }
    state.write_u8(0xFC);
    state.write_u64(len_word(asset.chunks.len()));
    for chunk in &asset.chunks {
        state.write_str(&chunk.name);
        state.write_u8(0);
        feed_mesh(&mut state, &chunk.mesh);
    }
    state.finish()
}

#[cfg(test)]
mod tests {
    use super::super::cooked::validate_mesh;
    use super::*;
    use crate::collapse::norm3;

    /// Grid mesh on the unit square: `n` cells per side, `2*n*n` triangles,
    /// `+Z` normals, `0..=1` UVs, `+X` tangents. Deterministic row-major order.
    fn grid_mesh(n: usize) -> CookedMesh {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        let mut tangents = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                let x = i as f32 / n as f32;
                let y = j as f32 / n as f32;
                positions.push([x, y, 0.0]);
                normals.push([0.0, 0.0, 1.0]);
                uv0.push([x, y]);
                tangents.push([1.0, 0.0, 0.0, 1.0]);
            }
        }
        let mut indices = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let a = (j * (n + 1) + i) as u32;
                let b = a + 1;
                let c = a + n as u32 + 1;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
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

    /// Unit cube shell: 8 verts, 12 tris, outward-ish normals from the center.
    fn cube_mesh() -> CookedMesh {
        let corners: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        let positions = corners.to_vec();
        let normals = corners.iter().map(|p| norm3(*p)).collect();
        let uv0 = corners
            .iter()
            .map(|p| [p[0] * 0.25 + 0.5, p[1] * 0.25 + 0.5])
            .collect();
        let tangents = vec![[1.0, 0.0, 0.0, 1.0]; 8];
        let indices = vec![
            0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0, 5,
            4, 3, 7, 6, 3, 6, 2,
        ];
        CookedMesh {
            positions,
            normals,
            uv0,
            tangents,
            indices,
        }
    }

    /// Tiny standard-base64 encoder for building glTF data URIs in tests.
    fn b64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
        for chunk in bytes.chunks(3) {
            let (a, b, c) = (
                chunk[0] as u32,
                *chunk.get(1).unwrap_or(&0) as u32,
                *chunk.get(2).unwrap_or(&0) as u32,
            );
            let bits = (a << 16) | (b << 8) | c;
            out.push(ALPHABET[((bits >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((bits >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((bits >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(bits & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    /// Minimal single-mesh glTF with `tris` disjoint triangles, `+Z` normals
    /// and `0..=1` UVs packed per triangle. No material, no tangents, no unit
    /// declaration: unitless by construction (the cook never guesses scale).
    fn make_gltf(tris: usize) -> Vec<u8> {
        assert!(tris >= 1, "test helper needs at least one triangle");
        let verts = tris * 3;
        let mut positions = Vec::with_capacity(verts * 12);
        let mut normals = Vec::with_capacity(verts * 12);
        let mut uvs = Vec::with_capacity(verts * 8);
        let mut indices = Vec::with_capacity(tris * 12);
        let mut xmax = 0.0f32;
        for tri in 0..tris {
            let x = tri as f32 * 2.0;
            for (dx, dy, uv) in [
                (0.0f32, 0.0f32, [0.0f32, 0.0f32]),
                (1.0f32, 0.0f32, [1.0f32, 0.0f32]),
                (0.0f32, 1.0f32, [0.0f32, 1.0f32]),
            ] {
                positions.extend_from_slice(&(x + dx).to_le_bytes());
                positions.extend_from_slice(&dy.to_le_bytes());
                positions.extend_from_slice(&0.0f32.to_le_bytes());
                normals.extend_from_slice(&0.0f32.to_le_bytes());
                normals.extend_from_slice(&0.0f32.to_le_bytes());
                normals.extend_from_slice(&1.0f32.to_le_bytes());
                uvs.extend_from_slice(&uv[0].to_le_bytes());
                uvs.extend_from_slice(&uv[1].to_le_bytes());
                xmax = xmax.max(x + dx);
            }
            for k in 0..3 {
                indices.extend_from_slice(&((tri * 3 + k) as u32).to_le_bytes());
            }
        }
        let pos_len = positions.len();
        let nrm_len = normals.len();
        let uv_len = uvs.len();
        let idx_len = indices.len();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&positions);
        buffer.extend_from_slice(&normals);
        buffer.extend_from_slice(&uvs);
        buffer.extend_from_slice(&indices);
        let json = format!(
            concat!(
                "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],",
                "\"nodes\":[{{\"mesh\":0,\"name\":\"Body\"}}],",
                "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":1,\"NORMAL\":2,",
                "\"TEXCOORD_0\":3}},\"indices\":0}}]}}],",
                "\"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{uri}\",",
                "\"byteLength\":{len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{nrm_off},\"byteLength\":{nrm_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{uv_off},\"byteLength\":{uv_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{idx_off},\"byteLength\":{idx_len}}}],",
                "\"accessors\":[",
                "{{\"bufferView\":3,\"byteOffset\":0,\"componentType\":5125,",
                "\"count\":{counts},\"type\":\"SCALAR\"}},",
                "{{\"bufferView\":0,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":{verts},\"type\":\"VEC3\",\"min\":[0.0,0.0,0.0],",
                "\"max\":[{xmax},1.0,0.0]}},",
                "{{\"bufferView\":1,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":{verts},\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":{verts},\"type\":\"VEC2\"}}]}}"
            ),
            uri = b64_encode(&buffer),
            len = buffer.len(),
            pos_len = pos_len,
            nrm_off = pos_len,
            nrm_len = nrm_len,
            uv_off = pos_len + nrm_len,
            uv_len = uv_len,
            idx_off = pos_len + nrm_len + uv_len,
            idx_len = idx_len,
            counts = tris * 3,
            verts = verts,
            xmax = xmax,
        );
        json.into_bytes()
    }

    /// Two-mesh glTF with named nodes: `FRAC_piece_a` and an unchunked `Body`
    /// share nothing, `FRAC_piece_b` rides the second mesh. Mesh 0 sits at
    /// `x = 0`, mesh 1 at `x = 5`, so chunk contents are distinguishable.
    fn make_frac_gltf() -> Vec<u8> {
        let mut buffer = Vec::new();
        let mut views: Vec<(usize, usize)> = Vec::new();
        let push = |bytes: &[u8], buffer: &mut Vec<u8>, views: &mut Vec<(usize, usize)>| {
            let off = buffer.len();
            buffer.extend_from_slice(bytes);
            views.push((off, bytes.len()));
        };
        let mut pos0 = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for v in p {
                pos0.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut pos1 = Vec::new();
        for p in [[5.0f32, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 0.0]] {
            for v in p {
                pos1.extend_from_slice(&v.to_le_bytes());
            }
        }
        let idx: Vec<u8> = [0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
        push(&pos0, &mut buffer, &mut views);
        push(&idx, &mut buffer, &mut views);
        push(&pos1, &mut buffer, &mut views);
        push(&idx, &mut buffer, &mut views);
        let json = format!(
            concat!(
                "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0,1,2]}}],",
                "\"nodes\":[",
                "{{\"mesh\":0,\"name\":\"FRAC_piece_a\"}},",
                "{{\"mesh\":1,\"name\":\"FRAC_piece_b\"}},",
                "{{\"mesh\":1,\"name\":\"Body\"}}],",
                "\"meshes\":[",
                "{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1}}]}},",
                "{{\"primitives\":[{{\"attributes\":{{\"POSITION\":2}},\"indices\":3}}]}}],",
                "\"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{uri}\",",
                "\"byteLength\":{len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":{o0},\"byteLength\":{l0}}},",
                "{{\"buffer\":0,\"byteOffset\":{o1},\"byteLength\":{l1}}},",
                "{{\"buffer\":0,\"byteOffset\":{o2},\"byteLength\":{l2}}},",
                "{{\"buffer\":0,\"byteOffset\":{o3},\"byteLength\":{l3}}}],",
                "\"accessors\":[",
                "{{\"bufferView\":0,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":3,\"type\":\"VEC3\",\"min\":[0.0,0.0,0.0],\"max\":[1.0,1.0,0.0]}},",
                "{{\"bufferView\":1,\"byteOffset\":0,\"componentType\":5123,",
                "\"count\":3,\"type\":\"SCALAR\"}},",
                "{{\"bufferView\":2,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":3,\"type\":\"VEC3\",\"min\":[5.0,0.0,0.0],\"max\":[6.0,1.0,0.0]}},",
                "{{\"bufferView\":3,\"byteOffset\":0,\"componentType\":5123,",
                "\"count\":3,\"type\":\"SCALAR\"}}]}}"
            ),
            uri = b64_encode(&buffer),
            len = buffer.len(),
            o0 = views[0].0,
            l0 = views[0].1,
            o1 = views[1].0,
            l1 = views[1].1,
            o2 = views[2].0,
            l2 = views[2].1,
            o3 = views[3].0,
            l3 = views[3].1,
        );
        json.into_bytes()
    }

    #[test]
    fn lod_chain_meets_triangle_budgets() {
        // 12x12 grid: 288 tris; LOD targets are 288, 144, 72, 36.
        let mesh = grid_mesh(12);
        assert_eq!(tri_count(&mesh), 288);
        let lods = build_lod_chain(&mesh, MAX_LOD_LEVELS).expect("grid simplifies");
        assert_eq!(lods.len(), MAX_LOD_LEVELS);
        assert_eq!(tri_count(&lods[0]), 288, "LOD0 keeps the base mesh");
        assert_eq!(lods[0], mesh);
        for (level, lod) in lods.iter().enumerate() {
            let target = lod_target(288, level);
            let tris = tri_count(lod);
            assert!(
                tris <= target,
                "LOD{level} holds {tris} tris over target {target}"
            );
            validate_mesh(lod).expect("every LOD stays a valid cooked mesh");
        }
        for pair in lods.windows(2) {
            assert!(
                tri_count(&pair[1]) <= tri_count(&pair[0]),
                "chain never grows: {} then {}",
                tri_count(&pair[0]),
                tri_count(&pair[1])
            );
        }
        // Quadric collapse is deterministic: the same mesh simplifies identically.
        let again = build_lod_chain(&mesh, MAX_LOD_LEVELS).expect("recook");
        assert_eq!(lods, again);
    }

    #[test]
    fn quadric_simplification_holds_shape() {
        let cube = cube_mesh();
        assert_eq!(tri_count(&cube), 12);
        let half = simplify(&cube, 6).expect("cube halves");
        let tris = tri_count(&half);
        assert!((1..=6).contains(&tris), "cube halves to {tris} tris");
        validate_mesh(&half).expect("simplified cube stays valid");
        // At most the target: a second run agrees bit for bit.
        assert_eq!(half, simplify(&cube, 6).expect("deterministic"));
        // At-target inputs clone through untouched.
        assert_eq!(simplify(&cube, 12).expect("at target"), cube);
        assert_eq!(simplify(&cube, 99).expect("over target"), cube);
    }

    #[test]
    fn hulls_contain_their_mesh() {
        let mesh = grid_mesh(8);
        let lods = build_lod_chain(&mesh, MAX_LOD_LEVELS).expect("grid chain");
        for (level, lod) in lods.iter().enumerate() {
            let hull = CollisionHull::build(lod).expect("hull builds");
            assert!(
                hull.contains_mesh(lod),
                "LOD{level} hull contains its {}-tri mesh",
                tri_count(lod)
            );
            assert!(
                !hull.support.is_empty() && hull.support.len() <= MAX_HULL_VERTS,
                "support set is bounded and non-empty"
            );
            // A far point is outside; determinism holds across builds.
            assert!(!hull.contains_point(&[100.0, 100.0, 100.0]));
            assert_eq!(hull, CollisionHull::build(lod).expect("hull deterministic"));
        }
        // The cube hull spans the full shell.
        let hull = CollisionHull::build(&cube_mesh()).expect("cube hull");
        assert!(hull.contains_mesh(&cube_mesh()));
        assert_eq!(hull.min, [-1.0, -1.0, -1.0]);
        assert_eq!(hull.max, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn frac_prefixed_chunks_import() {
        let bytes = make_frac_gltf();
        let chunks = import_frac_chunks(&bytes).expect("chunks import");
        assert_eq!(chunks.len(), 2, "only FRAC_ nodes chunk, got {chunks:?}");
        assert_eq!(chunks[0].name, "FRAC_piece_a");
        assert_eq!(chunks[1].name, "FRAC_piece_b");
        // Chunk contents follow their nodes: mesh 0 at x = 0, mesh 1 at x = 5.
        let xs = |mesh: &CookedMesh| mesh.positions.iter().map(|p| p[0]).collect::<Vec<_>>();
        assert_eq!(xs(&chunks[0].mesh), vec![0.0, 1.0, 0.0]);
        assert_eq!(xs(&chunks[1].mesh), vec![5.0, 6.0, 5.0]);
        for chunk in &chunks {
            validate_mesh(&chunk.mesh).expect("chunk mesh stays valid");
        }
        // Deterministic across imports.
        assert_eq!(chunks, import_frac_chunks(&bytes).expect("reimport"));
        // Files without chunk nodes cook to an empty chunk list.
        assert!(import_frac_chunks(&make_gltf(2)).expect("plain").is_empty());
        // The named-split path honors the same prefix rule.
        let named = vec![
            ("FRAC_piece_a".to_string(), chunks[0].mesh.clone()),
            ("Body".to_string(), chunks[0].mesh.clone()),
            ("COL_hit".to_string(), chunks[0].mesh.clone()),
        ];
        let split = split_frac_chunks(named).expect("split");
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].name, "FRAC_piece_a");
    }

    #[test]
    fn over_budget_meshes_refused_with_counts() {
        let mesh = grid_mesh(4);
        assert_eq!(tri_count(&mesh), 32);
        assert_eq!(
            check_budget(&mesh, 31),
            Err(MeshCookError::OverBudget { tris: 32, max: 31 })
        );
        check_budget(&mesh, 32).expect("exact budget passes");
        assert_eq!(
            cook_mesh(&mesh, MeshClass::Prop).map(|_| ()),
            Ok(()),
            "32 tris fit the 5k prop budget"
        );
        // Class budgets follow the content conventions.
        assert_eq!(MeshClass::Hero.max_tris(), HERO_MAX_TRIS);
        assert_eq!(MeshClass::Opponent.max_tris(), OPPONENT_MAX_TRIS);
        assert_eq!(MeshClass::Prop.max_tris(), PROP_MAX_TRIS);
        // Import-time path: a 5002-triangle file simplifies as a prop.
        let big = make_gltf(PROP_MAX_TRIS as usize + 2);
        let prop_asset = cook_gltf_bytes(&big, MeshClass::Prop).expect("over budget simplifies");
        assert_eq!(prop_asset.meshes.len(), 1);
        assert!(
            tri_count(&prop_asset.meshes[0].lods[0]) <= PROP_MAX_TRIS,
            "prop LOD0 over budget"
        );
        // The same file cooks as an opponent (25k budget).
        let asset = cook_gltf_bytes(&big, MeshClass::Opponent).expect("fits opponent");
        assert_eq!(asset.meshes.len(), 1);
        assert_eq!(asset.meshes[0].base_tris, PROP_MAX_TRIS + 2);
    }

    #[test]
    fn over_budget_simplifies_to_prop_budget() {
        // 5002-triangle prop input cooks to LOD0 within the 5000 budget.
        let bytes = make_gltf(PROP_MAX_TRIS as usize + 2);
        let asset = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("5002 simplifies");
        assert_eq!(asset.meshes.len(), 1);
        let out = &asset.meshes[0];
        let lod0_tris = tri_count(&out.lods[0]);
        assert!(
            lod0_tris <= PROP_MAX_TRIS,
            "LOD0 holds {lod0_tris} tris over prop budget {PROP_MAX_TRIS}"
        );
        assert!(
            out.base_tris <= PROP_MAX_TRIS,
            "base holds {} tris over prop budget",
            out.base_tris
        );
        assert!(
            !out.lods[0].indices.is_empty(),
            "simplified LOD0 keeps surface"
        );
        for (lod, hull) in out.lods.iter().zip(out.hulls.iter()) {
            assert!(hull.contains_mesh(lod), "every hull proves containment");
        }
        // Same bytes cook the same LOD0 twice.
        let again = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("recook");
        assert_eq!(asset, again);
        std::println!(
            "OVER-BUDGET-SIMPLIFY prop: input={} lod0={lod0_tris} base={} digest={}",
            PROP_MAX_TRIS + 2,
            out.base_tris,
            out.digest
        );
    }

    #[test]
    fn degenerate_inputs_refused() {
        assert_eq!(
            cook_mesh(&CookedMesh::default(), MeshClass::Prop),
            Err(MeshCookError::Empty)
        );
        let mut finite = grid_mesh(2);
        finite.positions[0][0] = f32::NAN;
        assert_eq!(
            cook_mesh(&finite, MeshClass::Prop),
            Err(MeshCookError::NonFinite("positions"))
        );
        // Collinear triangle: valid arrays, zero surface.
        let flat = CookedMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.5, 0.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
        };
        let err = cook_mesh(&flat, MeshClass::Prop).expect_err("flat refuses");
        assert!(matches!(err, MeshCookError::Degenerate { .. }), "got {err}");
        assert_eq!(
            simplify(&flat, 1).expect_err("simplify refuses"),
            err,
            "both paths refuse identically"
        );
        let mut ranged = grid_mesh(2);
        ranged.indices[0] = 999;
        assert_eq!(
            cook_mesh(&ranged, MeshClass::Prop),
            Err(MeshCookError::IndexOutOfRange(999))
        );
        assert_eq!(
            import_frac_chunks(b"not gltf").expect_err("malformed"),
            import_frac_chunks(b"not gltf").expect_err("deterministic refusal"),
        );
    }

    #[test]
    fn cook_identical_twice() {
        let mesh = grid_mesh(8);
        let first = cook_mesh(&mesh, MeshClass::Hero).expect("first cook");
        let second = cook_mesh(&mesh, MeshClass::Hero).expect("second cook");
        assert_eq!(first, second, "same mesh plus class, same cook");
        assert_eq!(first.digest, cook_digest(&second));
        // Any input bit flip changes the digest.
        let mut moved = mesh.clone();
        moved.positions[0][0] += 0.5;
        let other = cook_mesh(&moved, MeshClass::Hero).expect("moved cook");
        assert_ne!(first.digest, other.digest);
        // File cooks are stable too, including chunks and hulls.
        let bytes = make_frac_gltf();
        let a = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("file cook");
        let b = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("file recook");
        assert_eq!(a, b);
        assert_eq!(a.digest, asset_digest(&b));
        assert_eq!(a.meshes.len(), 2, "one output per mesh");
        assert_eq!(a.chunks.len(), 2, "both FRAC_ nodes collect");
        assert_eq!(a.meshes.len(), a.meshes.len());
        for output in &a.meshes {
            assert_eq!(output.lods.len(), MAX_LOD_LEVELS);
            assert_eq!(output.hulls.len(), output.lods.len());
            for (lod, hull) in output.lods.iter().zip(output.hulls.iter()) {
                assert!(hull.contains_mesh(lod), "every hull proves containment");
            }
        }
    }

    /// Sorted position-bit triples of every triangle: reorder-proof surface
    /// identity. Two meshes with equal lists carry the same triangles.
    fn sorted_tris(mesh: &CookedMesh) -> Vec<[[u32; 3]; 3]> {
        let mut out = Vec::new();
        for tri in mesh.indices.chunks_exact(3) {
            let mut corners = [[0u32; 3]; 3];
            for (slot, corner) in tri.iter().enumerate() {
                let p = mesh
                    .positions
                    .get(*corner as usize)
                    .copied()
                    .unwrap_or([0.0; 3]);
                corners[slot] = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
            }
            corners.sort_unstable();
            out.push(corners);
        }
        out.sort_unstable();
        out
    }

    /// Deterministic cache-hostile triangle permutation: stride 7 is coprime
    /// to the grid triangle counts below, so every triangle moves.
    fn scrambled_tris(mesh: &CookedMesh) -> CookedMesh {
        let tris: Vec<[u32; 3]> = mesh
            .indices
            .chunks_exact(3)
            .map(|tri| [tri[0], tri[1], tri[2]])
            .collect();
        let count = tris.len();
        let mut indices = Vec::with_capacity(mesh.indices.len());
        for k in 0..count {
            if let Some(tri) = tris.get((k * 7) % count) {
                indices.extend_from_slice(tri);
            }
        }
        let mut out = mesh.clone();
        out.indices = indices;
        out
    }

    #[test]
    fn vertex_cache_miss_ratio_scores_lru() {
        assert_eq!(vertex_cache_miss_ratio(&[], VERTEX_CACHE_SIZE), 0.0);
        assert_eq!(vertex_cache_miss_ratio(&[0, 1, 2], 0), 1.0);
        // One triangle: three cold misses.
        assert_eq!(vertex_cache_miss_ratio(&[0, 1, 2], VERTEX_CACHE_SIZE), 1.0);
        // A repeat hits the cache: three misses over six references.
        assert_eq!(
            vertex_cache_miss_ratio(&[0, 1, 2, 0, 1, 2], VERTEX_CACHE_SIZE),
            0.5
        );
        let grid = grid_mesh(4);
        let ratio = vertex_cache_miss_ratio(&grid.indices, VERTEX_CACHE_SIZE);
        assert_eq!(
            ratio,
            vertex_cache_miss_ratio(&grid.indices, VERTEX_CACHE_SIZE)
        );
        assert!((0.0..=1.0).contains(&ratio));
    }

    #[test]
    fn u64_to_f32_matches_cast_on_boundaries() {
        // Oracle: the test-only conversion is the bit pattern the helper keeps.
        let cases: &[u64] = &[
            0,
            1,
            2,
            3,
            5,
            255,
            256,
            1_000,
            65_535,
            65_536,
            16_777_215,
            16_777_216,
            16_777_217,
            16_777_218,
            33_554_431,
            33_554_432,
            33_554_433,
            4_294_967_295,
            4_294_967_296,
            9_007_199_254_740_991,
            9_007_199_254_740_992,
            18_446_744_073_709_551_615,
        ];
        for count in cases {
            assert_eq!(u64_to_f32(*count), *count as f32, "count {count}");
        }
    }

    #[test]
    fn index_order_keeps_bytes_never_regresses_cache() {
        let mesh = grid_mesh(12);
        assert_eq!(tri_count(&mesh), 288);
        let before = vertex_cache_miss_ratio(&mesh.indices, VERTEX_CACHE_SIZE);
        let ordered = optimize_index_order(&mesh);
        // Parity: same payload bytes, same counts, same triangles.
        assert_eq!(lod_bytes(&ordered), lod_bytes(&mesh));
        assert_eq!(tri_count(&ordered), tri_count(&mesh));
        assert_eq!(ordered.positions.len(), mesh.positions.len());
        assert_eq!(sorted_tris(&ordered), sorted_tris(&mesh));
        // The keep-old-path guard holds: never worse than the input.
        let after = vertex_cache_miss_ratio(&ordered.indices, VERTEX_CACHE_SIZE);
        assert!(after <= before, "cache {after} over {before}");
        validate_mesh(&ordered).expect("ordered mesh stays valid");
        // A strided order is strictly repaired, with bytes and surface kept.
        let scrambled = scrambled_tris(&grid_mesh(24));
        let fixed = optimize_vertex_cache(&scrambled);
        assert_eq!(lod_bytes(&fixed), lod_bytes(&scrambled));
        assert_eq!(sorted_tris(&fixed), sorted_tris(&scrambled));
        let sick = vertex_cache_miss_ratio(&scrambled.indices, VERTEX_CACHE_SIZE);
        let cured = vertex_cache_miss_ratio(&fixed.indices, VERTEX_CACHE_SIZE);
        assert!(cured < sick, "repair {cured} under {sick}");
        assert_eq!(fixed, optimize_vertex_cache(&scrambled));
        // Overdraw is a pure permutation: parity plus determinism.
        let drawn = optimize_overdraw(&mesh);
        assert_eq!(lod_bytes(&drawn), lod_bytes(&mesh));
        assert_eq!(sorted_tris(&drawn), sorted_tris(&mesh));
        assert_eq!(drawn, optimize_overdraw(&mesh));
        // Full pipeline is deterministic and hulls still contain their LODs.
        assert_eq!(ordered, optimize_index_order(&mesh));
        let hull = CollisionHull::build(&ordered).expect("hull of ordered");
        assert!(hull.contains_mesh(&ordered));
        // Cook level: same bytes and surface per LOD as the unordered chain.
        let chain = build_lod_chain(&mesh, MAX_LOD_LEVELS).expect("chain");
        let cooked = cook_mesh(&mesh, MeshClass::Prop).expect("cook");
        assert_eq!(cooked, cook_mesh(&mesh, MeshClass::Prop).expect("recook"));
        assert_eq!(cooked.lods.len(), chain.len());
        for (level, pair) in chain.iter().zip(cooked.lods.iter()).enumerate() {
            assert_eq!(
                lod_bytes(pair.1),
                lod_bytes(pair.0),
                "LOD{level} bytes parity"
            );
            assert_eq!(
                sorted_tris(pair.1),
                sorted_tris(pair.0),
                "LOD{level} same surface"
            );
        }
    }

    #[test]
    fn ordering_proof_hero_and_prop() {
        // Hero-class grid: 2*141*141 = 39_762 tris. Ordering only here; the
        // quadric simplify path is unchanged and dominates a full hero cook.
        let hero = grid_mesh(141);
        assert_eq!(tri_count(&hero), 39_762);
        let hero_bytes = lod_bytes(&hero);
        let hero_before = vertex_cache_miss_ratio(&hero.indices, VERTEX_CACHE_SIZE);
        let start = std::time::Instant::now();
        let hero_ordered = optimize_index_order(&hero);
        let hero_ms = start.elapsed().as_millis();
        let hero_after = vertex_cache_miss_ratio(&hero_ordered.indices, VERTEX_CACHE_SIZE);
        assert_eq!(lod_bytes(&hero_ordered), hero_bytes, "hero bytes parity");
        assert_eq!(
            sorted_tris(&hero_ordered),
            sorted_tris(&hero),
            "hero same surface"
        );
        assert!(hero_after <= hero_before, "hero cache never regresses");
        assert_eq!(
            hero_ordered,
            optimize_index_order(&hero),
            "hero deterministic"
        );
        std::println!(
            "ORDER-PROOF hero: tris=39762 bytes={hero_bytes} miss {hero_before:.4}->{hero_after:.4} wall_ms={hero_ms}"
        );
        // Prop-class grid: exactly 5_000 tris, full cook before/after per LOD.
        let prop = grid_mesh(50);
        assert_eq!(tri_count(&prop), PROP_MAX_TRIS);
        let start = std::time::Instant::now();
        let chain = build_lod_chain(&prop, MAX_LOD_LEVELS).expect("prop chain");
        let raw_ms = start.elapsed().as_millis();
        let start = std::time::Instant::now();
        let cooked = cook_mesh(&prop, MeshClass::Prop).expect("prop cook");
        let cook_ms = start.elapsed().as_millis();
        assert_eq!(
            cooked,
            cook_mesh(&prop, MeshClass::Prop).expect("prop recook")
        );
        assert_eq!(cooked.lods.len(), chain.len());
        for (level, pair) in chain.iter().zip(cooked.lods.iter()).enumerate() {
            let miss_before = vertex_cache_miss_ratio(&pair.0.indices, VERTEX_CACHE_SIZE);
            let miss_after = vertex_cache_miss_ratio(&pair.1.indices, VERTEX_CACHE_SIZE);
            assert_eq!(
                lod_bytes(pair.1),
                lod_bytes(pair.0),
                "prop LOD{level} bytes"
            );
            assert!(
                miss_after <= miss_before,
                "prop LOD{level} cache never regresses"
            );
            std::println!(
                "ORDER-PROOF prop LOD{level}: tris={} bytes={} miss {miss_before:.4}->{miss_after:.4}",
                tri_count(pair.1),
                lod_bytes(pair.1),
            );
        }
        std::println!(
            "ORDER-PROOF prop cook: unordered_chain_ms={raw_ms} ordered_cook_ms={cook_ms} digest={}",
            cooked.digest,
        );
    }

    #[test]
    fn naming_conventions_validated() {
        assert_eq!(lod_name("hero", 0), "hero");
        assert_eq!(lod_name("hero", 2), "hero_LOD2");
        assert!(is_lod_name("hero_LOD1") && is_lod_name("hero_LOD3"));
        assert!(!is_lod_name("hero") && !is_lod_name("hero_LOD4") && !is_lod_name("LOD1"));
        assert!(is_col_name("COL_hit") && !is_col_name("Body") && !is_col_name("COL_"));
        assert!(is_socket_name("SOCKET_hand") && !is_socket_name("Body"));
        assert_eq!(frac_piece("FRAC_chunk_3"), Some("chunk_3"));
        assert!(!is_frac_name("FRAC_") && !is_frac_name("Body"));
        assert_eq!(validate_frac_name("FRAC_a-1").expect("valid"), "a-1");
        assert!(validate_frac_name("FRAC_bad name").is_err());
        assert!(validate_frac_name("Body").is_err());
        let long = format!("FRAC_{}", "a".repeat(MAX_FRAC_PIECE_CHARS + 1));
        assert!(
            validate_frac_name(&long).is_err(),
            "over-long pieces refuse"
        );
        assert!(!format!("{}", MeshCookError::Empty).is_empty());
    }
}
