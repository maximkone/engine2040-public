//! `fx` preset authoring: emitter presets an artist can name, preview and store.
//!
//! The particle and decal passes already render break dust and blood, but the
//! emitter numbers lived in the effect call. This module owns the authored
//! data ([`EmitterPreset`]) and three verbs over it: `fx.preset.list`,
//! `fx.preset.preview`, `fx.preset.set` and `fx.preset.clear`. It renders
//! nothing: the review surface answers integer particle samples the render
//! pass can consume later.
//!
//! Units: ticks at [`engine_core::TICK_HZ`], millimeters, centimeters,
//! degrees and mm/s². Every sample is pure integer math seeded by the preset
//! digest, so the same preset and tick list always answer the same digest and
//! the same samples (no f32 on the emit path). A live preset is presentation
//! data, never hashed by the fixed tick.
//!
//! Session state: at most [`MAX_SESSION_PRESETS`] authored presets per
//! session, keyed by a lower_snake name that never shadows a shipped one.
//! Errors: [`McpToolError`] with `bad_args`, naming the field, its band and
//! one recovery.

use std::collections::btree_map::Entry as MapEntry;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use engine_core::Fnv64;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::PACK;
use crate::mcp::{McpServer, McpToolError};
use crate::packs::{bad_args as bad, PackDef, PackError, VerbSpec};
use crate::tool_capabilities::CapabilityCard;

/// Verb: list the shipped and session presets with their digests.
pub const TOOL_FX_PRESET_LIST: &str = "fx.preset.list";
/// Verb: sample the first particles of one preset at the named ticks.
pub const TOOL_FX_PRESET_PREVIEW: &str = "fx.preset.preview";
/// Verb: store one inline preset in this session.
pub const TOOL_FX_PRESET_SET: &str = "fx.preset.set";
/// Verb: remove one session preset by name.
pub const TOOL_FX_PRESET_CLEAR: &str = "fx.preset.clear";

/// Most authored presets one session may store.
pub const MAX_SESSION_PRESETS: usize = 32;
/// Most ticks one preview may sample.
pub const MAX_PREVIEW_TICKS: usize = 16;
/// Particles a preview samples, in launch order.
pub const PREVIEW_PARTICLES: u32 = 8;

/// Shortest particle life a preset may name, in ticks.
pub const LIFE_TICKS_MIN: u16 = 1;
/// Longest particle life a preset may name, in ticks.
pub const LIFE_TICKS_MAX: u16 = 600;
/// Lowest emission rate a preset may name, per 100 ticks.
pub const RATE_MIN: u16 = 0;
/// Highest emission rate a preset may name, per 100 ticks.
pub const RATE_MAX: u16 = 20_000;
/// Smallest particle a preset may name, in centimeters.
pub const SIZE_CM_MIN: u16 = 1;
/// Largest particle a preset may name, in centimeters.
pub const SIZE_CM_MAX: u16 = 500;
/// Narrowest launch cone a preset may name, in degrees.
pub const SPREAD_DEG_MIN: u16 = 0;
/// Widest launch cone a preset may name, in degrees.
pub const SPREAD_DEG_MAX: u16 = 180;
/// Weakest vertical acceleration a preset may name, in mm/s².
pub const GRAVITY_MM_S2_MIN: i16 = -30_000;
/// Strongest vertical acceleration a preset may name, in mm/s².
pub const GRAVITY_MM_S2_MAX: i16 = 30_000;
/// Shortest relaunch period a preset may name, in ticks.
pub const LOOP_TICKS_MIN: u16 = 1;
/// Longest relaunch period a preset may name, in ticks.
pub const LOOP_TICKS_MAX: u16 = 600;
/// Longest preset name, in chars.
pub const NAME_MAX_CHARS: usize = 48;

/// One authored emitter: the numbers the particle and decal passes read.
///
/// The first eight fields ride the wire as they are authored; the name is the
/// key `fx.preset.set`, `fx.preset.clear` and `fx.preset.preview` answer.
/// `gravity_mm_s2` is negative when particles fall, which is the usual case.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct EmitterPreset {
    /// Stored name: lower_snake, 1..=48 chars, unique among the listed presets.
    pub name: String,
    /// Ticks one particle lives, 1..=600.
    pub life_ticks: u16,
    /// Particles emitted per 100 ticks, 0..=20000 (0 emits nothing).
    pub rate_per_100_ticks: u16,
    /// Particle diameter in centimeters, 1..=500.
    pub size_cm: u16,
    /// Launch cone in degrees, 0..=180.
    pub spread_deg: u16,
    /// Vertical acceleration in mm/s², -30000..=30000 (negative falls).
    pub gravity_mm_s2: i16,
    /// Particle colour, one byte per red, green and blue channel.
    pub colour_rgb: [u8; 3],
    /// True when the emitter leaves a decal where a particle lands.
    pub decal: bool,
    /// Relaunch period in ticks, 1..=600; null is a one-shot emitter.
    #[serde(default)]
    pub loop_ticks: Option<u16>,
}

impl EmitterPreset {
    /// Check the name and every field against its band, naming the first
    /// offender with the legal range and one recovery.
    pub fn check(&self) -> Result<(), McpToolError> {
        check_name(&self.name)?;
        band(
            "life_ticks",
            self.life_ticks,
            LIFE_TICKS_MIN,
            LIFE_TICKS_MAX,
            "ticks",
        )?;
        band(
            "rate_per_100_ticks",
            self.rate_per_100_ticks,
            RATE_MIN,
            RATE_MAX,
            "particles per 100 ticks",
        )?;
        band(
            "size_cm",
            self.size_cm,
            SIZE_CM_MIN,
            SIZE_CM_MAX,
            "centimeters",
        )?;
        band(
            "spread_deg",
            self.spread_deg,
            SPREAD_DEG_MIN,
            SPREAD_DEG_MAX,
            "degrees",
        )?;
        band(
            "gravity_mm_s2",
            self.gravity_mm_s2,
            GRAVITY_MM_S2_MIN,
            GRAVITY_MM_S2_MAX,
            "mm/s² (negative falls)",
        )?;
        if let Some(period) = self.loop_ticks {
            band(
                "loop_ticks",
                period,
                LOOP_TICKS_MIN,
                LOOP_TICKS_MAX,
                "ticks",
            )?;
        }
        Ok(())
    }

    /// FNV-1a digest of the name and every number: the id the preset verbs
    /// answer and the seed of the preview's launch.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_len32(self.name.len());
        hasher.write_str(&self.name);
        hasher.write_u16(self.life_ticks);
        hasher.write_u16(self.rate_per_100_ticks);
        hasher.write_u16(self.size_cm);
        hasher.write_u16(self.spread_deg);
        hasher.write(&self.gravity_mm_s2.to_le_bytes());
        hasher.write(&self.colour_rgb);
        hasher.write_bool(self.decal);
        match self.loop_ticks {
            Some(period) => {
                hasher.write_bool(true);
                hasher.write_u16(period);
            }
            None => hasher.write_bool(false),
        }
        hasher.finish()
    }

    /// The digest as 16 lowercase hex chars.
    pub fn digest_hex(&self) -> String {
        format!("{:016x}", self.digest())
    }
}

/// One shipped emitter's numbers, before it is paired with its stored name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShippedEmitter {
    /// Ticks one particle lives.
    pub life_ticks: u16,
    /// Particles emitted per 100 ticks.
    pub rate_per_100_ticks: u16,
    /// Particle diameter in centimeters.
    pub size_cm: u16,
    /// Launch cone in degrees.
    pub spread_deg: u16,
    /// Vertical acceleration in mm/s².
    pub gravity_mm_s2: i16,
    /// Particle colour, one byte per channel.
    pub colour_rgb: [u8; 3],
    /// True when it leaves a decal.
    pub decal: bool,
    /// Relaunch period in ticks; null is one-shot.
    pub loop_ticks: Option<u16>,
}

impl ShippedEmitter {
    /// The shipped numbers under a stored name.
    pub fn named(self, name: &str) -> EmitterPreset {
        EmitterPreset {
            name: name.to_owned(),
            life_ticks: self.life_ticks,
            rate_per_100_ticks: self.rate_per_100_ticks,
            size_cm: self.size_cm,
            spread_deg: self.spread_deg,
            gravity_mm_s2: self.gravity_mm_s2,
            colour_rgb: self.colour_rgb,
            decal: self.decal,
            loop_ticks: self.loop_ticks,
        }
    }
}

/// The shipped dust emitter: two seconds, slow and wide, grey-brown, no decal.
pub const DUST: ShippedEmitter = ShippedEmitter {
    life_ticks: 120,
    rate_per_100_ticks: 20,
    size_cm: 18,
    spread_deg: 150,
    gravity_mm_s2: -400,
    colour_rgb: [138, 124, 106],
    decal: false,
    loop_ticks: None,
};

/// The shipped sparks emitter: short life, high rate, orange, no decal.
pub const SPARKS: ShippedEmitter = ShippedEmitter {
    life_ticks: 18,
    rate_per_100_ticks: 20_000,
    size_cm: 3,
    spread_deg: 120,
    gravity_mm_s2: -9_810,
    colour_rgb: [255, 150, 40],
    decal: false,
    loop_ticks: None,
};

/// The shipped blood emitter: one second, medium, red, leaves a decal.
pub const BLOOD: ShippedEmitter = ShippedEmitter {
    life_ticks: 60,
    rate_per_100_ticks: 900,
    size_cm: 9,
    spread_deg: 40,
    gravity_mm_s2: -6_000,
    colour_rgb: [150, 18, 22],
    decal: true,
    loop_ticks: None,
};

/// The three shipped emitters, name-sorted: blood, dust, sparks.
pub fn builtins() -> Vec<EmitterPreset> {
    vec![
        BLOOD.named("blood"),
        DUST.named("dust"),
        SPARKS.named("sparks"),
    ]
}

/// Refuse a name that is not lower_snake, naming the shape and one example.
fn check_name(name: &str) -> Result<(), McpToolError> {
    let starts_well = name.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
    let body_well = name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if starts_well && body_well && name.len() <= NAME_MAX_CHARS {
        return Ok(());
    }
    Err(bad(format!(
        "preset name `{name}` is not lower_snake; use 1..={NAME_MAX_CHARS} ascii lowercase \
         letters, digits or `_`, starting with a letter. Next: rename it, such as `dust_puff`."
    )))
}

/// Refuse a value outside its closed band, naming the key, the band and one
/// recovery.
fn band<T>(key: &str, value: T, min: T, max: T, unit: &str) -> Result<(), McpToolError>
where
    T: Copy + Ord + std::fmt::Display,
{
    if value < min || value > max {
        return Err(bad(format!(
            "preset field `{key}` is {value}; pass {min}..={max} {unit}. \
             Next: clamp it into the band and retry."
        )));
    }
    Ok(())
}

/// One session's authored presets, keyed by name.
#[derive(Default)]
struct PresetStore {
    presets: BTreeMap<String, EmitterPreset>,
}

impl PresetStore {
    /// Every listed preset: the shipped three plus this session's, name-sorted.
    fn rows(&self) -> Vec<EmitterPresetRow> {
        let mut rows: Vec<EmitterPresetRow> = builtins()
            .into_iter()
            .map(|preset| EmitterPresetRow::of(preset, true))
            .collect();
        rows.extend(
            self.presets
                .values()
                .map(|preset| EmitterPresetRow::of(preset.clone(), false)),
        );
        rows.sort_by(|left, right| left.name.cmp(&right.name));
        rows
    }

    /// Every listed name, for a refusal that names the legal set.
    fn names(&self) -> Vec<String> {
        self.rows().into_iter().map(|row| row.name).collect()
    }

    /// One listed preset by exact name.
    fn find(&self, name: &str) -> Option<EmitterPreset> {
        builtins()
            .into_iter()
            .find(|preset| preset.name == name)
            .or_else(|| self.presets.get(name).cloned())
    }

    /// Store one checked preset. A name already listed refuses without
    /// `replace`; a full store refuses with the cap and one recovery.
    fn set(
        &mut self,
        preset: EmitterPreset,
        replace: bool,
    ) -> Result<(String, u32, bool), McpToolError> {
        preset.check()?;
        if builtins().iter().any(|shipped| shipped.name == preset.name) {
            return Err(bad(format!(
                "preset `{}` is a shipped preset and cannot be replaced; pick another name, \
                 such as `{}_custom`, or preview the shipped one by name",
                preset.name, preset.name
            )));
        }
        let replaced = self.presets.contains_key(&preset.name);
        if replaced && !replace {
            return Err(bad(format!(
                "preset `{}` is already stored; pass replace: true to update it, or clear it \
                 first with fx.preset.clear",
                preset.name
            )));
        }
        if !replaced && self.presets.len() >= MAX_SESSION_PRESETS {
            return Err(bad(format!(
                "preset store holds {MAX_SESSION_PRESETS} presets, its cap; remove one with \
                 fx.preset.clear (fx.preset.list names them all), then retry"
            )));
        }
        let name = preset.name.clone();
        self.presets.insert(preset.name.clone(), preset);
        Ok((name, count(self.presets.len()), replaced))
    }

    /// Remove one session preset by exact name.
    fn clear(&mut self, name: &str) -> Result<(String, bool, u32), McpToolError> {
        if self.presets.remove(name).is_none() {
            return Err(bad(format!(
                "preset `{name}` is not stored in this session; stored names: [{}]. \
                 Next: clear one of those, or set `{name}` first",
                self.presets.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        Ok((name.to_owned(), true, count(self.presets.len())))
    }
}

/// A session store count as `u32`.
fn count(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn stores() -> &'static Mutex<BTreeMap<u64, PresetStore>> {
    static STORES: OnceLock<Mutex<BTreeMap<u64, PresetStore>>> = OnceLock::new();
    STORES.get_or_init(Mutex::default)
}

fn lock_stores() -> Result<std::sync::MutexGuard<'static, BTreeMap<u64, PresetStore>>, McpToolError>
{
    stores().lock().map_err(|_| {
        McpToolError::new(
            "lock_poisoned",
            "fx preset table is poisoned; restart the door",
        )
    })
}

/// Run `f` on this session's preset store under the audited dispatch seam.
fn with_store<R>(
    server: &McpServer,
    tool: &str,
    f: impl FnOnce(&mut PresetStore) -> Result<R, McpToolError>,
) -> Result<R, McpToolError> {
    server.dispatch(tool, |state| {
        let mut stores = lock_stores()?;
        let store = match stores.entry(state.session()) {
            MapEntry::Occupied(entry) => entry.into_mut(),
            MapEntry::Vacant(entry) => entry.insert(PresetStore::default()),
        };
        f(store)
    })
}

/// Narrow an integer into `i32`, saturating at the ends.
fn narrow_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// One seed word as -1000..=1000.
fn signed(word: u64) -> i64 {
    i64::try_from(word % 2_001).unwrap_or(0) - 1_000
}

fn lane(digest: u64, index: u32, salt: u8) -> u64 {
    let mut hasher = Fnv64::with_state(digest);
    hasher.write_u32(index);
    hasher.write_u8(salt);
    hasher.finish()
}

/// One particle's integer launch: velocity in mm per tick and size in mm.
struct Launch {
    velocity_mm_tick: [i32; 3],
    size_mm: u32,
}

/// The launch of particle `index`, seeded by the preset digest. The vertical
/// launch is zero, so gravity alone owns the fall.
fn launch(digest: u64, index: u32, preset: &EmitterPreset) -> Launch {
    let spread = i64::from(preset.spread_deg);
    let drift = |salt: u8| signed(lane(digest, index, salt)) * spread / 10_000;
    let jitter = i64::try_from(lane(digest, index, 3) % 401).unwrap_or(0) + 800;
    let size = i64::from(preset.size_cm) * 10 * jitter / 1_000;
    Launch {
        velocity_mm_tick: [narrow_i32(drift(1)), 0, narrow_i32(drift(2))],
        size_mm: u32::try_from(size).unwrap_or(1),
    }
}

/// The tick particle `index` leaves the emitter; `u32::MAX` never emits.
fn launch_tick(preset: &EmitterPreset, index: u32) -> u32 {
    let rate = u32::from(preset.rate_per_100_ticks);
    if rate == 0 {
        return u32::MAX;
    }
    let spacing = (100 / rate).max(1);
    index.saturating_mul(spacing)
}

/// The age of particle `index` at `tick`, or `None` when it is not alive.
fn age_at(preset: &EmitterPreset, index: u32, tick: u32) -> Option<u32> {
    let start = launch_tick(preset, index);
    if tick < start {
        return None;
    }
    let age = tick - start;
    match preset.loop_ticks {
        Some(period) => Some(age % u32::from(period).max(1)),
        None => (age < u32::from(preset.life_ticks)).then_some(age),
    }
}

/// The particle offset at `age`, in mm: lateral drift plus integer gravity
/// over the fixed tick rate.
fn offset_mm(preset: &EmitterPreset, launch: &Launch, age: u32) -> [i32; 3] {
    let age = i64::from(age);
    let hz = i64::from(engine_core::TICK_HZ_U32);
    let fall = i64::from(preset.gravity_mm_s2) * age * (age + 1) / (2 * hz * hz);
    [
        narrow_i32(i64::from(launch.velocity_mm_tick[0]) * age),
        narrow_i32(fall),
        narrow_i32(i64::from(launch.velocity_mm_tick[2]) * age),
    ]
}

/// Sample the first [`PREVIEW_PARTICLES`] particles at every named tick.
fn samples(preset: &EmitterPreset, ticks: &[u32]) -> Vec<ParticleSample> {
    let digest = preset.digest();
    let per_tick = usize::try_from(PREVIEW_PARTICLES).unwrap_or(0);
    let mut out = Vec::with_capacity(ticks.len() * per_tick);
    for &tick in ticks {
        for index in 0..PREVIEW_PARTICLES {
            let launch = launch(digest, index, preset);
            let (offset_mm, size_mm, alive) = match age_at(preset, index, tick) {
                Some(age) => (offset_mm(preset, &launch, age), launch.size_mm, true),
                None => ([0; 3], 0, false),
            };
            out.push(ParticleSample {
                tick,
                index,
                offset_mm,
                size_mm,
                alive,
            });
        }
    }
    out
}

/// Resolve the preview's preset: an inline one wins, else the named one.
fn resolve(
    store: &PresetStore,
    name: &Option<String>,
    inline: &Option<EmitterPreset>,
) -> Result<(String, EmitterPreset), McpToolError> {
    if let Some(preset) = inline {
        preset.check()?;
        return Ok((preset.name.clone(), preset.clone()));
    }
    let Some(key) = name else {
        return Err(bad(
            "pass `name` (a shipped or session preset) or `preset` (its own numbers): \
             fx.preset.list names the legal set"
                .to_owned(),
        ));
    };
    match store.find(key) {
        Some(preset) => Ok((key.clone(), preset)),
        None => Err(bad(format!(
            "preset `{key}` is unknown; legal names: [{}]. Next: name one of those, or pass an \
             inline `preset` with life_ticks {LIFE_TICKS_MIN}..={LIFE_TICKS_MAX} and size_cm \
             {SIZE_CM_MIN}..={SIZE_CM_MAX}",
            store.names().join(", ")
        ))),
    }
}

/// One listed preset: its name, digest, whether it ships, and its numbers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct EmitterPresetRow {
    /// Stored name.
    pub name: String,
    /// 16 hex chars over the name and numbers.
    pub digest: String,
    /// True for one of the shipped presets.
    pub builtin: bool,
    /// The preset's own numbers.
    pub preset: EmitterPreset,
}

impl EmitterPresetRow {
    /// One row for a preset.
    fn of(preset: EmitterPreset, builtin: bool) -> Self {
        Self {
            name: preset.name.clone(),
            digest: preset.digest_hex(),
            builtin,
            preset,
        }
    }
}

/// Args for `fx.preset.list`: none.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "a fieldless door argument DTO keeps the brace form its sibling DTOs use, so its JSON schema stays an object"
)]
pub struct FxPresetListArgs {}

/// What `fx.preset.list` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetListOutput {
    /// Rows the list holds.
    pub count: u32,
    /// Every preset, name-sorted.
    pub presets: Vec<EmitterPresetRow>,
}

/// Args for `fx.preset.preview`: which preset and which ticks to sample.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetPreviewArgs {
    /// Preset name: a shipped one (`blood`, `dust`, `sparks`) or one this session set.
    #[serde(default)]
    pub name: Option<String>,
    /// Inline preset; wins over `name` and passes the same checks.
    #[serde(default)]
    pub preset: Option<EmitterPreset>,
    /// Ticks to sample, 1..=16 entries; 0 is the launch tick.
    pub ticks: Vec<u32>,
}

/// One particle sample: where the particle is, how big, and whether it lives.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ParticleSample {
    /// The sampled tick.
    pub tick: u32,
    /// Particle index, 0..=7 in launch order.
    pub index: u32,
    /// Offset from the emitter in millimeters; zero when not alive.
    pub offset_mm: [i32; 3],
    /// Particle diameter in millimeters; zero when not alive.
    pub size_mm: u32,
    /// True when the particle exists at this tick.
    pub alive: bool,
}

/// What `fx.preset.preview` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetPreviewOutput {
    /// The preset that was sampled.
    pub preset: String,
    /// 16 hex chars over the name and numbers.
    pub digest: String,
    /// Samples in tick order, then particle index.
    pub samples: Vec<ParticleSample>,
}

/// Args for `fx.preset.set`: the preset and whether it may replace one.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetSetArgs {
    /// Preset to store, checked field by field.
    pub preset: EmitterPreset,
    /// True updates a stored preset of the same name; false refuses it.
    #[serde(default)]
    pub replace: bool,
}

/// What `fx.preset.set` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetSetOutput {
    /// The stored name.
    pub name: String,
    /// 16 hex chars over the name and numbers.
    pub digest: String,
    /// Presets this session holds after the call.
    pub stored: u32,
    /// True when it replaced an earlier preset of the same name.
    pub replaced: bool,
}

/// Args for `fx.preset.clear`: the session preset to remove.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetClearArgs {
    /// Stored preset name to remove.
    pub name: String,
}

/// What `fx.preset.clear` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct FxPresetClearOutput {
    /// The removed name.
    pub name: String,
    /// True when a preset was removed.
    pub removed: bool,
    /// Presets this session holds after the call.
    pub stored: u32,
}

fn preset_list(
    server: &McpServer,
    _args: &FxPresetListArgs,
) -> Result<FxPresetListOutput, McpToolError> {
    with_store(server, TOOL_FX_PRESET_LIST, |store| {
        let presets = store.rows();
        Ok(FxPresetListOutput {
            count: count(presets.len()),
            presets,
        })
    })
}

fn preset_preview(
    server: &McpServer,
    args: &FxPresetPreviewArgs,
) -> Result<FxPresetPreviewOutput, McpToolError> {
    if args.ticks.is_empty() || args.ticks.len() > MAX_PREVIEW_TICKS {
        return Err(bad(format!(
            "ticks holds {} entries; pass 1..={MAX_PREVIEW_TICKS} tick numbers, such as [0, 4, 8]",
            args.ticks.len()
        )));
    }
    with_store(server, TOOL_FX_PRESET_PREVIEW, |store| {
        let (preset_name, preset) = resolve(store, &args.name, &args.preset)?;
        let digest = preset.digest_hex();
        Ok(FxPresetPreviewOutput {
            preset: preset_name,
            digest,
            samples: samples(&preset, &args.ticks),
        })
    })
}

fn preset_set(
    server: &McpServer,
    args: &FxPresetSetArgs,
) -> Result<FxPresetSetOutput, McpToolError> {
    with_store(server, TOOL_FX_PRESET_SET, |store| {
        let (name, stored, replaced) = store.set(args.preset.clone(), args.replace)?;
        Ok(FxPresetSetOutput {
            digest: args.preset.digest_hex(),
            name,
            stored,
            replaced,
        })
    })
}

fn preset_clear(
    server: &McpServer,
    args: &FxPresetClearArgs,
) -> Result<FxPresetClearOutput, McpToolError> {
    with_store(server, TOOL_FX_PRESET_CLEAR, |store| {
        let (name, removed, stored) = store.clear(&args.name)?;
        Ok(FxPresetClearOutput {
            name,
            removed,
            stored,
        })
    })
}

crate::e2_tool!(
    /// `fx.preset.list`: the shipped and session presets with their digests.
    FxPresetListTool,
    name = TOOL_FX_PRESET_LIST,
    description = "List the shipped emitter presets (blood, dust, sparks) and this session's authored ones, name-sorted, each with its digest. Read-only.",
    readonly = true,
    args = FxPresetListArgs,
    output = FxPresetListOutput,
    invoke = preset_list
);

crate::e2_tool!(
    /// `fx.preset.preview`: integer particle samples at the named ticks.
    FxPresetPreviewTool,
    name = TOOL_FX_PRESET_PREVIEW,
    description = "Sample the first 8 particles of a preset (by name or inline) at 1..=16 ticks; answers offset_mm, size_mm and alive per sample with the preset digest. Read-only and deterministic.",
    readonly = true,
    args = FxPresetPreviewArgs,
    output = FxPresetPreviewOutput,
    invoke = preset_preview
);

crate::e2_tool!(
    /// `fx.preset.set`: store one inline emitter preset.
    FxPresetSetTool,
    name = TOOL_FX_PRESET_SET,
    description = "Store one checked emitter preset under its lower_snake name in this session, at most 32; a stored name needs replace: true. Answers the name, digest and stored count.",
    readonly = false,
    args = FxPresetSetArgs,
    output = FxPresetSetOutput,
    invoke = preset_set
);

crate::e2_tool!(
    /// `fx.preset.clear`: remove one session preset.
    FxPresetClearTool,
    name = TOOL_FX_PRESET_CLEAR,
    description = "Remove one session emitter preset by name; a name the session does not hold refuses bad_args naming the stored set. Answers the name and stored count.",
    readonly = false,
    args = FxPresetClearArgs,
    output = FxPresetClearOutput,
    invoke = preset_clear
);

/// Add the four preset verbs to the `fx` pack, in listing order.
pub fn register(pack: &mut PackDef) -> Result<(), PackError> {
    pack.add(VerbSpec::of::<FxPresetListTool>(
        TOOL_FX_PRESET_LIST,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<FxPresetPreviewTool>(
        TOOL_FX_PRESET_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<FxPresetSetTool>(
        TOOL_FX_PRESET_SET,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<FxPresetClearTool>(
        TOOL_FX_PRESET_CLEAR,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(())
}

#[cfg(test)]
mod preset_verbs {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server(session: u64) -> McpServer {
        McpServer::new(session, Capability::unrestricted(session))
    }

    fn inline(name: &str) -> EmitterPreset {
        DUST.named(name)
    }

    fn set_args(name: &str, replace: bool) -> FxPresetSetArgs {
        FxPresetSetArgs {
            preset: inline(name),
            replace,
        }
    }

    fn preview(server: &McpServer, name: &str, ticks: Vec<u32>) -> FxPresetPreviewOutput {
        FxPresetPreviewTool::invoke(
            server,
            FxPresetPreviewArgs {
                name: Some(name.to_owned()),
                preset: None,
                ticks,
            },
        )
        .expect("preview serves")
    }

    #[test]
    fn a_preview_is_deterministic_and_gravity_pulls_a_spark_down() {
        let server = server(81);
        let ticks = vec![0, 1, 2, 3, 4, 5];
        let first = preview(&server, "sparks", ticks.clone());
        let again = preview(&server, "sparks", ticks.clone());
        assert_eq!(first, again, "same input, same digest and samples");
        assert_eq!(first.samples.len(), ticks.len() * 8);
        let mut fall = Vec::new();
        for tick in &ticks {
            let sample = first
                .samples
                .iter()
                .find(|sample| sample.tick == *tick && sample.index == 0)
                .expect("particle 0 samples every tick");
            assert!(sample.alive, "a spark lives 18 ticks");
            fall.push(sample.offset_mm[1]);
        }
        assert!(
            fall.windows(2).all(|pair| pair[0] > pair[1]),
            "the y offset falls every tick: {fall:?}"
        );
        assert!(fall[0] == 0, "it starts at the emitter: {fall:?}");
    }

    #[test]
    fn the_three_shipped_presets_are_valid_and_distinct() {
        let shipped = builtins();
        assert_eq!(shipped.len(), 3);
        let names: Vec<&str> = shipped.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(names, vec!["blood", "dust", "sparks"], "name-sorted");
        for preset in &shipped {
            preset.check().expect("a shipped preset is in band");
        }
        let digests: Vec<String> = shipped.iter().map(EmitterPreset::digest_hex).collect();
        assert_eq!(
            digests
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "digests differ: {digests:?}"
        );
        assert!(BLOOD.named("blood").decal, "blood leaves a decal");
        assert!(!DUST.named("dust").decal, "dust leaves none");
        assert!(!SPARKS.named("sparks").decal, "sparks leave none");
    }

    #[test]
    fn a_duplicate_name_needs_replace() {
        let server = server(82);
        let stored = FxPresetSetTool::invoke(&server, set_args("muzzle_flash", false))
            .expect("the first set stores");
        assert_eq!(
            (stored.name.as_str(), stored.stored, stored.replaced),
            ("muzzle_flash", 1, false)
        );
        let refused = FxPresetSetTool::invoke(&server, set_args("muzzle_flash", false))
            .expect_err("a stored name refuses without replace");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains("muzzle_flash"),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains("replace: true"),
            "{}",
            refused.message
        );
        let replaced = FxPresetSetTool::invoke(&server, set_args("muzzle_flash", true))
            .expect("replace stores over it");
        assert!(replaced.replaced);
        assert_eq!(replaced.stored, 1, "one name, one slot");
    }

    #[test]
    fn the_store_refuses_the_thirty_third_preset() {
        let server = server(83);
        for index in 0..MAX_SESSION_PRESETS {
            let name = format!("emitter_{index}");
            FxPresetSetTool::invoke(&server, set_args(&name, false)).expect("under the cap");
        }
        let refused = FxPresetSetTool::invoke(&server, set_args("emitter_over", false))
            .expect_err("the cap refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(
            refused.message.contains(&MAX_SESSION_PRESETS.to_string()),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains("fx.preset.clear"),
            "{}",
            refused.message
        );
    }

    #[test]
    fn unknown_names_and_out_of_band_fields_are_refused() {
        let server = server(84);
        let unknown = FxPresetPreviewTool::invoke(
            &server,
            FxPresetPreviewArgs {
                name: Some("nope".to_owned()),
                preset: None,
                ticks: vec![0],
            },
        )
        .expect_err("an unknown preset refuses");
        assert_eq!(unknown.code, "bad_args");
        assert!(unknown.message.contains("nope"), "{}", unknown.message);
        assert!(
            unknown.message.contains("blood, dust, sparks"),
            "{}",
            unknown.message
        );
        let mut broken = inline("too_short");
        broken.life_ticks = 0;
        let life = FxPresetPreviewTool::invoke(
            &server,
            FxPresetPreviewArgs {
                name: None,
                preset: Some(broken.clone()),
                ticks: vec![0],
            },
        )
        .expect_err("life_ticks 0 refuses");
        assert_eq!(life.code, "bad_args");
        assert!(life.message.contains("life_ticks"), "{}", life.message);
        assert!(life.message.contains("1..=600"), "{}", life.message);
        broken.life_ticks = DUST.life_ticks;
        broken.gravity_mm_s2 = GRAVITY_MM_S2_MAX + 1;
        let gravity = FxPresetSetTool::invoke(
            &server,
            FxPresetSetArgs {
                preset: broken,
                replace: false,
            },
        )
        .expect_err("gravity 30001 refuses");
        assert!(
            gravity.message.contains("gravity_mm_s2"),
            "{}",
            gravity.message
        );
        assert!(
            gravity.message.contains("-30000..=30000"),
            "{}",
            gravity.message
        );
        let too_many = FxPresetPreviewTool::invoke(
            &server,
            FxPresetPreviewArgs {
                name: Some("dust".to_owned()),
                preset: None,
                ticks: vec![0; MAX_PREVIEW_TICKS + 1],
            },
        )
        .expect_err("more ticks than the cap refuses");
        assert_eq!(too_many.code, "bad_args");
        assert!(too_many.message.contains("1..=16"), "{}", too_many.message);
        let no_preset = FxPresetPreviewTool::invoke(
            &server,
            FxPresetPreviewArgs {
                name: None,
                preset: None,
                ticks: vec![0],
            },
        )
        .expect_err("no preset refuses");
        assert!(
            no_preset.message.contains("preset"),
            "{}",
            no_preset.message
        );
    }

    #[test]
    fn set_lists_sorts_and_clear_removes() {
        let server = server(85);
        FxPresetSetTool::invoke(&server, set_args("zap", false)).expect("set zap");
        FxPresetSetTool::invoke(&server, set_args("ash", false)).expect("set ash");
        let listed = FxPresetListTool::invoke(&server, FxPresetListArgs {}).expect("list serves");
        assert_eq!(listed.count, 5, "three shipped plus two authored");
        let names: Vec<&str> = listed.presets.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, vec!["ash", "blood", "dust", "sparks", "zap"]);
        assert!(!listed.presets[0].builtin);
        assert!(listed.presets.iter().filter(|row| row.builtin).count() == 3);
        assert_eq!(
            listed.presets[0].digest,
            inline("ash").digest_hex(),
            "the row carries the preset digest"
        );
        let previewed = preview(&server, "ash", vec![0, 3]);
        assert_eq!(previewed.preset, "ash");
        assert_eq!(previewed.digest, inline("ash").digest_hex());
        let cleared = FxPresetClearTool::invoke(
            &server,
            FxPresetClearArgs {
                name: "ash".to_owned(),
            },
        )
        .expect("clear ash");
        assert_eq!(
            (cleared.name.as_str(), cleared.removed, cleared.stored),
            ("ash", true, 1)
        );
        let missing = FxPresetClearTool::invoke(
            &server,
            FxPresetClearArgs {
                name: "ash".to_owned(),
            },
        )
        .expect_err("a cleared name refuses");
        assert!(missing.message.contains("zap"), "{}", missing.message);
    }
}
