//! bloom_dual: the live frame's dual-filter bloom plan, and its CPU oracle.
//!
//! Shape (Bevy `bloom.wesl`, Jimenez 2014): a Karis-weighted bright mip at
//! half the frame size, a 13-tap downsample ladder under it, then a 3x3 tent
//! upsample ladder that folds each smaller mip back onto the larger one. The
//! total is the halo the blit adds before the one tone map.
//!
//! Owner: render lane. Consumers: the device chain in
//! `present::gpu_present::bloom_dual` sizes its textures from
//! `DualBloomPlan` and runs the same math in WGSL; the blit samples the
//! half-res halo and adds it at `BloomConfig::intensity`. The oracle beside
//! the plan (test builds only) is the reference the device test compares.
//!
//! Units: linear HDR radiance in and out; `f32` throughout (the device stores
//! each mip as `Rgba16Float`, so a readback is the f32 value rounded to 11
//! significant bits, inside `GPU_PARITY_TOL_POST`). Determinism: pure, no
//! device, no clock, same input gives the same output twice. Presentation
//! only, never the tick hash.
//!
//! Every tap is a load or a linear sample at an exact power-of-two fraction
//! (0.5 for the 13-tap, 0.25/0.75 for the tent), so the oracle and the shader
//! compute the same bilinear weights.

use crate::post::{BloomConfig, PostError};

/// Mip levels the live chain may hold: mip 0 is half the frame, each later
/// mip halves again. The cap is [`crate::post::MAX_BLOOM_LEVELS`] via the
/// config; this is the device texture count.
pub const DUAL_BLOOM_MAX_LEVELS: u32 = crate::post::MAX_BLOOM_LEVELS;

/// Rec. 709 luma weights, the one set the crate's bloom taps use.
pub const BLOOM_LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// The five 13-tap weights: the four corner groups take `1/32` per tap and
/// the centre group takes `1/8`, so the thirteen taps sum to 1.
pub const DOWNSAMPLE_CORNER_WEIGHT: f32 = 1.0 / 32.0;

/// Centre-group tap weight of the 13-tap filter.
pub const DOWNSAMPLE_CENTRE_WEIGHT: f32 = 1.0 / 8.0;

/// One frame of the chain: the config values plus both frame sizes.
/// Sizing only (no texels): the device builds one texture per mip from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualBloomPlan {
    /// Knee centre in linear HDR luma.
    pub threshold: f32,
    /// Soft-knee fraction in `0..=1`.
    pub knee: f32,
    /// Halo add-back strength, `0..=2`.
    pub intensity: f32,
    /// Mips in the ladder, `1..=DUAL_BLOOM_MAX_LEVELS`; mip 0 is half size.
    pub levels: u32,
    /// Frame width in pixels (mip 0 is this over two).
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

impl DualBloomPlan {
    /// Build from the frame size and the one bloom config. Fails loudly on a
    /// zero frame or a bad config, so a caller never guesses a level count.
    pub fn for_frame(width: u32, height: u32, cfg: &BloomConfig) -> Result<Self, PostError> {
        cfg.validate()?;
        if width == 0 || height == 0 {
            return Err(PostError::Invalid("bloom frame size is zero".into()));
        }
        let mut levels = 0u32;
        let (mut w, mut h) = (width.div_ceil(2).max(1), height.div_ceil(2).max(1));
        let cap = cfg.levels.clamp(1, DUAL_BLOOM_MAX_LEVELS);
        loop {
            levels = levels.saturating_add(1);
            if levels >= cap || (w == 1 && h == 1) {
                break;
            }
            w = w.div_ceil(2).max(1);
            h = h.div_ceil(2).max(1);
        }
        Ok(Self {
            threshold: cfg.threshold,
            knee: cfg.knee,
            intensity: cfg.intensity,
            levels,
            width,
            height,
        })
    }

    /// Size of one mip in texels; mip 0 is half the frame.
    #[must_use]
    pub fn level_size(&self, level: u32) -> (u32, u32) {
        let (mut w, mut h) = (self.width.max(1), self.height.max(1));
        for _ in 0..=level.min(self.levels.saturating_sub(1)) {
            w = w.div_ceil(2).max(1);
            h = h.div_ceil(2).max(1);
        }
        (w, h)
    }

    /// Every mip's size, mip 0 first. Cold path: allocates the plan vector.
    #[must_use]
    pub fn level_sizes(&self) -> Vec<(u32, u32)> {
        (0..self.levels)
            .map(|level| self.level_size(level))
            .collect()
    }
}

/// The CPU oracle: the same chain on an `f32` image, texel for texel with
/// the shaders. Test builds only: it is the reference the device parity test
/// compares, never part of a shipped frame.
#[cfg(test)]
pub(crate) mod oracle {
    use super::*;

    /// Linear bilinear sample of one `[f32; 3]` image at a texel-centre
    /// coordinate, clamped to the edge exactly like `AddressMode::ClampToEdge`.
    fn bilinear(src: &[[f32; 3]], width: u32, height: u32, x: f32, y: f32) -> [f32; 3] {
        let w = width.max(1);
        let h = height.max(1);
        let last_x = f32::from(u16::try_from(w - 1).unwrap_or(0));
        let last_y = f32::from(u16::try_from(h - 1).unwrap_or(0));
        let cx = x.clamp(0.0, last_x);
        let cy = y.clamp(0.0, last_y);
        let x0 = cx.floor();
        let y0 = cy.floor();
        let x1 = (x0 + 1.0).min(last_x);
        let y1 = (y0 + 1.0).min(last_y);
        let tx = cx - x0;
        let ty = cy - y0;
        let at = |ix: f32, iy: f32| -> [f32; 3] {
            // Whole, non-negative coordinates below the image size, which the
            // clamp above proves; a miss reads zero rather than panicking.
            let ix = ix as u32;
            let iy = iy as u32;
            let index = usize::try_from(iy)
                .unwrap_or(0)
                .saturating_mul(usize::try_from(w).unwrap_or(1))
                .saturating_add(usize::try_from(ix).unwrap_or(0));
            src.get(index).copied().unwrap_or([0.0; 3])
        };
        let a = at(x0, y0);
        let b = at(x1, y0);
        let c = at(x0, y1);
        let d = at(x1, y1);
        let mut out = [0.0f32; 3];
        for ch in 0..3 {
            let top = a[ch] + (b[ch] - a[ch]) * tx;
            let bottom = c[ch] + (d[ch] - c[ch]) * tx;
            out[ch] = top + (bottom - top) * ty;
        }
        out
    }

    /// Texel-centre coordinate of destination texel `index` inside a source
    /// image: the one mapping the device shaders use (UV `(index + 0.5) /
    /// dst`, read back at texel coordinate `uv * src - 0.5`).
    fn dest_to_src(index: u32, dst: u32, src: u32) -> f32 {
        let index = f32::from(u16::try_from(index).unwrap_or(u16::MAX));
        let dst = f32::from(u16::try_from(dst).unwrap_or(u16::MAX));
        let src = f32::from(u16::try_from(src).unwrap_or(u16::MAX));
        (index + 0.5) * (src / dst.max(1.0)) - 0.5
    }

    /// Mip 0: the Karis-weighted bright extract at half size. Each destination
    /// texel averages its 2x2 source taps, weighted by `1 / (luma + 1)` so one
    /// hot texel cannot own the mip, and each tap is soft-knee extracted
    /// first, so a frame whose every tap sits under the knee writes exactly
    /// zero.
    #[must_use]
    pub(crate) fn bright_half(
        src: &[[f32; 3]],
        width: u32,
        height: u32,
        plan: &DualBloomPlan,
    ) -> Vec<[f32; 3]> {
        let (dw, dh) = plan.level_size(0);
        let mut out =
            vec![[0.0f32; 3]; usize::try_from(dw).unwrap_or(0) * usize::try_from(dh).unwrap_or(0)];
        for y in 0..dh {
            for x in 0..dw {
                let mut acc = [0.0f32; 3];
                let mut wsum = 0.0f32;
                for dy in 0..2u32 {
                    for dx in 0..2u32 {
                        let sx = (x * 2 + dx).min(width.saturating_sub(1));
                        let sy = (y * 2 + dy).min(height.saturating_sub(1));
                        let index = usize::try_from(sy)
                            .unwrap_or(0)
                            .saturating_mul(usize::try_from(width).unwrap_or(1))
                            .saturating_add(usize::try_from(sx).unwrap_or(0));
                        let Some(p) = src.get(index).copied() else {
                            continue;
                        };
                        let luma =
                            p[0] * BLOOM_LUMA[0] + p[1] * BLOOM_LUMA[1] + p[2] * BLOOM_LUMA[2];
                        let weight = 1.0 / (luma.max(0.0) + 1.0);
                        let knee = crate::post::bloom_weight(luma, plan.threshold, plan.knee);
                        for ch in 0..3 {
                            acc[ch] += p[ch] * knee * weight;
                        }
                        wsum += weight;
                    }
                }
                let inv = if wsum > 0.0 { 1.0 / wsum } else { 0.0 };
                let dst_index = usize::try_from(y)
                    .unwrap_or(0)
                    .saturating_mul(usize::try_from(dw).unwrap_or(1))
                    .saturating_add(usize::try_from(x).unwrap_or(0));
                if let Some(slot) = out.get_mut(dst_index) {
                    for ch in 0..3 {
                        slot[ch] = acc[ch] * inv;
                    }
                }
            }
        }
        out
    }

    /// One 13-tap step: a source mip into the next smaller mip. Destination
    /// texel `(x, y)` sits at the source coordinate `2x + 0.5, 2y + 0.5`, so
    /// every tap is a 2x2 bilinear average at weight 0.25; the four corner
    /// groups take `1/32` per tap and the centre group `1/8`.
    #[must_use]
    pub(crate) fn downsample13(
        src: &[[f32; 3]],
        width: u32,
        height: u32,
    ) -> (Vec<[f32; 3]>, u32, u32) {
        let dw = width.div_ceil(2).max(1);
        let dh = height.div_ceil(2).max(1);
        let mut out =
            vec![[0.0f32; 3]; usize::try_from(dw).unwrap_or(0) * usize::try_from(dh).unwrap_or(0)];
        let tap = |x: u32, y: u32, ox: f32, oy: f32| -> [f32; 3] {
            let cx = dest_to_src(x, dw, width) + ox;
            let cy = dest_to_src(y, dh, height) + oy;
            bilinear(src, width, height, cx, cy)
        };
        for y in 0..dh {
            for x in 0..dw {
                let a = tap(x, y, -2.0, -2.0);
                let b = tap(x, y, 0.0, -2.0);
                let c = tap(x, y, 2.0, -2.0);
                let d = tap(x, y, -1.0, -1.0);
                let e = tap(x, y, 1.0, -1.0);
                let f = tap(x, y, -2.0, 0.0);
                let g = tap(x, y, 0.0, 0.0);
                let h = tap(x, y, 2.0, 0.0);
                let i = tap(x, y, -1.0, 1.0);
                let j = tap(x, y, 1.0, 1.0);
                let k = tap(x, y, -2.0, 2.0);
                let l = tap(x, y, 0.0, 2.0);
                let m = tap(x, y, 2.0, 2.0);
                let mut value = [0.0f32; 3];
                for ch in 0..3 {
                    let corners = (a[ch] + b[ch] + f[ch] + g[ch])
                        + (b[ch] + c[ch] + g[ch] + h[ch])
                        + (f[ch] + g[ch] + k[ch] + l[ch])
                        + (g[ch] + h[ch] + l[ch] + m[ch]);
                    let centre = d[ch] + e[ch] + i[ch] + j[ch];
                    value[ch] =
                        corners * DOWNSAMPLE_CORNER_WEIGHT + centre * DOWNSAMPLE_CENTRE_WEIGHT;
                }
                let index = usize::try_from(y)
                    .unwrap_or(0)
                    .saturating_mul(usize::try_from(dw).unwrap_or(1))
                    .saturating_add(usize::try_from(x).unwrap_or(0));
                if let Some(slot) = out.get_mut(index) {
                    *slot = value;
                }
            }
        }
        (out, dw, dh)
    }

    /// One 3x3 tent upsample step: a smaller mip resampled to the larger one's
    /// grid, weights `1 2 1 / 2 4 2 / 1 2 1` over 16, each tap a bilinear
    /// sample.
    #[must_use]
    pub(crate) fn tent_upsample(
        src: &[[f32; 3]],
        width: u32,
        height: u32,
        dw: u32,
        dh: u32,
    ) -> Vec<[f32; 3]> {
        let mut out =
            vec![[0.0f32; 3]; usize::try_from(dw).unwrap_or(0) * usize::try_from(dh).unwrap_or(0)];
        for y in 0..dh {
            for x in 0..dw {
                let cx = dest_to_src(x, dw, width);
                let cy = dest_to_src(y, dh, height);
                let mut acc = [0.0f32; 3];
                let mut wsum = 0.0f32;
                for (oy, wy) in [(-1.0f32, 1.0f32), (0.0, 2.0), (1.0, 1.0)] {
                    for (ox, wx) in [(-1.0f32, 1.0f32), (0.0, 2.0), (1.0, 1.0)] {
                        let s = bilinear(src, width, height, cx + ox, cy + oy);
                        let w = wx * wy;
                        for ch in 0..3 {
                            acc[ch] += s[ch] * w;
                        }
                        wsum += w;
                    }
                }
                let inv = if wsum > 0.0 { 1.0 / wsum } else { 0.0 };
                let index = usize::try_from(y)
                    .unwrap_or(0)
                    .saturating_mul(usize::try_from(dw).unwrap_or(1))
                    .saturating_add(usize::try_from(x).unwrap_or(0));
                if let Some(slot) = out.get_mut(index) {
                    for ch in 0..3 {
                        slot[ch] = acc[ch] * inv;
                    }
                }
            }
        }
        out
    }

    /// The resolved down and up ladders, mip 0 first. The device records the
    /// same passes in the same order; the tests compare the two.
    #[derive(Clone, Debug)]
    pub(crate) struct DualBloomLadder {
        /// The bright extract plus every 13-tap mip, mip 0 first.
        pub(crate) down: Vec<Vec<[f32; 3]>>,
        /// The tent ladder, mip 0 first: `up[i] = (down[i] + tent(up[i+1])) / 2`.
        pub(crate) up: Vec<Vec<[f32; 3]>>,
    }

    /// Run the whole ladder on the CPU. Fails loudly on an image that does not
    /// match the plan's frame size, so the oracle never reads past the frame.
    pub(crate) fn dual_ladder(
        src: &[[f32; 3]],
        plan: &DualBloomPlan,
    ) -> Result<DualBloomLadder, PostError> {
        let want = usize::try_from(plan.width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(plan.height).unwrap_or(0));
        if src.len() != want {
            return Err(PostError::Invalid(format!(
                "bloom frame holds {} texels, plan wants {want}",
                src.len()
            )));
        }
        let sizes = plan.level_sizes();
        let mut down: Vec<Vec<[f32; 3]>> = Vec::with_capacity(sizes.len());
        down.push(bright_half(src, plan.width, plan.height, plan));
        for level in 1..sizes.len() {
            let (sw, sh) = sizes[level - 1];
            let Some(prev) = down.last() else {
                break;
            };
            let (mip, _, _) = downsample13(prev, sw, sh);
            down.push(mip);
        }
        // Tent ladder from the smallest mip up: `up[i] = (down[i] + tent(up[i+1])) / 2`.
        // The half blend keeps a flat frame's halo at the frame's own value
        // (every later rung adds only a damped copy), so a wide lit area
        // cannot grow with the ladder's length while a lone glint still
        // spreads over every mip.
        let mut up: Vec<Vec<[f32; 3]>> = vec![Vec::new(); sizes.len()];
        if let Some(last) = down.last() {
            if let Some(slot) = up.last_mut() {
                *slot = last.clone();
            }
        }
        for level in (0..sizes.len().saturating_sub(1)).rev() {
            let (sw, sh) = sizes[level + 1];
            let (dw, dh) = sizes[level];
            let (Some(smaller), Some(base)) = (up.get(level + 1), down.get(level)) else {
                continue;
            };
            let tent = tent_upsample(smaller, sw, sh, dw, dh);
            let mut acc = vec![[0.0f32; 3]; tent.len()];
            for (index, slot) in acc.iter_mut().enumerate() {
                let t = tent.get(index).copied().unwrap_or([0.0; 3]);
                let b = base.get(index).copied().unwrap_or([0.0; 3]);
                for ch in 0..3 {
                    slot[ch] = 0.5 * (b[ch] + t[ch]);
                }
            }
            if let Some(slot) = up.get_mut(level) {
                *slot = acc;
            }
        }
        Ok(DualBloomLadder { down, up })
    }

    /// The halo the blit adds: the tent ladder's mip 0, at half the frame size.
    pub(crate) fn halo(src: &[[f32; 3]], plan: &DualBloomPlan) -> Result<Vec<[f32; 3]>, PostError> {
        let ladder = dual_ladder(src, plan)?;
        Ok(ladder.up.into_iter().next().unwrap_or_default())
    }

    /// Composite the halo onto the frame the way the blit does: a bilinear
    /// sample of the half-res halo at each pixel centre, times `plan.intensity`,
    /// added to the linear frame before the tone map.
    #[must_use]
    pub(crate) fn composite(
        src: &[[f32; 3]],
        halo: &[[f32; 3]],
        plan: &DualBloomPlan,
    ) -> Vec<[f32; 3]> {
        let (hw, hh) = plan.level_size(0);
        let mut out = Vec::with_capacity(src.len());
        for y in 0..plan.height {
            for x in 0..plan.width {
                let uv_x = (f32::from(u16::try_from(x).unwrap_or(0)) + 0.5)
                    / f32::from(u16::try_from(plan.width).unwrap_or(1));
                let uv_y = (f32::from(u16::try_from(y).unwrap_or(0)) + 0.5)
                    / f32::from(u16::try_from(plan.height).unwrap_or(1));
                let hx = uv_x * f32::from(u16::try_from(hw).unwrap_or(1)) - 0.5;
                let hy = uv_y * f32::from(u16::try_from(hh).unwrap_or(1)) - 0.5;
                let h = bilinear(halo, hw, hh, hx, hy);
                let index = usize::try_from(y)
                    .unwrap_or(0)
                    .saturating_mul(usize::try_from(plan.width).unwrap_or(1))
                    .saturating_add(usize::try_from(x).unwrap_or(0));
                let base = src.get(index).copied().unwrap_or([0.0; 3]);
                let mut value = [0.0f32; 3];
                for ch in 0..3 {
                    value[ch] = base[ch] + h[ch] * plan.intensity;
                }
                out.push(value);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(width: u32, height: u32) -> DualBloomPlan {
        DualBloomPlan::for_frame(width, height, &BloomConfig::default()).expect("plan builds")
    }

    #[test]
    fn the_plan_halves_the_frame_and_stops_at_the_config_cap() {
        // Caller: the device chain's sizing (one texture per mip) and the
        // oracle. Behavior: mip 0 is half the frame, each later mip halves
        // again, and the count never passes the config's cap.
        let p = plan(1280, 720);
        assert_eq!(p.levels, BloomConfig::default().levels);
        assert_eq!(p.level_size(0), (640, 360));
        assert_eq!(p.level_size(1), (320, 180));
        assert_eq!(p.level_size(2), (160, 90));
        assert_eq!(p.level_size(3), (80, 45));
        let tiny = plan(4, 4);
        assert_eq!(tiny.level_sizes(), vec![(2, 2), (1, 1)]);
        let tall = plan(1, 1);
        assert_eq!(tall.level_sizes(), vec![(1, 1)]);
    }

    #[test]
    fn the_corner_and_centre_weights_sum_to_one() {
        // Caller: `bloom_dual_down.wgsl`, which spells the same numbers.
        // Behavior: the four corner groups plus the centre group weigh the
        // thirteen taps to exactly one.
        let corners = 4.0 * 4.0 * DOWNSAMPLE_CORNER_WEIGHT;
        let centre = 4.0 * DOWNSAMPLE_CENTRE_WEIGHT;
        assert!((corners + centre - 1.0).abs() <= 1e-6, "13 taps sum to 1");
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::oracle::*;
    use super::*;

    fn plan(width: u32, height: u32) -> DualBloomPlan {
        DualBloomPlan::for_frame(width, height, &BloomConfig::default()).expect("plan builds")
    }

    /// A seeded frame of dark-to-mid values with a handful of bright texels:
    /// the same input gives the same frame twice.
    fn seeded(width: u32, height: u32, bright: &[(u32, u32, f32)]) -> Vec<[f32; 3]> {
        let mut frame = Vec::with_capacity(width as usize * height as usize);
        let mut state = 0x9E37_79B9u32;
        for y in 0..height {
            for x in 0..width {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let base = 0.05 + (state >> 24) as f32 / 255.0 * 0.35;
                let mut p = [base, base * 0.8, base * 0.6];
                for (bx, by, nit) in bright {
                    if *bx == x && *by == y {
                        let n = *nit;
                        p = [n, n, n];
                    }
                }
                frame.push(p);
            }
        }
        frame
    }

    #[test]
    fn a_dark_frame_blooms_exactly_zero() {
        // Caller: the live gate and the capture twin. Behavior: when no texel
        // reaches the knee the bright extract and every mip are exactly zero,
        // so the halo can add nothing at all.
        let p = plan(32, 32);
        let dark = seeded(32, 32, &[]);
        let ladder = dual_ladder(&dark, &p).expect("ladder runs");
        for mip in ladder.down.iter().chain(ladder.up.iter()) {
            assert!(
                mip.iter().all(|t| t.iter().all(|c| *c == 0.0)),
                "a sub-knee frame resolves to exact zeros"
            );
        }
    }

    /// Mean of one linear frame over every channel.
    fn mean(frame: &[[f32; 3]]) -> f32 {
        let total: f32 = frame.iter().map(|p| p[0] + p[1] + p[2]).sum();
        total / (frame.len() as f32 * 3.0)
    }

    #[test]
    fn one_hot_texel_haloes_but_keeps_its_energy_small() {
        // Caller: the accept's 10000-nit leg. Behavior: one 10000-nit texel
        // lifts a local halo (the tent spreads it) while the Karis weight and
        // the 13-tap weights keep the halo's own value low, so the add-back
        // cannot own the frame.
        let p = plan(64, 64);
        let frame = seeded(64, 64, &[(32, 32, 10_000.0)]);
        let ladder = dual_ladder(&frame, &p).expect("ladder runs");
        let halo = halo(&frame, &p).expect("halo runs");
        assert_eq!(halo, ladder.up[0], "the blit's halo is rung 0");
        let peak = halo
            .iter()
            .flat_map(|t| t.iter())
            .fold(0.0f32, |m, v| m.max(*v));
        assert!(peak > 0.05, "the hot texel haloes, peak {peak}");
        assert!(
            peak < 2.0,
            "Karis and the 13-tap keep one hot texel's halo small, peak {peak}"
        );
        let lit = halo.iter().filter(|t| t[0] > 1e-5).count();
        assert!(
            lit >= 4,
            "the tent spreads the glint past its own texel: {lit} texels"
        );
        let total: f32 = halo.iter().map(|t| t[0]).sum();
        // The accept's frame-mean leg, on the CPU: the blit add-back leaves
        // the frame's mean inside one percent of itself.
        let base = mean(&frame);
        let lifted = mean(&composite(&frame, &halo, &p));
        let shift = (lifted - base).abs() / base;
        assert!(
            shift <= 0.01,
            "one 10000-nit texel moves the frame mean under 1%: {shift}"
        );
        eprintln!(
            "one 10000-nit texel: peak {peak}, lit {lit}, sum {total}, mean shift {:.4}%",
            shift * 100.0
        );
        assert!(
            total < 10_000.0 / 8.0,
            "the halo carries a small fraction of the texel's energy: {total}"
        );
    }

    #[test]
    fn the_ladder_is_repeatable_and_flat_frames_hold_their_value() {
        // Caller: the device parity test. Behavior: a flat frame above the
        // knee keeps its value through the 13-tap and tent ladders (all
        // weights sum to one), and the same input gives the same output twice.
        let p = plan(32, 32);
        let flat = vec![[4.0f32, 4.0, 4.0]; 32 * 32];
        let a = dual_ladder(&flat, &p).expect("ladder runs");
        let b = dual_ladder(&flat, &p).expect("ladder runs again");
        for (x, y) in a.down.iter().zip(b.down.iter()) {
            assert_eq!(x, y, "same input, same ladder twice");
        }
        for ch in 0..3 {
            for mip in a.down.iter().chain(a.up.iter()) {
                for t in mip {
                    assert!(
                        (t[ch] - 4.0).abs() <= 1e-4,
                        "a flat frame holds its value, got {}",
                        t[ch]
                    );
                }
            }
        }
    }
}

#[cfg(all(test, feature = "shader-validate"))]
mod shader_gate {
    //! The three kernels each build their own pipeline, so each must validate
    //! standalone. The crate-wide walk stops at an unrelated committed file
    //! (`outline.wgsl`), so this focused leg keeps RL36's kernels covered.

    #[test]
    fn the_three_bloom_kernels_validate_under_the_naga_gate() {
        for (name, src) in [
            (
                "post/bloom_dual_bright.wgsl",
                include_str!("shaders/post/bloom_dual_bright.wgsl"),
            ),
            (
                "post/bloom_dual_down.wgsl",
                include_str!("shaders/post/bloom_dual_down.wgsl"),
            ),
            (
                "post/bloom_dual_up.wgsl",
                include_str!("shaders/post/bloom_dual_up.wgsl"),
            ),
        ] {
            crate::shader_gate::validate_source(name, src).unwrap_or_else(|err| panic!("{err}"));
        }
    }
}
