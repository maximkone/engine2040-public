//! csg: offline CSG fracture bake from blade-plane cuts.
//!
//! Consumer: bonded collections in the arena match; baked chunks convert
//! through [`to_bonded_desc`] and break through the bonded impact path. Data
//! owner: `engine-destruction`. Units: meters for mesh vertices, planes as a
//! unit normal plus a distance. Determinism: index-order loops with `+ - * /`
//! and `sqrt` only, bit-exact floats, no hash maps, no wall clock. Errors:
//! typed [`CsgError`]; degenerate planes and missing cuts refuse, never no-op.
//!
//! Patterns reimplement the brusher BSP school (plane-split BSP with
//! union/intersect/subtract/knife over triangle shells, never vendored):
//! [`knife`] splits one shell, [`brush_subtract`] drops the tool interior,
//! [`brush_intersect`] keeps it, [`shell_union`] concatenates disjoint shells.
//! Shells stay open along cuts (no caps); chunk boxes close them for the
//! bonded collection.

use crate::bake3d::Chunk3;
use crate::bonded::{Bond, BondedDesc, BreakError, MIN_MASS};
use crate::shards3d::in_contact;

/// Most cut planes one bake accepts; bounds the chunk search up front.
pub const MAX_CUT_PLANES: usize = 64;
/// Most vertices one input mesh holds.
pub const MAX_CSG_VERTS: usize = 65_536;
/// Most triangles one input mesh holds.
pub const MAX_CSG_TRIS: usize = 65_536;
/// Most live triangles across all cells while a bake runs.
pub const MAX_BAKE_TRIS: usize = 262_144;
/// Most chunks one bake emits; matches the bonded collection bound.
pub const MAX_CSG_CHUNKS: usize = 1024;
/// Largest accepted vertex coordinate; meters.
pub const MAX_COORD_M: f32 = 10_000.0;
/// Thinnest box a bonded collection accepts; thinner baked slabs inflate to
/// this, meters.
pub const MIN_SLAB_M: f32 = 1e-4;
/// Bottom-layer tolerance when baked chunks pick anchors; meters.
pub const CSG_ANCHOR_EPS_M: f32 = 1e-3;
/// Shortest accepted cut-plane normal; shorter normals are degenerate.
pub const MIN_PLANE_LEN: f32 = 1e-9;
/// Vertex distances within this of a plane count as on-plane; meters.
const COPLANAR_EPS: f32 = 1e-6;

/// Cut plane: `normal . p = dist`. Validated and normalized at bake time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutPlane {
    /// Plane normal; need not be unit, must be finite and non-zero.
    pub normal: [f32; 3],
    /// Signed distance of the plane from the origin along `normal`; meters.
    pub dist: f32,
}

/// Triangle shell to fracture: vertex soup plus whole-triangle indices.
#[derive(Clone, Debug, PartialEq)]
pub struct PropMesh {
    /// Vertices in meters; 1..=`MAX_CSG_VERTS`, finite, within bounds.
    pub positions: Vec<[f32; 3]>,
    /// Whole triangles addressing `positions`; 3..=`MAX_CSG_TRIS` * 3.
    pub indices: Vec<u32>,
}

impl PropMesh {
    /// Triangle count of this shell.
    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// One baked fracture chunk: an open shell patch with its box and `FRAC_` name.
#[derive(Clone, Debug, PartialEq)]
pub struct FractureChunk {
    /// Chunk name (`FRAC_0000`, ...); satisfies the asset import naming
    /// convention.
    pub name: String,
    /// Open shell patch cut from the prop mesh; meters.
    pub mesh: PropMesh,
    /// AABB minimum corner; meters.
    pub min: [f32; 3],
    /// AABB maximum corner; meters.
    pub max: [f32; 3],
}

impl FractureChunk {
    /// Triangle count of this chunk.
    pub fn tri_count(&self) -> usize {
        self.mesh.indices.len() / 3
    }

    /// Bonded-collection box for this chunk: AABB center plus half extents
    /// with slabs below `MIN_SLAB_M` inflated and the mass floored at the
    /// collection's minimum, so every baked chunk passes its validation.
    pub fn to_chunk3(&self) -> Chunk3 {
        let mut half = [
            (self.max[0] - self.min[0]) * 0.5,
            (self.max[1] - self.min[1]) * 0.5,
            (self.max[2] - self.min[2]) * 0.5,
        ];
        for h in &mut half {
            // Validated finite vertices keep halves finite; the floor only
            // widens paper-thin patches cut from flat shells.
            if *h < MIN_SLAB_M {
                *h = MIN_SLAB_M;
            }
        }
        Chunk3 {
            x: (self.min[0] + self.max[0]) * 0.5,
            y: (self.min[1] + self.max[1]) * 0.5,
            z: (self.min[2] + self.max[2]) * 0.5,
            hx: half[0],
            hy: half[1],
            hz: half[2],
            // A flat patch inflated to the slab floor can fall under the
            // collection's mass floor and refuse the whole fracture.
            mass: (half[0] * half[1] * half[2] * 8.0).max(MIN_MASS),
        }
    }
}

/// Why a CSG bake failed; every refused input maps to one variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CsgError {
    /// No vertices or no indices supplied.
    EmptyMesh,
    /// Vertex at the carried slot is non-finite or out of bounds.
    BadVertex(usize),
    /// Index addresses no vertex; the offending value is carried.
    BadIndex(u32),
    /// Index count is not a multiple of three.
    BadTopology,
    /// A mesh bound was exceeded; the offending count is carried.
    TooLarge {
        /// Offending vertex, triangle or live-triangle count.
        count: usize,
    },
    /// No cut planes supplied; a bake with no cuts is a copy, refused here.
    NoCuts,
    /// More than `MAX_CUT_PLANES` planes supplied; count carried.
    TooManyPlanes(usize),
    /// Plane at the carried position has a zero or non-finite normal or dist.
    DegeneratePlane(usize),
    /// Cut at the carried position splits no live cell; misses never no-op.
    DegenerateCut(usize),
    /// The bake exceeds `MAX_CSG_CHUNKS`; live count carried.
    TooManyChunks(usize),
    /// A boolean removed the whole mesh; there is no shell left to ship.
    EmptyResult,
    /// Blade or swing input is non-finite, zero or parallel.
    BadBlade,
}

impl core::fmt::Display for CsgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CsgError::EmptyMesh => write!(f, "csg mesh has no vertices or indices"),
            CsgError::BadVertex(i) => write!(f, "csg vertex {i} is out of bounds"),
            CsgError::BadIndex(i) => write!(f, "csg index {i} addresses no vertex"),
            CsgError::BadTopology => write!(f, "csg indices do not form whole triangles"),
            CsgError::TooLarge { count } => write!(f, "csg mesh too large: {count}"),
            CsgError::NoCuts => write!(f, "csg bake needs at least one cut plane"),
            CsgError::TooManyPlanes(n) => write!(f, "csg bake exceeds MAX_CUT_PLANES: {n}"),
            CsgError::DegeneratePlane(i) => write!(f, "csg plane {i} is degenerate"),
            CsgError::DegenerateCut(i) => write!(f, "csg cut {i} splits nothing"),
            CsgError::TooManyChunks(n) => write!(f, "csg bake exceeds MAX_CSG_CHUNKS: {n}"),
            CsgError::EmptyResult => write!(f, "csg operation removed the whole mesh"),
            CsgError::BadBlade => write!(f, "csg blade input is degenerate"),
        }
    }
}

impl std::error::Error for CsgError {}

/// Validated unit plane used by every split; built once per cut, in order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NormPlane {
    n: [f32; 3],
    d: f32,
}

impl NormPlane {
    /// Signed distance: positive on the front side, exact-order float math.
    fn dist_to(&self, v: &[f32; 3]) -> f32 {
        self.n[0] * v[0] + self.n[1] * v[1] + self.n[2] * v[2] - self.d
    }
}

fn valid_coord(v: f32) -> bool {
    v.is_finite() && v.abs() <= MAX_COORD_M
}

pub(crate) fn validate_mesh(mesh: &PropMesh) -> Result<(), CsgError> {
    if mesh.positions.is_empty() || mesh.indices.is_empty() {
        return Err(CsgError::EmptyMesh);
    }
    if mesh.positions.len() > MAX_CSG_VERTS {
        return Err(CsgError::TooLarge {
            count: mesh.positions.len(),
        });
    }
    if mesh.indices.len() / 3 > MAX_CSG_TRIS {
        return Err(CsgError::TooLarge {
            count: mesh.indices.len() / 3,
        });
    }
    for (i, v) in mesh.positions.iter().enumerate() {
        if !valid_coord(v[0]) || !valid_coord(v[1]) || !valid_coord(v[2]) {
            return Err(CsgError::BadVertex(i));
        }
    }
    if !mesh.indices.len().is_multiple_of(3) {
        return Err(CsgError::BadTopology);
    }
    for index in &mesh.indices {
        // A u32 vertex id always fits usize on the targets we ship; a target too small to hold
        // it reads as out of range instead of addressing a wrapped slot.
        if usize::try_from(*index).map_or(true, |i| i >= mesh.positions.len()) {
            return Err(CsgError::BadIndex(*index));
        }
    }
    Ok(())
}

fn normalize_plane(plane: &CutPlane, index: usize) -> Result<NormPlane, CsgError> {
    let [nx, ny, nz] = plane.normal;
    if !nx.is_finite() || !ny.is_finite() || !nz.is_finite() || !plane.dist.is_finite() {
        return Err(CsgError::DegeneratePlane(index));
    }
    let len2 = nx * nx + ny * ny + nz * nz;
    // `sqrt` is deterministic authority math; transcendentals never appear here.
    let len = len2.sqrt();
    if !len2.is_finite() || !len.is_finite() || len <= MIN_PLANE_LEN {
        return Err(CsgError::DegeneratePlane(index));
    }
    Ok(NormPlane {
        n: [nx / len, ny / len, nz / len],
        d: plane.dist / len,
    })
}

fn normalize_all(planes: &[CutPlane]) -> Result<Vec<NormPlane>, CsgError> {
    if planes.is_empty() {
        return Err(CsgError::NoCuts);
    }
    if planes.len() > MAX_CUT_PLANES {
        return Err(CsgError::TooManyPlanes(planes.len()));
    }
    let mut out = Vec::with_capacity(planes.len());
    for (i, plane) in planes.iter().enumerate() {
        out.push(normalize_plane(plane, i)?);
    }
    Ok(out)
}

/// Bit-exact corner equality for sliver rejection.
fn same_bits(a: &[f32; 3], b: &[f32; 3]) -> bool {
    a[0].to_bits() == b[0].to_bits()
        && a[1].to_bits() == b[1].to_bits()
        && a[2].to_bits() == b[2].to_bits()
}

fn push_tri(
    out_pos: &mut Vec<[f32; 3]>,
    out_idx: &mut Vec<u32>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) {
    // `MAX_BAKE_TRIS` (262_144) caps the live triangle count, so the emitted vertex count
    // stays far below `u32::MAX` and this base cannot truncate.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "MAX_BAKE_TRIS caps the emitted vertex count below u32::MAX"
    )]
    let base = out_pos.len() as u32;
    out_pos.extend_from_slice(&[a, b, c]);
    out_idx.extend_from_slice(&[base, base + 1, base + 2]);
}

/// Sutherland-Hodgman clip of one triangle against one half-space, fan-emitted.
/// On-plane edges emit bit-identical corners, which the fan skips as slivers.
fn clip_emit(
    corners: [[f32; 3]; 3],
    ds: [f32; 3],
    keep_front: bool,
    scratch: &mut Vec<[f32; 3]>,
    out_pos: &mut Vec<[f32; 3]>,
    out_idx: &mut Vec<u32>,
) {
    scratch.clear();
    for i in 0..3 {
        let (cur, dcur) = (corners[i], ds[i]);
        let (nxt, dnxt) = (corners[(i + 1) % 3], ds[(i + 1) % 3]);
        let cur_in = if keep_front { dcur >= 0.0 } else { dcur <= 0.0 };
        let nxt_in = if keep_front { dnxt >= 0.0 } else { dnxt <= 0.0 };
        if cur_in {
            scratch.push(cur);
        }
        if cur_in != nxt_in {
            // Straddling edges never divide by zero; the fallback covers a
            // denormal denominator with the deterministic edge midpoint.
            let mut t = dcur / (dcur - dnxt);
            if !t.is_finite() {
                t = 0.5;
            }
            scratch.push([
                cur[0] + t * (nxt[0] - cur[0]),
                cur[1] + t * (nxt[1] - cur[1]),
                cur[2] + t * (nxt[2] - cur[2]),
            ]);
        }
    }
    if scratch.len() >= 3 {
        for i in 1..scratch.len() - 1 {
            let (p0, p1, p2) = (scratch[0], scratch[i], scratch[i + 1]);
            if same_bits(&p0, &p1) || same_bits(&p1, &p2) || same_bits(&p2, &p0) {
                continue;
            }
            push_tri(out_pos, out_idx, p0, p1, p2);
        }
    }
}

/// Plane-split BSP kernel: every triangle goes front, back, or is cut at the
/// exact zero crossing. Fully coplanar triangles route front, never both.
fn split_mesh(mesh: &PropMesh, plane: &NormPlane) -> (PropMesh, PropMesh) {
    let mut front_pos = Vec::new();
    let mut front_idx = Vec::new();
    let mut back_pos = Vec::new();
    let mut back_idx = Vec::new();
    let mut scratch: Vec<[f32; 3]> = Vec::with_capacity(8);
    for tri in mesh.indices.chunks_exact(3) {
        // Callers validate shells, and splits only emit resolved corners; the u32 ids always
        // fit usize on the targets we ship, so the fallback only names that invariant.
        let (pa, pb, pc) = (
            mesh.positions[usize::try_from(tri[0]).unwrap_or(usize::MAX)],
            mesh.positions[usize::try_from(tri[1]).unwrap_or(usize::MAX)],
            mesh.positions[usize::try_from(tri[2]).unwrap_or(usize::MAX)],
        );
        let ds = [plane.dist_to(&pa), plane.dist_to(&pb), plane.dist_to(&pc)];
        if ds[0].abs() <= COPLANAR_EPS && ds[1].abs() <= COPLANAR_EPS && ds[2].abs() <= COPLANAR_EPS
        {
            push_tri(&mut front_pos, &mut front_idx, pa, pb, pc);
            continue;
        }
        let corners = [pa, pb, pc];
        clip_emit(
            corners,
            ds,
            true,
            &mut scratch,
            &mut front_pos,
            &mut front_idx,
        );
        clip_emit(
            corners,
            ds,
            false,
            &mut scratch,
            &mut back_pos,
            &mut back_idx,
        );
    }
    (
        PropMesh {
            positions: front_pos,
            indices: front_idx,
        },
        PropMesh {
            positions: back_pos,
            indices: back_idx,
        },
    )
}

/// Blade plane through `point` spanning `blade_dir` and `swing_dir`: the
/// normal is `blade x swing`, so a sword hit cuts along the blade plane.
pub fn blade_plane(
    point: &[f32; 3],
    blade_dir: &[f32; 3],
    swing_dir: &[f32; 3],
) -> Result<CutPlane, CsgError> {
    for v in [point, blade_dir, swing_dir] {
        for f in v {
            if !f.is_finite() {
                return Err(CsgError::BadBlade);
            }
        }
    }
    let cross = [
        blade_dir[1] * swing_dir[2] - blade_dir[2] * swing_dir[1],
        blade_dir[2] * swing_dir[0] - blade_dir[0] * swing_dir[2],
        blade_dir[0] * swing_dir[1] - blade_dir[1] * swing_dir[0],
    ];
    let len2 = cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2];
    let len = len2.sqrt();
    if !len2.is_finite() || !len.is_finite() || len <= MIN_PLANE_LEN {
        return Err(CsgError::BadBlade);
    }
    let n = [cross[0] / len, cross[1] / len, cross[2] / len];
    let dist = n[0] * point[0] + n[1] * point[1] + n[2] * point[2];
    if !dist.is_finite() {
        return Err(CsgError::BadBlade);
    }
    Ok(CutPlane { normal: n, dist })
}

/// Knife: split one shell along a plane. A missing cut refuses with
/// `DegenerateCut` instead of returning the input silently.
pub fn knife(mesh: &PropMesh, plane: &CutPlane) -> Result<(PropMesh, PropMesh), CsgError> {
    validate_mesh(mesh)?;
    let norm = normalize_plane(plane, 0)?;
    let (front, back) = split_mesh(mesh, &norm);
    if front.indices.is_empty() || back.indices.is_empty() {
        return Err(CsgError::DegenerateCut(0));
    }
    Ok((front, back))
}

/// Split one shell against every brush plane, keeping both sides per cut.
fn split_all(mesh: &PropMesh, brush: &[NormPlane]) -> Result<Vec<PropMesh>, CsgError> {
    let mut frags = vec![mesh.clone()];
    for plane in brush {
        let mut next = Vec::new();
        for frag in &frags {
            let (front, back) = split_mesh(frag, plane);
            if !front.indices.is_empty() {
                next.push(front);
            }
            if !back.indices.is_empty() {
                next.push(back);
            }
        }
        let total: usize = next.iter().map(|f| f.indices.len() / 3).sum();
        if total > MAX_BAKE_TRIS {
            return Err(CsgError::TooLarge { count: total });
        }
        frags = next;
    }
    Ok(frags)
}

/// True when the fragment centroid sits inside the convex brush (every tool
/// distance within slack); boundary patches count as inside, deterministically.
fn inside_brush(frag: &PropMesh, brush: &[NormPlane]) -> bool {
    let mut sum = [0.0f32; 3];
    for v in &frag.positions {
        sum[0] += v[0];
        sum[1] += v[1];
        sum[2] += v[2];
    }
    // Fragments always carry vertices, so the divisor is non-zero. A fragment comes from a
    // mesh validated at or below MAX_CSG_TRIS, and clipping adds at most one corner per edge,
    // so the count stays far below 2^24 and is exact in f32.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a fragment vertex count stays far below 2^24 (MAX_CSG_TRIS plus clip corners)"
    )]
    let n = frag.positions.len() as f32;
    let center = [sum[0] / n, sum[1] / n, sum[2] / n];
    brush.iter().all(|p| p.dist_to(&center) <= COPLANAR_EPS)
}

fn append_mesh(out_pos: &mut Vec<[f32; 3]>, out_idx: &mut Vec<u32>, mesh: &PropMesh) {
    // `MAX_BAKE_TRIS` caps the live count, so the appended base cannot truncate.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "MAX_BAKE_TRIS caps the live vertex count below u32::MAX"
    )]
    let base = out_pos.len() as u32;
    out_pos.extend_from_slice(&mesh.positions);
    out_idx.extend(mesh.indices.iter().map(|i| base + i));
}

/// Subtract: the shell minus the convex brush interior. Fully removed shells
/// refuse with `EmptyResult`; there is no empty shell type to return.
pub fn brush_subtract(mesh: &PropMesh, brush: &[CutPlane]) -> Result<PropMesh, CsgError> {
    validate_mesh(mesh)?;
    let planes = normalize_all(brush)?;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for frag in split_all(mesh, &planes)? {
        if !inside_brush(&frag, &planes) {
            append_mesh(&mut positions, &mut indices, &frag);
        }
    }
    if indices.is_empty() {
        return Err(CsgError::EmptyResult);
    }
    Ok(PropMesh { positions, indices })
}

/// Intersect: the shell clipped to the convex brush interior.
pub fn brush_intersect(mesh: &PropMesh, brush: &[CutPlane]) -> Result<PropMesh, CsgError> {
    validate_mesh(mesh)?;
    let planes = normalize_all(brush)?;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for frag in split_all(mesh, &planes)? {
        if inside_brush(&frag, &planes) {
            append_mesh(&mut positions, &mut indices, &frag);
        }
    }
    if indices.is_empty() {
        return Err(CsgError::EmptyResult);
    }
    Ok(PropMesh { positions, indices })
}

/// Union of two disjoint shells: deterministic concatenation in argument
/// order. Fracture chunks never overlap, so no interior faces need removal.
pub fn shell_union(a: &PropMesh, b: &PropMesh) -> Result<PropMesh, CsgError> {
    validate_mesh(a)?;
    validate_mesh(b)?;
    if a.positions.len() + b.positions.len() > MAX_CSG_VERTS
        || (a.indices.len() + b.indices.len()) / 3 > MAX_CSG_TRIS
    {
        return Err(CsgError::TooLarge {
            count: a.positions.len() + b.positions.len(),
        });
    }
    let mut positions = a.positions.clone();
    let mut indices = a.indices.clone();
    append_mesh(&mut positions, &mut indices, b);
    Ok(PropMesh { positions, indices })
}

/// Chunk name for cell `index`: zero-padded `FRAC_` piece tokens stay within
/// the asset import charset (ASCII alphanumerics) and length bounds.
pub fn frac_name(index: usize) -> String {
    let mut out = String::with_capacity(9);
    frac_name_into(index, &mut out);
    out
}

/// Chunk name for cell `index` written into caller-owned `out` without
/// allocating when `out` has capacity; matches [`frac_name`] exactly.
/// Reuse one `String` across a bake to keep the name loop heap-quiet.
/// Determinism: integer digits only, no float, no hash map.
pub fn frac_name_into(index: usize, out: &mut String) {
    out.clear();
    out.push_str("FRAC_");
    // `write!` reuses `out` capacity, so a reserved buffer moves no heap.
    // Writing into a `String` never fails, so the infallible result is
    // discarded instead of panicking.
    use core::fmt::Write as _;
    let _ = write!(out, "{index:04}");
}

/// Squared distance between two points; `+ - *` only.
pub(crate) fn dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

/// Bisector plane between sites `a` and `b`: its back side (distance at most
/// zero) holds the points at least as close to `a`. `None` when the sites
/// coincide (no usable normal).
pub(crate) fn plane_between(a: &[f32; 3], b: &[f32; 3]) -> Option<NormPlane> {
    let n = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !len.is_finite() || len <= MIN_PLANE_LEN {
        return None;
    }
    let n = [n[0] / len, n[1] / len, n[2] / len];
    let mid = [
        (a[0] + b[0]) * 0.5,
        (a[1] + b[1]) * 0.5,
        (a[2] + b[2]) * 0.5,
    ];
    Some(NormPlane {
        n,
        d: n[0] * mid[0] + n[1] * mid[1] + n[2] * mid[2],
    })
}

/// Buffers one clip loop reuses across planes, so a cell's clips allocate
/// nothing once the first clip has grown them.
#[derive(Default)]
pub(crate) struct ClipScratch {
    poly: Vec<[f32; 3]>,
    out_pos: Vec<[f32; 3]>,
    out_idx: Vec<u32>,
    dists: Vec<f32>,
}

/// Keeps the part of a shell on the back side of a plane, in place. One fused
/// vertex scan records every plane distance while tracking the near/far span:
/// a plane that misses the shell (every vertex on one side, within the
/// on-plane slack) costs that single pass and no split, so the shell stays
/// whole when it lies behind and goes empty when it lies in front. Only a
/// crossing plane runs the BSP split, reusing the recorded distances with the
/// same corner order and coplanar rule as the plane-split kernel (a fully
/// coplanar triangle routes front, so it is dropped here). Returns whether
/// the shell changed.
pub(crate) fn clip_back_reused(
    mesh: &mut PropMesh,
    plane: &NormPlane,
    scratch: &mut ClipScratch,
) -> bool {
    let ClipScratch {
        poly,
        out_pos,
        out_idx,
        dists,
    } = scratch;
    dists.clear();
    dists.reserve(mesh.positions.len());
    let mut near = f32::INFINITY;
    let mut far = f32::NEG_INFINITY;
    for v in &mesh.positions {
        let d = plane.dist_to(v);
        dists.push(d);
        near = near.min(d);
        far = far.max(d);
    }
    if far <= COPLANAR_EPS {
        return false;
    }
    if near >= -COPLANAR_EPS {
        mesh.positions.clear();
        mesh.indices.clear();
        return true;
    }
    out_pos.clear();
    out_idx.clear();
    for tri in mesh.indices.chunks_exact(3) {
        // Indices are validated below `MAX_CSG_VERTS`, so the fallback never
        // fires; it only names the proven invariant.
        let [ia, ib, ic] =
            [tri[0], tri[1], tri[2]].map(|i| usize::try_from(i).unwrap_or(usize::MAX));
        let ds = [dists[ia], dists[ib], dists[ic]];
        if ds.iter().all(|d| d.abs() <= COPLANAR_EPS) {
            continue;
        }
        let corners = [mesh.positions[ia], mesh.positions[ib], mesh.positions[ic]];
        clip_emit(corners, ds, false, poly, out_pos, out_idx);
    }
    core::mem::swap(&mut mesh.positions, out_pos);
    core::mem::swap(&mut mesh.indices, out_idx);
    true
}

pub(crate) fn mesh_aabb(mesh: &PropMesh) -> ([f32; 3], [f32; 3]) {
    // Shells always carry vertices, so the box below is resolved, never empty.
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in &mesh.positions {
        for k in 0..3 {
            if v[k] < min[k] {
                min[k] = v[k];
            }
            if v[k] > max[k] {
                max[k] = v[k];
            }
        }
    }
    (min, max)
}

/// Bake: cut the prop mesh with each plane in order, BSP style. Every cut must
/// split at least one live cell, or the bake refuses with `DegenerateCut`.
/// Output order is deterministic: front then back per cell, cells in order.
pub fn bake(mesh: &PropMesh, cuts: &[CutPlane]) -> Result<Vec<FractureChunk>, CsgError> {
    validate_mesh(mesh)?;
    let planes = normalize_all(cuts)?;
    let mut cells = vec![mesh.clone()];
    for (i, plane) in planes.iter().enumerate() {
        let mut next = Vec::new();
        let mut split_any = false;
        for cell in &cells {
            let (front, back) = split_mesh(cell, plane);
            if front.indices.is_empty() || back.indices.is_empty() {
                next.push(cell.clone());
            } else {
                next.push(front);
                next.push(back);
                split_any = true;
            }
        }
        if !split_any {
            return Err(CsgError::DegenerateCut(i));
        }
        if next.len() > MAX_CSG_CHUNKS {
            return Err(CsgError::TooManyChunks(next.len()));
        }
        let total: usize = next.iter().map(|c| c.indices.len() / 3).sum();
        if total > MAX_BAKE_TRIS {
            return Err(CsgError::TooLarge { count: total });
        }
        cells = next;
    }
    let mut chunks = Vec::with_capacity(cells.len());
    for (i, cell) in cells.into_iter().enumerate() {
        let (min, max) = mesh_aabb(&cell);
        chunks.push(FractureChunk {
            name: frac_name(i),
            mesh: cell,
            min,
            max,
        });
    }
    Ok(chunks)
}

/// Length word of the bake encoder. A `Vec` length is a `usize`, which always fits `u64` on the
/// targets we ship, so the checked form never takes its fallback; it is written out loud rather
/// than truncated with a cast.
fn len_word(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Canonical bake bytes: chunk count plus, per chunk, name plus bit-exact
/// vertices and indices, all in bake order.
pub fn csg_bake_bytes(chunks: &[FractureChunk]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&len_word(chunks.len()).to_le_bytes());
    for chunk in chunks {
        out.extend_from_slice(&len_word(chunk.name.len()).to_le_bytes());
        out.extend_from_slice(chunk.name.as_bytes());
        out.extend_from_slice(&len_word(chunk.mesh.positions.len()).to_le_bytes());
        for v in &chunk.mesh.positions {
            for f in v {
                out.extend_from_slice(&f.to_bits().to_le_bytes());
            }
        }
        out.extend_from_slice(&len_word(chunk.mesh.indices.len()).to_le_bytes());
        for index in &chunk.mesh.indices {
            out.extend_from_slice(&index.to_le_bytes());
        }
    }
    out
}

/// Bake digest over [`csg_bake_bytes`]; identical inputs digest identical twice.
pub fn csg_bake_digest(chunks: &[FractureChunk]) -> u64 {
    engine_core::hash_bytes(&csg_bake_bytes(chunks))
}

/// Feed baked chunks to a bonded collection: AABB boxes with contact bonds at
/// `breaking_strain`, the bottom layer anchored, validated before return so a
/// bad strain or threshold refuses here instead of in the match.
pub fn to_bonded_desc(
    chunks: &[FractureChunk],
    breaking_strain: f32,
    break_threshold: f32,
) -> Result<BondedDesc, BreakError> {
    let boxes: Vec<Chunk3> = chunks.iter().map(|c| c.to_chunk3()).collect();
    // `validate` below refuses past MAX_BONDED_CHUNKS, so an id always fits u32; the checked
    // form routes an impossible overflow to that same refusal instead of wrapping.
    let id = |i: usize| u32::try_from(i).map_err(|_| BreakError::TooManyChunks);
    let mut bonds = Vec::new();
    for a in 0..boxes.len() {
        for b in (a + 1)..boxes.len() {
            if in_contact(&boxes[a], &boxes[b]) {
                bonds.push(Bond {
                    a: id(a)?,
                    b: id(b)?,
                    breaking_strain,
                });
            }
        }
    }
    let bottom = boxes
        .iter()
        .map(|c| c.y - c.hy)
        .reduce(f32::min)
        .unwrap_or(0.0);
    let mut anchors = Vec::new();
    for (i, c) in boxes.iter().enumerate() {
        if (c.y - c.hy) - bottom <= CSG_ANCHOR_EPS_M {
            anchors.push(id(i)?);
        }
    }
    let desc = BondedDesc {
        chunks: boxes,
        bonds,
        anchors,
        break_threshold,
    };
    desc.validate()?;
    Ok(desc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;
    use crate::bonded::{BondedCollection, BreakEvent};

    #[test]
    fn a_flat_sliver_patch_still_validates_as_a_chunk() {
        let patch = FractureChunk {
            name: frac_name(0),
            mesh: box_mesh(),
            min: [0.0, 0.0, 0.0],
            max: [0.01, 0.01, 0.0],
        };
        let desc = to_bonded_desc(&[patch], 1.0, BREAKING_THRESHOLD).expect("flat patch validates");
        assert!(desc.chunks[0].mass >= MIN_MASS);
    }

    /// Unit cube shell centered on the origin, edge 1, 12 triangles, meters.
    fn box_mesh() -> PropMesh {
        PropMesh {
            positions: vec![
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
        }
    }

    /// Blade through the cube center: blade along +x, swing along +z, so the
    /// normal is -y and the cut is the y = 0 plane.
    fn center_blade() -> CutPlane {
        blade_plane(&[0.0, 0.0, 0.0], &[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0]).expect("valid blade")
    }

    /// Import naming rules live in assets; the bake only emits names this helper
    /// accepts (prefix, charset, lengths), keeping the convention compatible.
    fn assert_frac_compatible(name: &str) {
        let piece = name.strip_prefix("FRAC_").expect("FRAC_ prefix");
        assert!(
            !piece.is_empty() && piece.len() <= 64,
            "piece bounds: {name}"
        );
        assert!(
            piece
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "piece charset: {name}"
        );
        assert!(name.len() <= 128, "name bounds: {name}");
    }

    #[test]
    fn frac_name_into_never_silently_truncates() {
        // The ignored `write!` result once let a truncated `FRAC_` name slip
        // through silently; the helper now refuses loudly on format failure,
        // and this pins exact bytes across the width boundary.
        for index in [
            0usize,
            1,
            9,
            99,
            999,
            1000,
            9999,
            10_000,
            123_456,
            usize::MAX,
        ] {
            let mut buffered = String::with_capacity(9);
            frac_name_into(index, &mut buffered);
            assert_eq!(
                buffered,
                frac_name(index),
                "buffered name matches owned for {index}"
            );
            assert_eq!(
                buffered,
                format!("FRAC_{index:04}"),
                "exact zero-padded bytes for {index}"
            );
            assert_frac_compatible(&buffered);
        }
        // Reuse keeps no stale prefix: a second call overwrites the first.
        let mut reused = String::with_capacity(9);
        frac_name_into(7, &mut reused);
        assert_eq!(&reused, "FRAC_0007");
        frac_name_into(12345, &mut reused);
        assert_eq!(&reused, "FRAC_12345");
    }

    #[test]
    fn casts_keep_the_csg_bake_digest() {
        // Pinned from the code before the CQ10 cast pass on `csg.rs`: the canonical bake bytes
        // (length words, vertices, indices) must not move.
        let chunks = bake(&box_mesh(), &[center_blade()]).expect("single cut bakes");
        assert_eq!(csg_bake_digest(&chunks), 17103417725578125922u64);
        // The chunk-count length word decodes back to the live count: the encoder writes the
        // same bytes a bare `as u64` did.
        let bytes = csg_bake_bytes(&chunks);
        let head = u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        assert_eq!(
            head,
            u64::try_from(chunks.len()).expect("chunk count fits"),
            "chunk-count length word"
        );
    }

    #[test]
    fn blade_plane_spans_blade_and_swing() {
        let plane = center_blade();
        assert_eq!(plane.normal[0].to_bits(), 0.0f32.to_bits());
        assert_eq!(plane.normal[1].to_bits(), (-1.0f32).to_bits());
        assert_eq!(plane.normal[2].to_bits(), 0.0f32.to_bits());
        assert_eq!(plane.dist.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn clip_back_reused_matches_the_split_kernel_across_buffer_reuse() {
        let cube = box_mesh();
        let cut = bake(&cube, &[center_blade()]).expect("blade cuts")[0]
            .mesh
            .clone();
        let crossing = [
            plane_between(&[-1.0, 0.0, 0.0], &[1.0, 0.0, 0.0]).expect("bisector"),
            plane_between(&[0.1, 0.2, 0.3], &[0.4, -0.5, 0.2]).expect("bisector"),
        ];
        // Bisectors x = 10.5 and x = -10.5: the shell lies wholly behind the
        // first and wholly in front of the second.
        let behind = plane_between(&[10.0, 0.0, 0.0], &[11.0, 0.0, 0.0]).expect("bisector");
        let front = plane_between(&[-11.0, 0.0, 0.0], &[-10.0, 0.0, 0.0]).expect("bisector");
        let mut scratch = ClipScratch::default();
        for source in [&cube, &cut] {
            for _round in 0..2 {
                for plane in &crossing {
                    let mut clipped = source.clone();
                    assert!(clip_back_reused(&mut clipped, plane, &mut scratch));
                    let (_, back) = split_mesh(source, plane);
                    assert_eq!(clipped, back, "crossing clip is the kernel's back half");
                    assert_ne!(&clipped, source, "the cut removed the front part");
                }
                let mut whole = source.clone();
                assert!(!clip_back_reused(&mut whole, &behind, &mut scratch));
                assert_eq!(&whole, source, "a miss keeps the shell whole");
                let mut gone = source.clone();
                assert!(clip_back_reused(&mut gone, &front, &mut scratch));
                assert!(gone.indices.is_empty(), "a front shell goes empty");
            }
        }
    }

    // TMPLEGACY start
    fn legacy_split_back(mesh: &PropMesh, plane: &NormPlane) -> PropMesh {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let mut scratch: Vec<[f32; 3]> = Vec::with_capacity(8);
        for tri in mesh.indices.chunks_exact(3) {
            let corners = [
                mesh.positions[tri[0] as usize],
                mesh.positions[tri[1] as usize],
                mesh.positions[tri[2] as usize],
            ];
            let ds = [
                plane.dist_to(&corners[0]),
                plane.dist_to(&corners[1]),
                plane.dist_to(&corners[2]),
            ];
            if ds.iter().all(|d| d.abs() <= COPLANAR_EPS) {
                continue;
            }
            clip_emit(
                corners,
                ds,
                false,
                &mut scratch,
                &mut positions,
                &mut indices,
            );
        }
        PropMesh { positions, indices }
    }

    fn legacy_clip_back(mesh: &mut PropMesh, plane: &NormPlane) {
        let mut near = f32::INFINITY;
        let mut far = f32::NEG_INFINITY;
        for v in &mesh.positions {
            let d = plane.dist_to(v);
            near = near.min(d);
            far = far.max(d);
        }
        if far <= COPLANAR_EPS {
            return;
        }
        if near >= -COPLANAR_EPS {
            mesh.positions.clear();
            mesh.indices.clear();
            return;
        }
        *mesh = legacy_split_back(mesh, plane);
    }

    fn l_dist2(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
    }

    fn legacy_cells(mesh: &PropMesh, sites: &[[f32; 3]]) -> Vec<PropMesh> {
        let mut out = Vec::new();
        for (i, site) in sites.iter().enumerate() {
            let mut order: Vec<usize> = (0..sites.len()).filter(|j| *j != i).collect();
            order.sort_by(|a, b| {
                l_dist2(site, &sites[*a])
                    .total_cmp(&l_dist2(site, &sites[*b]))
                    .then(a.cmp(b))
            });
            let mut cell = mesh.clone();
            for j in order {
                let Some(plane) = plane_between(site, &sites[j]) else {
                    if j < i {
                        cell.indices.clear();
                        break;
                    }
                    continue;
                };
                legacy_clip_back(&mut cell, &plane);
                if cell.indices.is_empty() {
                    break;
                }
            }
            if !cell.indices.is_empty() {
                out.push(cell);
            }
        }
        out
    }

    #[test]
    fn tmp_legacy_fracture_matches_new() {
        use crate::fracture::{fracture_counted, FractureSpec};
        let base = box_mesh();
        let (mut checked, mut clips, mut skipped) = (0usize, 0usize, 0usize);
        for (sx, sy, sz, off) in [
            (4.0f32, 3.0f32, 0.4f32, 0.0f32),
            (48.0, 3.0, 0.4, 0.0),
            (48.0, 3.0, 0.4, 6000.0),
            (1.0, 1.0, 1.0, 0.0),
            (200.0, 1.0, 1.0, 0.0),
        ] {
            let mut mesh = base.clone();
            for p in &mut mesh.positions {
                p[0] = p[0] * sx + off;
                p[1] = p[1] * sy + sy * 0.5;
                p[2] *= sz;
            }
            let (min, max) = mesh_aabb(&mesh);
            for cells in [2u32, 3, 5, 8, 16, 24, 32, 48, 64] {
                for seed in 0..12u64 {
                    for frac in [0.05f32, 0.5, 0.95] {
                        let hit = [
                            min[0] + (max[0] - min[0]) * frac,
                            min[1] + (max[1] - min[1]) * frac,
                            min[2] + (max[2] - min[2]) * frac,
                        ];
                        let spec = FractureSpec {
                            cells,
                            radius_m: 0.8,
                            seed,
                        };
                        let Ok((f, counts)) = fracture_counted(&mesh, hit, &spec) else {
                            continue;
                        };
                        clips += counts.clips;
                        skipped += counts.skipped;
                        if f.chunks.len() != usize::try_from(cells).unwrap() {
                            continue;
                        }
                        let old = legacy_cells(&mesh, &f.sites);
                        let new: Vec<&PropMesh> = f.chunks.iter().map(|c| &c.mesh).collect();
                        assert_eq!(
                            new,
                            old.iter().collect::<Vec<_>>(),
                            "{sx} {cells} {seed} {frac}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        println!("TMP legacy-vs-new checked={checked} clips={clips} skipped={skipped}");
    }
    // TMPLEGACY end

    #[test]
    fn blade_cut_splits_cube_identical_twice() {
        let mesh = box_mesh();
        let plane = center_blade();
        let a = bake(&mesh, &[plane]).expect("blade cuts the cube");
        let b = bake(&mesh, &[plane]).expect("blade cuts the cube");
        assert_eq!(a.len(), 2, "one plane makes two chunks: {a:?}");
        assert_eq!(a, b, "identical chunk sets twice");
        assert_eq!(csg_bake_bytes(&a), csg_bake_bytes(&b));
        assert_eq!(csg_bake_digest(&a), csg_bake_digest(&b));
        // Cut triangles fan into more tris, but the growth stays bounded.
        let tris: usize = a.iter().map(|c| c.tri_count()).sum();
        assert!((12..=28).contains(&tris), "split tri budget: {tris}");
        // Front (y <= 0) then back (y >= 0): the boxes tile without overlap.
        assert!(a[0].max[1] <= 1e-6, "front box ends at the cut");
        assert!(a[1].min[1] >= -1e-6, "back box starts at the cut");
        assert!(
            a[0].min[1] < -0.25 && a[1].max[1] > 0.25,
            "boxes keep the faces"
        );
        assert_eq!(a[0].name, "FRAC_0000");
        assert_eq!(a[1].name, "FRAC_0001");
        for chunk in &a {
            assert_frac_compatible(&chunk.name);
        }
    }

    #[test]
    fn two_perpendicular_cuts_make_four_chunks() {
        let mesh = box_mesh();
        let cuts = [
            CutPlane {
                normal: [1.0, 0.0, 0.0],
                dist: 0.0,
            },
            CutPlane {
                normal: [0.0, 0.0, 1.0],
                dist: 0.0,
            },
        ];
        let a = bake(&mesh, &cuts).expect("two cuts quarter the cube");
        let b = bake(&mesh, &cuts).expect("two cuts quarter the cube");
        assert_eq!(a.len(), 4);
        assert_eq!(a, b);
        assert_eq!(csg_bake_digest(&a), csg_bake_digest(&b));
        let mut names: Vec<&str> = a.iter().map(|c| c.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 4, "chunk names stay unique");
        for chunk in &a {
            assert!(chunk.tri_count() > 0);
            assert_frac_compatible(&chunk.name);
        }
    }

    #[test]
    fn boolean_patterns_agree_on_the_cube() {
        let mesh = box_mesh();
        let plane = CutPlane {
            normal: [1.0, 0.0, 0.0],
            dist: 0.0,
        };
        let (front, back) = knife(&mesh, &plane).expect("knife splits the cube");
        // Every kept corner sits on its own side of the cut.
        for v in &front.positions {
            assert!(v[0] >= -1e-6, "front corner {v:?}");
        }
        for v in &back.positions {
            assert!(v[0] <= 1e-6, "back corner {v:?}");
        }
        // Subtract/intersect against the x <= 0 half-space match the knife.
        let tool = [CutPlane {
            normal: [1.0, 0.0, 0.0],
            dist: 0.0,
        }];
        assert_eq!(
            brush_subtract(&mesh, &tool).expect("outside survives"),
            front,
            "subtract keeps the knife front"
        );
        assert_eq!(
            brush_intersect(&mesh, &tool).expect("inside survives"),
            back,
            "intersect keeps the knife back"
        );
        // Union concatenates disjoint shells in argument order.
        let joined = shell_union(&front, &back).expect("disjoint union");
        assert_eq!(joined.tri_count(), front.tri_count() + back.tri_count());
        assert_eq!(
            joined,
            shell_union(&front, &back).expect("union deterministic")
        );
        // A brush swallowing the cube removes everything, loudly.
        let room = [
            CutPlane {
                normal: [1.0, 0.0, 0.0],
                dist: 1.0,
            },
            CutPlane {
                normal: [-1.0, 0.0, 0.0],
                dist: 1.0,
            },
            CutPlane {
                normal: [0.0, 1.0, 0.0],
                dist: 1.0,
            },
            CutPlane {
                normal: [0.0, -1.0, 0.0],
                dist: 1.0,
            },
            CutPlane {
                normal: [0.0, 0.0, 1.0],
                dist: 1.0,
            },
            CutPlane {
                normal: [0.0, 0.0, -1.0],
                dist: 1.0,
            },
        ];
        assert_eq!(
            brush_subtract(&mesh, &room).unwrap_err(),
            CsgError::EmptyResult
        );
        assert_eq!(
            brush_intersect(&mesh, &room)
                .expect("room keeps all")
                .tri_count(),
            12
        );
    }

    #[test]
    fn sword_hit_breaks_baked_chunks_through_dx1() {
        fn sword_script() -> (u64, Vec<BreakEvent>, u64) {
            let chunks = bake(&box_mesh(), &[center_blade()]).expect("blade cuts");
            let digest = csg_bake_digest(&chunks);
            let desc =
                to_bonded_desc(&chunks, 1.0, BREAKING_THRESHOLD).expect("baked chunks convert");
            let mut field = BondedCollection::new(&desc).expect("valid collection");
            // Highest box wins; strict `>` keeps the lowest index on ties.
            let mut top = 0;
            for (i, c) in field.chunks().iter().enumerate() {
                if c.y > field.chunks()[top].y {
                    top = i;
                }
            }
            let impulse =
                field.chunk_mass(top).expect("top chunk mass") * field.break_threshold() + 1.0;
            field.apply_impact(top, impulse, 3).expect("valid impact");
            for _ in 0..30 {
                field.step(1.0 / 60.0);
            }
            (field.state_hash(), field.drain_events(), digest)
        }
        let (h1, e1, d1) = sword_script();
        let (h2, e2, d2) = sword_script();
        assert_eq!(d1, d2, "bake digest identical twice");
        assert_eq!(h1, h2, "DX1 state hash identical twice");
        assert_eq!(e1, e2, "break events identical twice");
        assert!(
            e1.iter()
                .any(|e| matches!(e, BreakEvent::BondBroken { .. })),
            "sword strain snaps a bond: {e1:?}"
        );
        assert!(
            e1.iter()
                .any(|e| matches!(e, BreakEvent::ChunkFreed { .. })),
            "detached chunk frees and falls: {e1:?}"
        );
    }

    #[test]
    fn degenerate_inputs_refused() {
        let mesh = box_mesh();
        assert_eq!(
            bake(
                &PropMesh {
                    positions: Vec::new(),
                    indices: Vec::new(),
                },
                &[center_blade()]
            )
            .unwrap_err(),
            CsgError::EmptyMesh
        );
        assert_eq!(
            bake(
                &PropMesh {
                    positions: vec![[f32::NAN, 0.0, 0.0]],
                    indices: vec![0, 0, 0],
                },
                &[center_blade()]
            )
            .unwrap_err(),
            CsgError::BadVertex(0)
        );
        assert_eq!(
            bake(
                &PropMesh {
                    positions: vec![[MAX_COORD_M + 1.0, 0.0, 0.0]],
                    indices: vec![0, 0, 0],
                },
                &[center_blade()]
            )
            .unwrap_err(),
            CsgError::BadVertex(0)
        );
        assert_eq!(
            bake(
                &PropMesh {
                    positions: vec![[0.0; 3]],
                    indices: vec![0, 0, 7],
                },
                &[center_blade()]
            )
            .unwrap_err(),
            CsgError::BadIndex(7)
        );
        assert_eq!(
            bake(
                &PropMesh {
                    positions: vec![[0.0; 3]],
                    indices: vec![0, 0],
                },
                &[center_blade()]
            )
            .unwrap_err(),
            CsgError::BadTopology
        );
        assert_eq!(
            bake(
                &mesh,
                &[CutPlane {
                    normal: [0.0, 0.0, 0.0],
                    dist: 0.0,
                }]
            )
            .unwrap_err(),
            CsgError::DegeneratePlane(0)
        );
        assert_eq!(
            bake(
                &mesh,
                &[CutPlane {
                    normal: [1.0, 0.0, 0.0],
                    dist: f32::NAN,
                }]
            )
            .unwrap_err(),
            CsgError::DegeneratePlane(0)
        );
        // A plane that misses the cube entirely is a degenerate cut.
        assert_eq!(
            bake(
                &mesh,
                &[CutPlane {
                    normal: [0.0, 1.0, 0.0],
                    dist: 50.0,
                }]
            )
            .unwrap_err(),
            CsgError::DegenerateCut(0)
        );
        assert_eq!(bake(&mesh, &[]).unwrap_err(), CsgError::NoCuts);
        assert_eq!(
            bake(&mesh, &vec![center_blade(); MAX_CUT_PLANES + 1]).unwrap_err(),
            CsgError::TooManyPlanes(MAX_CUT_PLANES + 1)
        );
        // Parallel blade and swing span no plane; NaN input is no blade.
        assert_eq!(
            blade_plane(&[0.0; 3], &[1.0, 0.0, 0.0], &[2.0, 0.0, 0.0]).unwrap_err(),
            CsgError::BadBlade
        );
        assert_eq!(
            blade_plane(&[0.0; 3], &[f32::NAN, 0.0, 0.0], &[0.0, 0.0, 1.0]).unwrap_err(),
            CsgError::BadBlade
        );
        // Oversized shells refuse before any split runs.
        let big = PropMesh {
            positions: vec![[0.0; 3]; MAX_CSG_VERTS + 1],
            indices: vec![0, 1, 2],
        };
        assert_eq!(
            bake(&big, &[center_blade()]).unwrap_err(),
            CsgError::TooLarge {
                count: MAX_CSG_VERTS + 1
            }
        );
        // A bad bonded-collection strain refuses at conversion, not in the match.
        let chunks = bake(&mesh, &[center_blade()]).expect("blade cuts");
        assert!(to_bonded_desc(&chunks, f32::NAN, BREAKING_THRESHOLD).is_err());
    }

    #[test]
    fn grid_cuts_overflow_chunk_budget() {
        // Twenty-one slabs per axis multiply toward 22^3 cells, past the
        // chunk bound while every cut still splits a live cell.
        let mut cuts = Vec::new();
        for axis in 0..3 {
            for k in 0..21 {
                let offset = -0.475 + k as f32 * 0.045;
                let mut normal = [0.0f32; 3];
                normal[axis] = 1.0;
                cuts.push(CutPlane {
                    normal,
                    dist: offset,
                });
            }
        }
        match bake(&box_mesh(), &cuts).unwrap_err() {
            CsgError::TooManyChunks(n) => assert!(n > MAX_CSG_CHUNKS, "live count {n}"),
            other => panic!("expected TooManyChunks, got {other:?}"),
        }
    }
}
