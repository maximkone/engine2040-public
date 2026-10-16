//! meshlet: cone-weighted meshlet cook step (64 vertices, 124 triangles).
//!
//! Owner: `engine-assets`; status: no mesh-cook caller yet (`mesh_cook.rs`
//! has no meshlet use); `cook_meshlet_lod` in this module clusters for the
//! render meshlet path. Units: meters in, cluster
//! lists out. Determinism: index-order loops, sorted frontier picks and
//! `total_cmp` float order, so the same positions plus indices always emit
//! the same meshlets. Errors: typed [`MeshletError`], never a panic.
//!
//! Cluster-growth pattern adapted from meshoptimizer (Zeux, MIT,
//! `zeux/meshoptimizer` meshlet builder): greedy cone-weighted growth
//! under the same 64v/124t limits. No vendored bytes; reimplemented here.

use super::import::validate_ktx2;
use engine_core::Fnv64;
use std::fmt;

/// Most vertices one meshlet holds (meshoptimizer-compatible limit).
pub const MAX_MESHLET_VERTICES: usize = 64;
/// Most triangles one meshlet holds (meshoptimizer-compatible limit).
pub const MAX_MESHLET_TRIANGLES: usize = 124;
/// Most input vertices one cook accepts; mirrors the import bound scale.
pub const MAX_INPUT_VERTICES: usize = 1_000_000;
/// Most input triangles one cook accepts; bounds adjacency work.
pub const MAX_INPUT_TRIANGLES: usize = 2_000_000;
/// Cone alignment weight against the distance penalty in the growth score.
const CONE_WEIGHT: f32 = 4.0;
/// Squared length at or below which a vector counts as degenerate.
const DEGENERATE_LEN2: f32 = 1e-24;

/// Why a meshlet cook refused its input; every malformed input maps here.
#[derive(Clone, Debug, PartialEq)]
pub enum MeshletError {
    /// No vertices or no triangles at all.
    EmptyMesh,
    /// Index buffer length is not a multiple of three; length carried.
    UnalignedIndices {
        /// Index count that failed the multiple-of-three check.
        len: usize,
    },
    /// Index addresses no vertex; culprit index carried.
    IndexOutOfRange(u32),
    /// Non-finite float in the named field.
    NonFinite(&'static str),
    /// Input exceeds a cook bound; count carried.
    TooLarge {
        /// `vertices` or `triangles`.
        field: &'static str,
        /// Offending count.
        count: usize,
    },
    /// A DAG level blew past its error bound; level plus measured error.
    ErrorOverBound { level: usize, error: f32 },
    /// A texture reference is not KTX2-baked; refused, never converted.
    NonKtx2 { image: String },
}

impl fmt::Display for MeshletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMesh => write!(f, "empty mesh"),
            Self::UnalignedIndices { len } => {
                write!(f, "index count {len} is not a multiple of three")
            }
            Self::IndexOutOfRange(index) => write!(f, "index {index} is out of range"),
            Self::NonFinite(field) => write!(f, "non-finite float in {field}"),
            Self::TooLarge { field, count } => {
                write!(f, "{field} count {count} exceeds the cook bound")
            }
            Self::ErrorOverBound { level, error } => {
                write!(f, "LOD {level} error {error} exceeds the bound")
            }
            Self::NonKtx2 { image } => write!(f, "texture {image} is not KTX2"),
        }
    }
}

/// One cluster: global vertex ids plus triangles as local indices.
#[derive(Clone, PartialEq, Debug)]
pub struct Meshlet {
    /// Global vertex ids packed into this cluster (at most 64).
    pub vertices: Vec<u32>,
    /// Triangles as local indices into [`Meshlet::vertices`] (at most 124).
    pub triangles: Vec<[u32; 3]>,
}

/// Bounding sphere plus normal cone of one meshlet.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MeshletBounds {
    /// Sphere center in meters.
    pub center: [f32; 3],
    /// Sphere radius in meters.
    pub radius_m: f32,
    /// Unit cone axis (face-normal mean, or `+Z` when degenerate).
    pub cone_axis: [f32; 3],
    /// Cosine of the cone half-angle (`1.0` is a perfectly flat cluster).
    pub cone_cos: f32,
}

/// A `u32` index as `usize`; failure names the culprit id.
fn idx_usize(id: u32) -> Result<usize, MeshletError> {
    usize::try_from(id).map_err(|_| MeshletError::IndexOutOfRange(id))
}

/// A count as `u32`; failure names the overflowing field.
fn count_u32(count: usize, field: &'static str) -> Result<u32, MeshletError> {
    u32::try_from(count).map_err(|_| MeshletError::TooLarge { field, count })
}

/// A count as `f32` for scores and means; saturates instead of wrapping.
///
/// Callers pass cluster-scale counts (at most hundreds), well inside `u16`.
fn count_f32(count: usize) -> f32 {
    match u16::try_from(count) {
        Ok(value) => f32::from(value),
        Err(_) => f32::from(u16::MAX),
    }
}

/// Per-triangle frames: unit face normals plus centroids, in index order.
type TriFrames = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// Validate positions plus indices; returns the triangle count.
///
/// Errors: empty input, unaligned indices, oversized input, non-finite
/// positions or out-of-range indices.
fn validate_inputs(positions: &[[f32; 3]], indices: &[u32]) -> Result<usize, MeshletError> {
    if positions.is_empty() || indices.is_empty() {
        return Err(MeshletError::EmptyMesh);
    }
    if !indices.len().is_multiple_of(3) {
        return Err(MeshletError::UnalignedIndices { len: indices.len() });
    }
    let tri_count = indices.len() / 3;
    if positions.len() > MAX_INPUT_VERTICES {
        return Err(MeshletError::TooLarge {
            field: "vertices",
            count: positions.len(),
        });
    }
    if tri_count > MAX_INPUT_TRIANGLES {
        return Err(MeshletError::TooLarge {
            field: "triangles",
            count: tri_count,
        });
    }
    if positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(MeshletError::NonFinite("positions"));
    }
    let bound = count_u32(positions.len(), "vertices")?;
    for index in indices {
        if *index >= bound {
            return Err(MeshletError::IndexOutOfRange(*index));
        }
    }
    Ok(tri_count)
}

/// Dot product of two 3-vectors.
fn dot(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Normalize, falling back to `+Z` for degenerate input.
fn normalize_or_z(v: &[f32; 3]) -> [f32; 3] {
    let len2 = dot(v, v);
    if len2 <= DEGENERATE_LEN2 {
        return [0.0, 0.0, 1.0];
    }
    let inv = 1.0 / len2.sqrt();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

/// One indexed corner position; validated indices always resolve.
fn corner_at(
    positions: &[[f32; 3]],
    indices: &[u32],
    base: usize,
    corner: usize,
) -> Result<[f32; 3], MeshletError> {
    let id = indices
        .get(base + corner)
        .copied()
        .ok_or(MeshletError::UnalignedIndices { len: indices.len() })?;
    positions
        .get(idx_usize(id)?)
        .copied()
        .ok_or(MeshletError::IndexOutOfRange(id))
}

/// Per-triangle unit face normals plus centroids, in index order.
///
/// Errors: the [`MeshletError`] cases from input validation.
fn tri_frames(
    positions: &[[f32; 3]],
    indices: &[u32],
    tri_count: usize,
) -> Result<TriFrames, MeshletError> {
    let mut normals = Vec::with_capacity(tri_count);
    let mut centroids = Vec::with_capacity(tri_count);
    for tri in 0..tri_count {
        let base = tri.saturating_mul(3);
        let a = corner_at(positions, indices, base, 0)?;
        let b = corner_at(positions, indices, base, 1)?;
        let c = corner_at(positions, indices, base, 2)?;
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        normals.push(normalize_or_z(&cross));
        centroids.push([
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ]);
    }
    Ok((normals, centroids))
}

/// True when a triangle's new vertices still fit the running cluster.
fn cluster_fits(verts: &[u32], tri_len: usize, global: &[u32; 3]) -> bool {
    if tri_len >= MAX_MESHLET_TRIANGLES {
        return false;
    }
    let mut fresh = 0;
    for vertex in global {
        if !verts.contains(vertex) {
            fresh += 1;
        }
    }
    verts.len() + fresh <= MAX_MESHLET_VERTICES
}

/// Push one global triangle into a running cluster's local index form.
///
/// Errors: [`MeshletError::TooLarge`] when a local slot cannot fit `u32`.
fn cluster_push(
    verts: &mut Vec<u32>,
    tris: &mut Vec<[u32; 3]>,
    global: [u32; 3],
) -> Result<(), MeshletError> {
    let mut local = [0_u32; 3];
    for (slot, vertex) in local.iter_mut().zip(global.iter()) {
        match verts.iter().position(|v| v == vertex) {
            Some(found) => {
                *slot = count_u32(found, "vertices")?;
            }
            None => {
                verts.push(*vertex);
                *slot = count_u32(verts.len().saturating_sub(1), "vertices")?;
            }
        }
    }
    tris.push(local);
    Ok(())
}

/// Naive baseline: pack triangles in input order under 64v/124t.
///
/// Same validation as the cone-weighted path, so counts compare directly.
/// Errors: the [`MeshletError`] cases from input validation.
pub fn build_meshlets_naive(
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Result<Vec<Meshlet>, MeshletError> {
    let tri_count = validate_inputs(positions, indices)?;
    let mut out = Vec::new();
    let mut verts = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for tri in 0..tri_count {
        let base = tri.saturating_mul(3);
        let Some(global) = indices
            .get(base..base.saturating_add(3))
            .and_then(|w| <[u32; 3]>::try_from(w).ok())
        else {
            return Err(MeshletError::UnalignedIndices { len: indices.len() });
        };
        if !tris.is_empty() && !cluster_fits(&verts, tris.len(), &global) {
            out.push(Meshlet {
                vertices: std::mem::take(&mut verts),
                triangles: std::mem::take(&mut tris),
            });
        }
        cluster_push(&mut verts, &mut tris, global)?;
    }
    if !tris.is_empty() {
        out.push(Meshlet {
            vertices: verts,
            triangles: tris,
        });
    }
    Ok(out)
}

/// Vertex-to-triangle adjacency in triangle-id order; bounds total work.
///
/// Errors: [`MeshletError::TooLarge`] when a triangle id cannot fit `u32`.
fn vertex_adjacency(
    vert_count: usize,
    tri_count: usize,
    indices: &[u32],
) -> Result<Vec<Vec<u32>>, MeshletError> {
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); vert_count];
    for tri in 0..tri_count {
        let id = count_u32(tri, "triangles")?;
        for corner in 0..3 {
            let Some(slot) = indices
                .get(tri.saturating_mul(3).saturating_add(corner))
                .copied()
                .and_then(|v| idx_usize(v).ok())
            else {
                return Err(MeshletError::UnalignedIndices { len: indices.len() });
            };
            adj[slot].push(id);
        }
    }
    Ok(adj)
}

/// Growth score: cone alignment minus the distance penalty (meters).
fn candidate_score(
    normal: &[f32; 3],
    centroid: &[f32; 3],
    axis: &[f32; 3],
    cluster_center: &[f32; 3],
) -> f32 {
    let align = dot(normal, axis);
    let dx = centroid[0] - cluster_center[0];
    let dy = centroid[1] - cluster_center[1];
    let dz = centroid[2] - cluster_center[2];
    align * CONE_WEIGHT - (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Mean of the member centroids; the spatial anchor of the growth score.
///
/// Errors: [`MeshletError::IndexOutOfRange`] for a member outside the frame.
fn cluster_center_of(centroids: &[[f32; 3]], members: &[u32]) -> Result<[f32; 3], MeshletError> {
    let mut sum = [0.0_f32; 3];
    for member in members {
        let centroid = centroids
            .get(idx_usize(*member)?)
            .copied()
            .ok_or(MeshletError::IndexOutOfRange(*member))?;
        sum[0] += centroid[0];
        sum[1] += centroid[1];
        sum[2] += centroid[2];
    }
    let count = count_f32(members.len());
    Ok([sum[0] / count, sum[1] / count, sum[2] / count])
}

/// Shared cook frames for cone-weighted growth: triangle normals,
/// centroids, indices and adjacency, all in triangle-id order.
struct Frames<'a> {
    /// Per-triangle unit face normals.
    normals: &'a [[f32; 3]],
    /// Per-triangle centroids in meters.
    centroids: &'a [[f32; 3]],
    /// Full index buffer, one triple per triangle.
    indices: &'a [u32],
    /// Vertex-to-triangle adjacency in triangle-id order.
    adj: &'a [Vec<u32>],
}

/// Best fitting frontier triangle: adjacent, unassigned and under 64v/124t.
///
/// Ties break toward the lowest triangle id, so the pick is stable.
/// Errors: the [`MeshletError`] cases from frame lookups.
fn frontier_best(
    verts: &[u32],
    members: &[u32],
    frames: &Frames<'_>,
    assigned: &[bool],
    scratch: &mut Vec<u32>,
    stamp: &mut [u32],
    generation: u32,
) -> Result<Option<u32>, MeshletError> {
    scratch.clear();
    for vertex in verts {
        let Some(neighbours) = frames.adj.get(idx_usize(*vertex)?) else {
            return Err(MeshletError::IndexOutOfRange(*vertex));
        };
        for candidate in neighbours {
            let slot = idx_usize(*candidate)?;
            let taken = assigned.get(slot).copied().unwrap_or(true);
            if taken || stamp[slot] == generation || members.contains(candidate) {
                continue;
            }
            stamp[slot] = generation;
            scratch.push(*candidate);
        }
    }
    scratch.sort_unstable();
    let mut axis_sum = [0.0_f32; 3];
    for member in members {
        let normal = frames
            .normals
            .get(idx_usize(*member)?)
            .copied()
            .ok_or(MeshletError::IndexOutOfRange(*member))?;
        axis_sum[0] += normal[0];
        axis_sum[1] += normal[1];
        axis_sum[2] += normal[2];
    }
    let axis = normalize_or_z(&axis_sum);
    let center = cluster_center_of(frames.centroids, members)?;
    let mut best: Option<(u32, f32)> = None;
    for candidate in scratch.iter() {
        let slot = idx_usize(*candidate)?;
        let tri_words = frames
            .indices
            .get(slot.saturating_mul(3)..slot.saturating_mul(3).saturating_add(3))
            .and_then(|w| <[u32; 3]>::try_from(w).ok());
        let Some(global) = tri_words else {
            return Err(MeshletError::UnalignedIndices {
                len: frames.indices.len(),
            });
        };
        if !cluster_fits(verts, members.len(), &global) {
            continue;
        }
        let Some(normal) = frames.normals.get(slot).copied() else {
            return Err(MeshletError::IndexOutOfRange(*candidate));
        };
        let Some(centroid) = frames.centroids.get(slot).copied() else {
            return Err(MeshletError::IndexOutOfRange(*candidate));
        };
        let score = candidate_score(&normal, &centroid, &axis, &center);
        let replace = match best {
            None => true,
            Some((id, value)) => {
                score.total_cmp(&value) == std::cmp::Ordering::Greater
                    || (score.total_cmp(&value) == std::cmp::Ordering::Equal && *candidate < id)
            }
        };
        if replace {
            best = Some((*candidate, score));
        }
    }
    Ok(best.map(|(id, _)| id))
}

/// Grow one cone-weighted cluster from its seed triangle.
///
/// Errors: the [`MeshletError`] cases from frame lookups.
fn grow_cluster(
    seed: u32,
    frames: &Frames<'_>,
    assigned: &[bool],
    scratch: &mut Vec<u32>,
    stamp: &mut [u32],
    generation: u32,
) -> Result<(Meshlet, Vec<u32>), MeshletError> {
    let mut verts = Vec::new();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    let mut members = Vec::new();
    let seed_slot = idx_usize(seed)?;
    let seed_words = frames
        .indices
        .get(seed_slot.saturating_mul(3)..seed_slot.saturating_mul(3).saturating_add(3))
        .and_then(|w| <[u32; 3]>::try_from(w).ok());
    let Some(seed_tri) = seed_words else {
        return Err(MeshletError::UnalignedIndices {
            len: frames.indices.len(),
        });
    };
    cluster_push(&mut verts, &mut tris, seed_tri)?;
    members.push(seed);
    let mut step = generation;
    while members.len() < MAX_MESHLET_TRIANGLES {
        step = step.wrapping_add(1);
        let next = frontier_best(&verts, &members, frames, assigned, scratch, stamp, step)?;
        match next {
            None => break,
            Some(id) => {
                let slot = idx_usize(id)?;
                let words = frames
                    .indices
                    .get(slot.saturating_mul(3)..slot.saturating_mul(3).saturating_add(3))
                    .and_then(|w| <[u32; 3]>::try_from(w).ok());
                let Some(tri) = words else {
                    return Err(MeshletError::UnalignedIndices {
                        len: frames.indices.len(),
                    });
                };
                cluster_push(&mut verts, &mut tris, tri)?;
                members.push(id);
            }
        }
    }
    Ok((
        Meshlet {
            vertices: verts,
            triangles: tris,
        },
        members,
    ))
}

/// Cone-weighted clusters under 64v/124t (meshoptimizer-style growth).
///
/// Greedy region growth: each cluster starts at the lowest unassigned
/// triangle id and absorbs the best-scoring adjacent triangle (normal-cone
/// alignment minus distance) until no adjacent triangle fits. Same
/// validation as [`build_meshlets_naive`], so counts compare directly.
/// Errors: the [`MeshletError`] cases from input validation.
pub fn build_meshlets(
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Result<Vec<Meshlet>, MeshletError> {
    let tri_count = validate_inputs(positions, indices)?;
    let (normals, centroids) = tri_frames(positions, indices, tri_count)?;
    let adj = vertex_adjacency(positions.len(), tri_count, indices)?;
    let frames = Frames {
        normals: &normals,
        centroids: &centroids,
        indices,
        adj: &adj,
    };
    let mut assigned = vec![false; tri_count];
    let mut stamp = vec![0_u32; tri_count];
    let mut scratch = Vec::new();
    let mut out = Vec::with_capacity(tri_count.div_ceil(MAX_MESHLET_TRIANGLES));
    let mut remaining = tri_count;
    let mut generation: u32 = 1;
    while remaining > 0 {
        let Some(seed) = assigned.iter().position(|done| !done) else {
            break;
        };
        let seed_id = count_u32(seed, "triangles")?;
        generation = generation.wrapping_add(1);
        let (meshlet, members) = grow_cluster(
            seed_id,
            &frames,
            &assigned,
            &mut scratch,
            &mut stamp,
            generation,
        )?;
        for member in members {
            assigned[idx_usize(member)?] = true;
            remaining = remaining.saturating_sub(1);
        }
        out.push(meshlet);
    }
    Ok(out)
}

/// One local corner as its global vertex id; `None` when unaddressable.
fn local_vertex(meshlet: &Meshlet, corner: u32) -> Option<u32> {
    meshlet.vertices.get(usize::try_from(corner).ok()?).copied()
}

/// One global vertex position; `None` when outside `positions`.
fn position_of(positions: &[[f32; 3]], id: u32) -> Option<[f32; 3]> {
    positions.get(usize::try_from(id).ok()?).copied()
}

/// Bounding sphere plus normal cone of one meshlet over `positions`.
///
/// Missing or degenerate triangles are skipped, so foreign meshlets
/// report the bounds of their resolvable part instead of panicking.
pub fn meshlet_bounds(positions: &[[f32; 3]], meshlet: &Meshlet) -> MeshletBounds {
    let empty = MeshletBounds {
        center: [0.0, 0.0, 0.0],
        radius_m: 0.0,
        cone_axis: [0.0, 0.0, 1.0],
        cone_cos: 1.0,
    };
    if meshlet.vertices.is_empty() || meshlet.triangles.is_empty() {
        return empty;
    }
    let mut points: Vec<[f32; 3]> = Vec::with_capacity(meshlet.vertices.len());
    for vertex in &meshlet.vertices {
        if let Some(point) = position_of(positions, *vertex) {
            points.push(point);
        }
    }
    if points.is_empty() {
        return empty;
    }
    let mut center = [0.0_f32; 3];
    for point in &points {
        center[0] += point[0];
        center[1] += point[1];
        center[2] += point[2];
    }
    let count = count_f32(points.len());
    center[0] /= count;
    center[1] /= count;
    center[2] /= count;
    let mut radius_m = 0.0_f32;
    for point in &points {
        let dx = point[0] - center[0];
        let dy = point[1] - center[1];
        let dz = point[2] - center[2];
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist.total_cmp(&radius_m) == std::cmp::Ordering::Greater {
            radius_m = dist;
        }
    }
    let mut axis_sum = [0.0_f32; 3];
    let mut face_normals: Vec<[f32; 3]> = Vec::with_capacity(meshlet.triangles.len());
    for tri in &meshlet.triangles {
        let (Some(a), Some(b), Some(c)) = (
            local_vertex(meshlet, tri[0]).and_then(|v| position_of(positions, v)),
            local_vertex(meshlet, tri[1]).and_then(|v| position_of(positions, v)),
            local_vertex(meshlet, tri[2]).and_then(|v| position_of(positions, v)),
        ) else {
            continue;
        };
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        if dot(&cross, &cross) > DEGENERATE_LEN2 {
            let normal = normalize_or_z(&cross);
            axis_sum[0] += normal[0];
            axis_sum[1] += normal[1];
            axis_sum[2] += normal[2];
            face_normals.push(normal);
        }
    }
    let cone_axis = normalize_or_z(&axis_sum);
    let mut cone_cos = 1.0_f32;
    for normal in &face_normals {
        let cos = dot(normal, &cone_axis);
        if cos.total_cmp(&cone_cos) == std::cmp::Ordering::Less {
            cone_cos = cos;
        }
    }
    MeshletBounds {
        center,
        radius_m,
        cone_axis,
        cone_cos,
    }
}

/// Largest meshlet bounding radius in meters; `0.0` for no meshlets.
pub fn max_bound_radius_m(positions: &[[f32; 3]], meshlets: &[Meshlet]) -> f32 {
    let mut max = 0.0_f32;
    for meshlet in meshlets {
        let radius_m = meshlet_bounds(positions, meshlet).radius_m;
        if radius_m.total_cmp(&max) == std::cmp::Ordering::Greater {
            max = radius_m;
        }
    }
    max
}

/// Triangle fraction each DAG level keeps from the base mesh.
pub const LOD_RATIOS: [f32; 4] = [1.0, 0.5, 0.25, 0.125];
/// Largest accepted simplification error relative to mesh extent.
pub const LOD_MAX_ERROR: f32 = 0.25;

/// One DAG level: simplified mesh plus one cluster set plus its digest.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshletLod {
    /// Target triangle fraction of the base mesh (see [`LOD_RATIOS`]).
    pub ratio: f32,
    /// Measured simplification error relative to mesh extent.
    pub error: f32,
    /// Compacted vertex positions in meters, input order kept.
    pub positions: Vec<[f32; 3]>,
    /// Triangle indices into [`MeshletLod::positions`].
    pub indices: Vec<u32>,
    /// Cone-weighted clusters of this level under 64v/124t.
    pub meshlets: Vec<Meshlet>,
    /// Running chained digest after folding this level in.
    pub digest: u64,
}

/// Cooked DAG: one [`MeshletLod`] per [`LOD_RATIOS`] level plus the digest.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshletLodCook {
    /// Levels in [`LOD_RATIOS`] order; level 0 is the base mesh.
    pub levels: Vec<MeshletLod>,
    /// Chained digest of every level plus every KTX2 texture byte.
    pub digest: u64,
}

/// Vertices on a boundary edge (a directed edge with no reverse); locked
/// against collapse so open borders never crack (the seam/border lock).
fn border_locked(indices: &[u32], vert_count: usize) -> Vec<bool> {
    let mut edges = Vec::with_capacity(indices.len());
    for tri in indices.chunks_exact(3) {
        for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
            edges.push((a, b));
        }
    }
    edges.sort_unstable();
    let mut locked = vec![false; vert_count];
    for (a, b) in &edges {
        if edges.binary_search(&(*b, *a)).is_err() {
            for end in [*a, *b] {
                if let Ok(slot) = idx_usize(end) {
                    if slot < vert_count {
                        locked[slot] = true;
                    }
                }
            }
        }
    }
    locked
}

/// Greedy midpoint edge collapse down to `target` triangles: cheapest open
/// edge first (index-order scan, first cheapest wins), locked vertices never
/// move, survivors keep input order, degenerate triangles drop. Returns the
/// compacted mesh plus the largest collapse cost relative to mesh extent.
/// Caller has validated ids, so indexing is in range. Errors: count failures.
fn simplify_level(
    positions: &[[f32; 3]],
    indices: &[u32],
    target: usize,
) -> Result<(Vec<[f32; 3]>, Vec<u32>, f32), MeshletError> {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in positions {
        for ((l, h), v) in lo.iter_mut().zip(hi.iter_mut()).zip(p.iter()) {
            *l = (*l).min(*v);
            *h = (*h).max(*v);
        }
    }
    let sp = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let scale = (sp[0] * sp[0] + sp[1] * sp[1] + sp[2] * sp[2]).max(1e-12);
    let locked = border_locked(indices, positions.len());
    let mut verts = positions.to_vec();
    let mut tris: Vec<[u32; 3]> = indices
        .chunks_exact(3)
        .map(|w| [w[0], w[1], w[2]])
        .collect();
    let mut error = 0.0_f32;
    while tris.len() > target {
        let mut best: Option<(f32, u32, u32)> = None;
        for tri in &tris {
            for (a, b) in [(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                if a == b {
                    continue;
                }
                let (ai, bi) = (idx_usize(a)?, idx_usize(b)?);
                if locked[ai] || locked[bi] {
                    continue;
                }
                let (pa, pb) = (verts[ai], verts[bi]);
                let d = [pa[0] - pb[0], pa[1] - pb[1], pa[2] - pb[2]];
                let cost = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) / scale;
                let take = match best {
                    None => true,
                    Some((v, _, _)) => cost.total_cmp(&v) == std::cmp::Ordering::Less,
                };
                if take {
                    best = Some((cost, a, b));
                }
            }
        }
        let Some((cost, a, b)) = best else {
            break;
        };
        error = error.max(cost);
        let (ai, bi) = (idx_usize(a)?, idx_usize(b)?);
        let (pa, pb) = (verts[ai], verts[bi]);
        verts[ai] = [
            (pa[0] + pb[0]) * 0.5,
            (pa[1] + pb[1]) * 0.5,
            (pa[2] + pb[2]) * 0.5,
        ];
        for tri in tris.iter_mut() {
            for corner in tri.iter_mut() {
                if *corner == b {
                    *corner = a;
                }
            }
        }
        tris.retain(|t| t[0] != t[1] && t[0] != t[2] && t[1] != t[2]);
    }
    let mut seen = vec![false; verts.len()];
    for tri in &tris {
        for corner in tri {
            seen[idx_usize(*corner)?] = true;
        }
    }
    let mut kept_pos = Vec::new();
    let mut remap = vec![0_u32; verts.len()];
    for (slot, point) in verts.iter().enumerate() {
        if seen[slot] {
            remap[slot] = count_u32(kept_pos.len(), "vertices")?;
            kept_pos.push(*point);
        }
    }
    let mut kept_idx = Vec::with_capacity(tris.len() * 3);
    for tri in &tris {
        for corner in tri {
            kept_idx.push(remap[idx_usize(*corner)?]);
        }
    }
    Ok((kept_pos, kept_idx, error))
}

/// LE-byte digest of one level in vertex-fetch order: ratio, error, then
/// positions and indices in stored order (the meshopt encode field order).
fn level_digest(
    chain: &mut Fnv64,
    ratio: f32,
    error: f32,
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Result<(), MeshletError> {
    chain.write_f32(ratio);
    chain.write_f32(error);
    chain.write_u64(u64::from(count_u32(positions.len(), "vertices")?));
    for point in positions {
        for lane in point {
            chain.write_f32(*lane);
        }
    }
    chain.write_u64(u64::from(count_u32(indices.len(), "indices")?));
    for index in indices {
        chain.write_u32(*index);
    }
    Ok(())
}

/// Cook the [`LOD_RATIOS`] DAG: per level simplify from the base mesh
/// (errors never compound), clusterize with [`build_meshlets`], digest the
/// encoded level, then gate every texture through `validate_ktx2` and fold
/// its bytes into the chained digest. Caller: this module (drives
/// `build_meshlets` per level); no render caller yet (`render/src/meshlet.rs`
/// never imports [`MeshletLodCook`]).
/// Errors: validation, [`MeshletError::ErrorOverBound`], [`MeshletError::NonKtx2`].
pub fn cook_meshlet_lod(
    positions: &[[f32; 3]],
    indices: &[u32],
    textures: &[(&str, &[u8])],
    max_error: f32,
) -> Result<MeshletLodCook, MeshletError> {
    let tri_count = validate_inputs(positions, indices)?;
    if !max_error.is_finite() || max_error < 0.0 {
        return Err(MeshletError::NonFinite("max_error"));
    }
    let mut levels = Vec::with_capacity(LOD_RATIOS.len());
    let mut chain = Fnv64::new();
    for (level, ratio) in LOD_RATIOS.iter().enumerate() {
        let target = tri_count.div_ceil(1_usize << level);
        let (kept_pos, kept_idx, error) = simplify_level(positions, indices, target)?;
        if error > max_error {
            return Err(MeshletError::ErrorOverBound { level, error });
        }
        let meshlets = build_meshlets(&kept_pos, &kept_idx)?;
        level_digest(&mut chain, *ratio, error, &kept_pos, &kept_idx)?;
        levels.push(MeshletLod {
            ratio: *ratio,
            error,
            positions: kept_pos,
            indices: kept_idx,
            meshlets,
            digest: chain.finish(),
        });
    }
    for (name, bytes) in textures {
        validate_ktx2(name, bytes).map_err(|_| MeshletError::NonKtx2 {
            image: (*name).to_owned(),
        })?;
        chain.write(bytes);
    }
    Ok(MeshletLodCook {
        levels,
        digest: chain.finish(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Planar grid with scattered triangle order: row-major quads permuted
    /// by a coprime stride, so input-order packing shares almost no verts.
    fn scattered_grid(cells: usize) -> (Vec<[f32; 3]>, Vec<u32>) {
        let side = cells.saturating_add(1);
        let mut positions = Vec::with_capacity(side.saturating_mul(side));
        for y in 0..side {
            for x in 0..side {
                let wide_x = u16::try_from(x).unwrap();
                let wide_y = u16::try_from(y).unwrap();
                positions.push([f32::from(wide_x), f32::from(wide_y), 0.0]);
            }
        }
        let at = |x: usize, y: usize| -> u32 { u32::try_from(y * side + x).unwrap() };
        let mut ordered = Vec::new();
        for y in 0..cells {
            for x in 0..cells {
                let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1));
                ordered.push([a, b, c]);
                ordered.push([b, d, c]);
            }
        }
        let count = ordered.len();
        let mut permuted: Vec<[u32; 3]> = vec![[0, 0, 0]; count];
        let stride = 7;
        for (slot, tri) in ordered.into_iter().enumerate() {
            permuted[(slot.saturating_mul(stride)) % count] = tri;
        }
        (positions, permuted.into_iter().flatten().collect())
    }

    fn small_mesh() -> (Vec<[f32; 3]>, Vec<u32>) {
        (
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            vec![0, 1, 2, 1, 3, 2],
        )
    }

    #[test]
    fn cone_weighted_beats_naive_by_fifteen_percent_at_equal_bounds() {
        let (positions, indices) = scattered_grid(24);
        let naive = build_meshlets_naive(&positions, &indices).unwrap();
        let cone = build_meshlets(&positions, &indices).unwrap();
        let naive_radius = max_bound_radius_m(&positions, &naive);
        let cone_radius = max_bound_radius_m(&positions, &cone);
        let reduction =
            100.0 * count_f32(naive.len().saturating_sub(cone.len())) / count_f32(naive.len());
        println!(
            "meshlet naive={} optimized={} reduction={reduction:.1}% \
             max_radius_naive={naive_radius:.3}m max_radius_opt={cone_radius:.3}m",
            naive.len(),
            cone.len()
        );
        assert!(
            cone.len().saturating_mul(100) <= naive.len().saturating_mul(85),
            "cone-weighted must beat naive by 15pct"
        );
        assert!(
            cone_radius <= naive_radius * 1.001 + 1e-6,
            "tighter clusters must not inflate bounds"
        );
    }

    #[test]
    fn clusters_respect_limits_cover_every_triangle_and_repeat() {
        let (positions, indices) = scattered_grid(12);
        let tri_count = indices.len() / 3;
        for meshlets in [
            build_meshlets_naive(&positions, &indices).unwrap(),
            build_meshlets(&positions, &indices).unwrap(),
        ] {
            assert!(!meshlets.is_empty());
            let mut covered = 0;
            for meshlet in &meshlets {
                assert!(meshlet.vertices.len() <= MAX_MESHLET_VERTICES);
                assert!(meshlet.triangles.len() <= MAX_MESHLET_TRIANGLES);
                assert!(!meshlet.triangles.is_empty());
                covered += meshlet.triangles.len();
            }
            assert_eq!(covered, tri_count);
        }
        let (positions, indices) = small_mesh();
        let first = build_meshlets(&positions, &indices).unwrap();
        let second = build_meshlets(&positions, &indices).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn malformed_input_is_refused() {
        let (positions, _) = small_mesh();
        assert_eq!(build_meshlets(&[], &[]), Err(MeshletError::EmptyMesh));
        assert_eq!(
            build_meshlets(&positions, &[0, 1]),
            Err(MeshletError::UnalignedIndices { len: 2 })
        );
        assert_eq!(
            build_meshlets(&positions, &[0, 1, 9]),
            Err(MeshletError::IndexOutOfRange(9))
        );
        let mut bad = positions.clone();
        bad[0][0] = f32::NAN;
        assert_eq!(
            build_meshlets(&bad, &[0, 1, 2]),
            Err(MeshletError::NonFinite("positions"))
        );
        assert_eq!(build_meshlets_naive(&[], &[]), Err(MeshletError::EmptyMesh));
    }
}
