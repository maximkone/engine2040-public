//! ui: retained HUD data lane. HUD is data; renderer draws from snapshots.
//!
//! Consumer: the HUD renderer plus the accessibility tree. Data owner:
//! `engine-ui`. Units: health/stamina are permille (`0..=1000`), kill-feed
//! ticks are fixed steps, minimap cells are quantized arena units, palette
//! colors are sRGB `[u8; 3]` triads. Determinism: every update is a pure
//! function on an owned struct; fighters keep insertion order, the feed is
//! oldest-first, and hashed state uses integers only. No IO, no wall clock.
//! Failure mode: invalid input returns [`HudError`]; updates never panic.

use engine_core::Fnv64;
use engine_input::{Buttons, FighterId, MAX_PARTICIPANTS};

pub mod access;
/// Main/pause menu state machine: pure transitions, fn-pointer mode hooks.
/// Std-only; the windowed loop drives it.
/// LAN lobby screen model: host list, join, ready, start, no typed address.
pub mod lobby_ui;
pub mod menus;
/// Play-frame HUD snapshot: button echo and hints, the fighter-you arrow,
/// FPS crosshair, hit flash and ammo as pure data beside [`DuelOverlay`].
pub mod play_hud;
/// Player settings: volume, graphics and bindings as staged batches with a
/// replay tape and undo.
pub mod settings;
pub mod text;
/// Guided first fight: fixed script rows over menu commands, skippable.
pub mod tutorial;

/// Health/stamina scale: permille, `0..=1000`.
pub const PERMILLE_MAX: u16 = 1000;
/// Default kill-feed capacity.
pub const DEFAULT_FEED_CAP: usize = 8;
/// Maximum kill-feed capacity.
pub const MAX_FEED_CAP: usize = 64;
/// Default minimap capacity in dots.
pub const DEFAULT_MAP_CAP: usize = 8;
/// Maximum minimap capacity in dots.
pub const MAX_MAP_CAP: usize = 16;
/// Minimap quantization: cells per arena meter.
pub const CELLS_PER_METER: f32 = 16.0;
/// Minimap half-extent in meters; world positions clamp to `+/-MAP_HALF_METERS`.
pub const MAP_HALF_METERS: f32 = 64.0;
/// Minimum accepted contrast ratio for palette pairs (large-text/UI bound).
pub const MIN_CONTRAST: f32 = 3.0;

/// HUD failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HudError {
    /// Fighter slot `>= MAX_PARTICIPANTS`, culprit carried.
    UnknownFighter(u8),
    /// HUD already tracks `MAX_PARTICIPANTS` fighters, count carried.
    TooManyFighters(usize),
    /// Kill-feed capacity outside `1..=MAX_FEED_CAP`, culprit carried.
    FeedCapOutOfRange(usize),
    /// Minimap capacity outside `1..=MAX_MAP_CAP`, culprit carried.
    MapCapOutOfRange(usize),
    /// Minimap is full and the dot id is not tracked yet.
    MapFull,
    /// Minimap coordinate is NaN or infinite.
    NonFiniteCoordinate,
    /// Palette pair contrast below [`MIN_CONTRAST`], ratio in permille.
    LowContrast { ratio_permille: u32 },
}

impl core::fmt::Display for HudError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFighter(id) => write!(f, "unknown fighter {id}"),
            Self::TooManyFighters(n) => write!(f, "too many fighters ({n})"),
            Self::FeedCapOutOfRange(cap) => write!(f, "feed cap out of range ({cap})"),
            Self::MapCapOutOfRange(cap) => write!(f, "map cap out of range ({cap})"),
            Self::MapFull => write!(f, "minimap full"),
            Self::NonFiniteCoordinate => write!(f, "non-finite coordinate"),
            Self::LowContrast { ratio_permille } => {
                write!(f, "low contrast (ratio permille {ratio_permille})")
            }
        }
    }
}

impl std::error::Error for HudError {}

/// Clamp a permille value to `0..=PERMILLE_MAX`. Pure.
#[must_use]
pub const fn clamp_permille(value: u16) -> u16 {
    if value > PERMILLE_MAX {
        PERMILLE_MAX
    } else {
        value
    }
}

/// Per-fighter HUD row: health/stamina in permille. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FighterHud {
    /// Which fighter this row describes.
    pub id: FighterId,
    /// Health in permille, `0..=1000`.
    pub hp_permille: u16,
    /// Stamina in permille, `0..=1000`.
    pub stamina: u16,
}

impl FighterHud {
    /// Build a row; out-of-range health/stamina clamp to bounds. Pure.
    #[must_use]
    pub const fn new(id: FighterId, hp_permille: u16, stamina: u16) -> Self {
        Self {
            id,
            hp_permille: clamp_permille(hp_permille),
            stamina: clamp_permille(stamina),
        }
    }

    /// Replace health; out-of-range values clamp to bounds. Pure.
    pub fn set_hp(&mut self, hp_permille: u16) {
        self.hp_permille = clamp_permille(hp_permille);
    }

    /// Replace stamina; out-of-range values clamp to bounds. Pure.
    pub fn set_stamina(&mut self, stamina: u16) {
        self.stamina = clamp_permille(stamina);
    }

    /// Reject rows that name a slot outside `engine-input`.
    pub fn validate(&self) -> Result<(), HudError> {
        if self.id.get() >= MAX_PARTICIPANTS {
            return Err(HudError::UnknownFighter(self.id.get()));
        }
        Ok(())
    }
}

/// How a kill happened. Pure label; balancing lives elsewhere.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KillCause {
    /// Direct strike.
    Strike,
    /// Ranged hit.
    Projectile,
    /// Lost to the arena (fall / ring-out).
    Fall,
    /// Arena hazard.
    Hazard,
}

impl KillCause {
    /// One-byte code fed to [`Hud::state_hash`]; gate digests depend on these
    /// values, so a variant never changes its code.
    const fn code(self) -> u8 {
        match self {
            Self::Strike => 0,
            Self::Projectile => 1,
            Self::Fall => 2,
            Self::Hazard => 3,
        }
    }
}

/// One kill-feed entry. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KillEntry {
    /// Fixed tick the kill happened on.
    pub tick: u64,
    /// Killer, or `None` for world kills.
    pub killer: Option<FighterId>,
    /// Who died.
    pub victim: FighterId,
    /// How it happened.
    pub cause: KillCause,
}

impl KillEntry {
    /// Reject entries that name a slot outside `engine-input`. Pure.
    pub fn validate(&self) -> Result<(), HudError> {
        if let Some(killer) = self.killer {
            if killer.get() >= MAX_PARTICIPANTS {
                return Err(HudError::UnknownFighter(killer.get()));
            }
        }
        if self.victim.get() >= MAX_PARTICIPANTS {
            return Err(HudError::UnknownFighter(self.victim.get()));
        }
        Ok(())
    }
}

/// Bounded kill feed, oldest-first; overflow drops the oldest. Pure data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KillFeed {
    entries: Vec<KillEntry>,
    cap: usize,
}

impl KillFeed {
    /// Empty feed holding at most `cap` entries (`1..=MAX_FEED_CAP`).
    pub fn new(cap: usize) -> Result<Self, HudError> {
        if cap == 0 || cap > MAX_FEED_CAP {
            return Err(HudError::FeedCapOutOfRange(cap));
        }
        Ok(Self {
            entries: Vec::new(),
            cap,
        })
    }

    /// Capacity in entries.
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Entry count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Oldest-first entries.
    pub fn iter(&self) -> core::slice::Iter<'_, KillEntry> {
        self.entries.iter()
    }

    /// Oldest entry, if any.
    #[must_use]
    pub fn oldest(&self) -> Option<KillEntry> {
        self.entries.first().copied()
    }

    /// Newest entry, if any.
    #[must_use]
    pub fn latest(&self) -> Option<KillEntry> {
        self.entries.last().copied()
    }

    /// Push an entry; a full feed drops the oldest first. Pure.
    pub fn push(&mut self, entry: KillEntry) -> Result<(), HudError> {
        entry.validate()?;
        if self.entries.len() >= self.cap {
            // `cap >= 1` and `len >= cap`, so the vec is non-empty.
            self.entries.remove(0);
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Drop every entry. Pure.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Default for KillFeed {
    fn default() -> Self {
        // `DEFAULT_FEED_CAP` is a valid cap by construction.
        Self {
            entries: Vec::new(),
            cap: DEFAULT_FEED_CAP,
        }
    }
}

/// One minimap dot: fighter plus quantized cells. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MinimapDot {
    /// Which fighter the dot marks.
    pub id: FighterId,
    /// Quantized x cell.
    pub x: i16,
    /// Quantized z cell.
    pub z: i16,
}

/// Bounded minimap: fighter dots on a quantized grid. Pure data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinimapLite {
    dots: Vec<MinimapDot>,
    cap: usize,
}

impl MinimapLite {
    /// Empty map holding at most `cap` dots (`1..=MAX_MAP_CAP`).
    pub fn new(cap: usize) -> Result<Self, HudError> {
        if cap == 0 || cap > MAX_MAP_CAP {
            return Err(HudError::MapCapOutOfRange(cap));
        }
        Ok(Self {
            dots: Vec::new(),
            cap,
        })
    }

    /// Capacity in dots.
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Dot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.dots.len()
    }

    /// True when no dots are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.dots.is_empty()
    }

    /// Stored dots in insertion order.
    pub fn iter(&self) -> core::slice::Iter<'_, MinimapDot> {
        self.dots.iter()
    }

    /// Dot for a fighter, if tracked.
    #[must_use]
    pub fn get(&self, id: FighterId) -> Option<MinimapDot> {
        self.dots.iter().find(|dot| dot.id == id).copied()
    }

    /// Quantize world meters to cells: clamp to the arena half-extent, then
    /// scale. Finite inputs only; `upsert` rejects NaN/infinite first. Pure.
    #[must_use]
    pub fn quantize(value_m: f32) -> i16 {
        let clamped = value_m.clamp(-MAP_HALF_METERS, MAP_HALF_METERS);
        (clamped * CELLS_PER_METER)
            .round()
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
    }

    /// Insert or move a dot; world positions clamp to the arena. Pure.
    pub fn upsert(&mut self, id: FighterId, x_m: f32, z_m: f32) -> Result<(), HudError> {
        if id.get() >= MAX_PARTICIPANTS {
            return Err(HudError::UnknownFighter(id.get()));
        }
        if !x_m.is_finite() || !z_m.is_finite() {
            return Err(HudError::NonFiniteCoordinate);
        }
        let dot = MinimapDot {
            id,
            x: Self::quantize(x_m),
            z: Self::quantize(z_m),
        };
        if let Some(slot) = self.dots.iter_mut().find(|slot| slot.id == id) {
            *slot = dot;
        } else {
            if self.dots.len() >= self.cap {
                return Err(HudError::MapFull);
            }
            self.dots.push(dot);
        }
        Ok(())
    }

    /// Stop tracking a fighter; missing ids are a no-op. Pure.
    pub fn remove(&mut self, id: FighterId) {
        self.dots.retain(|dot| dot.id != id);
    }

    /// Drop every dot. Pure.
    pub fn clear(&mut self) {
        self.dots.clear();
    }
}

impl Default for MinimapLite {
    fn default() -> Self {
        // `DEFAULT_MAP_CAP` is a valid cap by construction.
        Self {
            dots: Vec::new(),
            cap: DEFAULT_MAP_CAP,
        }
    }
}

/// Linearize one sRGB channel with a square gamma stand-in: deterministic,
/// integer-friendly, monotonic — enough to rank palette pairs. Pure.
fn linear_channel(channel: u8) -> f32 {
    let unit = f32::from(channel) / 255.0;
    unit * unit
}

/// Relative luminance under the square-gamma stand-in. Pure.
fn luminance(rgb: [u8; 3]) -> f32 {
    0.2126 * linear_channel(rgb[0])
        + 0.7152 * linear_channel(rgb[1])
        + 0.0722 * linear_channel(rgb[2])
}

/// Contrast ratio between two sRGB colors, `>= 1.0`. Pure.
#[must_use]
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (hi, lo) = {
        let (la, lb) = (luminance(a), luminance(b));
        if la >= lb {
            (la, lb)
        } else {
            (lb, la)
        }
    };
    (hi + 0.05) / (lo + 0.05)
}

/// Named HUD color set: background, foreground and accent as sRGB triads.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Palette {
    /// Palette name shown in options.
    pub name: &'static str,
    /// Background triad.
    pub bg: [u8; 3],
    /// Foreground (text/bar) triad.
    pub fg: [u8; 3],
    /// Accent (warnings, pings) triad.
    pub accent: [u8; 3],
}

impl Palette {
    /// Default set: near-black background, light foreground, amber accent.
    #[must_use]
    pub const fn default_palette() -> Self {
        Self {
            name: "default",
            bg: [16, 18, 24],
            fg: [245, 245, 245],
            accent: [255, 170, 0],
        }
    }

    /// Deuteranopia-safe set: sky-blue accent on black.
    #[must_use]
    pub const fn deuteranopia() -> Self {
        Self {
            name: "deuteranopia",
            bg: [0, 0, 0],
            fg: [255, 255, 255],
            accent: [86, 180, 233],
        }
    }

    /// Protanopia-safe set: yellow accent on near-black.
    #[must_use]
    pub const fn protanopia() -> Self {
        Self {
            name: "protanopia",
            bg: [10, 10, 12],
            fg: [250, 250, 250],
            accent: [240, 228, 66],
        }
    }

    /// Tritanopia-safe set: bluish-green accent on near-black.
    #[must_use]
    pub const fn tritanopia() -> Self {
        Self {
            name: "tritanopia",
            bg: [12, 12, 14],
            fg: [255, 255, 255],
            accent: [0, 158, 115],
        }
    }

    /// Check foreground/background and accent/background pairs against
    /// [`MIN_CONTRAST`]. Pure.
    pub fn validate(&self) -> Result<(), HudError> {
        for (a, b) in [(self.fg, self.bg), (self.accent, self.bg)] {
            let ratio = contrast_ratio(a, b);
            if ratio < MIN_CONTRAST {
                return Err(HudError::LowContrast {
                    ratio_permille: (ratio * 1000.0) as u32,
                });
            }
        }
        Ok(())
    }
}

/// HUD snapshot: chip bars, pooled numbers, feed. All ints, no floats in hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hud {
    pub tick: u64,
    pub hp: u32,
    pub max_hp: u32,
    pub ammo: u32,
    pub score: u64,
    pub kill_feed: u32,
    /// Per-fighter health/stamina rows, insertion order.
    pub fighters: Vec<FighterHud>,
    /// Bounded kill feed, oldest-first.
    pub feed: KillFeed,
    /// Bounded minimap dots.
    pub map: MinimapLite,
}

/// Boss bar + combo + trauma (0..100 fixed, no f32 in hash path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Juice {
    pub boss_hp: u32,
    pub combo: u32,
    pub trauma: u32,
}

impl Hud {
    /// Snapshot with no fighter rows, feed entries or minimap dots; `max_hp`
    /// is at least 1 and `hp` clamps to it. Pure.
    pub fn new(tick: u64, hp: u32, max_hp: u32, ammo: u32, score: u64) -> Self {
        Self {
            tick,
            hp: hp.min(max_hp.max(1)),
            max_hp: max_hp.max(1),
            ammo,
            score,
            kill_feed: 0,
            fighters: Vec::new(),
            feed: KillFeed::default(),
            map: MinimapLite::default(),
        }
    }

    /// Insert or replace a fighter row; at most `MAX_PARTICIPANTS` rows. Pure.
    pub fn upsert_fighter(&mut self, fighter: FighterHud) -> Result<(), HudError> {
        fighter.validate()?;
        if let Some(slot) = self.fighters.iter_mut().find(|row| row.id == fighter.id) {
            *slot = fighter;
        } else {
            if self.fighters.len() >= usize::from(MAX_PARTICIPANTS) {
                return Err(HudError::TooManyFighters(self.fighters.len()));
            }
            self.fighters.push(fighter);
        }
        Ok(())
    }

    /// Fighter row, if tracked. Pure.
    #[must_use]
    pub fn fighter(&self, id: FighterId) -> Option<FighterHud> {
        self.fighters.iter().find(|row| row.id == id).copied()
    }

    /// Replace a tracked fighter's health (clamped); unknown ids fail. Pure.
    pub fn set_hp(&mut self, id: FighterId, hp_permille: u16) -> Result<(), HudError> {
        if let Some(row) = self.fighters.iter_mut().find(|row| row.id == id) {
            row.set_hp(hp_permille);
            Ok(())
        } else {
            Err(HudError::UnknownFighter(id.get()))
        }
    }

    /// Replace a tracked fighter's stamina (clamped); unknown ids fail. Pure.
    pub fn set_stamina(&mut self, id: FighterId, stamina: u16) -> Result<(), HudError> {
        if let Some(row) = self.fighters.iter_mut().find(|row| row.id == id) {
            row.set_stamina(stamina);
            Ok(())
        } else {
            Err(HudError::UnknownFighter(id.get()))
        }
    }

    /// Record a kill in the feed and bump the legacy counter. Pure.
    pub fn push_kill(&mut self, entry: KillEntry) -> Result<(), HudError> {
        self.feed.push(entry)?;
        self.kill_feed = self.kill_feed.saturating_add(1);
        Ok(())
    }

    /// Insert or move a minimap dot. Pure.
    pub fn upsert_dot(&mut self, id: FighterId, x_m: f32, z_m: f32) -> Result<(), HudError> {
        self.map.upsert(id, x_m, z_m)
    }

    /// Chip-damage bar 0..1000: damage lags, heal snaps. Pure.
    pub fn chip(&self, shown: u32) -> u32 {
        if shown > self.hp {
            shown.saturating_sub(50).max(self.hp)
        } else {
            self.hp
        }
    }

    /// FNV-1a64 over ints (gallery screenshot and accessibility-tree gate
    /// input). Counters are widened to eight bytes so the stream layout never
    /// depends on a field's width.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(self.tick);
        h.write_u64(u64::from(self.hp));
        h.write_u64(u64::from(self.max_hp));
        h.write_u64(u64::from(self.ammo));
        h.write_u64(self.score);
        h.write_u64(u64::from(self.kill_feed));
        for row in &self.fighters {
            h.write_u8(row.id.get());
            h.write_u16(row.hp_permille);
            h.write_u16(row.stamina);
        }
        for entry in self.feed.iter() {
            h.write_u64(entry.tick);
            h.write_u8(entry.killer.map_or(u8::MAX, FighterId::get));
            h.write_u8(entry.victim.get());
            h.write_u8(entry.cause.code());
        }
        for dot in self.map.iter() {
            h.write_u8(dot.id.get());
            h.write(&dot.x.to_le_bytes());
            h.write(&dot.z.to_le_bytes());
        }
        h.finish()
    }
}

/// Duel overlay: names, HP pips, tick and attacker/victim flash.
///
/// Consumer: the `play.step` rasterizer, which decodes `from_bytes` and paints
/// names with `text::layout`, HP with `hp_pips`, the tick header and the hit
/// flash (the endpoint call site is frozen; the in-crate `FrozenFrame` stub in
/// tests pins the draw contract). Data owner: `engine-ui`. Units: ticks are
/// fixed steps, health is permille (`0..=1000`), pips count `0..=MAX_HP_PIPS`
/// blocks, palette ids index the four colorblind-safe `Palette` sets. Determinism: rows
/// keep insertion order, the wire is little-endian with length prefixes, and
/// `from_seed` builds identical bytes from one seed. Budgets: at most
/// `MAX_OVERLAY_ROWS` rows, names at most `MAX_NAME_LEN` bytes, frames at most
/// `OVERLAY_MAX_BYTES` bytes. Failures return `OverlayError`.
pub const OVERLAY_VERSION: u8 = 1;
/// Most fighters one overlay names; matches the `engine-input` slot cap.
pub const MAX_OVERLAY_ROWS: usize = 16;
/// Longest fighter name, in UTF-8 bytes.
pub const MAX_NAME_LEN: usize = 24;
/// HP bar resolution: full health lights every pip.
pub const MAX_HP_PIPS: u8 = 10;
/// Ticks a hit flash stays visible after the hit tick, inclusive.
pub const FLASH_TICKS: u64 = 30;
/// Largest encoded overlay, in bytes: version (1) + tick (8) + palette (1) +
/// attacker (1) + victim (1) + hit tick (8) + row count (1) + one full row
/// (id 1 + name length 1 + `MAX_NAME_LEN` + health 2) per slot.
pub const OVERLAY_MAX_BYTES: usize = 21 + MAX_OVERLAY_ROWS * 28;

/// Missing-hit sentinel on the wire; no fighter slot reaches `u8::MAX`.
const NO_HIT: u8 = u8::MAX;

/// Duel overlay failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum OverlayError {
    /// Fighter slot `>= MAX_PARTICIPANTS`, culprit carried.
    UnknownFighter(u8),
    /// Fighter name is empty.
    EmptyName,
    /// Fighter name exceeds `MAX_NAME_LEN` bytes, culprit carried.
    NameTooLong(usize),
    /// Fighter name is not valid UTF-8 or holds a control character.
    BadName,
    /// Health outside `0..=PERMILLE_MAX`, culprit carried.
    HpOutOfRange(u16),
    /// Palette id outside `0..=3`, culprit carried.
    BadPalette(u8),
    /// Row count outside `0..=MAX_OVERLAY_ROWS`, culprit carried.
    TooManyRows(usize),
    /// Same fighter slot twice on the wire, culprit carried.
    DuplicateFighter(u8),
    /// Wire version is not `OVERLAY_VERSION`, culprit carried.
    BadVersion(u8),
    /// Byte slice ends mid-field.
    UnexpectedEnd,
    /// Bytes trail the decoded overlay, remainder carried.
    TrailingBytes(usize),
}

impl core::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFighter(id) => write!(f, "unknown fighter {id}"),
            Self::EmptyName => write!(f, "empty fighter name"),
            Self::NameTooLong(len) => write!(f, "fighter name too long ({len} bytes)"),
            Self::BadName => write!(f, "invalid fighter name"),
            Self::HpOutOfRange(hp) => write!(f, "health out of range ({hp})"),
            Self::BadPalette(id) => write!(f, "unknown palette ({id})"),
            Self::TooManyRows(n) => write!(f, "too many overlay rows ({n})"),
            Self::DuplicateFighter(id) => write!(f, "duplicate fighter ({id})"),
            Self::BadVersion(version) => write!(f, "bad overlay version ({version})"),
            Self::UnexpectedEnd => write!(f, "truncated overlay bytes"),
            Self::TrailingBytes(n) => write!(f, "trailing overlay bytes ({n})"),
        }
    }
}

impl std::error::Error for OverlayError {}

/// Reject slots outside `engine-input`; shared by rows and hit pairs. Pure.
fn check_overlay_id(id: FighterId) -> Result<(), OverlayError> {
    if id.get() >= MAX_PARTICIPANTS {
        return Err(OverlayError::UnknownFighter(id.get()));
    }
    Ok(())
}

/// Reject empty, overlong or control-carrying names; UTF-8 is already proven
/// by the `&str` type on input and by `from_utf8` on decode. Pure.
fn check_overlay_name(name: &str) -> Result<(), OverlayError> {
    if name.is_empty() {
        return Err(OverlayError::EmptyName);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(OverlayError::NameTooLong(name.len()));
    }
    if name.chars().any(|glyph| glyph.is_control()) {
        return Err(OverlayError::BadName);
    }
    Ok(())
}

/// Reject health outside permille; constructors clamp instead. Pure.
fn check_overlay_hp(hp_permille: u16) -> Result<(), OverlayError> {
    if hp_permille > PERMILLE_MAX {
        return Err(OverlayError::HpOutOfRange(hp_permille));
    }
    Ok(())
}

/// Health in permille to lit pips, `0..=MAX_HP_PIPS`, halves round up.
/// Counts by thresholds (pip `k` lights at `100 * k - 50`) so the conversion
/// stays total with plain integer math: no casts, no panics. Pure.
#[must_use]
pub fn hp_pips(hp_permille: u16) -> u8 {
    let clamped = clamp_permille(hp_permille);
    let mut pips: u8 = 0;
    let mut threshold: u16 = 50;
    while pips < MAX_HP_PIPS {
        if clamped < threshold {
            break;
        }
        pips += 1;
        threshold += 100;
    }
    pips
}

/// One named fighter row: display name plus permille health. Pure data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayRow {
    /// Which fighter this row names.
    id: FighterId,
    /// Display name, `1..=MAX_NAME_LEN` UTF-8 bytes, no control characters.
    name: String,
    /// Health in permille, `0..=1000`.
    hp_permille: u16,
}

impl OverlayRow {
    /// Which fighter this row names.
    #[must_use]
    pub fn id(&self) -> FighterId {
        self.id
    }

    /// Display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Health in permille, `0..=1000`.
    #[must_use]
    pub fn hp_permille(&self) -> u16 {
        self.hp_permille
    }

    /// Lit HP pips for this row; see [`hp_pips`]. Pure.
    #[must_use]
    pub fn pips(&self) -> u8 {
        hp_pips(self.hp_permille)
    }
}

/// Per-row text-area descriptor: position plus clip for one overlay row.
/// PORT(grovesNL/glyphon@49dc8f7 TextArea) idea only: cells mirror left/top,
/// clip mirrors bounds, flash mirrors default color, wrap mirrors Wrap.
/// Encoder carries it; decoder ignores it, so bytes stay frozen. No new pass.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RowArea {
    /// Left edge in cells from the overlay origin.
    pub left_cells: u32,
    /// Top edge in cells from the overlay origin.
    pub top_cells: u32,
    /// Visible width in cells; layout wraps at `wrap_cells` then clips here.
    pub clip_w_cells: u32,
    /// Visible height in cells (one name line).
    pub clip_h_cells: u32,
    /// Hard-wrap width in cells (bounded layout knob).
    pub wrap_cells: u32,
    /// Flash color: accent while flashing, foreground otherwise.
    pub flash: [u8; 3],
}

/// Named duel frame: per-fighter names and health, the tick it describes and
/// the last attacker/victim pair for the hit flash. Rows keep insertion
/// order; the palette id always names one of the four colorblind-safe sets. Pure data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuelOverlay {
    /// Fixed tick this frame describes.
    tick: u64,
    /// Palette id: `0` default, `1` deuteranopia, `2` protanopia, `3`
    /// tritanopia.
    palette_id: u8,
    /// Named rows in insertion order, at most `MAX_OVERLAY_ROWS`.
    rows: Vec<OverlayRow>,
    /// Last attacker, or `None` when no hit is shown.
    attacker: Option<FighterId>,
    /// Last victim, or `None` when no hit is shown.
    victim: Option<FighterId>,
    /// Fixed tick the recorded hit landed on.
    hit_tick: u64,
}

/// Take one byte, advancing the cursor. Pure.
fn take_overlay_byte(bytes: &[u8], cursor: &mut usize) -> Result<u8, OverlayError> {
    let byte = bytes
        .get(*cursor)
        .copied()
        .ok_or(OverlayError::UnexpectedEnd)?;
    *cursor = cursor.wrapping_add(1);
    Ok(byte)
}

/// Take a little-endian `u16` by shifting, advancing the cursor. Pure.
fn take_overlay_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, OverlayError> {
    let end = cursor.checked_add(2).ok_or(OverlayError::UnexpectedEnd)?;
    let slice = bytes.get(*cursor..end).ok_or(OverlayError::UnexpectedEnd)?;
    *cursor = end;
    let mut value: u16 = 0;
    let mut shift: u32 = 0;
    for byte in slice {
        value |= u16::from(*byte) << shift;
        shift += 8;
    }
    Ok(value)
}

/// Take a little-endian `u64` by shifting, advancing the cursor. Pure.
fn take_overlay_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, OverlayError> {
    let end = cursor.checked_add(8).ok_or(OverlayError::UnexpectedEnd)?;
    let slice = bytes.get(*cursor..end).ok_or(OverlayError::UnexpectedEnd)?;
    *cursor = end;
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for byte in slice {
        value |= u64::from(*byte) << shift;
        shift += 8;
    }
    Ok(value)
}

/// Take one hit slot: the sentinel means no hit, anything else must name a
/// live slot. Pure.
fn take_hit_slot(bytes: &[u8], cursor: &mut usize) -> Result<Option<FighterId>, OverlayError> {
    let slot = take_overlay_byte(bytes, cursor)?;
    if slot == NO_HIT {
        return Ok(None);
    }
    let id = FighterId(slot);
    check_overlay_id(id)?;
    Ok(Some(id))
}

impl DuelOverlay {
    /// Empty overlay for `tick` with the default palette and no hit.
    /// Pure.
    #[must_use]
    pub fn new(tick: u64) -> Self {
        Self {
            tick,
            palette_id: 0,
            rows: Vec::new(),
            attacker: None,
            victim: None,
            hit_tick: 0,
        }
    }

    /// Deterministic two-fighter duel fixture from one seed: same seed, same
    /// bytes. Picks distinct human names, permille health and a still-visible
    /// attacker/victim flash from the seed bits with plain integer math, so
    /// no RNG owner is involved. Pure.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        const NAMES: [&str; 4] = ["Rook", "Sable", "Ivo", "Pax"];
        let mixed: u64 = seed | 1;
        let pick = |bits: u64| -> usize {
            match bits & 3 {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 3,
            }
        };
        let first = pick(mixed);
        let mut second = pick(mixed >> 2);
        if second == first {
            second = (first + 1) & 3;
        }
        // The remainder is below 1001, far inside `u16`; the fallback never
        // runs and keeps the conversion total.
        let hp = |shift: u32| -> u16 {
            let reduced: u64 = (mixed >> shift) % 1001;
            match u16::try_from(reduced) {
                Ok(health) => health,
                Err(_) => PERMILLE_MAX,
            }
        };
        let tick: u64 = (mixed % 600).wrapping_add(600);
        let attacker_slot: u8 = u8::from((mixed >> 32) & 1 != 0);
        let victim_slot: u8 = 1 - attacker_slot;
        let hit_tick: u64 = tick.saturating_sub(mixed % (FLASH_TICKS + 1));
        // Seed tables hold plain ASCII letters inside the name bound, so
        // direct construction skips validation that cannot fail; the hit
        // then goes through the paired path, keeping flash and hp together.
        let mut overlay = Self {
            tick,
            palette_id: 0,
            rows: vec![
                OverlayRow {
                    id: FighterId(0),
                    name: NAMES[first].to_owned(),
                    hp_permille: hp(8),
                },
                OverlayRow {
                    id: FighterId(1),
                    name: NAMES[second].to_owned(),
                    hp_permille: hp(20),
                },
            ],
            attacker: None,
            victim: None,
            hit_tick: 0,
        };
        let victim_hp: u16 = if victim_slot == 0 { hp(8) } else { hp(20) };
        // Seed slots and health pass every check by construction; the
        // fallback keeps these exact bytes in the impossible failure case.
        if overlay
            .apply_first_hit(
                FighterId(attacker_slot),
                FighterId(victim_slot),
                hit_tick,
                victim_hp,
            )
            .is_err()
        {
            overlay.attacker = Some(FighterId(attacker_slot));
            overlay.victim = Some(FighterId(victim_slot));
            overlay.hit_tick = hit_tick;
        }
        overlay
    }

    /// Fixed tick this frame describes.
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Move the frame clock; rows and the recorded hit are untouched. Pure.
    pub fn set_tick(&mut self, tick: u64) {
        self.tick = tick;
    }

    /// Palette id, `0..=3`.
    #[must_use]
    pub fn palette_id(&self) -> u8 {
        self.palette_id
    }

    /// Palette this overlay draws with. Pure.
    /// Single owner: [`crate::access::hud_palettes`] holds the four
    /// gated sets in id order, so this reads that table instead of
    /// repeating the match.
    #[must_use]
    pub fn palette(&self) -> Palette {
        match self.palette_id {
            0..=3 => crate::access::hud_palettes()[usize::from(self.palette_id)],
            _ => Palette::default_palette(),
        }
    }

    /// Select a palette by id (`0` default, `1` deuteranopia, `2`
    /// protanopia, `3` tritanopia); anything else fails. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError::BadPalette`] for ids above `3`.
    pub fn set_palette(&mut self, palette_id: u8) -> Result<(), OverlayError> {
        if palette_id > 3 {
            return Err(OverlayError::BadPalette(palette_id));
        }
        self.palette_id = palette_id;
        Ok(())
    }

    /// Named rows in insertion order.
    #[must_use]
    pub fn rows(&self) -> &[OverlayRow] {
        &self.rows
    }

    /// Row count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no rows are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Row for a fighter, if named. Pure.
    #[must_use]
    pub fn row(&self, id: FighterId) -> Option<&OverlayRow> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// Last attacker, if a hit is shown.
    #[must_use]
    pub fn attacker(&self) -> Option<FighterId> {
        self.attacker
    }

    /// Last victim, if a hit is shown.
    #[must_use]
    pub fn victim(&self) -> Option<FighterId> {
        self.victim
    }

    /// Fixed tick the recorded hit landed on.
    #[must_use]
    pub fn hit_tick(&self) -> u64 {
        self.hit_tick
    }

    /// Per-row area for `index`: 24-cell wrap plus clip with flash color.
    /// Encoder carries it; `from_bytes` ignores it, so bytes never move. Pure.
    #[must_use]
    pub fn row_area(&self, index: usize) -> Option<RowArea> {
        let row = self.rows.get(index)?;
        let order = u32::try_from(index).unwrap_or(u32::MAX);
        let top = 1u32.saturating_add(order.saturating_mul(2));
        let palette = self.palette();
        let flash = if self.is_flashing(row.id()) {
            palette.accent
        } else {
            palette.fg
        };
        Some(RowArea {
            left_cells: 0,
            top_cells: top,
            clip_w_cells: 24,
            clip_h_cells: 1,
            wrap_cells: 24,
            flash,
        })
    }

    /// Name a fighter or replace its row in place; at most
    /// `MAX_OVERLAY_ROWS` rows. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for bad slots, bad names, bad health, or a
    /// full overlay.
    pub fn upsert_fighter(
        &mut self,
        id: FighterId,
        name: &str,
        hp_permille: u16,
    ) -> Result<(), OverlayError> {
        check_overlay_id(id)?;
        check_overlay_name(name)?;
        check_overlay_hp(hp_permille)?;
        if let Some(slot) = self.rows.iter_mut().find(|row| row.id == id) {
            name.clone_into(&mut slot.name);
            slot.hp_permille = hp_permille;
        } else {
            if self.rows.len() >= MAX_OVERLAY_ROWS {
                return Err(OverlayError::TooManyRows(self.rows.len()));
            }
            self.rows.push(OverlayRow {
                id,
                name: name.to_owned(),
                hp_permille,
            });
        }
        Ok(())
    }

    /// Rename a tracked fighter; unknown ids fail. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for bad names or untracked slots.
    pub fn rename(&mut self, id: FighterId, name: &str) -> Result<(), OverlayError> {
        check_overlay_name(name)?;
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            name.clone_into(&mut row.name);
            Ok(())
        } else {
            Err(OverlayError::UnknownFighter(id.get()))
        }
    }

    /// Replace a tracked fighter's health; unknown ids fail. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for bad health or untracked slots.
    pub fn set_hp(&mut self, id: FighterId, hp_permille: u16) -> Result<(), OverlayError> {
        check_overlay_hp(hp_permille)?;
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.hp_permille = hp_permille;
            Ok(())
        } else {
            Err(OverlayError::UnknownFighter(id.get()))
        }
    }

    /// Record the last hit for the attacker/victim flash; either slot may be
    /// untracked, but both must be live slots. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError::UnknownFighter`] for slots outside the cap.
    pub fn record_hit(
        &mut self,
        attacker: FighterId,
        victim: FighterId,
        hit_tick: u64,
    ) -> Result<(), OverlayError> {
        check_overlay_id(attacker)?;
        check_overlay_id(victim)?;
        self.attacker = Some(attacker);
        self.victim = Some(victim);
        self.hit_tick = hit_tick;
        Ok(())
    }

    /// Pair one landed hit: move the victim's health and record the
    /// attacker/victim flash together, so hp movement and the flash never
    /// split across frames. The victim row must be tracked. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for bad slots, bad health, or untracked victim.
    pub fn apply_first_hit(
        &mut self,
        attacker: FighterId,
        victim: FighterId,
        hit_tick: u64,
        victim_hp: u16,
    ) -> Result<(), OverlayError> {
        check_overlay_id(attacker)?;
        self.set_hp(victim, victim_hp)?;
        self.record_hit(attacker, victim, hit_tick)
    }

    /// Hide the hit flash. Pure.
    pub fn clear_hit(&mut self) {
        self.attacker = None;
        self.victim = None;
    }

    /// True when `id` is the recorded attacker or victim and the overlay tick
    /// is at most [`FLASH_TICKS`] past the hit tick. Pure.
    #[must_use]
    pub fn is_flashing(&self, id: FighterId) -> bool {
        let marked = self.attacker == Some(id) || self.victim == Some(id);
        marked && self.tick >= self.hit_tick && self.tick - self.hit_tick <= FLASH_TICKS
    }

    /// Canonical wire bytes for the `play.step` rasterizer: version, tick,
    /// palette, attacker, victim, hit tick, row count, then one
    /// length-prefixed row each (`id | name length | name | health`), all
    /// integers little-endian. Field order is the future serde order, so a
    /// serde derive later must keep it. Appends to `out` for buffer reuse.
    /// Pure.
    pub fn append_bytes(&self, out: &mut Vec<u8>) {
        out.push(OVERLAY_VERSION);
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.push(self.palette_id);
        out.push(self.attacker.map_or(NO_HIT, FighterId::get));
        out.push(self.victim.map_or(NO_HIT, FighterId::get));
        out.extend_from_slice(&self.hit_tick.to_le_bytes());
        // Counts saturate instead of converting: the row bound is enforced by
        // every constructor, so saturation only guards future widening.
        let mut count: u8 = 0;
        for _ in &self.rows {
            count = count.saturating_add(1);
        }
        out.push(count);
        for row in &self.rows {
            out.push(row.id.get());
            let mut name_len: u8 = 0;
            for _ in row.name.as_bytes() {
                name_len = name_len.saturating_add(1);
            }
            out.push(name_len);
            out.extend_from_slice(row.name.as_bytes());
            out.extend_from_slice(&row.hp_permille.to_le_bytes());
        }
    }

    /// Canonical wire bytes in a fresh, bounded (`<= OVERLAY_MAX_BYTES`)
    /// buffer. Pure.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(OVERLAY_MAX_BYTES);
        self.append_bytes(&mut out);
        out
    }

    /// Decode canonical wire bytes, rejecting bad versions, bad slots, bad
    /// names, bad health, bad palettes, overlong rows, duplicates and
    /// trailing bytes. Row order is preserved. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for any malformed or out-of-range input.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OverlayError> {
        let mut cursor: usize = 0;
        let version = take_overlay_byte(bytes, &mut cursor)?;
        if version != OVERLAY_VERSION {
            return Err(OverlayError::BadVersion(version));
        }
        let tick = take_overlay_u64(bytes, &mut cursor)?;
        let palette_id = take_overlay_byte(bytes, &mut cursor)?;
        if palette_id > 3 {
            return Err(OverlayError::BadPalette(palette_id));
        }
        let attacker = take_hit_slot(bytes, &mut cursor)?;
        let victim = take_hit_slot(bytes, &mut cursor)?;
        let hit_tick = take_overlay_u64(bytes, &mut cursor)?;
        let count = take_overlay_byte(bytes, &mut cursor)?;
        if usize::from(count) > MAX_OVERLAY_ROWS {
            return Err(OverlayError::TooManyRows(usize::from(count)));
        }
        let mut overlay = Self {
            tick,
            palette_id,
            rows: Vec::new(),
            attacker,
            victim,
            hit_tick,
        };
        for _ in 0..count {
            let slot = take_overlay_byte(bytes, &mut cursor)?;
            let id = FighterId(slot);
            check_overlay_id(id)?;
            let name_len = take_overlay_byte(bytes, &mut cursor)?;
            if usize::from(name_len) > MAX_NAME_LEN {
                return Err(OverlayError::NameTooLong(usize::from(name_len)));
            }
            let end = cursor
                .checked_add(usize::from(name_len))
                .ok_or(OverlayError::UnexpectedEnd)?;
            let raw = bytes.get(cursor..end).ok_or(OverlayError::UnexpectedEnd)?;
            cursor = end;
            let name = core::str::from_utf8(raw).map_err(|_| OverlayError::BadName)?;
            check_overlay_name(name)?;
            let hp_permille = take_overlay_u16(bytes, &mut cursor)?;
            check_overlay_hp(hp_permille)?;
            if overlay.rows.iter().any(|row| row.id == id) {
                return Err(OverlayError::DuplicateFighter(slot));
            }
            overlay.rows.push(OverlayRow {
                id,
                name: name.to_owned(),
                hp_permille,
            });
        }
        if cursor != bytes.len() {
            return Err(OverlayError::TrailingBytes(bytes.len() - cursor));
        }
        Ok(overlay)
    }

    /// Presentation change checksum over the canonical fields. This digest is
    /// a view receipt outside the simulation hash scope, so it uses a plain
    /// wrapping multiply-add fold rather than the simulation's FNV owner
    /// (`engine-core` holds FNV and this crate has no edge to it yet). Pure.
    #[must_use]
    pub fn state_digest(&self) -> u64 {
        let mut digest: u64 = 0;
        let mut mix = |byte: u8| {
            digest = digest.wrapping_mul(131).wrapping_add(u64::from(byte));
        };
        for byte in self.tick.to_le_bytes() {
            mix(byte);
        }
        mix(self.palette_id);
        mix(self.attacker.map_or(NO_HIT, FighterId::get));
        mix(self.victim.map_or(NO_HIT, FighterId::get));
        for byte in self.hit_tick.to_le_bytes() {
            mix(byte);
        }
        for row in &self.rows {
            mix(row.id.get());
            // Length-prefix variable fields so ("ab", "c") and ("a", "bc")
            // cannot collide.
            let mut name_len: u8 = 0;
            for _ in row.name.as_bytes() {
                name_len = name_len.saturating_add(1);
            }
            mix(name_len);
            for byte in row.name.as_bytes() {
                mix(*byte);
            }
            for byte in row.hp_permille.to_le_bytes() {
                mix(byte);
            }
        }
        digest
    }
}

impl DuelOverlay {
    /// Build the play-frame snapshot pairing this overlay (names, HP bars,
    /// tick) with the FPS/echo layer: the fighter-you arrow, button echo,
    /// ammo, crosshair and hit marker. Pure; validates `you` and `echo`.
    /// Caller: the `play.step` rasterizer reads this beside the overlay, so
    /// overlay rows and play layers never split across frames.
    ///
    /// # Errors
    ///
    /// Returns [`play_hud::PlayHudError`] for bad you slots or illegal echo
    /// bits.
    pub fn play_hud(
        &self,
        you: Option<FighterId>,
        echo: Buttons,
        ammo: u32,
        max_ammo: u32,
        crosshair: play_hud::Crosshair,
        hit_tick: Option<u64>,
    ) -> Result<play_hud::PlayHud, play_hud::PlayHudError> {
        play_hud::PlayHud::from_parts(self.tick, you, echo, ammo, max_ammo, crosshair, hit_tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fighter_id(slot: u8) -> FighterId {
        FighterId::new(slot).expect("test slot must be valid")
    }

    fn kill(tick: u64, victim_slot: u8) -> KillEntry {
        KillEntry {
            tick,
            killer: Some(fighter_id(0)),
            victim: fighter_id(victim_slot),
            cause: KillCause::Strike,
        }
    }

    #[test]
    fn hud_hash_x2() {
        let a = Hud::new(7, 80, 100, 30, 1500);
        let b = Hud::new(7, 80, 100, 30, 1500);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn chip_lags_damage() {
        let h = Hud::new(0, 60, 100, 0, 0);
        assert_eq!(h.chip(100), 60);
        assert_eq!(h.chip(60), 60);
    }

    #[test]
    fn hp_and_stamina_clamp_at_bounds() {
        let id = fighter_id(0);
        let row = FighterHud::new(id, 1200, 2500);
        assert_eq!(row.hp_permille, PERMILLE_MAX);
        assert_eq!(row.stamina, PERMILLE_MAX);

        let row = FighterHud::new(id, 0, 0);
        assert_eq!((row.hp_permille, row.stamina), (0, 0));

        let mut hud = Hud::new(0, 100, 100, 0, 0);
        hud.upsert_fighter(FighterHud::new(id, 999, 999)).unwrap();
        hud.set_hp(id, 1001).unwrap();
        hud.set_stamina(id, u16::MAX).unwrap();
        let row = hud.fighter(id).unwrap();
        assert_eq!((row.hp_permille, row.stamina), (1000, 1000));
        hud.set_hp(id, 0).unwrap();
        assert_eq!(hud.fighter(id).unwrap().hp_permille, 0);
    }

    #[test]
    fn unknown_fighter_updates_fail() {
        let mut hud = Hud::new(0, 100, 100, 0, 0);
        assert_eq!(
            hud.set_hp(fighter_id(1), 500),
            Err(HudError::UnknownFighter(1))
        );
        assert_eq!(
            hud.upsert_fighter(FighterHud {
                id: FighterId(u8::MAX),
                hp_permille: 1,
                stamina: 1,
            }),
            Err(HudError::UnknownFighter(u8::MAX))
        );
    }

    #[test]
    fn sixteen_slots_validate_and_sixteen_refuses() {
        // Sixteen slots validate through the canonical cap.
        assert_eq!(MAX_PARTICIPANTS, 16);
        let mut hud = Hud::new(0, 100, 100, 0, 0);
        for slot in 0..MAX_PARTICIPANTS {
            hud.upsert_fighter(FighterHud::new(fighter_id(slot), 1_000, 1_000))
                .expect("slot inside the cap tracks");
        }
        assert_eq!(hud.fighter(fighter_id(15)).unwrap().hp_permille, 1_000);
        assert_eq!(
            hud.upsert_fighter(FighterHud {
                id: FighterId(MAX_PARTICIPANTS),
                hp_permille: 1,
                stamina: 1,
            }),
            Err(HudError::UnknownFighter(MAX_PARTICIPANTS))
        );
    }

    #[test]
    fn feed_cap_drops_oldest_and_keeps_order() {
        let mut feed = KillFeed::new(3).unwrap();
        for tick in 0..5 {
            feed.push(kill(tick, 1)).unwrap();
        }
        assert_eq!(feed.len(), 3);
        let ticks: Vec<u64> = feed.iter().map(|entry| entry.tick).collect();
        assert_eq!(ticks, vec![2, 3, 4]);
        assert_eq!(feed.oldest().unwrap().tick, 2);
        assert_eq!(feed.latest().unwrap().tick, 4);
    }

    #[test]
    fn feed_rejects_bad_caps_and_entries() {
        assert_eq!(KillFeed::new(0), Err(HudError::FeedCapOutOfRange(0)));
        assert_eq!(
            KillFeed::new(MAX_FEED_CAP + 1),
            Err(HudError::FeedCapOutOfRange(MAX_FEED_CAP + 1))
        );
        let mut feed = KillFeed::default();
        let bad = KillEntry {
            tick: 0,
            killer: None,
            victim: FighterId(u8::MAX),
            cause: KillCause::Fall,
        };
        assert_eq!(feed.push(bad), Err(HudError::UnknownFighter(u8::MAX)));
        assert!(feed.is_empty());
    }

    #[test]
    fn minimap_quantizes_and_bounds_positions() {
        assert_eq!(MinimapLite::quantize(0.0), 0);
        assert_eq!(MinimapLite::quantize(1.0), 16);
        assert_eq!(MinimapLite::quantize(-2.5), -40);
        // Arena half-extent (64 m) clamps to +/-1024 cells.
        assert_eq!(MinimapLite::quantize(10_000.0), 1024);
        assert_eq!(MinimapLite::quantize(-10_000.0), -1024);

        let mut map = MinimapLite::default();
        map.upsert(fighter_id(2), 1.0, -1.0).unwrap();
        assert_eq!(
            map.get(fighter_id(2)),
            Some(MinimapDot {
                id: fighter_id(2),
                x: 16,
                z: -16,
            })
        );
        // Moving the same fighter updates in place without growing.
        map.upsert(fighter_id(2), 10_000.0, 0.0).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(fighter_id(2)).unwrap().x, 1024);
        assert_eq!(
            map.upsert(fighter_id(0), f32::NAN, 0.0),
            Err(HudError::NonFiniteCoordinate)
        );
        assert_eq!(
            map.upsert(fighter_id(0), 0.0, f32::INFINITY),
            Err(HudError::NonFiniteCoordinate)
        );
    }

    #[test]
    fn palettes_pass_contrast_and_differ_by_mode() {
        for palette in [
            Palette::default_palette(),
            Palette::deuteranopia(),
            Palette::protanopia(),
            Palette::tritanopia(),
        ] {
            assert!(palette.validate().is_ok(), "palette {}", palette.name);
        }
        let (deut, prot, trit) = (
            Palette::deuteranopia(),
            Palette::protanopia(),
            Palette::tritanopia(),
        );
        assert_ne!(deut, prot);
        assert_ne!(deut, trit);
        assert_ne!(prot, trit);

        let weak = Palette {
            name: "weak",
            bg: [120, 120, 120],
            fg: [125, 125, 125],
            accent: [255, 255, 255],
        };
        assert!(matches!(weak.validate(), Err(HudError::LowContrast { .. })));
    }

    #[test]
    fn hud_push_kill_feeds_counter_and_hash() {
        let mut hud = Hud::new(3, 100, 100, 5, 0);
        hud.push_kill(kill(3, 1)).unwrap();
        assert_eq!(hud.kill_feed, 1);
        assert_eq!(hud.feed.len(), 1);
        let again = Hud::new(3, 100, 100, 5, 0);
        assert_ne!(hud.state_hash(), again.state_hash());
        let mut clone = again.clone();
        clone.push_kill(kill(3, 1)).unwrap();
        assert_eq!(hud.state_hash(), clone.state_hash());
    }

    #[test]
    fn hud_state_hash_digest_is_pinned() {
        // Gate digests hash this exact byte stream; a layout change moves
        // every screenshot and accessibility-tree gate input.
        assert_eq!(Hud::new(0, 1, 1, 0, 0).state_hash(), 0x075a_3e76_4cfc_7cc5);

        let mut hud = Hud::new(7, 80, 100, 30, 1500);
        hud.upsert_fighter(FighterHud::new(fighter_id(0), 750, 400))
            .unwrap();
        hud.upsert_fighter(FighterHud::new(fighter_id(3), 250, 1000))
            .unwrap();
        hud.push_kill(kill(5, 3)).unwrap();
        hud.push_kill(KillEntry {
            tick: 6,
            killer: None,
            victim: fighter_id(0),
            cause: KillCause::Hazard,
        })
        .unwrap();
        hud.upsert_dot(fighter_id(0), 3.5, -12.25).unwrap();
        hud.upsert_dot(fighter_id(3), -64.0, 64.0).unwrap();
        assert_eq!(hud.state_hash(), 0x6b9d_f007_ec51_d8ea);
    }

    #[test]
    fn kill_cause_codes_are_pinned() {
        let causes = [
            KillCause::Strike,
            KillCause::Projectile,
            KillCause::Fall,
            KillCause::Hazard,
        ];
        assert_eq!(causes.map(KillCause::code), [0, 1, 2, 3]);
    }

    fn sample_overlay() -> DuelOverlay {
        let mut overlay = DuelOverlay::new(600);
        overlay
            .upsert_fighter(fighter_id(0), "Rook", 750)
            .expect("valid sample row tracks");
        overlay
            .upsert_fighter(fighter_id(1), "Sable", 250)
            .expect("valid sample row tracks");
        overlay
            .record_hit(fighter_id(0), fighter_id(1), 595)
            .expect("valid sample hit records");
        overlay
    }

    /// Frozen raster stub: the `play.step` draw contract without the endpoint.
    /// Paints names through the text layout module and atlas, HP through pips,
    /// the tick as header bits and the hit as an accent border, so the demo
    /// proves decoded bytes drive every overlay element. Test-only.
    /// Cached name cell plus generation tag (`last_used` idea): hit-tick
    /// names the cell, tick names the header. Test-only.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct CachedNameCell {
        id: FighterId,
        codepoint: char,
        cell_x: u32,
        cell_y: u32,
        pips: u8,
        flashing: bool,
        hit_tick: u64,
    }

    struct FrozenFrame {
        width: usize,
        height: usize,
        pixels: Vec<[u8; 3]>,
        last_hit_tick: Option<u64>,
        name_cache: Vec<CachedNameCell>,
        last_reused: usize,
    }

    impl FrozenFrame {
        fn new(width: usize, height: usize, bg: [u8; 3]) -> Self {
            Self {
                width,
                height,
                pixels: vec![bg; width.saturating_mul(height)],
                last_hit_tick: None,
                name_cache: Vec::new(),
                last_reused: 0,
            }
        }

        fn reused_cells(&self) -> usize {
            self.last_reused
        }

        fn name_cell_count(&self) -> usize {
            self.name_cache.len()
        }

        fn set(&mut self, x: usize, y: usize, color: [u8; 3]) {
            if x < self.width && y < self.height {
                let index = y.saturating_mul(self.width).saturating_add(x);
                if let Some(slot) = self.pixels.get_mut(index) {
                    *slot = color;
                }
            }
        }

        fn draw_overlay(&mut self, overlay: &DuelOverlay) {
            let palette = overlay.palette();
            let hit_tick = overlay.hit_tick();
            let mut current: Vec<CachedNameCell> = Vec::new();
            let mut reused = 0usize;
            let cached_hit = self.last_hit_tick == Some(hit_tick);
            for bit in 0..64u32 {
                if overlay.tick() & (1u64 << bit) != 0 {
                    let x = usize::try_from(bit).expect("header bit fits the address space");
                    self.set(x, 0, palette.fg);
                }
            }
            let atlas = text::Atlas::new();
            for (index, row) in overlay.rows().iter().enumerate() {
                let area = overlay.row_area(index).expect("tracked row has an area");
                let mut cells: Vec<text::GlyphCell> = Vec::new();
                for cell in text::layout(row.name(), area.wrap_cells, text::Align::Left) {
                    if cell.cell_x < area.clip_w_cells && cell.cell_y < area.clip_h_cells {
                        cells.push(cell);
                    }
                }
                let mut row_fresh = cells.is_empty();
                for cell in &cells {
                    if current.len() >= text::MAX_CELLS {
                        break;
                    }
                    let key = CachedNameCell {
                        id: row.id(),
                        codepoint: cell.codepoint,
                        cell_x: cell.cell_x,
                        cell_y: cell.cell_y,
                        pips: row.pips(),
                        flashing: overlay.is_flashing(row.id()),
                        hit_tick,
                    };
                    current.push(key);
                    if cached_hit && self.name_cache.contains(&key) {
                        reused = reused.saturating_add(1);
                    } else {
                        row_fresh = true;
                    }
                }
                if cached_hit && !row_fresh {
                    continue;
                }
                let base_y = 8usize.saturating_add(index.saturating_mul(12));
                for cell in cells {
                    let cell_x =
                        usize::try_from(cell.cell_x).expect("layout cell fits the address space");
                    for glyph_y in 0..text::Atlas::GLYPH_H {
                        for glyph_x in 0..text::Atlas::GLYPH_W {
                            if atlas.pixel(cell.codepoint, glyph_x, glyph_y) {
                                let x = cell_x.saturating_mul(4).saturating_add(
                                    usize::try_from(glyph_x)
                                        .expect("3-pixel glyph width fits the address space"),
                                );
                                let y = base_y.saturating_add(
                                    usize::try_from(glyph_y)
                                        .expect("5-pixel glyph height fits the address space"),
                                );
                                self.set(x, y, palette.fg);
                            }
                        }
                    }
                }
                for pip in 0..row.pips() {
                    let bar_x = 96usize.saturating_add(usize::from(pip).saturating_mul(4));
                    for dy in 0..5usize {
                        for dx in 0..3usize {
                            self.set(
                                bar_x.saturating_add(dx),
                                base_y.saturating_add(dy),
                                palette.accent,
                            );
                        }
                    }
                }
                if overlay.is_flashing(row.id()) {
                    for x in 0..self.width {
                        self.set(x, base_y.saturating_sub(2), palette.accent);
                        self.set(x, base_y.saturating_add(7), palette.accent);
                    }
                    for y in base_y.saturating_sub(2)..=base_y.saturating_add(7) {
                        self.set(0, y, palette.accent);
                        self.set(self.width.saturating_sub(1), y, palette.accent);
                    }
                }
            }
            self.last_hit_tick = Some(hit_tick);
            self.name_cache = current;
            self.last_reused = reused;
        }

        fn digest(&self) -> u64 {
            let mut digest: u64 = 0;
            for pixel in &self.pixels {
                for byte in pixel {
                    digest = digest.wrapping_mul(131).wrapping_add(u64::from(*byte));
                }
            }
            digest
        }
    }

    #[test]
    fn overlay_bytes_round_trip() {
        let overlay = sample_overlay();
        let bytes = overlay.to_bytes();
        assert!(bytes.len() <= OVERLAY_MAX_BYTES);
        let decoded = DuelOverlay::from_bytes(&bytes).expect("sample overlay decodes");
        assert_eq!(decoded, overlay);
        assert_eq!(decoded.to_bytes(), bytes, "encoding is stable");
        assert_eq!(decoded.tick(), 600);
        assert_eq!(decoded.rows().len(), 2);
        assert_eq!(
            decoded.row(fighter_id(0)).expect("row 0 tracked").name(),
            "Rook"
        );
        assert_eq!(decoded.attacker(), Some(fighter_id(0)));
        assert_eq!(decoded.victim(), Some(fighter_id(1)));
        assert_eq!(decoded.hit_tick(), 595);
        assert!(decoded.is_flashing(fighter_id(0)));
        assert!(decoded.is_flashing(fighter_id(1)));
    }

    #[test]
    fn overlay_rejects_hostile_bytes() {
        assert_eq!(
            DuelOverlay::from_bytes(&[]),
            Err(OverlayError::UnexpectedEnd)
        );
        let bytes = sample_overlay().to_bytes();
        let mut bad_version = bytes.clone();
        bad_version[0] = OVERLAY_VERSION.wrapping_add(1);
        assert_eq!(
            DuelOverlay::from_bytes(&bad_version),
            Err(OverlayError::BadVersion(2))
        );
        // Every short prefix truncates mid-field: version(1) + tick(8) +
        // palette(1) + attacker(1) + victim(1) + hit tick(8) + count(1).
        for end in [1usize, 5, 12, 20, 21] {
            assert_eq!(
                DuelOverlay::from_bytes(&bytes[..end]),
                Err(OverlayError::UnexpectedEnd),
                "prefix of {end} bytes truncates"
            );
        }
        let mut trailed = bytes.clone();
        trailed.push(0);
        assert_eq!(
            DuelOverlay::from_bytes(&trailed),
            Err(OverlayError::TrailingBytes(1))
        );
        // Wire offsets: version 0, tick 1..9, palette 9, attacker 10.
        let mut bad_palette = bytes.clone();
        bad_palette[9] = 9;
        assert_eq!(
            DuelOverlay::from_bytes(&bad_palette),
            Err(OverlayError::BadPalette(9))
        );
        let mut bad_slot = bytes.clone();
        bad_slot[10] = 40;
        assert_eq!(
            DuelOverlay::from_bytes(&bad_slot),
            Err(OverlayError::UnknownFighter(40))
        );
        // First name byte is at 23 ("Rook" starts after id 21 + length 22).
        let mut bad_name = bytes.clone();
        bad_name[23] = 0xFF;
        assert_eq!(
            DuelOverlay::from_bytes(&bad_name),
            Err(OverlayError::BadName)
        );
        // First health is at 27..29 little-endian (750); 1001 refuses.
        let mut bad_hp = bytes.clone();
        bad_hp[27] = 0xE9;
        bad_hp[28] = 0x03;
        assert_eq!(
            DuelOverlay::from_bytes(&bad_hp),
            Err(OverlayError::HpOutOfRange(1001))
        );
    }

    #[test]
    fn overlay_decode_rejects_row_attacks() {
        let mut single = DuelOverlay::new(3);
        single
            .upsert_fighter(fighter_id(0), "Rook", 500)
            .expect("single row tracks");
        let bytes = single.to_bytes();
        // Count byte sits at 21 - 1 = 20; 17 rows exceed the slot cap.
        let mut many = bytes.clone();
        many[20] = 17;
        assert_eq!(
            DuelOverlay::from_bytes(&many),
            Err(OverlayError::TooManyRows(17))
        );
        // The same row body twice is a duplicate, not two fighters.
        let mut dup = bytes.clone();
        let row_body = bytes[21..].to_vec();
        dup.extend_from_slice(&row_body);
        dup[20] = 2;
        assert_eq!(
            DuelOverlay::from_bytes(&dup),
            Err(OverlayError::DuplicateFighter(0))
        );
        // An overlong name length refuses before the body is read.
        let mut long = bytes.clone();
        long[22] = u8::try_from(MAX_NAME_LEN + 1).expect("25 fits u8");
        assert_eq!(
            DuelOverlay::from_bytes(&long),
            Err(OverlayError::NameTooLong(MAX_NAME_LEN + 1))
        );
    }

    #[test]
    fn overlay_seeded_bytes_identical_twice() {
        let first = DuelOverlay::from_seed(7);
        let second = DuelOverlay::from_seed(7);
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(first.state_digest(), second.state_digest());
        let other = DuelOverlay::from_seed(8);
        assert_ne!(
            first.to_bytes(),
            other.to_bytes(),
            "distinct seeds build distinct duels"
        );
        assert_eq!(first.rows().len(), 2);
        assert_ne!(first.rows()[0].name(), first.rows()[1].name());
        for row in first.rows() {
            assert!(!row.name().is_empty() && row.name().len() <= MAX_NAME_LEN);
            assert!(row.hp_permille() <= PERMILLE_MAX);
            assert!(row.pips() <= MAX_HP_PIPS);
        }
        let flash_visible = first.rows().iter().any(|row| first.is_flashing(row.id()));
        assert!(flash_visible, "seeded hit stays inside the flash window");
    }

    #[test]
    fn overlay_palettes_use_the_colorblind_sets() {
        let expected = [
            Palette::default_palette(),
            Palette::deuteranopia(),
            Palette::protanopia(),
            Palette::tritanopia(),
        ];
        for (index, palette) in expected.iter().enumerate() {
            let mut overlay = DuelOverlay::new(0);
            let id = u8::try_from(index).expect("four palette ids fit u8");
            overlay.set_palette(id).expect("palette id selects");
            assert_eq!(overlay.palette_id(), id);
            assert_eq!(overlay.palette(), *palette);
            assert!(overlay.palette().validate().is_ok());
        }
        let mut overlay = DuelOverlay::new(0);
        assert_eq!(overlay.set_palette(4), Err(OverlayError::BadPalette(4)));
        assert_eq!(
            overlay.set_palette(u8::MAX),
            Err(OverlayError::BadPalette(u8::MAX))
        );
    }

    #[test]
    fn overlay_pips_span_empty_to_full() {
        assert_eq!(hp_pips(0), 0);
        assert_eq!(hp_pips(49), 0, "halves round up from 50");
        assert_eq!(hp_pips(50), 1);
        assert_eq!(hp_pips(PERMILLE_MAX), MAX_HP_PIPS);
        assert_eq!(
            hp_pips(1200),
            MAX_HP_PIPS,
            "over-range health clamps before lighting pips"
        );
        let mut prev = 0u8;
        for hp in (0u16..=PERMILLE_MAX).step_by(37) {
            let pips = hp_pips(hp);
            assert!(pips >= prev, "pips never dim as health rises");
            prev = pips;
        }
    }

    #[test]
    fn overlay_names_validate_at_the_boundary() {
        let mut overlay = DuelOverlay::new(0);
        assert_eq!(
            overlay.upsert_fighter(fighter_id(0), "", 500),
            Err(OverlayError::EmptyName)
        );
        assert_eq!(
            overlay.upsert_fighter(fighter_id(0), "Rook\nSable", 500),
            Err(OverlayError::BadName)
        );
        let long = "x".repeat(MAX_NAME_LEN + 1);
        assert_eq!(
            overlay.upsert_fighter(fighter_id(0), &long, 500),
            Err(OverlayError::NameTooLong(long.len()))
        );
        assert_eq!(
            overlay.upsert_fighter(fighter_id(0), "Rook", 1001),
            Err(OverlayError::HpOutOfRange(1001))
        );
        assert_eq!(
            overlay.upsert_fighter(FighterId(u8::MAX), "Rook", 500),
            Err(OverlayError::UnknownFighter(u8::MAX))
        );
        assert_eq!(
            overlay.set_hp(fighter_id(2), 10),
            Err(OverlayError::UnknownFighter(2))
        );
        overlay
            .upsert_fighter(fighter_id(0), "Rook", 500)
            .expect("valid row tracks");
        overlay
            .upsert_fighter(fighter_id(0), "Sable", 250)
            .expect("same slot replaces in place");
        assert_eq!(overlay.len(), 1);
        assert_eq!(
            overlay.row(fighter_id(0)).expect("row tracked").name(),
            "Sable"
        );
        overlay.rename(fighter_id(0), "Pax").expect("rename tracks");
        assert_eq!(
            overlay.row(fighter_id(0)).expect("row tracked").name(),
            "Pax"
        );
        assert_eq!(
            overlay.rename(fighter_id(3), "Pax"),
            Err(OverlayError::UnknownFighter(3))
        );
    }

    #[test]
    fn overlay_flash_expires_with_the_tick() {
        let mut overlay = sample_overlay();
        assert!(overlay.is_flashing(fighter_id(0)));
        overlay.set_tick(595 + FLASH_TICKS);
        assert!(overlay.is_flashing(fighter_id(1)), "window is inclusive");
        overlay.set_tick(595 + FLASH_TICKS + 1);
        assert!(!overlay.is_flashing(fighter_id(0)));
        assert!(!overlay.is_flashing(fighter_id(1)));
        overlay.clear_hit();
        overlay.set_tick(595);
        assert!(
            !overlay.is_flashing(fighter_id(0)),
            "cleared hit shows nothing"
        );
        assert_eq!(overlay.attacker(), None);
    }

    #[test]
    fn overlay_raster_shows_names_hp_tick_flash() {
        let overlay = sample_overlay();
        let bg = overlay.palette().bg;
        let bare = FrozenFrame::new(128, 64, bg);
        let mut framed = FrozenFrame::new(128, 64, bg);
        framed.draw_overlay(&overlay);
        assert_ne!(
            framed.digest(),
            bare.digest(),
            "overlay paints over a bare duel frame"
        );
        let lit = framed
            .pixels
            .iter()
            .filter(|pixel| **pixel == overlay.palette().fg)
            .count();
        assert!(lit > 0, "name glyphs light foreground pixels");

        let mut renamed = overlay.clone();
        renamed.rename(fighter_id(0), "Pax").expect("rename tracks");
        let mut renamed_frame = FrozenFrame::new(128, 64, bg);
        renamed_frame.draw_overlay(&renamed);
        assert_ne!(
            renamed_frame.digest(),
            framed.digest(),
            "a new name repaints glyph pixels"
        );

        let mut hurt = overlay.clone();
        hurt.set_hp(fighter_id(1), 900).expect("health tracks");
        let mut hurt_frame = FrozenFrame::new(128, 64, bg);
        hurt_frame.draw_overlay(&hurt);
        assert_ne!(
            hurt_frame.digest(),
            framed.digest(),
            "new health repaints the pip bar"
        );

        let mut ticked = overlay.clone();
        ticked.set_tick(601);
        let mut ticked_frame = FrozenFrame::new(128, 64, bg);
        ticked_frame.draw_overlay(&ticked);
        assert_ne!(
            ticked_frame.digest(),
            framed.digest(),
            "a new tick repaints the header"
        );

        let mut calm = overlay.clone();
        calm.clear_hit();
        let mut calm_frame = FrozenFrame::new(128, 64, bg);
        calm_frame.draw_overlay(&calm);
        assert_ne!(
            calm_frame.digest(),
            framed.digest(),
            "clearing the hit drops the flash border"
        );

        let decoded = DuelOverlay::from_bytes(&overlay.to_bytes()).expect("wire bytes decode");
        let mut via_bytes = FrozenFrame::new(128, 64, bg);
        via_bytes.draw_overlay(&decoded);
        assert_eq!(
            via_bytes.digest(),
            framed.digest(),
            "the rasterizer draws decoded bytes like the live overlay"
        );
        assert_eq!(overlay.row_area(0).expect("area").wrap_cells, 24);
        let mut reuse = FrozenFrame::new(128, 64, bg);
        reuse.draw_overlay(&overlay);
        let cells = reuse.name_cell_count();
        assert!(cells > 0);
        reuse.draw_overlay(&overlay);
        assert_eq!(reuse.reused_cells(), cells);
        reuse.draw_overlay(&ticked);
        assert_eq!(reuse.reused_cells(), cells);
    }

    #[test]
    fn overlay_wire_bytes_are_pinned() {
        let bytes = sample_overlay().to_bytes();
        assert_eq!(bytes.len(), 38, "sample wire is 21 header + 8 + 9");
        let expected_header: [u8; 21] = [
            OVERLAY_VERSION,
            0x58,
            0x02,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00, // tick 600 LE
            0x00, // palette 0
            0x00, // attacker 0
            0x01, // victim 1
            0x53,
            0x02,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00, // hit_tick 595 LE
            0x02, // count 2
        ];
        assert_eq!(&bytes[..21], &expected_header, "header bytes drift");
        let expected_rook: [u8; 8] = [0x00, 0x04, b'R', b'o', b'o', b'k', 0xEE, 0x02];
        assert_eq!(&bytes[21..29], &expected_rook, "Rook row drifts");
        let expected_sable: [u8; 9] = [0x01, 0x05, b'S', b'a', b'b', b'l', b'e', 0xFA, 0x00];
        assert_eq!(&bytes[29..38], &expected_sable, "Sable row drifts");
    }

    #[test]
    fn overlay_full_house_stays_inside_max_bytes() {
        let mut overlay = DuelOverlay::new(600);
        for slot in 0..16u8 {
            let name = format!("Fighter{slot:02}");
            overlay
                .upsert_fighter(fighter_id(slot), &name, 500)
                .expect("distinct slot tracks");
        }
        assert_eq!(overlay.len(), MAX_OVERLAY_ROWS);
        let bytes = overlay.to_bytes();
        assert!(
            bytes.len() <= OVERLAY_MAX_BYTES,
            "full house {} stays inside {OVERLAY_MAX_BYTES}",
            bytes.len()
        );
        let decoded = DuelOverlay::from_bytes(&bytes).expect("full house decodes");
        assert_eq!(decoded, overlay, "full house round-trips");
    }

    #[test]
    fn overlay_first_hit_pairs_hp_with_flash() {
        let mut overlay = DuelOverlay::new(70);
        overlay
            .upsert_fighter(fighter_id(0), "Rook", 1000)
            .expect("attacker tracks");
        overlay
            .upsert_fighter(fighter_id(1), "Sable", 1000)
            .expect("victim tracks");
        overlay
            .apply_first_hit(fighter_id(0), fighter_id(1), 44, 760)
            .expect("first hit pairs hp with flash");
        assert_eq!(
            overlay
                .row(fighter_id(1))
                .expect("victim tracked")
                .hp_permille(),
            760,
            "victim hp moves with the hit"
        );
        assert!(overlay.is_flashing(fighter_id(0)), "attacker flashes");
        assert!(overlay.is_flashing(fighter_id(1)), "victim flashes");
        assert_eq!(
            overlay.apply_first_hit(fighter_id(0), fighter_id(2), 44, 10),
            Err(OverlayError::UnknownFighter(2)),
            "untracked victim hp never moves without its row"
        );
    }
}
