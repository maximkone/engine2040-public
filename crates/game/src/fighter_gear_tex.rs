//! fighter_gear_tex: the procedural texels of the hero's iron, bronze, wood and painted
//! shield face (`fighter_gear`).
//!
//! The grafted gladius and scutum carry no texture of their own (their UVs point at an empty
//! corner of the body atlas, which reads black), so the look paints small maps here. Owner:
//! `fighter_gear`; consumer: `FighterLook::load_rig`. Units: sRGB RGBA8 texels, `u` and `v` in
//! `0..=1`. Determinism: a fixed integer hash seeds every value, so the same call gives the same
//! bytes. Budget: one cold pass at load, 128 x 128 or 256 x 256 texels per map.

/// A generated map: RGBA8, `width * height * 4` bytes, alpha always 255.
#[derive(Clone, PartialEq, Debug)]
pub struct Texels {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major sRGB RGBA8.
    pub rgba: Vec<u8>,
}

impl Texels {
    /// The map as PNG bytes (the capture codec: RGB8, stored deflate, so the same map gives the
    /// same bytes). The maps are opaque, so dropping alpha loses nothing. `None` when the codec
    /// refuses the size.
    #[must_use]
    pub fn png(&self) -> Option<Vec<u8>> {
        let rgb: Vec<u8> = self
            .rgba
            .chunks_exact(4)
            .flat_map(|texel| [texel[0], texel[1], texel[2]])
            .collect();
        crate::camera_feed::encode_capture_png(self.width, self.height, &rgb).ok()
    }
}

/// Edge of the small maps (blade, fittings, wood), pixels.
const SMALL: u32 = 128;
/// Edge of the shield face map, pixels.
const FACE: u32 = 256;

/// Steel blade base colour (sRGB, 0..1).
const STEEL: [f32; 3] = [0.70, 0.72, 0.76];
/// Bronze base colour.
const BRONZE: [f32; 3] = [0.78, 0.54, 0.24];
/// Wood base colour.
const WOOD: [f32; 3] = [0.44, 0.28, 0.14];
/// Painted shield red.
const PAINT: [f32; 3] = [0.62, 0.09, 0.07];
/// Gold of the shield's painted frame.
const GOLD: [f32; 3] = [0.86, 0.66, 0.20];

use super::fighter_kit_tex::byte;

/// One texel of a map being painted.
#[derive(Clone, Copy)]
struct Texel {
    col: u32,
    row: u32,
    /// Across the map, `0..1`.
    u: f32,
    /// Down the map, `0..1`.
    v: f32,
}

/// `n / d` as a float, for texel counts (a map edge is at most a few hundred).
fn ratio(n: u32, d: u32) -> f32 {
    let part = |x: u32| f32::from(u16::try_from(x).unwrap_or(u16::MAX));
    part(n) / part(d.max(1))
}

/// Integer lattice hash in `0..=1`.
fn hash01(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    ratio(h >> 16, 65_535)
}

/// Smooth value noise in `0..=1` over a lattice whose cells are `cx` by `cy` texels.
fn noise(at: Texel, cx: u32, cy: u32, seed: u32) -> f32 {
    let (ix, iy) = (at.col / cx, at.row / cy);
    let (tx, ty) = (ratio(at.col % cx, cx), ratio(at.row % cy, cy));
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let a = hash01(ix, iy, seed);
    let b = hash01(ix + 1, iy, seed);
    let c = hash01(ix, iy + 1, seed);
    let d = hash01(ix + 1, iy + 1, seed);
    let top = a + (b - a) * sx;
    let bottom = c + (d - c) * sx;
    top + (bottom - top) * sy
}

/// Scale a colour by `k` and clamp to 0..1.
fn shade(base: [f32; 3], k: f32) -> [f32; 3] {
    base.map(|c| (c * k).clamp(0.0, 1.0))
}

/// Paint a `size` square map from a per-texel colour function.
fn paint(size: u32, colour: impl Fn(Texel) -> [f32; 3]) -> Texels {
    let mut rgba = Vec::with_capacity(usize::try_from(size * size * 4).unwrap_or(0));
    for row in 0..size {
        for col in 0..size {
            let texel = Texel {
                col,
                row,
                u: (ratio(col, size)) + 0.5 / ratio(size, 1),
                v: (ratio(row, size)) + 0.5 / ratio(size, 1),
            };
            rgba.extend(colour(texel).map(byte));
            rgba.push(255);
        }
    }
    Texels {
        width: size,
        height: size,
        rgba,
    }
}

/// Brushed iron: streaks run along `v` (the blade), a darker fuller down the middle and
/// bright bevelled edges.
#[must_use]
pub fn steel() -> Texels {
    paint(SMALL, |t| {
        let streak = noise(t, 1, 42, 11);
        let mut k = 0.80 + 0.30 * streak;
        let from_middle = (t.u - 0.5).abs();
        if from_middle < 0.07 {
            k *= 0.80;
        }
        if from_middle > 0.42 {
            k *= 1.18;
        }
        shade(STEEL, k)
    })
}

/// Bronze: warm metal, a mottled sheen and dark pits.
#[must_use]
pub fn bronze() -> Texels {
    paint(SMALL, |t| {
        let sheen = noise(t, 9, 9, 23);
        let pit = noise(t, 2, 2, 29);
        let mut k = 0.78 + 0.34 * sheen;
        if pit > 0.82 {
            k *= 0.70;
        }
        shade(BRONZE, k)
    })
}

/// Wood: grain runs along `v`, rings wobble with low noise.
#[must_use]
pub fn wood() -> Texels {
    paint(SMALL, |t| {
        let wobble = noise(t, 25, 64, 31) * 2.5;
        let ring = ((t.u * 18.0 + wobble) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let fibre = noise(t, 1, 32, 37);
        let k = 0.70 + 0.22 * ring + 0.18 * fibre;
        shade(WOOD, k)
    })
}

/// The scutum's outer face: red paint over wood grain, a gold frame and spine, a bronze rim
/// band and a bronze boss. `u` runs across the short side, `v` along the long side; the
/// centre of the map is the boss. The hexagon of the slab sits inside the map square.
#[must_use]
pub fn shield_face() -> Texels {
    paint(FACE, |t| {
        let (a, b) = ((t.u - 0.5) * 2.0, (t.v - 0.5) * 2.0);
        // Distance to the rim follows the slab's cut corners.
        let edge = a.abs().max(b.abs()).max(0.72 * (a.abs() + b.abs()));
        let radius = (a * a + b * b).sqrt();
        let grain = 0.86 + 0.22 * noise(t, 28, 3, 41);
        let mut rgb = shade(PAINT, grain);
        if (0.68..0.72).contains(&edge) || a.abs() < 0.012 {
            rgb = shade(GOLD, grain);
        }
        if edge > 0.86 {
            rgb = shade(BRONZE, 0.80 + 0.30 * noise(t, 12, 12, 43));
        }
        if radius < 0.20 {
            let dome = 1.0 - radius / 0.20;
            rgb = shade(BRONZE, 0.70 + 0.55 * dome);
        }
        if (0.20..0.23).contains(&radius) {
            rgb = shade(BRONZE, 0.45);
        }
        rgb
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mean_luma(map: &Texels) -> f32 {
        let mut sum = 0.0;
        for texel in map.rgba.chunks_exact(4) {
            sum += 0.2126 * f32::from(texel[0])
                + 0.7152 * f32::from(texel[1])
                + 0.0722 * f32::from(texel[2]);
        }
        sum / (map.rgba.len() / 4) as f32 / 255.0
    }

    #[test]
    fn every_map_is_opaque_sized_and_not_black() {
        for (name, map, edge) in [
            ("steel", steel(), SMALL),
            ("bronze", bronze(), SMALL),
            ("wood", wood(), SMALL),
            ("face", shield_face(), FACE),
        ] {
            assert_eq!((map.width, map.height), (edge, edge), "{name} size");
            assert_eq!(map.rgba.len(), (edge * edge * 4) as usize, "{name} bytes");
            assert!(
                map.rgba.chunks_exact(4).all(|t| t[3] == 255),
                "{name} alpha"
            );
            assert!(mean_luma(&map) > 0.18, "{name} reads dark");
        }
    }

    #[test]
    fn the_maps_are_repeatable_and_differ_from_each_other() {
        assert_eq!(steel(), steel(), "same bytes twice");
        assert_ne!(steel().rgba, bronze().rgba);
        assert_ne!(bronze().rgba, wood().rgba);
    }

    #[test]
    fn byte_rounds_to_the_nearest_level() {
        assert_eq!(byte(0.0), 0);
        assert_eq!(byte(1.0), 255);
        assert_eq!(byte(0.5), 128);
        assert_eq!(byte(-3.0), 0);
        assert_eq!(byte(9.0), 255);
    }
}
