//! text: HUD text layout over a self-authored bitmap atlas plus quad emission.
//!
//! Consumer: the HUD renderer (the game draws the emitted quads; render's
//! `upload_mesh` is the shape reference, read-only; this crate does not
//! depend on render).
//! Data owner: `engine-ui`. Units: layout cells (one cell per glyph, origin
//! top-left, +x right, +y down), atlas UVs in `0..=1`, quad positions in
//! `scale` units. Determinism: pure functions of the inputs; ASCII-only,
//! insertion order kept, no floats in any hash path. Failure mode: bounded
//! inputs, clipped outputs, never panics.
//!
//! The [`Atlas`] below is the O-023 pixel font (`docs/archive/2026-10-08/assets-O-023/font-glyphs.json`,
//! CC0, authored in design-studio): a 5x7 monospace bitmap for printable
//! ASCII (`0x20..=0x7E`) in 6x8 cells on a 16 by 6 grid (atlas 96x48). The
//! API shape (`layout` cells, atlas UV rects, `emit_quads` arrays) is what
//! the game builds against.

use super::{Hud, KillCause, KillEntry, KillFeed};

/// Longest input honored, in chars; the rest clips. Bounds layout work.
pub const MAX_TEXT_CHARS: usize = 8192;
/// Widest line honored, in cells; larger `max_width_cells` clamps here.
pub const MAX_LINE_WIDTH: u32 = 128;
/// Most rows honored; extra lines clip. Bounds layout work.
pub const MAX_ROWS: u32 = 128;
/// Most glyph cells honored per call, for layout and quad emission.
pub const MAX_CELLS: usize = 4096;
/// Tab stop for `\t` expansion, in cells.
pub const TAB_WIDTH: u32 = 4;
/// Largest accepted quad scale; larger values clamp. Keeps positions finite.
pub const MAX_SCALE: f32 = 64.0;
/// HUD text size at 1080p, in pixels (Game Accessibility Guidelines minimum).
pub const HUD_TEXT_PX_1080: u16 = 28;
/// Smallest HUD text size honored on a short viewport, in pixels.
pub const HUD_TEXT_PX_MIN: u16 = 12;

/// HUD text size for a viewport height: 28 px at 1080p, scaled by height,
/// clamped to `HUD_TEXT_PX_MIN..=224`. Consumers are the HUD text commands;
/// the render pass takes this number as the font size. Pure; no floats.
#[must_use]
pub fn hud_text_px(viewport_height: u32) -> u16 {
    let height = viewport_height.min(16_384);
    let scaled = u32::from(HUD_TEXT_PX_1080)
        .saturating_mul(height)
        .checked_div(1080)
        .unwrap_or(0)
        .clamp(u32::from(HUD_TEXT_PX_MIN), 224);
    u16::try_from(scaled).unwrap_or(HUD_TEXT_PX_1080)
}

/// Editor text-entry IME wiring (Windows commit path).
///
/// Shape adapted from the DrYaling/Fyrox fork, commit `ea36f32` (MIT):
/// `fyrox-impl/src/engine/mod.rs` handles `WindowEvent::Ime::Commit(text)`
/// by appending the committed string and enables the path with
/// `Window::set_ime_allowed(true)`.
/// See <https://github.com/DrYaling/Fyrox/commit/ea36f32>.
/// Font bytes are OFL data and were NOT copied; only the event/flag shape
/// is ported, reimplemented here over the owned `TextEntry` buffer.
pub const MAX_ENTRY_CHARS: usize = MAX_TEXT_CHARS;

/// Window-agnostic IME event for editor text entry.
///
/// Mirrors the `winit` shape the window loop matches on: the committed
/// string is the only payload that mutates the buffer; pre-edit previews
/// never do. Pure data.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImeEvent {
    /// Final composed string from the OS IME (for example Windows
    /// `WindowEvent::Ime::Commit`). Inserted at the cursor.
    Commit(String),
    /// In-progress composition preview (for example Windows pre-edit).
    /// Observed only, never mutates the buffer.
    Preedit(String),
}

/// Single-line editor buffer with an explicit IME gate.
///
/// Consumer: editor text fields (the window loop forwards OS IME commits
/// here and toggles `TextEntry::set_ime_allowed` with focus). Data owner:
/// `engine-ui`. Units: `text` is UTF-8, `cursor` is the byte index of the
/// insertion point (always a char boundary). Determinism: pure functions of
/// the inputs; insertion order kept. Failure mode: bounded input, clipped
/// output, never panics.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TextEntry {
    text: String,
    cursor: usize,
    ime_allowed: bool,
}

impl TextEntry {
    /// Empty buffer with the cursor at the start and IME disabled (the
    /// window enables it on focus via `TextEntry::set_ime_allowed`). Pure.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current buffer contents (may hold non-ASCII from IME commits). Pure.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Byte index of the insertion point (always a char boundary). Pure.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// True when the window forwards OS IME commits to this buffer. Pure.
    #[must_use]
    pub fn ime_allowed(&self) -> bool {
        self.ime_allowed
    }

    /// Mirror of `Window::set_ime_allowed`: gate the commit path on focus.
    /// Pre-edit previews stay observable while disabled. Pure.
    pub fn set_ime_allowed(&mut self, allowed: bool) {
        self.ime_allowed = allowed;
    }

    /// Char count currently stored. Pure.
    #[must_use]
    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// True when no chars are stored. Pure.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Move the cursor to `byte` (clamped to the length, snapped back to a
    /// char boundary). Pure.
    pub fn set_cursor(&mut self, byte: usize) {
        self.cursor = self.text.floor_char_boundary(byte);
    }

    /// Insert `commit` at the cursor, dropping control chars and clipping to
    /// `MAX_ENTRY_CHARS` chars. Returns the inserted char count. Pure.
    /// Never panics.
    pub fn insert_text(&mut self, commit: &str) -> usize {
        if commit.is_empty() {
            return 0;
        }
        let room = MAX_ENTRY_CHARS.saturating_sub(self.len_chars());
        if room == 0 {
            return 0;
        }
        let mut kept = String::new();
        for glyph in commit.chars() {
            if kept.chars().count() >= room {
                break;
            }
            if glyph.is_control() {
                continue;
            }
            kept.push(glyph);
        }
        if kept.is_empty() {
            return 0;
        }
        let inserted = kept.chars().count();
        self.text.insert_str(self.cursor, &kept);
        self.cursor += kept.len();
        inserted
    }

    /// Apply one IME event: `Commit` inserts when `TextEntry::ime_allowed`
    /// is set (returns the inserted char count), `Preedit` only observes
    /// (returns `0`). Pure. Never panics.
    pub fn apply_ime(&mut self, event: &ImeEvent) -> usize {
        match event {
            ImeEvent::Commit(commit) => {
                if !self.ime_allowed {
                    return 0;
                }
                self.insert_text(commit)
            }
            ImeEvent::Preedit(_) => 0,
        }
    }
}

/// Horizontal alignment of each row inside the layout width.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Align {
    /// Rows start at the left edge.
    Left,
    /// Rows center in the leftover space (extra cell goes right).
    Center,
    /// Rows end at the right edge.
    Right,
}

/// One placed glyph: which cell of the grid it sits in. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GlyphCell {
    /// Printable ASCII glyph (`0x20..=0x7E`; anything else maps to `?`).
    pub codepoint: char,
    /// Column in cells from the left, after alignment.
    pub cell_x: u32,
    /// Row in cells from the top.
    pub cell_y: u32,
}

/// Monospace bitmap atlas for printable ASCII.
///
/// The O-023 pixel font (`docs/archive/2026-10-08/assets-O-023/font-glyphs.json`): 95 glyphs
/// (`0x20..=0x7E`), each 5 wide by 7 tall, packed row-major into a 16 by 6
/// cell grid (last grid cell unused) of 6x8 cells (atlas 96x48). Each bitmap
/// row keeps pixels in its low 5 bits, bit 4 on the left. UV rects name the
/// glyph's grid cell in `0..=1` with the origin at the atlas top-left; the
/// game's renderer flips `v` if its sampler needs it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Atlas;

impl Atlas {
    /// First covered codepoint (space).
    pub const FIRST: u32 = 0x20;
    /// Last covered codepoint (`~`).
    pub const LAST: u32 = 0x7E;
    /// Covered glyph count (`0x7E - 0x20 + 1`).
    pub const COUNT: usize = 95;
    /// Atlas grid columns.
    pub const COLS: u32 = 16;
    /// Atlas grid rows.
    pub const ROWS: u32 = 6;
    /// Glyph width in atlas pixels (O-023 `docs/archive/2026-10-08/assets-O-023/font-5x7.json`).
    pub const GLYPH_W: u32 = 5;
    /// Glyph height in atlas pixels (O-023 `docs/archive/2026-10-08/assets-O-023/font-5x7.json`).
    pub const GLYPH_H: u32 = 7;
    /// Atlas cell width in pixels: 5 glyph pixels plus 1 side bearing.
    pub const CELL_W: u32 = 6;
    /// Atlas cell height in pixels: 7 glyph pixels plus 1 descender row.
    pub const CELL_H: u32 = 8;
    /// Full font atlas width in pixels (`COLS * CELL_W`).
    pub const ATLAS_W: u32 = 96;
    /// Full font atlas height in pixels (`ROWS * CELL_H`).
    pub const ATLAS_H: u32 = 48;

    /// Empty atlas handle; the bitmaps live in static data. Pure.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// True for printable ASCII (`0x20..=0x7E`). Pure.
    #[must_use]
    pub fn contains(&self, codepoint: char) -> bool {
        matches!(u32::from(codepoint), 0x20..=0x7E)
    }

    /// Map any char into coverage: printable ASCII passes through, `\t`
    /// callers expand first, everything else becomes `?`. Pure.
    #[must_use]
    pub fn normalize(&self, codepoint: char) -> char {
        if self.contains(codepoint) {
            codepoint
        } else {
            '?'
        }
    }

    /// Bitmap rows for a glyph, low 5 bits per row, or `None` outside
    /// `0x20..=0x7E`. Pure.
    #[must_use]
    pub fn bitmap(&self, codepoint: char) -> Option<[u8; 7]> {
        let index = u32::from(codepoint).wrapping_sub(Self::FIRST) as usize;
        BITMAPS.get(index).copied()
    }

    /// True when the atlas pixel `(x, y)` of a glyph is lit; out-of-range
    /// pixels and glyphs read dark instead of failing. Pure.
    #[must_use]
    pub fn pixel(&self, codepoint: char, x: u32, y: u32) -> bool {
        if x >= Self::GLYPH_W || y >= Self::GLYPH_H {
            return false;
        }
        self.bitmap(codepoint).is_some_and(|rows| {
            let row = u32::from(rows[y as usize]);
            row & (1 << (4 - x)) != 0
        })
    }

    /// UV rect `[[u0, v0], [u1, v1]]` naming the glyph's grid cell, or `None`
    /// outside `0x20..=0x7E`. Pure.
    #[must_use]
    pub fn uv_rect(&self, codepoint: char) -> Option<[[f32; 2]; 2]> {
        let index = u32::from(codepoint).wrapping_sub(Self::FIRST);
        if index >= Self::COUNT as u32 {
            return None;
        }
        let col = index % Self::COLS;
        let row = index / Self::COLS;
        Some([
            [
                col as f32 / Self::COLS as f32,
                row as f32 / Self::ROWS as f32,
            ],
            [
                (col + 1) as f32 / Self::COLS as f32,
                (row + 1) as f32 / Self::ROWS as f32,
            ],
        ])
    }
}

/// O-023 5x7 rows from `docs/archive/2026-10-08/assets-O-023/font-glyphs.json` (`#` = lit), low 5
/// bits per row, bit 4 leftmost; index `i` covers `0x20 + i`.
const BITMAPS: [[u8; 7]; 95] = [
    [0, 0, 0, 0, 0, 0, 0],        // 0x20 space
    [4, 4, 4, 4, 4, 0, 4],        // 0x21 !
    [10, 10, 10, 0, 0, 0, 0],     // 0x22 "
    [10, 10, 31, 10, 31, 10, 10], // 0x23 #
    [4, 15, 20, 14, 5, 30, 4],    // 0x24 $
    [25, 25, 2, 4, 8, 19, 19],    // 0x25 %
    [12, 18, 20, 8, 21, 18, 13],  // 0x26 &
    [4, 4, 8, 0, 0, 0, 0],        // 0x27 '
    [2, 4, 8, 8, 8, 4, 2],        // 0x28 (
    [8, 4, 2, 2, 2, 4, 8],        // 0x29 )
    [0, 4, 21, 14, 21, 4, 0],     // 0x2A *
    [0, 4, 4, 31, 4, 4, 0],       // 0x2B +
    [0, 0, 0, 0, 12, 4, 8],       // 0x2C ,
    [0, 0, 0, 31, 0, 0, 0],       // 0x2D -
    [0, 0, 0, 0, 0, 12, 12],      // 0x2E .
    [0, 1, 2, 4, 8, 16, 0],       // 0x2F /
    [14, 17, 19, 21, 25, 17, 14], // 0x30 0
    [4, 12, 4, 4, 4, 4, 14],      // 0x31 1
    [14, 17, 1, 2, 4, 8, 31],     // 0x32 2
    [31, 2, 4, 2, 1, 17, 14],     // 0x33 3
    [2, 6, 10, 18, 31, 2, 2],     // 0x34 4
    [31, 16, 30, 1, 1, 17, 14],   // 0x35 5
    [6, 8, 16, 30, 17, 17, 14],   // 0x36 6
    [31, 1, 2, 4, 8, 8, 8],       // 0x37 7
    [14, 17, 17, 14, 17, 17, 14], // 0x38 8
    [14, 17, 17, 15, 1, 2, 12],   // 0x39 9
    [0, 12, 12, 0, 12, 12, 0],    // 0x3A :
    [0, 12, 12, 0, 12, 4, 8],     // 0x3B ;
    [2, 4, 8, 16, 8, 4, 2],       // 0x3C <
    [0, 0, 31, 0, 31, 0, 0],      // 0x3D =
    [8, 4, 2, 1, 2, 4, 8],        // 0x3E >
    [14, 17, 1, 2, 4, 0, 4],      // 0x3F ?
    [14, 17, 23, 21, 23, 16, 14], // 0x40 @
    [14, 17, 17, 31, 17, 17, 17], // 0x41 A
    [30, 17, 17, 30, 17, 17, 30], // 0x42 B
    [14, 17, 16, 16, 16, 17, 14], // 0x43 C
    [28, 18, 17, 17, 17, 18, 28], // 0x44 D
    [31, 16, 16, 30, 16, 16, 31], // 0x45 E
    [31, 16, 16, 30, 16, 16, 16], // 0x46 F
    [14, 17, 16, 23, 17, 17, 15], // 0x47 G
    [17, 17, 17, 31, 17, 17, 17], // 0x48 H
    [14, 4, 4, 4, 4, 4, 14],      // 0x49 I
    [7, 2, 2, 2, 2, 18, 12],      // 0x4A J
    [17, 18, 20, 24, 20, 18, 17], // 0x4B K
    [16, 16, 16, 16, 16, 16, 31], // 0x4C L
    [17, 27, 21, 21, 17, 17, 17], // 0x4D M
    [17, 17, 25, 21, 19, 17, 17], // 0x4E N
    [14, 17, 17, 17, 17, 17, 14], // 0x4F O
    [30, 17, 17, 30, 16, 16, 16], // 0x50 P
    [14, 17, 17, 17, 21, 18, 13], // 0x51 Q
    [30, 17, 17, 30, 20, 18, 17], // 0x52 R
    [15, 16, 16, 14, 1, 1, 30],   // 0x53 S
    [31, 4, 4, 4, 4, 4, 4],       // 0x54 T
    [17, 17, 17, 17, 17, 17, 14], // 0x55 U
    [17, 17, 17, 17, 17, 10, 4],  // 0x56 V
    [17, 17, 17, 21, 21, 27, 17], // 0x57 W
    [17, 17, 10, 4, 10, 17, 17],  // 0x58 X
    [17, 17, 10, 4, 4, 4, 4],     // 0x59 Y
    [31, 1, 2, 4, 8, 16, 31],     // 0x5A Z
    [14, 8, 8, 8, 8, 8, 14],      // 0x5B [
    [0, 16, 8, 4, 2, 1, 0],       // 0x5C backslash
    [14, 2, 2, 2, 2, 2, 14],      // 0x5D ]
    [4, 10, 17, 0, 0, 0, 0],      // 0x5E ^
    [0, 0, 0, 0, 0, 0, 31],       // 0x5F _
    [8, 4, 2, 0, 0, 0, 0],        // 0x60 `
    [0, 0, 14, 1, 15, 17, 15],    // 0x61 a
    [16, 16, 22, 25, 17, 17, 30], // 0x62 b
    [0, 0, 14, 16, 16, 17, 14],   // 0x63 c
    [1, 1, 13, 19, 17, 17, 15],   // 0x64 d
    [0, 0, 14, 17, 31, 16, 14],   // 0x65 e
    [6, 9, 8, 28, 8, 8, 8],       // 0x66 f
    [0, 15, 17, 17, 15, 1, 14],   // 0x67 g
    [16, 16, 22, 25, 17, 17, 17], // 0x68 h
    [4, 0, 12, 4, 4, 4, 14],      // 0x69 i
    [2, 0, 6, 2, 2, 18, 12],      // 0x6A j
    [16, 16, 18, 20, 24, 20, 18], // 0x6B k
    [12, 4, 4, 4, 4, 4, 14],      // 0x6C l
    [0, 0, 26, 21, 21, 17, 17],   // 0x6D m
    [0, 0, 22, 25, 17, 17, 17],   // 0x6E n
    [0, 0, 14, 17, 17, 17, 14],   // 0x6F o
    [0, 30, 17, 17, 30, 16, 16],  // 0x70 p
    [0, 15, 17, 17, 15, 1, 1],    // 0x71 q
    [0, 0, 22, 25, 16, 16, 16],   // 0x72 r
    [0, 0, 15, 16, 14, 1, 30],    // 0x73 s
    [8, 8, 28, 8, 8, 9, 6],       // 0x74 t
    [0, 0, 17, 17, 17, 19, 13],   // 0x75 u
    [0, 0, 17, 17, 17, 10, 4],    // 0x76 v
    [0, 0, 17, 17, 21, 21, 10],   // 0x77 w
    [0, 0, 17, 10, 4, 10, 17],    // 0x78 x
    [0, 17, 17, 17, 15, 1, 14],   // 0x79 y
    [0, 0, 31, 2, 4, 8, 31],      // 0x7A z
    [3, 4, 4, 8, 4, 4, 3],        // 0x7B {
    [4, 4, 4, 4, 4, 4, 4],        // 0x7C |
    [24, 4, 4, 2, 4, 4, 24],      // 0x7D }
    [0, 0, 8, 21, 2, 0, 0],       // 0x7E ~
];

/// Start a fresh row unless the row budget is spent (then clip). Pure.
fn newline(rows: &mut Vec<Vec<char>>, full: &mut bool) {
    if rows.len() as u32 >= MAX_ROWS {
        *full = true;
    } else {
        rows.push(Vec::new());
    }
}

/// Push one glyph, hard-wrapping at `width`; false when the rest clips. Pure.
fn push_glyph(glyph: char, width: u32, rows: &mut Vec<Vec<char>>, full: &mut bool) -> bool {
    if *full {
        return false;
    }
    let row_len = rows.last().map_or(u32::MAX, |row| row.len() as u32);
    if row_len >= width {
        if rows.len() as u32 >= MAX_ROWS {
            *full = true;
            return false;
        }
        rows.push(Vec::new());
    }
    match rows.last_mut() {
        Some(row) => {
            row.push(glyph);
            true
        }
        None => {
            *full = true;
            false
        }
    }
}

/// Lay `text` out in a `max_width_cells` grid with hard wrap, alignment and
/// overflow clipping. `\n` (and `\r`, with `\r\n` as one break) forces a new
/// row, `\t` jumps to the next multiple of [`TAB_WIDTH`], printable ASCII
/// lands one cell each, and anything else becomes `?`. Width `0` or empty
/// text yields no cells; widths above [`MAX_LINE_WIDTH`], inputs above
/// [`MAX_TEXT_CHARS`] chars, rows above [`MAX_ROWS`] and cells above
/// [`MAX_CELLS`] clip. Never panics. Pure.
#[must_use]
pub fn layout(text: &str, max_width_cells: u32, align: Align) -> Vec<GlyphCell> {
    if max_width_cells == 0 {
        return Vec::new();
    }
    let atlas = Atlas::new();
    let width = max_width_cells.min(MAX_LINE_WIDTH);
    let mut rows: Vec<Vec<char>> = vec![Vec::new()];
    let mut full = false;
    let mut skip_lf = false;
    for c in text.chars().take(MAX_TEXT_CHARS) {
        if full {
            break;
        }
        match c {
            '\r' => {
                newline(&mut rows, &mut full);
                skip_lf = true;
            }
            '\n' => {
                if skip_lf {
                    skip_lf = false;
                } else {
                    newline(&mut rows, &mut full);
                }
            }
            '\t' => {
                skip_lf = false;
                let col = rows.last().map_or(0, Vec::len) as u32;
                let mut spaces = (TAB_WIDTH - col % TAB_WIDTH) as usize;
                while spaces > 0 {
                    if !push_glyph(' ', width, &mut rows, &mut full) {
                        break;
                    }
                    spaces -= 1;
                }
            }
            other => {
                skip_lf = false;
                let glyph = atlas.normalize(other);
                if !push_glyph(glyph, width, &mut rows, &mut full) {
                    break;
                }
            }
        }
    }
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let len = row.len() as u32;
        // Rows never grow past `width`, so the subtractions cannot underflow.
        let offset = match align {
            Align::Left => 0,
            Align::Center => (width - len) / 2,
            Align::Right => width - len,
        };
        for (x, glyph) in row.iter().enumerate() {
            if out.len() >= MAX_CELLS {
                return out;
            }
            out.push(GlyphCell {
                codepoint: *glyph,
                cell_x: offset + x as u32,
                cell_y: y as u32,
            });
        }
    }
    out
}

/// Emit one screen quad per layout cell: 4 corner positions each (top-left,
/// top-right, bottom-right, bottom-left in `scale` units), 4 atlas UVs and 6
/// indices (`[base, base+1, base+2, base, base+2, base+3]` per quad), so the
/// renderer can upload the arrays as one triangle list. Non-finite or
/// non-positive `scale` falls back to `1.0`, larger scales clamp to
/// [`MAX_SCALE`], and cells past [`MAX_CELLS`] clip; glyphs missing from the
/// atlas (layout output is always covered) fall back to `?`. The three
/// arrays always describe the same quad count: `positions.len() / 4 ==
/// uvs.len() / 4 == indices.len() / 6 == cells.len()` after clipping. Never
/// panics. Pure.
#[must_use]
pub fn emit_quads(
    cells: &[GlyphCell],
    atlas: Atlas,
    scale: f32,
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>, Vec<u32>) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale.min(MAX_SCALE)
    } else {
        1.0
    };
    let count = cells.len().min(MAX_CELLS);
    let mut positions = Vec::with_capacity(count * 4);
    let mut uvs = Vec::with_capacity(count * 4);
    let mut indices = Vec::with_capacity(count * 6);
    for (i, cell) in cells.iter().take(count).enumerate() {
        let x0 = cell.cell_x as f32 * scale;
        let y0 = cell.cell_y as f32 * scale;
        positions.push([x0, y0]);
        positions.push([x0 + scale, y0]);
        positions.push([x0 + scale, y0 + scale]);
        positions.push([x0, y0 + scale]);
        let rect = atlas.uv_rect(cell.codepoint).or_else(|| atlas.uv_rect('?'));
        let ([u0, v0], [u1, v1]) =
            rect.map_or(([0.0, 0.0], [1.0, 1.0]), |corners| (corners[0], corners[1]));
        uvs.push([u0, v0]);
        uvs.push([u1, v0]);
        uvs.push([u1, v1]);
        uvs.push([u0, v1]);
        // `i < MAX_CELLS`, so `base + 3` stays far from `u32` overflow.
        let base = i as u32 * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (positions, uvs, indices)
}

/// HP line for a HUD snapshot: `HP <hp>/<max>`. ASCII-only. Pure.
#[must_use]
pub fn hp_text(hud: &Hud) -> String {
    format!("HP {}/{}", hud.hp, hud.max_hp)
}

/// Short ASCII cause label for the kill feed. Pure.
#[must_use]
pub fn kill_cause_label(cause: KillCause) -> &'static str {
    match cause {
        KillCause::Strike => "strike",
        KillCause::Projectile => "shot",
        KillCause::Fall => "fall",
        KillCause::Hazard => "hazard",
    }
}

/// One kill-feed line (`P<killer> beat P<victim> <cause>`, `*` for world
/// kills). ASCII-only. Pure.
#[must_use]
pub fn kill_entry_text(entry: &KillEntry) -> String {
    let cause = kill_cause_label(entry.cause);
    match entry.killer {
        Some(killer) => format!("P{} beat P{} {cause}", killer.get(), entry.victim.get()),
        None => format!("* beat P{} {cause}", entry.victim.get()),
    }
}

/// Oldest-first kill-feed lines, bounded by the feed itself. Pure.
#[must_use]
pub fn feed_lines(feed: &KillFeed) -> Vec<String> {
    feed.iter().map(kill_entry_text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::FighterId;

    fn fighter(slot: u8) -> FighterId {
        FighterId::new(slot).expect("test slot must be valid")
    }

    fn cells(coords: &[(char, u32, u32)]) -> Vec<GlyphCell> {
        coords
            .iter()
            .map(|(codepoint, cell_x, cell_y)| GlyphCell {
                codepoint: *codepoint,
                cell_x: *cell_x,
                cell_y: *cell_y,
            })
            .collect()
    }

    #[test]
    fn atlas_covers_all_printable_ascii() {
        assert_eq!(BITMAPS.len(), Atlas::COUNT);
        let atlas = Atlas::new();
        let mut rects = Vec::with_capacity(Atlas::COUNT);
        for code in Atlas::FIRST..=Atlas::LAST {
            let glyph = char::from_u32(code).expect("printable ASCII is valid");
            assert!(atlas.contains(glyph), "missing {glyph:?} U+{code:04X}");
            let bitmap = atlas.bitmap(glyph);
            assert!(bitmap.is_some(), "no bitmap for {glyph:?}");
            let rows = bitmap.expect("checked above");
            for row in rows {
                assert!(row < 32, "row bits escape 5 pixels for {glyph:?}");
            }
            let rect = atlas.uv_rect(glyph).expect("covered glyph needs UVs");
            let ([u0, v0], [u1, v1]) = (rect[0], rect[1]);
            assert!(u0 >= 0.0 && u0 < u1 && u1 <= 1.0, "bad U for {glyph:?}");
            assert!(v0 >= 0.0 && v0 < v1 && v1 <= 1.0, "bad V for {glyph:?}");
            assert!(!rects.contains(&rect), "shared UV cell for {glyph:?}");
            rects.push(rect);
        }
        assert_eq!(rects.len(), Atlas::COUNT);
        // Space is blank; every other glyph lights at least one pixel.
        assert_eq!(atlas.bitmap(' '), Some([0, 0, 0, 0, 0, 0, 0]));
        for code in (Atlas::FIRST + 1)..=Atlas::LAST {
            let glyph = char::from_u32(code).expect("printable ASCII is valid");
            let rows = atlas.bitmap(glyph).expect("covered above");
            assert!(rows.iter().any(|row| *row != 0), "{glyph:?} is blank");
        }
        // Outside coverage nothing resolves.
        for glyph in ['é', '😀', '\n', '\0', '\u{7F}'] {
            assert!(!atlas.contains(glyph));
            assert_eq!(atlas.bitmap(glyph), None);
            assert_eq!(atlas.uv_rect(glyph), None);
            assert_eq!(atlas.normalize(glyph), '?');
        }
    }

    #[test]
    fn layout_wraps_aligns_and_breaks_exactly() {
        // Hard wrap at the width.
        assert_eq!(
            layout("ABCD", 2, Align::Left),
            cells(&[('A', 0, 0), ('B', 1, 0), ('C', 0, 1), ('D', 1, 1)])
        );
        // Leftover space aligns per row.
        assert_eq!(
            layout("AB", 4, Align::Left),
            cells(&[('A', 0, 0), ('B', 1, 0)])
        );
        assert_eq!(
            layout("AB", 4, Align::Center),
            cells(&[('A', 1, 0), ('B', 2, 0)])
        );
        assert_eq!(
            layout("AB", 4, Align::Right),
            cells(&[('A', 2, 0), ('B', 3, 0)])
        );
        // Center keeps the extra cell on the right for odd leftovers.
        assert_eq!(
            layout("AB", 5, Align::Center),
            cells(&[('A', 1, 0), ('B', 2, 0)])
        );
        // Explicit breaks start new rows; wrap still applies after them.
        assert_eq!(
            layout("A\nB", 4, Align::Left),
            cells(&[('A', 0, 0), ('B', 0, 1)])
        );
        assert_eq!(
            layout("A\r\nB", 4, Align::Left),
            cells(&[('A', 0, 0), ('B', 0, 1)])
        );
        assert_eq!(
            layout("ABCDE", 3, Align::Left),
            cells(&[
                ('A', 0, 0),
                ('B', 1, 0),
                ('C', 2, 0),
                ('D', 0, 1),
                ('E', 1, 1)
            ])
        );
        // Alignment applies per wrapped row.
        assert_eq!(
            layout("ABCDE", 4, Align::Right),
            cells(&[
                ('A', 0, 0),
                ('B', 1, 0),
                ('C', 2, 0),
                ('D', 3, 0),
                ('E', 3, 1)
            ])
        );
        // Tabs jump to the next stop; unknown chars become `?`.
        assert_eq!(
            layout("A\tB", 8, Align::Left),
            cells(&[
                ('A', 0, 0),
                (' ', 1, 0),
                (' ', 2, 0),
                (' ', 3, 0),
                ('B', 4, 0)
            ])
        );
        assert_eq!(
            layout("Aé", 4, Align::Left),
            cells(&[('A', 0, 0), ('?', 1, 0)])
        );
        // Empty and zero-width inputs are empty, not errors.
        assert!(layout("", 8, Align::Left).is_empty());
        assert!(layout("ABC", 0, Align::Left).is_empty());
        assert!(layout("\n", 8, Align::Left).is_empty());
    }

    #[test]
    fn layout_clips_oversize_inputs_without_panic() {
        // Row budget clips: rows never reach MAX_ROWS, cells stay bounded.
        let huge = "A".repeat(100_000);
        let clipped = layout(&huge, 8, Align::Left);
        assert_eq!(clipped.len(), MAX_ROWS as usize * 8);
        assert!(clipped.iter().all(|cell| cell.cell_y < MAX_ROWS));
        // Cell budget clips when wide rows would pass it.
        let wide = layout(&huge, MAX_LINE_WIDTH, Align::Left);
        assert_eq!(wide.len(), MAX_CELLS);
        // Oversize widths clamp instead of overflowing alignment math.
        assert_eq!(
            layout("AB", u32::MAX, Align::Right),
            cells(&[('A', MAX_LINE_WIDTH - 2, 0), ('B', MAX_LINE_WIDTH - 1, 0)])
        );
        assert_eq!(
            layout("AB", u32::MAX, Align::Left),
            cells(&[('A', 0, 0), ('B', 1, 0)])
        );
        // Char budget clips: 1024 cells of width-1 text, one per row.
        let tall = "B".repeat(10_000);
        let clipped = layout(&tall, 1, Align::Left);
        assert_eq!(clipped.len(), MAX_ROWS as usize);
    }

    #[test]
    fn emit_quads_matches_layout_cells_one_to_one() {
        let atlas = Atlas::new();
        let laid = layout("AB", 4, Align::Left);
        let (positions, uvs, indices) = emit_quads(&laid, atlas, 2.0);
        assert_eq!(positions.len(), 8);
        assert_eq!(uvs.len(), 8);
        assert_eq!(indices.len(), 12);
        // First cell quad: 2x2 square at the origin, TL/TR/BR/BL order.
        assert_eq!(
            positions[0..4],
            [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]]
        );
        // Second cell sits one scaled cell to the right.
        assert_eq!(
            positions[4..8],
            [[2.0, 0.0], [4.0, 0.0], [4.0, 2.0], [2.0, 2.0]]
        );
        // UVs name each glyph's atlas cell in the same vertex order.
        let rect = atlas.uv_rect('A').expect("covered");
        assert_eq!(
            uvs[0..4],
            [
                rect[0],
                [rect[1][0], rect[0][1]],
                rect[1],
                [rect[0][0], rect[1][1]]
            ]
        );
        assert_eq!(indices, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
        for index in &indices {
            assert!((*index as usize) < positions.len());
        }
        // Bad scales sanitize to 1.0 instead of failing.
        let unit = emit_quads(&laid, atlas, 1.0);
        for bad in [0.0, -2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(emit_quads(&laid, atlas, bad), unit, "scale {bad}");
        }
        // Huge scales clamp; empty input stays empty.
        let (big, _, _) = emit_quads(&laid, atlas, 1.0e30);
        let (clamped, _, _) = emit_quads(&laid, atlas, MAX_SCALE);
        assert_eq!(big, clamped);
        let (positions, uvs, indices) = emit_quads(&[], atlas, 2.0);
        assert!(positions.is_empty() && uvs.is_empty() && indices.is_empty());
    }

    #[test]
    fn emit_quads_clips_oversize_slices_without_panic() {
        let atlas = Atlas::new();
        let many = vec![
            GlyphCell {
                codepoint: 'A',
                cell_x: 0,
                cell_y: 0,
            };
            100_000
        ];
        let (positions, uvs, indices) = emit_quads(&many, atlas, 1.0);
        assert_eq!(positions.len(), MAX_CELLS * 4);
        assert_eq!(uvs.len(), MAX_CELLS * 4);
        assert_eq!(indices.len(), MAX_CELLS * 6);
        assert!(indices.iter().all(|i| (*i as usize) < positions.len()));
    }

    #[test]
    fn hud_strings_flow_into_layout_and_quads() {
        let atlas = Atlas::new();
        let mut hud = Hud::new(3, 80, 100, 5, 0);
        assert_eq!(hp_text(&hud), "HP 80/100");
        let laid = layout(&hp_text(&hud), 32, Align::Left);
        assert_eq!(laid.len(), "HP 80/100".len());
        let (positions, uvs, indices) = emit_quads(&laid, atlas, 1.0);
        assert_eq!(positions.len(), laid.len() * 4);
        assert_eq!(uvs.len(), laid.len() * 4);
        assert_eq!(indices.len(), laid.len() * 6);

        hud.push_kill(KillEntry {
            tick: 3,
            killer: Some(fighter(0)),
            victim: fighter(1),
            cause: KillCause::Strike,
        })
        .expect("valid kill");
        hud.push_kill(KillEntry {
            tick: 4,
            killer: None,
            victim: fighter(1),
            cause: KillCause::Fall,
        })
        .expect("valid kill");
        let lines = feed_lines(&hud.feed);
        assert_eq!(lines, vec!["P0 beat P1 strike", "* beat P1 fall"]);
        for line in &lines {
            assert!(line.chars().all(|c| atlas.contains(c)));
            let laid = layout(line, 32, Align::Left);
            assert_eq!(laid.len(), line.len());
            let (positions, uvs, indices) = emit_quads(&laid, atlas, 1.0);
            assert_eq!(positions.len(), laid.len() * 4);
            assert_eq!(uvs.len(), laid.len() * 4);
            assert_eq!(indices.len(), laid.len() * 6);
        }
    }

    #[test]
    fn apply_ime_commit_gate_pins_windows_cjk_entry() {
        let mut entry = TextEntry::new();
        assert!(!entry.ime_allowed());
        // Disabled gate drops commits (Windows unfocused path).
        assert_eq!(entry.apply_ime(&ImeEvent::Commit("中文".to_owned())), 0);
        assert_eq!(entry.text(), "");
        assert_eq!(entry.cursor(), 0);
        // Preedit never mutates, even when enabled.
        entry.set_ime_allowed(true);
        assert!(entry.ime_allowed());
        assert_eq!(entry.apply_ime(&ImeEvent::Preedit("にほ".to_owned())), 0);
        assert_eq!(entry.text(), "");
        // Enabled commit inserts CJK at the cursor and reports chars.
        assert_eq!(entry.apply_ime(&ImeEvent::Commit("日本語".to_owned())), 3);
        assert_eq!(entry.text(), "日本語");
        assert_eq!(entry.cursor(), "日本語".len());
        // Disabled again drops further commits.
        entry.set_ime_allowed(false);
        assert_eq!(entry.apply_ime(&ImeEvent::Commit("한글".to_owned())), 0);
        assert_eq!(entry.text(), "日本語");
    }
}
