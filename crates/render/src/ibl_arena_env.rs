//! ibl_arena_env: the arena's own day sky as the live image-based light.
//!
//! Owner: render lane. Consumer: the window presenter's one-time IBL bake
//! (`presenter.rs`), through [`bake_arena_day`]. Until now the live bake read a
//! photo-studio HDRI (`polyhaven-studio-small-09-1k.hdr`: octagonal softboxes,
//! a white cyclorama, a black ceiling). The sand and the stands reflect their
//! environment through the prefiltered specular cube, so the arena floor
//! showed the studio: glowing white vertical streaks (the softboxes, sheared
//! over the normal-mapped sand) and hazy white patches with straight edges (the
//! cyclorama and its panels). The sky the arena is lit by already exists
//! ([`crate::sky::sky_radiance`] under [`crate::arena_day::arena_day_params`]);
//! this module samples it into the equirectangular [`EnvMap`] the bake takes.
//! Units: linear HDR radiance, equirectangular texels. Determinism: the same
//! parameters give the same map and the same bake twice; presentation only,
//! never the sim hash. Errors: typed `IblError`, loud. Budget: 8192 sky
//! samples at init (well under a millisecond), then the usual bake.

use crate::arena_day::{arena_day_params, arena_params_at};
use crate::ibl::{bake_env, project_sh, IblBake, IblError, IrradianceSh};
use crate::sky::{sky_radiance, SkyParams};
use crate::v1_ibl_shadow::{uv_to_dir, EnvMap};

/// Environment width in texels: the sky is smooth, 128 columns hold it.
pub const ARENA_ENV_W: u32 = 128;
/// Environment height in texels.
pub const ARENA_ENV_H: u32 = 64;
/// Largest sky radiance one texel keeps before the gain, linear: the sky near
/// the sun runs far past it (the solar disc), and the sun is the forward
/// shader's direct light, not a hot spot to reflect twice.
pub const ARENA_ENV_MAX: f32 = 1.5;
/// Scale on the sampled sky per channel, linear. Calibrated per channel so a
/// surface facing up takes the same ambient the studio bake gave it (the look's
/// exposure and the sand's warm tan were tuned against that): the raw sky is
/// blue overhead and left the sand grey, the tint keeps it tan. See the test
/// `the_arena_env_keeps_the_up_facing_ambient`.
pub const ARENA_ENV_GAIN: [f32; 3] = [3.75, 2.93, 1.86];
/// Radiance of the sand under the horizon, linear: the sunlit floor bounces
/// light back up, so a wall, a stand or a fighter's flank is lit from below
/// with the floor's own warm tan (the raw sky model leaves the ground nearly
/// black, which halved the ambient on every vertical surface against the
/// studio). Blended in over the 0.3 of elevation under the horizon, like the
/// sky model's own limb.
pub const ARENA_GROUND_BOUNCE: [f32; 3] = [0.62, 0.45, 0.28];
/// How far under the horizon (as the sine of the elevation) the sand takes
/// over from the sky model completely.
pub const ARENA_GROUND_BLEND: f32 = 0.3;

/// The arena day sky as an equirectangular environment: every texel is the
/// day-rig sky radiance along its direction (horizon haze included, the sun
/// disc clamped away at [`ARENA_ENV_MAX`]), scaled by [`ARENA_ENV_GAIN`], with
/// the sand's own bounce light under the horizon ([`ARENA_GROUND_BOUNCE`]).
/// Pure; the same call twice gives identical texels.
pub fn arena_day_env() -> Result<EnvMap, IblError> {
    arena_env(&arena_day_params(), ARENA_GROUND_BOUNCE)
}

/// The arena night sky as an equirectangular environment: the same env map
/// over the moonlit sky params, with the dark sand's own bounce light under
/// the horizon. Pure; the same call twice gives identical texels.
pub fn arena_night_env() -> Result<EnvMap, IblError> {
    arena_env(
        &crate::arena_day::arena_night_params(),
        ARENA_NIGHT_GROUND_BOUNCE,
    )
}

/// The night sand's own bounce light under the horizon: dark and cool, so
/// the warm torches read against it.
pub const ARENA_NIGHT_GROUND_BOUNCE: [f32; 3] = [0.16, 0.13, 0.10];

/// One arena sky as an equirectangular environment: every texel is the sky
/// radiance along its direction (horizon haze included, the sun disc clamped
/// away at [`ARENA_ENV_MAX`]), scaled by [`ARENA_ENV_GAIN`], with the
/// ground's own bounce light under the horizon. Pure; the same call twice
/// gives identical texels.
pub fn arena_env(params: &SkyParams, ground_bounce: [f32; 3]) -> Result<EnvMap, IblError> {
    arena_env_gain(params, ground_bounce, ARENA_ENV_GAIN)
}

/// [`arena_env`] with the sky's per-channel gain named by the caller.
pub fn arena_env_gain(
    params: &SkyParams,
    ground_bounce: [f32; 3],
    gain: [f32; 3],
) -> Result<EnvMap, IblError> {
    let (w, h) = (ARENA_ENV_W, ARENA_ENV_H);
    // A low sun lights the sky less (1.0 for the rig's own sun and every
    // higher one, and for the night family).
    let daylight = crate::arena_day::daylight_scale(params);
    let mut pixels = Vec::with_capacity(usize::try_from(w * h).unwrap_or(0));
    // Texel centers over the (small, `u16`-sized) map edges.
    let edge = |n: u32| f32::from(u16::try_from(n).unwrap_or(u16::MAX));
    for y in 0..h {
        for x in 0..w {
            let uv = [(edge(x) + 0.5) / edge(w), (edge(y) + 0.5) / edge(h)];
            let dir = uv_to_dir(uv);
            let radiance = sky_radiance(dir, params)
                .map_err(|e| IblError::Invalid(format!("arena env: {e}")))?;
            // 0 on and above the horizon, 1 once the sand fully takes over.
            let ground = (-dir[1] / ARENA_GROUND_BLEND).clamp(0.0, 1.0);
            let mut texel = [0.0f32; 3];
            for c in 0..3 {
                let sky = radiance[c].min(ARENA_ENV_MAX) * gain[c];
                texel[c] = (sky + (ground_bounce[c] - sky) * ground) * daylight;
            }
            pixels.push(texel);
        }
    }
    Ok(EnvMap {
        width: w,
        height: h,
        pixels,
    })
}

/// The live IBL bake from the arena day sky: what the presenter uploads for
/// the sand, the stands and the armor to reflect. Returns the bake plus its
/// wall ms (evidence only).
pub fn bake_arena_day() -> Result<(IblBake, f64), IblError> {
    let started = engine_clock::now();
    let env = arena_day_env()?;
    let (bake, _) = bake_env(&env)?;
    let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    Ok((bake, ms))
}

/// The live IBL bake from the arena night sky: the moonlit environment the
/// presenter uploads under the night preset, so the ambient sits far under
/// the day level and the torches carry the picture. Returns the bake plus
/// its wall ms (evidence only).
pub fn bake_arena_night() -> Result<(IblBake, f64), IblError> {
    let started = engine_clock::now();
    let env = arena_night_env()?;
    let (bake, _) = bake_env(&env)?;
    let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    Ok((bake, ms))
}

/// The real Colosseum morning sky (Rome, Poly Haven `colosseum_2k`, CC0),
/// the outdoor day probe for the establishing view. `content/arena/hdri/
/// arena_rome_2k.hdr`, 2048x1024, 6,090,156 bytes, sha256 in the HDRI
/// PROVENANCE row. The 1k `arena_day_1k.hdr` stays the potato fallback.
pub const ARENA_ROME_HDRI: &[u8] = include_bytes!("../../../content/arena/hdri/arena_rome_2k.hdr");

/// The arena's sky slots: the analytic day and night skies the ring is tuned
/// for, plus the real Rome HDRI the establishing view can pick.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ArenaSky {
    /// Analytic arena day (the duel's tuned ambient).
    #[default]
    Day,
    /// Analytic arena night (torches carry the picture).
    Night,
    /// The Poly Haven Colosseum morning HDRI, `arena_rome`.
    Rome,
}

impl ArenaSky {
    /// Parse a `--sky` / env name; an unknown name is `None`, never a guess.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "day" => Some(Self::Day),
            "night" => Some(Self::Night),
            "arena_rome" | "rome" => Some(Self::Rome),
            _ => None,
        }
    }
}

/// The env map for one slot: the Rome HDRI decodes straight to an [`EnvMap`],
/// the two analytic slots keep their sky models.
pub fn arena_env_for(sky: ArenaSky) -> Result<EnvMap, IblError> {
    match sky {
        ArenaSky::Day => arena_day_env(),
        ArenaSky::Night => arena_night_env(),
        ArenaSky::Rome => crate::v1_ibl_shadow::load_hdri(ARENA_ROME_HDRI).map_err(IblError::from),
    }
}

/// The live bake for one slot. Caller: the establishing view picks
/// [`ArenaSky::Rome`] when `arena_rome` is set.
pub fn bake_arena_sky(sky: ArenaSky) -> Result<(IblBake, f64), IblError> {
    let started = engine_clock::now();
    let env = arena_env_for(sky)?;
    let (bake, _) = bake_env(&env)?;
    let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    Ok((bake, ms))
}

/// Sun elevations of the five ambient keyframes, degrees: first light, early
/// morning, the day rig's own afternoon, high sun, near noon. The ambient of
/// any sun height between them is the lerp of the two neighbours' SH.
pub const SKY_KEY_ELEVATIONS_DEG: [f32; 5] = [5.0, 14.0, 28.0, 48.0, 72.0];

/// One time-of-day keyframe: the sky at one sun height as irradiance SH plus
/// the direct sun's colour there.
#[derive(Clone, PartialEq, Debug)]
pub struct SkyKeyframe {
    /// Sun elevation of this keyframe, degrees.
    pub elevation_deg: f32,
    /// Cosine-convolved irradiance of the arena env at that sun height.
    pub sh: IrradianceSh,
    /// Direct sun colour at that height ([`crate::sky::sun_color`]).
    pub sun_color: [f32; 3],
}

impl SkyKeyframe {
    fn write_bytes(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.elevation_deg.to_le_bytes());
        out.extend_from_slice(&self.sh.to_bytes());
        for c in self.sun_color {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
}

/// The five keyframes of the arena day, baked once at init from the same sky
/// model the skybox and the fog read, so the ambient follows the sun the sky
/// shows (RL40).
#[derive(Clone, PartialEq, Debug)]
pub struct SkyKeys {
    frames: Vec<SkyKeyframe>,
}

/// Bake the five ambient keyframes: each is [`arena_env`] under the day rig at
/// one sun height, projected to SH. Pure; the same call twice gives identical
/// bytes. Budget: 5 x 8192 sky samples, a few ms at init, never per frame.
pub fn bake_sky_keys() -> Result<SkyKeys, IblError> {
    let mut frames = Vec::with_capacity(SKY_KEY_ELEVATIONS_DEG.len());
    for elevation_deg in SKY_KEY_ELEVATIONS_DEG {
        let params = arena_params_at(elevation_deg);
        let env = arena_env(&params, ARENA_GROUND_BOUNCE)?;
        let sun_color = crate::sky::sun_color(&params)
            .map_err(|e| IblError::Invalid(format!("arena keyframe sun: {e}")))?;
        frames.push(SkyKeyframe {
            elevation_deg,
            sh: project_sh(&env),
            sun_color,
        });
    }
    Ok(SkyKeys { frames })
}

impl SkyKeys {
    /// The keyframes, lowest sun first.
    pub fn frames(&self) -> &[SkyKeyframe] {
        &self.frames
    }

    /// The ambient and sun colour at `elevation_deg`: a keyframe exactly on
    /// one, the lerp of its two neighbours between two, the end keyframe past
    /// either end. Pure, no allocation beyond the returned value.
    pub fn at(&self, elevation_deg: f32) -> SkyKeyframe {
        let (first, last) = (&self.frames[0], &self.frames[self.frames.len() - 1]);
        if elevation_deg.is_nan() || elevation_deg <= first.elevation_deg {
            return first.clone();
        }
        if elevation_deg >= last.elevation_deg {
            return last.clone();
        }
        for pair in self.frames.windows(2) {
            let (lo, hi) = (&pair[0], &pair[1]);
            if elevation_deg <= hi.elevation_deg {
                let t = (elevation_deg - lo.elevation_deg) / (hi.elevation_deg - lo.elevation_deg);
                // `a * (1 - t) + b * t` is exactly `a` at 0 and `b` at 1.
                let mix = |a: f32, b: f32| a * (1.0 - t) + b * t;
                let mut sh = IrradianceSh::zero();
                for (i, out) in sh.coeffs.iter_mut().enumerate() {
                    for (c, out_c) in out.iter_mut().enumerate() {
                        *out_c = mix(lo.sh.coeffs[i][c], hi.sh.coeffs[i][c]);
                    }
                }
                let sun_color = [0, 1, 2].map(|c| mix(lo.sun_color[c], hi.sun_color[c]));
                return SkyKeyframe {
                    elevation_deg,
                    sh,
                    sun_color,
                };
            }
        }
        last.clone()
    }

    /// Deterministic little-endian bytes of every keyframe, lowest sun first.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for frame in &self.frames {
            frame.write_bytes(&mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ibl::{bake, eval_sh, IBL_FACES, IBL_LEVELS};

    const STUDIO: &[u8] =
        include_bytes!("../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

    fn luma(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    /// SW120: the arena's Rome slot is the real Colosseum morning sky. The
    /// bytes are pinned here by length and FNV-64 over the whole file, and the
    /// HDRI PROVENANCE row carries the same byte count plus the sha256 the
    /// fetch computed, so a re-cooked or truncated file fails before it draws.
    #[test]
    fn rome_hdri_parses_to_2048x1024_and_its_sha_matches_the_row() {
        let env = crate::v1_ibl_shadow::load_hdri(ARENA_ROME_HDRI).expect("the Rome HDRI decodes");
        assert_eq!((env.width, env.height), (2048, 1024), "2k equirect");
        assert_eq!(ARENA_ROME_HDRI.len(), 6_090_156, "bytes match the row");
        assert_eq!(
            engine_core::hash_bytes(ARENA_ROME_HDRI),
            0x9aeb_2bed_543b_03ed,
            "every byte of the shipped HDRI is the one the cook checked"
        );
        let row: &str = include_str!("../../../content/arena/hdri/PROVENANCE.md");
        assert!(row.contains("6090156"), "the row records the byte count");
        let sha = "c5db7e22eec13ea08c089605727e0f4ad2e4c7b4ae74d78b48f00594c9b26eda";
        assert!(row.contains(sha), "the row records the fetch sha256");
        assert_eq!(ArenaSky::parse("arena_rome"), Some(ArenaSky::Rome));
        assert_eq!(ArenaSky::parse("rome"), Some(ArenaSky::Rome));
        assert_eq!(ArenaSky::parse("hoth"), None);
        // The slot bakes like the analytic skies do (the establishing view's
        // one job for it), and the day slot is untouched by its arrival.
        let (rome, _) = bake_arena_sky(ArenaSky::Rome).expect("Rome bakes");
        assert!(rome.cube.levels > 0, "the bake has mip levels");
        assert_eq!(
            arena_env_for(ArenaSky::Day).expect("day samples"),
            arena_day_env().expect("day samples"),
            "the day slot still comes from the analytic sky"
        );
    }

    /// Brightest texel over mean texel of one prefilter level (luma): how
    /// "spotty" the reflection a surface of that roughness sees is.
    fn peak_over_mean(bake: &IblBake, level: usize) -> f32 {
        let per_face = bake.cube.size as usize * bake.cube.size as usize;
        let from = level * IBL_FACES * per_face;
        let texels = &bake.cube.faces[from..from + IBL_FACES * per_face];
        let (mut peak, mut sum) = (0.0f32, 0.0f32);
        for texel in texels {
            let l = luma(*texel);
            peak = peak.max(l);
            sum += l;
        }
        peak / (sum / texels.len() as f32).max(1e-6)
    }

    #[test]
    fn the_arena_env_is_deterministic_finite_and_sky_shaped() {
        let a = arena_day_env().expect("sky samples");
        let b = arena_day_env().expect("sky samples twice");
        assert_eq!(a, b, "same call, same texels");
        assert_eq!((a.width, a.height), (ARENA_ENV_W, ARENA_ENV_H));
        assert!(a
            .pixels
            .iter()
            .flatten()
            .all(|c| c.is_finite() && *c >= 0.0));
        let top = ARENA_ENV_GAIN
            .iter()
            .fold(0.0f32, |top, gain| top.max(ARENA_ENV_MAX * gain));
        assert!(
            a.pixels.iter().flatten().all(|c| *c <= top),
            "no texel past the clamp (the sun is the direct light)"
        );
        // Row 0 is the zenith (blue), the middle row the bright horizon, the
        // last row the dark sand ground.
        let row = |y: u32| {
            a.pixels[(y * ARENA_ENV_W) as usize..][..ARENA_ENV_W as usize]
                .iter()
                .map(|c| luma(*c))
                .sum::<f32>()
                / ARENA_ENV_W as f32
        };
        assert!(row(0) > 0.01 && row(ARENA_ENV_H - 1) > 0.0);
        assert!(
            row(ARENA_ENV_H / 2 - 2) > row(ARENA_ENV_H - 1),
            "sky brighter than ground"
        );
    }

    #[test]
    fn the_arena_env_has_no_studio_lights_for_a_rough_surface_to_reflect() {
        // The 2026-10-18 live arena: white streaks and straight-edged white
        // patches on the sand were the studio's softboxes and cyclorama in
        // the prefiltered specular. At the rough levels the sand, the stands
        // and the stone sample, the arena sky must be a smooth wash.
        let (arena, _) = bake_arena_day().expect("arena day bakes");
        let (studio, _) = bake(STUDIO).expect("studio bakes");
        for level in [2, 3, IBL_LEVELS - 1] {
            let (arena_spot, studio_spot) = (
                peak_over_mean(&arena, level),
                peak_over_mean(&studio, level),
            );
            eprintln!("level {level}: peak/mean arena {arena_spot:.2} studio {studio_spot:.2}");
            assert!(
                arena_spot < 4.0,
                "level {level}: arena reflection is a wash, got {arena_spot}"
            );
            assert!(
                arena_spot * 1.5 < studio_spot,
                "level {level}: the arena env is far less spotty than the studio ({arena_spot} vs {studio_spot})"
            );
        }
    }

    #[test]
    fn the_arena_env_keeps_the_up_facing_ambient() {
        // The look's exposure and the sand's warm tan were tuned against the
        // studio's irradiance on an up-facing surface (the sand); the arena
        // sky is calibrated to it per channel, so swapping the environment
        // removes the studio's lights without re-exposing or re-tinting the
        // arena.
        let (arena, _) = bake_arena_day().expect("arena day bakes");
        let (studio, _) = bake(STUDIO).expect("studio bakes");
        let up = |b: &IblBake| eval_sh(&b.sh, [0.0, 1.0, 0.0]).expect("sh evaluates");
        let (a, s) = (up(&arena), up(&studio));
        eprintln!("RGB up: arena {a:?} studio {s:?}");
        for channel in 0..3 {
            let ratio = a[channel] / s[channel];
            assert!(
                (0.92..=1.08).contains(&ratio),
                "up-facing ambient channel {channel}: arena {} vs studio {}",
                a[channel],
                s[channel]
            );
        }
        // Outdoors the sides get less than the studio's white room, but the
        // sand's bounce keeps a wall or a flank well lit (80 to 100 percent).
        let side = |b: &IblBake| luma(eval_sh(&b.sh, [1.0, 0.0, 0.0]).expect("sh evaluates"));
        let down = |b: &IblBake| luma(eval_sh(&b.sh, [0.0, -1.0, 0.0]).expect("sh evaluates"));
        eprintln!(
            "side: arena {:.3} studio {:.3}; down: arena {:.3} studio {:.3}",
            side(&arena),
            side(&studio),
            down(&arena),
            down(&studio)
        );
        let fill = side(&arena) / side(&studio);
        assert!((0.7..=1.0).contains(&fill), "side fill ratio {fill}");
    }

    /// Solid-angle-weighted mean luminance of an env map: the sky capture mean
    /// the SH's L0 band has to carry (the same weights `project_sh` uses).
    fn weighted_mean_luma(env: &crate::v1_ibl_shadow::EnvMap) -> f32 {
        let (w, h) = (env.width as usize, env.height as usize);
        let (mut sum, mut weight) = (0.0f64, 0.0f64);
        for y in 0..h {
            let phi = (y as f64 + 0.5) / h as f64 * std::f64::consts::PI;
            for x in 0..w {
                sum += f64::from(luma(env.pixels[y * w + x])) * phi.sin();
                weight += phi.sin();
            }
        }
        (sum / weight) as f32
    }

    /// The mean radiance the SH's L0 band carries: its irradiance over pi,
    /// luminance (a uniform radiance `p` gives irradiance `pi * p`).
    fn l0_luma(sh: &IrradianceSh) -> f32 {
        luma(sh.coeffs[0]) * 0.282_094_8 / std::f32::consts::PI
    }

    #[test]
    fn sky_ibl_l0_matches_the_sky_capture_mean_at_five_elevations() {
        // RL40 accept: the keyframe SH keeps the sky's energy, at every sun
        // height of the day, to 0.02 of the capture's mean luminance.
        let keys = bake_sky_keys().expect("keyframes bake");
        assert_eq!(keys.frames().len(), 5);
        for (frame, elevation) in keys.frames().iter().zip(SKY_KEY_ELEVATIONS_DEG) {
            assert_eq!(frame.elevation_deg, elevation);
            let env = arena_env(&arena_params_at(elevation), ARENA_GROUND_BOUNCE).expect("env");
            let (mean, l0) = (weighted_mean_luma(&env), l0_luma(&frame.sh));
            eprintln!("sun {elevation:>4} deg: capture mean {mean:.4} SH L0 {l0:.4}");
            assert!(
                (mean - l0).abs() <= 0.02,
                "sun {elevation}: capture mean {mean} against SH L0 {l0}"
            );
            assert!(mean > 0.05, "a lit sky, not a black one: {mean}");
        }
    }

    #[test]
    fn sky_ibl_the_rig_sun_and_every_higher_one_keep_the_full_ambient() {
        use crate::arena_day::{daylight_scale, DAYLIGHT_FLOOR};
        let at = |deg: f32| daylight_scale(&arena_params_at(deg));
        assert_eq!(
            daylight_scale(&arena_day_params()),
            1.0,
            "the day rig itself"
        );
        for deg in [36.0, 48.0, 72.0, 89.0] {
            assert_eq!(at(deg), 1.0, "sun at {deg}");
        }
        let mut last = 1.0f32;
        for deg in [28.0, 20.0, 14.0, 8.0, 5.0, 1.0] {
            let scale = at(deg);
            assert!(scale < last && scale >= DAYLIGHT_FLOOR, "{deg}: {scale}");
            last = scale;
        }
        assert!(at(5.0) < 0.3, "first light is far under noon: {}", at(5.0));
        let night = crate::arena_day::arena_night_params();
        assert_eq!(
            daylight_scale(&night),
            1.0,
            "the night family has its own gain"
        );
    }

    #[test]
    fn sky_ibl_keyframe_bytes_x2_equal() {
        let (a, b) = (
            bake_sky_keys().expect("first bake"),
            bake_sky_keys().expect("second bake"),
        );
        assert_eq!(a, b, "same call, same keyframes");
        assert_eq!(a.to_bytes(), b.to_bytes(), "same call, same bytes");
        assert_eq!(a.to_bytes().len(), 5 * (4 + 9 * 12 + 12));
    }

    #[test]
    fn sky_ibl_dawn_and_noon_differ_on_shadow_side_sand() {
        // The shadow side of sand takes the ambient alone: an up-facing normal.
        // The sky at first light and near noon must light it differently, and
        // by a margin a frame shows (more than a fifth of the brighter one).
        let keys = bake_sky_keys().expect("keyframes bake");
        let up = [0.0, 1.0, 0.0];
        let at = |deg: f32| luma(eval_sh(&keys.at(deg).sh, up).expect("sh evaluates"));
        let (dawn, noon) = (at(5.0), at(72.0));
        eprintln!("shadow-side sand: dawn {dawn:.3} noon {noon:.3}");
        assert!(
            (noon - dawn).abs() > 0.2 * noon.max(dawn),
            "dawn {dawn} against noon {noon}"
        );
    }

    #[test]
    fn sky_ibl_the_lerp_hits_each_key_and_stays_between_neighbours() {
        let keys = bake_sky_keys().expect("keyframes bake");
        for frame in keys.frames() {
            assert_eq!(
                &keys.at(frame.elevation_deg),
                frame,
                "on a key it is the key"
            );
        }
        let (lo, hi) = (&keys.frames()[1], &keys.frames()[2]);
        let mid = keys.at(0.5 * (lo.elevation_deg + hi.elevation_deg));
        let c = |f: &SkyKeyframe| f.sh.coeffs[0][1];
        let (a, b, m) = (c(lo), c(hi), c(&mid));
        assert!(m > a.min(b) && m < a.max(b), "{a} < {m} < {b}");
        let past = keys.at(200.0);
        assert_eq!(
            &past,
            keys.frames().last().expect("five keys"),
            "past the end"
        );
        assert_eq!(keys.at(-3.0).elevation_deg, 5.0, "under the first key");
        assert_eq!(
            keys.at(f32::NAN).elevation_deg,
            5.0,
            "nan keeps the first key"
        );
    }

    #[test]
    fn sky_ibl_sun_colour_warms_as_the_sun_sinks_and_stays_normalized() {
        let sun_at = |deg: f32| crate::sky::sun_color(&arena_params_at(deg)).expect("sun colour");
        let mut previous_blue = f32::MAX;
        for deg in [72.0f32, 48.0, 28.0, 14.0, 5.0] {
            let c = sun_at(deg);
            eprintln!("sun {deg:>4} deg colour {c:?}");
            assert!(c[0] >= c[1] && c[1] >= c[2], "warm order at {deg}: {c:?}");
            assert!(
                (c[0] - 1.0).abs() < 1e-6,
                "red is the brightest channel: {c:?}"
            );
            assert!(c[2] < previous_blue, "blue falls as the sun sinks: {c:?}");
            previous_blue = c[2];
        }
        let low = sun_at(5.0);
        let high = sun_at(72.0);
        assert!(low[2] < 0.9 * high[2], "a low sun is visibly warmer");
    }

    #[test]
    fn sky_ibl_the_day_rig_sun_keeps_the_light_intensity() {
        let rig = crate::arena_day::sun_elevation_deg(crate::arena_day::ARENA_DAY_SUN_DIR)
            .expect("day rig elevation");
        let sun = crate::arena_day::arena_sun_at(rig).expect("sun at the rig elevation");
        let len: f32 = sun.dir_to_light.iter().map(|c| c * c).sum::<f32>().sqrt();
        assert!((len - 1.0).abs() < 1e-5, "a unit direction: {len}");
        assert!(
            (sun.intensity - crate::arena_day::ARENA_SUN_INTENSITY).abs() < 0.2,
            "the air dims the sun by under 7 percent at the rig height: {}",
            sun.intensity
        );
        let day = crate::arena_day::ARENA_DAY_SUN_DIR;
        let d = (day[0] * day[0] + day[1] * day[1] + day[2] * day[2]).sqrt();
        for (c, want) in day.iter().enumerate() {
            assert!(
                (sun.dir_to_light[c] - want / d).abs() < 1e-4,
                "the rig's own direction, axis {c}"
            );
        }
    }
}
