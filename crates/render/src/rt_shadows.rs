//! rt_shadows: ray-traced sun shadows over the arena sand (RL-RT slice 1).
//!
//! Owner: render lane. Caller: `crate::gi_probe` (probe shade plus the GI
//! on/off frame) and through it `crate::gi::GiRuntime::irradiance_mixed`.
//! Units: meters for rays and boxes, texels for frames, 0..=1 factors and
//! linear RGB triples. Determinism: fixed box order with `+ - * /` and
//! `sqrt` only; same inputs give bit-identical frames twice (pixels stay
//! presentation, never the sim hash). Errors are typed `RtShadowError`;
//! frame buffers are caller-owned, so the hot path allocates nothing.

#![forbid(unsafe_code)]

use std::fmt;

#[path = "restir_di.rs"]
/// DI reservoir oracle for the many-light reference (see module docs).
pub mod restir_di;

/// Frame width in texels: 16 m of sand at two texels per meter.
pub const RT_FRAME_W: usize = 32;
/// Frame height in texels, matching the width (square sand map).
pub const RT_FRAME_H: usize = 32;
/// Frame texels: the only accepted buffer length.
pub const RT_FRAME_TEXELS: usize = RT_FRAME_W * RT_FRAME_H;
/// Sand map half extent in meters: texels span `-8..+8` on x and z.
pub const RT_HALF_EXTENT_M: f32 = 8.0;
/// Texel pitch in meters, matching `2 * HALF / W` exactly.
pub const RT_TEXEL_M: f32 = 0.5;
/// Shadow-ray lift off the sand in meters: one centimeter dodges the plane
/// without detaching the wall shadow from its base.
pub const RT_BIAS_M: f32 = 0.01;
/// Shadow-ray far reach in meters: past the arena bounds.
pub const RT_T_MAX_M: f32 = 60.0;
/// Pixel-change epsilon in linear channel units (~1 LSB at 8 bit).
pub const RT_EPSILON: f32 = 0.004;
/// Sun color in linear RGB driving the direct term.
pub const RT_SUN_COLOR: [f32; 3] = [1.0, 0.95, 0.88];
/// Dry-sand albedo in 0..=1, matching the day-rig ground albedo.
pub const RT_SAND_ALBEDO: [f32; 3] = [0.42, 0.32, 0.20];
/// Flat ambient floor in linear RGB when the GI probe stub stays off
/// (same triple the twin forward shader carries, so off-pixels match it).
pub const RT_AMBIENT_FLAT: [f32; 3] = [0.20, 0.18, 0.16];

/// Axis-aligned occluder in meters, closed box (`lo <= hi` per axis).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner in meters.
    pub lo: [f32; 3],
    /// Maximum corner in meters.
    pub hi: [f32; 3],
}

/// Pinned occluders: the gate wall (same footprint as the `gi_dyn` analytic
/// oracle) plus one fighter-stand-in pillar. Fixed order, fixed count.
pub const RT_BOXES: [Aabb; 2] = [
    Aabb {
        lo: [-4.0, 0.0, -0.5],
        hi: [4.0, 3.0, 0.5],
    },
    Aabb {
        lo: [1.5, 0.0, 2.5],
        hi: [2.5, 1.8, 3.5],
    },
];

/// Ray-traced shadow failures: bad sun, bad colors or bad buffers.
#[derive(Clone, PartialEq, Debug)]
pub enum RtShadowError {
    /// Input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for RtShadowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "rt shadow input: {e}"),
        }
    }
}

impl std::error::Error for RtShadowError {}

fn invalid(what: &str) -> RtShadowError {
    RtShadowError::Invalid(what.into())
}

fn check_tri(name: &str, v: [f32; 3]) -> Result<(), RtShadowError> {
    if !v.iter().all(|c| c.is_finite()) {
        return Err(invalid(&format!("rt shadow {name} is not finite")));
    }
    Ok(())
}

/// Grid index to `f32` scalar (exact for frame sizes; saturates, never
/// wraps). Render owner: `gi_probe` and `gi_dyn` reuse this instead of a
/// second copy; the saturating step reuses
/// `engine_core::checked::saturating_usize_to_u32`. Note: the `assets`
/// `count_f32` twin keeps its own `u16::try_from` copy, not this helper.
pub(crate) fn idx_f32(i: usize) -> f32 {
    let wide = engine_core::checked::saturating_usize_to_u32(i);
    let narrow = u16::try_from(wide.min(u32::from(u16::MAX))).unwrap_or(u16::MAX);
    f32::from(narrow)
}

/// Unit sun from a day-rig-style direction: finite, nonzero, above the
/// horizon (a set sun would smear shadows parallel to the sand).
/// Default rig: `crate::arena_day::ARENA_DAY_SUN_DIR`.
pub fn unit_sun(sun_dir: [f32; 3]) -> Result<[f32; 3], RtShadowError> {
    check_tri("sun_dir", sun_dir)?;
    let len2 = sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2];
    if len2 < 1e-12 {
        return Err(invalid("rt shadow sun_dir has ~zero length"));
    }
    let len = len2.sqrt();
    let sun = [sun_dir[0] / len, sun_dir[1] / len, sun_dir[2] / len];
    if sun[1] <= 1e-3 {
        return Err(invalid("rt shadow sun sits at or under the horizon"));
    }
    Ok(sun)
}

/// Cosine of the sand incidence: the unit sun over the +Y sand normal,
/// clamped at zero so grazing light adds nothing instead of negatives.
pub fn sand_ndotl(sun: [f32; 3]) -> f32 {
    sun[1].max(0.0)
}

/// Slab test: does the ray meet the box inside `(t_min, t_max)`? A parallel
/// axis must ride inside the slab, else the ray misses outright. An origin
/// inside the box counts as a hit (sand under the wall is dark).
pub fn ray_hits_box(
    origin: [f32; 3],
    dir: [f32; 3],
    bb: &Aabb,
    t_min: f32,
    t_max: f32,
) -> Result<bool, RtShadowError> {
    check_tri("origin", origin)?;
    check_tri("dir", dir)?;
    for a in 0..3 {
        if !bb.lo[a].is_finite() || !bb.hi[a].is_finite() || bb.lo[a] > bb.hi[a] {
            return Err(invalid("rt shadow box is not a finite lo<=hi slab"));
        }
    }
    if !t_min.is_finite() || !t_max.is_finite() || t_min < 0.0 || t_max <= t_min {
        return Err(invalid("rt shadow range is not 0 <= t_min < t_max"));
    }
    let mut t0 = t_min;
    let mut t1 = t_max;
    for a in 0..3 {
        let o = origin[a];
        let d = dir[a];
        if d.abs() < 1e-9 {
            if o < bb.lo[a] || o > bb.hi[a] {
                return Ok(false);
            }
        } else {
            let inv = 1.0 / d;
            let mut ta = (bb.lo[a] - o) * inv;
            let mut tb = (bb.hi[a] - o) * inv;
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            if ta > t0 {
                t0 = ta;
            }
            if tb < t1 {
                t1 = tb;
            }
            if t0 > t1 {
                return Ok(false);
            }
        }
    }
    Ok(t1 > t_min && t0 < t_max)
}

/// Hard shadow over one sand point: `0.0` when the lifted ray toward the
/// sun meets a pinned box, `1.0` otherwise. The lift (not a depth compare)
/// keeps open-sky sand lit and footprint sand dark.
pub fn sand_shadow_factor(point: [f32; 3], sun_dir: [f32; 3]) -> Result<f32, RtShadowError> {
    check_tri("point", point)?;
    let sun = unit_sun(sun_dir)?;
    let origin = [point[0], point[1] + RT_BIAS_M, point[2]];
    for bb in RT_BOXES.iter() {
        if ray_hits_box(origin, sun, bb, 0.0, RT_T_MAX_M)? {
            return Ok(0.0);
        }
    }
    Ok(1.0)
}

/// One sand texel: `albedo * (sun * ndotl * shadow + ambient)`. The direct
/// term dies in shadow, the ambient term never does, so shadowed sand keeps
/// the day floor instead of black (same contract as the twin bind).
pub fn sand_radiance(
    sun_dir: [f32; 3],
    shadow: f32,
    ambient: [f32; 3],
) -> Result<[f32; 3], RtShadowError> {
    let sun = unit_sun(sun_dir)?;
    if !shadow.is_finite() || !(0.0..=1.0).contains(&shadow) {
        return Err(invalid("rt shadow factor is outside 0..=1"));
    }
    check_tri("ambient", ambient)?;
    if ambient.iter().any(|c| *c < 0.0) {
        return Err(invalid("rt shadow ambient is negative"));
    }
    let direct = sand_ndotl(sun) * shadow;
    Ok([
        RT_SAND_ALBEDO[0] * (RT_SUN_COLOR[0] * direct + ambient[0]),
        RT_SAND_ALBEDO[1] * (RT_SUN_COLOR[1] * direct + ambient[1]),
        RT_SAND_ALBEDO[2] * (RT_SUN_COLOR[2] * direct + ambient[2]),
    ])
}

/// Sand point under texel `(ix, iy)`: texel centers tile `-8..+8` on x and
/// z at `y = 0`. Column convention: x right, y up, z toward the viewer; no
/// view matrix (the orthographic ground map projects by construction).
pub fn sand_texel_pos(ix: usize, iy: usize) -> Result<[f32; 3], RtShadowError> {
    if ix >= RT_FRAME_W || iy >= RT_FRAME_H {
        return Err(invalid("rt shadow texel is past the frame"));
    }
    Ok([
        (idx_f32(ix) + 0.5) * RT_TEXEL_M - RT_HALF_EXTENT_M,
        0.0,
        (idx_f32(iy) + 0.5) * RT_TEXEL_M - RT_HALF_EXTENT_M,
    ])
}

/// Render the sand frame: every texel shades through [`sand_radiance`]
/// with its own shadow sample when `shadow_on`, forced lit when off (the
/// EG8 on/off convention, matching the twin `shadow_on` flag). `out` is
/// caller-owned with exactly [`RT_FRAME_TEXELS`] triples; returns the
/// texel count. Zero heap allocation on the success path.
pub fn render_sand_frame(
    sun_dir: [f32; 3],
    shadow_on: bool,
    ambient: [f32; 3],
    out: &mut [[f32; 3]],
) -> Result<usize, RtShadowError> {
    if out.len() != RT_FRAME_TEXELS {
        return Err(invalid("rt shadow frame buffer is not 32x32"));
    }
    unit_sun(sun_dir)?;
    check_tri("ambient", ambient)?;
    if ambient.iter().any(|c| *c < 0.0) {
        return Err(invalid("rt shadow ambient is negative"));
    }
    for iy in 0..RT_FRAME_H {
        for ix in 0..RT_FRAME_W {
            let point = sand_texel_pos(ix, iy)?;
            let shadow = if shadow_on {
                sand_shadow_factor(point, sun_dir)?
            } else {
                1.0
            };
            out[iy * RT_FRAME_W + ix] = sand_radiance(sun_dir, shadow, ambient)?;
        }
    }
    Ok(RT_FRAME_TEXELS)
}

/// EG8 pixel gate: count texels whose max-channel absolute difference tops
/// `epsilon`. Equal lengths required; non-finite texels and a non-positive
/// epsilon fail loudly instead of passing a broken frame as live.
pub fn count_changed(a: &[[f32; 3]], b: &[[f32; 3]], epsilon: f32) -> Result<usize, RtShadowError> {
    if a.len() != b.len() {
        return Err(invalid("rt shadow gate buffers differ in length"));
    }
    if !epsilon.is_finite() || epsilon <= 0.0 {
        return Err(invalid("rt shadow gate epsilon is not positive finite"));
    }
    let mut changed = 0usize;
    for (x, y) in a.iter().zip(b.iter()) {
        for c in 0..3 {
            if !x[c].is_finite() || !y[c].is_finite() {
                return Err(invalid("rt shadow gate texel is not finite"));
            }
        }
        let d0 = (x[0] - y[0]).abs();
        let d1 = (x[1] - y[1]).abs();
        let d2 = (x[2] - y[2]).abs();
        if d0 > epsilon || d1 > epsilon || d2 > epsilon {
            changed += 1;
        }
    }
    Ok(changed)
}

/// EG8 verdict: a pass is live only if pixels change with it on versus off.
#[must_use]
pub fn is_live(changed_texels: usize) -> bool {
    changed_texels > 0
}

/// Ray-query visibility for one reservoir pick toward one torch.
///
/// Caller: `restir_di` reservoir visibility ray only; shadowed torches stay in Rt.
pub fn torch_visibility(
    pixel: usize,
    light: usize,
    shadow_on: bool,
) -> Result<bool, RtShadowError> {
    if !shadow_on {
        return Ok(true);
    }
    let ix = pixel.wrapping_rem(800);
    let iy = pixel.wrapping_div(800);
    if ix >= 800 || iy >= 600 || light >= 250 {
        return Err(invalid("torch pick past the frame"));
    }
    let fx = f32::from(u16::try_from(ix).unwrap_or(0));
    let fz = f32::from(u16::try_from(iy).unwrap_or(0));
    let gx = fx * 0.02 - 8.0;
    let gz = fz * 0.02 - 8.0;
    let lx = light.wrapping_rem(25);
    let lz = light.wrapping_div(25);
    let tx = f32::from(u16::try_from(lx).unwrap_or(0)) - 12.0;
    let tz = f32::from(u16::try_from(lz).unwrap_or(0)) * 1.6 - 8.0;
    let origin = [gx, RT_BIAS_M, gz];
    let dx = tx - gx;
    let dy = 3.0 - RT_BIAS_M;
    let dz = tz - gz;
    let len2 = dx * dx + dy * dy + dz * dz;
    if len2 < 1e-12 {
        return Ok(true);
    }
    let len = len2.sqrt();
    let dir = [dx / len, dy / len, dz / len];
    let h0 = ray_hits_box(origin, dir, &RT_BOXES[0], 0.0, RT_T_MAX_M).unwrap_or(false);
    let h1 = ray_hits_box(origin, dir, &RT_BOXES[1], 0.0, RT_T_MAX_M).unwrap_or(false);
    Ok(!(h0 || h1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_day::ARENA_DAY_SUN_DIR;

    fn frames_equal(a: &[[f32; 3]], b: &[[f32; 3]]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b.iter()).all(|(x, y)| {
                x[0].to_bits() == y[0].to_bits()
                    && x[1].to_bits() == y[1].to_bits()
                    && x[2].to_bits() == y[2].to_bits()
            })
    }

    fn render_on_off() -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
        let mut on = vec![[0.0f32; 3]; RT_FRAME_TEXELS];
        let mut off = vec![[0.0f32; 3]; RT_FRAME_TEXELS];
        render_sand_frame(ARENA_DAY_SUN_DIR, true, RT_AMBIENT_FLAT, &mut on)
            .expect("rig on-frame renders");
        render_sand_frame(ARENA_DAY_SUN_DIR, false, RT_AMBIENT_FLAT, &mut off)
            .expect("rig off-frame renders");
        (on, off)
    }

    #[test]
    fn rig_shadow_band_is_live() {
        let (on, off) = render_on_off();
        let changed = count_changed(&on, &off, RT_EPSILON).expect("gate runs on two frames");
        // Wall plus pillar shade ~18% of the sand: a band, never zero and
        // never the whole frame. Wide bounds so a rig tweak stays honest.
        assert!(
            (64..=384).contains(&changed),
            "shadow band moves {changed} texels, want a band not a dot or a flood"
        );
        assert!(is_live(changed), "moved pixels count the pass live");
        for (lit, dark) in off.iter().zip(on.iter()) {
            for c in 0..3 {
                assert!(
                    dark[c] <= lit[c] + RT_EPSILON,
                    "shadow never brightens sand"
                );
            }
        }
    }

    #[test]
    fn off_frame_is_flat_lit() {
        let (_, off) = render_on_off();
        let first = off[0];
        assert!(
            off.iter().all(|t| frames_equal(&[*t], &[first])),
            "forced-lit sand is one flat triple"
        );
        let oracle =
            sand_radiance(ARENA_DAY_SUN_DIR, 1.0, RT_AMBIENT_FLAT).expect("lit texel shades");
        assert!(
            frames_equal(&[first], &[oracle]),
            "flat off-frame equals the lit equation, not a second truth"
        );
        assert!(!is_live(0), "zero moved pixels stay a probe");
    }

    #[test]
    fn known_points_wall_shadow() {
        // Hand-derived from the pinned wall (x -4..4, y 0..3, z -0.5..0.5)
        // under the day-rig sun: the ray from (0, 0, -2.5) climbs through
        // the wall slab, the ray from (0, 0, 3.0) leaves past its far face.
        let dark = sand_shadow_factor([0.0, 0.0, -2.5], ARENA_DAY_SUN_DIR)
            .expect("wall-side point shades");
        assert_eq!(dark, 0.0, "sand behind the wall sits in shadow");
        let lit =
            sand_shadow_factor([0.0, 0.0, 3.0], ARENA_DAY_SUN_DIR).expect("sun-side point shades");
        assert_eq!(lit, 1.0, "sand on the sun side stays lit");
    }

    #[test]
    fn frames_repeat_bitwise() {
        let (first, _) = render_on_off();
        let mut second = vec![[0.0f32; 3]; RT_FRAME_TEXELS];
        render_sand_frame(ARENA_DAY_SUN_DIR, true, RT_AMBIENT_FLAT, &mut second)
            .expect("second on-frame renders");
        assert!(
            frames_equal(&first, &second),
            "same rig gives bit-identical frames twice"
        );
    }

    #[test]
    fn gate_floor_and_verdict() {
        let (on, off) = render_on_off();
        let same = count_changed(&on, &on, RT_EPSILON).expect("self-diff runs");
        assert_eq!((same, is_live(same)), (0, false), "self-diff is no change");
        let mut near = on.clone();
        near[0][0] += RT_EPSILON * 0.5;
        let under = count_changed(&on, &near, RT_EPSILON).expect("sub-epsilon diff runs");
        assert_eq!(under, 0, "dust under epsilon never counts live");
        let over = count_changed(&on, &off, RT_EPSILON).expect("on/off diff runs");
        assert!(over > 0 && is_live(over), "real change counts live");
    }

    #[test]
    fn slab_axes_agree() {
        let wall = &RT_BOXES[0];
        let down = ray_hits_box([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], wall, 0.0, 10.0)
            .expect("plumb ray tests");
        assert!(down, "plumb ray through the wall hits");
        let wide = ray_hits_box([9.0, 5.0, 0.0], [0.0, -1.0, 0.0], wall, 0.0, 10.0)
            .expect("offset ray tests");
        assert!(!wide, "parallel ray past the wall misses");
        let inside = ray_hits_box([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], wall, 0.0, 10.0)
            .expect("inside ray tests");
        assert!(inside, "origin inside the box counts hit");
    }

    #[test]
    fn rejects_bad_inputs() {
        assert!(unit_sun([0.0, 0.0, 0.0]).is_err(), "zero sun fails");
        assert!(unit_sun([f32::NAN, 1.0, 0.0]).is_err(), "NaN sun fails");
        assert!(unit_sun([1.0, 0.0, 0.0]).is_err(), "horizon sun fails");
        assert!(
            sand_shadow_factor([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]).is_err(),
            "bad sun fails the factor"
        );
        assert!(
            sand_radiance(ARENA_DAY_SUN_DIR, 2.0, RT_AMBIENT_FLAT).is_err(),
            "shadow past 1 fails"
        );
        assert!(
            sand_radiance(ARENA_DAY_SUN_DIR, 1.0, [-0.1, 0.0, 0.0]).is_err(),
            "negative ambient fails"
        );
        assert!(sand_texel_pos(32, 0).is_err(), "texel past the edge fails");
        let mut short = vec![[0.0f32; 3]; 8];
        assert!(
            render_sand_frame(ARENA_DAY_SUN_DIR, true, RT_AMBIENT_FLAT, &mut short).is_err(),
            "short buffer fails"
        );
    }

    #[test]
    fn rejects_bad_gate() {
        let (on, off) = render_on_off();
        let short = &off[..8];
        assert!(
            count_changed(&on, short, RT_EPSILON).is_err(),
            "ragged buffers fail"
        );
        for bad in [0.0, -0.01, f32::NAN, f32::INFINITY] {
            assert!(
                count_changed(&on, &off, bad).is_err(),
                "epsilon {bad} fails"
            );
        }
        let mut poisoned = on.clone();
        poisoned[7] = [f32::NAN, 0.0, 0.0];
        assert!(
            count_changed(&on, &poisoned, RT_EPSILON).is_err(),
            "NaN texels fail instead of passing"
        );
    }

    #[test]
    fn idx_f32_saturates_like_u16() {
        // Pinning: exact for frame sizes, saturates at `u16::MAX`, never
        // wraps. Bit-exact so the `assets::count_f32` twin keeps one owner.
        let top = f32::from(u16::MAX);
        for small in [0usize, 1, 31, 32, 1024] {
            let want = f32::from(u16::try_from(small).expect("small fits u16"));
            assert_eq!(
                idx_f32(small).to_bits(),
                want.to_bits(),
                "small index {small} stays exact"
            );
        }
        assert_eq!(
            idx_f32(usize::from(u16::MAX)).to_bits(),
            top.to_bits(),
            "top stays exact"
        );
        for big in [
            usize::from(u16::MAX) + 1,
            1_000_000,
            usize::try_from(u32::MAX).expect("u32 fits in usize"),
            usize::MAX,
        ] {
            assert_eq!(
                idx_f32(big).to_bits(),
                top.to_bits(),
                "large index {big} saturates at u16::MAX"
            );
        }
    }
}
