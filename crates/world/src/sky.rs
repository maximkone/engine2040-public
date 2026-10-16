//! sky: seeded gradient sky + grass field. No GPU dep, std-only.
use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME, GOLDEN_GAMMA};
use engine_core::splitmix64;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyParams {
    pub zen: [f32; 3],
    pub hor: [f32; 3],
    pub gnd: [f32; 3],
    pub sun: [f32; 3],
    pub sun_i: f32,
    pub turb: f32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassParams {
    pub count: u32,
    pub h0: f32,
    pub h1: f32,
    pub width: f32,
    pub wamp: f32,
    pub wfreq: f32,
    pub cola: [f32; 3],
    pub colb: [f32; 3],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hero {
    pub seed: u64,
    pub sky: SkyParams,
    pub grass: GrassParams,
}
fn mx(s: u64, salt: u64) -> u64 {
    splitmix64(s ^ splitmix64(salt.wrapping_mul(GOLDEN_GAMMA).wrapping_add(0x85ebca6b)))
}
fn f01(h: u64) -> f32 {
    ((h >> 11) as f64 / 9007199254740992.0) as f32
}
fn fr(h: u64, a: f32, b: f32) -> f32 {
    a + (b - a) * f01(h)
}
/// Word-wise FNV step (xor a whole word, then multiply), not byte-wise FNV:
/// hero hashes are pinned to this fold.
fn fold(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(FNV64_PRIME)
}
fn fh(h: u64, f: f32) -> u64 {
    fold(h, u64::from(f.to_bits()))
}
fn fh3(mut h: u64, c: [f32; 3]) -> u64 {
    for v in c {
        h = fh(h, v);
    }
    h
}
/// Seeded hero trio. Same seed => identical bits.
pub fn hero(seed: u64) -> Hero {
    let az = fr(mx(seed, 11), 0.0, 6.2831855);
    let el = fr(mx(seed, 12), 0.15, 1.2);
    let (sa, ca) = az.sin_cos();
    let (se, ce) = el.sin_cos();
    let sky = SkyParams {
        zen: [
            fr(mx(seed, 21), 0.05, 0.20),
            fr(mx(seed, 22), 0.25, 0.50),
            fr(mx(seed, 23), 0.60, 0.92),
        ],
        hor: [
            fr(mx(seed, 24), 0.45, 0.80),
            fr(mx(seed, 25), 0.55, 0.80),
            fr(mx(seed, 26), 0.65, 0.90),
        ],
        gnd: [
            fr(mx(seed, 27), 0.10, 0.25),
            fr(mx(seed, 28), 0.10, 0.22),
            fr(mx(seed, 29), 0.08, 0.18),
        ],
        sun: [ca * ce, se, sa * ce],
        sun_i: fr(mx(seed, 30), 2.0, 6.0),
        turb: fr(mx(seed, 31), 1.5, 4.0),
    };
    let grass = GrassParams {
        count: 2048 + (mx(seed, 70) % 4096) as u32,
        h0: fr(mx(seed, 71), 0.35, 0.55),
        h1: fr(mx(seed, 72), 0.8, 1.3),
        width: fr(mx(seed, 73), 0.04, 0.09),
        wamp: fr(mx(seed, 74), 0.05, 0.25),
        wfreq: fr(mx(seed, 75), 0.8, 2.5),
        cola: [
            fr(mx(seed, 76), 0.08, 0.18),
            fr(mx(seed, 77), 0.30, 0.45),
            fr(mx(seed, 78), 0.08, 0.16),
        ],
        colb: [
            fr(mx(seed, 79), 0.35, 0.55),
            fr(mx(seed, 80), 0.55, 0.75),
            fr(mx(seed, 81), 0.20, 0.35),
        ],
    };
    Hero { seed, sky, grass }
}
/// FNV-1a64 over seed + all param bits. Accept gate: hero params hash.
pub fn hero_hash(h: &Hero) -> u64 {
    let mut x: u64 = FNV64_OFFSET;
    x = fold(x, h.seed);
    x = fh3(x, h.sky.zen);
    x = fh3(x, h.sky.hor);
    x = fh3(x, h.sky.gnd);
    x = fh3(x, h.sky.sun);
    x = fh(x, h.sky.sun_i);
    x = fh(x, h.sky.turb);
    x = fold(x, u64::from(h.grass.count));
    x = fh(x, h.grass.h0);
    x = fh(x, h.grass.h1);
    x = fh(x, h.grass.width);
    x = fh(x, h.grass.wamp);
    x = fh(x, h.grass.wfreq);
    x = fh3(x, h.grass.cola);
    x = fh3(x, h.grass.colb);
    x
}
/// Analytic sky gradient + sun glow. Mirrors render SKY_WGSL.
pub fn sky_color(s: &SkyParams, y: f32) -> [f32; 3] {
    let mut c = if y >= 0.0 {
        let t = y.clamp(0.0, 1.0);
        [
            s.hor[0] + (s.zen[0] - s.hor[0]) * t,
            s.hor[1] + (s.zen[1] - s.hor[1]) * t,
            s.hor[2] + (s.zen[2] - s.hor[2]) * t,
        ]
    } else {
        let t = (-y * 3.0).clamp(0.0, 1.0);
        [
            s.hor[0] + (s.gnd[0] - s.hor[0]) * t,
            s.hor[1] + (s.gnd[1] - s.hor[1]) * t,
            s.hor[2] + (s.gnd[2] - s.hor[2]) * t,
        ]
    };
    let d = (y - s.sun[1]).abs();
    let g = s.sun_i * 0.05 / (1.0 + 60.0 * d * d);
    c[0] += g;
    c[1] += g;
    c[2] += g;
    c
}
/// Grass blade i: unit-field pos, height, phase. Mirrors GRASS_WGSL sway input.
pub fn grass_blade(g: &GrassParams, seed: u64, i: u32) -> ([f32; 2], f32, f32) {
    let p = [
        f01(mx(seed, 1000 + u64::from(i))),
        f01(mx(seed, 2000 + u64::from(i))),
    ];
    let h = g.h0 + (g.h1 - g.h0) * f01(mx(seed, 3000 + u64::from(i)));
    let ph = f01(mx(seed, 4000 + u64::from(i))) * 6.2831855;
    (p, h, ph)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hero_hash_x2() {
        assert_eq!(hero_hash(&hero(7)), hero_hash(&hero(7)));
        assert_ne!(hero_hash(&hero(7)), hero_hash(&hero(8)));
    }
    #[test]
    fn hero_bits_identical() {
        assert_eq!(hero(2040), hero(2040));
    }
    #[test]
    fn sky_finite_unit() {
        let s = hero(7).sky;
        for y in [-1.0, -0.2, 0.0, 0.5, 1.0] {
            let c = sky_color(&s, y);
            assert!(c.iter().all(|v| v.is_finite() && *v >= -0.1 && *v < 8.0));
        }
    }
    #[test]
    fn grass_det_field() {
        let h = hero(11);
        let (a, ha, _) = grass_blade(&h.grass, h.seed, 3);
        let (b, hb, _) = grass_blade(&h.grass, h.seed, 3);
        assert_eq!((a, ha), (b, hb));
        assert!(
            (h.grass.count >= 2048)
                && (h.grass.count < 6144)
                && (ha >= h.grass.h0)
                && (ha <= h.grass.h1)
        );
        assert!(a[0] >= 0.0 && a[0] < 1.0);
    }
}
