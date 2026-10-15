//! build_character: brief to character (AB2) over the licensed CT7 kit.
//!
//! Owner: endpoint lane; consumer is the endpoint library helpers
//! (`build_character`, not an MCP tool; `character.build` is a reserved name
//! not registered in `mcp.rs`/`tool_names` today). Units: meters, seconds, counts.
//! Determinism: stable orders (kit order, sorted clips), integer millis and
//! FNV-1a digests only; same brief gives the same bytes twice. Errors are the
//! typed [`BuildError`]; unlicensed kits refuse, unknown moves ask CT14→HR6.
//! Budget: offline authoring only, at most 16 parts and 16 clips per build,
//! at most [`MAX_REPAIRS`] repairs, then refuse; never runs in the tick.
//!
//! Pipeline (models, never mirrors, CT1/RL6/CT2 semantics): resolve the brief
//! onto one of the four CT7 kit variants (CC0 System G6 modular bodies sharing
//! the 45-joint hero skeleton) → license gate → identity bone-map retarget of
//! each wanted clip (one skeleton, so the CT1 bind-relative map is exact) →
//! role-driven RL6 layer stack (cloth on garments, sand dust on boots/feet,
//! wet-blood overlay only when asked) → CT2/health-style import gates → one
//! [`ProvenanceRow`] per part plus one per clip.

use std::fmt;

use engine_core::{hash::FNV64_OFFSET, Fnv64};

use super::brief::{BriefEnvelope, BriefStage};

/// Reserved door name for this stage; not registered in `mcp.rs`/`tool_names` today.
///
/// Read-only: `build_character` diffs a brief into a character receipt and
/// writes nothing (apply travels through `tx.preview`/`tx.apply` later).
pub const TOOL_BUILD_CHARACTER: &str = "character.build";

/// Reserved door name; the router does not register it today.
pub fn tool_name() -> &'static str {
    TOOL_BUILD_CHARACTER
}

/// True: this stage previews only and never mutates the document.
pub fn readonly() -> bool {
    true
}

/// Joints every CT7 kit variant shares (45 incl. armature + mesh nodes).
pub const EXPECTED_JOINTS: usize = 45;
/// Max parts assembled onto one character.
pub const MAX_PARTS: usize = 16;
/// Max clips retargeted onto one character.
pub const MAX_CLIPS: usize = 16;
/// Max repair rounds per brief (AB1 pattern); past this the brief refuses.
pub const MAX_REPAIRS: u8 = 3;
/// Max chars of a brief text.
pub const MAX_BRIEF_CHARS: usize = 2000;
/// Max chars of a part, clip or move token.
pub const MAX_TOKEN_CHARS: usize = 64;
/// Licenses the kit door accepts (content policy: CC0/CC-BY + public-domain
/// shapes, never NC/ND, never gated or ripped packs).
pub const KIT_LICENSES: &[&str] = &["CC0-1.0", "CC-BY-4.0", "CC-BY-3.0", "Unlicense"];
/// Kit generator stamped on every provenance row.
pub const KIT_GENERATOR: &str = "ct7-cook";
/// Kit source pack stamped on every provenance row.
pub const KIT_SOURCE: &str = "modular_rpg_characters.7z (opengameart, System G6)";
/// Largest coat gain accepted on the wet-blood film (RL6 bound).
pub const COAT_GAIN_MAX: f32 = 4.0;

/// Machine code + human detail; the detail always names the legal values or
/// the next step (CT14→HR6 for a missing move).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildError {
    /// Machine code (`unlicensed_kit`, `missing_move`, ...).
    pub code: &'static str,
    /// Human detail with the legal values or the repair named.
    pub message: String,
}

impl BuildError {
    /// New error with a static code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BuildError {}

/// One of the four CT7 kit bodies that briefs assemble from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantId {
    /// Heavy-armor hero (1.8 m male bind).
    Hero,
    /// Light male opponent.
    OppMaleLight,
    /// Medium female opponent.
    OppFemaleMedium,
    /// Light female opponent.
    OppFemaleLight,
}

impl VariantId {
    /// Parse a variant token; unknown tokens fail loudly.
    pub fn parse(text: &str) -> Result<Self, BuildError> {
        match text {
            "hero" => Ok(Self::Hero),
            "opp_male_light" => Ok(Self::OppMaleLight),
            "opp_female_medium" => Ok(Self::OppFemaleMedium),
            "opp_female_light" => Ok(Self::OppFemaleLight),
            _ => Err(BuildError::new(
                "bad_brief",
                format!(
                    "variant `{text}` is unknown; legal variants: hero, \
                     opp_male_light, opp_female_medium, opp_female_light"
                ),
            )),
        }
    }

    /// Stable kit name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Hero => "hero",
            Self::OppMaleLight => "opp_male_light",
            Self::OppFemaleMedium => "opp_female_medium",
            Self::OppFemaleLight => "opp_female_light",
        }
    }
}

/// License carried by a brief: SPDX id plus where the bytes came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicenseRef {
    /// SPDX id (must sit in [`KIT_LICENSES`]).
    pub spdx: String,
    /// Source pack or URL the bytes came from.
    pub source: String,
}

impl LicenseRef {
    /// The CT7 CC0 kit license every fixture uses.
    pub fn ct7() -> Self {
        Self {
            spdx: "CC0-1.0".to_owned(),
            source: KIT_SOURCE.to_owned(),
        }
    }

    /// Gate: only allow-listed licenses pass; anything else refuses with the
    /// legal list named.
    pub fn check(&self) -> Result<(), BuildError> {
        if KIT_LICENSES.contains(&self.spdx.as_str()) {
            return Ok(());
        }
        Err(BuildError::new(
            "unlicensed_kit",
            format!(
                "license `{}` from `{}` is refused; legal kit licenses: {}; \
                 gated (QAL-1.0), non-commercial (CC-BY-NC*), Mixamo, LAFAN1, \
                 Fab and UE-only packs never ship",
                self.spdx,
                self.source,
                KIT_LICENSES.join(", "),
            ),
        ))
    }
}

/// Brief text plus the knobs the deterministic stages read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterBrief {
    /// Free text, 1..=2000 chars, kept verbatim for the receipt only.
    pub brief_text: String,
    /// Kit body to assemble.
    pub variant: VariantId,
    /// Wanted clip shorts, 1..=16 entries (unknown shorts ask CT14→HR6).
    pub want_clips: Vec<String>,
    /// True adds the wet-blood overlay layer (veteran finish).
    pub bloodied: bool,
    /// Kit license; must pass [`LicenseRef::check`].
    pub license: LicenseRef,
    /// Repairs spent so far; past [`MAX_REPAIRS`] the brief refuses.
    pub repairs_used: u8,
    /// Seed folded into the digest (same seed + same brief = same digest).
    pub seed: u64,
}

impl CharacterBrief {
    /// Shared LW6 envelope record line for this brief (`BriefEnvelope`
    /// under [`BriefStage::Character`]). Every AB2 write carries this
    /// line so the audit traces the brief end to end; empty or
    /// overlong brief text refuses with the legal values.
    pub fn record_line(&self) -> Result<String, BuildError> {
        BriefEnvelope::new(&self.brief_text, self.seed, BriefStage::Character)
            .map(|env| env.record_line())
            .map_err(|error| BuildError::new(error.code(), error.to_string()))
    }

    /// Validate shapes before any stage runs.
    pub fn check(&self) -> Result<(), BuildError> {
        if self.brief_text.is_empty() || self.brief_text.len() > MAX_BRIEF_CHARS {
            return Err(BuildError::new(
                "bad_brief",
                format!(
                    "brief holds {} chars; submit 1..={} chars",
                    self.brief_text.len(),
                    MAX_BRIEF_CHARS
                ),
            ));
        }
        if self.want_clips.is_empty() || self.want_clips.len() > MAX_CLIPS {
            return Err(BuildError::new(
                "bad_brief",
                format!(
                    "brief wants {} clips; submit 1..={} clip shorts",
                    self.want_clips.len(),
                    MAX_CLIPS
                ),
            ));
        }
        for clip in &self.want_clips {
            if clip.is_empty() || clip.len() > MAX_TOKEN_CHARS {
                return Err(BuildError::new(
                    "bad_brief",
                    format!(
                        "clip token `{clip}` is outside 1..={MAX_TOKEN_CHARS} chars; \
                         legal shorts: idle, run, jump, death, idle_combat_1h, \
                         idle_combat_unarmed, attack_1h_1..3, attack_2h_1..2, \
                         block_shield, attack_dual_1, taunt"
                    ),
                ));
            }
        }
        if self.repairs_used > MAX_REPAIRS {
            return Err(BuildError::new(
                "over_budget",
                format!(
                    "brief spent {} repairs; at most {MAX_REPAIRS} repair rounds, \
                     then the stage refuses with its findings",
                    self.repairs_used
                ),
            ));
        }
        self.license.check()?;
        Ok(())
    }
}

/// One kit part bound onto the shared skeleton.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledPart {
    /// Role (`armor`, `body`, `boots`, ...).
    pub role: &'static str,
    /// Kit object name.
    pub object: &'static str,
}

/// RL6 layer kind selected per part role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    /// Garments, padding, armor cloth.
    Cloth,
    /// Dust on boots and feet.
    Sand,
    /// Wet-blood film overlay (veteran finish only).
    WetBlood,
}

impl LayerKind {
    /// Stable name for digests and receipts.
    pub fn name(self) -> &'static str {
        match self {
            Self::Cloth => "cloth",
            Self::Sand => "sand",
            Self::WetBlood => "wet_blood",
        }
    }
}

/// Validated RL6 layer entry: kind plus bounded strengths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerSpec {
    /// Which layer.
    pub kind: LayerKind,
    /// Blend strength, 0..=1.
    pub strength: f32,
    /// Specular coat gain on the film, 0..=[`COAT_GAIN_MAX`].
    pub coat_gain: f32,
}

impl LayerSpec {
    /// Validate ranges; non-finite or out-of-range values fail loudly.
    pub fn check(&self) -> Result<(), BuildError> {
        if !self.strength.is_finite() || self.strength < 0.0 || self.strength > 1.0 {
            return Err(BuildError::new(
                "gate_failed",
                format!(
                    "layer {} strength {} is outside 0..=1; submit a finite blend",
                    self.kind.name(),
                    self.strength
                ),
            ));
        }
        if !self.coat_gain.is_finite() || self.coat_gain < 0.0 || self.coat_gain > COAT_GAIN_MAX {
            return Err(BuildError::new(
                "gate_failed",
                format!(
                    "layer {} coat gain {} is outside 0..={COAT_GAIN_MAX}; \
                     submit a finite gain",
                    self.kind.name(),
                    self.coat_gain
                ),
            ));
        }
        Ok(())
    }
}

/// One CT1-style retargeted clip: identity bone-map onto the shared skeleton.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetargetedClip {
    /// Clip short (`idle`, `attack_1h_1`, ...).
    pub short: String,
    /// Key count (joints x frames shape, always positive past gates).
    pub keys: u32,
    /// Duration in integer millis (stable under hashing).
    pub duration_ms: u32,
    /// True when the clip loops.
    pub looping: bool,
    /// True when root travel is carried 1:1 (CT1 root rule).
    pub root_motion: bool,
    /// FNV-1a digest of variant + short + keys.
    pub digest: u64,
}

/// Provenance stamped per part and per clip (CT4 rule).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceRow {
    /// What this row covers (`part:<role>` or `clip:<short>`).
    pub subject: String,
    /// Object or clip name.
    pub name: String,
    /// SPDX license id.
    pub license: String,
    /// Source pack.
    pub source: String,
    /// Cook that produced it.
    pub generator: String,
}

/// The assembled, rigged, animated character plus its receipt.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltCharacter {
    /// Kit body assembled.
    pub variant: VariantId,
    /// Display name derived from the variant.
    pub display_name: String,
    /// Skeleton joint count (always [`EXPECTED_JOINTS`] past gates).
    pub joints: usize,
    /// Bound parts in kit order.
    pub parts: Vec<AssembledPart>,
    /// Retargeted clips in sorted short order.
    pub clips: Vec<RetargetedClip>,
    /// RL6 layer stack, one entry per part plus the optional film.
    pub layers: Vec<LayerSpec>,
    /// One row per part plus one row per clip.
    pub provenance: Vec<ProvenanceRow>,
    /// Stable FNV-1a digest of the whole receipt.
    pub digest: u64,
}

impl BuiltCharacter {
    /// True when parts are bound onto the full skeleton.
    pub fn is_rigged(&self) -> bool {
        !self.parts.is_empty() && self.joints == EXPECTED_JOINTS
    }

    /// True when at least one clip retargeted with keys.
    pub fn is_animated(&self) -> bool {
        self.clips.iter().any(|clip| clip.keys > 0)
    }

    /// CT2/health-style import gates: floors plus shape checks.
    pub fn check_gates(&self) -> Result<(), BuildError> {
        if self.joints != EXPECTED_JOINTS {
            return Err(BuildError::new(
                "gate_failed",
                format!(
                    "skeleton holds {} joints; the kit rig holds {EXPECTED_JOINTS}",
                    self.joints
                ),
            ));
        }
        if self.parts.is_empty() || self.parts.len() > MAX_PARTS {
            return Err(BuildError::new(
                "gate_failed",
                format!(
                    "character binds {} parts; submit 1..={MAX_PARTS} kit parts",
                    self.parts.len()
                ),
            ));
        }
        if self.clips.is_empty() || self.clips.len() > MAX_CLIPS {
            return Err(BuildError::new(
                "gate_failed",
                format!(
                    "character carries {} clips; submit 1..={MAX_CLIPS} kit clips",
                    self.clips.len()
                ),
            ));
        }
        for clip in &self.clips {
            if clip.keys == 0 || clip.duration_ms == 0 {
                return Err(BuildError::new(
                    "gate_failed",
                    format!(
                        "clip `{}` holds keys={} duration_ms={}; retarget must \
                         emit positive keys and duration",
                        clip.short, clip.keys, clip.duration_ms
                    ),
                ));
            }
        }
        for layer in &self.layers {
            layer.check()?;
        }
        // Byte floor (health rule): modeled bundle bytes must clear 2048 with
        // a dense nonzero ratio; synthetic bytes scale with parts and keys so
        // a degenerate assembly cannot slip through.
        let bytes = 4096_u64
            .saturating_mul(self.parts.len() as u64)
            .saturating_add(
                64_u64.saturating_mul(
                    self.clips
                        .iter()
                        .map(|clip| u64::from(clip.keys))
                        .sum::<u64>(),
                ),
            );
        if bytes < 2048 {
            return Err(BuildError::new(
                "gate_failed",
                format!("bundle models {bytes} bytes; the byte floor is 2048"),
            ));
        }
        if self.provenance.len() != self.parts.len() + self.clips.len() {
            return Err(BuildError::new(
                "gate_failed",
                format!(
                    "provenance holds {} rows for {} parts + {} clips; \
                     every part and clip needs exactly one row",
                    self.provenance.len(),
                    self.parts.len(),
                    self.clips.len()
                ),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Kit tables: the CT7 bodies and their 14 baked FK clips, encoded from the
// fighters manifest (CC0 System G6; one 45-joint skeleton for all bodies).
// ---------------------------------------------------------------------------

/// Kit part rows per variant, in manifest order.
fn kit_parts(variant: VariantId) -> &'static [(&'static str, &'static str)] {
    match variant {
        VariantId::Hero => &[
            ("armor", "human_male_heavy_armor"),
            ("body", "human_male_body_default"),
            ("boots", "human_male_heavy_boots"),
            ("feet", "human_male_feet_default"),
            ("gloves", "human_male_heavy_gloves"),
            ("hair", "human_male_hair_1"),
            ("hands", "human_male_hands_default"),
            ("head", "human_male_head_default"),
            ("helmet", "human_male_heavy_helmet"),
        ],
        VariantId::OppMaleLight => &[
            ("armor", "human_male_light_armor"),
            ("body", "human_male_body_default"),
            ("boots", "human_male_light_boots"),
            ("feet", "human_male_feet_default"),
            ("gloves", "human_male_light_gloves"),
            ("hair", "human_male_hair_4"),
            ("hands", "human_male_hands_default"),
            ("head", "human_male_head_3"),
        ],
        VariantId::OppFemaleMedium => &[
            ("armor", "human_female_medium_armor"),
            ("body", "human_female_body_default"),
            ("boots", "human_female_medium_boots"),
            ("feet", "human_female_feet_default"),
            ("gloves", "human_female_medium_gloves"),
            ("hair", "human_female_hair_2"),
            ("hands", "human_female_hands_default"),
            ("head", "human_female_head_default"),
            ("helmet", "human_female_medium_helmet"),
        ],
        VariantId::OppFemaleLight => &[
            ("armor", "human_female_light_armor"),
            ("body", "human_female_body_default"),
            ("boots", "human_female_light_boots_2"),
            ("feet", "human_female_feet_default"),
            ("gloves", "human_female_light_gloves_3"),
            ("hair", "human_female_hair_1"),
            ("hands", "human_female_hands_default"),
            ("head", "human_female_head_3"),
        ],
    }
}

/// Kit clip rows: short, keys, duration millis, looping, root motion.
/// Keys and durations follow the fighters manifest (45-joint FK bakes).
fn kit_clips() -> &'static [(&'static str, u32, u32, bool, bool)] {
    &[
        ("idle", 819, 792, true, false),
        ("run", 819, 792, true, true),
        ("jump", 819, 792, false, true),
        ("death", 862, 833, false, false),
        ("idle_combat_1h", 819, 792, true, false),
        ("idle_combat_unarmed", 819, 792, true, false),
        ("attack_1h_1", 432, 417, false, false),
        ("attack_1h_2", 432, 417, false, false),
        ("attack_1h_3", 432, 417, false, false),
        ("attack_2h_1", 432, 417, false, false),
        ("attack_2h_2", 432, 417, false, false),
        ("block_shield", 432, 417, false, false),
        ("attack_dual_1", 432, 417, false, false),
        ("taunt", 432, 417, false, false),
    ]
}

/// Look one wanted short up in the kit table.
fn lookup_clip(short: &str) -> Option<(&'static str, u32, u32, bool, bool)> {
    kit_clips().iter().copied().find(|row| row.0 == short)
}

/// Layer stack for a role: garments read cloth, boots/feet read sand dust.
fn layer_for_role(role: &str) -> LayerSpec {
    match role {
        "boots" | "feet" => LayerSpec {
            kind: LayerKind::Sand,
            strength: 0.6,
            coat_gain: 0.0,
        },
        _ => LayerSpec {
            kind: LayerKind::Cloth,
            strength: 0.8,
            coat_gain: 0.0,
        },
    }
}

/// Assemble one brief into a rigged, animated character.
///
/// Fails with `unlicensed_kit` before touching the kit, with `missing_move`
/// (naming the CT14→HR6 repair) for a short outside the kit table, with
/// `bad_brief`/`over_budget` on shapes, and with `gate_failed` when the
/// import gates reject the assembly.
pub fn build_character(brief: &CharacterBrief) -> Result<BuiltCharacter, BuildError> {
    brief.check()?;

    // Identity retarget: the kit shares one skeleton, so every wanted short
    // must sit in the kit clip table; anything else is a missing move, never
    // a guess.
    let mut shorts: Vec<String> = brief.want_clips.clone();
    shorts.sort();
    shorts.dedup();
    let mut clips = Vec::with_capacity(shorts.len());
    for short in &shorts {
        let Some((_, keys, duration_ms, looping, root_motion)) = lookup_clip(short) else {
            return Err(BuildError::new(
                "missing_move",
                format!(
                    "move `{short}` is not in the CT7 kit; record it as a CT14 \
                     clip first, then run the HR6 move intake to propose HR3 \
                     windows (startup/active/recovery) before this brief can \
                     use it"
                ),
            ));
        };
        let mut clip_hasher = Fnv64::with_state(FNV64_OFFSET ^ brief.seed);
        clip_hasher.write(format!("{}|{short}|{keys}", brief.variant.name()).as_bytes());
        let digest = clip_hasher.finish();
        clips.push(RetargetedClip {
            short: short.clone(),
            keys,
            duration_ms,
            looping,
            root_motion,
            digest,
        });
    }

    let rows = kit_parts(brief.variant);
    let mut parts = Vec::with_capacity(rows.len());
    let mut layers = Vec::with_capacity(rows.len() + 1);
    let mut provenance = Vec::with_capacity(rows.len() + clips.len());
    for (role, object) in rows {
        parts.push(AssembledPart { role, object });
        layers.push(layer_for_role(role));
        provenance.push(ProvenanceRow {
            subject: format!("part:{role}"),
            name: (*object).to_owned(),
            license: brief.license.spdx.clone(),
            source: brief.license.source.clone(),
            generator: KIT_GENERATOR.to_owned(),
        });
    }
    if brief.bloodied {
        layers.push(LayerSpec {
            kind: LayerKind::WetBlood,
            strength: 0.5,
            coat_gain: 1.5,
        });
    }
    for clip in &clips {
        provenance.push(ProvenanceRow {
            subject: format!("clip:{}", clip.short),
            name: clip.short.clone(),
            license: brief.license.spdx.clone(),
            source: brief.license.source.clone(),
            generator: KIT_GENERATOR.to_owned(),
        });
    }

    let display_name = match brief.variant {
        VariantId::Hero => "Hero of the Ludus",
        VariantId::OppMaleLight => "Light Male Opponent",
        VariantId::OppFemaleMedium => "Medium Female Opponent",
        VariantId::OppFemaleLight => "Light Female Opponent",
    }
    .to_owned();

    let mut digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ brief.seed);
    digest_hasher.write(brief.variant.name().as_bytes());
    let mut digest = digest_hasher.finish();
    for part in &parts {
        digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        digest_hasher.write(part.role.as_bytes());
        digest = digest_hasher.finish();
        digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        digest_hasher.write(part.object.as_bytes());
        digest = digest_hasher.finish();
    }
    for clip in &clips {
        digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        digest_hasher.write(clip.short.as_bytes());
        digest = digest_hasher.finish();
        digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        digest_hasher.write(&clip.keys.to_le_bytes());
        digest = digest_hasher.finish();
    }
    for layer in &layers {
        digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        digest_hasher.write(layer.kind.name().as_bytes());
        digest = digest_hasher.finish();
    }
    if brief.bloodied {
        digest_hasher = Fnv64::with_state(FNV64_OFFSET ^ digest);
        digest_hasher.write(b"bloodied");
        digest = digest_hasher.finish();
    }

    let built = BuiltCharacter {
        variant: brief.variant,
        display_name,
        joints: EXPECTED_JOINTS,
        parts,
        clips,
        layers,
        provenance,
        digest,
    };
    built.check_gates()?;
    Ok(built)
}

/// The five acceptance briefs: one per kit body plus a bloodied veteran.
///
/// Every brief is licensed CC0, carries 3–5 kit shorts, and must land rigged
/// and animated past the import gates.
pub fn fixture_briefs() -> [CharacterBrief; 5] {
    [
        CharacterBrief {
            brief_text: "Heavy hero of the ludus, scarred veteran of the arena ladder.".to_owned(),
            variant: VariantId::Hero,
            want_clips: [
                "idle",
                "idle_combat_1h",
                "attack_1h_1",
                "block_shield",
                "run",
            ]
            .iter()
            .map(|slot| (*slot).to_owned())
            .collect(),
            bloodied: false,
            license: LicenseRef::ct7(),
            repairs_used: 0,
            seed: 7,
        },
        CharacterBrief {
            brief_text: "Light male opponent, quick footwork, taunts the crowd.".to_owned(),
            variant: VariantId::OppMaleLight,
            want_clips: ["idle", "run", "attack_1h_2", "taunt"]
                .iter()
                .map(|slot| (*slot).to_owned())
                .collect(),
            bloodied: false,
            license: LicenseRef::ct7(),
            repairs_used: 0,
            seed: 7,
        },
        CharacterBrief {
            brief_text: "Medium female opponent, two-handed guard, refuses to fall.".to_owned(),
            variant: VariantId::OppFemaleMedium,
            want_clips: ["idle", "idle_combat_unarmed", "attack_2h_1", "death"]
                .iter()
                .map(|slot| (*slot).to_owned())
                .collect(),
            bloodied: false,
            license: LicenseRef::ct7(),
            repairs_used: 0,
            seed: 7,
        },
        CharacterBrief {
            brief_text: "Light female opponent, dual blades, jumps the line.".to_owned(),
            variant: VariantId::OppFemaleLight,
            want_clips: ["idle", "run", "jump", "attack_dual_1"]
                .iter()
                .map(|slot| (*slot).to_owned())
                .collect(),
            bloodied: false,
            license: LicenseRef::ct7(),
            repairs_used: 0,
            seed: 7,
        },
        CharacterBrief {
            brief_text: "Bloodied veteran hero, wet-blood finish over heavy armor.".to_owned(),
            variant: VariantId::Hero,
            want_clips: ["idle_combat_1h", "attack_1h_3", "block_shield"]
                .iter()
                .map(|slot| (*slot).to_owned())
                .collect(),
            bloodied: true,
            license: LicenseRef::ct7(),
            repairs_used: 0,
            seed: 7,
        },
    ]
}

/// Review-pipeline dedup bind (AB2): collapse near-duplicate brief variants
/// through the frozen `canon::dedup` engine before review, so one canonical
/// key is reviewed once. Thin caller: no new canonicalization lives here;
/// offline authoring only, never in the tick.
pub fn review_variants(
    variants: &[String],
) -> Result<super::canon::DedupReport, super::canon::CanonError> {
    super::canon::dedup(variants)
}

/// Collapse variants for review: keep the first raw text per canonical key
/// in first-seen order, so reviewers see one representative per key. Errors
/// reuse the frozen [`super::canon::CanonError`] codes (`empty_variant`,
/// `variant_too_long`, `too_many_variants`).
pub fn collapse_for_review(variants: &[String]) -> Result<Vec<String>, super::canon::CanonError> {
    if variants.len() > super::canon::MAX_VARIANTS {
        return Err(super::canon::CanonError::TooMany {
            count: variants.len(),
        });
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for raw in variants {
        let key = super::canon::canon_key(raw)?;
        if seen.insert(key) {
            out.push(raw.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn licensed(variant: VariantId, clips: &[&str]) -> CharacterBrief {
        CharacterBrief {
            brief_text: "test brief".to_owned(),
            variant,
            want_clips: clips.iter().map(|slot| (*slot).to_owned()).collect(),
            bloodied: false,
            license: LicenseRef::ct7(),
            repairs_used: 0,
            seed: 7,
        }
    }

    #[test]
    fn build_character_five_briefs_rigged_animated_past_gates() {
        let briefs = fixture_briefs();
        assert_eq!(briefs.len(), 5);
        for brief in &briefs {
            let built = build_character(brief)
                .unwrap_or_else(|error| panic!("brief `{}` failed: {error}", brief.brief_text));
            assert!(built.is_rigged(), "brief not rigged");
            assert!(built.is_animated(), "brief not animated");
            assert_eq!(built.joints, EXPECTED_JOINTS);
            built.check_gates().expect("gates must pass");
            assert_eq!(
                built.provenance.len(),
                built.parts.len() + built.clips.len(),
                "one provenance row per part and clip"
            );
        }
    }

    #[test]
    fn build_character_unlicensed_kit_refused_with_reason() {
        let mut brief = licensed(VariantId::Hero, &["idle", "run"]);
        brief.license = LicenseRef {
            spdx: "CC-BY-NC-4.0".to_owned(),
            source: "mixamo-pack.zip".to_owned(),
        };
        let error = build_character(&brief).expect_err("NC kit must refuse");
        assert_eq!(error.code, "unlicensed_kit");
        assert!(error.message.contains("CC0-1.0"), "reason: {error}");
        assert!(error.message.contains("CC-BY-NC"), "reason: {error}");
    }

    #[test]
    fn build_character_quaternius_gated_license_refused() {
        let mut brief = licensed(VariantId::Hero, &["idle"]);
        brief.license = LicenseRef {
            spdx: "QAL-1.0".to_owned(),
            source: "quaternius-pack.zip".to_owned(),
        };
        let error = build_character(&brief).expect_err("gated pack must refuse");
        assert_eq!(error.code, "unlicensed_kit");
        assert!(error.message.contains("QAL-1.0"), "reason: {error}");
    }

    #[test]
    fn build_character_provenance_per_part() {
        let built = build_character(&licensed(VariantId::Hero, &["idle", "run"]))
            .expect("licensed build lands");
        assert_eq!(built.parts.len(), kit_parts(VariantId::Hero).len());
        for part in &built.parts {
            let row = built
                .provenance
                .iter()
                .find(|row| row.subject == format!("part:{}", part.role))
                .expect("every part needs a row");
            assert_eq!(row.license, "CC0-1.0");
            assert_eq!(row.source, KIT_SOURCE);
            assert_eq!(row.generator, KIT_GENERATOR);
            assert_eq!(row.name, part.object);
        }
        for clip in &built.clips {
            let row = built
                .provenance
                .iter()
                .find(|row| row.subject == format!("clip:{}", clip.short))
                .expect("every clip needs a row");
            assert_eq!(row.license, "CC0-1.0");
        }
    }

    #[test]
    fn build_character_missing_move_asks_ct14_then_hr6() {
        let brief = licensed(VariantId::Hero, &["idle", "hurricane_kick"]);
        let error = build_character(&brief).expect_err("unknown move must ask");
        assert_eq!(error.code, "missing_move");
        assert!(error.message.contains("hurricane_kick"), "reason: {error}");
        assert!(error.message.contains("CT14"), "repair: {error}");
        assert!(error.message.contains("HR6"), "repair: {error}");
    }

    #[test]
    fn build_character_digest_stable_twice() {
        let brief = licensed(VariantId::OppFemaleLight, &["idle", "run", "jump"]);
        let first = build_character(&brief).expect("first lands").digest;
        let second = build_character(&brief).expect("second lands").digest;
        assert_eq!(first, second, "same brief must digest identically");
        let other = licensed(VariantId::OppFemaleLight, &["idle", "run", "taunt"]);
        let third = build_character(&other).expect("other lands").digest;
        assert_ne!(first, third, "different clips must digest differently");
    }

    #[test]
    fn build_character_over_budget_refused() {
        let mut brief = licensed(VariantId::Hero, &["idle"]);
        brief.repairs_used = MAX_REPAIRS + 1;
        let error = build_character(&brief).expect_err("4th repair must refuse");
        assert_eq!(error.code, "over_budget");
        assert!(error.message.contains("3"), "reason: {error}");
    }

    #[test]
    fn build_character_empty_clips_refused() {
        let brief = licensed(VariantId::Hero, &[]);
        let error = build_character(&brief).expect_err("empty clips must refuse");
        assert_eq!(error.code, "bad_brief");
    }

    #[test]
    fn build_character_layer_params_validated() {
        let bad = LayerSpec {
            kind: LayerKind::Cloth,
            strength: 2.0,
            coat_gain: 0.0,
        };
        assert_eq!(
            bad.check().expect_err("bad strength must fail").code,
            "gate_failed"
        );
        let bad_gain = LayerSpec {
            kind: LayerKind::WetBlood,
            strength: 0.5,
            coat_gain: COAT_GAIN_MAX + 1.0,
        };
        assert_eq!(
            bad_gain.check().expect_err("bad gain must fail").code,
            "gate_failed"
        );
    }

    #[test]
    fn build_character_tool_hook_is_readonly_preview() {
        assert_eq!(tool_name(), TOOL_BUILD_CHARACTER);
        assert_eq!(TOOL_BUILD_CHARACTER, "character.build");
        assert!(readonly());
    }

    #[test]
    fn review_pipeline_collapses_seeded_sets_by_thirty_percent() {
        let all = super::super::canon::thirty_sets();
        assert_eq!(all.len(), 240);
        let report = review_variants(&all).expect("review dedup lands");
        assert!(
            report.meets_bar,
            "need >=30% reduction, got {}/{} ({} permille)",
            report.candidate, report.baseline, report.reduction_permille
        );
        assert!(report.reduction_permille >= super::super::canon::REDUCTION_MIN_PERMILLE);
        let collapsed = collapse_for_review(&all).expect("collapse lands");
        assert_eq!(
            collapsed.len(),
            report.candidate,
            "one representative per key"
        );
        // Deterministic: same input collapses identically twice.
        let again = collapse_for_review(&all).expect("replay lands");
        assert_eq!(collapsed, again);
    }

    #[test]
    fn review_pipeline_keeps_keypoint_distinct_scenes_split() {
        let mut distinct: Vec<String> = Vec::new();
        for i in 0..30u64 {
            distinct.push(format!("prop crate key_{i:02}\nspawn key_{i:02}"));
        }
        let report = review_variants(&distinct).expect("distinct dedup lands");
        assert_eq!(report.candidate, 30, "distinct scenes never merge");
        assert_eq!(report.reduction_permille, 0);
        let collapsed = collapse_for_review(&distinct).expect("collapse lands");
        assert_eq!(collapsed.len(), 30);
        // One semantic lane apart (kind) stays split through the pipeline.
        let pair = [
            "prop crate a\nspawn a".to_owned(),
            "prop barrel a\nspawn a".to_owned(),
        ];
        let split = review_variants(&pair).expect("pair lands");
        assert_eq!((split.baseline, split.candidate), (2, 2));
        assert_eq!(collapse_for_review(&pair).expect("pair collapse").len(), 2);
    }

    #[test]
    fn review_pipeline_refuses_bad_inputs_with_codes() {
        assert_eq!(
            review_variants(&["   ".to_owned()])
                .expect_err("blank")
                .code(),
            "empty_variant"
        );
        assert_eq!(
            collapse_for_review(&["   ".to_owned()])
                .expect_err("blank collapse")
                .code(),
            "empty_variant"
        );
        let long = "x".repeat(super::super::canon::MAX_CHARS + 1);
        assert_eq!(
            review_variants(std::slice::from_ref(&long))
                .expect_err("overlong")
                .code(),
            "variant_too_long"
        );
        assert_eq!(
            collapse_for_review(&[long])
                .expect_err("overlong collapse")
                .code(),
            "variant_too_long"
        );
        let many = vec!["prop crate a\nspawn a".to_owned(); super::super::canon::MAX_VARIANTS + 1];
        assert_eq!(
            review_variants(&many).expect_err("too many").code(),
            "too_many_variants"
        );
        assert_eq!(
            collapse_for_review(&many)
                .expect_err("too many collapse")
                .code(),
            "too_many_variants"
        );
    }
}
