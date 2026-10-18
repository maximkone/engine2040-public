//! record: the one material record, text in, text out.
//!
//! Owner: `engine-material`. A [`MaterialRecord`] is what an AI writes: one
//! JSON object holding the look (OpenPBR names: `base_color`,
//! `specular_roughness`, `coat`, `sheen`, ...), the gameplay `layers` (wet,
//! dirt, blood, damage, burn), and the sim half `body` + `sound` + `fx`,
//! which is exactly one `engine_destruction::material::MaterialRow`: the
//! destruction table row is composed, never mirrored. Two digests keep the golden
//! still: [`MaterialRecord::sim_digest`] covers the row bytes the break tool
//! pins (toughness, brittleness, shard, sound, dust, spark, decal, burn);
//! [`MaterialRecord::look_digest`] covers everything a frame reads. Changing
//! a roughness moves only the look digest; changing a toughness moves only
//! the sim digest. Determinism: parse and emit are pure over the text; sim
//! numbers go decimal text -> Q16 in integer math (`fixed`), look numbers
//! are `f32` parsed and printed by Rust's shortest round-trip formatting.
//! Errors: [`RecordError`] names the JSON path (`layers[1].driver`) and
//! what was wrong; nothing here panics on user text.

use std::fmt;

use engine_core::Fnv64;
use engine_destruction::material::{
    digest_rows, DecalKind, DustKind, MaterialRow, ShardRule, SoundKind, SparkKind,
    MATERIAL_VERSION_V1, MAX_BRITTLENESS_Q16, MAX_BURN_Q16, MAX_TOUGHNESS_Q16,
};

use crate::fixed::{decimal_to_q16, q16_to_decimal};
use crate::json::{self, JsonError, Value};

/// Record format version this crate reads and writes.
pub const RECORD_VERSION: u32 = 1;
/// Most gameplay layers one record may stack.
pub const MAX_LAYERS: usize = 8;
/// Longest material name in bytes (the editor binding rule).
pub const MAX_NAME_BYTES: usize = 64;
/// Longest texture reference text in bytes.
pub const MAX_TEXTURE_REF_BYTES: usize = 128;
/// Lowest accepted index of refraction (vacuum).
pub const MIN_IOR: f32 = 1.0;
/// Highest accepted index of refraction (diamond is 2.42).
pub const MAX_IOR: f32 = 3.0;
/// Highest accepted emission luminance in nits.
pub const MAX_LUMINANCE: f32 = 100_000.0;
/// Highest accepted transmission thickness in meters.
pub const MAX_THICKNESS_M: f32 = 10.0;
/// Highest accepted normal-map scale.
pub const MAX_NORMAL_SCALE: f32 = 4.0;

// ---------------------------------------------------------------------------
// Record types: plain data, every field bounded by `parse`.
// ---------------------------------------------------------------------------

/// Where the record is drawn. Only surfaces today; decal, particle and
/// post domains reuse the record later with a new name here.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Domain {
    /// An opaque or transmissive surface drawn by the forward pass.
    #[default]
    Surface,
}

impl Domain {
    /// Document name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Surface => "surface",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "surface" => Some(Self::Surface),
            _ => None,
        }
    }
}

/// Clear-coat lobe (OpenPBR `coat`): a thin dielectric film over the base.
/// A wet surface is a coat with the ior of water, `1.33`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Coat {
    /// Lobe weight in `0..=1`.
    pub weight: f32,
    /// Coat roughness in `0..=1`.
    pub roughness: f32,
    /// Coat index of refraction in `1..=3`.
    pub ior: f32,
}

/// Sheen lobe (OpenPBR `fuzz`): retro-reflective fibers over cloth.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sheen {
    /// Lobe weight in `0..=1`.
    pub weight: f32,
    /// Sheen tint, linear `0..=1`.
    pub color: [f32; 3],
    /// Sheen roughness in `0..=1`.
    pub roughness: f32,
}

/// Anisotropic specular (brushed metal): strength plus tangent rotation.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Anisotropy {
    /// Elongation in `0..=1`.
    pub strength: f32,
    /// Tangent rotation in turns, `0..=1`.
    pub rotation: f32,
}

/// Thin transmission with a volume (glass, ice).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transmission {
    /// Lobe weight in `0..=1`.
    pub weight: f32,
    /// Volume thickness in meters, `0..=MAX_THICKNESS_M`.
    pub thickness: f32,
    /// Beer-Lambert tint reached at `thickness`, linear `0..=1`.
    pub attenuation_color: [f32; 3],
}

/// Subsurface scattering (skin, marble, wax): the terminator LUT of `crate::skin`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Subsurface {
    /// Lobe weight in `0..=1`.
    pub weight: f32,
    /// Scatter radius per channel in meters, `0..=1`.
    pub radius: [f32; 3],
    /// Scatter tint, linear `0..=1`.
    pub color: [f32; 3],
}

/// Emission: tint times luminance in nits.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Emission {
    /// Emission tint, linear `0..=1`.
    pub color: [f32; 3],
    /// Luminance in nits, `0..=MAX_LUMINANCE`.
    pub luminance: f32,
}

/// Texture references by slot: `tex:<name>` or an asset id text, opaque
/// here; the cook resolves them.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Textures {
    /// sRGB base color map.
    pub base_color: Option<String>,
    /// Linear occlusion (R), roughness (G), metalness (B) map.
    pub orm: Option<String>,
    /// Tangent-space normal map.
    pub normal: Option<String>,
    /// sRGB emissive map.
    pub emissive: Option<String>,
}

impl Textures {
    /// True when no slot is filled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base_color.is_none()
            && self.orm.is_none()
            && self.normal.is_none()
            && self.emissive.is_none()
    }
}

/// The look half: OpenPBR-named base plus optional lobes.
#[derive(Clone, PartialEq, Debug)]
pub struct Look {
    /// Linear base color in `0..=1`.
    pub base_color: [f32; 3],
    /// Metalness in `0..=1`.
    pub base_metalness: f32,
    /// Perceptual specular roughness in `0..=1`.
    pub specular_roughness: f32,
    /// Base index of refraction in `1..=3` (`1.5` is the dielectric default).
    pub specular_ior: f32,
    /// Normal-map strength in `0..=MAX_NORMAL_SCALE`.
    pub normal_scale: f32,
    /// Occlusion-map strength in `0..=1`.
    pub occlusion_strength: f32,
    /// Clear-coat lobe, when present.
    pub coat: Option<Coat>,
    /// Sheen lobe, when present.
    pub sheen: Option<Sheen>,
    /// Anisotropic specular, when present.
    pub anisotropy: Option<Anisotropy>,
    /// Transmission lobe, when present.
    pub transmission: Option<Transmission>,
    /// Subsurface lobe, when present.
    pub subsurface: Option<Subsurface>,
    /// Emission, when present.
    pub emission: Option<Emission>,
    /// Texture slots.
    pub textures: Textures,
}

impl Default for Look {
    /// A plain grey dielectric: what a record with no `look` block means.
    fn default() -> Self {
        Self {
            base_color: [1.0, 1.0, 1.0],
            base_metalness: 0.0,
            specular_roughness: 0.5,
            specular_ior: 1.5,
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            coat: None,
            sheen: None,
            anisotropy: None,
            transmission: None,
            subsurface: None,
            emission: None,
            textures: Textures::default(),
        }
    }
}

/// What a gameplay layer does to the look when its channel is high.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LayerKind {
    /// Darker base, roughness toward a low value, a water coat.
    Wet,
    /// Dust and grime tint, roughness up.
    Dirt,
    /// Blood tint plus a thin coat.
    Blood,
    /// Cracks and chips from strain: a decal family plus normal damage.
    Damage,
    /// Scorch decal plus ember emission.
    Burn,
}

impl LayerKind {
    /// Document name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Wet => "wet",
            Self::Dirt => "dirt",
            Self::Blood => "blood",
            Self::Damage => "damage",
            Self::Burn => "burn",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "wet" => Some(Self::Wet),
            "dirt" => Some(Self::Dirt),
            "blood" => Some(Self::Blood),
            "damage" => Some(Self::Damage),
            "burn" => Some(Self::Burn),
            _ => None,
        }
    }
}

/// Per-object coverage channel a layer reads (`0..=1`, published by the
/// presentation side from sim events; never read back into the tick).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    /// Water coverage.
    Wetness,
    /// Dust and grime coverage.
    Dirt,
    /// Blood coverage.
    Blood,
    /// Structural strain from the bonded solver.
    Strain,
    /// Heat from the fire follow-up.
    Heat,
}

impl Channel {
    /// Document name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Wetness => "wetness",
            Self::Dirt => "dirt",
            Self::Blood => "blood",
            Self::Strain => "strain",
            Self::Heat => "heat",
        }
    }

    /// Parse a document name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "wetness" => Some(Self::Wetness),
            "dirt" => Some(Self::Dirt),
            "blood" => Some(Self::Blood),
            "strain" => Some(Self::Strain),
            "heat" => Some(Self::Heat),
            _ => None,
        }
    }

    /// Every channel in wire order.
    #[must_use]
    pub const fn all() -> [Channel; 5] {
        [
            Self::Wetness,
            Self::Dirt,
            Self::Blood,
            Self::Strain,
            Self::Heat,
        ]
    }
}

/// One gameplay layer: a kind, the channel driving it and its parameters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Layer {
    /// What the layer does.
    pub kind: LayerKind,
    /// Which coverage channel drives it.
    pub driver: Channel,
    /// Multiplier on the channel, `0..=1`.
    pub strength: f32,
    /// Tint mixed over the base color at full coverage, linear `0..=1`.
    pub tint: Option<[f32; 3]>,
    /// Base-color darkening at full coverage, `0..=1`.
    pub darken: Option<f32>,
    /// Roughness the surface moves toward at full coverage, `0..=1`.
    pub roughness_to: Option<f32>,
    /// Decal family laid at full coverage.
    pub decal: Option<DecalKind>,
}

impl Layer {
    /// A layer of `kind` driven by `driver` with no parameters set.
    #[must_use]
    pub const fn new(kind: LayerKind, driver: Channel) -> Self {
        Self {
            kind,
            driver,
            strength: 1.0,
            tint: None,
            darken: None,
            roughness_to: None,
            decal: None,
        }
    }
}

/// The one material record. See the module docs.
#[derive(Clone, PartialEq, Debug)]
pub struct MaterialRecord {
    /// Material name: `1..=MAX_NAME_BYTES` bytes of `0-9 A-Z a-z - _ .`.
    pub name: String,
    /// Record format version; [`RECORD_VERSION`].
    pub version: u32,
    /// Parent record this one was written against, when any.
    pub extends: Option<String>,
    /// Where it is drawn.
    pub domain: Domain,
    /// The look half.
    pub look: Look,
    /// Gameplay layers in application order.
    pub layers: Vec<Layer>,
    /// The sim half: one destruction table row (toughness, brittleness, shard, sound,
    /// dust, spark, decal, burn).
    pub body: MaterialRow,
}

// ---------------------------------------------------------------------------
// Errors: every refusal names the path and what to fix.
// ---------------------------------------------------------------------------

/// What was wrong at a path.
#[derive(Clone, PartialEq, Debug)]
pub enum RecordErrorKind {
    /// The text is not JSON.
    Json(JsonError),
    /// A required member is absent.
    Missing,
    /// The member has another JSON kind; expected kind carried.
    Expected(&'static str),
    /// A look number is NaN or infinite.
    NotFinite,
    /// A number is outside its bounds; bounds and value as text.
    OutOfRange {
        /// Lowest accepted value.
        min: String,
        /// Highest accepted value.
        max: String,
        /// What was written.
        got: String,
    },
    /// A sim number is not a plain non-negative decimal.
    BadDecimal,
    /// An enum name nobody knows; which enum and the text carried.
    UnknownName {
        /// Which enum (`shard`, `channel`, ...).
        what: &'static str,
        /// What was written.
        got: String,
    },
    /// A key the record does not define (a typo or an unknown lobe).
    UnknownKey,
    /// A name is used twice in one set.
    Duplicate(String),
    /// The record version is newer than this crate reads.
    VersionTooNew(u32),
    /// A list is longer than its cap.
    TooMany {
        /// The cap.
        max: usize,
    },
    /// A name holds a byte outside `0-9 A-Z a-z - _ .`; offset carried.
    BadName {
        /// Byte offset of the first offender.
        at: usize,
    },
    /// A name or reference is longer than its cap; length carried.
    TooLong {
        /// Byte length found.
        len: usize,
    },
    /// `extends` names a record the set does not hold.
    UnknownParent(String),
}

/// A refused record: where and what.
#[derive(Clone, PartialEq, Debug)]
pub struct RecordError {
    /// JSON path of the offender (`look.coat.ior`, `layers[1].driver`); `$`
    /// for the document itself.
    pub path: String,
    /// What was wrong.
    pub kind: RecordErrorKind,
}

impl RecordError {
    fn at(path: &str, kind: RecordErrorKind) -> Self {
        Self {
            path: path.to_string(),
            kind,
        }
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.path)?;
        match &self.kind {
            RecordErrorKind::Json(err) => write!(f, "{err}"),
            RecordErrorKind::Missing => f.write_str("missing"),
            RecordErrorKind::Expected(kind) => write!(f, "expected {kind}"),
            RecordErrorKind::NotFinite => f.write_str("not a finite number"),
            RecordErrorKind::OutOfRange { min, max, got } => {
                write!(f, "{got} outside {min}..={max}")
            }
            RecordErrorKind::BadDecimal => f.write_str("not a plain decimal (digits.digits)"),
            RecordErrorKind::UnknownName { what, got } => write!(f, "unknown {what} {got:?}"),
            RecordErrorKind::UnknownKey => f.write_str("unknown key"),
            RecordErrorKind::Duplicate(name) => write!(f, "duplicate material {name:?}"),
            RecordErrorKind::VersionTooNew(version) => {
                write!(f, "version {version} is newer than {RECORD_VERSION}")
            }
            RecordErrorKind::TooMany { max } => write!(f, "more than {max} entries"),
            RecordErrorKind::BadName { at } => {
                write!(f, "byte {at} is outside 0-9 A-Z a-z - _ .")
            }
            RecordErrorKind::TooLong { len } => write!(f, "{len} bytes is too long"),
            RecordErrorKind::UnknownParent(name) => write!(f, "extends unknown material {name:?}"),
        }
    }
}

impl std::error::Error for RecordError {}

// ---------------------------------------------------------------------------
// Key tables: the schema in code; an unknown key is refused by path.
// ---------------------------------------------------------------------------

pub(crate) const TOP_KEYS: &[&str] = &[
    "material", "version", "extends", "domain", "look", "layers", "body", "sound", "fx",
];
pub(crate) const LOOK_KEYS: &[&str] = &[
    "base_color",
    "base_metalness",
    "specular_roughness",
    "specular_ior",
    "normal_scale",
    "occlusion_strength",
    "coat",
    "sheen",
    "anisotropy",
    "transmission",
    "subsurface",
    "emission",
    "textures",
];
pub(crate) const COAT_KEYS: &[&str] = &["weight", "roughness", "ior"];
pub(crate) const SHEEN_KEYS: &[&str] = &["weight", "color", "roughness"];
pub(crate) const ANISOTROPY_KEYS: &[&str] = &["strength", "rotation"];
pub(crate) const TRANSMISSION_KEYS: &[&str] = &["weight", "thickness", "attenuation_color"];
pub(crate) const SUBSURFACE_KEYS: &[&str] = &["weight", "radius", "color"];
pub(crate) const EMISSION_KEYS: &[&str] = &["color", "luminance"];
pub(crate) const TEXTURE_KEYS: &[&str] = &["base_color", "orm", "normal", "emissive"];
pub(crate) const LAYER_KEYS: &[&str] = &[
    "kind",
    "driver",
    "strength",
    "tint",
    "darken",
    "roughness_to",
    "decal",
];
pub(crate) const BODY_KEYS: &[&str] = &["toughness", "brittleness", "shard", "burn"];
pub(crate) const SOUND_KEYS: &[&str] = &["impact"];
pub(crate) const FX_KEYS: &[&str] = &["dust", "spark", "decal"];

// ---------------------------------------------------------------------------
// Readers: each takes the value and its path, returns typed data or the
// first refusal in document order.
// ---------------------------------------------------------------------------

fn join(path: &str, key: &str) -> String {
    if path == "$" {
        return key.to_string();
    }
    format!("{path}.{key}")
}

fn check_keys(value: &Value, path: &str, allowed: &[&str]) -> Result<(), RecordError> {
    let Some(members) = value.as_object() else {
        return Err(RecordError::at(path, RecordErrorKind::Expected("object")));
    };
    for (key, _) in members {
        if !allowed.contains(&key.as_str()) {
            return Err(RecordError::at(
                &join(path, key),
                RecordErrorKind::UnknownKey,
            ));
        }
    }
    Ok(())
}

fn str_at<'a>(value: &'a Value, path: &str) -> Result<&'a str, RecordError> {
    value
        .as_str()
        .ok_or_else(|| RecordError::at(path, RecordErrorKind::Expected("string")))
}

fn number_f32(value: &Value, path: &str) -> Result<f32, RecordError> {
    let lexeme = value
        .number_text()
        .ok_or_else(|| RecordError::at(path, RecordErrorKind::Expected("number")))?;
    let parsed: f32 = lexeme
        .parse()
        .map_err(|_| RecordError::at(path, RecordErrorKind::Expected("number")))?;
    if !parsed.is_finite() {
        return Err(RecordError::at(path, RecordErrorKind::NotFinite));
    }
    Ok(parsed)
}

fn ranged_at(value: &Value, path: &str, min: f32, max: f32) -> Result<f32, RecordError> {
    let parsed = number_f32(value, path)?;
    if parsed < min || parsed > max {
        return Err(RecordError::at(
            path,
            RecordErrorKind::OutOfRange {
                min: format!("{min}"),
                max: format!("{max}"),
                got: format!("{parsed}"),
            },
        ));
    }
    Ok(parsed)
}

fn unit_at(value: &Value, path: &str) -> Result<f32, RecordError> {
    ranged_at(value, path, 0.0, 1.0)
}

fn rgb_at(value: &Value, path: &str) -> Result<[f32; 3], RecordError> {
    let items = value
        .as_array()
        .ok_or_else(|| RecordError::at(path, RecordErrorKind::Expected("array of 3 numbers")))?;
    if items.len() != 3 {
        return Err(RecordError::at(
            path,
            RecordErrorKind::Expected("array of 3 numbers"),
        ));
    }
    let mut out = [0.0f32; 3];
    for (i, item) in items.iter().enumerate() {
        out[i] = unit_at(item, &format!("{path}[{i}]"))?;
    }
    Ok(out)
}

fn q16_at(value: &Value, path: &str, min_q16: u32, max_q16: u32) -> Result<u32, RecordError> {
    let lexeme = value
        .number_text()
        .ok_or_else(|| RecordError::at(path, RecordErrorKind::Expected("number")))?;
    let q =
        decimal_to_q16(lexeme).ok_or_else(|| RecordError::at(path, RecordErrorKind::BadDecimal))?;
    if q < min_q16 || q > max_q16 {
        return Err(RecordError::at(
            path,
            RecordErrorKind::OutOfRange {
                min: q16_to_decimal(min_q16),
                max: q16_to_decimal(max_q16),
                got: lexeme.to_string(),
            },
        ));
    }
    Ok(q)
}

fn name_at<T>(
    value: &Value,
    path: &str,
    what: &'static str,
    from_name: fn(&str) -> Option<T>,
) -> Result<T, RecordError> {
    let text = str_at(value, path)?;
    from_name(text).ok_or_else(|| {
        RecordError::at(
            path,
            RecordErrorKind::UnknownName {
                what,
                got: text.to_string(),
            },
        )
    })
}

/// Validate a material name: non-empty, at most [`MAX_NAME_BYTES`], every
/// byte in `0-9 A-Z a-z - _ .` (the editor binding rule).
fn checked_name(text: &str, path: &str) -> Result<String, RecordError> {
    checked_text(text, path, MAX_NAME_BYTES, &[])
}

/// Validate a texture reference: the name rule plus `:` for `tex:<name>`.
fn checked_ref(text: &str, path: &str) -> Result<String, RecordError> {
    checked_text(text, path, MAX_TEXTURE_REF_BYTES, b":")
}

fn checked_text(text: &str, path: &str, max: usize, extra: &[u8]) -> Result<String, RecordError> {
    if text.is_empty() {
        return Err(RecordError::at(path, RecordErrorKind::Missing));
    }
    if text.len() > max {
        return Err(RecordError::at(
            path,
            RecordErrorKind::TooLong { len: text.len() },
        ));
    }
    for (at, byte) in text.bytes().enumerate() {
        let ok = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.')
            || extra.contains(&byte);
        if !ok {
            return Err(RecordError::at(path, RecordErrorKind::BadName { at }));
        }
    }
    Ok(text.to_string())
}

fn parse_coat(value: &Value, path: &str) -> Result<Coat, RecordError> {
    check_keys(value, path, COAT_KEYS)?;
    Ok(Coat {
        weight: required(value, path, "weight", unit_at)?,
        roughness: required(value, path, "roughness", unit_at)?,
        ior: required(value, path, "ior", |v, p| ranged_at(v, p, MIN_IOR, MAX_IOR))?,
    })
}

fn parse_sheen(value: &Value, path: &str) -> Result<Sheen, RecordError> {
    check_keys(value, path, SHEEN_KEYS)?;
    Ok(Sheen {
        weight: required(value, path, "weight", unit_at)?,
        color: required(value, path, "color", rgb_at)?,
        roughness: required(value, path, "roughness", unit_at)?,
    })
}

fn parse_anisotropy(value: &Value, path: &str) -> Result<Anisotropy, RecordError> {
    check_keys(value, path, ANISOTROPY_KEYS)?;
    Ok(Anisotropy {
        strength: required(value, path, "strength", unit_at)?,
        rotation: required(value, path, "rotation", unit_at)?,
    })
}

fn parse_transmission(value: &Value, path: &str) -> Result<Transmission, RecordError> {
    check_keys(value, path, TRANSMISSION_KEYS)?;
    Ok(Transmission {
        weight: required(value, path, "weight", unit_at)?,
        thickness: required(value, path, "thickness", |v, p| {
            ranged_at(v, p, 0.0, MAX_THICKNESS_M)
        })?,
        attenuation_color: required(value, path, "attenuation_color", rgb_at)?,
    })
}

fn parse_subsurface(value: &Value, path: &str) -> Result<Subsurface, RecordError> {
    check_keys(value, path, SUBSURFACE_KEYS)?;
    Ok(Subsurface {
        weight: required(value, path, "weight", unit_at)?,
        radius: required(value, path, "radius", rgb_at)?,
        color: required(value, path, "color", rgb_at)?,
    })
}

fn parse_emission(value: &Value, path: &str) -> Result<Emission, RecordError> {
    check_keys(value, path, EMISSION_KEYS)?;
    Ok(Emission {
        color: required(value, path, "color", rgb_at)?,
        luminance: required(value, path, "luminance", |v, p| {
            ranged_at(v, p, 0.0, MAX_LUMINANCE)
        })?,
    })
}

fn parse_textures(value: &Value, path: &str, base: &Textures) -> Result<Textures, RecordError> {
    check_keys(value, path, TEXTURE_KEYS)?;
    let mut out = base.clone();
    let slots: [(&str, &mut Option<String>); 4] = [
        ("base_color", &mut out.base_color),
        ("orm", &mut out.orm),
        ("normal", &mut out.normal),
        ("emissive", &mut out.emissive),
    ];
    for (key, slot) in slots {
        match value.get(key) {
            None => {}
            Some(Value::Null) => *slot = None,
            Some(v) => {
                let p = join(path, key);
                *slot = Some(checked_ref(str_at(v, &p)?, &p)?);
            }
        }
    }
    Ok(out)
}

/// Read `key` of `value` with `read`, refusing a missing member.
fn required<T>(
    value: &Value,
    path: &str,
    key: &str,
    read: impl Fn(&Value, &str) -> Result<T, RecordError>,
) -> Result<T, RecordError> {
    let p = join(path, key);
    let Some(member) = value.get(key) else {
        return Err(RecordError::at(&p, RecordErrorKind::Missing));
    };
    read(member, &p)
}

/// Read `key` of `value` with `read` when present, else keep `current`.
fn optional<T>(
    value: &Value,
    path: &str,
    key: &str,
    current: T,
    read: impl Fn(&Value, &str) -> Result<T, RecordError>,
) -> Result<T, RecordError> {
    match value.get(key) {
        None => Ok(current),
        Some(member) => read(member, &join(path, key)),
    }
}

/// Read an optional lobe: absent keeps `current`, `null` clears it, an
/// object replaces it.
fn lobe<T>(
    value: &Value,
    path: &str,
    key: &str,
    current: Option<T>,
    read: fn(&Value, &str) -> Result<T, RecordError>,
) -> Result<Option<T>, RecordError> {
    match value.get(key) {
        None => Ok(current),
        Some(Value::Null) => Ok(None),
        Some(member) => read(member, &join(path, key)).map(Some),
    }
}

fn parse_look(value: &Value, path: &str, base: &Look) -> Result<Look, RecordError> {
    check_keys(value, path, LOOK_KEYS)?;
    let ior = |v: &Value, p: &str| ranged_at(v, p, MIN_IOR, MAX_IOR);
    let scale = |v: &Value, p: &str| ranged_at(v, p, 0.0, MAX_NORMAL_SCALE);
    Ok(Look {
        base_color: optional(value, path, "base_color", base.base_color, rgb_at)?,
        base_metalness: optional(value, path, "base_metalness", base.base_metalness, unit_at)?,
        specular_roughness: optional(
            value,
            path,
            "specular_roughness",
            base.specular_roughness,
            unit_at,
        )?,
        specular_ior: optional(value, path, "specular_ior", base.specular_ior, ior)?,
        normal_scale: optional(value, path, "normal_scale", base.normal_scale, scale)?,
        occlusion_strength: optional(
            value,
            path,
            "occlusion_strength",
            base.occlusion_strength,
            unit_at,
        )?,
        coat: lobe(value, path, "coat", base.coat, parse_coat)?,
        sheen: lobe(value, path, "sheen", base.sheen, parse_sheen)?,
        anisotropy: lobe(value, path, "anisotropy", base.anisotropy, parse_anisotropy)?,
        transmission: lobe(
            value,
            path,
            "transmission",
            base.transmission,
            parse_transmission,
        )?,
        subsurface: lobe(value, path, "subsurface", base.subsurface, parse_subsurface)?,
        emission: lobe(value, path, "emission", base.emission, parse_emission)?,
        textures: match value.get("textures") {
            None => base.textures.clone(),
            Some(t) => parse_textures(t, &join(path, "textures"), &base.textures)?,
        },
    })
}

fn parse_layer(value: &Value, path: &str) -> Result<Layer, RecordError> {
    check_keys(value, path, LAYER_KEYS)?;
    let kind = required(value, path, "kind", |v, p| {
        name_at(v, p, "layer kind", LayerKind::from_name)
    })?;
    let driver = required(value, path, "driver", |v, p| {
        name_at(v, p, "channel", Channel::from_name)
    })?;
    let mut layer = Layer::new(kind, driver);
    layer.strength = optional(value, path, "strength", 1.0, unit_at)?;
    layer.tint = optional(value, path, "tint", None, |v, p| rgb_at(v, p).map(Some))?;
    layer.darken = optional(value, path, "darken", None, |v, p| unit_at(v, p).map(Some))?;
    layer.roughness_to = optional(value, path, "roughness_to", None, |v, p| {
        unit_at(v, p).map(Some)
    })?;
    layer.decal = optional(value, path, "decal", None, |v, p| {
        name_at(v, p, "decal", DecalKind::from_name).map(Some)
    })?;
    Ok(layer)
}

fn parse_layers(value: &Value, path: &str) -> Result<Vec<Layer>, RecordError> {
    let items = value
        .as_array()
        .ok_or_else(|| RecordError::at(path, RecordErrorKind::Expected("array")))?;
    if items.len() > MAX_LAYERS {
        return Err(RecordError::at(
            path,
            RecordErrorKind::TooMany { max: MAX_LAYERS },
        ));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| parse_layer(item, &format!("{path}[{i}]")))
        .collect()
}

/// Placeholder row every stand-alone record overwrites field by field;
/// [`require_blocks`] has already proven each field is written.
const UNSET_ROW: MaterialRow = MaterialRow {
    toughness_q16: 1,
    brittleness_q16: 0,
    shard: ShardRule::Mixed,
    sound: SoundKind::Stone,
    dust: DustKind::None,
    spark: SparkKind::None,
    decal: DecalKind::Crack,
    burn_q16: 0,
};

/// Read `body`, `sound` and `fx` into one row, starting from `base` (the
/// parent's row) or requiring every sim field when there is none.
fn parse_row(doc: &Value, base: Option<MaterialRow>) -> Result<MaterialRow, RecordError> {
    let mut row = match base {
        Some(row) => row,
        None => {
            require_blocks(doc)?;
            UNSET_ROW
        }
    };
    if let Some(body) = doc.get("body") {
        check_keys(body, "body", BODY_KEYS)?;
        row.toughness_q16 = optional(body, "body", "toughness", row.toughness_q16, |v, p| {
            q16_at(v, p, 1, MAX_TOUGHNESS_Q16)
        })?;
        row.brittleness_q16 =
            optional(body, "body", "brittleness", row.brittleness_q16, |v, p| {
                q16_at(v, p, 0, MAX_BRITTLENESS_Q16)
            })?;
        row.shard = optional(body, "body", "shard", row.shard, |v, p| {
            name_at(v, p, "shard rule", ShardRule::from_name)
        })?;
        row.burn_q16 = optional(body, "body", "burn", row.burn_q16, |v, p| {
            q16_at(v, p, 0, MAX_BURN_Q16)
        })?;
    }
    if let Some(sound) = doc.get("sound") {
        check_keys(sound, "sound", SOUND_KEYS)?;
        row.sound = optional(sound, "sound", "impact", row.sound, |v, p| {
            name_at(v, p, "sound", SoundKind::from_name)
        })?;
    }
    if let Some(fx) = doc.get("fx") {
        check_keys(fx, "fx", FX_KEYS)?;
        row.dust = optional(fx, "fx", "dust", row.dust, |v, p| {
            name_at(v, p, "dust", DustKind::from_name)
        })?;
        row.spark = optional(fx, "fx", "spark", row.spark, |v, p| {
            name_at(v, p, "spark", SparkKind::from_name)
        })?;
        row.decal = optional(fx, "fx", "decal", row.decal, |v, p| {
            name_at(v, p, "decal", DecalKind::from_name)
        })?;
    }
    Ok(row)
}

/// Without a parent every sim field must be written: a default would be a
/// silent number in the tick.
fn require_blocks(doc: &Value) -> Result<(), RecordError> {
    let needed: [(&str, &[&str]); 3] =
        [("body", BODY_KEYS), ("sound", SOUND_KEYS), ("fx", FX_KEYS)];
    for (block, keys) in needed {
        let Some(value) = doc.get(block) else {
            return Err(RecordError::at(block, RecordErrorKind::Missing));
        };
        for key in keys {
            if value.get(key).is_none() {
                return Err(RecordError::at(&join(block, key), RecordErrorKind::Missing));
            }
        }
    }
    Ok(())
}

impl MaterialRecord {
    /// Parse a stand-alone record (no `extends`). A record that extends
    /// another needs [`MaterialRecord::parse_with`].
    pub fn parse(text: &str) -> Result<Self, RecordError> {
        Self::parse_with(text, None)
    }

    /// Parse a record, taking defaults from `parent` when the text names it
    /// in `extends`. A text with `extends` and no matching parent is
    /// refused ([`RecordErrorKind::UnknownParent`]).
    pub fn parse_with(text: &str, parent: Option<&MaterialRecord>) -> Result<Self, RecordError> {
        let doc = json::parse(text).map_err(|e| RecordError::at("$", RecordErrorKind::Json(e)))?;
        check_keys(&doc, "$", TOP_KEYS)?;
        let name = required(&doc, "$", "material", |v, p| checked_name(str_at(v, p)?, p))?;
        let version = required(&doc, "$", "version", version_at)?;
        let extends = match doc.get("extends") {
            None | Some(Value::Null) => None,
            Some(v) => Some(checked_name(str_at(v, "extends")?, "extends")?),
        };
        let base = match (&extends, parent) {
            (Some(wanted), Some(p)) if p.name == *wanted => Some(p),
            (Some(wanted), _) => {
                return Err(RecordError::at(
                    "extends",
                    RecordErrorKind::UnknownParent(wanted.clone()),
                ))
            }
            (None, _) => None,
        };
        let domain = optional(
            &doc,
            "$",
            "domain",
            base.map_or(Domain::Surface, |b| b.domain),
            |v, p| name_at(v, p, "domain", Domain::from_name),
        )?;
        let default_look = Look::default();
        let base_look = base.map_or(&default_look, |b| &b.look);
        let look = match doc.get("look") {
            None => base_look.clone(),
            Some(v) => parse_look(v, "look", base_look)?,
        };
        let layers = match doc.get("layers") {
            None => base.map_or_else(Vec::new, |b| b.layers.clone()),
            Some(v) => parse_layers(v, "layers")?,
        };
        let body = parse_row(&doc, base.map(|b| b.body))?;
        Ok(Self {
            name,
            version,
            extends,
            domain,
            look,
            layers,
            body,
        })
    }

    /// Canonical JSON text of the record, ending in one newline. Parsing it
    /// back gives an equal record; emitting that gives these bytes again.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = json::write(&self.to_json());
        out.push('\n');
        out
    }

    /// The record as a JSON value tree in canonical key order.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut top = vec![
            ("material".to_string(), Value::Str(self.name.clone())),
            (
                "version".to_string(),
                Value::Number(self.version.to_string()),
            ),
        ];
        if let Some(parent) = &self.extends {
            top.push(("extends".to_string(), Value::Str(parent.clone())));
        }
        top.push((
            "domain".to_string(),
            Value::Str(self.domain.name().to_string()),
        ));
        top.push(("look".to_string(), look_json(&self.look)));
        if !self.layers.is_empty() {
            top.push((
                "layers".to_string(),
                Value::Array(self.layers.iter().map(layer_json).collect()),
            ));
        }
        let row = self.body;
        top.push((
            "body".to_string(),
            obj(vec![
                (
                    "toughness",
                    Value::Number(q16_to_decimal(row.toughness_q16)),
                ),
                (
                    "brittleness",
                    Value::Number(q16_to_decimal(row.brittleness_q16)),
                ),
                ("shard", text(row.shard.name())),
                ("burn", Value::Number(q16_to_decimal(row.burn_q16))),
            ]),
        ));
        top.push((
            "sound".to_string(),
            obj(vec![("impact", text(row.sound.name()))]),
        ));
        top.push((
            "fx".to_string(),
            obj(vec![
                ("dust", text(row.dust.name())),
                ("spark", text(row.spark.name())),
                ("decal", text(row.decal.name())),
            ]),
        ));
        Value::Object(top)
    }

    /// Digest of the sim half: the table row bytes under the table version,
    /// the same stream `MaterialTable::digest` folds. Look edits never move
    /// it.
    #[must_use]
    pub fn sim_digest(&self) -> u64 {
        digest_rows(MATERIAL_VERSION_V1, &[self.body])
    }

    /// Digest of everything a frame reads: domain, look and layers, over
    /// their canonical text. Sim edits never move it.
    #[must_use]
    pub fn look_digest(&self) -> u64 {
        let frame = Value::Object(vec![
            ("domain".to_string(), text(self.domain.name())),
            ("look".to_string(), look_json(&self.look)),
            (
                "layers".to_string(),
                Value::Array(self.layers.iter().map(layer_json).collect()),
            ),
        ]);
        let mut hasher = Fnv64::new();
        hasher.write_str(&json::write(&frame));
        hasher.finish()
    }
}

fn version_at(value: &Value, path: &str) -> Result<u32, RecordError> {
    let lexeme = value
        .number_text()
        .ok_or_else(|| RecordError::at(path, RecordErrorKind::Expected("integer")))?;
    let version: u32 = lexeme
        .parse()
        .map_err(|_| RecordError::at(path, RecordErrorKind::Expected("integer")))?;
    if version == 0 {
        return Err(RecordError::at(
            path,
            RecordErrorKind::OutOfRange {
                min: "1".to_string(),
                max: RECORD_VERSION.to_string(),
                got: lexeme.to_string(),
            },
        ));
    }
    if version > RECORD_VERSION {
        return Err(RecordError::at(
            path,
            RecordErrorKind::VersionTooNew(version),
        ));
    }
    Ok(version)
}

// ---------------------------------------------------------------------------
// Writers: value trees in canonical order; numbers by shortest round-trip.
// ---------------------------------------------------------------------------

fn num(value: f32) -> Value {
    Value::Number(format!("{value}"))
}

fn text(value: &str) -> Value {
    Value::Str(value.to_string())
}

fn rgb(value: [f32; 3]) -> Value {
    Value::Array(value.iter().map(|c| num(*c)).collect())
}

fn obj(members: Vec<(&str, Value)>) -> Value {
    Value::Object(
        members
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn look_json(look: &Look) -> Value {
    let mut members = vec![
        ("base_color", rgb(look.base_color)),
        ("base_metalness", num(look.base_metalness)),
        ("specular_roughness", num(look.specular_roughness)),
        ("specular_ior", num(look.specular_ior)),
        ("normal_scale", num(look.normal_scale)),
        ("occlusion_strength", num(look.occlusion_strength)),
    ];
    if let Some(c) = look.coat {
        members.push((
            "coat",
            obj(vec![
                ("weight", num(c.weight)),
                ("roughness", num(c.roughness)),
                ("ior", num(c.ior)),
            ]),
        ));
    }
    if let Some(s) = look.sheen {
        members.push((
            "sheen",
            obj(vec![
                ("weight", num(s.weight)),
                ("color", rgb(s.color)),
                ("roughness", num(s.roughness)),
            ]),
        ));
    }
    if let Some(a) = look.anisotropy {
        members.push((
            "anisotropy",
            obj(vec![
                ("strength", num(a.strength)),
                ("rotation", num(a.rotation)),
            ]),
        ));
    }
    if let Some(t) = look.transmission {
        members.push((
            "transmission",
            obj(vec![
                ("weight", num(t.weight)),
                ("thickness", num(t.thickness)),
                ("attenuation_color", rgb(t.attenuation_color)),
            ]),
        ));
    }
    if let Some(s) = look.subsurface {
        members.push((
            "subsurface",
            obj(vec![
                ("weight", num(s.weight)),
                ("radius", rgb(s.radius)),
                ("color", rgb(s.color)),
            ]),
        ));
    }
    if let Some(e) = look.emission {
        members.push((
            "emission",
            obj(vec![
                ("color", rgb(e.color)),
                ("luminance", num(e.luminance)),
            ]),
        ));
    }
    if !look.textures.is_empty() {
        let mut slots = Vec::new();
        for (key, slot) in [
            ("base_color", &look.textures.base_color),
            ("orm", &look.textures.orm),
            ("normal", &look.textures.normal),
            ("emissive", &look.textures.emissive),
        ] {
            if let Some(name) = slot {
                slots.push((key, text(name)));
            }
        }
        members.push(("textures", obj(slots)));
    }
    obj(members)
}

fn layer_json(layer: &Layer) -> Value {
    let mut members = vec![
        ("kind", text(layer.kind.name())),
        ("driver", text(layer.driver.name())),
    ];
    if layer.strength != 1.0 {
        members.push(("strength", num(layer.strength)));
    }
    if let Some(tint) = layer.tint {
        members.push(("tint", rgb(tint)));
    }
    if let Some(darken) = layer.darken {
        members.push(("darken", num(darken)));
    }
    if let Some(roughness_to) = layer.roughness_to {
        members.push(("roughness_to", num(roughness_to)));
    }
    if let Some(decal) = layer.decal {
        members.push(("decal", text(decal.name())));
    }
    obj(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_destruction::material::{MaterialId, MaterialTable};

    const MARBLE: &str = r#"{
  "material": "marble",
  "version": 1,
  "domain": "surface",
  "look": {
    "base_color": [0.82, 0.8, 0.76],
    "base_metalness": 0,
    "specular_roughness": 0.35,
    "specular_ior": 1.5,
    "normal_scale": 1,
    "occlusion_strength": 1,
    "textures": {
      "base_color": "tex:marble_a",
      "orm": "tex:marble_orm",
      "normal": "tex:marble_n"
    }
  },
  "layers": [
    {
      "kind": "wet",
      "driver": "wetness",
      "darken": 0.35,
      "roughness_to": 0.08
    },
    {
      "kind": "blood",
      "driver": "blood",
      "tint": [0.35, 0.02, 0.02]
    },
    {
      "kind": "damage",
      "driver": "strain",
      "decal": "crack"
    }
  ],
  "body": {
    "toughness": 7,
    "brittleness": 0.9,
    "shard": "shards",
    "burn": 0
  },
  "sound": {
    "impact": "stone"
  },
  "fx": {
    "dust": "pale",
    "spark": "few",
    "decal": "shatter"
  }
}
"#;

    fn marble() -> MaterialRecord {
        MaterialRecord::parse(MARBLE).expect("marble parses")
    }

    fn fail(text: &str) -> RecordError {
        MaterialRecord::parse(text).expect_err("must refuse")
    }

    #[test]
    fn marble_round_trips_byte_identical_twice() {
        let first = marble();
        let text1 = first.to_text();
        assert_eq!(text1, MARBLE);
        let second = MaterialRecord::parse(&text1).expect("second parse");
        assert_eq!(second, first);
        assert_eq!(second.to_text(), text1);
    }

    #[test]
    fn marble_body_equals_the_bx12_row() {
        let table = MaterialTable::siege_kit();
        assert_eq!(marble().body, table.row_of(MaterialId::Marble));
        assert_eq!(marble().layers.len(), 3);
        assert_eq!(marble().layers[0].roughness_to, Some(0.08));
    }

    #[test]
    fn look_edit_moves_only_the_look_digest() {
        let base = marble();
        let mut rough = base.clone();
        rough.look.specular_roughness = 0.6;
        assert_ne!(rough.look_digest(), base.look_digest());
        assert_eq!(rough.sim_digest(), base.sim_digest());
        let mut layered = base.clone();
        layered.layers.pop();
        assert_ne!(layered.look_digest(), base.look_digest());
        assert_eq!(layered.sim_digest(), base.sim_digest());
    }

    #[test]
    fn sim_edit_moves_only_the_sim_digest() {
        let base = marble();
        let mut tough = base.clone();
        tough.body.toughness_q16 += 1;
        assert_ne!(tough.sim_digest(), base.sim_digest());
        assert_eq!(tough.look_digest(), base.look_digest());
        let mut sound = base.clone();
        sound.body.sound = SoundKind::Soft;
        assert_ne!(sound.sim_digest(), base.sim_digest());
        assert_eq!(sound.look_digest(), base.look_digest());
    }

    #[test]
    fn sim_digest_is_the_table_digest_of_one_row() {
        let one_row = digest_rows(MATERIAL_VERSION_V1, &[marble().body]);
        assert_eq!(marble().sim_digest(), one_row);
    }

    #[test]
    fn fourteen_shipped_records_roundtrip_and_match_the_table_digest() {
        use crate::set::MaterialSet;
        use engine_destruction::material::MATERIAL_COUNT;

        // Frozen record budget: the editor binding rule.
        assert_eq!(RECORD_VERSION, 1);
        assert_eq!(MAX_LAYERS, 8);
        assert_eq!(MAX_NAME_BYTES, 64);

        let kit = MaterialSet::siege_kit();
        let table = MaterialTable::siege_kit();
        assert_eq!(kit.len(), MATERIAL_COUNT);
        assert_eq!(MATERIAL_COUNT, 14);

        // Every shipped record keeps its wire name and its table row.
        for id in MaterialId::all() {
            let record = kit.get(id.name()).expect("shipped record");
            assert_eq!(record.body, table.row_of(id));
            assert_eq!(record.version, RECORD_VERSION);
        }

        // Parse plus emit is byte-identical twice, per record.
        for record in kit.records() {
            let text = record.to_text();
            let again = MaterialRecord::parse(&text).expect("canonical text parses");
            assert_eq!(&again, record);
            assert_eq!(again.to_text(), text);
            assert_eq!(
                record.sim_digest(),
                digest_rows(MATERIAL_VERSION_V1, &[record.body])
            );
        }

        // The fourteen sim halves fold to the shipped table digest.
        assert_eq!(kit.sim_digest(), table.digest());

        // A sim edit moves the digest: the lock is live, not constant.
        let mut tampered = kit.records()[0].clone();
        tampered.body.toughness_q16 += 1;
        assert_ne!(tampered.sim_digest(), kit.records()[0].sim_digest());
        assert_eq!(tampered.look_digest(), kit.records()[0].look_digest());
    }

    #[test]
    fn missing_look_means_grey_dielectric() {
        let text = r#"{"material":"plain","version":1,"body":{"toughness":1,"brittleness":0.5,"shard":"mixed","burn":0},"sound":{"impact":"stone"},"fx":{"dust":"none","spark":"none","decal":"crack"}}"#;
        let record = MaterialRecord::parse(text).expect("parses");
        assert_eq!(record.look, Look::default());
        assert!(record.layers.is_empty());
        assert_eq!(record.domain, Domain::Surface);
    }

    #[test]
    fn five_seeded_bad_records_refuse_with_a_path() {
        let not_finite = MARBLE.replace(
            "\"specular_roughness\": 0.35",
            "\"specular_roughness\": 1e999",
        );
        let err = fail(&not_finite);
        assert_eq!(err.path, "look.specular_roughness");
        assert_eq!(err.kind, RecordErrorKind::NotFinite);

        let out_of_range = MARBLE.replace("\"brittleness\": 0.9", "\"brittleness\": 1.5");
        let err = fail(&out_of_range);
        assert_eq!(err.path, "body.brittleness");
        assert_eq!(
            err.kind,
            RecordErrorKind::OutOfRange {
                min: "0".to_string(),
                max: "1".to_string(),
                got: "1.5".to_string()
            }
        );

        let unknown_lobe = MARBLE.replace(
            "\"normal_scale\": 1,",
            "\"normal_scale\": 1, \"velvet\": {},",
        );
        let err = fail(&unknown_lobe);
        assert_eq!(err.path, "look.velvet");
        assert_eq!(err.kind, RecordErrorKind::UnknownKey);

        let too_new = MARBLE.replace("\"version\": 1", "\"version\": 2");
        let err = fail(&too_new);
        assert_eq!(err.path, "version");
        assert_eq!(err.kind, RecordErrorKind::VersionTooNew(2));

        let bad_channel = MARBLE.replace("\"driver\": \"blood\"", "\"driver\": \"gore\"");
        let err = fail(&bad_channel);
        assert_eq!(err.path, "layers[1].driver");
        assert_eq!(
            err.kind,
            RecordErrorKind::UnknownName {
                what: "channel",
                got: "gore".to_string()
            }
        );
        assert_eq!(
            err.to_string(),
            "layers[1].driver: unknown channel \"gore\""
        );
    }

    #[test]
    fn sim_numbers_must_be_plain_decimals() {
        let exponent = MARBLE.replace("\"toughness\": 7", "\"toughness\": 7e0");
        let err = fail(&exponent);
        assert_eq!(err.path, "body.toughness");
        assert_eq!(err.kind, RecordErrorKind::BadDecimal);
        let zero = MARBLE.replace("\"toughness\": 7", "\"toughness\": 0");
        let err = fail(&zero);
        assert_eq!(
            err.kind,
            RecordErrorKind::OutOfRange {
                min: "0.00002".to_string(),
                max: "64".to_string(),
                got: "0".to_string()
            }
        );
    }

    #[test]
    fn stand_alone_records_must_write_every_sim_field() {
        let no_fx = MARBLE.replace(
            "  \"fx\": {\n    \"dust\": \"pale\",\n    \"spark\": \"few\",\n    \"decal\": \"shatter\"\n  }\n",
            "  \"fx\": {\n    \"dust\": \"pale\",\n    \"spark\": \"few\"\n  }\n",
        );
        assert_ne!(no_fx, MARBLE);
        let err = fail(&no_fx);
        assert_eq!(err.path, "fx.decal");
        assert_eq!(err.kind, RecordErrorKind::Missing);
        let text = r#"{"material":"x","version":1}"#;
        assert_eq!(fail(text).path, "body");
    }

    #[test]
    fn names_follow_the_editor_binding_rule() {
        let bad = MARBLE.replace("\"material\": \"marble\"", "\"material\": \"mar ble\"");
        let err = fail(&bad);
        assert_eq!(err.path, "material");
        assert_eq!(err.kind, RecordErrorKind::BadName { at: 3 });
        let long = MARBLE.replace(
            "\"material\": \"marble\"",
            &format!("\"material\": \"{}\"", "m".repeat(65)),
        );
        assert_eq!(fail(&long).kind, RecordErrorKind::TooLong { len: 65 });
        let unknown_top = MARBLE.replace("\"version\": 1,", "\"version\": 1, \"colour\": 1,");
        assert_eq!(fail(&unknown_top).path, "colour");
        let not_json = fail("{");
        assert_eq!(not_json.path, "$");
        assert!(matches!(not_json.kind, RecordErrorKind::Json(_)));
    }

    #[test]
    fn extends_takes_defaults_from_the_parent_and_overrides_fields() {
        let parent = marble();
        let child_text = r#"{"material":"marble_wet","version":1,"extends":"marble","look":{"specular_roughness":0.08,"coat":{"weight":1,"roughness":0.05,"ior":1.33}},"body":{"toughness":6.5},"sound":{"impact":"soft"}}"#;
        let child = MaterialRecord::parse_with(child_text, Some(&parent)).expect("child parses");
        assert_eq!(child.extends.as_deref(), Some("marble"));
        assert_eq!(child.look.base_color, parent.look.base_color);
        assert_eq!(child.look.specular_roughness, 0.08);
        assert_eq!(child.look.coat.map(|c| c.ior), Some(1.33));
        assert_eq!(child.layers, parent.layers);
        assert_eq!(child.body.toughness_q16, 425_984);
        assert_eq!(child.body.sound, SoundKind::Soft);
        assert_eq!(child.body.shard, parent.body.shard);
        let full = child.to_text();
        let again = MaterialRecord::parse_with(&full, Some(&parent)).expect("full form parses");
        assert_eq!(again, child);
        assert_eq!(again.to_text(), full);
    }

    #[test]
    fn extends_without_the_parent_is_refused() {
        let child_text = r#"{"material":"marble_wet","version":1,"extends":"marble"}"#;
        let err = MaterialRecord::parse_with(child_text, None).expect_err("no parent");
        assert_eq!(err.path, "extends");
        assert_eq!(
            err.kind,
            RecordErrorKind::UnknownParent("marble".to_string())
        );
        let other = marble();
        let err = MaterialRecord::parse_with(
            &child_text.replace("\"marble\"", "\"granite\""),
            Some(&other),
        )
        .expect_err("wrong parent");
        assert_eq!(
            err.kind,
            RecordErrorKind::UnknownParent("granite".to_string())
        );
    }

    #[test]
    fn null_clears_a_lobe_and_a_texture_slot_in_a_child() {
        let mut parent = marble();
        parent.look.coat = Some(Coat {
            weight: 1.0,
            roughness: 0.1,
            ior: 1.5,
        });
        let child_text = r#"{"material":"marble_dry","version":1,"extends":"marble","look":{"coat":null,"textures":{"normal":null}}}"#;
        let child = MaterialRecord::parse_with(child_text, Some(&parent)).expect("child parses");
        assert_eq!(child.look.coat, None);
        assert_eq!(child.look.textures.normal, None);
        assert_eq!(child.look.textures.orm.as_deref(), Some("tex:marble_orm"));
    }

    #[test]
    fn every_lobe_round_trips() {
        let mut record = marble();
        record.look.coat = Some(Coat {
            weight: 0.8,
            roughness: 0.1,
            ior: 1.5,
        });
        record.look.sheen = Some(Sheen {
            weight: 0.6,
            color: [0.7, 0.65, 0.6],
            roughness: 0.7,
        });
        record.look.anisotropy = Some(Anisotropy {
            strength: 0.5,
            rotation: 0.25,
        });
        record.look.transmission = Some(Transmission {
            weight: 1.0,
            thickness: 0.01,
            attenuation_color: [1.0, 1.0, 1.0],
        });
        record.look.subsurface = Some(Subsurface {
            weight: 0.8,
            radius: [0.012, 0.004, 0.002],
            color: [0.9, 0.4, 0.3],
        });
        record.look.emission = Some(Emission {
            color: [1.0, 0.5, 0.2],
            luminance: 2500.0,
        });
        record.look.textures.emissive = Some("tex:ember".to_string());
        let text = record.to_text();
        let back = MaterialRecord::parse(&text).expect("parses");
        assert_eq!(back, record);
        assert_eq!(back.to_text(), text);
    }

    #[test]
    fn layer_caps_and_scalar_array_shapes_hold() {
        let many = format!(
            "[{}]",
            [r#"{"kind":"wet","driver":"wetness"}"#; MAX_LAYERS + 1].join(",")
        );
        let text = MARBLE.replace(
            &MARBLE[MARBLE.find("[\n    {").expect("layers")
                ..=MARBLE.find("\n  ],").expect("end") + 3],
            &many,
        );
        let err = fail(&text);
        assert_eq!(err.path, "layers");
        assert_eq!(err.kind, RecordErrorKind::TooMany { max: MAX_LAYERS });
        let short_rgb = MARBLE.replace("[0.82, 0.8, 0.76]", "[0.82, 0.8]");
        let err = fail(&short_rgb);
        assert_eq!(err.path, "look.base_color");
        assert_eq!(err.kind, RecordErrorKind::Expected("array of 3 numbers"));
        let hot = MARBLE.replace("[0.82, 0.8, 0.76]", "[0.82, 0.8, 1.2]");
        assert_eq!(fail(&hot).path, "look.base_color[2]");
    }

    #[test]
    fn names_tables_round_trip_every_variant() {
        for channel in Channel::all() {
            assert_eq!(Channel::from_name(channel.name()), Some(channel));
        }
        for kind in [
            LayerKind::Wet,
            LayerKind::Dirt,
            LayerKind::Blood,
            LayerKind::Damage,
            LayerKind::Burn,
        ] {
            assert_eq!(LayerKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(Domain::from_name("surface"), Some(Domain::Surface));
        assert_eq!(Domain::from_name("decal"), None);
    }
}
