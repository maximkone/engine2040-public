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
use std::collections::BTreeSet;

/// Most simplification passes before the loop yields its best effort.
const MAX_SIMPLIFY_PASSES: usize = 64;

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
        if !collapse_pass(
            &mut positions,
            &mut normals,
            &mut uv0,
            &mut tangents,
            &mut indices,
            max_collapses,
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
/// sort last but stay ordered by vertex indices, so reruns agree.
fn edge_costs(
    positions: &[[f32; 3]],
    quadrics: &[Quadric],
    indices: &[u32],
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
        let mid = [
            (f64::from(pa[0]) + f64::from(pb[0])) * 0.5,
            (f64::from(pa[1]) + f64::from(pb[1])) * 0.5,
            (f64::from(pa[2]) + f64::from(pb[2])) * 0.5,
        ];
        let mut combined = *qa;
        quadric_add(&mut combined, qb);
        let mut cost = quadric_cost(&combined, mid);
        if !cost.is_finite() {
            cost = f64::INFINITY;
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
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uv0: &mut Vec<[f32; 2]>,
    tangents: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    collapses: &[(usize, usize)],
) {
    for (a, b) in collapses {
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
) -> bool {
    let count = positions.len();
    let quadrics = build_quadrics(positions, indices);
    let costs = edge_costs(positions, &quadrics, indices);
    if costs.is_empty() {
        return false;
    }
    let collapses = pick_collapses(&costs, count, max_collapses);
    if collapses.is_empty() {
        return false;
    }
    apply_collapses(positions, normals, uv0, tangents, indices, &collapses);
    sweep_and_compact(positions, normals, uv0, tangents, indices)
}
