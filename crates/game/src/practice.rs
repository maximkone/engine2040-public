//! practice: running practice mode wiring the asset watcher, combat swaps and
//! AI swaps plus scene reload.
//!
//! Consumer: the editor tuning panel and the smooth-building evidence.
//! Data owner: this module owns the practice-mode publication boundary — the
//! per-frame watch poll, the versioned presentation slots (shader, material,
//! texture, scene) and the recorded rules-command logs (combat tables, AI
//! curves). Match authority stays in [`App`](super::App); fight
//! rules stay in `engine-combat`; AI curves stay in `engine-ai`; the shared
//! watcher/recook services stay in `engine-assets`; the GPU/JSON/image cooks
//! stay in `engine-render`/`engine-assets`. Units: fixed ticks at the arena
//! rate; payload sizes in bytes; versions are per-asset/path counters.
//! Determinism scope: same config plus same inputs plus same recorded rules
//! commands gives identical checkpoints; presentation swaps, file mtimes and
//! wall clock never enter hashed state. Errors: [`PresentError`] at the
//! presentation boundary, [`AiRulesError`] for AI snapshots,
//! [`ReloadRefusal`] for refused swaps; invalid input is refused loudly with
//! last-good kept, never applied half-way and never panics.
//!
//! Swap discipline (asset, combat and AI swaps through documented edges where the shapes agree;
//! render-local pre-checks and the flat rules log stay here, see below):
//!
//! - Presentation (shader/material/texture/scene) swaps at frame boundaries:
//!   [`PracticeMode::tick`] polls the watched files first, cooks each changed
//!   payload into an owned value, and only then steps authority. A failure
//!   keeps last-good and records its reason.
//! - Rules (combat tables/AI curves) apply only as recorded authority tick
//!   commands: [`PracticeMode::propose_combat_swap`] and
//!   [`PracticeMode::propose_ai_swap`] append `(at_tick, snapshot)` entries;
//!   the effective version for tick `t` is the latest entry with
//!   `at_tick <= t`, so a swap proposed at the cursor takes effect at the
//!   next tick boundary, never mid-tick. Old tapes (inputs without commands)
//!   replay byte-identical through the pinned V1 defaults.
//! - Competitive ([`CompetitiveMatch`]) pins its rules/content versions: every
//!   live-swap proposal fails with a typed [`ReloadRefusal::CompetitivePinned`].
//!
//! Honest scope: the combat snapshots are real combat [`SwapInput`] values (the
//! recorded rows provably change outcomes under
//! [`step_with_tables`](engine_combat::step_with_tables)) and the AI
//! snapshots are exact mirrors of `engine-ai`'s `AiConfig` (field-for-field, same
//! validation). Authority still steps under the arena's pinned V1 tables and
//! scripted brains — the arena exposes no swap-consumption API yet — so this
//! lane records the commands, pins the versions and proves replay discipline;
//! the orchestrator follow-up is an arena-side `SwapInput`/`AiConfig`
//! consumer (the conversion is one line each way; see [`AiRules`]). A broken
//! file keeps the last-good version with its reason, exactly like the asset watcher.

use std::collections::{BTreeMap, VecDeque};
use std::time::UNIX_EPOCH;

use engine_ai::curves::{MORALE_MAX, MORALE_MIN};
use engine_combat::SwapInput;

use super::{App, GameConfig, TickOutcome};

/// Largest accepted WGSL source: mirrors `render::live::MAX_SHADER_BYTES`.
pub const MAX_SHADER_BYTES: usize = 1 << 20;
/// Largest accepted material document: mirrors `render::live::MAX_MATERIAL_BYTES`.
pub const MAX_MATERIAL_BYTES: usize = 64 * 1024;
/// Largest accepted texture file: mirrors `render::live::MAX_TEXTURE_BYTES`
/// (the render lane is PNG-only, so the pre-check below looks for PNG magic).
pub const MAX_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
/// Largest accepted scene document (practice fixtures are kilobytes).
pub const MAX_SCENE_BYTES: usize = 1 << 20;
/// File bytes above this never enter the cook (mirrors the watcher's `MAX_CONTENT_BYTES`).
pub const MAX_WATCH_BYTES: usize = 8 * 1024 * 1024;
/// Cap on the remembered presentation error history; oldest entries fall off.
pub const MAX_ERRORS: usize = 64;
/// Cap on the [`ReloadReport::last_reason`] diagnostic string.
pub const MAX_REASON_CHARS: usize = 128;
/// Shipped AI curve version pinned for tapes without commands: the
/// shipped `engine-ai` version, referenced (not copied).
pub use engine_ai::DEFAULT_CURVES_VERSION as AI_VERSION_V1;
/// Shipped combat rules version pinned for tapes without commands: the
/// shipped combat row, referenced (not copied) so the pin can never drift.
pub use engine_combat::TABLES_VERSION_V1 as RULES_VERSION_V1;
/// PNG magic: the only texture container the pre-check accepts (the render
/// lane decodes PNG-only; full decode stays there).
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Truncate a diagnostic to [`MAX_REASON_CHARS`] chars (never panics).
fn cap_reason(text: String) -> String {
    text.chars().take(MAX_REASON_CHARS).collect()
}

/// Stable asset identity: the 16 UUID bytes of the watcher's `AssetId` (and the
/// live-reload `LiveAssetId`). `NIL` (all zeros) is never minted and never registered.
/// Conversion one-liners for the orchestrator edge:
/// `asset.as_object_id().as_uuid().into_bytes()` (watcher in) and
/// `LiveAssetId::from_bytes(handle.to_bytes())` (live reload out).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct AssetHandle([u8; 16]);

impl AssetHandle {
    /// The nil id: never minted, never registered, always refused.
    pub const NIL: Self = Self([0u8; 16]);

    /// Wrap 16 UUID bytes.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// The wrapped bytes, for logging or round-trips back to watcher and live-reload ids.
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    /// Mint handle `n` (`0` is [`AssetHandle::NIL`]; watches start at `1`).
    const fn mint(n: u64) -> Self {
        let bytes = n.to_be_bytes();
        Self([
            0, 0, 0, 0, 0, 0, 0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
            bytes[6], bytes[7],
        ])
    }

    /// True for the nil id.
    #[must_use]
    pub fn is_nil(self) -> bool {
        self.0 == [0u8; 16]
    }

    /// Parse 32 hex digits, with or without `-` separators (lenient: every
    /// `-` is skipped, positions unchecked). Anything else is
    /// [`PresentError::BadAsset`].
    pub fn parse_hex(text: &str) -> Result<Self, PresentError> {
        let mut digits = [0u8; 32];
        let mut count = 0usize;
        for ch in text.chars() {
            if ch == '-' {
                continue;
            }
            let Some(digit) = ch.to_digit(16) else {
                return Err(PresentError::BadAsset);
            };
            if count >= 32 {
                return Err(PresentError::BadAsset);
            }
            digits[count] = digit as u8;
            count += 1;
        }
        if count != 32 {
            return Err(PresentError::BadAsset);
        }
        let mut bytes = [0u8; 16];
        for (i, pair) in digits.chunks_exact(2).enumerate() {
            bytes[i] = (pair[0] << 4) | pair[1];
        }
        Ok(Self(bytes))
    }
}

impl core::fmt::Display for AssetHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0u8; 32];
        for (i, byte) in self.0.iter().enumerate() {
            text[i * 2] = HEX[usize::from(byte >> 4)];
            text[i * 2 + 1] = HEX[usize::from(byte & 0x0f)];
        }
        // Hex digits are ASCII by construction; a failure is a programming bug,
        // and Display must not panic on IO data — but this table is local, so
        // the invariant is proven here, not on user input.
        let rendered = std::str::from_utf8(&text).unwrap_or("????????????????????????????????");
        write!(f, "{rendered}")
    }
}

/// What kind of payload an asset id carries. Textures and scenes have no slot
/// model here: one versioned blob per id, validated by the pre-checks below.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PresentKind {
    /// WGSL pipeline source (structural pre-check, mirror of the live-reload cook).
    Shader,
    /// Material factor document (bounded UTF-8 pre-check; full JSON cook in render).
    Material,
    /// PNG image bytes (magic pre-check; full decode in render).
    Texture,
    /// Scene document (bounded UTF-8 pre-check; full codec in `engine-scene`).
    Scene,
}

impl core::fmt::Display for PresentKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Shader => write!(f, "shader"),
            Self::Material => write!(f, "material"),
            Self::Texture => write!(f, "texture"),
            Self::Scene => write!(f, "scene"),
        }
    }
}

/// Presentation swap failures. Every variant keeps last-good untouched;
/// cook failures additionally record into the bounded error history.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PresentError {
    /// Asset id text is not 32 hex digits.
    BadAsset,
    /// Notice version is `0`: the watcher's "never cooked", nothing to load.
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
    /// Material document is not valid UTF-8.
    MaterialNotUtf8,
    /// Material braces or brackets do not balance.
    MaterialUnbalanced,
    /// Texture bytes lack the PNG magic.
    TextureBadMagic,
    /// Scene document is not valid UTF-8.
    SceneNotUtf8,
    /// Scene braces do not balance.
    SceneUnbalanced,
}

impl core::fmt::Display for PresentError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadAsset => write!(f, "asset id is not 32 hex digits"),
            Self::VersionZero => write!(f, "notice version is 0 (never cooked)"),
            Self::EmptyPayload => write!(f, "payload is empty"),
            Self::PayloadTooLarge { bytes, limit } => {
                write!(f, "payload {bytes} bytes exceeds {limit}")
            }
            Self::ShaderNotUtf8 => write!(f, "shader is not valid UTF-8"),
            Self::ShaderMissingEntry(entry) => {
                write!(f, "shader lacks the {entry} entry point")
            }
            Self::ShaderUnbalanced => write!(f, "shader braces or parens do not balance"),
            Self::ShaderNul => write!(f, "shader contains a NUL byte"),
            Self::MaterialNotUtf8 => write!(f, "material is not valid UTF-8"),
            Self::MaterialUnbalanced => write!(f, "material braces or brackets do not balance"),
            Self::TextureBadMagic => write!(f, "texture lacks the PNG magic"),
            Self::SceneNotUtf8 => write!(f, "scene is not valid UTF-8"),
            Self::SceneUnbalanced => write!(f, "scene braces do not balance"),
        }
    }
}

impl std::error::Error for PresentError {}

/// Outcome of one presentation swap attempt.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PresentOutcome {
    /// Payload cooked and published; version now current for the asset.
    Swapped {
        /// Kind that swapped.
        kind: PresentKind,
        /// Installed version.
        version: u64,
    },
    /// Notice is stale (`version <= current`): ignored, nothing touched and
    /// nothing recorded, so rapid bursts coalesce with last version winning.
    Stale {
        /// Current version for the asset.
        current: u64,
    },
    /// Payload failed the pre-check (broken shader, bad magic, unbalanced
    /// document): last-good kept, error recorded.
    KeptLastGood {
        /// Why the cook failed.
        error: PresentError,
    },
    /// Payload refused by policy (version zero, nil id): last-good kept,
    /// error recorded.
    Refused {
        /// Why the swap was refused.
        error: PresentError,
    },
    /// Asset was never registered: ignored, nothing touched, nothing recorded.
    UnknownAsset,
}

/// One recorded presentation failure: which notice failed and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ErrorRecord {
    /// Asset the failed notice named.
    pub asset: AssetHandle,
    /// Version the failed notice carried.
    pub version: u64,
    /// Why the swap did not happen.
    pub error: PresentError,
}

/// AI curve validation failures: the `engine-ai` [`AiCurveError`] type itself,
/// re-exported under the practice name for match-arm exhaustiveness.
pub use engine_ai::AiCurveError as AiRulesError;

/// Largest legal table entry: `engine-ai` `BIAS_MAX`, referenced.
pub use engine_ai::BIAS_MAX as AI_BIAS_MAX;
/// Neutral bias: `engine-ai` `BIAS_NEUTRAL`, referenced.
pub use engine_ai::BIAS_NEUTRAL as AI_BIAS_NEUTRAL;
/// Curve table lanes: `engine-ai` `CURVE_LANES`, referenced (not copied).
pub use engine_ai::CURVE_LANES as AI_CURVE_LANES;
/// Scripted aggression lane: `engine-ai` `DEFAULT_AGGRESSION`, referenced.
pub use engine_ai::DEFAULT_AGGRESSION as AI_AGGRESSION_DEFAULT;
/// Scripted caution lane: `engine-ai` `DEFAULT_CAUTION`, referenced.
pub use engine_ai::DEFAULT_CAUTION as AI_CAUTION_DEFAULT;
/// Scripted engage range in cm: `engine-ai` `DEFAULT_ENGAGE_RANGE_CM`, referenced.
pub use engine_ai::DEFAULT_ENGAGE_RANGE_CM as AI_ENGAGE_DEFAULT_CM;
/// Scripted default morale: `engine-ai` `DEFAULT_MORALE`, referenced.
pub use engine_ai::DEFAULT_MORALE as AI_MORALE_DEFAULT;
/// Largest sane engage range in cm: `engine-ai` `ENGAGE_RANGE_MAX_CM`, referenced.
pub use engine_ai::ENGAGE_RANGE_MAX_CM as AI_ENGAGE_MAX_CM;
/// Smallest sane engage range in cm: `engine-ai` `ENGAGE_RANGE_MIN_CM`, referenced.
pub use engine_ai::ENGAGE_RANGE_MIN_CM as AI_ENGAGE_MIN_CM;

/// Immutable AI configuration snapshot: exact mirror of `engine-ai`'s `AiConfig`
/// (curve tables plus the morale a new brain starts at). Cloned into the
/// rules log at proposal time; later tuning edits never mutate a recorded
/// command. Conversion for the orchestrator edge, one line each way:
/// `AiConfig { curves: AiCurves { version, aggression, caution,
/// engage_range_cm }, morale_default }` with these field names and types.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AiRules {
    /// Curve-set version; `0` is never valid (it means "unversioned").
    pub version: u32,
    /// Morale-to-attack-bias table, `0..=1000` per lane, non-decreasing.
    pub aggression: [u16; AI_CURVE_LANES],
    /// Morale-to-caution table, `0..=1000` per lane, any shape.
    pub caution: [u16; AI_CURVE_LANES],
    /// Strike-commit range in centimetres, `50..=6000`.
    pub engage_range_cm: u32,
    /// Starting morale, `0..=1000`.
    pub morale_default: i32,
}

impl AiRules {
    /// Scripted defaults: the scripted policy as a validated curve set, neutral morale.
    pub const fn scripted_defaults() -> Self {
        Self {
            version: AI_VERSION_V1,
            aggression: AI_AGGRESSION_DEFAULT,
            caution: AI_CAUTION_DEFAULT,
            engage_range_cm: AI_ENGAGE_DEFAULT_CM,
            morale_default: AI_MORALE_DEFAULT,
        }
    }

    /// Range-check this snapshot; first failure wins, deterministically
    /// (mirrors `engine-ai`'s `validate_config` rule for rule).
    pub fn validate(&self) -> Result<(), AiRulesError> {
        if self.version == 0 {
            return Err(AiRulesError::ZeroVersion);
        }
        for (index, value) in self.aggression.iter().enumerate() {
            if *value > AI_BIAS_MAX {
                return Err(AiRulesError::AggressionOutOfRange {
                    index,
                    value: *value,
                });
            }
        }
        for (index, value) in self.caution.iter().enumerate() {
            if *value > AI_BIAS_MAX {
                return Err(AiRulesError::CautionOutOfRange {
                    index,
                    value: *value,
                });
            }
        }
        for index in 1..AI_CURVE_LANES {
            if self.aggression[index] < self.aggression[index - 1] {
                return Err(AiRulesError::AggressionNotMonotonic {
                    index,
                    prev: self.aggression[index - 1],
                    next: self.aggression[index],
                });
            }
        }
        if self.engage_range_cm < AI_ENGAGE_MIN_CM || self.engage_range_cm > AI_ENGAGE_MAX_CM {
            return Err(AiRulesError::EngageRangeOutOfRange {
                value: self.engage_range_cm,
            });
        }
        if self.morale_default < MORALE_MIN || self.morale_default > MORALE_MAX {
            return Err(AiRulesError::BadMoraleDefault {
                value: self.morale_default,
            });
        }
        Ok(())
    }

    /// True when [`AiRules::validate`] accepts this snapshot.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.validate().is_ok()
    }
}

impl Default for AiRules {
    fn default() -> Self {
        Self::scripted_defaults()
    }
}

/// Typed refusal for a rules or presentation swap proposal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReloadRefusal {
    /// Competitive matches pin their rules/content versions: the proposal
    /// names what it tried (`"combat tables"`, `"ai curves"`, `"presentation"`).
    CompetitivePinned {
        /// What the caller tried to swap.
        what: &'static str,
    },
    /// Combat version is not newer than the effective one: last-good kept.
    StaleCombat {
        /// Effective version at the cursor.
        current: u32,
        /// Offered version.
        offered: u32,
    },
    /// AI version is not newer than the effective one: last-good kept.
    StaleAi {
        /// Effective version at the cursor.
        current: u32,
        /// Offered version.
        offered: u32,
    },
    /// AI snapshot failed validation: last-good kept, cause carried.
    InvalidAi(AiRulesError),
}

impl core::fmt::Display for ReloadRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CompetitivePinned { what } => {
                write!(f, "competitive match pins its {what}: live swap refused")
            }
            Self::StaleCombat { current, offered } => write!(
                f,
                "combat version {offered} is not newer than effective {current}"
            ),
            Self::StaleAi { current, offered } => write!(
                f,
                "ai version {offered} is not newer than effective {current}"
            ),
            Self::InvalidAi(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ReloadRefusal {}

/// One recorded combat rules command: the snapshot plus the tick it takes
/// effect at. Snapshots are owned values, so later tuning never mutates a
/// recorded command.
#[derive(Clone, PartialEq, Eq, Debug)]
struct CombatCommand {
    at_tick: u32,
    swap: SwapInput,
}

/// One recorded AI rules command, same shape as [`CombatCommand`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct AiCommand {
    at_tick: u32,
    rules: AiRules,
}

/// File snapshot: mtime plus length (length rides along because some
/// filesystems tick mtime coarsely; either half changing is an event).
/// Mirrors `assets::watcher` one field for one field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Signature {
    secs: u64,
    nanos: u32,
    len: u64,
}

/// Read the current snapshot; `None` when the path has no readable metadata.
fn signature(path: &str) -> Option<Signature> {
    let meta = std::fs::metadata(path).ok()?;
    let (secs, nanos) = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or((0, 0), |d| (d.as_secs(), d.subsec_nanos()));
    Some(Signature {
        secs,
        nanos,
        len: meta.len(),
    })
}

/// One watched path: its kind, last-polled snapshot and cooked version.
#[derive(Clone, Debug)]
struct WatchRec {
    path: String,
    kind: PresentKind,
    known: Option<Signature>,
    version: u64,
}

/// Structural WGSL pre-check: bounded, UTF-8, both entry points, balanced
/// braces/parens, no NUL. Mirror of the live-reload cook (same accepts, same
/// rejects); full pipeline compilation stays with the wgpu layer.
fn cook_shader(bytes: &[u8]) -> Result<String, PresentError> {
    if bytes.is_empty() {
        return Err(PresentError::EmptyPayload);
    }
    if bytes.len() > MAX_SHADER_BYTES {
        return Err(PresentError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_SHADER_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PresentError::ShaderNotUtf8)?;
    if text.contains('\0') {
        return Err(PresentError::ShaderNul);
    }
    if !text.contains("@vertex") {
        return Err(PresentError::ShaderMissingEntry("@vertex"));
    }
    if !text.contains("@fragment") {
        return Err(PresentError::ShaderMissingEntry("@fragment"));
    }
    let mut braces: i64 = 0;
    let mut parens: i64 = 0;
    for ch in text.chars() {
        match ch {
            '{' => braces += 1,
            '}' => {
                braces -= 1;
                if braces < 0 {
                    return Err(PresentError::ShaderUnbalanced);
                }
            }
            '(' => parens += 1,
            ')' => {
                parens -= 1;
                if parens < 0 {
                    return Err(PresentError::ShaderUnbalanced);
                }
            }
            _ => {}
        }
    }
    if braces != 0 || parens != 0 {
        return Err(PresentError::ShaderUnbalanced);
    }
    Ok(text.to_string())
}

/// Material pre-check: bounded UTF-8 with balanced braces/brackets. The full
/// JSON factor/slot cook stays in render (`LiveSet`); this pre-check only
/// refuses what is certainly not a document.
fn cook_material(bytes: &[u8]) -> Result<(), PresentError> {
    if bytes.is_empty() {
        return Err(PresentError::EmptyPayload);
    }
    if bytes.len() > MAX_MATERIAL_BYTES {
        return Err(PresentError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_MATERIAL_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PresentError::MaterialNotUtf8)?;
    balanced(text, true)
        .then_some(())
        .ok_or(PresentError::MaterialUnbalanced)
}

/// Scene pre-check: bounded UTF-8 with balanced braces. The full scene-codec
/// validation stays in `engine-scene`; this pre-check only refuses what is
/// certainly not a document.
fn cook_scene(bytes: &[u8]) -> Result<(), PresentError> {
    if bytes.is_empty() {
        return Err(PresentError::EmptyPayload);
    }
    if bytes.len() > MAX_SCENE_BYTES {
        return Err(PresentError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_SCENE_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| PresentError::SceneNotUtf8)?;
    balanced(text, false)
        .then_some(())
        .ok_or(PresentError::SceneUnbalanced)
}

/// Balanced-delimiter scan: braces always, brackets only when `brackets`.
/// A closing delimiter below zero fails fast; nonzero totals fail at the end.
fn balanced(text: &str, brackets: bool) -> bool {
    let mut braces: i64 = 0;
    let mut squares: i64 = 0;
    for ch in text.chars() {
        match ch {
            '{' => braces += 1,
            '}' => {
                braces -= 1;
                if braces < 0 {
                    return false;
                }
            }
            '[' if brackets => squares += 1,
            ']' if brackets => {
                squares -= 1;
                if squares < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    braces == 0 && squares == 0
}

/// Texture pre-check: bounded bytes with the PNG magic up front. Full decode
/// (dimensions, texel budget) stays in render; this pre-check only refuses
/// what is certainly not a PNG.
fn cook_texture(bytes: &[u8]) -> Result<(), PresentError> {
    if bytes.is_empty() {
        return Err(PresentError::EmptyPayload);
    }
    if bytes.len() > MAX_TEXTURE_BYTES {
        return Err(PresentError::PayloadTooLarge {
            bytes: bytes.len(),
            limit: MAX_TEXTURE_BYTES,
        });
    }
    if bytes.len() < PNG_MAGIC.len() || bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err(PresentError::TextureBadMagic);
    }
    Ok(())
}

/// Versioned presentation slots fed by watched files and tuning notices.
/// Starts empty: no payload is live until the first good swap, so the caller
/// applies the shipped content before presenting. All iteration is
/// `BTreeMap` order; no wall clock, no threads, no GPU calls — publication
/// timing stays with [`PracticeMode::tick`].
#[derive(Clone, Debug, Default)]
struct PresentationStore {
    registered: BTreeMap<AssetHandle, PresentKind>,
    versions: BTreeMap<AssetHandle, u64>,
    shaders: BTreeMap<AssetHandle, String>,
    active_shader: Option<AssetHandle>,
    materials: BTreeMap<AssetHandle, ()>,
    textures: BTreeMap<AssetHandle, usize>,
    scenes: BTreeMap<AssetHandle, Vec<u8>>,
    errors: VecDeque<ErrorRecord>,
    dropped_errors: u64,
}

impl PresentationStore {
    fn new() -> Self {
        Self::default()
    }

    /// Bind an asset id to its payload kind (setup-time). Rebinding never
    /// mutates cooked entries: the next notice cooks under the new kind.
    fn register(&mut self, asset: AssetHandle, kind: PresentKind) {
        self.registered.insert(asset, kind);
    }

    /// Forget an asset binding. Cooked entries and versions stay readable;
    /// later notices for the id report [`PresentOutcome::UnknownAsset`].
    fn unregister(&mut self, asset: AssetHandle) -> bool {
        self.registered.remove(&asset).is_some()
    }

    /// Current version for an id, if it ever swapped successfully.
    fn version_for(&self, asset: AssetHandle) -> Option<u64> {
        self.versions.get(&asset).copied()
    }

    /// Live pipeline source: the last-good shader, if any good swap landed.
    /// The tuning panel reads this back as smooth-building evidence.
    fn shader_source(&self) -> Option<&str> {
        self.active_shader
            .and_then(|id| self.shaders.get(&id))
            .map(String::as_str)
    }

    /// Recorded presentation failures, oldest first, bounded by [`MAX_ERRORS`].
    fn error_history(&self) -> &VecDeque<ErrorRecord> {
        &self.errors
    }

    /// Apply one notice with its recooked bytes. Atomic per call: the payload
    /// cooks into an owned value first, and only a fully validated value is
    /// published — failures keep last-good and record the error, never
    /// half-swap.
    fn apply_notice(&mut self, asset: AssetHandle, version: u64, bytes: &[u8]) -> PresentOutcome {
        if version == 0 {
            let error = PresentError::VersionZero;
            self.record(asset, version, error.clone());
            return PresentOutcome::Refused { error };
        }
        if asset.is_nil() {
            self.record(asset, version, PresentError::BadAsset);
            return PresentOutcome::Refused {
                error: PresentError::BadAsset,
            };
        }
        let Some(kind) = self.registered.get(&asset).copied() else {
            return PresentOutcome::UnknownAsset;
        };
        let current = self.versions.get(&asset).copied().unwrap_or(0);
        if version <= current {
            return PresentOutcome::Stale { current };
        }
        let cooked = match kind {
            PresentKind::Shader => cook_shader(bytes).map(|_| ()),
            PresentKind::Material => cook_material(bytes),
            PresentKind::Texture => cook_texture(bytes),
            PresentKind::Scene => cook_scene(bytes),
        };
        if let Err(error) = cooked {
            self.record(asset, version, error.clone());
            return PresentOutcome::KeptLastGood { error };
        }
        match kind {
            PresentKind::Shader => {
                // The cook above validated; re-derive the owned source once
                // (UTF-8 was proven, so this cannot fail — the expect names
                // the local invariant, never IO or user data).
                let source = std::str::from_utf8(bytes)
                    .expect("shader UTF-8 proven by the cook above")
                    .to_string();
                self.shaders.insert(asset, source);
                self.active_shader = Some(asset);
            }
            PresentKind::Material => {
                self.materials.insert(asset, ());
            }
            PresentKind::Texture => {
                self.textures.insert(asset, bytes.len());
            }
            PresentKind::Scene => {
                self.scenes.insert(asset, bytes.to_vec());
            }
        }
        self.versions.insert(asset, version);
        PresentOutcome::Swapped { kind, version }
    }

    /// Record a failure in the bounded history, shedding the oldest first.
    fn record(&mut self, asset: AssetHandle, version: u64, error: PresentError) {
        self.errors.push_back(ErrorRecord {
            asset,
            version,
            error,
        });
        while self.errors.len() > MAX_ERRORS {
            if self.errors.pop_front().is_none() {
                break;
            }
            self.dropped_errors = self.dropped_errors.saturating_add(1);
        }
    }
}

/// One live presentation slot for the status report, in handle order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PresentEntry {
    /// The recooked asset; stable across recooks by construction.
    pub asset: AssetHandle,
    /// What kind of payload the id carries.
    pub kind: PresentKind,
    /// The installed per-path version.
    pub version: u64,
}

/// Reload status: everything a tuning panel or evidence receipt needs, as data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReloadReport {
    /// Live presentation slots, in handle order.
    pub presentation: Vec<PresentEntry>,
    /// Effective combat rules version at the cursor (1 when no swap recorded).
    pub combat_version: u32,
    /// Effective AI rules version at the cursor (1 when no swap recorded).
    pub ai_version: u32,
    /// Recorded combat commands (applied plus future — commands only ever
    /// target the cursor, so all are effective history).
    pub combat_commands: usize,
    /// Recorded AI commands.
    pub ai_commands: usize,
    /// Presentation swaps applied since construction.
    pub presentation_swaps: u64,
    /// Notices ignored as stale or unknown since construction.
    pub dropped_notices: u64,
    /// The last keep-last-good or refusal reason, if any (capped).
    pub last_reason: Option<String>,
}

/// Running practice mode: the [`App`] plus the asset and rules publication
/// boundary. Presentation swaps land at frame boundaries inside
/// [`PracticeMode::tick`]; rules swaps are recorded as authority tick
/// commands and take effect at the next tick boundary, never mid-tick.
pub struct PracticeMode {
    app: App,
    store: PresentationStore,
    watches: BTreeMap<AssetHandle, WatchRec>,
    next_handle: u64,
    combat_log: Vec<CombatCommand>,
    ai_log: Vec<AiCommand>,
    presentation_swaps: u64,
    dropped_notices: u64,
    last_reason: Option<String>,
    combat_rows_path: Option<String>,
    combat_rows_known: Option<Signature>,
}

impl PracticeMode {
    /// Start practice on a session config: validates like [`App::new`].
    pub fn new(config: GameConfig) -> Result<Self, super::GameError> {
        Ok(Self {
            app: App::new(config)?,
            store: PresentationStore::new(),
            watches: BTreeMap::new(),
            next_handle: 1,
            combat_log: Vec::new(),
            ai_log: Vec::new(),
            presentation_swaps: 0,
            dropped_notices: 0,
            last_reason: None,
            combat_rows_path: None,
            combat_rows_known: None,
        })
    }

    /// Session configuration.
    #[must_use]
    pub fn config(&self) -> GameConfig {
        self.app.config()
    }

    /// Fixed-tick cursor: ticks simulated so far.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.app.cursor()
    }

    /// Checkpoints for every simulated tick, in tick order.
    #[must_use]
    pub fn checkpoints(&self) -> &[super::Checkpoint] {
        self.app.checkpoints()
    }

    /// Live view: HUD, hp, progression, presentation tracks.
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }

    /// Live pipeline source: the last-good shader, if any good swap landed.
    /// The tuning panel reads this back as smooth-building evidence.
    #[must_use]
    pub fn shader_source(&self) -> Option<&str> {
        self.store.shader_source()
    }

    /// Live scene bytes for a handle, if its scene ever swapped successfully.
    /// Readback for scene-reload evidence.
    #[must_use]
    pub fn scene_for(&self, asset: AssetHandle) -> Option<&[u8]> {
        self.store.scenes.get(&asset).map(Vec::as_slice)
    }

    /// Recorded presentation failures, oldest first, bounded by [`MAX_ERRORS`].
    #[must_use]
    pub fn error_history(&self) -> &VecDeque<ErrorRecord> {
        self.store.error_history()
    }

    /// Watch a file for live reload. Mints a stable handle (first watch
    /// mints, later polls recook under it) and records the current snapshot
    /// as the silent baseline, so only later edits cook — mirroring the
    /// asset watcher default. Always succeeds; a missing file simply reports
    /// nothing until it appears.
    pub fn watch(&mut self, path: &str, kind: PresentKind) -> AssetHandle {
        // A path watched twice keeps its stable handle (reimport keeps IDs):
        // only the kind rebinds, cooked entries untouched.
        if let Some(handle) = self
            .watches
            .iter()
            .find(|(_, rec)| rec.path == path)
            .map(|(handle, _)| *handle)
        {
            if let Some(rec) = self.watches.get_mut(&handle) {
                rec.kind = kind;
            }
            self.store.register(handle, kind);
            return handle;
        }
        let handle = AssetHandle::mint(self.next_handle.max(1));
        self.next_handle = self.next_handle.max(1).saturating_add(1).max(1);
        self.store.register(handle, kind);
        self.watches.insert(
            handle,
            WatchRec {
                path: path.to_string(),
                kind,
                known: signature(path),
                version: 0,
            },
        );
        handle
    }

    /// Stop watching a handle. Cooked entries and versions stay readable;
    /// later notices for the id report [`PresentOutcome::UnknownAsset`].
    pub fn unwatch(&mut self, asset: AssetHandle) -> bool {
        let watched = self.watches.remove(&asset).is_some();
        let bound = self.store.unregister(asset);
        watched || bound
    }

    /// Feed one externally recooked notice (the tuning child-IPC path: the
    /// watcher's `ReloadNotice` converts with `AssetHandle::from_bytes` plus the kind
    /// from the watched path suffix). Applies at the call boundary — callers
    /// feed these between ticks, never mid-tick.
    pub fn notify(&mut self, asset: AssetHandle, version: u64, bytes: &[u8]) -> PresentOutcome {
        let outcome = self.store.apply_notice(asset, version, bytes);
        self.account(&outcome, asset, version);
        // A good external notice also moves the watch version forward when it
        // names a watched handle, so file polls do not recook it again.
        if let PresentOutcome::Swapped { version, .. } = outcome {
            if let Some(rec) = self.watches.get_mut(&asset) {
                rec.version = rec.version.max(version);
                rec.known = signature(&rec.path);
            }
        }
        outcome
    }

    /// Register an asset id with no file behind it (tuning-authored payloads
    /// fed through [`PracticeMode::notify`]). Mints a fresh stable handle.
    pub fn register_external(&mut self, kind: PresentKind) -> AssetHandle {
        let handle = AssetHandle::mint(self.next_handle.max(1));
        self.next_handle = self.next_handle.max(1).saturating_add(1).max(1);
        self.store.register(handle, kind);
        handle
    }

    /// Advance one fixed tick: poll watched files and apply presentation
    /// swaps at the frame boundary first, then step authority on the human
    /// input. Rules swaps recorded by `propose_*` take effect for this tick
    /// when their `at_tick` arrived — always at the boundary, never mid-tick.
    pub fn tick(&mut self, input: engine_input::FighterInput) -> TickOutcome {
        self.poll_watches();
        self.poll_combat_rows();
        self.app.tick(input)
    }

    /// Propose a combat rules swap: the validated [`SwapInput`] snapshot is
    /// recorded as an authority tick command at the cursor, effective from
    /// this tick on. Stale versions (not newer than effective) are refused
    /// with last-good kept. Never touches the in-flight tick mid-step.
    pub fn propose_combat_swap(&mut self, swap: SwapInput) -> Result<u32, ReloadRefusal> {
        let offered = swap.version();
        let current = self.effective_combat(self.app.cursor());
        if offered <= current {
            return Err(ReloadRefusal::StaleCombat { current, offered });
        }
        self.combat_log.push(CombatCommand {
            at_tick: self.app.cursor(),
            swap,
        });
        Ok(offered)
    }

    /// Propose an AI rules swap: same command discipline as
    /// [`PracticeMode::propose_combat_swap`]. Invalid snapshots are refused
    /// with their cause and keep last-good; stale versions are refused.
    pub fn propose_ai_swap(&mut self, rules: AiRules) -> Result<u32, ReloadRefusal> {
        if let Err(err) = rules.validate() {
            let refusal = ReloadRefusal::InvalidAi(err);
            self.last_reason = Some(cap_reason(refusal.to_string()));
            return Err(refusal);
        }
        let offered = rules.version;
        let current = self.effective_ai(self.app.cursor());
        if offered <= current {
            return Err(ReloadRefusal::StaleAi { current, offered });
        }
        self.ai_log.push(AiCommand {
            at_tick: self.app.cursor(),
            rules,
        });
        Ok(offered)
    }

    /// Watch a combat rows file for live reload (`content/design/combat.rows`
    /// shape through `engine_combat::tables_text::parse_tables`).
    ///
    /// Consumer: [`PracticeMode::tick`] polls the file at the frame boundary
    /// and records a valid newer row as an authority tick command, so
    /// designers change numbers with no Rust edit. Data owner: `engine-combat`
    /// (`Tables`, `SwapInput`, `validate_tables`). Units match `Tables`:
    /// damage permille, ticks at 60 Hz, stamina points. Determinism: same file
    /// bytes plus same inputs give the same command log and checkpoints;
    /// mtimes never enter hashed state. Errors: a stale or invalid file is
    /// refused and last-good stays, with the reason in
    /// [`PracticeMode::reload_status`]; a missing file reports nothing until
    /// it appears. Cold path only, never tick work. Production path is
    /// `content/design/combat.rows`.
    /// Caller: [`PracticeMode::tick`] (poll edge; the arena wires the path).
    pub fn watch_combat_rows(&mut self, path: &str) {
        self.combat_rows_path = Some(path.to_string());
        self.combat_rows_known = signature(path);
    }

    /// Poll the watched combat rows file once, at the frame boundary.
    /// Mirrors [`PracticeMode::poll_watches`]: the mtime-plus-length snapshot
    /// decides; only a changed file is read, parsed and proposed. Read
    /// failures and oversize files accept the snapshot silently; parse and
    /// range failures record their reason; stale versions refuse silently.
    /// A good newer row lands once via [`PracticeMode::propose_combat_swap`].
    fn poll_combat_rows(&mut self) {
        let current = match self.combat_rows_path.as_ref() {
            Some(path) => signature(path),
            None => return,
        };
        if current == self.combat_rows_known {
            return;
        }
        self.combat_rows_known = current;
        let Some(path) = self.combat_rows_path.clone() else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        if bytes.len() > MAX_WATCH_BYTES {
            return;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            self.last_reason = Some(cap_reason("combat.rows refused: not UTF-8".to_string()));
            return;
        };
        match engine_combat::tables_text::parse_tables(text) {
            Ok(swap) => {
                // Stale versions refuse without touching the log, so the same
                // file never lands twice.
                let _ = self.propose_combat_swap(swap);
            }
            Err(err) => {
                self.last_reason = Some(cap_reason(format!("combat.rows refused: {err:?}")));
            }
        }
    }

    /// Effective combat version for `tick`: the latest command with
    /// `at_tick <= tick`, or [`RULES_VERSION_V1`] when none recorded. Pure.
    #[must_use]
    pub fn combat_version_for_tick(&self, tick: u32) -> u32 {
        self.effective_combat(tick)
    }

    /// Effective AI version for `tick`: same discipline as
    /// [`PracticeMode::combat_version_for_tick`]. Pure.
    #[must_use]
    pub fn ai_version_for_tick(&self, tick: u32) -> u32 {
        self.effective_ai(tick)
    }

    /// Reload status for the tuning panel and evidence receipts, as data.
    #[must_use]
    pub fn reload_status(&self) -> ReloadReport {
        let presentation = self
            .store
            .registered
            .iter()
            .filter_map(|(asset, kind)| {
                self.store.version_for(*asset).map(|version| PresentEntry {
                    asset: *asset,
                    kind: *kind,
                    version,
                })
            })
            .collect();
        ReloadReport {
            presentation,
            combat_version: self.effective_combat(self.app.cursor()),
            ai_version: self.effective_ai(self.app.cursor()),
            combat_commands: self.combat_log.len(),
            ai_commands: self.ai_log.len(),
            presentation_swaps: self.presentation_swaps,
            dropped_notices: self.dropped_notices,
            last_reason: self.last_reason.clone(),
        }
    }

    /// Latest combat version effective at `tick` (log order is cursor order,
    /// so the last entry at or below `tick` wins). Pure.
    fn effective_combat(&self, tick: u32) -> u32 {
        let mut version = RULES_VERSION_V1;
        for command in &self.combat_log {
            if command.at_tick <= tick {
                version = command.swap.version();
            }
        }
        version
    }

    /// Latest AI version effective at `tick`. Pure.
    fn effective_ai(&self, tick: u32) -> u32 {
        let mut version = AI_VERSION_V1;
        for command in &self.ai_log {
            if command.at_tick <= tick {
                version = command.rules.version;
            }
        }
        version
    }

    /// Poll every watch in handle order; cook each change at this frame
    /// boundary. Read failures (deleted mid-poll, oversize) accept the new
    /// snapshot and keep last-good — mirroring the watcher's removal rule — while
    /// cook failures record their reason and keep last-good with it.
    fn poll_watches(&mut self) {
        // Snapshot the handles first: `BTreeMap` order is the deterministic
        // visit order, and the borrow must end before cooks mutate the store.
        let handles: Vec<AssetHandle> = self.watches.keys().copied().collect();
        for handle in handles {
            let Some(rec) = self.watches.get(&handle) else {
                continue;
            };
            let current = signature(&rec.path);
            if current == rec.known {
                continue;
            }
            let path = rec.path.clone();
            let candidate = rec.version.saturating_add(1).max(1);
            match std::fs::read(&path) {
                Ok(bytes) if bytes.len() <= MAX_WATCH_BYTES => {
                    let outcome = self.store.apply_notice(handle, candidate, &bytes);
                    self.account(&outcome, handle, candidate);
                    if matches!(outcome, PresentOutcome::Swapped { .. }) {
                        if let Some(rec) = self.watches.get_mut(&handle) {
                            rec.version = candidate;
                        }
                    }
                    if let Some(rec) = self.watches.get_mut(&handle) {
                        rec.known = current;
                    }
                }
                // Unreadable or oversize: accept the snapshot (a deletion
                // queues nothing), keep last-good silently.
                _ => {
                    if let Some(rec) = self.watches.get_mut(&handle) {
                        rec.known = current;
                    }
                }
            }
        }
    }

    /// Fold one presentation outcome into counters and the last reason.
    fn account(&mut self, outcome: &PresentOutcome, asset: AssetHandle, version: u64) {
        match outcome {
            PresentOutcome::Swapped { .. } => {
                self.presentation_swaps = self.presentation_swaps.saturating_add(1);
            }
            PresentOutcome::Stale { .. } | PresentOutcome::UnknownAsset => {
                self.dropped_notices = self.dropped_notices.saturating_add(1);
            }
            PresentOutcome::KeptLastGood { error } | PresentOutcome::Refused { error } => {
                self.last_reason = Some(cap_reason(format!("{asset} v{version}: {error}")));
            }
        }
    }
}

/// Competitive match: the same [`App`] authority with rules/content versions
/// pinned. Every live-swap proposal fails with a typed
/// [`ReloadRefusal::CompetitivePinned`] and changes nothing — old tapes
/// replay through the pinned V1 rules by construction.
pub struct CompetitiveMatch {
    app: App,
}

impl CompetitiveMatch {
    /// Start a competitive match: validates like [`App::new`].
    pub fn new(config: GameConfig) -> Result<Self, super::GameError> {
        Ok(Self {
            app: App::new(config)?,
        })
    }

    /// Session configuration.
    #[must_use]
    pub fn config(&self) -> GameConfig {
        self.app.config()
    }

    /// Fixed-tick cursor: ticks simulated so far.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.app.cursor()
    }

    /// Checkpoints for every simulated tick, in tick order.
    #[must_use]
    pub fn checkpoints(&self) -> &[super::Checkpoint] {
        self.app.checkpoints()
    }

    /// Live view: HUD, hp, progression, presentation tracks.
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }

    /// Advance one fixed tick on a human input (identical to [`App::tick`]).
    pub fn tick(&mut self, input: engine_input::FighterInput) -> TickOutcome {
        self.app.tick(input)
    }

    /// Refused: competitive matches pin their combat tables.
    pub fn propose_combat_swap(&mut self, _swap: SwapInput) -> Result<u32, ReloadRefusal> {
        Err(ReloadRefusal::CompetitivePinned {
            what: "combat tables",
        })
    }

    /// Refused: competitive matches pin their AI curves.
    pub fn propose_ai_swap(&mut self, _rules: AiRules) -> Result<u32, ReloadRefusal> {
        Err(ReloadRefusal::CompetitivePinned { what: "ai curves" })
    }

    /// Refused: competitive matches pin their content versions.
    pub fn notify(
        &mut self,
        _asset: AssetHandle,
        _version: u64,
        _bytes: &[u8],
    ) -> Result<u32, ReloadRefusal> {
        Err(ReloadRefusal::CompetitivePinned {
            what: "presentation",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::fighter::ATTACK;
    use engine_input::{Buttons, FighterId, FighterInput};

    const HUMAN: u8 = super::super::HUMAN_SLOT;

    fn small_config() -> GameConfig {
        GameConfig {
            seed: 7,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        }
    }

    fn idle(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    fn strict_v2_rows() -> engine_combat::Tables {
        let mut rows = engine_combat::TABLES_V1;
        rows.parry_window_ticks = rows.attack_startup_ticks - 1;
        rows.attack_damage = 240;
        rows
    }

    fn swap_v2() -> SwapInput {
        SwapInput::new(2, strict_v2_rows()).unwrap()
    }

    fn ai_v2() -> AiRules {
        AiRules {
            version: 2,
            ..AiRules::scripted_defaults()
        }
    }

    const SHADER_V1: &str = "@vertex fn vs() { } @fragment fn fs() { }";
    const SHADER_V2: &str = "@vertex fn vs() { let x = (1); } @fragment fn fs() { } // tuned";

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "e2040-game-hr5-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn shader_swap_applies_then_bad_reload_keeps_last_good() {
        let mut mode = PracticeMode::new(small_config()).unwrap();
        let handle = mode.register_external(PresentKind::Shader);
        // First good swap publishes version 1.
        assert_eq!(
            mode.notify(handle, 1, SHADER_V1.as_bytes()),
            PresentOutcome::Swapped {
                kind: PresentKind::Shader,
                version: 1,
            }
        );
        assert_eq!(mode.reload_status().presentation_swaps, 1);
        // Broken WGSL keeps last-good with its reason recorded.
        let bad = b"broken ((( no entries";
        assert_eq!(
            mode.notify(handle, 2, bad),
            PresentOutcome::KeptLastGood {
                error: PresentError::ShaderMissingEntry("@vertex"),
            }
        );
        let report = mode.reload_status();
        assert_eq!(report.presentation_swaps, 1);
        assert_eq!(report.presentation.len(), 1);
        assert_eq!(report.presentation[0].version, 1);
        assert_eq!(
            mode.shader_source(),
            Some(SHADER_V1),
            "broken reload must keep the last-good pipeline source"
        );
        assert!(
            report
                .last_reason
                .as_ref()
                .is_some_and(|reason| reason.contains("@vertex")),
            "reason must name the failure: {report:?}"
        );
        // A retry with fixed bytes publishes version 2 under the same handle.
        assert_eq!(
            mode.notify(handle, 2, SHADER_V2.as_bytes()),
            PresentOutcome::Swapped {
                kind: PresentKind::Shader,
                version: 2,
            }
        );
        assert_eq!(mode.shader_source(), Some(SHADER_V2));
        // Stale and unknown notices change nothing and count as dropped.
        assert_eq!(
            mode.notify(handle, 2, SHADER_V2.as_bytes()),
            PresentOutcome::Stale { current: 2 }
        );
        assert_eq!(
            mode.notify(AssetHandle::from_bytes([7u8; 16]), 1, SHADER_V1.as_bytes()),
            PresentOutcome::UnknownAsset
        );
        assert_eq!(mode.reload_status().dropped_notices, 2);
        // Policy refusals keep last-good too.
        assert!(matches!(
            mode.notify(handle, 0, SHADER_V1.as_bytes()),
            PresentOutcome::Refused {
                error: PresentError::VersionZero
            }
        ));
        assert!(matches!(
            mode.notify(AssetHandle::NIL, 1, SHADER_V1.as_bytes()),
            PresentOutcome::Refused {
                error: PresentError::BadAsset
            }
        ));
        assert_eq!(mode.store.shader_source(), Some(SHADER_V2));
    }

    #[test]
    fn material_texture_scene_payloads_validate_and_keep_last_good() {
        let mut mode = PracticeMode::new(small_config()).unwrap();
        let material = mode.register_external(PresentKind::Material);
        let texture = mode.register_external(PresentKind::Texture);
        let scene = mode.register_external(PresentKind::Scene);
        let good_material = br#"{"base_color":[1.0,1.0,1.0,1.0],"metallic":0.0}"#;
        let good_scene = br#"{"arena":"probe","props":[]}"#;
        let mut good_png = PNG_MAGIC.to_vec();
        good_png.extend_from_slice(&[0u8; 64]);
        assert!(matches!(
            mode.notify(material, 1, good_material),
            PresentOutcome::Swapped { .. }
        ));
        assert!(matches!(
            mode.notify(texture, 1, &good_png),
            PresentOutcome::Swapped { .. }
        ));
        assert!(matches!(
            mode.notify(scene, 1, good_scene),
            PresentOutcome::Swapped { .. }
        ));
        // Every bad payload keeps its own last-good with a typed reason.
        assert_eq!(
            mode.notify(material, 2, b"{unbalanced]]"),
            PresentOutcome::KeptLastGood {
                error: PresentError::MaterialUnbalanced,
            }
        );
        assert_eq!(
            mode.notify(texture, 2, b"not a png"),
            PresentOutcome::KeptLastGood {
                error: PresentError::TextureBadMagic,
            }
        );
        assert_eq!(
            mode.notify(scene, 2, b""),
            PresentOutcome::KeptLastGood {
                error: PresentError::EmptyPayload,
            }
        );
        assert_eq!(
            mode.notify(scene, 2, b"{oops"),
            PresentOutcome::KeptLastGood {
                error: PresentError::SceneUnbalanced,
            }
        );
        let report = mode.reload_status();
        assert_eq!(report.presentation.len(), 3);
        assert!(
            report.presentation.iter().all(|entry| entry.version == 1),
            "bad reloads keep last-good versions: {report:?}"
        );
        assert_eq!(report.presentation_swaps, 3);
        // Readbacks serve the tuning panel: live scene bytes and the bounded
        // error history behind the last reason.
        assert_eq!(mode.scene_for(scene), Some(good_scene.as_slice()));
        assert_eq!(mode.error_history().len(), 4);
    }

    #[test]
    fn combat_swaps_record_at_tick_boundaries_never_mid_tick() {
        let mut mode = PracticeMode::new(small_config()).unwrap();
        for tick in 0..8 {
            assert!(mode.tick(idle(tick)).input_accepted);
        }
        // Proposed at cursor 8: effective from tick 8 on, ticks below pin V1.
        assert_eq!(mode.propose_combat_swap(swap_v2()), Ok(2));
        assert_eq!(mode.combat_version_for_tick(7), RULES_VERSION_V1);
        assert_eq!(mode.combat_version_for_tick(8), 2);
        assert_eq!(mode.reload_status().combat_version, 2);
        assert_eq!(mode.reload_status().combat_commands, 1);
        // Stale versions refuse without touching the log.
        assert_eq!(
            mode.propose_combat_swap(swap_v2()),
            Err(ReloadRefusal::StaleCombat {
                current: 2,
                offered: 2,
            })
        );
        assert_eq!(mode.reload_status().combat_commands, 1);
        // Same inputs plus same command log twice give identical checkpoints:
        // the boundary discipline replays exactly.
        let mut replay = PracticeMode::new(small_config()).unwrap();
        for tick in 0..8 {
            assert!(replay.tick(idle(tick)).input_accepted);
        }
        assert_eq!(replay.propose_combat_swap(swap_v2()), Ok(2));
        for tick in 8..16 {
            let first = mode.tick(idle(tick));
            let second = replay.tick(idle(tick));
            assert!(first.input_accepted && second.input_accepted);
            assert_eq!(first.checkpoint, second.checkpoint);
        }
        assert_eq!(mode.checkpoints(), replay.checkpoints());
    }

    #[test]
    fn recorded_combat_rows_provably_change_outcomes() {
        // The recorded combat snapshot is not a stub: stepping real combat under
        // the recorded rows changes the landed damage versus V1.
        let attack = |fighter: u8, buttons: Buttons| FighterInput {
            tick: 0,
            fighter: FighterId::new(fighter).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons,
        };
        let none = |fighter: u8| attack(fighter, Buttons::NONE);
        let bout = |tables: &engine_combat::Tables| {
            let mut state = engine_combat::FightState::new(2).unwrap();
            let mut hits = Vec::new();
            hits.push(engine_combat::step_with_tables(
                &[attack(0, Buttons(ATTACK)), none(1)],
                &mut state,
                tables,
            ));
            for _ in 0..12 {
                hits.push(engine_combat::step_with_tables(
                    &[none(0), none(1)],
                    &mut state,
                    tables,
                ));
            }
            hits.iter().flatten().find_map(|event| match *event {
                engine_combat::FightEvent::Hit { damage, .. } => Some(damage),
                _ => None,
            })
        };
        let recorded = swap_v2();
        let v1_hit = bout(&engine_combat::TABLES_V1).expect("V1 lands the scripted hit");
        let v2_hit = bout(&recorded.tables()).expect("recorded rows land the scripted hit");
        assert_eq!(v2_hit, 240);
        assert_ne!(v1_hit, v2_hit);
    }

    #[test]
    fn combat_rows_file_reload_lands_once_and_replays_identical() {
        const V2_ROW: &str = "v2 attack_damage=120 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";
        const V3_ROW: &str = "v3 attack_damage=240 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";
        const V4_BAD_ROW: &str = "v4 attack_damage=0 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30";
        fn v2_file() -> String {
            format!("table combat version 2\n{V2_ROW}\n")
        }
        fn v3_file() -> String {
            format!("table combat version 3\n{V3_ROW}\n# tuned\n")
        }
        fn bad_file() -> String {
            format!("table combat version 4\n{V4_BAD_ROW}\n# broken\n")
        }
        let dir_a = scratch_dir("combat-rows-a");
        let dir_b = scratch_dir("combat-rows-b");
        let path_a = dir_a.join("combat.rows");
        let path_b = dir_b.join("combat.rows");
        let text_a = path_a.to_string_lossy().into_owned();
        let text_b = path_b.to_string_lossy().into_owned();
        std::fs::write(&path_a, v2_file()).unwrap();
        std::fs::write(&path_b, v2_file()).unwrap();
        let mut mode_a = PracticeMode::new(small_config()).unwrap();
        let mut mode_b = PracticeMode::new(small_config()).unwrap();
        mode_a.watch_combat_rows(&text_a);
        mode_b.watch_combat_rows(&text_b);
        for tick in 0..4 {
            assert!(mode_a.tick(idle(tick)).input_accepted);
            assert!(mode_b.tick(idle(tick)).input_accepted);
        }
        assert_eq!(mode_a.reload_status().combat_version, RULES_VERSION_V1);
        assert_eq!(mode_a.reload_status().combat_commands, 0);
        // Edit the file: the swap lands once at the next tick boundary.
        std::fs::write(&path_a, v3_file()).unwrap();
        std::fs::write(&path_b, v3_file()).unwrap();
        assert!(mode_a.tick(idle(4)).input_accepted);
        assert!(mode_b.tick(idle(4)).input_accepted);
        assert_eq!(mode_a.reload_status().combat_version, 3);
        assert_eq!(mode_b.reload_status().combat_version, 3);
        assert_eq!(mode_a.reload_status().combat_commands, 1);
        assert_eq!(mode_b.reload_status().combat_commands, 1);
        // No edit: nothing more lands.
        assert!(mode_a.tick(idle(5)).input_accepted);
        assert!(mode_b.tick(idle(5)).input_accepted);
        assert_eq!(mode_a.reload_status().combat_commands, 1);
        // Invalid file: refused, last-good stays with its reason.
        std::fs::write(&path_a, bad_file()).unwrap();
        assert!(mode_a.tick(idle(6)).input_accepted);
        assert!(mode_b.tick(idle(6)).input_accepted);
        assert_eq!(mode_a.reload_status().combat_version, 3);
        assert_eq!(mode_a.reload_status().combat_commands, 1);
        assert!(
            mode_a
                .reload_status()
                .last_reason
                .as_ref()
                .is_some_and(|reason| reason.contains("combat.rows")),
            "bad rows keep last-good with a reason"
        );
        // Stale file (older version): refused, last-good stays.
        std::fs::write(&path_a, v2_file()).unwrap();
        assert!(mode_a.tick(idle(7)).input_accepted);
        assert!(mode_b.tick(idle(7)).input_accepted);
        assert_eq!(mode_a.reload_status().combat_version, 3);
        assert_eq!(mode_a.reload_status().combat_commands, 1);
        // Same inputs plus same file swap twice give identical checkpoints.
        for tick in 8..16 {
            assert!(mode_a.tick(idle(tick)).input_accepted);
            assert!(mode_b.tick(idle(tick)).input_accepted);
        }
        assert_eq!(mode_a.checkpoints(), mode_b.checkpoints());
        std::fs::remove_dir_all(&dir_a).unwrap();
        std::fs::remove_dir_all(&dir_b).unwrap();
    }

    #[test]
    fn ai_swaps_validate_and_refuse_with_typed_reasons() {
        let mut mode = PracticeMode::new(small_config()).unwrap();
        assert_eq!(mode.ai_version_for_tick(0), AI_VERSION_V1);
        assert_eq!(mode.propose_ai_swap(ai_v2()), Ok(2));
        assert_eq!(mode.reload_status().ai_version, 2);
        assert_eq!(mode.reload_status().ai_commands, 1);
        // Stale version refuses.
        assert_eq!(
            mode.propose_ai_swap(ai_v2()),
            Err(ReloadRefusal::StaleAi {
                current: 2,
                offered: 2,
            })
        );
        // Invalid snapshots refuse with their cause and keep last-good.
        let mut zero = ai_v2();
        zero.version = 0;
        assert_eq!(
            mode.propose_ai_swap(zero),
            Err(ReloadRefusal::InvalidAi(AiRulesError::ZeroVersion))
        );
        let mut dipped = ai_v2();
        dipped.aggression[4] = dipped.aggression[3] - 1;
        assert!(matches!(
            mode.propose_ai_swap(dipped),
            Err(ReloadRefusal::InvalidAi(
                AiRulesError::AggressionNotMonotonic { .. }
            ))
        ));
        let mut far = ai_v2();
        far.engage_range_cm = AI_ENGAGE_MAX_CM + 1;
        assert!(matches!(
            mode.propose_ai_swap(far),
            Err(ReloadRefusal::InvalidAi(
                AiRulesError::EngageRangeOutOfRange { .. }
            ))
        ));
        let report = mode.reload_status();
        assert_eq!((report.ai_version, report.ai_commands), (2, 1));
        assert!(
            report
                .last_reason
                .as_ref()
                .is_some_and(|reason| !reason.is_empty()),
            "invalid proposals record their reason: {report:?}"
        );
        assert!(AiRules::scripted_defaults().is_valid());
        assert_eq!(AiRules::default(), AiRules::scripted_defaults());
    }

    #[test]
    fn old_tapes_replay_identical_after_swaps_wired() {
        // Baseline session with scripted inputs.
        let script = |tick: u32| FighterInput {
            tick,
            fighter: FighterId::new(HUMAN).unwrap(),
            mx: if tick % 20 < 10 { 100 } else { -100 },
            mz: 0,
            facing: 0,
            buttons: if tick.is_multiple_of(30) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        };
        let mut baseline = App::new(small_config()).unwrap();
        for tick in 0..small_config().ticks {
            assert!(baseline.tick(script(tick)).input_accepted);
        }
        // The same inputs through practice mode with no commands attached
        // replay byte-identical: wiring changed nothing for old tapes.
        let mut wired = PracticeMode::new(small_config()).unwrap();
        for tick in 0..small_config().ticks {
            assert!(wired.tick(script(tick)).input_accepted);
        }
        assert_eq!(wired.checkpoints(), baseline.checkpoints());
        // And a fresh practice session after another session recorded swaps
        // still replays the old tape identically when it carries no commands.
        let mut other = PracticeMode::new(small_config()).unwrap();
        assert!(other.tick(idle(0)).input_accepted);
        assert_eq!(other.propose_combat_swap(swap_v2()), Ok(2));
        assert_eq!(other.propose_ai_swap(ai_v2()), Ok(2));
        let mut fresh = PracticeMode::new(small_config()).unwrap();
        for tick in 0..small_config().ticks {
            assert!(fresh.tick(script(tick)).input_accepted);
        }
        assert_eq!(fresh.checkpoints(), baseline.checkpoints());
        assert_eq!(fresh.reload_status().combat_version, RULES_VERSION_V1);
        assert_eq!(fresh.reload_status().ai_version, AI_VERSION_V1);
    }

    #[test]
    fn competitive_matches_refuse_live_swaps_typed() {
        let script = |tick: u32| idle(tick);
        let mut pinned = CompetitiveMatch::new(small_config()).unwrap();
        let mut open = App::new(small_config()).unwrap();
        assert_eq!(
            pinned.propose_combat_swap(swap_v2()),
            Err(ReloadRefusal::CompetitivePinned {
                what: "combat tables",
            })
        );
        assert_eq!(
            pinned.propose_ai_swap(ai_v2()),
            Err(ReloadRefusal::CompetitivePinned { what: "ai curves" })
        );
        assert_eq!(
            pinned.notify(AssetHandle::from_bytes([3u8; 16]), 1, SHADER_V1.as_bytes()),
            Err(ReloadRefusal::CompetitivePinned {
                what: "presentation",
            })
        );
        // Refused swaps change nothing: the pinned match tracks the plain
        // session tick for tick.
        for tick in 0..small_config().ticks {
            let left = pinned.tick(script(tick));
            let right = open.tick(script(tick));
            assert!(left.input_accepted && right.input_accepted);
            assert_eq!(left.checkpoint, right.checkpoint);
        }
        assert_eq!(pinned.checkpoints(), open.checkpoints());
        assert_eq!(
            ReloadRefusal::CompetitivePinned {
                what: "combat tables",
            }
            .to_string(),
            "competitive match pins its combat tables: live swap refused"
        );
    }

    #[test]
    fn watch_rewatch_keeps_stable_handles_and_unwatch_releases() {
        let dir = scratch_dir("handles");
        let path = dir.join("tune.wgsl");
        let text = path.to_string_lossy().into_owned();
        std::fs::write(&path, SHADER_V1).unwrap();
        let mut mode = PracticeMode::new(small_config()).unwrap();
        let first = mode.watch(&text, PresentKind::Shader);
        assert!(!first.is_nil());
        // Rewatching the same path keeps the stable id and rebinds the kind.
        assert_eq!(mode.watch(&text, PresentKind::Shader), first);
        assert_eq!(
            AssetHandle::parse_hex(&first.to_string()).unwrap(),
            first,
            "handles round-trip through text"
        );
        assert!(AssetHandle::parse_hex("not-hex").is_err());
        assert!(mode.unwatch(first));
        assert!(!mode.unwatch(first));
        // After release the id is unknown again.
        assert_eq!(
            mode.notify(first, 1, SHADER_V1.as_bytes()),
            PresentOutcome::UnknownAsset
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "timing: wall-clock 1 s visibility of a small shader edit (HR5); run explicitly once"]
    fn small_fixture_edit_visible_within_1s() {
        use std::time::Instant;

        let dir = scratch_dir("latency");
        let path = dir.join("reload.wgsl");
        let text = path.to_string_lossy().into_owned();
        std::fs::write(&path, SHADER_V1).unwrap();
        let mut mode = PracticeMode::new(small_config()).unwrap();
        let handle = mode.watch(&text, PresentKind::Shader);
        // Baseline cooks nothing (the watcher's silent-baseline rule).
        assert!(mode.reload_status().presentation.is_empty());
        let started = Instant::now();
        // Different length so the change is visible even on coarse mtimes.
        std::fs::write(&path, SHADER_V2).unwrap();
        let mut seen = None;
        for _ in 0..small_config().ticks {
            let _ = mode.tick(idle(0));
            if mode
                .reload_status()
                .presentation
                .iter()
                .any(|entry| entry.asset == handle && entry.version == 1)
            {
                seen = Some(started.elapsed());
                break;
            }
            if started.elapsed().as_secs() >= 1 {
                break;
            }
        }
        let elapsed = seen.expect("small-fixture edit visible within 1 s");
        assert!(
            elapsed.as_secs() < 1,
            "edit took too long to surface: {elapsed:?}"
        );
        assert_eq!(mode.shader_source(), Some(SHADER_V2));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
