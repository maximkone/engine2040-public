//! Bitmap font atlas for HUD text quads.
//!
//! Owner: render lane. The glyph bitmaps are the CC0 O-023 5x7 pixel font
//! (`crates/ui/src/text.rs` holds the same data for HUD layout; the render
//! overlay needs it without a `engine-ui` edge). Units: grid cells for
//! indices, 0..1 for uvs, atlas pixels for advance and baseline. Determinism:
//! same code gives the same uv box and the same atlas bytes twice; fixed grid
//! order, no clock. Errors: a code outside 32..=126 gives None, never panics.
//! Budget: the atlas is 96x48 RGBA8 and built once into a fixed buffer; the
//! present path uploads it and draws staged quads in one call.
//! Caller: `crates/render/src/overlay.rs` stages glyph quads through this
//! atlas and the present path drains the ring after the blit.

/// Atlas grid columns (16 cells across).
pub const ATLAS_COLS: u8 = 16;
/// Atlas grid rows (6 rows hold the 95 printable codes).
pub const ATLAS_ROWS: u8 = 6;
/// First covered code (space).
pub const GLYPH_FIRST: u8 = 32;
/// Last covered code (tilde, the last printable ASCII).
pub const GLYPH_LAST: u8 = 126;
/// Covered code count (126 - 32 + 1).
pub const GLYPH_COUNT: usize = 95;
/// Glyph width in atlas pixels (O-023 5x7).
pub const GLYPH_W: u32 = 5;
/// Glyph height in atlas pixels.
pub const GLYPH_H: u32 = 7;
/// Atlas cell width in pixels: 5 glyph pixels plus 1 side bearing.
pub const CELL_W: u32 = 6;
/// Atlas cell height in pixels: 7 glyph pixels plus 1 descender row.
pub const CELL_H: u32 = 8;
/// Atlas image width in pixels (16 cells of 6).
pub const ATLAS_W: u32 = 96;
/// Atlas image height in pixels (6 cells of 8).
pub const ATLAS_H: u32 = 48;
/// Fixed advance per cell in atlas pixels.
pub const ADVANCE_PX: f32 = 6.0;
/// Fixed baseline per cell in atlas pixels.
pub const BASELINE_PX: f32 = 8.0;
/// Grid step in u (one of 16 columns).
pub const UV_STEP_U: f32 = 1.0 / 16.0;
/// Grid step in v (one of 6 rows).
pub const UV_STEP_V: f32 = 1.0 / 6.0;

/// Uv box [u0, v0, u1, v1] for one atlas cell; None outside 32..=126.
/// Pure; same inputs give the same box twice.
#[must_use]
pub fn glyph_uv(code: u8) -> Option<[f32; 4]> {
    if !(GLYPH_FIRST..=GLYPH_LAST).contains(&code) {
        return None;
    }
    let idx = code - GLYPH_FIRST;
    let col = idx % ATLAS_COLS;
    let row = idx / ATLAS_COLS;
    let u0 = f32::from(col) * UV_STEP_U;
    let v0 = f32::from(row) * UV_STEP_V;
    Some([u0, v0, u0 + UV_STEP_U, v0 + UV_STEP_V])
}

/// Uv box of the solid-white cell: the space cell (code 32) is filled opaque
/// and never stages a glyph quad, so HUD panels and bars sample it instead of
/// the old one-texel atlas and stay solid. Pure; same box twice.
#[must_use]
pub fn solid_uv() -> [f32; 4] {
    [0.0, 0.0, UV_STEP_U, UV_STEP_V]
}

/// Fixed advance per cell in atlas pixels. Pure.
#[must_use]
pub fn advance_px() -> f32 {
    ADVANCE_PX
}

/// Advance as a fraction of the text height: one cell is 6 wide and 8 tall,
/// so a square-pixel glyph advances 0.75 of its height. Callers staging text
/// multiply their height by this; the console and the ring share it. Pure.
#[must_use]
pub fn advance_ratio() -> f32 {
    ADVANCE_PX / BASELINE_PX
}

/// Fixed baseline per cell in atlas pixels. Pure.
#[must_use]
pub fn baseline_px() -> f32 {
    BASELINE_PX
}

/// The atlas as RGBA8 rows in pixel order: cell 0 (space) opaque white so
/// solid quads stay solid, every printable glyph white with alpha coverage,
/// every other texel transparent. Pure; same bytes twice. Built once and
/// shared, so a frame upload allocates nothing.
#[must_use]
pub fn atlas_rgba8() -> &'static [u8] {
    static ATLAS: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    ATLAS.get_or_init(build_atlas)
}

/// Fill the atlas once for [`atlas_rgba8`].
fn build_atlas() -> Vec<u8> {
    let len = usize::try_from(ATLAS_W * ATLAS_H * 4).unwrap_or(0);
    let mut px = vec![0u8; len];
    for y in 0..CELL_H {
        for x in 0..CELL_W {
            put(&mut px, x, y, 255);
        }
    }
    for (idx, rows) in BITMAPS.iter().enumerate().skip(1) {
        let idx = u32::try_from(idx).unwrap_or(0);
        let ox = (idx % u32::from(ATLAS_COLS)) * CELL_W;
        let oy = (idx / u32::from(ATLAS_COLS)) * CELL_H;
        for (gy, bits) in rows.iter().enumerate() {
            let py = oy + u32::try_from(gy).unwrap_or(0);
            for gx in 0..GLYPH_W {
                if bits & (1 << (4 - gx)) != 0 {
                    put(&mut px, ox + gx, py, 255);
                }
            }
        }
    }
    px
}

/// White RGB with the given alpha at `(x, y)`; bounds are the caller's grid.
fn put(px: &mut [u8], x: u32, y: u32, alpha: u8) {
    let at = usize::try_from(y * ATLAS_W + x).unwrap_or(0) * 4;
    px[at] = 255;
    px[at + 1] = 255;
    px[at + 2] = 255;
    px[at + 3] = alpha;
}

/// The CC0 O-023 5x7 font, printable ASCII 0x20..=0x7E, low 5 bits per row.
/// Same table as `engine_ui::text::Atlas::bitmap`; kept here because render
/// sits below ui and the overlay draws without a ui edge.
const BITMAPS: [[u8; 7]; GLYPH_COUNT] = [
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_atlas_printable_stable() {
        let uv = glyph_uv(65).expect("code 65 is printable");
        assert_eq!(uv, glyph_uv(65).expect("code 65 is printable"));
        assert_eq!(advance_px(), 6.0);
        assert_eq!(baseline_px(), 8.0);
        assert!((advance_ratio() - 0.75).abs() < 1e-6);
        assert!(glyph_uv(32).is_some());
        assert!(glyph_uv(126).is_some());
        assert!(glyph_uv(31).is_none());
        assert!(glyph_uv(127).is_none());
    }

    #[test]
    fn text_atlas_rejects_out_of_range() {
        assert!(glyph_uv(0).is_none());
        assert!(glyph_uv(31).is_none());
        assert!(glyph_uv(127).is_none());
        assert!(glyph_uv(200).is_none());
        assert!(glyph_uv(u8::MAX).is_none());
    }

    #[test]
    fn text_atlas_grid_steps_cover_full_grid() {
        assert_eq!(GLYPH_COUNT, 95);
        assert_eq!(ATLAS_W, u32::from(ATLAS_COLS) * CELL_W);
        assert_eq!(ATLAS_H, u32::from(ATLAS_ROWS) * CELL_H);
        let first = glyph_uv(GLYPH_FIRST).expect("space is covered");
        assert_eq!(first[0], 0.0);
        assert_eq!(first[1], 0.0);
        assert_eq!(first[2] - first[0], UV_STEP_U);
        assert_eq!(first[3] - first[1], UV_STEP_V);
        let last = glyph_uv(GLYPH_LAST).expect("tilde is covered");
        assert!(last[2] <= 1.0);
        assert!(last[3] <= 1.0);
        let mid = glyph_uv(65).expect("code 65 is printable");
        assert_eq!(mid[2] - mid[0], UV_STEP_U);
        // Row 2 sits on a rounded float, so the v step carries a 1-ulp error.
        assert!((mid[3] - mid[1] - UV_STEP_V).abs() < 1e-6);
    }

    #[test]
    fn text_atlas_deterministic_x2() {
        for code in GLYPH_FIRST..=GLYPH_LAST {
            assert_eq!(glyph_uv(code), glyph_uv(code));
        }
        assert_eq!(advance_px(), advance_px());
        assert_eq!(baseline_px(), baseline_px());
        assert_eq!(atlas_rgba8(), atlas_rgba8());
    }

    /// The uploaded atlas carries real glyphs: the space cell is opaque white
    /// (panels and bars sample it), a letter has lit texels, and the padding
    /// column is clear so nearest sampling cannot smear one glyph into the
    /// next.
    #[test]
    fn text_atlas_has_white_solid_cell_and_lit_glyphs() {
        let px = atlas_rgba8();
        assert_eq!(px.len(), (ATLAS_W * ATLAS_H * 4) as usize);
        for y in 0..CELL_H {
            for x in 0..CELL_W {
                let at = ((y * ATLAS_W + x) * 4) as usize;
                assert_eq!(&px[at..at + 4], &[255, 255, 255, 255]);
            }
        }
        let a = glyph_uv(b'A').expect("A is printable");
        let ax = (a[0] * ATLAS_W as f32) as u32;
        let ay = (a[1] * ATLAS_H as f32) as u32;
        let mut lit = 0;
        for y in 0..GLYPH_H {
            for x in 0..GLYPH_W {
                let at = (((ay + y) * ATLAS_W + ax + x) * 4) as usize;
                lit += u32::from(px[at + 3] != 0);
            }
        }
        assert!(lit > 5, "A is drawn, lit texels {lit}");
        // The bearing column (5) and descender row (7) of every glyph cell
        // stay clear.
        for y in 0..GLYPH_H {
            let at = (((ay + y) * ATLAS_W + ax + GLYPH_W) * 4) as usize;
            assert_eq!(px[at + 3], 0, "bearing column clear at row {y}");
        }
        for x in 0..CELL_W {
            let at = (((ay + GLYPH_H) * ATLAS_W + ax + x) * 4) as usize;
            assert_eq!(px[at + 3], 0, "descender row clear at column {x}");
        }
        assert_eq!(solid_uv(), glyph_uv(GLYPH_FIRST).expect("space"));
    }
}
