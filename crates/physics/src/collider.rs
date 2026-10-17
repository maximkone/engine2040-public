//! collider: static collision shapes and the reader for cooked collider bytes.
//!
//! Consumer: the arena kit zone builds its capsule footprints from these shapes,
//! and the door's cast verbs ray- and sphere-cast against them ([`crate::query`]).
//! Data owner: the asset cook (`engine-assets`, `collider_cook`) writes the bytes;
//! this module reads them, so the physics lane never links the importer. Units:
//! meters, Y up; one layer integer per collider ([`crate::layers`]).
//! Determinism: the same bytes decode to the same shapes, no clock, no hashing.
//! Errors: [`ColliderError`] names the first defect; a defective blob yields no
//! shapes at all, never a partial set. Budget: decode runs once at load.
//!
//! Wire format v1, little-endian:
//!
//! ```text
//! "E2CC" | u16 version | u16 count
//! count x ( u8 name_len | name | u32 layer | u8 kind
//!   kind 0 hull: u16 points | points x 3 f32 | u16 faces | faces x 3 u16
//!   kind 1 mesh: u32 verts  | verts  x 3 f32 | u32 indices | indices x u32 )
//! ```
//!
//! A hull stores its extreme points and outward-wound triangle faces; a mesh stores
//! welded vertices and triangle indices. Both are read as triangle soups by every
//! query, so the two kinds share one code path.

use core::fmt;

use crate::layers;

/// First four bytes of a cooked collider blob.
pub const MAGIC: [u8; 4] = *b"E2CC";
/// Format version this reader accepts.
pub const VERSION: u16 = 1;
/// Most colliders in one blob.
pub const MAX_COLLIDERS: usize = 256;
/// Longest collider id in bytes.
pub const MAX_NAME_BYTES: usize = 64;
/// Most points one hull keeps.
pub const MAX_HULL_POINTS: usize = 64;
/// Most vertices one mesh keeps.
pub const MAX_MESH_VERTS: usize = 65_536;
/// Most triangles one mesh keeps.
pub const MAX_MESH_TRIS: usize = 20_000;

const KIND_HULL: u8 = 0;
const KIND_MESH: u8 = 1;

/// One static collision shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// Convex hull: extreme points plus outward-wound triangle faces.
    Hull {
        /// Hull points in meters.
        points: Vec<[f32; 3]>,
        /// Triangle faces as indices into `points`.
        faces: Vec<[u16; 3]>,
    },
    /// Triangle mesh: welded vertices plus triangle indices.
    Mesh {
        /// Vertices in meters.
        vertices: Vec<[f32; 3]>,
        /// Flat triangle indices, three per triangle.
        indices: Vec<u32>,
    },
}

/// One cooked collider: an id, one layer bit and a shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Collider {
    /// Collider id (the art node name).
    pub id: String,
    /// Exactly one layer bit from [`crate::layers`].
    pub layer: u32,
    /// The shape.
    pub shape: Shape,
}

/// Triangle iterator over a shape; panic-free even when a hand-built shape holds a
/// bad index (that triangle is skipped).
pub struct Triangles<'a> {
    points: &'a [[f32; 3]],
    source: Source<'a>,
}

enum Source<'a> {
    Faces(core::slice::Iter<'a, [u16; 3]>),
    Indices(core::slice::ChunksExact<'a, u32>),
}

impl Iterator for Triangles<'_> {
    type Item = [[f32; 3]; 3];

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let ids: [usize; 3] = match &mut self.source {
                Source::Faces(faces) => faces.next().map(|f| f.map(usize::from))?,
                Source::Indices(chunks) => {
                    let c = chunks.next()?;
                    [c[0], c[1], c[2]].map(|i| usize::try_from(i).unwrap_or(usize::MAX))
                }
            };
            if let (Some(a), Some(b), Some(c)) = (
                self.points.get(ids[0]),
                self.points.get(ids[1]),
                self.points.get(ids[2]),
            ) {
                return Some([*a, *b, *c]);
            }
        }
    }
}

impl Shape {
    /// The shape as triangles.
    #[must_use]
    pub fn triangles(&self) -> Triangles<'_> {
        match self {
            Self::Hull { points, faces } => Triangles {
                points,
                source: Source::Faces(faces.iter()),
            },
            Self::Mesh { vertices, indices } => Triangles {
                points: vertices,
                source: Source::Indices(indices.chunks_exact(3)),
            },
        }
    }

    /// Axis-aligned bounds `[min, max]`, or `None` for a shape with no points.
    #[must_use]
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let points = match self {
            Self::Hull { points, .. } => points,
            Self::Mesh { vertices, .. } => vertices,
        };
        let first = *points.first()?;
        let mut lo = first;
        let mut hi = first;
        for p in points {
            for axis in 0..3 {
                lo[axis] = lo[axis].min(p[axis]);
                hi[axis] = hi[axis].max(p[axis]);
            }
        }
        Some((lo, hi))
    }
}

/// Why a cooked collider blob was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColliderError {
    /// The blob ended early; `want` bytes were needed from its start.
    Short {
        /// Bytes needed from the start.
        want: usize,
    },
    /// The first four bytes are not [`MAGIC`].
    BadMagic,
    /// The version is not [`VERSION`].
    BadVersion(u16),
    /// More colliders than [`MAX_COLLIDERS`].
    TooMany(usize),
    /// A collider id is empty, too long, or not UTF-8.
    BadName,
    /// Two colliders share an id.
    DuplicateName,
    /// The layer is not exactly one known layer bit.
    BadLayer(u32),
    /// The shape kind byte is unknown.
    BadKind(u8),
    /// A shape count or index is out of range; static reason.
    BadShape(&'static str),
    /// A coordinate is NaN or infinite.
    NonFinite,
    /// Bytes remain after the last collider.
    Trailing(usize),
}

impl fmt::Display for ColliderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { want } => write!(f, "collider blob ends early (needs {want} bytes)"),
            Self::BadMagic => write!(f, "collider blob does not start with E2CC"),
            Self::BadVersion(v) => write!(f, "collider blob version {v}, reader wants {VERSION}"),
            Self::TooMany(n) => write!(
                f,
                "collider blob holds {n} colliders, at most {MAX_COLLIDERS}"
            ),
            Self::BadName => write!(f, "collider id must be 1..={MAX_NAME_BYTES} bytes of UTF-8"),
            Self::DuplicateName => write!(f, "two colliders share an id"),
            Self::BadLayer(l) => write!(f, "collider layer {l} is not one known layer bit"),
            Self::BadKind(k) => write!(f, "collider kind {k} is unknown"),
            Self::BadShape(why) => write!(f, "collider shape: {why}"),
            Self::NonFinite => write!(f, "collider coordinate is not finite"),
            Self::Trailing(n) => write!(f, "{n} bytes follow the last collider"),
        }
    }
}

impl std::error::Error for ColliderError {}

/// Forward byte reader; a short read reports how many bytes the blob needed.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], ColliderError> {
        let end = self.pos.saturating_add(len);
        let out = self
            .bytes
            .get(self.pos..end)
            .ok_or(ColliderError::Short { want: end })?;
        self.pos = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ColliderError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, ColliderError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, ColliderError> {
        self.array().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, ColliderError> {
        self.array().map(u32::from_le_bytes)
    }

    fn f32(&mut self) -> Result<f32, ColliderError> {
        let value = self.array().map(f32::from_le_bytes)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(ColliderError::NonFinite)
        }
    }

    fn point(&mut self) -> Result<[f32; 3], ColliderError> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// `count` points, refusing a count the remaining bytes cannot hold before
    /// reserving anything.
    fn points(&mut self, count: usize) -> Result<Vec<[f32; 3]>, ColliderError> {
        if count.saturating_mul(12) > self.bytes.len().saturating_sub(self.pos) {
            return Err(ColliderError::Short {
                want: self.pos.saturating_add(count.saturating_mul(12)),
            });
        }
        (0..count).map(|_| self.point()).collect()
    }
}

/// Read a cooked collider blob into shapes, in blob order.
pub fn decode(bytes: &[u8]) -> Result<Vec<Collider>, ColliderError> {
    let mut r = Reader { bytes, pos: 0 };
    if r.array::<4>()? != MAGIC {
        return Err(ColliderError::BadMagic);
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(ColliderError::BadVersion(version));
    }
    let count = usize::from(r.u16()?);
    if count > MAX_COLLIDERS {
        return Err(ColliderError::TooMany(count));
    }
    let mut out: Vec<Collider> = Vec::with_capacity(count);
    for _ in 0..count {
        let collider = read_collider(&mut r)?;
        if out.iter().any(|c| c.id == collider.id) {
            return Err(ColliderError::DuplicateName);
        }
        out.push(collider);
    }
    match bytes.len() - r.pos {
        0 => Ok(out),
        extra => Err(ColliderError::Trailing(extra)),
    }
}

fn read_collider(r: &mut Reader<'_>) -> Result<Collider, ColliderError> {
    let name_len = usize::from(r.u8()?);
    if name_len == 0 || name_len > MAX_NAME_BYTES {
        return Err(ColliderError::BadName);
    }
    let id = core::str::from_utf8(r.take(name_len)?)
        .map_err(|_| ColliderError::BadName)?
        .to_owned();
    let layer = r.u32()?;
    if !layers::is_single(layer) {
        return Err(ColliderError::BadLayer(layer));
    }
    let shape = match r.u8()? {
        KIND_HULL => read_hull(r)?,
        KIND_MESH => read_mesh(r)?,
        other => return Err(ColliderError::BadKind(other)),
    };
    Ok(Collider { id, layer, shape })
}

fn read_hull(r: &mut Reader<'_>) -> Result<Shape, ColliderError> {
    let n_points = usize::from(r.u16()?);
    if !(4..=MAX_HULL_POINTS).contains(&n_points) {
        return Err(ColliderError::BadShape("hull point count outside 4..=64"));
    }
    let points = r.points(n_points)?;
    let n_faces = usize::from(r.u16()?);
    if !(4..=2 * MAX_HULL_POINTS).contains(&n_faces) {
        return Err(ColliderError::BadShape("hull face count outside 4..=128"));
    }
    let mut faces = Vec::with_capacity(n_faces);
    for _ in 0..n_faces {
        let face = [r.u16()?, r.u16()?, r.u16()?];
        if face.iter().any(|&i| usize::from(i) >= n_points) {
            return Err(ColliderError::BadShape("hull face index out of range"));
        }
        if face[0] == face[1] || face[1] == face[2] || face[0] == face[2] {
            return Err(ColliderError::BadShape("hull face repeats a point"));
        }
        faces.push(face);
    }
    Ok(Shape::Hull { points, faces })
}

fn read_mesh(r: &mut Reader<'_>) -> Result<Shape, ColliderError> {
    let n_verts = usize::try_from(r.u32()?).unwrap_or(usize::MAX);
    if !(3..=MAX_MESH_VERTS).contains(&n_verts) {
        return Err(ColliderError::BadShape(
            "mesh vertex count outside 3..=65536",
        ));
    }
    let vertices = r.points(n_verts)?;
    let n_indices = usize::try_from(r.u32()?).unwrap_or(usize::MAX);
    if n_indices == 0 || !n_indices.is_multiple_of(3) || n_indices > 3 * MAX_MESH_TRIS {
        return Err(ColliderError::BadShape(
            "mesh index count is not 3..=60000 in threes",
        ));
    }
    if n_indices.saturating_mul(4) > r.bytes.len().saturating_sub(r.pos) {
        return Err(ColliderError::Short {
            want: r.pos.saturating_add(n_indices.saturating_mul(4)),
        });
    }
    let mut indices = Vec::with_capacity(n_indices);
    for _ in 0..n_indices {
        let index = r.u32()?;
        if !usize::try_from(index).is_ok_and(|i| i < n_verts) {
            return Err(ColliderError::BadShape("mesh index out of range"));
        }
        indices.push(index);
    }
    Ok(Shape::Mesh { vertices, indices })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The unit cube `[0,1]^3` as a hull: eight points, twelve outward faces.
    pub(crate) fn cube_hull() -> Shape {
        let mut points = Vec::new();
        for x in [0.0f32, 1.0] {
            for y in [0.0f32, 1.0] {
                for z in [0.0f32, 1.0] {
                    points.push([x, y, z]);
                }
            }
        }
        // Point index = x*4 + y*2 + z.
        let faces = vec![
            [0, 1, 3],
            [0, 3, 2], // x = 0
            [4, 6, 7],
            [4, 7, 5], // x = 1
            [0, 4, 5],
            [0, 5, 1], // y = 0
            [2, 3, 7],
            [2, 7, 6], // y = 1
            [0, 2, 6],
            [0, 6, 4], // z = 0
            [1, 5, 7],
            [1, 7, 3], // z = 1
        ];
        Shape::Hull { points, faces }
    }

    /// Write shapes in the documented layout (the reader's mirror).
    pub(crate) fn encode(colliders: &[Collider]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&u16::try_from(colliders.len()).expect("few").to_le_bytes());
        for c in colliders {
            out.push(u8::try_from(c.id.len()).expect("short id"));
            out.extend_from_slice(c.id.as_bytes());
            out.extend_from_slice(&c.layer.to_le_bytes());
            match &c.shape {
                Shape::Hull { points, faces } => {
                    out.push(KIND_HULL);
                    out.extend_from_slice(&u16::try_from(points.len()).expect("few").to_le_bytes());
                    for v in points.iter().flatten() {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                    out.extend_from_slice(&u16::try_from(faces.len()).expect("few").to_le_bytes());
                    for i in faces.iter().flatten() {
                        out.extend_from_slice(&i.to_le_bytes());
                    }
                }
                Shape::Mesh { vertices, indices } => {
                    out.push(KIND_MESH);
                    out.extend_from_slice(
                        &u32::try_from(vertices.len()).expect("few").to_le_bytes(),
                    );
                    for v in vertices.iter().flatten() {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                    out.extend_from_slice(
                        &u32::try_from(indices.len()).expect("few").to_le_bytes(),
                    );
                    for i in indices {
                        out.extend_from_slice(&i.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    fn cube(id: &str, layer: u32) -> Collider {
        Collider {
            id: id.to_owned(),
            layer,
            shape: cube_hull(),
        }
    }

    fn quad_mesh() -> Shape {
        Shape::Mesh {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    #[test]
    fn a_hull_and_a_mesh_decode_back_to_what_was_written() {
        let set = vec![
            cube("COL_pillar", layers::WORLD),
            Collider {
                id: "COL_slab".to_owned(),
                layer: layers::PROP,
                shape: quad_mesh(),
            },
        ];
        let bytes = encode(&set);
        assert_eq!(decode(&bytes).expect("decodes"), set);
        assert_eq!(decode(&bytes), decode(&bytes), "decode is a pure read");
    }

    /// The cook's pinned bytes for one unit-cube hull (`engine-assets` `collider_cook`
    /// asserts its writer produces exactly this checked-in fixture): the reader and the
    /// writer agree on the layout through these bytes.
    const COOK_GOLDEN: &[u8] = include_bytes!("../../../content/physics/unit_cube_hull.e2cc");

    #[test]
    fn the_cooks_golden_bytes_decode_to_the_unit_cube() {
        assert_eq!(COOK_GOLDEN.len(), 193);
        let set = decode(COOK_GOLDEN).expect("the cook's bytes decode");
        assert_eq!(
            set,
            vec![Collider {
                id: "COL_box".to_owned(),
                layer: layers::PROP,
                shape: cube_hull(),
            }]
        );
        assert_eq!(
            encode(&set),
            COOK_GOLDEN,
            "the test writer mirrors the cook writer"
        );
    }

    #[test]
    fn triangles_walk_hull_faces_and_mesh_indices() {
        let hull = cube_hull();
        assert_eq!(hull.triangles().count(), 12);
        assert_eq!(quad_mesh().triangles().count(), 2);
        let first = hull.triangles().next().expect("a face");
        assert_eq!(first, [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 1.0]]);
        let (lo, hi) = hull.bounds().expect("points");
        assert_eq!((lo, hi), ([0.0; 3], [1.0; 3]));
        // A hand-built bad index is skipped, never a panic.
        let broken = Shape::Mesh {
            vertices: vec![[0.0; 3]; 3],
            indices: vec![0, 1, 9],
        };
        assert_eq!(broken.triangles().count(), 0);
    }

    #[test]
    fn every_defect_is_refused_by_name() {
        let good = encode(&[cube("COL_a", layers::WORLD)]);
        assert!(decode(&good).is_ok());
        let mut bad = good.clone();
        bad[0] = b'X';
        assert_eq!(decode(&bad), Err(ColliderError::BadMagic));
        let mut bad = good.clone();
        bad[4] = 9;
        assert_eq!(decode(&bad), Err(ColliderError::BadVersion(9)));
        assert!(matches!(
            decode(&good[..good.len() - 1]),
            Err(ColliderError::Short { .. })
        ));
        let mut bad = good.clone();
        bad.push(0);
        assert_eq!(decode(&bad), Err(ColliderError::Trailing(1)));
        // Layer bytes sit after magic(4) version(2) count(2) len(1) name(5).
        let mut bad = good.clone();
        bad[14..18].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(decode(&bad), Err(ColliderError::BadLayer(3)));
        let mut bad = good.clone();
        bad[18] = 7;
        assert_eq!(decode(&bad), Err(ColliderError::BadKind(7)));
        let nan = encode(&[Collider {
            id: "COL_n".to_owned(),
            layer: layers::WORLD,
            shape: Shape::Hull {
                points: vec![
                    [f32::NAN, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 0.0, 1.0],
                ],
                faces: vec![[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]],
            },
        }]);
        assert_eq!(decode(&nan), Err(ColliderError::NonFinite));
        let dup = encode(&[cube("COL_a", layers::WORLD), cube("COL_a", layers::PROP)]);
        assert_eq!(decode(&dup), Err(ColliderError::DuplicateName));
    }

    #[test]
    fn a_lying_count_cannot_reserve_memory() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.push(1);
        bytes.push(b'a');
        bytes.extend_from_slice(&layers::WORLD.to_le_bytes());
        bytes.push(KIND_MESH);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&bytes), Err(ColliderError::BadShape(_))));
        let mut bytes = bytes[..bytes.len() - 4].to_vec();
        bytes.extend_from_slice(&60_000u32.to_le_bytes());
        assert!(matches!(decode(&bytes), Err(ColliderError::Short { .. })));
    }

    #[test]
    fn bad_hull_and_mesh_indices_are_refused() {
        let mut hull = cube_hull();
        if let Shape::Hull { faces, .. } = &mut hull {
            faces[3] = [4, 7, 8];
        }
        let bytes = encode(&[Collider {
            id: "COL_h".to_owned(),
            layer: layers::WORLD,
            shape: hull,
        }]);
        assert_eq!(
            decode(&bytes),
            Err(ColliderError::BadShape("hull face index out of range"))
        );
        let mesh = Shape::Mesh {
            vertices: vec![[0.0; 3]; 3],
            indices: vec![0, 1, 3],
        };
        let bytes = encode(&[Collider {
            id: "COL_m".to_owned(),
            layer: layers::WORLD,
            shape: mesh,
        }]);
        assert_eq!(
            decode(&bytes),
            Err(ColliderError::BadShape("mesh index out of range"))
        );
    }
}
