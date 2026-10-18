//! The one mesh type the geo tools share: indexed triangles with optional normals and UV0.
//!
//! Orientation: counter-clockwise triangles seen from outside. Vertices may be split (a UV
//! seam repeats a position with another UV); topology questions weld by exact position bits, so
//! a seam never counts as an open edge.

use std::collections::BTreeMap;
use std::fmt;

use engine_core::Fnv64;

/// Why a geo tool refused its input or failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeoError {
    /// Attribute arrays differ in length, an index is out of range, or the index count is not a
    /// multiple of 3.
    Malformed(&'static str),
    /// A position, normal or UV is NaN or infinite.
    NonFinite,
    /// The tool needs a closed (watertight) input and got open edges.
    NotClosed,
    /// A donor library returned an error (text kept short).
    Donor(String),
    /// The input has no triangles.
    Empty,
}

impl fmt::Display for GeoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "geo: malformed mesh: {why}"),
            Self::NonFinite => write!(f, "geo: mesh has a non-finite value"),
            Self::NotClosed => write!(f, "geo: mesh is not closed"),
            Self::Donor(why) => write!(f, "geo: donor library: {why}"),
            Self::Empty => write!(f, "geo: mesh has no triangles"),
        }
    }
}

impl std::error::Error for GeoError {}

/// Edge census of a mesh after welding by position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeReport {
    /// Undirected edges.
    pub edges: usize,
    /// Edges that are not used by exactly one forward and one backward triangle side
    /// (boundary edges, non-manifold edges and flipped neighbours).
    pub open_edges: usize,
}

impl EdgeReport {
    /// True when every edge is shared by exactly two consistently oriented triangles.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.open_edges == 0 && self.edges > 0
    }
}

/// Indexed triangle mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeoMesh {
    /// Positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals, empty or one per position.
    pub normals: Vec<[f32; 3]>,
    /// First UV set, empty or one per position.
    pub uvs: Vec<[f32; 2]>,
    /// Three indices per triangle.
    pub indices: Vec<u32>,
}

/// A vertex index as a slice index.
pub(crate) fn ix(i: u32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}

/// A length or position as a vertex index (saturates, never wraps).
pub(crate) fn to_u32(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// A length as the 64-bit word a digest writes.
pub(crate) fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

// The one narrowing of an f64 result to the f32 a mesh stores.
pub(crate) use engine_core::checked::f32_from_f64 as narrow;

/// `i / n` as f64 (`n` of 0 counts as 1).
pub(crate) fn frac(i: usize, n: usize) -> f64 {
    f64::from(to_u32(i)) / f64::from(to_u32(n).max(1))
}

/// `count * ratio` rounded down (`ratio` in 0..=1).
#[cfg(any(feature = "lod", feature = "uv"))]
pub(crate) fn scaled_count(count: usize, ratio: f32) -> usize {
    let floor =
        engine_core::checked::saturating_f64_to_u64(f64::from(to_u32(count)) * f64::from(ratio));
    usize::try_from(floor).unwrap_or(usize::MAX)
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Area of the triangle `a b c`.
#[cfg(any(feature = "lod", feature = "uv"))]
pub(crate) fn triangle_area(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (u, v) = (sub3(b, a), sub3(c, a));
    let normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    0.5 * dot3(normal, normal).sqrt()
}

/// Squared distance from `p` to the closest point of the segment `a..b` (a zero-length
/// segment is the point `a`).
pub(crate) fn segment_distance_sq(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let (ab, ap) = (sub3(b, a), sub3(p, a));
    let len_sq = dot3(ab, ab);
    let t = if len_sq > 0.0 {
        (dot3(ap, ab) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (0..3).map(|k| (ap[k] - t * ab[k]).powi(2)).sum()
}

/// Squared distance from `p` to the triangle `a b c` (closest point by Voronoi region; a
/// triangle with no area is its three edges).
pub(crate) fn triangle_distance_sq(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let (ab, ac, ap) = (sub3(b, a), sub3(c, a), sub3(p, a));
    let (d1, d2) = (dot3(ab, ap), dot3(ac, ap));
    let (bp, cp) = (sub3(p, b), sub3(p, c));
    let (d3, d4) = (dot3(ab, bp), dot3(ac, bp));
    let (d5, d6) = (dot3(ab, cp), dot3(ac, cp));
    let (va, vb, vc) = (d3 * d6 - d5 * d4, d5 * d2 - d1 * d6, d1 * d4 - d3 * d2);
    let sum = va + vb + vc;
    // Inside the face: the plane distance. Everywhere else, the nearest edge covers it.
    if va >= 0.0 && vb >= 0.0 && vc >= 0.0 && sum > 0.0 {
        let (v, w) = (vb / sum, vc / sum);
        let q = [0, 1, 2].map(|k| a[k] + ab[k] * v + ac[k] * w);
        return dot3(sub3(p, q), sub3(p, q));
    }
    segment_distance_sq(p, a, b)
        .min(segment_distance_sq(p, b, c))
        .min(segment_distance_sq(p, c, a))
}

impl GeoMesh {
    /// Distance from `p` to the nearest triangle, by brute force over every triangle; infinity
    /// when the mesh has none. Indices must be in range (`validate`).
    #[must_use]
    pub fn surface_distance(&self, p: [f64; 3]) -> f64 {
        let corner = |i: u32| self.positions[ix(i)].map(f64::from);
        self.indices
            .chunks_exact(3)
            .map(|t| triangle_distance_sq(p, corner(t[0]), corner(t[1]), corner(t[2])))
            .fold(f64::INFINITY, f64::min)
            .sqrt()
    }

    /// Positions and indices only.
    #[must_use]
    pub fn from_positions(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> Self {
        Self {
            positions,
            normals: Vec::new(),
            uvs: Vec::new(),
            indices,
        }
    }

    /// Triangle count.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Check lengths, index range and finiteness.
    pub fn validate(&self) -> Result<(), GeoError> {
        let n = self.positions.len();
        if !self.normals.is_empty() && self.normals.len() != n {
            return Err(GeoError::Malformed("normals length"));
        }
        if !self.uvs.is_empty() && self.uvs.len() != n {
            return Err(GeoError::Malformed("uvs length"));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(GeoError::Malformed("index count not a multiple of 3"));
        }
        if self.indices.iter().any(|&i| ix(i) >= n) {
            return Err(GeoError::Malformed("index out of range"));
        }
        let finite = self.positions.iter().flatten().all(|v| v.is_finite())
            && self.normals.iter().flatten().all(|v| v.is_finite())
            && self.uvs.iter().flatten().all(|v| v.is_finite());
        if !finite {
            return Err(GeoError::NonFinite);
        }
        Ok(())
    }

    /// For each vertex the lowest vertex index with the exact same position bits.
    #[must_use]
    pub fn weld_map(&self) -> Vec<u32> {
        let mut first: BTreeMap<[u32; 3], u32> = BTreeMap::new();
        self.positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
                *first.entry(key).or_insert_with(|| to_u32(i))
            })
            .collect()
    }

    /// Edge census after welding by position. A triangle with two welded corners equal counts
    /// its collapsed side as an open edge. Indices must be in range (`validate`).
    #[must_use]
    pub fn edge_report(&self) -> EdgeReport {
        let weld = self.weld_map();
        let mut directed: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for tri in self.indices.chunks_exact(3) {
            for k in 0..3 {
                let a = weld[ix(tri[k])];
                let b = weld[ix(tri[(k + 1) % 3])];
                *directed.entry((a, b)).or_insert(0) += 1;
            }
        }
        let mut report = EdgeReport::default();
        for (&(a, b), &forward) in &directed {
            // Visit each undirected edge once, from its smaller-first direction (or the only one).
            if a > b && directed.contains_key(&(b, a)) {
                continue;
            }
            report.edges += 1;
            let backward = directed.get(&(b, a)).copied().unwrap_or(0);
            if a == b || forward != 1 || backward != 1 {
                report.open_edges += 1;
            }
        }
        report
    }

    /// Edges not shared by exactly two consistent triangles (0 for a closed mesh).
    #[must_use]
    pub fn open_edge_count(&self) -> usize {
        self.edge_report().open_edges
    }

    /// Signed volume in cubic meters (positive for outward counter-clockwise triangles).
    #[must_use]
    pub fn volume(&self) -> f64 {
        let mut sum = 0.0_f64;
        for tri in self.indices.chunks_exact(3) {
            let p = |i: u32| self.positions[ix(i)].map(f64::from);
            let (a, b, c) = (p(tri[0]), p(tri[1]), p(tri[2]));
            sum += a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
        }
        sum / 6.0
    }

    /// Axis-aligned bounds `(min, max)`; zeros for an empty mesh.
    #[must_use]
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut it = self.positions.iter();
        let Some(first) = it.next() else {
            return ([0.0; 3], [0.0; 3]);
        };
        let (mut lo, mut hi) = (*first, *first);
        for p in it {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        (lo, hi)
    }

    /// Length of the bounding-box diagonal in meters; 0 for an empty mesh.
    #[must_use]
    pub fn diagonal(&self) -> f64 {
        let (lo, hi) = self.bounds();
        (0..3)
            .map(|k| (f64::from(hi[k]) - f64::from(lo[k])).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    /// FNV-1a 64 over every array, lengths first, floats as bits: the cook receipt of a result.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(to_u64(self.positions.len()));
        for p in &self.positions {
            for &v in p {
                h.write_f32(v);
            }
        }
        h.write_u64(to_u64(self.normals.len()));
        for p in &self.normals {
            for &v in p {
                h.write_f32(v);
            }
        }
        h.write_u64(to_u64(self.uvs.len()));
        for p in &self.uvs {
            for &v in p {
                h.write_f32(v);
            }
        }
        h.write_u64(to_u64(self.indices.len()));
        for &i in &self.indices {
            h.write_u32(i);
        }
        h.finish()
    }

    /// A copy with identical positions merged to one vertex; normals and UVs are dropped
    /// (a merged vertex has no single UV), the surface is unchanged.
    #[must_use]
    pub fn welded(&self) -> Self {
        let map = self.weld_map();
        let mut new_index: BTreeMap<u32, u32> = BTreeMap::new();
        let mut positions = Vec::new();
        for (i, &root) in map.iter().enumerate() {
            if ix(root) == i {
                new_index.insert(root, to_u32(positions.len()));
                positions.push(self.positions[i]);
            }
        }
        let indices = self
            .indices
            .iter()
            .map(|&i| new_index[&map[ix(i)]])
            .collect();
        Self::from_positions(positions, indices)
    }

    /// Scale, then 90-degree turns about Y (`quarter_turns_y`), then translate: exact on f32
    /// for the small integer-ish inputs the seeded tests use, and platform independent.
    #[must_use]
    pub fn placed(&self, scale: [f32; 3], quarter_turns_y: u32, offset: [f32; 3]) -> Self {
        let mut out = self.clone();
        for p in &mut out.positions {
            let mut v = [p[0] * scale[0], p[1] * scale[1], p[2] * scale[2]];
            for _ in 0..(quarter_turns_y % 4) {
                v = [v[2], v[1], -v[0]];
            }
            *p = [v[0] + offset[0], v[1] + offset[1], v[2] + offset[2]];
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [f64; 3] = [0.0, 0.0, 0.0];
    const B: [f64; 3] = [2.0, 0.0, 0.0];
    const C: [f64; 3] = [0.0, 2.0, 0.0];

    #[test]
    fn triangle_distance_covers_face_edge_and_corner_regions() {
        let d = |p| triangle_distance_sq(p, A, B, C).sqrt();
        // Above the face, and on it.
        assert!((d([0.5, 0.5, 3.0]) - 3.0).abs() < 1e-12);
        assert_eq!(d([0.5, 0.5, 0.0]), 0.0);
        // Beside the hypotenuse B-C: nearest point is its middle (1, 1, 0).
        assert!((d([2.0, 2.0, 0.0]) - 2.0_f64.sqrt()).abs() < 1e-12);
        // Past corner A: nearest point is A itself.
        assert!((d([-3.0, -4.0, 0.0]) - 5.0).abs() < 1e-12);
        // Past the end of edge A-B: nearest point is B.
        assert!((d([5.0, -4.0, 0.0]) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn a_triangle_with_no_area_is_its_edges() {
        // Three points on one line: distance to the segment from (0,0,0) to (2,0,0).
        let line = triangle_distance_sq([1.0, 3.0, 0.0], A, [1.0, 0.0, 0.0], B);
        assert!((line - 9.0).abs() < 1e-12);
        let point = triangle_distance_sq([3.0, 4.0, 0.0], A, A, A);
        assert!((point - 25.0).abs() < 1e-12);
    }

    #[test]
    fn surface_distance_takes_the_nearest_triangle() {
        let two = GeoMesh::from_positions(
            vec![
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [0.0, 2.0, 0.0],
                [0.0, 0.0, 5.0],
                [2.0, 0.0, 5.0],
                [0.0, 2.0, 5.0],
            ],
            vec![0, 1, 2, 3, 4, 5],
        );
        assert!((two.surface_distance([0.5, 0.5, 1.0]) - 1.0).abs() < 1e-12);
        assert!((two.surface_distance([0.5, 0.5, 4.0]) - 1.0).abs() < 1e-12);
        assert!(GeoMesh::default().surface_distance([0.0; 3]).is_infinite());
        assert!((two.diagonal() - (4.0_f64 + 4.0 + 25.0).sqrt()).abs() < 1e-12);
        assert_eq!(GeoMesh::default().diagonal(), 0.0);
    }
}
