//! play_hud: per-frame play HUD snapshot as pure data.
//!
//! Consumer: the `play.step` rasterizer in `crates/game/src/windowed.rs`
//! (today `duel_overlay_for_app` plus `hud_cells_quads`; the play-step frame
//! builder reads [`PlayHud`] beside [`DuelOverlay`](super::DuelOverlay), so
//! HP bars and names ride the overlay while the button echo, button hints,
//! fighter-you arrow, FPS crosshair, hit flash and ammo ride this snapshot).
//! The windowed call site is frozen this packet, so the in-crate
//! `FrozenPlayFrame` stub in tests pins the draw contract the same way
//! [`DuelOverlay`](super::DuelOverlay)'s `FrozenFrame` does: decoded wire
//! bytes drive every painted element. Data owner: `engine-ui`. Units: ticks
//! are fixed steps, ammo is whole rounds, crosshair spread is whole pixels,
//! the button echo is `engine-input` button bits, hint lines are ASCII.
//! Determinism: every update is a pure function on an owned struct; hint
//! order is fixed (attack, block, dodge); the wire is little-endian with
//! `u8::MAX` sentinels for the absent `you`; `from_seed` builds identical
//! bytes from one seed. Budgets: the echo is one byte, hints are at most
//! [`MAX_HINT_LINES`] ASCII lines of at most [`MAX_HINT_LEN`] chars, the wire
//! is exactly [`PLAY_HUD_MAX_BYTES`] bytes; heap (`String`, `Vec`) lives on
//! the presentation path only (`lines`, `to_bytes`), zero per simulation
//! tick. Failure mode: invalid input returns [`PlayHudError`]; updates never
//! panic.

use engine_input::fighter::{ATTACK, BLOCK, DODGE, KNOWN_BUTTONS};
use engine_input::MAX_PARTICIPANTS;
use engine_input::{Bindings, BoundAction, Buttons, ParticipantId, PhysicalButton};

use super::PERMILLE_MAX;

/// Wire version of the play HUD snapshot.
pub const PLAY_HUD_VERSION: u8 = 1;
/// Ticks an FPS hit marker stays visible after the hit tick, inclusive.
/// Shorter than the duel overlay flash: the marker is a hit confirm, not a
/// damage readout.
pub const HIT_FLASH_TICKS: u64 = 12;
/// Widest crosshair spread, in whole pixels; wider spreads clamp.
pub const MAX_SPREAD_PX: u8 = 64;
/// Button hint lines per snapshot: attack, block, dodge, in that order.
pub const MAX_HINT_LINES: usize = 3;
/// Longest hint line, in chars; labels are bounded so lines never reach it.
pub const MAX_HINT_LEN: usize = 24;
/// Exact encoded snapshot size, in bytes: version (1) + tick (8) + you (1) +
/// echo (1) + ammo (4) + max ammo (4) + crosshair visible (1) + spread (1) +
/// hit flag (1) + hit tick (8).
pub const PLAY_HUD_MAX_BYTES: usize = 30;

/// Absent-you sentinel on the wire; no fighter slot reaches `u8::MAX`.
const NO_YOU: u8 = u8::MAX;

/// Play HUD failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PlayHudError {
    /// Fighter slot `>= MAX_PARTICIPANTS`, culprit carried.
    UnknownBody(u8),
    /// Echo byte with bits outside `KNOWN_BUTTONS`, raw byte carried.
    UnknownButtons(u8),
    /// Flag byte outside `0..=1`, culprit carried.
    BadFlag(u8),
    /// Wire version is not `PLAY_HUD_VERSION`, culprit carried.
    BadVersion(u8),
    /// Byte slice ends mid-field.
    UnexpectedEnd,
    /// Bytes trail the decoded snapshot, remainder carried.
    TrailingBytes(usize),
}

impl core::fmt::Display for PlayHudError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownBody(id) => write!(f, "unknown fighter {id}"),
            Self::UnknownButtons(byte) => write!(f, "unknown buttons {byte}"),
            Self::BadFlag(flag) => write!(f, "bad flag {flag}"),
            Self::BadVersion(version) => write!(f, "bad play HUD version {version}"),
            Self::UnexpectedEnd => write!(f, "truncated play HUD bytes"),
            Self::TrailingBytes(n) => write!(f, "trailing play HUD bytes ({n})"),
        }
    }
}

impl std::error::Error for PlayHudError {}

/// FPS crosshair: visibility plus spread in whole pixels. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Crosshair {
    visible: bool,
    spread_px: u8,
}

impl Crosshair {
    /// Crosshair with `spread_px` clamped to [`MAX_SPREAD_PX`]. Pure.
    #[must_use]
    pub const fn new(visible: bool, spread_px: u8) -> Self {
        Self {
            visible,
            spread_px: if spread_px > MAX_SPREAD_PX {
                MAX_SPREAD_PX
            } else {
                spread_px
            },
        }
    }

    /// True when the rasterizer draws the crosshair.
    #[must_use]
    pub const fn visible(self) -> bool {
        self.visible
    }

    /// Spread in whole pixels, `0..=MAX_SPREAD_PX`.
    #[must_use]
    pub const fn spread_px(self) -> u8 {
        self.spread_px
    }

    /// Replace the spread; over-bound values clamp. Pure.
    pub fn set_spread(&mut self, spread_px: u8) {
        self.spread_px = spread_px.min(MAX_SPREAD_PX);
    }
}

/// Per-frame play HUD snapshot: the fighter-you arrow, button echo, ammo,
/// crosshair and hit marker for one fixed tick. HP bars and names ride the
/// paired [`DuelOverlay`](super::DuelOverlay); this snapshot rides beside it.
/// Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PlayHud {
    tick: u64,
    you: Option<ParticipantId>,
    echo: Buttons,
    ammo: u32,
    max_ammo: u32,
    crosshair: Crosshair,
    last_hit: Option<u64>,
}

/// Take `count` bytes, advancing the cursor. Pure.
fn take_n<'bytes>(
    bytes: &'bytes [u8],
    cursor: &mut usize,
    count: usize,
) -> Result<&'bytes [u8], PlayHudError> {
    let end = cursor
        .checked_add(count)
        .ok_or(PlayHudError::UnexpectedEnd)?;
    let slice = bytes.get(*cursor..end).ok_or(PlayHudError::UnexpectedEnd)?;
    *cursor = end;
    Ok(slice)
}

/// Take one byte, advancing the cursor. Pure.
fn take_byte(bytes: &[u8], cursor: &mut usize) -> Result<u8, PlayHudError> {
    take_n(bytes, cursor, 1)?
        .first()
        .copied()
        .ok_or(PlayHudError::UnexpectedEnd)
}

/// Take a little-endian `u32`, advancing the cursor. Pure.
fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, PlayHudError> {
    let slice = take_n(bytes, cursor, 4)?;
    let raw: [u8; 4] = slice.try_into().map_err(|_| PlayHudError::UnexpectedEnd)?;
    Ok(u32::from_le_bytes(raw))
}

/// Take a little-endian `u64`, advancing the cursor. Pure.
fn take_u64(bytes: &[u8], cursor: &mut usize) -> Result<u64, PlayHudError> {
    let slice = take_n(bytes, cursor, 8)?;
    let raw: [u8; 8] = slice.try_into().map_err(|_| PlayHudError::UnexpectedEnd)?;
    Ok(u64::from_le_bytes(raw))
}

/// Take a `0`/`1` flag byte, advancing the cursor. Pure.
fn take_flag(bytes: &[u8], cursor: &mut usize) -> Result<bool, PlayHudError> {
    match take_byte(bytes, cursor)? {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(PlayHudError::BadFlag(other)),
    }
}

/// Reject slots outside `engine-input`; shared by the arrow and the wire.
/// Pure.
fn check_you(id: ParticipantId) -> Result<(), PlayHudError> {
    if id.get() >= MAX_PARTICIPANTS {
        return Err(PlayHudError::UnknownBody(id.get()));
    }
    Ok(())
}

/// Reject echo bytes with bits outside the fighter contract. Pure.
fn check_echo(echo: Buttons) -> Result<(), PlayHudError> {
    if echo.is_legal() {
        Ok(())
    } else {
        Err(PlayHudError::UnknownButtons(echo.0))
    }
}

impl PlayHud {
    /// Empty snapshot for `tick`: no you arrow, silent echo, zero ammo with
    /// unknown max, hidden crosshair, no hit. Pure.
    #[must_use]
    pub const fn new(tick: u64) -> Self {
        Self {
            tick,
            you: None,
            echo: Buttons::NONE,
            ammo: 0,
            max_ammo: 0,
            crosshair: Crosshair::new(false, 0),
            last_hit: None,
        }
    }

    /// Snapshot pairing one overlay tick with every play layer; the bridge
    /// [`DuelOverlay::play_hud`](super::DuelOverlay::play_hud) calls this, so
    /// the overlay tick and the play layers never split across frames. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`PlayHudError`] for bad you slots or illegal echo bits.
    pub fn from_parts(
        tick: u64,
        you: Option<ParticipantId>,
        echo: Buttons,
        ammo: u32,
        max_ammo: u32,
        crosshair: Crosshair,
        hit_tick: Option<u64>,
    ) -> Result<Self, PlayHudError> {
        let mut hud = Self::new(tick);
        hud.set_you(you)?;
        hud.set_echo(echo)?;
        hud.set_ammo(ammo, max_ammo);
        hud.set_crosshair(crosshair);
        if let Some(at) = hit_tick {
            hud.record_hit(at);
        }
        Ok(hud)
    }

    /// Deterministic seed95-style fixture from one seed: same seed, same
    /// bytes. Tick lands in `600..1200`, the arrow marks slot 0 or 1, echo
    /// carries contract bits, ammo sits inside a 30-round mag, the crosshair
    /// spreads `0..=32` px, and the hit stays inside the flash window with
    /// plain integer math, so no RNG owner is involved. Pure.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mixed: u64 = seed | 1;
        let tick: u64 = (mixed % 600).wrapping_add(600);
        let you_slot = u8::try_from((mixed >> 3) & 1).unwrap_or(0);
        let echo_byte = u8::try_from((mixed >> 5) & u64::from(KNOWN_BUTTONS)).unwrap_or(0);
        let ammo = u32::try_from((mixed >> 8) % 31).unwrap_or(0);
        let spread = u8::try_from((mixed >> 14) % 33).unwrap_or(0);
        let hit_ago: u64 = mixed % (HIT_FLASH_TICKS + 1);
        Self {
            tick,
            you: ParticipantId::new(you_slot),
            echo: Buttons(echo_byte),
            ammo,
            max_ammo: 30,
            crosshair: Crosshair::new((mixed >> 13) & 1 != 0, spread),
            last_hit: Some(tick.saturating_sub(hit_ago)),
        }
    }

    /// Fixed tick this frame describes.
    #[must_use]
    pub const fn tick(self) -> u64 {
        self.tick
    }

    /// Move the frame clock; every layer is untouched. Pure.
    pub fn set_tick(&mut self, tick: u64) {
        self.tick = tick;
    }

    /// Fighter carrying the you arrow, if any.
    #[must_use]
    pub const fn you(self) -> Option<ParticipantId> {
        self.you
    }

    /// True when `id` carries the you arrow. Pure.
    #[must_use]
    pub fn is_you(self, id: ParticipantId) -> bool {
        self.you == Some(id)
    }

    /// Move the you arrow; `None` hides it. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`PlayHudError::UnknownBody`] for slots outside the cap.
    pub fn set_you(&mut self, you: Option<ParticipantId>) -> Result<(), PlayHudError> {
        if let Some(id) = you {
            check_you(id)?;
        }
        self.you = you;
        Ok(())
    }

    /// Button echo bits for this tick.
    #[must_use]
    pub const fn echo(self) -> Buttons {
        self.echo
    }

    /// Replace the button echo; unknown bits fail and keep the old echo.
    /// Pure.
    ///
    /// # Errors
    ///
    /// Returns [`PlayHudError::UnknownButtons`] for bits outside the
    /// fighter contract.
    pub fn set_echo(&mut self, echo: Buttons) -> Result<(), PlayHudError> {
        check_echo(echo)?;
        self.echo = echo;
        Ok(())
    }

    /// Rounds in the mag.
    #[must_use]
    pub const fn ammo(self) -> u32 {
        self.ammo
    }

    /// Mag size; `0` means the snapshot carries no max.
    #[must_use]
    pub const fn max_ammo(self) -> u32 {
        self.max_ammo
    }

    /// Replace ammo; over-max values clamp into the mag, and a zero max
    /// stores ammo as-is. Pure.
    pub fn set_ammo(&mut self, ammo: u32, max_ammo: u32) {
        self.max_ammo = max_ammo;
        self.ammo = if max_ammo == 0 {
            ammo
        } else {
            ammo.min(max_ammo)
        };
    }

    /// Ammo as permille `0..=1000` for the bar; unknown max reads empty.
    /// Plain integer math, no floats. Pure.
    #[must_use]
    pub fn ammo_fraction_permille(self) -> u16 {
        if self.max_ammo == 0 {
            return 0;
        }
        let capped = self.ammo.min(self.max_ammo);
        let scaled = u64::from(capped).saturating_mul(1000) / u64::from(self.max_ammo);
        u16::try_from(scaled).unwrap_or(PERMILLE_MAX)
    }

    /// FPS crosshair.
    #[must_use]
    pub const fn crosshair(self) -> Crosshair {
        self.crosshair
    }

    /// Replace the crosshair. Pure.
    pub fn set_crosshair(&mut self, crosshair: Crosshair) {
        self.crosshair = crosshair;
    }

    /// Tick the last hit landed on, if any.
    #[must_use]
    pub const fn last_hit(self) -> Option<u64> {
        self.last_hit
    }

    /// Record a landed hit for the FPS hit marker. Pure.
    pub fn record_hit(&mut self, hit_tick: u64) {
        self.last_hit = Some(hit_tick);
    }

    /// Hide the hit marker. Pure.
    pub fn clear_hit(&mut self) {
        self.last_hit = None;
    }

    /// True when a hit is recorded and the frame tick is at most
    /// [`HIT_FLASH_TICKS`] past it. Pure.
    #[must_use]
    pub fn hit_flash_visible(self) -> bool {
        match self.last_hit {
            Some(at) => self.tick >= at && self.tick - at <= HIT_FLASH_TICKS,
            None => false,
        }
    }

    /// Echo line for the text path: `echo attack+block`, `echo -` when
    /// silent. ASCII-only. Pure; heap on the presentation path only.
    #[must_use]
    pub fn echo_line(self) -> String {
        let mut line = String::from("echo ");
        let mut plus = false;
        for (bit, word) in [(ATTACK, "attack"), (BLOCK, "block"), (DODGE, "dodge")] {
            if self.echo.0 & bit != 0 {
                if plus {
                    line.push('+');
                }
                line.push_str(word);
                plus = true;
            }
        }
        if !plus {
            line.push('-');
        }
        line
    }

    /// Ammo line for the text path: `AMMO 12/30`. ASCII-only. Pure; heap on
    /// the presentation path only.
    #[must_use]
    pub fn ammo_line(self) -> String {
        format!("AMMO {}/{}", self.ammo, self.max_ammo)
    }

    /// Tick line for the text path: `TICK 95`. ASCII-only. Pure; heap on the
    /// presentation path only.
    #[must_use]
    pub fn tick_line(self) -> String {
        format!("TICK {}", self.tick)
    }

    /// You-arrow line for the text path: `YOU P1`, `YOU -` with no arrow.
    /// ASCII-only. Pure; heap on the presentation path only.
    #[must_use]
    pub fn you_line(self) -> String {
        match self.you {
            Some(id) => format!("YOU P{}", id.get()),
            None => "YOU -".to_owned(),
        }
    }

    /// Short label for one bound button: key names for the engine key codes
    /// (`engine-input` `Key`: 0 space, 1 shift, 2 ctrl, 3 `W`, 4 `A`, 5 `S`,
    /// 6 `D`, 7 `E`, 8 `F`, 9 `Q`, 10 esc, 11 enter), `KEY<code>` past them,
    /// `MOUSE<i>` / `PAD<i>` for device buttons. ASCII-only. Pure; heap on
    /// the presentation path only.
    #[must_use]
    pub fn button_label(button: PhysicalButton) -> String {
        match button {
            PhysicalButton::Key(0) => "SPACE".to_owned(),
            PhysicalButton::Key(1) => "SHIFT".to_owned(),
            PhysicalButton::Key(2) => "CTRL".to_owned(),
            PhysicalButton::Key(3) => "W".to_owned(),
            PhysicalButton::Key(4) => "A".to_owned(),
            PhysicalButton::Key(5) => "S".to_owned(),
            PhysicalButton::Key(6) => "D".to_owned(),
            PhysicalButton::Key(7) => "E".to_owned(),
            PhysicalButton::Key(8) => "F".to_owned(),
            PhysicalButton::Key(9) => "Q".to_owned(),
            PhysicalButton::Key(10) => "ESC".to_owned(),
            PhysicalButton::Key(11) => "ENTER".to_owned(),
            PhysicalButton::Key(other) => format!("KEY{other}"),
            PhysicalButton::Mouse(index) => format!("MOUSE{index}"),
            PhysicalButton::Pad(index) => format!("PAD{index}"),
        }
    }

    /// Button hint lines for the text path in fixed order: attack, block,
    /// dodge as `<label> <action>`. ASCII-only, each within
    /// [`MAX_HINT_LEN`]. Pure; heap on the presentation path only.
    #[must_use]
    pub fn hint_lines(&self, bindings: &Bindings) -> [String; MAX_HINT_LINES] {
        const HINTS: [(BoundAction, &str); MAX_HINT_LINES] = [
            (BoundAction::Attack, "attack"),
            (BoundAction::Block, "block"),
            (BoundAction::Dodge, "dodge"),
        ];
        HINTS
            .map(|(action, word)| format!("{} {word}", Self::button_label(bindings.button(action))))
    }

    /// Every drawable text line in paint order: tick, echo, ammo, you arrow,
    /// then the button hints. The rasterizer lays each out through
    /// [`text::layout`](super::text::layout). Pure; heap on the presentation
    /// path only.
    #[must_use]
    pub fn lines(&self, bindings: &Bindings) -> Vec<String> {
        let mut out = Vec::with_capacity(4 + MAX_HINT_LINES);
        out.push(self.tick_line());
        out.push(self.echo_line());
        out.push(self.ammo_line());
        out.push(self.you_line());
        out.extend(self.hint_lines(bindings));
        out
    }

    /// Canonical wire bytes: version, tick, you slot (`u8::MAX` when hidden),
    /// echo bits, ammo, max ammo, crosshair visible, spread, hit flag, hit
    /// tick, all integers little-endian. Appends to `out` for buffer reuse.
    /// Pure.
    pub fn append_bytes(&self, out: &mut Vec<u8>) {
        out.push(PLAY_HUD_VERSION);
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.push(self.you.map_or(NO_YOU, ParticipantId::get));
        out.push(self.echo.0);
        out.extend_from_slice(&self.ammo.to_le_bytes());
        out.extend_from_slice(&self.max_ammo.to_le_bytes());
        out.push(u8::from(self.crosshair.visible));
        out.push(self.crosshair.spread_px);
        out.push(u8::from(self.last_hit.is_some()));
        out.extend_from_slice(&self.last_hit.unwrap_or(0).to_le_bytes());
    }

    /// Canonical wire bytes in a fresh, exactly (`PLAY_HUD_MAX_BYTES`)
    /// sized buffer. Pure.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PLAY_HUD_MAX_BYTES);
        self.append_bytes(&mut out);
        out
    }

    /// Decode canonical wire bytes, rejecting bad versions, bad slots, bad
    /// echo bits, bad flags and trailing bytes. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`PlayHudError`] for any malformed or out-of-range input.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PlayHudError> {
        let mut cursor: usize = 0;
        let version = take_byte(bytes, &mut cursor)?;
        if version != PLAY_HUD_VERSION {
            return Err(PlayHudError::BadVersion(version));
        }
        let tick = take_u64(bytes, &mut cursor)?;
        let you_byte = take_byte(bytes, &mut cursor)?;
        let you = if you_byte == NO_YOU {
            None
        } else {
            let id = ParticipantId::new(you_byte).ok_or(PlayHudError::UnknownBody(you_byte))?;
            Some(id)
        };
        let echo_byte = take_byte(bytes, &mut cursor)?;
        let echo = Buttons(echo_byte);
        check_echo(echo)?;
        let ammo = take_u32(bytes, &mut cursor)?;
        let max_ammo = take_u32(bytes, &mut cursor)?;
        let visible = take_flag(bytes, &mut cursor)?;
        let spread = take_byte(bytes, &mut cursor)?;
        let has_hit = take_flag(bytes, &mut cursor)?;
        let hit_at = take_u64(bytes, &mut cursor)?;
        if cursor != bytes.len() {
            return Err(PlayHudError::TrailingBytes(bytes.len() - cursor));
        }
        let mut hud = Self {
            tick,
            you,
            echo,
            ammo: 0,
            max_ammo: 0,
            crosshair: Crosshair::new(visible, spread),
            last_hit: None,
        };
        hud.set_ammo(ammo, max_ammo);
        if has_hit {
            hud.record_hit(hit_at);
        }
        Ok(hud)
    }

    /// Presentation change checksum over the canonical fields. This digest is
    /// a view receipt outside the simulation hash scope, so it uses a plain
    /// wrapping multiply-add fold like the duel overlay digest. Pure.
    #[must_use]
    pub fn state_digest(self) -> u64 {
        let mut digest: u64 = 0;
        let mut mix = |byte: u8| {
            digest = digest.wrapping_mul(131).wrapping_add(u64::from(byte));
        };
        mix(PLAY_HUD_VERSION);
        for byte in self.tick.to_le_bytes() {
            mix(byte);
        }
        mix(self.you.map_or(NO_YOU, ParticipantId::get));
        mix(self.echo.0);
        for byte in self.ammo.to_le_bytes() {
            mix(byte);
        }
        for byte in self.max_ammo.to_le_bytes() {
            mix(byte);
        }
        mix(u8::from(self.crosshair.visible));
        mix(self.crosshair.spread_px);
        mix(u8::from(self.last_hit.is_some()));
        for byte in self.last_hit.unwrap_or(0).to_le_bytes() {
            mix(byte);
        }
        digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::fighter::{ATTACK, BLOCK, DODGE};
    use engine_input::{Bindings, BoundAction, Buttons, FighterId, ParticipantId, PhysicalButton};

    use super::super::text::{layout, Align, Atlas};
    use crate::DuelOverlay;

    fn fighter(slot: u8) -> ParticipantId {
        ParticipantId::new(slot).expect("test slot must be valid")
    }

    /// Seed95 nod: the board accept names seed95 PNGs, so the sample frame
    /// sits on tick 95 with a hit five ticks back (inside the flash window).
    fn sample_play() -> PlayHud {
        let mut hud = PlayHud::new(95);
        hud.set_you(Some(fighter(1)))
            .expect("slot 1 takes the arrow");
        hud.set_echo(Buttons(ATTACK | DODGE))
            .expect("attack+dodge is legal");
        hud.set_ammo(12, 30);
        hud.set_crosshair(Crosshair::new(true, 8));
        hud.record_hit(90);
        hud
    }

    /// Frozen raster stub: the `play.step` draw contract without the game.
    /// Paints the tick header, one text lane per `lines()` entry, the ammo
    /// bar, overlay names plus HP pips, the you arrow, the crosshair plus
    /// and the hit border, so the demo proves decoded bytes drive every
    /// play element. Stub geometry only; shipped overlay coords live in
    /// render/game. Test-only.
    struct FrozenPlayFrame {
        width: usize,
        height: usize,
        pixels: Vec<[u8; 3]>,
    }

    impl FrozenPlayFrame {
        fn new(width: usize, height: usize, bg: [u8; 3]) -> Self {
            Self {
                width,
                height,
                pixels: vec![bg; width.saturating_mul(height)],
            }
        }

        fn set(&mut self, x: usize, y: usize, color: [u8; 3]) {
            if x < self.width && y < self.height {
                let index = y.saturating_mul(self.width).saturating_add(x);
                if let Some(slot) = self.pixels.get_mut(index) {
                    *slot = color;
                }
            }
        }

        fn draw_play(&mut self, overlay: &DuelOverlay, hud: &PlayHud, bindings: &Bindings) {
            let palette = overlay.palette();
            for bit in 0..64u32 {
                if hud.tick() & (1u64 << bit) != 0 {
                    let x = usize::try_from(bit).expect("header bit fits the address space");
                    self.set(x, 0, palette.fg);
                }
            }
            let atlas = Atlas::new();
            let lines = hud.lines(bindings);
            for (index, line) in lines.iter().enumerate() {
                let base_y = 8usize.saturating_add(index.saturating_mul(8));
                for cell in layout(line, 24, Align::Left) {
                    let cell_x =
                        usize::try_from(cell.cell_x).expect("layout cell fits the address space");
                    for glyph_y in 0..Atlas::GLYPH_H {
                        for glyph_x in 0..Atlas::GLYPH_W {
                            if atlas.pixel(cell.codepoint, glyph_x, glyph_y) {
                                let x = cell_x.saturating_mul(4).saturating_add(
                                    usize::try_from(glyph_x)
                                        .expect("5-pixel glyph width fits the address space"),
                                );
                                let y = base_y.saturating_add(
                                    usize::try_from(glyph_y)
                                        .expect("7-pixel glyph height fits the address space"),
                                );
                                self.set(x, y, palette.fg);
                            }
                        }
                    }
                }
            }
            let pips = hud.ammo_fraction_permille() / 100;
            let bar_y = 8usize
                .saturating_add(lines.len().saturating_mul(8))
                .saturating_add(4);
            for pip in 0..pips {
                let bar_x = 96usize.saturating_add(usize::from(pip).saturating_mul(3));
                for dy in 0..5usize {
                    for dx in 0..3usize {
                        self.set(
                            bar_x.saturating_add(dx),
                            bar_y.saturating_add(dy),
                            palette.accent,
                        );
                    }
                }
            }
            let overlay_base = bar_y.saturating_add(8);
            for (index, row) in overlay.rows().iter().enumerate() {
                let base_y = overlay_base.saturating_add(index.saturating_mul(12));
                for cell in layout(row.name(), 24, Align::Left) {
                    let cell_x =
                        usize::try_from(cell.cell_x).expect("layout cell fits the address space");
                    for glyph_y in 0..Atlas::GLYPH_H {
                        for glyph_x in 0..Atlas::GLYPH_W {
                            if atlas.pixel(cell.codepoint, glyph_x, glyph_y) {
                                let x = cell_x.saturating_mul(4).saturating_add(
                                    usize::try_from(glyph_x)
                                        .expect("5-pixel glyph width fits the address space"),
                                );
                                let y = base_y.saturating_add(
                                    usize::try_from(glyph_y)
                                        .expect("7-pixel glyph height fits the address space"),
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
            }
            if let Some(you) = hud.you() {
                if let Some(row) = overlay.rows().iter().position(|row| row.id() == you) {
                    let arrow_y = 8usize.saturating_add(row.saturating_mul(8));
                    for (dy, width) in [5usize, 3, 1].iter().enumerate() {
                        for dx in 0..*width {
                            self.set(
                                90usize.saturating_add(dx),
                                arrow_y.saturating_add(dy),
                                palette.accent,
                            );
                        }
                    }
                }
            }
            if hud.crosshair().visible() {
                let (center_x, center_y) = (self.width / 2, self.height / 2);
                let arm = 3usize.saturating_add(usize::from(hud.crosshair().spread_px()) / 8);
                for arm_at in 2..arm {
                    self.set(center_x.saturating_add(arm_at), center_y, palette.accent);
                    self.set(center_x.saturating_sub(arm_at), center_y, palette.accent);
                    self.set(center_x, center_y.saturating_add(arm_at), palette.accent);
                    self.set(center_x, center_y.saturating_sub(arm_at), palette.accent);
                }
                self.set(center_x, center_y, palette.accent);
            }
            if hud.hit_flash_visible() {
                for x in 0..self.width {
                    self.set(x, 1, palette.accent);
                    self.set(x, self.height.saturating_sub(1), palette.accent);
                }
                for y in 1..self.height {
                    self.set(0, y, palette.accent);
                    self.set(self.width.saturating_sub(1), y, palette.accent);
                }
            }
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

    fn sample_overlay() -> DuelOverlay {
        let mut overlay = DuelOverlay::new(95);
        overlay
            .upsert_fighter(fighter(0), "Rook", 750)
            .expect("row 0 tracks");
        overlay
            .upsert_fighter(fighter(1), "Sable", 250)
            .expect("row 1 tracks");
        overlay
    }

    #[test]
    fn seeded_bytes_identical_twice() {
        let first = PlayHud::from_seed(95);
        let second = PlayHud::from_seed(95);
        assert_eq!(first.to_bytes(), second.to_bytes());
        assert_eq!(first.state_digest(), second.state_digest());
        let other = PlayHud::from_seed(96);
        assert_ne!(
            first.to_bytes(),
            other.to_bytes(),
            "distinct seeds build distinct play frames"
        );
        assert!(first.hit_flash_visible(), "seeded hit stays flashing");
        assert!(first.you().is_some(), "seeded frame marks a you fighter");
    }

    #[test]
    fn wire_bytes_are_pinned() {
        let bytes = sample_play().to_bytes();
        assert_eq!(bytes.len(), PLAY_HUD_MAX_BYTES);
        let mut expected = vec![PLAY_HUD_VERSION];
        expected.extend_from_slice(&95u64.to_le_bytes());
        expected.push(0x01); // you: slot 1
        expected.push(0x05); // echo: attack + dodge
        expected.extend_from_slice(&12u32.to_le_bytes());
        expected.extend_from_slice(&30u32.to_le_bytes());
        expected.push(0x01); // crosshair visible
        expected.push(0x08); // spread 8 px
        expected.push(0x01); // hit recorded
        expected.extend_from_slice(&90u64.to_le_bytes());
        assert_eq!(bytes, expected, "play HUD wire drifts");
    }

    #[test]
    fn wire_round_trip_preserves_every_field() {
        let hud = sample_play();
        let decoded = PlayHud::from_bytes(&hud.to_bytes()).expect("sample decodes");
        assert_eq!(decoded, hud);
        assert_eq!(decoded.to_bytes(), hud.to_bytes(), "encoding is stable");
        assert_eq!(decoded.tick(), 95);
        assert_eq!(decoded.you(), Some(fighter(1)));
        assert_eq!(decoded.echo(), Buttons(ATTACK | DODGE));
        assert_eq!((decoded.ammo(), decoded.max_ammo()), (12, 30));
        assert_eq!(decoded.crosshair(), Crosshair::new(true, 8));
        assert_eq!(decoded.last_hit(), Some(90));
        assert!(decoded.hit_flash_visible());
    }

    #[test]
    fn hostile_bytes_fail_loudly() {
        assert_eq!(PlayHud::from_bytes(&[]), Err(PlayHudError::UnexpectedEnd));
        let bytes = sample_play().to_bytes();
        let mut bad_version = bytes.clone();
        bad_version[0] = PLAY_HUD_VERSION.wrapping_add(1);
        assert_eq!(
            PlayHud::from_bytes(&bad_version),
            Err(PlayHudError::BadVersion(2))
        );
        // Every short prefix truncates mid-field: version(1) + tick(8) +
        // you(1) + echo(1) + ammo(4) + max(4) + visible(1) + spread(1) +
        // hit flag(1) + hit tick(8).
        for end in [1usize, 9, 11, 19, 21, 29] {
            assert_eq!(
                PlayHud::from_bytes(&bytes[..end]),
                Err(PlayHudError::UnexpectedEnd),
                "prefix of {end} bytes truncates"
            );
        }
        let mut trailed = bytes.clone();
        trailed.push(0);
        assert_eq!(
            PlayHud::from_bytes(&trailed),
            Err(PlayHudError::TrailingBytes(1))
        );
        // Wire offsets: version 0, tick 1..9, you 9, echo 10, crosshair
        // visible 19, spread 20, hit flag 21.
        let mut bad_you = bytes.clone();
        bad_you[9] = 40;
        assert_eq!(
            PlayHud::from_bytes(&bad_you),
            Err(PlayHudError::UnknownBody(40))
        );
        let mut bad_echo = bytes.clone();
        bad_echo[10] = 0x08;
        assert_eq!(
            PlayHud::from_bytes(&bad_echo),
            Err(PlayHudError::UnknownButtons(0x08))
        );
        let mut bad_flag = bytes.clone();
        bad_flag[19] = 7;
        assert_eq!(
            PlayHud::from_bytes(&bad_flag),
            Err(PlayHudError::BadFlag(7))
        );
        let mut bad_hit_flag = bytes.clone();
        bad_hit_flag[21] = 7;
        assert_eq!(
            PlayHud::from_bytes(&bad_hit_flag),
            Err(PlayHudError::BadFlag(7))
        );
    }

    #[test]
    fn echo_bits_validate_and_render_as_text() {
        let mut hud = PlayHud::new(0);
        assert_eq!(hud.echo_line(), "echo -");
        hud.set_echo(Buttons(ATTACK | BLOCK | DODGE)).unwrap();
        assert_eq!(hud.echo_line(), "echo attack+block+dodge");
        hud.set_echo(Buttons(BLOCK)).unwrap();
        assert_eq!(hud.echo_line(), "echo block");
        let before = hud;
        assert_eq!(
            hud.set_echo(Buttons(0x08)),
            Err(PlayHudError::UnknownButtons(0x08))
        );
        assert_eq!(hud, before, "a refused echo changes nothing");
    }

    #[test]
    fn you_arrow_marks_exactly_one_fighter() {
        let mut hud = PlayHud::new(0);
        assert_eq!(hud.you_line(), "YOU -");
        assert!(!hud.is_you(fighter(0)));
        hud.set_you(Some(fighter(1))).unwrap();
        assert!(hud.is_you(fighter(1)));
        assert!(!hud.is_you(fighter(0)));
        assert_eq!(hud.you_line(), "YOU P1");
        hud.set_you(None).unwrap();
        assert!(!hud.is_you(fighter(1)));
        let before = hud;
        assert_eq!(
            hud.set_you(Some(FighterId(40))),
            Err(PlayHudError::UnknownBody(40))
        );
        assert_eq!(hud, before, "a refused arrow changes nothing");
    }

    #[test]
    fn ammo_fraction_spans_empty_to_full() {
        let mut hud = PlayHud::new(0);
        assert_eq!(hud.ammo_fraction_permille(), 0, "unknown max reads empty");
        assert_eq!(hud.ammo_line(), "AMMO 0/0");
        hud.set_ammo(12, 30);
        assert_eq!(hud.ammo_fraction_permille(), 400);
        assert_eq!(hud.ammo_line(), "AMMO 12/30");
        hud.set_ammo(30, 30);
        assert_eq!(hud.ammo_fraction_permille(), 1000);
        hud.set_ammo(99, 30);
        assert_eq!(
            (hud.ammo(), hud.max_ammo()),
            (30, 30),
            "over-max ammo clamps into the mag"
        );
        assert_eq!(hud.ammo_fraction_permille(), 1000);
        hud.set_ammo(0, 30);
        assert_eq!(hud.ammo_fraction_permille(), 0);
    }

    #[test]
    fn crosshair_spread_clamps_at_the_bound() {
        assert_eq!(Crosshair::new(true, 8).spread_px(), 8);
        assert_eq!(
            Crosshair::new(true, 255).spread_px(),
            MAX_SPREAD_PX,
            "over-bound spread clamps"
        );
        let mut hud = PlayHud::new(0);
        hud.set_crosshair(Crosshair::new(false, 200));
        assert!(!hud.crosshair().visible());
        assert_eq!(hud.crosshair().spread_px(), MAX_SPREAD_PX);
    }

    #[test]
    fn hit_flash_expires_with_the_tick() {
        let mut hud = PlayHud::new(95);
        assert!(!hud.hit_flash_visible(), "no hit recorded yet");
        hud.record_hit(95);
        assert!(hud.hit_flash_visible());
        hud.set_tick(95 + HIT_FLASH_TICKS);
        assert!(hud.hit_flash_visible(), "window is inclusive");
        hud.set_tick(95 + HIT_FLASH_TICKS + 1);
        assert!(!hud.hit_flash_visible());
        hud.clear_hit();
        hud.set_tick(95);
        assert!(!hud.hit_flash_visible(), "cleared hit shows nothing");
        assert_eq!(hud.last_hit(), None);
    }

    #[test]
    fn hints_name_the_bound_buttons_in_fixed_order() {
        let bindings = Bindings::new();
        let lines = sample_play().hint_lines(&bindings);
        assert_eq!(lines.len(), MAX_HINT_LINES);
        assert_eq!(
            lines.as_slice(),
            ["F attack", "SHIFT block", "SPACE dodge"].as_slice()
        );
        for line in &lines {
            assert!(!line.is_empty() && line.len() <= MAX_HINT_LEN);
            assert!(line.is_ascii());
        }
        assert_eq!(
            PlayHud::button_label(bindings.button(BoundAction::Attack)),
            "F"
        );
        assert_eq!(
            PlayHud::button_label(PhysicalButton::Mouse(1)),
            "MOUSE1".to_string()
        );
        assert_eq!(
            PlayHud::button_label(PhysicalButton::Pad(3)),
            "PAD3".to_string()
        );
    }

    #[test]
    fn overlay_bridge_carries_tick_and_layers() {
        let overlay = sample_overlay();
        let hud = overlay
            .play_hud(
                Some(fighter(1)),
                Buttons(ATTACK),
                12,
                30,
                Crosshair::new(true, 8),
                Some(90),
            )
            .expect("legal layers bridge");
        assert_eq!(hud.tick(), overlay.tick());
        assert_eq!(hud.you(), Some(fighter(1)));
        assert_eq!(hud.echo(), Buttons(ATTACK));
        assert_eq!((hud.ammo(), hud.max_ammo()), (12, 30));
        assert!(hud.hit_flash_visible());
        assert_eq!(
            overlay.play_hud(
                Some(fighter(1)),
                Buttons(0x08),
                0,
                0,
                Crosshair::new(false, 0),
                None,
            ),
            Err(PlayHudError::UnknownButtons(0x08))
        );
        assert_eq!(
            overlay.play_hud(
                Some(FighterId(40)),
                Buttons::NONE,
                0,
                0,
                Crosshair::new(false, 0),
                None,
            ),
            Err(PlayHudError::UnknownBody(40))
        );
    }

    #[test]
    fn every_palette_passes_the_play_frame_gate() {
        use crate::access::contrast_table;
        for row in contrast_table() {
            assert!(row.passes(), "{} fails the play-frame gate", row.name);
        }
        assert_eq!(contrast_table().len(), 4);
    }

    #[test]
    fn raster_shows_every_play_element() {
        let overlay = sample_overlay();
        let bindings = Bindings::new();
        let bg = overlay.palette().bg;
        let hud = sample_play();
        let mut framed = FrozenPlayFrame::new(128, 96, bg);
        framed.draw_play(&overlay, &hud, &bindings);
        let bare = FrozenPlayFrame::new(128, 96, bg);
        assert_ne!(
            framed.digest(),
            bare.digest(),
            "play HUD paints over a bare frame"
        );
        let lit = framed
            .pixels
            .iter()
            .filter(|pixel| **pixel == overlay.palette().fg)
            .count();
        assert!(lit > 0, "text lanes light foreground pixels");

        let mut changed = hud;
        changed.set_echo(Buttons(BLOCK)).expect("block echoes");
        let mut echo_frame = FrozenPlayFrame::new(128, 96, bg);
        echo_frame.draw_play(&overlay, &changed, &bindings);
        assert_ne!(echo_frame.digest(), framed.digest(), "echo repaints");

        let mut changed = hud;
        changed.set_ammo(29, 30);
        let mut ammo_frame = FrozenPlayFrame::new(128, 96, bg);
        ammo_frame.draw_play(&overlay, &changed, &bindings);
        assert_ne!(
            ammo_frame.digest(),
            framed.digest(),
            "ammo repaints the bar"
        );

        let mut changed = hud;
        changed.set_you(None).expect("arrow hides");
        let mut you_frame = FrozenPlayFrame::new(128, 96, bg);
        you_frame.draw_play(&overlay, &changed, &bindings);
        assert_ne!(
            you_frame.digest(),
            framed.digest(),
            "hiding the arrow repaints"
        );

        let mut changed = hud;
        changed.set_crosshair(Crosshair::new(false, 8));
        let mut cross_frame = FrozenPlayFrame::new(128, 96, bg);
        cross_frame.draw_play(&overlay, &changed, &bindings);
        assert_ne!(
            cross_frame.digest(),
            framed.digest(),
            "hiding the crosshair repaints"
        );

        let mut changed = hud;
        changed.clear_hit();
        let mut calm_frame = FrozenPlayFrame::new(128, 96, bg);
        calm_frame.draw_play(&overlay, &changed, &bindings);
        assert_ne!(
            calm_frame.digest(),
            framed.digest(),
            "clearing the hit drops the flash border"
        );

        let mut rebound = Bindings::new();
        rebound.rebind(BoundAction::Attack, PhysicalButton::Pad(3));
        let mut hint_frame = FrozenPlayFrame::new(128, 96, bg);
        hint_frame.draw_play(&overlay, &hud, &rebound);
        assert_ne!(
            hint_frame.digest(),
            framed.digest(),
            "a rebound attack repaints the hint lane"
        );

        let mut renamed = overlay.clone();
        renamed.rename(fighter(0), "Brook").expect("rename tracks");
        let mut name_frame = FrozenPlayFrame::new(128, 96, bg);
        name_frame.draw_play(&renamed, &hud, &bindings);
        assert_ne!(
            name_frame.digest(),
            framed.digest(),
            "renaming a fighter repaints the name lane"
        );

        let mut hurt = overlay.clone();
        hurt.set_hp(fighter(0), 150).expect("lower hp tracks");
        let mut hp_frame = FrozenPlayFrame::new(128, 96, bg);
        hp_frame.draw_play(&hurt, &hud, &bindings);
        assert_ne!(
            hp_frame.digest(),
            framed.digest(),
            "losing hp repaints the hp bar"
        );

        let mut ticked = hud;
        ticked.set_tick(96);
        let mut tick_frame = FrozenPlayFrame::new(128, 96, bg);
        tick_frame.draw_play(&overlay, &ticked, &bindings);
        assert_ne!(
            tick_frame.digest(),
            framed.digest(),
            "advancing the tick repaints the header"
        );

        let decoded = PlayHud::from_bytes(&hud.to_bytes()).expect("wire bytes decode");
        let mut via_bytes = FrozenPlayFrame::new(128, 96, bg);
        via_bytes.draw_play(&overlay, &decoded, &bindings);
        assert_eq!(
            via_bytes.digest(),
            framed.digest(),
            "the rasterizer draws decoded bytes like the live snapshot"
        );
    }

    #[test]
    fn error_display_names_each_case() {
        assert_eq!(
            PlayHudError::UnknownBody(40).to_string(),
            "unknown fighter 40"
        );
        assert_eq!(
            PlayHudError::UnknownButtons(8).to_string(),
            "unknown buttons 8"
        );
        assert_eq!(PlayHudError::BadFlag(7).to_string(), "bad flag 7");
        assert_eq!(
            PlayHudError::BadVersion(2).to_string(),
            "bad play HUD version 2"
        );
        assert_eq!(
            PlayHudError::UnexpectedEnd.to_string(),
            "truncated play HUD bytes"
        );
        assert_eq!(
            PlayHudError::TrailingBytes(1).to_string(),
            "trailing play HUD bytes (1)"
        );
    }
}
