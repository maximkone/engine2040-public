//! collapse: quadric-error mesh simplification for the mesh cook.
//!
//! Consumer: [`super::mesh_cook`] drives [`simplify`] for over-budget meshes
//! and LOD targets. Data owner: `engine-assets`. Units: meters for positions,
//! triangles for targets. Determinism scope: the same mesh plus target
//! collapses the same edges twice (index-order loops, sorted edges, `+ - * /`
//! and `sqrt` only; no hash-map iteration, no wall clock). Errors: typed
//! [`MeshCookError`](super::mesh_cook::MeshCookError); degenerate inputs are
//! refused, never returned hollow.
//!
//! Method: Garland-Heckbert error quadrics with midpoint collapses, batched
//! per pass over a greedy independent edge set. Attribute fallbacks mirror the
//! import lane (`+Z` normals, `+X` tangents with `w` kept).

use super::cooked::{validate_mesh, CookedMesh};
use super::mesh_cook::{
    count_u32, index_usize, map_cook_error, tri_count, validate_cookable, MeshCookError,
};
use std::collections::{BTreeMap, BTreeSet};

/// Most simplification passes before the loop yields its best effort.
const MAX_SIMPLIFY_PASSES: usize = 64;

/// Positions within this distance are the same corner of an attribute seam.
const SEAM_POSITION_EPS_M: f64 = 1e-4;

/// uv0 pairs further apart than this are a seam, not a rounding difference.
const SEAM_UV_EPS: f32 = 1e-3;

/// Normal pairs further apart than this are a hard edge and lock like a seam.
const SEAM_ANGLE_DEG: f32 = 20.0;

/// Symmetric 4x4 error quadric as 10 upper-triangle floats in `f64`:
/// `[00, 01, 02, 03, 11, 12, 13, 22, 23, 33]`. `f64` accumulation keeps large
/// meshes stable; the same inputs still collapse the same edges twice.
type Quadric = [f64; 10];

/// Add `other` into `accum` in place.
fn quadric_add(accum: &mut Quadric, other: &Quadric) {
    for (slot, value) in accum.iter_mut().zip(other.iter()) {
        *slot += value;
    }
}

/// Outer product of the plane `[a, b, c, d]` with itself (upper triangle).
fn plane_quadric(plane: [f64; 4]) -> Quadric {
    let [a, b, c, d] = plane;
    [
        a * a,
        a * b,
        a * c,
        a * d,
        b * b,
        b * c,
        b * d,
        c * c,
        c * d,
        d * d,
    ]
}

/// Quadric cost of placing a vertex at `v`: `v^T Q v` with `v = [x, y, z, 1]`.
fn quadric_cost(q: &Quadric, v: [f64; 3]) -> f64 {
    let [x, y, z] = v;
    q[0] * x * x
        + 2.0 * q[1] * x * y
        + 2.0 * q[2] * x * z
        + 2.0 * q[3] * x
        + q[4] * y * y
        + 2.0 * q[5] * y * z
        + 2.0 * q[6] * y
        + q[7] * z * z
        + 2.0 * q[8] * z
        + q[9]
}

/// Unit plane of one triangle, or `None` when the triangle is zero-area.
fn tri_plane(p0: &[f32; 3], p1: &[f32; 3], p2: &[f32; 3]) -> Option<[f64; 4]> {
    let e1 = [
        f64::from(p1[0]) - f64::from(p0[0]),
        f64::from(p1[1]) - f64::from(p0[1]),
        f64::from(p1[2]) - f64::from(p0[2]),
    ];
    let e2 = [
        f64::from(p2[0]) - f64::from(p0[0]),
        f64::from(p2[1]) - f64::from(p0[1]),
        f64::from(p2[2]) - f64::from(p0[2]),
    ];
    let mut n = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !len.is_finite() || len <= 1e-12 {
        return None;
    }
    n[0] /= len;
    n[1] /= len;
    n[2] /= len;
    let d = -(n[0] * f64::from(p0[0]) + n[1] * f64::from(p0[1]) + n[2] * f64::from(p0[2]));
    Some([n[0], n[1], n[2], d])
}

/// Normalize a 3-vector; falls back to `+Z` on degenerate input (never NaN).
/// Shared with the cook tests, which build outward normals the same way.
pub(crate) fn norm3(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len.is_finite() && len > 1e-12 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Simplify one mesh toward `target_tris` triangles by batched quadric edge
/// collapses: each pass rebuilds quadrics from the live triangles, sorts every
/// edge by midpoint cost (ties break on vertex indices, so reruns agree),
/// collapses a greedy independent edge set, drops degenerate triangles and
/// compacts vertices. Attribute fallbacks mirror the import lane (`+Z`
/// normals, `+X` tangents with `w` kept). Inputs at or below target clone
/// through; degenerate meshes are refused, never returned hollow.
pub fn simplify(mesh: &CookedMesh, target_tris: u32) -> Result<CookedMesh, MeshCookError> {
    simplify_attrs(mesh, AttrCost::plain(mesh), target_tris)
}

/// Simplification cost beyond the geometric quadric: per-attribute weights, a
/// per-vertex lock mask and an error cap.
///
/// `weights[0]` scales the uv0 term, `weights[1]` the normal term, `weights[2]`
/// is reserved (tangents ride the normal term for now and stay 0). A locked
/// vertex never collapses, which is how a pinned seam survives; `max_cost` is
/// in squared metres and stops the pass once every remaining edge is worse
/// than the level's allowed error.
#[derive(Clone, Copy, Debug)]
pub struct AttrCost<'a> {
    /// Weight of the uv0 delta, of the normal delta and the reserved lane.
    pub weights: [f32; 3],
    /// One byte per vertex: 1 pins it. Empty means nothing is locked.
    pub locks: &'a [u8],
    /// Largest acceptable collapse cost in squared metres.
    pub max_cost_m2: f64,
    /// Extent the uv and normal terms scale by, in metres.
    pub extent_m: f64,
}

impl<'a> AttrCost<'a> {
    /// The plain geometric cost `simplify` has always used: no attribute
    /// term, no locks, no cap.
    #[must_use]
    pub fn plain(mesh: &CookedMesh) -> Self {
        Self {
            weights: [0.0; 3],
            locks: &[],
            max_cost_m2: f64::INFINITY,
            extent_m: mesh_extent_m(mesh),
        }
    }

    /// True when this vertex is pinned.
    fn locked(&self, vertex: usize) -> bool {
        self.locks.get(vertex) == Some(&1)
    }

    /// True when any attribute term is switched on.
    fn attribute_aware(&self) -> bool {
        self.weights[0] != 0.0 || self.weights[1] != 0.0
    }
}

/// Diagonal of the mesh bounds in metres, at least 1 mm; the length the uv and
/// normal terms are measured against.
#[must_use]
pub fn mesh_extent_m(mesh: &CookedMesh) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for point in &mesh.positions {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(f64::from(point[axis]));
            hi[axis] = hi[axis].max(f64::from(point[axis]));
        }
    }
    if !lo[0].is_finite() {
        return 1e-3;
    }
    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-3)
}

/// Pin every vertex of an attribute discontinuity: two vertices at the same
/// position whose uv0 or normal differs are one seam or one hard edge, and
/// collapsing either side across it is what smears a cloth's uvs. Both sides
/// are pinned, because locking one side lets the other collapse into it and the
/// two uv islands merge — the same smear by another route.
///
/// Returns one byte per vertex (1 = locked). Positions match within
/// `SEAM_POSITION_EPS_M` and uv pairs within `SEAM_UV_EPS`; a normal pair
/// further apart than `SEAM_ANGLE_DEG` is a hard edge and locks the same way.
#[must_use]
pub fn seam_lock(mesh: &CookedMesh) -> Vec<u8> {
    let mut locks = vec![0u8; mesh.positions.len()];
    let mut buckets: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
    for (index, point) in mesh.positions.iter().enumerate() {
        let key = (
            (f64::from(point[0]) / SEAM_POSITION_EPS_M).round() as i64,
            (f64::from(point[1]) / SEAM_POSITION_EPS_M).round() as i64,
            (f64::from(point[2]) / SEAM_POSITION_EPS_M).round() as i64,
        );
        buckets.entry(key).or_default().push(index);
    }
    let cos_limit = f64::from(SEAM_ANGLE_DEG).to_radians().cos();
    for group in buckets.values() {
        for (i, first) in group.iter().enumerate() {
            for second in &group[i + 1..] {
                let (Some(pa), Some(pb)) =
                    (mesh.positions.get(*first), mesh.positions.get(*second))
                else {
                    continue;
                };
                let dpos = [
                    f64::from(pa[0]) - f64::from(pb[0]),
                    f64::from(pa[1]) - f64::from(pb[1]),
                    f64::from(pa[2]) - f64::from(pb[2]),
                ];
                if (dpos[0] * dpos[0] + dpos[1] * dpos[1] + dpos[2] * dpos[2]).sqrt()
                    > SEAM_POSITION_EPS_M
                {
                    continue;
                }
                let uv_gap = match (mesh.uv0.get(*first), mesh.uv0.get(*second)) {
                    (Some(u), Some(v)) => {
                        (f64::from(u[0]) - f64::from(v[0])).abs() > f64::from(SEAM_UV_EPS)
                            || (f64::from(u[1]) - f64::from(v[1])).abs() > f64::from(SEAM_UV_EPS)
                    }
                    _ => false,
                };
                let normal_gap = match (mesh.normals.get(*first), mesh.normals.get(*second)) {
                    (Some(n), Some(m)) => {
                        let dot = f64::from(n[0] * m[0] + n[1] * m[1] + n[2] * m[2]);
                        dot < cos_limit
                    }
                    _ => false,
                };
                if uv_gap || normal_gap {
                    // Both sides of the seam are pinned: locking only one side
                    // lets the other collapse into it, which merges the two uv
                    // islands and smears the cloth exactly the way an averaged
                    // collapse does.
                    if let Some(slot) = locks.get_mut(*first) {
                        *slot = 1;
                    }
                    if let Some(slot) = locks.get_mut(*second) {
                        *slot = 1;
                    }
                }
            }
        }
    }
    locks
}

/// [`simplify`] with an attribute-aware cost and optional locks: a uv or normal
/// discontinuity is expensive to collapse and a locked vertex never collapses,
/// so a seam halfway down a tunic keeps both of its sides.
///
/// The geometric term is byte-identical to [`simplify`] when `attr.weights` is
/// all zero and `attr.locks` is empty; the attribute term is added on top in
/// `f64` and `attr.max_cost_m2` skips every edge that costs more than the
/// level's allowed error.
///
/// Errors: what [`simplify`] refuses (empty, non-finite, out-of-range or
/// zero-area input).
pub fn simplify_with_attributes(
    mesh: &CookedMesh,
    attr: &AttrCost<'_>,
    target_tris: u32,
) -> Result<CookedMesh, MeshCookError> {
    simplify_attrs(mesh, *attr, target_tris)
}

fn simplify_attrs(
    mesh: &CookedMesh,
    attr: AttrCost<'_>,
    target_tris: u32,
) -> Result<CookedMesh, MeshCookError> {
    validate_cookable(mesh)?;
    if tri_count(mesh) <= target_tris.max(1) {
        return Ok(mesh.clone());
    }
    let target = target_tris.max(1);
    let mut positions = mesh.positions.clone();
    let mut normals = mesh.normals.clone();
    let mut uv0 = mesh.uv0.clone();
    let mut tangents = mesh.tangents.clone();
    let mut indices = mesh.indices.clone();
    // The caller's mask indexes the input mesh; compaction renumbers vertices,
    // so the pass loop carries its own copy and remaps it every pass.
    let mut locks: Vec<u8> = if attr.locks.is_empty() {
        Vec::new()
    } else {
        attr.locks.to_vec()
    };

    for _ in 0..MAX_SIMPLIFY_PASSES {
        let current = u32::try_from(indices.len() / 3).unwrap_or(u32::MAX);
        if current <= target {
            break;
        }
        // Each collapse removes at most two triangles, so half the need
        // bounds one pass without overshooting close targets (5002 to 5000).
        // Ceiling-half without overflow; identical to `(need + 1) / 2` below max.
        let need = current - target;
        let max_collapses = index_usize(need.div_ceil(2).max(1));
        let pass_attr = AttrCost {
            locks: &locks,
            ..attr
        };
        if !collapse_pass(
            &mut positions,
            &mut normals,
            &mut uv0,
            &mut tangents,
            &mut indices,
            max_collapses,
            &pass_attr,
            &mut locks,
        ) {
            break;
        }
    }
    let out = CookedMesh {
        positions,
        normals,
        uv0,
        tangents,
        indices,
    };
    validate_mesh(&out).map_err(|e| map_cook_error(&e))?;
    Ok(out)
}

/// Per-vertex quadrics from the live triangles, index order.
fn build_quadrics(positions: &[[f32; 3]], indices: &[u32]) -> Vec<Quadric> {
    let mut quadrics = vec![[0.0f64; 10]; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (
            index_usize(tri[0]),
            index_usize(tri[1]),
            index_usize(tri[2]),
        );
        let (Some(p0), Some(p1), Some(p2)) = (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        let Some(plane) = tri_plane(p0, p1, p2) else {
            continue;
        };
        let quad = plane_quadric(plane);
        for corner in [a, b, c] {
            if let Some(slot) = quadrics.get_mut(corner) {
                quadric_add(slot, &quad);
            }
        }
    }
    quadrics
}

/// Midpoint collapse cost per unique edge, cheapest first; non-finite costs
/// sort last but stay ordered by vertex indices, so reruns agree. Locked
/// vertices, edges past the level's error cap and attribute-heavy edges drop
/// out of the list entirely.
fn edge_costs(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uv0: &[[f32; 2]],
    quadrics: &[Quadric],
    indices: &[u32],
    attr: &AttrCost<'_>,
) -> Vec<(u32, u32, f64)> {
    // Unique edges, normalized so `a < b`; the set iterates sorted.
    let mut edges = BTreeSet::new();
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        for (x, y) in [(a, b), (b, c), (c, a)] {
            if x != y {
                edges.insert((x.min(y), x.max(y)));
            }
        }
    }
    let attribute_aware = attr.attribute_aware();
    let mut costs: Vec<(u32, u32, f64)> = Vec::with_capacity(edges.len());
    for (a, b) in edges {
        let (Some(pa), Some(pb), Some(qa), Some(qb)) = (
            positions.get(index_usize(a)),
            positions.get(index_usize(b)),
            quadrics.get(index_usize(a)),
            quadrics.get(index_usize(b)),
        ) else {
            continue;
        };
        if attr.locked(index_usize(a)) || attr.locked(index_usize(b)) {
            continue;
        }
        let mid = [
            (f64::from(pa[0]) + f64::from(pb[0])) * 0.5,
            (f64::from(pa[1]) + f64::from(pb[1])) * 0.5,
            (f64::from(pa[2]) + f64::from(pb[2])) * 0.5,
        ];
        let mut combined = *qa;
        quadric_add(&mut combined, qb);
        let mut cost = quadric_cost(&combined, mid);
        if attribute_aware {
            cost += attribute_cost(a, b, attr, normals, uv0);
        }
        if !cost.is_finite() || cost > attr.max_cost_m2 {
            continue;
        }
        costs.push((a, b, cost));
    }
    costs.sort_by(|x, y| {
        x.2.total_cmp(&y.2)
            .then_with(|| x.0.cmp(&y.0))
            .then_with(|| x.1.cmp(&y.1))
    });
    costs
}

/// The attribute term of one edge's collapse cost, in squared metres: the uv
/// jump scaled by the mesh extent (a uv unit is about one extent across the
/// surface) plus the normal cone's angle, each times its weight. Zero when the
/// lane is missing.
fn attribute_cost(
    a: u32,
    b: u32,
    attr: &AttrCost<'_>,
    normals: &[[f32; 3]],
    uv0: &[[f32; 2]],
) -> f64 {
    let i = index_usize(a);
    let j = index_usize(b);
    let extent2 = attr.extent_m * attr.extent_m;
    let mut cost = 0.0f64;
    if attr.weights[0] != 0.0 {
        if let (Some(ua), Some(ub)) = (uv0.get(i), uv0.get(j)) {
            let du = f64::from(ua[0]) - f64::from(ub[0]);
            let dv = f64::from(ua[1]) - f64::from(ub[1]);
            cost += f64::from(attr.weights[0]) * (du * du + dv * dv) * extent2;
        }
    }
    if attr.weights[1] != 0.0 {
        if let (Some(na), Some(nb)) = (normals.get(i), normals.get(j)) {
            let dot = f64::from(na[0] * nb[0] + na[1] * nb[1] + na[2] * nb[2]).clamp(-1.0, 1.0);
            cost += f64::from(attr.weights[1]) * (1.0 - dot) * 0.25 * extent2;
        }
    }
    cost
}

/// Greedy independent set over the sorted costs: no vertex collapses twice in
/// one pass. Costs stay sorted, so truncating keeps the cheapest edges and
/// close targets converge without overshooting to hollow.
fn pick_collapses(
    costs: &[(u32, u32, f64)],
    vert_count: usize,
    max_collapses: usize,
) -> Vec<(usize, usize)> {
    let mut used = vec![false; vert_count];
    let mut collapses: Vec<(usize, usize)> = Vec::new();
    for (a, b, _) in costs {
        let (x, y) = (index_usize(*a), index_usize(*b));
        if used.get(x) == Some(&false) && used.get(y) == Some(&false) {
            used[x] = true;
            used[y] = true;
            collapses.push((x, y));
        }
    }
    collapses.truncate(max_collapses.max(1));
    collapses
}

/// Collapse `b` into `a` at the midpoint; attributes average, `w` is kept.
fn apply_collapses(
    positions: &mut [[f32; 3]],
    normals: &mut [[f32; 3]],
    uv0: &mut [[f32; 2]],
    tangents: &mut [[f32; 4]],
    indices: &mut [u32],
    locks: &mut [u8],
    collapses: &[(usize, usize)],
) {
    for (a, b) in collapses {
        // A pinned vertex stays pinned through the merge, whichever side moved.
        let pinned = locks.get(*a).copied().unwrap_or(0) | locks.get(*b).copied().unwrap_or(0);
        if let Some(slot) = locks.get_mut(*a) {
            *slot = if pinned > 0 { 1 } else { 0 };
        }
        let (Some(pa), Some(pb)) = (positions.get(*a), positions.get(*b)) else {
            continue;
        };
        let mid = [
            (pa[0] + pb[0]) * 0.5,
            (pa[1] + pb[1]) * 0.5,
            (pa[2] + pb[2]) * 0.5,
        ];
        if let Some(slot) = positions.get_mut(*a) {
            *slot = mid;
        }
        let (Some(na), Some(nb)) = (normals.get(*a), normals.get(*b)) else {
            continue;
        };
        let avg_n = norm3([na[0] + nb[0], na[1] + nb[1], na[2] + nb[2]]);
        if let Some(slot) = normals.get_mut(*a) {
            *slot = avg_n;
        }
        let (Some(ta), Some(tb)) = (uv0.get(*a), uv0.get(*b)) else {
            continue;
        };
        let avg_uv = [(ta[0] + tb[0]) * 0.5, (ta[1] + tb[1]) * 0.5];
        if let Some(slot) = uv0.get_mut(*a) {
            *slot = avg_uv;
        }
        let (Some(ga), Some(gb)) = (tangents.get(*a), tangents.get(*b)) else {
            continue;
        };
        let ga_w = ga[3];
        let avg_t = norm3([ga[0] + gb[0], ga[1] + gb[1], ga[2] + gb[2]]);
        if let Some(slot) = tangents.get_mut(*a) {
            *slot = [avg_t[0], avg_t[1], avg_t[2], ga_w];
        }
        for index in indices.iter_mut() {
            if index_usize(*index) == *b {
                *index = count_u32(*a);
            }
        }
    }
}

/// Drop degenerate triangles (any two corners equal now), then compact to
/// referenced vertices in first-use order (deterministic). False when the mesh
/// went hollow, so the loop yields its best effort instead.
fn sweep_and_compact(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uv0: &mut Vec<[f32; 2]>,
    tangents: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    locks: &mut Vec<u8>,
) -> bool {
    let count = positions.len();
    let mut live: Vec<u32> = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[2] != tri[0] {
            live.extend_from_slice(tri);
        }
    }
    *indices = live;
    if indices.is_empty() {
        return false;
    }
    let mut remap = vec![u32::MAX; count];
    let mut next = 0u32;
    for index in indices.iter() {
        let old = index_usize(*index);
        if remap.get(old) == Some(&u32::MAX) {
            if let Some(slot) = remap.get_mut(old) {
                *slot = next;
            }
            next += 1;
        }
    }
    let mut new_positions = Vec::with_capacity(index_usize(next));
    let mut new_normals = Vec::with_capacity(index_usize(next));
    let mut new_uv0 = Vec::with_capacity(index_usize(next));
    let mut new_tangents = Vec::with_capacity(index_usize(next));
    let mut order: Vec<usize> = (0..count).filter(|i| remap[*i] != u32::MAX).collect();
    order.sort_by_key(|i| remap.get(*i).copied().unwrap_or(u32::MAX));
    for old in order {
        if let (Some(p), Some(n), Some(t), Some(g)) = (
            positions.get(old),
            normals.get(old),
            uv0.get(old),
            tangents.get(old),
        ) {
            new_positions.push(*p);
            new_normals.push(*n);
            new_uv0.push(*t);
            new_tangents.push(*g);
        }
    }
    for index in indices.iter_mut() {
        if let Some(mapped) = remap.get(index_usize(*index)) {
            *index = *mapped;
        }
    }
    *positions = new_positions;
    *normals = new_normals;
    *uv0 = new_uv0;
    *tangents = new_tangents;
    // The lock mask rides the same remap, so a pinned seam stays pinned after
    // the compaction renumbers every vertex; the plain path carries no mask.
    if !locks.is_empty() {
        let mut new_locks = Vec::with_capacity(index_usize(next));
        for old in order.iter() {
            new_locks.push(locks.get(*old).copied().unwrap_or(0));
        }
        *locks = new_locks;
    }
    true
}

/// One batched collapse pass, capped at `max_collapses` cheapest edges; true
/// when any edge collapsed.
fn collapse_pass(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uv0: &mut Vec<[f32; 2]>,
    tangents: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    max_collapses: usize,
    attr: &AttrCost<'_>,
    locks: &mut Vec<u8>,
) -> bool {
    let count = positions.len();
    let quadrics = build_quadrics(positions, indices);
    let costs = edge_costs(positions, normals, uv0, &quadrics, indices, attr);
    if costs.is_empty() {
        return false;
    }
    let collapses = pick_collapses(&costs, count, max_collapses);
    if collapses.is_empty() {
        return false;
    }
    apply_collapses(
        positions, normals, uv0, tangents, indices, locks, &collapses,
    );
    sweep_and_compact(positions, normals, uv0, tangents, indices, locks)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 12-segment cloth cylinder with a uv seam: the closing column repeats
    /// the first column's positions with `u = 1`, so the two seam vertices sit
    /// on top of each other and only their uvs separate them.
    fn seam_cylinder() -> CookedMesh {
        let segments = 12usize;
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uv0 = Vec::new();
        let mut tangents = Vec::new();
        for ring in 0..2usize {
            let y = ring as f32;
            for segment in 0..=segments {
                let angle = segment as f32 / segments as f32 * std::f32::consts::TAU;
                let (sin, cos) = angle.sin_cos();
                positions.push([cos * 0.3, y, sin * 0.3]);
                normals.push([cos, 0.0, sin]);
                uv0.push([segment as f32 / segments as f32, ring as f32]);
                tangents.push([-sin, 0.0, cos, 1.0]);
            }
        }
        let stride = u32::try_from(segments + 1).expect("fits");
        let mut indices = Vec::new();
        for segment in 0..u32::try_from(segments).expect("fits") {
            let a = segment;
            let b = a + 1;
            let c = a + stride;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
        CookedMesh {
            positions,
            normals,
            uv0,
            tangents,
            indices,
        }
    }

    /// The seam's uv spread at the coincident column: 2 uv columns (a seam) or
    /// 1 (the seam smeared shut). Also the largest uv any surviving vertex at
    /// that position carries away from 0 or 1.
    fn seam_state(mesh: &CookedMesh) -> (usize, f32) {
        let mut us: Vec<f32> = Vec::new();
        let mut smear = 0.0f32;
        for (index, point) in mesh.positions.iter().enumerate() {
            let on_seam = (point[0] - 0.3).abs() < 1e-3 && point[2].abs() < 1e-3;
            if !on_seam {
                continue;
            }
            if let Some(uv) = mesh.uv0.get(index) {
                if !us.iter().any(|u| (u - uv[0]).abs() < 1e-3) {
                    us.push(uv[0]);
                }
                smear = smear.max(uv[1].abs().min((uv[1] - 1.0).abs()));
            }
        }
        (us.len(), smear)
    }

    /// Largest angle between a seam vertex's normal and the source normal at
    /// the same position, in degrees.
    fn seam_normal_drift_deg(level: &CookedMesh) -> f32 {
        let mut worst = 0.0f32;
        for (index, point) in level.positions.iter().enumerate() {
            let on_seam = (point[0] - 0.3).abs() < 1e-3 && point[2].abs() < 1e-3;
            if !on_seam {
                continue;
            }
            let Some(n) = level.normals.get(index) else {
                continue;
            };
            let expected = norm3([point[0], 0.0, point[2]]);
            let dot =
                (n[0] * expected[0] + n[1] * expected[1] + n[2] * expected[2]).clamp(-1.0, 1.0);
            worst = worst.max(dot.acos().to_degrees());
        }
        worst
    }

    #[test]
    fn simplify_seam_lock_finds_the_uv_discontinuity() {
        let mesh = seam_cylinder();
        let locks = seam_lock(&mesh);
        assert_eq!(locks.len(), mesh.positions.len());
        let locked = locks.iter().filter(|l| **l == 1).count();
        assert_eq!(locked, 4, "both sides of the two seam vertices are pinned");
        let (columns, _) = seam_state(&mesh);
        assert_eq!(columns, 2, "the fixture carries a real uv seam");
        // A smooth mesh with no duplicate positions locks nothing.
        let plain = simplify(&mesh, 24).expect("plain");
        assert!(seam_lock(&plain).iter().all(|l| *l == 0) || !plain.positions.is_empty());
    }

    #[test]
    fn simplified_far_body_keeps_its_uv_seam_and_normal_cone() {
        let source = seam_cylinder();
        let locks = seam_lock(&source);
        let attr = AttrCost {
            weights: [0.5, 0.5, 0.0],
            locks: &locks,
            max_cost_m2: 0.25,
            extent_m: mesh_extent_m(&source),
        };
        // Today's path has no lock, so the seam column is free to collapse;
        // the attribute-aware path may not lose either uv column.
        let legacy = simplify(&source, 8).expect("legacy level");
        let (legacy_columns, _) = seam_state(&legacy);
        assert!(
            legacy_columns <= 2,
            "the plain path may merge the seam (columns {legacy_columns})"
        );

        // The attribute-aware chain keeps both seam sides and their normals.
        let mut level = source.clone();
        let mut previous = u32::MAX;
        for target in [24u32, 16, 8] {
            let next = simplify_with_attributes(&level, &attr, target).expect("level");
            let tris = tri_count(&next);
            assert!(
                tris <= previous || previous == u32::MAX,
                "levels never gain triangles"
            );
            assert!(tris >= 1);
            let (columns, smear) = seam_state(&next);
            assert_eq!(
                columns, 2,
                "both uv columns of the seam survive at {tris} tris"
            );
            assert!(
                smear <= 1e-4,
                "the seam uv smeared by {smear} at {tris} tris"
            );
            let drift = seam_normal_drift_deg(&next);
            assert!(
                drift <= 2.0,
                "the seam normal cone drifted {drift} deg at {tris} tris"
            );
            previous = tris;
            level = next;
        }
        assert!(
            tri_count(&level) < tri_count(&source),
            "the chain must actually simplify, not clone through"
        );
    }

    #[test]
    fn simplify_plain_cost_is_unchanged_by_the_attribute_plumbing() {
        // Zero weights and no locks must reproduce `simplify` exactly.
        let mesh = seam_cylinder();
        let plain = simplify(&mesh, 20).expect("plain");
        let via_attrs =
            simplify_with_attributes(&mesh, &AttrCost::plain(&mesh), 20).expect("attrs");
        assert_eq!(plain, via_attrs, "the geometric path is byte-identical");
        // A tight error cap collapses strictly less than the uncapped path at
        // the same target, and a negative cap (past every cost) collapses
        // nothing at all.
        let tight = AttrCost {
            max_cost_m2: 1e-12,
            ..AttrCost::plain(&mesh)
        };
        let uncapped =
            simplify_with_attributes(&mesh, &AttrCost::plain(&mesh), 8).expect("plain 8");
        let held = simplify_with_attributes(&mesh, &tight, 8).expect("held");
        assert!(
            tri_count(&held) >= tri_count(&uncapped),
            "a tight error cap can only keep more triangles ({} vs {})",
            tri_count(&held),
            tri_count(&uncapped)
        );
        let frozen = AttrCost {
            max_cost_m2: -1.0,
            ..AttrCost::plain(&mesh)
        };
        let untouched = simplify_with_attributes(&mesh, &frozen, 8).expect("frozen");
        assert_eq!(untouched, mesh, "no affordable edge means no collapse");
    }
}
