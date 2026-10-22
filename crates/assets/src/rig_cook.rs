//! rig_cook: the CT7 fighter rig as a body the page can hold.
//!
//! Consumer: the browser page fetches the cooked rig and hands it to
//! `engine_game::fighter_look::FighterLook::load_from`. The rig as it stands
//! does not fit the page: the hero body's four 1024x1024 atlas quadrants
//! decode to 16,777,216 B of RGBA, 84 % of the page's whole linear memory on
//! their own, and the whole rig is 6,239,621 B brotli against a 2,040,000 B
//! first-bundle budget. This cook is the lever, the same one the arena kit
//! already uses (`ktx2_cook` plus `kit_gltf`): the atlas becomes KTX2 block
//! maps and the body drops the images, so the same rig reads with a fraction
//! of the bytes and the memory.
//!
//! What the cook keeps, because the look needs it: the skeleton (the 45
//! joints, their names and parents), the node bind pose, the inverse binds,
//! the skin rows (joints and weights per vertex), the mesh (positions,
//! normals, UVs, indices) and the material factors. What it drops: the
//! embedded PNGs, which become KTX2 maps beside the body, and the textures
//! that pointed at them.
//!
//! Units: meters, UVs in `0..=1`, bytes. Determinism: the same body bytes
//! give the same cooked bytes - no clock, no RNG, no hash-order dependence.
//! Errors: [`RigCookError`], every one naming the file and the reason.
//! Budget: one-shot cook (a full read of a body), never on a tick.
//!
//! The cook edits the source document, not a new glTF: every node, skin,
//! accessor and buffer view stays as it was, so the rig that comes out is the
//! rig that went in, and only the images leave. The binary chunk is rebuilt
//! around the buffer views that stay.

use std::collections::BTreeMap;

use crate::body_import::{import_skinned_body, BodyImportError};
use crate::ktx2_cook::{cook_ktx2, Ktx2Error, MapKind};

/// The cooked directory inside the rig tree: the cooked bodies and their
/// KTX2 atlases live here, beside the manifest and the clips.
pub const COOKED_DIR: &str = "cooked";
/// The long edge of a cooked atlas, in pixels. The rig's quadrants are
/// 1024x1024 PNGs; at 256 the block maps hold a sixteenth of the pixels and a
/// fighter on a phone screen is a fraction of the page's height. The cap is
/// the lever the page's budgets need: 256 px puts the whole rig's atlases
/// under 800 KB, 512 px puts them over 2.8 MB.
pub const ATLAS_MAX_DIM: u32 = 256;
/// How many atlas quadrants one body carries.
pub const QUADRANTS: usize = 4;

/// Typed cook refusals; never a guessed rig.
#[derive(Clone, PartialEq, Debug)]
pub enum RigCookError {
    /// The body did not parse as a skinned body.
    Body {
        /// The body file that failed.
        file: String,
        /// The import's own reason.
        reason: BodyImportError,
    },
    /// The body carried a different number of parts than the rig expects.
    Parts {
        /// The body file that failed.
        file: String,
        /// Parts found.
        found: usize,
    },
    /// The body carried no base color image for a part.
    NoBaseColor {
        /// The body file that failed.
        file: String,
        /// The part index in document order.
        part: usize,
    },
    /// The atlas did not cook.
    Texture {
        /// The body file that failed.
        file: String,
        /// The quadrant index.
        quadrant: usize,
        /// The texture cook's own reason.
        reason: Ktx2Error,
    },
    /// The atlas image did not decode.
    Decode {
        /// The body file that failed.
        file: String,
        /// The quadrant index.
        quadrant: usize,
        /// The decoder's own reason.
        reason: String,
    },
    /// The body was not a glTF container the cook can read.
    Container {
        /// The body file that failed.
        file: String,
        /// The container's own reason.
        reason: String,
    },
    /// The body carried an image the cook cannot replace.
    Image {
        /// The body file that failed.
        file: String,
        /// The image index in document order.
        image: usize,
    },
    /// The document's JSON could not be written back.
    Json {
        /// The body file that failed.
        file: String,
        /// The JSON's own reason.
        reason: String,
    },
}

impl core::fmt::Display for RigCookError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Body { file, reason } => write!(f, "{file}: {reason}"),
            Self::Parts { file, found } => {
                write!(f, "{file}: {found} parts, expected {QUADRANTS}")
            }
            Self::NoBaseColor { file, part } => {
                write!(f, "{file}: part {part} carries no base color")
            }
            Self::Texture {
                file,
                quadrant,
                reason,
            } => write!(f, "{file}: quadrant {quadrant}: {reason}"),
            Self::Decode {
                file,
                quadrant,
                reason,
            } => write!(
                f,
                "{file}: quadrant {quadrant}: the atlas did not decode: {reason}"
            ),
            Self::Container { file, reason } => write!(f, "{file}: {reason}"),
            Self::Image { file, image } => {
                write!(f, "{file}: image {image} is not an embedded buffer view")
            }
            Self::Json { file, reason } => write!(f, "{file}: {reason}"),
        }
    }
}

impl std::error::Error for RigCookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Body { reason, .. } => Some(reason),
            Self::Texture { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

/// One cooked body: the compact glTF bytes plus the cooked atlases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedBody {
    /// The patched glTF bytes: the rig without its embedded images.
    pub glb: Vec<u8>,
    /// The cooked atlases, one per quadrant, in part order.
    pub textures: Vec<Vec<u8>>,
    /// Vertices in the body.
    pub vertices: usize,
    /// Triangles in the body.
    pub triangles: usize,
    /// Joints in the body.
    pub joints: usize,
    /// Bytes the source body took.
    pub source_bytes: usize,
}

/// Cook one body glTF into a body without its images plus its KTX2 atlases.
///
/// The body keeps its skeleton, bind pose, inverse binds, skin rows and mesh
/// byte for byte: only the embedded PNGs leave, and the atlases are cooked
/// from them at [`ATLAS_MAX_DIM`].
pub fn cook_body(file: &str, bytes: &[u8]) -> Result<CookedBody, RigCookError> {
    cook_body_at(file, bytes, ATLAS_MAX_DIM)
}

/// The same cook with the atlas edge as the caller wants it: 0 cooks the
/// atlas at its own size, any other value caps the long edge.
pub fn cook_body_at(
    file: &str,
    bytes: &[u8],
    atlas_max_dim: u32,
) -> Result<CookedBody, RigCookError> {
    let body = import_skinned_body(bytes).map_err(|reason| RigCookError::Body {
        file: file.to_string(),
        reason,
    })?;
    if body.parts.len() != QUADRANTS {
        return Err(RigCookError::Parts {
            file: file.to_string(),
            found: body.parts.len(),
        });
    }
    let mut textures = Vec::with_capacity(QUADRANTS);
    for (quadrant, part) in body.parts.iter().enumerate() {
        let Some(image) = &part.base_color_image else {
            return Err(RigCookError::NoBaseColor {
                file: file.to_string(),
                part: quadrant,
            });
        };
        textures.push(cook_atlas(file, quadrant, image, atlas_max_dim)?);
    }
    let glb = strip_images(file, bytes)?;
    let vertices = body
        .parts
        .iter()
        .map(|part| part.mesh.positions.len())
        .sum();
    let triangles = body
        .parts
        .iter()
        .map(|part| part.mesh.indices.len() / 3)
        .sum();
    Ok(CookedBody {
        glb,
        textures,
        vertices,
        triangles,
        joints: body.skeleton.joints.len(),
        source_bytes: bytes.len(),
    })
}

/// Cook one atlas quadrant: decode the PNG, cook it to a KTX2 block map.
fn cook_atlas(
    file: &str,
    quadrant: usize,
    png: &[u8],
    atlas_max_dim: u32,
) -> Result<Vec<u8>, RigCookError> {
    let decoded = image::load_from_memory(png).map_err(|err| RigCookError::Decode {
        file: file.to_string(),
        quadrant,
        reason: err.to_string(),
    })?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    cook_ktx2(
        rgba.as_raw(),
        width,
        height,
        MapKind::BaseColor,
        atlas_max_dim,
    )
    .map_err(|reason| RigCookError::Texture {
        file: file.to_string(),
        quadrant,
        reason,
    })
}

/// The body without its embedded images: the same rig, the PNGs left out.
///
/// The document keeps every node, skin, accessor and buffer view, so the rig
/// is the rig. The images and their buffer views go, and the binary chunk is
/// rebuilt around what stays.
fn strip_images(file: &str, bytes: &[u8]) -> Result<Vec<u8>, RigCookError> {
    let bad = |reason: String| RigCookError::Container {
        file: file.to_string(),
        reason,
    };
    let glb = gltf::binary::Glb::from_slice(bytes)
        .map_err(|err| bad(format!("glTF container: {err}")))?;
    let parsed =
        gltf::Gltf::from_slice(bytes).map_err(|err| bad(format!("glTF document: {err}")))?;
    let bin = glb.bin.as_deref().unwrap_or_default();

    let dropped = image_views(file, &parsed)?;
    if dropped.is_empty() {
        // No images to strip: the body is already cookable as it stands.
        return Ok(bytes.to_vec());
    }
    let (packed, moved) = pack_views(&parsed.as_json().buffer_views, bin, &dropped);
    let root = strip_document(parsed.document.into_json(), &dropped, &packed, &moved);
    let text = serde_json::to_string(&root).map_err(|err| RigCookError::Json {
        file: file.to_string(),
        reason: format!("the JSON write: {err}"),
    })?;
    Ok(write_glb(&text, &packed))
}

/// The buffer views the images point at, in image order.
fn image_views(file: &str, document: &gltf::Document) -> Result<Vec<usize>, RigCookError> {
    let mut dropped = Vec::new();
    for (image, entry) in document.as_json().images.iter().enumerate() {
        let Some(view) = entry.buffer_view else {
            return Err(RigCookError::Image {
                file: file.to_string(),
                image,
            });
        };
        dropped.push(view.value());
    }
    Ok(dropped)
}

/// The packed binary chunk: the views that stay, in order, four-byte aligned.
/// The map says where each kept view landed and how long it is.
fn pack_views(
    views: &[gltf::json::buffer::View],
    bin: &[u8],
    dropped: &[usize],
) -> (Vec<u8>, BTreeMap<usize, (usize, usize)>) {
    let mut kept: Vec<usize> = (0..views.len())
        .filter(|view| !dropped.contains(view))
        .collect();
    kept.sort_unstable();
    let mut packed = Vec::with_capacity(bin.len());
    let mut moved: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    for view in kept {
        let entry = &views[view];
        let offset = entry
            .byte_offset
            .map_or(0, |value| usize::try_from(value.0).unwrap_or(0));
        let length = usize::try_from(entry.byte_length.0).unwrap_or(0);
        let start = offset.min(bin.len());
        let end = start.saturating_add(length).min(bin.len());
        while packed.len() % 4 != 0 {
            packed.push(0);
        }
        let at = packed.len();
        packed.extend_from_slice(&bin[start..end]);
        moved.insert(view, (at, end - start));
    }
    (packed, moved)
}

/// The document without its images: the images, the textures and the samplers
/// go, the dropped buffer views leave the list, and the accessors follow.
fn strip_document(
    mut root: gltf::json::Root,
    dropped: &[usize],
    packed: &[u8],
    moved: &BTreeMap<usize, (usize, usize)>,
) -> gltf::json::Root {
    root.images.clear();
    root.textures.clear();
    root.samplers.clear();
    for material in &mut root.materials {
        material.pbr_metallic_roughness.base_color_texture = None;
    }
    // The dropped views leave the list; the ones after them move down.
    let mut old_index = 0;
    root.buffer_views.retain(|_| {
        let keep = !dropped.contains(&old_index);
        old_index += 1;
        keep
    });
    // The accessors point at the view that took a dropped view's place, and
    // the ones after it move down by the number of dropped views below them.
    for accessor in &mut root.accessors {
        if let Some(view) = &mut accessor.buffer_view {
            let old = view.value();
            let new = if dropped.contains(&old) {
                old
            } else {
                let below = dropped.iter().filter(|dropped| **dropped < old).count();
                old - below
            };
            *view = gltf::json::Index::new(u32::try_from(new).unwrap_or(u32::MAX));
        }
    }
    // The buffer views that moved get their new offset and length. The map
    // holds the old index of each kept view, so it names the new one by the
    // count of the dropped views below it.
    for (old, (offset, length)) in moved {
        let below = dropped.iter().filter(|dropped| **dropped < *old).count();
        let Some(entry) = root.buffer_views.get_mut(old - below) else {
            continue;
        };
        entry.byte_offset = Some(gltf::json::validation::USize64(
            u64::try_from(*offset).unwrap_or(0),
        ));
        entry.byte_length = gltf::json::validation::USize64(u64::try_from(*length).unwrap_or(0));
    }
    // The buffer itself is the packed chunk now.
    if let Some(buffer) = root.buffers.first_mut() {
        buffer.byte_length =
            gltf::json::validation::USize64(u64::try_from(packed.len()).unwrap_or(u64::MAX));
    }
    root
}

/// Write a glTF container: the header, the JSON chunk and the binary chunk.
///
/// The JSON chunk is padded with spaces, as the glTF spec asks: a JSON parser
/// reads the whole chunk, so null bytes would be trailing characters.
fn write_glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let json_bytes = json.as_bytes();
    let json_pad = (4 - (json_bytes.len() % 4)) % 4;
    let bin_pad = (4 - (bin.len() % 4)) % 4;
    let json_len = json_bytes.len() + json_pad;
    let bin_len = bin.len() + bin_pad;
    let total = 12 + 8 + json_len + 8 + bin_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(total).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&u32::try_from(json_len).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json_bytes);
    out.extend(std::iter::repeat_n(b' ', json_pad));
    out.extend_from_slice(&u32::try_from(bin_len).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out.extend(std::iter::repeat_n(0u8, bin_pad));
    out
}

/// The cooked file names of one body: the body and its atlases, by logical
/// path inside the rig tree.
#[must_use]
pub fn cooked_names(variant: &str) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    names.insert(
        format!("{COOKED_DIR}/{variant}_body.glb"),
        format!("{COOKED_DIR}/{variant}_body.glb"),
    );
    for quadrant in 0..QUADRANTS {
        names.insert(
            format!("{COOKED_DIR}/{variant}_q{quadrant}.ktx2"),
            format!("{COOKED_DIR}/{variant}_q{quadrant}.ktx2"),
        );
    }
    names
}
