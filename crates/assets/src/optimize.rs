//! optimize: glTF-Transform document pre-pass (dedup, prune, resize-record).
//! Consumer: GS22/GS23 fetch lists plus CT35/CT41 bake via
//! `cook_normalized_bytes`; `mesh_cook::cook_mesh` takes normalized meshes.
//! Units: meshes/materials/images in, bytes plus digests out. Determinism:
//! index-order loops, BTreeMap/BTreeSet order only, no wall clock, x2 equal.
//! Errors: typed `OptimizeError`; over-budget referenced textures downsample
//! with `image` when `import` is on, else refuse. No Node, no vendored bytes.
//! PORT(donmccurdy/glTF-Transform@3797aad585021e68cf5fbf3f2fba37783596ffac:
//! packages/functions/src/dedup.ts hash-join) plus prune.ts (EPS strip).
//! PORT(donmccurdy/glTF-Transform@3797aad585021e68cf5fbf3f2fba37783596ffac:
//! packages/functions/src/prune.ts unused strip).

use super::wire::bound_chars;
use engine_core::Fnv64;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Texture pixel budget (1024x1024); referenced images over it are resized.
pub const MAX_TEXTURE_PX: usize = 1_048_576;
/// Largest accepted document byte total; larger inputs are refused.
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
/// Largest accepted mesh count; bounds the dedup work.
pub const MAX_MESHES: usize = 1024;
/// Float tolerance for factor compare (donor prune.ts EPS 3/255).
pub const EPS: f32 = 0.011_764_706;
/// Bound for diagnostics carried in errors.
pub const MAX_DETAIL_CHARS: usize = 128;

/// One mesh: positions plus indices plus first UV set plus material slot.
#[derive(Clone, Debug, PartialEq)]
pub struct DocMesh {
    /// Positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Triangle indices into the attribute arrays.
    pub indices: Vec<u32>,
    /// First UV channel in 0..=1.
    pub uvs: Vec<[f32; 2]>,
    /// Material index into the document material list.
    pub material: usize,
}

/// One material: base-color factor plus optional referenced image id.
#[derive(Clone, Debug, PartialEq)]
pub struct DocMaterial {
    /// sRGB base-color factor in 0..=1.
    pub factor: [f32; 4],
    /// Referenced image id, if the material names a texture.
    pub texture: Option<u64>,
}

/// One image: stable id plus encoded bytes plus recorded dimensions.
#[derive(Clone, Debug, PartialEq)]
pub struct DocImage {
    /// Stable content id of the encoded bytes.
    pub id: u64,
    /// Encoded image bytes as stored.
    pub bytes: Vec<u8>,
    /// Decoded width in texels.
    pub width: u32,
    /// Decoded height in texels.
    pub height: u32,
}

/// Imported document the pre-pass normalizes: meshes plus materials plus images.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Document {
    /// Meshes in document order.
    pub meshes: Vec<DocMesh>,
    /// Materials in document order.
    pub materials: Vec<DocMaterial>,
    /// Images in document order.
    pub images: Vec<DocImage>,
}

/// Why normalization failed; every refused input maps to one variant.
#[derive(Clone, Debug, PartialEq)]
pub enum OptimizeError {
    /// Input exceeds a document bound; offending count carried.
    TooLarge {
        /// Offending byte or item count.
        count: usize,
    },
    /// Referenced texture exceeds the pixel cap; dims carried.
    OverBudget {
        /// Image width in texels.
        width: u32,
        /// Image height in texels.
        height: u32,
        /// Pixel count that exceeded the cap.
        pixels: usize,
    },
    /// Over-budget texture needs the `import` feature to downsample.
    FeatureOff,
    /// Image bytes failed to decode; bounded detail carried.
    ImageDecode {
        /// Bounded decoder detail.
        detail: String,
    },
}

impl fmt::Display for OptimizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { count } => write!(f, "optimize document too large: {count}"),
            Self::OverBudget {
                width,
                height,
                pixels,
            } => {
                write!(
                    f,
                    "optimize texture over budget: {width}x{height} ({pixels}px)"
                )
            }
            Self::FeatureOff => write!(f, "optimize needs the import feature to resize"),
            Self::ImageDecode { detail } => {
                write!(
                    f,
                    "optimize image decode: {}",
                    bound_chars(detail, MAX_DETAIL_CHARS)
                )
            }
        }
    }
}

impl std::error::Error for OptimizeError {}

/// Close factors within EPS on every lane (donor prune tolerance).
fn factor_close(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x - *y).abs() <= EPS)
}

/// Cooked byte total of one document: mesh attributes plus image bytes.
pub fn document_bytes(doc: &Document) -> usize {
    let mut total = 0usize;
    for mesh in &doc.meshes {
        total = total.saturating_add(mesh.positions.len().saturating_mul(12));
        total = total.saturating_add(mesh.indices.len().saturating_mul(4));
        total = total.saturating_add(mesh.uvs.len().saturating_mul(8));
    }
    for image in &doc.images {
        total = total.saturating_add(image.bytes.len());
    }
    total
}

/// Stable digest over one document: counts plus every mesh, material, image.
pub fn normalize_digest(doc: &Document) -> u64 {
    let mut state = Fnv64::new();
    state.write_u64(u64::try_from(doc.meshes.len()).unwrap_or(u64::MAX));
    for mesh in &doc.meshes {
        state.write_u64(u64::try_from(mesh.positions.len()).unwrap_or(u64::MAX));
        for p in &mesh.positions {
            for v in p {
                state.write_f32(*v);
            }
        }
        state.write_u64(u64::try_from(mesh.indices.len()).unwrap_or(u64::MAX));
        for index in &mesh.indices {
            state.write(&index.to_le_bytes());
        }
        for uv in &mesh.uvs {
            state.write_f32(uv[0]);
            state.write_f32(uv[1]);
        }
        state.write_u64(u64::try_from(mesh.material).unwrap_or(u64::MAX));
    }
    state.write_u64(u64::try_from(doc.materials.len()).unwrap_or(u64::MAX));
    for material in &doc.materials {
        for v in &material.factor {
            state.write_f32(*v);
        }
        state.write_u64(material.texture.unwrap_or(u64::MAX));
    }
    state.write_u64(u64::try_from(doc.images.len()).unwrap_or(u64::MAX));
    let mut ids: Vec<u64> = doc.images.iter().map(|image| image.id).collect();
    ids.sort_unstable();
    for id in ids {
        if let Some(image) = doc.images.iter().find(|candidate| candidate.id == id) {
            state.write_u64(image.id);
            state.write_u32(image.width);
            state.write_u32(image.height);
            state.write(&image.bytes);
        }
    }
    state.finish()
}

/// Mesh bytes for hash-join: positions plus indices plus uvs plus material.
fn mesh_fingerprint(mesh: &DocMesh) -> u64 {
    let mut state = Fnv64::new();
    for p in &mesh.positions {
        for v in p {
            state.write_f32(*v);
        }
    }
    for index in &mesh.indices {
        state.write(&index.to_le_bytes());
    }
    for uv in &mesh.uvs {
        state.write_f32(uv[0]);
        state.write_f32(uv[1]);
    }
    state.write_u64(u64::try_from(mesh.material).unwrap_or(u64::MAX));
    state.finish()
}

/// Hash-join byte-identical meshes; first-seen order wins.
fn dedup_meshes(meshes: &[DocMesh]) -> Vec<DocMesh> {
    let mut unique: Vec<DocMesh> = Vec::new();
    let mut groups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for mesh in meshes.iter() {
        let hash = mesh_fingerprint(mesh);
        let mut found = false;
        if let Some(candidates) = groups.get(&hash) {
            for slot in candidates {
                if unique.get(*slot) == Some(mesh) {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            let slot = unique.len();
            unique.push(mesh.clone());
            groups.entry(hash).or_default().push(slot);
        }
    }
    unique
}

/// Merge materials with equal textures plus EPS-close factors.
fn dedup_materials(materials: &[DocMaterial]) -> Vec<DocMaterial> {
    let mut unique: Vec<DocMaterial> = Vec::new();
    for material in materials {
        let mut found = false;
        for kept in unique.iter() {
            if kept.texture == material.texture && factor_close(&kept.factor, &material.factor) {
                found = true;
                break;
            }
        }
        if !found {
            unique.push(material.clone());
        }
    }
    unique
}

/// Hash-join byte-identical images; first-seen order wins.
fn dedup_images(images: &[DocImage]) -> Vec<DocImage> {
    let mut unique: Vec<DocImage> = Vec::new();
    let mut groups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for image in images {
        let mut found = false;
        if let Some(candidates) = groups.get(&image.id) {
            for slot in candidates {
                if unique.get(*slot) == Some(image) {
                    found = true;
                    break;
                }
            }
        }
        if !found {
            let slot = unique.len();
            unique.push(image.clone());
            groups.entry(image.id).or_default().push(slot);
        }
    }
    unique
}

/// Normalize one document: dedup, then prune, then resize-record in order.
/// Byte-identical meshes/materials/images merge (first-seen wins);
/// unreferenced images drop and UVs with no texture zero out; referenced
/// images over `MAX_TEXTURE_PX` downsample with `image` or refuse.
pub fn normalize_document(doc: &Document) -> Result<Document, OptimizeError> {
    if document_bytes(doc) > MAX_DOCUMENT_BYTES {
        return Err(OptimizeError::TooLarge {
            count: document_bytes(doc),
        });
    }
    if doc.meshes.len() > MAX_MESHES {
        return Err(OptimizeError::TooLarge {
            count: doc.meshes.len(),
        });
    }
    // (a) dedup: hash-join byte-identical meshes, materials, textures.
    let unique_meshes = dedup_meshes(&doc.meshes);
    let unique_materials = dedup_materials(&doc.materials);
    let unique_images = dedup_images(&doc.images);
    // (b) prune: drop unreferenced images, zero UVs with no texture.
    let mut referenced: BTreeSet<u64> = BTreeSet::new();
    for material in &unique_materials {
        if let Some(id) = material.texture {
            referenced.insert(id);
        }
    }
    let mut kept_images: Vec<DocImage> = Vec::new();
    for image in &unique_images {
        if referenced.contains(&image.id) {
            kept_images.push(image.clone());
        }
    }
    let mut pruned_meshes = unique_meshes;
    if referenced.is_empty() {
        for mesh in &mut pruned_meshes {
            for uv in &mut mesh.uvs {
                *uv = [0.0, 0.0];
            }
        }
    }
    // (c) resize-record: referenced dims against the cap.
    let mut out_images = kept_images;
    for image in &mut out_images {
        let pixels = usize::try_from(image.width)
            .unwrap_or(usize::MAX)
            .checked_mul(usize::try_from(image.height).unwrap_or(usize::MAX))
            .unwrap_or(usize::MAX);
        if pixels <= MAX_TEXTURE_PX {
            continue;
        }
        resize_over_budget(image, pixels)?;
    }
    out_images.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Document {
        meshes: pruned_meshes,
        materials: unique_materials,
        images: out_images,
    })
}

/// Downsample one over-budget referenced image, or refuse without the feature.
#[cfg(feature = "import")]
fn resize_over_budget(image: &mut DocImage, pixels: usize) -> Result<(), OptimizeError> {
    if pixels <= MAX_TEXTURE_PX {
        return Ok(());
    }
    let decoded =
        image::load_from_memory(&image.bytes).map_err(|e| OptimizeError::ImageDecode {
            detail: bound_chars(&format!("{e:?}"), MAX_DETAIL_CHARS),
        })?;
    let rgba = decoded.to_rgba8();
    let (mut width, mut height) = (rgba.width(), rgba.height());
    if width == 0 || height == 0 {
        return Err(OptimizeError::ImageDecode {
            detail: "empty image".to_string(),
        });
    }
    while usize::try_from(width)
        .unwrap_or(usize::MAX)
        .checked_mul(usize::try_from(height).unwrap_or(usize::MAX))
        .unwrap_or(usize::MAX)
        > MAX_TEXTURE_PX
    {
        width = width.div_ceil(2).max(1);
        height = height.div_ceil(2).max(1);
        if width == 1 && height == 1 {
            break;
        }
    }
    let small = image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Nearest);
    let mut bytes = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut bytes);
        image::DynamicImage::ImageRgba8(small)
            .write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| OptimizeError::ImageDecode {
                detail: bound_chars(&format!("{e:?}"), MAX_DETAIL_CHARS),
            })?;
    }
    image.bytes = bytes;
    image.width = width;
    image.height = height;
    Ok(())
}

/// Refuse over-budget referenced textures when `import` is off: never guess.
#[cfg(not(feature = "import"))]
fn resize_over_budget(_image: &DocImage, pixels: usize) -> Result<(), OptimizeError> {
    if pixels <= MAX_TEXTURE_PX {
        return Ok(());
    }
    Err(OptimizeError::FeatureOff)
}

/// Cook entry for fetch rows: normalize, then report bytes plus digest.
/// Calls `normalize_document`, so the pre-pass stays wired to a producer.
pub fn cook_normalized_bytes(doc: &Document) -> Result<(Document, usize, u64), OptimizeError> {
    let normalized = normalize_document(doc)?;
    let bytes = document_bytes(&normalized);
    let digest = normalize_digest(&normalized);
    Ok((normalized, bytes, digest))
}
