//! kit_gltf: the kit's compact glTF the packed web bundle ships.
//!
//! Consumer: `arena_bundle --format packed` writes one of these per kit file
//! under `src/<file>`, and `BundleSource` hands it to `import_kit_from` in the
//! page, which imports it exactly like the shipped file. The shipped kit glTFs
//! carry every texture in one base64 buffer (46 MB over the eight files); this
//! document keeps the merged geometry the cook already validated (364 KB over
//! the eight files), drops the dead images of the pieces a cooked set dresses,
//! and re-embeds the banner's own maps downscaled to `max_dim`.
//!
//! Determinism: fixed array and view order, shortest-round-trip `f32` text, no
//! map iteration; the same piece, images and cap give the same bytes. Units:
//! meters, texels. Errors: typed [`KitImportError`], never a panic.

use super::cooked::TextureSlot;
use super::kit_import::{texture_err, KitImage, KitImportError, KitPiece, MAX_KIT_DETAIL_CHARS};
use super::wire::bound_chars;
use std::fmt::Write as _;

/// One compact glTF document plus what it carries inside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactGltf {
    /// The glTF 2.0 JSON text, one mesh with one primitive.
    pub json: String,
    /// Images re-embedded in the document (the banner's maps; 0 for a set).
    pub images: usize,
    /// Encoded PNG bytes those images add to the document.
    pub image_bytes: usize,
}

fn refuse(field: &'static str, detail: &str) -> KitImportError {
    KitImportError::InvalidData {
        field,
        detail: bound_chars(detail, MAX_KIT_DETAIL_CHARS),
    }
}

/// Emit the compact glTF of one kit piece.
///
/// A piece that wears a cooked set carries no image: the native kit overrides
/// the set factors (white base, roughness 1, the set's metallic) and the bundle
/// ships the set's KTX2 maps under `cooked/`. The banner wears no set, so its
/// own base, normal and metallic-roughness maps ride along as PNG images
/// downscaled to `max_dim` on the long edge.
///
/// # Errors
/// [`KitImportError::InvalidData`] for an empty mesh, a ragged attribute
/// array, a non-finite value or `max_dim == 0`; [`KitImportError::Texture`]
/// when a banner image does not decode or encode.
pub fn compact_gltf(
    piece: &KitPiece,
    images: &[KitImage],
    max_dim: u32,
) -> Result<CompactGltf, KitImportError> {
    if max_dim == 0 {
        return Err(refuse("texture", "max_dim must be positive"));
    }
    let name = super::kit_import::kit_stem(&piece.file);
    let mesh = &piece.mesh;
    check_mesh(mesh)?;
    let mut blob = Blob::default();
    blob.push(&le_bytes(&mesh.positions), 34962);
    blob.push(&le_bytes(&mesh.normals), 34962);
    blob.push(&le_bytes(&mesh.uv0), 34962);
    blob.push(&le_bytes(&mesh.tangents), 34962);
    blob.push(&index_bytes(&mesh.indices), 34963);
    blob.mesh_bytes = blob.bytes.len();
    // Only a piece with no cooked set needs its own maps (the banner cloth).
    let mut embedded = Vec::new();
    if piece.set.is_none() {
        for image in images {
            let png = downscale_png(&image.bytes, max_dim)?;
            blob.push(&png, 0);
            embedded.push(Embedded {
                slot: image.slot,
                view: blob.views.len() - 1,
            });
        }
    }
    let image_bytes = blob.bytes.len().saturating_sub(blob.mesh_bytes);
    let json = document(piece, name, &blob, &embedded);
    Ok(CompactGltf {
        json,
        images: embedded.len(),
        image_bytes,
    })
}

/// Attribute arrays agree with the positions and hold whole triangles.
fn check_mesh(mesh: &super::cooked::CookedMesh) -> Result<(), KitImportError> {
    let vertices = mesh.positions.len();
    if vertices == 0 {
        return Err(refuse("positions", "no vertices"));
    }
    for (field, len) in [
        ("normals", mesh.normals.len()),
        ("uv0", mesh.uv0.len()),
        ("tangents", mesh.tangents.len()),
    ] {
        if len != vertices {
            return Err(refuse(field, "attribute length differs from positions"));
        }
    }
    if mesh.indices.is_empty() || !mesh.indices.len().is_multiple_of(3) {
        return Err(refuse("indices", "not whole triangles"));
    }
    for (field, finite) in [
        (
            "positions",
            mesh.positions.iter().flatten().all(|v| v.is_finite()),
        ),
        (
            "normals",
            mesh.normals.iter().flatten().all(|v| v.is_finite()),
        ),
        ("uv0", mesh.uv0.iter().flatten().all(|v| v.is_finite())),
        (
            "tangents",
            mesh.tangents.iter().flatten().all(|v| v.is_finite()),
        ),
    ] {
        if !finite {
            return Err(refuse(field, "non-finite value"));
        }
    }
    Ok(())
}

/// One image re-embedded in the document.
struct Embedded {
    slot: TextureSlot,
    /// Index of the image's buffer view (the buffer views follow the five
    /// geometry views in push order).
    view: usize,
}

/// A buffer view: byte offset into the buffer plus its length.
type View = (usize, usize);

/// The single binary buffer a compact document embeds as base64.
#[derive(Default)]
struct Blob {
    bytes: Vec<u8>,
    /// Views in push order: position, normal, uv, tangent, indices, images.
    views: Vec<(View, u32)>,
    /// Buffer length right after the index view; image bytes start here.
    mesh_bytes: usize,
}

impl Blob {
    /// Append `data` four-byte aligned and return its view.
    fn push(&mut self, data: &[u8], target: u32) -> View {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let offset = self.bytes.len();
        let len = data.len();
        self.bytes.extend_from_slice(data);
        self.views.push(((offset, len), target));
        (offset, len)
    }
}

/// Build the glTF 2.0 JSON text.
fn document(piece: &KitPiece, name: &str, blob: &Blob, embedded: &[Embedded]) -> String {
    let mesh = &piece.mesh;
    let (bounds_min, bounds_max) = position_bounds(&mesh.positions);
    let mut out = String::with_capacity(512 + blob.bytes.len() * 2);
    out.push_str("{\"asset\":{\"version\":\"2.0\",\"generator\":\"engine2040-arena-bundle\"}");
    let _ = write!(
        out,
        ",\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"name\":{name_json},\"mesh\":0}}]",
        name_json = quoted(name)
    );
    let _ = write!(
        out,
        ",\"meshes\":[{{\"name\":{name_json},\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\
\"NORMAL\":1,\"TEXCOORD_0\":2,\"TANGENT\":3}},\"indices\":4,\"material\":0}}]}}]",
        name_json = quoted(name)
    );
    out.push_str(",\"materials\":[");
    material(&mut out, piece, embedded);
    out.push(']');
    if !embedded.is_empty() {
        textures(&mut out, embedded);
    }
    accessors(&mut out, mesh, bounds_min, bounds_max);
    buffer_views(&mut out, blob);
    let _ = write!(
        out,
        ",\"buffers\":[{{\"byteLength\":{},\"uri\":\"data:application/octet-stream;base64,{}\"}}]}}",
        blob.bytes.len(),
        base64(&blob.bytes)
    );
    out
}

/// Position, normal, uv, tangent and index accessors, in buffer-view order.
fn accessors(out: &mut String, mesh: &super::cooked::CookedMesh, min: [f32; 3], max: [f32; 3]) {
    let count = mesh.positions.len();
    out.push_str(",\"accessors\":[");
    let _ = write!(
        out,
        "{{\"bufferView\":0,\"componentType\":5126,\"count\":{count},\"type\":\"VEC3\",\
\"min\":[{},{},{}],\"max\":[{},{},{}]}}",
        number(min[0]),
        number(min[1]),
        number(min[2]),
        number(max[0]),
        number(max[1]),
        number(max[2])
    );
    let _ = write!(
        out,
        ",{{\"bufferView\":1,\"componentType\":5126,\"count\":{count},\"type\":\"VEC3\"}},\
{{\"bufferView\":2,\"componentType\":5126,\"count\":{count},\"type\":\"VEC2\"}},\
{{\"bufferView\":3,\"componentType\":5126,\"count\":{count},\"type\":\"VEC4\"}}"
    );
    let _ = write!(
        out,
        ",{{\"bufferView\":4,\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}]",
        mesh.indices.len()
    );
}

/// Every buffer view in push order.
fn buffer_views(out: &mut String, blob: &Blob) {
    out.push_str(",\"bufferViews\":[");
    for (index, (view, target)) in blob.views.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{}",
            view.0, view.1
        );
        if *target != 0 {
            let _ = write!(out, ",\"target\":{target}");
        }
        out.push('}');
    }
    out.push(']');
}

/// One material: the set override for a set piece, the glTF factors (plus the
/// embedded maps) for the banner.
fn material(out: &mut String, piece: &KitPiece, embedded: &[Embedded]) {
    let factors = &piece.material;
    let _ = write!(
        out,
        "{{\"name\":{},\"pbrMetallicRoughness\":{{\"baseColorFactor\":[{},{},{},{}],\
\"metallicFactor\":{},\"roughnessFactor\":{}",
        quoted(super::kit_import::kit_stem(&piece.file)),
        number(factors.base_color_factor[0]),
        number(factors.base_color_factor[1]),
        number(factors.base_color_factor[2]),
        number(factors.base_color_factor[3]),
        number(factors.metallic_factor),
        number(factors.roughness_factor)
    );
    let mut normal = None;
    for (texture, image) in embedded.iter().enumerate() {
        match image.slot {
            TextureSlot::BaseColor => {
                let _ = write!(out, ",\"baseColorTexture\":{{\"index\":{texture}}}");
            }
            TextureSlot::MetallicRoughness => {
                let _ = write!(out, ",\"metallicRoughnessTexture\":{{\"index\":{texture}}}");
            }
            TextureSlot::Normal => normal = Some(texture),
            // The importer reads these three slots only; a source image that
            // lands in another slot never reaches the cooked material either.
            TextureSlot::Occlusion | TextureSlot::Emissive => {}
        }
    }
    out.push('}');
    if let Some(texture) = normal {
        let _ = write!(out, ",\"normalTexture\":{{\"index\":{texture}}}");
    }
    out.push('}');
}

/// Samplers, images and textures for the embedded maps: tiling repeat, linear
/// filtering with mips, the kit's own UV convention.
fn textures(out: &mut String, embedded: &[Embedded]) {
    out.push_str(
        ",\"samplers\":[{\"magFilter\":9729,\"minFilter\":9987,\"wrapS\":10497,\"wrapT\":10497}]",
    );
    out.push_str(",\"images\":[");
    for (index, image) in embedded.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"bufferView\":{},\"mimeType\":\"image/png\"}}",
            image.view
        );
    }
    out.push_str("],\"textures\":[");
    for index in 0..embedded.len() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"sampler\":0,\"source\":{index}}}");
    }
    out.push(']');
}

/// `bytes` as base64 text (standard alphabet, padded).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().saturating_mul(4) / 3 + 4);
    for chunk in bytes.chunks(3) {
        let a = u32::from(chunk.first().copied().unwrap_or(0));
        let b = u32::from(chunk.get(1).copied().unwrap_or(0));
        let c = u32::from(chunk.get(2).copied().unwrap_or(0));
        let word = (a << 16) | (b << 8) | c;
        for (slot, shift) in [(0usize, 18u32), (1, 12), (2, 6), (3, 0)] {
            let present = chunk.len() > slot.saturating_sub(1);
            let index = usize::try_from((word >> shift) & 63).unwrap_or(0);
            let byte = if present {
                ALPHABET.get(index).copied().unwrap_or(b'A')
            } else {
                b'='
            };
            out.push(char::from(byte));
        }
    }
    out
}

/// JSON string literal of `text` (kit names carry no control characters).
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Shortest text that reads back as the same `f32` (JSON accepts it as a
/// number; every value here is finite and validated).
fn number(value: f32) -> String {
    if value == 0.0 {
        "0".to_string()
    } else {
        format!("{value:?}")
    }
}

/// Little-endian bytes of every component of each row, in order.
fn le_bytes<const N: usize>(values: &[[f32; N]]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|row| row.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

/// Little-endian bytes of the triangle indices, in order.
fn index_bytes(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Component-wise position bounds; the glTF spec requires them on POSITION.
fn position_bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for position in positions {
        for axis in 0..3 {
            let value = position.get(axis).copied().unwrap_or(0.0);
            if let Some(slot) = min.get_mut(axis) {
                *slot = (*slot).min(value);
            }
            if let Some(slot) = max.get_mut(axis) {
                *slot = (*slot).max(value);
            }
        }
    }
    (min, max)
}

/// Decode one encoded image, shrink it to `max_dim` on the long edge and
/// re-encode it as PNG (the document's declared mime type).
fn downscale_png(bytes: &[u8], max_dim: u32) -> Result<Vec<u8>, KitImportError> {
    let image = image::load_from_memory(bytes)
        .map_err(|err| texture_err(format!("image decode: {err}")))?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let long = width.max(height);
    if long == 0 {
        return Err(texture_err("image has no texels"));
    }
    // Integer scaling: the long edge lands on the cap (or stays put when it is
    // already smaller), the short edge keeps the aspect with half-up rounding.
    let target_long = max_dim.min(long);
    let scaled = |value: u32| {
        let numerator = u64::from(value) * u64::from(target_long) + u64::from(long) / 2;
        u32::try_from(numerator / u64::from(long))
            .unwrap_or(1)
            .max(1)
    };
    let resized = image::imageops::resize(
        &rgba,
        scaled(width),
        scaled(height),
        image::imageops::FilterType::Triangle,
    );
    let mut png = Vec::new();
    resized
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|err| texture_err(format!("png encode: {err}")))?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit_import::{import_kit_gltf_full, is_lfs_pointer, KIT_FILES};

    fn arena_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena")
    }

    fn present(path: &std::path::Path) -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        if is_lfs_pointer(&bytes) {
            return None;
        }
        Some(bytes)
    }

    /// A shipped file's compact text, its source piece and its source images;
    /// `None` while the content is still a Git LFS pointer.
    fn compact(file: &str, max_dim: u32) -> Option<(CompactGltf, KitPiece, Vec<KitImage>)> {
        let bytes = present(&arena_root().join("src").join(file))?;
        let (piece, images) = import_kit_gltf_full(file, &bytes).expect("source imports");
        let compact = compact_gltf(&piece, &images, max_dim).expect("compacts");
        Some((compact, piece, images))
    }

    // Every kit file compacts and re-imports to the same merged geometry,
    // material and triangle count: the page sees the window's kit.
    #[test]
    fn every_compact_file_reimports_to_the_same_mesh() {
        let mut triangles = 0u64;
        for file in KIT_FILES {
            let Some((compact, source, images)) = compact(file, 128) else {
                return;
            };
            let (again, _) =
                import_kit_gltf_full(file, compact.json.as_bytes()).expect("compact imports");
            assert_eq!(again.file, file);
            assert_eq!(again.triangles, source.triangles, "{file} triangles");
            assert_eq!(
                again.mesh.positions, source.mesh.positions,
                "{file} positions"
            );
            assert_eq!(again.mesh.normals, source.mesh.normals, "{file} normals");
            assert_eq!(again.mesh.uv0, source.mesh.uv0, "{file} uv");
            assert_eq!(again.mesh.tangents, source.mesh.tangents, "{file} tangents");
            assert_eq!(again.mesh.indices, source.mesh.indices, "{file} indices");
            assert_eq!(again.material, source.material, "{file} material");
            assert_eq!(again.set, source.set, "{file} set");
            let twice = compact_gltf(&source, &images, 128).expect("recook");
            assert_eq!(twice, compact, "{file} deterministic");
            triangles += again.triangles;
        }
        assert_eq!(triangles, 7_472, "kit triangles survive the round trip");
    }

    // The banner wears no cooked set, so its own cloth maps ride in the file,
    // downscaled to the cap and re-imported into the same slots.
    #[test]
    fn banner_keeps_its_maps_downscaled_to_the_cap() {
        let Some((compact, source, images)) = compact("banner.gltf", 128) else {
            return;
        };
        assert!(source.set.is_none());
        assert_eq!(compact.images, images.len(), "every source map embedded");
        assert!(compact.image_bytes > 0, "banner maps add bytes");
        let (again, embedded) =
            import_kit_gltf_full("banner.gltf", compact.json.as_bytes()).expect("reimports");
        assert_eq!(embedded.len(), images.len(), "same slot count back");
        for (back, original) in embedded.iter().zip(&images) {
            assert_eq!(back.slot, original.slot, "slot order");
            let image = image::load_from_memory(&back.bytes).expect("decodes");
            assert!(
                image.width().max(image.height()) <= 128,
                "map capped at 128, got {}x{}",
                image.width(),
                image.height()
            );
        }
        assert!(
            again.material.textures.is_empty(),
            "a set-less piece keeps its factors"
        );
    }

    // The packed tree the page fetches: `BundleSource::open` + the same
    // importer and KTX2 reader `import_kit_from` uses, over the shipped files.
    #[test]
    fn packed_tree_reads_back_through_the_page_source() {
        use crate::asset_source::{
            content_digest, pack_to, AssetSource, BundleSource, MemorySource, PackRules,
        };
        use crate::ktx2_cook::parse_ktx2;
        let root = arena_root();
        let mut staged = MemorySource::new();
        for file in KIT_FILES {
            let Some(bytes) = present(&root.join("src").join(file)) else {
                return;
            };
            let (piece, images) = import_kit_gltf_full(file, &bytes).expect("source imports");
            let compact = compact_gltf(&piece, &images, 64).expect("compacts");
            staged
                .insert(&format!("src/{file}"), compact.json.into_bytes())
                .expect("stages");
        }
        for set in crate::kit_import::KitSet::ALL {
            for map in crate::kit_import::KitMap::ALL {
                let path = root.join(format!("cooked/{}_{}.ktx2", set.name(), map.name()));
                let Some(bytes) = present(&path) else {
                    return;
                };
                staged
                    .insert(&format!("cooked/{}_{}.ktx2", set.name(), map.name()), bytes)
                    .expect("stages");
            }
        }
        let mut packed = Vec::new();
        let report = pack_to(&staged, &PackRules::new(), &mut packed).expect("packs");
        assert_eq!(
            report.entries,
            KIT_FILES.len() + 15,
            "eight files, fifteen maps"
        );
        let source = BundleSource::open(packed).expect("the page's reader opens it");
        assert_eq!(content_digest(&source, "").expect("digest"), report.digest);
        let mut triangles = 0u64;
        for file in KIT_FILES {
            let json = source
                .read(&format!("src/{file}"))
                .expect("compact glTF reads back");
            let (piece, _) =
                import_kit_gltf_full(file, &json).expect("the page's importer reads it");
            assert_eq!(piece.file, file);
            triangles += piece.triangles;
        }
        assert_eq!(triangles, 7_472, "every kit triangle survives the bundle");
        for set in crate::kit_import::KitSet::ALL {
            let base = source
                .read(&format!("cooked/{}_base.ktx2", set.name()))
                .expect("set base reads back");
            let parsed = parse_ktx2(&base).expect("the page's KTX2 reader decodes it");
            assert_eq!(parsed.kind, crate::ktx2_cook::MapKind::BaseColor);
        }
    }

    // A set piece ships no image: the cooked KTX2 set dresses it, exactly as
    // the native kit does.
    #[test]
    fn set_pieces_ship_no_image() {
        let Some((compact, source, images)) = compact("walls.gltf", 128) else {
            return;
        };
        assert!(source.set.is_some());
        assert!(!images.is_empty(), "the shipped file does embed maps");
        assert_eq!(compact.images, 0, "no image in the compact file");
        assert_eq!(compact.image_bytes, 0, "no image bytes either");
        assert!(!compact.json.contains("\"images\""), "no image block");
        assert!(!compact.json.contains("\"textures\""), "no texture block");
    }
}
