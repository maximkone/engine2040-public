//! body_import: one skinned glTF body (meshes, skin, binds, base-color
//! images) plus clip sources with their bind pose, ready for render skinning.
//!
//! Consumer: `engine-game` fighter look (CT7 bodies drawn in the arena
//! presenter). Data owner: `engine-assets` owns glTF parsing; the anim
//! contract types come from `engine-anim`, render types stay in the game.
//! Units: meters, radians; inverse binds are AN1 row-vector matrices
//! (translation in the last row), read verbatim from the glTF column-major
//! accessor so `p' = p * inverse_bind * joint_world`.
//! Determinism: same bytes give the same import (document order, no maps).
//! Errors: typed [`BodyImportError`]; a body without a skin, an image that is
//! not an embedded buffer view, or a skin/mesh primitive mismatch fails loudly.
//!
//! Budget: cold load only (three bounded parses of one file, no tick work).

use super::cooked::{CookedMaterial, CookedMesh};
use super::mesh_import::{import_gltf_bytes, MeshImportError, MAX_BYTES};
use super::retarget::BindPose;
use super::skin_import::{
    import_skinned_bytes, skeleton_order, ImportedClip, Skin, SkinImportError,
};
use super::wire::bound_chars;
use engine_anim::contract::Skeleton;
use std::fmt;

/// Identity in the AN1 row-vector layout (non-skin joints carry it).
const IDENTITY: [[f32; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
/// Longest diagnostic carried in [`BodyImportError::Parse`].
const MAX_DETAIL_CHARS: usize = 160;

/// Why a body or clip-source import failed.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyImportError {
    /// Mesh primitives failed the mesh importer.
    Mesh(MeshImportError),
    /// Skeleton, skin or clips failed the skin importer.
    Skin(SkinImportError),
    /// Container parse or buffer load failed; detail bounded.
    Parse(String),
    /// A triangle primitive has no skin rows (every body part must deform).
    UnskinnedPart {
        /// Primitive index in document order.
        part: usize,
    },
    /// Inverse binds missing, short or non-finite.
    BadInverseBind,
    /// A base-color image is not an embedded buffer view.
    ExternalImage {
        /// Image index in document order.
        image: usize,
    },
    /// A skeleton node carries a scale. The engine reads joints as rotation
    /// plus translation, so a scaled node (an FBX armature's 0.01) would come
    /// in a hundred times too big with no error; it is refused instead.
    ScaledJoint {
        /// The node's name (or its index when it has none).
        joint: String,
        /// The scale the file carries.
        scale: [f32; 3],
    },
}

impl fmt::Display for BodyImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mesh(err) => write!(f, "body mesh: {err}"),
            Self::Skin(err) => write!(f, "body skin: {err}"),
            Self::Parse(detail) => write!(f, "body parse: {detail}"),
            Self::UnskinnedPart { part } => write!(f, "body part {part} has no skin rows"),
            Self::BadInverseBind => write!(f, "body inverse binds missing or non-finite"),
            Self::ExternalImage { image } => write!(f, "body image {image} is not embedded"),
            Self::ScaledJoint { joint, scale } => write!(
                f,
                "body joint {joint} carries scale {scale:?}: joints are read as rotation and                  translation only, so bake the object scale into the rig before export                  (Blender: Apply Scale on the armature and its meshes)"
            ),
        }
    }
}

impl std::error::Error for BodyImportError {}

/// One drawable body part: mesh, skin rows, factors and base-color bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPart {
    /// Cooked bind-pose mesh (meters, unit normals, clamped UVs).
    pub mesh: CookedMesh,
    /// Skin rows in skeleton joint indices, one per vertex.
    pub skin: Skin,
    /// PBR factors from the source material.
    pub material: CookedMaterial,
    /// Encoded base-color image (PNG/JPEG bytes) when the material has one.
    pub base_color_image: Option<Vec<u8>>,
}

/// One skinned body: skeleton, node bind pose, inverse binds and parts.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinnedBody {
    /// Every node as a joint, parent-first (same order as the skin rows).
    pub skeleton: Skeleton,
    /// Node bind pose (local translation plus rotation) in skeleton order.
    pub bind: BindPose,
    /// AN1 row-vector inverse binds in skeleton order (identity off-skin).
    pub inverse_binds: Vec<[[f32; 4]; 4]>,
    /// Triangle primitives in document order.
    pub parts: Vec<BodyPart>,
}

/// Clip source: skeleton plus bind pose plus clips of one animation file.
#[derive(Clone, Debug)]
pub struct ClipSource {
    /// Clip-file skeleton (every node, parent-first).
    pub skeleton: Skeleton,
    /// Clip-file node bind pose in skeleton order.
    pub bind: BindPose,
    /// Clips in document order.
    pub clips: Vec<ImportedClip>,
}

/// Parsed document plus loaded buffers (shared by the readers below).
struct Loaded {
    document: gltf::Document,
    buffers: Vec<gltf::buffer::Data>,
}

fn parse_error(text: &str) -> BodyImportError {
    BodyImportError::Parse(bound_chars(text, MAX_DETAIL_CHARS))
}

fn load(bytes: &[u8]) -> Result<Loaded, BodyImportError> {
    if bytes.len() > MAX_BYTES {
        return Err(parse_error("input over the import byte bound"));
    }
    let parsed = gltf::Gltf::from_slice(bytes).map_err(|err| parse_error(&format!("{err}")))?;
    let buffers = gltf::import_buffers(&parsed.document, None, parsed.blob)
        .map_err(|err| parse_error(&format!("{err}")))?;
    Ok(Loaded {
        document: parsed.document,
        buffers,
    })
}

/// Largest distance from 1 a joint's scale may have and still be "no scale".
pub const JOINT_SCALE_TOLERANCE: f32 = 1.0e-3;

/// Node bind pose in skeleton order. A joint with a scale is refused
/// ([`BodyImportError::ScaledJoint`]): the pose has no scale lane, and
/// dropping it silently drew a 0.01-scaled armature a hundred times too big.
fn bind_pose(document: &gltf::Document, order: &[usize]) -> Result<BindPose, BodyImportError> {
    let mut positions = Vec::with_capacity(order.len());
    let mut rotations = Vec::with_capacity(order.len());
    for node_index in order {
        let node = document
            .nodes()
            .nth(*node_index)
            .ok_or_else(|| parse_error("node order names a missing node"))?;
        let (translation, rotation, scale) = node.transform().decomposed();
        if scale
            .iter()
            .any(|s| !s.is_finite() || (s - 1.0).abs() > JOINT_SCALE_TOLERANCE)
        {
            return Err(BodyImportError::ScaledJoint {
                joint: node
                    .name()
                    .map_or_else(|| format!("#{node_index}"), str::to_string),
                scale,
            });
        }
        positions.push(translation);
        rotations.push(rotation);
    }
    Ok(BindPose {
        positions,
        rotations,
    })
}

/// Inverse binds of the first skin, scattered into skeleton order.
fn inverse_binds(loaded: &Loaded, order: &[usize]) -> Result<Vec<[[f32; 4]; 4]>, BodyImportError> {
    let skin = loaded
        .document
        .skins()
        .next()
        .ok_or(BodyImportError::BadInverseBind)?;
    let get = |buffer: gltf::Buffer<'_>| loaded.buffers.get(buffer.index()).map(|data| &*data.0);
    let matrices: Vec<[[f32; 4]; 4]> = skin
        .reader(get)
        .read_inverse_bind_matrices()
        .ok_or(BodyImportError::BadInverseBind)?
        .collect();
    let joints: Vec<usize> = skin.joints().map(|node| node.index()).collect();
    if matrices.len() != joints.len() || matrices.iter().flatten().flatten().any(|v| !v.is_finite())
    {
        return Err(BodyImportError::BadInverseBind);
    }
    let mut out = vec![IDENTITY; order.len()];
    for (node, matrix) in joints.iter().zip(matrices.iter()) {
        let slot = order
            .iter()
            .position(|candidate| candidate == node)
            .ok_or(BodyImportError::BadInverseBind)?;
        // glTF stores columns; read as rows they are the row-vector matrix.
        out[slot] = *matrix;
    }
    Ok(out)
}

/// Embedded base-color image bytes per triangle primitive, document order.
fn base_color_images(loaded: &Loaded) -> Result<Vec<Option<Vec<u8>>>, BodyImportError> {
    let mut out = Vec::new();
    for mesh in loaded.document.meshes() {
        for primitive in mesh.primitives() {
            let info = primitive
                .material()
                .pbr_metallic_roughness()
                .base_color_texture();
            let Some(info) = info else {
                out.push(None);
                continue;
            };
            let image = info.texture().source();
            match image.source() {
                gltf::image::Source::View { view, .. } => {
                    let data = loaded
                        .buffers
                        .get(view.buffer().index())
                        .ok_or_else(|| parse_error("image view names a missing buffer"))?;
                    let start = view.offset();
                    let end = start
                        .checked_add(view.length())
                        .filter(|end| *end <= data.0.len())
                        .ok_or_else(|| parse_error("image view past its buffer"))?;
                    out.push(Some(data.0[start..end].to_vec()));
                }
                gltf::image::Source::Uri { .. } => {
                    return Err(BodyImportError::ExternalImage {
                        image: image.index(),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Import one skinned body (`.glb` bytes): every triangle primitive must
/// carry skin rows, the first skin must hold finite inverse binds, and
/// base-color images must be embedded. Parts keep document order.
pub fn import_skinned_body(bytes: &[u8]) -> Result<SkinnedBody, BodyImportError> {
    let meshes = import_gltf_bytes(bytes).map_err(BodyImportError::Mesh)?;
    let skinned = import_skinned_bytes(bytes).map_err(BodyImportError::Skin)?;
    let loaded = load(bytes)?;
    let order = skeleton_order(&loaded.document);
    if order.len() != skinned.skeleton.len() {
        return Err(parse_error("skeleton and node order disagree"));
    }
    let bind = bind_pose(&loaded.document, &order)?;
    let inverse_binds = inverse_binds(&loaded, &order)?;
    let images = base_color_images(&loaded)?;
    if images.len() != meshes.meshes.len() || skinned.skins.len() != meshes.meshes.len() {
        return Err(BodyImportError::UnskinnedPart {
            part: skinned.skins.len().min(meshes.meshes.len()),
        });
    }
    let mut parts = Vec::with_capacity(meshes.meshes.len());
    for (part, ((mesh, material), (skin, image))) in meshes
        .meshes
        .into_iter()
        .zip(meshes.materials)
        .zip(skinned.skins.into_iter().zip(images))
        .enumerate()
    {
        if skin.skin.joints.len() != mesh.positions.len() {
            return Err(BodyImportError::UnskinnedPart { part });
        }
        parts.push(BodyPart {
            mesh,
            skin: skin.skin,
            material,
            base_color_image: image,
        });
    }
    Ok(SkinnedBody {
        skeleton: skinned.skeleton,
        bind,
        inverse_binds,
        parts,
    })
}

/// Import one animation file (`.glb` bytes): skeleton, node bind pose and
/// clips, so a retarget can move the clips onto a body skeleton.
pub fn import_clip_source(bytes: &[u8]) -> Result<ClipSource, BodyImportError> {
    let skinned = import_skinned_bytes(bytes).map_err(BodyImportError::Skin)?;
    let loaded = load(bytes)?;
    let order = skeleton_order(&loaded.document);
    if order.len() != skinned.skeleton.len() {
        return Err(parse_error("skeleton and node order disagree"));
    }
    let bind = bind_pose(&loaded.document, &order)?;
    Ok(ClipSource {
        skeleton: skinned.skeleton,
        bind,
        clips: skinned.clips,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fighters_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters/glb")
    }

    /// Multiply two AN1 row-vector matrices (`p * a * b`).
    fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
        let mut out = [[0.0f32; 4]; 4];
        for (i, row) in out.iter_mut().enumerate() {
            for (k, cell) in row.iter_mut().enumerate() {
                *cell = (0..4).map(|j| a[i][j] * b[j][k]).sum();
            }
        }
        out
    }

    /// Rewrite a GLB's JSON chunk with `edit`, keeping the binary chunk.
    fn with_json(bytes: &[u8], edit: impl Fn(&str) -> String) -> Vec<u8> {
        let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let json = std::str::from_utf8(&bytes[20..20 + json_len]).unwrap();
        let mut text = edit(json).into_bytes();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let rest = &bytes[20 + json_len..];
        let total = 12 + 8 + text.len() + rest.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&bytes[0..8]);
        out.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(text.len()).unwrap().to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        out.extend_from_slice(rest);
        out
    }

    /// A joint with a scale is refused by name, never drawn a hundred times
    /// too big: the 2026-10-22 Rocketbox body came in with a 0.01 armature and
    /// the window put the camera inside it.
    #[test]
    fn a_scaled_joint_is_refused_by_name() {
        let bytes = std::fs::read(fighters_dir().join("hero_body.glb")).expect("hero body on disk");
        // heel_L already writes a unit scale; set it to the FBX armature's 0.01.
        let scaled = with_json(&bytes, |json| {
            let node = json.find(r#"{"name":"heel_L","#).expect("heel_L node");
            let key = node
                + json[node..]
                    .find(r#""scale":["#)
                    .expect("heel_L has a scale");
            let open = key + r#""scale":["#.len();
            let close = open + json[open..].find(']').expect("scale closes");
            format!("{}0.01,0.01,0.01{}", &json[..open], &json[close..])
        });
        assert_ne!(scaled, bytes, "the edit landed");
        let err = import_skinned_body(&scaled).expect_err("a scaled joint is refused");
        assert!(
            matches!(&err, BodyImportError::ScaledJoint { joint, .. } if joint == "heel_L"),
            "wrong refusal: {err}"
        );
        assert!(
            err.to_string().contains("Apply Scale"),
            "the fix is named: {err}"
        );
        // The unedited body still imports: the shipped rigs carry no scale.
        import_skinned_body(&bytes).expect("the hero still imports");
    }

    #[test]
    fn hero_body_imports_skinned_parts_with_binds() {
        let bytes = std::fs::read(fighters_dir().join("hero_body.glb")).expect("hero body on disk");
        let body = import_skinned_body(&bytes).expect("hero imports");
        assert_eq!(body.skeleton.len(), 45, "45-node rig");
        assert_eq!(body.parts.len(), 4, "four atlas-quadrant parts");
        for part in &body.parts {
            assert_eq!(part.skin.joints.len(), part.mesh.positions.len());
            let image = part.base_color_image.as_ref().expect("embedded base color");
            assert_eq!(&image[1..4], b"PNG", "embedded PNG bytes");
        }
        // Rest pose composes back to identity: bind world times inverse bind.
        let pose = engine_anim::contract::LocalPose {
            positions: body.bind.positions.clone(),
            rotations: body.bind.rotations.clone(),
        };
        let world = engine_anim::contract::skinning_matrices(&body.skeleton, &pose).expect("pose");
        let pelvis = body
            .skeleton
            .joints
            .iter()
            .position(|joint| joint.name == "pelvis")
            .expect("pelvis joint");
        let rest = mul(&body.inverse_binds[pelvis], &world[pelvis]);
        for (i, row) in rest.iter().enumerate() {
            for (k, value) in row.iter().enumerate() {
                let want = if i == k { 1.0 } else { 0.0 };
                assert!((value - want).abs() < 1e-3, "rest[{i}][{k}] = {value}");
            }
        }
    }

    #[test]
    fn clip_source_carries_bind_and_clips() {
        let bytes = std::fs::read(fighters_dir().join("clips/human_male_run.glb"))
            .expect("run clip on disk");
        let source = import_clip_source(&bytes).expect("clip imports");
        assert_eq!(source.bind.positions.len(), source.skeleton.len());
        assert_eq!(source.clips.len(), 1);
        assert!(source.clips[0].duration > 0.1);
    }

    #[test]
    fn refuses_bodies_without_skin_and_garbage() {
        assert!(matches!(
            import_skinned_body(b"not a gltf"),
            Err(BodyImportError::Mesh(_))
        ));
        let bytes = std::fs::read(fighters_dir().join("clips/human_male_run.glb"))
            .expect("run clip on disk");
        assert!(
            import_skinned_body(&bytes).is_err(),
            "a clip file has no body"
        );
    }
}
