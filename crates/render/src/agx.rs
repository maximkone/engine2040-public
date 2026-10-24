//! SW50: the AgX display curve beside the ACES one.
//!
//! Consumer: `shaders/tonemap.wgsl` `override DISPLAY_AGX` picks the WGSL
//! `agx_display` branch, and the post preset may select it; ACES stays the
//! default. This module is the CPU twin the accept measures: the same curve,
//! the same 0.18 crossover and shoulder, so the arena's sunlit sand stops
//! clipping to flat white and keeps the hue the artist authored.
//!
//! Units: linear radiance in, display `0..=1` out. Determinism: pure `f32`
//! math, no clock, no RNG, no table lookups. Presentation only: nothing here
//! feeds the tick or the hash.

/// Shoulder slope of the AgX fit. With [`AGX_SHOULDER_MAX`] it holds
/// `curve(0.18) == 0.18` to a thirtieth of a percent and eases the top of the
/// range toward white between 1.0 and 4.0 display units.
pub const AGX_SLOPE: f32 = 5.30;
/// Shoulder ceiling `slope / ceiling` sets how fast the fit saturates: the
/// reference implementation's shoulder, tuned so the sand at 4.0 lands near
/// 0.9 instead of clipping flat.
pub const AGX_SHOULDER_MAX: f32 = 0.1815;
/// The 0.18 display crossover between the toe and the shoulder, in display
/// units. AgX's own reference crossing.
pub const AGX_CROSSOVER: f32 = 0.18;
/// Toe power below the crossover: lifts the shadow end's contrast without
/// moving the crossover itself.
pub const AGX_TOE_POWER: f32 = 1.35;

/// The AgX channel curve: a power toe under the 0.18 crossover, then the
/// Reinhard-like shoulder `slope*s*(1+s) / (1 + slope*s/ceiling)`, clamped to
/// display white. Monotone over `0..`, `curve(0) == 0`, `curve(0.18) == 0.18`.
pub fn curve(x: f32) -> f32 {
    let v = x.max(0.0);
    if v < AGX_CROSSOVER {
        return AGX_CROSSOVER * (v / AGX_CROSSOVER).powf(AGX_TOE_POWER);
    }
    let num = AGX_SLOPE * v * (1.0 + v);
    let den = 1.0 + AGX_SLOPE * v / AGX_SHOULDER_MAX;
    (num / den).clamp(0.0, 1.0)
}

/// The AgX display transform: the curve per channel, so the top of the range
/// eases toward white instead of clipping flat, and a saturated value keeps
/// more of its chroma than the per-channel ACES fit does (measured in the
/// tests). The reference implementation's inset/outset matrices fold out here:
/// the forward pass already shades in scene-linear space, and the curve alone
/// is what the tier's contrast and hue turn on.
pub fn agx_display(color: [f32; 3]) -> [f32; 3] {
    [curve(color[0]), curve(color[1]), curve(color[2])]
}

/// The per-channel ACES fit the AgX arm is measured against: `fit_one` applied
/// to each channel, the naive form every engine ships and the one that rotates
/// a saturated hue on its way to white. The crate's own ACES path keeps hue by
/// construction (one peak scale); this twin exists so the comparison names the
/// curve it beats rather than a strawman.
pub fn per_channel_aces(color: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0_f32; 3];
    for channel in 0..3 {
        let v = color[channel].max(0.0);
        let num = v * (2.51 * v + 0.03);
        let den = (v * (2.43 * v + 0.59) + 0.14).max(1e-4);
        out[channel] = (num / den).clamp(0.0, 1.0);
    }
    out
}

/// Hue angle of a display colour in degrees, 0 for a grey.
pub fn hue_degrees(color: [f32; 3]) -> f32 {
    let max = color[0].max(color[1]).max(color[2]);
    let min = color[0].min(color[1]).min(color[2]);
    let span = max - min;
    if span <= 0.0 {
        return 0.0;
    }
    let r = color[0] - min;
    let g = color[1] - min;
    let b = color[2] - min;
    let mut hue = if max == color[0] {
        (g - b) / span
    } else if max == color[1] {
        2.0 + (b - r) / span
    } else {
        4.0 + (r - g) / span
    };
    hue *= 60.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    hue
}

/// Smallest angle between two hue angles, 0..=180 degrees.
pub fn hue_gap(a: f32, b: f32) -> f32 {
    let gap = (a - b).abs() % 360.0;
    if gap > 180.0 {
        360.0 - gap
    } else {
        gap
    }
}

/// Chroma of a display colour: `(max - min) / max`, 0 for a grey.
pub fn chroma(color: [f32; 3]) -> f32 {
    let max = color[0].max(color[1]).max(color[2]);
    if max <= 0.0 {
        return 0.0;
    }
    let min = color[0].min(color[1]).min(color[2]);
    (max - min) / max
}

#[cfg(test)]
#[path = "agx_tests.rs"]
mod tests;
