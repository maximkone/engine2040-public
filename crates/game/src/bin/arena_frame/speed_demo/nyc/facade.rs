//! facade (HC3, docs/SPRINT-34-hero-city.md): the procedural facade tiles of the hero city.
//! One tile is one bay (3.0 m wide) by one floor (3.6 m tall) of a building face, and it
//! repeats in both directions, so its edges match: the left column equals the right column
//! and the top row equals the bottom row in every map. Per style it gives a base colour tile,
//! a tangent-space normal tile (GL convention, +y up), an ORM tile (occlusion, roughness,
//! metallic) and a lit mask of the window glass; `window_lit` decides which windows a night
//! look lights. Pure bytes, no GPU, no file IO. Presentation only, seeded.

use engine_core::checked::saturating_f64_to_u64;
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::splitmix64;

use super::super::num::{f32_of, floor_idx, idx, scale, unit, unit_f32};
use super::Style;

/// Tile edge in texels; the tile is square.
pub(super) const TILE_PX: u32 = 256;
/// Bay width in meters: the tile's width in world units.
const BAY_M: f32 = 3.0;
/// Floor height in meters: the tile's height in world units.
const FLOOR_M: f32 = 3.6;
/// Bricks across one bay: an even count, so the running bond repeats.
const BRICKS_PER_BAY: usize = 14;
/// Brick height in meters.
const BRICK_H_M: f32 = 0.065;
/// Mortar joint width in meters, horizontal and vertical.
const JOINT_M: f32 = 0.01;
/// Course pitch in meters: brick height plus one joint (48 courses per floor).
const COURSE_M: f32 = BRICK_H_M + JOINT_M;
/// Width of a window recess bevel in texels, from the wall down to the glass.
const BEVEL_PX: usize = 4;
/// Relief a recess bevel drops per texel, in texel units.
const BEVEL_STEP: f32 = 0.5;
/// Relief of a mortar joint or a stone course line, in texel units (recessed).
const JOINT_RELIEF: f32 = -0.5;
/// Relief of a raised metal mullion or spandrel, in texel units.
const METAL_RELIEF: f32 = 1.0;
/// Relief of a Deco pier stripe, in texel units.
const PIER_RELIEF: f32 = 0.5;
/// Occlusion inside a window recess (1 is unoccluded).
const RECESS_AO: f32 = 0.7;
/// Lit glass colour as RGBA bytes: warm white, opaque.
const LIT_RGBA: [u8; 4] = [255, 214, 160, 255];
/// Lit-window share bounds per building: a seeded 25..35% of its windows are lit.
const LIT_SHARE_MIN: f32 = 0.25;
/// Width of the seeded lit-share range above its minimum.
const LIT_SHARE_SPAN: f32 = 0.10;
/// Value-noise lattice cell in texels.
const NOISE_CELL: usize = 16;

/// Limestone albedo, sRGB.
const STONE_RGB: [f32; 3] = [205.0, 190.0, 165.0];
/// Darker stone of the lintel and sill, sRGB.
const STONE_SILL_RGB: [f32; 3] = [168.0, 156.0, 138.0];
/// Stone course line pitch in meters.
const STONE_COURSE_M: f32 = 0.6;
/// Height of the first stone course line above the floor line, in meters.
const STONE_COURSE_OFFSET_M: f32 = 0.3;
/// Lintel and sill band height in texels.
const LINTEL_PX: usize = 6;
/// Window glass of the Stone and Deco styles, sRGB.
const WINDOW_GLASS_RGB: [f32; 3] = [30.0, 44.0, 50.0];

/// Lighter cream Deco albedo, sRGB.
const DECO_RGB: [f32; 3] = [238.0, 228.0, 205.0];
/// Deco pier stripe albedo at the tile edges, sRGB.
const DECO_PIER_RGB: [f32; 3] = [214.0, 200.0, 168.0];
/// Deco decorative band under the window, sRGB.
const DECO_BAND_RGB: [f32; 3] = [196.0, 182.0, 152.0];
/// Deco pier stripe width in texels, on each side of the tile edge.
const PIER_PX: usize = 5;
/// First texel row of the Deco band under the window.
const DECO_BAND_Y0: usize = 196;
/// One past the last texel row of the Deco band.
const DECO_BAND_Y1: usize = 204;
/// Deco band inset from the tile's left and right edges, in texels.
const DECO_BAND_INSET_PX: usize = 8;

/// Dark blue-green reflective glass of the Glass style, sRGB.
const PANE_RGB: [f32; 3] = [30.0, 66.0, 74.0];
/// Dark grey spandrel metal at the floor line, sRGB.
const SPANDREL_RGB: [f32; 3] = [58.0, 60.0, 62.0];
/// Dark grey mullion metal, sRGB.
const MULLION_RGB: [f32; 3] = [70.0, 72.0, 76.0];
/// First texel row of the spandrel band (about 12% of the floor, low in the tile).
const SPANDREL_Y0: usize = 205;
/// One past the last texel row of the spandrel band.
const SPANDREL_Y1: usize = 236;
/// First texel column of the vertical mullion, centred in the bay.
const MULLION_X0: usize = 124;
/// One past the last texel column of the vertical mullion.
const MULLION_X1: usize = 132;

/// Brick albedo before the per-brick jitter, sRGB.
const BRICK_RGB: [f32; 3] = [142.0, 66.0, 52.0];
/// Light mortar albedo, sRGB.
const MORTAR_RGB: [f32; 3] = [200.0, 192.0, 178.0];
/// White sash frame and bar, sRGB.
const SASH_WHITE_RGB: [f32; 3] = [236.0, 234.0, 228.0];
/// Dark sash glass, sRGB.
const SASH_GLASS_RGB: [f32; 3] = [24.0, 34.0, 40.0];
/// Sash frame width in texels, inside the opening.
const SASH_FRAME_PX: usize = 4;

/// The four RGBA8 tile maps of one style's facade, 256 by 256 texels, rows top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FacadeTiles {
    /// Base colour (sRGB), alpha 255.
    pub base: Vec<u8>,
    /// Tangent-space normal, encoded `(n * 0.5 + 0.5) * 255`, alpha 255.
    pub normal: Vec<u8>,
    /// Occlusion (R), roughness (G), metallic (B), alpha 255.
    pub orm: Vec<u8>,
    /// Window glass mask: warm white where glass (alpha 255), transparent black elsewhere.
    pub lit: Vec<u8>,
}

/// The facade tiles of `style`, seeded by `seed`: the same seed gives the same bytes.
pub(super) fn tiles(style: Style, seed: u64) -> FacadeTiles {
    let n = idx(TILE_PX);
    let mut canvas = match style {
        Style::Brick => Canvas::from_fn(n, |x, y| brick_texel(x, y, n, seed)),
        Style::Stone => Canvas::from_fn(n, |x, y| stone_texel(x, y, n, seed)),
        Style::Deco => Canvas::from_fn(n, |x, y| deco_texel(x, y, n, seed)),
        Style::Glass => Canvas::from_fn(n, |x, y| glass_texel(x, y, seed)),
    };
    canvas.shade();
    canvas.make_seamless();
    canvas.encode()
}

/// Whether window `bay` on `floor` of a building seeded `seed` is lit at night. The building
/// has its own seeded lit share (25..35%); a window is lit when its hash falls under it.
pub(super) fn window_lit(seed: u64, bay: u32, floor: u32) -> bool {
    let share = LIT_SHARE_MIN + LIT_SHARE_SPAN * unit_f32(splitmix64(seed));
    unit_f32(hash2(seed, u64::from(bay), u64::from(floor))) < share
}

/// One texel before encoding.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Texel {
    /// Relief height in texel units; the normal is the slope of this.
    h: f32,
    /// Tangent-space normal, unit length (GL convention).
    normal: [f32; 3],
    /// Albedo, sRGB 0..255.
    rgb: [f32; 3],
    /// Ambient occlusion: 1 is unoccluded.
    ao: f32,
    /// Roughness, 0..1.
    rough: f32,
    /// Metallic, 0..1.
    metal: f32,
    /// Window glass: the lit mask covers it.
    glass: bool,
}

impl Texel {
    /// A flat wall texel of `rgb` with roughness `rough`: no relief, unoccluded, not metal.
    fn wall(rgb: [f32; 3], rough: f32) -> Self {
        Self {
            h: 0.0,
            normal: [0.0, 0.0, 1.0],
            rgb,
            ao: 1.0,
            rough,
            metal: 0.0,
            glass: false,
        }
    }
}

/// A square grid of texels, row-major, row 0 at the top.
struct Canvas {
    /// Edge length in texels.
    n: usize,
    /// The texels, `n * n` of them.
    px: Vec<Texel>,
}

impl Canvas {
    /// A canvas of `n` by `n` texels, each computed by `f(x, y)`.
    fn from_fn(n: usize, mut f: impl FnMut(usize, usize) -> Texel) -> Self {
        let mut px = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                px.push(f(x, y));
            }
        }
        Self { n, px }
    }

    /// Fills the normals from the relief: central differences, clamped at the edges. Tangent
    /// space in GL convention: `-dh/dx` for x and `+dh/drow` for y (rows grow downward).
    fn shade(&mut self) {
        let n = self.n;
        let h: Vec<f32> = self.px.iter().map(|t| t.h).collect();
        let at = |x: usize, y: usize| h.get(y * n + x).copied().unwrap_or(0.0);
        for (i, t) in self.px.iter_mut().enumerate() {
            let (x, y) = (i % n, i / n);
            let gx = (at((x + 1).min(n - 1), y) - at(x.saturating_sub(1), y)) * 0.5;
            let gy = (at(x, (y + 1).min(n - 1)) - at(x, y.saturating_sub(1))) * 0.5;
            t.normal = unit([-gx, gy, 1.0]);
        }
    }

    /// Makes the edges match exactly: the last column takes the first, then the last row
    /// takes the first, so the left column equals the right and the top equals the bottom.
    fn make_seamless(&mut self) {
        let n = self.n;
        for row in self.px.chunks_exact_mut(n) {
            let first = row.first().copied();
            if let (Some(first), Some(last)) = (first, row.last_mut()) {
                *last = first;
            }
        }
        self.px.copy_within(0..n, (n - 1) * n);
    }

    /// The four RGBA8 maps of the canvas.
    fn encode(&self) -> FacadeTiles {
        let len = self.px.len() * 4;
        let mut out = FacadeTiles {
            base: Vec::with_capacity(len),
            normal: Vec::with_capacity(len),
            orm: Vec::with_capacity(len),
            lit: Vec::with_capacity(len),
        };
        for t in &self.px {
            let [r, g, b] = t.rgb;
            out.base
                .extend_from_slice(&[byte_of(r), byte_of(g), byte_of(b), 255]);
            let [nx, ny, nz] = t.normal;
            out.normal
                .extend_from_slice(&[normal_byte(nx), normal_byte(ny), normal_byte(nz), 255]);
            out.orm.extend_from_slice(&[
                byte_of(t.ao * 255.0),
                byte_of(t.rough * 255.0),
                byte_of(t.metal * 255.0),
                255,
            ]);
            let lit = if t.glass { LIT_RGBA } else { [0, 0, 0, 0] };
            out.lit.extend_from_slice(&lit);
        }
        out
    }
}

/// A window opening in texels: columns `x0..x1`, rows `y0..y1`, row 0 at the top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Opening {
    /// First column.
    x0: usize,
    /// One past the last column.
    x1: usize,
    /// First row.
    y0: usize,
    /// One past the last row.
    y1: usize,
}

impl Opening {
    /// An opening of `width` by `height` (fractions of the tile), centred in an `n` texel tile.
    fn centred(n: usize, width: f32, height: f32) -> Self {
        let nf = f32_of(n);
        let x0 = texel_of(nf * (1.0 - width) * 0.5);
        let y0 = texel_of(nf * (1.0 - height) * 0.5);
        Self {
            x0,
            x1: n - x0,
            y0,
            y1: n - y0,
        }
    }

    /// Whether texel `(x, y)` lies inside the opening.
    fn contains(&self, x: usize, y: usize) -> bool {
        (self.x0..self.x1).contains(&x) && (self.y0..self.y1).contains(&y)
    }

    /// Texels from the nearest opening edge, 0 on the edge (for `(x, y)` inside the opening).
    fn depth(&self, x: usize, y: usize) -> usize {
        (x - self.x0)
            .min(self.x1 - 1 - x)
            .min(y - self.y0)
            .min(self.y1 - 1 - y)
    }

    /// Relief of texel `(x, y)` inside the opening: a bevel drops from the wall to the glass
    /// over `BEVEL_PX` texels, so the edges tilt inward.
    fn recess_h(&self, x: usize, y: usize) -> f32 {
        -BEVEL_STEP * f32_of(self.depth(x, y).min(BEVEL_PX - 1) + 1)
    }
}

/// A glass texel in a window recess: the bevel's relief, occluded, lit by the night mask.
fn recessed_glass(win: &Opening, x: usize, y: usize, seed: u64, rgb: [f32; 3]) -> Texel {
    let k = 1.0 + 0.03 * grain(seed, x, y);
    Texel {
        h: win.recess_h(x, y),
        ao: RECESS_AO,
        glass: true,
        ..Texel::wall(scale(rgb, k), 0.1)
    }
}

/// The Stone texel at `(x, y)`: limestone with course lines, a darker lintel and sill, and a
/// tall window centred in the bay.
fn stone_texel(x: usize, y: usize, n: usize, seed: u64) -> Texel {
    let win = Opening::centred(n, 0.45, 0.55);
    if win.contains(x, y) {
        return recessed_glass(&win, x, y, seed, WINDOW_GLASS_RGB);
    }
    let k = 1.0 + 0.03 * value_noise(seed, x, y);
    let band = (y + LINTEL_PX >= win.y0 && y < win.y0) || (y >= win.y1 && y < win.y1 + LINTEL_PX);
    let rgb = if band { STONE_SILL_RGB } else { STONE_RGB };
    let mut t = Texel::wall(scale(rgb, k), 0.8);
    if course_line(y, n, STONE_COURSE_M, STONE_COURSE_OFFSET_M) {
        t.rgb = scale(t.rgb, 0.9);
        t.h = JOINT_RELIEF;
    }
    t
}

/// The Deco texel at `(x, y)`: lighter cream with a pier stripe on each tile edge, a band
/// under a 40% wide window.
fn deco_texel(x: usize, y: usize, n: usize, seed: u64) -> Texel {
    let win = Opening::centred(n, 0.40, 0.50);
    if win.contains(x, y) {
        return recessed_glass(&win, x, y, seed, WINDOW_GLASS_RGB);
    }
    let k = 1.0 + 0.02 * value_noise(seed, x, y);
    if x < PIER_PX || x + PIER_PX >= n {
        return Texel {
            h: PIER_RELIEF,
            ..Texel::wall(scale(DECO_PIER_RGB, k), 0.8)
        };
    }
    let band = (DECO_BAND_Y0..DECO_BAND_Y1).contains(&y)
        && (DECO_BAND_INSET_PX..n - DECO_BAND_INSET_PX).contains(&x);
    let rgb = if band { DECO_BAND_RGB } else { DECO_RGB };
    Texel::wall(scale(rgb, k), 0.8)
}

/// The Glass texel at `(x, y)`: dark reflective glass with a dark grey spandrel band at the
/// floor line and a centred metal mullion.
fn glass_texel(x: usize, y: usize, seed: u64) -> Texel {
    if (SPANDREL_Y0..SPANDREL_Y1).contains(&y) {
        return Texel {
            h: METAL_RELIEF,
            metal: 0.9,
            rough: 0.35,
            ..Texel::wall(SPANDREL_RGB, 0.35)
        };
    }
    if (MULLION_X0..MULLION_X1).contains(&x) {
        return Texel {
            h: METAL_RELIEF,
            metal: 0.9,
            rough: 0.35,
            ..Texel::wall(MULLION_RGB, 0.35)
        };
    }
    let k = 1.0 + 0.04 * grain(seed, x, y);
    Texel {
        metal: 0.6,
        glass: true,
        ..Texel::wall(scale(PANE_RGB, k), 0.08)
    }
}

/// The Brick texel at `(x, y)`: a running bond of 14 bricks by 48 courses per tile with
/// recessed mortar, and a sash window centred in the bay.
fn brick_texel(x: usize, y: usize, n: usize, seed: u64) -> Texel {
    let win = Opening::centred(n, 0.40, 0.50);
    if win.contains(x, y) {
        return sash_texel(&win, x, y, seed);
    }
    let (course, brick, joint) = brick_cell(x, y, n);
    if joint {
        let k = 1.0 + 0.02 * grain(seed, x, y);
        return Texel {
            h: JOINT_RELIEF,
            ao: 0.8,
            ..Texel::wall(scale(MORTAR_RGB, k), 0.95)
        };
    }
    let jitter = 1.0 + 0.10 * signed(hash2(seed, word(course), word(brick)));
    let k = jitter * (1.0 + 0.04 * grain(seed, x, y));
    Texel::wall(scale(BRICK_RGB, k), 0.85)
}

/// The sash inside a brick window: a white frame, a white sash bar across the middle and dark
/// glass, all recessed.
fn sash_texel(win: &Opening, x: usize, y: usize, seed: u64) -> Texel {
    let mid = (win.y0 + win.y1) / 2;
    let frame = win.depth(x, y) < SASH_FRAME_PX;
    let bar = mid.abs_diff(y) < 2;
    let glass = !(frame || bar);
    let (rgb, rough) = if glass {
        (SASH_GLASS_RGB, 0.1)
    } else {
        (SASH_WHITE_RGB, 0.5)
    };
    let k = 1.0 + 0.03 * grain(seed, x, y);
    Texel {
        h: win.recess_h(x, y),
        ao: RECESS_AO,
        glass,
        ..Texel::wall(scale(rgb, k), rough)
    }
}

/// The brick at texel `(x, y)` as `(course, brick, in_joint)`. Odd courses shift half a brick
/// (running bond); the bay wraps after 14 bricks, so the pattern repeats.
fn brick_cell(x: usize, y: usize, n: usize) -> (usize, usize, bool) {
    let nf = f32_of(n);
    let u_m = (f32_of(x) + 0.5) / nf * BAY_M;
    let v_m = (f32_of(y) + 0.5) / nf * FLOOR_M;
    let course = floor_idx(v_m / COURSE_M);
    let local_v = v_m - f32_of(course) * COURSE_M;
    let pitch = BAY_M / f32_of(BRICKS_PER_BAY);
    let shift = if course % 2 == 1 { pitch * 0.5 } else { 0.0 };
    let u = (u_m + shift).rem_euclid(BAY_M);
    let brick = floor_idx(u / pitch);
    let local_u = u - f32_of(brick) * pitch;
    let joint = local_v >= BRICK_H_M || local_u >= pitch - JOINT_M;
    (course, brick, joint)
}

/// Whether texel row `y` sits on a thin course line, one at every `pitch_m` up the floor
/// starting `offset_m` above the floor line.
fn course_line(y: usize, n: usize, pitch_m: f32, offset_m: f32) -> bool {
    let v_m = (f32_of(y) + 0.5) / f32_of(n) * FLOOR_M;
    let m = (v_m - offset_m) / pitch_m;
    m - m.floor() < 0.035
}

/// A seeded hash of two words: SplitMix64 of the seed stepped by the golden gamma per word.
fn hash2(seed: u64, a: u64, b: u64) -> u64 {
    let s = splitmix64(seed.wrapping_add(GOLDEN_GAMMA.wrapping_mul(a.wrapping_add(1))));
    splitmix64(s.wrapping_add(GOLDEN_GAMMA.wrapping_mul(b.wrapping_add(1))))
}

/// A count as a hash word, saturating.
fn word(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// A hash word as a value in `[-1, 1)`.
fn signed(w: u64) -> f32 {
    unit_f32(w) * 2.0 - 1.0
}

/// White grain at texel `(x, y)` in `[-1, 1)`; the seed is complemented so it does not repeat
/// the value-noise lattice.
fn grain(seed: u64, x: usize, y: usize) -> f32 {
    signed(hash2(!seed, word(x), word(y)))
}

/// The value-noise lattice value at cell `(i, j)` in `[-1, 1)`.
fn lattice(seed: u64, i: usize, j: usize) -> f32 {
    signed(hash2(seed, word(i), word(j)))
}

/// Smooth value noise at texel `(x, y)` in `[-1, 1]`, one lattice cell per `NOISE_CELL`
/// texels; the lattice is seeded, so the same seed gives the same wall.
fn value_noise(seed: u64, x: usize, y: usize) -> f32 {
    let (i, j) = (x / NOISE_CELL, y / NOISE_CELL);
    let tx = smooth(f32_of(x % NOISE_CELL) / f32_of(NOISE_CELL));
    let ty = smooth(f32_of(y % NOISE_CELL) / f32_of(NOISE_CELL));
    let top = lerp(lattice(seed, i, j), lattice(seed, i + 1, j), tx);
    let bottom = lerp(lattice(seed, i, j + 1), lattice(seed, i + 1, j + 1), tx);
    lerp(top, bottom, ty)
}

/// Smoothstep of `t` in `[0, 1]`.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Linear interpolation from `a` to `b` by `t`.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A texel coordinate from a float in texels: rounded, never negative.
fn texel_of(v: f32) -> usize {
    usize::try_from(saturating_f64_to_u64(f64::from(v.round()))).unwrap_or(0)
}

/// A 0..255 channel as a byte: rounded and clamped (NaN reads as 0).
fn byte_of(v: f32) -> u8 {
    let r = f64::from(v.clamp(0.0, 255.0).round());
    u8::try_from(saturating_f64_to_u64(r)).unwrap_or(u8::MAX)
}

/// A unit normal component as a byte: `(n * 0.5 + 0.5) * 255`, so flat is 128.
fn normal_byte(v: f32) -> u8 {
    byte_of((v * 0.5 + 0.5) * 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RGBA bytes of texel `(x, y)` in a map of `TILE_PX` texels.
    fn px(map: &[u8], x: usize, y: usize) -> &[u8] {
        let n = idx(TILE_PX);
        &map[(y * n + x) * 4..][..4]
    }

    /// A decoded tangent-space normal from its four bytes.
    fn decode(b: &[u8]) -> [f32; 3] {
        [
            f32::from(b[0]) / 255.0 * 2.0 - 1.0,
            f32::from(b[1]) / 255.0 * 2.0 - 1.0,
            f32::from(b[2]) / 255.0 * 2.0 - 1.0,
        ]
    }

    /// Every buffer of every style is one 256 by 256 RGBA8 tile.
    #[test]
    fn every_buffer_is_one_256_square_rgba_tile() {
        let want = idx(TILE_PX) * idx(TILE_PX) * 4;
        for style in Style::ALL {
            let t = tiles(style, 1);
            for map in [&t.base, &t.normal, &t.orm, &t.lit] {
                assert_eq!(map.len(), want, "{style:?}");
            }
        }
    }

    /// Decoded normals are unit length within 0.03 and point out of the wall (z above 0.5).
    #[test]
    fn decoded_normals_are_unit_and_face_out() {
        for style in Style::ALL {
            let t = tiles(style, 2);
            for b in t.normal.chunks_exact(4) {
                let v = decode(b);
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                assert!((len - 1.0).abs() < 0.03, "{style:?} length {len}");
                assert!(v[2] > 0.5, "{style:?} z {}", v[2]);
            }
        }
    }

    /// The tile repeats: left column equals right, top row equals bottom, in base, normal and ORM.
    #[test]
    fn tiles_repeat_across_both_edges() {
        let n = idx(TILE_PX);
        for style in Style::ALL {
            let t = tiles(style, 4);
            for map in [&t.base, &t.normal, &t.orm] {
                for y in 0..n {
                    assert_eq!(px(map, 0, y), px(map, n - 1, y), "{style:?} column");
                }
                for x in 0..n {
                    assert_eq!(px(map, x, 0), px(map, x, n - 1), "{style:?} row");
                }
            }
        }
    }

    /// Glass covers more than 70% of the Glass style and 15 to 35% of the others.
    #[test]
    fn glass_share_is_in_range_per_style() {
        let area = f32_of(idx(TILE_PX) * idx(TILE_PX));
        for style in Style::ALL {
            let t = tiles(style, 6);
            let glass = t.lit.chunks_exact(4).filter(|p| p[3] == 255).count();
            let share = f32_of(glass) / area;
            match style {
                Style::Glass => assert!(share > 0.7, "Glass share {share}"),
                _ => assert!((0.15..=0.35).contains(&share), "{style:?} share {share}"),
            }
        }
    }

    /// Over 10,000 windows the lit share is between 20% and 40%.
    #[test]
    fn lit_share_over_ten_thousand_windows_is_twenty_to_forty_percent() {
        let mut lit = 0usize;
        for bay in 0..100u32 {
            for floor in 0..100u32 {
                if window_lit(7, bay, floor) {
                    lit += 1;
                }
            }
        }
        let share = f32_of(lit) / 10_000.0;
        assert!((0.2..0.4).contains(&share), "lit share {share}");
    }

    /// The same seed gives the same bytes in every style.
    #[test]
    fn same_seed_same_bytes() {
        for style in Style::ALL {
            assert!(tiles(style, 8) == tiles(style, 8), "{style:?}");
        }
    }
}
