//! baked_halton: Halton-sampled arena lightmap bake (RL5 bake-once path).
//! Owner: render lane; consumer `crate::baked_gi::sample_arena_frame`.
//! Units: texels, meters, Halton `0..1`. Determinism: fixed Halton taps;
//! same inputs give bit-identical bytes x2. Presentation only.
//! PORT(barnex/scathanna2@186bae1:halton.rs): halton shape, reimplemented.
//! PORT(same rev:integrator.rs): INITIAL_SAMPLES plus num_samples, reimpl.
//! PORT(same rev:sampling.rs): clamped hemisphere rule, reimplemented.
//! PORT(same rev:baking.rs): sun, sky, 2-bounce order, reimplemented.
#![forbid(unsafe_code)]
use crate::baked_bake::{ArenaBake, BakedGiError, ProbeSpec, SealedBox, BAKE_MAX_DIM};
use crate::gi::{BakeScene, Lightmap, ProbeGrid, SurfaceSample};
use std::time::Instant;
/// Initial taps per texel before refine.
pub const INITIAL_SAMPLES: u32 = 10;
/// Most taps any texel may take.
pub const MAX_SAMPLES: u32 = 64;
/// Error at/below this keeps the initial count.
pub const ERROR_TOL: f32 = 0.02;
/// First-bounce strength.
pub const BOUNCE_1: f32 = 0.35;
/// Second-bounce strength.
pub const BOUNCE_2: f32 = 0.12;
/// Radical inverse of `index` in `base`; `halton(0, _)` is `0.0`.
pub fn halton(mut index: u32, base: u32) -> f32 {
    if base < 2 {
        return 0.0;
    }
    let b = f32::from(u16::try_from(base).unwrap_or(2));
    let mut denom = 1.0f32;
    let mut out = 0.0f32;
    while index > 0 {
        denom *= b;
        let digit = u16::try_from(index % base).unwrap_or(0);
        out += f32::from(digit) / denom;
        index /= base;
    }
    out
}

/// Scrambled Halton 2/3 pair in `0..1`.
pub fn halton23_scrambled(index: u32) -> [f32; 2] {
    let x = halton(index.saturating_add(1), 2);
    let y = halton(index.saturating_add(1), 3);
    [
        x + 0.5 - (if x >= 0.5 { 1.0 } else { 0.0 }),
        y + 0.333_333_34 - (if y >= 0.666_666_7 { 1.0 } else { 0.0 }),
    ]
}
/// Cosine-weighted hemisphere dir about `n` from `u1`/`u2` (clamped).
pub fn hemisphere_dir(u1: f32, u2: f32, n: [f32; 3]) -> [f32; 3] {
    let a = u1.clamp(0.0, 1.0);
    let r = a.sqrt();
    let phi = u2.clamp(0.0, 1.0) * 6.283_185_5;
    let (lx, ly) = (r * phi.cos(), r * phi.sin());
    let lz = (1.0 - a).sqrt().max(0.0);
    let h = if n[1].abs() < 0.99 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let t = nz(engine_core::vec3::cross3(h, n));
    let q = engine_core::vec3::cross3(n, t);
    [
        t[0] * lx + q[0] * ly + n[0] * lz,
        t[1] * lx + q[1] * ly + n[1] * lz,
        t[2] * lx + q[2] * ly + n[2] * lz,
    ]
}
/// Error-driven tap count: initial at/below tol, more above.
pub fn num_samples(error: f32) -> u32 {
    if !error.is_finite() || error <= ERROR_TOL {
        INITIAL_SAMPLES
    } else if error <= 0.05 {
        32
    } else {
        MAX_SAMPLES
    }
}
fn nz(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}
/// Incoming radiance from `wd`: sky plus sun (bake-only, no occlusion mesh).
fn incoming(wd: [f32; 3], s: &BakeScene<'_>) -> [f32; 3] {
    let t = (wd[1] * 0.5 + 0.5).clamp(0.0, 1.0);
    let mut r = [
        s.sky.ground[0] + (s.sky.sky[0] - s.sky.ground[0]) * t,
        s.sky.ground[1] + (s.sky.sky[1] - s.sky.ground[1]) * t,
        s.sky.ground[2] + (s.sky.sky[2] - s.sky.ground[2]) * t,
    ];
    for d in s.dirs {
        let l = nz(d.dir);
        let c = (wd[0] * l[0] + wd[1] * l[1] + wd[2] * l[2]).max(0.0);
        r[0] += d.color[0] * c;
        r[1] += d.color[1] * c;
        r[2] += d.color[2] * c;
    }
    r
}
/// One texel: initial taps, spread as error, top-up; direct plus 2 bounces.
fn texel(s: &SurfaceSample, scene: &BakeScene<'_>) -> [f32; 3] {
    let n = nz(s.normal);
    let occ = s.occlusion.clamp(0.0, 1.0);
    let mut acc = [0.0f32; 3];
    let mut wsum = 0.0f32;
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for k in 0..INITIAL_SAMPLES {
        let uv = halton23_scrambled(k);
        let wd = hemisphere_dir(uv[0], uv[1], n);
        let cos = (wd[0] * n[0] + wd[1] * n[1] + wd[2] * n[2]).max(0.0);
        let inc = incoming(wd, scene);
        let lum = inc[0] + inc[1] + inc[2];
        if lum.is_finite() {
            lo = lo.min(lum);
            hi = hi.max(lum);
        }
        acc[0] += inc[0] * cos;
        acc[1] += inc[1] * cos;
        acc[2] += inc[2] * cos;
        wsum += cos;
    }
    let spread = if hi.is_finite() && lo.is_finite() {
        (hi - lo).max(0.0) / (acc[0] + acc[1] + acc[2] + 1e-6).max(1e-6)
    } else {
        0.0
    };
    for k in INITIAL_SAMPLES..num_samples(spread).max(INITIAL_SAMPLES) {
        let uv = halton23_scrambled(k.wrapping_add(7919));
        let wd = hemisphere_dir(uv[0], uv[1], n);
        let cos = (wd[0] * n[0] + wd[1] * n[1] + wd[2] * n[2]).max(0.0);
        let inc = incoming(wd, scene);
        acc[0] += inc[0] * cos;
        acc[1] += inc[1] * cos;
        acc[2] += inc[2] * cos;
        wsum += cos;
    }
    let inv = 1.0 / wsum.max(1e-6);
    let d = [acc[0] * inv, acc[1] * inv, acc[2] * inv];
    let am = (s.albedo[0] + s.albedo[1] + s.albedo[2]) / 3.0;
    let bounce = am * BOUNCE_1 + am * am * BOUNCE_2;
    [
        (s.albedo[0] * d[0] * (1.0 + bounce) * occ).max(0.0),
        (s.albedo[1] * d[1] * (1.0 + bounce) * occ).max(0.0),
        (s.albedo[2] * d[2] * (1.0 + bounce) * occ).max(0.0),
    ]
}
/// Halton bake into `Lightmap`/`ProbeGrid` via `baked_bake` owners; masked
/// texels bake black. SH stays on `ibl`; probes reuse `ProbeGrid::bake`.
pub fn halton_bake(
    w: u32,
    h: u32,
    samples: &[SurfaceSample],
    scene: &BakeScene<'_>,
    probe: &ProbeSpec,
    boxes: &[SealedBox],
) -> Result<(ArenaBake, f64), BakedGiError> {
    if w == 0 || h == 0 {
        return Err(BakedGiError::Invalid("halton bake dims are zero".into()));
    }
    if w > BAKE_MAX_DIM || h > BAKE_MAX_DIM {
        return Err(BakedGiError::Invalid(
            "halton bake dims exceed bound".into(),
        ));
    }
    let want = usize::try_from(w)
        .map_err(|_| BakedGiError::Invalid("halton width exceeds usize".into()))?
        .checked_mul(
            usize::try_from(h)
                .map_err(|_| BakedGiError::Invalid("halton height exceeds usize".into()))?,
        )
        .ok_or_else(|| BakedGiError::Invalid("halton dims overflow".into()))?;
    if samples.len() != want {
        return Err(BakedGiError::Invalid("halton samples mismatch dims".into()));
    }
    probe.validate()?;
    for b in boxes {
        b.validate()?;
    }
    let started = Instant::now();
    let mut tex = Vec::with_capacity(want);
    let mut valid = Vec::with_capacity(want);
    for s in samples {
        if crate::baked_bake::sample_sound(s) {
            let mut t = texel(s, scene);
            for c in &mut t {
                if !c.is_finite() || *c < 0.0 {
                    *c = 0.0;
                }
            }
            tex.push(t);
            valid.push(1u8);
        } else {
            tex.push([0.0, 0.0, 0.0]);
            valid.push(0u8);
        }
    }
    let mut map = Lightmap {
        w,
        h,
        tex,
        idx: vec![0u8; want],
    };
    map.quantize_all();
    let grid = ProbeGrid::bake(probe.nx, probe.ny, probe.nz, probe.min, probe.step, scene);
    let mut sealed = Vec::with_capacity(grid.sh.len());
    for iz in 0..probe.nz {
        for iy in 0..probe.ny {
            for ix in 0..probe.nx {
                let p = grid.probe_pos(
                    usize::try_from(ix).unwrap_or(0),
                    usize::try_from(iy).unwrap_or(0),
                    usize::try_from(iz).unwrap_or(0),
                );
                sealed.push(u8::from(boxes.iter().any(|b| b.contains(p))));
            }
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        ArenaBake {
            map,
            valid,
            probes: grid,
            sealed,
            boxes: boxes.to_vec(),
        },
        ms,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::baked_bake::{arena_fixture_parts, arena_floor_samples, arena_tunnel};
    #[test]
    fn halton_bake_stable_x2_and_samples_read() {
        assert!((halton(1, 2) - 0.5).abs() < 1e-7);
        assert!((halton(2, 2) - 0.25).abs() < 1e-7);
        assert!((halton(3, 2) - 0.75).abs() < 1e-7);
        assert!((halton(1, 3) - (1.0 / 3.0)).abs() < 1e-7);
        let (dirs, points, sky) = arena_fixture_parts();
        let scene = BakeScene {
            dirs: &dirs,
            points: &points,
            sky,
        };
        let spec = ProbeSpec {
            nx: 2,
            ny: 2,
            nz: 2,
            min: [-10.0, 0.25, -10.0],
            step: [20.0, 4.0, 20.0],
        };
        let bake_once =
            || halton_bake(8, 8, &arena_floor_samples(8, 8), &scene, &spec, &[]).expect("bakes");
        let (a, _) = bake_once();
        let (b, _) = bake_once();
        assert_eq!(a.to_bytes(), b.to_bytes(), "halton bytes stable x2");
        assert_eq!(a.digest(), b.digest());
        let good =
            crate::baked_gi::sample_arena_frame(&a, 0.1, 0.1, [8.0, 1.0, -8.0], [0.0, 1.0, 0.0])
                .expect("samples");
        assert!(good.light.is_some());
        assert!(good.irradiance.iter().all(|c| c.is_finite() && *c >= 0.0));
        let mut bad = arena_floor_samples(8, 8);
        bad[0] = SurfaceSample {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 0.0],
            albedo: [0.6, 0.5, 0.36],
            occlusion: 1.0,
        };
        let (c, _) =
            halton_bake(8, 8, &bad, &scene, &spec, &[arena_tunnel()]).expect("masked bakes");
        let masked =
            crate::baked_gi::sample_arena_frame(&c, 0.02, 0.02, [0.0, 1.0, 10.0], [0.0, 1.0, 0.0])
                .expect("masked samples");
        assert_eq!(masked.light, None);
        assert!(masked.irradiance.iter().all(|c| c.is_finite() && *c >= 0.0));
    }
}
