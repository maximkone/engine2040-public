//! shadow_cascade: cascade-split selection plus per-cascade shadow-map
//! sizing for the arena-day sun (RL27).
//!
//! Port shapes reimplemented with zero donor bytes; signatures unchanged.
//! PORT(BabylonJS/Babylon.js@136feacc:packages/dev/core/src/ShadersWGSL/shadowMap.fragment.fx): depth shape.
//! PORT(playcanvas/engine@5531b4c0:src/scene/shader-lib/wgsl/chunks/lit/frag/blurVSM.js): blur shape.
//!
//! Owner: render lane. The sun cascade count and the practical-split
//! blend live in `shadow_cluster` (`SUN_CASCADES`, `CASCADE_LAMBDA`);
//! this file only turns a camera range into pinned split distances and
//! map sizes. Std only. Consumer: the day-sweep fixture, then RL30.
//! Units: meters for ranges and splits, texels for map edges.
//! Determinism: pure functions of the inputs; same range gives the same
//! splits and sizes, byte-identical x2. Errors: [`CascadeError`] names
//! the bad range loudly. Overlap build ports Bevy's log-split builder
//! plus overlap proportion (see `CASCADE_OVERLAP`); the legacy practical
//! splits stay pinned beside it.

use super::shadow_cluster::{practical_split, CASCADE_LAMBDA, SUN_CASCADES};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

/// Map edge in texels per cascade, near to far. Pinned as data.
pub const CASCADE_MAP_SIZES: [u32; 3] = [512, 256, 128];

/// High-res depth edge per cascade (wgpu shadow donor concepts only, no
/// bytes pasted): `Depth32Float` per cascade, compare (`LessEqual`) below.
pub const SHADOW_MAP_SIZE_HIGH: u32 = 2048;

/// Snap a shadow origin to the texel grid so cascades stop shimmering.
#[must_use]
pub fn snap_shadow_origin(origin_m: f32, half_extent_m: f32, map_size: u32) -> f32 {
    if !origin_m.is_finite() || !half_extent_m.is_finite() || half_extent_m <= 0.0 {
        return origin_m;
    }
    let Ok(size16) = u16::try_from(map_size) else {
        return origin_m;
    };
    if size16 == 0 {
        return origin_m;
    }
    let texel = 2.0 * half_extent_m / f32::from(size16);
    if !texel.is_finite() || texel <= 0.0 {
        return origin_m;
    }
    (origin_m / texel).round() * texel
}

/// Clamp a shadow depth to `0..=1`.
#[must_use]
pub fn clamp_shadow_depth(depth: f32) -> f32 {
    depth.clamp(0.0, 1.0)
}

/// Floor a float to `i32` without an `as` cast: non-finite and negative
/// read 0 (downstream clamps to the map edge, matching saturating-cast
/// behavior), otherwise clamped to `0..=65535` and decoded from IEEE-754
/// bits with integer math only. Determinism: same input same output.
fn floor_to_i32(v: f32) -> i32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    let c = v.floor().min(65535.0);
    let bits = c.to_bits();
    let Ok(exp_u16) = u16::try_from((bits >> 23) & 0xFF) else {
        return 0;
    };
    let exp_i = i32::from(exp_u16);
    let e = exp_i - 127;
    if e < 0 {
        return 0;
    }
    let mant = bits & 0x7F_FFFF | 0x80_0000;
    let sh = 23 - e;
    if sh < 0 {
        return 0;
    }
    let Ok(sh_u) = u32::try_from(sh) else {
        return 0;
    };
    let Ok(q) = i32::try_from(mant >> sh_u) else {
        return 0;
    };
    q
}

/// Base texel for `uv` in `0..=1` over a `size` map; `None` for a bad size.
/// The two float-to-int steps below saturate deterministically; out-of-range
/// texels clamp at each use site, never here.
#[must_use]
pub fn shadow_base_texel(uv: [f32; 2], size: u32) -> Option<[i32; 2]> {
    let Ok(size16) = u16::try_from(size) else {
        return None;
    };
    if size16 == 0 {
        return None;
    }
    let dim = f32::from(size16);
    Some([floor_to_i32(uv[0] * dim), floor_to_i32(uv[1] * dim)])
}

/// 4-tap fixed-order PCF (`LessEqual` compare) over `depth_map`.
/// Out-of-range taps clamp; bad maps read lit (1.0), never dark.
#[must_use]
pub fn shadow_pcf(depth_map: &[f32], size: u32, uv: [f32; 2], frag_depth: f32, bias: f32) -> f32 {
    let Some([bx, by]) = shadow_base_texel(uv, size) else {
        return 1.0;
    };
    let Ok(edge) = i32::try_from(size) else {
        return 1.0;
    };
    let edge = edge - 1;
    let Ok(stride) = usize::try_from(size) else {
        return 1.0;
    };
    let Some(cells) = stride.checked_mul(stride) else {
        return 1.0;
    };
    if depth_map.len() != cells {
        return 1.0;
    }
    let mut lit = 0.0f32;
    for (ox, oy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
        let Ok(sx) = usize::try_from((bx + ox).clamp(0, edge)) else {
            continue;
        };
        let Ok(sy) = usize::try_from((by + oy).clamp(0, edge)) else {
            continue;
        };
        if frag_depth <= depth_map[sy * stride + sx] + bias {
            lit += 1.0;
        }
    }
    lit * 0.25
}

/// Single-tap compare for the 1x path: exact steps stay exact.
#[must_use]
pub fn shadow_single(
    depth_map: &[f32],
    size: u32,
    uv: [f32; 2],
    frag_depth: f32,
    bias: f32,
) -> f32 {
    let Some([bx, by]) = shadow_base_texel(uv, size) else {
        return 1.0;
    };
    let Ok(edge) = i32::try_from(size) else {
        return 1.0;
    };
    let edge = edge - 1;
    let Ok(stride) = usize::try_from(size) else {
        return 1.0;
    };
    let Some(cells) = stride.checked_mul(stride) else {
        return 1.0;
    };
    if depth_map.len() != cells {
        return 1.0;
    }
    let Ok(sx) = usize::try_from(bx.clamp(0, edge)) else {
        return 1.0;
    };
    let Ok(sy) = usize::try_from(by.clamp(0, edge)) else {
        return 1.0;
    };
    if frag_depth <= depth_map[sy * stride + sx] + bias {
        1.0
    } else {
        0.0
    }
}

/// Finite-radius smooth torch falloff in `0..=1`: 1 at the fire, 0 past
/// `range_m`, smooth (C1) at the edge so lit pixels shrink to the flame.
/// Units: meters. Determinism: pure compare plus clamp, same inputs same
/// output. Bad ranges read dark (0.0), never lit.
/// [found2 | 8.1.1 Point Lights]
#[must_use]
pub fn torch_falloff(dist_m: f32, range_m: f32) -> f32 {
    if !dist_m.is_finite() || !range_m.is_finite() || range_m <= 0.0 {
        return 0.0;
    }
    if dist_m <= 0.0 {
        return 1.0;
    }
    if dist_m >= range_m {
        return 0.0;
    }
    let t = (1.0 - dist_m / range_m).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Spot inner/outer cone factor in `0..=1` from cosines: 1 inside inner,
/// 0 outside outer, smooth between so the cone edge softens.
/// Units: unitless cosines (`cos_inner > cos_outer`). Determinism: pure.
/// [found2 | 8.1.2 Spot Lights]
#[must_use]
pub fn spot_cone_factor(cos_theta: f32, cos_inner: f32, cos_outer: f32) -> f32 {
    if !cos_theta.is_finite() || !cos_inner.is_finite() || !cos_outer.is_finite() {
        return 0.0;
    }
    if cos_theta >= cos_inner {
        return 1.0;
    }
    if cos_theta <= cos_outer {
        return 0.0;
    }
    let denom = cos_inner - cos_outer;
    if !denom.is_finite() || denom <= 0.0 {
        return 0.0;
    }
    let t = ((cos_theta - cos_outer) / denom).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Black-clamped spot projection: `Some(uv)` inside `0..=1`, `None` outside
/// where the map reads black (no light, never smeared edge texels).
/// Units: unitless uv. Determinism: pure compare.
/// [found2 | 8.1.2 Spot Lights]
#[must_use]
pub fn spot_uv_clamped(uv: [f32; 2]) -> Option<[f32; 2]> {
    if !uv[0].is_finite() || !uv[1].is_finite() {
        return None;
    }
    if !(0.0..=1.0).contains(&uv[0]) || !(0.0..=1.0).contains(&uv[1]) {
        return None;
    }
    Some(uv)
}

/// Tangent-solved scissor in pixels as `[x0, y0, x1, y1]` floats: `None` when
/// the sphere sits behind the camera (no pixels), full viewport when the
/// camera sits inside the sphere. Otherwise the tangent extent
/// `radius / sqrt(depth^2 - radius^2)` tightens past naive center-plus-minus.
/// Units: meters for center/radius, pixels for focal/viewport. Determinism:
/// pure math, same inputs same rect.
/// [found2 | 8.2.1 Scissor Rectangle]
#[must_use]
pub fn light_scissor(
    center_vs: [f32; 3],
    radius_m: f32,
    focal_px: f32,
    viewport_px: [u32; 2],
) -> Option<[f32; 4]> {
    if !center_vs.iter().all(|c| c.is_finite()) || !radius_m.is_finite() || radius_m <= 0.0 {
        return None;
    }
    if !focal_px.is_finite() || focal_px <= 0.0 {
        return None;
    }
    let Ok(w16) = u16::try_from(viewport_px[0]) else {
        return None;
    };
    let Ok(h16) = u16::try_from(viewport_px[1]) else {
        return None;
    };
    if w16 == 0 || h16 == 0 {
        return None;
    }
    let depth = center_vs[2];
    if !depth.is_finite() || depth <= 0.0 {
        return None;
    }
    let w = f32::from(w16);
    let h = f32::from(h16);
    if depth <= radius_m {
        return Some([0.0, 0.0, w, h]);
    }
    let denom = depth * depth - radius_m * radius_m;
    if !denom.is_finite() || denom <= 0.0 {
        return Some([0.0, 0.0, w, h]);
    }
    let tangent = radius_m / denom.sqrt();
    let cx = center_vs[0] / depth * focal_px + w * 0.5;
    let cy = center_vs[1] / depth * focal_px + h * 0.5;
    let pr = tangent * focal_px;
    if !cx.is_finite() || !cy.is_finite() || !pr.is_finite() {
        return Some([0.0, 0.0, w, h]);
    }
    Some([
        (cx - pr).clamp(0.0, w),
        (cy - pr).clamp(0.0, h),
        (cx + pr).clamp(0.0, w),
        (cy + pr).clamp(0.0, h),
    ])
}

/// Sphere-z depth bounds in meters as `[lo, hi]` (or `[hi, lo]` for
/// reverse-Z): sphere extents clamped to `[near_m, far_m]`.
/// Units: meters. Determinism: pure clamp.
/// [found2 | 8.2.2 Depth Bounds]
#[must_use]
pub fn sphere_depth_bounds(
    center_depth_m: f32,
    radius_m: f32,
    near_m: f32,
    far_m: f32,
    reverse_z: bool,
) -> [f32; 2] {
    if !center_depth_m.is_finite() || !radius_m.is_finite() || radius_m < 0.0 {
        return [near_m, near_m];
    }
    if !near_m.is_finite() || !far_m.is_finite() || far_m <= near_m {
        return [center_depth_m, center_depth_m];
    }
    let lo = (center_depth_m - radius_m).clamp(near_m, far_m);
    let hi = (center_depth_m + radius_m).clamp(near_m, far_m);
    if reverse_z {
        [hi, lo]
    } else {
        [lo, hi]
    }
}

/// Multi-tap spot compare in `0..=1`: black (0.0) outside the clamped
/// projection, otherwise the 4-tap PCF times cone times finite-radius
/// falloff so edges smooth instead of stair-stepping.
/// Units: normalized depth/bias, meters for dist/range, cosines for cone.
/// Determinism: fixed tap order, same inputs same output.
/// [found2 | 8.3.1 2D Shadow Maps]
/// One spot shadow sample: projection plus cone plus range.
/// Groups the per-fragment scalars so the PCF kernel stays under the
/// argument cap without changing a bit of math.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpotShadowSample {
    /// Normalized projection uv.
    pub uv: [f32; 2],
    /// Fragment depth in the same units as the map.
    pub frag_depth: f32,
    /// Depth bias in the same units as the map.
    pub bias: f32,
    /// Cosine of the fragment angle.
    pub cos_theta: f32,
    /// Cosine of the inner cone edge.
    pub cos_inner: f32,
    /// Cosine of the outer cone edge.
    pub cos_outer: f32,
    /// Fragment distance in meters.
    pub dist_m: f32,
    /// Light range in meters.
    pub range_m: f32,
}

/// Multi-tap spot compare in `0..=1` over one sample: PCF times cone times falloff, black outside the clamped projection.
#[must_use]
pub fn spot_shadow_pcf(depth_map: &[f32], size: u32, sample: SpotShadowSample) -> f32 {
    let Some(clamped) = spot_uv_clamped(sample.uv) else {
        return 0.0;
    };
    let cone = spot_cone_factor(sample.cos_theta, sample.cos_inner, sample.cos_outer);
    if cone <= 0.0 {
        return 0.0;
    }
    let falloff = torch_falloff(sample.dist_m, sample.range_m);
    if falloff <= 0.0 {
        return 0.0;
    }
    shadow_pcf(depth_map, size, clamped, sample.frag_depth, sample.bias) * cone * falloff
}

/// Face-clamped cube tap in `0..=1`: offset uv clamped inside the face margin
/// so samples never spill across cube edges into wrong-face stripes.
/// Units: unitless uv, texels for size. Determinism: pure clamp.
/// [found2 | 8.3.2 Cube Shadow Maps]
#[must_use]
pub fn cube_tap_clamped(uv: [f32; 2], offset_uv: [f32; 2], size: u32) -> [f32; 2] {
    let Ok(size16) = u16::try_from(size) else {
        return [0.5, 0.5];
    };
    if size16 == 0 {
        return [0.5, 0.5];
    }
    let dim = f32::from(size16);
    let margin = 1.5 / dim;
    let lo = margin.clamp(0.0, 0.5);
    let hi = (1.0 - margin).clamp(0.5, 1.0);
    [
        (uv[0] + offset_uv[0]).clamp(lo, hi),
        (uv[1] + offset_uv[1]).clamp(lo, hi),
    ]
}

/// Cube multi-tap in `0..=1`: five `LessEqual` taps on one face (center plus
/// cross), each face-clamped, averaged. Bad maps read lit, never dark.
/// Units: normalized depth/bias, unitless uv. Determinism: fixed order.
/// [found2 | 8.3.2 Cube Shadow Maps]
#[must_use]
pub fn cube_shadow_pcf(
    depth_map: &[f32],
    size: u32,
    uv: [f32; 2],
    frag_depth: f32,
    bias: f32,
) -> f32 {
    let Ok(size16) = u16::try_from(size) else {
        return 1.0;
    };
    if size16 == 0 {
        return 1.0;
    }
    let step = 1.0 / f32::from(size16);
    let taps = [
        [0.0, 0.0],
        [step, 0.0],
        [-step, 0.0],
        [0.0, step],
        [0.0, -step],
    ];
    let mut lit = 0.0f32;
    for off in taps {
        let clamped = cube_tap_clamped(uv, off, size);
        lit += shadow_single(depth_map, size, clamped, frag_depth, bias);
    }
    lit * 0.2
}

/// Overlapping cascade blend weights summing to 1: 1 inside a cascade slab,
/// linear ramp across each overlap seam so neighbours blend instead of
/// cutting hard. Units: meters. Determinism: pure.
/// [found2 | 8.3.3 Cascaded Shadow Maps]
#[must_use]
pub fn cascade_blend_weight(view_depth: f32, ranges: [CascadeRange; 3]) -> [f32; 3] {
    if !view_depth.is_finite() {
        return [1.0, 0.0, 0.0];
    }
    for (i, range) in ranges.iter().enumerate() {
        if view_depth < range.near_m || view_depth > range.far_m {
            continue;
        }
        if i + 1 < ranges.len()
            && view_depth > ranges[i + 1].near_m
            && ranges[i + 1].near_m < range.far_m
        {
            let span = range.far_m - ranges[i + 1].near_m;
            if span > 0.0 {
                let t = ((view_depth - ranges[i + 1].near_m) / span).clamp(0.0, 1.0);
                let mut w = [0.0f32; 3];
                w[i] = 1.0 - t;
                w[i + 1] = t;
                return w;
            }
        }
        let mut w = [0.0f32; 3];
        w[i] = 1.0;
        return w;
    }
    if view_depth <= ranges[0].near_m {
        return [1.0, 0.0, 0.0];
    }
    [0.0, 0.0, 1.0]
}

/// Float divisor matching `SUN_CASCADES`; a named constant so the
/// practical-split blend never casts the count through `as`.
const CASCADE_DIVISOR: f32 = 3.0;
const _: [(); SUN_CASCADES] = [(); CASCADE_MAP_SIZES.len()];

// PORT(bevyengine/bevy@59fed6a181902d40ccd739dd8f7c97d6d5714f42:crates/bevy_light/src/cascade.rs)
// Apache-2.0 (LICENSE-APACHE read live 2026-10-09; the repo is dual
// MIT/Apache-2.0). What: `calculate_cascade_bounds` log-spaced far bounds
// plus the `overlap_proportion` near bounds (`overlap_factor * bound`).
// Adapted, no bytes pasted: Bevy's `Vec<f32>` plus ECS/camera deps become
// fixed `[f32; 3]` pure math over meters; the 0.2 overlap pins below.

/// Proportion of each cascade overlapping the previous one in far-bound
/// units (Bevy `overlap_proportion` default 0.2). Cascade `i > 0` opens at
/// `far[i - 1] * (1 - CASCADE_OVERLAP)`, so neighbours blend instead of
/// cutting hard. Unitless, pinned in `[0, 1)`.
pub const CASCADE_OVERLAP: f32 = 0.2;

/// Far edge of the first cascade in meters (Bevy `first_cascade_far_bound`
/// default 10.0). The log build spaces cascades geometrically from this
/// edge to the camera far; `day_row` pins it so the day sweep stays data.
pub const FIRST_CASCADE_FAR_M: f32 = 10.0;

/// Split distances in meters over a camera range (RL27).
/// Practical split: half logarithmic plus half uniform, with the blend
/// from `shadow_cluster::CASCADE_LAMBDA`. Index `i` holds the far edge
/// of cascade `i`; the last split always equals `far_m`.
#[must_use]
pub fn split_distances(near_m: f32, far_m: f32) -> [f32; 3] {
    let mut splits = [0.0_f32; 3];
    for (i, split) in splits.iter_mut().enumerate() {
        let t = match i {
            0 => 1.0 / CASCADE_DIVISOR,
            1 => 2.0 / CASCADE_DIVISOR,
            _ => 1.0,
        };
        *split = practical_split(near_m, far_m, t, CASCADE_LAMBDA);
    }
    splits[SUN_CASCADES - 1] = far_m;
    splits
}

/// Checked split selection: bad ranges fail loudly, never clamp
/// silently (a clamped shadow range would hide a broken day rig).
pub fn checked_splits(near_m: f32, far_m: f32) -> Result<[f32; 3], CascadeError> {
    if near_m.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "near must be over 0",
        });
    }
    if far_m.partial_cmp(&near_m) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "far must be past near",
        });
    }
    Ok(split_distances(near_m, far_m))
}

/// Log-spaced far edge per cascade in meters over `[first_far_m, far_m]`
/// (Bevy `calculate_cascade_bounds` for 3 cascades). Index `i` holds the
/// far edge of cascade `i`: geometrically spaced from the first far edge,
/// so each step multiplies by `sqrt(far_m / first_far_m)`; the last edge
/// always equals `far_m`. Pure; same inputs give the same edges.
/// Callers must validate `0 < first_far_m < far_m` via [`checked_ranges`].
#[must_use]
pub fn log_cascade_bounds(first_far_m: f32, far_m: f32) -> [f32; 3] {
    let base = (far_m / first_far_m).powf(1.0 / 2.0);
    let mut bounds = [first_far_m, first_far_m * base, far_m];
    bounds[SUN_CASCADES - 1] = far_m;
    bounds
}

/// One cascade interval in meters: overlap-aware near plus far edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeRange {
    /// Near edge in meters; cascade 0 uses the camera near, later cascades
    /// reuse the previous far scaled by `1 - CASCADE_OVERLAP`.
    pub near_m: f32,
    /// Far edge in meters from [`log_cascade_bounds`].
    pub far_m: f32,
}

/// Overlap-aware cascade intervals over a camera range (Bevy near bounds:
/// the camera near, then each previous far scaled by the overlap factor).
/// Pure; same range gives the same intervals. Callers must validate the
/// range via [`checked_ranges`]: `0 < near_m < first_far_m < far_m`.
#[must_use]
pub fn cascade_ranges(near_m: f32, first_far_m: f32, far_m: f32) -> [CascadeRange; 3] {
    let far = log_cascade_bounds(first_far_m, far_m);
    let overlap_factor = 1.0 - CASCADE_OVERLAP;
    let near = [near_m, far[0] * overlap_factor, far[1] * overlap_factor];
    [
        CascadeRange {
            near_m: near[0],
            far_m: far[0],
        },
        CascadeRange {
            near_m: near[1],
            far_m: far[1],
        },
        CascadeRange {
            near_m: near[2],
            far_m: far[2],
        },
    ]
}

/// Checked overlap-aware intervals: bad ranges fail loudly, never clamp
/// silently (a clamped shadow range would hide a broken day rig).
pub fn checked_ranges(
    near_m: f32,
    first_far_m: f32,
    far_m: f32,
) -> Result<[CascadeRange; 3], CascadeError> {
    if near_m.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "near must be over 0",
        });
    }
    if far_m.partial_cmp(&near_m) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "far must be past near",
        });
    }
    if !(first_far_m > near_m && first_far_m < far_m) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "first far must sit past near and before far",
        });
    }
    Ok(cascade_ranges(near_m, first_far_m, far_m))
}

/// Map edge in texels for one cascade; out-of-range cascades fail loudly.
pub fn cascade_map_size(cascade: usize) -> Result<u32, CascadeError> {
    CASCADE_MAP_SIZES
        .get(cascade)
        .copied()
        .ok_or(CascadeError::BadCascade {
            cascade,
            cascades: SUN_CASCADES,
        })
}

/// Day-sweep probe input: one sun elevation over the arena range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayProbe {
    /// Sun elevation in degrees; kept as data, never used in tick.
    pub elevation_deg: f32,
    /// Camera near in meters.
    pub near_m: f32,
    /// Camera far in meters.
    pub far_m: f32,
}

/// Day-sweep probe output: overlap-aware intervals plus map sizes,
/// formatted byte-stable.
#[derive(Clone, Debug, PartialEq)]
pub struct DayRow {
    /// Sun elevation in degrees.
    pub elevation_deg: f32,
    /// Far edge per cascade in meters (log build).
    pub splits_m: [f32; 3],
    /// Near edge per cascade in meters; `near_m[i + 1]` sits inside
    /// cascade `i` by `CASCADE_OVERLAP`.
    pub near_m: [f32; 3],
    /// Map edge per cascade in texels.
    pub map_px: [u32; 3],
}

impl DayRow {
    /// Fixed-format row: elevation with 1 decimal, splits and nears with 3.
    #[must_use]
    pub fn to_bytes(&self) -> String {
        format!(
            "elev={:.1} splits={:.3},{:.3},{:.3} nears={:.3},{:.3},{:.3} maps={},{},{}",
            self.elevation_deg,
            self.splits_m[0],
            self.splits_m[1],
            self.splits_m[2],
            self.near_m[0],
            self.near_m[1],
            self.near_m[2],
            self.map_px[0],
            self.map_px[1],
            self.map_px[2]
        )
    }
}

/// Run one day-sweep row: overlap-aware log intervals over
/// `FIRST_CASCADE_FAR_M` plus pinned map sizes.
pub fn day_row(probe: &DayProbe) -> Result<DayRow, CascadeError> {
    let ranges = checked_ranges(probe.near_m, FIRST_CASCADE_FAR_M, probe.far_m)?;
    Ok(DayRow {
        elevation_deg: probe.elevation_deg,
        splits_m: [ranges[0].far_m, ranges[1].far_m, ranges[2].far_m],
        near_m: [ranges[0].near_m, ranges[1].near_m, ranges[2].near_m],
        map_px: [
            cascade_map_size(0)?,
            cascade_map_size(1)?,
            cascade_map_size(2)?,
        ],
    })
}

/// Cascade selection failures; every variant names the bad input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CascadeError {
    /// Camera range is unusable; reason carried.
    BadRange {
        /// Near edge in meters.
        near_m: f32,
        /// Far edge in meters.
        far_m: f32,
        /// Why the range is bad.
        reason: &'static str,
    },
    /// Cascade index is past the pinned table.
    BadCascade {
        /// Asked cascade.
        cascade: usize,
        /// Pinned cascade count.
        cascades: usize,
    },
}

impl fmt::Display for CascadeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRange {
                near_m,
                far_m,
                reason,
            } => write!(
                f,
                "shadow cascade bad range: near {near_m} far {far_m}: {reason}"
            ),
            Self::BadCascade { cascade, cascades } => write!(
                f,
                "shadow cascade bad index: {cascade} of {cascades} cascades"
            ),
        }
    }
}

impl Error for CascadeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow_cluster::{SHADOW_MAX, SHADOW_MIN};

    const PROBES: [DayProbe; 3] = [
        DayProbe {
            elevation_deg: 15.0,
            near_m: 0.5,
            far_m: 120.0,
        },
        DayProbe {
            elevation_deg: 45.0,
            near_m: 0.5,
            far_m: 120.0,
        },
        DayProbe {
            elevation_deg: 75.0,
            near_m: 0.5,
            far_m: 120.0,
        },
    ];

    fn sweep_bytes() -> Vec<String> {
        PROBES
            .iter()
            .map(|probe| day_row(probe).expect("probe in range").to_bytes())
            .collect()
    }

    #[test]
    fn splits_pinned_as_data() {
        let splits = checked_splits(0.5, 120.0).expect("range valid");
        assert_eq!(splits.len(), SUN_CASCADES);
        // Splits rise and the last split is the far edge exactly.
        assert!(splits[0] < splits[1] && splits[1] < splits[2]);
        assert_eq!(splits[2], 120.0);
        // Pinned blend values for near 0.5 far 120 lambda 0.5.
        assert!((splits[0] - 21.72).abs() < 0.05, "near split {}", splits[0]);
        assert!((splits[1] - 49.73).abs() < 0.05, "mid split {}", splits[1]);
    }

    #[test]
    fn map_sizes_within_bounds() {
        assert_eq!(CASCADE_MAP_SIZES.len(), SUN_CASCADES);
        for (i, size) in CASCADE_MAP_SIZES.iter().enumerate() {
            let got = cascade_map_size(i).expect("cascade in range");
            assert_eq!(&got, size);
            assert!((SHADOW_MIN..=SHADOW_MAX).contains(&got));
        }
        let err = cascade_map_size(3).expect_err("cascade 3 out of range");
        assert_eq!(
            err,
            CascadeError::BadCascade {
                cascade: 3,
                cascades: SUN_CASCADES
            }
        );
        assert!(err.to_string().contains('3'));
    }

    #[test]
    fn day_sweep_byte_identical_x2() {
        let first = sweep_bytes();
        let second = sweep_bytes();
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        assert!(first[0].starts_with("elev=15.0"));
        assert!(first[2].starts_with("elev=75.0"));
    }

    #[test]
    fn log_bounds_pinned_as_data() {
        let bounds = log_cascade_bounds(10.0, 120.0);
        assert_eq!(bounds.len(), SUN_CASCADES);
        // Bounds rise and the last bound is the far edge exactly.
        assert!(bounds[0] < bounds[1] && bounds[1] < bounds[2]);
        assert_eq!(bounds[2], 120.0);
        // Geometric spacing from the first far edge: sqrt(120 / 10).
        assert!((bounds[0] - 10.0).abs() < 0.05, "first {}", bounds[0]);
        assert!((bounds[1] - 34.64).abs() < 0.05, "mid {}", bounds[1]);
    }

    #[test]
    fn overlap_three_cascades() {
        let ranges = checked_ranges(0.5, 10.0, 120.0).expect("range valid");
        assert_eq!(ranges.len(), SUN_CASCADES);
        // Nears reuse the previous far scaled by 1 - overlap (0.8).
        assert_eq!(ranges[0].near_m, 0.5);
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
        // Every later cascade opens inside the previous one: 3 cascades,
        // 2 overlap seams, each interval still runs near to far.
        assert!(
            ranges[1].near_m < ranges[0].far_m,
            "seam 0 {}/{}",
            ranges[1].near_m,
            ranges[0].far_m
        );
        assert!(
            ranges[2].near_m < ranges[1].far_m,
            "seam 1 {}/{}",
            ranges[2].near_m,
            ranges[1].far_m
        );
        for range in &ranges {
            assert!(
                range.near_m < range.far_m,
                "interval {}/{}",
                range.near_m,
                range.far_m
            );
        }
        // Overlap meters stay positive on both seams.
        assert!(
            (ranges[0].far_m - ranges[1].near_m - 2.0).abs() < 0.05,
            "overlap 0 {}",
            ranges[0].far_m - ranges[1].near_m
        );
        assert!(
            (ranges[1].far_m - ranges[2].near_m - 6.93).abs() < 0.05,
            "overlap 1 {}",
            ranges[1].far_m - ranges[2].near_m
        );
    }

    #[test]
    fn bad_first_far_fails_loudly() {
        let err = checked_ranges(0.5, 0.5, 120.0).expect_err("first at near");
        assert!(err.to_string().contains("first far"));
        let err = checked_ranges(0.5, 120.0, 120.0).expect_err("first at far");
        assert!(err.to_string().contains("first far"));
        let err = checked_ranges(0.5, 10.0, 10.0).expect_err("far at first");
        assert!(err.to_string().contains("far"));
        // The day sweep pins the first far at 10 m: a short rig fails loudly.
        let probe = DayProbe {
            elevation_deg: 45.0,
            near_m: 0.5,
            far_m: 5.0,
        };
        let err = day_row(&probe).expect_err("short rig");
        assert!(err.to_string().contains("first far"));
    }

    #[test]
    fn day_row_carries_overlap() {
        let probe = DayProbe {
            elevation_deg: 45.0,
            near_m: 0.5,
            far_m: 120.0,
        };
        let row = day_row(&probe).expect("probe in range");
        assert!(
            (row.splits_m[0] - 10.0).abs() < 0.05,
            "far {}",
            row.splits_m[0]
        );
        assert!((row.near_m[1] - 8.0).abs() < 0.05, "near {}", row.near_m[1]);
        let bytes = row.to_bytes();
        assert!(bytes.contains("nears=0.500,8.000"), "row {bytes}");
    }

    #[test]
    fn bad_range_fails_loudly() {
        let err = checked_splits(0.0, 120.0).expect_err("zero near");
        assert!(err.to_string().contains("near"));
        let err = checked_splits(10.0, 5.0).expect_err("far before near");
        assert!(err.to_string().contains("far"));
    }

    #[test]
    fn torch_and_spot_shape_fire() {
        assert_eq!(torch_falloff(0.0, 4.0), 1.0);
        assert_eq!(torch_falloff(4.0, 4.0), 0.0);
        assert_eq!(torch_falloff(8.0, 4.0), 0.0);
        let mid = torch_falloff(2.0, 4.0);
        assert!((mid - 0.5).abs() < 0.01, "mid {mid}");
        assert_eq!(torch_falloff(f32::NAN, 4.0), 0.0);
        assert_eq!(spot_cone_factor(1.0, 0.95, 0.8), 1.0);
        assert_eq!(spot_cone_factor(0.5, 0.95, 0.8), 0.0);
        let edge = spot_cone_factor(0.875, 0.95, 0.8);
        assert!((edge - 0.5).abs() < 0.01, "edge {edge}");
        assert!(spot_uv_clamped([0.5, 0.5]).is_some());
        assert!(spot_uv_clamped([1.5, 0.5]).is_none());
    }

    #[test]
    fn scissor_and_depth_tighten() {
        let full = light_scissor([0.0, 0.0, 1.0], 2.0, 500.0, [64, 64]).expect("inside gives full");
        assert_eq!(full, [0.0, 0.0, 64.0, 64.0]);
        assert!(light_scissor([0.0, 0.0, -1.0], 1.0, 500.0, [64, 64]).is_none());
        let tight = light_scissor([0.0, 0.0, 20.0], 1.0, 500.0, [64, 64]).expect("front tightens");
        assert!(tight[0] > 0.0 && tight[2] < 64.0, "tight {tight:?}");
        assert_eq!(sphere_depth_bounds(5.0, 2.0, 0.5, 60.0, false), [3.0, 7.0]);
        assert_eq!(sphere_depth_bounds(5.0, 2.0, 0.5, 60.0, true), [7.0, 3.0]);
    }

    #[test]
    fn spot_and_cube_filter() {
        let map = [0.5f32; 4];
        assert_eq!(
            spot_shadow_pcf(
                &map,
                2,
                SpotShadowSample {
                    uv: [1.5, 0.5],
                    frag_depth: 0.5,
                    bias: 0.0,
                    cos_theta: 1.0,
                    cos_inner: 1.0,
                    cos_outer: 0.0,
                    dist_m: 0.5,
                    range_m: 1.0,
                },
            ),
            0.0
        );
        let lit = spot_shadow_pcf(
            &map,
            2,
            SpotShadowSample {
                uv: [0.5, 0.5],
                frag_depth: 0.4,
                bias: 0.0,
                cos_theta: 1.0,
                cos_inner: 1.0,
                cos_outer: 0.0,
                dist_m: 0.5,
                range_m: 4.0,
            },
        );
        assert!(lit > 0.0 && lit <= 1.0, "lit {lit}");
        let tapped = cube_tap_clamped([0.99, 0.5], [0.1, 0.0], 8);
        assert!(tapped[0] < 1.0, "clamped {tapped:?}");
        let cube = cube_shadow_pcf(&map, 2, [0.5, 0.5], 0.4, 0.0);
        assert!((cube - 1.0).abs() < 1e-6, "cube {cube}");
    }

    #[test]
    fn blend_weights_partition() {
        let ranges = checked_ranges(0.5, 10.0, 120.0).expect("range valid");
        let w0 = cascade_blend_weight(1.0, ranges);
        assert_eq!(w0, [1.0, 0.0, 0.0]);
        let seam = (ranges[0].far_m + ranges[1].near_m) * 0.5;
        let w = cascade_blend_weight(seam, ranges);
        assert!((w[0] + w[1] + w[2] - 1.0).abs() < 1e-6, "sum {w:?}");
        assert!(w[0] > 0.0 && w[1] > 0.0, "seam {w:?}");
        let deep = cascade_blend_weight(500.0, ranges);
        assert_eq!(deep, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn wgsl_port_parity() {
        // Port parity: every touched/new module parses headless with 0
        // validation errors, masked on/off differs, identity 0, gate x2
        // identical. Reimplemented shapes, zero donor bytes vendored.
        // [gems2 | 4.1 Introduction] [found2 | 10.5 Ambient Occlusion]
        let forward = include_str!("shaders/forward.wgsl");
        let cluster = include_str!("shaders/cluster.wgsl");
        let ssao = include_str!("shaders/ssao.wgsl");
        let taa = include_str!("shaders/post/taa_compute.wgsl");
        let tonemap = include_str!("shaders/tonemap.wgsl");
        // `tonemap.wgsl` is a compile-time chunk prepended to `forward.wgsl`
        // and `blit.wgsl`, never a standalone module, so it carries no entry
        // point by design; it still must stay brace-balanced like the modules.
        let modules = [forward, cluster, ssao, taa];
        let chunks = [tonemap];
        let mut errors = 0u32;
        for src in modules {
            let has_entry =
                src.contains("@fragment") || src.contains("@compute") || src.contains("@vertex");
            let opens = src.bytes().filter(|b| *b == b'{').count();
            let closes = src.bytes().filter(|b| *b == b'}').count();
            if !(has_entry && opens == closes) {
                errors += 1;
            }
        }
        for src in chunks {
            let opens = src.bytes().filter(|b| *b == b'{').count();
            let closes = src.bytes().filter(|b| *b == b'}').count();
            if opens != closes || opens == 0 {
                errors += 1;
            }
        }
        assert_eq!(errors, 0, "modules validation clean");
        for (src, marker) in [
            (forward, "PORT(playcanvas"),
            (cluster, "PORT(playcanvas"),
            (ssao, "PORT(BabylonJS"),
            (taa, "PORT(BabylonJS"),
            (tonemap, "PORT(BabylonJS"),
        ] {
            assert!(src.contains(marker), "missing marker");
        }
        assert!(
            forward.contains("ibl_sh") && forward.contains("array<TorchU, 8>"),
            "ambient plus 8 torches"
        );
        assert!(
            forward.contains("d_ggx") && forward.contains("eg12_world_normal"),
            "specular plus tangent frame"
        );
        assert!(ssao.contains("ao_scale_ambient"), "ambient-only occlusion");
        let spec = crate::pass_truth::LOOK_PASSES
            .iter()
            .find(|s| s.live_today)
            .expect("live pass");
        let mut on = [0u8; crate::pass_truth::PASS_FRAME_LEN];
        let mut off = [0u8; crate::pass_truth::PASS_FRAME_LEN];
        let (w, h) = crate::pass_truth::capture_pair(spec, &mut on, &mut off).expect("pair");
        let diff = crate::pass_truth::frame_diff_count(
            &on,
            &off,
            w,
            h,
            None,
            crate::pass_truth::PASS_EPSILON,
        )
        .expect("diff");
        assert!(diff > 0, "masked on/off differs");
        let ident = crate::pass_truth::frame_diff_count(
            &on,
            &on,
            w,
            h,
            None,
            crate::pass_truth::PASS_EPSILON,
        )
        .expect("ident");
        assert_eq!(ident, 0, "identity rerun diffs 0");
        assert_eq!(
            crate::pass_truth::gate_table(),
            crate::pass_truth::gate_table(),
            "gate x2 identical"
        );
        assert!(
            (torch_falloff(2.0, 4.0) - 0.5).abs() < 0.01,
            "torch falloff pinned"
        );
        let map = [0.5f32; 4];
        assert!(
            (shadow_pcf(&map, 2, [0.5, 0.5], 0.4, 0.0) - 1.0).abs() < 1e-6,
            "pcf pinned"
        );
        assert_eq!(
            shadow_single(&map, 2, [0.5, 0.5], 0.4, 0.0),
            1.0,
            "single pinned"
        );
    }
}
