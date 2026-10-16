//! Attribute-aware mesh simplification with a seam lock, on `meshopt` (meshoptimizer, MIT).
//!
//! The engine's own quadric chain averages UVs when it merges a vertex, so a collapse on a
//! texture seam drags the seam. Here meshoptimizer decides which edges collapse (error metric
//! over position, normal, UV and any caller channel such as skin weights) and every vertex that
//! sits on an attribute seam is locked, so no seam vertex is removed or moved: the output is a
//! sub-selection of the input vertices, bit for bit.

use std::collections::BTreeMap;

use meshopt::{simplify_with_attributes_and_locks, SimplifyOptions, VertexDataAdapter};

use crate::mesh::{ix, scaled_count, to_u32, GeoError, GeoMesh};

/// Options for one simplification.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodOptions {
    /// Fraction of the triangles to keep, `0.0..=1.0` (0.5 = half).
    pub target_ratio: f32,
    /// Largest accepted error relative to the mesh extent (meshoptimizer `target_error`).
    pub max_error: f32,
    /// Lock vertices where a position carries more than one UV.
    pub lock_uv_seams: bool,
    /// Lock vertices where a position carries more than one normal (hard edges).
    pub lock_normal_seams: bool,
    /// Lock vertices on the open border of the mesh.
    pub lock_border: bool,
    /// Weight of the normal in the error metric.
    pub normal_weight: f32,
    /// Weight of the UV in the error metric.
    pub uv_weight: f32,
}

impl Default for LodOptions {
    fn default() -> Self {
        Self {
            target_ratio: 0.5,
            max_error: 0.25,
            lock_uv_seams: true,
            lock_normal_seams: false,
            lock_border: true,
            normal_weight: 0.5,
            uv_weight: 1.0,
        }
    }
}

/// One simplified level.
#[derive(Clone, Debug, PartialEq)]
pub struct LodResult {
    /// The simplified mesh; vertices are the referenced subset of the input, input order kept.
    pub mesh: GeoMesh,
    /// For each output vertex the input vertex it came from.
    pub source_vertex: Vec<u32>,
    /// Triangles in the input.
    pub triangles_before: usize,
    /// Triangles in the output.
    pub triangles_after: usize,
    /// Relative error meshoptimizer reports for the result.
    pub error: f32,
    /// Input vertices that were locked as seam vertices.
    pub locked_vertices: usize,
}

/// Input vertices on an attribute seam: their position is shared by another vertex that has a
/// different UV (`uv`) or a different normal (`normal`).
#[must_use]
pub fn seam_vertices(mesh: &GeoMesh, uv: bool, normal: bool) -> Vec<bool> {
    let weld = mesh.weld_map();
    // Per position group: does any member differ from the group's first member?
    let mut differs: BTreeMap<u32, bool> = BTreeMap::new();
    for (i, &root) in weld.iter().enumerate() {
        let r = ix(root);
        if r == i {
            continue;
        }
        let mut d = false;
        if uv && !mesh.uvs.is_empty() {
            d |= mesh.uvs[i].map(f32::to_bits) != mesh.uvs[r].map(f32::to_bits);
        }
        if normal && !mesh.normals.is_empty() {
            d |= mesh.normals[i].map(f32::to_bits) != mesh.normals[r].map(f32::to_bits);
        }
        if d {
            differs.insert(root, true);
        }
    }
    weld.iter()
        .map(|root| differs.get(root).copied().unwrap_or(false))
        .collect()
}

/// Vertices that touch a boundary edge (an edge used by one triangle, after welding).
fn border_vertices(mesh: &GeoMesh) -> Vec<bool> {
    let weld = mesh.weld_map();
    let mut directed: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for tri in mesh.indices.chunks_exact(3) {
        for k in 0..3 {
            let a = weld[ix(tri[k])];
            let b = weld[ix(tri[(k + 1) % 3])];
            *directed.entry((a, b)).or_insert(0) += 1;
        }
    }
    let mut border_roots = BTreeMap::new();
    for &(a, b) in directed.keys() {
        if !directed.contains_key(&(b, a)) {
            border_roots.insert(a, ());
            border_roots.insert(b, ());
        }
    }
    weld.iter().map(|r| border_roots.contains_key(r)).collect()
}

/// Vertices the simplifier may not touch: attribute seams and, if asked, open borders.
fn lock_flags(mesh: &GeoMesh, options: &LodOptions) -> Vec<bool> {
    let mut locked = vec![false; mesh.positions.len()];
    if options.lock_uv_seams || options.lock_normal_seams {
        let seams = seam_vertices(mesh, options.lock_uv_seams, options.lock_normal_seams);
        for (l, s) in locked.iter_mut().zip(seams) {
            *l |= s;
        }
    }
    if options.lock_border {
        for (l, b) in locked.iter_mut().zip(border_vertices(mesh)) {
            *l |= b;
        }
    }
    locked
}

/// Interleaved attribute floats per vertex (normal, UV, caller channel) and the weight of each
/// float in the error metric.
fn attribute_buffers(
    mesh: &GeoMesh,
    extra: &[f32],
    extra_stride: usize,
    extra_weight: f32,
    options: &LodOptions,
) -> (Vec<f32>, Vec<f32>) {
    let has_normals = !mesh.normals.is_empty();
    let has_uvs = !mesh.uvs.is_empty();
    let mut attributes = Vec::new();
    let mut weights = Vec::new();
    if has_normals {
        weights.extend([options.normal_weight; 3]);
    }
    if has_uvs {
        weights.extend([options.uv_weight; 2]);
    }
    weights.extend(std::iter::repeat_n(extra_weight, extra_stride));
    for i in 0..mesh.positions.len() {
        if has_normals {
            attributes.extend_from_slice(&mesh.normals[i]);
        }
        if has_uvs {
            attributes.extend_from_slice(&mesh.uvs[i]);
        }
        if extra_stride > 0 {
            attributes.extend_from_slice(&extra[i * extra_stride..(i + 1) * extra_stride]);
        }
    }
    (attributes, weights)
}

/// Keep the referenced vertices in input order; returns the mesh and each vertex's source.
fn compact(mesh: &GeoMesh, simplified: &[u32]) -> (GeoMesh, Vec<u32>) {
    let n = mesh.positions.len();
    let mut used = vec![false; n];
    for &i in simplified {
        used[ix(i)] = true;
    }
    let mut remap = vec![u32::MAX; n];
    let mut out = GeoMesh::default();
    let mut source_vertex = Vec::new();
    for i in (0..n).filter(|&i| used[i]) {
        remap[i] = to_u32(out.positions.len());
        out.positions.push(mesh.positions[i]);
        if !mesh.normals.is_empty() {
            out.normals.push(mesh.normals[i]);
        }
        if !mesh.uvs.is_empty() {
            out.uvs.push(mesh.uvs[i]);
        }
        source_vertex.push(to_u32(i));
    }
    out.indices = simplified.iter().map(|&i| remap[ix(i)]).collect();
    (out, source_vertex)
}

/// Simplify to `options.target_ratio` of the triangles, keeping seams.
///
/// `extra` is an optional per-vertex channel (skin weights, vertex colors) of `extra_stride`
/// floats per vertex, scored with `extra_weight` per component. The result may keep more
/// triangles than asked when locked vertices or the error cap stop the collapse earlier.
pub fn simplify(
    mesh: &GeoMesh,
    extra: &[f32],
    extra_stride: usize,
    extra_weight: f32,
    options: &LodOptions,
) -> Result<LodResult, GeoError> {
    mesh.validate()?;
    if mesh.indices.is_empty() {
        return Err(GeoError::Empty);
    }
    let n = mesh.positions.len();
    if extra_stride > 0 && extra.len() != extra_stride * n {
        return Err(GeoError::Malformed("extra channel length"));
    }
    let ratio = options.target_ratio.clamp(0.0, 1.0);

    let locked = lock_flags(mesh, options);
    let locked_vertices = locked.iter().filter(|&&l| l).count();

    // Interleaved positions for the adapter, attributes in their own float array.
    let position_bytes: Vec<u8> = mesh
        .positions
        .iter()
        .flatten()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    let adapter = VertexDataAdapter::new(&position_bytes, 12, 0)
        .map_err(|e| GeoError::Donor(e.to_string()))?;
    let (attributes, weights) = attribute_buffers(mesh, extra, extra_stride, extra_weight, options);
    let stride = weights.len();

    let target_count = scaled_count(mesh.indices.len(), ratio) / 3 * 3;
    let mut error = 0.0_f32;
    let simplified = simplify_with_attributes_and_locks(
        &mesh.indices,
        &adapter,
        &attributes,
        &weights,
        stride * std::mem::size_of::<f32>(),
        &locked,
        target_count,
        options.max_error,
        SimplifyOptions::None,
        Some(&mut error),
    );

    let (out, source_vertex) = compact(mesh, &simplified);
    Ok(LodResult {
        triangles_after: out.triangle_count(),
        triangles_before: mesh.triangle_count(),
        mesh: out,
        source_vertex,
        error,
        locked_vertices,
    })
}

/// A LOD chain: `ratios` applied to the original mesh each time (not to the previous level),
/// so level k carries its own error and the chain never compounds.
pub fn chain(
    mesh: &GeoMesh,
    ratios: &[f32],
    options: &LodOptions,
) -> Result<Vec<LodResult>, GeoError> {
    ratios
        .iter()
        .map(|&r| {
            simplify(
                mesh,
                &[],
                0,
                0.0,
                &LodOptions {
                    target_ratio: r,
                    ..*options
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid_faces, uv_sphere};

    /// `(position bits, uv bits)` of every seam vertex of `mesh`.
    fn seam_keys(mesh: &GeoMesh) -> Vec<([u32; 3], [u32; 2])> {
        let seams = seam_vertices(mesh, true, false);
        let mut keys: Vec<_> = (0..mesh.positions.len())
            .filter(|&i| seams[i])
            .map(|i| {
                (
                    mesh.positions[i].map(f32::to_bits),
                    mesh.uvs[i].map(f32::to_bits),
                )
            })
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    #[test]
    fn sphere_halves_with_zero_seam_vertices_moved() {
        let sphere = uv_sphere(1.0, 24, 32);
        let seams_in = seam_keys(&sphere);
        assert!(!seams_in.is_empty(), "the seam column must exist");
        let result = simplify(&sphere, &[], 0, 0.0, &LodOptions::default()).unwrap();
        let before = result.triangles_before;
        assert!(
            result.triangles_after <= before * 55 / 100,
            "{} of {} triangles left",
            result.triangles_after,
            before
        );
        result.mesh.validate().unwrap();
        assert_eq!(result.mesh.open_edge_count(), 0, "still closed");
        // Every seam vertex of the input is still there with the same position and UV.
        let seams_out = seam_keys(&result.mesh);
        let moved = seams_in.iter().filter(|k| !seams_out.contains(k)).count();
        assert_eq!(moved, 0, "seam vertices moved");
        // No vertex invented: each output vertex equals its source vertex bit for bit.
        for (o, &s) in result.source_vertex.iter().enumerate() {
            assert_eq!(result.mesh.positions[o], sphere.positions[s as usize]);
            assert_eq!(result.mesh.uvs[o], sphere.uvs[s as usize]);
        }
    }

    #[test]
    fn unlocked_simplification_does_drop_seam_vertices() {
        // The control: with the lock off the same cut loses seam vertices, which is the defect
        // the lock removes.
        let sphere = uv_sphere(1.0, 24, 32);
        let seams_in = seam_keys(&sphere);
        let options = LodOptions {
            lock_uv_seams: false,
            lock_border: false,
            ..LodOptions::default()
        };
        let result = simplify(&sphere, &[], 0, 0.0, &options).unwrap();
        let seams_out = seam_keys(&result.mesh);
        let moved = seams_in.iter().filter(|k| !seams_out.contains(k)).count();
        assert!(moved > 0, "the control run lost no seam vertex");
    }

    #[test]
    fn result_repeats_and_chain_shrinks() {
        let sphere = uv_sphere(1.0, 20, 28);
        let a = chain(&sphere, &[0.5, 0.25], &LodOptions::default()).unwrap();
        let b = chain(&sphere, &[0.5, 0.25], &LodOptions::default()).unwrap();
        assert_eq!(a, b);
        assert!(a[1].triangles_after <= a[0].triangles_after);
        assert!(a[0].mesh.digest() == b[0].mesh.digest());
    }

    #[test]
    fn extra_channel_length_is_checked_and_weights_are_carried() {
        let sphere = uv_sphere(1.0, 12, 16);
        let n = sphere.positions.len();
        assert_eq!(
            simplify(&sphere, &[0.0; 3], 4, 1.0, &LodOptions::default()),
            Err(GeoError::Malformed("extra channel length"))
        );
        // A weight channel (4 floats per vertex) rides along and the result still repeats.
        let extra: Vec<f32> = (0..n * 4).map(|i| (i % 4) as f32 * 0.25).collect();
        let r1 = simplify(&sphere, &extra, 4, 2.0, &LodOptions::default()).unwrap();
        let r2 = simplify(&sphere, &extra, 4, 2.0, &LodOptions::default()).unwrap();
        assert_eq!(r1, r2);
        assert!(r1.triangles_after < r1.triangles_before);
    }

    #[test]
    fn a_box_with_every_edge_a_seam_cannot_shrink_but_stays_intact() {
        let b = cuboid_faces([1.0; 3]);
        let r = simplify(&b, &[], 0, 0.0, &LodOptions::default()).unwrap();
        assert_eq!(r.triangles_after, r.triangles_before);
        assert_eq!(r.mesh, b);
    }
}
