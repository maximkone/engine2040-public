//! gltf_import: glTF 2.0 materials and `KHR_materials_*` to records.
//!
//! Reads the JSON of a `.gltf` or the JSON chunk of a `.glb` with this
//! crate's own bounded parser, so every extension, typed by the `gltf` crate
//! or not (clearcoat, sheen, anisotropy, iridescence, diffuse transmission,
//! dispersion), is read the same way from the raw document. Std-only; no
//! buffer, image or URI is ever opened.
//!
//! The import receipt is the contract: every field the record cannot hold is
//! named in [`Import::warnings`] as [`WarningKind::Unsupported`] (dropped) or
//! [`WarningKind::Approximated`] (mapped onto the nearest lobe), and a value
//! pulled into range is [`WarningKind::Clamped`]. Nothing is dropped
//! silently. Draw-state fields that are not look (`doubleSided`) are listed in
//! [`Import::ignored`].
//!
//! Spec meaning (formulas are math, read from the Khronos extension texts):
//! `ior` stays `specular_ior`; `attenuationColor` at `attenuationDistance`
//! becomes the tint reached at `thickness` (`color^(thickness/distance)`);
//! `anisotropyRotation` radians become turns.

use engine_destruction::material::{
    DecalKind, DustKind, MaterialRow, ShardRule, SoundKind, SparkKind,
};

use crate::fixed::decimal_to_q16;
use crate::json::{self, JsonError, Value};
use crate::record::{
    Anisotropy, Coat, Domain, Emission, Look, MaterialRecord, Sheen, Subsurface, Textures,
    Transmission, MAX_LUMINANCE, MAX_NAME_BYTES, MAX_TEXTURE_REF_BYTES, RECORD_VERSION,
};

/// Largest JSON document (or `.glb` JSON chunk) read: 8 MiB.
pub const MAX_GLTF_JSON_BYTES: usize = 8 * 1024 * 1024;

/// Extensions this importer reads (fully or approximately). Anything else a
/// material names is reported as unsupported.
const KNOWN: &[&str] = &[
    "KHR_materials_emissive_strength",
    "KHR_materials_ior",
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
    "KHR_materials_anisotropy",
    "KHR_materials_transmission",
    "KHR_materials_volume",
    "KHR_materials_diffuse_transmission",
    "KHR_materials_specular",
    "KHR_materials_unlit",
    "KHR_materials_pbrSpecularGlossiness",
    "KHR_materials_iridescence",
    "KHR_materials_dispersion",
    "KHR_texture_transform",
];

/// What happened to a field the record cannot hold exactly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WarningKind {
    /// Dropped: the record has nothing for it.
    Unsupported,
    /// Mapped onto the nearest lobe; looks different from the source.
    Approximated,
    /// Out of the record's range, pulled to the nearest bound.
    Clamped,
}

/// One named loss.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Warning {
    /// Record name the field belonged to.
    pub material: String,
    /// glTF path of the field (`KHR_materials_iridescence`, `alphaMode`).
    pub field: String,
    /// What happened.
    pub kind: WarningKind,
}

/// The import receipt.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Import {
    /// One record per glTF material, in file order.
    pub records: Vec<MaterialRecord>,
    /// Every unsupported, approximated or clamped field, in file order.
    pub warnings: Vec<Warning>,
    /// Draw-state fields noticed and left to the mesh draw (`doubleSided`).
    pub ignored: Vec<String>,
}

impl Import {
    /// True when `material` imported with no warning at all.
    #[must_use]
    pub fn is_exact(&self, material: &str) -> bool {
        !self.warnings.iter().any(|w| w.material == material)
    }

    /// Number of records with no warning.
    #[must_use]
    pub fn exact_count(&self) -> usize {
        self.records
            .iter()
            .filter(|r| self.is_exact(&r.name))
            .count()
    }
}

/// Why a file was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportError {
    /// The JSON did not parse.
    Json(JsonError),
    /// A `.glb` whose header or chunks are wrong; what is wrong.
    Glb(&'static str),
    /// The document has no `asset.version` of 2.x.
    NotGltf2,
    /// The JSON text is not UTF-8.
    NotUtf8,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::Glb(why) => write!(f, "glb: {why}"),
            Self::NotGltf2 => f.write_str("not a glTF 2.x document"),
            Self::NotUtf8 => f.write_str("json is not utf-8"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Import every material of a `.gltf` or `.glb` byte image.
pub fn import_gltf(bytes: &[u8]) -> Result<Import, ImportError> {
    let json_bytes = if bytes.starts_with(b"glTF") {
        glb_json_chunk(bytes)?
    } else {
        bytes
    };
    let text = std::str::from_utf8(json_bytes).map_err(|_| ImportError::NotUtf8)?;
    let doc = json::parse_up_to(text, MAX_GLTF_JSON_BYTES).map_err(ImportError::Json)?;
    let version = doc
        .get("asset")
        .and_then(|a| a.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if !version.starts_with('2') {
        return Err(ImportError::NotGltf2);
    }
    let mut out = Import::default();
    let materials = doc
        .get("materials")
        .and_then(Value::as_array)
        .unwrap_or(&[]);
    for (index, material) in materials.iter().enumerate() {
        let record = import_material(&doc, material, index, &mut out);
        out.records.push(record);
    }
    Ok(out)
}

fn glb_json_chunk(bytes: &[u8]) -> Result<&[u8], ImportError> {
    let word = |at: usize| -> Option<u32> {
        let slice = bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    };
    if word(4) != Some(2) {
        return Err(ImportError::Glb("version is not 2"));
    }
    let total = word(8).ok_or(ImportError::Glb("truncated header"))? as usize;
    if total > bytes.len() {
        return Err(ImportError::Glb("length past the end of the file"));
    }
    let len = word(12).ok_or(ImportError::Glb("no first chunk"))? as usize;
    if word(16) != Some(0x4E4F_534A) {
        return Err(ImportError::Glb("first chunk is not JSON"));
    }
    bytes
        .get(20..20 + len)
        .ok_or(ImportError::Glb("json chunk past the end"))
}

fn num(value: Option<&Value>) -> Option<f64> {
    value?
        .number_text()?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn f(value: Option<&Value>, default: f32) -> f32 {
    num(value).map_or(default, |v| v as f32)
}

fn rgb_of(value: Option<&Value>, default: [f32; 3]) -> [f32; 3] {
    let Some(items) = value.and_then(Value::as_array) else {
        return default;
    };
    let mut out = default;
    for (slot, item) in out.iter_mut().zip(items) {
        if let Some(v) = num(Some(item)) {
            *slot = v as f32;
        }
    }
    out
}

/// `raw` with every byte outside the record name rule (`0-9 A-Z a-z - _ .`) turned into `_`.
pub(crate) fn clean_ident(raw: &str) -> String {
    raw.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                char::from(b)
            } else {
                '_'
            }
        })
        .collect()
}

/// A record name from a source material name: bytes outside the record rule
/// become `_`; empty names become `material_<index>`.
pub(crate) fn record_name(raw: Option<&str>, index: usize, taken: &[MaterialRecord]) -> String {
    let mut name = clean_ident(raw.unwrap_or(""));
    if name.is_empty() {
        name = format!("material_{index}");
    }
    name.truncate(MAX_NAME_BYTES - 8);
    if taken.iter().any(|r| r.name == name) {
        name = format!("{name}_{index}");
    }
    name
}

/// Where one material's losses are written.
pub(crate) struct Ctx<'a> {
    pub(crate) material: &'a str,
    pub(crate) warnings: &'a mut Vec<Warning>,
}

impl Ctx<'_> {
    pub(crate) fn warn(&mut self, field: &str, kind: WarningKind) {
        self.warnings.push(Warning {
            material: self.material.to_string(),
            field: field.to_string(),
            kind,
        });
    }

    pub(crate) fn unit(&mut self, field: &str, v: f32) -> f32 {
        let c = v.clamp(0.0, 1.0);
        if (c - v).abs() > f32::EPSILON {
            self.warn(field, WarningKind::Clamped);
        }
        c
    }

    pub(crate) fn color(&mut self, field: &str, v: [f32; 3]) -> [f32; 3] {
        [
            self.unit(field, v[0]),
            self.unit(field, v[1]),
            self.unit(field, v[2]),
        ]
    }
}

/// `tex:<image name>` for a glTF `textureInfo`, or `None` when absent.
fn texture_ref(doc: &Value, info: Option<&Value>) -> Option<String> {
    let index = num(info?.get("index"))? as usize;
    let texture = doc.get("textures")?.as_array()?.get(index)?;
    // `KHR_texture_basisu` and friends carry the image under an extension.
    let source = num(texture.get("source")).or_else(|| {
        texture
            .get("extensions")?
            .as_object()?
            .iter()
            .find_map(|(_, ext)| num(ext.get("source")))
    })? as usize;
    let image = doc.get("images")?.as_array()?.get(source)?;
    let raw = image
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            image.get("uri").and_then(Value::as_str).map(|u| {
                let last = u.rsplit(['/', '\\']).next().unwrap_or(u);
                last.rsplit_once('.')
                    .map_or(last, |(stem, _)| stem)
                    .to_string()
            })
        })
        .unwrap_or_else(|| format!("image{source}"));
    Some(texture_ref_of(&raw))
}

/// `tex:<name>` for an image file name or stem, cut to the record's texture-ref budget.
pub(crate) fn texture_ref_of(raw: &str) -> String {
    let mut name = clean_ident(raw);
    name.truncate(MAX_TEXTURE_REF_BYTES - 4);
    format!("tex:{name}")
}

/// The sim half a source material cannot name: stone, or metal / glass by the look.
fn default_body(look: &Look) -> MaterialRow {
    let sound = if look.base_metalness > 0.5 {
        SoundKind::Metal
    } else if look.transmission.is_some() {
        SoundKind::Glass
    } else {
        SoundKind::Stone
    };
    MaterialRow {
        toughness_q16: decimal_to_q16("2").unwrap_or(1),
        brittleness_q16: decimal_to_q16("0.5").unwrap_or(0),
        shard: ShardRule::Mixed,
        sound,
        dust: DustKind::None,
        spark: SparkKind::None,
        decal: DecalKind::Crack,
        burn_q16: 0,
    }
}

fn import_material(
    doc: &Value,
    material: &Value,
    index: usize,
    out: &mut Import,
) -> MaterialRecord {
    let name = record_name(
        material.get("name").and_then(Value::as_str),
        index,
        &out.records,
    );
    let mut cx = Ctx {
        material: &name,
        warnings: &mut out.warnings,
    };
    let mut look = Look::default();
    let mut textures = Textures::default();

    // Core metallic-roughness.
    let pbr = material.get("pbrMetallicRoughness");
    if let Some(pbr) = pbr {
        let base = rgb_of(pbr.get("baseColorFactor"), [1.0; 3]);
        look.base_color = cx.color("pbrMetallicRoughness.baseColorFactor", base);
        let alpha = num(pbr
            .get("baseColorFactor")
            .and_then(Value::as_array)
            .and_then(|a| a.get(3)));
        if alpha.is_some_and(|a| (a - 1.0).abs() > 1e-6) {
            cx.warn(
                "pbrMetallicRoughness.baseColorFactor[3]",
                WarningKind::Unsupported,
            );
        }
        let metal = f(pbr.get("metallicFactor"), 1.0);
        look.base_metalness = cx.unit("pbrMetallicRoughness.metallicFactor", metal);
        let rough = f(pbr.get("roughnessFactor"), 1.0);
        look.specular_roughness = cx.unit("pbrMetallicRoughness.roughnessFactor", rough);
        textures.base_color = texture_ref(doc, pbr.get("baseColorTexture"));
        textures.orm = texture_ref(doc, pbr.get("metallicRoughnessTexture"));
    } else {
        // glTF defaults when the block is absent.
        look.base_metalness = 1.0;
        look.specular_roughness = 1.0;
    }
    if let Some(normal) = material.get("normalTexture") {
        textures.normal = texture_ref(doc, Some(normal));
        let scale = f(normal.get("scale"), 1.0);
        look.normal_scale = scale.clamp(0.0, crate::record::MAX_NORMAL_SCALE);
        if (look.normal_scale - scale).abs() > f32::EPSILON {
            cx.warn("normalTexture.scale", WarningKind::Clamped);
        }
    }
    if let Some(occ) = material.get("occlusionTexture") {
        let strength = f(occ.get("strength"), 1.0);
        look.occlusion_strength = cx.unit("occlusionTexture.strength", strength);
        let occ_ref = texture_ref(doc, Some(occ));
        match (&occ_ref, &textures.orm) {
            (Some(o), Some(m)) if o == m => {}
            (Some(_), Some(_)) => cx.warn(
                "occlusionTexture(separate from metallicRoughness)",
                WarningKind::Unsupported,
            ),
            (Some(o), None) => textures.orm = Some(o.clone()),
            (None, _) => {}
        }
    }
    textures.emissive = texture_ref(doc, material.get("emissiveTexture"));
    let emissive = rgb_of(material.get("emissiveFactor"), [0.0; 3]);
    let ext = material.get("extensions");
    let ext_of = |key: &str| ext.and_then(|e| e.get(key));
    let strength = f(
        ext_of("KHR_materials_emissive_strength").and_then(|e| e.get("emissiveStrength")),
        1.0,
    );
    if emissive.iter().any(|c| *c > 0.0) {
        let color = cx.color("emissiveFactor", emissive);
        let lum = strength / crate::compile::NITS_TO_SCENE;
        let capped = lum.clamp(0.0, MAX_LUMINANCE);
        if (capped - lum).abs() > 1.0 {
            cx.warn(
                "KHR_materials_emissive_strength.emissiveStrength",
                WarningKind::Clamped,
            );
        }
        look.emission = Some(Emission {
            color,
            luminance: capped,
        });
    }

    // KHR_materials_ior and _specular.
    if let Some(v) = ext_of("KHR_materials_ior") {
        let ior = f(v.get("ior"), 1.5);
        look.specular_ior = ior.clamp(crate::record::MIN_IOR, crate::record::MAX_IOR);
        if (look.specular_ior - ior).abs() > f32::EPSILON {
            cx.warn("KHR_materials_ior.ior", WarningKind::Clamped);
        }
    }
    if let Some(v) = ext_of("KHR_materials_specular") {
        let sf = f(v.get("specularFactor"), 1.0);
        let sc = rgb_of(v.get("specularColorFactor"), [1.0; 3]);
        if (sf - 1.0).abs() > 1e-6 || sc.iter().any(|c| (c - 1.0).abs() > 1e-6) {
            cx.warn("KHR_materials_specular", WarningKind::Unsupported);
        }
        if v.get("specularTexture").is_some() || v.get("specularColorTexture").is_some() {
            cx.warn("KHR_materials_specular(texture)", WarningKind::Unsupported);
        }
    }

    // Clear coat.
    if let Some(v) = ext_of("KHR_materials_clearcoat") {
        let weight = f(v.get("clearcoatFactor"), 0.0);
        let roughness = f(v.get("clearcoatRoughnessFactor"), 0.0);
        if weight > 0.0 {
            look.coat = Some(Coat {
                weight: cx.unit("KHR_materials_clearcoat.clearcoatFactor", weight),
                roughness: cx.unit(
                    "KHR_materials_clearcoat.clearcoatRoughnessFactor",
                    roughness,
                ),
                ior: 1.5,
            });
        }
        for key in [
            "clearcoatTexture",
            "clearcoatRoughnessTexture",
            "clearcoatNormalTexture",
        ] {
            if v.get(key).is_some() {
                cx.warn(
                    &format!("KHR_materials_clearcoat.{key}"),
                    WarningKind::Unsupported,
                );
            }
        }
    }

    // Sheen.
    if let Some(v) = ext_of("KHR_materials_sheen") {
        let color = rgb_of(v.get("sheenColorFactor"), [0.0; 3]);
        let roughness = f(v.get("sheenRoughnessFactor"), 0.0);
        if color.iter().any(|c| *c > 0.0) {
            look.sheen = Some(Sheen {
                weight: 1.0,
                color: cx.color("KHR_materials_sheen.sheenColorFactor", color),
                roughness: cx.unit("KHR_materials_sheen.sheenRoughnessFactor", roughness),
            });
        }
        for key in ["sheenColorTexture", "sheenRoughnessTexture"] {
            if v.get(key).is_some() {
                cx.warn(
                    &format!("KHR_materials_sheen.{key}"),
                    WarningKind::Unsupported,
                );
            }
        }
    }

    // Anisotropy.
    if let Some(v) = ext_of("KHR_materials_anisotropy") {
        let strength = f(v.get("anisotropyStrength"), 0.0);
        let rotation = f(v.get("anisotropyRotation"), 0.0);
        if strength > 0.0 {
            let turns = (rotation / std::f32::consts::TAU).rem_euclid(1.0);
            look.anisotropy = Some(Anisotropy {
                strength: cx.unit("KHR_materials_anisotropy.anisotropyStrength", strength),
                rotation: turns,
            });
        }
        if v.get("anisotropyTexture").is_some() {
            cx.warn(
                "KHR_materials_anisotropy.anisotropyTexture",
                WarningKind::Unsupported,
            );
        }
    }

    // Transmission and volume.
    if let Some(v) = ext_of("KHR_materials_transmission") {
        let weight = f(v.get("transmissionFactor"), 0.0);
        if weight > 0.0 {
            let (thickness, color) = match ext_of("KHR_materials_volume") {
                Some(vol) => {
                    let thickness = f(vol.get("thicknessFactor"), 0.0);
                    let color = rgb_of(vol.get("attenuationColor"), [1.0; 3]);
                    let distance = num(vol.get("attenuationDistance"));
                    let t = thickness.clamp(0.0, crate::record::MAX_THICKNESS_M);
                    if (t - thickness).abs() > f32::EPSILON {
                        cx.warn("KHR_materials_volume.thicknessFactor", WarningKind::Clamped);
                    }
                    let tint = match distance {
                        Some(d) if d > 0.0 => {
                            let ratio = (f64::from(t) / d) as f32;
                            [
                                color[0].clamp(0.0, 1.0).powf(ratio),
                                color[1].clamp(0.0, 1.0).powf(ratio),
                                color[2].clamp(0.0, 1.0).powf(ratio),
                            ]
                        }
                        _ => [1.0; 3],
                    };
                    if vol.get("thicknessTexture").is_some() {
                        cx.warn(
                            "KHR_materials_volume.thicknessTexture",
                            WarningKind::Unsupported,
                        );
                    }
                    (t, tint)
                }
                None => (0.0, [1.0; 3]),
            };
            look.transmission = Some(Transmission {
                weight: cx.unit("KHR_materials_transmission.transmissionFactor", weight),
                thickness,
                attenuation_color: color,
            });
        }
        if v.get("transmissionTexture").is_some() {
            cx.warn(
                "KHR_materials_transmission.transmissionTexture",
                WarningKind::Unsupported,
            );
        }
    }

    // Diffuse transmission: the nearest lobe is wrapped subsurface.
    if let Some(v) = ext_of("KHR_materials_diffuse_transmission") {
        let weight = f(v.get("diffuseTransmissionFactor"), 0.0);
        if weight > 0.0 {
            let color = rgb_of(v.get("diffuseTransmissionColorFactor"), [1.0; 3]);
            look.subsurface = Some(Subsurface {
                weight: cx.unit("KHR_materials_diffuse_transmission", weight),
                radius: [0.01; 3],
                color: cx.color("KHR_materials_diffuse_transmission.color", color),
            });
            cx.warn(
                "KHR_materials_diffuse_transmission",
                WarningKind::Approximated,
            );
        }
    }

    // Unlit: emission carries the base color.
    if ext_of("KHR_materials_unlit").is_some() {
        let lum = 1.0 / crate::compile::NITS_TO_SCENE;
        look.emission = Some(Emission {
            color: look.base_color,
            luminance: lum,
        });
        look.base_color = [0.0; 3];
        look.base_metalness = 0.0;
        cx.warn("KHR_materials_unlit", WarningKind::Approximated);
    }

    // Legacy specular-glossiness: diffuse color and inverted glossiness.
    if let Some(v) = ext_of("KHR_materials_pbrSpecularGlossiness") {
        let diffuse = rgb_of(v.get("diffuseFactor"), [1.0; 3]);
        look.base_color = cx.color("KHR_materials_pbrSpecularGlossiness.diffuseFactor", diffuse);
        let gloss = f(v.get("glossinessFactor"), 1.0);
        look.specular_roughness = 1.0
            - cx.unit(
                "KHR_materials_pbrSpecularGlossiness.glossinessFactor",
                gloss,
            );
        look.base_metalness = 0.0;
        cx.warn(
            "KHR_materials_pbrSpecularGlossiness",
            WarningKind::Approximated,
        );
    }

    // Named losses: no lobe for these.
    for key in ["KHR_materials_iridescence", "KHR_materials_dispersion"] {
        if let Some(v) = ext_of(key) {
            let active = match key {
                "KHR_materials_iridescence" => f(v.get("iridescenceFactor"), 0.0) > 0.0,
                _ => f(v.get("dispersion"), 0.0) > 0.0,
            };
            if active {
                cx.warn(key, WarningKind::Unsupported);
            }
        }
    }
    if let Some(entries) = ext.and_then(Value::as_object) {
        for (key, _) in entries {
            if !KNOWN.contains(&key.as_str()) {
                cx.warn(key, WarningKind::Unsupported);
            }
        }
    }
    if let Some(v) = pbr {
        for key in ["baseColorTexture", "metallicRoughnessTexture"] {
            if let Some(info) = v.get(key) {
                if info
                    .get("extensions")
                    .and_then(|e| e.get("KHR_texture_transform"))
                    .is_some()
                {
                    cx.warn("KHR_texture_transform", WarningKind::Unsupported);
                    break;
                }
            }
        }
    }
    match material.get("alphaMode").and_then(Value::as_str) {
        None | Some("OPAQUE") => {}
        Some(_) => cx.warn("alphaMode", WarningKind::Unsupported),
    }
    if material.get("doubleSided").and_then(Value::as_bool) == Some(true) {
        out.ignored.push(format!("{name}: doubleSided"));
    }

    look.textures = textures;
    plain_record(name, look)
}

/// A surface record with no gameplay layers around an imported `look`.
pub(crate) fn plain_record(name: String, look: Look) -> MaterialRecord {
    let body = default_body(&look);
    MaterialRecord {
        name,
        version: RECORD_VERSION,
        extends: None,
        domain: Domain::Surface,
        look,
        layers: Vec::new(),
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = r#"{"asset":{"version":"2.0"},"images":[{"name":"albedo"},{"uri":"maps/rough_a.png"}],"textures":[{"source":0},{"source":1}],"materials":["#;

    fn import(materials: &str) -> Import {
        import_gltf(format!("{HEAD}{materials}]}}").as_bytes()).expect("imports")
    }

    #[test]
    fn plain_metal_rough_maps_exactly() {
        let imp = import(
            r#"{"name":"Steel Plate","pbrMetallicRoughness":{"baseColorFactor":[0.5,0.5,0.55,1],"metallicFactor":1,"roughnessFactor":0.3,"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":1}},"doubleSided":true}"#,
        );
        let r = &imp.records[0];
        assert_eq!(r.name, "Steel_Plate");
        assert_eq!(r.look.base_metalness, 1.0);
        assert_eq!(r.look.textures.base_color.as_deref(), Some("tex:albedo"));
        assert_eq!(r.look.textures.orm.as_deref(), Some("tex:rough_a"));
        assert!(imp.is_exact("Steel_Plate"));
        assert_eq!(imp.ignored, vec!["Steel_Plate: doubleSided".to_string()]);
        // Round-trips through the canonical text.
        let again = MaterialRecord::parse(&r.to_text()).expect("canonical");
        assert_eq!(&again, r);
    }

    #[test]
    fn khr_extensions_land_in_lobes() {
        let imp = import(
            r#"{"name":"lamp","pbrMetallicRoughness":{},"emissiveFactor":[1,0.5,0],
            "extensions":{
              "KHR_materials_emissive_strength":{"emissiveStrength":5},
              "KHR_materials_ior":{"ior":1.33},
              "KHR_materials_clearcoat":{"clearcoatFactor":0.8,"clearcoatRoughnessFactor":0.1},
              "KHR_materials_sheen":{"sheenColorFactor":[0.5,0.5,0.5],"sheenRoughnessFactor":0.4},
              "KHR_materials_anisotropy":{"anisotropyStrength":0.6,"anisotropyRotation":3.14159265},
              "KHR_materials_transmission":{"transmissionFactor":0.9},
              "KHR_materials_volume":{"thicknessFactor":2,"attenuationDistance":4,"attenuationColor":[0.25,1,1]}
            }}"#,
        );
        let r = &imp.records[0];
        assert!(imp.is_exact("lamp"), "{:?}", imp.warnings);
        assert_eq!(r.look.specular_ior, 1.33);
        assert_eq!(r.look.coat.map(|c| c.weight), Some(0.8));
        assert_eq!(r.look.sheen.map(|s| s.roughness), Some(0.4));
        let turns = r.look.anisotropy.expect("aniso").rotation;
        assert!((turns - 0.5).abs() < 1e-4);
        let t = r.look.transmission.expect("transmission");
        assert!(
            (t.attenuation_color[0] - 0.5).abs() < 1e-5,
            "{:?}",
            t.attenuation_color
        );
        let e = r.look.emission.expect("emission");
        assert!((e.luminance * crate::compile::NITS_TO_SCENE - 5.0).abs() < 1e-3);
        MaterialRecord::parse(&r.to_text()).expect("canonical");
    }

    #[test]
    fn nothing_is_dropped_silently() {
        let imp = import(
            r#"{"name":"oil","pbrMetallicRoughness":{"baseColorFactor":[1,1,1,0.5]},"alphaMode":"BLEND",
            "extensions":{"KHR_materials_iridescence":{"iridescenceFactor":1},
                          "KHR_materials_dispersion":{"dispersion":0.3},
                          "KHR_materials_unlit":{},
                          "EXT_something_new":{}}}"#,
        );
        let fields: Vec<&str> = imp.warnings.iter().map(|w| w.field.as_str()).collect();
        for want in [
            "pbrMetallicRoughness.baseColorFactor[3]",
            "alphaMode",
            "KHR_materials_iridescence",
            "KHR_materials_dispersion",
            "KHR_materials_unlit",
            "EXT_something_new",
        ] {
            assert!(fields.contains(&want), "{want} missing in {fields:?}");
        }
        assert_eq!(imp.exact_count(), 0);
    }

    #[test]
    fn clamps_are_named() {
        let imp = import(
            r#"{"pbrMetallicRoughness":{"roughnessFactor":1.5},"extensions":{"KHR_materials_ior":{"ior":9}}}"#,
        );
        assert_eq!(imp.records[0].name, "material_0");
        assert_eq!(imp.records[0].look.specular_roughness, 1.0);
        let kinds: Vec<_> = imp.warnings.iter().map(|w| w.kind).collect();
        assert_eq!(kinds, vec![WarningKind::Clamped, WarningKind::Clamped]);
    }

    #[test]
    fn glb_json_chunk_is_found_and_bad_files_refuse() {
        let json = br#"{"asset":{"version":"2.0"},"materials":[{"name":"a","pbrMetallicRoughness":{}}]}  "#;
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        let total = 12 + 8 + json.len();
        glb.extend_from_slice(&u32::try_from(total).expect("small").to_le_bytes());
        glb.extend_from_slice(&u32::try_from(json.len()).expect("small").to_le_bytes());
        glb.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        glb.extend_from_slice(json);
        let imp = import_gltf(&glb).expect("glb imports");
        assert_eq!(imp.records.len(), 1);
        assert!(matches!(import_gltf(&glb[..30]), Err(ImportError::Glb(_))));
        assert_eq!(
            import_gltf(br#"{"asset":{"version":"1.0"}}"#),
            Err(ImportError::NotGltf2)
        );
        assert!(matches!(import_gltf(b"{ nope"), Err(ImportError::Json(_))));
    }

    #[test]
    fn duplicate_material_names_stay_unique() {
        let imp = import(r#"{"name":"x"},{"name":"x"}"#);
        assert_eq!(imp.records[0].name, "x");
        assert_eq!(imp.records[1].name, "x_1");
    }
}
