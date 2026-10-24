//! hextile: RL51, sand and walls without a visible repeat.
//!
//! Owner: render lane. Consumers: the forward shader's `fs` entry
//! (`shaders/forward.wgsl`, the `RL51 surface detail` block) and the window
//! presenter, which names a mesh's [`SurfaceDetail`] through
//! `Presenter::set_surface_detail`. This file is the CPU twin of the shader's
//! hex-tiling grid, its PCG2D hash and its macro value noise: every constant
//! and every step is the same, so the tests below pin what the GPU draws.
//!
//! Hex tiling after Mikkelsen, "Practical Real-Time Hex-Tiling" (JCGT 2022),
//! translation only. A tile coordinate falls in one triangle of a skewed grid;
//! the triangle's three vertices each give the tile a hashed shift, the three
//! shifted taps blend by the barycentric weights raised to the sixth power.
//! A plain repeat shows its period (the same texel exactly one tile away);
//! the shifted taps do not. Translation only keeps the ripples and the mortar
//! lines in their direction and needs no normal rotation.
//!
//! Units: tile units for coordinates, meters for the macro noise. Determinism:
//! integer hash, same input same bytes on every adapter. Presentation only,
//! never hashed into the simulation. Budget: three taps per map instead of
//! one for a named mesh; zero for every other draw.

/// What the forward shader adds to one stone mesh: hex-tiled map taps plus a
/// macro term over the albedo. Rides the material uniform's parameter word
/// (bytes 28..32) of the stone kind, which no stone draw read before.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum SurfaceDetail {
    /// Plain taps, albedo as the texture says: every draw before RL51.
    #[default]
    None,
    /// Sand floor: hex taps and patches of dry and trampled sand.
    Floor,
    /// Wall or stands: hex taps, weathering streaks, dust on the foot.
    Wall,
}

impl SurfaceDetail {
    /// The parameter word the stone draw writes (bytes 28..32).
    pub const fn param(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Floor => 1.0,
            Self::Wall => 2.0,
        }
    }

    /// The detail a parameter word names, the shader's own rounding
    /// (`u32(max(param, 0) + 0.5)`); an unknown word is none.
    pub fn from_param(param: f32) -> Self {
        match floor_i32(param.max(0.0) + 0.5) {
            1 => Self::Floor,
            2 => Self::Wall,
            _ => Self::None,
        }
    }
}

/// Triangle-grid scale: `2 * sqrt(3)` cells per tile edge.
pub const HEX_GRID_SCALE: f32 = 3.464_101_6;

/// `floor(x)` as an integer, the shader's `i32(floor(x))`: a float-to-int
/// cast saturates, and a grid or lattice coordinate is far inside `i32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the floor of a grid coordinate is far inside i32 and the cast saturates"
)]
fn floor_i32(x: f32) -> i32 {
    x.floor() as i32
}

/// A 32-bit word as `f32`, rounded as the shader's `f32(u32)` is: two exact
/// 16-bit halves, one rounding in the sum.
fn word_to_f32(word: u32) -> f32 {
    let half = |v: u32| f32::from(u16::try_from(v & 0xffff).unwrap_or(0));
    half(word >> 16) * 65_536.0 + half(word)
}

/// PCG2D (Jarzynski and Olano 2020, "Hash Functions for GPU Rendering"): the
/// shader's `pcg2d`, wrapping integer math.
pub fn pcg2d(v_in: [u32; 2]) -> [u32; 2] {
    let mul = 1_664_525u32;
    let add = 1_013_904_223u32;
    let mut v = [
        v_in[0].wrapping_mul(mul).wrapping_add(add),
        v_in[1].wrapping_mul(mul).wrapping_add(add),
    ];
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(mul));
    v[1] = v[1].wrapping_add(v[0].wrapping_mul(mul));
    v = [v[0] ^ (v[0] >> 16), v[1] ^ (v[1] >> 16)];
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(mul));
    v[1] = v[1].wrapping_add(v[0].wrapping_mul(mul));
    [v[0] ^ (v[0] >> 16), v[1] ^ (v[1] >> 16)]
}

/// The shader's `hex_offset`: the tile shift of one grid vertex, `0..1`.
pub fn hex_offset(v: [i32; 2]) -> [f32; 2] {
    let h = pcg2d([v[0].cast_unsigned(), v[1].cast_unsigned()]);
    // 2^-32: 65_536 is exact, so the product and the division are exact.
    let unit = 1.0 / (65_536.0 * 65_536.0);
    [word_to_f32(h[0]) * unit, word_to_f32(h[1]) * unit]
}

/// The three taps of one tile coordinate and their normalized weights.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HexCell {
    /// The tile coordinate shifted by each of the triangle's three vertices.
    pub st: [[f32; 2]; 3],
    /// Blend weights, sharpened and normalized: sum 1.
    pub w: [f32; 3],
}

/// The shader's `hex_sharpen`: the sixth power, then normalized.
pub fn hex_sharpen(w: [f32; 3]) -> [f32; 3] {
    let k = w.map(|x| {
        let w2 = x * x;
        w2 * w2 * w2
    });
    let sum = k[0] + k[1] + k[2];
    k.map(|x| x / sum)
}

/// The shader's `hex_cell`.
pub fn hex_cell(st: [f32; 2]) -> HexCell {
    let g = [st[0] * HEX_GRID_SCALE, st[1] * HEX_GRID_SCALE];
    let skew = [g[0], -0.577_350_27 * g[0] + 1.154_700_5 * g[1]];
    let base = [skew[0].floor(), skew[1].floor()];
    let fr = [skew[0] - base[0], skew[1] - base[1]];
    let z = 1.0 - fr[0] - fr[1];
    let s = f32::from(z <= 0.0);
    let s2 = 2.0 * s - 1.0;
    let w = [-z * s2, s - fr[1] * s2, s - fr[0] * s2];
    let bi = [floor_i32(skew[0]), floor_i32(skew[1])];
    let si = i32::from(z <= 0.0);
    let at = |v: [i32; 2]| {
        let o = hex_offset(v);
        [st[0] + o[0], st[1] + o[1]]
    };
    HexCell {
        st: [
            at([bi[0] + si, bi[1] + si]),
            at([bi[0] + si, bi[1] + 1 - si]),
            at([bi[0] + 1 - si, bi[1] + si]),
        ],
        w: hex_sharpen(w),
    }
}

/// A square, tileable one-channel texture: the CPU stand-in for one map, wrap
/// addressing and bilinear taps like the forward pass's Repeat sampler.
#[derive(Clone, PartialEq, Debug)]
pub struct Tile {
    /// Texels per edge.
    pub size: usize,
    /// Row-major texels, `size * size`.
    pub texels: Vec<f32>,
}

impl Tile {
    /// Bilinear tap at a tile coordinate (one tile per unit, wrapping).
    pub fn sample(&self, st: [f32; 2]) -> f32 {
        let n = f32::from(u16::try_from(self.size).unwrap_or(u16::MAX));
        let edge = i32::try_from(self.size).unwrap_or(i32::MAX);
        let (fx, fy) = (st[0] * n - 0.5, st[1] * n - 0.5);
        let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
        let (x0, y0) = (floor_i32(fx), floor_i32(fy));
        let wrap = |v: i32| usize::try_from(v.rem_euclid(edge)).unwrap_or(0);
        let at = |x: i32, y: i32| self.texels[wrap(y) * self.size + wrap(x)];
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
        let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

/// The hex-tiled tap of one map: the shader's `hex_sample` over one channel.
pub fn hex_sample(tile: &Tile, st: [f32; 2]) -> f32 {
    let cell = hex_cell(st);
    (0..3).map(|i| tile.sample(cell.st[i]) * cell.w[i]).sum()
}

/// One value in `0..1` per integer lattice point: the shader's
/// `lattice_noise`.
pub fn lattice_noise(c: [i32; 2]) -> f32 {
    let h = pcg2d([c[0].cast_unsigned(), c[1].cast_unsigned()]);
    word_to_f32(h[0] >> 8) * (1.0 / 16_777_216.0)
}

/// Smooth value noise over a plane, `0..1`: the shader's `value_noise`.
pub fn value_noise(p: [f32; 2]) -> f32 {
    let i = [p[0].floor(), p[1].floor()];
    let f = [p[0] - i[0], p[1] - i[1]];
    let u = f.map(|x| x * x * (3.0 - 2.0 * x));
    let c = [floor_i32(p[0]), floor_i32(p[1])];
    let a = lattice_noise(c);
    let b = lattice_noise([c[0] + 1, c[1]]);
    let d = lattice_noise([c[0], c[1] + 1]);
    let e = lattice_noise([c[0] + 1, c[1] + 1]);
    let top = a + (b - a) * u[0];
    let bottom = d + (e - d) * u[0];
    top + (bottom - top) * u[1]
}

/// The luminance factor of the shader's `floor_macro` at a world position
/// (the tint rides on top of it and averages to one).
pub fn floor_macro_luma(world_xz: [f32; 2]) -> f32 {
    let n = 0.6 * value_noise([world_xz[0] * 0.23, world_xz[1] * 0.23])
        + 0.4 * value_noise([world_xz[0] * 0.9 + 17.0, world_xz[1] * 0.9 + 3.0]);
    0.86 + (1.1 - 0.86) * n
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A tileable noise texture: `size` squared white-noise texels from the
    /// same hash the grid uses, so it has energy at every frequency.
    fn noise_tile(size: usize) -> Tile {
        let texels = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as u32, (i / size) as u32);
                (pcg2d([x ^ 0x9e37, y ^ 0x79b9])[0] >> 8) as f32 / 16_777_216.0
            })
            .collect();
        Tile { size, texels }
    }

    /// A square patch of `periods` tile periods on a side, `px` pixels per
    /// period, one tap per pixel centre.
    fn patch(periods: usize, px: usize, tap: impl Fn([f32; 2]) -> f32) -> (usize, Vec<f32>) {
        let edge = periods * px;
        let mut out = Vec::with_capacity(edge * edge);
        for y in 0..edge {
            for x in 0..edge {
                out.push(tap([
                    (x as f32 + 0.5) / px as f32,
                    (y as f32 + 0.5) / px as f32,
                ]));
            }
        }
        (edge, out)
    }

    /// Normalized cross-correlation of the patch with itself shifted `lag`
    /// pixels along x, over the overlap: 1.0 means the shifted copy is the
    /// same picture, 0 means unrelated.
    fn autocorrelation(edge: usize, img: &[f32], lag: usize) -> f32 {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for y in 0..edge {
            for x in 0..edge - lag {
                a.push(img[y * edge + x]);
                b.push(img[y * edge + x + lag]);
            }
        }
        let n = a.len() as f32;
        let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
        let (mut cov, mut va, mut vb) = (0.0f32, 0.0f32, 0.0f32);
        for (p, q) in a.iter().zip(&b) {
            cov += (p - ma) * (q - mb);
            va += (p - ma) * (p - ma);
            vb += (q - mb) * (q - mb);
        }
        cov / (va * vb).sqrt().max(1e-12)
    }

    fn std_dev(img: &[f32]) -> f32 {
        let n = img.len() as f32;
        let m = img.iter().sum::<f32>() / n;
        (img.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / n).sqrt()
    }

    #[test]
    fn hextile_autocorrelation_at_one_period_drops_from_one() {
        // RL51 accept: on a 4x4-period patch the plain repeat reads the same
        // picture one tile away (autocorrelation 1.0, the row's F2P), the hex
        // taps do not (<= 0.5).
        let tile = noise_tile(32);
        let px = 64;
        let (edge, plain) = patch(4, px, |st| tile.sample(st));
        let (_, hex) = patch(4, px, |st| hex_sample(&tile, st));
        let plain_lag = autocorrelation(edge, &plain, px);
        let hex_lag = autocorrelation(edge, &hex, px);
        eprintln!("autocorrelation at one period: plain {plain_lag:.4} hex {hex_lag:.4}");
        assert!(
            plain_lag > 0.999,
            "the plain repeat is periodic: {plain_lag}"
        );
        assert!(hex_lag <= 0.5, "the hex taps break the period: {hex_lag}");
        // The same on the vertical lag: no axis keeps the repeat.
        let mut transposed = vec![0.0f32; edge * edge];
        for y in 0..edge {
            for x in 0..edge {
                transposed[x * edge + y] = hex[y * edge + x];
            }
        }
        let hex_lag_y = autocorrelation(edge, &transposed, px);
        assert!(hex_lag_y <= 0.5, "vertical lag: {hex_lag_y}");
    }

    #[test]
    fn hextile_keeps_most_of_the_contrast() {
        // The blend of two unrelated taps halves the contrast where it
        // happens; the sixth-power weights keep that band thin.
        let tile = noise_tile(32);
        let (_, plain) = patch(4, 64, |st| tile.sample(st));
        let (_, hex) = patch(4, 64, |st| hex_sample(&tile, st));
        let (sp, sh) = (std_dev(&plain), std_dev(&hex));
        eprintln!("contrast: plain {sp:.4} hex {sh:.4}");
        assert!(sh >= 0.75 * sp, "hex {sh} against plain {sp}");
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&plain) - mean(&hex)).abs() < 0.02, "the mean holds");
    }

    #[test]
    fn hextile_weights_are_a_partition_of_one() {
        for i in 0..2000u32 {
            let st = [(i % 47) as f32 * 0.173 - 3.0, (i / 47) as f32 * 0.311 - 5.0];
            let cell = hex_cell(st);
            let sum: f32 = cell.w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "weights sum {sum} at {st:?}");
            assert!(cell.w.iter().all(|w| (0.0..=1.0 + 1e-5).contains(w)));
        }
    }

    #[test]
    fn hextile_is_continuous_across_a_cell_edge() {
        // The blend must not step: two taps a hair apart agree closely.
        let tile = noise_tile(32);
        let mut worst = 0.0f32;
        for i in 0..4000u32 {
            let st = [
                (i % 61) as f32 * 0.0731 - 2.0,
                (i / 61) as f32 * 0.0517 - 2.0,
            ];
            let near = [st[0] + 0.0005, st[1]];
            worst = worst.max((hex_sample(&tile, st) - hex_sample(&tile, near)).abs());
        }
        assert!(worst < 0.05, "largest step over 0.0005 tile: {worst}");
    }

    #[test]
    fn hextile_offsets_are_a_pure_function_of_the_vertex() {
        assert_eq!(hex_offset([3, -7]), hex_offset([3, -7]));
        assert_ne!(hex_offset([3, -7]), hex_offset([4, -7]));
        for v in [[0, 0], [1, 1], [-5, 9], [123, -456]] {
            let o = hex_offset(v);
            assert!(o.iter().all(|c| (0.0..1.0).contains(c)), "{v:?} -> {o:?}");
        }
        // Pinned words (the PCG2D reference run on integers): the shader's
        // `pcg2d` is the same arithmetic, so it reads the same offsets.
        assert_eq!(pcg2d([0, 0]), [417_608_103, 90_043_601]);
        assert_eq!(pcg2d([1, 2]), [45_825_804, 214_070_181]);
        assert_eq!(pcg2d([3, (-7i32) as u32]), [1_779_034_556, 1_432_867_972]);
    }

    #[test]
    fn hextile_same_uv_same_result() {
        // Determinism: the taps, the weights and the macro term are pure
        // functions of the uv, so one uv gives one result on every call.
        let tile = noise_tile(32);
        for i in 0..500u32 {
            let st = [(i % 23) as f32 * 0.217 - 2.0, (i / 23) as f32 * 0.283 - 4.0];
            assert_eq!(hex_sample(&tile, st), hex_sample(&tile, st), "{st:?}");
            assert_eq!(hex_cell(st).w, hex_cell(st).w, "{st:?}");
            assert_eq!(floor_macro_luma(st), floor_macro_luma(st), "{st:?}");
        }
    }

    #[test]
    fn surface_detail_param_round_trips_and_stone_default_is_none() {
        for detail in [
            SurfaceDetail::None,
            SurfaceDetail::Floor,
            SurfaceDetail::Wall,
        ] {
            assert_eq!(SurfaceDetail::from_param(detail.param()), detail);
        }
        assert_eq!(SurfaceDetail::default(), SurfaceDetail::None);
        assert_eq!(SurfaceDetail::None.param(), 0.0, "stone's old image");
        assert_eq!(SurfaceDetail::from_param(7.0), SurfaceDetail::None);
        assert_eq!(SurfaceDetail::from_param(-1.0), SurfaceDetail::None);
    }

    #[test]
    fn macro_noise_is_deterministic_bounded_and_not_flat() {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..4000u32 {
            let p = [(i % 80) as f32 * 0.9 - 30.0, (i / 80) as f32 * 0.9 - 20.0];
            let n = value_noise(p);
            assert_eq!(n, value_noise(p), "same input, same noise");
            assert!((0.0..=1.0).contains(&n), "{n}");
            let luma = floor_macro_luma(p);
            assert!((0.86..=1.1).contains(&luma), "{luma}");
            lo = lo.min(luma);
            hi = hi.max(luma);
        }
        assert!(hi - lo > 0.12, "the sand patches move the tone: {lo}..{hi}");
    }
}
