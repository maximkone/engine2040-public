//! fighter_kit_tex: the procedural texels of the hero's Roman kit (`fighter_kit`).
//!
//! The kit is generated geometry with no texture file, so the look paints its maps here: the
//! lorica's steel plates with their rivets and leather lacing, the galea's bronze, the red
//! tunic with its leather belt, leather for the sandals, horsehair for the crest. Owner:
//! `fighter_kit`; consumer: `FighterLook::load_rig` (cold path, once per rig). Units: sRGB
//! RGBA8 texels, `u` across, `v` down, both in `0..=1`. Determinism: an integer lattice hash
//! seeds every value, so the same call gives the same bytes. Budget: one pass at load, at most
//! 256 x 64 texels a map.

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

/// Lorica steel (sRGB, 0..1): a cool, slightly warm silver.
const STEEL: [f32; 3] = [0.60, 0.62, 0.65];
/// Dark gap between two plates.
const GAP: [f32; 3] = [0.16, 0.17, 0.19];
/// Tanned leather.
const LEATHER: [f32; 3] = [0.30, 0.17, 0.09];
/// Pale stitching thread.
const THREAD: [f32; 3] = [0.62, 0.50, 0.32];
/// Galea and greave bronze.
const BRONZE: [f32; 3] = [0.66, 0.49, 0.26];
/// Legionary tunic red.
const RED: [f32; 3] = [0.58, 0.07, 0.05];
/// Brass studs and hem trim.
const BRASS: [f32; 3] = [0.80, 0.60, 0.26];

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

/// `n` as a float (a texel count is far under `u16::MAX`).
fn count(n: u32) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// Integer lattice hash in `0..=1`.
fn hash01(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    count(h >> 16) / 65_535.0
}

/// Smooth value noise in `0..=1` over a lattice whose cells are `cx` by `cy` texels. The
/// lattice wraps across the map's width (`cx` divides it), so a map that closes into a ring has
/// no seam.
fn noise(at: Texel, cx: u32, cy: u32, width: u32, seed: u32) -> f32 {
    let cells = (width / cx).max(1);
    let (ix, iy) = (at.col / cx, at.row / cy);
    let (tx, ty) = (
        count(at.col % cx) / count(cx),
        count(at.row % cy) / count(cy),
    );
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let at = |dx: u32, dy: u32| hash01((ix + dx) % cells, iy + dy, seed);
    let top = at(0, 0) * (1.0 - sx) + at(1, 0) * sx;
    let bottom = at(0, 1) * (1.0 - sx) + at(1, 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

/// Linear blend of two colours.
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// A colour scaled by `k`.
fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    c.map(|v| v * k)
}

/// A 0..1 channel as the nearest byte (a binary search over the 256 levels).
pub(super) fn byte(channel: f32) -> u8 {
    let target = channel.clamp(0.0, 1.0) * 255.0;
    let (mut lo, mut hi) = (0u8, 255u8);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f32::from(mid) + 0.5 <= target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Paint a `width` by `height` map from a per-texel colour function.
fn paint(width: u32, height: u32, colour: impl Fn(Texel) -> [f32; 3]) -> Texels {
    let mut rgba = Vec::with_capacity(usize::try_from(width * height * 4).unwrap_or(0));
    for row in 0..height {
        for col in 0..width {
            let texel = Texel {
                col,
                row,
                u: (count(col) + 0.5) / count(width),
                v: (count(row) + 0.5) / count(height),
            };
            rgba.extend(colour(texel).map(byte));
            rgba.push(255);
        }
    }
    Texels {
        width,
        height,
        rgba,
    }
}

/// One lorica plate: `u` runs around the ring (the front centre at `0.5`), `v` down the plate.
/// Brushed steel, a rolled bright top edge, a highlight and a dark gap at the lower edge where
/// the next plate tucks under, a row of rivets, leather lacing at the front and back centre and
/// a brass-buckled strap on each side.
#[must_use]
pub fn lorica() -> Texels {
    paint(256, 64, |t| {
        let grain = 0.5 * noise(t, 2, 11, 256, 11) + 0.5 * noise(t, 8, 21, 256, 5);
        let mut c = mix(scale(STEEL, 0.80), scale(STEEL, 1.12), grain);
        c = scale(c, 1.12 - 0.34 * t.v);
        if t.v < 0.07 {
            c = scale(c, 1.12);
        }
        if t.v > 0.84 && t.v < 0.90 {
            c = mix(c, [0.86, 0.88, 0.92], 0.55);
        }
        if t.v >= 0.90 {
            c = mix(c, GAP, (t.v - 0.90) / 0.05);
        }
        // Rivets.
        let across = (t.u * 24.0).fract() - 0.5;
        let down = (t.v - 0.28) * 7.0;
        let d = across.hypot(down);
        if t.v > 0.2 && t.v < 0.36 {
            if d < 0.16 {
                c = mix([1.0, 0.93, 0.78], c, d / 0.16 * 0.6);
            } else if d < 0.22 {
                c = scale(c, 0.55);
            }
        }
        // Lacing at the front and back centre.
        let from_seam = (t.u - 0.5).abs().min(t.u).min(1.0 - t.u);
        if from_seam < 0.0085 {
            c = scale(LEATHER, 0.8 + 0.4 * noise(t, 2, 2, 256, 3));
            if (t.col + t.row) % 5 == 0 {
                c = mix(c, THREAD, 0.7);
            }
        } else if from_seam < 0.0115 {
            c = scale(c, 0.55);
        }
        // Side straps, brass at the buckle.
        let side = (t.u - 0.25).abs().min((t.u - 0.75).abs());
        if side < 0.014 && t.v > 0.1 && t.v < 0.78 {
            c = scale(LEATHER, 0.9);
            if t.v > 0.36 && t.v < 0.56 && side < 0.011 {
                c = BRASS;
            }
        }
        c
    })
}

/// Bronze for the galea and the greaves: a mottled grain with a dark rim, rivets along the rim
/// and, in the lower quarter, the ribbed flange the neck guard and cheek plates use.
#[must_use]
pub fn bronze() -> Texels {
    paint(128, 128, |t| {
        let grain = 0.6 * noise(t, 2, 13, 128, 17) + 0.4 * noise(t, 8, 8, 128, 29);
        let mut c = mix(scale(BRONZE, 0.82), scale(BRONZE, 1.15), grain);
        c = scale(c, 1.15 - 0.30 * t.v);
        if t.v > 0.74 {
            let rib = ((t.v - 0.74) / 0.26 * 6.0).fract();
            c = scale(c, 0.78 + 0.32 * (rib * std::f32::consts::PI).sin());
        }
        if t.v > 0.70 && t.v < 0.74 {
            c = mix(c, [0.95, 0.78, 0.45], 0.7);
        }
        if t.v < 0.04 {
            c = scale(c, 0.55);
        }
        let d = ((t.u * 32.0).fract() - 0.5).hypot((t.v - 0.62) * 8.0);
        if t.v > 0.5 && t.v < 0.7 && d < 0.22 {
            c = mix([1.0, 0.86, 0.55], c, d / 0.22 * 0.6);
        }
        c
    })
}

/// The tunic: a leather belt with brass studs on top (`v` under 0.17), then red wool with long
/// folds, a darker hem and a thin brass trim above it.
#[must_use]
pub fn tunic() -> Texels {
    paint(128, 128, |t| {
        if t.v < 0.17 {
            let mut c = scale(LEATHER, 0.8 + 0.4 * noise(t, 2, 2, 128, 7));
            if t.v < 0.025 || t.v > 0.145 {
                c = scale(c, 0.55);
            }
            let d = ((t.u * 16.0).fract() - 0.5).hypot((t.v - 0.085) * 7.0);
            if d < 0.2 {
                c = mix(BRASS, c, d / 0.2 * 0.55);
            }
            return c;
        }
        let fold = 0.88
            + 0.16 * (t.u * std::f32::consts::TAU * 9.0 + noise(t, 16, 64, 128, 5) * 3.0).sin();
        let weave = 0.92 + 0.14 * noise(t, 1, 1, 128, 13);
        let mut c = scale(RED, fold * weave * (1.0 - 0.18 * (t.v - 0.17)));
        if t.v > 0.93 {
            c = scale(c, 0.62);
        } else if t.v > 0.90 {
            c = mix(c, [0.78, 0.62, 0.30], 0.55);
        }
        c
    })
}

/// Sandal leather: tan, mottled, a stitch row every few texels, darker at the edges.
#[must_use]
pub fn leather() -> Texels {
    paint(64, 64, |t| {
        let mut c = scale([0.40, 0.25, 0.13], 0.78 + 0.5 * noise(t, 2, 2, 64, 21));
        c = scale(c, 1.1 - 0.4 * t.v);
        if t.row % 9 == 4 {
            c = mix(c, [0.70, 0.55, 0.34], 0.5);
        }
        if t.u < 0.04 || t.u > 0.96 {
            c = scale(c, 0.6);
        }
        c
    })
}

/// Horsehair for the crest: deep red with fine streaks along the hair.
#[must_use]
pub fn plume() -> Texels {
    paint(64, 32, |t| {
        let streak = 0.7 + 0.5 * noise(t, 1, 8, 64, 31);
        scale([0.62, 0.06, 0.05], streak * (1.1 - 0.5 * t.v))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(map: &Texels, u: f32, v: f32) -> [u8; 3] {
        let col = ((u * count(map.width)) as u32).min(map.width - 1);
        let row = ((v * count(map.height)) as u32).min(map.height - 1);
        let i = ((row * map.width + col) * 4) as usize;
        [map.rgba[i], map.rgba[i + 1], map.rgba[i + 2]]
    }

    fn luma(map: &Texels, u: f32, v: f32) -> f32 {
        let t = rgb(map, u, v);
        (0.2126 * f32::from(t[0]) + 0.7152 * f32::from(t[1]) + 0.0722 * f32::from(t[2])) / 255.0
    }

    #[test]
    fn every_map_is_full_opaque_and_the_same_twice() {
        for (name, make) in [
            ("lorica", lorica as fn() -> Texels),
            ("bronze", bronze),
            ("tunic", tunic),
            ("leather", leather),
            ("plume", plume),
        ] {
            let (a, b) = (make(), make());
            assert_eq!(a, b, "{name} is deterministic");
            let want = usize::try_from(a.width * a.height * 4).expect("size fits");
            assert_eq!(a.rgba.len(), want, "{name} has every texel");
            assert!(
                a.rgba.chunks_exact(4).all(|t| t[3] == 255),
                "{name} is opaque"
            );
        }
    }

    #[test]
    fn the_metals_are_bright_and_the_gaps_between_plates_are_dark() {
        let plate = lorica();
        assert!(luma(&plate, 0.3, 0.45) > 0.35, "steel reads as steel");
        assert!(
            luma(&plate, 0.3, 0.97) < 0.2,
            "the gap under a plate is dark"
        );
        assert!(luma(&bronze(), 0.3, 0.3) > 0.3, "bronze is not black");
        let lace = luma(&plate, 0.5, 0.45);
        assert!(
            lace < luma(&plate, 0.3, 0.45),
            "the lacing is leather, darker than steel"
        );
    }

    #[test]
    fn the_tunic_is_red_under_a_leather_belt() {
        let map = tunic();
        let at = |u: f32, v: f32| rgb(&map, u, v);
        let wool = at(0.31, 0.5);
        assert!(
            wool[0] > 2 * wool[1] && wool[0] > 2 * wool[2],
            "wool is red: {wool:?}"
        );
        let belt = at(0.31, 0.07);
        assert!(
            belt[0] < 120 && belt[0] > belt[2],
            "the belt is brown: {belt:?}"
        );
    }
}
