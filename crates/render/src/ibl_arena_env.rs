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
//! never the sim hash. Errors: typed [`IblError`], loud. Budget: 8192 sky
//! samples at init (well under a millisecond), then the usual bake.

use crate::arena_day::arena_day_params;
use crate::ibl::{bake_env, IblBake, IblError};
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
    let (w, h) = (ARENA_ENV_W, ARENA_ENV_H);
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
                let sky = radiance[c].min(ARENA_ENV_MAX) * ARENA_ENV_GAIN[c];
                texel[c] = sky + (ground_bounce[c] - sky) * ground;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ibl::{bake, eval_sh, IBL_FACES, IBL_LEVELS};

    const STUDIO: &[u8] =
        include_bytes!("../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

    fn luma(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
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
}
