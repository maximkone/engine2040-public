//! arena_day: one EV100 arena day light-rig preset as data.
//!
//! Owner: render lane; consumers are the window presenter through the
//! shared look bake (one bake, three readers) and the capture bundle.
//! Units: unit-free sun direction, turbidity units, 0..=1 albedo, EV100
//! stops, meters for fog. Determinism: fixed 64x32 row-major bake; same
//! preset gives bit-identical bytes twice (pixels stay presentation,
//! never the sim hash). Errors are typed `ArenaDayError`; pass code is
//! frozen, this file only tunes data.
//!
//! Cost (base tier bars, enforced on boosted release clocks): sky 0.4 ms,
//! height fog 0.2 ms, contact 0.3 ms; this preset adds no pass.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::fog::FogParams;
use crate::shadow_cascade::{cascade_map_size, checked_ranges, CascadeRange, FIRST_CASCADE_FAR_M};
use crate::sky::{SkyCapture, SkyLookBake, SkyParams};

/// Sun direction for the arena day rig (any nonzero finite length).
/// Afternoon southwest rake, ~36 deg elevation: long readable cascade
/// shadows across the sand while the gate tunnel mouth stays dim.
pub const ARENA_DAY_SUN_DIR: [f32; 3] = [-0.62, 0.55, 0.45];
/// Air turbidity for the day rig, inside the sky accepted range.
pub const ARENA_DAY_TURBIDITY: f32 = 3.0;
/// Sand ground albedo per channel in 0..=1 (plausible dry sand 0.2..0.5).
pub const ARENA_DAY_GROUND_ALBEDO: [f32; 3] = [0.42, 0.32, 0.20];
/// Exposure anchor in EV100, matching the shared look-frame anchor so
/// the rig moves light angle and air, never the exposure scale.
pub const ARENA_DAY_EXPOSURE_EV100: f32 = 12.0;
/// Day-rig capture width in pixels feeding IBL plus fog.
pub const ARENA_DAY_CAPTURE_W: u32 = 64;
/// Day-rig capture height in pixels feeding IBL plus fog.
pub const ARENA_DAY_CAPTURE_H: u32 = 32;
/// Fog extinction density per meter (reuses the shared day feed value).
pub const ARENA_DAY_FOG_DENSITY_PER_M: f32 = 0.02;
/// Fog height falloff in meters (reuses the shared day feed value).
pub const ARENA_DAY_FOG_FALLOFF_M: f32 = 8.0;
/// Fog base height in meters (reuses the shared day feed value).
pub const ARENA_DAY_FOG_BASE_M: f32 = 0.0;
/// Fog cutoff distance in meters (reuses the shared day feed value).
pub const ARENA_DAY_FOG_MAX_DIST_M: f32 = 500.0;
/// Day-rig cascade camera near in meters: the day-sweep fixture range open.
pub const ARENA_DAY_CASCADE_NEAR_M: f32 = 0.5;
/// Day-rig cascade camera far in meters: the day-sweep fixture range close.
pub const ARENA_DAY_CASCADE_FAR_M: f32 = 120.0;

// PORT(FyroxEngine/Fyrox:fyrox-impl/src/renderer/shadow/csm.rs, MIT):
// cascade-split selection plus per-cascade map sizing for one directional
// sun. Adapted, no bytes pasted: Fyrox's CSM split plus per-cascade size
// becomes `shadow_cascade::checked_ranges` (overlap-aware log intervals
// from `FIRST_CASCADE_FAR_M`) plus `shadow_cascade::cascade_map_size`
// (`CASCADE_MAP_SIZES`); this file only feeds the day-rig sun range
// through those three intervals instead of a single split.

/// Day-rig sun cascades: three overlap-aware intervals over
/// `ARENA_DAY_CASCADE_NEAR_M..ARENA_DAY_CASCADE_FAR_M` plus one map edge
/// per cascade from `cascade_map_size`. Pure; same rig gives the same
/// intervals and sizes twice. Fails loudly when the frozen range refuses.
pub fn arena_day_cascades() -> Result<([CascadeRange; 3], [u32; 3]), ArenaDayError> {
    let ranges = checked_ranges(
        ARENA_DAY_CASCADE_NEAR_M,
        FIRST_CASCADE_FAR_M,
        ARENA_DAY_CASCADE_FAR_M,
    )
    .map_err(|e| ArenaDayError::Invalid(format!("arena day cascades: {e}")))?;
    let maps = [
        cascade_map_size(0).map_err(|e| ArenaDayError::Invalid(format!("arena day map 0: {e}")))?,
        cascade_map_size(1).map_err(|e| ArenaDayError::Invalid(format!("arena day map 1: {e}")))?,
        cascade_map_size(2).map_err(|e| ArenaDayError::Invalid(format!("arena day map 2: {e}")))?,
    ];
    Ok((ranges, maps))
}

/// Arena day-rig failures. Bad preset data fails loudly; device faults
/// never arise here because the bake is the std-only sky headless twin.
#[derive(Clone, PartialEq, Debug)]
pub enum ArenaDayError {
    /// Preset input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for ArenaDayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "arena day input: {e}"),
        }
    }
}

impl std::error::Error for ArenaDayError {}

fn sky_err(e: crate::sky::SkyError) -> ArenaDayError {
    ArenaDayError::Invalid(format!("arena day sky: {e}"))
}

fn fog_err(e: crate::fog::FogError) -> ArenaDayError {
    ArenaDayError::Invalid(format!("arena day fog: {e}"))
}

/// The one arena day preset as sky bake inputs: the rig's afternoon sun, or the
/// sun at [`sun_elevation_override`] when the run asked for another time of day
/// (`arena --sun-elev`). Every reader of the day rig (the skybox, the fog, the
/// baked ambient, the direct sun) goes through here, so they never disagree.
pub fn arena_day_params() -> SkyParams {
    match sun_elevation_override() {
        Some(elevation_deg) => arena_params_at(elevation_deg),
        None => day_rig_params(),
    }
}

/// The day rig as authored, whatever the override says.
fn day_rig_params() -> SkyParams {
    SkyParams {
        sun_dir: ARENA_DAY_SUN_DIR,
        turbidity: ARENA_DAY_TURBIDITY,
        ground_albedo: ARENA_DAY_GROUND_ALBEDO,
        exposure_ev100: ARENA_DAY_EXPOSURE_EV100,
        night_gain: 1.0,
    }
}

/// Bits of "no override": a NaN, which no elevation is.
const NO_SUN_OVERRIDE: u32 = 0x7fc0_0000;
/// The process-wide time of day the run asked for, as `f32` bits. Presentation
/// only (never read by the simulation); set once at startup, before the
/// presenter bakes its sky.
static SUN_ELEVATION_OVERRIDE: AtomicU32 = AtomicU32::new(NO_SUN_OVERRIDE);

/// Put the day rig's sun at `elevation_deg` for the rest of the run (`None`
/// restores the rig's own afternoon). Call before the presenter is built: the
/// sky, fog and ambient bake once at init.
pub fn set_sun_elevation_override(elevation_deg: Option<f32>) {
    let bits = match elevation_deg {
        Some(deg) if deg.is_finite() => deg.to_bits(),
        _ => NO_SUN_OVERRIDE,
    };
    SUN_ELEVATION_OVERRIDE.store(bits, Ordering::Relaxed);
}

/// The sun elevation the run asked for, degrees, if any.
pub fn sun_elevation_override() -> Option<f32> {
    let value = f32::from_bits(SUN_ELEVATION_OVERRIDE.load(Ordering::Relaxed));
    value.is_finite().then_some(value)
}

/// The arena day rig with the sun raised or lowered: the day preset's azimuth,
/// air and ground, the sun at `elevation_deg` (clamped to 0.5..=89 degrees so
/// the direction stays a direction). The one function every time-of-day reader
/// goes through, so the sky, the ambient keyframes and the sun light can never
/// disagree about where the sun is. At the day rig's own elevation it is
/// [`arena_day_params`] up to rounding.
pub fn arena_params_at(elevation_deg: f32) -> SkyParams {
    let base = day_rig_params();
    let (x, z) = (base.sun_dir[0], base.sun_dir[2]);
    let flat = (x * x + z * z).sqrt().max(1e-6);
    let e = elevation_deg.clamp(0.5, 89.0).to_radians();
    SkyParams {
        sun_dir: [x / flat * e.cos(), e.sin(), z / flat * e.cos()],
        ..base
    }
}

/// The sky light's floor under a sun on the horizon, as a share of its noon
/// strength.
pub const DAYLIGHT_FLOOR: f32 = 0.15;
/// The sine of the sun's elevation from which the sky light is at full
/// strength: 0.57 is 34.7 degrees, just under the day rig's own afternoon sun
/// (35.7), so the rig and every higher sun keep the ambient they always had.
pub const DAYLIGHT_FULL_SIN: f32 = 0.57;

/// How much of its noon strength the ambient keeps under this sun, in
/// `DAYLIGHT_FLOOR..=1`. The single scatter model dims the air along the view
/// ray, not along the sun's, so the sky it paints is as bright at dawn as at
/// noon; the baked ambient scales with the sun's height instead (a smoothstep
/// of the sine of the elevation), which is what makes a dawn frame read as
/// dawn. The night family (`night_gain` under 1) is already its own gain and
/// is never dimmed twice.
pub fn daylight_scale(params: &SkyParams) -> f32 {
    if params.night_gain < 1.0 {
        return 1.0;
    }
    let d = params.sun_dir;
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let t = ((d[1] / len) / DAYLIGHT_FULL_SIN).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    1.0 - (1.0 - DAYLIGHT_FLOOR) * (1.0 - t)
}

/// Intensity of the direct sun at the day rig, `color * intensity`: the same
/// 3.0 the default light carried, so the exposure scale does not move.
pub const ARENA_SUN_INTENSITY: f32 = 3.0;

/// The direct sun of the day rig at one elevation: where it is, what colour
/// the air leaves it and how strong it is. Feeds `DirectionalLight` (and so
/// the sun shadow map); the ambient comes from the same params through
/// `ibl_arena_env`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ArenaSun {
    /// Unit direction toward the sun.
    pub dir_to_light: [f32; 3],
    /// Linear colour, the brightest channel 1: [`crate::sky::sun_color`].
    pub color: [f32; 3],
    /// Scale on the colour: [`ARENA_SUN_INTENSITY`] under the air's own dimming.
    pub intensity: f32,
}

/// The sun of the day rig at `elevation_deg`. Pure; fails loudly when the sky
/// refuses the params.
pub fn arena_sun_at(elevation_deg: f32) -> Result<ArenaSun, ArenaDayError> {
    let params = arena_params_at(elevation_deg);
    let color = crate::sky::sun_color(&params).map_err(sky_err)?;
    let through = crate::sky::sun_transmittance(&params).map_err(sky_err)?;
    let len = params
        .sun_dir
        .iter()
        .map(|c| c * c)
        .sum::<f32>()
        .sqrt()
        .max(1e-6);
    Ok(ArenaSun {
        dir_to_light: params.sun_dir.map(|c| c / len),
        color,
        intensity: ARENA_SUN_INTENSITY * through[0].max(through[1]).max(through[2]),
    })
}

/// The sun this run lights the frame with: [`sun_elevation_override`]'s, else
/// the day rig's own afternoon sun.
pub fn arena_sun_now() -> Result<ArenaSun, ArenaDayError> {
    let elevation_deg = match sun_elevation_override() {
        Some(elevation_deg) => elevation_deg,
        None => sun_elevation_deg(ARENA_DAY_SUN_DIR)?,
    };
    arena_sun_at(elevation_deg)
}

/// Moon direction for the arena night rig: the sun's own angle with a low
/// elevation, so the moon rakes the sand from the same side as the day sun
/// and the torch shadows fall the same way.
pub const ARENA_NIGHT_SUN_DIR: [f32; 3] = [-0.62, 0.18, 0.45];
/// Air turbidity for the night rig: clear night air, so the stars read and
/// the horizon stays a clean dark blue.
pub const ARENA_NIGHT_TURBIDITY: f32 = 2.0;
/// Night ground albedo per channel in 0..=1: dark sand, a touch cooler
/// than the day ground so the warm torches read against it.
pub const ARENA_NIGHT_GROUND_ALBEDO: [f32; 3] = [0.16, 0.13, 0.10];
/// Exposure anchor in EV100 for the night rig: a moonlit scene sits many
/// stops under the noon sun, so the anchor drops and the torches carry the
/// picture instead of the sky.
pub const ARENA_NIGHT_EXPOSURE_EV100: f32 = 4.0;
/// How much of the sky's radiance the night rig keeps: the moonlight only.
/// The frame's ambient sits at this fraction of the day level, so the
/// placement's torches are the light the arena reads.
pub const ARENA_NIGHT_GAIN: f32 = 0.06;

/// The arena night preset as sky bake inputs: a moonlit sky over dark sand,
/// so the placement's torches are the light the frame reads. Pure: the same
/// preset gives the same bake bytes twice.
pub fn arena_night_params() -> SkyParams {
    SkyParams {
        sun_dir: ARENA_NIGHT_SUN_DIR,
        turbidity: ARENA_NIGHT_TURBIDITY,
        ground_albedo: ARENA_NIGHT_GROUND_ALBEDO,
        exposure_ev100: ARENA_NIGHT_EXPOSURE_EV100,
        night_gain: ARENA_NIGHT_GAIN,
    }
}

/// Sun elevation of the day-rig sun in degrees (35..37 expected).
/// Returns an error for a non-finite or ~zero sun direction.
pub fn sun_elevation_deg(sun_dir: [f32; 3]) -> Result<f32, ArenaDayError> {
    if !sun_dir.iter().all(|c| c.is_finite()) {
        return Err(ArenaDayError::Invalid(
            "arena day sun_dir is not finite".into(),
        ));
    }
    let len2 = sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2];
    if len2 < 1e-12 {
        return Err(ArenaDayError::Invalid(
            "arena day sun_dir has ~zero length".into(),
        ));
    }
    let elev = (sun_dir[1] / len2.sqrt()).clamp(-1.0, 1.0).asin();
    Ok(elev.to_degrees())
}

/// One baked day rig: the sky capture plus the look-bake readers
/// (skybox texels, ambient level, fog color) and the day-shape fog
/// params ramped from the same capture, so sky, ambient and fog can
/// never drift apart.
#[derive(Clone, PartialEq, Debug)]
pub struct ArenaDayRig {
    /// Preset inputs the bake ran from.
    pub params: SkyParams,
    /// Lat-long day capture in linear HDR, row 0 at the zenith.
    pub capture: SkyCapture,
    /// Look bake over the same capture (skybox plus precomputed means).
    pub look: SkyLookBake,
    /// Ambient level: mean radiance from the same bake.
    pub ibl_ambient: [f32; 3],
    /// Fog color: horizon ring from the same bake.
    pub fog_color: [f32; 3],
    /// Fog params ramped from the same capture with day extinction.
    pub fog: FogParams,
    /// Stops-to-scale exposure from the EV100 anchor.
    pub exposure: f32,
    /// Day-sun cascade intervals in meters: three overlap-aware ranges
    /// from `arena_day_cascades`, replacing the old single split.
    pub sun_cascades: [CascadeRange; 3],
    /// Shadow map edge per cascade in texels from `cascade_map_size`.
    pub sun_cascade_maps: [u32; 3],
}

impl ArenaDayRig {
    /// Deterministic little-endian bytes: preset floats, capture bytes,
    /// ambient triple, fog triple, fog extinction, exposure, cascade
    /// intervals plus map sizes. Same preset baked twice gives
    /// bit-identical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(32 + self.capture.texels.len() * 12 + 24 + 20 + 4 + 24 + 12);
        for c in self.params.sun_dir {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&self.params.turbidity.to_le_bytes());
        for c in self.params.ground_albedo {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&self.params.exposure_ev100.to_le_bytes());
        out.extend_from_slice(&self.capture.to_bytes());
        for c in self.ibl_ambient.iter().chain(self.fog_color.iter()) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for c in [
            self.fog.density_per_m,
            self.fog.falloff_m,
            self.fog.base_height_m,
            self.fog.max_distance_m,
            self.fog.exposure_ev100,
        ] {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&self.exposure.to_le_bytes());
        for range in self.sun_cascades {
            out.extend_from_slice(&range.near_m.to_le_bytes());
            out.extend_from_slice(&range.far_m.to_le_bytes());
        }
        for size in self.sun_cascade_maps {
            out.extend_from_slice(&size.to_le_bytes());
        }
        out
    }
}

/// Bake the arena day preset: sky capture, look bake, ambient level,
/// fog color plus fog params, EV100 exposure scale. Fails loudly on
/// ragged or non-finite bake output.
///
/// ```rust
/// let rig = engine_render::arena_day::bake_arena_day().expect("day rig bakes");
/// assert!(rig.exposure > 0.0);
/// ```
pub fn bake_arena_day() -> Result<ArenaDayRig, ArenaDayError> {
    bake_preset(&arena_day_params())
}

/// Bake the arena night preset: the same one-bake-three-readers path over
/// the moonlit sky params, so the night sky, ambient and fog read from one
/// capture and can never drift. Fails loudly on ragged or non-finite bake
/// output, exactly like the day bake.
pub fn bake_arena_night() -> Result<ArenaDayRig, ArenaDayError> {
    bake_preset(&arena_night_params())
}

/// Bake any day-style preset through the same one-bake-three-readers path.
/// The fog extinction always rides the day-rig values so ad-hoc callers
/// cannot fork a second fog truth.
pub fn bake_preset(params: &SkyParams) -> Result<ArenaDayRig, ArenaDayError> {
    params.validate().map_err(sky_err)?;
    let (capture, _) =
        crate::sky::bake_sky(params, ARENA_DAY_CAPTURE_W, ARENA_DAY_CAPTURE_H).map_err(sky_err)?;
    let (look, _) = crate::sky::bake_look_from_day(&capture).map_err(sky_err)?;
    let ibl_ambient = crate::sky::look_ibl_ambient(&look);
    let fog_color = crate::sky::look_fog_color(&look);
    if !ibl_ambient.iter().all(|c| c.is_finite()) || ibl_ambient.iter().any(|c| *c < 0.0) {
        return Err(ArenaDayError::Invalid(format!(
            "arena day ambient {ibl_ambient:?} is not finite nonnegative"
        )));
    }
    let fog = FogParams::from_sky_capture(
        &capture,
        ARENA_DAY_FOG_DENSITY_PER_M,
        ARENA_DAY_FOG_FALLOFF_M,
        ARENA_DAY_FOG_BASE_M,
        ARENA_DAY_FOG_MAX_DIST_M,
        params.exposure_ev100,
    )
    .map_err(fog_err)?;
    let exposure = crate::sky::ev100_to_exposure(params.exposure_ev100).map_err(sky_err)?;
    let (sun_cascades, sun_cascade_maps) = arena_day_cascades()?;
    Ok(ArenaDayRig {
        params: *params,
        capture,
        look,
        ibl_ambient,
        fog_color,
        fog,
        exposure,
        sun_cascades,
        sun_cascade_maps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_validates_and_points_afternoon() {
        let params = arena_day_params();
        params.validate().expect("day preset validates");
        let elev = sun_elevation_deg(params.sun_dir).expect("sun elevation reads");
        assert!((30.0..=45.0).contains(&elev), "elev {elev} near 36 deg");
        assert!(
            (params.exposure_ev100 - ARENA_DAY_EXPOSURE_EV100).abs() < 1e-6,
            "anchor stays EV100 12"
        );
    }

    #[test]
    fn preset_differs_from_midday_in_angle_not_anchor() {
        let day = arena_day_params();
        let mid = SkyParams::midday();
        assert_ne!(day.sun_dir, mid.sun_dir, "sun angle moves");
        assert_ne!(day.turbidity, mid.turbidity, "air moves");
        assert_eq!(
            day.exposure_ev100, mid.exposure_ev100,
            "exposure anchor shared"
        );
    }

    #[test]
    fn bake_twice_gives_identical_bytes() {
        let a = bake_arena_day().expect("first bake holds");
        let b = bake_arena_day().expect("second bake holds");
        assert_eq!(a.to_bytes(), b.to_bytes(), "day rig bytes x2");
        assert_eq!(a.capture.texels.len(), 64 * 32, "capture backing full");
    }

    /// VS7: the night rig is a moonlit sky: the ambient sits far under the
    /// day's, so the placement's torches are the light the frame reads.
    #[test]
    fn the_night_rig_sits_far_under_the_day_ambient() {
        let day = bake_arena_day().expect("day rig bakes");
        let night = bake_arena_night().expect("night rig bakes");
        let day_mean = day.ibl_ambient.iter().sum::<f32>() / 3.0;
        let night_mean = night.ibl_ambient.iter().sum::<f32>() / 3.0;
        assert!(
            night_mean < day_mean * 0.2,
            "the night ambient is under a fifth of the day's ({night_mean} vs {day_mean})"
        );
        assert!(night_mean > 0.0, "the night is moonlit, not black");
        // The anchor is the fact: the night rig sits under the day's.
        assert!(
            night.params.exposure_ev100 < day.params.exposure_ev100,
            "the night anchor sits under the day's"
        );
        // The scale follows the anchor: `2^-ev100`, so the lower anchor
        // gives the WIDER exposure, which is what lets a dim night scene
        // read at all.
        assert!(
            night.exposure > day.exposure,
            "the night exposure sits over the day's"
        );
    }

    /// VS7: the day rig keeps every bit of the shipped look. The night gain
    /// multiplies by exactly 1.0 on the day path, so the day bake is the
    /// same bytes it was before the night preset landed.
    #[test]
    fn the_day_rig_keeps_the_shipped_bytes() {
        let day = arena_day_params();
        assert_eq!(day.night_gain, 1.0, "the day rig keeps all the sky");
        let with_gain = bake_preset(&day).expect("day rig bakes");
        let without_gain = bake_preset(&SkyParams {
            night_gain: 1.0,
            ..day
        })
        .expect("day rig bakes again");
        assert_eq!(
            with_gain.to_bytes(),
            without_gain.to_bytes(),
            "the day bake is the shipped bytes"
        );
    }

    /// VS7: the night bake is as deterministic as the day bake.
    #[test]
    fn the_night_bake_gives_identical_bytes_twice() {
        let a = bake_arena_night().expect("first night bake holds");
        let b = bake_arena_night().expect("second night bake holds");
        assert_eq!(a.to_bytes(), b.to_bytes(), "night rig bytes x2");
    }

    #[test]
    fn readers_come_from_the_same_backing() {
        let rig = bake_arena_day().expect("rig bakes");
        let direct_fog = crate::sky::fog_color(&rig.capture).expect("fog color reads");
        let direct_mean = rig.capture.mean_radiance().expect("mean reads");
        assert_eq!(rig.fog_color, direct_fog, "fog reader matches ramp");
        assert_eq!(rig.ibl_ambient, direct_mean, "ambient reader matches mean");
        assert_eq!(rig.fog.fog_color, direct_fog, "fog params ramp matches");
        assert!(
            rig.exposure.is_finite() && rig.exposure > 0.0,
            "exposure scales"
        );
    }

    #[test]
    fn bad_sun_fails_loudly() {
        assert!(
            sun_elevation_deg([0.0, 0.0, 0.0]).is_err(),
            "zero sun refused"
        );
        assert!(
            sun_elevation_deg([f32::NAN, 0.0, 0.0]).is_err(),
            "NaN sun refused"
        );
        let mut params = arena_day_params();
        params.turbidity = 99.0;
        assert!(bake_preset(&params).is_err(), "haze outside range refused");
    }

    #[test]
    fn day_rig_feeds_three_cascades() {
        let (ranges, maps) = arena_day_cascades().expect("day cascades build");
        assert_eq!(ranges.len(), 3, "day sun rides three cascades");
        assert_eq!(ranges[0].near_m, ARENA_DAY_CASCADE_NEAR_M);
        assert!(
            (ranges[0].far_m - 10.0).abs() < 0.05,
            "far {}",
            ranges[0].far_m
        );
        assert!(
            (ranges[1].near_m - 8.0).abs() < 0.05,
            "near {}",
            ranges[1].near_m
        );
        assert!(
            (ranges[2].near_m - 27.71).abs() < 0.05,
            "near {}",
            ranges[2].near_m
        );
        assert_eq!(ranges[2].far_m, ARENA_DAY_CASCADE_FAR_M);
        assert!(ranges[1].near_m < ranges[0].far_m, "seam 0 blends");
        assert!(ranges[2].near_m < ranges[1].far_m, "seam 1 blends");
        assert_eq!(maps, [512, 256, 128], "sizes ride cascade_map_size");
        for (i, size) in maps.iter().enumerate() {
            assert_eq!(
                cascade_map_size(i).expect("cascade in range"),
                *size,
                "map {i} comes from the helper"
            );
        }
        let rig = bake_arena_day().expect("rig bakes");
        assert_eq!(rig.sun_cascades, ranges, "bake carries the intervals");
        assert_eq!(rig.sun_cascade_maps, maps, "bake carries the sizes");
        let again = bake_arena_day().expect("second bake holds");
        assert_eq!(rig.sun_cascades, again.sun_cascades, "cascades x2");
        assert_eq!(
            rig.to_bytes(),
            again.to_bytes(),
            "rig bytes x2 with cascades"
        );
    }

    #[test]
    fn day_maps_refuse_a_fourth_cascade() {
        assert!(
            cascade_map_size(3).is_err(),
            "fourth cascade has no map size"
        );
        assert!(
            checked_ranges(0.5, ARENA_DAY_CASCADE_FAR_M, ARENA_DAY_CASCADE_FAR_M).is_err(),
            "first far at far refused"
        );
    }
}
