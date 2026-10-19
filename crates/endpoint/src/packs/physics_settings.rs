//! settings pack: the physics and render knobs an AI can set as typed data.
//!
//! Three verbs over one session value. `settings.get` prints the knob table
//! (name, unit, legal range and value), the six material rows and the four
//! quality tiers with their body and ragdoll caps. `settings.set` writes one
//! knob at a boundary tick, and `settings.undo` writes the previous state back
//! at a later tick. Every knob takes an integer code with a range, so a bad
//! number refuses with the range in the reason and changes nothing.
//!
//! Replay: [`SettingsTape`] holds one whole-state record per applied change and
//! [`SettingsTape::settings_at`] folds it from the defaults, so a replay reads
//! the old value on the tick before the record and the new value on the record
//! tick. Undo appends the previous state as a new record: history before the
//! undo tick stays replayable and the tape is never rewritten.
//!
//! The settings are a door session value. They never enter the fixed tick, a
//! simulated tape or the golden hash. The defaults equal the shipped constants
//! (gore full, ground stone, Earth gravity, full render scale, comfort caps);
//! a test reads the owner sources and fails when one of them drifts.
//!
//! DTO structs carry no doc comments on purpose: every doc line inflates the
//! discovery schemas the door budgets.

use super::{bad_args as bad, PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;
use engine_combat::gore::GoreSetting;
use engine_core::Fnv64;
use engine_physics::material::{Material, GROUND};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Pack name.
pub const PACK: &str = "settings";
/// Verb: the knob table, the material rows and the live values.
pub const TOOL_SETTINGS_GET: &str = "settings.get";
/// Verb: write one knob at a boundary tick.
pub const TOOL_SETTINGS_SET: &str = "settings.set";
/// Verb: write the previous state back at a later tick.
pub const TOOL_SETTINGS_UNDO: &str = "settings.undo";

/// Most records the tape holds (applies plus undos).
pub const MAX_TAPE_RECORDS: usize = 256;
/// Smallest render scale in percent, the settings screen's bound.
pub const RENDER_SCALE_MIN: u8 = 50;
/// Largest render scale in percent.
pub const RENDER_SCALE_MAX: u8 = 100;
/// Earth gravity in millimetres per second squared: the shipped flight row.
pub const GRAVITY_MM_S2: u32 = 9_810;
/// Largest accepted gravity in millimetres per second squared: 32 m/s^2.
pub const GRAVITY_MAX_MM_S2: u32 = 32_000;
/// Smallest debris body cap: the weakest tier's dynamic bodies.
pub const DEBRIS_CAP_MIN: u16 = 96;
/// Largest debris body cap: the strongest tier's dynamic bodies.
pub const DEBRIS_CAP_MAX: u16 = 768;
/// Smallest ragdoll cap: the weakest tier's ragdolls.
pub const RAGDOLL_CAP_MIN: u8 = 2;
/// Largest ragdoll cap: the strongest tier's ragdolls.
pub const RAGDOLL_CAP_MAX: u8 = 16;

/// Hardware class a client runs on, weakest first. The numbers mirror the
/// render quality table and the physical-layer cap table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Weak GPU and little RAM.
    Potato,
    /// A browser tab.
    Web,
    /// The comfortable desktop tier.
    Comfort,
    /// Headroom.
    High,
}

/// The caps of one tier that a setting can receive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TierCaps {
    /// Dynamic bodies alive at once.
    pub bodies: u16,
    /// Ragdolls alive at once.
    pub ragdolls: u8,
}

impl Tier {
    /// Every tier, weakest first.
    pub const ALL: [Tier; 4] = [Tier::Potato, Tier::Web, Tier::Comfort, Tier::High];

    /// Lowercase tier name, as the page prints `tier=<name>`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Potato => "potato",
            Self::Web => "web",
            Self::Comfort => "comfort",
            Self::High => "high",
        }
    }

    /// The tier named `word` (any letter case).
    pub fn from_name(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tier| tier.name().eq_ignore_ascii_case(word.trim()))
    }

    /// The body and ragdoll caps of this tier.
    pub const fn caps(self) -> TierCaps {
        match self {
            Self::Potato => TierCaps {
                bodies: DEBRIS_CAP_MIN,
                ragdolls: RAGDOLL_CAP_MIN,
            },
            Self::Web => TierCaps {
                bodies: 192,
                ragdolls: 4,
            },
            Self::Comfort => TierCaps {
                bodies: 384,
                ragdolls: 8,
            },
            Self::High => TierCaps {
                bodies: DEBRIS_CAP_MAX,
                ragdolls: RAGDOLL_CAP_MAX,
            },
        }
    }
}

/// One knob: the word a caller names and the range its integer code must sit in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knob {
    /// Blood policy: 0 off, 1 reduced, 2 full.
    Gore,
    /// Dynamic bodies alive at once.
    DebrisCap,
    /// Ragdolls alive at once.
    RagdollCap,
    /// Downward acceleration in millimetres per second squared.
    Gravity,
    /// Which material row new ground and debris use (wood, stone, flesh, metal, sand, cloth).
    Material,
    /// Quality tier: 0 potato, 1 web, 2 comfort, 3 high.
    Quality,
    /// Render scale in percent.
    RenderScale,
}

impl Knob {
    /// Every knob in the row's order.
    pub const ALL: [Knob; 7] = [
        Knob::Gore,
        Knob::DebrisCap,
        Knob::RagdollCap,
        Knob::Gravity,
        Knob::Material,
        Knob::Quality,
        Knob::RenderScale,
    ];

    /// The word a caller names this knob by.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gore => "gore",
            Self::DebrisCap => "debris_cap",
            Self::RagdollCap => "ragdoll_cap",
            Self::Gravity => "gravity",
            Self::Material => "material",
            Self::Quality => "quality",
            Self::RenderScale => "render_scale",
        }
    }

    /// Unit of the integer code.
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Gore | Self::Material | Self::Quality => "code",
            Self::DebrisCap => "bodies",
            Self::RagdollCap => "ragdolls",
            Self::Gravity => "mm/s^2",
            Self::RenderScale => "%",
        }
    }

    /// Smallest accepted code.
    pub fn min(self) -> i64 {
        match self {
            Self::Gore => 0,
            Self::DebrisCap => i64::from(DEBRIS_CAP_MIN),
            Self::RagdollCap => i64::from(RAGDOLL_CAP_MIN),
            Self::Gravity => 0,
            Self::Material => 0,
            Self::Quality => 0,
            Self::RenderScale => i64::from(RENDER_SCALE_MIN),
        }
    }

    /// Largest accepted code.
    pub fn max(self) -> i64 {
        match self {
            Self::Gore => 2,
            Self::DebrisCap => i64::from(DEBRIS_CAP_MAX),
            Self::RagdollCap => i64::from(RAGDOLL_CAP_MAX),
            Self::Gravity => i64::from(GRAVITY_MAX_MM_S2),
            Self::Material => 5,
            Self::Quality => 3,
            Self::RenderScale => i64::from(RENDER_SCALE_MAX),
        }
    }

    /// The code of this knob's shipped default.
    pub fn default_code(self) -> i64 {
        match self {
            Self::Gore => 2,
            Self::DebrisCap => 384,
            Self::RagdollCap => 8,
            Self::Gravity => i64::from(GRAVITY_MM_S2),
            Self::Material => 1,
            Self::Quality => 2,
            Self::RenderScale => i64::from(RENDER_SCALE_MAX),
        }
    }

    /// The knob named `word`.
    pub fn from_name(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|knob| knob.name().eq_ignore_ascii_case(word.trim()))
    }

    /// Why `code` is outside this knob's range, if it is.
    ///
    /// # Errors
    /// [`SettingsError::OutOfRange`] carrying the knob, the code and its range.
    pub fn check(self, code: i64) -> Result<(), SettingsError> {
        if (self.min()..=self.max()).contains(&code) {
            Ok(())
        } else {
            Err(SettingsError::OutOfRange {
                knob: self,
                value: code,
                min: self.min(),
                max: self.max(),
            })
        }
    }
}

/// The live knobs of one session. Every field sits in its knob's range.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// Blood policy.
    pub gore: GoreSetting,
    /// Dynamic bodies alive at once.
    pub debris_cap: u16,
    /// Ragdolls alive at once.
    pub ragdoll_cap: u8,
    /// Downward acceleration in millimetres per second squared.
    pub gravity_mm_s2: u32,
    /// Material row new ground and debris use.
    pub material: Material,
    /// Quality tier.
    pub tier: Tier,
    /// Render scale in percent.
    pub render_scale_pct: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            gore: GoreSetting::default(),
            debris_cap: Tier::Comfort.caps().bodies,
            ragdoll_cap: Tier::Comfort.caps().ragdolls,
            gravity_mm_s2: GRAVITY_MM_S2,
            material: GROUND,
            tier: Tier::Comfort,
            render_scale_pct: RENDER_SCALE_MAX,
        }
    }
}

impl Settings {
    /// The integer code of one knob's current value.
    pub fn code(&self, knob: Knob) -> i64 {
        match knob {
            Knob::Gore => i64::from(gore_code(self.gore)),
            Knob::DebrisCap => i64::from(self.debris_cap),
            Knob::RagdollCap => i64::from(self.ragdoll_cap),
            Knob::Gravity => i64::from(self.gravity_mm_s2),
            Knob::Material => i64::from(material_code(self.material)),
            Knob::Quality => i64::from(tier_code(self.tier)),
            Knob::RenderScale => i64::from(self.render_scale_pct),
        }
    }

    /// Write one already-checked code. The tape folds with this, so it never
    /// refuses: a replay must land where the live run landed.
    fn write(&mut self, knob: Knob, code: i64) -> Result<(), SettingsError> {
        knob.check(code)?;
        match knob {
            Knob::Gore => self.gore = gore_at(code),
            Knob::DebrisCap => self.debris_cap = narrow(knob, code)?,
            Knob::RagdollCap => self.ragdoll_cap = narrow(knob, code)?,
            Knob::Gravity => self.gravity_mm_s2 = narrow(knob, code)?,
            Knob::Material => self.material = material_at(code),
            Knob::Quality => self.tier = tier_at(code),
            Knob::RenderScale => self.render_scale_pct = narrow(knob, code)?,
        }
        Ok(())
    }

    fn fold_into(&self, hasher: &mut Fnv64) {
        hasher.write_u8(gore_code(self.gore));
        hasher.write_u16(self.debris_cap);
        hasher.write_u8(self.ragdoll_cap);
        hasher.write_u32(self.gravity_mm_s2);
        hasher.write_u8(material_code(self.material));
        hasher.write_u8(tier_code(self.tier));
        hasher.write_u8(self.render_scale_pct);
    }
}

/// Narrow one checked code into its field type.
fn narrow<T>(knob: Knob, code: i64) -> Result<T, SettingsError>
where
    T: TryFrom<i64>,
{
    T::try_from(code).map_err(|_| SettingsError::OutOfRange {
        knob,
        value: code,
        min: knob.min(),
        max: knob.max(),
    })
}

const fn gore_code(gore: GoreSetting) -> u8 {
    match gore {
        GoreSetting::Off => 0,
        GoreSetting::Reduced => 1,
        GoreSetting::Full => 2,
    }
}

const fn gore_at(code: i64) -> GoreSetting {
    match code {
        0 => GoreSetting::Off,
        1 => GoreSetting::Reduced,
        _ => GoreSetting::Full,
    }
}

const fn material_code(material: Material) -> u8 {
    match material {
        Material::Wood => 0,
        Material::Stone => 1,
        Material::Flesh => 2,
        Material::Metal => 3,
        Material::Sand => 4,
        Material::Cloth => 5,
    }
}

const fn material_at(code: i64) -> Material {
    match code {
        0 => Material::Wood,
        1 => Material::Stone,
        2 => Material::Flesh,
        3 => Material::Metal,
        4 => Material::Sand,
        _ => Material::Cloth,
    }
}

const fn tier_code(tier: Tier) -> u8 {
    match tier {
        Tier::Potato => 0,
        Tier::Web => 1,
        Tier::Comfort => 2,
        Tier::High => 3,
    }
}

const fn tier_at(code: i64) -> Tier {
    match code {
        0 => Tier::Potato,
        1 => Tier::Web,
        2 => Tier::Comfort,
        _ => Tier::High,
    }
}

/// Why a settings call refused. A refusal changes nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SettingsError {
    /// The code is outside the knob's range; the range rides along.
    OutOfRange {
        /// Knob that refused.
        knob: Knob,
        /// Code asked for.
        value: i64,
        /// Smallest accepted code.
        min: i64,
        /// Largest accepted code.
        max: i64,
    },
    /// The boundary tick is before the newest tape record.
    BoundaryInPast {
        /// Tick asked for.
        at_tick: u32,
        /// Earliest tick the call may stamp.
        earliest: u32,
    },
    /// Nothing applied to undo.
    NothingToUndo,
    /// The tape holds [`MAX_TAPE_RECORDS`] records.
    TapeFull,
}

impl core::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfRange {
                knob,
                value,
                min,
                max,
            } => write!(
                f,
                "knob `{}` value {value} is outside {min}..={max} {}; re-run with a code in \
                 {min}..={max}",
                knob.name(),
                knob.unit()
            ),
            Self::BoundaryInPast { at_tick, earliest } => write!(
                f,
                "tick {at_tick} is in the past (earliest {earliest}); re-run at that tick or later"
            ),
            Self::NothingToUndo => write!(
                f,
                "nothing applied to undo; call `{}` first",
                TOOL_SETTINGS_SET
            ),
            Self::TapeFull => write!(
                f,
                "settings tape is full ({MAX_TAPE_RECORDS} records); stop adding changes"
            ),
        }
    }
}

impl std::error::Error for SettingsError {}

/// One tape row: the whole settings state in force from `at_tick` on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Record {
    at_tick: u32,
    settings: Settings,
}

/// Append-only journal of applied settings states. Replaying it from the
/// defaults gives the same values on the same tick on every machine.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct SettingsTape {
    records: Vec<Record>,
}

impl SettingsTape {
    /// Empty tape: every tick reads the defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records on the tape.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when nothing was ever applied.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Tick of the newest record, `0` on an empty tape.
    pub fn last_tick(&self) -> u32 {
        self.records.last().map_or(0, |record| record.at_tick)
    }

    /// Settings in force on `tick`: the newest record at or before it, or the
    /// defaults when no record has landed yet. Pure.
    pub fn settings_at(&self, tick: u32) -> Settings {
        self.records
            .iter()
            .rev()
            .find(|record| record.at_tick <= tick)
            .map_or_else(Settings::default, |record| record.settings)
    }

    /// Digest of the whole tape for replay checks. A view receipt outside any
    /// simulation hash scope. Same records give the same digest. Pure.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u64(u64::try_from(self.records.len()).unwrap_or(u64::MAX));
        for record in &self.records {
            hasher.write_u32(record.at_tick);
            record.settings.fold_into(&mut hasher);
        }
        hasher.finish()
    }

    fn push(&mut self, at_tick: u32, settings: Settings) -> Result<(), SettingsError> {
        if self.records.len() >= MAX_TAPE_RECORDS {
            return Err(SettingsError::TapeFull);
        }
        self.records.push(Record { at_tick, settings });
        Ok(())
    }
}

/// What one apply or undo stamped on the tape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SettingsReceipt {
    /// Tick the record takes effect on.
    pub at_tick: u32,
    /// Tape digest after the record.
    pub tape_digest: u64,
}

/// One session's settings: the live state, the replay tape and the undo stack.
#[derive(Clone, Default, Debug)]
pub struct SettingsSlot {
    live: Settings,
    tape: SettingsTape,
    undo: Vec<Settings>,
}

impl SettingsSlot {
    /// Settings in force now.
    pub fn live(&self) -> &Settings {
        &self.live
    }

    /// The replay tape.
    pub fn tape(&self) -> &SettingsTape {
        &self.tape
    }

    /// Batches an undo can still take back.
    pub fn undoable(&self) -> usize {
        self.undo.len()
    }

    /// Write one knob to take effect on `at_tick` (the next boundary):
    /// validate, then append a whole-state record. Earlier ticks keep the old
    /// value, so a replay reads the change exactly on its tick.
    ///
    /// # Errors
    /// [`SettingsError::OutOfRange`] for a bad code, `BoundaryInPast` before
    /// the newest record or `TapeFull`; nothing changes.
    pub fn set(
        &mut self,
        at_tick: u32,
        knob: Knob,
        code: i64,
    ) -> Result<SettingsReceipt, SettingsError> {
        let mut next = self.live;
        next.write(knob, code)?;
        self.check_boundary(at_tick)?;
        self.tape.push(at_tick, next)?;
        self.undo.push(self.live);
        self.live = next;
        Ok(self.receipt(at_tick))
    }

    /// Undo the newest applied change by stamping the previous state on
    /// `at_tick`. Earlier ticks keep replaying the change.
    ///
    /// # Errors
    /// [`SettingsError::NothingToUndo`], `BoundaryInPast` or `TapeFull`;
    /// nothing changes.
    pub fn undo(&mut self, at_tick: u32) -> Result<SettingsReceipt, SettingsError> {
        let previous = *self.undo.last().ok_or(SettingsError::NothingToUndo)?;
        self.check_boundary(at_tick)?;
        self.tape.push(at_tick, previous)?;
        self.undo.pop();
        self.live = previous;
        Ok(self.receipt(at_tick))
    }

    fn check_boundary(&self, at_tick: u32) -> Result<(), SettingsError> {
        let earliest = self.tape.last_tick();
        if at_tick < earliest {
            Err(SettingsError::BoundaryInPast { at_tick, earliest })
        } else {
            Ok(())
        }
    }

    fn receipt(&self, at_tick: u32) -> SettingsReceipt {
        SettingsReceipt {
            at_tick,
            tape_digest: self.tape.digest(),
        }
    }
}

/// Where a read starts: the live state, or the tape replayed to `at_tick`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", default)]
#[schemars(crate = "rmcp::schemars")]
pub struct SettingsGetArgs {
    pub at_tick: Option<u32>,
}

/// One knob: its word, unit, range and code.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct KnobOut {
    pub name: String,
    pub unit: String,
    pub min: i64,
    pub max: i64,
    pub value: i64,
}

/// One material row: the numbers every reader of the table shares.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct MaterialOut {
    pub code: i64,
    pub name: String,
    pub density_kg_m3: u16,
    pub friction_pm: u16,
    pub restitution_pm: u16,
    pub linear_damping_pm: u16,
    pub angular_damping_pm: u16,
    pub step_mm: u16,
    pub sound: String,
}

/// One tier row with the caps it admits.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TierOut {
    pub code: i64,
    pub name: String,
    pub debris_cap: u16,
    pub ragdoll_cap: u8,
}

/// The knob table, the material rows, the tier rows and the tape receipt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SettingsGetOutput {
    pub at_tick: Option<u32>,
    pub knobs: Vec<KnobOut>,
    pub materials: Vec<MaterialOut>,
    pub tiers: Vec<TierOut>,
    pub tape_records: u64,
    pub tape_digest: u64,
}

/// One knob write.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SettingsSetArgs {
    pub at_tick: u32,
    pub knob: String,
    pub value: i64,
}

/// What a write stamped.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SettingsSetOutput {
    pub at_tick: u32,
    pub knob: String,
    pub value: i64,
    pub unit: String,
    pub tape_digest: u64,
}

/// One undo at a boundary tick.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SettingsUndoArgs {
    pub at_tick: u32,
}

/// What an undo stamped.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SettingsUndoOutput {
    pub at_tick: u32,
    pub tape_digest: u64,
}

fn material_row(material: Material) -> MaterialOut {
    let props = material.props();
    MaterialOut {
        code: i64::from(material_code(material)),
        name: format!("{material:?}").to_lowercase(),
        density_kg_m3: props.density_kg_m3,
        friction_pm: props.friction_pm,
        restitution_pm: props.restitution_pm,
        linear_damping_pm: props.linear_damping_pm,
        angular_damping_pm: props.angular_damping_pm,
        step_mm: props.step_mm,
        sound: format!("{:?}", props.sound).to_lowercase(),
    }
}

fn tier_row(tier: Tier) -> TierOut {
    let caps = tier.caps();
    TierOut {
        code: i64::from(tier_code(tier)),
        name: tier.name().to_owned(),
        debris_cap: caps.bodies,
        ragdoll_cap: caps.ragdolls,
    }
}

/// The knob table, the material rows, the tier rows and the tape receipt for
/// one read point: the live state, or the tape replayed to `at_tick`. Pure.
fn report(slot: &SettingsSlot, at_tick: Option<u32>) -> SettingsGetOutput {
    let settings = match at_tick {
        Some(tick) => slot.tape().settings_at(tick),
        None => *slot.live(),
    };
    SettingsGetOutput {
        at_tick,
        knobs: Knob::ALL
            .into_iter()
            .map(|knob| KnobOut {
                name: knob.name().to_owned(),
                unit: knob.unit().to_owned(),
                min: knob.min(),
                max: knob.max(),
                value: settings.code(knob),
            })
            .collect(),
        materials: Material::ALL.into_iter().map(material_row).collect(),
        tiers: Tier::ALL.into_iter().map(tier_row).collect(),
        tape_records: u64::try_from(slot.tape().len()).unwrap_or(u64::MAX),
        tape_digest: slot.tape().digest(),
    }
}

/// `settings.get` handler: read the tables and one read point. Writes nothing.
fn settings_get(
    service: &McpServer,
    args: &SettingsGetArgs,
) -> Result<SettingsGetOutput, McpToolError> {
    service.dispatch(TOOL_SETTINGS_GET, |state| {
        Ok(report(&state.settings, args.at_tick))
    })
}

/// `settings.set` handler: resolve the knob word, then write one code at a
/// boundary tick through the session slot.
fn settings_set(
    service: &McpServer,
    args: &SettingsSetArgs,
) -> Result<SettingsSetOutput, McpToolError> {
    let knob = Knob::from_name(&args.knob).ok_or_else(|| {
        let legal: Vec<&str> = Knob::ALL.iter().map(|knob| knob.name()).collect();
        bad(format!(
            "knob `{}` is unknown; legal knobs: [{}]; re-run with one of those names",
            args.knob,
            legal.join(", ")
        ))
    })?;
    let receipt = service.dispatch(TOOL_SETTINGS_SET, |state| {
        state
            .settings
            .set(args.at_tick, knob, args.value)
            .map_err(|error| bad(error.to_string()))
    })?;
    Ok(SettingsSetOutput {
        at_tick: receipt.at_tick,
        knob: knob.name().to_owned(),
        value: args.value,
        unit: knob.unit().to_owned(),
        tape_digest: receipt.tape_digest,
    })
}

/// `settings.undo` handler: write the newest applied state back at `at_tick`.
fn settings_undo(
    service: &McpServer,
    args: &SettingsUndoArgs,
) -> Result<SettingsUndoOutput, McpToolError> {
    let receipt = service.dispatch(TOOL_SETTINGS_UNDO, |state| {
        state
            .settings
            .undo(args.at_tick)
            .map_err(|error| bad(error.to_string()))
    })?;
    Ok(SettingsUndoOutput {
        at_tick: receipt.at_tick,
        tape_digest: receipt.tape_digest,
    })
}

crate::e2_tool!(
    /// `settings.get`: the knob table, the material rows and the live values.
    SettingsGetTool,
    name = TOOL_SETTINGS_GET,
    description = "Read the physics and render knobs (gore, debris_cap, ragdoll_cap, gravity, material, quality, render_scale): each knob's unit, legal range and code, the six material rows, the four tier caps and the tape digest. at_tick replays the tape. Read-only.",
    readonly = true,
    args = SettingsGetArgs,
    output = SettingsGetOutput,
    invoke = settings_get
);

crate::e2_tool!(
    /// `settings.set`: write one knob at a boundary tick.
    SettingsSetTool,
    name = TOOL_SETTINGS_SET,
    description = "Write one knob at a boundary tick: knob is gore, debris_cap, ragdoll_cap, gravity, material, quality or render_scale; value is its integer code. A code outside the knob's range refuses with the range and changes nothing. Undo restores it.",
    readonly = false,
    args = SettingsSetArgs,
    output = SettingsSetOutput,
    invoke = settings_set
);

crate::e2_tool!(
    /// `settings.undo`: write the previous state back at a later tick.
    SettingsUndoTool,
    name = TOOL_SETTINGS_UNDO,
    description = "Undo the newest settings change by writing the previous state at a later tick; earlier ticks keep replaying the change. Refuses nothing_to_undo when no change is pending.",
    readonly = false,
    args = SettingsUndoArgs,
    output = SettingsUndoOutput,
    invoke = settings_undo
);

/// The settings pack: one read-only verb and two writers, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "physics and render knobs an AI can set: gore, debris and ragdoll caps, gravity, the \
         material table, quality tier and render scale as integer codes with ranges, a replay \
         tape that reads a change on its tick, and step-by-step undo",
        false,
    );
    pack.add(VerbSpec::of::<SettingsGetTool>(
        TOOL_SETTINGS_GET,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<SettingsSetTool>(
        TOOL_SETTINGS_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<SettingsUndoTool>(
        TOOL_SETTINGS_UNDO,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_combat::projectile::{FlightTables, GRAVITY_V1};
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn set_args(at_tick: u32, knob: Knob, value: i64) -> SettingsSetArgs {
        SettingsSetArgs {
            at_tick,
            knob: knob.name().to_owned(),
            value,
        }
    }

    fn get(server: &McpServer, at_tick: Option<u32>) -> SettingsGetOutput {
        SettingsGetTool::invoke(server, SettingsGetArgs { at_tick }).expect("settings read")
    }

    fn knob_value(out: &SettingsGetOutput, knob: Knob) -> i64 {
        out.knobs
            .iter()
            .find(|row| row.name == knob.name())
            .expect("every knob is listed")
            .value
    }

    #[test]
    fn out_of_range_values_are_refused_with_the_range_in_the_reason() {
        let server = server();
        for (knob, code, range) in [
            (Knob::Gore, 3, "0..=2"),
            (Knob::DebrisCap, 0, "96..=768"),
            (Knob::DebrisCap, 5_000, "96..=768"),
            (Knob::RagdollCap, 99, "2..=16"),
            (Knob::Gravity, 40_000, "0..=32000"),
            (Knob::Material, 6, "0..=5"),
            (Knob::Quality, 4, "0..=3"),
            (Knob::RenderScale, 49, "50..=100"),
            (Knob::RenderScale, 120, "50..=100"),
        ] {
            let refused = SettingsSetTool::invoke(&server, set_args(5, knob, code))
                .expect_err("out of range refuses");
            assert_eq!(refused.code, "bad_args");
            assert!(
                refused.message.contains(knob.name()),
                "the refusal names the knob: {}",
                refused.message
            );
            assert!(
                refused.message.contains(range),
                "the refusal names the range {range}: {}",
                refused.message
            );
        }
        // A refusal changes nothing: no record, no live value.
        let out = get(&server, None);
        assert_eq!(out.tape_records, 0);
        for knob in Knob::ALL {
            assert_eq!(knob_value(&out, knob), knob.default_code());
        }
    }

    #[test]
    fn an_unknown_knob_is_refused_naming_the_legal_knobs() {
        let server = server();
        let refused = SettingsSetTool::invoke(
            &server,
            SettingsSetArgs {
                at_tick: 1,
                knob: "gore_level".to_owned(),
                value: 1,
            },
        )
        .expect_err("unknown knob refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("gore_level"));
        for legal in ["gore", "debris_cap", "render_scale"] {
            assert!(
                refused.message.contains(legal),
                "the refusal names {legal}: {}",
                refused.message
            );
        }
    }

    #[test]
    fn the_recorded_tape_replays_the_change_at_its_tick() {
        let door = server();
        SettingsSetTool::invoke(&door, set_args(10, Knob::RenderScale, 75)).expect("lands");
        // Before the tick the tape folds to the defaults; on the tick it changes.
        let before = get(&door, Some(9));
        let at = get(&door, Some(10));
        let live = get(&door, None);
        assert_eq!(knob_value(&before, Knob::RenderScale), 100);
        assert_eq!(knob_value(&at, Knob::RenderScale), 75);
        assert_eq!(knob_value(&live, Knob::RenderScale), 75);
        assert_eq!((before.tape_records, at.tape_records), (1, 1));
        // Two sessions with the same tape replay the same values and digest.
        let other = server();
        SettingsSetTool::invoke(&other, set_args(10, Knob::RenderScale, 75)).expect("lands");
        let again = get(&other, Some(10));
        assert_eq!(again.tape_digest, at.tape_digest);
        assert_eq!(again.knobs, at.knobs);
        // A different value is a different tape.
        let third = server();
        SettingsSetTool::invoke(&third, set_args(10, Knob::RenderScale, 80)).expect("lands");
        assert_ne!(get(&third, None).tape_digest, live.tape_digest);
    }

    #[test]
    fn undo_restores_the_previous_state_and_history_stays_replayable() {
        let server = server();
        SettingsSetTool::invoke(&server, set_args(10, Knob::DebrisCap, 512)).expect("lands");
        assert_eq!(get(&server, None).tape_records, 1);
        SettingsUndoTool::invoke(&server, SettingsUndoArgs { at_tick: 30 }).expect("undo lands");
        // The change stays live until its undo tick.
        assert_eq!(knob_value(&get(&server, Some(25)), Knob::DebrisCap), 512);
        assert_eq!(knob_value(&get(&server, Some(30)), Knob::DebrisCap), 384);
        assert_eq!(knob_value(&get(&server, None), Knob::DebrisCap), 384);
        // A second undo has nothing left and changes nothing.
        let refused = SettingsUndoTool::invoke(&server, SettingsUndoArgs { at_tick: 40 })
            .expect_err("nothing to undo");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("nothing applied to undo"));
        assert_eq!(get(&server, None).tape_records, 2);
    }

    #[test]
    fn every_knob_writes_and_undoes_in_one_step() {
        for (knob, code) in [
            (Knob::Gore, 0),
            (Knob::DebrisCap, 192),
            (Knob::RagdollCap, 4),
            (Knob::Gravity, 0),
            (Knob::Material, 3),
            (Knob::Quality, 0),
            (Knob::RenderScale, 50),
        ] {
            let mut slot = SettingsSlot::default();
            slot.set(4, knob, code).expect("legal value lands");
            assert_eq!(slot.live().code(knob), code, "{knob:?}");
            assert_eq!(slot.tape().settings_at(3).code(knob), knob.default_code());
            assert_eq!(slot.tape().settings_at(4).code(knob), code);
            slot.undo(9).expect("undo lands");
            assert_eq!(slot.live().code(knob), knob.default_code(), "{knob:?}");
            assert_eq!(slot.tape().settings_at(9).code(knob), knob.default_code());
        }
    }

    #[test]
    fn a_boundary_before_the_newest_record_is_refused_and_changes_nothing() {
        let mut slot = SettingsSlot::default();
        slot.set(20, Knob::Gravity, 0).expect("lands");
        let digest = slot.tape().digest();
        assert_eq!(
            slot.set(5, Knob::Gore, 0),
            Err(SettingsError::BoundaryInPast {
                at_tick: 5,
                earliest: 20
            })
        );
        assert_eq!(slot.tape().digest(), digest);
        assert_eq!(slot.live().gore, GoreSetting::Full);
    }

    #[test]
    fn defaults_equal_the_shipped_constants() {
        let settings = Settings::default();
        assert_eq!(settings.gore, GoreSetting::Full);
        assert_eq!(settings.material, GROUND);
        assert_eq!(settings.material, Material::Stone);
        assert_eq!(settings.tier, Tier::Comfort);
        assert_eq!(settings.debris_cap, Tier::Comfort.caps().bodies);
        assert_eq!(settings.ragdoll_cap, Tier::Comfort.caps().ragdolls);
        assert_eq!(settings.gravity_mm_s2, GRAVITY_MM_S2);
        assert_eq!(settings.render_scale_pct, RENDER_SCALE_MAX);
        for knob in Knob::ALL {
            let code = settings.code(knob);
            assert_eq!(code, knob.default_code(), "{knob:?} default");
            knob.check(code).expect("the default sits in its range");
        }
        // The code tables round-trip through the typed values.
        for (index, material) in Material::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(material_code(material)), index);
            assert_eq!(material_at(i64::from(material_code(material))), material);
        }
        for (index, tier) in Tier::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(tier_code(tier)), index);
            assert_eq!(tier_at(i64::from(tier_code(tier))), tier);
        }
        for (index, gore) in [GoreSetting::Off, GoreSetting::Reduced, GoreSetting::Full]
            .into_iter()
            .enumerate()
        {
            assert_eq!(usize::from(gore_code(gore)), index);
            assert_eq!(gore_at(i64::from(gore_code(gore))), gore);
        }
    }

    #[test]
    fn defaults_do_not_drift_from_the_owner_sources() {
        // Gravity: the shipped flight row and its 32 m/s^2 bound, in mm/s^2.
        let gravity = GRAVITY_V1.to_bits();
        assert_eq!(
            i64::from(GRAVITY_MM_S2),
            (gravity * 1_000 + 32_768) / 65_536,
            "Earth gravity follows the flight row"
        );
        assert_eq!(
            i64::from(GRAVITY_MAX_MM_S2),
            FlightTables::MAX_GRAVITY.to_bits() * 1_000 / 65_536,
            "the gravity ceiling is the flight table bound"
        );
        // Render scale: the settings screen's bounds.
        let ui = include_str!("../../../ui/src/settings.rs");
        assert!(ui.contains("pub const RENDER_SCALE_MIN: u8 = 50;"));
        assert!(ui.contains("pub const RENDER_SCALE_MAX: u8 = 100;"));
        assert_eq!(i64::from(RENDER_SCALE_MIN), Knob::RenderScale.min());
        assert_eq!(i64::from(RENDER_SCALE_MAX), Knob::RenderScale.max());
        // Caps: the physical layer's per-tier table.
        let rows = caps_from_source(include_str!("../../../physfx/src/caps.rs"));
        assert_eq!(rows.len(), Tier::ALL.len());
        for (name, bodies, ragdolls) in rows {
            let tier = Tier::from_name(&name).expect("a known tier");
            assert_eq!(i64::from(tier.caps().bodies), bodies, "{name} bodies");
            assert_eq!(i64::from(tier.caps().ragdolls), ragdolls, "{name} ragdolls");
        }
        // Tier words: the render table prints the same four names.
        let render = include_str!("../../../render/src/quality.rs");
        for tier in Tier::ALL {
            assert!(
                render.contains(&format!("\"{}\"", tier.name())),
                "render prints tier {}",
                tier.name()
            );
        }
    }

    /// The `bodies` and `ragdolls` numbers of every `Tier::<name> => Caps { .. }`
    /// arm of the physical-layer cap source.
    fn caps_from_source(table: &str) -> Vec<(String, i64, i64)> {
        let mut rows = Vec::new();
        for tier in ["Potato", "Web", "Comfort", "High"] {
            let head = format!("Tier::{tier} => Caps {{");
            let arm = table.split(&head).nth(1).unwrap_or_default();
            let body = arm.split('}').next().unwrap_or_default();
            let field = |name: &str| -> i64 {
                body.split(&format!("{name}:"))
                    .nth(1)
                    .and_then(|rest| rest.split(',').next())
                    .and_then(|number| number.trim().parse().ok())
                    .unwrap_or(-1)
            };
            rows.push((tier.to_owned(), field("bodies"), field("ragdolls")));
        }
        rows
    }

    #[test]
    fn the_tables_publish_the_owner_rows() {
        let server = server();
        let out = get(&server, None);
        assert_eq!(out.knobs.len(), Knob::ALL.len());
        assert_eq!(out.materials.len(), Material::ALL.len());
        assert_eq!(out.tiers.len(), Tier::ALL.len());
        for (row, material) in out.materials.iter().zip(Material::ALL) {
            let props = material.props();
            assert_eq!(row.code, i64::from(material_code(material)));
            assert_eq!(row.density_kg_m3, props.density_kg_m3);
            assert_eq!(row.friction_pm, props.friction_pm);
            assert_eq!(row.restitution_pm, props.restitution_pm);
            assert_eq!(row.linear_damping_pm, props.linear_damping_pm);
            assert_eq!(row.angular_damping_pm, props.angular_damping_pm);
            assert_eq!(row.step_mm, props.step_mm);
        }
        for (row, tier) in out.tiers.iter().zip(Tier::ALL) {
            assert_eq!(row.name, tier.name());
            assert_eq!(row.debris_cap, tier.caps().bodies);
            assert_eq!(row.ragdoll_cap, tier.caps().ragdolls);
        }
    }

    #[test]
    fn settings_pack_registers_one_reader_and_two_writers() {
        let pack = pack().expect("pack builds");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![TOOL_SETTINGS_GET, TOOL_SETTINGS_SET, TOOL_SETTINGS_UNDO]
        );
        let cards: Vec<CapabilityCard> = pack.verbs().iter().map(VerbSpec::card).collect();
        assert_eq!(
            cards,
            vec![
                CapabilityCard::Observe,
                CapabilityCard::Commit,
                CapabilityCard::Commit
            ]
        );
        assert!(pack.verbs()[0].readonly());
        assert!(!pack.verbs()[1].readonly() && !pack.verbs()[2].readonly());
        assert!(!pack.default_loaded(), "the pack loads on demand");
    }
}
