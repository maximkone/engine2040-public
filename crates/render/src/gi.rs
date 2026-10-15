//! gi: baked GI (hemisphere lightmap + SH probes). Bake offline, runtime is lookup-only.

#![forbid(unsafe_code)]

use engine_core::Fnv64;

use std::f32::consts::{FRAC_1_SQRT_2 as SQ, PI};

/// palette size; baked indices are u8.
pub const PALETTE_LEN: usize = 16;
/// warm-gray ramp; every runtime texel is a byte-exact row of this table.
pub const PALETTE: [[u8; 3]; PALETTE_LEN] = [
    [0, 0, 0],
    [18, 13, 11],
    [35, 27, 22],
    [52, 40, 33],
    [70, 54, 44],
    [88, 68, 56],
    [106, 82, 68],
    [124, 96, 80],
    [142, 110, 92],
    [160, 124, 104],
    [178, 138, 116],
    [196, 152, 128],
    [213, 168, 143],
    [228, 186, 161],
    [242, 205, 182],
    [255, 226, 205],
];
/// hemisphere taps per texel (fixed table, no trig).
pub const HEMI_SAMPLES: usize = 9;
/// sphere taps per probe (fixed table, no trig).
pub const SPHERE_SAMPLES: usize = 18;
/// L2 spherical-harmonic coeffs per probe per channel.
pub const SH_COEFFS: usize = 9;
/// runtime op budget per lookup (index+lerp+27-mul, zero alloc).
pub const RUNTIME_OPS_PER_SAMPLE: u32 = 64;

/// fixed hemisphere dirs about +Z (unit, z >= 0.5).
const HEMI: [[f32; 3]; HEMI_SAMPLES] = [
    [0.0, 0.0, 1.0],
    [SQ, 0.0, SQ],
    [-SQ, 0.0, SQ],
    [0.0, SQ, SQ],
    [0.0, -SQ, SQ],
    [0.5, 0.5, SQ],
    [-0.5, 0.5, SQ],
    [0.5, -0.5, SQ],
    [-0.5, -0.5, SQ],
];
/// fixed sphere dirs (6 axes + 12 edge diagonals, all unit).
const SPHERE: [[f32; 3]; SPHERE_SAMPLES] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
    [SQ, SQ, 0.0],
    [SQ, -SQ, 0.0],
    [-SQ, SQ, 0.0],
    [-SQ, -SQ, 0.0],
    [SQ, 0.0, SQ],
    [SQ, 0.0, -SQ],
    [-SQ, 0.0, SQ],
    [-SQ, 0.0, -SQ],
    [0.0, SQ, SQ],
    [0.0, SQ, -SQ],
    [0.0, -SQ, SQ],
    [0.0, -SQ, -SQ],
];

/// directional light (bake input only; `dir` points toward the light).
#[derive(Clone, Copy, Debug)]
pub struct DirLight {
    /// direction to light (normalized at bake).
    pub dir: [f32; 3],
    /// linear color*intensity.
    pub color: [f32; 3],
}
/// point light (bake input only).
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    /// world pos.
    pub pos: [f32; 3],
    /// linear color*intensity.
    pub color: [f32; 3],
    /// 1/range^2 falloff knob.
    pub inv_range_sq: f32,
}
/// sky/ground hemisphere ambient (bake input only, +Y up).
#[derive(Clone, Copy, Debug)]
pub struct SkyAmbient {
    /// zenith color.
    pub sky: [f32; 3],
    /// nadir color.
    pub ground: [f32; 3],
}
/// offline bake scene (never referenced by runtime fns).
pub struct BakeScene<'a> {
    /// directional lights.
    pub dirs: &'a [DirLight],
    /// point lights.
    pub points: &'a [PointLight],
    /// hemisphere ambient.
    pub sky: SkyAmbient,
}
/// per-texel surface input for the lightmap bake.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSample {
    /// world pos.
    pub pos: [f32; 3],
    /// world normal (normalized at bake).
    pub normal: [f32; 3],
    /// linear albedo.
    pub albedo: [f32; 3],
    /// baked AO 0..1 (1 = unoccluded).
    pub occlusion: f32,
}

/// normalize (zero-guarded).
fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}
/// cross product.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// tangent frame about n (deterministic, trig-free).
fn frame(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let h = if n[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = norm(cross(h, n));
    (t, cross(n, t))
}
/// diffuse direct radiance for a facing (bake-only; the lights loop).
fn direct(p: [f32; 3], n: [f32; 3], s: &BakeScene<'_>) -> [f32; 3] {
    let t = (n[1] * 0.5 + 0.5).clamp(0.0, 1.0);
    let mut r = [
        s.sky.ground[0] + (s.sky.sky[0] - s.sky.ground[0]) * t,
        s.sky.ground[1] + (s.sky.sky[1] - s.sky.ground[1]) * t,
        s.sky.ground[2] + (s.sky.sky[2] - s.sky.ground[2]) * t,
    ];
    for d in s.dirs {
        let l = norm(d.dir);
        let c = (n[0] * l[0] + n[1] * l[1] + n[2] * l[2]).max(0.0);
        r[0] += d.color[0] * c;
        r[1] += d.color[1] * c;
        r[2] += d.color[2] * c;
    }
    for q in s.points {
        let lx = q.pos[0] - p[0];
        let ly = q.pos[1] - p[1];
        let lz = q.pos[2] - p[2];
        let d2 = lx * lx + ly * ly + lz * lz;
        let c = ((lx * n[0] + ly * n[1] + lz * n[2]) / (d2 + 1e-6).sqrt()).max(0.0);
        let mut a = (1.0 - d2 * q.inv_range_sq).max(0.0);
        a *= a;
        let g = a * c / (1.0 + d2);
        r[0] += q.color[0] * g;
        r[1] += q.color[1] * g;
        r[2] += q.color[2] * g;
    }
    r
}

/// reinhard HDR->u8 shared by bake+gate (deterministic, saturating).
pub fn tonemap_u8(c: [f32; 3]) -> [u8; 3] {
    let mut o = [0u8; 3];
    for k in 0..3 {
        let x = c[k].clamp(0.0, 4.0);
        o[k] = ((x / (1.0 + x)) * 255.0 + 0.5) as u8;
    }
    o
}
/// nearest palette index (sq distance, ties -> lowest index).
fn nearest(c: [u8; 3]) -> u8 {
    let (mut bi, mut bd) = (0u8, u32::MAX);
    for (i, p) in PALETTE.iter().enumerate() {
        let d = (c[0].abs_diff(p[0]) as u32).pow(2)
            + (c[1].abs_diff(p[1]) as u32).pow(2)
            + (c[2].abs_diff(p[2]) as u32).pow(2);
        if d < bd {
            bd = d;
            bi = i as u8;
        }
    }
    bi
}
/// decode an index (wraps defensively; baked indices never wrap).
pub fn palette_entry(i: u8) -> [u8; 3] {
    PALETTE[(i as usize) % PALETTE_LEN]
}

/// baked hemisphere lightmap: HDR `tex` + runtime palette `idx`.
pub struct Lightmap {
    /// width in texels.
    pub w: u32,
    /// height in texels.
    pub h: u32,
    /// HDR linear texels, row-major.
    pub tex: Vec<[f32; 3]>,
    /// runtime palette indices, row-major (same len as `tex`).
    pub idx: Vec<u8>,
}

impl Lightmap {
    /// offline hemisphere bake: texel = albedo*occlusion*cos-weighted sky+lights.
    pub fn bake(w: u32, h: u32, samples: &[SurfaceSample], scene: &BakeScene<'_>) -> Self {
        assert_eq!((w as usize) * (h as usize), samples.len());
        let mut tex = Vec::with_capacity(samples.len());
        for s in samples {
            let n = norm(s.normal);
            let (t, b) = frame(n);
            let mut acc = [0.0f32; 3];
            let mut wsum = 0.0f32;
            for k in HEMI {
                let wd = [
                    t[0] * k[0] + b[0] * k[1] + n[0] * k[2],
                    t[1] * k[0] + b[1] * k[1] + n[1] * k[2],
                    t[2] * k[0] + b[2] * k[1] + n[2] * k[2],
                ];
                let cos = (wd[0] * n[0] + wd[1] * n[1] + wd[2] * n[2]).max(0.0);
                let inc = direct(s.pos, wd, scene);
                acc[0] += inc[0] * cos;
                acc[1] += inc[1] * cos;
                acc[2] += inc[2] * cos;
                wsum += cos;
            }
            let inv = 1.0 / wsum.max(1e-6);
            let occ = s.occlusion.clamp(0.0, 1.0);
            tex.push([
                s.albedo[0] * acc[0] * inv * occ,
                s.albedo[1] * acc[1] * inv * occ,
                s.albedo[2] * acc[2] * inv * occ,
            ]);
        }
        let mut m = Self {
            w,
            h,
            tex,
            idx: vec![0u8; samples.len()],
        };
        m.quantize_all();
        m
    }
    /// offline bloom-bleed: 3x3 box blur on HDR, then re-quantize.
    /// separable 1x3 + 3x1 with one scratch reused for all passes
    /// (6 taps/texel/pass, no per-pass clone; same 3x3 clamped box).
    pub fn blur_bleed(&mut self, passes: u32) {
        let (w, h) = (self.w as usize, self.h as usize);
        if w == 0 || h == 0 {
            self.quantize_all();
            return;
        }
        let mut tmp = vec![[0.0f32; 3]; self.tex.len()];
        for _ in 0..passes {
            // horizontal 1x3 (clamped) into scratch: row sums, no divide yet.
            for y in 0..h {
                let row = y * w;
                for x in 0..w {
                    let xm = x.saturating_sub(1);
                    let xp = (x + 1).min(w - 1);
                    let a = self.tex[row + xm];
                    let b = self.tex[row + x];
                    let c = self.tex[row + xp];
                    tmp[row + x] = [a[0] + b[0] + c[0], a[1] + b[1] + c[1], a[2] + b[2] + c[2]];
                }
            }
            // vertical 3x1 (clamped) from scratch back into tex, single /9.
            for y in 0..h {
                let ym = y.saturating_sub(1).min(h - 1) * w;
                let mid = y * w;
                let yp = ((y + 1).min(h - 1)) * w;
                for x in 0..w {
                    let a = tmp[ym + x];
                    let b = tmp[mid + x];
                    let c = tmp[yp + x];
                    self.tex[mid + x] = [
                        (a[0] + b[0] + c[0]) / 9.0,
                        (a[1] + b[1] + c[1]) / 9.0,
                        (a[2] + b[2] + c[2]) / 9.0,
                    ];
                }
            }
        }
        self.quantize_all();
    }
    /// snap every texel to its nearest palette index.
    pub fn quantize_all(&mut self) {
        for i in 0..self.tex.len() {
            self.idx[i] = nearest(tonemap_u8(self.tex[i]));
        }
    }
    /// rebake gate digest (dims + HDR bits + indices).
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(&self.w.to_le_bytes());
        hasher.write(&self.h.to_le_bytes());
        for t in &self.tex {
            for c in t {
                hasher.write(&c.to_le_bytes());
            }
        }
        hasher.write(&self.idx);
        hasher.finish()
    }
    /// PALETTE gate: indices in range and 1:1 with texels.
    pub fn is_palette_exact(&self) -> bool {
        self.tex.len() == self.idx.len() && self.idx.iter().all(|&i| (i as usize) < PALETTE_LEN)
    }
    /// runtime nearest fetch (clamped uv; always a byte-exact palette row).
    pub fn sample_nn(&self, u: f32, v: f32) -> [u8; 3] {
        if self.idx.is_empty() || self.w == 0 || self.h == 0 {
            return PALETTE[0];
        }
        let w = self.w as usize;
        let h = self.h as usize;
        let x = ((u.clamp(0.0, 1.0) * w as f32).floor() as usize).min(w - 1);
        let y = ((v.clamp(0.0, 1.0) * h as f32).floor() as usize).min(h - 1);
        palette_entry(self.idx[y * w + x])
    }
    /// runtime nearest fetch as 0..1 floats (same texel as `sample_nn`).
    pub fn sample_nn_f32(&self, u: f32, v: f32) -> [f32; 3] {
        let c = self.sample_nn(u, v);
        [
            c[0] as f32 / 255.0,
            c[1] as f32 / 255.0,
            c[2] as f32 / 255.0,
        ]
    }
}

/// baked SH probe grid (L2, 9 coeffs x RGB per probe).
pub struct ProbeGrid {
    /// probes on x.
    pub nx: u32,
    /// probes on y.
    pub ny: u32,
    /// probes on z.
    pub nz: u32,
    /// grid origin (probe 0,0,0 pos).
    pub min: [f32; 3],
    /// spacing per axis.
    pub step: [f32; 3],
    /// coeffs in (iz*ny+iy)*nx+ix order.
    pub sh: Vec<[[f32; 3]; SH_COEFFS]>,
}

impl ProbeGrid {
    /// offline probe bake: project sphere-sampled radiance onto L2 SH.
    pub fn bake(
        nx: u32,
        ny: u32,
        nz: u32,
        min: [f32; 3],
        step: [f32; 3],
        scene: &BakeScene<'_>,
    ) -> Self {
        let mut sh = Vec::with_capacity((nx as usize) * (ny as usize) * (nz as usize));
        for iz in 0..nz {
            for iy in 0..ny {
                for ix in 0..nx {
                    let p = [
                        min[0] + ix as f32 * step[0],
                        min[1] + iy as f32 * step[1],
                        min[2] + iz as f32 * step[2],
                    ];
                    let mut acc = [[0.0f32; 3]; SH_COEFFS];
                    for d in SPHERE {
                        let rad = direct(p, d, scene);
                        let b = sh_basis(d);
                        for i in 0..SH_COEFFS {
                            acc[i][0] += rad[0] * b[i];
                            acc[i][1] += rad[1] * b[i];
                            acc[i][2] += rad[2] * b[i];
                        }
                    }
                    let k = 4.0 * PI / SPHERE_SAMPLES as f32;
                    for a in acc.iter_mut() {
                        a[0] *= k;
                        a[1] *= k;
                        a[2] *= k;
                    }
                    sh.push(acc);
                }
            }
        }
        Self {
            nx,
            ny,
            nz,
            min,
            step,
            sh,
        }
    }
    /// probe count.
    pub fn probe_count(&self) -> usize {
        self.sh.len()
    }
    /// probe world pos (clamped indices, never panics).
    pub fn probe_pos(&self, ix: usize, iy: usize, iz: usize) -> [f32; 3] {
        let x = ix.min(self.nx.saturating_sub(1) as usize) as f32;
        let y = iy.min(self.ny.saturating_sub(1) as usize) as f32;
        let z = iz.min(self.nz.saturating_sub(1) as usize) as f32;
        [
            self.min[0] + x * self.step[0],
            self.min[1] + y * self.step[1],
            self.min[2] + z * self.step[2],
        ]
    }
    /// rebake gate digest (dims + origin/step bits + SH bits).
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(&self.nx.to_le_bytes());
        hasher.write(&self.ny.to_le_bytes());
        hasher.write(&self.nz.to_le_bytes());
        for c in self.min.iter().chain(self.step.iter()) {
            hasher.write(&c.to_le_bytes());
        }
        for p in &self.sh {
            for c in p {
                for f in c {
                    hasher.write(&f.to_le_bytes());
                }
            }
        }
        hasher.finish()
    }
    /// trilinear probe SH at pos (runtime: 8 fetches + lerp, no alloc).
    pub fn sample_trilerp(&self, pos: [f32; 3]) -> [[f32; 3]; SH_COEFFS] {
        if self.sh.is_empty() || self.nx == 0 || self.ny == 0 || self.nz == 0 {
            return [[0.0; 3]; SH_COEFFS];
        }
        let (nx, ny, nz) = (self.nx as usize, self.ny as usize, self.nz as usize);
        let fx = ((pos[0] - self.min[0]) / self.step[0].max(1e-9)).clamp(0.0, (self.nx - 1) as f32);
        let fy = ((pos[1] - self.min[1]) / self.step[1].max(1e-9)).clamp(0.0, (self.ny - 1) as f32);
        let fz = ((pos[2] - self.min[2]) / self.step[2].max(1e-9)).clamp(0.0, (self.nz - 1) as f32);
        let (x0, y0, z0) = (
            fx.floor() as usize,
            fy.floor() as usize,
            fz.floor() as usize,
        );
        let (x1, y1, z1) = (
            (x0 + 1).min(nx - 1),
            (y0 + 1).min(ny - 1),
            (z0 + 1).min(nz - 1),
        );
        let (tx, ty, tz) = (fx - x0 as f32, fy - y0 as f32, fz - z0 as f32);
        let at = |x: usize, y: usize, z: usize| self.sh[(z * ny + y) * nx + x];
        let mut o = [[0.0f32; 3]; SH_COEFFS];
        for (i, o) in o.iter_mut().enumerate() {
            for (c, o) in o.iter_mut().enumerate() {
                let x00 = at(x0, y0, z0)[i][c] + (at(x1, y0, z0)[i][c] - at(x0, y0, z0)[i][c]) * tx;
                let x10 = at(x0, y1, z0)[i][c] + (at(x1, y1, z0)[i][c] - at(x0, y1, z0)[i][c]) * tx;
                let x01 = at(x0, y0, z1)[i][c] + (at(x1, y0, z1)[i][c] - at(x0, y0, z1)[i][c]) * tx;
                let x11 = at(x0, y1, z1)[i][c] + (at(x1, y1, z1)[i][c] - at(x0, y1, z1)[i][c]) * tx;
                let z0v = x00 + (x10 - x00) * ty;
                let z1v = x01 + (x11 - x01) * ty;
                *o = z0v + (z1v - z0v) * tz;
            }
        }
        o
    }
}

/// L2 SH basis (order: L00, L1y, L10, L11, L2-2, L2-1, L20, L21, L22).
pub fn sh_basis(d: [f32; 3]) -> [f32; SH_COEFFS] {
    let (x, y, z) = (d[0], d[1], d[2]);
    [
        0.282095,
        -0.488603 * y,
        0.488603 * z,
        -0.488603 * x,
        1.092548 * x * y,
        -1.092548 * y * z,
        0.315392 * (3.0 * z * z - 1.0),
        -1.092548 * x * z,
        0.546274 * (x * x - y * y),
    ]
}

/// evaluate SH for a direction (runtime math: 27 mul-add, no alloc).
pub fn sh_irradiance(sh: &[[f32; 3]; SH_COEFFS], d: [f32; 3]) -> [f32; 3] {
    let b = sh_basis(norm(d));
    let mut o = [0.0f32; 3];
    for i in 0..SH_COEFFS {
        o[0] += sh[i][0] * b[i];
        o[1] += sh[i][1] * b[i];
        o[2] += sh[i][2] * b[i];
    }
    o
}

/// joined runtime view (borrows bakes; lookup-only, no scene access).
pub struct GiRuntime<'a> {
    /// baked lightmap.
    pub map: &'a Lightmap,
    /// baked probe grid.
    pub probes: &'a ProbeGrid,
}

impl<'a> GiRuntime<'a> {
    /// palette texel at uv (byte-exact palette row).
    pub fn light(&self, u: f32, v: f32) -> [u8; 3] {
        self.map.sample_nn(u, v)
    }
    /// interpolated SH irradiance at pos for direction d.
    pub fn irradiance(&self, pos: [f32; 3], d: [f32; 3]) -> [f32; 3] {
        sh_irradiance(&self.probes.sample_trilerp(pos), d)
    }
}
