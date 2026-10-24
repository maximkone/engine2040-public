//! morph_import: glTF morph targets (blend shapes) into plain delta arrays.
//!
//! Owner: `engine-assets` (feature `import`); consumer: a face or body shape lane that blends
//! `positions + sum(weight_i * delta_i)`. Skin import keeps rejecting morph channels; this is
//! the one place that reads them. Units: meters in, meters out. Determinism: document order,
//! no clock, so identical bytes import identically.
//!
//! One [`MorphMesh`](crate::morph_import::MorphMesh) per glTF primitive that has targets, in mesh then primitive order. Only
//! the `POSITION` deltas are read (normal and tangent deltas are ignored). Target names come
//! from the mesh's `extras.targetNames` (what Blender writes), else `target_{i}`. Default
//! weights come from the mesh `weights`, else zeros. Refused with a typed error instead of a
//! guess: sparse accessors, a delta count that differs from the vertex count, a weights array
//! that differs from the target count, non-finite data, required extensions.

use super::mesh_import::{bound_detail, required_extension_name, MAX_BYTES, MAX_VERTICES};
use std::fmt;

/// Most targets one primitive may carry; bounds the work.
pub const MAX_TARGETS: usize = 128;

/// One blend shape: a displacement per vertex.
#[derive(Clone, Debug, PartialEq)]
pub struct MorphTarget {
    /// Name from `extras.targetNames`, else `target_{i}`.
    pub name: String,
    /// Displacement per vertex in meters, one entry per position.
    pub position_deltas: Vec<[f32; 3]>,
}

/// A primitive's base shape and its blend shapes.
#[derive(Clone, Debug, PartialEq)]
pub struct MorphMesh {
    /// Base positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Triangle indices; the sequence `0..vertices` when the file has none.
    pub indices: Vec<u32>,
    /// The blend shapes in file order.
    pub targets: Vec<MorphTarget>,
    /// Weight per target the file starts with (zeros when the mesh has no `weights`).
    pub default_weights: Vec<f32>,
}

/// Why a morph import failed; each malformed input maps to one variant with bounded detail.
#[derive(Clone, Debug, PartialEq)]
pub enum MorphImportError {
    /// Input is empty or larger than [`MAX_BYTES`]; byte count carried.
    BadSize {
        /// Input length in bytes.
        count: usize,
    },
    /// Container or JSON parse or validation failed; detail bounded.
    Parse {
        /// Truncated parser message.
        detail: String,
    },
    /// The file requires a glTF extension this importer does not interpret.
    UnsupportedExtension {
        /// Truncated extension name.
        name: String,
    },
    /// A buffer has no resolvable bytes in a slice import.
    MissingBuffer {
        /// Truncated detail naming the buffer.
        detail: String,
    },
    /// The base positions or a target's positions are a sparse accessor.
    SparseAccessor {
        /// Mesh index.
        mesh: usize,
        /// Primitive index inside the mesh.
        primitive: usize,
    },
    /// A target has a different number of deltas than the primitive has positions.
    CountMismatch {
        /// Mesh index.
        mesh: usize,
        /// Primitive index inside the mesh.
        primitive: usize,
        /// Target index.
        target: usize,
        /// Position count.
        expected: usize,
        /// Delta count.
        got: usize,
    },
    /// The mesh `weights` array length differs from the target count.
    WeightCount {
        /// Mesh index.
        mesh: usize,
        /// Target count.
        expected: usize,
        /// Weight count.
        got: usize,
    },
    /// A position or delta is NaN or infinite.
    NonFinite {
        /// Mesh index.
        mesh: usize,
        /// Primitive index inside the mesh.
        primitive: usize,
        /// Target index, or `None` for the base positions.
        target: Option<usize>,
    },
    /// A primitive has more than [`MAX_TARGETS`] targets or more than [`MAX_VERTICES`] vertices.
    TooLarge {
        /// Mesh index.
        mesh: usize,
        /// Primitive index inside the mesh.
        primitive: usize,
    },
}

impl fmt::Display for MorphImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize { count } => write!(f, "morph import: bad input size {count}"),
            Self::Parse { detail } => write!(f, "morph import parse: {}", bound_detail(detail)),
            Self::UnsupportedExtension { name } => {
                write!(f, "morph import: unsupported extension {}", bound_detail(name))
            }
            Self::MissingBuffer { detail } => {
                write!(f, "morph import missing buffer: {}", bound_detail(detail))
            }
            Self::SparseAccessor { mesh, primitive } => {
                write!(f, "morph import: mesh {mesh} primitive {primitive} has a sparse accessor")
            }
            Self::CountMismatch {
                mesh,
                primitive,
                target,
                expected,
                got,
            } => write!(
                f,
                "morph import: mesh {mesh} primitive {primitive} target {target} has {got} deltas for {expected} vertices"
            ),
            Self::WeightCount {
                mesh,
                expected,
                got,
            } => write!(
                f,
                "morph import: mesh {mesh} has {got} weights for {expected} targets"
            ),
            Self::NonFinite {
                mesh,
                primitive,
                target,
            } => write!(
                f,
                "morph import: mesh {mesh} primitive {primitive} target {target:?} has a non-finite value"
            ),
            Self::TooLarge { mesh, primitive } => {
                write!(f, "morph import: mesh {mesh} primitive {primitive} is too large")
            }
        }
    }
}

impl std::error::Error for MorphImportError {}

/// `extras.targetNames` of a mesh: the strings of that array, empty when absent or malformed.
fn target_names(mesh: &gltf::Mesh<'_>) -> Vec<String> {
    let Some(raw) = mesh.extras().as_ref() else {
        return Vec::new();
    };
    let Ok(value) = gltf::json::deserialize::from_str::<gltf::json::Value>(raw.get()) else {
        return Vec::new();
    };
    value["targetNames"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .map(|n| n.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn all_finite(values: &[[f32; 3]]) -> bool {
    values.iter().flatten().all(|v| v.is_finite())
}

/// One primitive's morph mesh; `None` when it has no targets.
fn import_primitive(
    buffers: &[gltf::buffer::Data],
    mesh: &gltf::Mesh<'_>,
    primitive: &gltf::Primitive<'_>,
) -> Result<Option<MorphMesh>, MorphImportError> {
    let (m, p) = (mesh.index(), primitive.index());
    let count = primitive.morph_targets().len();
    if count == 0 {
        return Ok(None);
    }
    if count > MAX_TARGETS {
        return Err(MorphImportError::TooLarge {
            mesh: m,
            primitive: p,
        });
    }
    let sparse = |a: Option<gltf::Accessor<'_>>| a.is_some_and(|a| a.sparse().is_some());
    if sparse(primitive.get(&gltf::mesh::Semantic::Positions))
        || primitive.morph_targets().any(|t| sparse(t.positions()))
    {
        return Err(MorphImportError::SparseAccessor {
            mesh: m,
            primitive: p,
        });
    }
    let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| &*d.0));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| MorphImportError::Parse {
            detail: "primitive has no POSITION".to_string(),
        })?
        .collect();
    if positions.len() > MAX_VERTICES {
        return Err(MorphImportError::TooLarge {
            mesh: m,
            primitive: p,
        });
    }
    if !all_finite(&positions) {
        return Err(MorphImportError::NonFinite {
            mesh: m,
            primitive: p,
            target: None,
        });
    }
    let indices: Vec<u32> = match reader.read_indices() {
        Some(read) => read.into_u32().collect(),
        None => (0..u32::try_from(positions.len()).unwrap_or(u32::MAX)).collect(),
    };
    let names = target_names(mesh);
    let mut targets = Vec::with_capacity(count);
    for (i, (deltas, _, _)) in reader.read_morph_targets().enumerate() {
        // A target without POSITION moves nothing.
        let position_deltas: Vec<[f32; 3]> = match deltas {
            Some(read) => read.collect(),
            None => vec![[0.0; 3]; positions.len()],
        };
        if position_deltas.len() != positions.len() {
            return Err(MorphImportError::CountMismatch {
                mesh: m,
                primitive: p,
                target: i,
                expected: positions.len(),
                got: position_deltas.len(),
            });
        }
        if !all_finite(&position_deltas) {
            return Err(MorphImportError::NonFinite {
                mesh: m,
                primitive: p,
                target: Some(i),
            });
        }
        let name = match names.get(i) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => format!("target_{i}"),
        };
        targets.push(MorphTarget {
            name,
            position_deltas,
        });
    }
    let default_weights = match mesh.weights() {
        Some(w) if w.len() == targets.len() => w.to_vec(),
        Some(w) => {
            return Err(MorphImportError::WeightCount {
                mesh: m,
                expected: targets.len(),
                got: w.len(),
            })
        }
        None => vec![0.0; targets.len()],
    };
    Ok(Some(MorphMesh {
        positions,
        indices,
        targets,
        default_weights,
    }))
}

/// Read every morphed primitive of a `.gltf` text or `.glb` binary file.
pub fn import_morph_meshes(bytes: &[u8]) -> Result<Vec<MorphMesh>, MorphImportError> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(MorphImportError::BadSize { count: bytes.len() });
    }
    let parsed = gltf::Gltf::from_slice(bytes).map_err(|error| match &error {
        gltf::Error::Validation(errors) => match required_extension_name(errors) {
            Some(name) => MorphImportError::UnsupportedExtension {
                name: bound_detail(&name),
            },
            None => MorphImportError::Parse {
                detail: bound_detail(&format!("{error:?}")),
            },
        },
        _ => MorphImportError::Parse {
            detail: bound_detail(&format!("{error:?}")),
        },
    })?;
    if let Some(name) = parsed.document.extensions_required().next() {
        return Err(MorphImportError::UnsupportedExtension {
            name: bound_detail(name),
        });
    }
    let buffers = gltf::import_buffers(&parsed.document, None, parsed.blob).map_err(|error| {
        MorphImportError::MissingBuffer {
            detail: bound_detail(&format!("{error:?}")),
        }
    })?;
    let mut out = Vec::new();
    for mesh in parsed.document.meshes() {
        for primitive in mesh.primitives() {
            out.extend(import_primitive(&buffers, &mesh, &primitive)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard base64 with padding, 20 lines and no dependency.
    fn b64(bytes: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for (k, shift) in [18, 12, 6, 0].into_iter().enumerate() {
                let live = k <= c.len();
                out.push(if live {
                    A[((n >> shift) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    }

    fn f32s(values: &[[f32; 3]]) -> Vec<u8> {
        values
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    /// A one-triangle file: base positions, then one VEC3 accessor per entry of `targets`
    /// (each entry is the deltas the buffer holds and the count its accessor claims),
    /// plus the mesh JSON tail (`weights`, `extras`).
    fn gltf_text(targets: &[(Vec<[f32; 3]>, usize)], mesh_tail: &str) -> Vec<u8> {
        let mut buffer = f32s(&TRIANGLE);
        let mut views = vec!["{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36}".to_string()];
        let mut accessors = vec![
            "{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":[0,0,0],\"max\":[1,1,0]}".to_string(),
        ];
        let mut refs = Vec::new();
        for (i, (deltas, claimed)) in targets.iter().enumerate() {
            let bytes = f32s(deltas);
            views.push(format!(
                "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{}}}",
                buffer.len(),
                bytes.len()
            ));
            buffer.extend(bytes);
            accessors.push(format!(
                "{{\"bufferView\":{},\"componentType\":5126,\"count\":{claimed},\"type\":\"VEC3\"}}",
                i + 1
            ));
            refs.push(format!("{{\"POSITION\":{}}}", i + 1));
        }
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
             \"nodes\":[{{\"mesh\":0}}],\
             \"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"targets\":[{}]}}]{mesh_tail}}}],\
             \"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{}\",\"byteLength\":{}}}],\
             \"bufferViews\":[{}],\"accessors\":[{}]}}",
            refs.join(","),
            b64(&buffer),
            buffer.len(),
            views.join(","),
            accessors.join(",")
        )
        .into_bytes()
    }

    fn two_targets() -> Vec<(Vec<[f32; 3]>, usize)> {
        vec![(vec![[0.0, 0.0, 0.1]; 3], 3), (vec![[0.2, 0.0, 0.0]; 3], 3)]
    }

    #[test]
    fn two_named_targets_come_back_exact() {
        let tail = ",\"weights\":[0.0,0.5],\"extras\":{\"targetNames\":[\"smile\",\"wide\"]}";
        let meshes = import_morph_meshes(&gltf_text(&two_targets(), tail)).expect("imports");
        assert_eq!(meshes.len(), 1);
        let mesh = &meshes[0];
        assert_eq!(mesh.positions, TRIANGLE.to_vec());
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.default_weights, vec![0.0, 0.5]);
        let names: Vec<&str> = mesh.targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["smile", "wide"]);
        assert_eq!(mesh.targets[0].position_deltas, vec![[0.0, 0.0, 0.1]; 3]);
        assert_eq!(mesh.targets[1].position_deltas, vec![[0.2, 0.0, 0.0]; 3]);
        // Same bytes, same result.
        assert_eq!(
            meshes,
            import_morph_meshes(&gltf_text(&two_targets(), tail)).unwrap()
        );
    }

    #[test]
    fn missing_names_and_weights_fall_back() {
        let meshes = import_morph_meshes(&gltf_text(&two_targets(), "")).expect("imports");
        let names: Vec<&str> = meshes[0].targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["target_0", "target_1"]);
        assert_eq!(meshes[0].default_weights, vec![0.0, 0.0]);
        // One name for two targets: the second falls back by its index.
        let tail = ",\"extras\":{\"targetNames\":[\"smile\"]}";
        let meshes = import_morph_meshes(&gltf_text(&two_targets(), tail)).expect("imports");
        assert_eq!(meshes[0].targets[0].name, "smile");
        assert_eq!(meshes[0].targets[1].name, "target_1");
    }

    #[test]
    fn a_mesh_without_targets_is_skipped() {
        let text = String::from_utf8(gltf_text(&two_targets(), "")).unwrap();
        let plain = text.replace(",\"targets\":[{\"POSITION\":1},{\"POSITION\":2}]", "");
        assert_ne!(plain, text, "the fixture really lost its targets");
        assert_eq!(import_morph_meshes(plain.as_bytes()), Ok(Vec::new()));
    }

    #[test]
    fn a_delta_count_mismatch_is_its_own_error() {
        let mut targets = two_targets();
        targets[1] = (vec![[0.2, 0.0, 0.0]; 3], 2);
        assert_eq!(
            import_morph_meshes(&gltf_text(&targets, "")),
            Err(MorphImportError::CountMismatch {
                mesh: 0,
                primitive: 0,
                target: 1,
                expected: 3,
                got: 2
            })
        );
        let tail = ",\"weights\":[0.0]";
        assert_eq!(
            import_morph_meshes(&gltf_text(&two_targets(), tail)),
            Err(MorphImportError::WeightCount {
                mesh: 0,
                expected: 2,
                got: 1
            })
        );
    }

    #[test]
    fn nan_data_is_its_own_error() {
        let mut targets = two_targets();
        targets[0].0[1][2] = f32::NAN;
        assert_eq!(
            import_morph_meshes(&gltf_text(&targets, "")),
            Err(MorphImportError::NonFinite {
                mesh: 0,
                primitive: 0,
                target: Some(0)
            })
        );
    }

    #[test]
    fn a_sparse_target_accessor_is_its_own_error() {
        // Target 0 becomes sparse: 1 replaced element, indices and values in two more views.
        let mut buffer = f32s(&TRIANGLE);
        buffer.extend_from_slice(&0_u16.to_le_bytes());
        buffer.extend_from_slice(&[0, 0]);
        buffer.extend(f32s(&[[0.0, 0.0, 0.5]]));
        let text = format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\
             \"nodes\":[{{\"mesh\":0}}],\
             \"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"targets\":[{{\"POSITION\":1}}]}}]}}],\
             \"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{}\",\"byteLength\":{}}}],\
             \"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36}},\
             {{\"buffer\":0,\"byteOffset\":36,\"byteLength\":2}},\
             {{\"buffer\":0,\"byteOffset\":40,\"byteLength\":12}}],\
             \"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"min\":[0,0,0],\"max\":[1,1,0]}},\
             {{\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",\"sparse\":{{\"count\":1,\
             \"indices\":{{\"bufferView\":1,\"componentType\":5123}},\"values\":{{\"bufferView\":2}}}}}}]}}",
            b64(&buffer),
            buffer.len()
        );
        assert_eq!(
            import_morph_meshes(text.as_bytes()),
            Err(MorphImportError::SparseAccessor {
                mesh: 0,
                primitive: 0
            })
        );
    }

    #[test]
    fn bad_input_is_typed_not_a_panic() {
        assert_eq!(
            import_morph_meshes(&[]),
            Err(MorphImportError::BadSize { count: 0 })
        );
        assert!(matches!(
            import_morph_meshes(b"not gltf"),
            Err(MorphImportError::Parse { .. })
        ));
        // An external buffer has no bytes in a slice import.
        let text = String::from_utf8(gltf_text(&two_targets(), "")).unwrap();
        let start = text.find("data:application/octet-stream;base64,").unwrap();
        let end = text[start..].find('"').unwrap() + start;
        let external = format!("{}other.bin{}", &text[..start], &text[end..]);
        assert!(matches!(
            import_morph_meshes(external.as_bytes()),
            Err(MorphImportError::MissingBuffer { .. })
        ));
    }
}
