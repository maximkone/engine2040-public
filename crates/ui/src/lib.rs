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
/// Integer-pixel panel layout: constraints, gaps and margins to exact rects.
pub mod layout;
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
/// The participant cap as a count: the canonical cap constant
/// (`MAX_PARTICIPANTS`) is a `u8`, and the map dots, the overlay rows and the
/// overlay byte budget below are counts.
///
/// 2026-10-21: a literal (never a numeric cast) pinned to the canonical cap
/// at compile time, the house pattern of `crates/game/src/director.rs:271`:
/// a cap change fails the build here instead of mis-sizing the overlay.
const PARTICIPANT_CAP: usize = 32;
const _: () = assert!(MAX_PARTICIPANTS == 32);
/// Maximum minimap capacity in dots: the canonical match-slot cap from
/// `engine-input` (one cap everywhere; legal range `1..=MAX_PARTICIPANTS`).
pub const MAX_MAP_CAP: usize = PARTICIPANT_CAP;
/// Minimap quantization: cells per arena meter.
pub const CELLS_PER_METER: f32 = 16.0;
/// Minimap half-extent in meters; world positions clamp to `+/-MAP_HALF_METERS`.
pub const MAP_HALF_METERS: f32 = 64.0;
/// Minimum accepted contrast ratio for palette pairs (large-text/UI bound).
pub const MIN_CONTRAST: f32 = 3.0;
/// Minimum text contrast for HUD labels (WCAG AA normal text, 4.5:1).
pub const HUD_TEXT_CONTRAST: f32 = 4.5;
/// Poise ring segments around a fighter portrait.
pub const MAX_POISE_SEGMENTS: u8 = 8;

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
/// Most fighters one overlay names: the canonical match-slot cap from
/// `engine-input` (one cap everywhere; legal range `1..=MAX_PARTICIPANTS`).
pub const MAX_OVERLAY_ROWS: usize = PARTICIPANT_CAP;
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
    /// Stamina or poise outside `0..=PERMILLE_MAX`, culprit carried.
    StatOutOfRange(u16),
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
            Self::StatOutOfRange(value) => write!(f, "stat out of range ({value})"),
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

/// Reject stamina or poise outside permille; row defaults are full. Pure.
fn check_overlay_stat(value: u16) -> Result<(), OverlayError> {
    if value > PERMILLE_MAX {
        return Err(OverlayError::StatOutOfRange(value));
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

/// Poise in permille to lit ring segments, `0..=MAX_POISE_SEGMENTS`.
/// Partial poise lights the next segment (a sliver is visible), so a broken
/// guard reads before it empties. Pure integer math, never panics.
#[must_use]
pub fn poise_segments(poise_permille: u16) -> u8 {
    let clamped = u32::from(clamp_permille(poise_permille));
    let scaled = clamped.saturating_mul(u32::from(MAX_POISE_SEGMENTS));
    let lit = scaled.saturating_add(u32::from(PERMILLE_MAX) - 1) / u32::from(PERMILLE_MAX);
    u8::try_from(lit).unwrap_or(MAX_POISE_SEGMENTS)
}

/// One named fighter row: display name plus permille health, stamina and
/// poise. Health rides the wire; stamina and poise are presentation-side and
/// default full on decode, so wire bytes stay frozen. Pure data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayRow {
    /// Which fighter this row names.
    id: FighterId,
    /// Display name, `1..=MAX_NAME_LEN` UTF-8 bytes, no control characters.
    name: String,
    /// Health in permille, `0..=1000`.
    hp_permille: u16,
    /// Stamina in permille, `0..=1000`; starts full.
    stamina_permille: u16,
    /// Poise in permille, `0..=1000`; starts full.
    poise_permille: u16,
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

    /// Stamina in permille, `0..=1000`.
    #[must_use]
    pub fn stamina_permille(&self) -> u16 {
        self.stamina_permille
    }

    /// Poise in permille, `0..=1000`.
    #[must_use]
    pub fn poise_permille(&self) -> u16 {
        self.poise_permille
    }

    /// Lit HP pips for this row; see [`hp_pips`]. Pure.
    #[must_use]
    pub fn pips(&self) -> u8 {
        hp_pips(self.hp_permille)
    }

    /// Lit poise ring segments for this row; see [`poise_segments`]. Pure.
    #[must_use]
    pub fn poise_segments(&self) -> u8 {
        poise_segments(self.poise_permille)
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
                    stamina_permille: hp(28),
                    poise_permille: hp(36),
                },
                OverlayRow {
                    id: FighterId(1),
                    name: NAMES[second].to_owned(),
                    hp_permille: hp(20),
                    stamina_permille: hp(44),
                    poise_permille: hp(52),
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
                stamina_permille: PERMILLE_MAX,
                poise_permille: PERMILLE_MAX,
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

    /// Replace a tracked fighter's stamina; unknown ids fail. Presentation
    /// only: stamina never enters the overlay wire. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for bad stamina or untracked slots.
    pub fn set_stamina(
        &mut self,
        id: FighterId,
        stamina_permille: u16,
    ) -> Result<(), OverlayError> {
        check_overlay_stat(stamina_permille)?;
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.stamina_permille = stamina_permille;
            Ok(())
        } else {
            Err(OverlayError::UnknownFighter(id.get()))
        }
    }

    /// Replace a tracked fighter's poise; unknown ids fail. Presentation
    /// only: poise never enters the overlay wire. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] for bad poise or untracked slots.
    pub fn set_poise(&mut self, id: FighterId, poise_permille: u16) -> Result<(), OverlayError> {
        check_overlay_stat(poise_permille)?;
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.poise_permille = poise_permille;
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
                // Stamina and poise are presentation-side; the frozen wire
                // has no bytes for them, so a decoded row starts full.
                stamina_permille: PERMILLE_MAX,
                poise_permille: PERMILLE_MAX,
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

/// What a HUD rectangle means. The renderer paints every rect the same way;
/// the tag keeps tests and accessibility tools able to name each element.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HudRole {
    /// Row background panel.
    Panel,
    /// Empty part of a bar or pip.
    BarBack,
    /// One health pip.
    HpPip,
    /// Filled part of the stamina bar.
    StaminaFill,
    /// Lit poise ring segment.
    PoiseSegment,
    /// Unlit poise ring segment.
    PoiseEmpty,
    /// Filled part of the health bar.
    HpFill,
    /// One part of the weapon icon.
    WeaponIcon,
}

/// One HUD command kind; each kind is one draw batch.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HudCommandKind {
    /// Filled rectangle.
    Rect,
    /// Rectangle outline.
    Border,
    /// Text run.
    Text,
    /// Clip rectangle for the commands that follow.
    Scissor,
}

/// One flat HUD command in the Clay shape: rect, border, text, scissor.
/// Integer pixels, top-left origin, sRGB RGBA color. Pure data; the render
/// HUD pass uploads rects as one instanced draw and text through its text
/// path. Scissor changes state, it is not a draw.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HudCommand {
    /// Filled rectangle at `(x, y)` with size `(w, h)`.
    Rect {
        /// What this rectangle draws.
        role: HudRole,
        /// Left edge in pixels.
        x: i32,
        /// Top edge in pixels.
        y: i32,
        /// Width in pixels.
        w: i32,
        /// Height in pixels.
        h: i32,
        /// sRGB color plus alpha.
        color: [u8; 4],
    },
    /// Outline `thickness` pixels wide inside `(x, y, w, h)`.
    Border {
        /// Left edge in pixels.
        x: i32,
        /// Top edge in pixels.
        y: i32,
        /// Width in pixels.
        w: i32,
        /// Height in pixels.
        h: i32,
        /// Stroke width in pixels.
        thickness: i32,
        /// sRGB color plus alpha.
        color: [u8; 4],
    },
    /// One text run with its top-left corner, font size and color.
    Text {
        /// Left edge in pixels.
        x: i32,
        /// Top edge in pixels.
        y: i32,
        /// Font size in pixels.
        px: u16,
        /// sRGB color plus alpha.
        color: [u8; 4],
        /// ASCII text (the font covers `0x20..=0x7E`).
        text: String,
    },
    /// Clip every following command to `(x, y, w, h)`.
    Scissor {
        /// Left edge in pixels.
        x: i32,
        /// Top edge in pixels.
        y: i32,
        /// Width in pixels.
        w: i32,
        /// Height in pixels.
        h: i32,
    },
}

impl HudCommand {
    /// Command kind, one draw batch each.
    #[must_use]
    pub fn kind(&self) -> HudCommandKind {
        match self {
            Self::Rect { .. } => HudCommandKind::Rect,
            Self::Border { .. } => HudCommandKind::Border,
            Self::Text { .. } => HudCommandKind::Text,
            Self::Scissor { .. } => HudCommandKind::Scissor,
        }
    }

    /// Text payload for a text command. Pure.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. } => Some(text),
            Self::Rect { .. } | Self::Border { .. } | Self::Scissor { .. } => None,
        }
    }
}

/// One HUD frame as a flat command list plus the numbers the render pass
/// needs: viewport, text size and batch counts. Pure data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HudFrame {
    width: u32,
    height: u32,
    text_px: u16,
    commands: Vec<HudCommand>,
}

impl HudFrame {
    /// Viewport width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Viewport height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Text font size in pixels.
    #[must_use]
    pub fn text_px(&self) -> u16 {
        self.text_px
    }

    /// Commands in draw order.
    #[must_use]
    pub fn commands(&self) -> &[HudCommand] {
        &self.commands
    }

    /// `(rect, border, text)` command counts; one draw per non-empty count.
    #[must_use]
    pub fn batch_counts(&self) -> (usize, usize, usize) {
        let mut counts = (0usize, 0usize, 0usize);
        for command in &self.commands {
            match command.kind() {
                HudCommandKind::Rect => counts.0 += 1,
                HudCommandKind::Border => counts.1 += 1,
                HudCommandKind::Text => counts.2 += 1,
                HudCommandKind::Scissor => {}
            }
        }
        counts
    }

    /// Draw calls this frame needs: one per non-empty batch, at most 3.
    #[must_use]
    pub fn draws(&self) -> usize {
        let (rects, borders, texts) = self.batch_counts();
        usize::from(rects > 0) + usize::from(borders > 0) + usize::from(texts > 0)
    }

    /// Build a frame from a prepared command list. [`DuelOverlay::hud_frame`]
    /// builds the duel HUD; the windowed proof feeds a stock bar, a minimap
    /// placeholder and text lines through this door. Commands keep draw order;
    /// the render pass clips them to its surface. Pure.
    #[must_use]
    pub fn from_commands(width: u32, height: u32, text_px: u16, commands: Vec<HudCommand>) -> Self {
        Self {
            width,
            height,
            text_px,
            commands,
        }
    }
}

/// Poise ring offsets in 1080p pixels, clockwise from the top. Scaled by the
/// viewport; the segment count is [`MAX_POISE_SEGMENTS`].
const POISE_RING: [(i32, i32); 8] = [
    (0, -40),
    (28, -28),
    (40, 0),
    (28, 28),
    (0, 40),
    (-28, 28),
    (-40, 0),
    (-28, -28),
];

/// sRGB triad to an opaque-or-not RGBA command color. Pure.
fn rgba(color: [u8; 3], alpha: u8) -> [u8; 4] {
    [color[0], color[1], color[2], alpha]
}

/// Mix `share` percent of `b` into `a`; integer sRGB, deterministic. Pure.
fn mix_srgb(a: [u8; 3], b: [u8; 3], share: u32) -> [u8; 3] {
    let keep = 100u32.saturating_sub(share);
    let channel = |left: u8, right: u8| -> u8 {
        let mixed = u32::from(left)
            .saturating_mul(keep)
            .saturating_add(u32::from(right).saturating_mul(share))
            / 100;
        u8::try_from(mixed).unwrap_or(0)
    };
    [
        channel(a[0], b[0]),
        channel(a[1], b[1]),
        channel(a[2], b[2]),
    ]
}

impl DuelOverlay {
    /// Flat HUD command list for `width` x `height`: one panel per named
    /// fighter with the name, HP pips, stamina bar under the HP pips, poise
    /// ring segments around the portrait and numeric labels for every bar.
    /// Text is at least [`text::HUD_TEXT_PX_1080`] at 1080p and every label
    /// uses the palette foreground on its palette background. Pure; bounded
    /// by [`MAX_OVERLAY_ROWS`], no IO, no clock.
    #[must_use]
    pub fn hud_frame(&self, width: u32, height: u32) -> HudFrame {
        let palette = self.palette();
        let text_px = text::hud_text_px(height);
        let width_px = i32::try_from(width.min(8192)).unwrap_or(1920);
        let height_px = i32::try_from(height.min(8192)).unwrap_or(1080);
        let scale = |base: i32| -> i32 { (base.saturating_mul(height_px) / 1080).max(1) };
        let margin = scale(24);
        let panel_w = scale(360);
        let row_h = scale(128);
        let gap = scale(24);
        let usable = height_px
            .saturating_sub(margin.saturating_mul(2))
            .max(row_h);
        let per_column = usize::try_from(usable / row_h).unwrap_or(1).max(1);
        let panel_bg = rgba(palette.bg, 255);
        let text_color = rgba(palette.fg, 255);
        let bar_back = rgba(mix_srgb(palette.bg, palette.fg, 30), 255);
        let stamina_color = rgba(palette.accent, 255);
        let poise_empty = rgba(mix_srgb(palette.bg, palette.fg, 20), 255);
        let mut commands = vec![HudCommand::Scissor {
            x: 0,
            y: 0,
            w: width_px,
            h: height_px,
        }];
        for (index, row) in self.rows.iter().enumerate() {
            let column = i32::try_from(index / per_column).unwrap_or(i32::MAX / 4096);
            let line = i32::try_from(index % per_column).unwrap_or(i32::MAX / 4096);
            let x = margin.saturating_add(column.saturating_mul(panel_w.saturating_add(gap)));
            let y = margin.saturating_add(line.saturating_mul(row_h));
            commands.push(HudCommand::Scissor {
                x: x.saturating_sub(scale(12)),
                y: y.saturating_sub(scale(8)),
                w: panel_w,
                h: row_h,
            });
            commands.push(HudCommand::Rect {
                role: HudRole::Panel,
                x: x.saturating_sub(scale(12)),
                y: y.saturating_sub(scale(8)),
                w: panel_w,
                h: row_h.saturating_sub(scale(8)),
                color: panel_bg,
            });
            let portrait = scale(56);
            let center_x = x.saturating_add(portrait / 2);
            let center_y = y.saturating_add(portrait / 2);
            if self.is_flashing(row.id()) {
                commands.push(HudCommand::Border {
                    x,
                    y,
                    w: portrait,
                    h: portrait,
                    thickness: scale(3),
                    color: rgba(palette.accent, 255),
                });
            }
            let lit = row.poise_segments();
            for (segment, (dx, dy)) in POISE_RING.iter().enumerate() {
                let role = if segment < usize::from(lit) {
                    HudRole::PoiseSegment
                } else {
                    HudRole::PoiseEmpty
                };
                let seg_w = scale(6);
                let seg_h = scale(12);
                commands.push(HudCommand::Rect {
                    role,
                    x: center_x
                        .saturating_add(scale(*dx))
                        .saturating_sub(seg_w / 2),
                    y: center_y
                        .saturating_add(scale(*dy))
                        .saturating_sub(seg_h / 2),
                    w: seg_w,
                    h: seg_h,
                    color: if segment < usize::from(lit) {
                        stamina_color
                    } else {
                        poise_empty
                    },
                });
            }
            let text_x = x.saturating_add(portrait).saturating_add(scale(40));
            commands.push(HudCommand::Text {
                x: text_x,
                y,
                px: text_px,
                color: text_color,
                text: row.name().to_owned(),
            });
            commands.push(HudCommand::Text {
                x: text_x,
                y: y.saturating_add(scale(28)),
                px: text_px,
                color: text_color,
                text: format!("HP {}", row.hp_permille()),
            });
            commands.push(HudCommand::Text {
                x: text_x.saturating_add(scale(120)),
                y: y.saturating_add(scale(28)),
                px: text_px,
                color: text_color,
                text: format!("STA {}", row.stamina_permille()),
            });
            let pip_w = scale(18);
            let pip_h = scale(8);
            let pip_gap = scale(2);
            let bar_w = pip_w
                .saturating_add(pip_gap)
                .saturating_mul(i32::from(MAX_HP_PIPS))
                .saturating_sub(pip_gap);
            let pips_y = y.saturating_add(scale(64));
            commands.push(HudCommand::Border {
                x: text_x.saturating_sub(scale(2)),
                y: pips_y.saturating_sub(scale(2)),
                w: bar_w.saturating_add(scale(4)),
                h: pip_h.saturating_add(scale(4)),
                thickness: scale(1),
                color: text_color,
            });
            for pip in 0..MAX_HP_PIPS {
                let lit_pip = pip < row.pips();
                commands.push(HudCommand::Rect {
                    role: HudRole::HpPip,
                    x: text_x.saturating_add(i32::from(pip).saturating_mul(pip_w + pip_gap)),
                    y: pips_y,
                    w: pip_w,
                    h: pip_h,
                    color: if lit_pip { text_color } else { bar_back },
                });
            }
            let stamina_y = y.saturating_add(scale(80));
            commands.push(HudCommand::Rect {
                role: HudRole::BarBack,
                x: text_x,
                y: stamina_y,
                w: bar_w,
                h: pip_h,
                color: bar_back,
            });
            let fill = bar_w.saturating_mul(i32::from(row.stamina_permille())) / 1000;
            if fill > 0 {
                commands.push(HudCommand::Rect {
                    role: HudRole::StaminaFill,
                    x: text_x,
                    y: stamina_y,
                    w: fill,
                    h: pip_h,
                    color: stamina_color,
                });
            }
            commands.push(HudCommand::Text {
                x: text_x,
                y: y.saturating_add(scale(96)),
                px: text_px,
                color: text_color,
                text: format!("POISE {}/{}", lit, MAX_POISE_SEGMENTS),
            });
        }
        HudFrame {
            width,
            height,
            text_px,
            commands,
        }
    }
}

// ---------------------------------------------------------------------------
// Legionary HUD: the three elements of the helmet-POV pictures.
// ---------------------------------------------------------------------------

/// Default objective line of the HUD: the words of `helmet-pov-march.jpg`.
///
/// The march mode reads its own line from its rules file; this is the honest
/// fallback for a mode that has none.
pub const LEGIONARY_DEFAULT_OBJECTIVE: &str = "MARCH TO THE CAMP.";
/// Longest objective line the HUD shows, in characters; the rest clips.
pub const LEGIONARY_MAX_OBJECTIVE_CHARS: usize = 48;
/// Label above the objective line, as the march picture writes it.
pub const LEGIONARY_OBJECTIVE_LABEL: &str = "OBJECTIVE:";
/// Health bar fill: the blood red of `helmet-pov-battle.jpg`.
pub const LEGIONARY_HP_COLOR: [u8; 3] = [160, 30, 24];
/// Stamina bar fill: the antique gold of `helmet-pov-battle.jpg`.
pub const LEGIONARY_STAMINA_COLOR: [u8; 3] = [176, 160, 73];
/// Bar track: the near-black the fills sit on.
pub const LEGIONARY_TRACK_COLOR: [u8; 3] = [18, 18, 16];
/// Health label: the white of the `HP 78%` line.
pub const LEGIONARY_LABEL_COLOR: [u8; 3] = [240, 238, 230];
/// Weapon icon: the steel of the gladius.
pub const LEGIONARY_ICON_COLOR: [u8; 3] = [198, 190, 174];
/// Objective line: the ivory of `helmet-pov-march.jpg`.
pub const LEGIONARY_OBJECTIVE_COLOR: [u8; 3] = [237, 230, 210];

/// Reference viewport of the layout numbers: 1920 by 1080 pixels. The
/// pictures are 2000 by 1116 and 1376 by 768; the numbers below are the same
/// places read off a picture and converted to that reference.
const LEGIONARY_REF_H: i32 = 1080;
/// Left edge of the bottom-left cluster, in reference pixels.
const LEGIONARY_BARS_X: i32 = 99;
/// Width of both bars, in reference pixels.
const LEGIONARY_BARS_W: i32 = 244;
/// Top edge of the health bar, in reference pixels.
const LEGIONARY_HP_BAR_Y: i32 = 993;
/// Height of the health bar, in reference pixels.
const LEGIONARY_HP_BAR_H: i32 = 19;
/// Top edge of the stamina bar, in reference pixels.
const LEGIONARY_STAMINA_BAR_Y: i32 = 1021;
/// Height of the stamina bar, in reference pixels.
const LEGIONARY_STAMINA_BAR_H: i32 = 12;
/// Top edge of the `HP 78%` line, in reference pixels.
const LEGIONARY_HP_LABEL_Y: i32 = 959;
/// Height of the weapon icon box, in reference pixels.
const LEGIONARY_ICON_H: i32 = 28;
/// Right margin of the weapon icon, in reference pixels.
const LEGIONARY_ICON_RIGHT_MARGIN: i32 = 58;
/// Bottom margin of the weapon icon, in reference pixels.
const LEGIONARY_ICON_BOTTOM_MARGIN: i32 = 65;
/// Right margin of the objective line, in per mille of the viewport width.
const LEGIONARY_OBJECTIVE_RIGHT_PERMILLE: u32 = 100;
/// Top margin of the objective line, in per mille of the viewport height.
const LEGIONARY_OBJECTIVE_TOP_PERMILLE: u32 = 50;
/// Gap between the objective label and the objective line, in reference pixels.
const LEGIONARY_OBJECTIVE_GAP: i32 = 6;

/// The kit item the weapon slot shows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LegionaryWeapon {
    /// The short sword: leaf blade, crossguard, ribbed grip, round pommel.
    Gladius,
    /// The curved shield: painted panel with an iron boss.
    Scutum,
    /// The throwing spear: long shaft, small iron head.
    Pilum,
}

/// One rectangle of the legionary HUD, in pixels, top-left origin.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LegionaryRect {
    /// Left edge in pixels.
    pub x: i32,
    /// Top edge in pixels.
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl LegionaryRect {
    /// Right edge in pixels, exclusive.
    #[must_use]
    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    /// Bottom edge in pixels, exclusive.
    #[must_use]
    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }
}

/// One bar of the legionary HUD: the empty track and the filled part.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LegionaryBar {
    /// The empty track the fill sits on.
    pub track: LegionaryRect,
    /// The filled part, in the bar's own colour.
    pub fill: LegionaryRect,
}

/// The weapon icon: one kit item drawn as a few rectangles.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LegionaryIcon {
    /// Which kit item the icon shows.
    pub weapon: LegionaryWeapon,
    /// The parts of the icon, in draw order.
    pub parts: Vec<LegionaryRect>,
}

/// One text run of the legionary HUD.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LegionaryText {
    /// Where the run starts, in pixels.
    pub at: LegionaryRect,
    /// The text of the run.
    pub text: String,
}

/// Every element of the legionary HUD, with its rectangle.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LegionaryLayout {
    /// The health bar and its fill.
    pub hp: LegionaryBar,
    /// The stamina bar and its fill.
    pub stamina: LegionaryBar,
    /// The `HP 78%` line above the bars.
    pub hp_label: LegionaryText,
    /// The weapon icon.
    pub weapon: LegionaryIcon,
    /// The `OBJECTIVE:` label.
    pub objective_label: LegionaryText,
    /// The objective line itself.
    pub objective: LegionaryText,
}

/// The gladius of the battle picture, drawn as rectangles: the leaf blade
/// points left, the crossguard stands at its heel, the grip is ribbed and
/// the pommel is round.
const LEGIONARY_GLADIUS: [LegionaryRect; 5] = [
    LegionaryRect {
        x: 0,
        y: 8,
        w: 18,
        h: 9,
    },
    LegionaryRect {
        x: 18,
        y: 5,
        w: 92,
        h: 16,
    },
    LegionaryRect {
        x: 110,
        y: 0,
        w: 12,
        h: 28,
    },
    LegionaryRect {
        x: 124,
        y: 7,
        w: 21,
        h: 13,
    },
    LegionaryRect {
        x: 148,
        y: 4,
        w: 12,
        h: 18,
    },
];

/// The scutum of the legionary's kit: the painted panel and its iron boss.
const LEGIONARY_SCUTUM: [LegionaryRect; 2] = [
    LegionaryRect {
        x: 0,
        y: 0,
        w: 114,
        h: 28,
    },
    LegionaryRect {
        x: 48,
        y: 8,
        w: 18,
        h: 14,
    },
];

/// The pilum of the legionary's kit: the iron head, the long shaft and the
/// butt spike.
const LEGIONARY_PILUM: [LegionaryRect; 3] = [
    LegionaryRect {
        x: 0,
        y: 7,
        w: 16,
        h: 14,
    },
    LegionaryRect {
        x: 16,
        y: 11,
        w: 126,
        h: 6,
    },
    LegionaryRect {
        x: 142,
        y: 10,
        w: 12,
        h: 8,
    },
];

impl LegionaryWeapon {
    /// The icon parts of this kit item, relative to the icon box, in
    /// reference pixels, in draw order.
    #[must_use]
    fn parts(self) -> &'static [LegionaryRect] {
        match self {
            Self::Gladius => &LEGIONARY_GLADIUS,
            Self::Scutum => &LEGIONARY_SCUTUM,
            Self::Pilum => &LEGIONARY_PILUM,
        }
    }
}

/// The legionary HUD of the helmet-POV pictures.
///
/// `docs/design/legionary/helmet-pov-battle.jpg` is the look bar: the health
/// and the stamina bars sit bottom left, the weapon icon sits bottom right.
/// `helmet-pov-march.jpg` adds the one objective line top right. In
/// legionary mode the HUD is only those three things.
///
/// Consumer: the HUD renderer (the same flat command list as the arena HUD)
/// and the legionary mode of `crates/game`. Data owner: `engine-ui`. Units:
/// health and stamina are permille (`0..=1000`), rectangles are pixels of
/// the viewport the layout was built for, colours are sRGB triads.
/// Determinism: a pure function of the inputs, integer math only, no clock,
/// no IO, no clock and no drawing device. Failure mode: permille values clamp, the objective line
/// clips to [`LEGIONARY_MAX_OBJECTIVE_CHARS`], nothing panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LegionaryHud {
    width: u32,
    height: u32,
    hp_permille: u16,
    stamina_permille: u16,
    weapon: LegionaryWeapon,
    objective: String,
}

impl LegionaryHud {
    /// Build the HUD for one viewport. Health and stamina clamp to
    /// `0..=PERMILLE_MAX`, the objective line clips to
    /// [`LEGIONARY_MAX_OBJECTIVE_CHARS`] and the viewport clamps to
    /// `1..=8192` on both sides. Pure.
    #[must_use]
    pub fn new(
        width: u32,
        height: u32,
        hp_permille: u16,
        stamina_permille: u16,
        weapon: LegionaryWeapon,
        objective: &str,
    ) -> Self {
        Self {
            width: width.clamp(1, 8192),
            height: height.clamp(1, 8192),
            hp_permille: clamp_permille(hp_permille),
            stamina_permille: clamp_permille(stamina_permille),
            weapon,
            objective: objective
                .chars()
                .take(LEGIONARY_MAX_OBJECTIVE_CHARS)
                .collect(),
        }
    }

    /// Health in permille, `0..=1000`.
    #[must_use]
    pub fn hp_permille(&self) -> u16 {
        self.hp_permille
    }

    /// Stamina in permille, `0..=1000`.
    #[must_use]
    pub fn stamina_permille(&self) -> u16 {
        self.stamina_permille
    }

    /// The kit item the weapon slot shows.
    #[must_use]
    pub fn weapon(&self) -> LegionaryWeapon {
        self.weapon
    }

    /// The objective line the HUD shows.
    #[must_use]
    pub fn objective(&self) -> &str {
        &self.objective
    }

    /// The health line above the bars: the health in whole per cent, as the
    /// battle picture writes it.
    #[must_use]
    pub fn hp_label(&self) -> String {
        format!("HP {}%", self.hp_permille / 10)
    }

    /// The layout as data: every element of the HUD with its rectangle. The
    /// numbers come from the two pictures and scale with the viewport
    /// height. Pure.
    #[must_use]
    pub fn layout(&self) -> LegionaryLayout {
        let width = i32::try_from(self.width.min(8192)).unwrap_or(1920);
        let height = i32::try_from(self.height.min(8192)).unwrap_or(1080);
        let scale = |base: i32| -> i32 {
            base.saturating_mul(height)
                .checked_div(LEGIONARY_REF_H)
                .unwrap_or(base)
                .max(1)
        };
        let offset = |base: i32| -> i32 {
            base.saturating_mul(height)
                .checked_div(LEGIONARY_REF_H)
                .unwrap_or(base)
        };
        let text_px = text::hud_text_px(self.height);
        let text_px_i = i32::from(text_px);
        let bars_x = offset(LEGIONARY_BARS_X);
        let bars_w = scale(LEGIONARY_BARS_W);
        let hp_track = LegionaryRect {
            x: bars_x,
            y: offset(LEGIONARY_HP_BAR_Y),
            w: bars_w,
            h: scale(LEGIONARY_HP_BAR_H),
        };
        let stamina_track = LegionaryRect {
            x: bars_x,
            y: offset(LEGIONARY_STAMINA_BAR_Y),
            w: bars_w,
            h: scale(LEGIONARY_STAMINA_BAR_H),
        };
        let hp = LegionaryBar {
            fill: LegionaryRect {
                w: bars_w.saturating_mul(i32::from(self.hp_permille)) / 1000,
                ..hp_track
            },
            track: hp_track,
        };
        let stamina = LegionaryBar {
            fill: LegionaryRect {
                w: bars_w.saturating_mul(i32::from(self.stamina_permille)) / 1000,
                ..stamina_track
            },
            track: stamina_track,
        };
        let hp_label = LegionaryText {
            at: LegionaryRect {
                x: bars_x,
                y: offset(LEGIONARY_HP_LABEL_Y),
                w: 0,
                h: text_px_i,
            },
            text: self.hp_label(),
        };
        let (objective_label, objective) = self.objective_block(width, height, text_px);
        LegionaryLayout {
            hp,
            stamina,
            hp_label,
            weapon: self.weapon_icon(width, height),
            objective_label,
            objective,
        }
    }

    /// The flat command list the HUD renderer draws: the two bars bottom
    /// left, the weapon icon bottom right, the objective lines top right and
    /// nothing else. Pure.
    #[must_use]
    pub fn frame(&self) -> HudFrame {
        let layout = self.layout();
        let text_px = text::hud_text_px(self.height);
        let mut commands: Vec<HudCommand> = Vec::new();
        push_legionary_bar(
            &mut commands,
            &layout.hp,
            HudRole::HpFill,
            LEGIONARY_HP_COLOR,
        );
        push_legionary_bar(
            &mut commands,
            &layout.stamina,
            HudRole::StaminaFill,
            LEGIONARY_STAMINA_COLOR,
        );
        for part in &layout.weapon.parts {
            commands.push(HudCommand::Rect {
                role: HudRole::WeaponIcon,
                x: part.x,
                y: part.y,
                w: part.w,
                h: part.h,
                color: rgba(LEGIONARY_ICON_COLOR, 255),
            });
        }
        push_legionary_text(
            &mut commands,
            &layout.hp_label,
            text_px,
            LEGIONARY_LABEL_COLOR,
        );
        push_legionary_text(
            &mut commands,
            &layout.objective_label,
            text_px,
            LEGIONARY_OBJECTIVE_COLOR,
        );
        push_legionary_text(
            &mut commands,
            &layout.objective,
            text_px,
            LEGIONARY_OBJECTIVE_COLOR,
        );
        HudFrame::from_commands(self.width, self.height, text_px, commands)
    }

    /// The objective block: the label above the line, both right aligned to
    /// the top right corner. Pure.
    fn objective_block(
        &self,
        width: i32,
        height: i32,
        text_px: u16,
    ) -> (LegionaryText, LegionaryText) {
        let scale = |base: i32| -> i32 {
            base.saturating_mul(height)
                .checked_div(LEGIONARY_REF_H)
                .unwrap_or(base)
                .max(1)
        };
        let text_px_i = i32::from(text_px);
        let right = width.saturating_sub(
            width.saturating_mul(i32::try_from(LEGIONARY_OBJECTIVE_RIGHT_PERMILLE).unwrap_or(0))
                / 1000,
        );
        let top = height
            .saturating_mul(i32::try_from(LEGIONARY_OBJECTIVE_TOP_PERMILLE).unwrap_or(0))
            / 1000;
        let label_w = legionary_text_width(LEGIONARY_OBJECTIVE_LABEL, text_px);
        let line_w = legionary_text_width(&self.objective, text_px);
        let label = LegionaryText {
            at: LegionaryRect {
                x: right.saturating_sub(label_w),
                y: top,
                w: label_w,
                h: text_px_i,
            },
            text: LEGIONARY_OBJECTIVE_LABEL.to_owned(),
        };
        let line = LegionaryText {
            at: LegionaryRect {
                x: right.saturating_sub(line_w),
                y: top
                    .saturating_add(text_px_i)
                    .saturating_add(scale(LEGIONARY_OBJECTIVE_GAP)),
                w: line_w,
                h: text_px_i,
            },
            text: self.objective.clone(),
        };
        (label, line)
    }

    /// The weapon icon, anchored to the bottom right corner. Pure.
    fn weapon_icon(&self, width: i32, height: i32) -> LegionaryIcon {
        let scale = |base: i32| -> i32 {
            base.saturating_mul(height)
                .checked_div(LEGIONARY_REF_H)
                .unwrap_or(base)
                .max(1)
        };
        let offset = |base: i32| -> i32 {
            base.saturating_mul(height)
                .checked_div(LEGIONARY_REF_H)
                .unwrap_or(base)
        };
        let source = self.weapon.parts();
        let box_w = source
            .iter()
            .map(|part| part.x.saturating_add(part.w))
            .max()
            .unwrap_or(0);
        let icon_x = width
            .saturating_sub(scale(LEGIONARY_ICON_RIGHT_MARGIN))
            .saturating_sub(scale(box_w));
        let icon_y = height
            .saturating_sub(scale(LEGIONARY_ICON_BOTTOM_MARGIN))
            .saturating_sub(scale(LEGIONARY_ICON_H));
        let parts = source
            .iter()
            .map(|part| LegionaryRect {
                x: icon_x.saturating_add(offset(part.x)),
                y: icon_y.saturating_add(offset(part.y)),
                w: scale(part.w),
                h: scale(part.h),
            })
            .collect();
        LegionaryIcon {
            weapon: self.weapon,
            parts,
        }
    }
}

/// Push the track and the fill of one legionary bar. Pure.
fn push_legionary_bar(
    commands: &mut Vec<HudCommand>,
    bar: &LegionaryBar,
    fill_role: HudRole,
    fill_color: [u8; 3],
) {
    commands.push(HudCommand::Rect {
        role: HudRole::BarBack,
        x: bar.track.x,
        y: bar.track.y,
        w: bar.track.w,
        h: bar.track.h,
        color: rgba(LEGIONARY_TRACK_COLOR, 255),
    });
    commands.push(HudCommand::Rect {
        role: fill_role,
        x: bar.fill.x,
        y: bar.fill.y,
        w: bar.fill.w,
        h: bar.fill.h,
        color: rgba(fill_color, 255),
    });
}

/// Push one text run of the legionary HUD. Pure.
fn push_legionary_text(
    commands: &mut Vec<HudCommand>,
    run: &LegionaryText,
    text_px: u16,
    color: [u8; 3],
) {
    commands.push(HudCommand::Text {
        x: run.at.x,
        y: run.at.y,
        px: text_px,
        color: rgba(color, 255),
        text: run.text.clone(),
    });
}

/// Width of a text run in pixels at `text_px`.
///
/// The HUD pass gives every character one square cell of `text_px` pixels
/// and steps `text_px` pixels per character (`crates/render/src/hud_pass.rs`),
/// so a run of N characters is N * `text_px` wide. The right aligned runs
/// need that number, or they overhang the margin they sit on. Pure.
fn legionary_text_width(text: &str, text_px: u16) -> i32 {
    let chars = i32::try_from(text.chars().count()).unwrap_or(i32::MAX);
    chars.saturating_mul(i32::from(text_px))
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
    fn thirty_two_slots_validate_and_32_refuses() {
        // Thirty-two slots validate through the canonical cap.
        assert_eq!(MAX_PARTICIPANTS, 32);
        let mut hud = Hud::new(0, 100, 100, 0, 0);
        for slot in 0..MAX_PARTICIPANTS {
            hud.upsert_fighter(FighterHud::new(fighter_id(slot), 1_000, 1_000))
                .expect("slot inside the cap tracks");
        }
        assert_eq!(hud.fighter(fighter_id(31)).unwrap().hp_permille, 1_000);
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
        // Count byte sits at 21 - 1 = 20; one row over the cap is refused.
        let over_cap = MAX_OVERLAY_ROWS + 1;
        let mut many = bytes.clone();
        many[20] = u8::try_from(over_cap).expect("the over-cap count fits in a byte");
        assert_eq!(
            DuelOverlay::from_bytes(&many),
            Err(OverlayError::TooManyRows(over_cap))
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
        for slot in 0..MAX_PARTICIPANTS {
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

    fn raster_pixels(frame: &HudFrame) -> [usize; 8] {
        let width = usize::try_from(frame.width()).expect("frame width fits usize");
        let height = usize::try_from(frame.height()).expect("frame height fits usize");
        let mut grid: Vec<Option<HudRole>> = vec![None; width.saturating_mul(height)];
        for command in frame.commands() {
            if let HudCommand::Rect {
                role, x, y, w, h, ..
            } = *command
            {
                let x0 = i64::from(x).max(0);
                let y0 = i64::from(y).max(0);
                let x1 = i64::from(x).saturating_add(i64::from(w)).min(width as i64);
                let y1 = i64::from(y).saturating_add(i64::from(h)).min(height as i64);
                let mut py = y0;
                while py < y1 {
                    let mut px = x0;
                    while px < x1 {
                        let index = py as usize * width + px as usize;
                        grid[index] = Some(role);
                        px += 1;
                    }
                    py += 1;
                }
            }
        }
        let mut counts = [0usize; 8];
        for role in grid.into_iter().flatten() {
            counts[role_index(role)] += 1;
        }
        counts
    }

    fn role_index(role: HudRole) -> usize {
        match role {
            HudRole::Panel => 0,
            HudRole::BarBack => 1,
            HudRole::HpPip => 2,
            HudRole::StaminaFill => 3,
            HudRole::PoiseSegment => 4,
            HudRole::PoiseEmpty => 5,
            HudRole::HpFill => 6,
            HudRole::WeaponIcon => 7,
        }
    }

    #[test]
    fn hud_frame_is_three_batches_of_28px_text() {
        let overlay = sample_overlay();
        let frame = overlay.hud_frame(1920, 1080);
        assert_eq!((frame.width(), frame.height()), (1920, 1080));
        assert_eq!(frame.text_px(), 28);
        assert!(matches!(
            frame.commands().first(),
            Some(HudCommand::Scissor { .. })
        ));
        let (rects, borders, texts) = frame.batch_counts();
        assert!(rects > 0, "rect batch");
        assert!(borders > 0, "border batch");
        assert!(texts > 0, "text batch");
        assert!(
            frame.draws() <= 3,
            "one batch per kind, {} draw(s)",
            frame.draws()
        );
        for command in frame.commands() {
            if let HudCommand::Text { px, .. } = command {
                assert!(*px >= 28, "text at 1080p is {px} px");
            }
        }
    }

    #[test]
    fn hud_frame_puts_stamina_under_hp_with_poise_ring_text() {
        let mut overlay = sample_overlay();
        overlay
            .set_stamina(fighter_id(0), 400)
            .expect("stamina tracks");
        overlay.set_poise(fighter_id(1), 500).expect("poise tracks");
        let frame = overlay.hud_frame(1920, 1080);
        let mut rows: Vec<Vec<&HudCommand>> = Vec::new();
        for command in frame.commands() {
            if matches!(
                command,
                HudCommand::Rect {
                    role: HudRole::Panel,
                    ..
                }
            ) {
                rows.push(Vec::new());
            }
            if let Some(current) = rows.last_mut() {
                current.push(command);
            }
        }
        assert_eq!(rows.len(), overlay.len(), "one panel per row");
        for row in &rows {
            let hp = row
                .iter()
                .find_map(|command| match *command {
                    HudCommand::Rect {
                        role: HudRole::HpPip,
                        x,
                        y,
                        w,
                        h,
                        ..
                    } => Some((x, y, w, h)),
                    _ => None,
                })
                .expect("hp pips draw");
            let stamina = row
                .iter()
                .find_map(|command| match *command {
                    HudCommand::Rect {
                        role: HudRole::StaminaFill,
                        x,
                        y,
                        w,
                        h,
                        ..
                    } => Some((x, y, w, h)),
                    _ => None,
                })
                .expect("stamina fill draws");
            let bar = row
                .iter()
                .find_map(|command| match *command {
                    HudCommand::Rect {
                        role: HudRole::BarBack,
                        x,
                        y,
                        w,
                        h,
                        ..
                    } => Some((x, y, w, h)),
                    _ => None,
                })
                .expect("stamina back draws");
            let hp_span = row
                .iter()
                .filter_map(|command| match *command {
                    HudCommand::Rect {
                        role: HudRole::HpPip,
                        x,
                        w,
                        ..
                    } => Some(x.saturating_add(*w)),
                    _ => None,
                })
                .max()
                .expect("hp pips draw")
                .saturating_sub(*hp.0);
            assert_eq!(*stamina.0, *hp.0, "stamina starts at the hp edge");
            assert_eq!(*stamina.0, *bar.0, "stamina fill starts at its back");
            assert_eq!(*bar.2, hp_span, "stamina spans the hp width");
            assert!(*stamina.2 <= *bar.2, "the fill stays inside its back");
            assert!(*stamina.1 > *hp.1, "stamina sits under hp");
            let labels: Vec<&str> = row.iter().filter_map(|command| command.text()).collect();
            assert!(labels.iter().any(|text| text.starts_with("HP ")));
            assert!(labels.iter().any(|text| text.starts_with("STA ")));
            assert!(labels.iter().any(|text| text.starts_with("POISE ")));
        }
        let lit = |row: &[&HudCommand]| {
            row.iter()
                .filter(|command| {
                    matches!(
                        command,
                        HudCommand::Rect {
                            role: HudRole::PoiseSegment,
                            ..
                        }
                    )
                })
                .count()
        };
        assert_eq!(lit(&rows[0]), 8, "full poise lights every segment");
        assert_eq!(lit(&rows[1]), 4, "half poise lights half the ring");
        assert!(
            rows[1].iter().any(|command| matches!(
                command,
                HudCommand::Rect {
                    role: HudRole::PoiseEmpty,
                    ..
                }
            )),
            "the unlit ring stays visible"
        );
    }

    #[test]
    fn hud_frame_text_passes_45_contrast_on_every_palette() {
        for palette_id in 0..4u8 {
            let mut overlay = sample_overlay();
            overlay.set_palette(palette_id).expect("palette selects");
            let palette = overlay.palette();
            assert!(
                contrast_ratio(palette.fg, palette.bg) >= HUD_TEXT_CONTRAST,
                "palette {palette_id} text contrast"
            );
            let frame = overlay.hud_frame(1920, 1080);
            let mut measured = 0usize;
            for command in frame.commands() {
                if let HudCommand::Text { color, .. } = command {
                    assert_eq!(
                        *color,
                        [palette.fg[0], palette.fg[1], palette.fg[2], 255],
                        "text uses the palette foreground"
                    );
                    let ratio = contrast_ratio([color[0], color[1], color[2]], palette.bg);
                    assert!(
                        ratio >= HUD_TEXT_CONTRAST,
                        "palette {palette_id} text measures {ratio}"
                    );
                    measured += 1;
                }
            }
            assert!(measured > 0, "palette {palette_id} draws text");
        }
    }

    #[test]
    fn hud_frame_pixels_reach_stamina_poise_and_pips() {
        let mut overlay = sample_overlay();
        overlay
            .set_stamina(fighter_id(0), 400)
            .expect("stamina tracks");
        overlay.set_poise(fighter_id(0), 500).expect("poise tracks");
        let frame = overlay.hud_frame(1920, 1080);
        let counts = raster_pixels(&frame);
        assert!(counts[role_index(HudRole::Panel)] > 0, "panel pixels");
        assert!(counts[role_index(HudRole::HpPip)] > 0, "pip pixels");
        assert!(
            counts[role_index(HudRole::StaminaFill)] > 0,
            "stamina pixels"
        );
        assert!(
            counts[role_index(HudRole::PoiseSegment)] > 0,
            "poise pixels"
        );
        assert!(
            counts[role_index(HudRole::PoiseEmpty)] > 0,
            "unlit poise pixels"
        );
    }

    #[test]
    fn overlay_stamina_and_poise_do_not_move_the_wire() {
        let mut overlay = sample_overlay();
        let before = overlay.to_bytes();
        overlay
            .set_stamina(fighter_id(0), 123)
            .expect("stamina tracks");
        overlay.set_poise(fighter_id(0), 456).expect("poise tracks");
        assert_eq!(overlay.to_bytes(), before, "stats stay presentation-side");
        let decoded = DuelOverlay::from_bytes(&before).expect("wire bytes decode");
        let row = decoded.row(fighter_id(0)).expect("row decodes");
        assert_eq!(row.stamina_permille(), PERMILLE_MAX);
        assert_eq!(row.poise_permille(), PERMILLE_MAX);
        assert_eq!(
            overlay.set_stamina(fighter_id(0), 1001),
            Err(OverlayError::StatOutOfRange(1001))
        );
        assert_eq!(
            overlay.set_poise(fighter_id(0), 1001),
            Err(OverlayError::StatOutOfRange(1001))
        );
        assert_eq!(poise_segments(0), 0);
        assert_eq!(poise_segments(1), 1);
        assert_eq!(poise_segments(500), 4);
        assert_eq!(poise_segments(PERMILLE_MAX), MAX_POISE_SEGMENTS);
    }

    #[test]
    fn hud_text_size_holds_28_at_1080p() {
        assert_eq!(text::hud_text_px(1080), 28);
        assert_eq!(text::hud_text_px(2160), 56);
        assert!(text::hud_text_px(720) < 28, "small screens scale down");
        assert_eq!(text::hud_text_px(0), 12, "zero viewport stays readable");
    }

    /// The legionary HUD of the battle picture: 78 percent health, a gladius
    /// in the weapon slot and the march objective line.
    fn legionary_hud() -> LegionaryHud {
        LegionaryHud::new(
            1920,
            1080,
            780,
            720,
            LegionaryWeapon::Gladius,
            LEGIONARY_DEFAULT_OBJECTIVE,
        )
    }

    #[test]
    fn legionary_hud_bars_sit_bottom_left_at_1080p() {
        let layout = legionary_hud().layout();
        assert_eq!(
            layout.hp.track,
            LegionaryRect {
                x: 99,
                y: 993,
                w: 244,
                h: 19
            },
            "the health bar keeps the place of the battle picture"
        );
        assert_eq!(
            layout.stamina.track,
            LegionaryRect {
                x: 99,
                y: 1021,
                w: 244,
                h: 12
            },
            "the stamina bar sits under the health bar"
        );
        assert!(layout.hp.track.x < 1920 / 8, "the bars hug the left edge");
        assert!(
            layout.stamina.track.bottom() > 1080 - 60,
            "the bars hug the bottom edge"
        );
        assert!(
            layout.stamina.track.y > layout.hp.track.bottom(),
            "the stamina bar sits under the health bar"
        );
        assert_eq!(
            layout.hp_label.text, "HP 78%",
            "the picture writes the percent"
        );
        assert_eq!(layout.hp_label.at.x, layout.hp.track.x);
        assert!(
            layout.hp_label.at.y + layout.hp_label.at.h <= layout.hp.track.y,
            "the health line sits above the bar"
        );
    }

    #[test]
    fn legionary_hud_bars_scale_to_720p() {
        let hud = LegionaryHud::new(
            1280,
            720,
            780,
            720,
            LegionaryWeapon::Gladius,
            LEGIONARY_DEFAULT_OBJECTIVE,
        );
        let layout = hud.layout();
        assert_eq!(
            layout.hp.track,
            LegionaryRect {
                x: 66,
                y: 662,
                w: 162,
                h: 12
            },
            "the 720p bar is the 1080p bar scaled down"
        );
        assert_eq!(
            layout.stamina.track,
            LegionaryRect {
                x: 66,
                y: 680,
                w: 162,
                h: 8
            }
        );
        assert!(layout.hp.track.x < 1280 / 8, "the bars stay bottom left");
        assert!(
            layout.stamina.track.bottom() > 720 - 40,
            "the bars stay low"
        );
    }

    #[test]
    fn legionary_hud_weapon_icon_sits_bottom_right() {
        let layout = legionary_hud().layout();
        let icon = &layout.weapon;
        assert_eq!(icon.weapon, LegionaryWeapon::Gladius);
        assert!(!icon.parts.is_empty(), "the icon draws parts");
        let right = icon
            .parts
            .iter()
            .map(|part| part.right())
            .max()
            .expect("the icon has parts");
        let bottom = icon
            .parts
            .iter()
            .map(|part| part.bottom())
            .max()
            .expect("the icon has parts");
        assert_eq!(right, 1920 - 58, "the icon keeps its right margin");
        assert_eq!(bottom, 1080 - 65, "the icon keeps its bottom margin");
        assert!(right > 1920 - 400, "the icon hugs the right edge");
        assert!(bottom > 1080 - 120, "the icon hugs the bottom edge");
        // The gladius points left: the tip is the leftmost and the thinnest part.
        let tip = icon
            .parts
            .iter()
            .min_by_key(|part| part.x)
            .expect("the icon has parts");
        assert!(tip.h < icon.parts[1].h, "the tip is thinner than the blade");
    }

    #[test]
    fn legionary_hud_objective_line_sits_top_right() {
        let layout = legionary_hud().layout();
        assert_eq!(layout.objective_label.text, LEGIONARY_OBJECTIVE_LABEL);
        assert_eq!(layout.objective.text, LEGIONARY_DEFAULT_OBJECTIVE);
        assert_eq!(
            layout.objective.at.right(),
            1920 - 192,
            "the objective line ends at the picture's right margin"
        );
        assert_eq!(
            layout.objective_label.at.right(),
            layout.objective.at.right(),
            "the label and the line share the right edge"
        );
        assert!(
            layout.objective.at.y < 1080 / 4,
            "the line sits near the top"
        );
        assert!(
            layout.objective.at.y > layout.objective_label.at.y,
            "the label sits above the line"
        );
    }

    #[test]
    fn legionary_hud_text_runs_end_on_the_margin() {
        // The HUD pass draws one square cell of `px` pixels per character and
        // steps `px` pixels per character (crates/render/src/hud_pass.rs), so
        // a right aligned run must reserve that width, or it overhangs the
        // margin it sits on.
        assert_eq!(legionary_text_width("OBJECTIVE:", 28), 10 * 28);
        assert_eq!(legionary_text_width("MARCH TO THE CAMP.", 28), 18 * 28);
        assert_eq!(legionary_text_width("", 28), 0);
        let layout = legionary_hud().layout();
        let text_px = i32::from(text::hud_text_px(1080));
        assert_eq!(
            layout.objective.at.right() - layout.objective.at.x,
            18 * text_px,
            "the line is as wide as the HUD pass draws it"
        );
        assert_eq!(
            layout.objective_label.at.right() - layout.objective_label.at.x,
            10 * text_px,
            "the label is as wide as the HUD pass draws it"
        );
    }

    #[test]
    fn legionary_hud_draws_nothing_else() {
        let hud = legionary_hud();
        let layout = hud.layout();
        let frame = hud.frame();
        let (rects, borders, texts) = frame.batch_counts();
        assert_eq!(borders, 0, "the legionary HUD draws no borders");
        assert_eq!(
            texts, 3,
            "the health line, the objective label and the line"
        );
        assert_eq!(
            rects,
            4 + layout.weapon.parts.len(),
            "two tracks, two fills and the icon parts, nothing else"
        );
        for command in frame.commands() {
            assert!(
                !matches!(command, HudCommand::Scissor { .. }),
                "the legionary HUD draws no scissor"
            );
            if let HudCommand::Rect { x, y, w, h, .. } = *command {
                assert!(x >= 0 && y >= 0, "every part stays on the screen");
                assert!(
                    x.saturating_add(w) <= 1920 && y.saturating_add(h) <= 1080,
                    "every part stays inside the viewport"
                );
            }
        }
    }

    #[test]
    fn legionary_hud_fills_follow_health_and_stamina() {
        let hud = LegionaryHud::new(
            1920,
            1080,
            780,
            400,
            LegionaryWeapon::Gladius,
            LEGIONARY_DEFAULT_OBJECTIVE,
        );
        let layout = hud.layout();
        assert_eq!(layout.hp.fill.w, 244 * 780 / 1000);
        assert_eq!(layout.stamina.fill.w, 244 * 400 / 1000);
        assert_eq!(layout.hp.fill.x, layout.hp.track.x);
        assert_eq!(layout.hp.fill.h, layout.hp.track.h);
        assert!(
            layout.hp.fill.w < layout.hp.track.w,
            "the fill stays in its track"
        );
        assert!(layout.stamina.fill.w < layout.stamina.track.w);
    }

    #[test]
    fn legionary_hud_objective_comes_from_the_caller() {
        let hud = LegionaryHud::new(
            1920,
            1080,
            1000,
            1000,
            LegionaryWeapon::Pilum,
            "HOLD THE LINE",
        );
        assert_eq!(hud.objective(), "HOLD THE LINE");
        let frame = hud.frame();
        let texts: Vec<&str> = frame
            .commands()
            .iter()
            .filter_map(|row| row.text())
            .collect();
        assert!(texts.contains(&"HOLD THE LINE"), "the line is drawn");
        assert!(
            texts.contains(&LEGIONARY_OBJECTIVE_LABEL),
            "the label is drawn"
        );
    }

    #[test]
    fn legionary_hud_clamps_health_and_clips_the_objective() {
        let long = "M".repeat(LEGIONARY_MAX_OBJECTIVE_CHARS + 12);
        let hud = LegionaryHud::new(1920, 1080, 5000, 5000, LegionaryWeapon::Scutum, &long);
        assert_eq!(hud.hp_permille(), PERMILLE_MAX);
        assert_eq!(hud.stamina_permille(), PERMILLE_MAX);
        assert_eq!(
            hud.objective().chars().count(),
            LEGIONARY_MAX_OBJECTIVE_CHARS,
            "the long line clips"
        );
        let layout = hud.layout();
        assert_eq!(
            layout.hp.fill.w, layout.hp.track.w,
            "a full bar fills the track"
        );
        let tiny = LegionaryHud::new(1, 1, 0, 0, LegionaryWeapon::Pilum, "");
        assert_eq!(tiny.hp_permille(), 0);
        assert_eq!(tiny.stamina_permille(), 0);
    }

    #[test]
    fn legionary_hud_kit_icons_differ() {
        let mut shapes: Vec<Vec<LegionaryRect>> = Vec::new();
        for weapon in [
            LegionaryWeapon::Gladius,
            LegionaryWeapon::Scutum,
            LegionaryWeapon::Pilum,
        ] {
            let hud =
                LegionaryHud::new(1920, 1080, 1000, 1000, weapon, LEGIONARY_DEFAULT_OBJECTIVE);
            shapes.push(hud.layout().weapon.parts.clone());
        }
        assert_ne!(shapes[0], shapes[1], "the gladius is not the scutum");
        assert_ne!(shapes[1], shapes[2], "the scutum is not the pilum");
        assert_ne!(shapes[0], shapes[2], "the gladius is not the pilum");
        for shape in &shapes {
            assert!(!shape.is_empty(), "every icon draws parts");
        }
    }

    #[test]
    fn legionary_hud_is_the_same_twice() {
        assert_eq!(legionary_hud().frame(), legionary_hud().frame());
        assert_eq!(legionary_hud().layout(), legionary_hud().layout());
    }

    /// Write the legionary HUD frame as JSON for the headless capture
    /// renderer: the layout the crate computed, nothing invented. Test-only.
    fn write_legionary_capture(hud: &LegionaryHud, path: &std::path::Path) {
        let frame = hud.frame();
        let mut json = String::from("{\"width\":");
        json.push_str(&frame.width().to_string());
        json.push_str(",\"height\":");
        json.push_str(&frame.height().to_string());
        json.push_str(",\"text_px\":");
        json.push_str(&frame.text_px().to_string());
        json.push_str(",\"commands\":[");
        let mut glyphs: Vec<char> = Vec::new();
        for (index, command) in frame.commands().iter().enumerate() {
            if index > 0 {
                json.push(',');
            }
            match command {
                HudCommand::Rect {
                    x, y, w, h, color, ..
                } => {
                    json.push_str(&format!(
                        "{{\"kind\":\"rect\",\"x\":{x},\"y\":{y},\"w\":{w},\"h\":{h},\"color\":[{},{},{},{}]}}",
                        color[0], color[1], color[2], color[3]
                    ));
                }
                HudCommand::Text {
                    x,
                    y,
                    px,
                    color,
                    text,
                    ..
                } => {
                    json.push_str(&format!(
                        "{{\"kind\":\"text\",\"x\":{x},\"y\":{y},\"px\":{px},\"color\":[{},{},{},{}],\"text\":\"{text}\"}}",
                        color[0], color[1], color[2], color[3]
                    ));
                    for glyph in text.chars() {
                        if !glyphs.contains(&glyph) {
                            glyphs.push(glyph);
                        }
                    }
                }
                _ => {}
            }
        }
        json.push_str("],\"glyphs\":{");
        let atlas = text::Atlas::new();
        for (index, glyph) in glyphs.iter().enumerate() {
            if index > 0 {
                json.push(',');
            }
            json.push_str(&format!("\"{glyph}\":["));
            let rows = atlas.bitmap(*glyph).unwrap_or([0; 7]);
            for (row_index, row) in rows.iter().enumerate() {
                if row_index > 0 {
                    json.push(',');
                }
                json.push_str(&row.to_string());
            }
            json.push(']');
        }
        json.push_str("}}");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("the report directory must be writable");
        }
        std::fs::write(path, json).expect("the capture dump must be written");
    }

    #[test]
    fn legionary_hud_capture_dumps_the_layout() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/reports");
        for (width, height) in [(1920u32, 1080u32), (1280, 720)] {
            let hud = LegionaryHud::new(
                width,
                height,
                780,
                720,
                LegionaryWeapon::Gladius,
                LEGIONARY_DEFAULT_OBJECTIVE,
            );
            let path = dir.join(format!("legionary-hud-{width}x{height}.json"));
            write_legionary_capture(&hud, &path);
            let dumped = std::fs::read_to_string(&path).expect("the dump must be readable");
            assert!(dumped.contains("\"kind\":\"rect\""), "the bars are dumped");
            assert!(dumped.contains("\"kind\":\"text\""), "the lines are dumped");
            assert!(
                dumped.contains(LEGIONARY_DEFAULT_OBJECTIVE),
                "the objective line is dumped"
            );
        }
    }
}
