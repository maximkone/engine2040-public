//! Bitmap font atlas for HUD text quads.
//!
//! Owner: render lane. Units: grid cells for indices, 0..1 for uvs,
//! atlas pixels for advance and baseline. Determinism: same code gives
//! the same uv box twice; fixed grid order, no clock. Errors: a code
//! outside 32..=127 gives None, never panics. Budget: no heap and no
//! GPU work here; the present path uploads the texture once and draws
//! staged quads in one call.
//! Caller: `crates/render/src/overlay.rs` stages glyph quads through
//! this atlas and the present path drains the ring after the blit.

/// Atlas grid columns (16 cells across).
pub const ATLAS_COLS: u8 = 16;
/// First covered code (space).
pub const GLYPH_FIRST: u8 = 32;
/// Last covered code (full grid, 96 codes).
pub const GLYPH_LAST: u8 = 127;
/// Covered code count (127 - 32 + 1).
pub const GLYPH_COUNT: usize = 96;
/// Fixed advance per cell in atlas pixels.
pub const ADVANCE_PX: f32 = 8.0;
/// Fixed baseline per cell in atlas pixels.
pub const BASELINE_PX: f32 = 13.0;
/// Grid step in u (one of 16 columns).
pub const UV_STEP_U: f32 = 1.0 / 16.0;
/// Grid step in v (one of 16 rows; 6 rows hold the 96 codes).
pub const UV_STEP_V: f32 = 1.0 / 16.0;

/// Uv box [u0, v0, u1, v1] for one atlas cell; None outside 32..=127.
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

/// Fixed advance per cell in atlas pixels. Pure.
#[must_use]
pub fn advance_px() -> f32 {
    ADVANCE_PX
}

/// Fixed baseline per cell in atlas pixels. Pure.
#[must_use]
pub fn baseline_px() -> f32 {
    BASELINE_PX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_atlas_printable_stable() {
        let uv = glyph_uv(65).expect("code 65 is printable");
        assert_eq!(uv, glyph_uv(65).expect("code 65 is printable"));
        assert_eq!(advance_px(), 8.0);
        assert_eq!(baseline_px(), 13.0);
        assert!(glyph_uv(32).is_some());
        assert!(glyph_uv(127).is_some());
        assert!(glyph_uv(31).is_none());
    }

    #[test]
    fn text_atlas_rejects_out_of_range() {
        assert!(glyph_uv(0).is_none());
        assert!(glyph_uv(31).is_none());
        assert!(glyph_uv(128).is_none());
        assert!(glyph_uv(200).is_none());
        assert!(glyph_uv(u8::MAX).is_none());
    }

    #[test]
    fn text_atlas_grid_steps_cover_full_grid() {
        assert_eq!(GLYPH_COUNT, 96);
        let first = glyph_uv(GLYPH_FIRST).expect("space is covered");
        assert_eq!(first[0], 0.0);
        assert_eq!(first[1], 0.0);
        assert_eq!(first[2] - first[0], UV_STEP_U);
        assert_eq!(first[3] - first[1], UV_STEP_V);
        let last = glyph_uv(GLYPH_LAST).expect("code 127 is covered");
        assert!(last[2] <= 1.0);
        assert!(last[3] <= 1.0);
        let mid = glyph_uv(65).expect("code 65 is printable");
        assert_eq!(mid[2] - mid[0], UV_STEP_U);
        assert_eq!(mid[3] - mid[1], UV_STEP_V);
    }

    #[test]
    fn text_atlas_deterministic_x2() {
        for code in GLYPH_FIRST..=GLYPH_LAST {
            assert_eq!(glyph_uv(code), glyph_uv(code));
        }
        assert_eq!(advance_px(), advance_px());
        assert_eq!(baseline_px(), baseline_px());
    }
}
