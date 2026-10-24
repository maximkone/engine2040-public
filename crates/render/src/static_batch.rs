//! static_batch: the arena's dressing baked into one vertex/index pair at load
//! (SW153). Banners, torch brackets, urns and benches never move, so the potato
//! tier pays one `draw_indexed` for them instead of one per prop.
//!
//! Shape (ideas from three.js `BufferGeometryUtils`): [`merge`] pre-transforms
//! every mesh by its row-major `[f32; 16]` matrix, rebases each index by the
//! running vertex count and refuses a mixed attribute set by name;
//! [`merge_vertices`] welds vertices that share a quantised
//! `(position, normal, uv)` key, so a prop's duplicate corners come out once.
//!
//! Units: whatever the meshes carry (meters in this engine). Determinism:
//! `+ - *` in input order, an ordered map for the weld, no clock and no
//! parallel fold, so the same meshes and matrices give the same bytes twice.
//! Load-time only: nothing here runs inside a frame.

use core::fmt;
use std::collections::BTreeMap;

/// Position and uv quantisation for the weld: keys are whole
/// ten-thousandths, so 1e-4 is the tolerance.
const QUANTUM: f32 = 10_000.0;
/// Row-major `[f32; 16]` elements read as a 4x4 matrix; these are the columns
/// of the translation (elements 3, 7, 11).
const M03: usize = 3;
const M07: usize = 7;
const M11: usize = 11;

/// One mesh handed to the batch: positions, normals, optional uvs and indices.
#[derive(Clone, Copy, Debug)]
pub struct MeshRef<'a> {
    /// Vertex positions, one per vertex.
    pub positions: &'a [[f32; 3]],
    /// Vertex normals, one per vertex; a mesh without them cannot batch.
    pub normals: &'a [[f32; 3]],
    /// Texture coordinates, one per vertex, or `None` when the mesh has none.
    pub uvs: Option<&'a [[f32; 2]]>,
    /// Triangle indices into the mesh's own vertices.
    pub indices: &'a [u32],
}

/// One baked vertex/index pair, ready for one draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawItem {
    /// Vertex positions, in batch order.
    pub positions: Vec<[f32; 3]>,
    /// Vertex normals, in batch order.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates, in batch order.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle indices into the batch's vertices.
    pub indices: Vec<u32>,
}

impl DrawItem {
    /// Vertices in the batch.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Triangles in the batch.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Why a batch could not be baked. Every refusal names the mesh it saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchError {
    /// No mesh was offered: a batch of nothing is a caller bug.
    Empty,
    /// The matrix list is not one per mesh.
    MatrixCount {
        /// Meshes offered.
        meshes: usize,
        /// Matrices offered.
        matrices: usize,
    },
    /// One mesh is short of an attribute the batch needs.
    MissingAttribute {
        /// Position of the mesh in the input list.
        mesh: usize,
        /// The attribute's name: `positions`, `normals` or `uvs`.
        attribute: &'static str,
    },
    /// One mesh has an attribute length the others do not share.
    MismatchedAttribute {
        /// Position of the mesh in the input list.
        mesh: usize,
        /// The attribute's name.
        attribute: &'static str,
    },
    /// One index points past the mesh's own vertices.
    IndexOutOfRange {
        /// Position of the mesh in the input list.
        mesh: usize,
        /// The index read.
        index: u32,
    },
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "a batch needs at least one mesh"),
            Self::MatrixCount { meshes, matrices } => {
                write!(
                    f,
                    "{meshes} meshes take {meshes} matrices, {matrices} given"
                )
            }
            Self::MissingAttribute { mesh, attribute } => {
                write!(f, "mesh {mesh} carries no {attribute}")
            }
            Self::MismatchedAttribute { mesh, attribute } => {
                write!(
                    f,
                    "mesh {mesh} has a {attribute} count that breaks the attribute set"
                )
            }
            Self::IndexOutOfRange { mesh, index } => {
                write!(f, "mesh {mesh} indexes {index} past its own vertices")
            }
        }
    }
}

impl std::error::Error for BatchError {}

/// True when every mesh carries the same attribute set: normals always, uvs
/// either on all of them or on none.
fn attribute_set(meshes: &[MeshRef<'_>]) -> Result<bool, BatchError> {
    let mut uvs = None;
    for (mesh, piece) in meshes.iter().enumerate() {
        if piece.positions.is_empty() {
            return Err(BatchError::MissingAttribute {
                mesh,
                attribute: "positions",
            });
        }
        if piece.normals.len() != piece.positions.len() {
            return Err(BatchError::MissingAttribute {
                mesh,
                attribute: "normals",
            });
        }
        match (piece.uvs, uvs) {
            (None, None) => uvs = Some(false),
            (None, Some(false)) => {}
            (None, Some(true)) => {
                return Err(BatchError::MissingAttribute {
                    mesh,
                    attribute: "uvs",
                })
            }
            (Some(list), None) => {
                if list.len() != piece.positions.len() {
                    return Err(BatchError::MismatchedAttribute {
                        mesh,
                        attribute: "uvs",
                    });
                }
                uvs = Some(true);
            }
            (Some(list), Some(_)) => {
                if list.len() != piece.positions.len() {
                    return Err(BatchError::MismatchedAttribute {
                        mesh,
                        attribute: "uvs",
                    });
                }
                if let Some(false) = uvs {
                    return Err(BatchError::MissingAttribute {
                        mesh,
                        attribute: "uvs",
                    });
                }
            }
        }
    }
    Ok(uvs.unwrap_or(false))
}

/// Transform one position by a row-major 4x4 matrix, `w = 1`.
fn transform_point(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[M03],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[M07],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[M11],
    ]
}

/// Transform one normal by a row-major 4x4 matrix, `w = 0`; the matrices are
/// rigid or uniformly scaled, so no inverse transpose is needed.
fn transform_normal(m: &[f32; 16], n: [f32; 3]) -> [f32; 3] {
    [
        m[0] * n[0] + m[1] * n[1] + m[2] * n[2],
        m[4] * n[0] + m[5] * n[1] + m[6] * n[2],
        m[8] * n[0] + m[9] * n[1] + m[10] * n[2],
    ]
}

/// Bake `meshes`, each placed by its own row-major `[f32; 16]` matrix, into one
/// vertex/index pair. Indices are rebased by the running vertex count.
///
/// # Errors
///
/// Returns [`BatchError::Empty`] with no meshes, [`BatchError::MatrixCount`]
/// when the lists differ in length, [`BatchError::MissingAttribute`] or
/// [`BatchError::MismatchedAttribute`] when the meshes do not share one
/// attribute set, and [`BatchError::IndexOutOfRange`] for a bad index.
pub fn merge(meshes: &[MeshRef<'_>], matrices: &[[f32; 16]]) -> Result<DrawItem, BatchError> {
    if meshes.is_empty() {
        return Err(BatchError::Empty);
    }
    if meshes.len() != matrices.len() {
        return Err(BatchError::MatrixCount {
            meshes: meshes.len(),
            matrices: matrices.len(),
        });
    }
    let with_uv = attribute_set(meshes)?;
    let mut item = DrawItem::default();
    let vertices: usize = meshes.iter().map(|m| m.positions.len()).sum();
    let indices: usize = meshes.iter().map(|m| m.indices.len()).sum();
    item.positions.reserve(vertices);
    item.normals.reserve(vertices);
    if with_uv {
        item.uvs.reserve(vertices);
    }
    item.indices.reserve(indices);
    for (mesh, (piece, matrix)) in meshes.iter().zip(matrices).enumerate() {
        let base = u32::try_from(item.positions.len()).unwrap_or(u32::MAX);
        for (vertex, position) in piece.positions.iter().enumerate() {
            item.positions.push(transform_point(matrix, *position));
            item.normals
                .push(transform_normal(matrix, piece.normals[vertex]));
            if let Some(uvs) = piece.uvs {
                item.uvs.push(uvs[vertex]);
            }
        }
        for index in piece.indices {
            if usize::try_from(*index).unwrap_or(usize::MAX) >= piece.positions.len() {
                return Err(BatchError::IndexOutOfRange {
                    mesh,
                    index: *index,
                });
            }
            item.indices.push(base.saturating_add(*index));
        }
    }
    Ok(item)
}

/// One welded vertex: the quantised key the ordered map sorts on.
type WeldKey = (i64, i64, i64, i64, i64, i64, i64, i64);

/// Quantise one coordinate to whole ten-thousandths.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the value is multiplied by 1e4 and rounded; a mesh coordinate is far from i64::MAX"
)]
fn quantise(value: f32) -> i64 {
    (value * QUANTUM).round() as i64
}

/// The weld key of one vertex: position, normal and uv on the 1e-4 grid.
fn weld_key(item: &DrawItem, vertex: usize) -> WeldKey {
    let p = item.positions[vertex];
    let n = item.normals[vertex];
    let uv = item.uvs.get(vertex).copied().unwrap_or([0.0, 0.0]);
    (
        quantise(p[0]),
        quantise(p[1]),
        quantise(p[2]),
        quantise(n[0]),
        quantise(n[1]),
        quantise(n[2]),
        quantise(uv[0]),
        quantise(uv[1]),
    )
}

/// Weld vertices that share a quantised `(position, normal, uv)` key, in place
/// and in first-seen order, and remap the indices. Returns how many vertices
/// were dropped. Two vertices within 1e-4 of each other count as one.
pub fn merge_vertices(item: &mut DrawItem) -> usize {
    if item.positions.is_empty() {
        return 0;
    }
    let mut first: BTreeMap<WeldKey, u32> = BTreeMap::new();
    let mut remap = vec![0u32; item.positions.len()];
    let mut positions = Vec::with_capacity(item.positions.len());
    let mut normals = Vec::with_capacity(item.normals.len());
    let mut uvs = Vec::with_capacity(item.uvs.len());
    for vertex in 0..item.positions.len() {
        let key = weld_key(item, vertex);
        let slot = match first.get(&key) {
            Some(slot) => *slot,
            None => {
                let slot = u32::try_from(positions.len()).unwrap_or(u32::MAX);
                first.insert(key, slot);
                positions.push(item.positions[vertex]);
                normals.push(item.normals[vertex]);
                if let Some(uv) = item.uvs.get(vertex) {
                    uvs.push(*uv);
                }
                slot
            }
        };
        remap[vertex] = slot;
    }
    let dropped = item.positions.len() - positions.len();
    for index in &mut item.indices {
        let slot = usize::try_from(*index).unwrap_or(usize::MAX);
        *index = remap.get(slot).copied().unwrap_or(*index);
    }
    item.positions = positions;
    item.normals = normals;
    item.uvs = uvs;
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube: 24 vertices (4 per face), 36 indices, per-face normals.
    fn cube() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        for face in 0..6 {
            let axis = face / 2;
            let sign = if face % 2 == 0 { 1.0f32 } else { -1.0 };
            let mut normal = [0.0f32; 3];
            normal[axis] = sign;
            for corner in 0..4 {
                let u = f32::from(u8::try_from(corner & 1).unwrap_or(0));
                let v = f32::from(u8::try_from((corner >> 1) & 1).unwrap_or(0));
                let mut p = [0.0f32; 3];
                p[axis] = sign * 0.5;
                let other = [(axis + 1) % 3, (axis + 2) % 3];
                p[other[0]] = u - 0.5;
                p[other[1]] = v - 0.5;
                positions.push(p);
                normals.push(normal);
                uvs.push([u, v]);
            }
            let base = u32::try_from(face * 4).unwrap_or(0);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        (positions, normals, uvs, indices)
    }

    /// The identity matrix, row-major.
    fn identity() -> [f32; 16] {
        let mut m = [0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[15] = 1.0;
        m
    }

    /// A translation matrix, row-major.
    fn translate(x: f32, y: f32, z: f32) -> [f32; 16] {
        let mut m = identity();
        m[M03] = x;
        m[M07] = y;
        m[M11] = z;
        m
    }

    #[test]
    fn thirty_props_become_one_draw_item() {
        let (positions, normals, uvs, indices) = cube();
        let props: Vec<(Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>)> = (0..30)
            .map(|_| {
                (
                    positions.clone(),
                    normals.clone(),
                    uvs.clone(),
                    indices.clone(),
                )
            })
            .collect();
        let meshes: Vec<MeshRef<'_>> = props
            .iter()
            .map(|(p, n, u, i)| MeshRef {
                positions: p,
                normals: n,
                uvs: Some(u),
                indices: i,
            })
            .collect();
        let matrices: Vec<[f32; 16]> = (0..30)
            .map(|i| translate(f32::from(u8::try_from(i).unwrap_or(0)) * 2.0, 0.0, -5.0))
            .collect();
        let item = merge(&meshes, &matrices).expect("the dressing batches");
        assert_eq!(item.vertex_count(), 30 * 24, "30 props of 24 vertices");
        assert_eq!(item.triangle_count(), 30 * 12, "30 props of 12 triangles");
        let highest = item.indices.iter().copied().max().unwrap_or(0);
        assert_eq!(highest, 719, "indices rebased into 0..720");
        // The seventh prop's first vertex sits where its matrix put it.
        let seventh = 7 * 24;
        assert_eq!(
            item.positions[seventh],
            transform_point(&matrices[7], (cube().0)[0]),
            "the transform is applied, not just the offset"
        );
    }

    #[test]
    fn mixed_attributes_are_refused() {
        let (positions, normals, uvs, indices) = cube();
        let plain = MeshRef {
            positions: &positions,
            normals: &normals,
            uvs: None,
            indices: &indices,
        };
        let textured = MeshRef {
            positions: &positions,
            normals: &normals,
            uvs: Some(&uvs),
            indices: &indices,
        };
        let error = merge(&[plain, textured], &[identity(), identity()])
            .expect_err("one mesh has uvs and the other does not");
        assert!(
            matches!(
                error,
                BatchError::MissingAttribute {
                    mesh: 1,
                    attribute: "uvs"
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("uvs"), "{error}");
    }

    #[test]
    fn a_normal_less_mesh_is_refused_by_name() {
        let (positions, _, _, indices) = cube();
        let bare = MeshRef {
            positions: &positions,
            normals: &[],
            uvs: None,
            indices: &indices,
        };
        let error = merge(&[bare], &[identity()]).expect_err("no normals, no batch");
        assert!(
            matches!(
                error,
                BatchError::MissingAttribute {
                    mesh: 0,
                    attribute: "normals"
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("normals"), "{error}");
    }

    #[test]
    fn merge_vertices_drops_a_duplicated_quad() {
        // Two quads on the same square: 8 vertices, 4 after the weld.
        let positions = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let normals = vec![[0.0, 0.0, 1.0]; 8];
        let uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
            .into_iter()
            .cycle()
            .take(8)
            .collect::<Vec<_>>();
        let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        let mut item = DrawItem {
            positions,
            normals,
            uvs,
            indices,
        };
        let dropped = merge_vertices(&mut item);
        assert_eq!(dropped, 4, "the second quad is the first one again");
        assert_eq!(item.vertex_count(), 4, "4 of 8 vertices survive");
        assert_eq!(item.indices, vec![0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn the_same_batch_twice_is_the_same_bytes() {
        let (positions, normals, uvs, indices) = cube();
        let meshes = [
            MeshRef {
                positions: &positions,
                normals: &normals,
                uvs: Some(&uvs),
                indices: &indices,
            },
            MeshRef {
                positions: &positions,
                normals: &normals,
                uvs: Some(&uvs),
                indices: &indices,
            },
        ];
        let matrices = [identity(), translate(3.0, 0.0, 0.0)];
        let mut first = merge(&meshes, &matrices).expect("the pair batches");
        let mut second = merge(&meshes, &matrices).expect("the pair batches");
        let dropped = merge_vertices(&mut first);
        assert_eq!(dropped, merge_vertices(&mut second), "the weld is stable");
        assert_eq!(first, second, "same meshes, same bytes");
    }

    #[test]
    fn a_short_matrix_list_is_refused() {
        let (positions, normals, uvs, indices) = cube();
        let mesh = MeshRef {
            positions: &positions,
            normals: &normals,
            uvs: Some(&uvs),
            indices: &indices,
        };
        let error = merge(&[mesh], &[]).expect_err("one mesh takes one matrix");
        assert!(
            matches!(
                error,
                BatchError::MatrixCount {
                    meshes: 1,
                    matrices: 0
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn an_index_past_the_mesh_is_refused() {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let normals = vec![[0.0, 0.0, 1.0]; 3];
        let indices = [0u32, 1, 7];
        let mesh = MeshRef {
            positions: &positions,
            normals: &normals,
            uvs: None,
            indices: &indices,
        };
        let error = merge(&[mesh], &[identity()]).expect_err("index 7 is past 3 vertices");
        assert!(
            matches!(error, BatchError::IndexOutOfRange { mesh: 0, index: 7 }),
            "{error}"
        );
    }
}
