//! live: HR2 live shader, material and texture reload through HR1 notices.
//!
//! The assets lane watches files, debounces and recooks them (HR1) and hands
//! the caller an immutable versioned [`ReloadNotice`]; this module owns the
//! render-side publication boundary the HR1 docs leave to consumers. HR5
//! (practice mode) fetches the recooked bytes from the recook cache and calls
//! [`LiveSet::apply_notice`] at a frame boundary; competitive matches never
//! call it mid-match.
//!
//! Swap discipline, mirroring HR1's failed-cook-keeps-last-good rule:
//!
//! - Every payload cooks fully into an owned value before anything is
//!   inserted, so a failure never leaves a half-swapped state: the maps keep
//!   the last-good entry and the error is recorded in a bounded history.
//! - A broken WGSL edit keeps the last-good pipeline source: the active
//!   shader pointer only moves on a successful cook.
//! - Stale or duplicate notices (`version <= current`) are ignored without
//!   touching state or the error history, so rapid bursts coalesce and the
//!   last version wins.
//! - A material that references a texture asset under a different slot than
//!   the cooked texture recorded is refused ([`LiveError::SlotMismatch`]) and
//!   keeps its last-good version.
//!
//! Dependency note (SL1, documented edges): the HR1 [`ReloadNotice`] and
//! asset-id shapes stay render-local (16-byte UUID identity plus the lenient
//! 32-hex material schema), and so do the three texture budget bounds below —
//! `assets::texture` rides the `import` feature whose `image` dep enables
//! default formats incl. the NCSA-licensed avif chain the license gate
//! rejects, so importing it here would trade 3 local consts for a license
//! regression. Values match `assets::texture` exactly (16 MiB, 4096 px,
//! 2^24); if those ever move, move these with them. Replacing the id/slot
//! structs with `TextureSlot`/`TextureRef` would additionally change the
//! identity width (`u64` vs 16 bytes) and the material JSON schema, so that
//! stays a deliberate publication boundary, not a missed import.
//!
//! Validation scope is honest: WGSL gets a structural pre-check (bounded
//! size, UTF-8, both pipeline entry points, balanced braces/parens, no NUL)
//! so a typo'd edit is refused deterministically without a GPU. Full pipeline
//! compilation stays with the wgpu layer at the HR5 frame boundary, which
//! reports its own validation errors; a `Swapped` result means "structurally
//! valid and published", never "GPU-compiled".

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted WGSL source: shaders are tens of KiB; 1 MiB is headroom.
pub const MAX_SHADER_BYTES: usize = 1 << 20;
/// Largest accepted material document: factors plus at most three refs.
pub const MAX_MATERIAL_BYTES: usize = 64 * 1024;
/// Largest accepted texture file: matches `assets::texture::MAX_TEXTURE_BYTES`
/// by value (16 MiB). Kept local, not imported: `assets::texture` rides the
/// `import` feature whose `image` dep enables default formats incl. the
/// NCSA-licensed avif chain, which the license gate rejects in our graph.
/// A drift test below pins equality.
pub const MAX_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
/// Largest accepted texture side: matches `assets::texture::MAX_TEXTURE_DIM`
/// by value (see above for why local).
pub const MAX_TEXTURE_DIM: u32 = 4096;
/// Largest accepted texel count: matches `assets::texture::MAX_TEXTURE_PIXELS`
/// by value (see above for why local).
pub const MAX_TEXTURE_PIXELS: usize = 16_777_216;
/// Most texture refs one material may carry: one per slot.
pub const MAX_MATERIAL_TEXTURES: usize = 3;
/// Cap on the remembered error history; oldest entries fall off.
pub const MAX_ERRORS: usize = 64;
/// Cap on embedded error detail strings (serde/image diagnostics).
pub const MAX_DETAIL_CHARS: usize = 128;

/// Truncate a diagnostic to [`MAX_DETAIL_CHARS`] chars (never panics).
fn cap_detail(text: String) -> String {
    text.chars().take(MAX_DETAIL_CHARS).collect()
}

// ---------------------------------------------------------------------------
// Id + notice: read-only mirrors of the HR1 `AssetId` / `ReloadNotice`
// shapes. See the module docs for the conversion one-liner.
// ---------------------------------------------------------------------------

/// Stable asset identity: the 16 UUID bytes of HR1's `AssetId`.
///
/// `NIL` (all zeros) is never registered; notices for it are refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct LiveAssetId(pub [u8; 16]);

impl LiveAssetId {
    /// The nil id: never minted, never registered.
    pub const NIL: Self = Self([0u8; 16]);

    /// Wrap 16 UUID bytes (HR1: `asset.as_object_id().as_uuid().into_bytes()`).
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The wrapped bytes, for logging or round-trips back to HR1 ids.
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// True for the nil id.
    pub fn is_nil(self) -> bool {
        self.0 == [0u8; 16]
    }

    /// Parse 32 hex digits, with or without `-` separators (lenient: every
    /// `-` is skipped, positions unchecked). Anything else is [`LiveError::BadAssetId`].
    pub fn parse_hex(text: &str) -> Result<Self, LiveError> {
        let mut digits = [0u8; 32];
        let mut count = 0usize;
        for ch in text.chars() {
            if ch == '-' {
                continue;
            }
            let Some(digit) = ch.to_digit(16) else {
                return Err(LiveError::BadAssetId);
            };
            if count >= 32 {
                return Err(LiveError::BadAssetId);
            }
            digits[count] = digit as u8;
            count += 1;
        }
        if count != 32 {
            return Err(LiveError::BadAssetId);
        }
        let mut bytes = [0u8; 16];
        for (i, pair) in digits.chunks_exact(2).enumerate() {
            bytes[i] = (pair[0] << 4) | pair[1];
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for LiveAssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0u8; 32];
        for (i, byte) in self.0.iter().enumerate() {
            text[i * 2] = HEX[(byte >> 4) as usize];
            text[i * 2 + 1] = HEX[(byte & 0x0f) as usize];
        }
        // Hex digits are ASCII by construction; a failure is a programming bug.
        let rendered = std::str::from_utf8(&text).expect("hex table is ASCII");
        write!(f, "{rendered}")
    }
}

/// Immutable versioned reload notification: mirror of HR1's `ReloadNotice`.
///
/// `version` is the per-path recook counter; `0` means "never cooked" in HR1
/// and is always refused here. The payload kind comes from registration
/// ([`LiveSet::register`]): HR1 notices carry no kind, HR5 knows it from the
/// watched path (`.wgsl`, material JSON, image file).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ReloadNotice {
    /// The recooked asset; stable across recooks by HR1 construction.
    pub asset: LiveAssetId,
    /// The new per-path version to load; must be `> 0` and newer than current.
    pub version: u64,
}

// ---------------------------------------------------------------------------
// Slots + kinds: mirrors of `TextureSlot` / `TextureRef` shapes.
// ---------------------------------------------------------------------------

/// Which material slot a texture fills. Mirrors `assets::TextureSlot`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LiveTextureSlot {
    /// sRGB base-color map.
    BaseColor,
    /// Linear metallic-roughness map (G = roughness, B = metallic).
    MetallicRoughness,
    /// Linear normal map.
    Normal,
}

impl LiveTextureSlot {
    /// Canonical document name, matching the material JSON schema below.
    pub const fn name(self) -> &'static str {
        match self {
            Self::BaseColor => "base_color",
            Self::MetallicRoughness => "metallic_roughness",
            Self::Normal => "normal",
        }
    }

    /// Parse a document slot name; unknown names are [`LiveError::UnknownSlot`].
    pub fn parse(text: &str) -> Result<Self, LiveError> {
        match text {
            "base_color" => Ok(Self::BaseColor),
            "metallic_roughness" => Ok(Self::MetallicRoughness),
            "normal" => Ok(Self::Normal),
            _ => Err(LiveError::UnknownSlot(cap_detail(text.to_string()))),
        }
    }
}

impl fmt::Display for LiveTextureSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Reference to a cooked texture by stable asset id plus its slot.
/// Mirrors `assets::TextureRef`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LiveTextureRef {
    /// Stable asset id of the texture.
    pub asset: LiveAssetId,
    /// Which slot this texture fills.
    pub slot: LiveTextureSlot,
}

/// What kind of payload an asset id carries. Textures declare the slot they
/// fill at registration; the material cross-check enforces it on swap.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LiveKind {
    /// WGSL pipeline source.
    Shader,
    /// Material factor document (JSON, see [`LiveSet::apply_notice`]).
    Material,
    /// Decoded image bytes filling the carried slot.
    Texture(LiveTextureSlot),
}

// ---------------------------------------------------------------------------
// Errors: typed, bounded diagnostics naming the bad field.
// ---------------------------------------------------------------------------

/// Live-reload failures. Every variant keeps the last-good state untouched;
/// cook failures additionally record into the bounded error history.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LiveError {
    /// Asset id text is not 32 hex digits.
    BadAssetId,
    /// Notice version is `0`: HR1's "never cooked", nothing to load.
    VersionZero,
    /// Payload is empty.
    EmptyPayload,
    /// Payload exceeds its lane budget; byte count and limit carried.
    PayloadTooLarge {
        /// Offending byte count.
        bytes: usize,
        /// Budget that refused it.
        limit: usize,
    },
    /// WGSL is not valid UTF-8.
    ShaderNotUtf8,
    /// WGSL lacks a pipeline entry point (`@vertex` or `@fragment` carried).
    ShaderMissingEntry(&'static str),
    /// WGSL braces or parens do not balance.
    ShaderUnbalanced,
    /// WGSL contains a NUL byte.
    ShaderNul,
    /// Material document is not the expected JSON; capped decoder detail.
    MaterialJson(String),
    /// A material factor is missing, non-numeric or out of `0..=1`; field carried.
    MaterialBadValue {
        /// `base_color`, `metallic` or `roughness`.
        field: &'static str,
    },
    /// Two refs in one material fill the same slot.
    DuplicateSlot(LiveTextureSlot),
    /// A texture ref names an unknown slot; capped name carried.
    UnknownSlot(String),
    /// A material references a cooked texture under a different slot than the
    /// texture recorded: asset plus expected (recorded) and got (material) slots.
    SlotMismatch {
        /// Texture asset both sides name.
        asset: LiveAssetId,
        /// Slot the cooked texture recorded.
        expected: LiveTextureSlot,
        /// Slot the material claims.
        got: LiveTextureSlot,
    },
    /// Image bytes failed to decode; capped decoder detail.
    TextureDecode(String),
    /// Decoded dimensions are zero or exceed [`MAX_TEXTURE_DIM`].
    BadTextureDimensions {
        /// Decoded width in texels.
        width: u32,
        /// Decoded height in texels.
        height: u32,
    },
    /// Decoded texel count exceeds [`MAX_TEXTURE_PIXELS`].
    TexturePixelBudget {
        /// Offending texel count.
        pixels: usize,
    },
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadAssetId => write!(f, "live asset id is not 32 hex digits"),
            Self::VersionZero => write!(f, "live notice version is 0 (never cooked)"),
            Self::EmptyPayload => write!(f, "live payload is empty"),
            Self::PayloadTooLarge { bytes, limit } => {
                write!(f, "live payload {bytes} bytes exceeds {limit}")
            }
            Self::ShaderNotUtf8 => write!(f, "live shader is not valid UTF-8"),
            Self::ShaderMissingEntry(entry) => {
                write!(f, "live shader lacks the {entry} entry point")
            }
            Self::ShaderUnbalanced => write!(f, "live shader braces or parens do not balance"),
            Self::ShaderNul => write!(f, "live shader contains a NUL byte"),
            Self::MaterialJson(detail) => write!(f, "live material: {detail}"),
            Self::MaterialBadValue { field } => {
                write!(f, "live material {field} is missing or outside 0..=1")
            }
            Self::DuplicateSlot(slot) => {
                write!(f, "live material fills slot {slot} twice")
            }
            Self::UnknownSlot(name) => write!(f, "live texture slot {name} is unknown"),
            Self::SlotMismatch {
                asset,
                expected,
                got,
            } => write!(
                f,
                "live texture {asset} fills slot {expected}, material claims {got}"
            ),
            Self::TextureDecode(detail) => write!(f, "live texture: {detail}"),
            Self::BadTextureDimensions { width, height } => {
                write!(f, "live texture {width}x{height} is zero or oversized")
            }
            Self::TexturePixelBudget { pixels } => {
                write!(f, "live texture {pixels} texels exceed budget")
            }
        }
    }
}

impl std::error::Error for LiveError {}

// ---------------------------------------------------------------------------
// Cooked values: versioned live entries, all plain data.
// ---------------------------------------------------------------------------

/// Live pipeline source: one WGSL document plus the notice version that
/// installed it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VersionedShader {
    /// Notice version that installed this source.
    pub version: u64,
    /// Structurally validated WGSL.
    pub source: String,
}

/// Live material: validated factors plus texture refs. Mirrors the
/// `CookedMaterial` shape (sRGB base-color factor, metallic/roughness in
/// `0..=1`, at most one ref per slot).
#[derive(Clone, PartialEq, Debug)]
pub struct LiveMaterial {
    /// Notice version that installed these factors.
    pub version: u64,
    /// sRGB base-color factor, components in `0..=1`.
    pub base_color_factor: [f32; 4],
    /// Metallic factor in `0..=1`.
    pub metallic_factor: f32,
    /// Roughness factor in `0..=1`.
    pub roughness_factor: f32,
    /// Texture refs by slot, at most one per slot.
    pub textures: Vec<LiveTextureRef>,
}

/// Live texture: decoded descriptor plus the slot it fills. Pixel bytes stay
/// with the GPU upload path (HR5); the set records the validated descriptor
/// so material cross-checks have something to check against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LiveTexture {
    /// Notice version that installed this descriptor.
    pub version: u64,
    /// Decoded width in texels.
    pub width: u32,
    /// Decoded height in texels.
    pub height: u32,
    /// Slot this texture fills (fixed at registration).
    pub slot: LiveTextureSlot,
}

/// One recorded failure: which notice failed and why. Bounded history; see
/// [`LiveSet::last_error`] and [`LiveSet::dropped_errors`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ErrorRecord {
    /// Asset the failed notice named.
    pub asset: LiveAssetId,
    /// Version the failed notice carried.
    pub version: u64,
    /// Why the swap did not happen.
    pub error: LiveError,
}

/// Outcome of one [`LiveSet::apply_notice`] call.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SwapResult {
    /// Payload cooked and published; version now current for the asset.
    Swapped {
        /// Kind that swapped.
        kind: LiveKind,
        /// Installed version.
        version: u64,
    },
    /// Notice is stale (`version <= current`): ignored, nothing touched and
    /// nothing recorded, so rapid bursts coalesce with last version winning.
    Stale {
        /// Current version for the asset.
        current: u64,
    },
    /// Payload failed to cook (broken shader, bad JSON, undecodable image):
    /// last-good kept, error recorded.
    KeptLastGood {
        /// Why the cook failed.
        error: LiveError,
    },
    /// Payload cooked but policy refused it (version zero, slot mismatch):
    /// last-good kept, error recorded.
    Refused {
        /// Why the swap was refused.
        error: LiveError,
    },
    /// Asset was never registered: ignored, nothing touched, nothing recorded.
    UnknownAsset,
}

// ---------------------------------------------------------------------------
// Cooks: byte-level validators, each total over its input.
// ---------------------------------------------------------------------------

/// Structural WGSL pre-check (see module docs): bounded, UTF-8, both entry
/// points, balanced braces/parens, no NUL. Returns the owned source on
/// success so the caller can publish without re-reading the bytes.
fn cook_shader(bytes: &[u8]) -> Result<String, LiveError> {
    if bytes.is_empty() {
        return Err(LiveError::EmptyPayload);
    }
    if bytes.len() > MAX_SHADER_BYTES {
        return Err(LiveError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_SHADER_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| LiveError::ShaderNotUtf8)?;
    if text.contains('\0') {
        return Err(LiveError::ShaderNul);
    }
    if !text.contains("@vertex") {
        return Err(LiveError::ShaderMissingEntry("@vertex"));
    }
    if !text.contains("@fragment") {
        return Err(LiveError::ShaderMissingEntry("@fragment"));
    }
    let mut braces: i64 = 0;
    let mut parens: i64 = 0;
    for ch in text.chars() {
        match ch {
            '{' => braces += 1,
            '}' => {
                braces -= 1;
                if braces < 0 {
                    return Err(LiveError::ShaderUnbalanced);
                }
            }
            '(' => parens += 1,
            ')' => {
                parens -= 1;
                if parens < 0 {
                    return Err(LiveError::ShaderUnbalanced);
                }
            }
            _ => {}
        }
    }
    if braces != 0 || parens != 0 {
        return Err(LiveError::ShaderUnbalanced);
    }
    Ok(text.to_string())
}

/// Material factor document (JSON):
/// `{"base_color":[r,g,b,a],"metallic":m,"roughness":r,
/// "textures":[{"asset":"<32 hex>","slot":"base_color"}]}`.
/// `textures` is optional and defaults to empty. Factors must be finite and
/// in `0..=1`; slots must name known slots with no duplicates. Returns the
/// unversioned material; the caller stamps the notice version on publish.
fn cook_material(bytes: &[u8]) -> Result<LiveMaterial, LiveError> {
    if bytes.is_empty() {
        return Err(LiveError::EmptyPayload);
    }
    if bytes.len() > MAX_MATERIAL_BYTES {
        return Err(LiveError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_MATERIAL_BYTES,
        });
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| LiveError::MaterialJson(cap_detail(e.to_string())))?;
    let doc = value
        .as_object()
        .ok_or_else(|| LiveError::MaterialJson("expected an object".to_string()))?;

    let number_in_range = |key: &'static str| -> Result<f32, LiveError> {
        let raw = doc
            .get(key)
            .and_then(serde_json::Value::as_f64)
            .ok_or(LiveError::MaterialBadValue { field: key })?;
        if !(0.0..=1.0).contains(&raw) {
            return Err(LiveError::MaterialBadValue { field: key });
        }
        Ok(raw as f32)
    };
    let base_array = doc
        .get("base_color")
        .and_then(serde_json::Value::as_array)
        .ok_or(LiveError::MaterialBadValue {
            field: "base_color",
        })?;
    if base_array.len() != 4 {
        return Err(LiveError::MaterialBadValue {
            field: "base_color",
        });
    }
    let mut base_color_factor = [0.0f32; 4];
    for (i, entry) in base_array.iter().enumerate() {
        let raw = entry.as_f64().ok_or(LiveError::MaterialBadValue {
            field: "base_color",
        })?;
        if !(0.0..=1.0).contains(&raw) {
            return Err(LiveError::MaterialBadValue {
                field: "base_color",
            });
        }
        base_color_factor[i] = raw as f32;
    }
    let metallic_factor = number_in_range("metallic")?;
    let roughness_factor = number_in_range("roughness")?;

    let mut textures = Vec::new();
    if let Some(refs) = doc.get("textures") {
        let items = refs
            .as_array()
            .ok_or_else(|| LiveError::MaterialJson("textures must be an array".to_string()))?;
        if items.len() > MAX_MATERIAL_TEXTURES {
            return Err(LiveError::MaterialJson("too many texture refs".to_string()));
        }
        for item in items {
            let entry = item.as_object().ok_or_else(|| {
                LiveError::MaterialJson("texture ref must be an object".to_string())
            })?;
            let asset_text = entry
                .get("asset")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    LiveError::MaterialJson("texture ref needs an asset id".to_string())
                })?;
            let asset = LiveAssetId::parse_hex(asset_text)?;
            let slot_text = entry
                .get("slot")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| LiveError::MaterialJson("texture ref needs a slot".to_string()))?;
            let slot = LiveTextureSlot::parse(slot_text)?;
            if textures
                .iter()
                .any(|seen: &LiveTextureRef| seen.slot == slot)
            {
                return Err(LiveError::DuplicateSlot(slot));
            }
            textures.push(LiveTextureRef { asset, slot });
        }
    }
    Ok(LiveMaterial {
        version: 0,
        base_color_factor,
        metallic_factor,
        roughness_factor,
        textures,
    })
}

/// Texture cook: bounded bytes, decodable image, bounded dimensions and texel
/// budget. Returns the descriptor; pixel upload stays with HR5's GPU path.
fn cook_texture(bytes: &[u8]) -> Result<(u32, u32), LiveError> {
    if bytes.is_empty() {
        return Err(LiveError::EmptyPayload);
    }
    if bytes.len() > MAX_TEXTURE_BYTES {
        return Err(LiveError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_TEXTURE_BYTES,
        });
    }
    let image = image::load_from_memory(bytes)
        .map_err(|e| LiveError::TextureDecode(cap_detail(e.to_string())))?;
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 || width > MAX_TEXTURE_DIM || height > MAX_TEXTURE_DIM {
        return Err(LiveError::BadTextureDimensions { width, height });
    }
    let pixels = width as usize * height as usize;
    if pixels > MAX_TEXTURE_PIXELS {
        return Err(LiveError::TexturePixelBudget { pixels });
    }
    Ok((width, height))
}

// ---------------------------------------------------------------------------
// LiveSet: the render-side publication boundary.
// ---------------------------------------------------------------------------

/// Versioned live shader/material/texture slots fed by HR1 notices.
///
/// Register every reloadable asset up front (HR5 wires watched paths to ids
/// and kinds at startup); then each HR1 notice arrives with its recooked
/// bytes via [`LiveSet::apply_notice`]. The set starts empty: no pipeline is
/// live until the first good shader swap, so HR5 applies the shipped shader
/// before presenting. All iteration is `BTreeMap` order; no wall clock, no
/// threads, no GPU calls — publication timing stays with the caller.
#[derive(Clone, Debug, Default)]
pub struct LiveSet {
    registered: BTreeMap<LiveAssetId, LiveKind>,
    versions: BTreeMap<LiveAssetId, u64>,
    shaders: BTreeMap<LiveAssetId, VersionedShader>,
    active_shader: Option<LiveAssetId>,
    materials: BTreeMap<LiveAssetId, LiveMaterial>,
    textures: BTreeMap<LiveAssetId, LiveTexture>,
    errors: VecDeque<ErrorRecord>,
    dropped_errors: u64,
}

impl LiveSet {
    /// Empty set: nothing registered, no pipeline live, no errors.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind an asset id to its payload kind (setup-time; HR5 wires watched
    /// paths here). Rebinding an id never mutates cooked entries: the next
    /// notice cooks under the new kind.
    pub fn register(&mut self, asset: LiveAssetId, kind: LiveKind) {
        self.registered.insert(asset, kind);
    }

    /// Forget an asset binding. Cooked entries and versions stay readable;
    /// later notices for the id report [`SwapResult::UnknownAsset`].
    pub fn unregister(&mut self, asset: LiveAssetId) -> bool {
        self.registered.remove(&asset).is_some()
    }

    /// Current version for an id, if it ever swapped successfully.
    pub fn version_for(&self, asset: LiveAssetId) -> Option<u64> {
        self.versions.get(&asset).copied()
    }

    /// Live pipeline source: the last-good shader, if any good swap landed.
    pub fn shader_source(&self) -> Option<&str> {
        self.active_shader
            .and_then(|id| self.shaders.get(&id))
            .map(|entry| entry.source.as_str())
    }

    /// Cooked material entry for an id, if it ever swapped successfully.
    pub fn material_for(&self, asset: LiveAssetId) -> Option<&LiveMaterial> {
        self.materials.get(&asset)
    }

    /// Cooked texture descriptor for an id, if it ever swapped successfully.
    pub fn texture_for(&self, asset: LiveAssetId) -> Option<&LiveTexture> {
        self.textures.get(&asset)
    }

    /// Most recent recorded failure, if any.
    pub fn last_error(&self) -> Option<&ErrorRecord> {
        self.errors.back()
    }

    /// Recorded failures, oldest first, bounded by [`MAX_ERRORS`].
    pub fn errors(&self) -> &VecDeque<ErrorRecord> {
        &self.errors
    }

    /// Error records shed by the history cap so far.
    pub fn dropped_errors(&self) -> u64 {
        self.dropped_errors
    }

    /// Apply one HR1 notice with its recooked bytes (HR5 fetches the bytes
    /// from the recook cache). Atomic per call: the payload cooks into an
    /// owned value first, and only a fully validated value is published —
    /// failures keep last-good and record the error, never half-swap.
    pub fn apply_notice(&mut self, notice: ReloadNotice, bytes: &[u8]) -> SwapResult {
        if notice.version == 0 {
            let error = LiveError::VersionZero;
            self.record(notice, error.clone());
            return SwapResult::Refused { error };
        }
        if notice.asset.is_nil() {
            self.record(notice, LiveError::BadAssetId);
            return SwapResult::Refused {
                error: LiveError::BadAssetId,
            };
        }
        let Some(kind) = self.registered.get(&notice.asset).copied() else {
            return SwapResult::UnknownAsset;
        };
        let current = self.versions.get(&notice.asset).copied().unwrap_or(0);
        if notice.version <= current {
            return SwapResult::Stale { current };
        }
        match kind {
            LiveKind::Shader => match cook_shader(bytes) {
                Ok(source) => {
                    self.shaders.insert(
                        notice.asset,
                        VersionedShader {
                            version: notice.version,
                            source,
                        },
                    );
                    self.versions.insert(notice.asset, notice.version);
                    self.active_shader = Some(notice.asset);
                    SwapResult::Swapped {
                        kind,
                        version: notice.version,
                    }
                }
                Err(error) => {
                    self.record(notice, error.clone());
                    SwapResult::KeptLastGood { error }
                }
            },
            LiveKind::Material => match cook_material(bytes) {
                Ok(mut material) => {
                    if let Some(error) = self.check_texture_slots(&material) {
                        self.record(notice, error.clone());
                        return SwapResult::Refused { error };
                    }
                    material.version = notice.version;
                    self.materials.insert(notice.asset, material);
                    self.versions.insert(notice.asset, notice.version);
                    SwapResult::Swapped {
                        kind,
                        version: notice.version,
                    }
                }
                Err(error) => {
                    self.record(notice, error.clone());
                    SwapResult::KeptLastGood { error }
                }
            },
            LiveKind::Texture(slot) => match cook_texture(bytes) {
                Ok((width, height)) => {
                    self.textures.insert(
                        notice.asset,
                        LiveTexture {
                            version: notice.version,
                            width,
                            height,
                            slot,
                        },
                    );
                    self.versions.insert(notice.asset, notice.version);
                    SwapResult::Swapped {
                        kind,
                        version: notice.version,
                    }
                }
                Err(error) => {
                    self.record(notice, error.clone());
                    SwapResult::KeptLastGood { error }
                }
            },
        }
    }

    /// Refuse a material whose refs disagree with already-cooked textures:
    /// every ref naming a cooked texture must use that texture's slot.
    /// Refs to uncooked (or unknown) textures are forward references and pass.
    fn check_texture_slots(&self, material: &LiveMaterial) -> Option<LiveError> {
        for reference in &material.textures {
            if let Some(cooked) = self.textures.get(&reference.asset) {
                if cooked.slot != reference.slot {
                    return Some(LiveError::SlotMismatch {
                        asset: reference.asset,
                        expected: cooked.slot,
                        got: reference.slot,
                    });
                }
            }
        }
        None
    }

    /// Record one failure, shedding the oldest when over the history cap.
    fn record(&mut self, notice: ReloadNotice, error: LiveError) {
        if self.errors.len() >= MAX_ERRORS {
            self.errors.pop_front();
            self.dropped_errors = self.dropped_errors.saturating_add(1);
        }
        self.errors.push_back(ErrorRecord {
            asset: notice.asset,
            version: notice.version,
            error,
        });
    }

    /// Apply one staged reload at its exact frame boundary (RA7 frame-exact
    /// material/shader hot-reload). Off-boundary frames refuse as
    /// [`FrameSwapError::WrongFrame`] without touching state or the error
    /// history (the stale-notice discipline); the exact boundary delegates
    /// to [`LiveSet::apply_notice`], so failures keep last-good with a
    /// diagnostic, never a half-swapped pipeline.
    pub fn apply_staged(
        &mut self,
        staged: &StagedReload,
        bytes: &[u8],
        frame: u64,
    ) -> Result<SwapResult, FrameSwapError> {
        if frame != staged.frame {
            return Err(FrameSwapError::WrongFrame {
                expected: staged.frame,
                got: frame,
            });
        }
        Ok(self.apply_notice(staged.notice, bytes))
    }
}

// ---------------------------------------------------------------------------
// Frame-exact staging: a reload bound to one frame index. Units are frame
// indices (`u64`, never wall time); ordering stays with the caller (HR5/ED1
// frame loop), which holds the staged value and calls `apply_staged` once
// the counter reaches the boundary.
// ---------------------------------------------------------------------------

/// A reload staged for exactly one frame index: the HR1 notice plus its
/// recooked bytes' boundary. Staging stores no bytes and spawns no work;
/// the swap still happens inside [`LiveSet::apply_staged`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StagedReload {
    /// The versioned notice to apply at the boundary.
    pub notice: ReloadNotice,
    /// Frame index the swap lands on.
    pub frame: u64,
}

impl StagedReload {
    /// Stage a notice for one frame index.
    pub const fn new(notice: ReloadNotice, frame: u64) -> Self {
        Self { notice, frame }
    }

    /// True only at the exact staged frame: no early apply, no late catch-up.
    pub fn is_due(self, frame: u64) -> bool {
        self.frame == frame
    }
}

/// Frame-boundary refusal. Off-frame applies never touch render state, so
/// the only recovery is retrying at the expected frame.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FrameSwapError {
    /// The staged swap belongs to another frame; expected and offered
    /// frames carried.
    WrongFrame {
        /// Frame the swap was staged for.
        expected: u64,
        /// Frame the caller offered.
        got: u64,
    },
}

impl fmt::Display for FrameSwapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongFrame { expected, got } => write!(
                f,
                "staged reload is for frame {expected}, offered frame {got}"
            ),
        }
    }
}

impl std::error::Error for FrameSwapError {}

// ---------------------------------------------------------------------------
// Tests: good swaps versions; broken keeps last-good + records; rapid
// notices coalesce with last winning; slot mismatch refused.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    const SHADER_ID: LiveAssetId = LiveAssetId([0x11; 16]);
    const MATERIAL_ID: LiveAssetId = LiveAssetId([0x22; 16]);
    const TEXTURE_ID: LiveAssetId = LiveAssetId([0x33; 16]);
    const GHOST_ID: LiveAssetId = LiveAssetId([0x99; 16]);

    /// Minimal structurally valid WGSL carrying a marker for swap checks.
    fn valid_wgsl(marker: &str) -> Vec<u8> {
        format!(
            "@vertex\nfn vs() -> vec4<f32> {{ return vec4<f32>(0.0, 0.0, 0.0, 1.0); }}\n\
             // {marker}\n\
             @fragment\nfn fs() -> vec4<f32> {{ return vec4<f32>(1.0, 0.0, 0.0, 1.0); }}\n"
        )
        .into_bytes()
    }

    fn material_bytes(asset_hex: &str, slot: &str) -> Vec<u8> {
        format!(
            "{{\"base_color\":[0.72,0.45,0.28,1.0],\"metallic\":0.9,\
             \"roughness\":0.35,\"textures\":[{{\"asset\":\"{asset_hex}\",\"slot\":\"{slot}\"}}]}}"
        )
        .into_bytes()
    }

    fn bare_material_bytes() -> Vec<u8> {
        b"{\"base_color\":[1.0,1.0,1.0,1.0],\"metallic\":0.0,\"roughness\":1.0}".to_vec()
    }

    /// 4x4 red PNG bytes for texture cooks.
    fn png_bytes() -> Vec<u8> {
        let mut pixels = vec![0u8; 4 * 4 * 4];
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[200, 50, 50, 255]);
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out)
            .write_image(&pixels, 4, 4, image::ExtendedColorType::Rgba8)
            .expect("test png encodes");
        out
    }

    fn notice(asset: LiveAssetId, version: u64) -> ReloadNotice {
        ReloadNotice { asset, version }
    }

    fn wired_set() -> LiveSet {
        let mut set = LiveSet::new();
        set.register(SHADER_ID, LiveKind::Shader);
        set.register(MATERIAL_ID, LiveKind::Material);
        set.register(TEXTURE_ID, LiveKind::Texture(LiveTextureSlot::BaseColor));
        set
    }

    #[test]
    fn good_reload_swaps_versions() {
        let mut set = wired_set();

        let first = set.apply_notice(notice(SHADER_ID, 1), &valid_wgsl("coat-v1"));
        assert_eq!(
            first,
            SwapResult::Swapped {
                kind: LiveKind::Shader,
                version: 1
            }
        );
        assert_eq!(set.version_for(SHADER_ID), Some(1));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("coat-v1"));

        // Material and texture swap through the same path.
        let texture_hex = TEXTURE_ID.to_string();
        assert_eq!(
            set.apply_notice(notice(TEXTURE_ID, 1), &png_bytes()),
            SwapResult::Swapped {
                kind: LiveKind::Texture(LiveTextureSlot::BaseColor),
                version: 1
            }
        );
        let cooked = set.texture_for(TEXTURE_ID).expect("texture cooked");
        assert_eq!(
            (cooked.width, cooked.height, cooked.slot),
            (4, 4, LiveTextureSlot::BaseColor)
        );
        assert_eq!(
            set.apply_notice(
                notice(MATERIAL_ID, 1),
                &material_bytes(&texture_hex, "base_color")
            ),
            SwapResult::Swapped {
                kind: LiveKind::Material,
                version: 1
            }
        );
        let material = set.material_for(MATERIAL_ID).expect("material cooked");
        assert_eq!(material.version, 1);
        assert_eq!(material.textures.len(), 1);

        // A newer shader version replaces the pipeline source.
        assert_eq!(
            set.apply_notice(notice(SHADER_ID, 2), &valid_wgsl("coat-v2")),
            SwapResult::Swapped {
                kind: LiveKind::Shader,
                version: 2
            }
        );
        assert_eq!(set.version_for(SHADER_ID), Some(2));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("coat-v2"));
        assert!(set.last_error().is_none());
    }

    #[test]
    fn broken_shader_keeps_last_good_and_records_error() {
        let mut set = wired_set();
        assert!(matches!(
            set.apply_notice(notice(SHADER_ID, 1), &valid_wgsl("good-coat")),
            SwapResult::Swapped { .. }
        ));

        // Missing entry points: refused, last-good pipeline untouched.
        let broken = set.apply_notice(notice(SHADER_ID, 2), b"this edit broke everything (((");
        assert_eq!(
            broken,
            SwapResult::KeptLastGood {
                error: LiveError::ShaderMissingEntry("@vertex"),
            }
        );
        assert_eq!(set.version_for(SHADER_ID), Some(1));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("good-coat"));

        // Unbalanced with entry points present: same keep-last-good path.
        let unbalanced = b"@vertex\n@fragment\nfn f( { } ) )";
        let outcome = set.apply_notice(notice(SHADER_ID, 3), unbalanced);
        assert_eq!(
            outcome,
            SwapResult::KeptLastGood {
                error: LiveError::ShaderUnbalanced,
            }
        );
        assert_eq!(set.version_for(SHADER_ID), Some(1));

        // Both failures recorded, oldest first.
        assert_eq!(set.errors().len(), 2);
        let last = set.last_error().expect("error recorded");
        assert_eq!(last.asset, SHADER_ID);
        assert_eq!(last.version, 3);
        assert_eq!(last.error, LiveError::ShaderUnbalanced);

        // A later good cook recovers the pipeline and moves the version.
        assert!(matches!(
            set.apply_notice(notice(SHADER_ID, 4), &valid_wgsl("fixed-coat")),
            SwapResult::Swapped { version: 4, .. }
        ));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("fixed-coat"));
    }

    #[test]
    fn rapid_notices_coalesce_last_version_wins() {
        let mut set = wired_set();

        // Burst of versions in order: each newer one wins.
        for version in [1, 2, 3, 4] {
            let marker = format!("burst-v{version}");
            let outcome = set.apply_notice(notice(SHADER_ID, version), &valid_wgsl(&marker));
            assert_eq!(
                outcome,
                SwapResult::Swapped {
                    kind: LiveKind::Shader,
                    version
                }
            );
        }
        assert_eq!(set.version_for(SHADER_ID), Some(4));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("burst-v4"));

        // Stale replays of superseded versions change nothing and record nothing.
        let errors_before = set.errors().len();
        for stale in [2, 3, 4] {
            assert_eq!(
                set.apply_notice(notice(SHADER_ID, stale), &valid_wgsl("stale")),
                SwapResult::Stale { current: 4 }
            );
        }
        assert_eq!(set.version_for(SHADER_ID), Some(4));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("burst-v4"));
        assert_eq!(set.errors().len(), errors_before);

        // A broken payload at a newer version also keeps the burst winner.
        assert!(matches!(
            set.apply_notice(notice(SHADER_ID, 5), b"nope"),
            SwapResult::KeptLastGood { .. }
        ));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("burst-v4"));
    }

    #[test]
    fn texture_slot_mismatch_refused() {
        let mut set = wired_set();
        let texture_hex = TEXTURE_ID.to_string();

        // Texture cooks as its registered BaseColor slot.
        assert!(matches!(
            set.apply_notice(notice(TEXTURE_ID, 1), &png_bytes()),
            SwapResult::Swapped { .. }
        ));

        // A material claiming that texture under Normal is refused: no entry,
        // no version, error recorded.
        let mismatch = set.apply_notice(
            notice(MATERIAL_ID, 1),
            &material_bytes(&texture_hex, "normal"),
        );
        assert_eq!(
            mismatch,
            SwapResult::Refused {
                error: LiveError::SlotMismatch {
                    asset: TEXTURE_ID,
                    expected: LiveTextureSlot::BaseColor,
                    got: LiveTextureSlot::Normal,
                },
            }
        );
        assert!(set.material_for(MATERIAL_ID).is_none());
        assert_eq!(set.version_for(MATERIAL_ID), None);
        let record = set.last_error().expect("mismatch recorded");
        assert_eq!(record.asset, MATERIAL_ID);

        // The corrected material (matching slot) swaps normally.
        assert_eq!(
            set.apply_notice(
                notice(MATERIAL_ID, 2),
                &material_bytes(&texture_hex, "base_color")
            ),
            SwapResult::Swapped {
                kind: LiveKind::Material,
                version: 2
            }
        );
        assert_eq!(set.version_for(MATERIAL_ID), Some(2));

        // Two refs filling the same slot are refused the same way.
        let dupe = format!(
            "{{\"base_color\":[1.0,1.0,1.0,1.0],\"metallic\":0.0,\"roughness\":1.0,\
             \"textures\":[{{\"asset\":\"{texture_hex}\",\"slot\":\"base_color\"}},\
             {{\"asset\":\"{texture_hex}\",\"slot\":\"base_color\"}}]}}"
        );
        assert_eq!(
            set.apply_notice(notice(MATERIAL_ID, 3), dupe.as_bytes()),
            SwapResult::KeptLastGood {
                error: LiveError::DuplicateSlot(LiveTextureSlot::BaseColor),
            }
        );
        assert_eq!(set.version_for(MATERIAL_ID), Some(2));
    }

    #[test]
    fn unknown_asset_stale_and_version_zero() {
        let mut set = wired_set();

        // Never registered: ignored without state or history.
        assert_eq!(
            set.apply_notice(notice(GHOST_ID, 1), &valid_wgsl("ghost")),
            SwapResult::UnknownAsset
        );
        assert_eq!(set.version_for(GHOST_ID), None);
        assert!(set.last_error().is_none());

        // Version zero (HR1's never-cooked) is refused and recorded.
        assert_eq!(
            set.apply_notice(notice(SHADER_ID, 0), &valid_wgsl("zero")),
            SwapResult::Refused {
                error: LiveError::VersionZero,
            }
        );
        assert_eq!(set.version_for(SHADER_ID), None);
        assert!(set.shader_source().is_none());

        // Nil-asset notices are refused the same way.
        assert_eq!(
            set.apply_notice(notice(LiveAssetId::NIL, 1), &valid_wgsl("nil")),
            SwapResult::Refused {
                error: LiveError::BadAssetId,
            }
        );
    }

    #[test]
    fn broken_material_and_texture_keep_last_good() {
        let mut set = wired_set();

        assert_eq!(
            set.apply_notice(notice(MATERIAL_ID, 1), &bare_material_bytes()),
            SwapResult::Swapped {
                kind: LiveKind::Material,
                version: 1
            }
        );
        // Not JSON at all: kept, recorded.
        assert_eq!(
            set.apply_notice(notice(MATERIAL_ID, 2), b"copper-ish, shiny"),
            SwapResult::KeptLastGood {
                error: LiveError::MaterialJson("expected value at line 1 column 1".to_string()),
            }
        );
        // Out-of-range factor: kept, recorded.
        let bad_factor = b"{\"base_color\":[1.0,1.0,1.0,1.0],\"metallic\":0.0,\"roughness\":2.0}";
        assert_eq!(
            set.apply_notice(notice(MATERIAL_ID, 3), bad_factor),
            SwapResult::KeptLastGood {
                error: LiveError::MaterialBadValue { field: "roughness" },
            }
        );
        assert_eq!(set.version_for(MATERIAL_ID), Some(1));

        // Garbage image bytes: kept, recorded.
        assert!(matches!(
            set.apply_notice(notice(TEXTURE_ID, 1), b"definitely not an image"),
            SwapResult::KeptLastGood {
                error: LiveError::TextureDecode(_),
            }
        ));
        assert!(set.texture_for(TEXTURE_ID).is_none());
        assert_eq!(set.errors().len(), 3);
    }

    #[test]
    fn asset_id_hex_round_trip() {
        let id = LiveAssetId::from_bytes([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, //
            0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
        ]);
        assert_eq!(id.to_string(), "0123456789abcdeffedcba9876543210");
        assert_eq!(LiveAssetId::parse_hex(&id.to_string()), Ok(id));
        assert!(LiveAssetId::NIL.is_nil());
        assert!(!id.is_nil());
        assert_eq!(LiveAssetId::parse_hex("short"), Err(LiveError::BadAssetId));
        assert_eq!(
            LiveAssetId::parse_hex(&"zz".repeat(16)),
            Err(LiveError::BadAssetId)
        );
    }
}

// ---------------------------------------------------------------------------
// Frame-boundary tests (RA7): staged reloads land only on their exact
// frame, off-frame applies name the frame and touch nothing, and a broken
// payload at the boundary keeps last-good with a diagnostic.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod frame_boundary_tests {
    use super::*;

    const SHADER_ID: LiveAssetId = LiveAssetId([0x11; 16]);

    /// Minimal structurally valid WGSL carrying a marker for swap checks.
    fn valid_wgsl(marker: &str) -> Vec<u8> {
        format!(
            "@vertex\nfn vs() -> vec4<f32> {{ return vec4<f32>(0.0, 0.0, 0.0, 1.0); }}\n\
             // {marker}\n\
             @fragment\nfn fs() -> vec4<f32> {{ return vec4<f32>(1.0, 0.0, 0.0, 1.0); }}\n"
        )
        .into_bytes()
    }

    fn staged(version: u64, frame: u64) -> StagedReload {
        StagedReload::new(
            ReloadNotice {
                asset: SHADER_ID,
                version,
            },
            frame,
        )
    }

    fn wired_set() -> LiveSet {
        let mut set = LiveSet::new();
        set.register(SHADER_ID, LiveKind::Shader);
        set
    }

    #[test]
    fn staged_swap_lands_on_its_exact_frame() {
        let mut set = wired_set();
        let pending = staged(1, 41);
        assert!(!pending.is_due(40));
        assert!(pending.is_due(41));
        let outcome = set
            .apply_staged(&pending, &valid_wgsl("frame-41"), 41)
            .expect("lands on frame 41");
        assert_eq!(
            outcome,
            SwapResult::Swapped {
                kind: LiveKind::Shader,
                version: 1
            }
        );
        assert_eq!(set.version_for(SHADER_ID), Some(1));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("frame-41"));
    }

    #[test]
    fn off_frame_apply_names_the_frame_and_touches_nothing() {
        let mut set = wired_set();
        let pending = staged(1, 41);
        for frame in [40, 42] {
            assert_eq!(
                set.apply_staged(&pending, &valid_wgsl("early"), frame),
                Err(FrameSwapError::WrongFrame {
                    expected: 41,
                    got: frame,
                })
            );
        }
        // Nothing published, nothing recorded.
        assert_eq!(set.version_for(SHADER_ID), None);
        assert!(set.shader_source().is_none());
        assert!(set.last_error().is_none());
    }

    #[test]
    fn broken_payload_at_boundary_keeps_last_good() {
        let mut set = wired_set();
        assert!(matches!(
            set.apply_notice(
                ReloadNotice {
                    asset: SHADER_ID,
                    version: 1
                },
                &valid_wgsl("good-coat")
            ),
            SwapResult::Swapped { .. }
        ));
        // Broken cook at the exact boundary: last-good kept, error recorded.
        let outcome = set
            .apply_staged(&staged(2, 9), b"this edit broke everything (((", 9)
            .expect("boundary reached, cook refused");
        assert_eq!(
            outcome,
            SwapResult::KeptLastGood {
                error: LiveError::ShaderMissingEntry("@vertex"),
            }
        );
        assert_eq!(set.version_for(SHADER_ID), Some(1));
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("good-coat"));
        let record = set.last_error().expect("failure recorded");
        assert_eq!((record.asset, record.version), (SHADER_ID, 2));
    }

    #[test]
    fn staged_stale_version_still_coalesces() {
        let mut set = wired_set();
        assert!(matches!(
            set.apply_staged(&staged(3, 10), &valid_wgsl("winner"), 10),
            Ok(SwapResult::Swapped { .. })
        ));
        // A superseded version staged for a later frame still reports stale
        // with the boundary reached: versioning stays authoritative.
        assert_eq!(
            set.apply_staged(&staged(2, 11), &valid_wgsl("stale"), 11)
                .expect("boundary reached"),
            SwapResult::Stale { current: 3 }
        );
        assert!(set
            .shader_source()
            .expect("pipeline live")
            .contains("winner"));
    }
}
