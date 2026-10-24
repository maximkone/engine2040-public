//! shadow_post: CSM params + ACES tonemap + Sunset/Noir presets. Data-first, std-only.

pub const MAX_CASCADES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CsmConfig {
    pub cascades: u8,
    pub map_size: u32,
    pub lambda: f32,
    pub near_m: f32,
    pub far_m: f32,
    pub bias: f32,
    pub normal_bias_m: f32,
    pub blend_frac: f32,
    pub snap_texels: bool,
}
impl Default for CsmConfig {
    fn default() -> Self {
        Self {
            cascades: 3,
            map_size: 2048,
            lambda: 0.75,
            near_m: 1.0,
            far_m: 120.0,
            bias: 0.0006,
            normal_bias_m: 0.025,
            blend_frac: 0.08,
            snap_texels: true,
        }
    }
}
impl CsmConfig {
    pub fn validate(&self) -> bool {
        (1..=MAX_CASCADES as u8).contains(&self.cascades)
            && self.map_size >= 512
            && self.map_size <= 4096
            && self.map_size.is_power_of_two()
            && (0.0..=1.0).contains(&self.lambda)
            && self.near_m > 0.0
            && self.far_m > self.near_m
            && self.bias >= 0.0
            && self.bias < 0.05
            && self.normal_bias_m >= 0.0
            && self.normal_bias_m < 1.0
            && (0.0..=0.5).contains(&self.blend_frac)
    }
    pub fn splits(&self) -> [f32; MAX_CASCADES] {
        cascade_splits(self.near_m, self.far_m, self.cascades, self.lambda)
    }
}

/// Practical split scheme (Persson/three-csm): mix of log + uniform by lambda.
/// Unused lanes hold `far`. Deterministic pure fn.
pub fn cascade_splits(near: f32, far: f32, n: u8, lambda: f32) -> [f32; MAX_CASCADES] {
    let mut out = [far; MAX_CASCADES];
    let n = n.clamp(1, MAX_CASCADES as u8) as usize;
    let l = lambda.clamp(0.0, 1.0);
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        *slot = crate::shadow_cluster::practical_split(near, far, (i + 1) as f32 / n as f32, l);
    }
    for i in 0..n {
        let lo = if i == 0 { near } else { out[i - 1] };
        if out[i] <= lo {
            out[i] = lo + 0.001;
        }
    }
    out[n - 1] = far;
    out
}

/// Texel snap for CSM stability: jitter < texel keeps origin fixed.
pub fn snap_texel(v: f32, texel: f32) -> f32 {
    if texel <= 0.0 || !texel.is_finite() || !v.is_finite() {
        return v;
    }
    (v / texel).floor() * texel
}
pub fn cascade_select(depth: f32, splits: [f32; MAX_CASCADES], n: u8) -> usize {
    let n = n.clamp(1, MAX_CASCADES as u8) as usize;
    for (i, s) in splits.iter().enumerate().take(n) {
        if depth <= *s {
            return i;
        }
    }
    n - 1
}
/// Fade to next cascade at split boundary: 0 = solid, 1 = fully next.
pub fn cascade_fade(depth: f32, splits: [f32; MAX_CASCADES], n: u8, blend: f32) -> (usize, f32) {
    let idx = cascade_select(depth, splits, n);
    let n = n.clamp(1, MAX_CASCADES as u8) as usize;
    if blend <= 0.0 || idx + 1 >= n {
        return (idx, 0.0);
    }
    let band = (splits[idx] * blend).max(1e-4);
    let d = splits[idx] - depth;
    if d >= band {
        (idx, 0.0)
    } else {
        (idx, 1.0 - d / band)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcesConfig {
    pub exposure_ev: f32,
    pub white: f32,
}
impl Default for AcesConfig {
    fn default() -> Self {
        Self {
            exposure_ev: 0.0,
            white: 11.2,
        }
    }
}
pub fn ev_mult(ev: f32) -> f32 {
    2f32.powf(ev)
}
/// Narkowicz ACES fit (filament): cheap, monotonic, 0->0, 1->~0.804, big->1.
/// One owner: `post::aces_fitted` (this path re-exports it). It is a pointer
/// to the one source's fit, not a second curve.
pub use crate::post::aces_fitted;
/// The shadow path's tone map: the crate's one hue-keeping curve
/// (`crate::tonemap::tone_map`, the CPU twin of the WGSL `tonemap.wgsl`
/// chunk). One scale for every channel, so the input ratios - the hue -
/// survive; the per-channel fit this replaced shifted a torch flame's hue
/// by +32.3 degrees.
pub fn tonemap_aces(c: [f32; 3], cfg: &AcesConfig) -> [f32; 3] {
    let m = ev_mult(cfg.exposure_ev);
    let exposed = [c[0] * m, c[1] * m, c[2] * m];
    crate::tonemap::tone_map(exposed, 1.0, cfg.white)
}
pub fn luma_srgb(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
pub fn desat3(c: [f32; 3], amt: f32) -> [f32; 3] {
    let a = amt.clamp(0.0, 1.0);
    let l = luma_srgb(c);
    [
        c[0] + (l - c[0]) * a,
        c[1] + (l - c[1]) * a,
        c[2] + (l - c[2]) * a,
    ]
}
pub fn linear_to_srgb(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}
pub fn srgb_u8(x: f32) -> u8 {
    (linear_to_srgb(x) * 255.0).round().clamp(0.0, 255.0) as u8
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub name: &'static str,
    pub csm: CsmConfig,
    pub aces: AcesConfig,
    pub sun_dir: [f32; 3],
    pub sun_color: [f32; 3],
    pub ambient: [f32; 3],
    pub desat: f32,
}
impl Look {
    pub fn sunset() -> Self {
        Self {
            name: "Sunset",
            csm: CsmConfig {
                lambda: 0.65,
                ..Default::default()
            },
            aces: AcesConfig {
                exposure_ev: 0.35,
                white: 11.2,
            },
            sun_dir: [-0.55, 0.28, 0.79],
            sun_color: [1.0, 0.52, 0.26],
            ambient: [0.16, 0.13, 0.16],
            desat: 0.0,
        }
    }
    pub fn noir() -> Self {
        Self {
            name: "Noir",
            csm: CsmConfig {
                lambda: 0.85,
                blend_frac: 0.05,
                ..Default::default()
            },
            aces: AcesConfig {
                exposure_ev: -0.15,
                white: 11.2,
            },
            sun_dir: [0.2, 0.85, 0.48],
            sun_color: [0.95, 0.96, 1.0],
            ambient: [0.05, 0.05, 0.06],
            desat: 1.0,
        }
    }
    pub fn preset(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("sunset") {
            Some(Self::sunset())
        } else if name.eq_ignore_ascii_case("noir") {
            Some(Self::noir())
        } else {
            None
        }
    }
}

/// Deterministic headless "shot": vertical depth ramp picks cascade (shade),
/// horizontal+vertical gradient lit by warm/cold sun, then ACES+desat+sRGB.
/// CSM params and look both feed pixels, so shot hash proves CSM-stable + ACES.
pub fn shot_rgb8(look: &Look, w: usize, h: usize) -> Vec<u8> {
    let (w, h) = (w.max(1), h.max(1));
    let splits = look.csm.splits();
    let elev = look.sun_dir[1].clamp(0.0, 1.0);
    let sun_k = 0.35 + 0.65 * elev;
    let mut out = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        let v = if h > 1 {
            y as f32 / (h - 1) as f32
        } else {
            0.0
        };
        let depth = look.csm.near_m + (look.csm.far_m - look.csm.near_m) * v;
        let shade = 1.0 - 0.15 * cascade_select(depth, splits, look.csm.cascades) as f32;
        for x in 0..w {
            let u = if w > 1 {
                x as f32 / (w - 1) as f32
            } else {
                0.0
            };
            let t = (u * 0.6 + v * 0.4) * sun_k * shade;
            let lin = [
                look.ambient[0] + look.sun_color[0] * t,
                look.ambient[1] + look.sun_color[1] * t,
                look.ambient[2] + look.sun_color[2] * t,
            ];
            let ds = desat3(tonemap_aces(lin, &look.aces), look.desat);
            out.push(srgb_u8(ds[0]));
            out.push(srgb_u8(ds[1]));
            out.push(srgb_u8(ds[2]));
        }
    }
    out
}
pub use crate::types::fnv1a64;
fn mx(h: u64, v: u32) -> u64 {
    (h ^ v as u64).wrapping_mul(0x100000001b3)
}
fn mf(h: u64, v: f32) -> u64 {
    mx(h, v.to_bits())
}
pub fn config_hash(l: &Look) -> u64 {
    let mut h = fnv1a64(l.name.as_bytes());
    h = mx(h, l.csm.cascades as u32);
    h = mx(h, l.csm.map_size);
    h = mf(h, l.csm.lambda);
    h = mf(h, l.csm.near_m);
    h = mf(h, l.csm.far_m);
    h = mf(h, l.csm.bias);
    h = mf(h, l.csm.normal_bias_m);
    h = mf(h, l.csm.blend_frac);
    h = mx(h, l.csm.snap_texels as u32);
    h = mf(h, l.aces.exposure_ev);
    h = mf(h, l.aces.white);
    for v in l.sun_dir {
        h = mf(h, v);
    }
    for v in l.sun_color {
        h = mf(h, v);
    }
    for v in l.ambient {
        h = mf(h, v);
    }
    h = mf(h, l.desat);
    h
}
pub fn shot_hash(l: &Look, w: usize, h: usize) -> u64 {
    fnv1a64(&shot_rgb8(l, w, h))
}
