//! Native mesh ops: weld, mirror, array, flip, normals (std only, no feature).
//!
//! Owner: `engine-geo`; consumer: the door `geo` pack (`crates/endpoint`).
//! Units: meters; angles are windings, not degrees. Determinism: pure
//! functions of the input mesh and options (BTreeMap order, fixed loop
//! order), so a cook run twice gives the same bytes; receipts are
//! `GeoMesh::digest`. Errors: none; indices must be in range (see
//! `GeoMesh::validate`), out-of-range panics like `volume`.

use std::collections::BTreeMap;

use crate::mesh::{ix, narrow, to_u32, GeoMesh};

/// Mirror plane through the origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Reflect x.
    X,
    /// Reflect y.
    Y,
    /// Reflect z.
    Z,
}

impl Axis {
    /// Lane index of the axis.
    #[must_use]
    pub const fn lane(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }
}

/// Grid cell of a position at spacing `eps`.
fn cell_key(p: [f32; 3], eps: f64) -> [u32; 3] {
    let snap = |v: f32| narrow((f64::from(v) / eps).round() * eps).to_bits();
    [snap(p[0]), snap(p[1]), snap(p[2])]
}

/// Merge vertices sharing a quantized grid cell (`epsilon` spacing) to the
/// lowest index; degenerate triangles are dropped. Normals and UVs come from
/// the kept vertex. A non-positive or non-finite `epsilon` welds exact
/// duplicates only.
#[must_use]
pub fn weld_by_distance(mesh: &GeoMesh, epsilon: f32) -> GeoMesh {
    let count = mesh.positions.len();
    let has_normals = mesh.normals.len() == count;
    let has_uvs = mesh.uvs.len() == count;
    let eps = f64::from(epsilon);
    let quantized = eps.is_finite() && eps > 0.0;
    let mut first: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut exact: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut remap = vec![0_u32; count];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (i, p) in mesh.positions.iter().enumerate() {
        let root = if quantized {
            *first.entry(cell_key(*p, eps)).or_insert_with(|| to_u32(i))
        } else {
            *exact
                .entry(p.map(f32::to_bits))
                .or_insert_with(|| to_u32(i))
        };
        if ix(root) == i {
            remap[i] = to_u32(positions.len());
            positions.push(*p);
            if has_normals {
                normals.push(mesh.normals[i]);
            }
            if has_uvs {
                uvs.push(mesh.uvs[i]);
            }
        } else {
            remap[i] = remap[ix(root)];
        }
    }
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (remap[ix(tri[0])], remap[ix(tri[1])], remap[ix(tri[2])]);
        if a == b || b == c || a == c {
            continue;
        }
        indices.extend_from_slice(&[a, b, c]);
    }
    GeoMesh {
        positions,
        normals,
        uvs,
        indices,
    }
}

/// The mesh plus its reflection through the origin plane of `axis`.
/// Reflected windings are reversed; vertices within 1e-6 of the plane are
/// snapped to 0 and merged, and triangles with all three corners on the
/// plane are dropped from both halves.
#[must_use]
pub fn mirror(mesh: &GeoMesh, axis: Axis) -> GeoMesh {
    let lane = axis.lane();
    let count = mesh.positions.len();
    let has_normals = mesh.normals.len() == count;
    let has_uvs = mesh.uvs.len() == count;
    let mut combined = Vec::with_capacity(2 * count);
    combined.extend_from_slice(&mesh.positions);
    for p in &mesh.positions {
        let mut q = *p;
        q[lane] = -q[lane];
        combined.push(q);
    }
    let mut combined_normals = Vec::new();
    if has_normals {
        combined_normals.extend_from_slice(&mesh.normals);
        for v in &mesh.normals {
            let mut q = *v;
            q[lane] = -q[lane];
            combined_normals.push(q);
        }
    }
    let mut combined_uvs = Vec::new();
    if has_uvs {
        combined_uvs.extend_from_slice(&mesh.uvs);
        combined_uvs.extend_from_slice(&mesh.uvs);
    }
    for p in &mut combined {
        if p[lane].abs() <= 1e-6 {
            p[lane] = 0.0;
        }
    }
    let mut positions = Vec::with_capacity(combined.len());
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut remap = vec![0_u32; combined.len()];
    let mut seen: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    for (j, p) in combined.iter().enumerate() {
        if p[lane] == 0.0 {
            let key = p.map(f32::to_bits);
            if let Some(&id) = seen.get(&key) {
                remap[j] = id;
                continue;
            }
            let id = to_u32(positions.len());
            seen.insert(key, id);
            remap[j] = id;
            positions.push(*p);
            if has_normals {
                normals.push(combined_normals[j]);
            }
            if has_uvs {
                uvs.push(combined_uvs[j]);
            }
        } else {
            remap[j] = to_u32(positions.len());
            positions.push(*p);
            if has_normals {
                normals.push(combined_normals[j]);
            }
            if has_uvs {
                uvs.push(combined_uvs[j]);
            }
        }
    }
    let on_plane = |j: usize| combined[j][lane] == 0.0;
    let mut indices = Vec::with_capacity(mesh.indices.len() * 2);
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (ix(tri[0]), ix(tri[1]), ix(tri[2]));
        if !(on_plane(a) && on_plane(b) && on_plane(c)) {
            indices.extend_from_slice(&[remap[a], remap[b], remap[c]]);
        }
    }
    for tri in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (ix(tri[0]) + count, ix(tri[1]) + count, ix(tri[2]) + count);
        if !(on_plane(a) && on_plane(b) && on_plane(c)) {
            indices.extend_from_slice(&[remap[a], remap[c], remap[b]]);
        }
    }
    GeoMesh {
        positions,
        normals,
        uvs,
        indices,
    }
}

/// `count` copies (`1..=64`); copy `k` is moved by `k` times `offset`.
/// Normals are copied unchanged, UVs are duplicated.
#[must_use]
pub fn array(mesh: &GeoMesh, count: u32, offset: [f32; 3]) -> GeoMesh {
    let copies = usize::try_from(count).unwrap_or(0);
    if copies == 0 {
        return GeoMesh::default();
    }
    let count_verts = mesh.positions.len();
    let has_normals = mesh.normals.len() == count_verts;
    let has_uvs = mesh.uvs.len() == count_verts;
    let mut positions = Vec::with_capacity(count_verts * copies);
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    if has_normals {
        normals.reserve(count_verts * copies);
    }
    if has_uvs {
        uvs.reserve(count_verts * copies);
    }
    let mut indices = Vec::with_capacity(mesh.indices.len() * copies);
    for k in 0..count {
        let shift = [
            narrow(f64::from(k) * f64::from(offset[0])),
            narrow(f64::from(k) * f64::from(offset[1])),
            narrow(f64::from(k) * f64::from(offset[2])),
        ];
        for p in &mesh.positions {
            positions.push([p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]]);
        }
        if has_normals {
            normals.extend_from_slice(&mesh.normals);
        }
        if has_uvs {
            uvs.extend_from_slice(&mesh.uvs);
        }
        let base = to_u32(ix(k) * count_verts);
        for idx in &mesh.indices {
            indices.push(to_u32(ix(base) + ix(*idx)));
        }
    }
    GeoMesh {
        positions,
        normals,
        uvs,
        indices,
    }
}

/// Reverse every winding and negate the normals.
#[must_use]
pub fn flip(mesh: &GeoMesh) -> GeoMesh {
    let mut out = mesh.clone();
    for tri in out.indices.chunks_exact_mut(3) {
        tri.swap(1, 2);
    }
    if out.normals.len() == out.positions.len() {
        for v in &mut out.normals {
            v[0] = -v[0];
            v[1] = -v[1];
            v[2] = -v[2];
        }
    }
    out
}

/// Area-weighted face normals summed per vertex index, normalized.
/// Isolated vertices get `+y`. Positions, UVs and indices are unchanged.
#[must_use]
pub fn recompute_normals(mesh: &GeoMesh) -> GeoMesh {
    let count = mesh.positions.len();
    let mut acc = vec![[0.0_f64; 3]; count];
    for tri in mesh.indices.chunks_exact(3) {
        let (ai, bi, ci) = (ix(tri[0]), ix(tri[1]), ix(tri[2]));
        let a = mesh.positions[ai].map(f64::from);
        let b = mesh.positions[bi].map(f64::from);
        let c = mesh.positions[ci].map(f64::from);
        let edge1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let edge2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let face = [
            edge1[1] * edge2[2] - edge1[2] * edge2[1],
            edge1[2] * edge2[0] - edge1[0] * edge2[2],
            edge1[0] * edge2[1] - edge1[1] * edge2[0],
        ];
        for idx in [ai, bi, ci] {
            acc[idx][0] += face[0];
            acc[idx][1] += face[1];
            acc[idx][2] += face[2];
        }
    }
    let mut normals = Vec::with_capacity(count);
    for v in &acc {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len > 0.0 {
            normals.push([narrow(v[0] / len), narrow(v[1] / len), narrow(v[2] / len)]);
        } else {
            normals.push([0.0, 1.0, 0.0]);
        }
    }
    GeoMesh {
        positions: mesh.positions.clone(),
        normals,
        uvs: if mesh.uvs.len() == count {
            mesh.uvs.clone()
        } else {
            Vec::new()
        },
        indices: mesh.indices.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid, cuboid_faces, uv_sphere};

    #[test]
    fn weld_merges_split_cube() {
        let welded = weld_by_distance(&cuboid_faces([1.0; 3]), 1e-5);
        assert_eq!(welded.positions.len(), 8, "verts");
        assert_eq!(welded.triangle_count(), 12, "tris");
        assert!(welded.edge_report().is_closed(), "closed");
    }

    #[test]
    fn mirror_doubles_volume() {
        let half = cuboid([1.0, 2.0, 2.0]).placed([1.0; 3], 0, [0.5, 0.0, 0.0]);
        let doubled = mirror(&half, Axis::X);
        assert!(doubled.edge_report().is_closed(), "closed");
        assert!(
            (doubled.volume() - 2.0 * half.volume()).abs() < 1e-6,
            "volume {} vs {}",
            doubled.volume(),
            half.volume()
        );
        assert_eq!(doubled.triangle_count(), 20, "tris");
        assert_eq!(doubled.positions.len(), 12, "verts");
    }

    #[test]
    fn array_three_cubes() {
        let cube = cuboid([2.0, 2.0, 2.0]);
        let three = array(&cube, 3, [3.0, 0.0, 0.0]);
        assert!(
            (three.volume() - 3.0 * cube.volume()).abs() < 1e-6,
            "volume {}",
            three.volume()
        );
        assert_eq!(three.triangle_count(), 36, "tris");
        let (_, hi) = three.bounds();
        assert!((hi[0] - 7.0).abs() < 1e-6, "max x {}", hi[0]);
    }

    #[test]
    fn flip_twice_is_identity() {
        let cube = cuboid_faces([2.0, 1.0, 1.0]);
        assert_eq!(flip(&flip(&cube)).digest(), cube.digest());
    }

    #[test]
    fn normals_unit_and_outward() {
        let sphere = uv_sphere(1.0, 16, 24);
        let fixed = recompute_normals(&sphere);
        let last = fixed.positions.len() - 1;
        for (i, (n, p)) in fixed.normals.iter().zip(&fixed.positions).enumerate() {
            if i == 0 || i == last {
                continue;
            }
            let len = (f64::from(n[0]) * f64::from(n[0])
                + f64::from(n[1]) * f64::from(n[1])
                + f64::from(n[2]) * f64::from(n[2]))
            .sqrt();
            assert!((len - 1.0).abs() < 1e-5, "unit {i}: {len}");
            let dot = f64::from(n[0]) * f64::from(p[0])
                + f64::from(n[1]) * f64::from(p[1])
                + f64::from(n[2]) * f64::from(p[2]);
            assert!(dot >= 0.98, "outward {i}: {dot}");
        }
    }

    #[test]
    fn ops_repeat() {
        let cube = cuboid([2.0, 2.0, 2.0]);
        let half = cuboid([1.0, 2.0, 2.0]).placed([1.0; 3], 0, [0.5, 0.0, 0.0]);
        let sphere = uv_sphere(1.0, 16, 24);
        assert_eq!(
            weld_by_distance(&cuboid_faces([1.0; 3]), 1e-5).digest(),
            weld_by_distance(&cuboid_faces([1.0; 3]), 1e-5).digest()
        );
        assert_eq!(
            mirror(&half, Axis::X).digest(),
            mirror(&half, Axis::X).digest()
        );
        assert_eq!(
            array(&cube, 3, [3.0, 0.0, 0.0]).digest(),
            array(&cube, 3, [3.0, 0.0, 0.0]).digest()
        );
        assert_eq!(flip(&cube).digest(), flip(&cube).digest());
        assert_eq!(
            recompute_normals(&sphere).digest(),
            recompute_normals(&sphere).digest()
        );
    }
}
