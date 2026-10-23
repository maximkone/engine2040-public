//! cooked: `CookedMesh`/`CookedMaterial` types that render consumes.
//! Assets import into these types; the glTF importer fills them. Units: meters,
//! UVs in `0..=1`, PBR factors in `0..=1`, tangents as `[x, y, z, w]` with
//! `w` either `-1.0` or `1.0`. Validation is typed; nothing here panics.

/// Length as the `u64` word cook digests fold; `usize` never exceeds 64 bits,
/// so the clamp is unreachable.
pub(crate) fn len_word(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Texture slot a reference fills.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureSlot {
    /// sRGB base-color map.
    BaseColor,
    /// Linear metallic-roughness map (G = roughness, B = metallic).
    MetallicRoughness,
    /// Linear normal map.
    Normal,
    /// Linear ambient-occlusion map (red channel).
    Occlusion,
    /// sRGB emissive map, linearized on cook like `BaseColor`.
    Emissive,
}

/// Reference to a cooked texture by stable asset id plus its slot.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TextureRef {
    /// Stable asset id from the registry (kept across reimports).
    pub asset: u64,
    /// Which slot this texture fills.
    pub slot: TextureSlot,
}

/// How a cooked texture's texels are encoded.
///
/// `BaseColor` albedo ships sRGB bytes that must be linearized on cook so
/// lighting works in linear space; every other slot is linear data that
/// must never be linearized (guessing washes out or darkens albedo).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextureEncoding {
    /// sRGB source, texels linearized on cook (`BaseColor` only).
    Srgb,
    /// Linear data, texels are `byte / 255` with no transfer curve.
    Linear,
}

/// One mip level: row-major linear `RGBA` floats in `0..=1`.
#[derive(Clone, PartialEq, Debug)]
pub struct Mip {
    /// Level width in texels (last level is 1).
    pub width: u32,
    /// Level height in texels (last level is 1).
    pub height: u32,
    /// `width * height` texels in row-major order.
    pub texels: Vec<[f32; 4]>,
}

/// Cooked texture: base dimensions plus the full box-filter mip chain.
#[derive(Clone, PartialEq, Debug)]
pub struct CookedTexture {
    /// Base-level width in texels.
    pub width: u32,
    /// Base-level height in texels.
    pub height: u32,
    /// Mip chain, `mips[0]` is the base level, last level is 1x1.
    pub mips: Vec<Mip>,
    /// Which material slot this texture fills.
    pub slot: TextureSlot,
    /// Transfer encoding of the texels (driven by `slot`, never guessed).
    pub encoding: TextureEncoding,
}

/// Cooked triangle mesh: parallel attribute arrays plus indices.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct CookedMesh {
    /// Positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals.
    pub normals: Vec<[f32; 3]>,
    /// First UV channel in `0..=1`.
    pub uv0: Vec<[f32; 2]>,
    /// Tangents as `[x, y, z, w]`, `w` is `-1.0` or `1.0`.
    pub tangents: Vec<[f32; 4]>,
    /// Triangle indices into the attribute arrays.
    pub indices: Vec<u32>,
}

/// Cooked PBR material: factors plus texture references plus the shading
/// family byte.
#[derive(Clone, PartialEq, Debug)]
pub struct CookedMaterial {
    /// sRGB base-color factor, components in `0..=1`.
    pub base_color_factor: [f32; 4],
    /// Metallic factor in `0..=1`.
    pub metallic_factor: f32,
    /// Roughness factor in `0..=1`.
    pub roughness_factor: f32,
    /// Optional texture references by slot.
    pub textures: Vec<TextureRef>,
    /// Shading family: [`MATERIAL_KIND_STONE`] and up. Imported from the
    /// source material name, never guessed from factors, because factors
    /// cannot tell cloth from skin.
    pub kind: u8,
}

/// Kind byte for today's single-BRDF path; every older cook without the byte
/// loads as this, so stone stays the default everywhere.
pub const MATERIAL_KIND_STONE: u8 = 0;
/// Kind byte for the skin lobe (wrap plus thickness transmittance).
pub const MATERIAL_KIND_SKIN: u8 = 1;
/// Kind byte for the anisotropic metal lobe.
pub const MATERIAL_KIND_METAL: u8 = 2;
/// Kind byte for the Charlie sheen cloth lobe.
pub const MATERIAL_KIND_CLOTH: u8 = 3;
/// Highest kind byte a cook writes; a higher one is a refused payload.
pub const MATERIAL_KIND_MAX: u8 = MATERIAL_KIND_CLOTH;

/// Classify a source material name into a kind byte. Names split on every
/// byte outside `0-9 A-Z a-z` and compare lowercase, so `MAT_banner_cloth`
/// carries `cloth` and `MAT_sword_steel` carries `steel`. A name with no
/// family token stays stone: a body whose parts are named `MAT_hero_q0` keeps
/// today's path until its owner maps kinds per part.
pub fn material_kind_from_name(name: &str) -> u8 {
    for token in name.split(|c: char| !c.is_ascii_alphanumeric()) {
        // Cold import path only: one lowercase buffer per token.
        match token.to_ascii_lowercase().as_str() {
            "skin" | "flesh" => return MATERIAL_KIND_SKIN,
            "metal" | "steel" | "iron" | "bronze" | "brass" | "copper" | "gold" | "silver"
            | "mail" | "blade" => return MATERIAL_KIND_METAL,
            "cloth" | "fabric" | "wool" | "linen" | "silk" | "leather" => {
                return MATERIAL_KIND_CLOTH
            }
            _ => {}
        }
    }
    MATERIAL_KIND_STONE
}

/// The family token a compact glTF appends to a material name, so a compact
/// file re-imports with the kind byte its source carried.
pub const fn material_kind_token(kind: u8) -> &'static str {
    match kind {
        MATERIAL_KIND_SKIN => "skin",
        MATERIAL_KIND_METAL => "metal",
        MATERIAL_KIND_CLOTH => "cloth",
        _ => "stone",
    }
}

impl Default for CookedMaterial {
    fn default() -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 1.0,
            textures: Vec::new(),
            kind: MATERIAL_KIND_STONE,
        }
    }
}

/// Cooked-data failures: bounded diagnostics naming the bad field.
#[derive(Clone, PartialEq, Debug)]
pub enum CookError {
    /// No vertices at all.
    EmptyMesh,
    /// Attribute array length differs from positions; field + lengths carried.
    LengthMismatch {
        /// Attribute that disagrees (`normals`, `uv0`, `tangents`).
        field: &'static str,
        /// Vertex count from positions.
        expected: usize,
        /// Count of the disagreeing array.
        got: usize,
    },
    /// Index addresses no vertex, culprit index carried.
    IndexOutOfRange(u32),
    /// Non-finite float in the named field.
    NonFinite(&'static str),
    /// PBR factor outside `0..=1`, field + value carried.
    BadFactor {
        /// `base_color`, `metallic` or `roughness`.
        field: &'static str,
        /// Offending value.
        value: f32,
    },
    /// Two references fill the same texture slot.
    DuplicateSlot(TextureSlot),
    /// Kind byte outside the four this cook writes.
    BadKind(u8),
    /// Texture bytes failed to decode; bounded decoder detail.
    TextureDecode(String),
    /// Texture input or pixel count exceeds the cook budget; count carried.
    TextureTooLarge {
        /// Offending byte length or texel count.
        count: usize,
    },
    /// Decoded dimensions are zero or exceed the cook bound.
    BadTextureDimensions {
        /// Decoded width in texels.
        width: u32,
        /// Decoded height in texels.
        height: u32,
    },
}

fn finite3(values: &[[f32; 3]], field: &'static str) -> Result<(), CookError> {
    if values.iter().flatten().any(|v| !v.is_finite()) {
        return Err(CookError::NonFinite(field));
    }
    Ok(())
}

/// Tangent handedness rule for normal maps: `w` is exactly `1.0` or `-1.0`,
/// never `0.0` or anything in between. NaN fails both compares, so it
/// refuses without a separate finite check. Deterministic: exact float
/// compare, same input gives the same bool.
pub fn valid_tangent_handedness(w: f32) -> bool {
    w == 1.0 || w == -1.0
}

/// Validate a cooked mesh: parallel arrays, finite data, indices in range.
pub fn validate_mesh(mesh: &CookedMesh) -> Result<(), CookError> {
    if mesh.positions.is_empty() {
        return Err(CookError::EmptyMesh);
    }
    let expected = mesh.positions.len();
    for (field, len) in [
        ("normals", mesh.normals.len()),
        ("uv0", mesh.uv0.len()),
        ("tangents", mesh.tangents.len()),
    ] {
        if len != expected {
            return Err(CookError::LengthMismatch {
                field,
                expected,
                got: len,
            });
        }
    }
    finite3(&mesh.positions, "positions")?;
    finite3(&mesh.normals, "normals")?;
    if mesh
        .uv0
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(CookError::NonFinite("uv0"));
    }
    if mesh
        .tangents
        .iter()
        .any(|t| !t.iter().all(|v| v.is_finite()) || !valid_tangent_handedness(t[3]))
    {
        return Err(CookError::NonFinite("tangents"));
    }
    let bound = expected as u32;
    for index in &mesh.indices {
        if *index >= bound {
            return Err(CookError::IndexOutOfRange(*index));
        }
    }
    Ok(())
}

/// Validate a cooked material: finite factors in range, one ref per slot.
pub fn validate_material(material: &CookedMaterial) -> Result<(), CookError> {
    if material
        .base_color_factor
        .iter()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err(CookError::BadFactor {
            field: "base_color",
            value: material
                .base_color_factor
                .iter()
                .copied()
                .find(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                .unwrap_or(f32::NAN),
        });
    }
    for (field, value) in [
        ("metallic", material.metallic_factor),
        ("roughness", material.roughness_factor),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(CookError::BadFactor { field, value });
        }
    }
    let mut seen: u8 = 0;
    for reference in &material.textures {
        let bit = match reference.slot {
            TextureSlot::BaseColor => 0x01,
            TextureSlot::MetallicRoughness => 0x02,
            TextureSlot::Normal => 0x04,
            TextureSlot::Occlusion => 0x08,
            TextureSlot::Emissive => 0x10,
        };
        if seen & bit != 0 {
            return Err(CookError::DuplicateSlot(reference.slot));
        }
        seen |= bit;
    }
    if material.kind > MATERIAL_KIND_MAX {
        return Err(CookError::BadKind(material.kind));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh() -> CookedMesh {
        CookedMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn valid_mesh_and_material_pass() {
        validate_mesh(&mesh()).unwrap();
        validate_material(&CookedMaterial::default()).unwrap();
    }

    #[test]
    fn mesh_rejections_name_the_field() {
        assert_eq!(
            validate_mesh(&CookedMesh::default()),
            Err(CookError::EmptyMesh)
        );
        let mut bad = mesh();
        bad.normals.pop();
        assert_eq!(
            validate_mesh(&bad),
            Err(CookError::LengthMismatch {
                field: "normals",
                expected: 3,
                got: 2
            })
        );
        let mut bad = mesh();
        bad.indices = vec![0, 1, 9];
        assert_eq!(validate_mesh(&bad), Err(CookError::IndexOutOfRange(9)));
        let mut bad = mesh();
        bad.positions[0][0] = f32::NAN;
        assert_eq!(validate_mesh(&bad), Err(CookError::NonFinite("positions")));
        let mut bad = mesh();
        bad.tangents[0][3] = 0.0;
        assert_eq!(validate_mesh(&bad), Err(CookError::NonFinite("tangents")));
    }

    #[test]
    fn tangent_handedness_accepts_only_unit_w() {
        assert!(valid_tangent_handedness(1.0));
        assert!(valid_tangent_handedness(-1.0));
        for w in [0.0, 0.5, -0.5, 2.0, f32::INFINITY, f32::NAN] {
            assert!(!valid_tangent_handedness(w), "w {w} must refuse");
        }
        let mut keep = mesh();
        keep.tangents[0][3] = -1.0;
        validate_mesh(&keep).expect("w -1.0 stays valid");
        keep.tangents[0][3] = 0.0;
        assert_eq!(validate_mesh(&keep), Err(CookError::NonFinite("tangents")));
    }

    #[test]
    fn material_kind_names() {
        for (name, want) in [
            ("MAT_banner_cloth", MATERIAL_KIND_CLOTH),
            ("MAT_sword_steel", MATERIAL_KIND_METAL),
            ("Skin_torso", MATERIAL_KIND_SKIN),
            ("LEATHER-strap", MATERIAL_KIND_CLOTH),
            ("MAT_hero_q0", MATERIAL_KIND_STONE),
            ("", MATERIAL_KIND_STONE),
            ("castle wall", MATERIAL_KIND_STONE),
        ] {
            assert_eq!(material_kind_from_name(name), want, "name {name:?}");
        }
        assert_eq!(
            CookedMaterial::default().kind,
            MATERIAL_KIND_STONE,
            "the default material is stone"
        );
        let bad = CookedMaterial {
            kind: MATERIAL_KIND_MAX + 1,
            ..CookedMaterial::default()
        };
        assert_eq!(
            validate_material(&bad),
            Err(CookError::BadKind(MATERIAL_KIND_MAX + 1))
        );
    }

    #[test]
    fn material_rejections_name_the_field() {
        let bad = CookedMaterial {
            roughness_factor: 2.0,
            ..CookedMaterial::default()
        };
        assert_eq!(
            validate_material(&bad),
            Err(CookError::BadFactor {
                field: "roughness",
                value: 2.0
            })
        );
        let dupe = CookedMaterial {
            textures: vec![
                TextureRef {
                    asset: 1,
                    slot: TextureSlot::Normal,
                },
                TextureRef {
                    asset: 2,
                    slot: TextureSlot::Normal,
                },
            ],
            ..CookedMaterial::default()
        };
        assert_eq!(
            validate_material(&dupe),
            Err(CookError::DuplicateSlot(TextureSlot::Normal))
        );
    }
}
