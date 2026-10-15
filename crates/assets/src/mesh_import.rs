//! mesh_import: glTF bytes to [`CookedMesh`]/[`CookedMaterial`].
//!
//! Consumer: `engine-render` uploads [`CookedMesh`]/[`CookedMaterial`]; the
//! cooker/registry owns identity. Units: meters in, meters out; no scale is
//! ever guessed. UVs are clamped to `0..=1` for the cooked contract; PBR
//! factors use the glTF defaults (`base_color [1,1,1,1]`, `metallic 1.0`,
//! `roughness 1.0`) when the file omits them.
//!
//! Documented fallbacks (only when the source attribute is absent entirely):
//! missing `TEXCOORD_0` is filled with `(0,0)`; missing `NORMAL` is computed
//! as area-weighted smooth face normals (`+Z` for isolated verts); missing
//! `TANGENT` is `+X` orthogonalized against the normal with `w = 1.0`.
//! Missing indices become the sequence `0..verts`. Anything else malformed
//! (bad container, external buffer with no bytes, sparse accessors,
//! non-triangle modes, required extensions we cannot interpret, out-of-range
//! indices, non-finite data) is a typed [`MeshImportError`], never a panic.
//!
//! Texture identity: [`TextureRef`]`.asset` is [`content_hash`] of the
//! texture content (encoded bytes for buffer-view images, URI text for
//! external/data-URI references), so reimporting identical bytes keeps the
//! same ids. This mints no registry ids and changes no registry semantics:
//! identity stays with the registry, version with the digest (see
//! `ARCHITECTURE.md` identity vs version).
//!
//! The document is parsed with glTF validation on, so schema-level problems
//! (absent `POSITION`, missing `min`/`max`, dangling indices) surface as
//! [`MeshImportError::Parse`]: still typed, bounded and panic-free.

use super::cooked::{validate_material, validate_mesh, CookedMaterial, CookedMesh, TextureRef};
use super::cooked::{CookError, TextureSlot};
use super::registry::content_hash;
use super::wire::bound_chars;
use std::collections::BTreeMap;
use std::fmt;

/// Max accepted input size (16 MiB); larger inputs are rejected bounded.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Max vertices per primitive; matches the `polyBudget` gate scale.
pub const MAX_VERTICES: usize = 1_000_000;
/// Max primitives per file; bounds total work.
pub const MAX_PRIMITIVES: usize = 1024;
/// Bound for diagnostic strings carried in errors.
pub const MAX_DIAGNOSTIC_CHARS: usize = 256;
/// Bound for echoed file paths carried in errors.
pub const MAX_PATH_CHARS: usize = 128;

/// Why a glTF import failed; every malformed input maps to one variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshImportError {
    /// File path does not end in `.gltf` or `.glb`.
    UnsupportedFileExtension {
        /// Truncated path (bounded).
        path: String,
    },
    /// Container or JSON parse/validation failed; detail bounded.
    Parse {
        /// Truncated parser message.
        detail: String,
    },
    /// File requires a glTF extension this importer does not interpret.
    UnsupportedExtension {
        /// Truncated extension name.
        name: String,
    },
    /// A buffer or image reference has no resolvable bytes in a slice import.
    MissingBuffer {
        /// Truncated detail naming the missing reference.
        detail: String,
    },
    /// The file contains no meshes to import.
    NoMeshes,
    /// A primitive is not triangles (only triangles cook to indexed meshes).
    UnsupportedMode {
        /// Mesh index in document order.
        mesh: usize,
        /// Primitive index within the mesh.
        primitive: usize,
    },
    /// A sparse accessor needs de-sparsifying this importer does not do.
    SparseUnsupported {
        /// `positions`, `normals`, `uv0`, `tangents` or `indices`.
        field: &'static str,
    },
    /// A primitive exceeds the vertex or primitive budget.
    TooLarge {
        /// Offending vertex or primitive count.
        count: usize,
    },
    /// Data failed cooked validation; field plus bounded reason.
    InvalidData {
        /// `positions`, `normals`, `uv0`, `tangents`, `indices` or `material`.
        field: &'static str,
        /// Bounded reason (never echoes unbounded input).
        detail: String,
    },
}

impl fmt::Display for MeshImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFileExtension { path } => {
                write!(f, "gltf import {path}: want .gltf or .glb")
            }
            Self::Parse { detail } => write!(f, "gltf import failed to parse: {detail}"),
            Self::UnsupportedExtension { name } => {
                write!(f, "gltf import needs unsupported extension {name}")
            }
            Self::MissingBuffer { detail } => {
                write!(f, "gltf import missing buffer: {detail}")
            }
            Self::NoMeshes => write!(f, "gltf import found no meshes"),
            Self::UnsupportedMode { mesh, primitive } => write!(
                f,
                "gltf import mesh {mesh} primitive {primitive} is not triangles"
            ),
            Self::SparseUnsupported { field } => {
                write!(f, "gltf import sparse accessor unsupported: {field}")
            }
            Self::TooLarge { count } => write!(f, "gltf import too large: {count}"),
            Self::InvalidData { field, detail } => {
                write!(f, "gltf import bad {field}: {detail}")
            }
        }
    }
}

impl std::error::Error for MeshImportError {}

/// One import run: parallel `meshes[i]` plus `materials[i]` per primitive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfImport {
    /// One cooked mesh per imported triangle primitive, document order.
    pub meshes: Vec<CookedMesh>,
    /// One cooked material per mesh (glTF defaults when absent).
    pub materials: Vec<CookedMaterial>,
    /// Encoded base-color image bytes keyed by texture asset id.
    ///
    /// Consumer: `engine-game` kit staging decodes one entry per file into
    /// `TextureUpload`. Units: encoded PNG/JPEG bytes as stored. Empty when
    /// no primitive has a base-color texture or when the image is an
    /// external URI with no bytes in a slice import. One entry per distinct
    /// image (shared images dedupe); iteration is `BTreeMap` order, so the
    /// same bytes give the same map. Cold import only, bounded by
    /// [`MAX_BYTES`].
    pub images: BTreeMap<u64, Vec<u8>>,
}

/// Stable texture id for raw content bytes: the crate content-hash canon, so
/// identical bytes keep the same id across reimports.
pub fn texture_asset_id(bytes: &[u8]) -> u64 {
    content_hash(bytes)
}

/// Truncate a diagnostic to at most `MAX_DIAGNOSTIC_CHARS` chars.
pub(crate) fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_DIAGNOSTIC_CHARS)
}

/// Truncate an echoed path to at most `MAX_PATH_CHARS` chars.
fn bound_path(path: &str) -> String {
    bound_chars(path, MAX_PATH_CHARS)
}

fn invalid(field: &'static str, detail: &str) -> MeshImportError {
    MeshImportError::InvalidData {
        field,
        detail: bound_detail(detail),
    }
}

/// Import glTF bytes (`.gltf` text or `.glb` binary) into cooked data.
pub fn import_gltf_bytes(bytes: &[u8]) -> Result<GltfImport, MeshImportError> {
    if bytes.len() > MAX_BYTES {
        return Err(MeshImportError::TooLarge { count: bytes.len() });
    }
    if bytes.is_empty() {
        return Err(MeshImportError::Parse {
            detail: "empty input".to_string(),
        });
    }
    let parsed = gltf::Gltf::from_slice(bytes).map_err(|e| match &e {
        // Unknown *required* extensions fail validation: surface the
        // extension name instead of a raw validation dump. Anything else
        // stays a bounded parse error.
        gltf::Error::Validation(errors) => match required_extension_name(errors) {
            Some(name) => MeshImportError::UnsupportedExtension {
                name: bound_detail(&name),
            },
            None => MeshImportError::Parse {
                detail: bound_detail(&format!("{e:?}")),
            },
        },
        _ => MeshImportError::Parse {
            detail: bound_detail(&format!("{e:?}")),
        },
    })?;
    import_document(&parsed.document, parsed.blob)
}

/// Import glTF bytes with a file-extension gate (`.gltf`/`.glb` only).
pub fn import_gltf_path(path: &str, bytes: &[u8]) -> Result<GltfImport, MeshImportError> {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".gltf") || lower.ends_with(".glb")) {
        return Err(MeshImportError::UnsupportedFileExtension {
            path: bound_path(path),
        });
    }
    import_gltf_bytes(bytes)
}

/// Name of the first required extension flagged by validation, if any.
/// The debug row looks like `Path("extensionsRequired[0] = \"KHR_foo\"")`;
/// only ASCII name characters survive the filter, and the result is bounded
/// by the caller.
pub(crate) fn required_extension_name(
    errors: &[(gltf::json::Path, gltf::json::validation::Error)],
) -> Option<String> {
    for (path, _) in errors {
        let text = format!("{path:?}");
        if let Some(pos) = text.find("extensionsRequired") {
            let tail = &text[pos..];
            if let Some(eq) = tail.find('=') {
                let name: String = tail[eq + 1..]
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            return Some("unknown-required-extension".to_string());
        }
    }
    None
}

fn import_document(
    document: &gltf::Document,
    blob: Option<Vec<u8>>,
) -> Result<GltfImport, MeshImportError> {
    // Defense in depth: validation already rejects unknown required
    // extensions (mapped above), but a future feature set could let one
    // through, and required means we must not silently ignore it.
    if let Some(name) = document.extensions_required().next() {
        return Err(MeshImportError::UnsupportedExtension {
            name: bound_detail(name),
        });
    }
    let buffers = gltf::import_buffers(document, None, blob).map_err(|e| match e {
        gltf::Error::ExternalReferenceInSliceImport
        | gltf::Error::UnsupportedScheme
        | gltf::Error::MissingBlob => MeshImportError::MissingBuffer {
            detail: bound_detail(&format!("{e:?}")),
        },
        gltf::Error::BufferLength {
            buffer,
            expected,
            actual,
        } => MeshImportError::MissingBuffer {
            detail: bound_detail(&format!("buffer {buffer}: {actual} of {expected} bytes")),
        },
        other => MeshImportError::Parse {
            detail: bound_detail(&format!("{other:?}")),
        },
    })?;
    let primitive_count: usize = document.meshes().map(|m| m.primitives().len()).sum();
    if primitive_count == 0 {
        return Err(MeshImportError::NoMeshes);
    }
    if primitive_count > MAX_PRIMITIVES {
        return Err(MeshImportError::TooLarge {
            count: primitive_count,
        });
    }
    let mut out = GltfImport::default();
    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                return Err(MeshImportError::UnsupportedMode {
                    mesh: mesh.index(),
                    primitive: primitive.index(),
                });
            }
            let (cooked, material) = import_primitive(document, &buffers, &primitive)?;
            // Expose the base-color bytes next to the texture ids: one entry
            // per distinct image, first wins, so shared images clone once.
            if let Some((id, slice)) =
                base_color_image_slice(document, &buffers, &primitive.material())
            {
                out.images.entry(id).or_insert_with(|| slice.to_vec());
            }
            out.meshes.push(cooked);
            out.materials.push(material);
        }
    }
    if out.meshes.is_empty() {
        return Err(MeshImportError::NoMeshes);
    }
    Ok(out)
}

fn import_primitive(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    primitive: &gltf::Primitive<'_>,
) -> Result<(CookedMesh, CookedMaterial), MeshImportError> {
    reject_sparse(primitive)?;
    let get_buffer = |buffer: gltf::Buffer<'_>| buffers.get(buffer.index()).map(|data| &*data.0);
    let reader = primitive.reader(get_buffer);

    // The document was parsed with validation, so a missing POSITION is
    // rejected as `Parse` before we get here; a `None` here would mean the
    // validator relaxed, so report it as invalid data rather than panicking.
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| invalid("positions", "no POSITION accessor"))?
        .collect();
    if positions.is_empty() {
        return Err(invalid("positions", "no vertices"));
    }
    if positions.len() > MAX_VERTICES {
        return Err(MeshImportError::TooLarge {
            count: positions.len(),
        });
    }
    if positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(invalid("positions", "non-finite value"));
    }
    let count = positions.len();

    let indices: Vec<u32> = match reader.read_indices() {
        Some(read) => read.into_u32().collect(),
        None => (0..count as u32).collect(),
    };
    if indices.is_empty() || !indices.len().is_multiple_of(3) {
        return Err(invalid("indices", "empty or not triangles"));
    }
    let bound = count as u32;
    for index in &indices {
        if *index >= bound {
            return Err(invalid("indices", "index out of range"));
        }
    }

    let normals: Vec<[f32; 3]> = match reader.read_normals() {
        Some(read) => {
            let values: Vec<[f32; 3]> = read.collect();
            if values.len() != count {
                return Err(invalid("normals", "length differs from positions"));
            }
            if values.iter().flatten().any(|v| !v.is_finite()) {
                return Err(invalid("normals", "non-finite value"));
            }
            values
        }
        None => fallback_normals(&positions, &indices),
    };

    let uv0: Vec<[f32; 2]> = match reader.read_tex_coords(0) {
        Some(read) => {
            let mut values: Vec<[f32; 2]> = read.into_f32().collect();
            if values.len() != count {
                return Err(invalid("uv0", "length differs from positions"));
            }
            for uv in &values {
                if uv.iter().any(|v| !v.is_finite()) {
                    return Err(invalid("uv0", "non-finite value"));
                }
            }
            // Cooked contract is `0..=1`; clamp source UVs deterministically.
            for uv in &mut values {
                uv[0] = uv[0].clamp(0.0, 1.0);
                uv[1] = uv[1].clamp(0.0, 1.0);
            }
            values
        }
        None => vec![[0.0, 0.0]; count],
    };

    let tangents: Vec<[f32; 4]> = match reader.read_tangents() {
        Some(read) => {
            let values: Vec<[f32; 4]> = read.collect();
            if values.len() != count {
                return Err(invalid("tangents", "length differs from positions"));
            }
            if values
                .iter()
                .any(|t| !t.iter().all(|v| v.is_finite()) || (t[3] != 1.0 && t[3] != -1.0))
            {
                return Err(invalid("tangents", "bad handedness or non-finite"));
            }
            values
        }
        // Documented fallback: +X orthogonalized against the normal, w = 1.
        None => fallback_tangents(&normals),
    };

    let cooked = CookedMesh {
        positions,
        normals,
        uv0,
        tangents,
        indices,
    };
    validate_mesh(&cooked).map_err(|e| cook_to_invalid(&e))?;

    let material = import_material(document, buffers, &primitive.material())?;
    Ok((cooked, material))
}

/// Reject sparse accessors: silently de-sparsifying would guess at data.
fn reject_sparse(primitive: &gltf::Primitive<'_>) -> Result<(), MeshImportError> {
    use gltf::mesh::Semantic;
    for (semantic, accessor) in primitive.attributes() {
        if accessor.sparse().is_some() {
            let field = match semantic {
                Semantic::Positions => "positions",
                Semantic::Normals => "normals",
                Semantic::Tangents => "tangents",
                Semantic::TexCoords(_) => "uv0",
                _ => continue,
            };
            return Err(MeshImportError::SparseUnsupported { field });
        }
    }
    if let Some(indices) = primitive.indices() {
        if indices.sparse().is_some() {
            return Err(MeshImportError::SparseUnsupported { field: "indices" });
        }
    }
    Ok(())
}

fn cook_to_invalid(error: &CookError) -> MeshImportError {
    match error {
        CookError::EmptyMesh => invalid("positions", "no vertices"),
        CookError::LengthMismatch { field, .. } => invalid(field_name(field), "length mismatch"),
        CookError::IndexOutOfRange(_) => invalid("indices", "index out of range"),
        CookError::NonFinite(field) => invalid(field_name(field), "non-finite value"),
        CookError::BadFactor { field, .. } => invalid("material", &format!("bad {field} factor")),
        CookError::DuplicateSlot(_) => invalid("material", "duplicate texture slot"),
        CookError::TextureDecode(detail) => invalid("texture", &bound_detail(detail)),
        CookError::TextureTooLarge { count } => MeshImportError::TooLarge { count: *count },
        CookError::BadTextureDimensions { width, height } => {
            invalid("texture", &format!("bad dimensions {width}x{height}"))
        }
    }
}

fn field_name(field: &str) -> &'static str {
    match field {
        "normals" => "normals",
        "uv0" => "uv0",
        "tangents" => "tangents",
        "positions" => "positions",
        _ => "positions",
    }
}

fn import_material(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    material: &gltf::Material<'_>,
) -> Result<CookedMaterial, MeshImportError> {
    let pbr = material.pbr_metallic_roughness();
    let base_color_factor = pbr.base_color_factor();
    let metallic_factor = pbr.metallic_factor();
    let roughness_factor = pbr.roughness_factor();
    for (field, value) in [
        ("base_color", base_color_factor[0]),
        ("base_color", base_color_factor[1]),
        ("base_color", base_color_factor[2]),
        ("base_color", base_color_factor[3]),
        ("metallic", metallic_factor),
        ("roughness", roughness_factor),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(invalid("material", &format!("bad {field} factor")));
        }
    }
    let mut textures = Vec::new();
    if let Some(info) = pbr.base_color_texture() {
        if let Some(asset) = texture_image_asset(document, buffers, &info.texture()) {
            textures.push(TextureRef {
                asset,
                slot: TextureSlot::BaseColor,
            });
        }
    }
    if let Some(info) = pbr.metallic_roughness_texture() {
        if let Some(asset) = texture_image_asset(document, buffers, &info.texture()) {
            textures.push(TextureRef {
                asset,
                slot: TextureSlot::MetallicRoughness,
            });
        }
    }
    if let Some(info) = material.normal_texture() {
        if let Some(asset) = texture_image_asset(document, buffers, &info.texture()) {
            textures.push(TextureRef {
                asset,
                slot: TextureSlot::Normal,
            });
        }
    }
    let cooked = CookedMaterial {
        base_color_factor,
        metallic_factor,
        roughness_factor,
        textures,
    };
    validate_material(&cooked).map_err(|e| cook_to_invalid(&e))?;
    Ok(cooked)
}

/// Stable id plus encoded bytes for a material's base-color image: the slice
/// borrows the buffer so the caller clones once per distinct image. Returns
/// `None` when the material has no base-color texture, when the texture
/// names no image, or when the image is an external URI with no bytes in a
/// slice import (the [`TextureRef`] id still covers the URI text).
fn base_color_image_slice<'a>(
    document: &gltf::Document,
    buffers: &'a [gltf::buffer::Data],
    material: &gltf::Material<'_>,
) -> Option<(u64, &'a [u8])> {
    let info = material.pbr_metallic_roughness().base_color_texture()?;
    let texture = info.texture();
    let wanted = texture.source().index();
    let image = document.images().nth(wanted)?;
    match image.source() {
        gltf::image::Source::View { view, .. } => {
            let data = buffers.get(view.buffer().index())?;
            let bytes = data
                .0
                .get(view.offset()..view.offset().checked_add(view.length())?)?;
            Some((content_hash(bytes), bytes))
        }
        gltf::image::Source::Uri { .. } => None,
    }
}

/// Stable id for a texture's image: content hash of the encoded bytes when
/// the image lives in a buffer view, else of the URI text. Returns `None`
/// when the texture names no image (valid glTF may omit the source).
fn texture_image_asset(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    texture: &gltf::Texture<'_>,
) -> Option<u64> {
    // `Texture::source` unwraps a validated image index (the document was
    // parsed with validation), so the local invariant holds here; the
    // defensive re-resolution below skips rather than panics if it ever
    // does not.
    let wanted = texture.source().index();
    let image = document.images().nth(wanted)?;
    match image.source() {
        gltf::image::Source::View { view, .. } => {
            let data = buffers.get(view.buffer().index())?;
            let bytes = data
                .0
                .get(view.offset()..view.offset().checked_add(view.length())?)?;
            Some(content_hash(bytes))
        }
        gltf::image::Source::Uri { uri, .. } => Some(content_hash(uri.as_bytes())),
    }
}

/// Area-weighted smooth normals; zero-area faces contribute nothing and
/// isolated verts face `+Z`. Deterministic in index order.
fn fallback_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0f32; 3]; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let (Some(p0), Some(p1), Some(p2)) = (positions.get(a), positions.get(b), positions.get(c))
        else {
            continue;
        };
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let n = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for (idx, weight) in [(a, n), (b, n), (c, n)] {
            if let Some(out) = normals.get_mut(idx) {
                out[0] += weight[0];
                out[1] += weight[1];
                out[2] += weight[2];
            }
        }
    }
    for normal in &mut normals {
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if len.is_finite() && len > 1e-12 {
            normal[0] /= len;
            normal[1] /= len;
            normal[2] /= len;
        } else {
            *normal = [0.0, 0.0, 1.0];
        }
    }
    normals
}

/// `+X` (or `+Z` when the normal is near `+X`) orthogonalized against the
/// normal, `w = 1.0`. Deterministic per vertex.
fn fallback_tangents(normals: &[[f32; 3]]) -> Vec<[f32; 4]> {
    normals
        .iter()
        .map(|n| {
            let reference = if n[0].abs() > 0.9 {
                [0.0, 0.0, 1.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let dot = reference[0] * n[0] + reference[1] * n[1] + reference[2] * n[2];
            let mut t = [
                reference[0] - n[0] * dot,
                reference[1] - n[1] * dot,
                reference[2] - n[2] * dot,
            ];
            let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            if len.is_finite() && len > 1e-12 {
                t[0] /= len;
                t[1] /= len;
                t[2] /= len;
            } else {
                t = [1.0, 0.0, 0.0];
            }
            [t[0], t[1], t[2], 1.0]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::cooked::{validate_material, validate_mesh};
    use super::*;

    const TRIANGLE: &[u8] = include_bytes!("../../../fixtures/khronos/Triangle.gltf");
    const TRIANGLE_PLAIN: &[u8] =
        include_bytes!("../../../fixtures/khronos/TriangleWithoutIndices.gltf");
    const TRIANGLE_ANIM: &[u8] = include_bytes!("../../../fixtures/khronos/AnimatedTriangle.gltf");

    fn assert_cooked(imported: &GltfImport, label: &str) {
        assert!(!imported.meshes.is_empty(), "{label} must yield meshes");
        assert_eq!(
            imported.meshes.len(),
            imported.materials.len(),
            "{label} meshes/materials pair up"
        );
        for mesh in &imported.meshes {
            validate_mesh(mesh).unwrap_or_else(|e| panic!("{label} mesh invalid: {e:?}"));
            assert!(!mesh.positions.is_empty(), "{label} needs positions");
            assert!(!mesh.normals.is_empty(), "{label} needs normals");
            assert!(!mesh.uv0.is_empty(), "{label} needs uv0");
            assert!(!mesh.tangents.is_empty(), "{label} needs tangents");
            assert!(!mesh.indices.is_empty(), "{label} needs indices");
        }
        for material in &imported.materials {
            validate_material(material)
                .unwrap_or_else(|e| panic!("{label} material invalid: {e:?}"));
            assert!(
                material.base_color_factor.iter().all(|v| v.is_finite()),
                "{label} PBR factors finite"
            );
            assert!(
                material.metallic_factor.is_finite(),
                "{label} metallic finite"
            );
            assert!(
                material.roughness_factor.is_finite(),
                "{label} roughness finite"
            );
        }
    }

    #[test]
    fn khronos_fixtures_import() {
        for (label, bytes) in [
            ("Triangle", TRIANGLE),
            ("TriangleWithoutIndices", TRIANGLE_PLAIN),
            ("AnimatedTriangle", TRIANGLE_ANIM),
        ] {
            let imported = import_gltf_bytes(bytes)
                .unwrap_or_else(|e| panic!("fixture {label} must import: {e}"));
            assert_cooked(&imported, label);
        }
    }

    #[test]
    fn fixtures_import_through_path_gate() {
        let imported = import_gltf_path("mesh/Triangle.gltf", TRIANGLE).expect("gltf path ok");
        assert_cooked(&imported, "path Triangle");
        let upper = import_gltf_path("mesh/HERO.GLB", &glb_of(TRIANGLE)).expect("glb path ok");
        assert_cooked(&upper, "path glb");
    }

    #[test]
    fn unsupported_file_extension_rejected() {
        let err = import_gltf_path("mesh/hero.fbx", TRIANGLE).unwrap_err();
        assert!(
            matches!(err, MeshImportError::UnsupportedFileExtension { .. }),
            "want extension error, got {err}"
        );
        assert!(err.to_string().len() <= MAX_DIAGNOSTIC_CHARS + 64);
    }

    #[test]
    fn malformed_files_rejected() {
        let oversize = vec![0u8; MAX_BYTES + 1];
        for (label, bytes) in [
            ("empty", b"".as_slice()),
            ("garbage", b"not gltf at all".as_slice()),
            ("truncated-json", b"{\"asset\":".as_slice()),
            (
                "wrong-version",
                br#"{"asset":{"version":"1.0"}}"#.as_slice(),
            ),
            (
                "no-meshes",
                br#"{"asset":{"version":"2.0"},"scene":0}"#.as_slice(),
            ),
            ("oversize", oversize.as_slice()),
        ] {
            let err = import_gltf_bytes(bytes).expect_err(&format!("{label} must fail"));
            assert!(!format!("{err}").is_empty(), "{label} names the reason");
            assert!(
                format!("{err:?}").chars().count() <= MAX_DIAGNOSTIC_CHARS + 128,
                "{label} diagnostics bounded"
            );
        }
    }

    #[test]
    fn missing_buffer_rejected_without_panic() {
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
            "buffers":[
                {"uri":"missing-external.bin","byteLength":36},
                {"uri":"data:application/octet-stream;base64,AAABAAIAAAA=","byteLength":6}
            ],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":36},
                {"buffer":1,"byteOffset":0,"byteLength":6}
            ],
            "accessors":[
                {"bufferView":0,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC3",
                    "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]},
                {"bufferView":1,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"SCALAR"}
            ]
        }"#;
        let err = import_gltf_bytes(text.as_bytes()).expect_err("external buffer must fail");
        assert_eq!(
            err,
            MeshImportError::MissingBuffer {
                detail: err_detail(&err),
            },
            "missing buffer maps to MissingBuffer"
        );
    }

    #[test]
    fn missing_positions_rejected() {
        // NORMAL without POSITION reuses the Triangle buffer bytes: schema
        // validation rejects the absent required accessor with a typed error.
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"NORMAL":1},"indices":0}]}],
            "buffers":[{"uri":"data:application/octet-stream;base64,AAABAAIAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPwAAAAA=","byteLength":44}],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":6},
                {"buffer":0,"byteOffset":8,"byteLength":36}
            ],
            "accessors":[
                {"bufferView":0,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"SCALAR"},
                {"bufferView":1,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC3"}
            ]
        }"#;
        let err = import_gltf_bytes(text.as_bytes()).expect_err("missing POSITION must fail");
        assert!(
            matches!(err, MeshImportError::Parse { .. }),
            "want typed Parse for missing POSITION, got {err}"
        );
    }

    #[test]
    fn unsupported_required_extension_rejected() {
        // Otherwise-valid file that requires an unknown extension: the name
        // must surface as `UnsupportedExtension`, never be silently ignored.
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "extensionsRequired":["KHR_future_guess"],
            "extensionsUsed":["KHR_future_guess"],
            "nodes":[{"mesh":0}],
            "meshes":[{"primitives":[
                {"attributes":{"POSITION":1},"indices":0}
            ]}],
            "buffers":[{"uri":"data:application/octet-stream;base64,AAABAAIAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPwAAAAA=","byteLength":44}],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":6},
                {"buffer":0,"byteOffset":8,"byteLength":36}
            ],
            "accessors":[
                {"bufferView":0,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"SCALAR"},
                {"bufferView":1,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC3",
                    "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}
            ]
        }"#;
        let err = import_gltf_bytes(text.as_bytes()).expect_err("required extension must fail");
        match err {
            MeshImportError::UnsupportedExtension { name } => {
                assert!(
                    name.contains("KHR_future_guess"),
                    "names the extension, got {name}"
                );
            }
            other => panic!("want UnsupportedExtension, got {other}"),
        }
    }

    #[test]
    fn pbr_factors_and_texture_refs_import() {
        // Material with explicit factors plus three texture slots over two
        // buffer-view images (encoded bytes are hashed, never decoded).
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],
            "meshes":[{"primitives":[
                {"attributes":{"POSITION":1},"indices":0,"material":0}
            ]}],
            "materials":[{
                "pbrMetallicRoughness":{
                    "baseColorFactor":[0.5,0.25,0.125,1.0],
                    "metallicFactor":0.2,"roughnessFactor":0.8,
                    "baseColorTexture":{"index":0},
                    "metallicRoughnessTexture":{"index":1}},
                "normalTexture":{"index":0}}],
            "textures":[{"source":0},{"source":1}],
            "images":[
                {"bufferView":2,"mimeType":"image/png"},
                {"bufferView":3,"mimeType":"image/png"}],
            "buffers":[
                {"uri":"data:application/octet-stream;base64,AAABAAIAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPwAAAAA=","byteLength":44},
                {"uri":"data:application/octet-stream;base64,AQIDBA==","byteLength":4},
                {"uri":"data:application/octet-stream;base64,BQYHCA==","byteLength":4}],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":6},
                {"buffer":0,"byteOffset":8,"byteLength":36},
                {"buffer":1,"byteOffset":0,"byteLength":4},
                {"buffer":2,"byteOffset":0,"byteLength":4}],
            "accessors":[
                {"bufferView":0,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"SCALAR"},
                {"bufferView":1,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC3",
                    "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}]
        }"#;
        let bytes = text.as_bytes();
        let imported = import_gltf_bytes(bytes).expect("textured file must import");
        assert_eq!(imported.meshes.len(), 1);
        assert_cooked(&imported, "textured");
        let material = &imported.materials[0];
        assert_eq!(material.base_color_factor, [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(material.metallic_factor, 0.2);
        assert_eq!(material.roughness_factor, 0.8);
        let slot_asset = |slot| {
            material
                .textures
                .iter()
                .find(|r| r.slot == slot)
                .map(|r| r.asset)
        };
        use super::super::cooked::TextureSlot;
        // Texture ids are content digests of the encoded image bytes.
        assert_eq!(
            slot_asset(TextureSlot::BaseColor),
            Some(content_hash(&[1, 2, 3, 4]))
        );
        assert_eq!(
            slot_asset(TextureSlot::MetallicRoughness),
            Some(content_hash(&[5, 6, 7, 8]))
        );
        assert_eq!(
            slot_asset(TextureSlot::Normal),
            Some(content_hash(&[1, 2, 3, 4]))
        );
        // Reimport keeps every id.
        let again = import_gltf_bytes(bytes).expect("reimport");
        assert_eq!(imported, again);
    }

    #[test]
    fn base_color_bytes_round_trip_image_id() {
        // Same textured file as above: the exposed bytes hash back to the
        // base-color asset id, and only the base-color image is exposed.
        let text = r#"{
            "asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0}],
            "meshes":[{"primitives":[
                {"attributes":{"POSITION":1},"indices":0,"material":0}
            ]}],
            "materials":[{
                "pbrMetallicRoughness":{
                    "baseColorFactor":[0.5,0.25,0.125,1.0],
                    "metallicFactor":0.2,"roughnessFactor":0.8,
                    "baseColorTexture":{"index":0},
                    "metallicRoughnessTexture":{"index":1}},
                "normalTexture":{"index":0}}],
            "textures":[{"source":0},{"source":1}],
            "images":[
                {"bufferView":2,"mimeType":"image/png"},
                {"bufferView":3,"mimeType":"image/png"}],
            "buffers":[
                {"uri":"data:application/octet-stream;base64,AAABAAIAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAAAAAAAAACAPwAAAAA=","byteLength":44},
                {"uri":"data:application/octet-stream;base64,AQIDBA==","byteLength":4},
                {"uri":"data:application/octet-stream;base64,BQYHCA==","byteLength":4}],
            "bufferViews":[
                {"buffer":0,"byteOffset":0,"byteLength":6},
                {"buffer":0,"byteOffset":8,"byteLength":36},
                {"buffer":1,"byteOffset":0,"byteLength":4},
                {"buffer":2,"byteOffset":0,"byteLength":4}],
            "accessors":[
                {"bufferView":0,"byteOffset":0,"componentType":5123,
                    "count":3,"type":"SCALAR"},
                {"bufferView":1,"byteOffset":0,"componentType":5126,
                    "count":3,"type":"VEC3",
                    "min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]}]
        }"#;
        let bytes = text.as_bytes();
        let imported = import_gltf_bytes(bytes).expect("textured file must import");
        use super::super::cooked::TextureSlot;
        let base_id = imported.materials[0]
            .textures
            .iter()
            .find(|r| r.slot == TextureSlot::BaseColor)
            .map(|r| r.asset)
            .expect("base-color slot present");
        // Exposed bytes round-trip the image id through both hashes.
        let stored = imported
            .images
            .get(&base_id)
            .expect("base-color bytes exposed by asset id");
        assert_eq!(stored, &vec![1u8, 2, 3, 4]);
        assert_eq!(texture_asset_id(stored), base_id);
        assert_eq!(content_hash(stored), base_id);
        // Only the base-color image is exposed: metallic-roughness bytes
        // stay as ids only, and reimport keeps the map.
        assert_eq!(imported.images.len(), 1);
        let again = import_gltf_bytes(bytes).expect("reimport");
        assert_eq!(imported, again);
        // Untextured fixtures expose no bytes.
        let plain = import_gltf_bytes(TRIANGLE).expect("plain triangle imports");
        assert!(
            plain.images.is_empty(),
            "no base-color texture means no bytes"
        );
    }

    #[test]
    fn reimport_keeps_everything() {
        let first = import_gltf_bytes(TRIANGLE).expect("first import");
        let second = import_gltf_bytes(TRIANGLE).expect("second import");
        assert_eq!(first, second, "identical bytes import identically");
        // Texture ids are content digests: stable across reimports.
        assert_eq!(
            texture_asset_id(b"tex-bytes"),
            texture_asset_id(b"tex-bytes")
        );
        assert_ne!(texture_asset_id(b"tex-a"), texture_asset_id(b"tex-b"));
    }

    #[test]
    fn errors_display_bounded() {
        let long_path = "x".repeat(4096);
        let err = import_gltf_path(&format!("{long_path}.fbx"), TRIANGLE).unwrap_err();
        assert!(format!("{err}").chars().count() <= MAX_DIAGNOSTIC_CHARS + 64);
        for err in [
            MeshImportError::NoMeshes,
            MeshImportError::SparseUnsupported { field: "uv0" },
            MeshImportError::TooLarge { count: 1 },
        ] {
            assert!(!format!("{err}").is_empty());
        }
    }

    #[cfg(feature = "registry")]
    #[test]
    fn registry_reimport_keeps_asset_id() {
        use super::super::registry::AssetRegistry;
        let mut registry = AssetRegistry::new();
        let id = registry.register(TRIANGLE, &[]).expect("register fixture");
        let kept = registry.reimport(id, TRIANGLE, &[]).expect("reimport same");
        assert_eq!(kept.kept, id);
        assert!(!kept.changed, "identical bytes change nothing");
        // Imported geometry is identical too, so the artifact is stable.
        assert_eq!(
            import_gltf_bytes(TRIANGLE).expect("import"),
            import_gltf_bytes(TRIANGLE).expect("reimport")
        );
    }

    /// Wrap `.gltf` bytes in a minimal `.glb` container for path-gate tests.
    fn glb_of(gltf: &[u8]) -> Vec<u8> {
        // Pad JSON chunk to 4 bytes; BIN chunk omitted (no .bin refs here
        // because fixtures inline buffers as data URIs).
        let mut json = gltf.to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total = (12 + 8 + json.len()) as u32;
        let mut out = Vec::with_capacity(total as usize);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&total.to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(&json);
        out
    }

    fn err_detail(err: &MeshImportError) -> String {
        match err {
            MeshImportError::MissingBuffer { detail } => detail.clone(),
            _ => panic!("expected MissingBuffer, got {err}"),
        }
    }
}
