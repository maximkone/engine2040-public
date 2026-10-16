//! Overlay quad ring (EG14 slice 1): CPU staging of HUD quads.
//!
//! Owner: render lane. Units: 0..1 quad space, texels for atlas cells.
//! Determinism: same inputs give same bytes twice; fixed order, no clock.
//! Errors: full ring refuses with false, out-of-range glyph gives None.
//! Budget: fixed 128 quads, no heap work per frame; aspect stays in MVP.

/// Bitmap font atlas plus HUD metrics; the ring stages glyph quads
/// through it and the present path drains the ring after the blit.
#[path = "text.rs"]
pub mod text;

/// Maximum staged HUD quads; the 129th push is refused.
pub const OVERLAY_MAX_QUADS: usize = 128;

/// Vertices per staged quad in the triangle-list drain.
pub const VERTS_PER_QUAD: usize = 4;

/// One GPU vertex for the next slice: 0..1 position, 0..1 uv, sRGB bytes.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct QuadVertex {
    /// Position in 0..1 quad space.
    pub pos2: [f32; 2],
    /// Texture uv in 0..1 space.
    pub uv2: [f32; 2],
    /// sRGB color bytes.
    pub col4: [u8; 4],
}

/// One staged HUD quad: 0..1 rect, 0..1 uv box, sRGB bytes.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Quad {
    /// Rect as [x0, y0, x1, y1] in 0..1 space.
    pub rect: [f32; 4],
    /// Uv box as [u0, v0, u1, v1] in 0..1 space.
    pub uv: [f32; 4],
    /// sRGB color bytes.
    pub col: [u8; 4],
}

/// Fixed ring of [`Quad`] drained in push order by the next slice.
#[derive(Clone, Debug)]
pub struct OverlayRing {
    quads: [Quad; OVERLAY_MAX_QUADS],
    len: usize,
}

impl Default for OverlayRing {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayRing {
    /// Empty ring holding no quads.
    pub fn new() -> Self {
        Self {
            quads: [Quad::default(); OVERLAY_MAX_QUADS],
            len: 0,
        }
    }

    /// Stage one quad clamped to 0..1; false and no store when full.
    pub fn push_quad(&mut self, quad: Quad) -> bool {
        if self.len >= OVERLAY_MAX_QUADS {
            return false;
        }
        let q = Quad {
            rect: [
                clamp01(quad.rect[0]),
                clamp01(quad.rect[1]),
                clamp01(quad.rect[2]),
                clamp01(quad.rect[3]),
            ],
            uv: [
                clamp01(quad.uv[0]),
                clamp01(quad.uv[1]),
                clamp01(quad.uv[2]),
                clamp01(quad.uv[3]),
            ],
            col: quad.col,
        };
        self.quads[self.len] = q;
        self.len += 1;
        true
    }

    /// Drop all staged quads; the backing stays for the next frame.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Number of staged quads.
    pub fn quad_count(&self) -> usize {
        self.len
    }

    /// Stage one hp bar as two quads: dark back plus fill from fraction.
    /// Fraction is clamped to 0..1; false and no store when fewer than
    /// two slots are free.
    pub fn push_hp_bar(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, fraction: f32) -> bool {
        if self.len + 2 > OVERLAY_MAX_QUADS {
            return false;
        }
        let x0c = clamp01(x0);
        let y0c = clamp01(y0);
        let x1c = clamp01(x1);
        let y1c = clamp01(y1);
        let (lx0, lx1) = order_pair(x0c, x1c);
        let (ly0, ly1) = order_pair(y0c, y1c);
        let back = Quad {
            rect: [lx0, ly0, lx1, ly1],
            uv: [0.0, 0.0, 1.0, 1.0],
            col: [12, 12, 14, 255],
        };
        let fill = Quad {
            rect: [lx0, ly0, hp_fill_x1(lx0, lx1, fraction), ly1],
            uv: [0.0, 0.0, 1.0, 1.0],
            col: [196, 48, 44, 255],
        };
        let ok0 = self.push_quad(back);
        let ok1 = self.push_quad(fill);
        ok0 && ok1
    }

    /// Stage one text line as glyph quads from the atlas; returns staged count.
    /// Origin is the top-left in 0..1 space; each cell is `height` tall and
    /// `height * advance / 16` wide. Space advances with no quad; codes
    /// outside 32..=127 are skipped with no advance. Stops when the ring is
    /// full; the count tells the caller what reached the ring.
    pub fn push_text(&mut self, x: f32, y: f32, height: f32, col: [u8; 4], line: &str) -> usize {
        if !height.is_finite() || height <= 0.0 {
            return 0;
        }
        let w = height * text::advance_px() / 16.0;
        let mut staged = 0usize;
        let mut cx = x;
        for byte in line.bytes() {
            if byte == b' ' {
                cx += w;
                continue;
            }
            let uv = match text::glyph_uv(byte) {
                Some(u) => u,
                None => continue,
            };
            let ok = self.push_quad(Quad {
                rect: [cx, y, cx + w, y + height],
                uv,
                col,
            });
            if !ok {
                break;
            }
            staged += 1;
            cx += w;
        }
        staged
    }

    /// Expand staged quads into `out` as a triangle-list vertex drain: four
    /// verts per quad (top-left, top-right, bottom-left, bottom-right), uv
    /// matched per corner, color copied. Returns verts written; stops at a
    /// quad boundary when `out` is short. Pure; same ring, same bytes.
    pub fn write_vertices(&self, out: &mut [QuadVertex]) -> usize {
        let mut n = 0usize;
        for q in self.quads() {
            if out.len() - n < VERTS_PER_QUAD {
                break;
            }
            let [x0, y0, x1, y1] = q.rect;
            let [u0, v0, u1, v1] = q.uv;
            out[n] = QuadVertex {
                pos2: [x0, y0],
                uv2: [u0, v0],
                col4: q.col,
            };
            out[n + 1] = QuadVertex {
                pos2: [x1, y0],
                uv2: [u1, v0],
                col4: q.col,
            };
            out[n + 2] = QuadVertex {
                pos2: [x0, y1],
                uv2: [u0, v1],
                col4: q.col,
            };
            out[n + 3] = QuadVertex {
                pos2: [x1, y1],
                uv2: [u1, v1],
                col4: q.col,
            };
            n += VERTS_PER_QUAD;
        }
        n
    }

    /// Staged quads in push order; empty after [`OverlayRing::clear`].
    pub fn quads(&self) -> &[Quad] {
        self.quads.split_at(self.len).0
    }
}

/// Fixed 8x16 cell atlas for printable codes; no per-glyph upload.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct FontAtlas;

impl FontAtlas {
    /// Empty atlas; the cell table is pure math, no upload.
    pub fn new() -> Self {
        Self
    }

    /// Uv box [u0, v0, u1, v1] for one atlas cell; None outside 32..=127.
    /// Single owner is `text`; this wrapper keeps staged callers compiling.
    pub fn glyph_uv(&self, code: u8) -> Option<[f32; 4]> {
        text::glyph_uv(code)
    }

    /// Fixed advance for every cell in the same units as the uv grid.
    pub fn advance(&self) -> f32 {
        text::advance_px()
    }

    /// Fixed baseline for every cell in the same units as the uv grid.
    pub fn baseline(&self) -> f32 {
        text::baseline_px()
    }
}

/// Ortho from 0..1 quad space to NDC with x divided by aspect.
/// Column-major (outer index is column) for the shader upload:
/// ndc x = m[0][0] * x + m[3][0], ndc y = m[1][1] * y + m[3][1].
/// Bad aspect (zero, negative, non-finite) reads as 1.0.
pub fn overlay_mvp(aspect: f32) -> [[f32; 4]; 4] {
    let a = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    };
    let sx = 2.0 / a;
    [
        [sx, 0.0, 0.0, 0.0],
        [0.0, 2.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-1.0, -1.0, 0.0, 1.0],
    ]
}

/// After-blit single-draw params for the staged ring: one draw for the
/// whole ring, zero draws when empty. Units: quad/vert counts plus the
/// 0..1-to-NDC matrix. Pure; same ring and aspect repeat the same values.
/// No heap and no GPU work; the present path encodes the one draw next.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OverlayDraw {
    /// Staged quads covered by the single draw.
    pub quads: usize,
    /// Triangle-list verts (`quads * 4`) for the single draw.
    pub verts: usize,
    /// Ortho from 0..1 quad space to NDC with x divided by aspect.
    pub mvp: [[f32; 4]; 4],
}

impl OverlayRing {
    /// Single-draw params for the after-blit pass; `None` when empty so
    /// the present path skips the draw and saves one call. Bad aspect
    /// falls back through the shared ortho helper. Pure; empty stays
    /// empty and full stays at the fixed cap.
    /// Caller: `crates/render/src/present/gpu_present/live.rs`.
    #[must_use]
    pub fn draw_plan(&self, aspect: f32) -> Option<OverlayDraw> {
        if self.len == 0 {
            return None;
        }
        Some(OverlayDraw {
            quads: self.len,
            verts: self.len * VERTS_PER_QUAD,
            mvp: overlay_mvp(aspect),
        })
    }

    /// Single-draw params for staged HP bars; `None` when empty so the
    /// live path skips the draw and saves one call. Same values as
    /// [`OverlayRing::draw_plan`]; the staged ring holds HP bars this
    /// slice, so the live call has one named entry point. Pure.
    /// Caller: `crates/render/src/present/gpu_present/live.rs`.
    #[must_use]
    pub fn hp_bars_plan(&self, aspect: f32) -> Option<OverlayDraw> {
        self.draw_plan(aspect)
    }
}

/// Textured overlay shader for the after-blit draw: pass-through
/// vertex from `pos2`, `uv2` and `col4` through the `overlay_mvp` uniform,
/// atlas-modulated fragment with alpha blend, no depth, 0..1 coords.
/// Solid bars use a white atlas cell so color stays exact; text samples
/// its glyph cell. Caller: `crates/render/src/present/gpu_present/submit.rs`.
pub const OVERLAY_WGSL: &str = r#"
struct VsIn {
    @location(0) pos2: vec2<f32>,
    @location(1) uv2: vec2<f32>,
    @location(2) col4: vec4<f32>,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};
@group(0) @binding(0) var<uniform> overlay_mvp: mat4x4<f32>;
@group(0) @binding(1) var overlay_atlas: texture_2d<f32>;
@group(0) @binding(2) var overlay_sampler: sampler;
@vertex
fn vs(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = overlay_mvp * vec4<f32>(in.pos2, 0.0, 1.0);
    out.color = in.col4;
    out.uv = in.uv2;
    return out;
}
@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let tex = textureSample(overlay_atlas, overlay_sampler, in.uv);
    return in.color * tex;
}
"#;

/// Byte layout of [`QuadVertex`] for the after-blit draw.
/// Caller: `crates/render/src/present/gpu_present/submit.rs`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OverlayVertexLayout {
    /// Byte stride of one vertex.
    pub stride: usize,
    /// Byte offset of `pos2` in [`QuadVertex`].
    pub pos_offset: usize,
    /// Byte offset of `col4` in [`QuadVertex`].
    pub col_offset: usize,
}

/// Pure layout for the after-blit draw; same vertex, same bytes.
/// Caller: `crates/render/src/present/gpu_present/submit.rs`.
#[must_use]
pub fn overlay_vertex_layout() -> OverlayVertexLayout {
    OverlayVertexLayout {
        stride: core::mem::size_of::<QuadVertex>(),
        pos_offset: core::mem::offset_of!(QuadVertex, pos2),
        col_offset: core::mem::offset_of!(QuadVertex, col4),
    }
}

/// Pipeline shape for the after-blit draw: entries plus blend and depth.
/// Caller: `crates/render/src/present/gpu_present/live.rs`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OverlayPipelineDesc {
    /// Vertex entry in [`OVERLAY_WGSL`].
    pub vs_entry: &'static str,
    /// Fragment entry in [`OVERLAY_WGSL`].
    pub fs_entry: &'static str,
    /// True when the draw blends over the blit.
    pub alpha_blend: bool,
    /// True when the draw reads depth.
    pub has_depth: bool,
}

/// Pure shape for the after-blit draw; solid color blends, no depth.
/// Caller: `crates/render/src/present/gpu_present/live.rs`.
#[must_use]
pub fn overlay_pipeline_desc() -> OverlayPipelineDesc {
    OverlayPipelineDesc {
        vs_entry: "vs",
        fs_entry: "fs",
        alpha_blend: true,
        has_depth: false,
    }
}

/// 0..1 rect for one HP bar slot: bars stack top-left, slot 0 above slot 1,
/// narrow on x so a 16:9 frame never stretches a bar square. Pure; slots past
/// 15 clamp to the last row, so all 16 rows fit under y 1.0 (16 bars of 0.04
/// plus 15 gaps of 0.01 from 0.05 end at 0.84). Caller: the game presentation
/// leg stages one rect per duel overlay row.
#[must_use]
pub fn hp_bar_rect(slot: usize) -> [f32; 4] {
    const X0: f32 = 0.04;
    const X1: f32 = 0.34;
    const Y0: f32 = 0.05;
    const H: f32 = 0.04;
    const GAP: f32 = 0.01;
    let capped = u8::try_from(slot.min(15)).unwrap_or(u8::MAX);
    let y0 = Y0 + f32::from(capped) * (H + GAP);
    [X0, y0, X1, y0 + H]
}

/// HP permille 0..=1000 as a 0..1 fill fraction; above range clamps. Pure.
/// Caller: the game presentation leg fills one bar per duel overlay row.
#[must_use]
pub fn hp_fraction(permille: u16) -> f32 {
    f32::from(permille.min(1000)) / 1000.0
}

/// Right edge of the HP fill quad for an ordered 0..1 bar.
///
/// Pure; same inputs give the same edge twice. Fraction is clamped to 0..1
/// through the shared helper, so non-finite reads as empty at the left edge,
/// empty stays at the left edge and full reaches the right edge. No
/// allocation, no GPU work.
///
/// Caller: `OverlayRing::push_hp_bar` (the present path stages one
/// back-plus-fill pair per duel row through it).
#[must_use]
pub fn hp_fill_x1(lx0: f32, lx1: f32, fraction: f32) -> f32 {
    let t = clamp01(fraction);
    lx0 + (lx1 - lx0) * t
}

fn clamp01(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn order_pair(a: f32, b: f32) -> (f32, f32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_present_frame_hp_rect_moves() {
        let mut low = OverlayRing::new();
        assert!(low.push_hp_bar(0.2, 0.1, 0.6, 0.18, 0.25));
        assert_eq!(low.quad_count(), 2);
        let mut high = OverlayRing::new();
        assert!(high.push_hp_bar(0.2, 0.1, 0.6, 0.18, 0.75));
        assert_eq!(high.quad_count(), 2);
        let low_q = low.quads();
        let high_q = high.quads();
        assert_eq!(low_q[0].rect, [0.2, 0.1, 0.6, 0.18]);
        assert_eq!(low_q[1].rect[1], 0.1);
        assert!(low_q[1].rect[2] < high_q[1].rect[2]);
        assert_eq!(low_q[1].rect[2], 0.2 + 0.4 * 0.25);
        assert_eq!(high_q[1].rect[2], 0.2 + 0.4 * 0.75);
    }

    #[test]
    fn overlay_present_frame_ring_full_refuses() {
        let mut ring = OverlayRing::new();
        let q = Quad {
            rect: [0.1, 0.1, 0.2, 0.2],
            uv: [0.0, 0.0, 1.0, 1.0],
            col: [255, 255, 255, 255],
        };
        for _ in 0..OVERLAY_MAX_QUADS {
            assert!(ring.push_quad(q));
        }
        assert_eq!(ring.quad_count(), OVERLAY_MAX_QUADS);
        assert!(!ring.push_quad(q));
        assert!(!ring.push_hp_bar(0.1, 0.1, 0.5, 0.2, 0.5));
        assert_eq!(ring.quad_count(), OVERLAY_MAX_QUADS);
    }

    #[test]
    fn overlay_present_frame_atlas_stable() {
        let atlas = FontAtlas::new();
        let uv0 = atlas.glyph_uv(65).expect("code 65 is printable");
        let uv1 = atlas.glyph_uv(65).expect("code 65 is printable");
        assert_eq!(uv0, uv1);
        assert_eq!(atlas.advance(), 8.0);
        assert_eq!(atlas.baseline(), 13.0);
        assert!(atlas.glyph_uv(31).is_none());
        let m0 = overlay_mvp(16.0 / 9.0);
        let m1 = overlay_mvp(16.0 / 9.0);
        assert_eq!(m0, m1);
        let mut a = OverlayRing::new();
        let mut b = OverlayRing::new();
        assert!(a.push_hp_bar(0.1, 0.1, 0.5, 0.2, 0.5));
        assert!(b.push_hp_bar(0.1, 0.1, 0.5, 0.2, 0.5));
        assert_eq!(a.quads(), b.quads());
    }

    #[test]
    fn overlay_present_frame_hp_bar_rect_stays_in_unit_space() {
        let top = hp_bar_rect(0);
        let next = hp_bar_rect(1);
        assert_eq!(top[0], 0.04);
        assert_eq!(top[1], 0.05);
        assert_eq!(top[2], 0.34);
        assert!((top[3] - 0.09).abs() < 1e-6);
        assert!(next[1] > top[1]);
        assert_eq!(next[0], top[0]);
        assert_eq!(next[2], top[2]);
        for slot in 0..16usize {
            let rect = hp_bar_rect(slot);
            assert!(rect[0] >= 0.0 && rect[2] <= 1.0);
            assert!(rect[1] >= 0.0 && rect[3] <= 1.0);
            assert!(rect[0] < rect[2] && rect[1] < rect[3]);
        }
        assert_eq!(hp_bar_rect(99), hp_bar_rect(15));
    }

    #[test]
    fn overlay_present_frame_hp_fraction_clamps_permille() {
        assert_eq!(hp_fraction(0), 0.0);
        assert_eq!(hp_fraction(500), 0.5);
        assert_eq!(hp_fraction(1000), 1.0);
        assert_eq!(hp_fraction(1200), 1.0);
        assert_eq!(hp_fraction(u16::MAX), 1.0);
    }

    #[test]
    fn overlay_hp_fill_edge_tracks_fraction_and_failures() {
        assert_eq!(hp_fill_x1(0.2, 0.6, 0.0), 0.2);
        assert_eq!(hp_fill_x1(0.2, 0.6, 0.25), 0.2 + 0.4 * 0.25);
        assert_eq!(hp_fill_x1(0.2, 0.6, 1.0), 0.6);
        assert_eq!(hp_fill_x1(0.2, 0.6, 1.5), 0.6);
        assert_eq!(hp_fill_x1(0.2, 0.6, -0.5), 0.2);
        assert_eq!(hp_fill_x1(0.2, 0.6, f32::NAN), 0.2);
        assert_eq!(hp_fill_x1(0.2, 0.6, f32::INFINITY), 0.2);
        assert_eq!(hp_fill_x1(0.2, 0.6, f32::NEG_INFINITY), 0.2);
        assert_eq!(hp_fill_x1(0.2, 0.6, 0.5), hp_fill_x1(0.2, 0.6, 0.5));
    }

    #[test]
    fn overlay_font_atlas_matches_text_owner() {
        let atlas = FontAtlas::new();
        assert_eq!(atlas.advance(), text::advance_px());
        assert_eq!(atlas.baseline(), text::baseline_px());
        for code in [32u8, 48, 65, 90, 97, 122, 127] {
            assert_eq!(atlas.glyph_uv(code), text::glyph_uv(code));
        }
        assert_eq!(atlas.glyph_uv(31), text::glyph_uv(31));
        assert_eq!(atlas.glyph_uv(128), text::glyph_uv(128));
    }

    #[test]
    fn overlay_draw_plan_skips_when_empty() {
        let ring = OverlayRing::new();
        assert!(ring.draw_plan(16.0 / 9.0).is_none());
    }

    #[test]
    fn overlay_draw_plan_single_draw_for_staged_bars() {
        let mut ring = OverlayRing::new();
        assert!(ring.push_hp_bar(0.04, 0.05, 0.34, 0.09, 0.5));
        let plan = ring.draw_plan(16.0 / 9.0).expect("one staged bar draws");
        assert_eq!(plan.quads, 2);
        assert_eq!(plan.verts, 8);
        assert_eq!(plan.mvp, overlay_mvp(16.0 / 9.0));
        let again = ring.draw_plan(16.0 / 9.0).expect("same inputs repeat");
        assert_eq!(plan, again);
    }

    #[test]
    fn overlay_drain_matches_single_draw_plan() {
        let mut ring = OverlayRing::new();
        assert!(ring.push_hp_bar(0.04, 0.05, 0.34, 0.09, 0.5));
        assert_eq!(ring.quad_count(), 2);
        let plan = ring.draw_plan(16.0 / 9.0).expect("staged bar draws once");
        assert_eq!(plan.quads, 2);
        assert_eq!(plan.verts, 8);
        let mut verts = vec![QuadVertex::default(); plan.verts];
        let written = ring.write_vertices(&mut verts);
        assert_eq!(written, plan.verts);
        assert_eq!(verts[0].pos2, [0.04, 0.05]);
        assert_eq!(verts[1].pos2, [0.34, 0.05]);
        assert_eq!(verts[2].pos2, [0.04, 0.09]);
        assert_eq!(verts[3].pos2, [0.34, 0.09]);
        assert_eq!(verts[0].uv2, [0.0, 0.0]);
        assert_eq!(verts[1].uv2, [1.0, 0.0]);
        assert_eq!(verts[2].uv2, [0.0, 1.0]);
        assert_eq!(verts[3].uv2, [1.0, 1.0]);
        for v in verts.iter().take(4) {
            assert_eq!(v.col4, [12, 12, 14, 255]);
        }
        assert_eq!(verts[4].pos2[0], 0.04);
        assert_eq!(verts[5].pos2[0], 0.04 + 0.30 * 0.5);
        assert_eq!(verts[7].pos2, [0.04 + 0.30 * 0.5, 0.09]);
        for v in verts.iter().skip(4) {
            assert_eq!(v.col4, [196, 48, 44, 255]);
        }
        let empty = OverlayRing::new();
        assert!(empty.draw_plan(16.0 / 9.0).is_none());
        let mut one = vec![QuadVertex::default(); 4];
        assert_eq!(empty.write_vertices(&mut one), 0);
        let mut short = vec![QuadVertex::default(); 6];
        assert_eq!(ring.write_vertices(&mut short), 4);
    }

    #[test]
    fn overlay_draw_plan_falls_back_on_bad_aspect() {
        let mut ring = OverlayRing::new();
        assert!(ring.push_hp_bar(0.04, 0.05, 0.34, 0.09, 1.0));
        let plan = ring.draw_plan(0.0).expect("bad aspect still draws");
        assert_eq!(plan.mvp, overlay_mvp(1.0));
    }

    #[test]
    fn overlay_draw_plan_caps_at_fixed_ring() {
        let mut ring = OverlayRing::new();
        let q = Quad {
            rect: [0.1, 0.1, 0.2, 0.2],
            uv: [0.0, 0.0, 1.0, 1.0],
            col: [255, 255, 255, 255],
        };
        for _ in 0..OVERLAY_MAX_QUADS {
            assert!(ring.push_quad(q));
        }
        let plan = ring.draw_plan(16.0 / 9.0).expect("full ring draws once");
        assert_eq!(plan.quads, OVERLAY_MAX_QUADS);
        assert_eq!(plan.verts, OVERLAY_MAX_QUADS * VERTS_PER_QUAD);
    }

    #[test]
    fn overlay_wgsl_parses_and_vertex_layout_matches_quad_vertex() {
        assert!(OVERLAY_WGSL.contains("@vertex"));
        assert!(OVERLAY_WGSL.contains("@fragment"));
        assert!(OVERLAY_WGSL.contains("overlay_mvp"));
        assert!(OVERLAY_WGSL.contains("pos2"));
        assert!(OVERLAY_WGSL.contains("col4"));
        assert!(OVERLAY_WGSL.contains("fn vs"));
        assert!(OVERLAY_WGSL.contains("fn fs"));
        let open = OVERLAY_WGSL.bytes().filter(|b| *b == b'{').count();
        let close = OVERLAY_WGSL.bytes().filter(|b| *b == b'}').count();
        assert_eq!(open, close);
        assert!(open > 0);
        let layout = overlay_vertex_layout();
        assert_eq!(layout.stride, core::mem::size_of::<QuadVertex>());
        assert_eq!(layout.pos_offset, core::mem::offset_of!(QuadVertex, pos2));
        assert_eq!(layout.col_offset, core::mem::offset_of!(QuadVertex, col4));
        assert!(layout.pos_offset < layout.col_offset);
        assert!(layout.col_offset.saturating_add(4) <= layout.stride);
        let desc = overlay_pipeline_desc();
        assert_eq!((desc.vs_entry, desc.fs_entry), ("vs", "fs"));
        assert!(desc.alpha_blend);
        assert!(!desc.has_depth);
    }

    #[test]
    fn overlay_wgsl_samples_atlas_texture_for_textured_quads() {
        assert!(
            OVERLAY_WGSL.contains("uv2"),
            "textured quad passes uv2 to the fragment"
        );
        assert!(
            OVERLAY_WGSL.contains("textureSample"),
            "textured quad samples the atlas texture"
        );
        assert!(
            OVERLAY_WGSL.contains("overlay_atlas"),
            "atlas texture binding is named overlay_atlas"
        );
    }

    #[test]
    fn overlay_hp_entry_delegates_to_single_draw_plan() {
        // Caller: `crates/render/src/present/gpu_present/live.rs` keeps its
        // named `hp_bars_plan` entry point while one owner builds the plan,
        // so a second construction site cannot drift from `draw_plan`.
        // Structural: exactly one `Some(OverlayDraw` construction site stays.
        // (The needle is joined so this test's own source never matches it.)
        let src = include_str!("overlay.rs");
        let needle = ["Some(OverlayDraw", " {"].concat();
        let sites = src.match_indices(needle.as_str()).count();
        assert_eq!(sites, 1, "one draw-plan owner; hp entry delegates");
        // Behavioral: the named hp entry still matches the draw plan on the
        // capped full ring and on the bad-aspect fallback.
        let mut full = OverlayRing::new();
        let q = Quad {
            rect: [0.1, 0.1, 0.2, 0.2],
            uv: [0.0, 0.0, 1.0, 1.0],
            col: [255, 255, 255, 255],
        };
        for _ in 0..OVERLAY_MAX_QUADS {
            assert!(full.push_quad(q));
        }
        assert_eq!(
            full.hp_bars_plan(16.0 / 9.0),
            full.draw_plan(16.0 / 9.0),
            "full ring stays one plan"
        );
        let mut ring = OverlayRing::new();
        assert!(ring.push_hp_bar(0.04, 0.05, 0.34, 0.09, 1.0));
        assert_eq!(
            ring.hp_bars_plan(0.0),
            ring.draw_plan(0.0),
            "bad aspect still matches"
        );
        assert_eq!(
            ring.hp_bars_plan(0.0).expect("bad aspect draws").mvp,
            overlay_mvp(1.0),
            "bad aspect falls back together"
        );
    }
}
