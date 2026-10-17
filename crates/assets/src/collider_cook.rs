//! collider_cook: colliders cooked from the art, not from hand-placed boxes.
//!
//! Consumer: the arena kit zone and the door's cast verbs read the cooked bytes
//! through `engine-physics` (`collider::decode`); this module is the writer.
//! Data owner: `engine-assets`; the glTF `COL_` nodes own the geometry
//! ([`super::mesh_cook::COL_PREFIX`] convention). Units: meters, Y up; one layer
//! integer per collider, chosen by the caller (the layer table lives in
//! `engine-physics`, which this crate does not link). Determinism: `+ - * /` and
//! `sqrt` only, canonical point order, no hash maps, no clock; the same art gives
//! the same bytes, and a hull's bytes do not depend on input point order.
//! Errors: typed [`ColliderCookError`], never a panic.
//!
//! Two shapes are cooked. A dynamic prop becomes a convex hull: the extreme
//! points plus outward-wound triangle faces, capped at [`MAX_HULL_POINTS`] points
//! (a capped hull is inside the true hull; the cap keeps the farthest-spread
//! points). A static kit piece becomes a triangle mesh: vertices welded by exact
//! position, degenerate triangles dropped. Heightfields are not cooked: no kit
//! piece needs one yet.
//!
//! Wire format v1 (little-endian), read by `engine_physics::collider::decode`:
//!
//! ```text
//! "E2CC" | u16 version | u16 count
//! count x ( u8 name_len | name | u32 layer | u8 kind
//!   kind 0 hull: u16 points | points x 3 f32 | u16 faces | faces x 3 u16
//!   kind 1 mesh: u32 verts  | verts  x 3 f32 | u32 indices | indices x u32 )
//! ```

use super::wire::{push_u16, push_u32};
use core::fmt;
use std::collections::BTreeMap;

/// First four bytes of a cooked collider blob.
pub const MAGIC: [u8; 4] = *b"E2CC";
/// Format version written.
pub const VERSION: u16 = 1;
/// Most colliders in one blob.
pub const MAX_COLLIDERS: usize = 256;
/// Longest collider id in bytes.
pub const MAX_NAME_BYTES: usize = 64;
/// Most points one cooked hull keeps.
pub const MAX_HULL_POINTS: usize = 64;
/// Most vertices one cooked mesh keeps.
pub const MAX_MESH_VERTS: usize = 65_536;
/// Most triangles one cooked mesh keeps.
pub const MAX_MESH_TRIS: usize = 20_000;
/// Most input points one hull cook accepts.
pub const MAX_INPUT_POINTS: usize = 100_000;

/// Hull plane slack relative to the input's diagonal: a point this close to a
/// face plane counts as on it, so coplanar points never join the hull.
const PLANE_SLACK: f64 = 1.0e-7;

const KIND_HULL: u8 = 0;
const KIND_MESH: u8 = 1;

/// Why a collider cook was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColliderCookError {
    /// A coordinate is NaN or infinite.
    NonFinite,
    /// Fewer than four distinct points.
    TooFewPoints {
        /// Distinct points given.
        got: usize,
    },
    /// All points lie on one line or one plane: no solid to wrap.
    Flat,
    /// The hull cap is outside `4..=MAX_HULL_POINTS`.
    BadCap(usize),
    /// A mesh has no usable triangle or an index outside its vertices.
    BadMesh(&'static str),
    /// An input exceeds a cook bound.
    TooLarge {
        /// Which bound.
        what: &'static str,
        /// The offending count.
        count: usize,
    },
    /// A collider id is empty or longer than [`MAX_NAME_BYTES`].
    BadName,
    /// Two colliders share an id.
    DuplicateName,
    /// A layer is not exactly one bit.
    BadLayer(u32),
    /// The glTF has no `COL_` node with a mesh.
    NoCollider,
    /// The glTF failed to parse or read; bounded reason.
    Gltf(String),
}

impl fmt::Display for ColliderCookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => write!(f, "collider cook: a coordinate is not finite"),
            Self::TooFewPoints { got } => {
                write!(
                    f,
                    "collider cook: a hull needs 4 distinct points, got {got}"
                )
            }
            Self::Flat => write!(f, "collider cook: the points are flat, a hull needs volume"),
            Self::BadCap(cap) => write!(
                f,
                "collider cook: hull cap {cap} is outside 4..={MAX_HULL_POINTS}"
            ),
            Self::BadMesh(why) => write!(f, "collider cook: bad mesh, {why}"),
            Self::TooLarge { what, count } => {
                write!(f, "collider cook: {what} {count} is over its bound")
            }
            Self::BadName => write!(f, "collider cook: id must be 1..={MAX_NAME_BYTES} bytes"),
            Self::DuplicateName => write!(f, "collider cook: two colliders share an id"),
            Self::BadLayer(layer) => {
                write!(f, "collider cook: layer {layer} is not exactly one bit")
            }
            Self::NoCollider => write!(f, "collider cook: the glTF has no COL_ node with a mesh"),
            Self::Gltf(detail) => write!(f, "collider cook: glTF {detail}"),
        }
    }
}

impl std::error::Error for ColliderCookError {}

/// One cooked shape.
#[derive(Clone, Debug, PartialEq)]
pub enum ColliderShape {
    /// Convex hull: extreme points plus outward-wound triangle faces.
    Hull {
        /// Hull points in meters, sorted by (x, y, z).
        points: Vec<[f32; 3]>,
        /// Triangle faces as indices into `points`, canonical order.
        faces: Vec<[u16; 3]>,
    },
    /// Triangle mesh: welded vertices plus flat triangle indices.
    Mesh {
        /// Vertices in meters, first-use order.
        vertices: Vec<[f32; 3]>,
        /// Three indices per triangle.
        indices: Vec<u32>,
    },
}

/// One cooked collider.
#[derive(Clone, Debug, PartialEq)]
pub struct CookedCollider {
    /// Collider id (the art node name).
    pub name: String,
    /// Exactly one layer bit, chosen by the caller.
    pub layer: u32,
    /// The shape.
    pub shape: ColliderShape,
}

/// An ordered set of cooked colliders: the unit written to the bundle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColliderSet {
    /// Colliders in write order.
    pub colliders: Vec<CookedCollider>,
}

type V3 = [f64; 3];

fn wide(p: [f32; 3]) -> V3 {
    p.map(f64::from)
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    let ([ax, ay, az], [bx, by, bz]) = (a, b);
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// One outward hull face: winding, unit normal and plane offset.
struct Face {
    v: [usize; 3],
    n: V3,
    d: f64,
}

impl Face {
    /// Face over three points in the given winding; `None` when they are collinear.
    fn new(p: &[V3], v: [usize; 3]) -> Option<Self> {
        let n = cross(sub(p[v[1]], p[v[0]]), sub(p[v[2]], p[v[0]]));
        let len = norm(n);
        if len <= 1.0e-300 {
            return None;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        Some(Self {
            v,
            n,
            d: dot(n, p[v[0]]),
        })
    }

    /// Signed distance of `q` above the plane (positive is outside).
    fn height(&self, q: V3) -> f64 {
        dot(self.n, q) - self.d
    }
}

/// Squared distance between two wide points.
fn dist2(a: V3, b: V3) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

/// The index with the greatest key. Keys within `eps` of each other tie, and a tie goes
/// to the point farthest from `centre` (then the lowest index): among points sharing a
/// face plane that is always a corner of the face, never a point on its edge or inside.
fn argbest(
    p: &[V3],
    centre: V3,
    eps: f64,
    keys: impl Iterator<Item = (usize, f64)>,
) -> Option<usize> {
    let mut best: Option<(usize, f64, f64)> = None;
    for (i, key) in keys {
        let far = dist2(p[i], centre);
        let better = match best {
            None => true,
            Some((_, bk, bf)) => key > bk + eps || (key >= bk - eps && far > bf),
        };
        if better {
            best = Some((i, key, far));
        }
    }
    best.map(|b| b.0)
}

/// Four affinely independent corner points: the widest extreme pair, the point
/// farthest from their line, the point farthest from that plane.
fn initial_simplex(p: &[V3], centre: V3, eps: f64) -> Result<[usize; 4], ColliderCookError> {
    let mut extremes = Vec::with_capacity(6);
    for axis in 0..3 {
        for sign in [-1.0, 1.0] {
            let keys = p.iter().enumerate().map(|(i, v)| (i, sign * v[axis]));
            extremes.extend(argbest(p, centre, eps, keys));
        }
    }
    let (mut a, mut b, mut widest) = (0, 0, -1.0);
    for &i in &extremes {
        for &j in &extremes {
            let d = dist2(p[i], p[j]);
            if d > widest {
                (a, b, widest) = (i, j, d);
            }
        }
    }
    let span = widest.sqrt();
    if span <= eps {
        return Err(ColliderCookError::Flat);
    }
    let ab = sub(p[b], p[a]);
    let off_line = |i: usize| norm(cross(ab, sub(p[i], p[a]))) / span;
    let c = argbest(p, centre, eps, (0..p.len()).map(|i| (i, off_line(i))))
        .ok_or(ColliderCookError::Flat)?;
    if off_line(c) <= eps {
        return Err(ColliderCookError::Flat);
    }
    let normal = cross(ab, sub(p[c], p[a]));
    let normal_len = norm(normal);
    let off_plane = |i: usize| dot(normal, sub(p[i], p[a])).abs() / normal_len;
    let d = argbest(p, centre, eps, (0..p.len()).map(|i| (i, off_plane(i))))
        .ok_or(ColliderCookError::Flat)?;
    if off_plane(d) <= eps {
        return Err(ColliderCookError::Flat);
    }
    Ok([a, b, c, d])
}

/// Quickhull over canonically ordered points: the faces of the hull, wound outward, as
/// indices into `pts`. Every unplaced point waits in the outside list of one face that
/// sees it; the point farthest outside a face (ties to a corner) is always a true hull
/// vertex, so points lying on edges or faces of the final hull are never added.
fn hull_triangles(pts: &[[f32; 3]]) -> Result<Vec<[usize; 3]>, ColliderCookError> {
    let p: Vec<V3> = pts.iter().copied().map(wide).collect();
    let mut lo = p[0];
    let mut hi = p[0];
    for v in &p {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(v[axis]);
            hi[axis] = hi[axis].max(v[axis]);
        }
    }
    let eps = norm(sub(hi, lo)) * PLANE_SLACK;
    let centre = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let simplex = initial_simplex(&p, centre, eps)?;
    let inside = {
        let sum = simplex.iter().fold([0.0; 3], |s, &i| {
            [s[0] + p[i][0], s[1] + p[i][1], s[2] + p[i][2]]
        });
        [sum[0] / 4.0, sum[1] / 4.0, sum[2] / 4.0]
    };
    let [a, b, c, d] = simplex;
    let mut faces: Vec<Face> = Vec::with_capacity(8);
    for v in [[a, b, c], [a, c, d], [a, d, b], [b, d, c]] {
        let face = Face::new(&p, v).ok_or(ColliderCookError::Flat)?;
        // Wind every simplex face away from the simplex centre.
        faces.push(if face.height(inside) > 0.0 {
            Face::new(&p, [v[0], v[2], v[1]]).ok_or(ColliderCookError::Flat)?
        } else {
            face
        });
    }
    let mut outside: Vec<Vec<usize>> = vec![Vec::new(); faces.len()];
    for (i, q) in p.iter().enumerate() {
        if simplex.contains(&i) {
            continue;
        }
        if let Some(k) = faces.iter().position(|f| f.height(*q) > eps) {
            outside[k].push(i);
        }
    }
    while let Some(k) = outside.iter().position(|list| !list.is_empty()) {
        let eye = argbest(
            &p,
            centre,
            eps,
            outside[k].iter().map(|&i| (i, faces[k].height(p[i]))),
        )
        .ok_or(ColliderCookError::Flat)?;
        let visible: Vec<usize> = (0..faces.len())
            .filter(|&f| faces[f].height(p[eye]) > eps)
            .collect();
        let edges: Vec<(usize, usize)> = visible
            .iter()
            .flat_map(|&f| {
                let v = faces[f].v;
                [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])]
            })
            .collect();
        let horizon: Vec<(usize, usize)> = edges
            .iter()
            .copied()
            .filter(|&(x, y)| !edges.contains(&(y, x)))
            .collect();
        if horizon.is_empty() {
            return Err(ColliderCookError::Flat);
        }
        let mut orphans: Vec<usize> = Vec::new();
        let mut kept_faces = Vec::with_capacity(faces.len());
        let mut kept_outside = Vec::with_capacity(faces.len());
        for (f, (face, list)) in faces.into_iter().zip(outside).enumerate() {
            if visible.contains(&f) {
                orphans.extend(list.into_iter().filter(|&i| i != eye));
            } else {
                kept_faces.push(face);
                kept_outside.push(list);
            }
        }
        faces = kept_faces;
        outside = kept_outside;
        let first_new = faces.len();
        for (x, y) in horizon {
            faces.push(Face::new(&p, [x, y, eye]).ok_or(ColliderCookError::Flat)?);
            outside.push(Vec::new());
        }
        for i in orphans {
            if let Some(off) = faces[first_new..].iter().position(|f| f.height(p[i]) > eps) {
                outside[first_new + off].push(i);
            }
        }
    }
    Ok(faces.into_iter().map(|f| f.v).collect())
}

/// Sort, de-duplicate and normalize points into the canonical set order.
fn canonical_points(points: &[[f32; 3]]) -> Result<Vec<[f32; 3]>, ColliderCookError> {
    if points.len() > MAX_INPUT_POINTS {
        return Err(ColliderCookError::TooLarge {
            what: "hull input points",
            count: points.len(),
        });
    }
    if points.iter().flatten().any(|v| !v.is_finite()) {
        return Err(ColliderCookError::NonFinite);
    }
    // `+ 0.0` turns -0.0 into 0.0, so equal positions compare equal.
    let mut out: Vec<[f32; 3]> = points.iter().map(|p| p.map(|v| v + 0.0)).collect();
    out.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
    out.dedup();
    Ok(out)
}

/// Hull points plus their faces.
type HullParts = (Vec<[f32; 3]>, Vec<[u16; 3]>);

/// Hull of canonical points as (used points in the same order, faces over them).
fn hull_of(pts: &[[f32; 3]]) -> Result<HullParts, ColliderCookError> {
    if pts.len() < 4 {
        return Err(ColliderCookError::TooFewPoints { got: pts.len() });
    }
    let tris = hull_triangles(pts)?;
    let mut used = vec![false; pts.len()];
    for &i in tris.iter().flatten() {
        used[i] = true;
    }
    let mut remap = vec![0usize; pts.len()];
    let mut verts = Vec::new();
    for (i, keep) in used.iter().enumerate() {
        if *keep {
            remap[i] = verts.len();
            verts.push(pts[i]);
        }
    }
    let mut faces = Vec::with_capacity(tris.len());
    for t in tris {
        // Rotate the smallest index first (winding kept), then sort the faces.
        let at = (0..3).min_by_key(|&k| t[k]).unwrap_or(0);
        let r = [t[at], t[(at + 1) % 3], t[(at + 2) % 3]].map(|i| remap[i]);
        let r = r.map(u16::try_from);
        let [Ok(x), Ok(y), Ok(z)] = r else {
            return Err(ColliderCookError::TooLarge {
                what: "hull points",
                count: verts.len(),
            });
        };
        faces.push([x, y, z]);
    }
    faces.sort_unstable();
    Ok((verts, faces))
}

/// `cap` points of `verts` spread as far apart as possible, farthest-point
/// sampling from the first point, in the original order.
fn spread_sample(verts: &[[f32; 3]], cap: usize) -> Vec<[f32; 3]> {
    let wide_verts: Vec<V3> = verts.iter().copied().map(wide).collect();
    let mut chosen = vec![0usize];
    let mut nearest: Vec<f64> = wide_verts
        .iter()
        .map(|v| dist2(*v, wide_verts[0]))
        .collect();
    while chosen.len() < cap {
        let mut best = 0;
        for (i, &d) in nearest.iter().enumerate() {
            if d > nearest[best] {
                best = i;
            }
        }
        if nearest[best] <= 0.0 {
            break;
        }
        chosen.push(best);
        for (i, v) in wide_verts.iter().enumerate() {
            nearest[i] = nearest[i].min(dist2(*v, wide_verts[best]));
        }
    }
    chosen.sort_unstable();
    chosen.into_iter().map(|i| verts[i]).collect()
}

/// Cook the convex hull of a point cloud, at most `cap` points.
///
/// The result depends on the point set only, not on its order. Points on a face
/// plane or inside the hull never become hull points. A cloud whose hull has more
/// than `cap` extreme points keeps `cap` of them spread by farthest-point sampling
/// and re-wraps them, so the capped hull sits inside the true one.
pub fn cook_hull(points: &[[f32; 3]], cap: usize) -> Result<ColliderShape, ColliderCookError> {
    if !(4..=MAX_HULL_POINTS).contains(&cap) {
        return Err(ColliderCookError::BadCap(cap));
    }
    let pts = canonical_points(points)?;
    let (mut verts, mut faces) = hull_of(&pts)?;
    if verts.len() > cap {
        (verts, faces) = hull_of(&spread_sample(&verts, cap))?;
    }
    Ok(ColliderShape::Hull {
        points: verts,
        faces,
    })
}

/// Cook a triangle mesh: weld vertices by exact position (first-use order), drop
/// triangles that collapse to a line or point, refuse an empty or out-of-range mesh.
pub fn cook_mesh(
    positions: &[[f32; 3]],
    indices: &[u32],
) -> Result<ColliderShape, ColliderCookError> {
    if positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(ColliderCookError::NonFinite);
    }
    if indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err(ColliderCookError::BadMesh(
            "indices are empty or not in threes",
        ));
    }
    if indices.len() / 3 > MAX_MESH_TRIS * 4 {
        return Err(ColliderCookError::TooLarge {
            what: "mesh input triangles",
            count: indices.len() / 3,
        });
    }
    let mut seen: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut out: Vec<u32> = Vec::new();
    for tri in indices.chunks_exact(3) {
        let mut corners = [[0.0f32; 3]; 3];
        for (slot, &index) in corners.iter_mut().zip(tri) {
            let at = usize::try_from(index).unwrap_or(usize::MAX);
            let p = positions.get(at).ok_or(ColliderCookError::BadMesh(
                "an index is outside the vertices",
            ))?;
            *slot = p.map(|v| v + 0.0);
        }
        let [a, b, c] = corners.map(wide);
        if norm(cross(sub(b, a), sub(c, a))) <= 0.0 {
            continue;
        }
        for corner in corners {
            let key = corner.map(f32::to_bits);
            let next = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            let id = *seen.entry(key).or_insert_with(|| {
                vertices.push(corner);
                next
            });
            out.push(id);
        }
    }
    if out.is_empty() {
        return Err(ColliderCookError::BadMesh("every triangle is degenerate"));
    }
    if vertices.len() > MAX_MESH_VERTS {
        return Err(ColliderCookError::TooLarge {
            what: "mesh vertices",
            count: vertices.len(),
        });
    }
    if out.len() / 3 > MAX_MESH_TRIS {
        return Err(ColliderCookError::TooLarge {
            what: "mesh triangles",
            count: out.len() / 3,
        });
    }
    Ok(ColliderShape::Mesh {
        vertices,
        indices: out,
    })
}

fn push_point(out: &mut Vec<u8>, p: &[f32; 3]) {
    for v in p {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn count_u16(count: usize) -> Result<u16, ColliderCookError> {
    u16::try_from(count).map_err(|_| ColliderCookError::TooLarge {
        what: "count",
        count,
    })
}

fn count_u32(count: usize) -> Result<u32, ColliderCookError> {
    u32::try_from(count).map_err(|_| ColliderCookError::TooLarge {
        what: "count",
        count,
    })
}

impl ColliderSet {
    /// Write the set as cooked bytes. Refuses what the reader would refuse: too
    /// many colliders, a bad id, a repeated id, a layer that is not one bit, a
    /// shape beyond its bound or with an index outside its points.
    pub fn encode(&self) -> Result<Vec<u8>, ColliderCookError> {
        if self.colliders.len() > MAX_COLLIDERS {
            return Err(ColliderCookError::TooLarge {
                what: "colliders",
                count: self.colliders.len(),
            });
        }
        let mut out = MAGIC.to_vec();
        push_u16(&mut out, VERSION);
        push_u16(&mut out, count_u16(self.colliders.len())?);
        for (at, c) in self.colliders.iter().enumerate() {
            let name_len = u8::try_from(c.name.len()).map_err(|_| ColliderCookError::BadName)?;
            if name_len == 0 || usize::from(name_len) > MAX_NAME_BYTES {
                return Err(ColliderCookError::BadName);
            }
            if self.colliders[..at].iter().any(|o| o.name == c.name) {
                return Err(ColliderCookError::DuplicateName);
            }
            if !c.layer.is_power_of_two() {
                return Err(ColliderCookError::BadLayer(c.layer));
            }
            out.push(name_len);
            out.extend_from_slice(c.name.as_bytes());
            push_u32(&mut out, c.layer);
            encode_shape(&mut out, &c.shape)?;
        }
        Ok(out)
    }
}

fn encode_shape(out: &mut Vec<u8>, shape: &ColliderShape) -> Result<(), ColliderCookError> {
    match shape {
        ColliderShape::Hull { points, faces } => {
            if !(4..=MAX_HULL_POINTS).contains(&points.len())
                || !(4..=2 * MAX_HULL_POINTS).contains(&faces.len())
            {
                return Err(ColliderCookError::BadCap(points.len()));
            }
            if faces
                .iter()
                .flatten()
                .any(|&i| usize::from(i) >= points.len())
            {
                return Err(ColliderCookError::BadMesh(
                    "a hull face index is outside the points",
                ));
            }
            if points.iter().flatten().any(|v| !v.is_finite()) {
                return Err(ColliderCookError::NonFinite);
            }
            out.push(KIND_HULL);
            push_u16(out, count_u16(points.len())?);
            for p in points {
                push_point(out, p);
            }
            push_u16(out, count_u16(faces.len())?);
            for &i in faces.iter().flatten() {
                push_u16(out, i);
            }
        }
        ColliderShape::Mesh { vertices, indices } => {
            if !(3..=MAX_MESH_VERTS).contains(&vertices.len())
                || indices.is_empty()
                || !indices.len().is_multiple_of(3)
                || indices.len() > 3 * MAX_MESH_TRIS
            {
                return Err(ColliderCookError::TooLarge {
                    what: "mesh",
                    count: vertices.len(),
                });
            }
            if indices
                .iter()
                .any(|&i| !usize::try_from(i).is_ok_and(|i| i < vertices.len()))
            {
                return Err(ColliderCookError::BadMesh(
                    "an index is outside the vertices",
                ));
            }
            if vertices.iter().flatten().any(|v| !v.is_finite()) {
                return Err(ColliderCookError::NonFinite);
            }
            out.push(KIND_MESH);
            push_u32(out, count_u32(vertices.len())?);
            for p in vertices {
                push_point(out, p);
            }
            push_u32(out, count_u32(indices.len())?);
            for &i in indices {
                push_u32(out, i);
            }
        }
    }
    Ok(())
}

#[cfg(feature = "import")]
pub use from_gltf::{cook_gltf_colliders, ColliderRole};

/// glTF extraction: every `COL_` node with a mesh becomes one collider.
#[cfg(feature = "import")]
mod from_gltf {
    use super::{cook_hull, cook_mesh, ColliderCookError, ColliderSet, CookedCollider};
    use super::{MAX_HULL_POINTS, MAX_NAME_BYTES};
    use crate::mesh_cook::is_col_name;
    use crate::mesh_import::{bound_detail, MAX_BYTES};

    /// Deepest node nesting followed; deeper art is refused, never walked.
    const MAX_DEPTH: usize = 64;

    /// What a file's `COL_` nodes become.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ColliderRole {
        /// A static kit piece: a triangle mesh of the node's triangles.
        StaticKit,
        /// A dynamic prop: the convex hull of the node's points.
        DynamicProp,
    }

    type Mat = [[f32; 4]; 4];

    const IDENTITY: Mat = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    /// `a * b` for column-major matrices (`m[column][row]`).
    fn mul(a: &Mat, b: &Mat) -> Mat {
        let mut out = [[0.0f32; 4]; 4];
        for (c, column) in out.iter_mut().enumerate() {
            for (r, cell) in column.iter_mut().enumerate() {
                *cell =
                    a[0][r] * b[c][0] + a[1][r] * b[c][1] + a[2][r] * b[c][2] + a[3][r] * b[c][3];
            }
        }
        out
    }

    fn apply(m: &Mat, p: [f32; 3]) -> [f32; 3] {
        let row = |r: usize| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
        [row(0), row(1), row(2)]
    }

    struct Cook<'a> {
        buffers: &'a [gltf::buffer::Data],
        role: ColliderRole,
        layer: u32,
        out: Vec<CookedCollider>,
    }

    impl Cook<'_> {
        /// Walk one node and its children, parent transform first.
        fn visit(
            &mut self,
            node: &gltf::Node<'_>,
            parent: &Mat,
            depth: usize,
        ) -> Result<(), ColliderCookError> {
            if depth > MAX_DEPTH {
                return Err(ColliderCookError::Gltf("nests nodes too deep".to_owned()));
            }
            let world = mul(parent, &node.transform().matrix());
            let name = node.name().unwrap_or("");
            if let (true, Some(mesh)) = (is_col_name(name), node.mesh()) {
                self.collect(name, &mesh, &world)?;
            }
            for child in node.children() {
                self.visit(&child, &world, depth + 1)?;
            }
            Ok(())
        }

        /// Merge a node's primitives (world space) and cook the one collider.
        fn collect(
            &mut self,
            name: &str,
            mesh: &gltf::Mesh<'_>,
            world: &Mat,
        ) -> Result<(), ColliderCookError> {
            if name.len() > MAX_NAME_BYTES {
                return Err(ColliderCookError::BadName);
            }
            let mut positions: Vec<[f32; 3]> = Vec::new();
            let mut indices: Vec<u32> = Vec::new();
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    return Err(ColliderCookError::Gltf(
                        "collider primitive is not triangles".to_owned(),
                    ));
                }
                let buffers = self.buffers;
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &*d.0));
                let base =
                    u32::try_from(positions.len()).map_err(|_| ColliderCookError::TooLarge {
                        what: "vertices",
                        count: positions.len(),
                    })?;
                let local: Vec<[f32; 3]> = reader
                    .read_positions()
                    .ok_or_else(|| ColliderCookError::Gltf("collider has no POSITION".to_owned()))?
                    .collect();
                let count =
                    u32::try_from(local.len()).map_err(|_| ColliderCookError::TooLarge {
                        what: "vertices",
                        count: local.len(),
                    })?;
                match reader.read_indices() {
                    Some(read) => indices.extend(read.into_u32().map(|i| i.saturating_add(base))),
                    None => indices.extend((0..count).map(|i| i + base)),
                }
                positions.extend(local.into_iter().map(|p| apply(world, p)));
            }
            let shape = match self.role {
                ColliderRole::DynamicProp => cook_hull(&positions, MAX_HULL_POINTS)?,
                ColliderRole::StaticKit => cook_mesh(&positions, &indices)?,
            };
            self.out.push(CookedCollider {
                name: name.to_owned(),
                layer: self.layer,
                shape,
            });
            Ok(())
        }
    }

    /// Cook every `COL_` node with a mesh in glTF bytes (`.gltf` or `.glb`) into
    /// one collider each, in scene document order, node transforms applied (so a
    /// file authored in place cooks in place). `role` picks hull or mesh and
    /// `layer` is the single layer bit every collider carries.
    pub fn cook_gltf_colliders(
        bytes: &[u8],
        role: ColliderRole,
        layer: u32,
    ) -> Result<ColliderSet, ColliderCookError> {
        if bytes.len() > MAX_BYTES {
            return Err(ColliderCookError::TooLarge {
                what: "glTF bytes",
                count: bytes.len(),
            });
        }
        if !layer.is_power_of_two() {
            return Err(ColliderCookError::BadLayer(layer));
        }
        let gltf_err =
            |e: &dyn core::fmt::Debug| ColliderCookError::Gltf(bound_detail(&format!("{e:?}")));
        let parsed = gltf::Gltf::from_slice(bytes).map_err(|e| gltf_err(&e))?;
        let buffers = gltf::import_buffers(&parsed.document, None, parsed.blob.clone())
            .map_err(|e| gltf_err(&e))?;
        let mut cook = Cook {
            buffers: &buffers,
            role,
            layer,
            out: Vec::new(),
        };
        for scene in parsed.document.scenes() {
            for node in scene.nodes() {
                cook.visit(&node, &IDENTITY, 0)?;
            }
        }
        if cook.out.is_empty() {
            return Err(ColliderCookError::NoCollider);
        }
        let set = ColliderSet {
            colliders: cook.out,
        };
        // Refuse a repeated id or bound here, with the typed reason, not at write time.
        set.encode()?;
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::hash_bytes;

    const WORLD: u32 = 1 << 4;
    const PROP: u32 = 1 << 1;

    /// The unit cube `[0,1]^3` corners.
    fn cube_points() -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        for x in [0.0f32, 1.0] {
            for y in [0.0f32, 1.0] {
                for z in [0.0f32, 1.0] {
                    out.push([x, y, z]);
                }
            }
        }
        out
    }

    fn hull_parts(shape: ColliderShape) -> (Vec<[f32; 3]>, Vec<[u16; 3]>) {
        match shape {
            ColliderShape::Hull { points, faces } => (points, faces),
            ColliderShape::Mesh { .. } => panic!("expected a hull"),
        }
    }

    /// The pinned wire form of one unit-cube hull named `COL_box` on layer 2, a checked-in
    /// fixture: the reader in `engine-physics`, the arena zone and the door verbs decode these
    /// same bytes in their own tests.
    const GOLDEN: &[u8] = include_bytes!("../../../content/physics/unit_cube_hull.e2cc");
    /// FNV-1a 64 of the golden bytes.
    const GOLDEN_DIGEST: u64 = 0x7b89_68a4_7251_9727;

    fn golden_set() -> ColliderSet {
        ColliderSet {
            colliders: vec![CookedCollider {
                name: "COL_box".to_owned(),
                layer: PROP,
                shape: ColliderShape::Hull {
                    points: cube_points(),
                    faces: vec![
                        [0, 1, 3],
                        [0, 3, 2],
                        [4, 6, 7],
                        [4, 7, 5],
                        [0, 4, 5],
                        [0, 5, 1],
                        [2, 3, 7],
                        [2, 7, 6],
                        [0, 2, 6],
                        [0, 6, 4],
                        [1, 5, 7],
                        [1, 7, 3],
                    ],
                },
            }],
        }
    }

    #[test]
    fn collider_cook_hull_of_a_box_is_eight_points() {
        let (points, faces) =
            hull_parts(cook_hull(&cube_points(), MAX_HULL_POINTS).expect("cooks"));
        assert_eq!(points.len(), 8);
        assert_eq!(faces.len(), 12, "a box is twelve triangles");
        assert_eq!(points, cube_points(), "points come out sorted");
        // Closed and outward: every directed edge once, its reverse once, and each
        // face normal points away from the centre.
        let mut edges = Vec::new();
        for f in &faces {
            for (a, b) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                edges.push((a, b));
            }
            let [a, b, c] = f.map(|i| points[usize::from(i)]);
            let n = [
                (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
                (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
                (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
            ];
            let out = n[0] * (a[0] - 0.5) + n[1] * (a[1] - 0.5) + n[2] * (a[2] - 0.5);
            assert!(out > 0.0, "face {f:?} winds inward");
        }
        for &(a, b) in &edges {
            assert_eq!(edges.iter().filter(|&&e| e == (a, b)).count(), 1);
            assert!(edges.contains(&(b, a)), "edge {a}->{b} has no twin");
        }
    }

    #[test]
    fn collider_cook_points_on_or_inside_the_hull_never_join_it() {
        let mut cloud = cube_points();
        cloud.push([0.5, 0.5, 0.5]); // inside
        cloud.push([0.5, 0.5, 0.0]); // face centre
        cloud.push([0.5, 0.0, 0.0]); // edge midpoint
        let (points, _) = hull_parts(cook_hull(&cloud, MAX_HULL_POINTS).expect("cooks"));
        assert_eq!(points, cube_points());
    }

    #[test]
    fn collider_cook_bytes_equal_twice_and_ignore_point_order() {
        let encode = |pts: &[[f32; 3]]| {
            ColliderSet {
                colliders: vec![CookedCollider {
                    name: "COL_a".to_owned(),
                    layer: WORLD,
                    shape: cook_hull(pts, MAX_HULL_POINTS).expect("cooks"),
                }],
            }
            .encode()
            .expect("encodes")
        };
        let mut skewed = cube_points();
        skewed.push([0.25, 0.5, 2.0]);
        skewed.push([-1.0, 0.5, 0.25]);
        let first = encode(&skewed);
        assert_eq!(first, encode(&skewed), "same input, same bytes");
        let mut reversed = skewed.clone();
        reversed.reverse();
        assert_eq!(
            first,
            encode(&reversed),
            "input order does not move the bytes"
        );
        let mut rotated = skewed.clone();
        rotated.rotate_left(3);
        assert_eq!(first, encode(&rotated));
        assert_eq!(hash_bytes(&first), hash_bytes(&encode(&skewed)));
    }

    /// Paraboloid lattice: every point is its own hull vertex, so 169 points
    /// all want to stay.
    fn paraboloid() -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        for i in -6i8..=6 {
            for j in -6i8..=6 {
                let (x, z) = (f32::from(i), f32::from(j));
                out.push([x, x * x + z * z, z]);
            }
        }
        out
    }

    #[test]
    fn collider_cook_point_cap_is_respected() {
        let cloud = paraboloid();
        let (_, natural) = {
            let pts = canonical_points(&cloud).expect("finite");
            hull_of(&pts).expect("wraps")
        };
        assert!(natural.len() > 2 * MAX_HULL_POINTS, "the cloud needs a cap");
        for cap in [4, 12, MAX_HULL_POINTS] {
            let (points, faces) = hull_parts(cook_hull(&cloud, cap).expect("cooks"));
            assert!(
                points.len() <= cap,
                "cap {cap} held {} points",
                points.len()
            );
            assert!(points.len() >= 4);
            assert!(faces
                .iter()
                .flatten()
                .all(|&i| usize::from(i) < points.len()));
        }
        assert_eq!(cook_hull(&cloud, 3), Err(ColliderCookError::BadCap(3)));
        assert_eq!(cook_hull(&cloud, 65), Err(ColliderCookError::BadCap(65)));
        // A capped hull still bytes-equal twice.
        assert_eq!(cook_hull(&cloud, 12), cook_hull(&cloud, 12));
    }

    #[test]
    fn collider_cook_refuses_flat_short_and_non_finite_clouds() {
        let flat: Vec<[f32; 3]> = (0..6u8)
            .map(|i| [f32::from(i), f32::from(i % 2), 0.0])
            .collect();
        assert_eq!(cook_hull(&flat, 16), Err(ColliderCookError::Flat));
        let line: Vec<[f32; 3]> = (0..6u8).map(|i| [f32::from(i), 0.0, 0.0]).collect();
        assert_eq!(cook_hull(&line, 16), Err(ColliderCookError::Flat));
        let few = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        assert_eq!(
            cook_hull(&few, 16),
            Err(ColliderCookError::TooFewPoints { got: 3 })
        );
        let mut nan = cube_points();
        nan[3][1] = f32::NAN;
        assert_eq!(cook_hull(&nan, 16), Err(ColliderCookError::NonFinite));
        assert_eq!(
            cook_hull(&[], 16),
            Err(ColliderCookError::TooFewPoints { got: 0 })
        );
    }

    type Quad = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

    /// A box mesh with duplicated corner vertices per face, like an exporter writes.
    fn split_box() -> (Vec<[f32; 3]>, Vec<u32>) {
        let faces: [Quad; 2] = [
            (
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ),
            (
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ),
        ];
        let mut pos = Vec::new();
        let mut idx = Vec::new();
        for (a, b, c, d) in faces {
            let base = u32::try_from(pos.len()).expect("small");
            pos.extend([a, b, c, d]);
            idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        // A side quad that reuses the two faces' corners as separate vertices.
        let base = u32::try_from(pos.len()).expect("small");
        pos.extend([
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ]);
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        (pos, idx)
    }

    #[test]
    fn collider_cook_mesh_welds_positions_and_drops_degenerate_triangles() {
        let (mut pos, mut idx) = split_box();
        // A zero-area triangle: three collinear corners.
        let base = u32::try_from(pos.len()).expect("small");
        pos.extend([[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [1.0, 0.0, 0.0]]);
        idx.extend([base, base + 1, base + 2]);
        let ColliderShape::Mesh { vertices, indices } = cook_mesh(&pos, &idx).expect("cooks")
        else {
            panic!("expected a mesh");
        };
        assert_eq!(vertices.len(), 8, "12 written corners weld to 8 positions");
        assert_eq!(indices.len(), 18, "six real triangles survive");
        assert_eq!(
            cook_mesh(&pos, &idx),
            cook_mesh(&pos, &idx),
            "twice the same"
        );
        assert_eq!(vertices[0], [0.0, 0.0, 0.0], "first-use order");
    }

    #[test]
    fn collider_cook_mesh_refuses_bad_input() {
        let (pos, idx) = split_box();
        assert!(matches!(
            cook_mesh(&pos, &[]),
            Err(ColliderCookError::BadMesh(_))
        ));
        assert!(matches!(
            cook_mesh(&pos, &idx[..4]),
            Err(ColliderCookError::BadMesh(_))
        ));
        let mut bad = idx.clone();
        bad[2] = 99;
        assert!(matches!(
            cook_mesh(&pos, &bad),
            Err(ColliderCookError::BadMesh(_))
        ));
        let flat = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        assert!(matches!(
            cook_mesh(&flat, &[0, 1, 2]),
            Err(ColliderCookError::BadMesh(_))
        ));
        let mut nan = pos.clone();
        nan[0][0] = f32::INFINITY;
        assert_eq!(cook_mesh(&nan, &idx), Err(ColliderCookError::NonFinite));
    }

    #[test]
    fn collider_cook_golden_bytes_and_digest_are_pinned() {
        let bytes = golden_set().encode().expect("encodes");
        assert_eq!(
            bytes, GOLDEN,
            "the writer reproduces the checked-in fixture"
        );
        assert_eq!(hash_bytes(&bytes), GOLDEN_DIGEST);
        assert_eq!(
            bytes,
            golden_set().encode().expect("encodes"),
            "twice the same"
        );
        assert_eq!(bytes.len(), 193);
    }

    #[test]
    fn collider_cook_cooked_cube_has_a_closed_pinned_shape() {
        // The cook of the cube corners is the same hull the golden set spells out
        // by points; only the diagonal choice of each quad is the cook's.
        let (points, faces) =
            hull_parts(cook_hull(&cube_points(), MAX_HULL_POINTS).expect("cooks"));
        let golden = golden_set();
        let ColliderShape::Hull {
            points: gp,
            faces: gf,
        } = &golden.colliders[0].shape
        else {
            panic!("golden is a hull");
        };
        assert_eq!(&points, gp);
        assert_eq!(faces.len(), gf.len());
    }

    #[test]
    fn collider_cook_encode_refuses_what_the_reader_would() {
        let one = |name: &str, layer: u32| ColliderSet {
            colliders: vec![CookedCollider {
                name: name.to_owned(),
                layer,
                shape: golden_set().colliders[0].shape.clone(),
            }],
        };
        assert_eq!(one("", WORLD).encode(), Err(ColliderCookError::BadName));
        assert_eq!(
            one(&"n".repeat(65), WORLD).encode(),
            Err(ColliderCookError::BadName)
        );
        assert_eq!(
            one("COL_a", 0).encode(),
            Err(ColliderCookError::BadLayer(0))
        );
        assert_eq!(
            one("COL_a", 3).encode(),
            Err(ColliderCookError::BadLayer(3))
        );
        let mut twice = one("COL_a", WORLD);
        twice.colliders.push(twice.colliders[0].clone());
        assert_eq!(twice.encode(), Err(ColliderCookError::DuplicateName));
        let mut broken = one("COL_a", WORLD);
        if let ColliderShape::Hull { faces, .. } = &mut broken.colliders[0].shape {
            faces[0] = [0, 1, 40];
        }
        assert!(matches!(
            broken.encode(),
            Err(ColliderCookError::BadMesh(_))
        ));
        assert!(
            ColliderSet::default().encode().is_ok(),
            "an empty set is a valid blob"
        );
    }

    #[cfg(feature = "import")]
    mod gltf_files {
        use super::*;

        fn b64(bytes: &[u8]) -> String {
            const A: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let n = chunk
                    .iter()
                    .enumerate()
                    .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
                for k in 0..4 {
                    if k <= chunk.len() {
                        out.push(char::from(
                            A[usize::try_from((n >> (18 - 6 * k)) & 63).expect("six bits")],
                        ));
                    } else {
                        out.push('=');
                    }
                }
            }
            out
        }

        /// A glTF with one cube mesh (corners at +-0.5) and the given nodes JSON.
        fn cube_gltf(nodes: &str) -> Vec<u8> {
            let mut buffer = Vec::new();
            for x in [-0.5f32, 0.5] {
                for y in [-0.5f32, 0.5] {
                    for z in [-0.5f32, 0.5] {
                        for v in [x, y, z] {
                            buffer.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
            let faces: [[u16; 3]; 12] = [
                [0, 1, 3],
                [0, 3, 2],
                [4, 6, 7],
                [4, 7, 5],
                [0, 4, 5],
                [0, 5, 1],
                [2, 3, 7],
                [2, 7, 6],
                [0, 2, 6],
                [0, 6, 4],
                [1, 5, 7],
                [1, 7, 3],
            ];
            for i in faces.iter().flatten() {
                buffer.extend_from_slice(&i.to_le_bytes());
            }
            format!(
                concat!(
                    "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0,1]}}],",
                    "\"nodes\":{nodes},",
                    "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1}}]}}],",
                    "\"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{uri}\",\"byteLength\":{len}}}],",
                    "\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":96}},",
                    "{{\"buffer\":0,\"byteOffset\":96,\"byteLength\":72}}],",
                    "\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":8,\"type\":\"VEC3\",",
                    "\"min\":[-0.5,-0.5,-0.5],\"max\":[0.5,0.5,0.5]}},",
                    "{{\"bufferView\":1,\"componentType\":5123,\"count\":36,\"type\":\"SCALAR\"}}]}}"
                ),
                nodes = nodes,
                uri = b64(&buffer),
                len = buffer.len(),
            )
            .into_bytes()
        }

        const PLACED: &str = "[{\"name\":\"Body\",\"mesh\":0},{\"name\":\"COL_pillar\",\"mesh\":0,\"translation\":[4,1,2],\"scale\":[1,3,1]}]";

        #[test]
        fn collider_cook_a_prop_node_becomes_a_hull_in_world_space() {
            let set = cook_gltf_colliders(&cube_gltf(PLACED), ColliderRole::DynamicProp, PROP)
                .expect("cooks");
            assert_eq!(set.colliders.len(), 1, "only the COL_ node cooks");
            let c = &set.colliders[0];
            assert_eq!((c.name.as_str(), c.layer), ("COL_pillar", PROP));
            let (points, faces) = hull_parts(c.shape.clone());
            assert_eq!(points.len(), 8, "the hull of a box is 8 points");
            assert_eq!(faces.len(), 12);
            for p in &points {
                assert!((3.5..=4.5).contains(&p[0]), "x placed: {p:?}");
                assert!(
                    (-0.5..=2.5).contains(&p[1]),
                    "y scaled by 3 then lifted by 1: {p:?}"
                );
                assert!((1.5..=2.5).contains(&p[2]), "z placed: {p:?}");
            }
        }

        #[test]
        fn collider_cook_a_static_node_becomes_a_welded_mesh_and_cooks_twice_equal() {
            let gltf = cube_gltf(PLACED);
            let first = cook_gltf_colliders(&gltf, ColliderRole::StaticKit, WORLD).expect("cooks");
            let ColliderShape::Mesh { vertices, indices } = &first.colliders[0].shape else {
                panic!("a static piece is a mesh");
            };
            assert_eq!((vertices.len(), indices.len()), (8, 36));
            let again = cook_gltf_colliders(&gltf, ColliderRole::StaticKit, WORLD).expect("cooks");
            assert_eq!(
                first.encode().expect("bytes"),
                again.encode().expect("bytes")
            );
        }

        #[test]
        fn collider_cook_refuses_a_file_without_colliders_or_a_bad_layer_or_junk() {
            let plain =
                cube_gltf("[{\"name\":\"Body\",\"mesh\":0},{\"name\":\"Prop\",\"mesh\":0}]");
            assert_eq!(
                cook_gltf_colliders(&plain, ColliderRole::StaticKit, WORLD),
                Err(ColliderCookError::NoCollider)
            );
            assert_eq!(
                cook_gltf_colliders(&cube_gltf(PLACED), ColliderRole::StaticKit, 0),
                Err(ColliderCookError::BadLayer(0))
            );
            assert!(matches!(
                cook_gltf_colliders(b"not a gltf", ColliderRole::StaticKit, WORLD),
                Err(ColliderCookError::Gltf(_))
            ));
        }

        /// The shipped kit art already carries `COL_` boxes (each face cut 4x4): the
        /// crate is a prop (eight points), the walls a static piece (the welded
        /// 98-vertex, 192-triangle skin of a box).
        #[test]
        fn collider_cook_reads_the_shipped_kit_colliders() {
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../content/arena/src/");
            let crate_bytes = std::fs::read(format!("{dir}crate.gltf")).expect("kit crate");
            let set =
                cook_gltf_colliders(&crate_bytes, ColliderRole::DynamicProp, PROP).expect("cooks");
            let (points, _) = hull_parts(set.colliders[0].shape.clone());
            assert_eq!(
                (set.colliders[0].name.as_str(), points.len()),
                ("COL_crate", 8)
            );
            let walls_bytes = std::fs::read(format!("{dir}walls.gltf")).expect("kit walls");
            let walls =
                cook_gltf_colliders(&walls_bytes, ColliderRole::StaticKit, WORLD).expect("cooks");
            let ColliderShape::Mesh { vertices, indices } = &walls.colliders[0].shape else {
                panic!("walls are a mesh");
            };
            assert_eq!(indices.len(), 3 * 192, "6 faces x 16 cells x 2 triangles");
            assert!(
                (98..=150).contains(&vertices.len()),
                "welding joins the shared box edges, got {}",
                vertices.len()
            );
        }
    }
}
